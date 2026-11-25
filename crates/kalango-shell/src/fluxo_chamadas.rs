//! The calls of an internet trigger (design "Gatilho pela internet · registro de chamadas" and its narrow sheet): the
//! last calls with time, status, size, signature, origin and what became of the flow; the filters by status and by
//! failed signature; and the detail of one call, where the body is only a hash and a size until the person asks to see
//! it (and confirms: it is external content, and it is recorded that it was shown).
//!
//! Origin is the hash the service keeps (never an address); this computer has no country list, so the country the design
//! shows beside it is not shown.

use crate::internet_modelo::{self as modelo, Call, Filter};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A window narrower than this has no room for the detail pane.
const NARROW: i32 = 900;

pub struct CallsView {
    pub root: gtk::Box,
    trigger_id: String,
    flow_name: String,
    calls: RefCell<Vec<Call>>,
    filter: RefCell<Filter>,
    chosen: Cell<Option<usize>>,
    rate: Cell<u32>,
    list: gtk::Box,
    chips: gtk::Box,
    detail: gtk::Box,
    sub: gtk::Label,
    title: gtk::Label,
    /// the rate card and its chart, above the table
    alert: gtk::Box,
    /// the detail as a full-height sheet (a narrow window), when it is open
    sheet: RefCell<Option<(gtk::ApplicationWindow, gtk::Box)>>,
    confirming: Cell<bool>,
    body_text: RefCell<Option<String>>,
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

fn hour(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M:%S").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn cell(text: &str, width: i32, class: &str) -> gtk::Label {
    let l = rotulo(text, class);
    l.set_xalign(0.0);
    l.set_size_request(width, -1);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l
}

impl CallsView {
    /// `back` returns to the flow's editor.
    pub fn new(trigger_id: &str, flow_name: &str, back: impl Fn() + 'static) -> Rc<CallsView> {
        let root = caixa(Orientation::Horizontal, 0, "");
        root.set_hexpand(true);
        root.set_vexpand(true);
        let main = caixa(Orientation::Vertical, 8, "int-chamadas");
        main.set_hexpand(true);
        let top = caixa(Orientation::Horizontal, 8, "");
        let bk = gtk::Button::with_label(&format!("‹ {}", tr("Edit")));
        bk.add_css_class("privacidade-link");
        bk.connect_clicked(move |_| back());
        top.append(&bk);
        main.append(&top);
        let title = rotulo(&trf("{flow} · calls", &[("flow", flow_name.to_string())]), "central-titulo");
        title.set_xalign(0.0);
        main.append(&title);
        let alert = caixa(Orientation::Vertical, 4, "int-aviso-ouro");
        alert.set_visible(false);
        let sub = rotulo("", "central-sub");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(1);
        main.append(&sub);
        let chips = caixa(Orientation::Horizontal, 6, "");
        chips.set_margin_top(6);
        main.append(&chips);
        main.append(&alert);
        let head = caixa(Orientation::Horizontal, 8, "int-chamadas-cab");
        for (t, w) in [(tr("TIME"), 80), (tr("STATUS"), 62), (tr("SIZE"), 70), (tr("SIGNATURE"), 130), (tr("ORIGIN"), 110), (tr("FLOW"), 130)] {
            head.append(&cell(t, w, "fluxo-secao-cab"));
        }
        main.append(&head);
        let list = caixa(Orientation::Vertical, 2, "");
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sc.set_vexpand(true);
        sc.set_child(Some(&list));
        main.append(&sc);
        let foot = rotulo(tr("The content of a call stays only in the record of its run: here there is just the hash and the size."), "fluxo-chave-sub");
        foot.set_xalign(0.0);
        foot.set_wrap(true);
        foot.set_max_width_chars(1);
        main.append(&foot);
        root.append(&main);
        let detail = caixa(Orientation::Vertical, 10, "central-painel");
        detail.set_size_request(320, -1);
        detail.set_hexpand(false);
        root.append(&detail);
        let me = Rc::new(CallsView {
            root,
            trigger_id: trigger_id.to_string(),
            flow_name: flow_name.to_string(),
            calls: RefCell::default(),
            filter: RefCell::new(Filter::All),
            chosen: Cell::new(None),
            rate: Cell::new(30),
            list,
            chips,
            detail,
            sub,
            title,
            alert,
            sheet: RefCell::default(),
            confirming: Cell::new(false),
            body_text: RefCell::default(),
        });
        let keep = me.clone();
        me.root.connect_destroy(move |_| drop(keep.clone()));
        // (a narrow window has no room for the pane: the detail comes as a sheet instead)
        let w = Rc::downgrade(&me);
        me.root.add_tick_callback(move |root, _| {
            if let Some(me) = w.upgrade() {
                let narrow = root.width() > 0 && root.width() < NARROW;
                if me.detail.is_visible() == narrow {
                    me.detail.set_visible(!narrow);
                }
            }
            glib::ControlFlow::Continue
        });
        me.reload();
        me
    }

