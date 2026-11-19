//! KalangoOS · prova de conceito
//!
//! Duas superfícies do *sistema* (não são janelas de app) desenhadas em Rust + GTK4 e presas às bordas da tela
//! pelo protocolo layer-shell do Wayland:
//!   * barra superior (32 px, reserva espaço);
//!   * barra lateral direita com três modos — Avisos, Controles, Agente.
//!
//! O movimento não vem do CSS: é uma mola integrada a cada quadro, com os mesmos números do Design System
//! (suave 300/30, rápida 520/38). O estilo usa só os tokens exportados do Design System.
//!
//! Uso:  kalango-sidebar-poc [open avisos|controles|agente] [close] [toggle] [theme] [quit]
//! Uma segunda chamada fala com a instância que já está rodando (como um atalho de teclado faria).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

const TOKENS_DARK: &str = include_str!("../assets/tokens-dark.css");
const TOKENS_LIGHT: &str = include_str!("../assets/tokens-light.css");
const STYLE: &str = include_str!("../assets/style.css");

const PANEL_W: i32 = 400; // largura da barra (token layout)
const SLACK: i32 = 32; // folga à esquerda para a mola poder passar do ponto
const MARGIN: i32 = 8; // distância da borda quando não está fixada
const SURFACE_W: i32 = SLACK + PANEL_W + MARGIN;
const STAGGER_MS: f64 = 24.0; // coreografia/cascata

/// Mola amortecida (massa 1). Mesmos parâmetros da coleção "Motion" do Design System.
#[derive(Clone, Copy)]
struct Spring {
    x: f64,
    v: f64,
    target: f64,
    k: f64,
    c: f64,
}

impl Spring {
    const SUAVE: (f64, f64) = (300.0, 30.0);
    const RAPIDA: (f64, f64) = (520.0, 38.0);

    fn new(x: f64) -> Self {
        Spring { x, v: 0.0, target: x, k: Self::SUAVE.0, c: Self::SUAVE.1 }
    }
    fn go(&mut self, target: f64, (k, c): (f64, f64)) {
        self.target = target;
        self.k = k;
        self.c = c;
    }
    fn step(&mut self, dt: f64) {
        let n = (dt / 0.004).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            let a = -self.k * (self.x - self.target) - self.c * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
        if (self.x - self.target).abs() < 0.0005 && self.v.abs() < 0.0005 {
            self.x = self.target;
            self.v = 0.0;
        }
    }
    fn resting(&self) -> bool {
        self.x == self.target && self.v == 0.0
    }
}

struct Motion {
    open: bool,
    panel: Spring,          // 0 = fora da tela, 1 = no lugar
    items: Vec<Spring>,     // opacidade de cada bloco (cascata)
    opened_at: Option<i64>, // tempo do quadro em que abriu (µs)
    last: i64,
    height: i32,
    trace: bool,
}

#[derive(Clone)]
struct Ui {
    layer: bool, // true: superfície do sistema (layer-shell) · false: pré-visualização numa janela comum
    win: gtk::ApplicationWindow,
    fixed: gtk::Fixed,
    panel: gtk::Box,
    handle: gtk::Box,
    stack: gtk::Stack,
    tabs: [gtk::ToggleButton; 3],
    sections: Rc<Vec<gtk::Widget>>,
    tokens: gtk::CssProvider,
    light: Rc<RefCell<bool>>,
    motion: Rc<RefCell<Motion>>,
    ticking: Rc<Cell<bool>>, // há um relógio de quadros ligado? (parada, a barra não acorda a CPU)
}

const MODES: [&str; 3] = ["avisos", "controles", "agente"];

impl Ui {
    /// Liga o relógio de quadros só enquanto algo se move. Em repouso a barra não consome CPU.
    fn ensure_tick(&self) {
        if self.ticking.replace(true) {
            return;
        }
        let u = self.clone();
        self.fixed.add_tick_callback(move |_, clock| {
            if u.tick(clock.frame_time()) {
                glib::ControlFlow::Continue
            } else {
                u.ticking.set(false);
                u.motion.borrow_mut().last = 0;
                glib::ControlFlow::Break
            }
        });
    }

