//! Spaces by intention (design "Espaços por intenção", Super+Tab or `kalango-shell espacos`).
//!
//! niri's workspaces become spaces with an intention. Each one has a colour and the rule for what may interrupt it:
//! everything, only approvals, or nothing (the notifications wait in the list). It can also keep the agents quiet.
//! The notifications follow the rule of the space in focus (Settings › Appearance › Dock and multitasking ›
//! "Espaços por intenção" turns this off). The screen shows each space of the output with:
//! - a picture of its windows over the wallpaper;
//! - when it was last visited, its apps and its Super+N key;
//! - its agents (the routines that run on entering it, or the quiet rule) and its interruption rule (a click changes
//!   them).
//!
//! What the screen does:
//! - **Going to a space:** a click, Enter or its number.
//! - **Moving windows:** dragging an app's icon to another space moves that app's windows there.
//! - **"Montar espaço":** the local model names a new space from a sentence and picks the apps to open in it, which
//!   the shell opens there.
//! - **A habit:** when a space is entered around the same hour on three days or more, the screen offers to go there
//!   by itself every day at that hour and keep the agents quiet ("Trocar de espaço por rotina").
//!
//! Kept only on this computer: the rules in ~/.config/kalango/espacos.json, the visits in
//! $XDG_STATE_HOME/kalango/espacos-uso.json (the last 500).

use crate::alternador::{carregar_miniatura, thumbs_dir};
use crate::apps;
use crate::conversa::ui_icon;
use crate::estado::{Espaco, Janela};
use crate::servicos::niri_acao;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// What may interrupt a space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interrupt {
    All,
    Approvals,
    Hold,
}

impl Interrupt {
    fn id(self) -> &'static str {
        match self {
            Interrupt::All => "tudo",
            Interrupt::Approvals => "aprovacoes",
            Interrupt::Hold => "guardar",
        }
    }
    fn from_id(s: &str) -> Interrupt {
        match s {
            "aprovacoes" => Interrupt::Approvals,
            "guardar" => Interrupt::Hold,
            _ => Interrupt::All,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Interrupt::All => tr("Everything may interrupt"),
            Interrupt::Approvals => tr("Only approvals interrupt"),
            Interrupt::Hold => tr("Notifications held"),
        }
    }
    fn next(self) -> Interrupt {
        match self {
            Interrupt::All => Interrupt::Approvals,
            Interrupt::Approvals => Interrupt::Hold,
            Interrupt::Hold => Interrupt::All,
        }
    }
}

/// A space's own rules.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub color: usize,
    pub interrupt: Interrupt,
    pub agents_quiet: bool,
}

/// "Trocar de espaço por rotina": every day at `hour`, to the space `space` (and its agents quiet).
#[derive(Clone, Debug, PartialEq)]
pub struct AutoSwitch {
    pub space: String,
    pub hour: u32,
    pub quiet: bool,
}

/// What the file keeps.
#[derive(Clone, Debug, Default, PartialEq)]
struct Kept {
    rules: HashMap<String, Rules>,
    switches: Vec<AutoSwitch>,
    dismissed: Vec<String>,
}

fn rules_file() -> PathBuf {
    glib::user_config_dir().join("kalango/espacos.json")
}

fn visits_file() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/espacos-uso.json")
}

