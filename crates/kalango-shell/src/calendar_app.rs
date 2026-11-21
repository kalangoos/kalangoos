//! KalangoOS's own Calendar (design "Calendário", 62581; app id os.kalango.Calendario; `kalango-shell calendario`):
//! the week (or a day, or the month) of the person's local calendars (agenda.rs: KalangoOS's own "Pessoal" and
//! "Trabalho", which it writes, and the desktop's, read only) and the agents' routines (rotinas.rs) as events — so
//! one sees when each will run and moves it like any appointment. Dragging on the grid makes an event: the block is
//! born under the pointer (a quick spring) and its name is typed there. The agent notices a meeting with no agenda
//! and drafts one; a request ("marque dentista quinta às 14h") becomes the event.

use crate::agenda;
use crate::conversa::ui_icon;
use crate::rotinas::{Gatilho, Rotinas};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The grid's hour (px), its hours' column and the minutes a drag snaps to.
const HOUR: f64 = 56.0;
const HOURS_COL: f64 = 50.0;
const SNAP: u32 = 15;
const ROUTINES: &str = "rotinas";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Day,
    Week,
    Month,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// an event of a calendar file (its UID; whether it repeats)
    File(PathBuf, String, bool),
    /// an agent's routine (its id)
    Routine(u64),
}

#[derive(Clone, Debug)]
struct Item {
    /// days after the first one in view
    day: i64,
    start: u32,
    end: u32,
    title: String,
    /// the calendar's key ("pessoal", "trabalho", "rotinas", or a file's path)
    calendar: String,
    source: Source,
}

#[derive(Clone, Debug)]
enum Drag {
    Create { day: i64, from: u32, to: u32 },
    Move { idx: usize, day: i64, start: u32, offset: u32, moved: bool },
}

pub struct CalendarApp {
    window: gtk::ApplicationWindow,
    month_label: gtk::Label,
    heads: gtk::DrawingArea,
    grid: gtk::DrawingArea,
    scroll: gtk::ScrolledWindow,
    mini: gtk::DrawingArea,
    mini_label: gtk::Label,
    views: Vec<(View, gtk::Button)>,
    rail: Vec<(View, gtk::Button)>,
    notice: gtk::Box,
    notice_text: gtk::Label,
    agendas: gtk::Box,
    status: gtk::Label,
    ask: gtk::Entry,
    pop: gtk::Popover,
    pop_box: gtk::Box,
    view: Cell<View>,
    /// the day in view (its week, its month)
    anchor: RefCell<glib::DateTime>,
    mini_month: RefCell<glib::DateTime>,
    items: RefCell<Vec<Item>>,
    hidden: RefCell<HashSet<String>>,
    drag: RefCell<Option<Drag>>,
    born: RefCell<Option<(i64, u32, i64)>>,
    dismissed: RefCell<HashSet<String>>,
    shell: Weak<Shell>,
}

thread_local! {
    static APP: RefCell<Option<Rc<CalendarApp>>> = const { RefCell::new(None) };
}

pub fn open(shell: &Rc<Shell>) {
    let a = APP.with(|a| a.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let a = CalendarApp::new(&app, shell);
        APP.with(|x| *x.borrow_mut() = Some(a.clone()));
        a
    });
    if !a.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        a.window.set_default_size(w, h);
    }
    a.window.present();
    a.reload();
}

fn today() -> glib::DateTime {
    glib::DateTime::now_local().unwrap_or_else(|_| glib::DateTime::from_unix_local(0).unwrap())
}

fn midnight(d: &glib::DateTime) -> glib::DateTime {
    glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).unwrap_or_else(|_| d.clone())
}

/// The calendar's colour (the design's: Pessoal teal, Trabalho lavender, routines coral, others gold).
fn color_of(cal: &str) -> &'static str {
    match cal {
        "pessoal" => "#8ED2C3",
        "trabalho" => "#A5A1E2",
        ROUTINES => "#F28B82",
        _ => "#E5C98C",
    }
}

fn hidden_file() -> PathBuf {
    glib::user_config_dir().join("kalango/calendario.json")
}

fn clock(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn weekday_short(n: i32) -> &'static str {
    // 1 Monday … 7 Sunday
    [tr("MON"), tr("TUE"), tr("WED"), tr("THU"), tr("FRI"), tr("SAT"), tr("SUN")][((n - 1).rem_euclid(7)) as usize]
}

pub(crate) fn month_name(m: i32) -> &'static str {
    [tr("January"), tr("February"), tr("March"), tr("April"), tr("May"), tr("June"), tr("July"), tr("August"), tr("September"),
        tr("October"), tr("November"), tr("December")][((m - 1).rem_euclid(12)) as usize]
}

fn layout(cr: &cairo::Context, text: &str, font: &str, width: f64) -> gtk::pango::Layout {
    let l = pangocairo::functions::create_layout(cr);
    l.set_font_description(Some(&gtk::pango::FontDescription::from_string(font)));
    l.set_text(text);
    if width > 0.0 {
        l.set_width((width * f64::from(gtk::pango::SCALE)) as i32);
        l.set_wrap(gtk::pango::WrapMode::WordChar);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    l
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    crate::tema::rgb(hex)
}

/// A calendar's own colour (it names the calendar: it does not follow the theme's accent).
fn cal_rgb(hex: &str) -> (f64, f64, f64) {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x808080);
    (f64::from((v >> 16) & 255) / 255.0, f64::from((v >> 8) & 255) / 255.0, f64::from(v & 255) / 255.0)
}

