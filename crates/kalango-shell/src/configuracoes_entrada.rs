//! Settings › Input (designs "Configurações · Teclado", "Configurações · Teclado e atalhos", "Configurações · Mouse e
//! touchpad"): three tabs in the section's frame.
//!
//! - Keyboard: the layouts (any of xkeyboard-config's, the one in use from niri, switched at once), the keys that
//!   switch them, Compose, Caps Lock and Num Lock, the repeat's speed and delay with a field to try them; the panel
//!   makes a shortcut by request (the local model writes it, the person sees it before it is kept) and says what Super
//!   does alone.
//! - Shortcuts: every shortcut of the system, searchable; a click takes the new keys (Esc cancels, Backspace goes back
//!   to the default), a clash is said before anything changes; the person's own shortcuts can be removed.
//! - Mouse and touchpad: speed, acceleration, the main button, scrolling; the touchpad's tap, natural scrolling and
//!   "off while typing"; the panel shows the gestures the compositor does.
//!
//! Everything is written as niri's own configuration (input_settings.rs, shortcuts.rs) and takes effect at once.

use crate::configuracoes::{deslizador, depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::input_settings::{self as input, CatalogEntry, Layout, CAPS, COMPOSE, SWITCH_KEYS};
use crate::shortcuts::{self, Custom, Group, SHORTCUTS};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

fn left(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

/// A row of a card: the text (and its line under it) on the left, the control on the right.
fn row(title: &str, sub: Option<&str>, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(title, "cfg-texto-13"));
    if let Some(s) = sub {
        t.append(&left(s, "cfg-sub"));
        r.add_css_class("alta");
    }
    r.append(&t);
    control.set_valign(Align::Center);
    r.append(control);
    r
}

fn selector(items: &[&str], w: i32) -> gtk::DropDown {
    let d = gtk::DropDown::new(Some(gtk::StringList::new(items)), None::<gtk::Expression>);
    d.add_css_class("cfg-seletor");
    d.set_size_request(w, 34);
    d.set_valign(Align::Center);
    d
}

/// Two cards side by side that stack when the page is narrow (a small screen with the panel open).
fn duo() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.add_css_class("cfg-duo");
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_min_children_per_line(1);
    f.set_max_children_per_line(2);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f.set_valign(Align::Start);
    f
}

/// A card into a [`duo`] (not a stop for the keyboard of its own).
fn duo_add(f: &gtk::FlowBox, card: &impl IsA<gtk::Widget>) {
    let c = gtk::FlowBoxChild::new();
    c.set_child(Some(card));
    c.set_focusable(false);
    f.append(&c);
}

fn slider(w: i32) -> gtk::Scale {
    let s = deslizador();
    s.set_hexpand(false);
    s.set_size_request(w, -1);
    s
}

/// The key caps of a combination ("Super" "Espaço").
fn caps(combo: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 4, "");
    b.set_valign(Align::Center);
    for k in shortcuts::caps_of(combo) {
        b.append(&rotulo(&k, "cfg-tecla"));
    }
    b
}

fn cap_text(t: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 4, "");
    b.set_valign(Align::Center);
    b.append(&rotulo(t, "cfg-tecla"));
    b
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// The input devices' names (a mouse, a touchpad), from the kernel's list.
fn devices() -> (Option<String>, Option<String>) {
    let txt = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    let (mut mouse, mut touchpad, mut any_mouse) = (None, None, None);
    for block in txt.split("\n\n") {
        let name = block.lines().find_map(|l| l.strip_prefix("N: Name=")).map(|n| n.trim_matches('"').to_string());
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers=")).unwrap_or("");
        let Some(name) = name else { continue };
        let low = name.to_lowercase();
        if low.contains("touchpad") || low.contains("trackpad") {
            touchpad.get_or_insert(name);
        } else if handlers.contains("mouse") && !low.contains("virtual") {
            if low.contains("keyboard") {
                any_mouse.get_or_insert(name);
            } else {
                mouse.get_or_insert(name);
            }
        }
    }
    (mouse.or(any_mouse), touchpad)
}

/// The "Criar atalho por pedido" card (both keyboard panels show it).
struct ByRequest {
    card: gtk::Box,
    entry: gtk::Entry,
    note: gtk::Label,
    create: gtk::Button,
    preview: gtk::Box,
}

pub struct InputPage {
    pub frame: Rc<Moldura>,
    settings: Weak<crate::configuracoes::Configuracoes>,
    loading: Cell<bool>,
    catalog: RefCell<Option<Rc<Vec<CatalogEntry>>>>,
    body: gtk::Stack,
    panel: gtk::Stack,
    tab: RefCell<String>,
    // keyboard
    layouts: gtk::Box,
    switch_b: gtk::MenuButton,
    switch_list: gtk::Box,
    add_list: gtk::Box,
    add_search: gtk::Entry,
    compose: gtk::DropDown,
    caps_lock: gtk::DropDown,
    numlock: gtk::Switch,
    rate: gtk::Scale,
    delay: gtk::Scale,
    super_tap: gtk::DropDown,
    super_hold: gtk::DropDown,
    requests: Vec<ByRequest>,
    // the shortcuts tab's panel: the layouts in short and typing
    panel_layouts: gtk::Box,
    panel_rate: gtk::Scale,
    panel_compose: gtk::DropDown,
    // shortcuts
    search: gtk::Entry,
    list: gtk::Box,
    capturing: RefCell<Option<String>>,
    // mouse and touchpad
    mouse_speed: gtk::Scale,
    accel: gtk::Switch,
    left_b: gtk::ToggleButton,
    right_b: gtk::ToggleButton,
    scroll: gtk::Scale,
    smooth: gtk::Switch,
    tp_speed: gtk::Scale,
    tap: gtk::Switch,
    natural: gtk::Switch,
    dwt: gtk::Switch,
    touchpad_card: gtk::Box,
    gesture: gtk::DrawingArea,
    gesture_caption: gtk::Label,
    gesture_kind: Cell<u8>,
    save_timer: RefCell<Option<glib::SourceId>>,
}