    fn narrow(&self) -> bool {
        self.root.width() > 0 && self.root.width() < NARROW
    }

    pub fn reload(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("internet.calls", json!({"trigger_id": self.trigger_id, "limit": 200, "include_unknown": true}), move |r| {
            let Some(me) = w.upgrade() else { return };
            *me.calls.borrow_mut() = r.map(|v| modelo::parse_calls(&v)).unwrap_or_default();
            me.draw();
        });
        let w = Rc::downgrade(self);
        crate::fluxos::call("internet.trigger_list", json!({}), move |r| {
            if let (Some(me), Ok(v)) = (w.upgrade(), r) {
                let found = v.as_array().and_then(|a| a.iter().find(|t| t["id"] == me.trigger_id.as_str())).cloned();
                if let Some(t) = found {
                    if let Some(n) = t["rate_per_minute"].as_u64() {
                        me.rate.set(n as u32);
                    }
                    // (the name the person gave it, else the flow's)
                    if let Some(name) = t["name"].as_str() {
                        me.title.set_label(&trf("{flow} · calls", &[("flow", name.to_string())]));
                    }
                    me.draw();
                }
            }
        });
    }

    fn visible(&self) -> Vec<(usize, Call)> {
        let f = self.filter.borrow().clone();
        self.calls.borrow().iter().cloned().enumerate().filter(|(_, c)| modelo::matches(&f, c)).collect()
    }