fn read_kept() -> Kept {
    let v: Value = std::fs::read_to_string(rules_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let rules = v["espacos"].as_object().map(|o| o.iter().map(|(k, r)| (k.clone(), Rules {
        color: r["cor"].as_u64().unwrap_or(0) as usize % crate::visao::DOTS.len(),
        interrupt: Interrupt::from_id(r["interromper"].as_str().unwrap_or("tudo")),
        agents_quiet: r["agentes_silencio"].as_bool().unwrap_or(false),
    })).collect()).unwrap_or_default();
    let switches = v["rotinas"].as_array().map(|l| l.iter().filter_map(|r| Some(AutoSwitch {
        space: r["espaco"].as_str()?.to_string(), hour: r["hora"].as_u64()?.min(23) as u32, quiet: r["silenciar"].as_bool().unwrap_or(true),
    })).collect()).unwrap_or_default();
    let dismissed = v["dispensadas"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    Kept { rules, switches, dismissed }
}

fn write_kept(k: &Kept) {
    let mut espacos = serde_json::Map::new();
    for (key, r) in &k.rules {
        espacos.insert(key.clone(), json!({"cor": r.color, "interromper": r.interrupt.id(), "agentes_silencio": r.agents_quiet}));
    }
    let v = json!({
        "espacos": espacos,
        "rotinas": k.switches.iter().map(|s| json!({"espaco": s.space, "hora": s.hour, "silenciar": s.quiet})).collect::<Vec<_>>(),
        "dispensadas": k.dismissed,
    });
    let p = rules_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// A space's key: its name, or its place for an unnamed one.
pub fn key(e: &Espaco) -> String {
    e.nome.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("#{}", e.idx))
}

fn display_name(e: &Espaco) -> String {
    e.nome.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| trf("Workspace {n}", &[("n", e.idx.to_string())]))
}

/// The rules of a space (the defaults: its place's colour, everything may interrupt).
pub fn rules_of(e: &Espaco) -> Rules {
    read_kept().rules.get(&key(e)).cloned()
        .unwrap_or(Rules { color: (e.idx.max(1) as usize - 1) % crate::visao::DOTS.len(), interrupt: Interrupt::All, agents_quiet: false })
}

fn set_rules(e: &Espaco, r: Rules) {
    let mut k = read_kept();
    k.rules.insert(key(e), r);
    write_kept(&k);
}

/// "Espaços por intenção": the spaces' rules apply.
pub fn intentions_on() -> bool {
    crate::configuracoes::ajustes()["espacos_por_intencao"].as_bool().unwrap_or(true)
}

/// "Trocar de espaço por rotina".
pub fn by_routine() -> bool {
    crate::configuracoes::ajustes()["espacos_por_rotina"].as_bool().unwrap_or(false)
}

pub fn set_intentions(on: bool) {
    crate::configuracoes::salvar_ajuste("espacos_por_intencao", json!(on));
    if debug() {
        eprintln!("debug espaços: por intenção {on}");
    }
}

pub fn set_by_routine(on: bool) {
    crate::configuracoes::salvar_ajuste("espacos_por_rotina", json!(on));
    if debug() {
        eprintln!("debug espaços: por rotina {on}");
    }
}

fn focused_space() -> Option<Espaco> {
    let s = crate::shell_atual()?;
    let e = s.estado.espacos.borrow().iter().find(|e| e.focado).cloned();
    e
}

/// Whether a notice waits in the list instead of showing, by the focused space's rules (critical ones always show).
pub fn holds(app: &str, approval: bool, critical: bool) -> bool {
    if critical || !intentions_on() {
        return false;
    }
    let Some(e) = focused_space() else { return false };
    let r = rules_of(&e);
    let quiet_agent = r.agents_quiet && app == tr("Agents") && !approval;
    match r.interrupt {
        Interrupt::All => quiet_agent,
        Interrupt::Approvals => !approval,
        Interrupt::Hold => true,
    }
}

// ------------------------------------------------------------------------------------------------------ visits

fn read_visits() -> Vec<(String, i64)> {
    std::fs::read_to_string(visits_file()).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default()
        .into_iter().filter_map(|x| Some((x[0].as_str()?.to_string(), x[1].as_i64()?))).collect()
}

fn record_visit(k: &str) {
    let mut v = read_visits();
    v.push((k.to_string(), now()));
    let start = v.len().saturating_sub(500);
    let list: Vec<Value> = v[start..].iter().map(|(k, t)| json!([k, t])).collect();
    let f = visits_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, Value::Array(list).to_string());
}

/// The space entered around the same hour on three days or more in the last two weeks (named spaces only), not yet
/// switched to by routine nor turned down: (space, hour). `local` gives a time's (day number, hour).
pub fn habit(visits: &[(String, i64)], now: i64, switches: &[AutoSwitch], dismissed: &[String], local: impl Fn(i64) -> Option<(i64, u32)>) -> Option<(String, u32)> {
    let mut days: HashMap<(String, u32), Vec<i64>> = HashMap::new();
    for (k, t) in visits.iter().filter(|(k, t)| !k.starts_with('#') && now - t < 14 * 86400) {
        if let Some((day, hour)) = local(*t) {
            let d = days.entry((k.clone(), hour)).or_default();
            if !d.contains(&day) {
                d.push(day);
            }
        }
    }
    let mut best: Vec<((String, u32), usize)> = days.into_iter().map(|(k, d)| (k, d.len())).filter(|(_, n)| *n >= 3)
        .filter(|((s, h), _)| !switches.iter().any(|w| w.space == *s && w.hour == *h) && !dismissed.contains(&format!("{s}@{h}"))).collect();
    best.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    best.into_iter().next().map(|(k, _)| k)
}

