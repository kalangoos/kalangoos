//! The Terminal's preferences (design "Terminal · suspenso · ajustes"; Ctrl+, or the menu's "Preferências"): a dialog
//! over the dimmed screen, as the system's dialogs open, with its sections on the left — Geral (the shell, the lines
//! kept), Aparência (the font size, the colours following the system's theme, the cursor), Atalhos (the Terminal's own
//! keys) and Modo suspenso (the drop-down Terminal of terminal_dropdown.rs: on or off, its two shortcuts — changed here
//! through the shortcuts of the system, shortcuts.rs, so niri binds the new keys —, its height and width, the screen,
//! hiding on focus loss, staying above, opening with the session and the animation).
//!
//! Kept in the shell's settings (configuracoes.json, key "terminal"); every change applies at once to both Terminals.
//! Changing the height or the width shows the drop-down Terminal behind the dialog at that size for a moment.

use crate::conversa::{set_icon, ui_icon};
use crate::shortcuts;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The drop-down Terminal's two shortcuts (shortcuts.rs), shown and changed here.
pub const TOGGLE_ID: &str = "terminal-suspenso";
pub const EXPAND_ID: &str = "terminal-suspenso-expandir";

/// The heights the bottom edge sticks to while dragged (percent of the screen).
pub const SNAPS: [u32; 4] = [30, 45, 60, 100];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// points, of JetBrains Mono
    pub font_size: f64,
    /// the light theme makes the terminal light too (off: always dark)
    pub follow_theme: bool,
    /// "barra", "bloco" or "sublinhado"
    pub cursor: String,
    pub blink: bool,
    /// lines kept above the screen (-1: no limit)
    pub scrollback: i64,
    /// the shell of new tabs ("": the account's)
    pub shell: String,
    pub dropdown: Dropdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dropdown {
    pub enabled: bool,
    /// percent of the screen's height
    pub height: u32,
    /// false: the whole width; true: centred at `width` percent
    pub centered: bool,
    pub width: u32,
    /// an output's connector ("": the one with the pointer)
    pub output: String,
    pub hide_on_focus_loss: bool,
    pub keep_above: bool,
    pub open_with_session: bool,
    pub animation: bool,
    /// how many times the keys' hint showed (it shows the first three)
    pub hints_shown: u32,
    pub expand_hints_shown: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            font_size: 10.5,
            follow_theme: true,
            cursor: "barra".into(),
            blink: true,
            scrollback: 10_000,
            shell: String::new(),
            dropdown: Dropdown::default(),
        }
    }
}

impl Default for Dropdown {
    fn default() -> Self {
        Dropdown {
            enabled: true,
            height: 45,
            centered: false,
            width: 70,
            output: String::new(),
            hide_on_focus_loss: true,
            keep_above: true,
            open_with_session: false,
            animation: true,
            hints_shown: 0,
            expand_hints_shown: 0,
        }
    }
}

pub fn load() -> Prefs {
    serde_json::from_value(crate::configuracoes::ajustes()["terminal"].clone()).unwrap_or_default()
}

pub fn save(p: &Prefs) {
    crate::configuracoes::salvar_ajuste("terminal", serde_json::to_value(p).unwrap_or_default());
}

/// A height let go near one of [`SNAPS`] sticks to it; the rest is kept as it is (20 % to 100 %).
pub fn snap_height(percent: f64) -> u32 {
    let p = percent.clamp(20.0, 100.0);
    SNAPS.iter().copied().find(|s| (f64::from(*s) - p).abs() <= 3.0).unwrap_or(p.round() as u32)
}

/// The drop-down's height on a screen `screen` tall: `percent` of it, or all of it under the top bar (a little gap
/// at the bottom, as drawn) when expanded — never more than that, never less than its toolbar and a few rows.
pub fn panel_height(screen: i32, bar: i32, percent: u32, expanded: bool) -> i32 {
    let most = (screen - bar - 8).max(120);
    if expanded {
        return most;
    }
    ((f64::from(screen) * f64::from(percent) / 100.0).round() as i32).clamp(160.min(most), most)
}