/// The rate (repeats per second) and delay (ms) on 0…1 sliders; the scroll factor on a log scale (½ → 1×).
fn rate_to(x: f64) -> u32 {
    (10.0 + x * 50.0).round() as u32
}
fn rate_from(r: u32) -> f64 {
    ((r as f64 - 10.0) / 50.0).clamp(0.0, 1.0)
}
fn delay_to(x: f64) -> u32 {
    (150.0 + x * 850.0).round() as u32
}
fn delay_from(d: u32) -> f64 {
    ((d as f64 - 150.0) / 850.0).clamp(0.0, 1.0)
}
fn scroll_to(x: f64) -> f64 {
    4f64.powf(2.0 * x - 1.0)
}
fn scroll_from(f: f64) -> f64 {
    ((f.max(0.01).ln() / 4f64.ln() + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// The gestures the compositor does (niri: three fingers scroll the windows and switch spaces, four open the overview).
const GESTURES: &[(&str, &str, u8)] = &[
    (kalango_i18n::n_("3 fingers up or down"), kalango_i18n::n_("Switch space"), 0),
    (kalango_i18n::n_("3 fingers sideways"), kalango_i18n::n_("Scroll through the windows"), 1),
    (kalango_i18n::n_("4 fingers up"), kalango_i18n::n_("Overview"), 2),
    (kalango_i18n::n_("Pinch with 2 fingers"), kalango_i18n::n_("Zoom, in the app"), 3),
];

impl InputPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<InputPage> {
        let m = Moldura::new(tr("Input"), &[("teclado", kalango_i18n::n_("Keyboard")), ("atalhos", kalango_i18n::n_("Shortcuts")), ("mouse", kalango_i18n::n_("Mouse and touchpad"))], tr("Keyboard"));
        let body = gtk::Stack::new();
        body.set_vhomogeneous(false);
        body.set_transition_type(gtk::StackTransitionType::Crossfade);
        body.set_transition_duration(160);
        body.set_vexpand(true);
        m.corpo.append(&body);
        let panel = gtk::Stack::new();
        panel.set_vhomogeneous(false);
        // each tab's panel measured on its own (the panel keeps its width; a hidden one does not widen it)
        panel.set_hhomogeneous(false);
        panel.set_transition_type(gtk::StackTransitionType::Crossfade);
        panel.set_transition_duration(160);
        m.painel.append(&panel);

        // ---------------- Keyboard
        let kb = caixa(Orientation::Vertical, 12, "");
        let lay_card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        lay_card.append(&rotulo_cartao("teclado-d", tr("KEYBOARD LAYOUTS")));
        let layouts = caixa(Orientation::Vertical, 2, "");
        lay_card.append(&layouts);
        // the keys that switch layouts: their caps, a click lists the others
        let switch_b = gtk::MenuButton::new();
        switch_b.add_css_class("cfg-teclas-botao");
        let sp = gtk::Popover::new();
        sp.add_css_class("arq-menu");
        sp.set_has_arrow(false);
        let switch_list = caixa(Orientation::Vertical, 2, "");
        switch_list.set_size_request(200, -1);
        sp.set_child(Some(&switch_list));
        switch_b.set_popover(Some(&sp));
        let switch_row = row(tr("Switch layout"), None, &switch_b);
        lay_card.append(&switch_row);
        // add one: any layout or variant, searched by its name
        let add_b = gtk::MenuButton::new();
        add_b.add_css_class("cfg-link-acao");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.append(&ui_icon("mais", "#B9BAC4", 14));
        ac.append(&rotulo(tr("Add layout"), ""));
        add_b.set_child(Some(&ac));
        add_b.set_halign(Align::Start);
        let ap = gtk::Popover::new();
        ap.add_css_class("arq-menu");
        ap.set_has_arrow(false);
        let apc = caixa(Orientation::Vertical, 6, "");
        let add_search = gtk::Entry::new();
        add_search.add_css_class("cfg-campo");
        add_search.set_placeholder_text(Some(tr("Search a layout or language…")));
        apc.append(&add_search);
        let add_list = caixa(Orientation::Vertical, 2, "");
        let aps = gtk::ScrolledWindow::new();
        aps.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        aps.set_size_request(340, 320);
        aps.set_child(Some(&add_list));
        apc.append(&aps);
        ap.set_child(Some(&apc));
        add_b.set_popover(Some(&ap));
        add_b.set_margin_top(4);
        lay_card.append(&add_b);
        kb.append(&lay_card);
        let duo = duo();
        let keys_card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        keys_card.set_valign(Align::Start);
        keys_card.append(&rotulo_cartao("teclado-d", tr("KEYS")));
        let compose = selector(&COMPOSE.iter().map(|(_, _, n)| tr(n)).collect::<Vec<_>>(), 150);
        keys_card.append(&row(tr("Compose key"), None, &compose));
        let caps_lock = selector(&CAPS.iter().map(|(_, _, n)| tr(n)).collect::<Vec<_>>(), 150);
        keys_card.append(&row(tr("Caps Lock"), None, &caps_lock));
        let numlock = interruptor();
        keys_card.append(&row(tr("Num Lock on at start"), Some(tr("the number pad starts on")), &numlock));
        duo_add(&duo, &keys_card);
        let typing = caixa(Orientation::Vertical, 2, "cfg-cartao");
        typing.set_valign(Align::Start);
        typing.append(&rotulo_cartao("ajustes", tr("TYPING")));
        let rate = slider(200);
        typing.append(&row(tr("Repeat speed"), None, &rate));
        let delay = slider(200);
        typing.append(&row(tr("Delay before repeating"), None, &delay));
        let test = caixa(Orientation::Horizontal, 8, "cfg-campo-caixa");
        test.append(&ui_icon("teclado-d", "#8E8F99", 16));
        let te = gtk::Entry::new();
        te.add_css_class("cfg-campo-livre");
        te.set_placeholder_text(Some(tr("Type here to try the repeat and the accents…")));
        te.set_hexpand(true);
        test.append(&te);
        test.set_margin_top(6);
        typing.append(&test);
        duo_add(&duo, &typing);
        kb.append(&duo);
        body.add_named(&kb, Some("teclado"));
        // its panel: a shortcut by request; what Super does
        let kp = caixa(Orientation::Vertical, 16, "");
        let r1 = Self::by_request_card();
        kp.append(&r1.card);
        let sup = caixa(Orientation::Vertical, 2, "cfg-cartao");
        sup.append(&rotulo_cartao("teclado-d", tr("SUPER KEY")));
        let super_tap = selector(&[tr("Universal search"), tr("Nothing")], 150);
        sup.append(&row(tr("Tapped alone"), None, &super_tap));
        let super_hold = selector(&[tr("Voice mode"), tr("Nothing")], 150);
        sup.append(&row(tr("Held"), None, &super_hold));
        let sn = left(tr("With another key, Super does the shortcuts of the Shortcuts tab."), "cfg-sub");
        sn.set_wrap(true);
        sn.set_max_width_chars(52);
        sn.set_margin_start(10);
        sup.append(&sn);
        kp.append(&sup);
        panel.add_named(&kp, Some("teclado"));

        // ---------------- Shortcuts
        let sc = caixa(Orientation::Vertical, 12, "");
        let sb = caixa(Orientation::Horizontal, 8, "cfg-campo-caixa");
        sb.append(&ui_icon("busca", "#8E8F99", 16));
        let search = gtk::Entry::new();
        search.add_css_class("cfg-campo-livre");
        search.set_placeholder_text(Some(tr("Search a shortcut or action…")));
        search.set_hexpand(true);
        sb.append(&search);
        sc.append(&sb);
        let list = caixa(Orientation::Vertical, 12, "");
        let sroll = gtk::ScrolledWindow::new();
        sroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sroll.set_vexpand(true);
        sroll.set_child(Some(&list));
        sc.append(&sroll);
        body.add_named(&sc, Some("atalhos"));
        let spn = caixa(Orientation::Vertical, 16, "");
        let r2 = Self::by_request_card();
        spn.append(&r2.card);
        let pl = caixa(Orientation::Vertical, 2, "cfg-cartao");
        pl.append(&rotulo_cartao("teclado-d", tr("KEYBOARD LAYOUTS")));
        let panel_layouts = caixa(Orientation::Vertical, 2, "");
        pl.append(&panel_layouts);
        spn.append(&pl);
        let pt = caixa(Orientation::Vertical, 2, "cfg-cartao");
        pt.append(&rotulo_cartao("ajustes", tr("TYPING")));
        let panel_rate = slider(200);
        pt.append(&row(tr("Repeat speed"), None, &panel_rate));
        let panel_compose = selector(&COMPOSE.iter().map(|(_, _, n)| tr(n)).collect::<Vec<_>>(), 150);
        pt.append(&row(tr("Compose key"), None, &panel_compose));
        spn.append(&pt);
        panel.add_named(&spn, Some("atalhos"));

        // ---------------- Mouse and touchpad
        let mt = caixa(Orientation::Vertical, 12, "");
        let mc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        mc.append(&rotulo_cartao("mouse-d", tr("MOUSE")));
        let mouse_speed = slider(200);
        mc.append(&row(tr("Pointer speed"), None, &mouse_speed));
        let accel = interruptor();
        mc.append(&row(tr("Acceleration"), Some(tr("the pointer goes further when the gesture is quick")), &accel));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.add_css_class("escuro");
        seg.set_homogeneous(true);
        let left_b = gtk::ToggleButton::with_label(tr("Left"));
        left_b.add_css_class("cfg-seg");
        let right_b = gtk::ToggleButton::with_label(tr("Right"));
        right_b.add_css_class("cfg-seg");
        right_b.set_group(Some(&left_b));
        seg.append(&left_b);
        seg.append(&right_b);
        mc.append(&row(tr("Main button"), None, &seg));
        let scroll = slider(200);
        mc.append(&row(tr("Scroll speed"), None, &scroll));
        let smooth = interruptor();
        mc.append(&row(tr("Smooth scrolling"), Some(tr("with inertia and a spring at the end of lists")), &smooth));
        mt.append(&mc);
        let touchpad_card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        touchpad_card.append(&rotulo_cartao("celular-d", tr("TOUCHPAD")));
        let tp_speed = slider(200);
        touchpad_card.append(&row(tr("Speed"), None, &tp_speed));
        let tap = interruptor();
        touchpad_card.append(&row(tr("Tap to click"), None, &tap));
        let natural = interruptor();
        touchpad_card.append(&row(tr("Natural scrolling"), Some(tr("the content follows the fingers")), &natural));
        let dwt = interruptor();
        touchpad_card.append(&row(tr("Off while typing"), None, &dwt));
        mt.append(&touchpad_card);
        let mroll = gtk::ScrolledWindow::new();
        mroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        mroll.set_vexpand(true);
        mroll.set_child(Some(&mt));
        body.add_named(&mroll, Some("mouse"));
        // its panel: the gestures (the one hovered drawn above)
        let gp = caixa(Orientation::Vertical, 8, "");
        gp.append(&rotulo_cartao("mouse-d", tr("GESTURES")));
        let gbox = caixa(Orientation::Vertical, 6, "cfg-gesto-palco");
        let gesture = gtk::DrawingArea::new();
        gesture.set_content_width(220);
        gesture.set_content_height(134);
        gesture.set_halign(Align::Center);
        gbox.append(&gesture);
        let gesture_caption = rotulo("", "cfg-sub");
        gbox.append(&gesture_caption);
        gp.append(&gbox);
        let grows = caixa(Orientation::Vertical, 2, "");
        gp.append(&grows);
        let gn = left(tr("These are the compositor's own gestures; apps get pinches and two-finger scrolling."), "cfg-sub");
        gn.set_wrap(true);
        gn.set_max_width_chars(52);
        gn.set_margin_top(6);
        gp.append(&gn);
        panel.add_named(&gp, Some("mouse"));

        let page = Rc::new(InputPage {
            frame: m, settings: Rc::downgrade(c), loading: Cell::new(false), catalog: RefCell::default(), body, panel, tab: RefCell::new("teclado".into()),
            layouts, switch_b, switch_list, add_list, add_search, compose, caps_lock, numlock, rate, delay, super_tap, super_hold, requests: vec![r1, r2],
            panel_layouts, panel_rate, panel_compose, search, list, capturing: RefCell::default(),
            mouse_speed, accel, left_b, right_b, scroll, smooth, tp_speed, tap, natural, dwt, touchpad_card, gesture, gesture_caption, gesture_kind: Cell::new(0),
            save_timer: RefCell::default(),
        });
        // the gestures' rows: hovering one draws it
        for (k, (g, a, kind)) in GESTURES.iter().enumerate() {
            let r = caixa(Orientation::Horizontal, 12, "cfg-gesto-linha");
            if k == 0 {
                r.add_css_class("ativa");
            }
            let t = left(tr(g), "cfg-texto-13");
            t.set_hexpand(true);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_tooltip_text(Some(tr(g)));
            r.append(&t);
            let v = caixa(Orientation::Horizontal, 8, "cfg-gesto-valor");
            v.set_size_request(200, 34);
            v.set_hexpand(false);
            let vl = left(tr(a), "cfg-texto-13");
            vl.set_hexpand(true);
            vl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            vl.set_tooltip_text(Some(tr(a)));
            v.append(&vl);
            r.append(&v);
            let hover = gtk::EventControllerMotion::new();
            let (w, kind, rw) = (Rc::downgrade(&page), *kind, r.downgrade());
            hover.connect_enter(move |_, _, _| {
                if let (Some(p), Some(r)) = (w.upgrade(), rw.upgrade()) {
                    let mut c = r.parent().and_then(|p| p.first_child());
                    while let Some(x) = c {
                        x.remove_css_class("ativa");
                        c = x.next_sibling();
                    }
                    r.add_css_class("ativa");
                    p.gesture_kind.set(kind);
                    p.gesture_caption.set_text(tr(GESTURES[kind as usize].0));
                }
            });
            r.add_controller(hover);
            grows.append(&r);
        }
        page.gesture_caption.set_text(tr(GESTURES[0].0));
        page.draw_gesture();
        page.connect_signals();
        let w = Rc::downgrade(&page);
        page.frame.ao_trocar_aba(move |id| if let Some(p) = w.upgrade() { p.show_tab(id) });
        page
    }

    fn by_request_card() -> ByRequest {
        let card = caixa(Orientation::Vertical, 10, "cfg-cartao");
        card.append(&rotulo_cartao("spark", tr("CREATE A SHORTCUT BY REQUEST")));
        let f = caixa(Orientation::Horizontal, 8, "cfg-campo-caixa");
        f.append(&estrela(13, "#8E8F99"));
        let entry = gtk::Entry::new();
        entry.add_css_class("cfg-campo-livre");
        entry.set_placeholder_text(Some(tr("“Super+T opens the terminal in the current project's folder”")));
        entry.set_hexpand(true);
        f.append(&entry);
        card.append(&f);
        let note = left(tr("The agent puts the shortcut together, shows what it will do and says if the keys are already in use."), "cfg-sub");
        note.set_wrap(true);
        note.set_max_width_chars(52);
        card.append(&note);
        let preview = caixa(Orientation::Vertical, 8, "cfg-atalho-previa");
        preview.set_visible(false);
        card.append(&preview);
        let create = gtk::Button::new();
        create.add_css_class("cfg-botao-destaque");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.append(&ui_icon("mais", "#1C1D24", 14));
        cc.append(&rotulo(tr("Create shortcut"), ""));
        create.set_child(Some(&cc));
        create.set_halign(Align::Start);
        card.append(&create);
        ByRequest { card, entry, note, create, preview }
    }

    fn register(&self, name: &str, w: &impl IsA<gtk::Widget>) {
        if let Some(c) = self.settings.upgrade() {
            crate::configuracoes::registrar_posicao(&c, name, w.upcast_ref());
        }
    }

    fn catalog(&self) -> Rc<Vec<CatalogEntry>> {
        if let Some(c) = self.catalog.borrow().clone() {
            return c;
        }
        let c = Rc::new(input::catalog());
        *self.catalog.borrow_mut() = Some(c.clone());
        c
    }

    /// Writes the settings a moment after the last change (a slider being dragged writes once).
    fn save_later(self: &Rc<Self>, f: impl FnOnce(&mut input::InputSettings) + 'static) {
        if self.loading.get() {
            return;
        }
        let antes = input::read();
        let mut depois = antes.clone();
        f(&mut depois);
        if depois == antes {
            return;
        }
        if let Err(e) = input::save(&depois) {
            eprintln!("kalango-shell: entrada: {e}");
        }
        let s = depois;
        if let Some(t) = self.save_timer.borrow_mut().take() {
            t.remove();
        }
        let _ = s;
        if depurando() {
            let w = Rc::downgrade(self);
            let t = glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if let Some(p) = w.upgrade() {
                    p.save_timer.borrow_mut().take();
                    let s = input::read();
                    eprintln!("debug entrada: {} layouts, opções \"{}\", repetição {}/{} ms, mouse {:.2} {}{}, rolagem {:.2}, touchpad {:.2}{}{}{}",
                        s.layouts.len(), input::xkb_options(&s), s.repeat_rate, s.repeat_delay, s.mouse_speed, if s.mouse_accel { "adaptativo" } else { "plano" },
                        if s.left_handed { " canhoto" } else { "" }, s.scroll_speed, s.touchpad_speed, if s.tap { " toque" } else { "" }, if s.natural_scroll { " natural" } else { "" },
                        if s.dwt { " dwt" } else { "" });
                }
            });
            *self.save_timer.borrow_mut() = Some(t);
        }
    }

    fn connect_signals(self: &Rc<Self>) {
        let pick = |d: &gtk::DropDown, w: Weak<InputPage>, table: &'static [(&'static str, &'static str, &'static str)], f: fn(&mut input::InputSettings, String)| {
            d.connect_selected_notify(move |d| {
                let Some(p) = w.upgrade() else { return };
                let id = table.get(d.selected() as usize).map(|(i, _, _)| i.to_string()).unwrap_or_default();
                p.save_later(move |s| f(s, id));
                p.refresh_soon();
            });
        };
        pick(&self.compose, Rc::downgrade(self), COMPOSE, |s, v| s.compose = v);
        pick(&self.panel_compose, Rc::downgrade(self), COMPOSE, |s, v| s.compose = v);
        pick(&self.caps_lock, Rc::downgrade(self), CAPS, |s, v| s.caps = v);
        let w = Rc::downgrade(self);
        self.numlock.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade() {
                p.save_later(move |s| s.numlock = on);
            }
            glib::Propagation::Proceed
        });
        for sc in [&self.rate, &self.panel_rate] {
            let w = Rc::downgrade(self);
            sc.connect_value_changed(move |sc| {
                let Some(p) = w.upgrade() else { return };
                let v = rate_to(sc.value());
                p.save_later(move |s| s.repeat_rate = v);
                // the other slider follows
                for o in [&p.rate, &p.panel_rate] {
                    if o != sc && (o.value() - sc.value()).abs() > 0.001 {
                        p.loading.set(true);
                        o.set_value(sc.value());
                        p.loading.set(false);
                    }
                }
            });
        }
        let w = Rc::downgrade(self);
        self.delay.connect_value_changed(move |sc| if let Some(p) = w.upgrade() { let v = delay_to(sc.value()); p.save_later(move |s| s.repeat_delay = v) });
        for (d, key) in [(&self.super_tap, "super_tocar"), (&self.super_hold, "super_segurar")] {
            let w = Rc::downgrade(self);
            d.connect_selected_notify(move |d| {
                if w.upgrade().is_some_and(|p| !p.loading.get()) {
                    crate::configuracoes::salvar_ajuste(key, serde_json::json!(d.selected() == 0));
                    if depurando() {
                        eprintln!("debug entrada: {key} {}", d.selected() == 0);
                    }
                }
            });
        }
        // mouse and touchpad
        let w = Rc::downgrade(self);
        self.mouse_speed.connect_value_changed(move |sc| if let Some(p) = w.upgrade() { let v = sc.value() * 2.0 - 1.0; p.save_later(move |s| s.mouse_speed = v) });
        let w = Rc::downgrade(self);
        self.tp_speed.connect_value_changed(move |sc| if let Some(p) = w.upgrade() { let v = sc.value() * 2.0 - 1.0; p.save_later(move |s| s.touchpad_speed = v) });
        let w = Rc::downgrade(self);
        self.scroll.connect_value_changed(move |sc| if let Some(p) = w.upgrade() { let v = scroll_to(sc.value()); p.save_later(move |s| s.scroll_speed = v) });
        let w = Rc::downgrade(self);
        self.right_b.connect_toggled(move |b| if let Some(p) = w.upgrade() { let on = b.is_active(); p.save_later(move |s| s.left_handed = on) });
        for (sw, f) in [(&self.accel, (|s: &mut input::InputSettings, on: bool| s.mouse_accel = on) as fn(&mut input::InputSettings, bool)),
            (&self.smooth, |s, on| s.smooth_scroll = on), (&self.tap, |s, on| s.tap = on), (&self.natural, |s, on| s.natural_scroll = on), (&self.dwt, |s, on| s.dwt = on)] {
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if let Some(p) = w.upgrade() {
                    p.save_later(move |s| f(s, on));
                }
                glib::Propagation::Proceed
            });
        }
        // the layouts: the search of "Add layout"
        let w = Rc::downgrade(self);
        self.add_search.connect_changed(move |_| if let Some(p) = w.upgrade() { p.fill_catalog() });
        if let Some(pop) = self.add_search.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
            let w = Rc::downgrade(self);
            pop.connect_show(move |_| if let Some(p) = w.upgrade() { p.add_search.set_text(""); p.fill_catalog(); p.add_search.grab_focus(); });
        }
        // shortcuts: the search, and the keys being taken (the window's keys, before the page's controls)
        let w = Rc::downgrade(self);
        self.search.connect_changed(move |_| if let Some(p) = w.upgrade() { p.fill_shortcuts() });
        let k = gtk::EventControllerKey::new();
        k.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        k.connect_key_pressed(move |_, key, _, mods| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            if p.capturing.borrow().is_none() {
                return glib::Propagation::Proceed;
            }
            p.take_keys(key, mods);
            glib::Propagation::Stop
        });
        self.frame.raiz.add_controller(k);
        // by request
        for (i, r) in self.requests.iter().enumerate() {
            let w = Rc::downgrade(self);
            r.create.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.make_by_request(i) });
            let w = Rc::downgrade(self);
            r.entry.connect_activate(move |_| if let Some(p) = w.upgrade() { p.make_by_request(i) });
        }
        self.register("cfg entrada criar", &self.requests[0].create);
        self.register("cfg entrada pedido", &self.requests[0].entry);
        self.register("cfg entrada busca", &self.search);
        self.register("cfg entrada canhoto", &self.right_b);
        self.register("cfg entrada adicionar", &self.add_search.ancestor(gtk::Popover::static_type()).and_then(|p| p.parent()).unwrap_or_else(|| self.layouts.clone().upcast()));
        self.register("cfg entrada caps", &self.caps_lock);
    }

    fn refresh_soon(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::idle_add_local_once(move || if let Some(p) = w.upgrade() { p.mostrar() });
    }

    fn show_tab(self: &Rc<Self>, id: &str) {
        *self.tab.borrow_mut() = id.to_string();
        self.capturing.borrow_mut().take();
        self.body.set_visible_child_name(id);
        self.panel.set_visible_child_name(id);
        self.frame.painel_titulo.set_text(if id == "mouse" { tr("Gestures") } else { tr("Keyboard") });
        self.subtitle();
        if id == "atalhos" {
            self.fill_shortcuts();
        }
    }

    fn subtitle(&self) {
        let s = input::read();
        let cat = self.catalog();
        let first = s.layouts.first().map(|l| input::layout_name(l, &cat)).unwrap_or_default();
        let t = match self.tab.borrow().as_str() {
            "atalhos" => format!("{first} · {}", trf("{n} shortcuts", &[("n", shortcuts::count().to_string())])),
            "mouse" => {
                let (m, t) = devices();
                match (m, t) {
                    (Some(m), Some(t)) => format!("{m} · {t}"),
                    (Some(m), None) => m,
                    (None, Some(t)) => t,
                    (None, None) => tr("No mouse or touchpad found").to_string(),
                }
            }
            _ => format!("{first} · {}", if s.layouts.len() == 1 { tr("1 layout").to_string() } else { trf("{n} layouts", &[("n", s.layouts.len().to_string())]) }),
        };
        self.frame.sub.set_text(&t);
    }

    /// The page as the settings are now.
    pub fn mostrar(self: &Rc<Self>) {
        self.loading.set(true);
        let s = input::read();
        let idx = |table: &[(&str, &str, &str)], id: &str| table.iter().position(|(i, _, _)| *i == id).unwrap_or(0) as u32;
        for (d, v) in [(&self.compose, idx(COMPOSE, &s.compose)), (&self.panel_compose, idx(COMPOSE, &s.compose)), (&self.caps_lock, idx(CAPS, &s.caps))] {
            if d.selected() != v {
                d.set_selected(v);
            }
        }
        self.numlock.set_active(s.numlock);
        self.rate.set_value(rate_from(s.repeat_rate));
        self.panel_rate.set_value(rate_from(s.repeat_rate));
        self.delay.set_value(delay_from(s.repeat_delay));
        let a = crate::configuracoes::ajustes();
        for (d, k) in [(&self.super_tap, "super_tocar"), (&self.super_hold, "super_segurar")] {
            let v = if a[k].as_bool().unwrap_or(true) { 0 } else { 1 };
            if d.selected() != v {
                d.set_selected(v);
            }
        }
        self.mouse_speed.set_value((s.mouse_speed + 1.0) / 2.0);
        self.accel.set_active(s.mouse_accel);
        if s.left_handed { self.right_b.set_active(true) } else { self.left_b.set_active(true) }
        self.scroll.set_value(scroll_from(s.scroll_speed));
        self.smooth.set_active(s.smooth_scroll);
        self.tp_speed.set_value((s.touchpad_speed + 1.0) / 2.0);
        self.tap.set_active(s.tap);
        self.natural.set_active(s.natural_scroll);
        self.dwt.set_active(s.dwt);
        self.touchpad_card.set_sensitive(true);
        self.loading.set(false);
        self.fill_layouts(&s);
        self.fill_switch(&s);
        self.subtitle();
        if self.tab.borrow().as_str() == "atalhos" {
            self.fill_shortcuts();
        }
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }

    // ---- layouts

    fn fill_layouts(self: &Rc<Self>, s: &input::InputSettings) {
        let cat = self.catalog();
        let now = input::current_layout();
        clear(&self.layouts);
        clear(&self.panel_layouts);
        for (i, l) in s.layouts.iter().enumerate() {
            for (target, full) in [(&self.layouts, true), (&self.panel_layouts, false)] {
                let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
                let t = caixa(Orientation::Vertical, 1, "");
                t.set_hexpand(true);
                t.set_valign(Align::Center);
                t.append(&left(&input::layout_name(l, &cat), "cfg-texto-13"));
                if i == 0 {
                    t.append(&left(tr("default"), "cfg-sub"));
                    r.add_css_class("alta");
                }
                r.append(&t);
                if now == Some(i) || (now.is_none() && i == 0) {
                    let ch = rotulo(tr("in use"), "cfg-chip-verde");
                    ch.set_valign(Align::Center);
                    r.append(&ch);
                }
                if full && s.layouts.len() > 1 {
                    let mb = gtk::MenuButton::new();
                    mb.add_css_class("cfg-icone-botao");
                    mb.set_child(Some(&ui_icon("chevron-n", "#7A7B88", 16)));
                    mb.set_valign(Align::Center);
                    let pop = gtk::Popover::new();
                    pop.add_css_class("arq-menu");
                    pop.set_has_arrow(false);
                    let c = caixa(Orientation::Vertical, 2, "");
                    c.set_size_request(200, -1);
                    let usar = crate::arquivos::item_menu_publico(&c, "check", "#E2E2E6", tr("Use now"));
                    let padrao = crate::arquivos::item_menu_publico(&c, "seta-cima", "#E2E2E6", tr("Make it the default"));
                    padrao.set_visible(i > 0);
                    let remover = crate::arquivos::item_menu_publico(&c, "lixeira", "#EFA2A6", tr("Remove"));
                    remover.add_css_class("perigo");
                    pop.set_child(Some(&c));
                    mb.set_popover(Some(&pop));
                    let pw = pop.downgrade();
                    let fechar = move || if let Some(p) = pw.upgrade() { p.popdown() };
                    let (w, f) = (Rc::downgrade(self), fechar.clone());
                    usar.connect_clicked(move |_| {
                        f();
                        if let Err(e) = input::use_layout(i) {
                            eprintln!("kalango-shell: layout: {e}");
                        } else if depurando() {
                            eprintln!("debug entrada: layout em uso {i}");
                        }
                        if let Some(p) = w.upgrade() {
                            let w2 = Rc::downgrade(&p);
                            glib::timeout_add_local_once(Duration::from_millis(200), move || if let Some(p) = w2.upgrade() { p.mostrar() });
                        }
                    });
                    let (w, f) = (Rc::downgrade(self), fechar.clone());
                    padrao.connect_clicked(move |_| {
                        f();
                        if let Some(p) = w.upgrade() {
                            p.save_later(move |s| {
                                let l = s.layouts.remove(i);
                                s.layouts.insert(0, l);
                            });
                            p.refresh_soon();
                        }
                    });
                    let (w, f) = (Rc::downgrade(self), fechar);
                    remover.connect_clicked(move |_| {
                        f();
                        if let Some(p) = w.upgrade() {
                            p.save_later(move |s| {
                                if s.layouts.len() > 1 {
                                    s.layouts.remove(i);
                                }
                            });
                            p.refresh_soon();
                        }
                    });
                    r.append(&mb);
                    self.register(&format!("cfg entrada layout {i}"), &mb);
                }
                target.append(&r);
            }
        }
        // the shortcuts tab's panel: the switch keys, and "add" goes to the keyboard tab
        let sw = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
        sw.append(&{
            let l = left(tr("Switch layout"), "cfg-texto-13");
            l.set_hexpand(true);
            l
        });
        sw.append(&caps(SWITCH_KEYS.iter().find(|(i, _, _)| *i == s.switch_keys).map(|(_, _, k)| *k).unwrap_or("Alt+Shift")));
        self.panel_layouts.append(&sw);
        let add = gtk::Button::new();
        add.add_css_class("cfg-link-acao");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.append(&ui_icon("mais", "#B9BAC4", 14));
        ac.append(&rotulo(tr("Add layout"), ""));
        add.set_child(Some(&ac));
        add.set_halign(Align::Start);
        add.set_margin_top(4);
        let w = Rc::downgrade(self);
        add.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.frame.aba("teclado") });
        self.panel_layouts.append(&add);
    }

    fn fill_switch(self: &Rc<Self>, s: &input::InputSettings) {
        let now = SWITCH_KEYS.iter().find(|(i, _, _)| *i == s.switch_keys).map(|(_, _, k)| *k).unwrap_or("Alt+Shift");
        let c = caixa(Orientation::Horizontal, 6, "");
        c.append(&caps(now));
        c.append(&ui_icon("chevron-baixo", "#7A7B88", 12));
        self.switch_b.set_child(Some(&c));
        self.switch_b.set_sensitive(s.layouts.len() > 1);
        clear(&self.switch_list);
        let pop = self.switch_b.popover();
        for (id, _, k) in SWITCH_KEYS {
            let b = crate::arquivos::item_menu_publico(&self.switch_list, if *k == now { "check" } else { "nenhum" }, "#A5A1E2", k);
            let (w, id, pop) = (Rc::downgrade(self), id.to_string(), pop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(p) = w.upgrade() {
                    let id = id.clone();
                    p.save_later(move |s| s.switch_keys = id);
                    p.refresh_soon();
                }
            });
        }
    }

    /// "Adicionar layout": the catalog's entries whose name (or code) has what was typed; the ones already there out.
    fn fill_catalog(self: &Rc<Self>) {
        clear(&self.add_list);
        let q = self.add_search.text().trim().to_lowercase();
        let have = input::read().layouts;
        let cat = self.catalog();
        let pop = self.add_search.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>();
        let mut n = 0;
        for e in cat.iter().filter(|e| !have.contains(&e.layout)) {
            let code = if e.layout.variant.is_empty() { e.layout.layout.clone() } else { format!("{} {}", e.layout.layout, e.layout.variant) };
            if !q.is_empty() && !e.name.to_lowercase().contains(&q) && !code.contains(&q) {
                continue;
            }
            let b = gtk::Button::new();
            b.add_css_class("arq-menu-item");
            let c = caixa(Orientation::Horizontal, 8, "");
            c.set_margin_start(10);
            let t = left(&e.name, "");
            t.set_hexpand(true);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_max_width_chars(1);
            c.append(&t);
            c.append(&rotulo(&code, "arq-menu-atalho"));
            b.set_child(Some(&c));
            let (w, l, pop) = (Rc::downgrade(self), e.layout.clone(), pop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(p) = w.upgrade() {
                    let l = l.clone();
                    if depurando() {
                        eprintln!("debug entrada: layout adicionado {} {}", l.layout, l.variant);
                    }
                    p.save_later(move |s| s.layouts.push(l));
                    p.refresh_soon();
                }
            });
            self.add_list.append(&b);
            n += 1;
            if n >= 120 {
                break;
            }
        }
        if n == 0 {
            let l = left(tr("No layout with this name."), "cfg-sub");
            l.set_margin_start(10);
            self.add_list.append(&l);
        }
    }

    // ---- shortcuts

    fn fill_shortcuts(self: &Rc<Self>) {
        clear(&self.list);
        let q = self.search.text().trim().to_lowercase();
        let changes = shortcuts::read_changes();
        let custom = shortcuts::read_custom();
        let capturing = self.capturing.borrow().clone();
        let matches = |name: &str, keys: &str| q.is_empty() || name.to_lowercase().contains(&q) || shortcuts::caps_of(keys).join(" ").to_lowercase().contains(&q);
        for (group, title) in [(Group::System, tr("SYSTEM")), (Group::Windows, tr("WINDOWS AND SPACES")), (Group::Terminal, tr("TERMINAL"))] {
            let card = caixa(Orientation::Vertical, 2, "cfg-cartao");
            card.append(&left(title, "cfg-rotulo"));
            let mut any = false;
            if group == Group::System {
                for (name, glyph, keys) in [(tr("Universal search"), "busca", tr("tap Super")), (tr("Voice mode"), "microfone", tr("hold Super"))] {
                    if !matches(name, keys) {
                        continue;
                    }
                    let r = caixa(Orientation::Horizontal, 12, "cfg-atalho-linha");
                    r.add_css_class("fixa");
                    r.append(&ui_icon(glyph, "#8E8F99", 16));
                    let n = left(name, "cfg-texto-13");
                    n.set_hexpand(true);
                    r.append(&n);
                    r.append(&cap_text(keys));
                    r.set_tooltip_text(Some(tr("Super alone comes from the compositor: see the Keyboard tab.")));
                    card.append(&r);
                    any = true;
                }
            }
            for s in SHORTCUTS.iter().filter(|s| s.group == group) {
                let keys = shortcuts::keys_now(s, &changes);
                if !matches(tr(s.name), &keys) {
                    continue;
                }
                let b = gtk::Button::new();
                b.add_css_class("cfg-atalho-linha");
                let r = caixa(Orientation::Horizontal, 12, "");
                let g: gtk::Widget = if s.glyph == "spark" { estrela(13, "#8E8F99").upcast() } else { ui_icon(s.glyph, "#8E8F99", 16).upcast() };
                r.append(&g);
                let n = left(tr(s.name), "cfg-texto-13");
                n.set_hexpand(true);
                r.append(&n);
                if capturing.as_deref() == Some(s.id) {
                    b.add_css_class("capturando");
                    r.append(&rotulo(tr("Press the new keys… (Esc cancels, ⌫ default)"), "cfg-atalho-aviso"));
                } else {
                    if keys != s.default {
                        let reset = gtk::Button::new();
                        reset.add_css_class("cfg-icone-botao");
                        reset.set_child(Some(&ui_icon("desfazer", "#8E8F99", 14)));
                        reset.set_tooltip_text(Some(tr("Back to the default keys")));
                        let (w, id) = (Rc::downgrade(self), s.id);
                        reset.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.set_keys(id, None) });
                        r.append(&reset);
                    }
                    r.append(&caps(&keys));
                }
                b.set_child(Some(&r));
                let (w, id) = (Rc::downgrade(self), s.id);
                b.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        *p.capturing.borrow_mut() = Some(id.to_string());
                        p.fill_shortcuts();
                        if depurando() {
                            eprintln!("debug entrada: mudando o atalho {id}");
                        }
                    }
                });
                self.register(&format!("cfg atalho {}", s.id), &b);
                card.append(&b);
                any = true;
            }
            if any {
                self.list.append(&card);
            }
        }
        // the person's own
        let own: Vec<&Custom> = custom.iter().filter(|c| matches(&c.name, &c.keys)).collect();
        if !own.is_empty() {
            let card = caixa(Orientation::Vertical, 2, "cfg-cartao");
            card.append(&left(tr("YOUR SHORTCUTS"), "cfg-rotulo"));
            for c in own {
                let r = caixa(Orientation::Horizontal, 12, "cfg-atalho-linha");
                r.add_css_class("fixa");
                r.append(&estrela(13, "#A5A1E2"));
                let col = caixa(Orientation::Vertical, 1, "");
                col.set_hexpand(true);
                col.set_valign(Align::Center);
                col.append(&left(&c.name, "cfg-texto-13"));
                let cmd = left(&c.command, "cfg-sub");
                cmd.set_ellipsize(gtk::pango::EllipsizeMode::End);
                cmd.set_max_width_chars(1);
                col.append(&cmd);
                r.append(&col);
                r.append(&caps(&c.keys));
                let x = gtk::Button::new();
                x.add_css_class("cfg-icone-botao");
                x.set_child(Some(&ui_icon("lixeira", "#EFA2A6", 14)));
                x.set_tooltip_text(Some(tr("Remove")));
                let (w, keys) = (Rc::downgrade(self), c.keys.clone());
                x.connect_clicked(move |_| {
                    let mut all = shortcuts::read_custom();
                    all.retain(|c| c.keys != keys);
                    if let Err(e) = shortcuts::save_custom(&all) {
                        eprintln!("kalango-shell: atalhos: {e}");
                    }
                    if let Some(p) = w.upgrade() {
                        p.fill_shortcuts();
                        p.subtitle();
                    }
                });
                r.append(&x);
                card.append(&r);
            }
            self.list.append(&card);
        }
        if self.list.first_child().is_none() {
            let l = left(tr("No shortcut with this name or keys."), "cfg-sub");
            self.list.append(&l);
        }
    }

    /// The keys pressed while a shortcut waits for its new ones.
    fn take_keys(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) {
        let Some(id) = self.capturing.borrow().clone() else { return };
        if key == gdk::Key::Escape {
            self.capturing.borrow_mut().take();
            self.fill_shortcuts();
            return;
        }
        if key == gdk::Key::BackSpace && mods.is_empty() {
            self.capturing.borrow_mut().take();
            self.set_keys(&id, None);
            return;
        }
        let Some(combo) = shortcuts::from_event(key, mods) else { return };
        // a shortcut without a modifier would take the key from every app (only function and print keys may)
        if !combo.contains('+') && !(combo.starts_with('F') && combo.len() <= 3) && combo != "Print" {
            self.warn(&id, tr("Use it with Super, Ctrl or Alt."));
            return;
        }
        let changes = shortcuts::read_changes();
        if let Some(other) = shortcuts::clash(&combo, &id, &changes, &shortcuts::read_custom()) {
            self.warn(&id, &trf("{keys} is already “{name}”. Press others.", &[("keys", shortcuts::caps_of(&combo).join("+")), ("name", other)]));
            if depurando() {
                eprintln!("debug entrada: atalho {combo} já em uso");
            }
            return;
        }
        // a pair (left / right) keeps its pair: the new modifiers with the same two keys
        let s = SHORTCUTS.iter().find(|s| s.id == id);
        let combo = match s {
            Some(s) if s.default.ends_with(" Right") => {
                let mods = combo.rsplit_once('+').map(|(m, _)| m.to_string()).unwrap_or_default();
                format!("{mods}+Left Right")
            }
            _ => combo,
        };
        self.capturing.borrow_mut().take();
        self.set_keys(&id, Some(combo));
    }

    fn warn(&self, id: &str, text: &str) {
        let mut c = self.list.first_child();
        while let Some(card) = c {
            let mut r = card.first_child();
            while let Some(row) = r {
                if row.has_css_class("capturando") {
                    if let Some(l) = row.first_child().and_then(|b| b.last_child()).and_downcast::<gtk::Label>() {
                        l.set_text(text);
                        l.add_css_class("erro");
                    }
                }
                r = row.next_sibling();
            }
            c = card.next_sibling();
        }
        let _ = id;
    }

    /// A shortcut's new keys (None: its default again), written at once.
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
        if depurando() {
            eprintln!("debug entrada: atalho {id} → {}", keys.as_deref().unwrap_or("padrão"));
        }
        self.fill_shortcuts();
    }

    /// "Criar atalho": the local model writes it from the request; the person sees the keys, what it runs and whether
    /// the keys are taken, and keeps it (or not).
    fn make_by_request(self: &Rc<Self>, i: usize) {
        let r = &self.requests[i];
        let pedido = r.entry.text().trim().to_string();
        if pedido.is_empty() {
            r.entry.grab_focus();
            return;
        }
        r.create.set_sensitive(false);
        r.note.set_text(tr("The agent is putting the shortcut together…"));
        r.note.add_css_class("pulsando");
        if depurando() {
            eprintln!("debug entrada: pedido de atalho \"{pedido}\"");
        }
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("atalho").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(pedido.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()
        }, move |v| {
            let Some(p) = w.upgrade() else { return };
            let r = &p.requests[i];
            r.create.set_sensitive(true);
            r.note.remove_css_class("pulsando");
            let keys = v.as_ref().and_then(|v| v["keys"].as_str()).and_then(shortcuts::normalize);
            let name = v.as_ref().and_then(|v| v["name"].as_str()).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
            let command = v.as_ref().and_then(|v| v["command"].as_str()).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
            let (Some(keys), Some(name), Some(command)) = (keys, name, command) else {
                r.note.set_text(tr("The agent could not make a shortcut of this. Try saying the keys and what they should do."));
                if depurando() {
                    eprintln!("debug entrada: atalho por pedido falhou");
                }
                return;
            };
            if depurando() {
                eprintln!("debug entrada: atalho proposto {keys} \"{name}\" ({command})");
            }
            p.show_proposal(i, Custom { keys, name, command });
        });
    }

    fn show_proposal(self: &Rc<Self>, i: usize, c: Custom) {
        let r = &self.requests[i];
        clear(&r.preview);
        r.note.set_visible(false);
        r.create.set_visible(false);
        let top = caixa(Orientation::Horizontal, 10, "");
        top.append(&caps(&c.keys));
        let n = left(&c.name, "cfg-texto-13");
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&n);
        r.preview.append(&top);
        let cmd = left(&c.command, "cfg-codigo");
        cmd.set_wrap(true);
        cmd.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        cmd.set_max_width_chars(46);
        cmd.set_selectable(true);
        r.preview.append(&cmd);
        let clash = shortcuts::clash(&c.keys, "", &shortcuts::read_changes(), &shortcuts::read_custom());
        if let Some(o) = &clash {
            let wl = left(&trf("These keys are already “{name}”: creating it replaces that.", &[("name", o.clone())]), "cfg-sub-alerta");
            wl.set_wrap(true);
            wl.set_max_width_chars(52);
            r.preview.append(&wl);
        }
        let bs = caixa(Orientation::Horizontal, 8, "");
        let ok = gtk::Button::with_label(tr("Create"));
        ok.add_css_class("cfg-botao-destaque");
        let no = gtk::Button::with_label(tr("Cancel"));
        no.add_css_class("cfg-botao");
        bs.append(&ok);
        bs.append(&no);
        r.preview.append(&bs);
        r.preview.set_visible(true);
        self.register("cfg entrada confirmar", &ok);
        let (w, c2) = (Rc::downgrade(self), c.clone());
        ok.connect_clicked(move |_| {
            let mut all = shortcuts::read_custom();
            all.retain(|x| x.keys.to_lowercase() != c2.keys.to_lowercase());
            all.push(c2.clone());
            match shortcuts::save_custom(&all) {
                Ok(()) => {
                    if depurando() {
                        eprintln!("debug entrada: atalho criado {}", c2.keys);
                    }
                }
                Err(e) => eprintln!("kalango-shell: atalhos: {e}"),
            }
            if let Some(p) = w.upgrade() {
                p.close_proposal(i, true);
            }
        });
        let w = Rc::downgrade(self);
        no.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.close_proposal(i, false) });
    }

    fn close_proposal(self: &Rc<Self>, i: usize, made: bool) {
        let r = &self.requests[i];
        r.preview.set_visible(false);
        clear(&r.preview);
        r.note.set_visible(true);
        r.create.set_visible(true);
        r.note.set_text(if made { tr("Created: it works now (see it in the Shortcuts tab).") } else { tr("The agent puts the shortcut together, shows what it will do and says if the keys are already in use.") });
        if made {
            r.entry.set_text("");
            self.subtitle();
            if self.tab.borrow().as_str() == "atalhos" {
                self.fill_shortcuts();
            }
        }
    }

    // ---- gestures

    /// The hovered gesture, drawn: the fingers on the pad and where they go (a short loop, the system's motion).
    fn draw_gesture(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.gesture.set_draw_func(move |da, cr, width, height| {
            let Some(p) = w.upgrade() else { return };
            let kind = p.gesture_kind.get();
            let t = (da.frame_clock().map(|c| c.frame_time()).unwrap_or(0) as f64 / 1_000_000.0) % 1.6 / 1.6;
            // ease in and out, a pause at the end
            let k = if t < 0.75 { let x = t / 0.75; x * x * (3.0 - 2.0 * x) } else { 1.0 };
            let (w, h) = (width as f64, height as f64);
            let pad = (w * 0.5 - 70.0, 8.0, 140.0, h - 16.0);
            cr.set_source_rgba(0.165, 0.169, 0.208, 1.0);
            rounded(cr, pad.0, pad.1, pad.2, pad.3, 8.0);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.18, 0.184, 0.227, 1.0);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let accent = (0.647, 0.631, 0.886);
            let fingers: Vec<f64> = match kind {
                2 => vec![-27.0, -9.0, 9.0, 27.0],
                3 => vec![-14.0, 14.0],
                _ => vec![-18.0, 0.0, 18.0],
            };
            let (cx, cy) = (w * 0.5, h * 0.5);
            for (n, f) in fingers.iter().enumerate() {
                // where the finger starts and where it is now (up, sideways, or apart for a pinch)
                let (x0, y0, x, y) = match kind {
                    0 | 2 => (cx + f, cy + 30.0, cx + f, cy + 30.0 - 52.0 * k),
                    1 => (cx + f - 28.0, cy + 8.0, cx + f - 28.0 + 56.0 * k, cy + 8.0),
                    _ => {
                        let d = 10.0 + 22.0 * k;
                        let side = if n == 0 { -1.0 } else { 1.0 };
                        (cx + side * 10.0, cy + 6.0, cx + side * d, cy + 6.0)
                    }
                };
                cr.set_source_rgba(accent.0, accent.1, accent.2, 0.45);
                cr.set_line_width(3.0);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.move_to(x0, y0);
                cr.line_to(x, y);
                let _ = cr.stroke();
                cr.set_source_rgba(accent.0, accent.1, accent.2, 0.95);
                cr.arc(x, y, 7.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        });
        // the loop: redrawn while the page shows
        self.gesture.add_tick_callback(|da, _| {
            if da.is_mapped() {
                da.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

#[allow(dead_code)]
fn _unused(_: BTreeMap<String, String>, _: Layout) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_map_to_the_settings_and_back() {
        for r in [10, 25, 60] {
            assert_eq!(rate_to(rate_from(r)), r);
        }
        for d in [150, 600, 1000] {
            assert_eq!(delay_to(delay_from(d)), d);
        }
        assert!((scroll_to(0.5) - 1.0).abs() < 1e-9);
        assert!((scroll_to(scroll_from(2.0)) - 2.0).abs() < 1e-6);
    }
}
