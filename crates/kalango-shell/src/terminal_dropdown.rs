//! The Terminal's drop-down mode (designs "Terminal · suspenso (aberto, 45%)", "(expandido e fixado)" and
//! "Terminal · suspenso · estados"; `kalango-shell terminal-suspenso [expandir]`, F12 and Shift+F12 by default): a second
//! Terminal, with its own tabs, on a layer surface of the top layer anchored to the top edge under the top bar — the
//! whole width or centred, 45 % of the screen's height —, on the screen with the pointer (or the one chosen).
//!
//! - **Comes down and goes up:** it slides in from −100 % with the soft spring and goes up with the quick one; the
//!   keyboard is in the terminal from the first frame. Reduced motion (or the animation turned off): a 90 ms fade.
//! - **Expands** to the whole height and back (its height animates with the soft spring; the tabs lay their lines out
//!   once, at the end). The handle on the bottom edge changes the height (it sticks at 30/45/60/100 %), kept.
//! - **Hides when it loses focus**, unless pinned (the pin) or the preference is off; "stay above" off lets the other
//!   windows cover it while it is not in use.
//! - **The keys' hint** under it the first three times (inside it when expanded), gone in 2 s.

use crate::conversa::{set_icon, ui_icon};
use crate::superficies::{caixa, camada, rotulo, BARRA_ALTURA};
use crate::terminal::{Kind, Terminal};
use crate::terminal_prefs::{self, keys_text, panel_height, panel_width, EXPAND_ID, TOGGLE_ID};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal, Mola};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Hidden,
    Showing,
    Shown,
    Hiding,
}

pub struct Dropdown {
    term: Rc<Terminal>,
    slide: Animavel,
    panel: gtk::Box,
    pin: gtk::ToggleButton,
    state: Cell<State>,
    /// bumped by each show and hide: what an older one left to do at its end is dropped
    generation: Cell<u32>,
    expanded: Cell<bool>,
    height: Cell<i32>,
    width: Cell<Option<i32>>,
    /// the screen it is on: its connector and logical size
    screen: RefCell<(String, i32, i32)>,
    resize_tick: RefCell<Option<gtk::TickCallbackId>>,
    inner_hint: gtk::Box,
    inner_text: gtk::Label,
    outer_hint: RefCell<Option<(gtk::ApplicationWindow, Animavel, gtk::Label)>>,
    hint_generation: Cell<u32>,
    /// shown behind the preferences, at the size being chosen (no keyboard, no hiding on focus loss)
    previewing: Cell<bool>,
    drag_from: Cell<i32>,
    /// where the drag started, in the surface's coordinates (the handle itself moves with the edge)
    drag_start_y: Cell<f64>,
    /// when the pointer last left it (focus following the pointer moves the keyboard on leaving: not a click outside)
    left_at: Cell<Option<std::time::Instant>>,
}