/// The drop-down's width: None for the whole width, else `percent` of the screen (at least 480 px when it fits).
pub fn panel_width(screen: i32, centered: bool, percent: u32) -> Option<i32> {
    centered.then(|| ((f64::from(screen) * f64::from(percent.clamp(30, 100)) / 100.0).round() as i32).max(480.min(screen)).min(screen))
}

/// The keys a shortcut has now, as written on the caps and joined ("Shift+F12").
pub fn keys_text(id: &str) -> String {
    let changes = shortcuts::read_changes();
    shortcuts::SHORTCUTS.iter().find(|s| s.id == id).map(|s| shortcuts::caps_of(&shortcuts::keys_now(s, &changes)).join("+")).unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------- the dialog

struct Dialog {
    window: gtk::ApplicationWindow,
    card: Animavel,
    frame: gtk::Box,
    shell: RefCell<std::rc::Weak<Shell>>,
    section: RefCell<String>,
    /// the shortcut waiting for its new keys
    capturing: RefCell<Option<&'static str>>,
    /// (each moment of preview pushes the drop-down's hiding further)
    preview_gen: Cell<u32>,
}

thread_local! {
    static DIALOG: RefCell<Option<Rc<Dialog>>> = const { RefCell::new(None) };
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

/// An option: its name (and what it means, wrapping) and its control on the right.
fn row(title: &str, sub: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "termp-linha");
    let t = caixa(Orientation::Vertical, 2, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(title, "termp-titulo"));
    if !sub.is_empty() {
        let l = left(sub, "termp-sub");
        l.set_wrap(true);
        l.set_max_width_chars(1);
        t.append(&l);
    }
    r.append(&t);
    control.set_valign(Align::Center);
    r.append(control);
    r
}

fn divider() -> gtk::Box {
    let d = caixa(Orientation::Horizontal, 0, "termp-div");
    d.set_hexpand(true);
    d
}

fn switch(on: bool, f: impl Fn(&mut Prefs, bool) + 'static) -> gtk::Switch {
    let s = crate::configuracoes::interruptor();
    s.set_active(on);
    s.connect_active_notify(move |s| change(|p| f(p, s.is_active())));
    s
}

fn segmented(options: &[(&'static str, String)], current: &str, on_pick: impl Fn(&'static str) + 'static) -> gtk::Box {
    let seg = caixa(Orientation::Horizontal, 4, "termp-segmentado");
    let on_pick = Rc::new(on_pick);
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name) in options {
        let b = gtk::ToggleButton::with_label(name);
        b.add_css_class("termp-seg");
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(*id == current);
        let (id, f) = (*id, on_pick.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                f(id);
            }
        });
        crate::central::posicao_debug(&format!("terminal prefs {id}"), b.upcast_ref());
        seg.append(&b);
    }
    seg
}

/// A change: kept, and applied to both Terminals at once.
fn change(f: impl FnOnce(&mut Prefs)) {
    let mut p = load();
    let before = p.clone();
    f(&mut p);
    if p == before {
        return;
    }
    save(&p);
    crate::terminal::apply_prefs();
    crate::terminal_dropdown::apply_prefs();
    if debug() {
        eprintln!("debug terminal: preferências {}", serde_json::to_string(&p).unwrap_or_default());
    }
}

/// The screens to choose from: (connector, how it is named).
fn outputs() -> Vec<(String, String)> {
    let Some(d) = gdk::Display::default() else { return vec![] };
    let ms = d.monitors();
    (0..ms.n_items())
        .filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>())
        .filter_map(|m| {
            let c = m.connector()?.to_string();
            let g = m.geometry();
            let name = m.model().map(|x| x.to_string()).filter(|x| !x.is_empty()).unwrap_or_else(|| c.clone());
            Some((c.clone(), format!("{name} · {}×{}", g.width(), g.height())))
        })
        .collect()
}