    fn set_mode(&self, mode: &str) {
        if let Some(i) = MODES.iter().position(|m| *m == mode) {
            self.tabs[i].set_active(true);
            self.stack.set_visible_child_name(mode);
        }
    }

    fn open(&self, mode: Option<&str>) {
        if let Some(m) = mode {
            self.set_mode(m);
        }
        let mut st = self.motion.borrow_mut();
        if !st.open {
            st.open = true;
            st.opened_at = None;
            st.panel.go(1.0, Spring::SUAVE);
            if self.layer {
                self.win.set_keyboard_mode(KeyboardMode::OnDemand);
            }
            self.input_region(true, st.height);
        }
        drop(st);
        self.ensure_tick();
    }

    fn close(&self) {
        let mut st = self.motion.borrow_mut();
        if st.open {
            st.open = false;
            st.panel.go(0.0, Spring::RAPIDA);
            for s in st.items.iter_mut() {
                s.go(0.0, Spring::RAPIDA);
            }
            if self.layer {
                self.win.set_keyboard_mode(KeyboardMode::None);
            }
            self.input_region(false, st.height);
        }
        drop(st);
        self.ensure_tick();
    }

    fn toggle(&self) {
        let open = self.motion.borrow().open;
        if open {
            self.close()
        } else {
            self.open(None)
        }
    }

    fn toggle_theme(&self) {
        let mut light = self.light.borrow_mut();
        *light = !*light;
        self.tokens.load_from_string(if *light { TOKENS_LIGHT } else { TOKENS_DARK });
    }

    /// Fechada, a superfície só recebe cliques na alça da borda: o resto passa para as janelas de baixo.
    fn input_region(&self, open: bool, h: i32) {
        if !self.layer {
            return;
        }
        if let Some(surface) = self.win.surface() {
            let rect = if open {
                gtk::cairo::RectangleInt::new(SLACK, 0, PANEL_W + MARGIN, h.max(1))
            } else {
                gtk::cairo::RectangleInt::new(SURFACE_W - 12, (h / 2 - 60).max(0), 12, 120)
            };
            surface.set_input_region(&gtk::cairo::Region::create_rectangle(&rect));
        }
    }

    fn command(&self, args: &[String]) {
        let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        match a.as_slice() {
            ["open", mode, ..] => self.open(Some(mode)),
            ["open"] | [] => self.open(Some("controles")),
            ["close", ..] => self.close(),
            ["toggle", ..] => self.toggle(),
            ["theme", ..] => self.toggle_theme(),
            ["quit", ..] => {
                if let Some(app) = self.win.application() {
                    app.quit();
                }
            }
            _ => eprintln!("uso: kalango-sidebar-poc [open avisos|controles|agente | close | toggle | theme]"),
        }
    }