    fn draw(self: &Rc<Self>) {
        self.sub.set_label(&trf("Internet trigger · last calls · the origin is shown only as a hash, never the address", &[]));
        // the limit passed: the 429s of the last minute and a chart per minute
        while let Some(c) = self.alert.first_child() {
            self.alert.remove(&c);
        }
        let t_now = now();
        match modelo::rate_summary(&self.calls.borrow(), t_now, self.rate.get()) {
            Some((title, text)) => {
                self.alert.append(&rotulo(&title, "int-aviso-titulo"));
                let l = rotulo(&text, "fluxo-chave-sub");
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_max_width_chars(1);
                self.alert.append(&l);
                self.alert.append(&crate::fluxo_internet::chart(&modelo::by_minute(&self.calls.borrow(), t_now, 30), self.rate.get()));
                self.alert.set_visible(true);
            }
            None => self.alert.set_visible(false),
        }
        // the filters
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let current = self.filter.borrow().clone();
        for (label, f) in modelo::filters(&self.calls.borrow()) {
            let b = gtk::ToggleButton::with_label(&label);
            b.add_css_class("int-chip");
            b.set_active(f == current);
            let (w, f2) = (Rc::downgrade(self), f.clone());
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    *me.filter.borrow_mut() = f2.clone();
                    me.chosen.set(None);
                    me.draw();
                }
            });
            self.chips.append(&b);
        }
        // the rows
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let rows = self.visible();
        if rows.is_empty() {
            let l = rotulo(tr("No calls yet."), "fluxo-chave-sub");
            l.set_xalign(0.0);
            l.set_margin_top(10);
            self.list.append(&l);
        }
        let chosen = self.chosen.get().or_else(|| rows.first().map(|(i, _)| *i));
        self.chosen.set(chosen);
        for (i, c) in rows {
            let b = gtk::Button::new();
            b.add_css_class("int-chamada-linha");
            if Some(i) == chosen {
                b.add_css_class("ativa");
            }
            let r = caixa(Orientation::Horizontal, 8, "");
            r.append(&cell(&hour(c.at), 80, "int-mono"));
            let st = cell(&c.status.to_string(), 62, "int-estado");
            st.add_css_class(if c.status < 300 { "ok" } else if c.status < 500 { "aviso" } else { "erro" });
            r.append(&st);
            r.append(&cell(&modelo::size(c.body_bytes), 70, "fluxo-chave-sub"));
            let sg = modelo::signature(&c);
            let sc = cell(&modelo::signature_text(sg), 130, "fluxo-chave-sub");
            if matches!(sg, modelo::Sig::Failed | modelo::Sig::Stale) {
                sc.add_css_class("int-vermelho");
            }
            r.append(&sc);
            r.append(&cell(&modelo::origin(&c.client), 110, "int-mono"));
            r.append(&cell(&modelo::flow_text(&c, self.rate.get()), 130, "fluxo-chave-sub"));
            b.set_child(Some(&r));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(me) = w.upgrade() {
                    me.chosen.set(Some(i));
                    me.confirming.set(false);
                    *me.body_text.borrow_mut() = None;
                    me.draw();
                    if me.narrow() {
                        me.open_sheet();
                    }
                }
            });
            self.list.append(&b);
        }
        self.draw_detail();
    }

    fn draw_detail(self: &Rc<Self>) {
        let pane = self.detail.clone();
        self.fill_detail(&pane);
        // the sheet, if open, shows the same call
        let sheet = self.sheet.borrow().clone();
        if let Some((_, b)) = sheet {
            self.fill_detail(&b);
        }
    }

    /// A full-height sheet on the right with the detail (a narrow window: design “janela estreita”).
    fn open_sheet(self: &Rc<Self>) {
        let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
        if let Some((w, _)) = self.sheet.borrow_mut().take() {
            w.close();
        }
        let win = crate::superficies::camada(&app, "kalango-chamada-folha", gtk4_layer_shell::Layer::Overlay, &[gtk4_layer_shell::Edge::Top, gtk4_layer_shell::Edge::Bottom, gtk4_layer_shell::Edge::Right]);
        win.add_css_class("transparente");
        gtk4_layer_shell::LayerShell::set_keyboard_mode(&win, gtk4_layer_shell::KeyboardMode::OnDemand);
        let shell = caixa(Orientation::Vertical, 10, "int-folha-corpo");
        shell.add_css_class("int-folha-lateral");
        shell.set_size_request(380, -1);
        let close = gtk::Button::with_label(&format!("‹ {}", tr("Calls")));
        close.add_css_class("privacidade-link");
        close.set_halign(Align::Start);
        shell.append(&close);
        let content = caixa(Orientation::Vertical, 10, "");
        shell.append(&content);
        win.set_child(Some(&shell));
        let w = win.clone();
        close.connect_clicked(move |_| w.close());
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
        let me = Rc::downgrade(self);
        win.connect_close_request(move |_| {
            if let Some(me) = me.upgrade() {
                *me.sheet.borrow_mut() = None;
            }
            glib::Propagation::Proceed
        });
        *self.sheet.borrow_mut() = Some((win.clone(), content.clone()));
        self.fill_detail(&content);
        win.present();
    }

    /// Draws the detail of the chosen call into `target` (the pane or the sheet).
    fn fill_detail(self: &Rc<Self>, target: &gtk::Box) {
        while let Some(c) = target.first_child() {
            target.remove(&c);
        }
        let Some(c) = self.chosen.get().and_then(|i| self.calls.borrow().get(i).cloned()) else { return };
        let t = rotulo(&trf("Call · {time}", &[("time", hour(c.at))]), "privacidade-titulo");
        t.set_xalign(0.0);
        target.append(&t);
        let box_ = caixa(Orientation::Vertical, 6, "tunel-quadro");
        let fact = |k: &str, v: &str| {
            let r = caixa(Orientation::Horizontal, 10, "");
            let kl = cell(k, 80, "fluxo-chave-sub");
            r.append(&kl);
            let vl = rotulo(v, "int-mono");
            vl.set_xalign(0.0);
            vl.set_wrap(true);
            vl.set_max_width_chars(1);
            vl.set_hexpand(true);
            r.append(&vl);
            box_.append(&r);
        };
        fact(tr("Status"), &format!("{} · {}", c.status, status_word(c.status)));
        fact(tr("Method"), &c.method);
        fact(tr("Size"), &modelo::size(c.body_bytes));
        fact(tr("Signature"), &modelo::signature_text(modelo::signature(&c)));
        fact(tr("Origin"), &format!("{} ({})", modelo::origin(&c.client), tr("hash")));
        fact(tr("Flow"), &modelo::flow_text(&c, self.rate.get()));
        target.append(&box_);
        let body_head = rotulo(tr("Body"), "privacidade-titulo");
        body_head.set_xalign(0.0);
        body_head.set_margin_top(8);
        target.append(&body_head);
        let b = caixa(Orientation::Vertical, 6, "tunel-quadro");
        let h = rotulo(&format!("sha256  {}", modelo::body_hash(&c.body_sha256)), "int-mono");
        h.set_xalign(0.0);
        h.set_wrap(true);
        h.set_max_width_chars(1);
        b.append(&h);
        let size = rotulo(&trf("{n} bytes", &[("n", c.body_bytes.to_string())]), "fluxo-chave-sub");
        size.set_xalign(0.0);
        b.append(&size);
        let shown = self.body_text.borrow().clone();
        match (shown, c.run_id.clone()) {
            (Some(text), _) => {
                let l = rotulo(&text, "int-mono");
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_max_width_chars(1);
                l.set_selectable(true);
                b.append(&l);
                let n = rotulo(tr("External content: treat it as untrusted."), "int-aviso-titulo");
                n.set_xalign(0.0);
                b.append(&n);
            }
            (None, Some(run)) => {
                let btn = gtk::Button::with_label(if self.confirming.get() { tr("Confirm: show the content (it is recorded)") } else { tr("Show content") });
                btn.add_css_class("tunel-botao");
                let w = Rc::downgrade(self);
                btn.connect_clicked(move |_| {
                    let Some(me) = w.upgrade() else { return };
                    if !me.confirming.get() {
                        me.confirming.set(true);
                        me.draw_detail();
                        return;
                    }
                    let w2 = Rc::downgrade(&me);
                    crate::fluxos::call("internet.call_body", json!({"run_id": run}), move |r| {
                        let Some(me) = w2.upgrade() else { return };
                        *me.body_text.borrow_mut() = Some(r.map(|v| v["body"].as_str().unwrap_or("").to_string()).unwrap_or_else(|e| e));
                        me.confirming.set(false);
                        me.draw_detail();
                    });
                });
                b.append(&btn);
                let n = rotulo(tr("Showing asks for confirmation and is recorded. Content from outside: treat it as untrusted."), "fluxo-chave-sub");
                n.set_xalign(0.0);
                n.set_wrap(true);
                n.set_max_width_chars(1);
                b.append(&n);
            }
            (None, None) => {
                let n = rotulo(tr("This call did not start a run: there is no content kept."), "fluxo-chave-sub");
                n.set_xalign(0.0);
                n.set_wrap(true);
                n.set_max_width_chars(1);
                b.append(&n);
            }
        }
        target.append(&b);
        let _ = (&self.flow_name, now());
    }
}

fn status_word(status: i64) -> String {
    match status {
        200 | 202 => tr("accepted").to_string(),
        400 => tr("invalid request").to_string(),
        401 => tr("not authorized").to_string(),
        403 => tr("forbidden").to_string(),
        404 => tr("wrong path").to_string(),
        405 => tr("method not allowed").to_string(),
        408 => tr("too slow").to_string(),
        409 => tr("repeated").to_string(),
        413 => tr("too large").to_string(),
        415 => tr("type not accepted").to_string(),
        429 => tr("too many calls").to_string(),
        _ => tr("refused").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_the_receiver_gives_has_a_word() {
        for s in [200, 202, 400, 401, 403, 404, 405, 408, 409, 413, 415, 422, 429] {
            assert!(!status_word(s).is_empty(), "{s}");
        }
        assert_ne!(status_word(202), status_word(401));
        assert_eq!(status_word(200), status_word(202));
    }
}