/// Opens the preferences on a section ("geral", "aparencia", "atalhos", "suspenso").
pub fn open(shell: &Rc<Shell>, section: &str) {
    let d = DIALOG.with(|d| d.borrow().clone()).unwrap_or_else(|| {
        let d = build();
        DIALOG.with(|x| *x.borrow_mut() = Some(d.clone()));
        d
    });
    *d.shell.borrow_mut() = Rc::downgrade(shell);
    *d.section.borrow_mut() = section.to_string();
    d.capturing.borrow_mut().take();
    // (the drop-down goes up behind it: the dialog takes the keyboard)
    crate::terminal_dropdown::hide();
    d.fill();
    // on the screen in use, at the design's 900 × 760 when it fits
    let (w, h) = crate::superficies::tamanho_de_janela(900, 760);
    d.card.set_size_request(w, h.max(360));
    d.window.present();
    crate::superficies::take_keyboard(&d.window);
    d.window.add_css_class("aberto");
    d.card.entrar((0.0, 10.0), 0.96);
    if debug() {
        eprintln!("debug terminal: preferências abertas ({section}, {w}×{h})");
    }
}

fn build() -> Rc<Dialog> {
    let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
    let window = camada(&app, "kalango-terminal-preferencias", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    window.add_css_class("termp-fundo");
    window.set_exclusive_zone(-1);
    let frame = caixa(Orientation::Horizontal, 0, "termp-cartao");
    frame.set_overflow(gtk::Overflow::Hidden);
    let card = Animavel::new(&frame);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let d = Rc::new(Dialog {
        window,
        card,
        frame,
        shell: RefCell::default(),
        section: RefCell::new("geral".into()),
        capturing: RefCell::default(),
        preview_gen: Cell::new(0),
    });
    // the keys: a shortcut being changed takes them all; Esc closes
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let w = Rc::downgrade(&d);
    keys.connect_key_pressed(move |_, k, _, m| {
        let Some(d) = w.upgrade() else { return glib::Propagation::Proceed };
        if d.capturing.borrow().is_some() {
            d.take_keys(k, m);
            return glib::Propagation::Stop;
        }
        if k == gdk::Key::Escape {
            d.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    d.window.add_controller(keys);
    // a click on the dimmed screen closes it
    let outside = gtk::GestureClick::new();
    let w = Rc::downgrade(&d);
    outside.connect_pressed(move |g, _, x, y| {
        let Some(d) = w.upgrade() else { return };
        let inside = d.card.compute_bounds(&d.window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
        if !inside {
            g.set_state(gtk::EventSequenceState::Claimed);
            d.close();
        }
    });
    d.window.add_controller(outside);
    d
}

impl Dialog {
    fn close(self: &Rc<Self>) {
        self.stop_capturing();
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.remove_css_class("aberto");
        let w = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| !d.window.has_css_class("aberto")) {
                d.window.set_visible(false);
            }
        });
        if debug() {
            eprintln!("debug terminal: preferências fechadas");
        }
    }

    /// Everything again, as the preferences are now (a section chosen, a shortcut changed).
    fn fill(self: &Rc<Self>) {
        while let Some(c) = self.frame.first_child() {
            self.frame.remove(&c);
        }
        let (w, _) = crate::superficies::tamanho_de_janela(900, 760);
        // a narrow screen: the sections in a row above the options
        let narrow = w < 640;
        self.frame.set_orientation(if narrow { Orientation::Vertical } else { Orientation::Horizontal });
        let nav = caixa(if narrow { Orientation::Horizontal } else { Orientation::Vertical }, 4, "termp-nav");
        if !narrow {
            nav.set_size_request(200, -1);
            nav.append(&left(tr("Preferences"), "termp-nav-titulo"));
        }
        let current = self.section.borrow().clone();
        for (id, glyph, name) in [
            ("geral", "engrenagem", tr("General")),
            ("aparencia", "paleta", tr("Appearance")),
            ("atalhos", "teclado-p", tr("Shortcuts")),
            ("suspenso", "quadrado", tr("Drop-down mode")),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("termp-nav-item");
            let r = caixa(Orientation::Horizontal, 8, "");
            let on = id == current;
            let icon = ui_icon(glyph, "#8E8F99", 16);
            if on {
                set_icon(&icon, glyph, "#A5A1E2");
                b.add_css_class("ativa");
            }
            r.append(&icon);
            r.append(&left(name, "termp-nav-nome"));
            b.set_child(Some(&r));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    d.stop_capturing();
                    *d.section.borrow_mut() = id.to_string();
                    d.fill();
                }
            });
            crate::central::posicao_debug(&format!("terminal prefs secao {id}"), b.upcast_ref());
            nav.append(&b);
        }
        let nav_w: gtk::Widget = if narrow {
            let s = gtk::ScrolledWindow::new();
            s.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
            s.set_child(Some(&nav));
            s.add_css_class("termp-nav-fundo");
            s.upcast()
        } else {
            nav.add_css_class("termp-nav-fundo");
            nav.clone().upcast()
        };
        self.frame.append(&nav_w);
        let content = caixa(Orientation::Vertical, 14, "termp-conteudo");
        let p = load();
        match current.as_str() {
            "aparencia" => self.appearance(&content, &p),
            "atalhos" => self.keys(&content),
            "suspenso" => self.dropdown(&content, &p),
            _ => self.general(&content, &p),
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&content));
        self.frame.append(&scroll);
    }

    fn heading(&self, content: &gtk::Box, title: &str, control: Option<&gtk::Switch>) {
        let h = caixa(Orientation::Horizontal, 8, "");
        let t = left(title, "termp-cabecalho");
        t.set_hexpand(true);
        h.append(&t);
        if let Some(c) = control {
            h.append(c);
        }
        content.append(&h);
    }

    fn general(self: &Rc<Self>, content: &gtk::Box, p: &Prefs) {
        self.heading(content, tr("General"), None);
        // the shell of new tabs: the account's, or one of /etc/shells
        let account = crate::terminal::account_shell();
        let mut shells: Vec<String> = std::fs::read_to_string("/etc/shells")
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('/') && std::path::Path::new(l).is_file())
            .map(str::to_string)
            .collect();
        // (/bin/zsh and /usr/bin/zsh are the same program: one of each name)
        let mut seen = std::collections::BTreeSet::new();
        shells.retain(|s| seen.insert(std::path::Path::new(s).file_name().map(|n| n.to_owned()).unwrap_or_default()));
        let shell_name = std::path::Path::new(&account).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut labels = vec![trf("The account's ({shell})", &[("shell", shell_name)])];
        labels.extend(shells.iter().cloned());
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dd = gtk::DropDown::from_strings(&refs);
        dd.add_css_class("cfg-seletor");
        dd.set_size_request(240, -1);
        dd.set_selected(shells.iter().position(|s| *s == p.shell).map(|i| i as u32 + 1).unwrap_or(0));
        let list = shells.clone();
        dd.connect_selected_notify(move |d| {
            let chosen = (d.selected() as usize).checked_sub(1).and_then(|i| list.get(i).cloned()).unwrap_or_default();
            change(|p| p.shell = chosen);
        });
        content.append(&row(tr("Shell"), tr("For the tabs opened from now on"), &dd));
        // the lines kept
        let options: [(i64, String); 4] =
            [(1_000, "1.000".into()), (10_000, "10.000".into()), (100_000, "100.000".into()), (-1, tr("No limit").to_string())];
        let refs: Vec<&str> = options.iter().map(|(_, s)| s.as_str()).collect();
        let dd = gtk::DropDown::from_strings(&refs);
        dd.add_css_class("cfg-seletor");
        dd.set_size_request(240, -1);
        dd.set_selected(options.iter().position(|(n, _)| *n == p.scrollback).unwrap_or(1) as u32);
        dd.connect_selected_notify(move |d| {
            let n = options.get(d.selected() as usize).map(|(n, _)| *n).unwrap_or(10_000);
            change(|p| p.scrollback = n);
        });
        content.append(&row(tr("Lines kept"), tr("What scrolls off the top stays for scrolling back"), &dd));
    }

    fn appearance(self: &Rc<Self>, content: &gtk::Box, p: &Prefs) {
        self.heading(content, tr("Appearance"), None);
        let size = caixa(Orientation::Horizontal, 12, "");
        let s = gtk::Scale::with_range(Orientation::Horizontal, 8.0, 20.0, 0.5);
        s.add_css_class("termp-escala");
        s.set_draw_value(false);
        s.set_size_request(220, -1);
        s.set_value(p.font_size);
        let value = rotulo(&format_points(p.font_size), "termp-valor");
        value.set_size_request(48, -1);
        value.set_xalign(1.0);
        size.append(&s);
        size.append(&value);
        s.connect_value_changed(move |s| {
            let v = (s.value() * 2.0).round() / 2.0;
            value.set_text(&format_points(v));
            change(|p| p.font_size = v);
        });
        crate::central::posicao_debug("terminal prefs fonte", s.upcast_ref());
        content.append(&row(tr("Text size"), tr("Ctrl + and Ctrl − change it for a moment"), &size));
        let sw = switch(p.follow_theme, |p, v| p.follow_theme = v);
        crate::central::posicao_debug("terminal prefs tema", sw.upcast_ref());
        content.append(&row(tr("Follow the system's theme"), tr("Off: the terminal stays dark on the light theme too"), &sw));
        content.append(&divider());
        let seg = segmented(
            &[("barra", tr("Bar").to_string()), ("bloco", tr("Block").to_string()), ("sublinhado", tr("Underline").to_string())],
            &p.cursor,
            |id| change(|p| p.cursor = id.to_string()),
        );
        content.append(&row(tr("Cursor"), "", &seg));
        content.append(&row(tr("Blinking cursor"), "", &switch(p.blink, |p, v| p.blink = v)));
    }

    /// The Terminal's own keys (they work inside it; the system's are in Settings › Input › Shortcuts).
    fn keys(self: &Rc<Self>, content: &gtk::Box) {
        self.heading(content, tr("Shortcuts"), None);
        let sub = left(
            tr("Inside the Terminal. Ctrl alone belongs to the programs that run in it, so the Terminal's keys use Ctrl+Shift."),
            "termp-descricao",
        );
        sub.set_wrap(true);
        sub.set_max_width_chars(1);
        content.append(&sub);
        let open_keys =
            shortcuts::SHORTCUTS.iter().find(|s| s.id == "terminal").map(|s| shortcuts::keys_now(s, &shortcuts::read_changes())).unwrap_or_default();
        let list: Vec<(&str, Vec<String>)> = vec![
            (tr("Open the Terminal"), shortcuts::caps_of(&open_keys)),
            (tr("Show and hide the drop-down Terminal"), vec![keys_text(TOGGLE_ID)]),
            (tr("New tab"), vec!["Ctrl".into(), "Shift".into(), "T".into()]),
            (tr("Close the tab"), vec!["Ctrl".into(), "Shift".into(), "W".into()]),
            (tr("Next tab"), vec!["Ctrl".into(), "PgDn".into()]),
            (tr("Previous tab"), vec!["Ctrl".into(), "PgUp".into()]),
            (tr("Copy"), vec!["Ctrl".into(), "Shift".into(), "C".into()]),
            (tr("Paste"), vec!["Ctrl".into(), "Shift".into(), "V".into()]),
            (tr("Ask for a command"), vec!["Ctrl".into(), "Shift".into(), "K".into()]),
            (tr("Bigger text"), vec!["Ctrl".into(), "+".into()]),
            (tr("Smaller text"), vec!["Ctrl".into(), "−".into()]),
            (tr("Original text size"), vec!["Ctrl".into(), "0".into()]),
            (tr("Preferences"), vec!["Ctrl".into(), ",".into()]),
        ];
        for (name, caps) in list {
            let b = caixa(Orientation::Horizontal, 4, "");
            for c in caps.iter().flat_map(|c| {
                if c.contains('+') && c.len() > 1 {
                    c.split('+').map(str::to_string).collect::<Vec<_>>()
                } else {
                    vec![c.clone()]
                }
            }) {
                b.append(&rotulo(&c, "termp-tecla"));
            }
            content.append(&row(name, "", &b));
        }
    }

    fn dropdown(self: &Rc<Self>, content: &gtk::Box, p: &Prefs) {
        let d = &p.dropdown;
        let on = switch(d.enabled, |p, v| p.dropdown.enabled = v);
        crate::central::posicao_debug("terminal prefs suspenso ligado", on.upcast_ref());
        self.heading(content, tr("Drop-down mode"), Some(&on));
        let what = left(
            tr("The Terminal comes down from the top edge with a shortcut, over everything, and goes away when you are done."),
            "termp-descricao",
        );
        what.set_wrap(true);
        what.set_max_width_chars(1);
        content.append(&what);
        // what follows the switch (dimmed while it is off)
        let rest = caixa(Orientation::Vertical, 14, "");
        rest.set_sensitive(d.enabled);
        let rw = rest.downgrade();
        on.connect_active_notify(move |s| {
            if let Some(r) = rw.upgrade() {
                r.set_sensitive(s.is_active());
            }
        });
        // the two shortcuts
        for (id, title, sub) in [
            (TOGGLE_ID, tr("Show and hide"), tr("Works in any app; also in Settings › Keyboard and shortcuts")),
            (EXPAND_ID, tr("Expand and restore"), ""),
        ] {
            rest.append(&self.shortcut_row(id, title, sub));
        }
        rest.append(&divider());
        // the height: a slider of the whole range (0–100 %), 20 % at least
        let hb = caixa(Orientation::Horizontal, 12, "");
        let s = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
        s.add_css_class("termp-escala");
        s.set_draw_value(false);
        s.set_size_request(220, -1);
        s.set_value(f64::from(d.height));
        let value = rotulo(&format!("{}%", d.height), "termp-valor");
        value.set_size_request(40, -1);
        value.set_xalign(1.0);
        hb.append(&s);
        hb.append(&value);
        let w = Rc::downgrade(self);
        s.connect_value_changed(move |s| {
            if s.value() < 20.0 {
                s.set_value(20.0);
                return;
            }
            let v = s.value().round() as u32;
            value.set_text(&format!("{v}%"));
            change(|p| p.dropdown.height = v);
            if let Some(d) = w.upgrade() {
                d.preview();
            }
        });
        crate::central::posicao_debug("terminal prefs altura", s.upcast_ref());
        rest.append(&row(tr("Height"), tr("You can also drag the bottom edge"), &hb));
        // the width: the whole screen, or centred at a percent of it
        let wb = caixa(Orientation::Horizontal, 12, "");
        let pct = gtk::MenuButton::new();
        pct.add_css_class("termp-pct");
        pct.set_label(&format!("{}%", d.width));
        pct.set_sensitive(d.centered);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let ps = gtk::Scale::with_range(Orientation::Horizontal, 50.0, 90.0, 5.0);
        ps.add_css_class("termp-escala");
        ps.set_draw_value(false);
        ps.set_size_request(180, -1);
        ps.set_value(f64::from(d.width));
        pop.set_child(Some(&ps));
        pct.set_popover(Some(&pop));
        let (pw, w) = (pct.downgrade(), Rc::downgrade(self));
        ps.connect_value_changed(move |s| {
            let v = ((s.value() / 5.0).round() * 5.0) as u32;
            if let Some(b) = pw.upgrade() {
                b.set_label(&format!("{v}%"));
            }
            change(|p| p.dropdown.width = v);
            if let Some(d) = w.upgrade() {
                d.preview();
            }
        });
        let (pw, w) = (pct.downgrade(), Rc::downgrade(self));
        let seg = segmented(
            &[("inteira", tr("Whole screen").to_string()), ("centralizado", tr("Centred").to_string())],
            if d.centered { "centralizado" } else { "inteira" },
            move |id| {
                if let Some(b) = pw.upgrade() {
                    b.set_sensitive(id == "centralizado");
                }
                change(|p| p.dropdown.centered = id == "centralizado");
                if let Some(d) = w.upgrade() {
                    d.preview();
                }
            },
        );
        wb.append(&seg);
        wb.append(&pct);
        crate::central::posicao_debug("terminal prefs largura pct", pct.upcast_ref());
        rest.append(&row(tr("Width"), "", &wb));
        // the screen
        let outs = outputs();
        let mut labels = vec![tr("The one with the pointer").to_string()];
        labels.extend(outs.iter().map(|(_, n)| n.clone()));
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dd = gtk::DropDown::from_strings(&refs);
        dd.add_css_class("cfg-seletor");
        dd.add_css_class("termp-seletor");
        dd.set_size_request(240, -1);
        dd.set_selected(outs.iter().position(|(c, _)| *c == d.output).map(|i| i as u32 + 1).unwrap_or(0));
        dd.connect_selected_notify(move |dd| {
            let c = (dd.selected() as usize).checked_sub(1).and_then(|i| outs.get(i)).map(|(c, _)| c.clone()).unwrap_or_default();
            change(|p| p.dropdown.output = c);
        });
        rest.append(&row(tr("On which screen"), "", &dd));
        rest.append(&divider());
        let sw = switch(d.hide_on_focus_loss, |p, v| p.dropdown.hide_on_focus_loss = v);
        crate::central::posicao_debug("terminal prefs esconder", sw.upcast_ref());
        let toggle = keys_text(TOGGLE_ID);
        rest.append(&row(
            tr("Hide when it loses focus"),
            &trf("Off = it stays until you press {keys} (the pin does the same at once)", &[("keys", toggle)]),
            &sw,
        ));
        rest.append(&row(tr("Stay above the other windows"), "", &switch(d.keep_above, |p, v| p.dropdown.keep_above = v)));
        rest.append(&row(
            tr("Open with the session"),
            tr("Ready and hidden, so the first press is instant"),
            &switch(d.open_with_session, |p, v| p.dropdown.open_with_session = v),
        ));
        rest.append(&row(
            tr("Animation"),
            tr("Comes down and goes up with a spring; with the system's reduced motion, it just appears and goes"),
            &switch(d.animation, |p, v| p.dropdown.animation = v),
        ));
        content.append(&rest);
    }

    /// A shortcut's row: its keys on caps and "Mudar…" (then: press the new keys).
    fn shortcut_row(self: &Rc<Self>, id: &'static str, title: &str, sub: &str) -> gtk::Box {
        let capturing = *self.capturing.borrow() == Some(id);
        let controls = caixa(Orientation::Horizontal, 12, "");
        let caps = caixa(Orientation::Horizontal, 4, "");
        caps.set_valign(Align::Center);
        if !capturing {
            for c in keys_text(id).split('+').filter(|c| !c.is_empty()) {
                caps.append(&rotulo(c, "termp-tecla"));
            }
        }
        controls.append(&caps);
        let change_b = gtk::Button::with_label(if capturing { tr("Cancel") } else { tr("Change…") });
        change_b.add_css_class("termp-mudar");
        let w = Rc::downgrade(self);
        change_b.connect_clicked(move |_| {
            let Some(d) = w.upgrade() else { return };
            if d.capturing.borrow().is_some() {
                d.stop_capturing();
            } else {
                d.start_capturing(id);
            }
            d.fill();
        });
        crate::central::posicao_debug(&format!("terminal prefs mudar {id}"), change_b.upcast_ref());
        controls.append(&change_b);
        let text = if capturing { tr("Press the new keys… (Esc cancels, ⌫ default)") } else { sub };
        let r = row(title, text, &controls);
        if capturing {
            r.add_css_class("capturando");
        }
        r
    }

    fn start_capturing(&self, id: &'static str) {
        *self.capturing.borrow_mut() = Some(id);
        // the compositor's own binds (F12 itself) would never reach the dialog: they are held back meanwhile
        if let Some(t) = self.window.surface().and_downcast::<gdk::Toplevel>() {
            t.inhibit_system_shortcuts(None::<&gdk::Event>);
        }
        if debug() {
            eprintln!("debug terminal: mudando o atalho {id}");
        }
    }

    fn stop_capturing(&self) {
        if self.capturing.borrow_mut().take().is_some() {
            if let Some(t) = self.window.surface().and_downcast::<gdk::Toplevel>() {
                t.restore_system_shortcuts();
            }
        }
    }

    /// The keys pressed while a shortcut waits for its new ones (the rules of Settings › Input › Shortcuts).
    fn take_keys(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) {
        let Some(id) = *self.capturing.borrow() else { return };
        if key == gdk::Key::Escape {
            self.stop_capturing();
            self.fill();
            return;
        }
        if key == gdk::Key::BackSpace && mods.is_empty() {
            self.stop_capturing();
            self.set_keys(id, None);
            return;
        }
        let Some(combo) = shortcuts::from_event(key, mods) else { return };
        if !combo.contains('+') && !(combo.starts_with('F') && combo.len() <= 3) && combo != "Print" {
            self.warn(tr("Use it with Super, Ctrl or Alt."));
            return;
        }
        let changes = shortcuts::read_changes();
        if let Some(other) = shortcuts::clash(&combo, id, &changes, &shortcuts::read_custom()) {
            self.warn(&trf("{keys} is already “{name}”. Press others.", &[("keys", shortcuts::caps_of(&combo).join("+")), ("name", other)]));
            return;
        }
        self.stop_capturing();
        self.set_keys(id, Some(combo));
    }

    fn warn(&self, text: &str) {
        // the row being changed: its line under the name
        fn find(w: &gtk::Widget) -> Option<gtk::Widget> {
            if w.has_css_class("capturando") {
                return Some(w.clone());
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                if let Some(f) = find(&x) {
                    return Some(f);
                }
                c = x.next_sibling();
            }
            None
        }
        let Some(r) = find(self.frame.upcast_ref()) else { return };
        if let Some(l) = r.first_child().and_then(|t| t.last_child()).and_downcast::<gtk::Label>() {
            l.set_text(text);
            l.add_css_class("erro");
        }
        if debug() {
            eprintln!("debug terminal: atalho recusado: {text}");
        }
    }

    /// New keys (None: the default again), written through the system's shortcuts: niri binds them at once.
    fn set_keys(self: &Rc<Self>, id: &str, keys: Option<String>) {
        let mut changes = shortcuts::read_changes();
        match &keys {
            Some(k) => {
                changes.insert(id.to_string(), k.clone());
            }
            None => {
                changes.remove(id);
            }
        }
        if let Err(e) = shortcuts::save(&changes) {
            eprintln!("kalango-shell: atalhos: {e}");
        }
        if debug() {
            eprintln!("debug terminal: atalho {id} → {}", keys.as_deref().unwrap_or("padrão"));
        }
        self.fill();
    }

    /// The drop-down at the size chosen, behind the dialog, for a moment (1.5 s after the last change).
    fn preview(self: &Rc<Self>) {
        crate::terminal_dropdown::preview(true);
        let n = self.preview_gen.get() + 1;
        self.preview_gen.set(n);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if w.upgrade().is_some_and(|d| d.preview_gen.get() == n) {
                crate::terminal_dropdown::preview(false);
            }
        });
    }
}

