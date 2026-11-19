//! KalangoOS's Calculator (designs "Calculadora", "· Científica", "· Conversões"; app id os.kalango.Calculadora;
//! `kalango-shell calculadora`). Three pages under the same window:
//! - **Básica / Científica:** the same display; the scientific keys add sin, cos, tan (degrees), ln, log, √, x², xʸ,
//!   π, e and parentheses.
//! - **Conversões:** a category (money, length, mass, temperature, volume, data), "De" typed with the digits and
//!   "Para" worked out at once; ⇅ swaps them; money with the day's rates (kept for when there is no internet).
//!
//! The Básica page:
//! - **Display:** the expression (mono, quiet) over the result (big).
//! - **Words:** "ou escreva", a sentence instead of keys. "15% de 1.240" is worked out here; other sentences go to
//!   the local model, which answers with an expression the calculator works out itself.
//! - **Keys:** C ± % ÷ / 7 8 9 × / 4 5 6 − / 1 2 3 + / 0 , ⌫ =, and the keyboard's.
//!
//! Motion (the design's notes): a key pressed sinks 2% with the quick spring; the result changes with a short
//! vertical slide, like a counter.

use crate::calc_engine::{convert, currency_prefix, eval, format, parse_conversion, parse_number, percent_of, units, CATEGORIES};
use std::collections::HashMap;
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::tr;
use kalango_movimento::{self as mv, Animavel, Canal};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, CString};
use std::rc::Rc;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

thread_local! {
    static CALC: RefCell<Option<Rc<Calculadora>>> = const { RefCell::new(None) };
}

/// Opens the Calculator (the same window again if it was open).
pub fn open(shell: &Rc<Shell>) {
    let c = CALC.with(|c| c.borrow().clone()).unwrap_or_else(|| {
        let app = gtk::gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let c = Calculadora::new(&app, shell);
        CALC.with(|x| *x.borrow_mut() = Some(c.clone()));
        c
    });
    c.show();
}

pub struct Calculadora {
    pub window: gtk::ApplicationWindow,
    expression: gtk::Label,
    result: gtk::Label,
    result_motion: Animavel,
    words: gtk::Entry,
    /// what is being typed: an expression in the calculator's symbols ("1.240 × 15%")
    typed: RefCell<String>,
    /// the last result shown (after =, a key continues from it)
    done: Cell<bool>,
    keys: RefCell<Vec<(String, gtk::Button)>>,
    /// "basica", "cientifica" or "conversoes"
    mode: Cell<&'static str>,
    tabs: Vec<(&'static str, gtk::ToggleButton)>,
    top: gtk::Stack,
    keypads: gtk::Stack,
    conv: Conv,
}

/// The converter's parts and state.
struct Conv {
    chips: Vec<(&'static str, gtk::ToggleButton)>,
    value: gtk::Label,
    result: gtk::Label,
    from: gtk::DropDown,
    to: gtk::DropDown,
    reference: gtk::Label,
    cat: Cell<&'static str>,
    typed: RefCell<String>,
    rates: RefCell<HashMap<String, f64>>,
    /// (filling the unit lists: their signals change nothing)
    filling: Cell<bool>,
}

const KEYS: [[&str; 4]; 5] = [["C", "±", "%", "÷"], ["7", "8", "9", "×"], ["4", "5", "6", "−"], ["1", "2", "3", "+"], ["0", ",", "⌫", "="]];
const SCI: [[&str; 5]; 6] = [["sin", "cos", "tan", "π", "C"], ["ln", "log", "√", "x²", "÷"], ["(", "7", "8", "9", "×"], [")", "4", "5", "6", "−"],
    ["xʸ", "1", "2", "3", "+"], ["e", "0", ",", "⌫", "="]];
const DIGITS: [[&str; 3]; 4] = [["7", "8", "9"], ["4", "5", "6"], ["1", "2", "3"], ["0", ",", "⌫"]];

fn key_class(k: &str) -> &'static str {
    match k {
        "=" => "calc-igual",
        "÷" | "×" | "−" | "+" => "calc-operacao",
        "C" | "±" | "%" | "⌫" | "sin" | "cos" | "tan" | "π" | "ln" | "log" | "√" | "x²" | "(" | ")" | "xʸ" | "e" => "calc-funcao",
        _ => "calc-digito",
    }
}

impl Calculadora {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Calculadora> {
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Calculator")).default_width(460).default_height(612).build();
        window.add_css_class("calc-janela");
        // the title bar: the app, its name, the menu and the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "calc-barra");
        if let Some(a) = shell.todos.iter().find(|a| a.id == "os.kalango.Calculadora") {
            bar.append(&crate::apps::imagem(a, 24));
        }
        let name = rotulo(tr("Calculator"), "calc-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        bar.append(&name);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let copy = gtk::Button::with_label(tr("Copy the result"));
        copy.add_css_class("menu-item");
        pop.set_child(Some(&copy));
        menu.set_popover(Some(&pop));
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        // (as the system's other windows: no title bar of GTK's, the header is part of the window)
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));

