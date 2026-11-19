//! "Agente usando a tela" (design 9534): while the agent works inside one of the system's apps (Files organising a
//! folder, renaming, moving), the person sees it everywhere — a banner under the top bar ("Agente está usando
//! Arquivos · 42/118 arquivos  [Parar (Esc)]"), the island turns into the task's progress, and the app itself
//! outlines its window and shows the agent's cursor on what it touches. Stopping (the banner's button, or Esc in the
//! app) stops the agent's turn.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::Orientation;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

/// (who, done, total) for the island; None: back to the clock.
type Ilha = Box<dyn Fn(Option<(String, usize, usize)>)>;
type Parar = Box<dyn Fn()>;
type Mudou = Box<dyn Fn(bool)>;

pub struct AgenteUsando {
    janela: gtk::Window,
    titulo: gtk::Label,
    progresso: gtk::Label,
    parar: RefCell<Option<Parar>>,
    ilha: RefCell<Option<Ilha>>,
    /// others that move out of its way (the approval card goes under it)
    ao_mudar: RefCell<Vec<Mudou>>,
    botao: RefCell<Option<gtk::Button>>,
    /// what it said last: (who, done, total) — the expanded island shows it
    ultimo: RefCell<Option<(String, usize, usize)>>,
}

/// Its height on screen, with the gap under it (the approval card moves down by this much).
pub const ALTURA: i32 = 39 + 8;

impl AgenteUsando {
    pub fn new(app: &gtk::Application) -> Rc<AgenteUsando> {
        let janela = gtk::Window::new();
        janela.set_application(Some(app));
        janela.init_layer_shell();
        janela.set_layer(Layer::Top);
        janela.set_namespace(Some("kalango-agente-usando"));
        janela.set_anchor(Edge::Top, true);
        janela.set_margin(Edge::Top, crate::superficies::BARRA_ALTURA + 10);
        janela.add_css_class("pedido-aviso-janela");
        // (gaps after the spark: 1 px less than the design's, its image has 1 px more for the outline)
        let c = caixa(Orientation::Horizontal, 9, "usando-faixa");
        c.append(&estrela(13, "#A5A1E2"));
        let titulo = rotulo("", "usando-titulo");
        c.append(&titulo);
        let progresso = rotulo("", "usando-progresso");
        c.append(&progresso);
        let parar = gtk::Button::new();
        parar.add_css_class("usando-parar");
        let pc = caixa(Orientation::Horizontal, 6, "");
        pc.append(&ui_icon("parar", "#1C1D24", 8));
        pc.append(&rotulo(tr("Stop (Esc)"), ""));
        parar.set_child(Some(&pc));
        c.append(&parar);
        janela.set_child(Some(&c));
        let a = Rc::new(AgenteUsando { janela, titulo, progresso, parar: RefCell::default(), ilha: RefCell::default(), ao_mudar: RefCell::default(), botao: RefCell::default(), ultimo: RefCell::default() });
        let w = Rc::downgrade(&a);
        parar.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.parar_agora();
            }
        });
        *a.botao.borrow_mut() = Some(parar);
        a
    }

    pub fn set_ilha(&self, f: impl Fn(Option<(String, usize, usize)>) + 'static) {
        *self.ilha.borrow_mut() = Some(Box::new(f));
    }

    pub fn ao_mudar(&self, f: impl Fn(bool) + 'static) {
        self.ao_mudar.borrow_mut().push(Box::new(f));
    }

    pub fn visivel(&self) -> bool {
        self.janela.is_visible()
    }

    /// The agent's progress on its task, while it shows: (who, done, total).
    pub fn progresso(&self) -> Option<(String, usize, usize)> {
        self.ultimo.borrow().clone().filter(|_| self.visivel())
    }

    /// The agent is using `app`: `feitos` of `total` items so far; `parar` stops it.
    pub fn mostrar(&self, quem: &str, app: &str, feitos: usize, total: usize, unidade: &str, parar: impl Fn() + 'static) {
        self.titulo.set_text(&trf("{who} is using {app}", &[("who", quem.to_string()), ("app", app.to_string())]));
        self.progresso.set_text(&format!("· {feitos}/{total} {unidade}"));
        *self.parar.borrow_mut() = Some(Box::new(parar));
        *self.ultimo.borrow_mut() = Some((quem.to_string(), feitos, total));
        let estava = self.janela.is_visible();
        self.janela.present();
        if let Some(f) = self.ilha.borrow().as_ref() {
            f(Some((quem.to_string(), feitos, total)));
        }
        if !estava {
            for f in self.ao_mudar.borrow().iter() {
                f(true);
            }
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug usando: {app} ({feitos}/{total})");
                // the stop button on screen (the banner is centred under the top bar)
                let (j, b) = (self.janela.clone(), self.botao.borrow().clone());
                gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                    let (Some(b), Some(m)) = (b, gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>()) else { return };
                    if let Some(r) = b.compute_bounds(&j) {
                        let x0 = (m.geometry().width() - j.width()) as f32 / 2.0;
                        // (its margin counts from below the top bar, which keeps its own height)
                        let y0 = (2 * crate::superficies::BARRA_ALTURA + 10) as f32;
                        eprintln!("debug usando: parar na tela {:.0} {:.0}", x0 + r.x() + r.width() / 2.0, y0 + r.y() + r.height() / 2.0);
                    }
                });
            }
        }
    }

    pub fn esconder(&self) {
        if !self.janela.is_visible() {
            return;
        }
        self.janela.set_visible(false);
        self.parar.borrow_mut().take();
        if let Some(f) = self.ilha.borrow().as_ref() {
            f(None);
        }
        for f in self.ao_mudar.borrow().iter() {
            f(false);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug usando: terminou");
        }
    }

    pub fn parar_agora(&self) {
        let f = self.parar.borrow_mut().take();
        if let Some(f) = f {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug usando: parado pela pessoa");
            }
            f();
        }
        self.esconder();
    }
}

/// The island while the agent works on a task with a known size: "✦ Agente ▬▬▬ 42/118".
/// Sets the island's task: who, done, total.
pub type DefinirTarefa = Box<dyn Fn(&str, usize, usize)>;

pub fn ilha_da_tarefa() -> (gtk::Box, DefinirTarefa) {
    let p = caixa(Orientation::Horizontal, 7, "ilha");
    p.add_css_class("ilha-tarefa");
    p.set_valign(gtk::Align::Center);
    p.append(&estrela(13, "#A5A1E2"));
    let quem = rotulo("", "ilha-tarefa-nome");
    p.append(&quem);
    let barra = gtk::LevelBar::for_interval(0.0, 1.0);
    barra.add_css_class("ilha-tarefa-barra");
    barra.set_size_request(80, 4);
    barra.set_valign(gtk::Align::Center);
    p.append(&barra);
    let conta = rotulo("", "ilha-tarefa-conta");
    p.append(&conta);
    let div = caixa(Orientation::Vertical, 0, "ilha-tarefa-div");
    div.set_size_request(1, 14);
    div.set_valign(gtk::Align::Center);
    p.append(&div);
    let hora = rotulo("", "ilha-tarefa-hora");
    p.append(&hora);
    let definir = move |q: &str, feitos: usize, total: usize| {
        quem.set_text(q);
        barra.set_value(if total == 0 { 0.0 } else { (feitos as f64 / total as f64).min(1.0) });
        conta.set_text(&format!("{feitos}/{total}"));
        hora.set_text(&gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default());
    };
    (p, Box::new(definir))
}
