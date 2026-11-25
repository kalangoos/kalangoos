//! The internet trigger of a flow, in the flow editor (design "Gatilho pela internet · inspetor do editor de fluxos" and
//! "folha completa"): the address and its copy button, the signing key (shown once, when it is made or replaced), what
//! it accepts, how it answers, who may call, the level and the public (unsigned) switch; in the wide sheet also the
//! signed example in curl, Python and JavaScript and how a call is checked.
//!
//! Everything is read from and written to the service (`internet.trigger_*`): each change is one call and the form is
//! drawn again from what the service answers, so what is shown is what the service holds (a public trigger comes back
//! at the lowest level and 5 calls a minute whatever was asked). Raising the level goes through kalangod's approval
//! (`gatilho_internet.editar`), never through the editor alone. Nothing here holds a key except the one just made,
//! until the person leaves it.

use crate::conversa::ui_icon;
use crate::internet_modelo as modelo;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::tr;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    /// The key just made or replaced, until the person leaves the screen (shown once, never read back).
    static FRESH_KEY: RefCell<HashMap<String, String>> = RefCell::default();
}

const TYPES: [(&str, &str); 3] = [("application/json", "application/json"), ("application/x-www-form-urlencoded", "form"), ("text/plain", "text/plain")];
const METHODS: [&str; 3] = ["POST", "PUT", "GET"];

pub struct InternetForm {
    pub root: gtk::Box,
    id: String,
    wide: bool,
    desc: RefCell<Value>,
    lang: RefCell<String>,
    error: gtk::Label,
    /// the latest calls (for the “limite de chamadas” card)
    calls: RefCell<Vec<modelo::Call>>,
}

/// How long what was copied (a key, a command with it) stays on the clipboard.
const CLIPBOARD_SECONDS: u32 = 30;

/// Copies, and takes it off the clipboard after a while (only if it is still what was copied: not something the person
/// copied since).
fn copy(text: &str) {
    if let Some(d) = gtk::gdk::Display::default() {
        d.clipboard().set_text(text);
        let (text, display) = (text.to_string(), d);
        gtk::glib::timeout_add_seconds_local_once(CLIPBOARD_SECONDS, move || {
            let clipboard = display.clipboard();
            let c2 = clipboard.clone();
            clipboard.read_text_async(gtk::gio::Cancellable::NONE, move |r| {
                if matches!(r, Ok(Some(ref now)) if now.as_str() == text) {
                    c2.set_text("");
                }
            });
        });
    }
}

fn section(t: &str) -> gtk::Label {
    let l = rotulo(&t.to_uppercase(), "fluxo-secao-cab");
    l.set_xalign(0.0);
    l.set_margin_top(14);
    l
}

