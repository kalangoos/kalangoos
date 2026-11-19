//! Window switcher (Alt+Tab), as in the design ("12 · Janelas, busca e sistema", frame "Alternar janelas"):
//! the windows in the order they were last used, each with a live picture (niri renders it for us, without the
//! clipboard), its icon and title. Alt+Tab next, Alt+Shift+Tab previous, Q closes the selected window, Space asks
//! the agent about it, releasing Alt goes to it, Esc cancels.
//!
//! It appears only if Alt stays down for 120 ms: a quick Alt+Tab goes straight to the previous window. The
//! selection slides between the cards with the quick spring.

use crate::apps;
use crate::servicos::niri_acao;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

const SHOW_AFTER: Duration = Duration::from_millis(120);

pub struct Alternador {
    janela: gtk::ApplicationWindow,
    fundo: Animavel,
    painel: Animavel,
    faixa: gtk::Box,
    /// (window id, card)
    itens: RefCell<Vec<(u64, gtk::Widget)>>,
    sel: Cell<usize>,
    aberto: Cell<bool>,
    visivel: Cell<bool>,
    desde: Cell<Instant>,
    shell: RefCell<Weak<Shell>>,
    /// Alt's state as the keyboard reports it to us (None until our surface gets the keyboard)
    alt: Cell<Option<bool>>,
}

/// A window picture from niri, without the shadow GTK apps draw around themselves (it is part of their surface):
/// the box of fully opaque pixels is the window. Windows without that margin, or see-through ones, stay whole.
pub(crate) fn carregar_miniatura(path: &std::path::Path) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    if !pb.has_alpha() || pb.n_channels() != 4 {
        return Some(pb);
    }
    let (w, h, rs) = (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize);
    let px = pb.read_pixel_bytes();
    let opaco = |x: usize, y: usize| px.get(y * rs + x * 4 + 3).is_some_and(|a| *a >= 250);
    let linha = |y: usize| (0..w).step_by(2).any(|x| opaco(x, y));
    let coluna = |x: usize| (0..h).step_by(2).any(|y| opaco(x, y));
    let (Some(y0), Some(y1)) = ((0..h).find(|y| linha(*y)), (0..h).rev().find(|y| linha(*y))) else { return Some(pb) };
    let (Some(x0), Some(x1)) = ((0..w).find(|x| coluna(*x)), (0..w).rev().find(|x| coluna(*x))) else { return Some(pb) };
    let (cw, ch) = (x1 + 1 - x0, y1 + 1 - y0);
    if cw < 8 || ch < 8 || (cw + 4 >= w && ch + 4 >= h) {
        return Some(pb);
    }
    Some(pb.new_subpixbuf(x0 as i32, y0 as i32, cw as i32, ch as i32).copy().unwrap_or(pb))
}

