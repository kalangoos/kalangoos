//! App grid (design "Grade de aplicativos", three zones and the agent): the whole screen, not a block in the middle.
//! - AGORA, at the left: what is usually opened at this hour, approvals waiting, the music that was playing, the app
//!   used last, and the files used last (opened with their app);
//! - the apps, in the middle: cards by category (Criação, Trabalho, Internet, Desenvolvimento, Inteligência, Jogos,
//!   Sistema) that flow in columns with the width — three on a Full HD screen, wide ones with bigger icons on an
//!   ultrawide; typing filters them;
//! - AGENTE, at the right: what the field asks ("algo para editar fotos"), answered by the local model with what this
//!   computer has, what the Central de Apps' catalog (AppStream) offers to install (the agent installs it, with its
//!   approval), and a recent file to open with it.
//! The scrim follows the theme (the wallpaper frosted light in the light theme, dark in the dark one).
//!
//! Motion: opens from the dock button with 0.96 → 1 and a fade; the side zones come in from 16 px out, the cards in
//! the 24 ms cascade.

use crate::apps::{self, App};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, entrada_sem_furos, furos, rotulo, veu_de, Furo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The cards, in this order.
const CATEGORIAS: &[(&str, &str)] = &[
    ("criacao", n_("CREATION")),
    ("trabalho", n_("WORK")),
    ("internet", n_("INTERNET")),
    ("desenvolvimento", n_("DEVELOPMENT")),
    ("ia", n_("INTELLIGENCE")),
    ("jogos", n_("GAMES")),
    ("sistema", n_("SYSTEM")),
    ("outros", n_("OTHERS")),
];

/// A suggestion card: (icon, name, what it does, why it is suggested).
type Suggestion = (gtk::Widget, String, Rc<dyn Fn()>, String);

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The grid's card for an app (the desktop's categories, finer than the shell's own).
pub fn cartao_de(a: &App) -> &'static str {
    let geral = apps::categoria(a);
    if geral == "ia" || geral == "desenvolvimento" {
        return geral;
    }
    let cats = a.info.categories().map(|c| c.to_string()).unwrap_or_default();
    let has = |c: &str| cats.split(';').any(|x| x == c);
    if has("Game") {
        "jogos"
    } else if has("Office") || has("TextEditor") || has("Calendar") || has("Email") || has("ContactManagement") || has("FileManager") || has("Calculator") || has("Archiving") || has("Viewer") && has("Office") {
        "trabalho"
    } else if has("Graphics") || has("Photography") || has("AudioVideo") || has("Audio") || has("Video") {
        "criacao"
    } else {
        geral
    }
}

/// What the local model says a request means for apps.
#[derive(Default, Clone)]
struct Sentido {
    palavras: Vec<String>,
    categorias: Vec<String>,
    tipos: Vec<String>,
}

/// An app the catalog offers (AppStream): its name, what it is, its package.
#[derive(Clone)]
struct Oferta {
    id: String,
    nome: String,
    resumo: String,
    pacote: String,
}

