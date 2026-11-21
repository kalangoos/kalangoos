//! Agents center › Ajustes › Chaves › "Aceitar da rede de casa" (design "Ajustes · Chaves e cofre", dark and light):
//! other agents of the home network (A2A) calling the person's agents through kalango-fluxos' listener. Off by
//! default. The key is made by the service (`a2a.new_key`) and shown once, with a copy button, never again; "Trocar
//! chave" makes a new one after a confirmation (callers with the old one stop working). The person ticks which agents
//! are offered (none: nothing is callable) and copies each one's address. Only callers of the home network (private
//! ranges) are accepted; the service refuses the rest.

use crate::conversa::ui_icon;
use crate::fluxos_estados::action_button;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::net::{IpAddr, Ipv4Addr};
use std::rc::Rc;

// ---------------------------------------------------------------- pure pieces (tested below)

/// The vault name of the listener's key (kalango-fluxos a2a_listener.rs `KEY`).
pub(crate) const KEY_NAME: &str = "a2a.chave";

/// The listener's port when none is set (kalango-fluxos a2a_listener.rs `DEFAULT_PORT`).
pub(crate) const DEFAULT_PORT: u16 = 8790;

/// Whether an address is one the listener accepts: the home network (private, loopback, link-local).
pub(crate) fn home_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local(),
        IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// A port typed by the person: 1024 to 65535.
pub(crate) fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1024)
}

/// The address another agent of the home network reads an agent's card from.
pub(crate) fn agent_card_url(ip: &str, port: u16, id: &str) -> String {
    format!("http://{ip}:{port}/agentes/{id}/.well-known/agent-card.json")
}

/// The service's refusals in the person's words.
pub(crate) fn error_text(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("make the key first") {
        tr("Make the key first: “Copy key once”, then turn it on.").to_string()
    } else if l.contains("port goes from") {
        tr("The port goes from 1024 to 65535.").to_string()
    } else if l.contains("address already in use") || l.contains("in use") {
        tr("Another program already uses that port. Choose another one.").to_string()
    } else if crate::fluxos_estados::is_offline_error(e) {
        tr("The flows service is not answering. Try again when it is back.").to_string()
    } else {
        trf("Not changed: {why}", &[("why", e.to_string())])
    }
}

