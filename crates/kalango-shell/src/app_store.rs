//! Central de Apps (design "Central de Apps (descobrir)", 42430; app id os.kalango.Apps; `kalango-shell loja
//! [busca]`): the store treats agents and local models as first-class, beside the apps.
//! - Descobrir: the week's highlight (agents that work with you: explore them, or read how their permissions work),
//!   the category chips, and one row of each — the agents of the system's catalog, each card already saying what it
//!   reads, changes, runs and reaches and how much it decides alone; the essential apps (Flatpak, or already here by
//!   other means: then "Abrir"); the local models, each saying whether it fits this computer's graphics card now and
//!   how fast it ran here.
//! - Apps, Agentes, Modelos: the same section with all of its cards; the search reaches the whole Flatpak catalog.
//! - Atualizações: the Flatpak apps with a new version, updated one by one or all at once.
//! Installing an app is Flatpak's own (`flatpak install`, the progress in its button); an agent installs through
//! "Instalar agente" (its permissions first); a model downloads into ~/.local/share/kalango/modelos.
//!
//! Motion: the cards come in with the 24 ms cascade; hovering one lifts it 2 px (quick spring).

use crate::app_store_data::{self as data, CatalogApp, Fit, ModelEntry, ShelfEntry, Tone, Transfer};
use crate::conversa::ui_icon;
use crate::install_agent::Manifesto;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The rail: (page, glyph, tooltip).
const PAGES: &[(&str, &str, &str)] = &[
    ("descobrir", "casa", kalango_i18n::n_("Discover")),
    ("apps", "grade", kalango_i18n::n_("Apps")),
    ("agentes", "spark", kalango_i18n::n_("Agents")),
    ("modelos", "cerebro", kalango_i18n::n_("Models")),
    ("atualizacoes", "download", kalango_i18n::n_("Updates")),
];

/// The agents' icon colours, in the catalog's order (the design's teal, lilac, gold…).
const AGENT_COLOURS: &[&str] = &["#8ED2C3", "#A5A1E2", "#E5C98C", "#7CC3DF", "#EFA2A6", "#C7A7DC"];

/// The smallest card of each section (as many columns as fit), and the gap between cards.
const AGENT_MIN: i32 = 440;
const CARD_MIN: i32 = 300;
const GAP: i32 = 14;

/// An app's card: what it is and how it runs here.
#[derive(Clone)]
struct AppItem {
    id: String,
    name: String,
    kind: String,
    icon: Option<PathBuf>,
    category: String,
    /// installed: what opens it (its desktop entry), or Flatpak's run
    desktop: Option<gio::DesktopAppInfo>,
    flatpak: bool,
    /// where Flatpak installs it from: (remote, the person's own installation)
    remote: Option<(String, bool)>,
}

impl AppItem {
    fn installed(&self) -> bool {
        self.desktop.is_some() || self.flatpak
    }
}

/// Something running in the background for a card (an install, a download, an update).
#[derive(Clone)]
struct Job {
    /// 0…1000
    permille: Arc<std::sync::atomic::AtomicU64>,
    finished: Arc<std::sync::atomic::AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    transfer: Option<Transfer>,
}

impl Job {
    fn new() -> Job {
        Job { permille: Arc::default(), finished: Arc::default(), error: Arc::default(), transfer: None }
    }
    fn fraction(&self) -> f64 {
        match &self.transfer {
            Some(t) if t.total > 0 => t.done.load(Ordering::Relaxed) as f64 / t.total as f64,
            _ => self.permille.load(Ordering::Relaxed) as f64 / 1000.0,
        }
    }
}

/// A section: its title row ("ver todos") and its cards.
struct Section {
    root: gtk::Box,
    title: gtk::Label,
    more: gtk::Button,
    grid: gtk::FlowBox,
    min: i32,
}

impl Section {
    fn new(title: &str, min: i32, key: &str) -> Section {
        let root = caixa(Orientation::Vertical, 18, "");
        let head = caixa(Orientation::Horizontal, 8, "");
        let t = rotulo(title, "loja-secao");
        t.set_xalign(0.0);
        t.set_hexpand(true);
        head.append(&t);
        let more = gtk::Button::with_label(tr("see all"));
        more.add_css_class("loja-ver-todos");
        more.set_valign(Align::Center);
        crate::central::posicao_debug_sempre(&format!("loja ver todos {key}"), more.upcast_ref());
        head.append(&more);
        root.append(&head);
        let grid = gtk::FlowBox::new();
        grid.add_css_class("loja-grade");
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.set_homogeneous(true);
        grid.set_column_spacing(GAP as u32);
        grid.set_row_spacing(GAP as u32);
        grid.set_valign(Align::Start);
        root.append(&grid);
        Section { root, title: t, more, grid, min }
    }

    fn clear(&self) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
    }

    /// How many cards fit side by side in `width`.
    fn columns(&self, width: i32) -> u32 {
        (((width + GAP) / (self.min + GAP)).max(1)) as u32
    }
}

pub struct AppStore {
    window: gtk::ApplicationWindow,
    shell: Weak<Shell>,
    rail: Vec<(gtk::Button, gtk::Image)>,
    search: gtk::Entry,
    hero: gtk::Box,
    hero_art: gtk::Image,
    chips_row: gtk::Box,
    chips: Vec<gtk::ToggleButton>,
    agents: Section,
    apps: Section,
    models: Section,
    downloads: Rc<crate::modelos_fonte::Downloads>,
    updates: Section,
    system_updates: gtk::Box,
    empty: gtk::Label,
    page: Cell<&'static str>,
    category: Cell<usize>,
    /// the content's width (the page's padding off): the columns follow it
    width: Cell<i32>,
    catalog: RefCell<Rc<Vec<CatalogApp>>>,
    catalog_state: Cell<u8>,
    shelf: RefCell<Vec<ShelfEntry>>,
    agent_list: RefCell<Vec<(PathBuf, Manifesto)>>,
    model_list: RefCell<Vec<ModelEntry>>,
    flatpaks: RefCell<HashSet<String>>,
    user_remotes: RefCell<HashSet<String>>,
    /// the Flatpak apps with a new version (None: not looked yet)
    pending_updates: RefCell<Option<Vec<String>>>,
    jobs: RefCell<HashMap<String, Job>>,
    /// the progress bars and texts on screen for each job
    bars: RefCell<HashMap<String, (glib::WeakRef<gtk::LevelBar>, glib::WeakRef<gtk::Label>)>>,
    ticking: Cell<bool>,
    refresh_pending: RefCell<Option<glib::SourceId>>,
    /// Apps criativos: a coleção, o detalhe de um app, a parte deles nas Atualizações, a chamada (criativos.rs)
    criativos: Rc<crate::criativos::Criativos>,
}

thread_local! {
    static STORE: RefCell<Option<Rc<AppStore>>> = const { RefCell::new(None) };
}