/// `appstreamcli search` for one word: the desktop apps, in its own words (the person's language).
fn catalogo(palavra: &str) -> Vec<Oferta> {
    let Ok(o) = std::process::Command::new("appstreamcli").args(["search", palavra]).stderr(std::process::Stdio::null()).output() else { return vec![] };
    let texto = String::from_utf8_lossy(&o.stdout);
    texto.split("---").filter_map(|bloco| {
        // "Chave: valor" lines, translated keys: by their order (Identifier, Name, Summary, Package)
        let campos: Vec<(String, String)> = bloco.lines().filter_map(|l| l.split_once(": ").map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        let id = campos.first()?.1.clone();
        if !id.contains("desktop-application") {
            return None;
        }
        let pega = |i: usize| campos.get(i).map(|x| x.1.clone()).unwrap_or_default();
        Some(Oferta { id: id.split_whitespace().next().unwrap_or("").trim_end_matches(".desktop").to_string(), nome: pega(1), resumo: pega(2), pacote: pega(3) })
    }).filter(|o| !o.nome.is_empty() && !o.pacote.is_empty()).collect()
}

pub struct Grade {
    janela: gtk::ApplicationWindow,
    fundo: Animavel,
    conteudo: Animavel,
    esquerda: Animavel,
    direita: Animavel,
    zona_esquerda: gtk::Box,
    zona_direita: gtk::Box,
    entrada: gtk::Entry,
    sugeridos: gtk::Box,
    recentes: gtk::Box,
    /// the cards' columns (a masonry: each card goes to the shortest column, hugging its apps)
    cartoes: gtk::Box,
    agente: gtk::Box,
    pedido: Cell<u64>,
    espera: RefCell<Option<glib::SourceId>>,
    icone_px: Cell<i32>,
    /// the cards' columns and width, and the icons per line in each (measured for this screen)
    colunas: Cell<(u32, i32, u32)>,
    /// the agent's answer: the apps it found, shown in the cards while the request stands
    achados: RefCell<Option<Vec<App>>>,
    buracos: Rc<RefCell<Vec<Furo>>>,
    pub aberta: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
}

fn secao(texto: &str, n: Option<usize>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    let ic = ui_icon("spark", "#A5A1E2", 12);
    ic.set_valign(Align::Center);
    r.append(&ic);
    r.append(&rotulo(texto, "grade-secao"));
    if let Some(n) = n {
        r.append(&rotulo(&n.to_string(), "grade-secao-n"));
    }
    r
}

fn rotulo_esq(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

impl Grade {
    pub fn new(app: &gtk::Application) -> Rc<Grade> {
        let janela = camada(app, "kalango-grade-de-apps", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let raiz = gtk::Overlay::new();
        // the scrim (the theme's background over the wallpaper) leaves the bar and the dock out
        let buracos: Rc<RefCell<Vec<Furo>>> = Rc::default();
        let fundo = Animavel::new(&veu_de("#16171B", 0.9, buracos.clone()));
        raiz.set_child(Some(&fundo));
        let col = caixa(Orientation::Vertical, 20, "");
        // search (and ask): "Buscar apps ou pedir…"
        let busca = caixa(Orientation::Horizontal, 10, "grade-busca");
        busca.set_halign(Align::Center);
        busca.set_size_request(640, -1);
        busca.append(&ui_icon("spark", "#A5A1E2", 16));
        let entrada = gtk::Entry::new();
        entrada.set_hexpand(true);
        entrada.add_css_class("grade-campo");
        entrada.set_placeholder_text(Some(tr("Search apps or ask: “open something to edit photos”")));
        busca.append(&entrada);
        busca.append(&rotulo("Ctrl K", "tecla"));
        col.append(&busca);
        // the three zones
        let zonas = caixa(Orientation::Horizontal, 24, "");
        zonas.set_vexpand(true);
        let zona_esquerda = caixa(Orientation::Vertical, 12, "grade-zona");
        zona_esquerda.set_valign(Align::Start);
        zona_esquerda.append(&secao(tr("NOW"), None));
        let sugeridos = caixa(Orientation::Vertical, 4, "");
        zona_esquerda.append(&sugeridos);
        let recentes = caixa(Orientation::Vertical, 2, "");
        zona_esquerda.append(&recentes);
        let esquerda = Animavel::new(&zona_esquerda);
        esquerda.set_valign(Align::Start);
        zonas.append(&esquerda);
        let cartoes = caixa(Orientation::Horizontal, 16, "");
        cartoes.set_valign(Align::Start);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_child(Some(&cartoes));
        rolagem.set_hexpand(true);
        rolagem.set_vexpand(true);
        zonas.append(&rolagem);
        let zona_direita = caixa(Orientation::Vertical, 12, "grade-zona");
        zona_direita.add_css_class("agente");
        zona_direita.set_valign(Align::Start);
        zona_direita.append(&secao(tr("AGENT"), None));
        let agente = caixa(Orientation::Vertical, 10, "");
        zona_direita.append(&agente);
        let direita = Animavel::new(&zona_direita);
        direita.set_valign(Align::Start);
        zonas.append(&direita);
        col.append(&zonas);
        let conteudo = Animavel::new(&col);
        // design: the search at y = 60 (32 px of bar + 28), the zones 40 px from the sides, above the dock
        conteudo.set_margin_top(60);
        conteudo.set_margin_start(40);
        conteudo.set_margin_end(40);
        conteudo.set_margin_bottom(92);
        raiz.add_overlay(&conteudo);
        janela.set_child(Some(&raiz));

        let g = Rc::new(Grade {
            janela, fundo, conteudo, esquerda, direita, zona_esquerda, zona_direita, entrada, sugeridos, recentes, cartoes, agente, pedido: Cell::new(0),
            espera: RefCell::default(), icone_px: Cell::new(52), colunas: Cell::new((3, 352, 3)), achados: RefCell::default(), buracos, aberta: Cell::new(false), shell: RefCell::default(),
        });
        let w = Rc::downgrade(&g);
        g.entrada.connect_changed(move |e| {
            let Some(g) = w.upgrade() else { return };
            g.preencher(false);
            // a request (two words or more), once the typing pauses: the agent answers it
            if let Some(t) = g.espera.borrow_mut().take() {
                t.remove();
            }
            let q = e.text().trim().to_string();
            g.achados.borrow_mut().take();
            if q.split_whitespace().count() >= 2 {
                let w = Rc::downgrade(&g);
                *g.espera.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    if let Some(g) = w.upgrade() {
                        g.espera.borrow_mut().take();
                        g.perguntar(&q);
                    }
                }));
            } else if q.is_empty() {
                g.agente_ocioso();
            }
        });
        let w = Rc::downgrade(&g);
        g.entrada.connect_activate(move |e| {
            let Some(g) = w.upgrade() else { return };
            let q = e.text().trim().to_string();
            if q.is_empty() {
                return;
            }
            // a name: the first app opens; anything else, the agent answers here
            let todos = g.todos();
            match apps::buscar(&todos, &q).first() {
                Some(a) => g.abrir_app(a),
                None => g.perguntar(&q),
            }
        });
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&g);
        teclas.connect_key_pressed(move |_, k, _, m| {
            let Some(g) = w.upgrade() else { return glib::Propagation::Proceed };
            match k {
                gdk::Key::Escape => g.fechar(),
                gdk::Key::Return | gdk::Key::KP_Enter if m.contains(gdk::ModifierType::CONTROL_MASK) => {
                    let q = g.entrada.text().trim().to_string();
                    if !q.is_empty() {
                        g.perguntar(&q);
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        g.janela.add_controller(teclas);
        // a click on the scrim (outside the zones) closes
        let clique = gtk::GestureClick::new();
        let w = Rc::downgrade(&g);
        clique.connect_pressed(move |_, _, x, y| {
            let Some(g) = w.upgrade() else { return };
            let p = gtk::graphene::Point::new(x as f32, y as f32);
            let dentro = [g.esquerda.upcast_ref::<gtk::Widget>(), g.direita.upcast_ref(), g.cartoes.upcast_ref(), g.entrada.upcast_ref()].iter()
                .any(|z| z.is_visible() && z.compute_bounds(&g.janela).is_some_and(|b| b.contains_point(&p)));
            if !dentro {
                g.fechar();
            }
        });
        g.fundo.add_controller(clique);
        g
    }

    pub fn ligar(&self, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
    }

    fn todos(&self) -> Vec<App> {
        self.shell.borrow().upgrade().map(|s| s.todos.clone()).unwrap_or_default()
    }

    fn abrir_app(&self, a: &App) {
        self.fechar();
        if let Err(e) = apps::abrir(a) {
            eprintln!("kalango-shell: abrir {}: {e}", a.nome);
        }
    }

    /// The app's tile: its icon and name; a click opens it.
    fn tile(self: &Rc<Self>, a: &App) -> gtk::Widget {
        let px = self.icone_px.get();
        let tile = caixa(Orientation::Vertical, 6, "grade-app");
        tile.set_size_request(if px > 60 { 140 } else { 96 }, -1);
        let img = apps::imagem(a, px);
        img.set_halign(Align::Center);
        tile.append(&img);
        let n = rotulo(&a.nome, "grade-nome");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        n.set_hexpand(true);
        tile.append(&n);
        tile.set_tooltip_text(Some(&a.nome));
        let clique = gtk::GestureClick::new();
        let (w, app) = (Rc::downgrade(self), a.clone());
        clique.connect_released(move |_, _, _, _| {
            if let Some(g) = w.upgrade() {
                g.abrir_app(&app);
            }
        });
        tile.add_controller(clique);
        tile.upcast()
    }

    /// The category cards, for the field's words (a card with nothing that matches goes).
    fn preencher(self: &Rc<Self>, cascata: bool) {
        while let Some(c) = self.cartoes.first_child() {
            self.cartoes.remove(&c);
        }
        let (n_colunas, _, _) = self.colunas.get();
        let colunas: Vec<gtk::Box> = (0..n_colunas.max(1)).map(|_| {
            let c = caixa(Orientation::Vertical, 16, "");
            c.set_valign(Align::Start);
            self.cartoes.append(&c);
            c
        }).collect();
        // each column's height so far, in rows of icons (a card: its title and its rows)
        let mut alturas = vec![0u32; colunas.len()];
        let q = self.entrada.text().trim().to_string();
        let todos = self.todos();
        let (_, largura_cartao, por_linha) = self.colunas.get();
        let lista: Vec<App> = match (q.is_empty(), self.achados.borrow().clone()) {
            (true, _) => todos.clone(),
            (false, Some(a)) => a,
            (false, None) => apps::buscar(&todos, &q).into_iter().cloned().collect(),
        };
        let mut anims = Vec::new();
        for (id, nome) in CATEGORIAS {
            let mut deste: Vec<&App> = lista.iter().filter(|a| cartao_de(a) == *id).collect();
            if deste.is_empty() {
                continue;
            }
            if q.is_empty() {
                deste.sort_by_key(|a| a.nome.to_lowercase());
            }
            let cartao = caixa(Orientation::Vertical, 10, "grade-cartao");
            cartao.set_size_request(largura_cartao, -1);
            cartao.append(&secao(tr(nome), Some(deste.len())));
            let tiles = gtk::FlowBox::new();
            tiles.set_selection_mode(gtk::SelectionMode::None);
            tiles.set_homogeneous(true);
            tiles.set_min_children_per_line(por_linha);
            tiles.set_max_children_per_line(por_linha);
            tiles.set_row_spacing(4);
            tiles.set_column_spacing(0);
            let deste_n = deste.len();
            for a in deste {
                tiles.insert(&self.tile(a), -1);
            }
            cartao.append(&tiles);
            let anim = Animavel::new(&cartao);
            let k = (0..colunas.len()).min_by_key(|&i| alturas[i]).unwrap_or(0);
            alturas[k] += 1 + (deste_n as u32).div_ceil(por_linha.max(1));
            colunas[k].append(&anim);
            anims.push(anim);
        }
        if anims.is_empty() && !q.is_empty() {
            let v = rotulo(&trf("No app called “{q}”. The agent looks for it at the right.", &[("q", q)]), "grade-vazio");
            v.set_xalign(0.0);
            colunas[0].append(&v);
        }
        mv::cascata(&anims, if cascata { 6.0 } else { 3.0 });
        if debug() {
            eprintln!("debug grade: {} cartões", anims.len());
        }
    }

    /// AGORA's suggestions: real reasons only (the person's own use, the system's state).
    fn render_sugeridos(self: &Rc<Self>) {
        while let Some(c) = self.sugeridos.first_child() {
            self.sugeridos.remove(&c);
        }
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        let todos = self.todos();
        let agora = glib::DateTime::now_local().ok();
        let hora = agora.as_ref().map(|d| d.hour()).unwrap_or(12);
        let now = agora.as_ref().map(|d| d.to_unix()).unwrap_or(0);
        let mut cards: Vec<Suggestion> = Vec::new();
        let mut usados: Vec<String> = Vec::new();
        let uses = apps::uses();
        // the app usually opened around this hour (last 30 days, at least twice)
        let mut conta: std::collections::HashMap<String, usize> = Default::default();
        for (id, t) in uses.iter().filter(|(_, t)| now - t < 30 * 86400) {
            if let Ok(d) = glib::DateTime::from_unix_local(*t) {
                if (d.hour() - hora).abs() <= 1 {
                    *conta.entry(id.clone()).or_default() += 1;
                }
            }
        }
        let mut por_hora: Vec<(String, usize)> = conta.into_iter().filter(|(_, n)| *n >= 2).collect();
        por_hora.sort_by_key(|x| std::cmp::Reverse(x.1));
        if let Some((id, _)) = por_hora.first() {
            if let Some(a) = todos.iter().find(|a| a.id == *id) {
                let app = a.clone();
                let w = Rc::downgrade(self);
                cards.push((apps::imagem(a, 32).upcast(), a.nome.clone(), Rc::new(move || if let Some(g) = w.upgrade() { g.abrir_app(&app) }), trf("you usually open it around {h}h", &[("h", hora.to_string())])));
                usados.push(id.clone());
            }
        }
        // approvals waiting: the agent in the side bar
        if let Some(n) = shell.estado.pendentes.get().filter(|n| *n > 0) {
            let s = Rc::downgrade(&shell);
            let me = Rc::downgrade(self);
            cards.push((ui_icon("spark", "#A5A1E2", 32).upcast(), tr("Agents").to_string(), Rc::new(move || {
                if let Some(g) = me.upgrade() {
                    g.fechar();
                }
                if let Some(s) = s.upgrade() {
                    s.right_bar().abrir(crate::lateral::Modo::Agente);
                }
            }), if n == 1 { tr("1 approval waiting").to_string() } else { trf("{n} approvals waiting", &[("n", n.to_string())]) }));
        }
        // Downloads changed in the last 2 hours: Files
        let downloads = glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(|| glib::home_dir().join("Downloads"));
        let mudou = std::fs::metadata(&downloads).ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < 7200);
        if mudou {
            if let Some(a) = todos.iter().find(|a| a.id.to_lowercase().contains("nautilus") || a.icone_kalango == Some("arquivos")) {
                let me = Rc::downgrade(self);
                let d = downloads.clone();
                cards.push((apps::imagem(a, 32).upcast(), a.nome.clone(), Rc::new(move || {
                    if let Some(g) = me.upgrade() {
                        g.fechar();
                    }
                    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&d).uri(), gdk::Display::default().map(|x| x.app_launch_context()).as_ref());
                }), tr("Downloads changed").to_string()));
                usados.push(a.id.clone());
            }
        }
        // the music that was playing
        if let Some(m) = shell.midia.atual() {
            let s = Rc::downgrade(&shell);
            let me = Rc::downgrade(self);
            cards.push((crate::midia::capa(&m, 32), m.app.clone(), Rc::new(move || {
                if let Some(g) = me.upgrade() {
                    g.fechar();
                }
                if let Some(s) = s.upgrade() {
                    s.midia.comando("PlayPause");
                }
            }), trf("continue {title}", &[("title", m.titulo.clone())])));
        }
        // the app used last (in the last 6 hours)
        if let Some((id, t)) = uses.iter().rev().find(|(id, t)| now - t < 6 * 3600 && !usados.contains(id)) {
            if let Some(a) = todos.iter().find(|a| a.id == *id) {
                let app = a.clone();
                let w = Rc::downgrade(self);
                let min = ((now - t) / 60).max(1);
                cards.push((apps::imagem(a, 32).upcast(), a.nome.clone(), Rc::new(move || if let Some(g) = w.upgrade() { g.abrir_app(&app) }),
                    if min < 60 { trf("used {n} min ago", &[("n", min.to_string())]) } else { trf("used {n} h ago", &[("n", (min / 60).to_string())]) }));
            }
        }
        if debug() {
            eprintln!("debug grade: {} sugestões: {}", cards.len(), cards.iter().map(|c| format!("{} ({})", c.1, c.3)).collect::<Vec<_>>().join("; "));
        }
        if cards.is_empty() {
            let l = rotulo_esq(tr("What you use shows up here: the apps of this hour, the files of today."), "grade-agente-nota");
            l.set_wrap(true);
            self.sugeridos.append(&l);
        }
        for (icone, nome, acao, motivo) in cards.into_iter().take(5) {
            let b = gtk::Button::new();
            b.add_css_class("grade-sugestao");
            let linha = caixa(Orientation::Horizontal, 10, "");
            linha.append(&icone);
            let col = caixa(Orientation::Vertical, 0, "");
            col.set_valign(Align::Center);
            col.append(&rotulo_esq(&nome, "grade-sugestao-nome"));
            let m = rotulo_esq(&motivo, "grade-sugestao-motivo");
            m.set_ellipsize(gtk::pango::EllipsizeMode::End);
            m.set_max_width_chars(1);
            m.set_hexpand(true);
            col.append(&m);
            col.set_hexpand(true);
            linha.append(&col);
            b.set_child(Some(&linha));
            b.connect_clicked(move |_| acao());
            self.sugeridos.append(&b);
        }
    }

    /// The files used last (the system's recent list), each opened with its app.
    fn render_recentes(self: &Rc<Self>) {
        while let Some(c) = self.recentes.first_child() {
            self.recentes.remove(&c);
        }
        let lista: Vec<(PathBuf, String, i64)> = crate::arquivos::seletor::recentes_com_tipo(4);
        if lista.is_empty() {
            return;
        }
        let t = secao(tr("RECENT FILES"), None);
        t.set_margin_top(8);
        self.recentes.append(&t);
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        for (p, tipo, quando) in lista {
            let app = gio::AppInfo::default_for_type(&tipo, false);
            let b = gtk::Button::new();
            b.add_css_class("grade-recente");
            let linha = caixa(Orientation::Horizontal, 10, "");
            let glifo = if tipo.starts_with("image/") { "imagem" } else if tipo.starts_with("video/") { "video-d" } else if tipo.starts_with("audio/") { "musica" } else { "doc" };
            let ic = ui_icon(glifo, "#8E8F99", 16);
            ic.set_valign(Align::Center);
            linha.append(&ic);
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            let n = rotulo_esq(&p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), "grade-recente-nome");
            n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            n.set_max_width_chars(1);
            n.set_hexpand(true);
            col.append(&n);
            let min = ((agora - quando) / 60).max(1);
            let ha = if min < 60 { trf("{n} min ago", &[("n", min.to_string())]) } else if min < 1440 { trf("{n} h ago", &[("n", (min / 60).to_string())]) } else { trf("{n} days ago", &[("n", (min / 1440).to_string())]) };
            col.append(&rotulo_esq(&app.as_ref().map(|a| format!("{} · {ha}", a.name())).unwrap_or(ha), "grade-sugestao-motivo"));
            linha.append(&col);
            b.set_child(Some(&linha));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(g) = w.upgrade() {
                    g.fechar();
                }
                let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&p).uri(), gdk::Display::default().map(|x| x.app_launch_context()).as_ref());
            });
            self.recentes.append(&b);
        }
    }

    fn limpar_agente(&self) {
        while let Some(c) = self.agente.first_child() {
            self.agente.remove(&c);
        }
    }

    /// The agent's zone before a request: what it can do, and examples to try.
    fn agente_ocioso(self: &Rc<Self>) {
        self.pedido.set(self.pedido.get() + 1);
        self.limpar_agente();
        let t = rotulo_esq(tr("Say what you want to do: the agent looks on this computer and in the Central de Apps."), "grade-agente-texto");
        t.set_wrap(true);
        self.agente.append(&t);
        for ex in [tr("something to edit photos"), tr("record the screen"), tr("read a PDF")] {
            let b = gtk::Button::with_label(&format!("“{ex}”"));
            b.add_css_class("grade-exemplo");
            b.set_halign(Align::Start);
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(g) = w.upgrade() {
                    g.entrada.set_text(ex);
                    g.entrada.set_position(-1);
                    g.perguntar(ex);
                }
            });
            self.agente.append(&b);
        }
    }

    /// The request, answered: the local model says what it means; this computer's apps, the catalog's offers and a
    /// recent file follow.
    fn perguntar(self: &Rc<Self>, q: &str) {
        let n = self.pedido.get() + 1;
        self.pedido.set(n);
        self.limpar_agente();
        let bolha = caixa(Orientation::Horizontal, 0, "grade-pedido");
        let t = rotulo_esq(&format!("“{q}”"), "grade-agente-texto");
        t.set_wrap(true);
        bolha.append(&t);
        self.agente.append(&bolha);
        let p = rotulo_esq(tr("looking…"), "grade-agente-nota");
        p.add_css_class("pulsando");
        self.agente.append(&p);
        if debug() {
            eprintln!("debug grade: o agente procura «{q}»");
        }
        let todos = self.todos();
        let q2 = q.to_string();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let sentido = std::process::Command::new(&bin).arg("apps-pedido").arg(&q2).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()
                .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
                .map(|v| {
                    let lista = |k: &str| v[k].as_array().into_iter().flatten().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>();
                    Sentido { palavras: lista("palavras").into_iter().map(|s| s.to_lowercase()).collect(), categorias: lista("categorias"), tipos: lista("tipos") }
                })
                // (no model: the request's own words)
                .unwrap_or_else(|| Sentido { palavras: q2.split_whitespace().filter(|w| w.chars().count() > 3).map(str::to_lowercase).collect(), ..Default::default() });
            let mut ofertas: Vec<Oferta> = Vec::new();
            for p in sentido.palavras.iter().take(2) {
                for o in catalogo(p) {
                    if !ofertas.iter().any(|x| x.id == o.id) {
                        ofertas.push(o);
                    }
                }
            }
            let recentes = crate::arquivos::seletor::recentes_com_tipo(20);
            (sentido, ofertas, recentes)
        }, move |(sentido, ofertas, recentes)| {
            let Some(g) = w.upgrade() else { return };
            if g.pedido.get() != n {
                return;
            }
            g.responder(&todos, &sentido, ofertas, recentes);
        });
    }

    fn responder(self: &Rc<Self>, todos: &[App], s: &Sentido, ofertas: Vec<Oferta>, recentes: Vec<(PathBuf, String, i64)>) {
        // the request's bubble stays; the rest is the answer
        if let Some(p) = self.agente.last_child() {
            self.agente.remove(&p);
        }
        // this computer: apps whose categories or words match
        let mut aqui: Vec<(u8, &App)> = todos.iter().filter_map(|a| {
            let cats = a.info.categories().map(|c| c.to_string()).unwrap_or_default();
            let texto = format!("{} {} {}", a.nome, a.info.keywords().iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" "),
                a.info.generic_name().map(|x| x.to_string()).unwrap_or_default()).to_lowercase();
            let por_palavra = s.palavras.iter().any(|p| texto.contains(p.as_str()));
            let por_categoria = s.categorias.iter().any(|c| cats.split(';').any(|x| x == c));
            match (por_palavra, por_categoria) {
                (true, true) => Some((0, a)),
                (true, false) => Some((1, a)),
                (false, true) => Some((2, a)),
                _ => None,
            }
        }).collect();
        aqui.sort_by_key(|(r, a)| (*r, a.nome.to_lowercase()));
        let todos_achados: Vec<App> = aqui.iter().map(|(_, a)| (*a).clone()).collect();
        let aqui: Vec<&App> = aqui.into_iter().map(|(_, a)| a).take(3).collect();
        let instalados: Vec<String> = todos.iter().map(|a| a.id.trim_end_matches(".desktop").to_lowercase()).collect();
        let ofertas: Vec<Oferta> = ofertas.into_iter().filter(|o| !instalados.contains(&o.id.to_lowercase()) && !todos.iter().any(|a| a.nome.eq_ignore_ascii_case(&o.nome))).take(2).collect();
        let arquivo = recentes.into_iter().find(|(_, t, _)| s.tipos.iter().any(|x| t.starts_with(x.as_str())));
        if debug() {
            eprintln!("debug grade: o agente achou {} aqui, {} no catálogo{}", aqui.len(), ofertas.len(), if arquivo.is_some() { ", 1 arquivo" } else { "" });
        }
        // the cards show what it found (when the field has no app by that name)
        if !todos_achados.is_empty() && apps::buscar(todos, self.entrada.text().trim()).is_empty() {
            *self.achados.borrow_mut() = Some(todos_achados);
            self.preencher(true);
        }
        if !aqui.is_empty() {
            self.agente.append(&rotulo_esq(tr("ON THIS COMPUTER"), "grade-agente-secao"));
            for (k, a) in aqui.iter().enumerate() {
                let (w, app) = (Rc::downgrade(self), (*a).clone());
                let sub = a.info.generic_name().map(|x| x.to_string()).or_else(|| a.info.description().map(|x| x.to_string())).unwrap_or_default();
                self.linha_agente(apps::imagem(a, 36).upcast(), &a.nome, &sub, tr("Open"), k == 0, move || if let Some(g) = w.upgrade() { g.abrir_app(&app) });
            }
        }
        if !ofertas.is_empty() {
            self.agente.append(&rotulo_esq(tr("IN THE CENTRAL DE APPS"), "grade-agente-secao"));
            for o in ofertas {
                let tile = caixa(Orientation::Horizontal, 0, "grade-agente-tile");
                tile.set_size_request(36, 36);
                tile.set_hexpand(false);
                tile.set_valign(Align::Center);
                let ic = ui_icon("download", "#B9BAC4", 16);
                ic.set_hexpand(true);
                ic.set_halign(Align::Center);
                tile.append(&ic);
                let w = Rc::downgrade(self);
                let (nome, pacote) = (o.nome.clone(), o.pacote.clone());
                self.linha_agente(tile.upcast(), &o.nome, &o.resumo, tr("Install"), false, move || {
                    let Some(g) = w.upgrade() else { return };
                    // the agent installs it (kalangod's package tool, with its approval)
                    let pedido = trf("Install {name} (package {package}).", &[("name", nome.clone()), ("package", pacote.clone())]);
                    let shell = g.shell.borrow().upgrade();
                    if let Some(s) = shell {
                        g.fechar();
                        s.right_bar().ask_agent(&pedido);
                    }
                });
            }
        }
        if let (Some((p, _, _)), Some(a)) = (arquivo, aqui.first()) {
            self.agente.append(&rotulo_esq(tr("WITH A RECENT FILE"), "grade-agente-secao"));
            let nome = p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let (w, info) = (Rc::downgrade(self), a.info.clone());
            let ic = ui_icon("imagem", "#B9BAC4", 16);
            self.linha_agente(ic.upcast(), &nome, &trf("open in {app}", &[("app", a.nome.clone())]), tr("Open"), false, move || {
                if let Some(g) = w.upgrade() {
                    g.fechar();
                }
                let _ = info.launch(&[gio::File::for_path(&p)], gdk::Display::default().map(|x| x.app_launch_context()).as_ref());
            });
        }
        if self.agente.first_child().and_then(|c| c.next_sibling()).is_none() {
            self.agente.append(&rotulo_esq(tr("Nothing here or in the catalog for that. Ctrl+Enter asks the agent in the side bar."), "grade-agente-nota"));
        }
        let nota = caixa(Orientation::Horizontal, 8, "");
        nota.set_margin_top(4);
        nota.append(&ui_icon("cpu", "#8E8F99", 14));
        nota.append(&rotulo_esq(tr("the local model chose · nothing leaves the computer"), "grade-agente-nota"));
        self.agente.append(&nota);
        if debug() {
            let mut c = self.agente.first_child();
            while let Some(x) = c {
                if let Some(b) = x.last_child().and_downcast::<gtk::Button>() {
                    let nome = x.widget_name();
                    if !nome.is_empty() {
                        crate::central::posicao_debug(&format!("grade agente {nome}"), b.upcast_ref());
                    }
                }
                c = x.next_sibling();
            }
        }
    }

    fn linha_agente(&self, icone: gtk::Widget, nome: &str, sub: &str, acao: &str, principal: bool, f: impl Fn() + 'static) {
        let r = caixa(Orientation::Horizontal, 10, "grade-agente-linha");
        r.set_widget_name(nome);
        icone.set_valign(Align::Center);
        r.append(&icone);
        let col = caixa(Orientation::Vertical, 1, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let n = rotulo_esq(nome, "grade-sugestao-nome");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        n.set_hexpand(true);
        col.append(&n);
        if !sub.is_empty() {
            let s = rotulo_esq(sub, "grade-sugestao-motivo");
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            s.set_max_width_chars(1);
            s.set_hexpand(true);
            col.append(&s);
        }
        r.append(&col);
        let b = gtk::Button::with_label(acao);
        b.add_css_class("grade-agente-botao");
        if principal {
            b.add_css_class("principal");
        }
        b.set_valign(Align::Center);
        b.connect_clicked(move |_| f());
        r.append(&b);
        self.agente.append(&r);
    }

    /// Sizes for this screen: the side zones' width, the icons (bigger on tall screens).
    fn medir(&self) {
        let (w, h) = crate::superficies::tamanho_da_saida().unwrap_or((1920, 1080));
        let lado = if w >= 2560 { 420 } else if w >= 1600 { 340 } else { 300 };
        self.zona_esquerda.set_size_request(lado, -1);
        self.zona_direita.set_size_request(lado, -1);
        // a narrower screen (below 1600): the apps and the agent (AGORA goes, the apps keep two columns)
        self.esquerda.set_visible(w >= 1600);
        let px = if h >= 1300 { 76 } else { 52 };
        self.icone_px.set(px);
        // the room for the cards: the screen less the margins, the side zones and their gaps
        let lados = if w >= 1600 { 2 * (lado + 24) } else { lado + 24 };
        let espaco = (w - 80 - lados).max(300);
        let colunas = if espaco >= 1000 { 3 } else if espaco >= 640 { 2 } else { 1 };
        let largura = (espaco - 16 * (colunas - 1)) / colunas;
        let tile = if px > 60 { 140 } else { 96 };
        let por_linha = ((largura - 24) / tile).max(1) as u32;
        self.colunas.set((colunas as u32, largura, por_linha));
        if debug() {
            eprintln!("debug grade: tela {w}x{h}, {colunas} colunas de {largura} px, {por_linha} apps por linha");
        }
    }

    pub fn abrir(self: &Rc<Self>) {
        self.aberta.set(true);
        self.achados.borrow_mut().take();
        self.medir();
        self.entrada.set_text("");
        self.render_sugeridos();
        self.render_recentes();
        self.agente_ocioso();
        self.preencher(true);
        self.janela.present();
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(g) = w.upgrade() {
                crate::superficies::take_keyboard(&g.janela);
                g.recortar();
            }
        });
        self.fundo.definir(Canal::Opacidade, 0.0);
        self.fundo.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.conteudo.entrar((0.0, 0.0), 0.96);
        self.esquerda.entrar((-16.0, 0.0), 1.0);
        self.direita.entrar((16.0, 0.0), 1.0);
        self.entrada.grab_focus();
    }

    /// The bar and the dock stay out of the scrim and keep their clicks (once the window has its size).
    fn recortar(self: &Rc<Self>) {
        let Some(s) = self.shell.borrow().upgrade() else { return };
        let (w, h) = (self.janela.width(), self.janela.height());
        if w == 0 {
            let g = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(30), move || {
                if let Some(g) = g.upgrade().filter(|g| g.aberta.get()) {
                    g.recortar();
                }
            });
            return;
        }
        *self.buracos.borrow_mut() = furos(&s, w, h);
        if let Some(c) = self.fundo.first_child() {
            c.queue_draw();
        }
        entrada_sem_furos(&self.janela, &self.buracos.borrow());
    }

    pub fn fechar(&self) {
        if !self.aberta.replace(false) {
            return;
        }
        self.pedido.set(self.pedido.get() + 1);
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        let j = self.janela.downgrade();
        self.conteudo.sair((0.0, 0.0), 0.96, move || {
            if let Some(j) = j.upgrade() {
                j.set_visible(false);
            }
        });
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.aberta.get() { self.fechar() } else { self.abrir() }
    }
}