        let page = caixa(Orientation::Vertical, 12, "calc-pagina");
        // the pages (only "Básica" for now: the others are not drawn yet)
        let tabs = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        tabs.add_css_class("escuro");
        tabs.set_homogeneous(true);
        let mut tab_buttons: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, t) in [("basica", tr("Basic")), ("cientifica", tr("Scientific")), ("conversoes", tr("Conversions"))] {
            let b = gtk::ToggleButton::with_label(t);
            b.add_css_class("cfg-seg");
            if let Some((_, first)) = tab_buttons.first() {
                b.set_group(Some(first));
            } else {
                b.set_active(true);
            }
            tabs.append(&b);
            tab_buttons.push((id, b));
        }
        page.append(&tabs);
        let top = gtk::Stack::new();
        top.set_vhomogeneous(false);
        top.set_interpolate_size(true);
        page.append(&top);
        // the display
        let display = caixa(Orientation::Vertical, 4, "calc-visor");
        display.set_size_request(-1, 103);
        let expression = rotulo("", "calc-conta");
        expression.set_halign(Align::End);
        expression.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        expression.set_max_width_chars(40);
        let result = rotulo("0", "calc-resultado");
        result.set_halign(Align::End);
        result.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        result.set_max_width_chars(14);
        let result_motion = Animavel::new(&result);
        result_motion.set_halign(Align::End);
        result_motion.set_vexpand(true);
        result_motion.set_valign(Align::End);
        result_motion.set_overflow(gtk::Overflow::Hidden);
        display.append(&expression);
        display.append(&result_motion);
        top.add_named(&display, Some("visor"));
        // the converter (design "Calculadora · Conversões")
        let conv_box = caixa(Orientation::Vertical, 6, "");
        // (2 px apart: the six fit the window's 460 px with the design's 8 px inside each)
        let chip_row = caixa(Orientation::Horizontal, 2, "");
        let mut chips: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, name, _) in CATEGORIES {
            let b = gtk::ToggleButton::with_label(name);
            b.add_css_class("conv-chip");
            if let Some((_, first)) = chips.first() {
                b.set_group(Some(first));
            }
            chip_row.append(&b);
            chips.push((id, b));
        }
        conv_box.append(&chip_row);
        let side = |label: &str| {
            let card = caixa(Orientation::Horizontal, 8, "conv-cartao");
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            let l = rotulo(label, "conv-rotulo");
            l.set_xalign(0.0);
            col.append(&l);
            let v = rotulo("0", "conv-valor");
            v.set_xalign(0.0);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.set_width_chars(1);
            col.append(&v);
            card.append(&col);
            let dd = gtk::DropDown::from_strings(&[]);
            dd.add_css_class("conv-unidade");
            dd.set_valign(Align::Center);
            card.append(&dd);
            (card, v, dd)
        };
        let (from_card, value, from) = side(tr("From"));
        conv_box.append(&from_card);
        let swap = gtk::Button::with_label("⇅");
        swap.add_css_class("conv-trocar");
        swap.set_halign(Align::Center);
        swap.set_tooltip_text(Some(tr("Swap")));
        conv_box.append(&swap);
        let (to_card, conv_result, to) = side(tr("To"));
        conv_box.append(&to_card);
        let reference = rotulo("", "conv-ref");
        conv_box.append(&reference);
        top.add_named(&conv_box, Some("conversor"));
        // in words
        let words = gtk::Entry::new();
        words.add_css_class("calc-palavras");
        words.set_placeholder_text(Some(tr("or write: “15% of 1,240”")));
        words.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        page.append(&words);
        // the keys: one pad for each page
        let keypads = gtk::Stack::new();
        keypads.set_vexpand(true);
        // (each pad its own height: the digits' four rows of 44 px under the converter)
        keypads.set_vhomogeneous(false);
        let pad = || {
            let g = gtk::Grid::new();
            g.set_row_spacing(6);
            g.set_column_spacing(6);
            g.set_column_homogeneous(true);
            g.set_row_homogeneous(true);
            g.set_vexpand(true);
            g
        };
        let (grid, sci_grid, dig_grid) = (pad(), pad(), pad());
        keypads.add_named(&grid, Some("basica"));
        keypads.add_named(&sci_grid, Some("cientifica"));
        keypads.add_named(&dig_grid, Some("conversoes"));
        page.append(&keypads);
        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&handle);
        root.append(&page);
        window.set_child(Some(&root));

        let c = Rc::new(Calculadora {
            window, expression, result, result_motion, words, typed: RefCell::default(), done: Cell::new(false), keys: RefCell::default(),
            mode: Cell::new("basica"), tabs: tab_buttons.clone(), top, keypads,
            conv: Conv { chips, value, result: conv_result, from, to, reference, cat: Cell::new("comprimento"), typed: RefCell::new("100".into()),
                rates: RefCell::new(load_rates()), filling: Cell::new(false) },
        });
        // (page, grid, row, column, key)
        let mut all: Vec<(&'static str, gtk::Grid, usize, usize, &str)> = Vec::new();
        for (r, row) in KEYS.iter().enumerate() {
            for (col, k) in row.iter().enumerate() {
                all.push(("basica", grid.clone(), r, col, k));
            }
        }
        for (r, row) in SCI.iter().enumerate() {
            for (col, k) in row.iter().enumerate() {
                all.push(("cientifica", sci_grid.clone(), r, col, k));
            }
        }
        for (r, row) in DIGITS.iter().enumerate() {
            for (col, k) in row.iter().enumerate() {
                all.push(("conversoes", dig_grid.clone(), r, col, k));
            }
        }
        for (pagina, grid, r, col, k) in all {
            {
                let b = gtk::Button::new();
                b.add_css_class("calc-tecla");
                b.add_css_class(key_class(k));
                // (the scientific and the digits' keys are 44 px tall)
                if pagina != "basica" {
                    b.add_css_class("baixa");
                }
                let label = rotulo(k, "");
                let motion = Animavel::new(&label);
                b.set_child(Some(&motion));
                // pressed: it sinks 2% with the quick spring
                let press = gtk::GestureClick::new();
                let (m1, m2) = (motion.clone(), motion.clone());
                press.connect_pressed(move |_, _, _, _| {
                    m1.ir(Canal::Escala, 0.98, mv::RAPIDA);
                });
                press.connect_released(move |_, _, _, _| {
                    m2.ir(Canal::Escala, 1.0, mv::RAPIDA);
                });
                press.connect_stopped(move |g| {
                    if let Some(w) = g.widget() {
                        if let Some(a) = w.first_child().and_downcast::<Animavel>() {
                            a.ir(Canal::Escala, 1.0, mv::RAPIDA);
                        }
                    }
                });
                b.add_controller(press);
                let (w, key) = (Rc::downgrade(&c), k.to_string());
                b.connect_clicked(move |_| {
                    if let Some(c) = w.upgrade() {
                        c.key(&key);
                    }
                });
                grid.attach(&b, col as i32, r as i32, 1, 1);
                // (the keyboard's keys light the key of the page in front)
                c.keys.borrow_mut().push((format!("{pagina}:{k}"), b));
            }
        }
        // the keyboard's keys (before the buttons: Enter is "=", not the focused button's click)
        let kb = gtk::EventControllerKey::new();
        kb.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&c);
        kb.connect_key_pressed(move |_, k, _, _| {
            let Some(c) = w.upgrade() else { return glib::Propagation::Proceed };
            // (typing in the words field: the keys are the field's — its focus is on the text inside it)
            if gtk::prelude::GtkWindowExt::focus(&c.window).is_some_and(|f| f == c.words.clone().upcast::<gtk::Widget>() || f.is_ancestor(&c.words)) {
                return glib::Propagation::Proceed;
            }
            let key = match k {
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::equal | gdk::Key::KP_Equal => "=".to_string(),
                gdk::Key::BackSpace => "⌫".into(),
                gdk::Key::Escape | gdk::Key::Delete => "C".into(),
                gdk::Key::plus | gdk::Key::KP_Add => "+".into(),
                gdk::Key::minus | gdk::Key::KP_Subtract => "−".into(),
                gdk::Key::asterisk | gdk::Key::KP_Multiply => "×".into(),
                gdk::Key::slash | gdk::Key::KP_Divide => "÷".into(),
                gdk::Key::percent => "%".into(),
                gdk::Key::comma | gdk::Key::period | gdk::Key::KP_Decimal | gdk::Key::KP_Separator => ",".into(),
                _ => match k.to_unicode().filter(|ch| ch.is_ascii_digit()) {
                    Some(d) => d.to_string(),
                    None => return glib::Propagation::Proceed,
                },
            };
            c.key(&key);
            // (the key on screen sinks too)
            let id = format!("{}:{key}", c.mode.get());
            if let Some((_, b)) = c.keys.borrow().iter().find(|(k, _)| *k == id) {
                if let Some(a) = b.child().and_downcast::<Animavel>() {
                    a.definir(Canal::Escala, 0.98);
                    a.ir(Canal::Escala, 1.0, mv::RAPIDA);
                }
            }
            glib::Propagation::Stop
        });
        c.window.add_controller(kb);
        let w = Rc::downgrade(&c);
        c.words.connect_activate(move |e| {
            if let Some(c) = w.upgrade() {
                c.in_words(&e.text());
            }
        });
        let w = Rc::downgrade(&c);
        copy.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let _ = crate::clipboard::copy_text(&c.result.text(), false);
            }
        });
        // the app id: the dock and Alt+Tab know it as Calculadora
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = CString::new("os.kalango.Calculadora").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        c.window.connect_realize(move |j| app_id(j));
        c.window.connect_map(move |j| app_id(j));
        c.window.set_hide_on_close(true);
        for (id, b) in tab_buttons {
            let w = Rc::downgrade(&c);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(c) = w.upgrade() {
                        c.set_mode(id);
                    }
                }
            });
        }
        for (id, b) in &c.conv.chips {
            let (w, id) = (Rc::downgrade(&c), *id);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(c) = w.upgrade() {
                        c.set_category(id, None, None);
                    }
                }
            });
        }
        for dd in [&c.conv.from, &c.conv.to] {
            let w = Rc::downgrade(&c);
            dd.connect_selected_notify(move |_| {
                if let Some(c) = w.upgrade() {
                    if !c.conv.filling.get() {
                        c.update_conversion();
                    }
                }
            });
        }
        let w = Rc::downgrade(&c);
        swap.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let (f, t) = (c.conv.from.selected(), c.conv.to.selected());
                c.conv.filling.set(true);
                c.conv.from.set_selected(t);
                c.conv.to.set_selected(f);
                c.conv.filling.set(false);
                // the result becomes what is converted
                if let Some(v) = parse_number(&c.conv.result.text().replace(['R', '$', 'U', 'S', '€', '£', ' '], "")) {
                    *c.conv.typed.borrow_mut() = format(v);
                }
                c.update_conversion();
            }
        });
        if let Some((_, b)) = c.conv.chips.iter().find(|(id, _)| *id == "comprimento") {
            b.set_active(true);
        }
        c.set_category("comprimento", Some("km"), Some("mi"));
        c
    }

    /// A page: its display (or the converter), its keys and its example in words.
    fn set_mode(self: &Rc<Self>, mode: &'static str) {
        self.mode.set(mode);
        // (the tab follows, when the page changed by a sentence)
        if let Some((_, b)) = self.tabs.iter().find(|(id, _)| *id == mode) {
            if !b.is_active() {
                b.set_active(true);
            }
        }
        self.keypads.set_visible_child_name(mode);
        self.top.set_visible_child_name(if mode == "conversoes" { "conversor" } else { "visor" });
        self.words.set_placeholder_text(Some(match mode {
            "cientifica" => tr("or write: “square root of 2 times pi”"),
            "conversoes" => tr("or write: “100 km in miles”"),
            _ => tr("or write: “15% of 1,240”"),
        }));
        if mode == "conversoes" && self.conv.cat.get() == "moeda" {
            self.refresh_rates();
        }
        if debug() {
            eprintln!("debug calculadora: página {mode}");
            self.log_places();
        }
    }

    /// A category of the converter: its units in both lists (the first two, or the ones asked for).
    fn set_category(self: &Rc<Self>, cat: &'static str, from: Option<&str>, to: Option<&str>) {
        self.conv.cat.set(cat);
        let us = units(cat);
        let names: Vec<&str> = us.iter().map(|u| u.plural).collect();
        self.conv.filling.set(true);
        for dd in [&self.conv.from, &self.conv.to] {
            dd.set_model(Some(&gtk::StringList::new(&names)));
        }
        let pos = |id: Option<&str>, def: u32| id.and_then(|i| us.iter().position(|u| u.id == i)).map(|p| p as u32).unwrap_or(def);
        self.conv.from.set_selected(pos(from, 0));
        self.conv.to.set_selected(pos(to, 1.min(us.len().saturating_sub(1) as u32)));
        self.conv.filling.set(false);
        if let Some((_, b)) = self.conv.chips.iter().find(|(id, _)| *id == cat) {
            if !b.is_active() {
                b.set_active(true);
            }
        }
        if cat == "moeda" {
            self.refresh_rates();
        }
        self.update_conversion();
    }

    /// "Para" from "De" now, and how much one of the first unit is.
    fn update_conversion(&self) {
        let cat = self.conv.cat.get();
        let us = units(cat);
        let (Some(f), Some(t)) = (us.get(self.conv.from.selected() as usize), us.get(self.conv.to.selected() as usize)) else { return };
        let typed = self.conv.typed.borrow().clone();
        self.conv.value.set_text(if typed.is_empty() { "0" } else { &typed });
        let v = parse_number(&typed).unwrap_or(0.0);
        let rates = self.conv.rates.borrow();
        match (convert(cat, f.id, t.id, v, &rates), convert(cat, f.id, t.id, 1.0, &rates)) {
            (Some(r), Some(one)) => {
                let r = if cat == "moeda" { (r * 100.0).round() / 100.0 } else { (r * 1e4).round() / 1e4 };
                self.conv.result.set_text(&format(r));
                let one = (one * 1e4).round() / 1e4;
                self.conv.reference.set_text(&format!("1 {} = {} {}", f.singular, format(one), if one.abs() < 2.0 { t.singular.to_string() } else { t.singular_or_plural() }));
                if debug() {
                    eprintln!("debug calculadora: {} {} = {} {}", typed, f.id, format(r), t.id);
                }
            }
            _ => {
                self.conv.result.set_text("—");
                self.conv.reference.set_text(tr("No exchange rate yet: connect to the internet once"));
            }
        }
    }

    /// The day's rates (units per euro), fetched once a day and kept for when there is no internet.
    fn refresh_rates(self: &Rc<Self>) {
        let today = glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default();
        if rates_file_date().as_deref() == Some(today.as_str()) && !self.conv.rates.borrow().is_empty() {
            return;
        }
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || fetch_rates().ok(), move |r: Option<HashMap<String, f64>>| {
            let Some(c) = w.upgrade() else { return };
            if let Some(r) = r {
                *c.conv.rates.borrow_mut() = r;
                if debug() {
                    eprintln!("debug calculadora: cotações de hoje ({} moedas)", c.conv.rates.borrow().len());
                }
            }
            c.update_conversion();
        });
    }

    /// A key of the converter's pad: the number of "De".
    fn conv_key(&self, k: &str) {
        let mut t = self.conv.typed.borrow().clone();
        match k {
            "⌫" => {
                t.pop();
            }
            "C" => t.clear(),
            "," => {
                if !t.contains(',') {
                    t.push_str(if t.is_empty() { "0," } else { "," });
                }
            }
            d if d.chars().all(|c| c.is_ascii_digit()) => {
                if t == "0" {
                    t.clear();
                }
                let raw = format!("{}{d}", t.replace('.', ""));
                t = match raw.split_once(',') {
                    Some((int, frac)) => format!("{},{frac}", group(int)),
                    None => group(&raw),
                };
            }
            _ => return,
        }
        *self.conv.typed.borrow_mut() = t;
        self.update_conversion();
    }

    pub fn show(&self) {
        self.window.present();
        if debug() {
            eprintln!("debug calculadora: aberta");
            self.log_places();
        }
    }

    /// Debug: where the tabs, the words field, the keys and the swap button are in the window (the session test clicks
    /// them), once laid out.
    fn log_places(&self) {
        let w = self.window.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
            let Some(win) = w.upgrade() else { return };
            let mut stack = vec![win.clone().upcast::<gtk::Widget>()];
            while let Some(x) = stack.pop() {
                if !x.is_mapped() {
                    continue;
                }
                let nome = if x.has_css_class("cfg-seg") {
                    x.downcast_ref::<gtk::ToggleButton>().and_then(|b| b.label()).map(|l| format!("aba {l}"))
                } else if x.has_css_class("calc-palavras") {
                    Some("campo".to_string())
                } else if x.has_css_class("conv-trocar") {
                    Some("trocar".to_string())
                } else if x.has_css_class("calc-tecla") {
                    x.first_child().and_then(|a| a.first_child()).and_downcast::<gtk::Label>().map(|l| format!("tecla {}", l.text()))
                } else {
                    None
                };
                if let (Some(n), Some(b)) = (nome, x.compute_bounds(&win)) {
                    eprintln!("debug calculadora: {n} em {} {}", (b.x() + b.width() / 2.) as i32, (b.y() + b.height() / 2.) as i32);
                }
                let mut c = x.last_child();
                while let Some(y) = c {
                    stack.push(y.clone());
                    c = y.prev_sibling();
                }
            }
        });
    }

    /// The result shown, sliding in from below (a counter).
    fn show_result(&self, text: &str) {
        if self.result.text() == text {
            return;
        }
        self.result.set_text(text);
        self.result_motion.definir(Canal::Y, 14.0).definir(Canal::Opacidade, 0.0);
        self.result_motion.ir(Canal::Y, 0.0, mv::RAPIDA);
        self.result_motion.ir(Canal::Opacidade, 1.0, mv::RAPIDA);
    }

    /// The number being typed now (the end of the expression).
    fn last_number(s: &str) -> (usize, String) {
        let start = s.rfind(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.')).map(|i| i + s[i..].chars().next().map(char::len_utf8).unwrap_or(1)).unwrap_or(0);
        (start, s[start..].to_string())
    }

    fn key(&self, k: &str) {
        if self.mode.get() == "conversoes" {
            self.conv_key(k);
            return;
        }
        let mut t = self.typed.borrow().clone();
        let after_equal = self.done.replace(false);
        // the scientific keys put their function or constant into the expression
        let insert = match k {
            "sin" | "cos" | "tan" | "ln" | "log" => Some(format!("{k}(")),
            "√" => Some("√(".to_string()),
            "x²" => Some("²".to_string()),
            "xʸ" => Some("^".to_string()),
            "π" | "e" | "(" | ")" => Some(k.to_string()),
            _ => None,
        };
        if let Some(ins) = insert {
            if after_equal && ins.ends_with('(') {
                t = format!("{ins}{t})");
            } else if after_equal && (ins == "π" || ins == "e" || ins == "(") {
                t = ins;
            } else {
                t.push_str(&ins);
            }
            self.expression.set_text(&t);
            if let Some(v) = eval(&t) {
                self.show_result(&format(v));
            }
            *self.typed.borrow_mut() = t;
            return;
        }
        match k {
            "C" => {
                t.clear();
                self.expression.set_text("");
                self.show_result("0");
            }
            "⌫" => {
                if after_equal {
                    t.clear();
                } else {
                    t.pop();
                    while t.ends_with(' ') {
                        t.pop();
                    }
                }
            }
            "=" => {
                if let Some(v) = eval(&t) {
                    self.expression.set_text(&t);
                    let r = format(v);
                    self.show_result(&r);
                    if debug() {
                        eprintln!("debug calculadora: {t} = {r}");
                    }
                    t = r;
                    self.done.set(true);
                }
                *self.typed.borrow_mut() = t;
                return;
            }
            "±" => {
                let (i, n) = Self::last_number(&t);
                if !n.is_empty() {
                    let before = &t[..i];
                    t = if before.ends_with('-') && (before.len() == 1 || before[..before.len() - 1].ends_with(' ')) {
                        format!("{}{n}", &before[..before.len() - 1])
                    } else {
                        format!("{before}-{n}")
                    };
                }
            }
            "÷" | "×" | "−" | "+" => {
                let trimmed = t.trim_end().to_string();
                if trimmed.is_empty() {
                    return;
                }
                // a second operation replaces the first
                let base = trimmed.trim_end_matches(['÷', '×', '−', '+']).trim_end().to_string();
                t = format!("{base} {k} ");
            }
            "%" => {
                if !t.trim_end().is_empty() && !t.ends_with('%') {
                    t = format!("{}%", t.trim_end());
                }
            }
            "," => {
                if after_equal {
                    t = "0,".into();
                } else {
                    let (_, n) = Self::last_number(&t);
                    if !n.contains(',') {
                        t.push_str(if n.is_empty() { "0," } else { "," });
                    }
                }
            }
            d => {
                if after_equal {
                    t.clear();
                }
                // thousands as people write them: the number regrouped as it grows
                let (i, n) = Self::last_number(&t);
                let raw = format!("{}{d}", n.replace('.', ""));
                let grouped = match raw.split_once(',') {
                    Some((int, frac)) => format!("{},{frac}", group(int)),
                    None => group(&raw),
                };
                t = format!("{}{grouped}", &t[..i]);
            }
        }
        // the expression as it is; its value so far underneath
        self.expression.set_text(&t);
        let partial = eval(t.trim_end().trim_end_matches(['÷', '×', '−', '+']).trim_end());
        self.show_result(&partial.map(format).unwrap_or_else(|| if t.is_empty() { "0".into() } else { self.result.text().to_string() }));
        *self.typed.borrow_mut() = t;
    }

    /// A sentence: "15% de 1.240" here; anything else to the local model, which gives back an expression.
    fn in_words(self: &Rc<Self>, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some((cat, from, to, v)) = parse_conversion(&text) {
            *self.conv.typed.borrow_mut() = format(v);
            if self.mode.get() != "conversoes" {
                // (the page changes with the tab, which the converter follows)
                self.set_mode("conversoes");
            }
            self.set_category(cat, Some(from), Some(to));
            self.words.set_text("");
            if debug() {
                eprintln!("debug calculadora: «{text}» → conversão");
            }
            return;
        }
        if let Some(v) = percent_of(&text).or_else(|| eval(&text)) {
            // (worked out: the field is free for the next one)
            self.words.set_text("");
            self.expression.set_text(&text);
            self.show_result(&format!("{}{}", currency_prefix(&text), format(v)));
            *self.typed.borrow_mut() = format(v);
            self.done.set(true);
            if debug() {
                eprintln!("debug calculadora: «{text}» = {}", format(v));
            }
            return;
        }
        self.words.set_sensitive(false);
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        let t2 = text.clone();
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("conta").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(t2.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()?["expressao"].as_str().map(str::to_string)
        }, move |expr: Option<String>| {
            let Some(c) = w.upgrade() else { return };
            c.words.set_sensitive(true);
            match expr.as_deref().and_then(|e| eval(e).map(|v| (e.to_string(), v))) {
                Some((e, v)) => {
                    c.words.set_text("");
                    c.expression.set_text(&e);
                    c.show_result(&format!("{}{}", currency_prefix(&text), format(v)));
                    *c.typed.borrow_mut() = format(v);
                    c.done.set(true);
                    if debug() {
                        eprintln!("debug calculadora: «{text}» → {e} = {}", format(v));
                    }
                }
                None => {
                    c.expression.set_text(tr("I could not make a calculation of that"));
                    if debug() {
                        eprintln!("debug calculadora: «{text}» sem conta");
                    }
                }
            }
        });
    }
}