/// Opens the store (with a search already typed, from `kalango-shell loja TEXT`).
pub fn open(shell: &Rc<Shell>, query: Option<String>) {
    let s = STORE.with(|s| s.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let s = AppStore::new(&app, shell);
        STORE.with(|x| *x.borrow_mut() = Some(s.clone()));
        s
    });
    if !s.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        s.window.set_default_size(w, h);
    }
    s.window.present();
    s.reload();
    if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
        s.search.set_text(&q);
    }
}

/// Opens it on a page ("apps", "agentes", "modelos", "atualizacoes").
pub fn open_page(shell: &Rc<Shell>, page: &str) {
    open(shell, None);
    if let Some(s) = STORE.with(|s| s.borrow().clone()) {
        // (`--pagina=criativo:filmcraft`: a creative app's detail)
        if let Some(id) = page.strip_prefix("criativo:") {
            *s.criativos.atual.borrow_mut() = kalango_craft::catalogo().app(id).map(|a| a.id.clone());
            s.go("criativo");
        } else {
            s.go(page);
        }
    }
}

fn link_button(glyph: &str, colour: &str, text: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    let r = caixa(Orientation::Horizontal, 6, "");
    r.set_halign(Align::Center);
    r.append(&ui_icon(glyph, colour, 16));
    r.append(&rotulo(text, ""));
    b.set_child(Some(&r));
    b
}

fn action_button(text: &str, primary: bool) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("loja-acao");
    if primary {
        b.add_css_class("principal");
    }
    b.set_valign(Align::Center);
    b
}

/// A card in its spring wrapper: lifted 2 px while the pointer is on it.
fn lift(card: &gtk::Box) -> Animavel {
    let a = Animavel::new(card);
    let hover = gtk::EventControllerMotion::new();
    let w = a.downgrade();
    hover.connect_enter(move |_, _, _| {
        if let Some(a) = w.upgrade() {
            a.ir(Canal::Y, -2.0, mv::RAPIDA);
        }
    });
    let w = a.downgrade();
    hover.connect_leave(move |_| {
        if let Some(a) = w.upgrade() {
            a.ir(Canal::Y, 0.0, mv::RAPIDA);
        }
    });
    card.add_controller(hover);
    a
}