impl CalendarApp {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<CalendarApp> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Calendar")).default_width(w).default_height(h).build();
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        // the rail: the app, the views
        let rail = caixa(Orientation::Vertical, 6, "mus-trilho");
        let logo = crate::apps::imagem_kalango("calendario", 28);
        logo.set_halign(Align::Center);
        rail.append(&logo);
        let sep = gtk::Separator::new(Orientation::Horizontal);
        sep.add_css_class("mus-trilho-linha");
        rail.append(&sep);
        let mut views = Vec::new();
        for (v, glyph, name, id) in [(View::Day, "relogio", tr("Day"), "dia"), (View::Week, "grade", tr("Week"), "semana"), (View::Month, "lista", tr("Month"), "mes")] {
            let b = gtk::Button::new();
            b.add_css_class("mus-trilho-botao");
            b.set_child(Some(&ui_icon(glyph, "#8E8F99", 20)));
            b.set_tooltip_text(Some(name));
            b.set_halign(Align::Center);
            crate::central::posicao_debug_sempre(&format!("calendario trilho {id}"), b.upcast_ref());
            rail.append(&b);
            views.push((v, b));
        }
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        let name = rotulo(tr("Calendar"), "img-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        bar.append(&name);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(340, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Search or ask: “book the dentist on Thursday at 2 pm”")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ab = gtk::Overlay::new();
        ab.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ab.add_overlay(&key);
        bar.append(&ab);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));

        // the title row: the month, the arrows, today, the views
        let top = caixa(Orientation::Horizontal, 8, "cal-topo");
        let month_label = rotulo("", "cal-mes");
        month_label.set_hexpand(true);
        month_label.set_xalign(0.0);
        top.append(&month_label);
        let prev = gtk::Button::new();
        prev.add_css_class("cal-seta");
        prev.set_child(Some(&ui_icon("seta-esquerda", "#E2E2E6", 12)));
        let today_b = gtk::Button::with_label(tr("Today"));
        today_b.add_css_class("cal-hoje");
        let next = gtk::Button::new();
        next.add_css_class("cal-seta");
        next.set_child(Some(&ui_icon("seta-direita", "#E2E2E6", 12)));
        for (b, id) in [(&prev, "anterior"), (&today_b, "hoje"), (&next, "proxima")] {
            crate::central::posicao_debug_sempre(&format!("calendario {id}"), b.upcast_ref());
            top.append(b);
        }
        let seg = caixa(Orientation::Horizontal, 0, "cal-segmentos");
        seg.set_margin_start(16);
        let mut seg_buttons = Vec::new();
        for (v, n) in [(View::Day, tr("Day")), (View::Week, tr("Week")), (View::Month, tr("Month"))] {
            let b = gtk::Button::with_label(n);
            b.add_css_class("cal-segmento");
            seg.append(&b);
            seg_buttons.push((v, b));
        }
        top.append(&seg);
        // the days' heads and the grid (scrolled through the day; it opens at 08:00)
        let heads = gtk::DrawingArea::new();
        heads.set_content_height(44);
        heads.set_hexpand(true);
        let grid = gtk::DrawingArea::new();
        grid.set_content_height((24.0 * HOUR) as i32 + 1);
        grid.set_hexpand(true);
        crate::central::posicao_debug_sempre("calendario grade", grid.upcast_ref());
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&grid));
        let main = caixa(Orientation::Vertical, 10, "cal-principal");
        main.set_hexpand(true);
        main.append(&top);
        main.append(&heads);
        main.append(&scroll);
        // the side: the month, the agent's notice, the calendars
        let side = caixa(Orientation::Vertical, 14, "cal-lado");
        side.set_size_request(300, -1);
        let mini_box = caixa(Orientation::Vertical, 8, "cal-cartao");
        let mh = caixa(Orientation::Horizontal, 6, "");
        let mini_label = rotulo("", "cal-mini-mes");
        mini_label.set_hexpand(true);
        mini_label.set_xalign(0.0);
        mh.append(&mini_label);
        let mprev = gtk::Button::new();
        mprev.add_css_class("cal-seta-mini");
        mprev.set_child(Some(&ui_icon("seta-esquerda", "#8E8F99", 10)));
        let mnext = gtk::Button::new();
        mnext.add_css_class("cal-seta-mini");
        mnext.set_child(Some(&ui_icon("seta-direita", "#8E8F99", 10)));
        mh.append(&mprev);
        mh.append(&mnext);
        mini_box.append(&mh);
        let mini = gtk::DrawingArea::new();
        mini.set_content_height(250);
        mini.set_cursor_from_name(Some("pointer"));
        crate::central::posicao_debug_sempre("calendario mini", mini.upcast_ref());
        mini_box.append(&mini);
        side.append(&mini_box);
        let notice = caixa(Orientation::Vertical, 8, "cal-aviso");
        let nh = caixa(Orientation::Horizontal, 8, "");
        nh.append(&ui_icon("spark", "#A5A1E2", 14));
        let nt = rotulo(tr("THE AGENT NOTICED"), "dsc-secao");
        nh.append(&nt);
        notice.append(&nh);
        let notice_text = rotulo("", "cal-aviso-texto");
        notice_text.set_wrap(true);
        notice_text.set_xalign(0.0);
        notice_text.set_max_width_chars(1);
        notice.append(&notice_text);
        let nb = caixa(Orientation::Horizontal, 8, "");
        let draft = gtk::Button::with_label(tr("Draft an agenda"));
        draft.add_css_class("cal-aviso-sim");
        let later = gtk::Button::with_label(tr("Later"));
        later.add_css_class("cal-aviso-nao");
        crate::central::posicao_debug_sempre("calendario rascunhar", draft.upcast_ref());
        nb.append(&draft);
        nb.append(&later);
        notice.append(&nb);
        notice.set_visible(false);
        side.append(&notice);
        let ag = caixa(Orientation::Vertical, 8, "cal-cartao");
        let at = rotulo(tr("CALENDARS"), "dsc-secao");
        at.set_xalign(0.0);
        ag.append(&at);
        let agendas = caixa(Orientation::Vertical, 4, "");
        ag.append(&agendas);
        side.append(&ag);
        let status = rotulo("", "dsc-modelo");
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.set_max_width_chars(1);
        side.append(&status);
        let body = caixa(Orientation::Horizontal, 20, "cal-corpo");
        body.set_vexpand(true);
        body.append(&main);
        body.append(&side);
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        let measured = gtk::Overlay::new();
        measured.set_child(Some(&body));
        measured.add_overlay(&meter);
        measured.set_measure_overlay(&meter, false);
        measured.set_vexpand(true);
        let sd = side.clone();
        meter.connect_resize(move |_, w, _| sd.set_visible(w >= 1000));
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_hexpand(true);
        col.append(&handle);
        col.append(&measured);
        let root = caixa(Orientation::Horizontal, 0, "doc-fundo");
        root.append(&rail);
        root.append(&col);
        window.set_child(Some(&root));
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        pop.set_parent(&grid);
        pop.set_autohide(true);
        let pop_box = caixa(Orientation::Vertical, 6, "cal-pop");
        pop.set_child(Some(&pop_box));
        let hidden: HashSet<String> = std::fs::read_to_string(hidden_file()).ok().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).unwrap_or_default().into_iter().collect();
        let a = Rc::new(CalendarApp {
            window, month_label, heads, grid, scroll, mini, mini_label, views: seg_buttons, rail: views.clone(), notice, notice_text, agendas, status, ask, pop, pop_box,
            view: Cell::new(View::Week), anchor: RefCell::new(today()), mini_month: RefCell::new(today()), items: RefCell::default(),
            hidden: RefCell::new(hidden), drag: RefCell::default(), born: RefCell::default(), dismissed: RefCell::default(), shell: Rc::downgrade(shell),
        });
        a.wire(&views, &prev, &today_b, &next, &mprev, &mnext, &draft, &later);
        // (the week is the view at first: shown on the rail and the switch)
        for (v, b) in a.views.iter().chain(a.rail.iter()) {
            if *v == View::Week {
                b.add_css_class("ativo");
            }
        }
        a
    }

    #[allow(clippy::too_many_arguments)]
    fn wire(self: &Rc<Self>, rail: &[(View, gtk::Button)], prev: &gtk::Button, today_b: &gtk::Button, next: &gtk::Button, mprev: &gtk::Button,
        mnext: &gtk::Button, draft: &gtk::Button, later: &gtk::Button) {
        for (v, b) in rail.iter().chain(self.views.iter()) {
            let (v, w) = (*v, Rc::downgrade(self));
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    a.set_view(v);
                }
            });
        }
        type H = Box<dyn Fn(&Rc<CalendarApp>)>;
        let pairs: Vec<(&gtk::Button, H)> = vec![
            (prev, Box::new(|a| a.step(-1))),
            (next, Box::new(|a| a.step(1))),
            (today_b, Box::new(|a| {
                *a.anchor.borrow_mut() = today();
                *a.mini_month.borrow_mut() = today();
                a.reload();
            })),
            (mprev, Box::new(|a| a.mini_step(-1))),
            (mnext, Box::new(|a| a.mini_step(1))),
            (draft, Box::new(|a| a.draft_agenda())),
            (later, Box::new(|a| {
                if let Some(k) = a.notice_key() {
                    a.dismissed.borrow_mut().insert(k);
                }
                a.notice.set_visible(false);
            })),
        ];
        for (b, f) in pairs {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    f(&a);
                }
            });
        }
        let w = Rc::downgrade(self);
        self.heads.set_draw_func(move |_, cr, width, height| {
            if let Some(a) = w.upgrade() {
                a.draw_heads(cr, f64::from(width), f64::from(height));
            }
        });
        let w = Rc::downgrade(self);
        self.grid.set_draw_func(move |_, cr, width, height| {
            if let Some(a) = w.upgrade() {
                a.draw_grid(cr, f64::from(width), f64::from(height));
            }
        });
        let w = Rc::downgrade(self);
        self.mini.set_draw_func(move |_, cr, width, height| {
            if let Some(a) = w.upgrade() {
                a.draw_mini(cr, f64::from(width), f64::from(height));
            }
        });
        // the grid: a drag makes an event (or moves one); a click on one shows it
        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(a) = w.upgrade() {
                a.drag_begin(x, y);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_update(move |g, dx, dy| {
            if let (Some(a), Some((x, y))) = (w.upgrade(), g.start_point()) {
                a.drag_update(x + dx, y + dy);
            }
        });
        let w = Rc::downgrade(self);
        drag.connect_drag_end(move |g, dx, dy| {
            if let (Some(a), Some((x, y))) = (w.upgrade(), g.start_point()) {
                a.drag_end(x + dx, y + dy);
            }
        });
        self.grid.add_controller(drag);
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        click.connect_released(move |_, _, x, y| {
            if let Some(a) = w.upgrade() {
                a.mini_click(x, y);
            }
        });
        self.mini.add_controller(click);
        // the request
        let w = Rc::downgrade(self);
        self.ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(a), false) = (w.upgrade(), t.is_empty()) {
                e.set_text("");
                gtk::prelude::GtkWindowExt::set_focus(&a.window, None::<&gtk::Widget>);
                a.request(t);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(a) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            let typing = gtk::prelude::GtkWindowExt::focus(&a.window).is_some_and(|f| f.downcast_ref::<gtk::Text>().is_some());
            if ctrl && k == gdk::Key::k {
                a.ask.grab_focus();
                return glib::Propagation::Stop;
            }
            if typing {
                return glib::Propagation::Proceed;
            }
            match k {
                gdk::Key::Left | gdk::Key::Page_Up => a.step(-1),
                gdk::Key::Right | gdk::Key::Page_Down => a.step(1),
                gdk::Key::t | gdk::Key::T => {
                    *a.anchor.borrow_mut() = today();
                    a.reload();
                }
                gdk::Key::d | gdk::Key::D => a.set_view(View::Day),
                gdk::Key::w | gdk::Key::s | gdk::Key::S => a.set_view(View::Week),
                gdk::Key::m | gdk::Key::M => a.set_view(View::Month),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Calendario").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        let w = Rc::downgrade(self);
        self.window.connect_map(move |j| {
            app_id(j);
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
            // (the grid opens at 08:00)
            if let Some(a) = w.upgrade() {
                let s = a.scroll.clone();
                glib::timeout_add_local_once(Duration::from_millis(50), move || s.vadjustment().set_value(8.0 * HOUR - 6.0));
            }
        });
        // the clock: the now line moves, the routines may change
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(60), move || {
            let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
            if a.window.is_visible() {
                a.reload();
            }
            glib::ControlFlow::Continue
        });
    }

    // ------------------------------------------------------------------------------------------- the range

    fn first_day(&self) -> glib::DateTime {
        let a = midnight(&self.anchor.borrow());
        match self.view.get() {
            View::Day => a,
            // (the week starts on Sunday, as in the design)
            View::Week => a.add_days(-(a.day_of_week() % 7)).unwrap_or(a),
            View::Month => {
                let first = glib::DateTime::from_local(a.year(), a.month(), 1, 0, 0, 0.0).unwrap_or(a);
                first.add_days(-(first.day_of_week() % 7)).unwrap_or(first)
            }
        }
    }

    fn days(&self) -> i64 {
        match self.view.get() {
            View::Day => 1,
            View::Week => 7,
            View::Month => 42,
        }
    }

    fn set_view(self: &Rc<Self>, v: View) {
        self.view.set(v);
        for (x, b) in self.views.iter().chain(self.rail.iter()) {
            if *x == v {
                b.add_css_class("ativo");
            } else {
                b.remove_css_class("ativo");
            }
        }
        // (the month has no hours: its cells fill the window)
        self.grid.set_content_height(if v == View::Month { 600 } else { (24.0 * HOUR) as i32 + 1 });
        self.reload();
        if v != View::Month {
            let s = self.scroll.clone();
            glib::idle_add_local_once(move || s.vadjustment().set_value(8.0 * HOUR - 6.0));
        }
        if debug() {
            eprintln!("debug calendario: vista {v:?}");
        }
    }

    fn step(self: &Rc<Self>, d: i32) {
        let a = self.anchor.borrow().clone();
        let n = match self.view.get() {
            View::Day => a.add_days(d),
            View::Week => a.add_days(7 * d),
            View::Month => a.add_months(d),
        };
        if let Ok(n) = n {
            *self.anchor.borrow_mut() = n.clone();
            *self.mini_month.borrow_mut() = n;
        }
        self.reload();
    }

    fn mini_step(&self, d: i32) {
        let m = self.mini_month.borrow().add_months(d);
        if let Ok(m) = m {
            *self.mini_month.borrow_mut() = m;
        }
        self.update_mini_label();
        self.mini.queue_draw();
    }

    fn update_mini_label(&self) {
        let m = self.mini_month.borrow();
        self.mini_label.set_text(&format!("{} {}", month_name(m.month()), m.year()));
    }

    /// The events and routines of the days in view.
    fn reload(self: &Rc<Self>) {
        let first = self.first_day();
        let n = self.days();
        let mut items = Vec::new();
        let files = agenda::sources();
        let routines = Rotinas::carregar();
        for k in 0..n {
            let Ok(d) = first.add_days(k as i32) else { continue };
            for f in &files {
                let cal = agenda::OWN.iter().find(|(id, _)| *f == agenda::own_file(id)).map(|(id, _)| id.to_string()).unwrap_or_else(|| f.display().to_string());
                for e in agenda::day_of(f, d.year(), d.month(), d.day_of_month()) {
                    // (an all-day event sits at the top of its day, its first hour)
                    let start = e.start.unwrap_or(0);
                    let end = e.end.unwrap_or(start + 60).max(start + SNAP);
                    items.push(Item { day: k, start, end, title: e.title, calendar: cal.clone(), source: Source::File(f.clone(), e.uid, e.repeats) });
                }
            }
            // the agents' routines that run at a time on this day (1 Monday … 7 Sunday)
            let wd = d.day_of_week() as u8;
            for r in routines.lista.iter().filter(|r| r.ativa) {
                if let Gatilho::Horario { dias, hora, minuto } = &r.gatilho {
                    if dias.is_empty() || dias.contains(&wd) {
                        let s = u32::from(*hora) * 60 + u32::from(*minuto);
                        items.push(Item { day: k, start: s, end: (s + 30).min(24 * 60), title: r.nome.clone(), calendar: ROUTINES.into(), source: Source::Routine(r.id) });
                    }
                }
            }
        }
        if debug() {
            eprintln!("debug calendario: {} eventos de {} a {} ({} de rotinas)", items.len(), first.format("%F").map(|s| s.to_string()).unwrap_or_default(),
                first.add_days(n as i32 - 1).ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default(),
                items.iter().filter(|i| i.calendar == ROUTINES).count());
        }
        *self.items.borrow_mut() = items;
        let a = self.anchor.borrow().clone();
        self.month_label.set_text(&match self.view.get() {
            View::Day => format!("{} {} {}", a.day_of_month(), month_name(a.month()).to_lowercase(), a.year()),
            _ => trf("{month} of {year}", &[("month", month_name(a.month()).to_string()), ("year", a.year().to_string())]),
        });
        self.update_mini_label();
        self.fill_agendas();
        self.heads.set_visible(self.view.get() != View::Month);
        self.heads.queue_draw();
        self.grid.queue_draw();
        self.mini.queue_draw();
        self.watch();
    }

    fn fill_agendas(self: &Rc<Self>) {
        while let Some(c) = self.agendas.first_child() {
            self.agendas.remove(&c);
        }
        let mut list: Vec<(String, String)> = agenda::OWN.iter().map(|(id, n)| (id.to_string(), tr(n).to_string())).collect();
        list.push((ROUTINES.into(), tr("Agents' routines").to_string()));
        for f in agenda::sources().into_iter().filter(|f| !f.starts_with(agenda::own_dir())) {
            let n = agenda::name_of(&f).unwrap_or_else(|| tr("Local calendar").to_string());
            list.push((f.display().to_string(), n));
        }
        for (key, name) in list {
            let r = caixa(Orientation::Horizontal, 10, "cal-agenda");
            let dot = caixa(Orientation::Horizontal, 0, "cal-agenda-cor");
            dot.set_valign(Align::Center);
            let css = gtk::CssProvider::new();
            css.load_from_string(&format!(".cal-agenda-cor {{ background: {}; }}", color_of(&key)));
            #[allow(deprecated)]
            dot.style_context().add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            r.append(&dot);
            let l = rotulo(&name, "cal-agenda-nome");
            l.set_hexpand(true);
            l.set_xalign(0.0);
            r.append(&l);
            let shown = !self.hidden.borrow().contains(&key);
            let eye = gtk::Button::new();
            eye.add_css_class("cal-olho");
            eye.set_child(Some(&ui_icon("olho", if shown { "#8E8F99" } else { "#4A4B57" }, 14)));
            eye.set_tooltip_text(Some(if shown { tr("Hide") } else { tr("Show") }));
            crate::central::posicao_debug_sempre(&format!("calendario agenda {key}"), eye.upcast_ref());
            let w = Rc::downgrade(self);
            eye.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    let now_hidden = {
                        let mut h = a.hidden.borrow_mut();
                        if !h.remove(&key) {
                            h.insert(key.clone());
                            true
                        } else {
                            false
                        }
                    };
                    let list: Vec<String> = a.hidden.borrow().iter().cloned().collect();
                    let _ = std::fs::create_dir_all(hidden_file().parent().unwrap());
                    let _ = std::fs::write(hidden_file(), serde_json::to_string(&list).unwrap_or_default());
                    if debug() {
                        eprintln!("debug calendario: agenda {key} {}", if now_hidden { "escondida" } else { "à vista" });
                    }
                    a.reload();
                }
            });
            r.append(&eye);
            self.agendas.append(&r);
        }
    }

    fn visible_items(&self) -> Vec<(usize, Item)> {
        let hidden = self.hidden.borrow();
        self.items.borrow().iter().cloned().enumerate().filter(|(_, i)| !hidden.contains(&i.calendar)).collect()
    }

    // ------------------------------------------------------------------------------------------- drawing

    fn col_w(&self, w: f64) -> f64 {
        (w - HOURS_COL) / self.days().min(7) as f64
    }

    fn draw_heads(&self, cr: &cairo::Context, w: f64, h: f64) {
        let first = self.first_day();
        let t = midnight(&today());
        let cw = self.col_w(w);
        for k in 0..self.days().min(7) {
            let Ok(d) = first.add_days(k as i32) else { continue };
            let is_today = d.year() == t.year() && d.day_of_year() == t.day_of_year();
            let x = HOURS_COL + k as f64 * cw + 10.0;
            let (r, g, b) = rgb(if is_today { "#A5A1E2" } else { "#8E8F99" });
            cr.set_source_rgb(r, g, b);
            let l = layout(cr, weekday_short(d.day_of_week()), "Inter Semi-Bold 10", 0.0);
            cr.move_to(x, h - 40.0);
            pangocairo::functions::show_layout(cr, &l);
            let (r, g, b) = rgb(if is_today { "#A5A1E2" } else { "#E2E2E6" });
            cr.set_source_rgb(r, g, b);
            let l = layout(cr, &d.day_of_month().to_string(), "Inter Bold 16", 0.0);
            cr.move_to(x, h - 26.0);
            pangocairo::functions::show_layout(cr, &l);
        }
    }

    fn block_rect(&self, cw: f64, it: &Item) -> (f64, f64, f64, f64) {
        let x = HOURS_COL + it.day as f64 * cw + 4.0;
        let y = f64::from(it.start) / 60.0 * HOUR + 1.0;
        let h = (f64::from(it.end.saturating_sub(it.start)) / 60.0 * HOUR - 3.0).max(18.0);
        (x, y, cw - 8.0, h)
    }

    fn draw_block(&self, cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, title: &str, color: &str, time: Option<String>) {
        let (r, g, b) = cal_rgb(color);
        // the design's block: its colour faint under, a 3 px bar on the left, the name
        cr.set_source_rgba(r, g, b, 0.22);
        cr.rectangle(x, y, w, h);
        let _ = cr.fill();
        cr.set_source_rgb(r, g, b);
        cr.rectangle(x, y, 3.0, h);
        let _ = cr.fill();
        let (tr_, tg, tb) = rgb("#E2E2E6");
        cr.set_source_rgb(tr_, tg, tb);
        let l = layout(cr, title, "Inter Semi-Bold 10", w - 16.0);
        l.set_height(((h - 10.0).max(12.0) * f64::from(gtk::pango::SCALE)) as i32);
        cr.move_to(x + 9.0, y + 5.0);
        pangocairo::functions::show_layout(cr, &l);
        if let Some(t) = time.filter(|_| h >= 44.0) {
            let (_, lh) = l.pixel_size();
            let (mr, mg, mb) = rgb("#8E8F99");
            cr.set_source_rgb(mr, mg, mb);
            let l2 = layout(cr, &t, "Inter 9", w - 16.0);
            cr.move_to(x + 9.0, y + 7.0 + f64::from(lh));
            pangocairo::functions::show_layout(cr, &l2);
        }
    }

    fn draw_grid(&self, cr: &cairo::Context, w: f64, h: f64) {
        if self.view.get() == View::Month {
            return self.draw_month(cr, w, h);
        }
        let n = self.days();
        let cw = self.col_w(w);
        let first = self.first_day();
        let t = midnight(&today());
        // today's column, a little lighter
        for k in 0..n {
            if first.add_days(k as i32).is_ok_and(|d| d.year() == t.year() && d.day_of_year() == t.day_of_year()) {
                let (r, g, b) = rgb("#1E1F26");
                cr.set_source_rgb(r, g, b);
                cr.rectangle(HOURS_COL + k as f64 * cw, 0.0, cw, h);
                let _ = cr.fill();
            }
        }
        // the hours' lines and labels, the columns' lines
        let (lr, lg, lb) = rgb("#26272F");
        for hr in 0..=24 {
            let y = f64::from(hr) * HOUR + 0.5;
            cr.set_source_rgb(lr, lg, lb);
            cr.move_to(HOURS_COL, y);
            cr.line_to(w, y);
            let _ = cr.stroke();
            if hr < 24 {
                let (mr, mg, mb) = rgb("#7A7B88");
                cr.set_source_rgb(mr, mg, mb);
                let l = layout(cr, &clock(hr * 60), "JetBrains Mono 8", 0.0);
                cr.move_to(4.0, y - 6.0);
                pangocairo::functions::show_layout(cr, &l);
            }
        }
        for k in 0..=n {
            let x = HOURS_COL + k as f64 * cw + 0.5;
            cr.set_source_rgb(lr, lg, lb);
            cr.move_to(x, 0.0);
            cr.line_to(x, h);
            let _ = cr.stroke();
        }
        // the events (the one being moved drawn where it goes)
        let drag = self.drag.borrow().clone();
        for (idx, it) in self.visible_items() {
            let mut it = it.clone();
            if let Some(Drag::Move { idx: m, day, start, moved: true, .. }) = &drag {
                if *m == idx {
                    let len = it.end - it.start;
                    it.day = *day;
                    it.start = *start;
                    it.end = start + len;
                }
            }
            let (x, y, bw, bh) = self.block_rect(cw, &it);
            self.draw_block(cr, x, y, bw, bh, &it.title, color_of(&it.calendar), Some(format!("{} – {}", clock(it.start), clock(it.end))));
        }
        // the block being made, under the pointer (born with a quick spring)
        if let Some(Drag::Create { day, from, to }) = &drag {
            let (s, e) = ((*from).min(*to), (*from).max(*to).max((*from).min(*to) + SNAP));
            let it = Item { day: *day, start: s, end: e, title: format!("{} – {}", clock(s), clock(e)), calendar: "pessoal".into(), source: Source::Routine(0) };
            let (x, y, bw, bh) = self.block_rect(cw, &it);
            let k = self.born.borrow().as_ref().map(|(_, _, t0)| ((glib::monotonic_time() - t0) as f64 / 160_000.0).min(1.0)).unwrap_or(1.0);
            // (a little overshoot: 0.85 → 1.04 → 1)
            let sc = if k < 0.7 { 0.85 + 0.19 * (k / 0.7) } else { 1.04 - 0.04 * ((k - 0.7) / 0.3) };
            cr.save().ok();
            cr.translate(x + bw / 2.0, y + bh / 2.0);
            cr.scale(sc, sc);
            self.draw_block(cr, -bw / 2.0, -bh / 2.0, bw, bh, &it.title, "#8ED2C3", None);
            cr.restore().ok();
        }
        // now
        for k in 0..n {
            if first.add_days(k as i32).is_ok_and(|d| d.year() == t.year() && d.day_of_year() == t.day_of_year()) {
                let now = today();
                let y = f64::from(now.hour() * 60 + now.minute()) / 60.0 * HOUR;
                let (r, g, b) = rgb("#F28B82");
                cr.set_source_rgb(r, g, b);
                cr.rectangle(HOURS_COL + k as f64 * cw, y - 1.0, cw, 2.0);
                let _ = cr.fill();
                cr.arc(HOURS_COL + k as f64 * cw, y, 4.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
    }

    fn draw_month(&self, cr: &cairo::Context, w: f64, h: f64) {
        let first = self.first_day();
        let a = self.anchor.borrow().clone();
        let t = midnight(&today());
        let (cw, ch) = (w / 7.0, h / 6.0);
        let items = self.visible_items();
        let (lr, lg, lb) = rgb("#26272F");
        for k in 0..42 {
            let Ok(d) = first.add_days(k) else { continue };
            let (x, y) = (f64::from(k % 7) * cw, f64::from(k / 7) * ch);
            cr.set_source_rgb(lr, lg, lb);
            cr.rectangle(x + 0.5, y + 0.5, cw, ch);
            let _ = cr.stroke();
            let in_month = d.month() == a.month();
            let is_today = d.year() == t.year() && d.day_of_year() == t.day_of_year();
            let (r, g, b) = rgb(if is_today { "#A5A1E2" } else if in_month { "#E2E2E6" } else { "#4A4B57" });
            cr.set_source_rgb(r, g, b);
            let l = layout(cr, &d.day_of_month().to_string(), "Inter Semi-Bold 11", 0.0);
            cr.move_to(x + 8.0, y + 6.0);
            pangocairo::functions::show_layout(cr, &l);
            let mut row = 0.0;
            let mut today_items: Vec<&Item> = items.iter().map(|(_, i)| i).filter(|i| i.day == i64::from(k)).collect();
            today_items.sort_by_key(|i| i.start);
            for it in today_items.iter().take(((ch - 30.0) / 18.0).max(0.0) as usize) {
                self.draw_block(cr, x + 4.0, y + 26.0 + row, cw - 8.0, 16.0, &format!("{} {}", clock(it.start), it.title), color_of(&it.calendar), None);
                row += 18.0;
            }
        }
    }

    fn draw_mini(&self, cr: &cairo::Context, w: f64, _h: f64) {
        let m = self.mini_month.borrow().clone();
        let first = glib::DateTime::from_local(m.year(), m.month(), 1, 0, 0, 0.0).unwrap_or(m.clone());
        let start = first.add_days(-(first.day_of_week() % 7)).unwrap_or(first);
        let t = midnight(&today());
        let cw = w / 7.0;
        for (i, n) in ["D", "S", "T", "Q", "Q", "S", "S"].iter().enumerate() {
            let (r, g, b) = rgb("#8E8F99");
            cr.set_source_rgb(r, g, b);
            let l = layout(cr, if kalango_i18n::system_locale().is_none_or(|l| l.starts_with("pt")) { n } else { ["S", "M", "T", "W", "T", "F", "S"][i] }, "Inter Semi-Bold 9", 0.0);
            let (lw, _) = l.pixel_size();
            cr.move_to(i as f64 * cw + (cw - f64::from(lw)) / 2.0, 0.0);
            pangocairo::functions::show_layout(cr, &l);
        }
        for k in 0..42 {
            let Ok(d) = start.add_days(k) else { continue };
            let (cx, cy) = (f64::from(k % 7) * cw + cw / 2.0, 38.0 + f64::from(k / 7) * 36.0);
            let is_today = d.year() == t.year() && d.day_of_year() == t.day_of_year();
            if is_today {
                let (r, g, b) = rgb("#A5A1E2");
                cr.set_source_rgb(r, g, b);
                cr.arc(cx, cy, 15.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            let (r, g, b) = rgb(if is_today { "#1C1D24" } else if d.month() == m.month() { "#E2E2E6" } else { "#4A4B57" });
            cr.set_source_rgb(r, g, b);
            let l = layout(cr, &d.day_of_month().to_string(), if is_today { "Inter Bold 10" } else { "Inter 10" }, 0.0);
            let (lw, lh) = l.pixel_size();
            cr.move_to(cx - f64::from(lw) / 2.0, cy - f64::from(lh) / 2.0);
            pangocairo::functions::show_layout(cr, &l);
        }
    }

    fn mini_click(self: &Rc<Self>, x: f64, y: f64) {
        let w = f64::from(self.mini.width());
        let (col, row) = ((x / (w / 7.0)) as i32, ((y - 20.0) / 36.0) as i32);
        if !(0..7).contains(&col) || !(0..6).contains(&row) {
            return;
        }
        let m = self.mini_month.borrow().clone();
        let first = glib::DateTime::from_local(m.year(), m.month(), 1, 0, 0, 0.0).unwrap_or(m.clone());
        let start = first.add_days(-(first.day_of_week() % 7)).unwrap_or(first);
        if let Ok(d) = start.add_days(row * 7 + col) {
            *self.anchor.borrow_mut() = d;
            self.reload();
        }
    }

    // ------------------------------------------------------------------------------------------- making and moving

    /// The day and minute under a point of the grid (snapped to 15 min).
    fn at(&self, x: f64, y: f64) -> Option<(i64, u32)> {
        let cw = self.col_w(f64::from(self.grid.width()));
        if x < HOURS_COL {
            return None;
        }
        let day = (((x - HOURS_COL) / cw) as i64).clamp(0, self.days() - 1);
        let min = ((y / HOUR * 60.0) as u32 / SNAP * SNAP).min(24 * 60 - SNAP);
        Some((day, min))
    }

    fn item_at(&self, x: f64, y: f64) -> Option<usize> {
        let cw = self.col_w(f64::from(self.grid.width()));
        self.visible_items().into_iter().rev().find(|(_, it)| {
            let (bx, by, bw, bh) = self.block_rect(cw, it);
            x >= bx && x <= bx + bw && y >= by && y <= by + bh
        }).map(|(i, _)| i)
    }

    fn drag_begin(self: &Rc<Self>, x: f64, y: f64) {
        if self.view.get() == View::Month {
            return;
        }
        let Some((day, min)) = self.at(x, y) else { return };
        if let Some(idx) = self.item_at(x, y) {
            let it = self.items.borrow()[idx].clone();
            *self.drag.borrow_mut() = Some(Drag::Move { idx, day, start: it.start, offset: min.saturating_sub(it.start), moved: false });
        } else {
            *self.drag.borrow_mut() = Some(Drag::Create { day, from: min, to: min + SNAP * 2 });
            *self.born.borrow_mut() = Some((day, min, glib::monotonic_time()));
            let w = Rc::downgrade(self);
            self.grid.add_tick_callback(move |g, _| {
                g.queue_draw();
                let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
                let going = a.born.borrow().as_ref().is_some_and(|(_, _, t0)| glib::monotonic_time() - t0 < 200_000);
                if going { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
            });
        }
        self.grid.queue_draw();
    }

    fn drag_update(&self, x: f64, y: f64) {
        let Some((day, min)) = self.at(x, y) else { return };
        let mut d = self.drag.borrow_mut();
        match d.as_mut() {
            Some(Drag::Create { to, .. }) => *to = min,
            Some(Drag::Move { idx, day: dd, start, offset, moved }) => {
                let it = &self.items.borrow()[*idx];
                let len = it.end - it.start;
                let s = min.saturating_sub(*offset).min(24 * 60 - len);
                if s != it.start || day != it.day {
                    *moved = true;
                }
                *dd = day;
                *start = s;
            }
            None => {}
        }
        drop(d);
        self.grid.queue_draw();
    }

    fn drag_end(self: &Rc<Self>, x: f64, y: f64) {
        self.drag_update(x, y);
        let d = self.drag.borrow_mut().take();
        self.born.borrow_mut().take();
        match d {
            Some(Drag::Create { day, from, to }) => {
                let (s, e) = (from.min(to), from.max(to).max(from.min(to) + SNAP));
                self.ask_title(day, s, e);
            }
            Some(Drag::Move { idx, day, start, moved: true, .. }) => self.move_item(idx, day, start),
            Some(Drag::Move { idx, moved: false, .. }) => self.show_item(idx),
            None => {}
        }
        self.grid.queue_draw();
    }

    fn date_of(&self, day: i64) -> (i32, i32, i32) {
        let d = self.first_day().add_days(day as i32).unwrap_or_else(|_| today());
        (d.year(), d.month(), d.day_of_month())
    }

    fn point_popover(&self, day: i64, start: u32, end: u32) {
        let cw = self.col_w(f64::from(self.grid.width()));
        let it = Item { day, start, end, title: String::new(), calendar: String::new(), source: Source::Routine(0) };
        let (x, y, w, h) = self.block_rect(cw, &it);
        self.pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, w as i32, h as i32)));
    }

    fn clear_pop(&self) {
        while let Some(c) = self.pop_box.first_child() {
            self.pop_box.remove(&c);
        }
    }

    /// The new event's name, typed where it was drawn; Enter keeps it (in "Pessoal").
    fn ask_title(self: &Rc<Self>, day: i64, start: u32, end: u32) {
        self.clear_pop();
        let when = rotulo(&format!("{} – {}", clock(start), clock(end)), "dsc-modelo");
        when.set_xalign(0.0);
        self.pop_box.append(&when);
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(tr("Event name")));
        e.set_width_chars(24);
        self.pop_box.append(&e);
        let cal = gtk::DropDown::from_strings(&agenda::OWN.iter().map(|(_, n)| tr(n)).collect::<Vec<_>>());
        self.pop_box.append(&cal);
        let w = Rc::downgrade(self);
        let (pop, cal2) = (self.pop.clone(), cal.clone());
        e.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            let Some(a) = w.upgrade() else { return };
            pop.popdown();
            if t.is_empty() {
                return;
            }
            let id = agenda::OWN.get(cal2.selected() as usize).map(|(id, _)| *id).unwrap_or("pessoal");
            match agenda::add(id, &t, a.date_of(day), start, end) {
                Ok(uid) => {
                    if debug() {
                        let (y, m, d) = a.date_of(day);
                        eprintln!("debug calendario: evento criado «{t}» {y:04}-{m:02}-{d:02} {}–{} em {id} ({uid})", clock(start), clock(end));
                    }
                    a.reload();
                }
                Err(err) => a.status.set_text(&trf("The event could not be saved: {error}", &[("error", err.to_string())])),
            }
        });
        self.point_popover(day, start, end);
        self.pop.popup();
        e.grab_focus();
        if debug() {
            eprintln!("debug calendario: novo bloco {}–{}", clock(start), clock(end));
        }
    }

    fn move_item(self: &Rc<Self>, idx: usize, day: i64, start: u32) {
        let Some(it) = self.items.borrow().get(idx).cloned() else { return };
        let end = start + (it.end - it.start);
        let date = self.date_of(day);
        let r = match &it.source {
            Source::File(f, uid, _) => agenda::move_to(f, uid, date, start, end).map_err(|e| e.to_string()).and_then(|ok| if ok { Ok(()) } else { Err(tr("this calendar is read only").to_string()) }),
            Source::Routine(id) => {
                let mut rs = Rotinas::carregar();
                let wd = self.first_day().add_days(day as i32).map(|d| d.day_of_week() as u8).unwrap_or(1);
                if let Some(r) = rs.lista.iter_mut().find(|r| r.id == *id) {
                    if let Gatilho::Horario { dias, hora, minuto } = &mut r.gatilho {
                        *hora = (start / 60) as u8;
                        *minuto = (start % 60) as u8;
                        // (one day of the week: it goes to the new day; every day or several: only the time changes)
                        if dias.len() == 1 {
                            *dias = vec![wd];
                        }
                    }
                }
                rs.salvar();
                if let Some(ag) = self.shell.upgrade().and_then(|s| s.agendador()) {
                    for f in ag.ao_mudar.borrow().iter() {
                        f();
                    }
                }
                Ok(())
            }
        };
        match r {
            Ok(()) => {
                if debug() {
                    eprintln!("debug calendario: movido «{}» para {:04}-{:02}-{:02} {}", it.title, date.0, date.1, date.2, clock(start));
                }
                self.status.set_text("");
            }
            Err(e) => self.status.set_text(&trf("It could not be moved: {error}", &[("error", e)])),
        }
        self.reload();
    }

    /// An event's details: when, which calendar, and (for one of KalangoOS's) delete; a routine opens its page.
    fn show_item(self: &Rc<Self>, idx: usize) {
        let Some(it) = self.items.borrow().get(idx).cloned() else { return };
        self.clear_pop();
        let t = rotulo(&it.title, "dsc-confirmar-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(28);
        self.pop_box.append(&t);
        let (y, m, d) = self.date_of(it.day);
        let when = rotulo(&format!("{d:02}/{m:02}/{y} · {} – {}", clock(it.start), clock(it.end)), "dsc-modelo");
        when.set_xalign(0.0);
        self.pop_box.append(&when);
        let cal_name = match it.calendar.as_str() {
            ROUTINES => tr("Agents' routines").to_string(),
            c => agenda::OWN.iter().find(|(id, _)| *id == c).map(|(_, n)| tr(n).to_string()).unwrap_or_else(|| agenda::name_of(std::path::Path::new(c)).unwrap_or_else(|| tr("Local calendar").to_string())),
        };
        let cl = rotulo(&cal_name, "dsc-modelo");
        cl.set_xalign(0.0);
        self.pop_box.append(&cl);
        match &it.source {
            Source::File(f, uid, _) if f.starts_with(agenda::own_dir()) => {
                // its name, editable here (Enter keeps it)
                let name = gtk::Entry::new();
                name.set_text(&it.title);
                let (w, f2, uid2, pop2) = (Rc::downgrade(self), f.clone(), uid.clone(), self.pop.clone());
                name.connect_activate(move |e| {
                    let t = e.text().trim().to_string();
                    pop2.popdown();
                    if let (Some(a), false) = (w.upgrade(), t.is_empty()) {
                        if agenda::rename(&f2, &uid2, &t).unwrap_or(false) && debug() {
                            eprintln!("debug calendario: renomeado «{t}»");
                        }
                        a.reload();
                    }
                });
                self.pop_box.append(&name);
                let del = gtk::Button::with_label(tr("Delete"));
                del.add_css_class("dsc-perigo");
                crate::central::posicao_debug_sempre("calendario apagar", del.upcast_ref());
                let (w, f, uid, title, pop) = (Rc::downgrade(self), f.clone(), uid.clone(), it.title.clone(), self.pop.clone());
                del.connect_clicked(move |_| {
                    pop.popdown();
                    if let Some(a) = w.upgrade() {
                        if agenda::remove(&f, &uid).unwrap_or(false) && debug() {
                            eprintln!("debug calendario: apagado «{title}»");
                        }
                        a.reload();
                    }
                });
                self.pop_box.append(&del);
            }
            Source::Routine(_) => {
                let open = gtk::Button::with_label(tr("Open in the agents center"));
                open.add_css_class("dsc-acao");
                let (w, pop) = (Rc::downgrade(self), self.pop.clone());
                open.connect_clicked(move |_| {
                    pop.popdown();
                    if let Some(s) = w.upgrade().and_then(|a| a.shell.upgrade()) {
                        s.abrir_central(Some("rotinas"));
                    }
                });
                self.pop_box.append(&open);
            }
            _ => {}
        }
        self.point_popover(it.day, it.start, it.end);
        self.pop.popup();
        if debug() {
            eprintln!("debug calendario: detalhes «{}»", it.title);
        }
    }

    // ------------------------------------------------------------------------------------------- the agent

    /// Today's (or tomorrow's) meeting that has no agenda yet.
    fn notice_target(&self) -> Option<(String, (i32, i32, i32))> {
        let t = midnight(&today());
        for k in 0..2 {
            let Ok(d) = t.add_days(k) else { continue };
            let date = (d.year(), d.month(), d.day_of_month());
            for e in agenda::day(date.0, date.1, date.2) {
                let low = e.title.to_lowercase();
                let meeting = ["reunião", "reuniao", "meeting", "1:1", "call", "alinhamento"].iter().any(|w| low.contains(w));
                if meeting && !agenda_file(&e.title, date).exists() && !self.dismissed.borrow().contains(&format!("{}-{date:?}", e.title)) {
                    return Some((e.title, date));
                }
            }
        }
        None
    }

    fn notice_key(&self) -> Option<String> {
        self.notice_target().map(|(t, d)| format!("{t}-{d:?}"))
    }

    fn watch(&self) {
        match self.notice_target() {
            Some((title, date)) => {
                let t = midnight(&today());
                let when = if date == (t.year(), t.month(), t.day_of_month()) { tr("today") } else { tr("tomorrow") };
                self.notice_text.set_text(&trf("“{title}” {when} has no agenda yet. Shall I draft one from this week's calendar?",
                    &[("title", title.clone()), ("when", when.to_string())]));
                self.notice.set_visible(true);
                if debug() {
                    eprintln!("debug calendario: o agente notou «{title}» sem pauta");
                }
            }
            None => self.notice.set_visible(false),
        }
    }

    /// The agenda drafted by the local model from the meeting and the week's events, saved and opened in the Editor.
    fn draft_agenda(self: &Rc<Self>) {
        let Some((title, date)) = self.notice_target() else { return };
        let week: Vec<String> = self.items.borrow().iter().map(|i| {
            let (y, m, d) = self.date_of(i.day);
            format!("{d:02}/{m:02}/{y} {} {}", clock(i.start), i.title)
        }).collect();
        let input = serde_json::json!({"reuniao": title, "data": format!("{:04}-{:02}-{:02}", date.0, date.1, date.2), "semana": week});
        self.notice_text.set_text(tr("The local model is drafting the agenda…"));
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || ask_agent("calendario-pauta", &input), move |r| {
            let Some(a) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    let text = v["pauta"].as_str().unwrap_or("").to_string();
                    let f = agenda_file(&title, date);
                    if let Some(d) = f.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    match std::fs::write(&f, format!("# {}\n\n{}\n", trf("Agenda: {title}", &[("title", title.clone())]), text)) {
                        Ok(()) => {
                            if debug() {
                                eprintln!("debug calendario: pauta salva «{}» ({} caracteres)", f.display(), text.chars().count());
                            }
                            a.notice.set_visible(false);
                            a.status.set_text(&trf("Agenda saved in {path}.", &[("path", f.display().to_string())]));
                            if let Some(s) = a.shell.upgrade() {
                                crate::editor::open(&s, Some(f.clone()));
                            }
                        }
                        Err(e) => a.notice_text.set_text(&e.to_string()),
                    }
                }
                Err(e) => a.notice_text.set_text(&trf("The agent could not answer: {error}", &[("error", e)])),
            }
        });
    }

    /// A request ("marque dentista quinta às 14h", "mova a academia para as 19h"): an event made, moved or deleted.
    fn request(self: &Rc<Self>, text: String) {
        let t = today();
        let listed: Vec<serde_json::Value> = self.items.borrow().iter().enumerate().map(|(i, it)| {
            let (y, m, d) = self.date_of(it.day);
            serde_json::json!({"id": i, "titulo": it.title, "data": format!("{y:04}-{m:02}-{d:02}"), "inicio": clock(it.start), "fim": clock(it.end),
                "agenda": it.calendar})
        }).collect();
        let input = serde_json::json!({"pedido": text, "hoje": t.format("%Y-%m-%d (%A)").map(|s| s.to_string()).unwrap_or_default(), "eventos": listed});
        self.status.set_text(tr("The agent is reading the request…"));
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || ask_agent("calendario-pedido", &input), move |r| {
            let Some(a) = w.upgrade() else { return };
            a.status.set_text("");
            let v = match r {
                Ok(v) => v,
                Err(e) => return a.status.set_text(&trf("The agent could not answer: {error}", &[("error", e)])),
            };
            let act = v["acao"].as_str().unwrap_or("responder");
            let minutes = |k: &str| v[k].as_str().and_then(|s| s.split_once(':')).and_then(|(h, m)| Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?));
            let date = resolve_day(v["dia"].as_str().unwrap_or("data"), v["data"].as_str().unwrap_or(""), minutes("inicio"), &today());
            if debug() {
                eprintln!("debug calendario: pedido → {act} «{}» {:?} {:?}", v["titulo"].as_str().unwrap_or(""), date, minutes("inicio"));
            }
            match act {
                "criar" => {
                    let (Some(date), Some(s)) = (date, minutes("inicio")) else { return a.status.set_text(tr("Say the day and the time.")) };
                    let e = minutes("fim").filter(|e| *e > s).unwrap_or(s + 60);
                    let cal = if v["agenda"] == "trabalho" { "trabalho" } else { "pessoal" };
                    let title = v["titulo"].as_str().unwrap_or("").to_string();
                    match agenda::add(cal, &title, date, s, e) {
                        Ok(_) => {
                            if debug() {
                                eprintln!("debug calendario: evento criado «{title}» {:04}-{:02}-{:02} {}–{} em {cal} (pedido)", date.0, date.1, date.2, clock(s), clock(e));
                            }
                            if let Ok(d) = glib::DateTime::from_local(date.0, date.1, date.2, 12, 0, 0.0) {
                                *a.anchor.borrow_mut() = d;
                            }
                            a.status.set_text(&trf("“{title}” is on the calendar.", &[("title", title)]));
                            a.reload();
                        }
                        Err(err) => a.status.set_text(&err.to_string()),
                    }
                }
                "mover" | "apagar" => {
                    let Some(idx) = v["evento"].as_u64().map(|x| x as usize).filter(|i| *i < a.items.borrow().len()) else {
                        return a.status.set_text(tr("Which event? It is not in view."));
                    };
                    if act == "apagar" {
                        let it = a.items.borrow()[idx].clone();
                        if let Source::File(f, uid, _) = &it.source {
                            if agenda::remove(f, uid).unwrap_or(false) {
                                a.status.set_text(&trf("“{title}” deleted.", &[("title", it.title)]));
                            }
                        }
                        a.reload();
                    } else if let Some(s) = minutes("inicio") {
                        let day = date.and_then(|d| (0..a.days()).find(|k| a.date_of(*k) == d)).unwrap_or(a.items.borrow()[idx].day);
                        a.move_item(idx, day, s);
                    }
                }
                _ => a.status.set_text(v["texto"].as_str().unwrap_or("")),
            }
        });
    }
}