/// The rates file (units per euro) in the profile's cache, and the day it was fetched.
fn rates_path() -> std::path::PathBuf {
    glib::user_cache_dir().join("kalango/cotacoes.json")
}

fn rates_file_date() -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(rates_path()).ok()?).ok()?;
    v["buscado"].as_str().map(str::to_string)
}

fn load_rates() -> HashMap<String, f64> {
    let v: serde_json::Value = std::fs::read(rates_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    v["rates"].as_object().map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect()).unwrap_or_default()
}

/// The European Central Bank's rates of the day (frankfurter.app: only currency codes go out), kept in the cache.
fn fetch_rates() -> Result<HashMap<String, f64>, String> {
    let url = std::env::var("KALANGO_COTACOES_URL").unwrap_or_else(|_| "https://api.frankfurter.app/latest".into());
    let v: serde_json::Value = ureq::get(&url).timeout(std::time::Duration::from_secs(8)).call().map_err(|e| e.to_string())?
        .into_json().map_err(|e| e.to_string())?;
    let rates: HashMap<String, f64> = v["rates"].as_object().ok_or("sem cotações")?.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect();
    let today = glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default();
    let p = rates_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, serde_json::json!({"buscado": today, "base": "EUR", "rates": rates}).to_string());
    Ok(rates)
}

/// "1240" → "1.240".
fn group(int: &str) -> String {
    let neg = int.starts_with('-');
    let d = int.trim_start_matches('-');
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}