/// This computer's address on the home network (the interface a packet to a private address would leave by; nothing
/// is sent). None: no home network address.
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(10, 255, 255, 255), 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (home_network(ip) && !ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

// ---------------------------------------------------------------- the block

#[derive(Default)]
struct State {
    enabled: bool,
    port: u16,
    offered: Vec<String>,
    has_key: bool,
    listening: Option<u16>,
    /// (id, name) of every agent
    agents: Vec<(String, String)>,
}

pub struct HomeNetwork {
    pub body: gtk::Box,
    switch: gtk::Switch,
    sub: gtk::Label,
    detail: gtk::Revealer,
    port: gtk::Entry,
    agents: gtk::Box,
    urls: gtk::Box,
    error: gtk::Label,
    copy_once: gtk::Button,
    replace: gtk::Button,
    confirm: gtk::Revealer,
    key_box: gtk::Revealer,
    key_label: gtk::Label,
    state: RefCell<State>,
    filling: Cell<bool>,
    /// a refusal to show once the state is read again (the service may keep part of a change it refused)
    pending_error: RefCell<Option<String>>,
}

impl HomeNetwork {
    pub fn new() -> Rc<HomeNetwork> {
        let body = caixa(Orientation::Vertical, 8, "");
        // the switch
        let ll = caixa(Orientation::Horizontal, 12, "");
        ll.set_margin_top(4);
        let lc = caixa(Orientation::Vertical, 2, "");
        lc.set_hexpand(true);
        let lt = rotulo(tr("Accept from the home network"), "fx-opcao-titulo");
        lt.set_xalign(0.0);
        lc.append(&lt);
        let sub = rotulo("", "central-agente-sub");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(1);
        lc.append(&sub);
        ll.append(&lc);
        let switch = gtk::Switch::new();
        switch.add_css_class("central-switch");
        switch.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes rede de casa", switch.upcast_ref());
        ll.append(&switch);
        body.append(&ll);

        // port, agents offered, their addresses (shown while on)
        let detail = gtk::Revealer::new();
        detail.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        detail.set_transition_duration(140);
        let dc = caixa(Orientation::Vertical, 8, "");
        dc.set_margin_top(6);
        let pr = caixa(Orientation::Horizontal, 12, "");
        let pl = rotulo(tr("Port"), "central-agente-sub");
        pl.set_hexpand(true);
        pl.set_xalign(0.0);
        pr.append(&pl);
        let port = gtk::Entry::new();
        port.add_css_class("fx-campo");
        port.add_css_class("mono");
        port.set_width_chars(6);
        port.set_max_width_chars(6);
        port.set_input_purpose(gtk::InputPurpose::Digits);
        crate::central::posicao_debug_sempre("ajustes rede de casa porta", port.upcast_ref());
        pr.append(&port);
        dc.append(&pr);
        let al = rotulo(tr("Agents offered (none ticked: nothing can be called)"), "central-agente-sub");
        al.set_xalign(0.0);
        al.set_wrap(true);
        al.set_max_width_chars(1);
        dc.append(&al);
        let agents = caixa(Orientation::Vertical, 0, "");
        dc.append(&agents);
        let urls = caixa(Orientation::Vertical, 4, "");
        dc.append(&urls);
        detail.set_child(Some(&dc));
        body.append(&detail);

        let error = rotulo("", "rotinas-erro");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_max_width_chars(1);
        error.set_visible(false);
        body.append(&error);

        // the key shown once
        let key_box = gtk::Revealer::new();
        key_box.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        key_box.set_transition_duration(140);
        let kb = caixa(Orientation::Vertical, 6, "fx-confirmar");
        let kr = caixa(Orientation::Horizontal, 8, "");
        let key_label = rotulo("", "fx-mono");
        key_label.set_xalign(0.0);
        key_label.set_hexpand(true);
        key_label.set_selectable(true);
        key_label.set_wrap(true);
        key_label.set_wrap_mode(gtk::pango::WrapMode::Char);
        key_label.set_max_width_chars(1);
        kr.append(&key_label);
        let copy_key = action_button(tr("Copy"), "copiar", "");
        copy_key.add_css_class("pequeno");
        copy_key.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes rede de casa copiar chave", copy_key.upcast_ref());
        kr.append(&copy_key);
        kb.append(&kr);
        let kn = rotulo(tr("Keep it now: it is not shown again. The other agent sends it with every message."), "central-agente-sub");
        kn.set_xalign(0.0);
        kn.set_wrap(true);
        kn.set_max_width_chars(1);
        kb.append(&kn);
        key_box.set_child(Some(&kb));
        body.append(&key_box);

        // "Trocar chave" asks first
        let confirm = gtk::Revealer::new();
        confirm.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        confirm.set_transition_duration(140);
        let cb = caixa(Orientation::Horizontal, 8, "fx-confirmar");
        let ct = rotulo(tr("Replace the key? Agents that call with the old one stop working until they get the new one."), "fx-tabela-texto");
        ct.set_xalign(0.0);
        ct.set_wrap(true);
        ct.set_max_width_chars(1);
        ct.set_hexpand(true);
        ct.set_valign(Align::Center);
        cb.append(&ct);
        let no = action_button(tr("Cancel"), "", "");
        no.set_valign(Align::Center);
        let yes = action_button(tr("Replace"), "", "perigo");
        yes.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes rede de casa trocar confirmar", yes.upcast_ref());
        cb.append(&no);
        cb.append(&yes);
        confirm.set_child(Some(&cb));
        body.append(&confirm);

        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_homogeneous(true);
        let copy_once = action_button(tr("Copy key once"), "compartilhar", "");
        copy_once.add_css_class("alto");
        crate::central::posicao_debug_sempre("ajustes rede de casa copiar uma vez", copy_once.upcast_ref());
        let replace = action_button(tr("Replace key"), "", "perigo");
        replace.add_css_class("alto");
        crate::central::posicao_debug_sempre("ajustes rede de casa trocar", replace.upcast_ref());
        bs.append(&copy_once);
        bs.append(&replace);
        body.append(&bs);

        let t = Rc::new(HomeNetwork {
            body, switch, sub, detail, port, agents, urls, error, copy_once, replace, confirm, key_box, key_label,
            state: RefCell::default(), filling: Cell::new(false), pending_error: RefCell::default(),
        });
        let w = Rc::downgrade(&t);
        t.switch.connect_active_notify(move |s| {
            if let Some(t) = w.upgrade().filter(|t| !t.filling.get()) {
                t.set(json!({"enabled": s.is_active()}));
            }
        });
        let w = Rc::downgrade(&t);
        let save_port = move || {
            let Some(t) = w.upgrade() else { return };
            let text = t.port.text();
            match parse_port(&text) {
                Some(p) if p != t.state.borrow().port => t.set(json!({"port": p})),
                Some(_) => {}
                None => t.fail(tr("The port goes from 1024 to 65535.")),
            }
        };
        let sp = Rc::new(save_port);
        let s1 = sp.clone();
        t.port.connect_activate(move |_| s1());
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| sp());
        t.port.add_controller(focus);
        let w = Rc::downgrade(&t);
        t.copy_once.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.new_key();
            }
        });
        let w = Rc::downgrade(&t);
        t.replace.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.confirm.set_reveal_child(!t.confirm.reveals_child());
            }
        });
        let c = t.confirm.clone();
        no.connect_clicked(move |_| c.set_reveal_child(false));
        let w = Rc::downgrade(&t);
        yes.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.confirm.set_reveal_child(false);
                t.new_key();
            }
        });
        let kl = t.key_label.clone();
        copy_key.connect_clicked(move |b| {
            b.clipboard().set_text(&kl.text());
        });
        t
    }

    /// Reads the listener and the agents again. The key shown once goes away.
    pub fn reload(self: &Rc<Self>) {
        self.forget_key();
        let w = Rc::downgrade(self);
        crate::fluxos_estados::call_all(vec![("a2a.listener", json!({})), ("agents.list", json!({}))], move |r| {
            let Some(t) = w.upgrade() else { return };
            if let Some(Err(e)) = r.iter().find(|x| x.is_err()) {
                t.fail(&error_text(e));
                return;
            }
            let v: Vec<Value> = r.into_iter().map(|x| x.unwrap_or_default()).collect();
            {
                let mut s = t.state.borrow_mut();
                s.enabled = v[0]["enabled"].as_bool().unwrap_or(false);
                s.port = v[0]["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(DEFAULT_PORT);
                s.offered = v[0]["agents"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
                s.has_key = v[0]["has_key"].as_bool().unwrap_or(false);
                s.listening = v[0]["listening"].as_u64().and_then(|p| u16::try_from(p).ok());
                s.agents = v[1]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| Some((a["agent"]["id"].as_str()?.to_string(), a["agent"]["name"].as_str().unwrap_or("").to_string())))
                    .collect();
            }
            t.error.set_visible(false);
            t.render();
            let pendente = t.pending_error.borrow_mut().take();
            if let Some(e) = pendente {
                t.fail(&e);
            }
        });
    }

    fn fail(&self, why: &str) {
        self.error.set_text(why);
        self.error.set_visible(true);
    }

    fn forget_key(&self) {
        self.key_label.set_text("");
        self.key_box.set_reveal_child(false);
    }

    /// Changes the listener; the service answers with where it listens now.
    fn set(self: &Rc<Self>, change: Value) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("a2a.set_listener", change, move |r| {
            let Some(t) = w.upgrade() else { return };
            match r {
                Ok(_) => {
                    let shown = t.key_label.text().to_string();
                    t.reload();
                    // (a change of port or agents does not hide a key the person has not copied yet)
                    if !shown.is_empty() {
                        t.show_key(&shown);
                    }
                }
                Err(e) => {
                    *t.pending_error.borrow_mut() = Some(error_text(&e));
                    let shown = t.key_label.text().to_string();
                    t.reload();
                    if !shown.is_empty() {
                        t.show_key(&shown);
                    }
                }
            }
        });
    }

    fn show_key(&self, key: &str) {
        self.key_label.set_text(key);
        self.key_box.set_reveal_child(true);
    }

    /// A new key from the service, shown this once.
    fn new_key(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("a2a.new_key", json!({}), move |r| {
            let Some(t) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let k = v["key"].as_str().unwrap_or("").to_string();
                    t.state.borrow_mut().has_key = true;
                    t.error.set_visible(false);
                    t.render();
                    t.show_key(&k);
                }
                Err(e) => t.fail(&error_text(&e)),
            }
        });
    }

    fn render(self: &Rc<Self>) {
        self.filling.set(true);
        let s = self.state.borrow();
        self.switch.set_active(s.enabled);
        let ip = lan_ip();
        let ip_text = ip.map(|i| i.to_string()).unwrap_or_else(|| tr("this-computer").to_string());
        self.sub.set_text(&match (s.enabled, s.listening) {
            (false, _) => trf("A2A on {ip}:{port} → needs the key. Only the home network (private addresses) is accepted.", &[("ip", ip_text.clone()), ("port", s.port.to_string())]),
            (true, Some(p)) => trf("Listening on {ip}:{port} · only the home network (private addresses)", &[("ip", ip_text.clone()), ("port", p.to_string())]),
            (true, None) => tr("On, but not listening: see the message below.").to_string(),
        });
        self.detail.set_reveal_child(s.enabled);
        if self.port.text().as_str() != s.port.to_string() && !self.port.has_focus() {
            self.port.set_text(&s.port.to_string());
        }
        // the agents offered, as chips that toggle
        while let Some(c) = self.agents.first_child() {
            self.agents.remove(&c);
        }
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(6);
        flow.set_row_spacing(6);
        flow.set_max_children_per_line(8);
        for (id, name) in &s.agents {
            let b = gtk::ToggleButton::with_label(if name.is_empty() { id } else { name });
            b.add_css_class("fx-quem");
            b.set_halign(Align::Start);
            b.set_active(s.offered.contains(id));
            crate::central::posicao_debug_sempre(&format!("ajustes rede de casa agente {id}"), b.upcast_ref());
            let (w, id) = (Rc::downgrade(self), id.clone());
            b.connect_toggled(move |b| {
                let Some(t) = w.upgrade().filter(|t| !t.filling.get()) else { return };
                let mut offered = t.state.borrow().offered.clone();
                offered.retain(|x| x != &id);
                if b.is_active() {
                    offered.push(id.clone());
                }
                t.set(json!({"agents": offered}));
            });
            let fc = gtk::FlowBoxChild::new();
            fc.set_child(Some(&b));
            fc.set_focusable(false);
            flow.insert(&fc, -1);
        }
        self.agents.append(&flow);
        // each offered agent's address, with a copy button
        while let Some(c) = self.urls.first_child() {
            self.urls.remove(&c);
        }
        for id in &s.offered {
            let url = agent_card_url(&ip_text, s.port, id);
            let r = caixa(Orientation::Horizontal, 6, "");
            let l = rotulo(&url, "fx-mono");
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_max_width_chars(1);
            l.set_selectable(true);
            l.set_tooltip_text(Some(&url));
            r.append(&l);
            let c = gtk::Button::new();
            c.add_css_class("fx-kebab");
            c.set_child(Some(&ui_icon("copiar", "#8E8F99", 14)));
            c.set_tooltip_text(Some(tr("Copy the address")));
            crate::central::posicao_debug_sempre(&format!("ajustes rede de casa copiar endereco {id}"), c.upcast_ref());
            c.connect_clicked(move |b| b.clipboard().set_text(&url));
            r.append(&c);
            self.urls.append(&r);
        }
        // the key: made once, then only replaced
        self.copy_once.set_sensitive(!s.has_key);
        self.copy_once.set_tooltip_text(Some(if s.has_key { tr("The key was already shown once. Replace it to see a new one.") } else { tr("Makes the key and shows it this once") }));
        self.replace.set_sensitive(s.has_key);
        drop(s);
        self.filling.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_home_network_and_valid_ports() {
        assert!(home_network("192.168.1.20".parse().unwrap()));
        assert!(home_network("10.0.0.5".parse().unwrap()));
        assert!(home_network("172.16.3.1".parse().unwrap()));
        assert!(home_network("fe80::1".parse().unwrap()));
        assert!(!home_network("8.8.8.8".parse().unwrap()));
        assert!(!home_network("2001:db8::1".parse().unwrap()));
        assert_eq!(parse_port("8790"), Some(8790));
        assert_eq!(parse_port(" 1024 "), Some(1024));
        assert_eq!(parse_port("80"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("x"), None);
        assert_eq!(agent_card_url("192.168.1.20", 8790, "planejador"), "http://192.168.1.20:8790/agentes/planejador/.well-known/agent-card.json");
    }

    #[test]
    fn the_service_refusals_read_in_plain_words() {
        assert_eq!(error_text("make the key first (a2a.new_key)"), tr("Make the key first: “Copy key once”, then turn it on."));
        assert_eq!(error_text("the port goes from 1024 to 65535"), tr("The port goes from 1024 to 65535."));
        assert_eq!(error_text("Address already in use (os error 98)"), tr("Another program already uses that port. Choose another one."));
    }
}