pub(crate) fn thumbs_dir() -> PathBuf {
    let d = crate::voz::private_dir().join("miniaturas");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn key(t: &str) -> gtk::Label {
    rotulo(t, "tecla")
}

impl Alternador {
    pub fn new(app: &gtk::Application) -> Rc<Alternador> {
        let janela = camada(app, "kalango-alternador", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let raiz = gtk::Overlay::new();
        let escuro = caixa(Orientation::Vertical, 0, "alternador-fundo");
        escuro.set_hexpand(true);
        escuro.set_vexpand(true);
        let fundo = Animavel::new(&escuro);
        raiz.set_child(Some(&fundo));
        let caixa_painel = caixa(Orientation::Vertical, 12, "alternador");
        let faixa = caixa(Orientation::Horizontal, 10, "");
        caixa_painel.append(&faixa);
        let atalhos = caixa(Orientation::Horizontal, 14, "");
        atalhos.set_halign(Align::Center);
        for (keys, text) in [(vec!["Alt", "Tab"], tr("next")), (vec!["Alt", "Shift", "Tab"], tr("previous")), (vec!["Q"], tr("close")), (vec![tr("Space")], tr("ask the agent about this window"))] {
            let g = caixa(Orientation::Horizontal, 4, "");
            for k in keys {
                g.append(&key(k));
            }
            g.append(&rotulo(text, "alternador-atalho"));
            atalhos.append(&g);
        }
        caixa_painel.append(&atalhos);
        let painel = Animavel::new(&caixa_painel);
        painel.set_halign(Align::Center);
        painel.set_valign(Align::Center);
        raiz.add_overlay(&painel);
        janela.set_child(Some(&raiz));
        let a = Rc::new(Alternador {
            janela, fundo, painel, faixa, itens: RefCell::default(), sel: Cell::new(0), aberto: Cell::new(false), visivel: Cell::new(false),
            desde: Cell::new(Instant::now()), shell: RefCell::default(), alt: Cell::new(None),
        });
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let w = Rc::downgrade(&a);
            teclas.connect_key_pressed(move |_, k, _, m| {
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alternador: tecla {k:?} {m:?}");
                }
                let Some(a) = w.upgrade() else { return glib::Propagation::Proceed };
                match k {
                    gdk::Key::Tab | gdk::Key::Right => a.mover(1),
                    gdk::Key::ISO_Left_Tab | gdk::Key::Left => a.mover(-1),
                    gdk::Key::q | gdk::Key::Q => a.fechar_selecionada(),
                    gdk::Key::space => a.perguntar(),
                    gdk::Key::Return | gdk::Key::KP_Enter => a.confirmar(),
                    gdk::Key::Escape => a.cancelar(),
                    _ => return glib::Propagation::Proceed,
                }
                let _ = m;
                glib::Propagation::Stop
            });
            let w = Rc::downgrade(&a);
            // releasing Alt goes to the selected window
            teclas.connect_key_released(move |_, k, _, m| {
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alternador: soltou {k:?} {m:?}");
                }
                if matches!(k, gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::Meta_L | gdk::Key::Meta_R) {
                    if let Some(a) = w.upgrade() {
                        a.confirmar();
                    }
                }
            });
        }
        // the modifier state arrives with the keyboard focus (and whenever it changes)
        {
            let w = Rc::downgrade(&a);
            teclas.connect_modifiers(move |_, m| {
                let held = m.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug alternador: modificadores {m:?}");
                }
                if let Some(a) = w.upgrade() {
                    a.alt.set(Some(held));
                    // Alt went up: go to the selected window (a quick tap arrives here too)
                    if !held && a.aberto.get() {
                        a.confirmar();
                    }
                }
                glib::Propagation::Proceed
            });
        }
        a.janela.add_controller(teclas);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let f = gtk::EventControllerFocus::new();
            f.connect_enter(|_| eprintln!("debug alternador: teclado chegou"));
            a.janela.add_controller(f);
        }
        // when the keyboard arrives, Alt may already be up (a quick tap): then go straight to the window. The
        // modifier state comes right after the keyboard, so it is read a moment later
        let w = Rc::downgrade(&a);
        a.janela.connect_is_active_notify(move |j| {
            if !j.is_active() {
                return;
            }
            let w = w.clone();
            glib::timeout_add_local_once(Duration::from_millis(60), move || {
                if let Some(a) = w.upgrade() {
                    if a.aberto.get() && a.alt.get() == Some(false) {
                        a.confirmar();
                    }
                }
            });
        });
        a
    }

    pub fn ligar(&self, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
    }

    /// The windows, the one used last first (the shell follows niri's focus changes).
    fn ordem(&self) -> Vec<(u64, String, String)> {
        let Some(s) = self.shell.borrow().upgrade() else { return vec![] };
        let mut v: Vec<(u64, u64, String, String)> = s.estado.janelas.borrow().values().map(|j| (j.foco_em, j.id, j.app_id.clone(), j.titulo.clone())).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.0));
        v.into_iter().map(|(_, id, app, t)| (id, app, t)).collect()
    }

    /// Alt+Tab (or Alt+Shift+Tab with `back`): opens with the next window selected, or moves the selection.
    pub fn alternar(self: &Rc<Self>, back: bool) {
        if self.aberto.get() {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug alternador: pedido com ele aberto (move a seleção)");
            }
            self.mover(if back { -1 } else { 1 });
            return;
        }
        let janelas = self.ordem();
        if janelas.len() < 2 {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                // what niri itself has, next to what the shell knows (they should be the same)
                let niri: Vec<String> = crate::servicos::niri_pedido(serde_json::json!("Windows")).ok()
                    .and_then(|v| v["Windows"].as_array().cloned()).unwrap_or_default().iter()
                    .map(|w| format!("{}:{}@{}", w["id"], w["app_id"].as_str().unwrap_or("?"), w["workspace_id"])).collect();
                eprintln!("debug alternador: pedido com {} janela(s): nada a alternar (o shell: {:?}; o niri: {:?})", janelas.len(),
                    janelas.iter().map(|(i, a, _)| format!("{i}:{a}")).collect::<Vec<_>>(), niri);
            }
            return;
        }
        self.aberto.set(true);
        self.visivel.set(false);
        self.alt.set(None);
        self.desde.set(Instant::now());
        self.montar(&janelas);
        self.select(if back { janelas.len() - 1 } else { 1 });
        // keyboard now (to see Alt go up), the picture only after 120 ms
        // 1% instead of 0: invisible, but the window draws real frames, so the keyboard settings reach niri now
        self.fundo.definir(Canal::Opacidade, 0.01);
        self.painel.definir(Canal::Opacidade, 0.0);
        // exclusive keyboard from the first commit, and again after mapping (niri re-evaluates the focus). A
        // transparent, still window draws no frame, and the change only reaches niri with a frame: ask for one
        crate::superficies::take_keyboard(&self.janela);
        self.janela.present();
        let j = self.janela.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            if let Some(j) = j.upgrade() {
                j.set_keyboard_mode(KeyboardMode::None);
                crate::superficies::take_keyboard(&j);
                j.queue_draw();
                if let Some(c) = j.child() {
                    c.queue_draw();
                }
            }
        });
        let w = Rc::downgrade(self);
        let inicio = Instant::now();
        glib::timeout_add_local(SHOW_AFTER, move || {
            let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
            if !a.aberto.get() {
                return glib::ControlFlow::Break;
            }
            // until the keyboard is here, Alt's state is not known yet: wait for it (a little); without news, Alt
            // is taken as held (the shortcut that opened us had it) and its release will come to us
            let known = a.janela.is_active() || a.alt.get().is_some();
            if !known && inicio.elapsed() < Duration::from_millis(600) {
                return glib::ControlFlow::Continue;
            }
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                let m = gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard()).map(|k| k.modifier_state());
                eprintln!("debug alternador: decisão ativo={} alt={} modificadores={m:?} após {} ms", a.janela.is_active(), alt_down(), inicio.elapsed().as_millis());
            }
            // with the keyboard here, GTK knows whether Alt is still down
            let held = if a.janela.is_active() { alt_down() } else { a.alt.get().unwrap_or(true) };
            if !held {
                a.confirmar();
                return glib::ControlFlow::Break;
            }
            a.visivel.set(true);
            a.fundo.ir(Canal::Opacidade, 1.0, mv::RAPIDA);
            a.painel.entrar((0.0, 8.0), 0.97);
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug alternador: aberto com {} janelas", a.itens.borrow().len());
            }
            glib::ControlFlow::Break
        });
    }

    fn montar(self: &Rc<Self>, janelas: &[(u64, String, String)]) {
        while let Some(c) = self.faixa.first_child() {
            self.faixa.remove(&c);
        }
        let todos = self.shell.borrow().upgrade().map(|s| s.todos.clone()).unwrap_or_default();
        let mut itens = Vec::new();
        for (id, app_id, titulo) in janelas.iter().take(8) {
            let card = caixa(Orientation::Vertical, 8, "alternador-cartao");
            let quadro = gtk::Overlay::new();
            let fundo = caixa(Orientation::Vertical, 0, "alternador-miniatura");
            fundo.set_size_request(220, 138);
            quadro.set_child(Some(&fundo));
            let app = apps::da_janela(app_id, &todos);
            // the app icon while the picture is made; then the picture
            let icone = app.map(|a| apps::imagem(a, 48)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable"));
            icone.set_halign(Align::Center);
            icone.set_valign(Align::Center);
            quadro.add_overlay(&icone);
            card.append(&quadro);
            let linha = caixa(Orientation::Horizontal, 6, "");
            if let Some(a) = app {
                linha.append(&apps::imagem(a, 16));
            }
            let nome = app.map(|a| a.nome.clone()).unwrap_or_else(|| app_id.clone());
            let texto = if titulo.is_empty() || *titulo == nome { nome } else { format!("{titulo} — {nome}") };
            let l = rotulo(&texto, "alternador-titulo");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            l.set_xalign(0.0);
            linha.append(&l);
            card.append(&linha);
            let clique = gtk::GestureClick::new();
            let (w, i) = (Rc::downgrade(self), itens.len());
            clique.connect_released(move |_, _, _, _| {
                if let Some(a) = w.upgrade() {
                    a.select(i);
                    a.confirmar();
                }
            });
            card.add_controller(clique);
            self.faixa.append(&card);
            itens.push((*id, card.clone().upcast::<gtk::Widget>()));
            pedir_miniatura(*id, &quadro);
        }
        *self.itens.borrow_mut() = itens;
    }

    fn select(&self, i: usize) {
        let itens = self.itens.borrow();
        for (k, (_, w)) in itens.iter().enumerate() {
            if k == i { w.add_css_class("selecionado") } else { w.remove_css_class("selecionado") }
        }
        self.sel.set(i);
    }

    fn mover(&self, d: i32) {
        let n = self.itens.borrow().len() as i32;
        if n == 0 {
            return;
        }
        self.select(((self.sel.get() as i32 + d).rem_euclid(n)) as usize);
    }

    fn selecionada(&self) -> Option<u64> {
        self.itens.borrow().get(self.sel.get()).map(|(id, _)| *id)
    }

    fn esconder(&self) {
        self.aberto.set(false);
        self.janela.set_keyboard_mode(KeyboardMode::None);
        if self.visivel.replace(false) {
            self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
            let j = self.janela.downgrade();
            self.painel.sair((0.0, 8.0), 0.97, move || {
                if let Some(j) = j.upgrade() {
                    j.set_visible(false);
                }
            });
        } else {
            self.janela.set_visible(false);
        }
    }

    /// Goes to the selected window.
    fn confirmar(&self) {
        if !self.aberto.get() {
            return;
        }
        let id = self.selecionada();
        self.esconder();
        if let Some(id) = id {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug alternador: foco na janela {id}");
            }
            let _ = niri_acao(json!({"FocusWindow": {"id": id}}));
        }
    }

    fn cancelar(&self) {
        if self.aberto.get() {
            self.esconder();
        }
    }

    /// Q: closes the selected window; the switcher stays open with the others.
    fn fechar_selecionada(self: &Rc<Self>) {
        let Some(id) = self.selecionada() else { return };
        let _ = niri_acao(json!({"CloseWindow": {"id": id}}));
        let i = self.sel.get();
        let card = self.itens.borrow_mut().remove(i).1;
        self.faixa.remove(&card);
        if self.itens.borrow().is_empty() {
            self.cancelar();
            return;
        }
        self.select(i.min(self.itens.borrow().len() - 1));
    }

    /// Space: goes to the window and opens the command bar with it as context.
    fn perguntar(&self) {
        self.confirmar();
        let s = self.shell.borrow().clone();
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(s) = s.upgrade() {
                s.comando().abrir("");
            }
        });
    }
}

