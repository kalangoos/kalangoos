//! The frame every Settings section shares in the designs (Aparência, Rede, Bluetooth, Sistema…): the title and its
//! line, the search, the tabs with their underline, the body on the left, and the panel on the right (460 px) with
//! its own title, the button that hides it and the window's controls. A section fills the body and the panel.

use crate::configuracoes::{busca, controles_janela};
use crate::conversa::ui_icon_fitted;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

pub struct Moldura {
    pub raiz: gtk::Box,
    pub titulo: gtk::Label,
    pub sub: gtk::Label,
    /// the body, under the tabs' line (or under the title when there are no tabs)
    pub corpo: gtk::Box,
    /// the panel's content (scrolls when it is taller than the window)
    pub painel: gtk::Box,
    pub painel_titulo: gtk::Label,
    pub abas: Vec<(&'static str, gtk::Button, gtk::Box)>,
    painel_raiz: gtk::Box,
    busca: gtk::Box,
    busca_e: gtk::Entry,
    /// the panel's button and the window's controls in the top row, while the panel is hidden
    topo_extra: gtk::Box,
    ouvintes_aba: RefCell<Vec<Box<dyn Fn(&str)>>>,
    /// the panel as the person left it (shown or hidden), and whether a narrow window hid it for now
    escolha_painel: std::cell::Cell<bool>,
    recolhido: std::cell::Cell<bool>,
}

fn esquerda(t: &str, classe: &str) -> gtk::Label {
    let l = rotulo(t, classe);
    l.set_xalign(0.0);
    l
}

impl Moldura {
    /// `abas`: (id, name) of each tab (none: no tab row).
    pub fn new(titulo: &str, abas: &[(&'static str, &'static str)], painel_titulo: &str) -> Rc<Moldura> {
        let raiz = caixa(Orientation::Horizontal, 0, "");
        let principal = caixa(Orientation::Vertical, 18, "cfg-principal");
        principal.add_css_class("com-painel");
        principal.set_hexpand(true);
        let topo = caixa(Orientation::Horizontal, 16, "");
        let tt = caixa(Orientation::Vertical, 2, "");
        let titulo_l = esquerda(titulo, "cfg-titulo");
        // (a long title — Japanese, German — gives way before the search does: the window keeps its width)
        titulo_l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tt.append(&titulo_l);
        let sub = esquerda("", "cfg-sub");
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tt.append(&sub);
        topo.append(&tt);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        topo.append(&esp);
        let (bx, busca_e) = busca();
        bx.set_hexpand(false);
        topo.append(&bx);
        let topo_extra = caixa(Orientation::Horizontal, 8, "");
        topo_extra.set_visible(false);
        topo.append(&topo_extra);
        let th = gtk::WindowHandle::new();
        th.set_child(Some(&topo));
        principal.append(&th);
        let mut lista = Vec::new();
        if !abas.is_empty() {
            let linha_abas = caixa(Orientation::Horizontal, 22, "");
            for (id, nome) in abas {
                let b = gtk::Button::new();
                b.add_css_class("cfg-aba");
                let col = caixa(Orientation::Vertical, 6, "");
                col.append(&rotulo(tr(nome), ""));
                let sub = caixa(Orientation::Horizontal, 0, "cfg-aba-linha");
                sub.set_size_request(24, 2);
                sub.set_halign(Align::Center);
                col.append(&sub);
                b.set_child(Some(&col));
                b.set_widget_name(tr(nome));
                crate::central::posicao_debug(&format!("cfg aba {id}"), b.upcast_ref());
                linha_abas.append(&b);
                lista.push((*id, b, sub));
            }
            // (tabs longer than the window — Russian, German — scroll sideways instead of widening the window)
            let rolagem_abas = gtk::ScrolledWindow::new();
            rolagem_abas.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
            rolagem_abas.set_propagate_natural_width(true);
            rolagem_abas.set_child(Some(&linha_abas));
            principal.append(&rolagem_abas);
            let linha = caixa(Orientation::Horizontal, 0, "cfg-div");
            linha.set_size_request(-1, 1);
            principal.append(&linha);
        }
        let corpo = caixa(Orientation::Vertical, 16, "");
        corpo.set_vexpand(true);
        // (every page scrolls when it is taller than the window: a long list — forty virtual network interfaces —
        // went past the bottom; a page with a scroll of its own gets the whole visible height, as before)
        let corpo_rolagem = gtk::ScrolledWindow::new();
        corpo_rolagem.set_hscrollbar_policy(gtk::PolicyType::Never);
        corpo_rolagem.set_vexpand(true);
        corpo_rolagem.add_css_class("cfg-corpo-rolagem");
        corpo_rolagem.set_child(Some(&corpo));
        principal.append(&corpo_rolagem);
        raiz.append(&principal);
        // the panel
        let painel_raiz = caixa(Orientation::Vertical, 14, "cfg-painel");
        // 460 in the design, whose window border is drawn over its last pixel; its title expands within it, the
        // panel itself keeps its width (the body takes the rest)
        painel_raiz.set_size_request(459, -1);
        painel_raiz.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.set_size_request(-1, 38);
        let painel_titulo = esquerda(painel_titulo, "cfg-painel-titulo");
        painel_titulo.set_hexpand(true);
        cab.append(&painel_titulo);
        let alternar = gtk::Button::new();
        alternar.add_css_class("cfg-icone-botao");
        alternar.set_child(Some(&ui_icon_fitted("painel-direita", "#8E8F99", 16)));
        alternar.set_tooltip_text(Some(tr("Hide the panel")));
        alternar.set_valign(Align::Center);
        cab.append(&alternar);
        cab.append(&controles_janela());
        let ch = gtk::WindowHandle::new();
        ch.set_child(Some(&cab));
        painel_raiz.append(&ch);
        let painel = caixa(Orientation::Vertical, 14, "");
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_hscrollbar_policy(gtk::PolicyType::Never);
        rolagem.set_propagate_natural_height(true);
        rolagem.set_propagate_natural_width(true);
        rolagem.set_max_content_width(420);
        rolagem.set_min_content_width(420);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&painel));
        painel_raiz.append(&rolagem);
        raiz.append(&painel_raiz);
        // (measures the page: a narrow window — half the screen — hides the panel for now, the body keeps the room)
        let medidor = gtk::DrawingArea::new();
        medidor.set_can_target(false);
        medidor.set_content_height(1);
        medidor.set_hexpand(true);
        principal.prepend(&medidor);
        let m = Rc::new(Moldura { raiz, titulo: titulo_l, sub, corpo, painel, painel_titulo, abas: lista, painel_raiz, busca: bx, busca_e, topo_extra,
            ouvintes_aba: RefCell::default(), escolha_painel: std::cell::Cell::new(true), recolhido: std::cell::Cell::new(false) });
        let w = Rc::downgrade(&m);
        medidor.connect_resize(move |_, largura, _| {
            if let Some(m) = w.upgrade() {
                let painel = if m.painel_raiz.is_visible() { m.painel_raiz.width().max(459) } else { 0 };
                let total = largura + painel;
                let w2 = Rc::downgrade(&m);
                glib::idle_add_local_once(move || if let Some(m) = w2.upgrade() { m.caber(total) });
            }
        });
        let w = Rc::downgrade(&m);
        alternar.connect_clicked(move |_| if let Some(m) = w.upgrade() { m.mostrar_painel(false) });
        let mostrar = gtk::Button::new();
        mostrar.add_css_class("cfg-icone-botao");
        mostrar.set_child(Some(&ui_icon_fitted("painel-direita", "#8E8F99", 16)));
        mostrar.set_tooltip_text(Some(tr("Show the panel")));
        mostrar.set_valign(Align::Center);
        let w = Rc::downgrade(&m);
        mostrar.connect_clicked(move |_| if let Some(m) = w.upgrade() { m.mostrar_painel(true) });
        m.topo_extra.append(&mostrar);
        m.topo_extra.append(&controles_janela());
        for (id, b, _) in &m.abas {
            let (w, id) = (Rc::downgrade(&m), *id);
            b.connect_clicked(move |_| if let Some(m) = w.upgrade() { m.aba(id) });
        }
        if let Some((id, _, _)) = m.abas.first() {
            let id = *id;
            m.marcar_aba(id);
        }
        m
    }

    /// The panel hides (the body takes the room); its button and the window's controls go to the top row. (The
    /// person's choice: kept when the window is wide again.)
    pub fn mostrar_painel(&self, on: bool) {
        self.escolha_painel.set(on);
        self.recolhido.set(false);
        self.painel_raiz.set_visible(on);
        self.topo_extra.set_visible(!on);
    }

    /// A page narrower than 1000 px (body and panel — the body would get less than about 540 px): the panel waits
    /// behind its button; wider, as the person left it. (The small screen's design, 1280 px, still has the panel.)
    fn caber(&self, total: i32) {
        let estreita = total < 1000;
        if estreita && self.painel_raiz.is_visible() {
            self.recolhido.set(true);
            self.painel_raiz.set_visible(false);
            self.topo_extra.set_visible(true);
        } else if !estreita && self.recolhido.get() {
            self.recolhido.set(false);
            self.painel_raiz.set_visible(self.escolha_painel.get());
            self.topo_extra.set_visible(!self.escolha_painel.get());
        }
    }

    /// The design's taller search on some pages (Bluetooth): 44 px, 14 px text.
    pub fn busca_grande(&self) {
        self.busca.add_css_class("grande");
        self.busca.set_size_request(460, 44);
    }

    /// On a small screen the search is the compact design's (280 px).
    pub fn compactar(&self, c: bool) {
        let alto = if self.busca.has_css_class("grande") { 44 } else { 32 };
        self.busca.set_size_request(if c { 280 } else { 460 }, alto);
        self.busca_e.set_placeholder_text(Some(if c { tr("Search or ask…") } else { tr("Search or ask: “make the screen warmer at night”") }));
    }

    fn marcar_aba(&self, id: &str) {
        for (i, b, linha) in &self.abas {
            if *i == id {
                b.add_css_class("ativa");
                linha.add_css_class("ativa");
            } else {
                b.remove_css_class("ativa");
                linha.remove_css_class("ativa");
            }
        }
    }

    /// Shows a tab (its section fills the body and the panel for it).
    pub fn aba(&self, id: &str) {
        self.marcar_aba(id);
        self.mostrar_painel(true);
        let ouvintes = self.ouvintes_aba.take();
        for f in &ouvintes {
            f(id);
        }
        let mut o = self.ouvintes_aba.borrow_mut();
        let novos = std::mem::take(&mut *o);
        *o = ouvintes;
        o.extend(novos);
        if crate::configuracoes::depurando() {
            eprintln!("debug configuracoes: {} › {id}", self.titulo.text());
        }
    }

    pub fn ao_trocar_aba(&self, f: impl Fn(&str) + 'static) {
        self.ouvintes_aba.borrow_mut().push(Box::new(f));
    }

    /// A tab's button (for the session test's positions).
    pub fn botao_aba(&self, id: &str) -> Option<gtk::Button> {
        self.abas.iter().find(|(i, _, _)| *i == id).map(|(_, b, _)| b.clone())
    }
}

/// A panel's line: the name on the left, the value on the right (the designs' "Estado … Conectado há 3 h").
pub fn linha_valor(nome: &str) -> (gtk::Box, gtk::Label) {
    let r = caixa(Orientation::Horizontal, 8, "");
    let n = esquerda(nome, "cfg-sub");
    n.set_hexpand(true);
    r.append(&n);
    let v = rotulo("", "cfg-valor-pequeno");
    v.set_xalign(1.0);
    v.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    r.append(&v);
    (r, v)
}

/// A panel's switch line: the text and the switch at the right.
pub fn linha_interruptor(texto: &str) -> (gtk::Box, gtk::Switch) {
    let r = caixa(Orientation::Horizontal, 8, "");
    let t = esquerda(texto, "cfg-texto");
    t.set_hexpand(true);
    t.set_wrap(true);
    t.set_valign(Align::Center);
    r.append(&t);
    let s = crate::configuracoes::interruptor();
    r.append(&s);
    (r, s)
}