fn note(t: &str) -> gtk::Label {
    let l = rotulo(t, "fluxo-chave-sub");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

/// A row of exclusive choices (the segmented control of the design).
fn segmented(options: &[(String, String)], current: &str, on_pick: impl Fn(&str) + 'static) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 0, "int-seg");
    b.set_halign(Align::Start);
    let on_pick = Rc::new(on_pick);
    for (value, label) in options {
        let t = gtk::ToggleButton::with_label(label);
        t.add_css_class("int-seg-item");
        t.set_active(value == current);
        let (v, f) = (value.clone(), on_pick.clone());
        t.connect_clicked(move |_| f(&v));
        b.append(&t);
    }
    b
}

/// A chip that is on or off.
fn chip(label: &str, on: bool, on_toggle: impl Fn(bool) + 'static) -> gtk::ToggleButton {
    let t = gtk::ToggleButton::with_label(&if on { format!("✓ {label}") } else { label.to_string() });
    t.add_css_class("int-chip");
    t.set_active(on);
    t.connect_clicked(move |b| on_toggle(b.is_active()));
    t
}

fn entry_box(text: &str, placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(text);
    e.set_placeholder_text(Some(placeholder));
    e.add_css_class("int-campo");
    e.set_hexpand(true);
    e
}

/// A kalangod tool that may wait for the person's approval: off the main thread, the answer on the main loop.
fn kalangod_async(tool: &'static str, args: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::servicos::kalangod_chamar(tool, args));
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(r) => {
            if let Some(d) = done.take() {
                d(r);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

/// The tunnel's address (the base of the trigger's) from the state file: the quick one's random name, or the domain.
fn base_url() -> Option<String> {
    let t = std::fs::read_to_string(modelo::state_file()).ok().and_then(|t| modelo::parse_tunnel(&t))?;
    (t.open && !t.url.is_empty()).then_some(t.url)
}

impl InternetForm {
    /// The form of the trigger `id`; `wide`: the sheet, with the example on the right.
    pub fn new(id: &str, wide: bool) -> Rc<InternetForm> {
        let root = caixa(Orientation::Vertical, 6, "");
        let error = rotulo("", "fluxo-erro");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_max_width_chars(1);
        error.set_visible(false);
        let me = Rc::new(InternetForm { root, id: id.to_string(), wide, desc: RefCell::default(), lang: RefCell::new("curl".into()), error, calls: RefCell::default() });
        // (the form lives as long as its widget: the callers only keep the widget)
        let keep = me.clone();
        me.root.connect_destroy(move |_| drop(keep.clone()));
        me.reload();
        me
    }

    /// Asks the service for the trigger as it is now and draws it.
    pub fn reload(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("internet.trigger_list", json!({}), move |r| {
            let Some(me) = w.upgrade() else { return };
            let id = me.id.clone();
            let found = r.ok().and_then(|v| v.as_array().and_then(|a| a.iter().find(|t| t["id"] == id.as_str()).cloned()));
            match found {
                Some(d) => {
                    *me.desc.borrow_mut() = d;
                    me.draw();
                    // the latest calls, for the card that says the limit was passed (drawn again when they arrive)
                    let w2 = Rc::downgrade(&me);
                    crate::fluxos::call("internet.calls", json!({"trigger_id": me.id, "limit": 100}), move |r| {
                        if let (Some(me), Ok(v)) = (w2.upgrade(), r) {
                            *me.calls.borrow_mut() = modelo::parse_calls(&v);
                            me.draw();
                        }
                    });
                }
                None => me.show_error(tr("This trigger no longer exists.")),
            }
        });
    }

    fn show_error(&self, t: &str) {
        self.error.set_label(t);
        self.error.set_visible(!t.is_empty());
    }

    /// One change: the service applies it and answers with the trigger as it holds it.
    fn update(self: &Rc<Self>, fields: Value) {
        let mut p = fields;
        p["id"] = json!(self.id);
        let w = Rc::downgrade(self);
        crate::fluxos::call("internet.trigger_update", p, move |r| {
            let Some(me) = w.upgrade() else { return };
            match r {
                Ok(_) => {
                    me.show_error("");
                    me.reload();
                }
                Err(e) => {
                    me.show_error(&e);
                    me.reload();
                }
            }
        });
    }

    /// The level: a rise asks on kalangod's approval screen first (and the choice goes back if it is not given).
    fn pick_level(self: &Rc<Self>, to: &str) {
        let from = self.desc.borrow()["level"].as_str().unwrap_or("observar").to_string();
        if to == from {
            return;
        }
        if !modelo::rises(&from, to) {
            self.update(json!({"max_level": to}));
            return;
        }
        let w = Rc::downgrade(self);
        let id = self.id.clone();
        kalangod_async("gatilho_internet.editar", json!({"id": id, "nivel": to}), move |r| {
            let Some(me) = w.upgrade() else { return };
            if let Err(e) = r {
                me.show_error(&e);
            }
            me.reload();
        });
    }

    fn draw(self: &Rc<Self>) {
        let d = self.desc.borrow().clone();
        let root = &self.root;
        while let Some(c) = root.first_child() {
            root.remove(&c);
        }
        // (the form must never say "signed" without the service saying so: a missing field reads as public)
        let signed = d["signed"].as_bool() == Some(true);
        let level = d["level"].as_str().unwrap_or("observar").to_string();
        let enabled = d["enabled"].as_bool() == Some(true);
        let left = caixa(Orientation::Vertical, 6, "");
        left.set_hexpand(true);

        // the head: the globe, the name, the switch
        let head = caixa(Orientation::Horizontal, 10, "");
        let tile = caixa(Orientation::Horizontal, 0, "fluxo-gatilho-tile");
        tile.set_size_request(28, 28);
        tile.set_hexpand(false);
        tile.append(&ui_icon("globo", "#C9CAD1", 16));
        head.append(&tile);
        let names = caixa(Orientation::Vertical, 1, "");
        names.set_hexpand(true);
        let t = rotulo(tr("Over the internet"), "fluxo-chave-titulo");
        t.set_xalign(0.0);
        names.append(&t);
        names.append(&note(tr("trigger · calls this flow from outside the computer")));
        head.append(&names);
        let sw = gtk::Switch::new();
        sw.set_active(enabled);
        sw.set_valign(Align::Center);
        let w = Rc::downgrade(self);
        sw.connect_state_set(move |_, on| {
            if let Some(me) = w.upgrade() {
                let w2 = Rc::downgrade(&me);
                crate::fluxos::call("internet.trigger_set_enabled", json!({"id": me.id, "on": on}), move |r| {
                    if let Some(me) = w2.upgrade() {
                        if let Err(e) = r {
                            me.show_error(&e);
                        }
                        me.reload();
                    }
                });
            }
            glib::Propagation::Proceed
        });
        head.append(&sw);
        left.append(&head);
        left.append(&self.error);

        // off (design “Gatilho desligado”): since when and how many calls it refused since
        if !enabled {
            let when = d["disabled_at"].as_i64().and_then(|t| glib::DateTime::from_unix_local(t).ok()).and_then(|x| x.format("%d/%m %H:%M").ok()).map(|x| x.to_string()).unwrap_or_default();
            let (title, text) = modelo::off_card(d["disabled_at"].as_i64(), d["refused_since_disabled"].as_i64().unwrap_or(0), &when);
            let b = caixa(Orientation::Vertical, 3, "int-endereco");
            b.append(&rotulo(&title, "fluxo-chave-titulo"));
            b.append(&note(&text));
            b.append(&note(tr("Turning it on again does not change the address or the key; if the tunnel is closed, opening it asks for approval.")));
            left.append(&b);
        }
        // the limit passed (design “Limite de chamadas”): the 429s of the last minute, with a small chart per minute
        let rate = d["rate_per_minute"].as_u64().unwrap_or(30) as u32;
        let now_s = glib::DateTime::now_local().map(|x| x.to_unix()).unwrap_or(0);
        if let Some((title, text)) = modelo::rate_summary(&self.calls.borrow(), now_s, rate) {
            let b = caixa(Orientation::Vertical, 4, "int-aviso-ouro");
            b.append(&rotulo(&title, "int-aviso-titulo"));
            b.append(&note(&text));
            b.append(&chart(&modelo::by_minute(&self.calls.borrow(), now_s, 30), rate));
            left.append(&b);
        }

        // the name (optional): what the cards and the log call this trigger; empty means the flow's name
        left.append(&section(tr("Name (optional)")));
        let name_entry = entry_box(if d["name_set"] == true { d["name"].as_str().unwrap_or("") } else { "" }, d["flow_name"].as_str().unwrap_or(""));
        let w = Rc::downgrade(self);
        name_entry.connect_activate(move |e| {
            if let Some(me) = w.upgrade() {
                me.update(json!({"name": e.text().trim()}));
            }
        });
        left.append(&name_entry);

        // the address
        left.append(&section(tr("Address")));
        let base = base_url();
        let path = d["path"].as_str().unwrap_or("").to_string();
        let shown = match &base {
            Some(b) => format!("{b}{path}"),
            None => format!("https://…{path}"),
        };
        let row = caixa(Orientation::Horizontal, 8, "int-endereco");
        let addr = rotulo(&shown, "int-mono");
        addr.set_xalign(0.0);
        addr.set_hexpand(true);
        addr.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        addr.set_max_width_chars(1);
        addr.set_selectable(true);
        row.append(&addr);
        let cp = gtk::Button::with_label(tr("Copy"));
        cp.add_css_class("privacidade-link");
        let s2 = shown.clone();
        cp.connect_clicked(move |_| copy(&s2));
        row.append(&cp);
        left.append(&row);
        left.append(&note(if base.is_some() {
            tr("Quick tunnel: a random address that changes at each opening. My domain: a fixed address (Settings › Tunnels).")
        } else {
            tr("The tunnel is closed: the address shows when it opens. Opening asks for your approval, each time.")
        }));

        // the key: when it was made, never what it is — except the one just made
        left.append(&section(tr("Signing key (HMAC-SHA256)")));
        let fresh = FRESH_KEY.with(|k| k.borrow().get(&self.id).cloned());
        if let Some(key) = fresh {
            let b = caixa(Orientation::Vertical, 6, "int-chave-nova");
            b.append(&rotulo(tr("Copy now: it does not show again"), "int-chave-aviso"));
            let r = caixa(Orientation::Horizontal, 8, "");
            let k = rotulo(&key, "int-mono");
            k.set_xalign(0.0);
            k.set_hexpand(true);
            k.set_ellipsize(gtk::pango::EllipsizeMode::End);
            k.set_max_width_chars(1);
            r.append(&k);
            let cb = gtk::Button::with_label(tr("Copy"));
            cb.add_css_class("privacidade-link");
            let (id, key2, w) = (self.id.clone(), key.clone(), Rc::downgrade(self));
            cb.connect_clicked(move |_| {
                copy(&key2);
                // (copied: it is not kept here any longer)
                FRESH_KEY.with(|k| k.borrow_mut().remove(&id));
                if let Some(me) = w.upgrade() {
                    me.draw();
                }
            });
            r.append(&cb);
            b.append(&r);
            b.append(&note(tr("Kept in the system vault. If you lose it, make a new one.")));
            left.append(&b);
        } else if signed {
            let r = caixa(Orientation::Horizontal, 8, "int-endereco");
            let k = rotulo(tr("···· kept in the system vault"), "int-mono");
            k.set_xalign(0.0);
            k.set_hexpand(true);
            r.append(&k);
            let new = gtk::Button::with_label(tr("Make a new key"));
            new.add_css_class("privacidade-link");
            let w = Rc::downgrade(self);
            new.connect_clicked(move |_| {
                let Some(me) = w.upgrade() else { return };
                let w2 = Rc::downgrade(&me);
                crate::fluxos::call("internet.trigger_rotate_secret", json!({"id": me.id, "reveal_secret": true}), move |r| {
                    let Some(me) = w2.upgrade() else { return };
                    match r {
                        Ok(v) => {
                            if let Some(k) = v["secret"].as_str() {
                                FRESH_KEY.with(|f| f.borrow_mut().insert(me.id.clone(), k.to_string()));
                            }
                            me.draw();
                        }
                        Err(e) => me.show_error(&e),
                    }
                });
            });
            r.append(&new);
            left.append(&r);
        } else {
            left.append(&note(tr("No key: this trigger is public.")));
        }
        left.append(&note(&format!(
            "{}: {} ({}), {} ({}) · {} ({})",
            tr("Headers"),
            modelo::HEADER_SIGNATURE,
            tr("HMAC of the body + time"),
            modelo::HEADER_TIME,
            tr("up to 5 min of difference"),
            modelo::HEADER_NONCE,
            tr("optional; each nonce is worth once")
        )));

        // what it accepts
        left.append(&section(tr("What it accepts")));
        let methods: Vec<String> = d["methods"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        let mrow = caixa(Orientation::Horizontal, 6, "");
        for m in METHODS {
            let (cur, w) = (methods.clone(), Rc::downgrade(self));
            mrow.append(&chip(m, methods.iter().any(|x| x == m), move |on| {
                let mut next: Vec<String> = cur.iter().filter(|x| x.as_str() != m).cloned().collect();
                if on {
                    next.push(m.to_string());
                }
                if next.is_empty() {
                    // (at least one method: the choice is put back)
                    if let Some(me) = w.upgrade() {
                        me.draw();
                    }
                } else if let Some(me) = w.upgrade() {
                    me.update(json!({"methods": next}));
                }
            }));
        }
        left.append(&mrow);
        let types: Vec<String> = d["content_types"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        let trow = caixa(Orientation::Horizontal, 6, "");
        for (value, label) in TYPES {
            let (cur, w) = (types.clone(), Rc::downgrade(self));
            trow.append(&chip(label, types.iter().any(|x| x == value), move |on| {
                let mut next: Vec<String> = cur.iter().filter(|x| x.as_str() != value).cloned().collect();
                if on {
                    next.push(value.to_string());
                }
                if let Some(me) = w.upgrade() {
                    if next.is_empty() {
                        me.draw();
                    } else {
                        me.update(json!({"content_types": next}));
                    }
                }
            }));
        }
        left.append(&trow);
        let limits = caixa(Orientation::Horizontal, 10, "");
        let kb = d["max_body_bytes"].as_u64().unwrap_or(65536) / 1024;
        let size = caixa(Orientation::Vertical, 3, "");
        size.set_hexpand(true);
        size.append(&note(tr("Maximum size")));
        let se = entry_box(&format!("{kb} KB"), "64 KB");
        size.append(&se);
        let rate = caixa(Orientation::Vertical, 3, "");
        rate.set_hexpand(true);
        rate.append(&note(tr("Calls")));
        let re = entry_box(&format!("{} / min", d["rate_per_minute"].as_u64().unwrap_or(30)), "30 / min");
        rate.append(&re);
        limits.append(&size);
        limits.append(&rate);
        left.append(&limits);
        let w = Rc::downgrade(self);
        se.connect_activate(move |e| {
            let kb: u64 = e.text().chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
            if let Some(me) = w.upgrade() {
                me.update(json!({"max_body_bytes": kb.clamp(1, 1024) * 1024}));
            }
        });
        let w = Rc::downgrade(self);
        re.connect_activate(move |e| {
            let n: u64 = e.text().chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
            if let Some(me) = w.upgrade() {
                me.update(json!({"rate_per_minute": n.clamp(1, 600)}));
            }
        });

        // how it answers
        left.append(&section(tr("Answer")));
        let waits = d["respond"]["mode"] == "wait";
        let w = Rc::downgrade(self);
        let resp = segmented(
            &[("accepted".into(), tr("Immediate (202)").to_string()), ("wait".into(), tr("Wait for the result (up to 30 s)").to_string())],
            if waits { "wait" } else { "accepted" },
            move |v| {
                if let Some(me) = w.upgrade() {
                    me.update(json!({"respond": v, "wait_timeout_s": 30}));
                }
            },
        );
        resp.set_sensitive(signed);
        left.append(&resp);
        if waits {
            // (said where it is chosen: the result of the flow goes back to whoever calls, even at the lowest level)
            left.append(&note(tr("Whoever calls gets the result of the flow in the answer, even at the lowest level.")));
        }

        // who may call
        left.append(&section(tr("Allowed origins (optional)")));
        let ips: Vec<String> = d["allow_ips"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        let orow = caixa(Orientation::Horizontal, 8, "");
        let oe = entry_box(&ips.join(", "), tr("any origin"));
        orow.append(&oe);
        let limit = gtk::Button::with_label(tr("Limit"));
        limit.add_css_class("privacidade-link");
        let (oe2, w) = (oe.clone(), Rc::downgrade(self));
        limit.connect_clicked(move |_| {
            let list: Vec<String> = oe2.text().split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            if let Some(me) = w.upgrade() {
                me.update(json!({"allow_ips": list}));
            }
        });
        orow.append(&limit);
        left.append(&orow);

        // the level
        left.append(&section(tr("Level of this trigger")));
        let w = Rc::downgrade(self);
        let lv = segmented(&modelo::levels().into_iter().map(|(v, l)| (v.to_string(), l)).collect::<Vec<_>>(), &level, move |to| {
            if let Some(me) = w.upgrade() {
                me.pick_level(to);
            }
        });
        lv.set_sensitive(signed);
        left.append(&lv);
        let (warn, text) = modelo::level_note(&level);
        if warn {
            let b = caixa(Orientation::Vertical, 3, "int-aviso-ouro");
            b.append(&rotulo(&modelo::level_warning_title(&level), "int-aviso-titulo"));
            b.append(&note(&text));
            left.append(&b);
        } else {
            left.append(&note(&text));
        }

        // security: public, without a signature
        left.append(&section(tr("Security")));
        let prow = caixa(Orientation::Horizontal, 10, "");
        let pn = caixa(Orientation::Vertical, 1, "");
        pn.set_hexpand(true);
        let pt = rotulo(tr("Public without signature"), if signed { "fluxo-chave-titulo" } else { "int-vermelho" });
        pt.set_xalign(0.0);
        pn.append(&pt);
        pn.append(&note(tr("Accepts calls without the key")));
        prow.append(&pn);
        let psw = gtk::Switch::new();
        psw.set_active(!signed);
        psw.set_valign(Align::Center);
        let w = Rc::downgrade(self);
        psw.connect_state_set(move |_, on| {
            if let Some(me) = w.upgrade() {
                let was_public = me.desc.borrow()["signed"].as_bool() != Some(true);
                if on != was_public {
                    let mut p = json!({"public_unsigned": on});
                    if !on {
                        p["reveal_secret"] = json!(true);
                    }
                    let w2 = Rc::downgrade(&me);
                    let mut fields = p;
                    fields["id"] = json!(me.id);
                    crate::fluxos::call("internet.trigger_update", fields, move |r| {
                        let Some(me) = w2.upgrade() else { return };
                        match r {
                            Ok(v) => {
                                if let Some(k) = v["secret"].as_str() {
                                    FRESH_KEY.with(|f| f.borrow_mut().insert(me.id.clone(), k.to_string()));
                                }
                                me.show_error("");
                            }
                            Err(e) => me.show_error(&e),
                        }
                        me.reload();
                    });
                }
            }
            glib::Propagation::Proceed
        });
        prow.append(&psw);
        left.append(&prow);
        if !signed {
            let (title, text) = modelo::public_warning();
            let b = caixa(Orientation::Vertical, 3, "int-aviso-vermelho");
            b.append(&rotulo(&title, "int-vermelho"));
            b.append(&note(&text));
            left.append(&b);
        }

        // the sheet: the example on the right
        if self.wide {
            let cols = caixa(Orientation::Horizontal, 18, "");
            cols.append(&left);
            cols.append(&self.example_column(&shown, signed));
            root.append(&cols);
        } else {
            root.append(&left);
        }
    }

    fn example_column(self: &Rc<Self>, url: &str, signed: bool) -> gtk::Box {
        let col = caixa(Orientation::Vertical, 10, "");
        col.set_size_request(360, -1);
        col.set_hexpand(false);
        let t = rotulo(tr("Example of a signed call"), "fluxo-chave-titulo");
        t.set_xalign(0.0);
        col.append(&t);
        let lang = self.lang.borrow().clone();
        let w = Rc::downgrade(self);
        let tabs = segmented(&[("curl".into(), "curl".into()), ("python".into(), "Python".into()), ("javascript".into(), "JavaScript".into())], &lang, move |l| {
            if let Some(me) = w.upgrade() {
                *me.lang.borrow_mut() = l.to_string();
                me.draw();
            }
        });
        col.append(&tabs);
        let code = modelo::example(&lang, url);
        let box_ = caixa(Orientation::Vertical, 0, "int-codigo");
        let l = rotulo(&code, "int-mono");
        l.set_xalign(0.0);
        l.set_selectable(true);
        l.set_wrap(false);
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        sc.set_propagate_natural_height(true);
        sc.set_child(Some(&l));
        box_.append(&sc);
        col.append(&box_);
        let cp = gtk::Button::with_label(tr("Copy"));
        cp.add_css_class("privacidade-link");
        cp.set_halign(Align::End);
        cp.connect_clicked(move |_| copy(&code));
        col.append(&cp);
        let how = caixa(Orientation::Vertical, 4, "int-aviso-ouro");
        how.append(&rotulo(tr("How it is checked"), "int-aviso-titulo"));
        how.append(&note(if signed {
            tr("Kalango recomputes the HMAC with the key from the Vault, refuses a time more than 5 minutes off and a repeated nonce. If it fails: 401 and the flow does not run.")
        } else {
            tr("This trigger is public: nothing is checked, and the flow runs at the lowest level.")
        }));
        col.append(&how);
        col
    }
}

/// A small chart: calls per minute, the minutes over the limit in amber, the limit as a line.
pub(crate) fn chart(bars: &[u32], limit: u32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(48);
    area.set_hexpand(true);
    let (bars, limit) = (bars.to_vec(), limit);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let top = bars.iter().copied().max().unwrap_or(1).max(limit).max(1) as f64;
        let n = bars.len().max(1) as f64;
        let bw = (w / n - 2.0).max(1.0);
        for (i, v) in bars.iter().enumerate() {
            let bh = (*v as f64 / top * (h - 4.0)).max(if *v > 0 { 2.0 } else { 0.0 });
            if *v > limit { cr.set_source_rgb(0.898, 0.788, 0.549) } else { cr.set_source_rgb(0.557, 0.824, 0.765) }
            cr.rectangle(i as f64 * (bw + 2.0), h - bh, bw, bh);
            let _ = cr.fill();
        }
        cr.set_source_rgba(0.937, 0.635, 0.651, 0.9);
        cr.set_line_width(1.0);
        let y = h - (limit as f64 / top * (h - 4.0));
        cr.move_to(0.0, y);
        cr.line_to(w, y);
        let _ = cr.stroke();
    });
    area
}

/// Forgets the key just made (leaving the screen).
pub fn forget_fresh_keys() {
    FRESH_KEY.with(|k| k.borrow_mut().clear());
}

/// Makes the internet trigger of a flow (the key comes back once, kept here until the form shows it) and calls `done`.
pub fn create(flow_id: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    crate::fluxos::call("internet.trigger_create", json!({"flow_id": flow_id, "reveal_secret": true}), move |r| match r {
        Ok(v) => {
            let id = v["id"].as_str().unwrap_or("").to_string();
            if let Some(k) = v["secret"].as_str() {
                FRESH_KEY.with(|f| f.borrow_mut().insert(id.clone(), k.to_string()));
            }
            done(Ok(id));
        }
        Err(e) => done(Err(e)),
    });
}

/// The full sheet (design "folha completa"): the form in two columns, with the example, and "Save and open the tunnel".
/// In a narrow window it is one column (design "janela estreita").
pub fn open_sheet(trigger_id: &str, flow_name: &str) {
    let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
    let screen_w = gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>().map(|m| m.geometry().width()).unwrap_or(1280);
    let wide = screen_w >= 1300;
    // a layer over everything, centred (like the other sheets of the shell): it does not depend on where the compositor
    // would put a new window
    let win = crate::superficies::camada(&app, "kalango-internet-folha", gtk4_layer_shell::Layer::Overlay, &[]);
    win.add_css_class("transparente");
    gtk4_layer_shell::LayerShell::set_keyboard_mode(&win, gtk4_layer_shell::KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 10, "int-folha-corpo");
    // (the width is the sheet's own, never more than the screen leaves)
    body.set_size_request(if wide { 900.min(screen_w - 120) } else { 520.min(screen_w - 40) }, -1);
    let head = caixa(Orientation::Horizontal, 8, "");
    let title = rotulo(&kalango_i18n::trf("Internet trigger · {flow}", &[("flow", flow_name.to_string())]), "privacidade-titulo");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    head.append(&title);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("privacidade-link");
    head.append(&close);
    body.append(&head);
    let form = InternetForm::new(trigger_id, wide);
    let sc = gtk::ScrolledWindow::new();
    sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sc.set_propagate_natural_height(true);
    sc.set_max_content_height(screen_w.min(900) / 2 + 200);
    sc.set_child(Some(&form.root));
    body.append(&sc);
    let foot = caixa(Orientation::Horizontal, 8, "");
    let hint = rotulo(tr("Opening the tunnel asks for your approval, each time."), "fluxo-chave-sub");
    hint.set_xalign(0.0);
    hint.set_hexpand(true);
    hint.set_wrap(true);
    hint.set_max_width_chars(1);
    foot.append(&hint);
    let cancel = gtk::Button::with_label(tr("Cancel"));
    cancel.add_css_class("tunel-botao");
    let save = gtk::Button::new();
    save.add_css_class("rotinas-nova");
    save.set_child(Some(&rotulo(tr("Save and open the tunnel"), "")));
    foot.append(&cancel);
    foot.append(&save);
    body.append(&foot);
    win.set_child(Some(&body));
    let w = win.clone();
    close.connect_clicked(move |_| w.close());
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let (w, f) = (win.clone(), form.clone());
    save.connect_clicked(move |_| {
        // (every change was already saved as it was made: this opens the tunnel, which asks)
        let (w, f) = (w.clone(), f.clone());
        crate::fluxos::call("internet.tunnel_open", json!({"mode": "quick"}), move |r| match r {
            Ok(_) => w.close(),
            Err(e) => f.show_error(&e),
        });
    });
    let keys = gtk::EventControllerKey::new();
    let w = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    win.connect_close_request(|_| {
        forget_fresh_keys();
        glib::Propagation::Proceed
    });
    win.present();
}