thread_local! {
    static DROPDOWN: RefCell<Option<Rc<Dropdown>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<Dropdown>> {
    DROPDOWN.with(|d| d.borrow().clone())
}

/// The drop-down's Terminal, when there is one (for the preferences and the System Monitor).
pub fn terminal() -> Option<Rc<Terminal>> {
    current().map(|d| d.term.clone())
}

fn motion(p: &terminal_prefs::Prefs) -> bool {
    p.dropdown.animation && !mv::reduzido()
}

/// `kalango-shell terminal-suspenso` (shows or hides it) and `… expandir` (expands or restores it).
pub fn command(shell: &Rc<Shell>, args: &[String]) {
    if !terminal_prefs::load().dropdown.enabled {
        if debug() {
            eprintln!("debug terminal-suspenso: desligado nas preferências");
        }
        hide();
        return;
    }
    let d = get(shell);
    match args.get(1).map(String::as_str) {
        Some("expandir") => d.toggle_expanded(),
        Some("mostrar") => d.show(true),
        Some("esconder") => d.hide(),
        _ => d.toggle(),
    }
}

/// At the session's start: ready and hidden when the preferences ask for it ("Abrir junto com a sessão").
pub fn at_start(shell: &Rc<Shell>) {
    let p = terminal_prefs::load();
    if p.dropdown.enabled && p.dropdown.open_with_session {
        let d = get(shell);
        if !d.term.has_tabs() {
            d.term.open_tab();
        }
        if debug() {
            eprintln!("debug terminal-suspenso: pronto e escondido (abrir junto com a sessão)");
        }
    }
}

pub fn hide() {
    if let Some(d) = current() {
        d.hide();
    }
}

/// The preferences changed: the drop-down takes them now (its size, its screen's side, the layer).
pub fn apply_prefs() {
    let Some(d) = current() else { return };
    let p = terminal_prefs::load();
    if !p.dropdown.enabled {
        d.hide();
        return;
    }
    if matches!(d.state.get(), State::Shown | State::Showing) {
        let target = d.configure(&p);
        d.animate_height(target);
    }
}

/// The preferences' preview: the drop-down at the size being chosen, behind the dialog (`false`: it goes up again).
pub fn preview(on: bool) {
    let Some(shell) = crate::shell_atual() else { return };
    if !terminal_prefs::load().dropdown.enabled {
        return;
    }
    let d = get(&shell);
    if on {
        d.previewing.set(true);
        if matches!(d.state.get(), State::Hidden | State::Hiding) {
            d.show(false);
        } else {
            let target = d.configure(&terminal_prefs::load());
            d.animate_height(target);
        }
    } else if d.previewing.get() {
        d.hide();
    }
}

fn get(shell: &Rc<Shell>) -> Rc<Dropdown> {
    if let Some(d) = current() {
        return d;
    }
    let d = Dropdown::new(shell);
    DROPDOWN.with(|x| *x.borrow_mut() = Some(d.clone()));
    d
}

/// The screen to come down on: the chosen one if it is there, else the one with the pointer (niri's focused output
/// follows the pointer across screens), else the first.
fn target_output(chosen: &str) -> Option<(String, gdk::Monitor)> {
    let display = gdk::Display::default()?;
    let ms = display.monitors();
    let all: Vec<gdk::Monitor> = (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()).collect();
    let by = |name: &str| all.iter().find(|m| m.connector().is_some_and(|c| c == name)).cloned();
    if !chosen.is_empty() {
        if let Some(m) = by(chosen) {
            return Some((chosen.to_string(), m));
        }
    }
    let focused =
        crate::servicos::niri_pedido(serde_json::json!("FocusedOutput")).ok().and_then(|v| v["FocusedOutput"]["name"].as_str().map(str::to_string));
    if let Some(m) = focused.as_deref().and_then(by) {
        return Some((focused.unwrap_or_default(), m));
    }
    let m = all.into_iter().next()?;
    Some((m.connector().map(|c| c.to_string()).unwrap_or_default(), m))
}

/// A quick fade of a widget's opacity (reduced motion: 90 ms), then `done`. A timer, not the frame clock: a surface
/// still invisible may get no frames from the compositor.
fn fade(w: &gtk::Widget, from: f64, to: f64, done: impl FnOnce() + 'static) {
    w.set_opacity(from);
    let start = glib::monotonic_time();
    let (w, done) = (w.downgrade(), RefCell::new(Some(done)));
    glib::timeout_add_local(Duration::from_millis(12), move || {
        let Some(w) = w.upgrade() else { return glib::ControlFlow::Break };
        let t = ((glib::monotonic_time() - start) as f64 / 1000.0 / mv::DISSOLVE_MS).clamp(0.0, 1.0);
        w.set_opacity(from + (to - from) * t);
        if t >= 1.0 {
            if let Some(f) = done.borrow_mut().take() {
                f();
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/// A point of a gesture's widget in its surface's coordinates.
fn surface_y(g: &gtk::GestureDrag, x: f64, y: f64, window: &gtk::ApplicationWindow) -> f64 {
    g.widget().and_then(|w| w.compute_point(window, &gtk::graphene::Point::new(x as f32, y as f32))).map(|p| f64::from(p.y())).unwrap_or(y)
}

/// The keys' hint (design "Dica de atalho"): a keyboard and what the keys do.
fn hint_chip() -> (gtk::Box, gtk::Label) {
    let chip = caixa(Orientation::Horizontal, 8, "term-suspenso-dica");
    chip.append(&ui_icon("teclado-p", "#8E8F99", 16));
    let l = rotulo("", "term-suspenso-dica-texto");
    l.set_ellipsize(pango::EllipsizeMode::End);
    chip.append(&l);
    chip.set_can_target(false);
    (chip, l)
}

impl Dropdown {
    fn new(shell: &Rc<Shell>) -> Rc<Dropdown> {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let term = Terminal::new(&app, shell, Kind::Dropdown);
        let window = term.window.clone();
        // the toolbar's end: the pin and expand (no window controls)
        let pin = gtk::ToggleButton::new();
        pin.add_css_class("term-suspenso-botao");
        let pin_icon = ui_icon("fixar", "#8E8F99", 28);
        pin.set_child(Some(&pin_icon));
        pin.set_tooltip_text(Some(tr("Pin (it does not hide when it loses focus)")));
        let expand = gtk::Button::new();
        expand.add_css_class("term-suspenso-botao");
        expand.set_child(Some(&ui_icon("expandir-altura", "#8E8F99", 16)));
        expand.set_tooltip_text(Some(&trf("Expand and restore ({keys})", &[("keys", keys_text(EXPAND_ID))])));
        term.bar_end.append(&pin);
        term.bar_end.append(&expand);
        // the bottom edge's handle (a wider strip to grab than the line drawn)
        let grip = caixa(Orientation::Horizontal, 0, "term-suspenso-alca");
        grip.set_size_request(56, 4);
        grip.set_halign(Align::Center);
        grip.set_valign(Align::End);
        grip.set_margin_bottom(4);
        let handle = gtk::Box::new(Orientation::Vertical, 0);
        handle.set_size_request(160, 22);
        handle.set_halign(Align::Center);
        handle.set_valign(Align::End);
        handle.append(&grip);
        grip.set_vexpand(true);
        handle.set_cursor_from_name(Some("ns-resize"));
        handle.set_tooltip_text(Some(tr("Drag to change the height")));
        term.over.add_overlay(&handle);
        // the hint inside it (expanded: there is no room under it)
        let (inner_hint, inner_text) = hint_chip();
        inner_hint.set_halign(Align::Center);
        inner_hint.set_valign(Align::End);
        inner_hint.set_margin_bottom(32);
        inner_hint.set_visible(false);
        term.over.add_overlay(&inner_hint);
        let panel = caixa(Orientation::Vertical, 0, "term-suspenso");
        panel.set_overflow(gtk::Overflow::Hidden);
        panel.append(&term.over);
        term.over.set_vexpand(true);
        let slide = Animavel::new(&panel);
        window.set_child(Some(&slide));
        let d = Rc::new(Dropdown {
            term,
            slide,
            panel,
            pin: pin.clone(),
            state: Cell::new(State::Hidden),
            generation: Cell::new(0),
            expanded: Cell::new(false),
            height: Cell::new(0),
            width: Cell::new(None),
            screen: RefCell::new((String::new(), 1920, 1080)),
            resize_tick: RefCell::default(),
            inner_hint,
            inner_text,
            outer_hint: RefCell::default(),
            hint_generation: Cell::new(0),
            previewing: Cell::new(false),
            drag_from: Cell::new(0),
            drag_start_y: Cell::new(0.0),
            left_at: Cell::new(None),
        });
        pin.connect_toggled(move |b| {
            set_icon(&pin_icon, "fixar", if b.is_active() { "#A5A1E2" } else { "#8E8F99" });
            if debug() {
                eprintln!("debug terminal-suspenso: {}", if b.is_active() { "fixado" } else { "solto" });
            }
        });
        let w = Rc::downgrade(&d);
        expand.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                d.toggle_expanded();
            }
        });
        crate::central::posicao_debug_sempre("terminal suspenso alfinete", pin.upcast_ref());
        crate::central::posicao_debug_sempre("terminal suspenso expandir", expand.upcast_ref());
        crate::central::posicao_debug_sempre("terminal suspenso alca", handle.upcast_ref());
        // dragging the bottom edge: the height follows the pointer, sticks near 30/45/60/100 % when let go, and is kept
        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(&d);
        drag.connect_drag_begin(move |g, x, y| {
            let Some(d) = w.upgrade() else { return };
            d.stop_resizing();
            d.drag_from.set(d.height.get());
            d.drag_start_y.set(surface_y(g, x, y, &d.term.window));
            d.expanded.set(false);
            d.term.set_frozen(true);
        });
        let w = Rc::downgrade(&d);
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(d) = w.upgrade() else { return };
            let Some((x0, y0)) = g.start_point() else { return };
            let dy = surface_y(g, x0 + dx, y0 + dy, &d.term.window) - d.drag_start_y.get();
            let (_, _, sh) = d.screen.borrow().clone();
            let h =
                (d.drag_from.get() + dy.round() as i32).clamp(panel_height(sh, BARRA_ALTURA, 20, false), panel_height(sh, BARRA_ALTURA, 100, true));
            d.set_height(h);
        });
        let w = Rc::downgrade(&d);
        drag.connect_drag_end(move |_, _, _| {
            let Some(d) = w.upgrade() else { return };
            let (_, _, sh) = d.screen.borrow().clone();
            let percent = terminal_prefs::snap_height(f64::from(d.height.get()) * 100.0 / f64::from(sh.max(1)));
            let mut p = terminal_prefs::load();
            p.dropdown.height = percent;
            terminal_prefs::save(&p);
            d.animate_height(panel_height(sh, BARRA_ALTURA, percent, false));
            if debug() {
                eprintln!("debug terminal-suspenso: altura {percent}% (arrastada)");
            }
        });
        handle.add_controller(drag);
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(&d);
        motion.connect_leave(move |_| {
            if let Some(d) = w.upgrade() {
                d.left_at.set(Some(std::time::Instant::now()));
            }
        });
        d.term.window.add_controller(motion);
        // a click on the desktop is a click outside it
        if let Some(area) = shell.area_de_trabalho() {
            let w = Rc::downgrade(&d);
            area.on_click(move || {
                if let Some(d) = w.upgrade() {
                    d.outside_click();
                }
            });
        }
        // losing the keyboard (a click on another window): up, unless pinned or the preference says to stay
        let w = Rc::downgrade(&d);
        d.term.window.connect_is_active_notify(move |win| {
            let Some(d) = w.upgrade() else { return };
            let p = terminal_prefs::load();
            if win.is_active() {
                win.set_layer(Layer::Top);
                return;
            }
            if !matches!(d.state.get(), State::Shown | State::Showing) || d.previewing.get() {
                return;
            }
            // (focus following the pointer took it as the pointer left: no click; it stays until a click outside or the
            // key, and takes the keyboard again when the pointer comes back over it)
            let hovered_away = crate::window_prefs::focus_follows() && d.left_at.get().is_some_and(|t| t.elapsed() < Duration::from_millis(400));
            if hovered_away {
                if debug() {
                    eprintln!("debug terminal-suspenso: o foco seguiu o ponteiro para fora; fica");
                }
                return;
            }
            if d.pin.is_active() || !p.dropdown.hide_on_focus_loss {
                // (not kept above: the windows used now may cover it)
                if !p.dropdown.keep_above {
                    win.set_layer(Layer::Bottom);
                }
                if debug() {
                    eprintln!("debug terminal-suspenso: perdeu o foco e fica ({})", if d.pin.is_active() { "fixado" } else { "não esconde" });
                }
                return;
            }
            if debug() {
                eprintln!("debug terminal-suspenso: perdeu o foco e sobe");
            }
            d.hide();
        });
        d
    }

    /// A click on the desktop: as losing the focus to another window.
    fn outside_click(self: &Rc<Self>) {
        let p = terminal_prefs::load();
        if !matches!(self.state.get(), State::Shown | State::Showing)
            || self.previewing.get()
            || self.pin.is_active()
            || !p.dropdown.hide_on_focus_loss
        {
            return;
        }
        if debug() {
            eprintln!("debug terminal-suspenso: clique na área de trabalho; sobe");
        }
        self.hide();
    }

    /// Puts it on its screen at its width; the height it should have now.
    fn configure(&self, p: &terminal_prefs::Prefs) -> i32 {
        let window = &self.term.window;
        let (conn, (sw, sh)) = match target_output(&p.dropdown.output) {
            Some((c, m)) => {
                window.set_monitor(Some(&m));
                let g = m.geometry();
                (c, (g.width(), g.height()))
            }
            None => (String::new(), (1920, 1080)),
        };
        *self.screen.borrow_mut() = (conn, sw, sh);
        let width = panel_width(sw, p.dropdown.centered, p.dropdown.width);
        self.width.set(width);
        window.set_anchor(Edge::Left, width.is_none());
        window.set_anchor(Edge::Right, width.is_none());
        window.set_layer(Layer::Top);
        if width.is_some() {
            self.panel.add_css_class("centralizado");
        } else {
            self.panel.remove_css_class("centralizado");
        }
        panel_height(sh, BARRA_ALTURA, p.dropdown.height, self.expanded.get())
    }

    fn set_height(&self, h: i32) {
        self.height.set(h);
        let w = self.width.get();
        self.panel.set_size_request(w.unwrap_or(-1), h);
        self.term.window.set_default_size(w.unwrap_or(1), h);
        if let Some((win, _, _)) = self.outer_hint.borrow().as_ref() {
            win.set_margin(Edge::Top, h + 14);
        }
    }

    fn stop_resizing(&self) {
        if let Some(id) = self.resize_tick.borrow_mut().take() {
            id.remove();
        }
    }

    /// To a new height with the soft spring (the content keeps its size until the end); at once without motion.
    fn animate_height(self: &Rc<Self>, target: i32) {
        self.stop_resizing();
        let from = self.height.get();
        if !motion(&terminal_prefs::load()) || from == target || from == 0 || self.state.get() == State::Hidden {
            self.set_height(target);
            self.term.set_frozen(false);
            return;
        }
        self.term.set_frozen(true);
        let mut spring = Mola::new(0.0);
        spring.ir(1.0, mv::SUAVE);
        let spring = Cell::new(spring);
        let last = Cell::new(0i64);
        let w = Rc::downgrade(self);
        let id = self.slide.add_tick_callback(move |_, clock| {
            let Some(d) = w.upgrade() else { return glib::ControlFlow::Break };
            let t = clock.frame_time();
            let dt = if last.get() == 0 { 1.0 / 120.0 } else { ((t - last.get()) as f64 / 1e6).clamp(0.0, 0.05) };
            last.set(t);
            let mut s = spring.get();
            s.passo(dt);
            spring.set(s);
            d.set_height(from + ((target - from) as f64 * s.x).round() as i32);
            if s.parada() {
                d.set_height(target);
                d.resize_tick.borrow_mut().take();
                d.term.set_frozen(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.resize_tick.borrow_mut() = Some(id);
    }

    fn toggle(self: &Rc<Self>) {
        match self.state.get() {
            State::Shown | State::Showing => self.hide(),
            State::Hidden | State::Hiding => self.show(true),
        }
    }

    fn toggle_expanded(self: &Rc<Self>) {
        if matches!(self.state.get(), State::Hidden | State::Hiding) {
            self.expanded.set(true);
            self.show(true);
            self.expand_hint();
            return;
        }
        self.expanded.set(!self.expanded.get());
        let target = self.configure(&terminal_prefs::load());
        self.animate_height(target);
        if self.expanded.get() {
            self.hide_outer_hint();
            self.expand_hint();
        } else {
            self.inner_hint.set_visible(false);
        }
        if debug() {
            eprintln!("debug terminal-suspenso: {} ({} px)", if self.expanded.get() { "expandido" } else { "restaurado" }, target);
        }
    }

    /// Down from the top edge (`focus`: with the keyboard in the terminal; the preview takes none).
    fn show(self: &Rc<Self>, focus: bool) {
        if matches!(self.state.get(), State::Shown | State::Showing) {
            if focus && self.previewing.get() {
                // (the preview turned into a real one)
                self.previewing.set(false);
                self.term.window.set_visible(false);
                self.state.set(State::Hidden);
            } else {
                return;
            }
        }
        if !focus {
            self.previewing.set(true);
        } else {
            self.previewing.set(false);
        }
        let p = terminal_prefs::load();
        let window = self.term.window.clone();
        if window.is_visible() {
            // (going up still: it comes down again from where it is, mapped anew so the keyboard comes with it)
            window.set_visible(false);
        }
        self.stop_resizing();
        let target = self.configure(&p);
        self.set_height(target);
        self.term.set_frozen(false);
        if !self.term.has_tabs() {
            self.term.open_tab();
        }
        window.set_keyboard_mode(if focus { KeyboardMode::OnDemand } else { KeyboardMode::None });
        self.state.set(State::Showing);
        let n = self.generation.get() + 1;
        self.generation.set(n);
        let moving = motion(&p);
        if moving {
            self.slide.set_opacity(1.0);
            self.slide.definir(Canal::Opacidade, 1.0).definir(Canal::Y, -f64::from(target));
        } else {
            self.slide.definir(Canal::Opacidade, 1.0).definir(Canal::Y, 0.0);
            self.slide.set_opacity(0.0);
        }
        // the keyboard in the terminal from the first frame
        self.term.focus_current();
        window.present();
        let w = Rc::downgrade(self);
        let arrived = move || {
            if let Some(d) = w.upgrade().filter(|d| d.generation.get() == n) {
                d.state.set(State::Shown);
                if debug() {
                    eprintln!(
                        "debug terminal-suspenso: aberto ({} px de altura{})",
                        d.height.get(),
                        if d.expanded.get() { ", expandido" } else { "" }
                    );
                }
            }
        };
        if moving {
            self.slide.ir(Canal::Y, 0.0, mv::SUAVE);
            self.slide.ao_parar(arrived);
        } else {
            fade(self.slide.upcast_ref(), 0.0, 1.0, arrived);
        }
        if debug() {
            let (c, sw, sh) = self.screen.borrow().clone();
            eprintln!(
                "debug terminal-suspenso: descendo em {c} ({sw}×{sh}), {} × {target}{}",
                self.width.get().map(|w| w.to_string()).unwrap_or_else(|| "tela inteira".into()),
                if focus { "" } else { " (prévia)" }
            );
        }
        if focus && !self.expanded.get() {
            self.keys_hint();
        }
    }

    fn hide(self: &Rc<Self>) {
        if matches!(self.state.get(), State::Hidden | State::Hiding) {
            return;
        }
        self.state.set(State::Hiding);
        self.previewing.set(false);
        let n = self.generation.get() + 1;
        self.generation.set(n);
        self.hide_outer_hint();
        self.inner_hint.set_visible(false);
        self.stop_resizing();
        self.term.set_frozen(false);
        let w = Rc::downgrade(self);
        let gone = move || {
            if let Some(d) = w.upgrade().filter(|d| d.generation.get() == n) {
                d.term.window.set_visible(false);
                d.term.window.set_keyboard_mode(KeyboardMode::None);
                d.slide.set_opacity(1.0);
                d.state.set(State::Hidden);
                if debug() {
                    eprintln!("debug terminal-suspenso: escondido");
                }
            }
        };
        if motion(&terminal_prefs::load()) {
            self.slide.ir(Canal::Y, -f64::from(self.height.get()), mv::RAPIDA);
            self.slide.ao_parar(gone);
        } else {
            fade(self.slide.upcast_ref(), 1.0, 0.0, gone);
        }
        if debug() {
            eprintln!("debug terminal-suspenso: subindo");
        }
    }

    /// "F12 esconde · Shift+F12 expande · arraste a borda de baixo…" under it, the first three times, for 2 s.
    fn keys_hint(self: &Rc<Self>) {
        let mut p = terminal_prefs::load();
        if p.dropdown.hints_shown >= 3 {
            return;
        }
        p.dropdown.hints_shown += 1;
        terminal_prefs::save(&p);
        // (the string on trf's own line: the catalog's test finds texts that way)
        let text = trf("{toggle} hides · {expand} expands · drag the bottom edge to change the height",
            &[("toggle", keys_text(TOGGLE_ID)), ("expand", keys_text(EXPAND_ID))]);
        if self.outer_hint.borrow().is_none() {
            let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
            let win = camada(&app, "kalango-terminal-suspenso-dica", Layer::Top, &[Edge::Top]);
            win.add_css_class("term-suspenso-dica-janela");
            let (chip, label) = hint_chip();
            let a = Animavel::new(&chip);
            win.set_child(Some(&a));
            *self.outer_hint.borrow_mut() = Some((win, a, label));
        }
        let Some((win, a, label)) = self.outer_hint.borrow().clone() else { return };
        label.set_text(&text);
        let (_, sw, _) = self.screen.borrow().clone();
        // (a narrow screen: the hint is cut, not wider than it)
        label.set_max_width_chars(((sw - 80) / 7).max(20));
        if let Some(m) = self.term.window.monitor() {
            win.set_monitor(Some(&m));
        }
        win.set_margin(Edge::Top, self.height.get() + 14);
        a.definir(Canal::Opacidade, 0.0);
        win.present();
        a.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.later_hide_hint();
        if debug() {
            eprintln!("debug terminal-suspenso: dica «{text}»");
        }
    }

    /// Expanded: "Shift+F12 volta para 45% · fixado: não esconde ao clicar fora" inside it, the first three times.
    fn expand_hint(self: &Rc<Self>) {
        let mut p = terminal_prefs::load();
        if p.dropdown.expand_hints_shown >= 3 {
            return;
        }
        p.dropdown.expand_hints_shown += 1;
        terminal_prefs::save(&p);
        let mut text = trf("{expand} goes back to {n}%", &[("expand", keys_text(EXPAND_ID)), ("n", p.dropdown.height.to_string())]);
        if self.pin.is_active() {
            text = format!("{text} · {}", tr("pinned: it does not hide on a click outside"));
        }
        self.inner_text.set_text(&text);
        self.inner_hint.set_visible(true);
        self.later_hide_hint();
        if debug() {
            eprintln!("debug terminal-suspenso: dica «{text}»");
        }
    }

    fn later_hide_hint(self: &Rc<Self>) {
        let n = self.hint_generation.get() + 1;
        self.hint_generation.set(n);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(2000), move || {
            if let Some(d) = w.upgrade().filter(|d| d.hint_generation.get() == n) {
                d.hide_outer_hint();
                let w = d.inner_hint.downgrade();
                if d.inner_hint.is_visible() {
                    fade(d.inner_hint.upcast_ref(), 1.0, 0.0, move || {
                        if let Some(h) = w.upgrade() {
                            h.set_visible(false);
                            h.set_opacity(1.0);
                        }
                    });
                }
            }
        });
    }

    fn hide_outer_hint(&self) {
        let Some((win, a, _)) = self.outer_hint.borrow().clone() else { return };
        if !win.is_visible() {
            return;
        }
        let ww = win.downgrade();
        a.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        a.ao_parar(move || {
            if let Some(w) = ww.upgrade() {
                w.set_visible(false);
            }
        });
    }
}