    /// Um quadro: avança as molas e aplica posição e opacidade. Devolve `false` quando tudo parou.
    fn tick(&self, now: i64) -> bool {
        let mut st = self.motion.borrow_mut();
        let dt = if st.last == 0 { 1.0 / 60.0 } else { ((now - st.last) as f64 / 1e6).clamp(0.0, 0.05) };
        st.last = now;

        let h = self.fixed.height();
        if h > 0 && h != st.height {
            st.height = h;
            self.panel.set_size_request(PANEL_W, h);
            self.fixed.move_(&self.handle, (SURFACE_W - 5) as f64, (h / 2 - 48) as f64);
            self.input_region(st.open, h);
        }

        if st.open && st.opened_at.is_none() {
            st.opened_at = Some(now);
        }
        if let Some(t0) = st.opened_at {
            let elapsed_ms = (now - t0) as f64 / 1e3;
            if st.open {
                for (i, s) in st.items.iter_mut().enumerate() {
                    if s.target == 0.0 && elapsed_ms >= 90.0 + i as f64 * STAGGER_MS {
                        s.go(1.0, Spring::SUAVE);
                    }
                }
            }
        }

        let moving = !st.panel.resting() || st.items.iter().any(|s| !s.resting());
        st.panel.step(dt);
        for s in st.items.iter_mut() {
            s.step(dt);
        }
        let p = st.panel.x;
        if moving && st.trace {
            eprintln!("mola {:>7.1} ms  p={:.4}", st.opened_at.map(|t| (now - t) as f64 / 1e3).unwrap_or(0.0), p);
        }

        let x = SLACK as f64 + (1.0 - p) * (PANEL_W + MARGIN + 8) as f64;
        let t = gsk::Transform::new().translate(&graphene::Point::new(x as f32, 0.0));
        self.fixed.set_child_transform(&self.panel, Some(&t));
        self.panel.set_opacity(p.clamp(0.0, 1.0));
        self.handle.set_opacity((1.0 - p).clamp(0.0, 1.0));
        for (w, s) in self.sections.iter().zip(st.items.iter()) {
            w.set_opacity(s.x.clamp(0.0, 1.0));
        }
        let pending = st.open && st.items.iter().any(|s| s.target == 0.0);
        pending || st.height == 0 || !st.panel.resting() || st.items.iter().any(|s| !s.resting())
    }
}

// ------------------------------------------------------------------ pequenos construtores de interface
fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    for c in class.split_whitespace() {
        l.add_css_class(c);
    }
    l
}

fn wrap_label(text: &str, class: &str) -> gtk::Label {
    let l = label(text, class);
    l.set_wrap(true);
    l.set_max_width_chars(42);
    l
}

fn vbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

fn hbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

fn icon(name: &str) -> gtk::Image {
    gtk::Image::from_icon_name(name)
}

fn icon_btn(name: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::builder().child(&icon(name)).build();
    b.add_css_class("icon-btn");
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

fn btn(text: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("btn");
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

fn two_lines(title: &str, sub: &str) -> gtk::Box {
    let t = vbox(1, "");
    t.set_valign(gtk::Align::Center);
    t.set_hexpand(true);
    t.append(&label(title, "strong"));
    t.append(&label(sub, "muted"));
    t
}

fn row(icon_name: &str, title: &str, sub: &str, trail: &str) -> gtk::Box {
    let r = hbox(12, "row");
    r.append(&icon(icon_name));
    r.append(&two_lines(title, sub));
    if trail == ">" {
        r.append(&icon("go-next-symbolic"));
    } else if !trail.is_empty() {
        r.append(&label(trail, "muted"));
    }
    r
}

fn qtile(icon_name: &str, title: &str, sub: &str, on: bool) -> gtk::ToggleButton {
    let inner = hbox(10, "");
    inner.append(&icon(icon_name));
    let t = vbox(0, "");
    t.set_valign(gtk::Align::Center);
    t.append(&label(title, "t"));
    t.append(&label(sub, "s"));
    inner.append(&t);
    let b = gtk::ToggleButton::builder().child(&inner).active(on).hexpand(true).build();
    b.add_css_class("qtile");
    b
}

fn segmented(labels: &[&str], active: usize) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let seg = hbox(2, "seg");
    seg.set_homogeneous(true);
    seg.set_hexpand(true);
    let mut out: Vec<gtk::ToggleButton> = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        let b = gtk::ToggleButton::with_label(l);
        if let Some(first) = out.first() {
            b.set_group(Some(first));
        }
        b.set_active(i == active);
        seg.append(&b);
        out.push(b);
    }
    (seg, out)
}

fn slider(icon_name: &str, value: f64, class: &str) -> gtk::Box {
    let r = hbox(10, "");
    r.append(&icon(icon_name));
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    s.set_value(value);
    s.set_hexpand(true);
    if !class.is_empty() {
        s.add_css_class(class);
    }
    let pct = label(&format!("{value:.0}%"), "muted");
    let pct2 = pct.clone();
    s.connect_value_changed(move |s| pct2.set_label(&format!("{:.0}%", s.value())));
    r.append(&s);
    r.append(&pct);
    r
}