/// "agora", "há 20 min", "há 2 h", "ontem", "domingo", "12 nov".
fn when(t: i64, now: i64) -> String {
    let dif = (now - t).max(0);
    if dif < 120 {
        return tr("now").to_string();
    }
    if dif < 3600 {
        return trf("{n} min ago", &[("n", (dif / 60).to_string())]);
    }
    let (Ok(d), Ok(n)) = (glib::DateTime::from_unix_local(t), glib::DateTime::from_unix_local(now)) else { return String::new() };
    let dias = n.day_of_year() - d.day_of_year() + if n.year() != d.year() { 365 } else { 0 };
    if dias == 0 {
        return trf("{n} h ago", &[("n", (dif / 3600).to_string())]);
    }
    if dias == 1 {
        return tr("yesterday").to_string();
    }
    if dias < 7 {
        return d.format("%A").map(|s| s.to_lowercase()).unwrap_or_default();
    }
    d.format("%e %b").map(|s| s.trim().to_lowercase()).unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------------ the screen

pub struct Spaces {
    window: gtk::ApplicationWindow,
    dim: Animavel,
    content: Animavel,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    entry: gtk::Entry,
    build: gtk::Button,
    suggestion: gtk::Box,
    suggestion_text: gtk::Label,
    accept: gtk::Button,
    /// "1…4": as many keys as spaces (up to 9)
    keys_range: gtk::Label,
    open: Cell<bool>,
    opened_at: Cell<Option<Instant>>,
    selected: Cell<usize>,
    /// the cards in order: (space, card, its motion)
    cards: RefCell<Vec<(Espaco, gtk::Box, Animavel)>>,
    thumbs: RefCell<HashMap<u64, Pixbuf>>,
    /// thumbnails asked for and not here yet (a card drawn again does not ask twice)
    asked: RefCell<Vec<u64>>,
    habit: RefCell<Option<(String, u32)>>,
    shell: RefCell<Weak<Shell>>,
    last_space: RefCell<Option<String>>,
    switched_today: RefCell<Vec<String>>,
}

thread_local! {
    static SPACES: RefCell<Option<Rc<Spaces>>> = const { RefCell::new(None) };
}

/// The screen (made at the shell's start).
pub fn get() -> Option<Rc<Spaces>> {
    SPACES.with(|s| s.borrow().clone())
}

/// Makes the screen and follows the spaces.
pub fn start(app: &gtk::Application, shell: &Rc<Shell>) {
    let s = Spaces::new(app);
    s.ligar(shell);
    SPACES.with(|x| *x.borrow_mut() = Some(s));
}

const CARD_W: i32 = 396;
const GAP: i32 = 20;

impl Spaces {
    pub fn new(app: &gtk::Application) -> Rc<Spaces> {
        let window = camada(app, "kalango-espacos", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        let root = gtk::Overlay::new();
        let shade = caixa(Orientation::Vertical, 0, "espacos-escurecer");
        shade.set_hexpand(true);
        shade.set_vexpand(true);
        let dim = Animavel::new(&shade);
        root.set_child(Some(&dim));

        let col = caixa(Orientation::Vertical, 26, "");
        col.set_halign(Align::Center);
        col.set_valign(Align::Start);
        col.set_margin_top(70);
        let head = caixa(Orientation::Vertical, 6, "");
        head.set_halign(Align::Center);
        head.append(&rotulo(tr("Spaces"), "espacos-titulo"));
        head.append(&rotulo(tr("Each space keeps windows, agents and focus rules. Come back and everything is exactly where you left it."), "espacos-sub"));
        col.append(&head);
        let list = caixa(Orientation::Horizontal, GAP, "");
        list.set_halign(Align::Center);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_hscrollbar_policy(gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        col.append(&scroll);

        // "Novo espaço": a sentence, and the agent makes the space
        let new_row = caixa(Orientation::Horizontal, 12, "espacos-novo");
        new_row.set_halign(Align::Center);
        new_row.set_size_request(780, -1);
        let spark = caixa(Orientation::Vertical, 0, "espacos-novo-icone");
        spark.set_size_request(30, 30);
        spark.set_valign(Align::Center);
        let spark_icon = ui_icon("spark", "#A5A1E2", 13);
        spark_icon.set_vexpand(true);
        spark.append(&spark_icon);
        new_row.append(&spark);
        let entry = gtk::Entry::new();
        entry.add_css_class("espacos-novo-campo");
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some(tr("New space: say what you will do — “edit the trip video”")));
        new_row.append(&entry);
        let build = gtk::Button::new();
        build.add_css_class("espacos-montar");
        let bl = caixa(Orientation::Horizontal, 6, "");
        bl.append(&ui_icon("mais", "#1C1D24", 16));
        bl.append(&rotulo(tr("Build space"), ""));
        build.set_child(Some(&bl));
        build.set_valign(Align::Center);
        new_row.append(&build);
        col.append(&new_row);

        // the habit, offered
        let suggestion = caixa(Orientation::Horizontal, 10, "espacos-sugestao");
        suggestion.set_halign(Align::Center);
        suggestion.append(&ui_icon("spark", "#A5A1E2", 16));
        let suggestion_text = rotulo("", "espacos-sugestao-texto");
        suggestion_text.set_wrap(true);
        suggestion_text.set_xalign(0.0);
        suggestion_text.set_max_width_chars(80);
        suggestion.append(&suggestion_text);
        let accept = gtk::Button::with_label(tr("Turn on routine"));
        accept.add_css_class("espacos-ativar");
        accept.set_valign(Align::Center);
        let later = gtk::Button::with_label(tr("Not now"));
        later.add_css_class("espacos-agora-nao");
        later.set_valign(Align::Center);
        suggestion.append(&accept);
        suggestion.append(&later);
        suggestion.set_visible(false);
        col.append(&suggestion);

        // the keys
        let keys = caixa(Orientation::Horizontal, 14, "");
        keys.set_halign(Align::Center);
        let keys_range = rotulo("1…9", "tecla");
        for (caps, what) in [(vec!["Super", "Tab"], tr("switch")), (vec!["Super", ""], tr("go straight")), (vec![tr("Drag")], tr("move a window to a space"))] {
            let k = caixa(Orientation::Horizontal, 4, "");
            for c in caps {
                if c.is_empty() {
                    k.append(&keys_range);
                } else {
                    k.append(&rotulo(c, "tecla"));
                }
            }
            k.append(&rotulo(what, "espacos-dica"));
            keys.append(&k);
        }
        col.append(&keys);
        let content = Animavel::new(&col);
        content.origem(0.5, 0.0);
        content.set_halign(Align::Center);
        content.set_valign(Align::Start);
        root.add_overlay(&content);
        window.set_child(Some(&root));

        let s = Rc::new(Spaces {
            window, dim, content, list, scroll, entry, build, suggestion, suggestion_text, accept: accept.clone(), keys_range, open: Cell::new(false), opened_at: Cell::new(None),
            selected: Cell::new(0), cards: RefCell::default(), thumbs: RefCell::default(), asked: RefCell::default(), habit: RefCell::default(), shell: RefCell::default(),
            last_space: RefCell::default(), switched_today: RefCell::default(),
        });
        // a click on the dim (outside the content) closes
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(&s);
        click.connect_pressed(move |_, _, x, y| {
            let Some(s) = w.upgrade() else { return };
            let inside = s.content.compute_bounds(&s.window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                s.close(None);
            }
        });
        s.dim.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&s);
        keys.connect_key_pressed(move |_, k, _, m| w.upgrade().map(|s| s.key(k, m)).unwrap_or(glib::Propagation::Proceed));
        let w = Rc::downgrade(&s);
        keys.connect_key_released(move |_, k, _, _| {
            // Super+Tab tapped (released at once): straight to the space selected, the one before (as Alt+Tab)
            let Some(s) = w.upgrade() else { return };
            if matches!(k, gdk::Key::Super_L | gdk::Key::Super_R) && s.opened_at.get().is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
                s.go_selected();
            }
        });
        s.window.add_controller(keys);
        let w = Rc::downgrade(&s);
        s.build.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.build_space() });
        let w = Rc::downgrade(&s);
        s.entry.connect_activate(move |_| if let Some(s) = w.upgrade() { s.build_space() });
        let w = Rc::downgrade(&s);
        accept.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.accept_habit() });
        let w = Rc::downgrade(&s);
        later.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.dismiss_habit() });
        s
    }

    /// Follows the spaces: each visit is kept; once a minute, the routines' switches.
    pub fn ligar(self: &Rc<Self>, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
        let w = Rc::downgrade(self);
        shell.estado.ouvir(move || {
            let Some(s) = w.upgrade() else { return };
            let Some(e) = focused_space() else { return };
            let k = key(&e);
            if s.last_space.borrow().as_deref() != Some(k.as_str()) {
                *s.last_space.borrow_mut() = Some(k.clone());
                record_visit(&k);
                if debug() {
                    eprintln!("debug espaços: no espaço {k} ({})", rules_of(&e).interrupt.id());
                }
            }
        });
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(30), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            s.routine_tick();
            glib::ControlFlow::Continue
        });
    }

    /// "Trocar de espaço por rotina": at the hour, to the space (once a day), its agents quiet.
    fn routine_tick(&self) {
        if !by_routine() {
            return;
        }
        let Ok(n) = glib::DateTime::now_local() else { return };
        let today = n.format("%F").map(|s| s.to_string()).unwrap_or_default();
        for sw in read_kept().switches {
            let mark = format!("{today} {}@{}", sw.space, sw.hour);
            if n.hour() as u32 != sw.hour || n.minute() > 10 || self.switched_today.borrow().contains(&mark) {
                continue;
            }
            self.switched_today.borrow_mut().push(mark);
            let espacos = crate::shell_atual().map(|s| s.estado.espacos.borrow().clone()).unwrap_or_default();
            let Some(e) = espacos.iter().find(|e| key(e) == sw.space) else { continue };
            if sw.quiet {
                let mut r = rules_of(e);
                r.agents_quiet = true;
                set_rules(e, r);
            }
            let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": e.id}}}));
            if debug() {
                eprintln!("debug espaços: rotina das {}h → {}", sw.hour, sw.space);
            }
        }
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.open.get() {
            // Super+Tab again: the next space
            self.select((self.selected.get() + 1) % self.cards.borrow().len().max(1));
        } else {
            self.show();
        }
    }

    fn spaces_of_output(&self) -> Vec<Espaco> {
        let Some(shell) = self.shell.borrow().upgrade() else { return Vec::new() };
        let all = shell.estado.espacos.borrow().clone();
        let saida = all.iter().find(|e| e.focado).map(|e| e.saida.clone()).unwrap_or_default();
        let mut l: Vec<Espaco> = all.into_iter().filter(|e| e.saida == saida).collect();
        l.sort_by_key(|e| e.idx);
        // niri keeps an empty space at the end: it is where a new one is made
        let janelas = shell.estado.janelas.borrow();
        while l.len() > 1 && l.last().is_some_and(|e| !janelas.values().any(|j| j.espaco == Some(e.id))) && l.last().is_some_and(|e| e.nome.is_none()) {
            l.pop();
        }
        l
    }

    pub fn show(self: &Rc<Self>) {
        if self.open.replace(true) {
            return;
        }
        self.opened_at.set(Some(Instant::now()));
        // (the windows as they are now: the pictures are taken again)
        self.thumbs.borrow_mut().clear();
        self.render();
        // the space before this one, as Alt+Tab does with windows
        let focused = self.cards.borrow().iter().position(|(e, _, _)| e.focado).unwrap_or(0);
        let visits = read_visits();
        let current = self.cards.borrow().get(focused).map(|(e, _, _)| key(e));
        let before = visits.iter().rev().map(|(k, _)| k).find(|k| Some(*k) != current.as_ref())
            .and_then(|k| self.cards.borrow().iter().position(|(e, _, _)| key(e) == *k));
        self.select(before.unwrap_or(focused));
        self.prepare_habit();
        self.window.present();
        crate::superficies::take_keyboard(&self.window);
        self.dim.definir(Canal::Opacidade, 0.0);
        self.dim.ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.content.entrar((0.0, -8.0), 0.98);
        let motions: Vec<Animavel> = self.cards.borrow().iter().map(|(_, _, a)| a.clone()).collect();
        mv::cascata(&motions, 12.0);
        if debug() {
            eprintln!("debug espaços: aberto ({} espaços)", self.cards.borrow().len());
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                if let Some(s) = w.upgrade() {
                    s.log_places();
                }
            });
        }
    }

    /// Debug: where each card and the new-space controls are (the session test clicks them).
    fn log_places(&self) {
        let centro = |w: &gtk::Widget| w.compute_bounds(&self.window).filter(|_| w.is_mapped()).map(|b| ((b.x() + b.width() / 2.) as i32, (b.y() + b.height() / 2.) as i32));
        for (i, (e, c, _)) in self.cards.borrow().iter().enumerate() {
            if let Some(b) = c.compute_bounds(&self.window) {
                eprintln!("debug espaços: cartão {} {} {} {}", i + 1, key(e), (b.x() + b.width() / 2.) as i32, (b.y() + 60.) as i32);
            }
            // its two rule lines (agents, interruptions)
            let mut regras = Vec::new();
            let mut stack = vec![c.clone().upcast::<gtk::Widget>()];
            while let Some(w) = stack.pop() {
                if w.has_css_class("espacos-regra") {
                    regras.push(w.clone());
                }
                let mut ch = w.last_child();
                while let Some(x) = ch {
                    stack.push(x.clone());
                    ch = x.prev_sibling();
                }
            }
            for (nome, w) in ["agentes", "foco"].iter().zip(regras.iter()) {
                if let Some((x, y)) = centro(w) {
                    eprintln!("debug espaços: {nome} {} em {x} {y}", i + 1);
                }
            }
        }
        if let Some((x, y)) = centro(self.accept.upcast_ref()) {
            eprintln!("debug espaços: ativar em {x} {y}");
        }
        for (nome, w) in [("campo", self.entry.upcast_ref::<gtk::Widget>()), ("montar", self.build.upcast_ref())] {
            if let Some(b) = w.compute_bounds(&self.window) {
                eprintln!("debug espaços: {nome} em {} {}", (b.x() + b.width() / 2.) as i32, (b.y() + b.height() / 2.) as i32);
            }
        }
    }

    pub fn close(&self, go_to: Option<u64>) {
        if !self.open.replace(false) {
            return;
        }
        self.opened_at.set(None);
        self.window.set_keyboard_mode(KeyboardMode::None);
        if let Some(id) = go_to {
            let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": id}}}));
            if debug() {
                eprintln!("debug espaços: para o espaço {id}");
            }
        }
        self.dim.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        let w = self.window.downgrade();
        let c = self.content.downgrade();
        self.content.sair((0.0, -8.0), 0.98, move || {
            if let (Some(w), Some(c)) = (w.upgrade(), c.upgrade()) {
                if c.valor(Canal::Opacidade) < 0.05 {
                    w.set_visible(false);
                }
            }
        });
    }

    fn select(&self, i: usize) {
        self.selected.set(i);
        for (n, (_, c, _)) in self.cards.borrow().iter().enumerate() {
            if n == i { c.add_css_class("escolhido") } else { c.remove_css_class("escolhido") }
        }
    }

    fn go_selected(&self) {
        let id = self.cards.borrow().get(self.selected.get()).map(|(e, _, _)| e.id);
        self.close(id);
    }

    fn key(self: &Rc<Self>, k: gdk::Key, m: gdk::ModifierType) -> glib::Propagation {
        let n = self.cards.borrow().len().max(1);
        let typing = self.entry.has_focus();
        match k {
            gdk::Key::Escape => self.close(None),
            gdk::Key::Return | gdk::Key::KP_Enter if !typing => self.go_selected(),
            gdk::Key::Right | gdk::Key::Tab if !typing && !m.contains(gdk::ModifierType::SHIFT_MASK) => self.select((self.selected.get() + 1) % n),
            gdk::Key::Left | gdk::Key::ISO_Left_Tab | gdk::Key::Tab if !typing => self.select((self.selected.get() + n - 1) % n),
            _ if !typing => {
                let Some(d) = k.to_unicode().and_then(|c| c.to_digit(10)).filter(|d| *d >= 1) else { return glib::Propagation::Proceed };
                let id = self.cards.borrow().iter().find(|(e, _, _)| e.idx == u64::from(d)).map(|(e, _, _)| e.id);
                if id.is_some() {
                    self.close(id);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn render(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        self.cards.borrow_mut().clear();
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        let espacos = self.spaces_of_output();
        let (ow, oh) = crate::superficies::tamanho_da_saida().unwrap_or((1920, 1080));
        // as wide as the design's card while they fit the screen, narrower down to 300, then the row scrolls
        let n = espacos.len().max(1) as i32;
        self.keys_range.set_text(&if n > 1 { format!("1…{}", n.min(9)) } else { "1".to_string() });
        let room = ow - 96;
        let card_w = ((room - GAP * (n - 1)) / n).clamp(300, CARD_W);
        self.scroll.set_min_content_width((card_w * n + GAP * (n - 1)).min(room));
        // (the card's 12 px of padding and its 1 px border on each side)
        let pic_w = card_w - 26;
        let pic_h = pic_w * 209 / 372;
        let papel = crate::visao::papel_pequeno(ow, oh);
        let janelas: Vec<Janela> = shell.estado.janelas.borrow().values().cloned().collect();
        let rotinas = crate::rotinas::Rotinas::carregar();
        let visits = read_visits();
        let t_now = now();
        for e in &espacos {
            let dela: Vec<&Janela> = janelas.iter().filter(|j| j.espaco == Some(e.id)).collect();
            for j in &dela {
                if !self.thumbs.borrow().contains_key(&j.id) {
                    self.request_thumb(j.id);
                }
            }
            let r = rules_of(e);
            let card = caixa(Orientation::Vertical, 10, "espacos-cartao");
            card.set_size_request(card_w, -1);
            let top = caixa(Orientation::Vertical, 10, "");
            // the picture: the wallpaper and the windows where they are
            let pic = gtk::Overlay::new();
            pic.set_size_request(pic_w, pic_h);
            pic.add_css_class("espacos-miniatura");
            pic.set_overflow(gtk::Overflow::Hidden);
            let fixed = gtk::Fixed::new();
            if let Some(p) = &papel {
                fixed.put(&crate::visao::figura(p, pic_w, pic_h), 0.0, 0.0);
            }
            let places = crate::visao::lugares_reais(&dela, f64::from(ow));
            let (kx, ky) = (f64::from(pic_w) / f64::from(ow), f64::from(pic_h) / f64::from(oh));
            for j in &dela {
                let Some(p) = places.get(&j.id) else { continue };
                if p.x >= f64::from(ow) || p.x + p.w <= 0.0 {
                    continue;
                }
                let (w, h) = ((p.w * kx).max(4.0) as i32, (p.h * ky).max(4.0) as i32);
                let mini: gtk::Widget = match self.thumbs.borrow().get(&j.id) {
                    Some(t) => crate::visao::figura(t, w, h).upcast(),
                    None => {
                        let c = caixa(Orientation::Vertical, 0, "visao-mini");
                        c.set_size_request(w, h);
                        c.upcast()
                    }
                };
                mini.add_css_class("visao-mini-borda");
                fixed.put(&mini, p.x * kx, p.y * ky);
            }
            pic.set_child(Some(&fixed));
            top.append(&pic);
            // name, colour, last visit
            let title = caixa(Orientation::Horizontal, 8, "");
            let dot = caixa(Orientation::Vertical, 0, "visao-ponto");
            dot.add_css_class(&format!("cor{}", r.color));
            dot.set_size_request(8, 8);
            dot.set_valign(Align::Center);
            title.append(&dot);
            let name = rotulo(&display_name(e), "espacos-nome");
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_width_chars(1);
            title.append(&name);
            let last = if e.focado { Some(t_now) } else { visits.iter().rev().find(|(k, _)| *k == key(e)).map(|(_, t)| *t) };
            title.append(&rotulo(&last.map(|t| when(t, t_now)).unwrap_or_default(), "espacos-quando"));
            top.append(&title);
            // its apps (each one can be dragged to another space) and its key
            let apps_row = caixa(Orientation::Horizontal, 6, "");
            let mut seen: Vec<String> = Vec::new();
            for j in &dela {
                let Some(a) = apps::da_janela(&j.app_id, &shell.todos) else { continue };
                if seen.contains(&a.id) || seen.len() >= 8 {
                    continue;
                }
                seen.push(a.id.clone());
                let icon = apps::imagem(a, 24);
                icon.set_tooltip_text(Some(&a.nome));
                let drag = gtk::DragSource::new();
                drag.set_actions(gdk::DragAction::MOVE);
                let payload = format!("{}|{}", e.id, j.app_id);
                drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
                icon.add_controller(drag);
                apps_row.append(&icon);
            }
            let gap = caixa(Orientation::Horizontal, 0, "");
            gap.set_hexpand(true);
            apps_row.append(&gap);
            if e.idx <= 9 {
                apps_row.append(&rotulo("Super", "tecla"));
                apps_row.append(&rotulo(&e.idx.to_string(), "tecla"));
            }
            apps_row.set_size_request(-1, 24);
            top.append(&apps_row);
            card.append(&top);
            // the rules (a click changes them)
            let rules = caixa(Orientation::Vertical, 4, "");
            let agent_text = if r.agents_quiet {
                tr("Agents quiet").to_string()
            } else if let Some(rt) = rotinas.lista.iter().find(|x| x.ativa && matches!(&x.gatilho, crate::rotinas::Gatilho::Espaco(n) if *n == key(e))) {
                trf("{routine} on entering", &[("routine", rt.nome.clone())])
            } else {
                tr("Agents may notify").to_string()
            };
            let agent = self.rule_button("spark", "#A5A1E2", &agent_text, "espacos-regra-agente", tr("Click to keep the agents quiet here (or not)"));
            let interrupt = self.rule_button("sino-mudo", "#8E8F99", r.interrupt.label(), "espacos-regra-foco", tr("Click to change what may interrupt this space"));
            rules.append(&agent);
            rules.append(&interrupt);
            card.append(&rules);
            {
                let (w, e2) = (Rc::downgrade(self), e.clone());
                agent.connect_clicked(move |_| {
                    let mut r = rules_of(&e2);
                    r.agents_quiet = !r.agents_quiet;
                    set_rules(&e2, r.clone());
                    if debug() {
                        eprintln!("debug espaços: {} agentes em silêncio {}", key(&e2), r.agents_quiet);
                    }
                    if let Some(s) = w.upgrade() {
                        s.refresh();
                    }
                });
                let (w, e2) = (Rc::downgrade(self), e.clone());
                interrupt.connect_clicked(move |_| {
                    let mut r = rules_of(&e2);
                    r.interrupt = r.interrupt.next();
                    set_rules(&e2, r.clone());
                    if debug() {
                        eprintln!("debug espaços: {} interrompe: {}", key(&e2), r.interrupt.id());
                    }
                    if let Some(s) = w.upgrade() {
                        s.refresh();
                    }
                });
            }
            // a click on the picture or the name goes there
            let go = gtk::GestureClick::new();
            let (w, id) = (Rc::downgrade(self), e.id);
            go.connect_released(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                if let Some(s) = w.upgrade() {
                    s.close(Some(id));
                }
            });
            top.add_controller(go);
            // an app's icon dropped here: its windows come to this space
            let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
            let (w, to) = (Rc::downgrade(self), e.id);
            drop.connect_drop(move |_, v, _, _| {
                let Some(s) = w.upgrade() else { return false };
                let Ok(p) = v.get::<String>() else { return false };
                let Some((from, app)) = p.split_once('|') else { return false };
                if from.parse::<u64>().ok() == Some(to) {
                    return false;
                }
                s.move_app(from.parse().unwrap_or(0), app, to);
                true
            });
            card.add_controller(drop);
            let motion = Animavel::new(&card);
            self.list.append(&motion);
            self.cards.borrow_mut().push((e.clone(), card, motion));
        }
    }

    fn rule_button(&self, icon: &str, color: &str, text: &str, class: &str, tip: &str) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("espacos-regra");
        let row = caixa(Orientation::Horizontal, 6, "");
        row.append(&ui_icon(icon, color, 16));
        row.append(&rotulo(text, class));
        b.set_child(Some(&row));
        b.set_halign(Align::Start);
        b.set_tooltip_text(Some(tip));
        b
    }

    /// The cards drawn again, the selection kept.
    fn refresh(self: &Rc<Self>) {
        let sel = self.selected.get();
        self.render();
        self.select(sel.min(self.cards.borrow().len().saturating_sub(1)));
    }

    fn request_thumb(self: &Rc<Self>, id: u64) {
        if self.asked.borrow().contains(&id) {
            return;
        }
        self.asked.borrow_mut().push(id);
        let path = thumbs_dir().join(format!("espacos-{id}.png"));
        let _ = std::fs::remove_file(&path);
        if niri_acao(json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 480}})).is_err() {
            return;
        }
        let w = Rc::downgrade(self);
        let t0 = Instant::now();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            if path.is_file() {
                let Some(pb) = carregar_miniatura(&path) else { return glib::ControlFlow::Continue };
                let _ = std::fs::remove_file(&path);
                s.asked.borrow_mut().retain(|x| *x != id);
                s.thumbs.borrow_mut().insert(id, pb);
                if s.open.get() {
                    s.refresh();
                }
                return glib::ControlFlow::Break;
            }
            if t0.elapsed() > Duration::from_secs(2) {
                s.asked.borrow_mut().retain(|x| *x != id);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// An app's windows from one space to another.
    fn move_app(self: &Rc<Self>, from: u64, app_id: &str, to: u64) {
        let ids: Vec<u64> = self.shell.borrow().upgrade().map(|s| s.estado.janelas.borrow().values()
            .filter(|j| j.espaco == Some(from) && j.app_id == app_id).map(|j| j.id).collect()).unwrap_or_default();
        for id in &ids {
            let _ = niri_acao(json!({"MoveWindowToWorkspace": {"window_id": id, "reference": {"Id": to}, "focus": false}}));
        }
        if debug() {
            eprintln!("debug espaços: {app_id} ({} janelas) do espaço {from} para o {to}", ids.len());
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(s) = w.upgrade() {
                s.refresh();
            }
        });
    }

    // -------------------------------------------------------------------------------------------- the habit

    fn prepare_habit(&self) {
        let k = read_kept();
        let h = habit(&read_visits(), now(), &k.switches, &k.dismissed, |t| {
            let d = glib::DateTime::from_unix_local(t).ok()?;
            Some((d.to_unix() / 86400, d.hour() as u32))
        });
        // only for a space that is still there
        let h = h.filter(|(s, _)| self.cards.borrow().iter().any(|(e, _, _)| key(e) == *s));
        match &h {
            Some((s, hour)) => {
                self.suggestion_text.set_text(&trf("At {h}h you usually go to {space}. Shall I switch by myself and keep the agents quiet?",
                    &[("h", hour.to_string()), ("space", s.clone())]));
                self.suggestion.set_visible(true);
                if debug() {
                    eprintln!("debug espaços: hábito {s} às {hour}h");
                }
            }
            None => self.suggestion.set_visible(false),
        }
        *self.habit.borrow_mut() = h;
    }

    fn accept_habit(&self) {
        let Some((s, hour)) = self.habit.borrow_mut().take() else { return };
        let mut k = read_kept();
        k.switches.push(AutoSwitch { space: s.clone(), hour, quiet: true });
        write_kept(&k);
        set_by_routine(true);
        // (made during its own hour, it starts tomorrow: the person is already there, and a switch right after
        // the click would take them away from what they are doing)
        if let Ok(n) = glib::DateTime::now_local() {
            if n.hour() as u32 == hour {
                let today = n.format("%F").map(|d| d.to_string()).unwrap_or_default();
                self.switched_today.borrow_mut().push(format!("{today} {s}@{hour}"));
            }
        }
        self.suggestion.set_visible(false);
        if debug() {
            eprintln!("debug espaços: rotina ativada ({s} às {hour}h)");
        }
    }

    fn dismiss_habit(&self) {
        let Some((s, hour)) = self.habit.borrow_mut().take() else { return };
        let mut k = read_kept();
        k.dismissed.push(format!("{s}@{hour}"));
        write_kept(&k);
        self.suggestion.set_visible(false);
    }

    // -------------------------------------------------------------------------------------------- "Montar espaço"

    /// The sentence to the local model: a name and the apps; the space is the empty one niri keeps at the end, named,
    /// with the apps opened in it.
    fn build_space(self: &Rc<Self>) {
        let pedido = self.entry.text().trim().to_string();
        if pedido.is_empty() {
            self.entry.grab_focus();
            return;
        }
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        self.entry.set_sensitive(false);
        self.build.set_sensitive(false);
        if debug() {
            eprintln!("debug espaços: montar “{pedido}”");
        }
        // (each app with what it is for: its desktop categories — its description too made the small model answer
        // names instead of ids, and fewer apps)
        let lista: Vec<Value> = shell.todos.iter().map(|a| json!({"id": a.id, "nome": a.nome,
            "tipo": a.info.categories().map(|c| c.to_string()).unwrap_or_default().trim_end_matches(';').replace(';', ", ")})).collect();
        let input = json!({"pedido": pedido, "apps": lista}).to_string();
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        let fallback = pedido.chars().take(24).collect::<String>();
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("espaco").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(input.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            serde_json::from_slice::<Value>(&o.stdout).ok()
        }, move |v| {
            let Some(s) = w.upgrade() else { return };
            s.entry.set_sensitive(true);
            s.build.set_sensitive(true);
            let nome = v.as_ref().and_then(|v| v["nome"].as_str()).map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or(fallback);
            let ids: Vec<String> = v.as_ref().and_then(|v| v["apps"].as_array().cloned()).unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
            s.make_space(&nome, &ids);
        });
    }

    fn make_space(self: &Rc<Self>, nome: &str, ids: &[String]) {
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        // the empty space at the end of this output
        let all = shell.estado.espacos.borrow().clone();
        let saida = all.iter().find(|e| e.focado).map(|e| e.saida.clone()).unwrap_or_default();
        let janelas = shell.estado.janelas.borrow().clone();
        let empty = all.iter().filter(|e| e.saida == saida).max_by_key(|e| e.idx).filter(|e| !janelas.values().any(|j| j.espaco == Some(e.id))).cloned();
        let Some(e) = empty else { return };
        let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": e.id}}}));
        let _ = niri_acao(json!({"SetWorkspaceName": {"name": nome, "workspace": {"Id": e.id}}}));
        let used: Vec<usize> = read_kept().rules.values().map(|r| r.color).collect();
        let color = (0..crate::visao::DOTS.len()).find(|c| !used.contains(c)).unwrap_or((e.idx as usize).saturating_sub(1) % crate::visao::DOTS.len());
        let mut k = read_kept();
        k.rules.insert(nome.to_string(), Rules { color, interrupt: Interrupt::Approvals, agents_quiet: false });
        write_kept(&k);
        // an app already open (a single-instance one would only raise its window where it is) brings its window here;
        // the others are opened in it
        let mut opened = Vec::new();
        for id in ids {
            let Some(a) = shell.todos.iter().find(|a| a.id == *id) else { continue };
            let existing = janelas.values().filter(|j| apps::da_janela(&j.app_id, &shell.todos).is_some_and(|x| x.id == a.id))
                .max_by_key(|j| j.foco_em).map(|j| j.id);
            let ok = match existing {
                Some(w) => niri_acao(json!({"MoveWindowToWorkspace": {"window_id": w, "reference": {"Id": e.id}, "focus": false}})).is_ok(),
                None => apps::abrir(a).is_ok(),
            };
            if ok {
                opened.push(a.id.clone());
            }
        }
        // (an app's activation may have taken the focus elsewhere)
        let id = e.id;
        glib::timeout_add_local_once(Duration::from_millis(1200), move || {
            let _ = niri_acao(json!({"FocusWorkspace": {"reference": {"Id": id}}}));
        });
        if debug() {
            eprintln!("debug espaços: espaço montado “{nome}” (espaço {}, apps: {})", e.id, opened.join(", "));
        }
        self.entry.set_text("");
        self.close(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_habit_is_three_days_at_the_same_hour() {
        let day = 86400;
        let now = 30 * day;
        let at = |d: i64, h: i64| now - d * day + h * 3600 - 12 * 3600;
        let local = |t: i64| Some((t / 86400, ((t % 86400) / 3600) as u32));
        let visits: Vec<(String, i64)> = vec![
            ("Relaxar".into(), at(1, 19)), ("Relaxar".into(), at(2, 19)), ("Relaxar".into(), at(2, 19) + 600), ("Relaxar".into(), at(3, 19)),
            ("Estudar".into(), at(1, 9)), ("Estudar".into(), at(2, 9)), ("#3".into(), at(1, 8)), ("#3".into(), at(2, 8)), ("#3".into(), at(3, 8)),
        ];
        let hour = local(at(1, 19)).unwrap().1;
        assert_eq!(habit(&visits, now, &[], &[], local), Some(("Relaxar".to_string(), hour)));
        // already switched by routine, or turned down: nothing
        let sw = vec![AutoSwitch { space: "Relaxar".into(), hour, quiet: true }];
        assert_eq!(habit(&visits, now, &sw, &[], local), None);
        assert_eq!(habit(&visits, now, &[], &[format!("Relaxar@{hour}")], local), None);
    }

    #[test]
    fn interruption_rules_cycle() {
        assert_eq!(Interrupt::All.next().next().next(), Interrupt::All);
        assert_eq!(Interrupt::from_id(Interrupt::Hold.id()), Interrupt::Hold);
    }
}