fn ellipsized(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

impl AppStore {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<AppStore> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("App Center")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Apps");
        window.set_hide_on_close(true);
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden)
        crate::superficies::release_resources_when_hidden(&window, || true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        // the rail: the app, a line, its pages
        let rail_box = caixa(Orientation::Vertical, 4, "mon-trilho");
        rail_box.add_css_class("loja-trilho");
        // (the design's art is 30 px in its 36 px box)
        let logo = crate::apps::imagem_kalango("central-de-apps", 30);
        logo.set_size_request(36, 36);
        rail_box.append(&logo);
        let div = caixa(Orientation::Vertical, 0, "central-trilho-div");
        div.set_size_request(28, 1);
        div.set_halign(Align::Center);
        rail_box.append(&div);
        let mut rail = Vec::new();
        for (id, glyph, tip) in PAGES {
            let b = gtk::Button::new();
            b.add_css_class("mon-pagina");
            let i = ui_icon(glyph, "#8E8F99", 16);
            b.set_child(Some(&i));
            b.set_tooltip_text(Some(tr(tip)));
            crate::central::posicao_debug(&format!("loja página {id}"), b.upcast_ref());
            rail_box.append(&b);
            rail.push((b, i));
        }
        // the toolbar: the title, the search, the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        let title = rotulo(tr("App Center"), "mon-titulo");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        bar.append(&title);
        let search = gtk::Entry::new();
        search.add_css_class("ed-pedir");
        search.set_size_request(340, 32);
        search.set_valign(Align::Center);
        search.set_placeholder_text(Some(tr("Search apps, agents and models…")));
        search.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        crate::central::posicao_debug_sempre("loja busca", search.upcast_ref());
        let sbox = gtk::Overlay::new();
        sbox.set_child(Some(&search));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        sbox.add_overlay(&key);
        bar.append(&sbox);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        pop.set_child(Some(&items));
        menu.set_popover(Some(&pop));
        crate::central::posicao_debug_sempre("loja menu", menu.upcast_ref());
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));

        // the page
        let page = caixa(Orientation::Vertical, 18, "loja-pagina");
        // the week's highlight
        let hero = caixa(Orientation::Horizontal, 24, "loja-destaque");
        let ht = caixa(Orientation::Vertical, 8, "");
        ht.set_hexpand(true);
        ht.set_valign(Align::Center);
        let label = caixa(Orientation::Horizontal, 6, "");
        label.append(&ui_icon("spark", "#A5A1E2", 16));
        label.append(&rotulo(tr("HIGHLIGHT OF THE WEEK"), "loja-destaque-rotulo"));
        ht.append(&label);
        let h1 = rotulo(tr("Agents that work with you"), "loja-destaque-titulo");
        h1.set_xalign(0.0);
        h1.set_wrap(true);
        ht.append(&h1);
        let body = rotulo(tr("Each agent shows what it can read, change and send before you install it — and everything it does goes into the Timeline, with Undo."), "loja-destaque-texto");
        body.set_xalign(0.0);
        body.set_wrap(true);
        body.set_width_chars(20);
        // (GTK counts that width in "characters" of the font's own average width: the design's 760 px in them)
        body.connect_realize(|l| {
            let ctx = l.pango_context();
            let m = ctx.metrics(ctx.font_description().as_ref(), None);
            let cw = m.approximate_char_width().max(m.approximate_digit_width()) as f64 / gtk::pango::SCALE as f64;
            if cw > 0.0 {
                l.set_max_width_chars((780.0 / cw).floor() as i32);
                if debug() {
                    eprintln!("debug loja: destaque com {} caracteres de {cw:.1} px", (780.0 / cw).floor());
                }
            }
        });
        // (in a grid beside a spring: its natural width, not the column's — it wraps where the design's 760 px box does,
        // and sooner on a narrow window; a box would give it the width of one line)
        let body_row = gtk::Grid::new();
        body_row.attach(&body, 0, 0, 1, 1);
        let spring = caixa(Orientation::Horizontal, 0, "");
        spring.set_hexpand(true);
        body_row.attach(&spring, 1, 0, 1, 1);
        ht.append(&body_row);
        let acts = caixa(Orientation::Horizontal, 8, "loja-destaque-acoes");
        let explore = link_button("spark", "#1C1D24", tr("Explore agents"), "loja-explorar");
        crate::central::posicao_debug_sempre("loja explorar", explore.upcast_ref());
        acts.append(&explore);
        let how = link_button("escudo", "#8E8F99", tr("How permissions work"), "loja-como");
        crate::central::posicao_debug_sempre("loja permissoes", how.upcast_ref());
        acts.append(&how);
        ht.append(&acts);
        hero.append(&ht);
        let hero_art = gtk::Image::from_paintable(gdk::Texture::from_bytes(&glib::Bytes::from_static(include_bytes!("../assets/loja-destaque.png"))).ok().as_ref());
        hero_art.set_pixel_size(150);
        hero_art.set_valign(Align::Center);
        hero_art.set_margin_end(3);
        hero.append(&hero_art);
        page.append(&hero);
        // the categories
        let chips_row = caixa(Orientation::Horizontal, 6, "");
        let mut chips: Vec<gtk::ToggleButton> = Vec::new();
        for (id, name) in data::CATEGORIES {
            let c = gtk::ToggleButton::with_label(tr(name));
            c.add_css_class("loja-chip");
            c.set_valign(Align::Center);
            if let Some(f) = chips.first() {
                c.set_group(Some(f));
            }
            crate::central::posicao_debug_sempre(&format!("loja categoria {}", if id.is_empty() { "tudo" } else { id }), c.upcast_ref());
            chips_row.append(&c);
            chips.push(c);
        }
        chips[0].set_active(true);
        page.append(&chips_row);
        let agents = Section::new(tr("Trending agents"), AGENT_MIN, "agentes");
        let apps = Section::new(tr("Essential apps"), CARD_MIN, "apps");
        let models = Section::new(tr("Local models"), CARD_MIN, "modelos");
        let downloads = crate::modelos_fonte::Downloads::new();
        let updates = Section::new(tr("Updates"), CARD_MIN, "atualizacoes");
        updates.more.set_label(tr("Update all"));
        let system_updates = caixa(Orientation::Horizontal, 10, "loja-nota");
        page.append(&downloads.root);
        let criativos = crate::criativos::compartilhado();
        page.append(&criativos.chamada);
        page.append(&criativos.colecao);
        page.append(&criativos.detalhe);
        page.append(&criativos.atualizacoes);
        for s in [&agents, &apps, &models, &updates] {
            page.append(&s.root);
        }
        page.append(&system_updates);
        let empty = rotulo("", "loja-vazio");
        empty.set_xalign(0.0);
        empty.set_wrap(true);
        page.append(&empty);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&page));
        // (measures the content: the columns follow the window's width)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        meter.set_content_height(1);
        meter.set_hexpand(true);
        let content = caixa(Orientation::Vertical, 0, "");
        content.set_hexpand(true);
        content.append(&meter);
        content.append(&handle);
        content.append(&scroll);
        let root = caixa(Orientation::Horizontal, 0, "");
        root.append(&rail_box);
        root.append(&content);
        window.set_child(Some(&root));

        let s = Rc::new(AppStore {
            window, shell: Rc::downgrade(shell), rail, search, hero, hero_art, chips_row, chips, agents, apps, models, downloads, updates, system_updates, empty,
            page: Cell::new("descobrir"), category: Cell::new(0), width: Cell::new(0), catalog: RefCell::new(Rc::new(Vec::new())), catalog_state: Cell::new(0),
            shelf: RefCell::default(), agent_list: RefCell::default(), model_list: RefCell::default(), flatpaks: RefCell::default(), user_remotes: RefCell::default(),
            pending_updates: RefCell::default(), jobs: RefCell::default(), bars: RefCell::default(), ticking: Cell::new(false), refresh_pending: RefCell::default(),
            criativos,
        });
        // the creative apps ask for the page again, open an app's detail, go back to the collection
        let w = Rc::downgrade(&s);
        *s.criativos.ao_mudar.borrow_mut() = Some(Box::new(move || {
            if let Some(s) = w.upgrade() {
                s.refresh(false);
            }
        }));
        let w = Rc::downgrade(&s);
        *s.criativos.ao_abrir.borrow_mut() = Some(Box::new(move |id: &str| {
            if let Some(s) = w.upgrade() {
                if id.is_empty() {
                    s.go("criativos");
                } else {
                    *s.criativos.atual.borrow_mut() = Some(id.to_string());
                    s.go("criativo");
                }
            }
        }));
        let w = Rc::downgrade(&s);
        *s.criativos.ao_voltar.borrow_mut() = Some(Box::new(move || {
            if let Some(s) = w.upgrade() {
                s.go("criativos");
            }
        }));
        // the menu: the catalog again from the remotes, the models' folder
        for (glyph, text, what) in [("reiniciar", tr("Refresh the catalog"), 0), ("pasta", tr("Open the models folder"), 1)] {
            let b = link_button(glyph, "#C9CAD1", text, "menu-item");
            crate::central::posicao_debug_sempre(if what == 0 { "loja menu catalogo" } else { "loja menu modelos" }, b.upcast_ref());
            if let Some(r) = b.child() {
                r.set_halign(Align::Start);
            }
            let (w, pop) = (Rc::downgrade(&s), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(p) = pop.upgrade() {
                    p.popdown();
                }
                let Some(s) = w.upgrade() else { return };
                if what == 0 {
                    s.refresh_catalog();
                } else if let Some(sh) = s.shell.upgrade() {
                    let d = data::download_dir();
                    let _ = std::fs::create_dir_all(&d);
                    sh.abrir_arquivos(Some(&d));
                }
            });
            items.append(&b);
        }
        let w = Rc::downgrade(&s);
        meter.connect_resize(move |_, width, _| {
            if let Some(s) = w.upgrade() {
                // (the page's padding, 28 on each side)
                let inner = width - 56;
                if s.width.replace(inner) != inner {
                    s.hero_art.set_visible(inner >= 900);
                    s.criativos.ajustar(inner);
                    let w2 = Rc::downgrade(&s);
                    glib::idle_add_local_once(move || {
                        if let Some(s) = w2.upgrade() {
                            s.lay_out();
                        }
                    });
                }
            }
        });
        for (i, (b, _)) in s.rail.iter().enumerate() {
            let w = Rc::downgrade(&s);
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.go(PAGES[i].0);
                }
            });
        }
        for (i, c) in s.chips.iter().enumerate() {
            let w = Rc::downgrade(&s);
            c.connect_toggled(move |c| {
                if let (Some(s), true) = (w.upgrade(), c.is_active()) {
                    s.category.set(i);
                    if debug() {
                        eprintln!("debug loja: categoria {}", if data::CATEGORIES[i].0.is_empty() { "tudo" } else { data::CATEGORIES[i].0 });
                    }
                    s.refresh(true);
                }
            });
        }
        for (sec, page) in [(&s.agents, "agentes"), (&s.apps, "apps"), (&s.models, "modelos")] {
            let w = Rc::downgrade(&s);
            sec.more.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.go(page);
                }
            });
        }
        let w = Rc::downgrade(&s);
        s.updates.more.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                let ids = s.pending_updates.borrow().clone().unwrap_or_default();
                for id in ids {
                    s.update_app(&id);
                }
            }
        });
        let w = Rc::downgrade(&s);
        explore.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.go("agentes");
            }
        });
        let w = Rc::downgrade(&s);
        how.connect_clicked(move |_| {
            if let Some(sh) = w.upgrade().and_then(|s| s.shell.upgrade()) {
                crate::help_app::open_topic(&sh, "05-agentes-permissoes");
                if debug() {
                    eprintln!("debug loja: como funcionam as permissões (Ajuda)");
                }
            }
        });
        // the search: what is typed filters (the whole catalog for apps); a sentence goes to the agent with Enter
        let w = Rc::downgrade(&s);
        s.search.connect_changed(move |_| {
            if let Some(s) = w.upgrade() {
                if let Some(id) = s.refresh_pending.borrow_mut().take() {
                    id.remove();
                }
                let w2 = Rc::downgrade(&s);
                *s.refresh_pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(160), move || {
                    if let Some(s) = w2.upgrade() {
                        s.refresh_pending.borrow_mut().take();
                        s.refresh(true);
                    }
                }));
            }
        });
        let w = Rc::downgrade(&s);
        s.search.connect_activate(move |e| {
            let Some(s) = w.upgrade() else { return };
            let q = e.text().trim().to_string();
            if q.split_whitespace().count() >= 3 {
                if let Some(sh) = s.shell.upgrade() {
                    sh.right_bar().ask_agent(&format!("{q}\n\n{}", tr("(asked in the App Center: the agent may suggest or install apps, agents or local models, with approval)")));
                    if debug() {
                        eprintln!("debug loja: ao agente «{q}»");
                    }
                }
                e.set_text("");
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&s);
        keys.connect_key_pressed(move |_, k, _, mods| {
            let Some(s) = w.upgrade() else { return glib::Propagation::Proceed };
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && k.to_lower() == gdk::Key::k {
                s.search.grab_focus();
                return glib::Propagation::Stop;
            }
            if k == gdk::Key::Escape && !s.search.text().is_empty() {
                s.search.set_text("");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        s.window.add_controller(keys);
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(su) = j.surface().filter(|su| su.type_().name().contains("Wayland")) {
                let id = CString::new("os.kalango.Apps").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(su.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        s.window.connect_realize(move |j| app_id(j));
        s.window.connect_map(move |j| {
            app_id(j);
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
        s.window.connect_close_request(|win| {
            win.set_visible(false);
            if debug() {
                eprintln!("debug loja: fechada");
            }
            glib::Propagation::Stop
        });
        s.go("descobrir");
        s
    }

    /// Reads what changes between showings: the agents, the shelf, the models, what is installed — and the catalog,
    /// once (off the interface's thread).
    fn reload(self: &Rc<Self>) {
        *self.agent_list.borrow_mut() = data::agent_catalog();
        *self.shelf.borrow_mut() = data::shelf();
        *self.model_list.borrow_mut() = data::model_catalog();
        *self.flatpaks.borrow_mut() = data::flatpak_installed();
        *self.user_remotes.borrow_mut() = data::user_remotes();
        self.criativos.recarregar();
        if self.catalog_state.get() == 0 {
            self.load_catalog();
        }
        self.refresh(true);
    }

    fn load_catalog(self: &Rc<Self>) {
        self.catalog_state.set(1);
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let t0 = std::time::Instant::now();
            let v = gio::spawn_blocking(data::load_catalog).await.unwrap_or_default();
            let Some(s) = w.upgrade() else { return };
            if debug() {
                eprintln!("debug loja: catálogo com {} apps ({} ms)", v.len(), t0.elapsed().as_millis());
            }
            *s.catalog.borrow_mut() = Rc::new(v);
            s.catalog_state.set(2);
            s.refresh(false);
        });
    }

    /// "Atualizar o catálogo": the remotes' catalogs again (Flatpak downloads them), then read again.
    fn refresh_catalog(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let user = !self.user_remotes.borrow().is_empty();
        glib::spawn_future_local(async move {
            let ok = gio::spawn_blocking(move || {
                std::process::Command::new(data::flatpak()).args([if user { "--user" } else { "--system" }, "update", "--appstream", "--noninteractive"])
                    .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
            }).await.unwrap_or(false);
            if debug() {
                eprintln!("debug loja: catálogo atualizado ({})", if ok { "ok" } else { "falhou" });
            }
            if let Some(s) = w.upgrade() {
                s.catalog_state.set(0);
                s.reload();
            }
        });
    }

    fn go(self: &Rc<Self>, page: &str) {
        // (the creative apps' collection and an app's detail are pages of Apps: the rail shows Apps)
        let extra = match page {
            "criativos" => Some("criativos"),
            "criativo" => Some("criativo"),
            _ => None,
        };
        let i = if extra.is_some() { 1 } else { PAGES.iter().position(|(p, _, _)| *p == page).unwrap_or(0) };
        self.page.set(extra.unwrap_or(PAGES[i].0));
        for (n, (b, img)) in self.rail.iter().enumerate() {
            if n == i { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
            crate::conversa::set_icon(img, PAGES[n].1, if n == i { "#A5A1E2" } else { "#8E8F99" });
        }
        if PAGES[i].0 == "atualizacoes" {
            self.look_for_updates();
        }
        if debug() {
            eprintln!("debug loja: página {}", self.page.get());
        }
        self.refresh(true);
    }

    fn query(&self) -> String {
        self.search.text().trim().to_lowercase()
    }

    fn category_id(&self) -> &'static str {
        data::CATEGORIES[self.category.get()].0
    }

    // ------------------------------------------------------------------------------------------ what each section has

    fn agent_items(&self) -> Vec<(usize, PathBuf, Manifesto)> {
        let (q, cat) = (self.query(), self.category_id());
        self.agent_list.borrow().iter().enumerate().filter(|(_, (_, m))| {
            (cat.is_empty() || m.categoria == cat)
                && (q.is_empty() || [&m.nome, &m.descricao, &m.resumo, &m.editor].iter().any(|t| t.to_lowercase().contains(&q)))
        }).map(|(i, (p, m))| (i, p.clone(), m.clone())).collect()
    }

    fn app_item(&self, id: &str, cat: &CatalogApp) -> AppItem {
        let flatpak = self.flatpaks.borrow().contains(id);
        AppItem {
            id: id.to_string(), name: cat.name.clone(), kind: tr(data::kind_of(&cat.categories)).to_string(), icon: cat.icon.clone(),
            category: data::category_of(&cat.categories).to_string(), desktop: data::desktop_of(id, &[]), flatpak,
            remote: Some((cat.remote.clone(), cat.user || self.user_remotes.borrow().contains(&cat.remote))),
        }
    }

    fn app_items(&self) -> Vec<AppItem> {
        let (q, cat) = (self.query(), self.category_id());
        let catalog = self.catalog.borrow().clone();
        let by_id: HashMap<&str, &CatalogApp> = catalog.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut v: Vec<AppItem> = Vec::new();
        for e in self.shelf.borrow().iter() {
            let local = data::desktop_of(&e.id, &e.local);
            let item = match by_id.get(e.id.as_str()) {
                Some(c) => {
                    let mut it = self.app_item(&e.id, c);
                    it.desktop = local;
                    if !e.categoria.is_empty() {
                        it.category = e.categoria.clone();
                    }
                    if !e.tipo.is_empty() {
                        it.kind = e.tipo.clone();
                    }
                    it
                }
                // not in a catalog here: shown only when it is installed some other way
                None => match local {
                    Some(d) => AppItem {
                        id: e.id.clone(), name: d.display_name().to_string(), kind: if e.tipo.is_empty() { tr("App").to_string() } else { e.tipo.clone() },
                        icon: None, category: e.categoria.clone(), desktop: Some(d), flatpak: false, remote: None,
                    },
                    None => continue,
                },
            };
            if (cat.is_empty() || item.category == cat) && (q.is_empty() || item.name.to_lowercase().contains(&q) || item.kind.to_lowercase().contains(&q)) {
                v.push(item);
            }
        }
        // a search reaches the whole catalog (names first, then what they are and their keywords)
        if !q.is_empty() {
            let shelf: HashSet<String> = v.iter().map(|a| a.id.clone()).collect();
            let mut found: Vec<(u8, &CatalogApp)> = catalog.iter().filter(|a| !shelf.contains(&a.id)).filter_map(|a| {
                let n = a.name.to_lowercase();
                let score = if n == q { 0 } else if n.starts_with(&q) { 1 } else if n.contains(&q) { 2 } else if a.keywords.iter().any(|k| k.contains(&q)) { 3 } else if a.summary.to_lowercase().contains(&q) { 4 } else { return None };
                Some((score, a))
            }).filter(|(_, a)| cat.is_empty() || data::category_of(&a.categories) == cat).collect();
            found.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.name.to_lowercase().cmp(&y.1.name.to_lowercase())));
            v.extend(found.into_iter().take(48).map(|(_, a)| self.app_item(&a.id, a)));
        }
        v
    }

    fn model_items(&self) -> Vec<ModelEntry> {
        let (q, cat) = (self.query(), self.category_id());
        if !cat.is_empty() {
            return vec![];
        }
        // (on the models' page the five kinds are the downloads' rows above)
        let only_others = self.page.get() == "modelos";
        self.model_list.borrow().iter().filter(|m| !(only_others && crate::modelos_fonte::is_slot(&m.id))).filter(|m| q.is_empty() || m.nome.to_lowercase().contains(&q) || m.descricao.to_lowercase().contains(&q)).cloned().collect()
    }

    // ------------------------------------------------------------------------------------------ the page

    /// The columns for the window's width (the cards are laid again only when they change).
    fn lay_out(self: &Rc<Self>) {
        let w = self.width.get();
        if w <= 0 {
            return;
        }
        let mut changed = false;
        for s in [&self.agents, &self.apps, &self.models, &self.updates] {
            let c = s.columns(w);
            if s.grid.max_children_per_line() != c {
                s.grid.set_min_children_per_line(c);
                s.grid.set_max_children_per_line(c);
                changed = true;
            }
        }
        if changed {
            if debug() {
                eprintln!("debug loja: colunas {} agentes, {} apps (largura {w})", self.agents.columns(w), self.apps.columns(w));
            }
            self.refresh(false);
        }
    }

    fn refresh(self: &Rc<Self>, cascade: bool) {
        let page = self.page.get();
        let q = self.query();
        let w = self.width.get().max(CARD_MIN);
        let discover = page == "descobrir";
        self.hero.set_visible(discover && q.is_empty());
        self.chips_row.set_visible(matches!(page, "descobrir" | "apps" | "agentes"));
        // the creative apps
        let cr = &self.criativos;
        cr.chamada.set_visible(matches!(page, "descobrir" | "apps") && q.is_empty() && self.category.get() == 0);
        cr.colecao.set_visible(page == "criativos");
        cr.detalhe.set_visible(page == "criativo");
        cr.atualizacoes.set_visible(page == "atualizacoes");
        match page {
            "criativos" => cr.mostrar_colecao(),
            "criativo" => {
                let id = cr.atual.borrow().clone().unwrap_or_default();
                cr.mostrar_detalhe(&id);
            }
            "atualizacoes" => cr.mostrar_atualizacoes(),
            _ => {}
        }
        let mut anims: Vec<Animavel> = Vec::new();
        let mut counts = (0, 0, 0);
        // agents
        let show_agents = matches!(page, "descobrir" | "agentes");
        self.agents.clear();
        if show_agents {
            let items = self.agent_items();
            let limit = if discover && q.is_empty() { self.agents.columns(w) as usize } else { usize::MAX };
            for (i, path, m) in items.iter().take(limit) {
                let a = self.agent_card(*i, path, m);
                self.agents.grid.append(&a);
                anims.push(a);
            }
            counts.0 = items.len();
            self.agents.title.set_text(if page == "agentes" { tr("Agents") } else { tr("Trending agents") });
            self.agents.more.set_visible(discover && q.is_empty());
        }
        self.agents.root.set_visible(show_agents && counts.0 > 0);
        // apps
        let show_apps = matches!(page, "descobrir" | "apps");
        self.apps.clear();
        if show_apps {
            let items = self.app_items();
            let limit = if discover && q.is_empty() { self.apps.columns(w) as usize } else { usize::MAX };
            for it in items.iter().take(limit) {
                let a = self.app_card(it);
                self.apps.grid.append(&a);
                anims.push(a);
            }
            counts.1 = items.len();
            self.apps.title.set_text(if !q.is_empty() { tr("Apps") } else if page == "apps" { tr("Essential apps") } else { tr("Essential apps") });
            self.apps.more.set_visible(discover && q.is_empty());
        }
        self.apps.root.set_visible(show_apps && counts.1 > 0);
        // models
        let show_models = matches!(page, "descobrir" | "modelos");
        self.models.clear();
        if show_models {
            let items = self.model_items();
            let limit = if discover && q.is_empty() { self.models.columns(w) as usize } else { usize::MAX };
            let gpu = crate::widgets::dados::gpu_now().and_then(|g| g.vram);
            let ram = crate::widgets::dados::free_memory();
            for m in items.iter().take(limit) {
                let a = self.model_card(m, gpu, ram);
                self.models.grid.append(&a);
                anims.push(a);
            }
            counts.2 = items.len();
            self.models.more.set_visible(discover && q.is_empty());
        }
        self.models.root.set_visible(show_models && counts.2 > 0);
        self.downloads.root.set_visible(page == "modelos" && q.is_empty());
        // updates
        let show_updates = page == "atualizacoes";
        self.updates.clear();
        self.updates.root.set_visible(show_updates);
        self.system_updates.set_visible(show_updates);
        if show_updates {
            self.fill_updates(&mut anims);
        }
        // nothing
        let nothing = !show_updates && !matches!(page, "criativos" | "criativo") && counts == (0, 0, 0);
        self.empty.set_visible(nothing);
        if nothing {
            self.empty.set_text(&if !q.is_empty() {
                trf("Nothing here for “{query}”. Press Enter with a sentence to ask the agent.", &[("query", self.search.text().to_string())])
            } else {
                tr("Nothing in this category yet.").to_string()
            });
        }
        if cascade {
            mv::cascata(&anims, 8.0);
        }
        if debug() {
            eprintln!("debug loja: {} · {} agentes, {} apps, {} modelos{}", page, counts.0, counts.1, counts.2, if q.is_empty() { String::new() } else { format!(" para «{q}»") });
        }
    }

    // ------------------------------------------------------------------------------------------ the cards

    fn agent_card(self: &Rc<Self>, i: usize, path: &std::path::Path, m: &Manifesto) -> Animavel {
        let colour = AGENT_COLOURS[i % AGENT_COLOURS.len()];
        let card = caixa(Orientation::Vertical, 10, "loja-cartao");
        card.add_css_class("loja-agente");
        let top = caixa(Orientation::Horizontal, 10, "");
        let ic = caixa(Orientation::Horizontal, 0, "loja-agente-icone");
        ic.set_size_request(40, 40);
        let tint = gtk::CssProvider::new();
        tint.load_from_string(&format!(".loja-agente-icone {{ background: alpha({}, 0.14); }}", crate::tema::cor(colour)));
        #[allow(deprecated)]
        ic.style_context().add_provider(&tint, gtk::STYLE_PROVIDER_PRIORITY_USER + 2);
        ic.set_hexpand(false);
        ic.set_halign(Align::Start);
        ic.set_valign(Align::Center);
        let st = ui_icon("spark", colour, 16);
        st.set_hexpand(true);
        ic.append(&st);
        top.append(&ic);
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        t.append(&ellipsized(&m.nome, "loja-agente-nome"));
        let who = caixa(Orientation::Horizontal, 4, "");
        let ed = rotulo(if m.editor.is_empty() { tr("unknown publisher") } else { &m.editor }, "loja-sub");
        ed.set_ellipsize(gtk::pango::EllipsizeMode::End);
        who.append(&ed);
        // (it is in the system's catalog: KalangoOS ships it)
        let ck = ui_icon("check", "#8ED2C3", 16);
        ck.set_tooltip_text(Some(tr("In KalangoOS's catalog")));
        who.append(&ck);
        t.append(&who);
        top.append(&t);
        card.append(&top);
        let resumo = if m.resumo.is_empty() { &m.descricao } else { &m.resumo };
        card.append(&ellipsized(resumo, "loja-sub"));
        // (one line: on a narrow card the chips' texts shorten in the middle, the whole text in their tooltip)
        let perms = caixa(Orientation::Horizontal, 6, "loja-permissoes");
        let chips = data::chips(m);
        if debug() {
            eprintln!("debug loja: agente {}: {} · {}", if m.id.is_empty() { &m.nome } else { &m.id }, chips.iter().map(|c| c.1.as_str()).collect::<Vec<_>>().join(" | "), data::autonomy_text(&m.autonomia));
        }
        for (glyph, text, tone) in chips {
            let (class, colour) = match tone {
                Tone::Plain => ("", "#8E8F99"),
                Tone::Network => ("rede", "#E5C98C"),
                Tone::Offline => ("sem-rede", "#8ED2C3"),
                Tone::Screen => ("tela", "#EFA2A6"),
            };
            let chip = caixa(Orientation::Horizontal, 5, "loja-perm");
            if !class.is_empty() {
                chip.add_css_class(class);
            }
            chip.append(&ui_icon(glyph, colour, 16));
            let l = rotulo(&text, "loja-perm-texto");
            // (only a folder's path shortens: what an agent sees and reaches stays whole)
            if glyph == "pasta" || glyph == "olho" || glyph == "lixeira" {
                l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            }
            chip.append(&l);
            chip.set_tooltip_text(Some(&text));
            perms.append(&chip);
        }
        card.append(&perms);
        let foot = caixa(Orientation::Horizontal, 6, "loja-rodape");
        foot.set_vexpand(true);
        foot.set_valign(Align::End);
        foot.append(&ui_icon("escudo", "#8E8F99", 16));
        foot.append(&ellipsized(data::autonomy_text(&m.autonomia), "loja-autonomia"));
        let installed = data::installed_agent(m);
        let key = if m.id.is_empty() { m.nome.clone() } else { m.id.clone() };
        let b = action_button(if installed.is_some() { tr("Open") } else { tr("Install") }, installed.is_none());
        crate::central::posicao_debug_sempre(&format!("loja {} {key}", if installed.is_some() { "abrir" } else { "instalar" }), b.upcast_ref());
        let (w, path) = (Rc::downgrade(self), path.to_path_buf());
        b.connect_clicked(move |_| {
            let Some(sh) = w.upgrade().and_then(|s| s.shell.upgrade()) else { return };
            if installed.is_some() {
                sh.abrir_central(Some("rotinas"));
            } else {
                sh.instalar_agente(&path);
            }
            if debug() {
                eprintln!("debug loja: agente {key} → {}", if installed.is_some() { "rotinas" } else { "instalar" });
            }
        });
        foot.append(&b);
        card.append(&foot);
        lift(&card)
    }

    fn app_icon(it: &AppItem) -> gtk::Image {
        let img = match (&it.icon, &it.desktop) {
            (Some(p), _) => gtk::Image::from_file(p),
            // (an app whose package left its icon where the theme does not look gets the generic glyph, not the
            // theme's "missing" picture)
            (None, Some(d)) => d.icon().filter(|i| {
                i.downcast_ref::<gio::FileIcon>().is_some() || gdk::Display::default().is_some_and(|dp| gtk::IconTheme::for_display(&dp).has_gicon(i))
            }).map(|i| gtk::Image::from_gicon(&i)).unwrap_or_else(|| ui_icon("apps-n", "#8E8F99", 24)),
            _ => ui_icon("apps-n", "#8E8F99", 24),
        };
        // (the design's app art is 32 px in its 40 px box)
        if img.storage_type() != gtk::ImageType::Paintable {
            img.set_pixel_size(32);
        }
        img.set_size_request(40, 40);
        img.set_valign(Align::Center);
        img
    }

    fn app_card(self: &Rc<Self>, it: &AppItem) -> Animavel {
        let card = caixa(Orientation::Horizontal, 12, "loja-cartao");
        card.add_css_class("loja-app");
        card.append(&Self::app_icon(it));
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        t.append(&ellipsized(&it.name, "loja-app-nome"));
        let sub = ellipsized(&it.kind, "loja-sub");
        t.append(&sub);
        card.append(&t);
        let key = format!("app:{}", it.id);
        if self.jobs.borrow().contains_key(&key) {
            card.append(&self.progress_widget(&key, Some(&sub)));
        } else if it.installed() {
            let b = action_button(tr("Open"), false);
            crate::central::posicao_debug_sempre(&format!("loja abrir {}", it.id), b.upcast_ref());
            let it2 = it.clone();
            b.connect_clicked(move |_| {
                let r = match &it2.desktop {
                    Some(d) => d.launch(&[], gdk::Display::default().map(|d| d.app_launch_context()).as_ref()).map_err(|e| e.to_string()),
                    None => std::process::Command::new(data::flatpak()).args(["run", &it2.id]).spawn().map(|_| ()).map_err(|e| e.to_string()),
                };
                if let Err(e) = r {
                    eprintln!("kalango-shell: abrir {}: {e}", it2.id);
                }
                if debug() {
                    eprintln!("debug loja: abrir {}", it2.id);
                }
            });
            card.append(&b);
        } else if let Some((remote, user)) = it.remote.clone() {
            let b = action_button(tr("Install"), true);
            crate::central::posicao_debug_sempre(&format!("loja instalar {}", it.id), b.upcast_ref());
            let (w, id) = (Rc::downgrade(self), it.id.clone());
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.install_app(&id, &remote, user);
                }
            });
            card.append(&b);
        }
        lift(&card)
    }

    fn model_card(self: &Rc<Self>, m: &ModelEntry, gpu: Option<(f64, f64)>, ram: f64) -> Animavel {
        let card = caixa(Orientation::Vertical, 8, "loja-cartao");
        card.add_css_class("loja-modelo");
        let top = caixa(Orientation::Horizontal, 10, "");
        let ic = caixa(Orientation::Horizontal, 0, "loja-modelo-icone");
        ic.set_size_request(32, 32);
        ic.set_hexpand(false);
        ic.set_valign(Align::Center);
        let br = ui_icon("cerebro", "#A5A1E2", 16);
        br.set_hexpand(true);
        ic.append(&br);
        top.append(&ic);
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.append(&ellipsized(&m.nome, "loja-app-nome"));
        t.append(&ellipsized(&m.descricao, "loja-sub"));
        top.append(&t);
        let size = rotulo(&data::gb(m.bytes() as f64), "loja-tamanho");
        size.set_valign(Align::Center);
        top.append(&size);
        card.append(&top);
        let foot = caixa(Orientation::Horizontal, 6, "loja-rodape");
        let installed = data::model_installed(m);
        let speed = if installed { m.main_file().and_then(|f| data::measured_speed(&f.nome)) } else { None };
        // (installed models are measured against what is free with them loaded: they fit where they are)
        let fit = if installed { data::Fit::Roomy } else { data::fit(m.bytes(), gpu, ram) };
        let fit = if installed && gpu.is_none() { Fit::Cpu(true) } else { fit };
        let (text, ok) = data::fit_text(&fit, speed);
        let dot = caixa(Orientation::Horizontal, 0, if ok { "loja-ponto" } else { "loja-ponto-aviso" });
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        foot.append(&dot);
        let l = rotulo(&text, if ok { "loja-cabe" } else { "loja-nao-cabe" });
        l.set_xalign(0.0);
        // (wraps, never cut: a "does not fit" says why; GTK would reserve whole font lines for a set number of lines)
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_max_width_chars(1);
        // (the design's 13 px lines: as a layout attribute, so the label also measures with it)
        let at = gtk::pango::AttrList::new();
        at.insert(gtk::pango::AttrFloat::new_line_height(0.92));
        l.set_attributes(Some(&at));
        l.set_hexpand(true);
        foot.append(&l);
        let key = format!("modelo:{}", m.id);
        if self.jobs.borrow().contains_key(&key) {
            foot.append(&self.progress_widget(&key, Some(&l)));
        } else if installed {
            let b = action_button(tr("Already installed"), false);
            b.set_tooltip_text(Some(tr("Choose it in Settings › Agents and AI")));
            crate::central::posicao_debug_sempre(&format!("loja instalado {}", m.id), b.upcast_ref());
            b.connect_clicked(|_| crate::configuracoes::abrir_painel("agentes"));
            foot.append(&b);
        } else {
            let b = action_button(tr("Download"), true);
            crate::central::posicao_debug_sempre(&format!("loja baixar {}", m.id), b.upcast_ref());
            let (w, m2) = (Rc::downgrade(self), m.clone());
            b.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.download_model(&m2);
                }
            });
            foot.append(&b);
        }
        card.append(&foot);
        if debug() {
            eprintln!("debug loja: modelo {} {} ({text})", m.id, if installed { "instalado" } else { "a baixar" });
        }
        lift(&card)
    }

    // ------------------------------------------------------------------------------------------ installs and downloads

    /// The bar a card shows while its job runs (a click cancels a download).
    fn progress_widget(self: &Rc<Self>, key: &str, text: Option<&gtk::Label>) -> gtk::Widget {
        let b = gtk::Button::new();
        b.add_css_class("loja-progresso");
        b.set_valign(Align::Center);
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.add_css_class("plano-barra");
        bar.set_size_request(96, -1);
        bar.set_valign(Align::Center);
        if let Some(j) = self.jobs.borrow().get(key) {
            bar.set_value(j.fraction());
        }
        b.set_child(Some(&bar));
        b.set_tooltip_text(Some(if key.starts_with("modelo:") { tr("Downloading — click to cancel") } else { tr("Installing…") }));
        crate::central::posicao_debug_sempre(&format!("loja progresso {key}"), b.upcast_ref());
        let (w, k) = (Rc::downgrade(self), key.to_string());
        b.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                if let Some(t) = s.jobs.borrow().get(&k).and_then(|j| j.transfer.clone()) {
                    t.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        let label = text.cloned().unwrap_or_else(|| rotulo("", ""));
        self.bars.borrow_mut().insert(key.to_string(), (bar.downgrade(), label.downgrade()));
        self.tick();
        b.upcast()
    }

    fn start_job(self: &Rc<Self>, key: &str, job: Job) {
        self.jobs.borrow_mut().insert(key.to_string(), job);
        self.refresh(false);
    }

    /// Follows the jobs while any runs: their bars, and the page again when one ends.
    fn tick(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            let mut ended = Vec::new();
            for (k, j) in s.jobs.borrow().iter() {
                let x = j.fraction();
                if let Some((bar, label)) = s.bars.borrow().get(k) {
                    if let Some(b) = bar.upgrade() {
                        b.set_value(x.clamp(0.0, 1.0));
                    }
                    if let (Some(l), Some(t)) = (label.upgrade(), j.transfer.as_ref()) {
                        l.set_text(&trf("downloading… {percent}% of {size}", &[("percent", format!("{:.0}", x * 100.0)), ("size", data::gb(t.total as f64))]));
                    }
                }
                if j.finished.load(Ordering::Relaxed) {
                    ended.push((k.clone(), j.error.lock().ok().and_then(|e| e.clone())));
                }
            }
            for (k, err) in &ended {
                s.jobs.borrow_mut().remove(k);
                s.bars.borrow_mut().remove(k);
                match err {
                    Some(e) => {
                        eprintln!("kalango-shell: loja: {k}: {e}");
                        if debug() {
                            eprintln!("debug loja: {k} falhou ({e})");
                        }
                    }
                    None if debug() => eprintln!("debug loja: {k} pronto"),
                    None => {}
                }
            }
            if !ended.is_empty() {
                *s.flatpaks.borrow_mut() = data::flatpak_installed();
                if ended.iter().any(|(k, e)| k.starts_with("atualizar:") && e.is_none()) {
                    let done: Vec<String> = ended.iter().filter_map(|(k, e)| k.strip_prefix("atualizar:").filter(|_| e.is_none()).map(str::to_string)).collect();
                    if let Some(v) = s.pending_updates.borrow_mut().as_mut() {
                        v.retain(|x| !done.contains(x));
                    }
                }
                s.refresh(false);
            }
            if s.jobs.borrow().is_empty() {
                s.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Runs flatpak with these arguments for a job, its percentages to the bar.
    fn run_flatpak(job: &Job, args: Vec<String>) {
        let (permille, finished, error) = (job.permille.clone(), job.finished.clone(), job.error.clone());
        std::thread::spawn(move || {
            let r = (|| -> Result<(), String> {
                let mut child = std::process::Command::new(data::flatpak()).args(&args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
                    .spawn().map_err(|e| e.to_string())?;
                let mut err_text = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let mut buf = [0u8; 4096];
                    use std::io::Read;
                    while let Ok(n) = out.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        if let Some(p) = data::last_percent(&String::from_utf8_lossy(&buf[..n])) {
                            permille.store((p * 1000.0) as u64, Ordering::Relaxed);
                        }
                    }
                }
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err_text);
                }
                let st = child.wait().map_err(|e| e.to_string())?;
                if st.success() { Ok(()) } else { Err(err_text.lines().last().unwrap_or("flatpak falhou").to_string()) }
            })();
            if let Err(e) = r {
                if let Ok(mut x) = error.lock() {
                    *x = Some(e);
                }
            }
            permille.store(1000, Ordering::Relaxed);
            finished.store(true, Ordering::Relaxed);
        });
    }

    fn install_app(self: &Rc<Self>, id: &str, remote: &str, user: bool) {
        let key = format!("app:{id}");
        if self.jobs.borrow().contains_key(&key) {
            return;
        }
        let job = Job::new();
        let args = data::install_args(remote, id, user);
        if debug() {
            eprintln!("debug loja: instalando {id} ({} {remote})", if user { "--user" } else { "--system" });
        }
        Self::run_flatpak(&job, args);
        self.start_job(&key, job);
    }

    fn download_model(self: &Rc<Self>, m: &ModelEntry) {
        let key = format!("modelo:{}", m.id);
        if self.jobs.borrow().contains_key(&key) {
            return;
        }
        let mut job = Job::new();
        let t = Transfer { total: m.bytes(), ..Default::default() };
        job.transfer = Some(t.clone());
        let (finished, error, files) = (job.finished.clone(), job.error.clone(), m.arquivos.clone());
        if debug() {
            eprintln!("debug loja: baixando modelo {} ({})", m.id, data::gb(m.bytes() as f64));
        }
        std::thread::spawn(move || {
            if let Err(e) = data::download(&files, &data::download_dir(), &t) {
                if let Ok(mut x) = error.lock() {
                    *x = Some(e);
                }
            }
            finished.store(true, Ordering::Relaxed);
        });
        self.start_job(&key, job);
    }

    // ------------------------------------------------------------------------------------------ updates

    fn look_for_updates(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let v = gio::spawn_blocking(|| {
                std::process::Command::new(data::flatpak()).args(["remote-ls", "--updates", "--app", "--columns=application"]).stderr(std::process::Stdio::null()).output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with("Application")).collect::<Vec<_>>())
                    .unwrap_or_default()
            }).await.unwrap_or_default();
            let Some(s) = w.upgrade() else { return };
            if debug() {
                eprintln!("debug loja: {} atualizações do Flatpak", v.len());
            }
            *s.pending_updates.borrow_mut() = Some(v);
            if s.page.get() == "atualizacoes" {
                s.refresh(true);
            }
        });
    }

    fn update_app(self: &Rc<Self>, id: &str) {
        let key = format!("atualizar:{id}");
        if self.jobs.borrow().contains_key(&key) {
            return;
        }
        let job = Job::new();
        if debug() {
            eprintln!("debug loja: atualizando {id}");
        }
        Self::run_flatpak(&job, ["update", "--noninteractive", "-y", id].iter().map(|s| s.to_string()).collect());
        self.start_job(&key, job);
    }

    fn fill_updates(self: &Rc<Self>, anims: &mut Vec<Animavel>) {
        let pending = self.pending_updates.borrow().clone();
        let catalog = self.catalog.borrow().clone();
        self.updates.grid.set_visible(pending.as_ref().is_some_and(|v| !v.is_empty()));
        if debug() {
            eprintln!("debug loja: atualizações na tela: {}", pending.as_ref().map(|v| v.join(", ")).unwrap_or_else(|| "procurando".into()));
        }
        match pending.as_deref() {
            None => {
                self.updates.title.set_text(tr("Looking for updates…"));
                self.updates.more.set_visible(false);
            }
            Some([]) => {
                self.updates.title.set_text(tr("Every app is up to date"));
                self.updates.more.set_visible(false);
            }
            Some(ids) => {
                self.updates.title.set_text(&trf("Updates ({count})", &[("count", ids.len().to_string())]));
                self.updates.more.set_visible(true);
                for id in ids {
                    let (name, kind, icon) = catalog.iter().find(|a| &a.id == id).map(|a| (a.name.clone(), tr(data::kind_of(&a.categories)).to_string(), a.icon.clone()))
                        .unwrap_or_else(|| (data::desktop_of(id, &[]).map(|d| d.display_name().to_string()).unwrap_or_else(|| id.clone()), id.clone(), None));
                    let it = AppItem { id: id.clone(), name, kind, icon, category: String::new(), desktop: data::desktop_of(id, &[]), flatpak: true, remote: None };
                    let card = caixa(Orientation::Horizontal, 12, "loja-cartao");
                    card.add_css_class("loja-app");
                    card.append(&Self::app_icon(&it));
                    let t = caixa(Orientation::Vertical, 1, "");
                    t.set_hexpand(true);
                    t.set_valign(Align::Center);
                    t.append(&ellipsized(&it.name, "loja-app-nome"));
                    let sub = ellipsized(&it.kind, "loja-sub");
                    t.append(&sub);
                    card.append(&t);
                    let key = format!("atualizar:{id}");
                    if self.jobs.borrow().contains_key(&key) {
                        card.append(&self.progress_widget(&key, Some(&sub)));
                    } else {
                        let b = action_button(tr("Update app"), true);
                        crate::central::posicao_debug_sempre(&format!("loja atualizar {id}"), b.upcast_ref());
                        let (w, id2) = (Rc::downgrade(self), id.clone());
                        b.connect_clicked(move |_| {
                            if let Some(s) = w.upgrade() {
                                s.update_app(&id2);
                            }
                        });
                        card.append(&b);
                    }
                    let a = lift(&card);
                    self.updates.grid.append(&a);
                    anims.push(a);
                }
            }
        }
        // the system's own packages are Settings' (PackageKit, installed on restart)
        while let Some(c) = self.system_updates.first_child() {
            self.system_updates.remove(&c);
        }
        self.system_updates.append(&ui_icon("escudo", "#8E8F99", 16));
        let l = rotulo(tr("The system's own updates are installed on restart, from Settings › System."), "loja-sub");
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_wrap(true);
        self.system_updates.append(&l);
        let b = action_button(tr("Open Settings"), false);
        b.connect_clicked(|_| crate::configuracoes::abrir_painel("sistema"));
        self.system_updates.append(&b);
    }
}
