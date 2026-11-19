//! "Notificação de aprovação": an agent asks for permission and its conversation is not on screen (the side bar is
//! closed, the agents center is behind other windows). The island becomes "Agente pede aprovação  [Ver] [Permitir]"
//! and a card under it says what and where. "Ver detalhes" shows the conversation where the request is; "Permitir
//! uma vez" answers it exactly as the conversation's own button (once; "always" and "deny" stay in the conversation,
//! with the full arguments).

use crate::conversa::{ui_icon, Conversa};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

type Ilha = Box<dyn Fn(Option<String>)>;
type Abrir = Box<dyn Fn(&Rc<Conversa>)>;

pub struct AvisoPedido {
    janela: gtk::Window,
    titulo: gtk::Label,
    detalhe: gtk::Label,
    quando: gtk::Label,
    atual: RefCell<Option<(Weak<Conversa>, u64)>>,
    ilha: RefCell<Option<Ilha>>,
    abrir: RefCell<Option<Abrir>>,
    /// the island's "Permitir" (its place, for the session test)
    botao_ilha: RefCell<Option<gtk::Button>>,
}

impl AvisoPedido {
    pub fn new(app: &gtk::Application) -> Rc<AvisoPedido> {
        let janela = gtk::Window::new();
        janela.set_application(Some(app));
        janela.init_layer_shell();
        janela.set_layer(Layer::Top);
        janela.set_namespace(Some("kalango-pedido"));
        janela.set_anchor(Edge::Top, true);
        janela.set_margin(Edge::Top, crate::superficies::BARRA_ALTURA + 10);
        janela.add_css_class("pedido-aviso-janela");
        let c = caixa(Orientation::Vertical, 8, "pedido-aviso");
        c.set_size_request(380, -1);
        let h = caixa(Orientation::Horizontal, 8, "");
        h.append(&crate::apps::imagem_kalango("agentes", 14));
        h.append(&rotulo(tr("Agent"), "pedido-aviso-quem"));
        let quando = rotulo("", "central-agente-sub");
        quando.set_hexpand(true);
        quando.set_xalign(0.0);
        h.append(&quando);
        h.append(&ui_icon("escudo", "#E5C98C", 15));
        c.append(&h);
        let titulo = rotulo("", "pedido-aviso-titulo");
        titulo.set_xalign(0.0);
        titulo.set_wrap(true);
        c.append(&titulo);
        let detalhe = rotulo("", "central-agente-sub");
        detalhe.set_xalign(0.0);
        detalhe.set_wrap(true);
        detalhe.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        c.append(&detalhe);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_homogeneous(true);
        bs.set_margin_top(4);
        let ver = gtk::Button::with_label(tr("See details"));
        ver.add_css_class("central-botao");
        bs.append(&ver);
        let permitir = gtk::Button::with_label(tr("Allow once"));
        permitir.add_css_class("linha-desfazer");
        bs.append(&permitir);
        c.append(&bs);
        janela.set_child(Some(&c));
        let a = Rc::new(AvisoPedido { janela, titulo, detalhe, quando, atual: RefCell::default(), ilha: RefCell::default(), abrir: RefCell::default(), botao_ilha: RefCell::default() });
        let w = Rc::downgrade(&a);
        ver.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.ver();
            }
        });
        let w = Rc::downgrade(&a);
        permitir.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.permitir();
            }
        });
        a
    }

    /// Moves the card down (under the "agent is using" banner) or back.
    pub fn deslocar(&self, px: i32) {
        self.janela.set_margin(Edge::Top, crate::superficies::BARRA_ALTURA + 10 + px);
    }

    /// The bar: shows (Some(text)) or hides (None) the island's request state.
    pub fn set_ilha(&self, f: impl Fn(Option<String>) + 'static) {
        *self.ilha.borrow_mut() = Some(Box::new(f));
    }

    /// The shell: how to show a conversation (the side bar's, or the agents center's).
    pub fn set_abrir(&self, f: impl Fn(&Rc<Conversa>) + 'static) {
        *self.abrir.borrow_mut() = Some(Box::new(f));
    }

    pub fn mostrar(&self, conv: &Rc<Conversa>, id: u64, titulo: &str, detalhe: &str) {
        *self.atual.borrow_mut() = Some((Rc::downgrade(conv), id));
        self.titulo.set_text(&format!("{}?", titulo.trim_end_matches(['.', '?'])));
        self.detalhe.set_text(detalhe);
        self.detalhe.set_visible(!detalhe.is_empty());
        self.quando.set_text(&format!("· {}", tr("now")));
        self.janela.present();
        if let Some(f) = self.ilha.borrow().as_ref() {
            f(Some(trf("{who} asks for approval", &[("who", tr("Agent").to_string())])));
        }
        if let Some(b) = self.botao_ilha.borrow().as_ref() {
            crate::central::posicao_debug("pedido permitir", b.upcast_ref());
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug aviso pedido: mostrado {id}");
            let j = self.janela.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(b) = j.child() {
                    eprintln!("debug aviso pedido: cartão {}x{}", b.width(), b.height());
                }
            });
        }
    }

    /// The request was answered (here or in the conversation): the notice goes.
    pub fn respondido(&self, id: u64) {
        let atual = self.atual.borrow().as_ref().map(|(_, i)| *i);
        if atual == Some(id) {
            self.esconder();
        }
    }

    fn esconder(&self) {
        self.atual.borrow_mut().take();
        self.janela.set_visible(false);
        if let Some(f) = self.ilha.borrow().as_ref() {
            f(None);
        }
    }

    /// "Ver": the conversation with the request (the request stays there, waiting).
    pub fn ver(&self) {
        let conv = self.atual.borrow().as_ref().and_then(|(c, _)| c.upgrade());
        self.janela.set_visible(false);
        if let Some(f) = self.ilha.borrow().as_ref() {
            f(None);
        }
        if let (Some(c), Some(f)) = (conv, self.abrir.borrow().as_ref()) {
            f(&c);
        }
    }

    /// "Permitir (uma vez)": answers it as the conversation's own button.
    pub fn permitir(&self) {
        let atual = self.atual.borrow().as_ref().and_then(|(c, i)| c.upgrade().map(|c| (c, *i)));
        if let Some((c, id)) = atual {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso pedido: permitido {id}");
            }
            c.permitir_pedido(id);
        }
        self.esconder();
    }
}

/// The island's request state (a pill with its own buttons: buttons do not go inside the island button).
pub fn pilula_da_ilha(aviso: &Rc<AvisoPedido>) -> (gtk::Box, gtk::Label) {
    let p = caixa(Orientation::Horizontal, 8, "ilha");
    p.add_css_class("ilha-pedido");
    p.set_valign(Align::Center);
    p.append(&ui_icon("escudo", "#E5C98C", 14));
    let t = rotulo("", "ilha-pedido-texto");
    p.append(&t);
    let ver = gtk::Button::with_label(tr("See"));
    ver.add_css_class("ilha-pedido-botao");
    p.append(&ver);
    let permitir = gtk::Button::with_label(tr("Allow"));
    permitir.add_css_class("ilha-pedido-botao");
    permitir.add_css_class("principal");
    p.append(&permitir);
    let w = Rc::downgrade(aviso);
    ver.connect_clicked(move |_| {
        if let Some(a) = w.upgrade() {
            a.ver();
        }
    });
    let w = Rc::downgrade(aviso);
    permitir.connect_clicked(move |_| {
        if let Some(a) = w.upgrade() {
            a.permitir();
        }
    });
    *aviso.botao_ilha.borrow_mut() = Some(permitir);
    (p, t)
}