// ------------------------------------------------------------------ os três modos
fn page_controls(sections: &mut Vec<gtk::Widget>) -> gtk::Box {
    let page = vbox(12, "");

    let user = hbox(10, "");
    let av = label("R", "avatar");
    av.set_xalign(0.5);
    user.append(&av);
    user.append(&two_lines("Roo", "82% · carregando"));
    user.append(&icon_btn("emblem-system-symbolic", ""));
    user.append(&icon_btn("system-lock-screen-symbolic", ""));
    user.append(&icon_btn("system-shutdown-symbolic", "danger"));
    page.append(&user);

    let grid = gtk::Grid::builder().row_spacing(8).column_spacing(8).column_homogeneous(true).build();
    let tiles = [
        ("network-wireless-symbolic", "Wi-Fi", "Lunar-Casa_5G", true),
        ("bluetooth-active-symbolic", "Bluetooth", "Fone WH-1000", true),
        ("notifications-disabled-symbolic", "Não perturbe", "desligado", false),
        ("night-light-symbolic", "Luz noturna", "até 06:00", true),
        ("airplane-mode-symbolic", "Modo avião", "desligado", false),
        ("security-high-symbolic", "Pausar agentes", "2 ativos", false),
    ];
    for (i, (ic, t, s, on)) in tiles.iter().enumerate() {
        grid.attach(&qtile(ic, t, s, *on), (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    page.append(&grid);

    let sound = vbox(12, "kcard");
    sound.append(&slider("audio-volume-high-symbolic", 62.0, ""));
    sound.append(&slider("display-brightness-symbolic", 70.0, "gold"));
    page.append(&sound);

    let energy = vbox(8, "kcard");
    energy.append(&label("MODO DE ENERGIA", "cap"));
    energy.append(&segmented(&["Economia", "Equilibrado", "Desempenho"], 1).0);
    page.append(&energy);

    let media = hbox(10, "kcard");
    media.append(&hbox(0, "cover"));
    media.append(&two_lines("Alright", "Jonathan Ogden"));
    for ic in ["media-skip-backward-symbolic", "media-playback-pause-symbolic", "media-skip-forward-symbolic"] {
        media.append(&icon_btn(ic, "ghost"));
    }
    page.append(&media);

    let devices = vbox(2, "kcard list");
    devices.append(&row("audio-headphones-symbolic", "Fone WH-1000", "conectado · LDAC", "80%"));
    devices.append(&row("phone-symbolic", "Pixel de Roo", "continuar: docs.rs", ">"));
    devices.append(&row("video-display-symbolic", "Monitor 2", "1080 × 1920 · estendido", ">"));
    page.append(&devices);

    for w in [user.upcast::<gtk::Widget>(), grid.upcast(), sound.upcast(), energy.upcast(), media.upcast(), devices.upcast()] {
        sections.push(w);
    }
    page
}

fn page_notices() -> gtk::Box {
    let page = vbox(12, "");
    let sum = vbox(7, "kcard tint");
    sum.append(&label("12 AVISOS RESUMIDOS EM 3 PONTOS", "cap"));
    for t in ["Marina pediu a pauta da reunião das 15:00.", "Organizador terminou: 118 arquivos em 4 pastas.", "A fatura de energia vence em 20/11."] {
        sum.append(&wrap_label(&format!("•  {t}"), ""));
    }
    let a = hbox(8, "");
    a.append(&btn("Ver os 12", "tint"));
    a.append(&btn("Marcar como lidos", "ghost"));
    sum.append(&a);
    page.append(&sum);

    page.append(&label("PRECISAM DE VOCÊ", "cap"));
    for (app, when, title, body) in [
        ("Claude", "2 min", "Revisão pronta", "Terminei a revisão do lunar-shell: 3 sugestões de melhoria na dock e no relógio."),
        ("Discord", "5 min", "Roo e mais 2", "viu o novo tema? ficou muito limpo sem as sombras"),
    ] {
        let c = vbox(6, "kcard");
        let h = hbox(6, "");
        h.append(&label(app, "muted strong"));
        h.append(&label(&format!("· {when}"), "dim"));
        c.append(&h);
        c.append(&label(title, "strong"));
        c.append(&wrap_label(body, "muted"));
        page.append(&c);
    }
    let cal = gtk::Calendar::new();
    cal.add_css_class("kcard");
    page.append(&cal);
    page
}

fn page_agent() -> gtk::Box {
    let page = vbox(12, "");
    let head = hbox(8, "");
    head.append(&icon("starred-symbolic"));
    head.append(&label("Organizador", "strong"));
    head.append(&label("qwen 27B · local", "badge"));
    page.append(&head);

    let bubble = hbox(0, "bubble");
    bubble.set_halign(gtk::Align::End);
    bubble.append(&wrap_label("Organize meus Downloads por tipo e apague duplicados", ""));
    page.append(&bubble);
    page.append(&wrap_label("Encontrei 128 itens. Vou separar em PDFs, Imagens, Instaladores e Compactados, e marcar 3 duplicados para você aprovar.", ""));

    let steps = vbox(2, "kcard list");
    steps.append(&row("object-select-symbolic", "Leu ~/Downloads", "128 itens", "0,1 s"));
    steps.append(&row("folder-symbolic", "Movendo para PDFs", "42 / 118", "agora"));
    page.append(&steps);

    let ask = vbox(10, "kcard warn");
    let t = hbox(8, "");
    t.append(&icon("dialog-warning-symbolic"));
    t.append(&label("Permissão necessária", "strong"));
    let risk = label("risco alto", "badge gold");
    risk.set_hexpand(true);
    risk.set_halign(gtk::Align::End);
    t.append(&risk);
    ask.append(&t);
    ask.append(&label("rm ~/Downloads/fatura-copia.pdf\nrm ~/Downloads/foto(1).png\nrm ~/Downloads/setup-old.deb", "mono"));
    let a = hbox(8, "");
    a.append(&btn("Negar", ""));
    a.append(&btn("Permitir uma vez", "primary"));
    ask.append(&a);
    page.append(&ask);
    page
}

// ------------------------------------------------------------------ superfícies do sistema
fn build_sidebar(win: &gtk::ApplicationWindow, tokens: &gtk::CssProvider, layer: bool) -> Ui {
    let fixed = gtk::Fixed::new();
    fixed.set_size_request(SURFACE_W, -1);
    fixed.set_overflow(gtk::Overflow::Hidden);

    let panel = vbox(12, "sidebar");
    let (seg, tab_vec) = segmented(&["Avisos", "Controles", "Agente"], 1);
    let head = hbox(6, "");
    head.append(&seg);
    head.append(&icon_btn("view-pin-symbolic", "ghost"));
    let close = icon_btn("window-close-symbolic", "ghost");
    head.append(&close);
    panel.append(&head);

    let mut sections: Vec<gtk::Widget> = Vec::new();
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::SlideLeftRight).transition_duration(220).vexpand(true).build();
    stack.add_named(&page_notices(), Some("avisos"));
    stack.add_named(&page_controls(&mut sections), Some("controles"));
    stack.add_named(&page_agent(), Some("agente"));
    stack.set_visible_child_name("controles");
    panel.append(&stack);

    let foot = hbox(8, "");
    foot.append(&icon("security-high-symbolic"));
    let note = label("nada enviado à nuvem hoje", "muted");
    note.set_hexpand(true);
    foot.append(&note);
    foot.append(&btn("Todas as configurações", "ghost"));
    panel.append(&foot);

    let handle = hbox(0, "edge-handle");
    fixed.put(&panel, SURFACE_W as f64, 0.0);
    fixed.put(&handle, (SURFACE_W - 5) as f64, 300.0);

    let n = sections.len();
    let tabs: [gtk::ToggleButton; 3] = [tab_vec[0].clone(), tab_vec[1].clone(), tab_vec[2].clone()];
    let ui = Ui {
        layer,
        win: win.clone(),
        fixed: fixed.clone(),
        panel,
        handle: handle.clone(),
        stack: stack.clone(),
        tabs,
        sections: Rc::new(sections),
        tokens: tokens.clone(),
        light: Rc::new(RefCell::new(false)),
        motion: Rc::new(RefCell::new(Motion {
            open: false,
            panel: Spring::new(0.0),
            items: vec![Spring::new(0.0); n],
            opened_at: None,
            last: 0,
            height: 0,
            trace: std::env::var_os("KALANGO_POC_TRACE").is_some(),
        })),
        ticking: Rc::new(Cell::new(false)),
    };

    for (i, t) in ui.tabs.iter().enumerate() {
        let stack = stack.clone();
        t.connect_toggled(move |b| {
            if b.is_active() {
                stack.set_visible_child_name(MODES[i]);
            }
        });
    }
    let u = ui.clone();
    close.connect_clicked(move |_| u.close());

    // encostar o ponteiro na borda abre a barra
    let hover = gtk::EventControllerMotion::new();
    let u = ui.clone();
    hover.connect_enter(move |_, _, _| u.open(None));
    handle.add_controller(hover);

    // teclado: Esc fecha · 1/2/3 trocam o modo · T troca o tema
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        match key {
            gdk::Key::Escape => u.close(),
            gdk::Key::_1 => u.set_mode("avisos"),
            gdk::Key::_2 => u.set_mode("controles"),
            gdk::Key::_3 => u.set_mode("agente"),
            gdk::Key::t | gdk::Key::T => u.toggle_theme(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    win.add_controller(keys);

    // o relógio de quadros liga ao abrir, ao fechar e quando a superfície muda de tamanho
    let u = ui.clone();
    win.connect_realize(move |w| {
        if let Some(surface) = w.surface() {
            let u = u.clone();
            surface.connect_layout(move |_, _, _| u.ensure_tick());
        }
    });
    ui.ensure_tick();
    ui
}

fn build_topbar(ui: &Ui) -> gtk::CenterBox {
    let bar = gtk::CenterBox::new();
    bar.add_css_class("topbar");

    let spaces = hbox(5, "pill");
    for i in 0..4 {
        let d = hbox(0, if i == 0 { "dot on" } else { "dot" });
        d.set_valign(gtk::Align::Center);
        spaces.append(&d);
    }
    spaces.set_valign(gtk::Align::Center);
    bar.set_start_widget(Some(&spaces));

    let clock = gtk::Button::with_label("—");
    clock.add_css_class("pill");
    clock.set_valign(gtk::Align::Center);
    let set_time = {
        let clock = clock.clone();
        move || {
            if let Ok(now) = glib::DateTime::now_local() {
                if let Ok(s) = now.format("%a %d %b   %H:%M") {
                    clock.set_label(&s);
                }
            }
        }
    };
    set_time();
    glib::timeout_add_seconds_local(20, move || {
        set_time();
        glib::ControlFlow::Continue
    });
    let u = ui.clone();
    clock.connect_clicked(move |_| u.open(Some("avisos")));
    bar.set_center_widget(Some(&clock));

    let end = hbox(8, "");
    end.set_valign(gtk::Align::Center);
    let agents_in = hbox(6, "");
    agents_in.append(&icon("starred-symbolic"));
    agents_in.append(&label("2 agentes", ""));
    let agents = gtk::Button::builder().child(&agents_in).build();
    agents.add_css_class("pill");
    agents.add_css_class("agents");
    let u = ui.clone();
    agents.connect_clicked(move |_| u.open(Some("agente")));
    end.append(&agents);

    let status_in = hbox(10, "");
    for ic in ["network-wireless-symbolic", "bluetooth-active-symbolic", "audio-volume-high-symbolic", "system-shutdown-symbolic"] {
        status_in.append(&icon(ic));
    }
    let status = gtk::Button::builder().child(&status_in).build();
    status.add_css_class("pill");
    let u = ui.clone();
    status.connect_clicked(move |_| u.open(Some("controles")));
    end.append(&status);
    bar.set_end_widget(Some(&end));
    bar
}

/// Sobre Wayland com layer-shell: duas superfícies do sistema presas às bordas.
/// Sem isso (X11, ou KALANGO_POC_PREVIEW=1): a mesma interface dentro de uma janela comum, só para ver e tocar.
fn build(app: &gtk::Application, tokens: &gtk::CssProvider) -> Ui {
    let wayland = gdk::Display::default().map(|d| d.backend().is_wayland()).unwrap_or(false);
    let layer = wayland && std::env::var_os("KALANGO_POC_PREVIEW").is_none() && gtk4_layer_shell::is_supported();
    if layer {
        let side = gtk::ApplicationWindow::builder().application(app).default_width(SURFACE_W).build();
        side.init_layer_shell();
        side.set_namespace(Some("kalango-barra-lateral-direita"));
        side.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Right, Edge::Bottom] {
            side.set_anchor(edge, true);
        }
        side.set_margin(Edge::Top, MARGIN);
        side.set_margin(Edge::Bottom, MARGIN);
        side.set_keyboard_mode(KeyboardMode::None);
        let ui = build_sidebar(&side, tokens, true);
        side.set_child(Some(&ui.fixed));
        side.present();

        let top = gtk::ApplicationWindow::builder().application(app).build();
        top.init_layer_shell();
        top.set_namespace(Some("kalango-barra-superior"));
        top.set_layer(Layer::Top);
        for edge in [Edge::Top, Edge::Left, Edge::Right] {
            top.set_anchor(edge, true);
        }
        top.auto_exclusive_zone_enable();
        top.set_child(Some(&build_topbar(&ui)));
        top.present();
        ui
    } else {
        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .title("KalangoOS · prova de conceito (pré-visualização)")
            .default_width(1360)
            .default_height(820)
            .build();
        let ui = build_sidebar(&win, tokens, false);
        let overlay = gtk::Overlay::new();
        if let Some(path) = std::env::var_os("KALANGO_POC_WALLPAPER") {
            let wall = gtk::Picture::for_filename(path);
            wall.set_content_fit(gtk::ContentFit::Cover);
            overlay.set_child(Some(&wall));
        }
        let bar = build_topbar(&ui);
        bar.set_valign(gtk::Align::Start);
        overlay.add_overlay(&bar);
        ui.fixed.set_halign(gtk::Align::End);
        ui.fixed.set_margin_top(32 + MARGIN);
        ui.fixed.set_margin_bottom(MARGIN);
        overlay.add_overlay(&ui.fixed);
        win.set_child(Some(&overlay));
        win.present();
        ui
    }
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(std::env::var("KALANGO_POC_APP_ID").unwrap_or_else(|_| "os.kalango.SidebarPoc".into()))
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let ui: Rc<RefCell<Option<Ui>>> = Rc::new(RefCell::new(None));

    app.connect_command_line(move |app, cmd| {
        let first = ui.borrow().is_none();
        let wants_quit = cmd.arguments().get(1).map(|a| a == "quit" || a == "close").unwrap_or(false);
        if first && wants_quit {
            return 0; // nada rodando: não há o que fechar
        }
        if first {
            let display = gdk::Display::default().expect("sem display Wayland");
            let tokens = gtk::CssProvider::new();
            tokens.load_from_string(TOKENS_DARK);
            let style = gtk::CssProvider::new();
            style.load_from_string(STYLE);
            gtk::style_context_add_provider_for_display(&display, &tokens, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            gtk::style_context_add_provider_for_display(&display, &style, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            *ui.borrow_mut() = Some(build(app, &tokens));
        }
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
        if !(first && args.is_empty()) {
            if let Some(u) = ui.borrow().as_ref() {
                u.command(&args);
            }
        }
        0
    });
    app.run()
}