/// The date a request means: "hoje", "amanha", a weekday (today's when its time is still ahead, else the next one),
/// or an explicit YYYY-MM-DD ("data").
fn resolve_day(dia: &str, data: &str, start: Option<u32>, now: &glib::DateTime) -> Option<(i32, i32, i32)> {
    let ymd = |d: &glib::DateTime| (d.year(), d.month(), d.day_of_month());
    let wd = ["segunda", "terca", "quarta", "quinta", "sexta", "sabado", "domingo"];
    match dia {
        "hoje" => Some(ymd(now)),
        "amanha" => now.add_days(1).ok().map(|d| ymd(&d)),
        w if wd.contains(&w) => {
            let target = wd.iter().position(|x| *x == w)? as i32 + 1;
            let mut ahead = (target - now.day_of_week()).rem_euclid(7);
            let now_min = (now.hour() * 60 + now.minute()) as u32;
            if ahead == 0 && start.is_some_and(|s| s <= now_min) {
                ahead = 7;
            }
            now.add_days(ahead).ok().map(|d| ymd(&d))
        }
        _ => {
            let p: Vec<i32> = data.split('-').filter_map(|x| x.parse().ok()).collect();
            (p.len() == 3).then(|| (p[0], p[1], p[2]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_a_request_means() {
        // Thursday 19 Nov 2026, 05:00
        let now = glib::DateTime::from_local(2026, 11, 19, 5, 0, 0.0).unwrap();
        assert_eq!(resolve_day("quinta", "", Some(14 * 60), &now), Some((2026, 11, 19)));
        assert_eq!(resolve_day("quinta", "", Some(4 * 60), &now), Some((2026, 11, 26)));
        assert_eq!(resolve_day("segunda", "", None, &now), Some((2026, 11, 23)));
        assert_eq!(resolve_day("amanha", "", None, &now), Some((2026, 11, 20)));
        assert_eq!(resolve_day("data", "2026-12-24", None, &now), Some((2026, 12, 24)));
    }
}

/// Where a meeting's agenda is kept: ~/Documentos/Pautas/<meeting> <date>.md.
fn agenda_file(title: &str, (y, m, d): (i32, i32, i32)) -> PathBuf {
    crate::pastas::documentos().join(tr("Agendas")).join(format!("{} {y:04}-{m:02}-{d:02}.md", title.replace(['/', '\\'], "-")))
}

fn ask_agent(cmd: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let mut c = std::process::Command::new(crate::voz::agent_binary()).arg(cmd).stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    c.stdin.take().ok_or("stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
}