/// Is Alt (or the Mod key) held right now?
fn alt_down() -> bool {
    gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard())
        .map(|k| k.modifier_state().intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK))
        .unwrap_or(false)
}

/// Asks niri for a small picture of the window and shows it in the card when it is ready.
fn pedir_miniatura(id: u64, quadro: &gtk::Overlay) {
    let path = thumbs_dir().join(format!("{id}.png"));
    let _ = std::fs::remove_file(&path);
    if niri_acao(json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 220}})).is_err() {
        return;
    }
    let q = quadro.downgrade();
    let t0 = Instant::now();
    glib::timeout_add_local(Duration::from_millis(30), move || {
        let Some(q) = q.upgrade() else { return glib::ControlFlow::Break };
        if path.is_file() {
            let Some(pb) = carregar_miniatura(&path) else { return glib::ControlFlow::Continue };
            #[allow(deprecated)]
            let tex = gdk::Texture::for_pixbuf(&pb);
            let pic = gtk::Picture::for_paintable(&tex);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(220, 138);
            pic.add_css_class("alternador-imagem");
            q.add_overlay(&pic);
            let _ = std::fs::remove_file(&path);
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug alternador: miniatura da janela {id} ({}x{})", tex.width(), tex.height());
            }
            return glib::ControlFlow::Break;
        }
        if t0.elapsed() > Duration::from_secs(2) {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