/// "10,5 pt" (the decimal comma as written in Portuguese).
fn format_points(v: f64) -> String {
    if (v - v.round()).abs() < 0.01 {
        format!("{} pt", v.round() as i64)
    } else {
        format!("{v:.1} pt").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_stick_and_fit_the_screen() {
        assert_eq!(snap_height(46.5), 45);
        assert_eq!(snap_height(98.0), 100);
        assert_eq!(snap_height(52.0), 52);
        assert_eq!(snap_height(5.0), 20);
        // the design: 45 % of 1080 is 486; expanded, all of it under the 32 px bar but 8
        assert_eq!(panel_height(1080, 32, 45, false), 486);
        assert_eq!(panel_height(1080, 32, 45, true), 1040);
        assert_eq!(panel_height(1080, 32, 100, false), 1040);
        // a vertical screen and a tiny one
        assert_eq!(panel_height(1920, 32, 45, false), 864);
        assert_eq!(panel_height(300, 32, 20, false), 160);
    }

    #[test]
    fn widths_whole_or_centred() {
        assert_eq!(panel_width(1920, false, 70), None);
        assert_eq!(panel_width(1920, true, 70), Some(1344));
        // (at least 480 px, never more than the screen)
        assert_eq!(panel_width(600, true, 50), Some(480));
        assert_eq!(panel_width(400, true, 50), Some(400));
    }

    #[test]
    fn old_or_missing_settings_fall_back_to_the_defaults() {
        let p: Prefs = serde_json::from_value(serde_json::json!({"font_size": 12.0, "dropdown": {"height": 60}})).unwrap();
        assert_eq!(p.font_size, 12.0);
        assert_eq!(p.dropdown.height, 60);
        assert!(p.dropdown.enabled && p.dropdown.hide_on_focus_loss && !p.dropdown.open_with_session);
        assert_eq!(p.scrollback, 10_000);
        let none: Prefs = serde_json::from_value(serde_json::Value::Null).unwrap_or_default();
        assert_eq!(none, Prefs::default());
    }
}
