//! Agents center › Settings › Túneis (design "Ajustes · Túneis"): how the triggers that come from the internet reach
//! this computer. The kill switch at the top while a tunnel is open; "Túnel com meu domínio (fixo)" with the domain,
//! the token (kept in the system vault, never shown again), the state of the connection and the program (the pinned
//! cloudflared); the four steps to point a domain here; "Colar token" and "Testar"; and, on the side, quick tunnel
//! against own domain. Everything is read from the service (`internet.domain`) and the tunnel's state file.

use crate::conversa::ui_icon;
use crate::internet_modelo as modelo;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub struct TunnelsTab {
    pub body: gtk::Box,
    pub side: gtk::Box,
    banner: gtk::Box,
    banner_sub: gtk::Label,
    /// why the last opening failed (design “Falhou ao abrir”), when it did
    failure: gtk::Box,
    domain: gtk::Entry,
    token: gtk::Label,
    delete_token: gtk::Button,
    state: gtk::Label,
    state_dot: gtk::Box,
    program: gtk::Label,
    steps: gtk::Box,
    result: gtk::Label,
    data: RefCell<Value>,
}

fn dim(text: &str) -> gtk::Label {
    let l = rotulo(text, "central-agente-sub");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

fn field(grid: &gtk::Grid, row: i32, name: &str, w: &impl IsA<gtk::Widget>) {
    let k = rotulo(name, "central-agente-sub");
    k.set_xalign(0.0);
    k.set_valign(Align::Center);
    k.set_size_request(88, -1);
    grid.attach(&k, 0, row, 1, 1);
    w.set_hexpand(true);
    w.set_valign(Align::Center);
    grid.attach(w, 1, row, 1, 1);
}

impl TunnelsTab {
    pub fn new(side_width: i32) -> Rc<TunnelsTab> {
        let body = caixa(Orientation::Vertical, 0, "");
        body.set_margin_bottom(16);
        let intro = dim(tr("How the triggers from the internet reach this computer."));
        intro.set_margin_top(16);
        body.append(&intro);
        // the kill switch: only while a tunnel is open
        let banner = caixa(Orientation::Horizontal, 12, "tunel-banner");
        banner.set_margin_top(14);
        banner.set_visible(false);
        banner.append(&ui_icon("alerta", "#EFA2A6", 16));
        let bt = caixa(Orientation::Vertical, 2, "");
        bt.set_hexpand(true);
        let bh = rotulo(tr("Close all tunnels"), "privacidade-linha-titulo");
        bh.set_xalign(0.0);
        bt.append(&bh);
        let banner_sub = dim("");
        bt.append(&banner_sub);
        banner.append(&bt);
        let kill = gtk::Button::new();
        kill.add_css_class("tunel-fechar-todos");
        let kr = caixa(Orientation::Horizontal, 6, "");
        kr.append(&rotulo("✕", "tunel-fechar-x"));
        kr.append(&rotulo(tr("Close all"), "tunel-fechar-texto"));
        kill.set_child(Some(&kr));
        kill.set_valign(Align::Center);
        kill.connect_clicked(|_| crate::tunel_agora::ask_to_close());
        banner.append(&kill);
        body.append(&banner);
        let failure = caixa(Orientation::Vertical, 8, "");
        failure.set_margin_top(14);
        failure.set_visible(false);
        body.append(&failure);

        let h = rotulo(tr("Tunnel with my domain (fixed)"), "grade-agente-secao");
        h.set_xalign(0.0);
        h.set_margin_top(18);
        body.append(&h);
        let card = caixa(Orientation::Vertical, 0, "tunel-quadro");
        card.set_margin_top(8);
        let grid = gtk::Grid::new();
        grid.set_row_spacing(10);
        grid.set_column_spacing(14);
        let domain = gtk::Entry::new();
        domain.set_placeholder_text(Some("gatilhos.exemplo.com.br"));
        domain.add_css_class("tunel-entrada");
        field(&grid, 0, tr("Domain"), &domain);
        let tk = caixa(Orientation::Horizontal, 10, "");
        let token = rotulo("", "tunel-token");
        token.set_xalign(0.0);
        token.set_hexpand(true);
        token.set_ellipsize(gtk::pango::EllipsizeMode::End);
        token.set_max_width_chars(1);
        tk.append(&ui_icon("cadeado", "#8E8F99", 12));
        tk.append(&token);
        let delete_token = gtk::Button::with_label(tr("Delete token"));
        delete_token.add_css_class("privacidade-link");
        tk.append(&delete_token);
        field(&grid, 1, tr("Token"), &tk);
        let st = caixa(Orientation::Horizontal, 8, "");
        let state_dot = caixa(Orientation::Horizontal, 0, "privacidade-ponto");
        state_dot.set_size_request(7, 7);
        state_dot.set_valign(Align::Center);
        st.append(&state_dot);
        let state = rotulo("", "privacidade-linha-titulo");
        state.set_xalign(0.0);
        state.set_ellipsize(gtk::pango::EllipsizeMode::End);
        state.set_max_width_chars(1);
        state.set_hexpand(true);
        st.append(&state);
        field(&grid, 2, tr("State"), &st);
        let program = rotulo("", "privacidade-linha-titulo");
        program.set_xalign(0.0);
        program.set_wrap(true);
        program.set_max_width_chars(1);
        field(&grid, 3, tr("Program"), &program);
        card.append(&grid);
        body.append(&card);

        let h2 = rotulo(tr("How to point the domain here"), "grade-agente-secao");
        h2.set_xalign(0.0);
        h2.set_margin_top(20);
        body.append(&h2);
        let steps = caixa(Orientation::Vertical, 10, "");
        steps.set_margin_top(10);
        body.append(&steps);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_margin_top(14);
        let paste = gtk::Button::new();
        paste.add_css_class("tunel-botao");
        paste.set_child(Some(&rotulo(tr("Paste token"), "")));
        let test = gtk::Button::new();
        test.add_css_class("rotinas-nova");
        test.set_child(Some(&rotulo(tr("Test"), "")));
        buttons.append(&paste);
        buttons.append(&test);
        let result = dim("");
        result.set_valign(Align::Center);
        buttons.append(&result);
        body.append(&buttons);

        // the side: quick tunnel against own domain
        let side = caixa(Orientation::Vertical, 0, "central-painel");
        side.set_size_request(side_width, -1);
        side.set_hexpand(false);
        let st = rotulo(tr("Quick tunnel × my domain"), "central-painel-agente");
        st.set_xalign(0.0);
        side.append(&st);
        for t in [
            tr("Quick tunnel: no account, a random address at each opening; good for tests."),
            tr("My domain: a fixed address to put in sites and services; needs the token."),
            tr("Both pass through the approval at each opening and show in the bar while open."),
        ] {
            let l = dim(t);
            l.set_margin_top(8);
            side.append(&l);
        }

        let me = Rc::new(TunnelsTab { body, side, banner, banner_sub, failure, domain: domain.clone(), token, delete_token: delete_token.clone(), state, state_dot, program, steps, result: result.clone(), data: RefCell::default() });
        let w = Rc::downgrade(&me);
        domain.connect_activate({
            let w = w.clone();
            move |e| {
                let name = e.text().to_string();
                let w = w.clone();
                crate::fluxos::call("internet.domain_set", json!({"hostname": name}), move |r| {
                    if let Some(me) = w.upgrade() {
                        match r {
                            Ok(_) => me.reload(),
                            Err(e) => me.result.set_label(&e),
                        }
                    }
                });
            }
        });
        delete_token.connect_clicked({
            let w = w.clone();
            move |_| {
                let w = w.clone();
                crate::fluxos::call("internet.token_delete", json!({}), move |_| {
                    if let Some(me) = w.upgrade() {
                        me.reload();
                    }
                });
            }
        });
        paste.connect_clicked({
            let w = w.clone();
            move |_| {
                let Some(display) = gtk::gdk::Display::default() else { return };
                let w = w.clone();
                display.clipboard().read_text_async(gtk::gio::Cancellable::NONE, move |r| {
                    let Some(me) = w.upgrade() else { return };
                    let Ok(Some(text)) = r else {
                        me.result.set_label(tr("Nothing to paste: copy the token from your tunnel provider first."));
                        return;
                    };
                    // the token leaves the clipboard as soon as it is read: it lives in the vault from here on
                    if let Some(d) = gtk::gdk::Display::default() {
                        d.clipboard().set_text("");
                    }
                    let w2 = Rc::downgrade(&me);
                    crate::fluxos::call("internet.token_store", json!({"value": text.to_string()}), move |r| {
                        if let Some(me) = w2.upgrade() {
                            me.result.set_label(if r.is_ok() { tr("Token stored in the system vault.") } else { tr("That does not look like a token.") });
                            me.reload();
                        }
                    });
                });
            }
        });
        test.connect_clicked({
            let w = w.clone();
            move |_| {
                let Some(me) = w.upgrade() else { return };
                me.result.set_label(tr("Calling the address…"));
                let w = Rc::downgrade(&me);
                crate::fluxos::call("internet.tunnel_test", json!({}), move |r| {
                    let Some(me) = w.upgrade() else { return };
                    me.result.set_label(&match r {
                        Ok(v) if v["reaches_the_receiver"] == true => trf("It answers · {ms} ms · the way to this computer works.", &[("ms", v["millis"].to_string())]),
                        Ok(v) => trf("It answered {status}, but not from this computer's receiver.", &[("status", v["status"].to_string())]),
                        Err(e) => e,
                    });
                });
            }
        });
        // the state file changes while the page is open (the tunnel opens, closes, connects)
        glib::timeout_add_local(Duration::from_secs(2), {
            let w = Rc::downgrade(&me);
            move || match w.upgrade() {
                Some(me) => {
                    if me.body.is_mapped() {
                        me.paint();
                    }
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
        me
    }

    /// Reads the service (the domain, the token, the program) and draws.
    pub fn reload(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("internet.domain", json!({}), move |r| {
            if let Some(me) = w.upgrade() {
                *me.data.borrow_mut() = r.unwrap_or(Value::Null);
                me.paint();
            }
        });
        self.paint();
    }

    fn paint(&self) {
        let d = self.data.borrow().clone();
        let tunnel = std::fs::read_to_string(modelo::state_file()).ok().and_then(|t| modelo::parse_tunnel(&t)).unwrap_or_default();
        let host = d["hostname"].as_str().unwrap_or("").to_string();
        if !self.domain.has_focus() && self.domain.text() != host.as_str() {
            self.domain.set_text(&host);
        }
        // the banner: the kill switch while a tunnel is open
        self.banner.set_visible(tunnel.open);
        self.banner_sub.set_label(&trf("Closes the open tunnel now and pauses the triggers from the internet. Opening again asks for approval.", &[]));
        // the last opening that failed: what happened and what to do
        while let Some(c) = self.failure.first_child() {
            self.failure.remove(&c);
        }
        self.failure.set_visible(tunnel.failure.is_some() && !tunnel.open && !tunnel.opening);
        if let (Some((kind, message)), false) = (tunnel.failure.clone(), tunnel.open || tunnel.opening) {
            let (title, text, tone) = modelo::failure_card(&kind);
            let card = caixa(Orientation::Vertical, 6, if tone == "coral" { "int-aviso-vermelho" } else { "int-aviso-ouro" });
            let t = rotulo(&title, if tone == "coral" { "int-vermelho" } else { "int-aviso-titulo" });
            t.set_xalign(0.0);
            card.append(&t);
            card.append(&dim(&text));
            let details = dim(&message);
            details.set_visible(false);
            details.add_css_class("int-mono");
            let row = caixa(Orientation::Horizontal, 8, "");
            let retry = gtk::Button::with_label(tr("Try again"));
            retry.add_css_class("tunel-botao");
            retry.connect_clicked(|_| {
                // (asks for the approval again: the card of the design)
                crate::fluxos::call("internet.tunnel_open", json!({"mode": "quick"}), |_| {});
            });
            row.append(&retry);
            if kind != "denied" {
                let more = gtk::Button::with_label(tr("See details"));
                more.add_css_class("privacidade-link");
                let d2 = details.clone();
                more.connect_clicked(move |_| d2.set_visible(!d2.is_visible()));
                row.append(&more);
            }
            card.append(&row);
            card.append(&details);
            self.failure.append(&card);
        }
        // the token: since when, never what
        let stored = d["token_stored"].as_bool() == Some(true);
        self.token.set_label(&match d["token_created_at"].as_i64() {
            Some(t) if stored => trf("kept in the system vault · created on {date}", &[("date", date(t))]),
            _ if stored => tr("kept in the system vault").to_string(),
            _ => tr("no token yet").to_string(),
        });
        self.delete_token.set_visible(stored);
        // the connection
        let named = tunnel.open && tunnel.mode == "named";
        self.state.set_label(&if named { modelo::connection_line(&tunnel, d["port"].as_u64()) } else { tr("closed").to_string() });
        for c in ["teal", "ouro", "mudo"] {
            self.state_dot.remove_css_class(c);
        }
        self.state_dot.add_css_class(if named && tunnel.healthy { "teal" } else if named { "ouro" } else { "mudo" });
        self.program.set_label(&match (d["program"]["found_and_trusted"].as_bool(), d["program"]["version"].as_str()) {
            (Some(true), Some(v)) => trf("cloudflared {v} · signature of the program checked", &[("v", v.to_string())]),
            // (the service says why in English; the page says it in the person's language: not there, or not the verified one)
            (Some(false), _) if d["program"]["error"].as_str().is_some_and(|e| e.contains("SHA-256")) => tr("The signature of cloudflared does not match: the program was blocked and no tunnel opens.").to_string(),
            (Some(false), _) => tr("cloudflared is not installed: install it from the App Center (verified package) and try again.").to_string(),
            _ => String::new(),
        });
        // the four steps, with the domain and the port filled in
        while let Some(c) = self.steps.first_child() {
            self.steps.remove(&c);
        }
        let name = if host.is_empty() { "gatilhos.exemplo.com.br".to_string() } else { host };
        let port = d["port"].as_u64().map(|p| p.to_string()).unwrap_or_else(|| "47911".into());
        for (i, t) in [
            tr("In your tunnel provider's panel, create a named tunnel and copy the token (it is not shown there again).").to_string(),
            tr("Paste the token here: it goes to the system vault and never shows again.").to_string(),
            trf("Create the public name {name} pointing to http://localhost:{port}.", &[("name", name), ("port", port)]),
            tr("Come back here and press Test: Kalango calls the address itself and shows the result.").to_string(),
        ]
        .iter()
        .enumerate()
        {
            let r = caixa(Orientation::Horizontal, 10, "");
            let n = rotulo(&(i + 1).to_string(), "tunel-passo");
            n.set_valign(Align::Start);
            r.append(&n);
            r.append(&dim(t));
            self.steps.append(&r);
        }
    }
}

/// `22/11/2026` from a Unix time.
fn date(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%d/%m/%Y").ok()).map(|s| s.to_string()).unwrap_or_default()
}
