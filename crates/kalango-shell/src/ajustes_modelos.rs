//! Agents center › Ajustes › Modelos e provedores (design "Ajustes · Modelos e provedores", dark and light): the
//! local models of KalangoOS by capability (text, vision, voice, omni; the decision models apart), the free memory
//! and what it allows, the cloud providers (OpenAI-compatible, with a token kept in the vault), and on the right the
//! routing order, "personal data never leaves the computer", retries and the usage and cost per agent.
//!
//! Everything comes from kalango-fluxos (`models.local`, `models.providers`, `models.usage`, `agents.list`,
//! `secrets.list`) and changes through it (`models.set_providers`, `models.set_routing`, `secrets.store`). Nothing is
//! downloaded here: a model that is not installed only says so. Below ~900 px the right column goes under the list.

use crate::conversa::ui_icon;
use crate::fluxos_estados::{self as estados, action_button};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

// ---------------------------------------------------------------- pure pieces (tested below)

/// (MemAvailable, MemTotal) in bytes, from the text of /proc/meminfo.
pub(crate) fn mem_info(meminfo: &str) -> (u64, u64) {
    let field = |k: &str| meminfo.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 1024;
    (field("MemAvailable:"), field("MemTotal:"))
}

/// The engine's rule (kalango-agente, modelo.rs `fits`): a model starts beside the others only when the free memory
/// holds twice its files and 1 GiB more; otherwise the idle ones are ended first.
pub(crate) fn fits(available: u64, need: u64) -> bool {
    available > need.saturating_mul(2) + (1 << 30)
}

/// "7,8 GB free of 16 GB · one large model at a time" (or room for more than one).
pub(crate) fn memory_headline(available: u64, total: u64, largest_extra: u64) -> String {
    let base = trf("{free} free of {total}", &[("free", crate::app_store_data::gb(available as f64)), ("total", crate::app_store_data::gb(total as f64))]);
    if largest_extra == 0 || fits(available, largest_extra) {
        format!("{base} · {}", tr("room for more than one model"))
    } else {
        format!("{base} · {}", tr("one large model at a time"))
    }
}

/// The line under each place of the routing order.
pub(crate) fn routing_hint(index: usize, local: bool, enabled: bool) -> &'static str {
    match (enabled, index, local) {
        (false, ..) => tr("turned off: skipped"),
        (_, 0, true) => tr("always first"),
        (_, 0, false) => tr("first"),
        (_, _, true) => tr("on this computer"),
        (_, 1, false) => tr("if the agent allows the cloud"),
        _ => tr("only if the ones above fail"),
    }
}

/// The first moment of this month on this computer's clock, in unix seconds (for `models.usage` "since").
pub(crate) fn month_start() -> i64 {
    glib::DateTime::now_local()
        .and_then(|n| glib::DateTime::from_local(n.year(), n.month(), 1, 0, 0, 0.0))
        .map(|d| d.to_unix())
        .unwrap_or(0)
}

/// How costs are shown (`models.currency`): the service counts micro-USD, the person reads them in this currency.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Currency {
    pub code: String,
    pub per_usd: f64,
}

impl Default for Currency {
    fn default() -> Self {
        Currency { code: "USD".into(), per_usd: 1.0 }
    }
}

impl Currency {
    pub(crate) fn from_value(v: &Value) -> Currency {
        let code = v["code"].as_str().filter(|c| c.len() == 3).map(str::to_uppercase);
        let per = v["per_usd"].as_f64().filter(|r| r.is_finite() && *r > 0.0);
        match (code, per) {
            (Some(code), Some(per_usd)) => Currency { code, per_usd },
            _ => Currency::default(),
        }
    }

    pub(crate) fn symbol(&self) -> String {
        match self.code.as_str() {
            "BRL" => "R$".into(),
            "USD" => "US$".into(),
            "EUR" => "€".into(),
            c => c.into(),
        }
    }

    /// "R$ 4,12" from micro-USD.
    pub(crate) fn money(&self, micro_usd: i64) -> String {
        format!("{} {}", self.symbol(), format!("{:.2}", micro_usd as f64 / 1e6 * self.per_usd).replace('.', ","))
    }

    /// An amount typed in this currency ("10", "2,50"), as micro-USD; None: not an amount.
    pub(crate) fn to_micro_usd(&self, text: &str) -> Option<u64> {
        let t = text.trim().trim_start_matches(self.symbol().as_str()).trim().to_string();
        parse_price(&t).map(|v| (v as f64 / self.per_usd).round() as u64)
    }

    /// micro-USD as typed back in this currency ("10", "" for none).
    pub(crate) fn field(&self, micro_usd: u64) -> String {
        price_field((micro_usd as f64 * self.per_usd).round() as u64)
    }

    /// "R$ 5,40/US$".
    pub(crate) fn rate_text(&self) -> String {
        format!("{} {}/US$", self.symbol(), format!("{:.2}", self.per_usd).replace('.', ","))
    }
}

/// "182 thousand tokens", "1,2 million tokens".
pub(crate) fn tokens_text(n: i64) -> String {
    if n >= 1_000_000 {
        trf("{n} million tokens", &[("n", format!("{:.1}", n as f64 / 1e6).replace('.', ","))])
    } else if n >= 1000 {
        trf("{n} thousand tokens", &[("n", ((n + 500) / 1000).to_string())])
    } else {
        trf("{n} tokens", &[("n", n.to_string())])
    }
}

/// One agent's line of "Uso e custo por agente".
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentUsage {
    pub id: String,
    pub name: String,
    pub tokens: i64,
    pub cost: i64,
    pub cloud: bool,
}

/// The rows of `models.usage` (per agent and provider) summed per agent, the costliest first.
pub(crate) fn usage_by_agent(rows: &[Value], names: &BTreeMap<String, String>) -> Vec<AgentUsage> {
    let mut m: BTreeMap<String, AgentUsage> = BTreeMap::new();
    for r in rows {
        let id = r["agent_id"].as_str().unwrap_or("");
        let name = if id.is_empty() { tr("Without an agent").to_string() } else { names.get(id).cloned().unwrap_or_else(|| id.to_string()) };
        let e = m.entry(id.to_string()).or_insert(AgentUsage { id: id.to_string(), name, tokens: 0, cost: 0, cloud: false });
        e.tokens += r["tokens"].as_i64().unwrap_or(0);
        e.cost += r["cost_micro_usd"].as_i64().unwrap_or(0);
        e.cloud |= r["provider"].as_str().is_some_and(|p| p != "local");
    }
    let mut v: Vec<AgentUsage> = m.into_values().collect();
    v.sort_by(|a, b| b.cost.cmp(&a.cost).then(b.tokens.cmp(&a.tokens)).then(a.name.cmp(&b.name)));
    v
}

/// The share of calls answered on this computer, in percent (None: nothing counted yet).
pub(crate) fn local_share(rows: &[Value]) -> Option<u32> {
    let (mut local, mut all) = (0i64, 0i64);
    for r in rows {
        let n = r["calls"].as_i64().unwrap_or(0);
        all += n;
        if r["provider"] == "local" {
            local += n;
        }
    }
    (all > 0).then(|| ((local as f64 / all as f64) * 100.0).round() as u32)
}

/// A provider id from its name, not used yet ("Open Router" → "open-router", then "open-router-2").
pub(crate) fn provider_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let base = base.trim_matches('-').chars().take(40).collect::<String>();
    let base = if base.is_empty() || base == "local" { "provedor".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Moves an item of the order (drag or the arrows).
pub(crate) fn move_item<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from < v.len() && to < v.len() && from != to {
        let x = v.remove(from);
        v.insert(to, x);
    }
}

/// Whether a tab's right column goes under the list. Under it when the list and the column together are under 900 px,
/// the list would be under 560 px, or the window is already wider than the screen (GTK could not shrink it: the
/// column beside is what does not fit). Beside it again when the list keeps 560 px next to it and
/// the window is not wider than the screen. `screen_width` 0: unknown.
pub(crate) fn narrow_next(narrow: bool, list_width: i32, side: i32, window_width: i32, screen_width: i32) -> bool {
    let overflows = screen_width > 0 && window_width > screen_width;
    if !narrow {
        list_width + side < 900 || list_width < 560 || overflows
    } else {
        // (under it, the list already spans the window: the column beside takes its room from the list, the window
        // does not grow)
        !(list_width - side >= 560 && list_width >= 900 && !overflows)
    }
}

/// A price typed in US$ per million tokens ("0,50", "1.5", "" = 0) as the service keeps it (micro-USD per million
/// tokens). None: not a price.
pub(crate) fn parse_price(text: &str) -> Option<u64> {
    let t = text.trim().trim_start_matches("US$").trim().replace(',', ".");
    if t.is_empty() {
        return Some(0);
    }
    let v: f64 = t.parse().ok()?;
    (0.0..1e6).contains(&v).then(|| (v * 1e6).round() as u64)
}

/// A price kept by the service, as typed back into the form ("0,5", "" for free).
pub(crate) fn price_field(micro: u64) -> String {
    if micro == 0 {
        return String::new();
    }
    let s = format!("{:.6}", micro as f64 / 1e6);
    s.trim_end_matches('0').trim_end_matches('.').replace('.', ",")
}

/// The limits a provider is called within (simultaneous requests, per minute, prices): Err(why) in plain words.
pub(crate) fn parse_limits(parallel: &str, per_minute: &str, price_in: &str, price_out: &str) -> Result<(u64, u64, u64, u64), &'static str> {
    let par = if parallel.trim().is_empty() { 2 } else { parallel.trim().parse::<u64>().ok().filter(|n| (1..=64).contains(n)).ok_or(tr("Requests at the same time go from 1 to 64."))? };
    let pm = if per_minute.trim().is_empty() { 0 } else { per_minute.trim().parse::<u64>().ok().filter(|n| *n <= 100_000).ok_or(tr("Requests per minute is a whole number (0: no limit)."))? };
    let pi = parse_price(price_in).ok_or(tr("The price is in US$ per million tokens, like 0,50."))?;
    let po = parse_price(price_out).ok_or(tr("The price is in US$ per million tokens, like 0,50."))?;
    Ok((par, pm, pi, po))
}

/// "2 at a time · 20 per minute · US$ 0,50 / 1,50 per million tokens" for a provider's row.
pub(crate) fn limits_text(p: &Value) -> String {
    let par = p["max_parallel"].as_u64().unwrap_or(2);
    let pm = p["per_minute"].as_u64().unwrap_or(0);
    let (pi, po) = (p["price_in"].as_u64().unwrap_or(0), p["price_out"].as_u64().unwrap_or(0));
    let mut v = vec![trf("{n} at a time", &[("n", par.to_string())])];
    v.push(if pm == 0 { tr("no limit per minute").to_string() } else { trf("{n} per minute", &[("n", pm.to_string())]) });
    if pi == 0 && po == 0 {
        v.push(tr("no price set").to_string());
    } else {
        let f = |m: u64| format!("{:.2}", m as f64 / 1e6).replace('.', ",");
        v.push(trf("US$ {in} / {out} per million tokens", &[("in", f(pi)), ("out", f(po))]));
    }
    v.join(" · ")
}

/// The cloud cost of `models.usage` rows (the local model costs nothing).
pub(crate) fn cloud_cost(rows: &[Value]) -> i64 {
    rows.iter().filter(|r| r["provider"] != "local").map(|r| r["cost_micro_usd"].as_i64().unwrap_or(0)).sum()
}

/// An agent's monthly cloud budget (an assistant's `config.monthly_budget_micro_usd`; 0: none).
pub(crate) fn agent_budget(agent: &Value) -> u64 {
    agent["config"]["monthly_budget_micro_usd"].as_u64().unwrap_or(0)
}

/// How much of a budget is spent, 0 to 1.
pub(crate) fn budget_fraction(cost: i64, budget: u64) -> f64 {
    if budget == 0 {
        return 0.0;
    }
    (cost.max(0) as f64 / budget as f64).clamp(0.0, 1.0)
}

/// What a new provider needs: Err(why) in plain words.
pub(crate) fn check_provider(name: &str, url: &str, model: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err(tr("Give the provider a name."));
    }
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) || u.len() < 12 {
        return Err(tr("The address starts with https:// (for example https://openrouter.ai/api/v1)."));
    }
    if model.trim().is_empty() {
        return Err(tr("Say which model of this provider to use."));
    }
    Ok(())
}

// ---------------------------------------------------------------- the local models

/// Each kind of `models.local`: (kind, name, what it is, what it does, glyph, tile tone, detail when idle).
struct Kind {
    id: &'static str,
    /// its word in the summary ("modelos locais (texto, visão, fala, omni)")
    short: &'static str,
    name: &'static str,
    chip: &'static str,
    caps: &'static [&'static str],
    glyph: &'static str,
    tone: &'static str,
    extra: &'static str,
    lfm: bool,
}

const LOCAL: &[Kind] = &[
    Kind { id: "texto", short: kalango_i18n::n_("text"), name: "LFM3 1.7B", chip: kalango_i18n::n_("text and tools"), caps: &[kalango_i18n::n_("text"), kalango_i18n::n_("tools")], glyph: "cpu", tone: "teal", extra: kalango_i18n::n_("agents' default"), lfm: true },
    Kind { id: "visao", short: kalango_i18n::n_("vision"), name: "LFM3-VL 2B", chip: kalango_i18n::n_("vision"), caps: &[kalango_i18n::n_("image"), kalango_i18n::n_("screen"), kalango_i18n::n_("PDF pages")], glyph: "olho", tone: "azul", extra: kalango_i18n::n_("unloads after 10 min idle"), lfm: true },
    Kind { id: "audio", short: kalango_i18n::n_("speech"), name: "LFM3-Audio 2B", chip: kalango_i18n::n_("speech"), caps: &[kalango_i18n::n_("audio → text"), kalango_i18n::n_("text → voice")], glyph: "microfone", tone: "roxo", extra: kalango_i18n::n_("voices: female, male"), lfm: true },
    Kind { id: "omni", short: kalango_i18n::n_("omni"), name: "LFM3-Omni", chip: kalango_i18n::n_("text, image and audio in one"), caps: &[kalango_i18n::n_("all together")], glyph: "spark", tone: "ouro", extra: kalango_i18n::n_("unloads after 10 min idle"), lfm: true },
];

const DECISION: &[Kind] = &[
    Kind { id: "decisao", short: "", name: "d1-2B", chip: kalango_i18n::n_("decides on text and image"), caps: &[kalango_i18n::n_("text"), kalango_i18n::n_("image"), kalango_i18n::n_("probabilities")], glyph: "compartilhar", tone: "azul", extra: kalango_i18n::n_("unloads after 10 min idle"), lfm: false },
    Kind { id: "decisao-omni", short: "", name: "d1-Omni-2B", chip: kalango_i18n::n_("also decides on audio"), caps: &[kalango_i18n::n_("text"), kalango_i18n::n_("image"), kalango_i18n::n_("audio")], glyph: "compartilhar", tone: "roxo", extra: kalango_i18n::n_("unloads after 10 min idle"), lfm: false },
];

fn tone_hex(t: &str) -> &'static str {
    match t {
        "teal" => "#8ED2C3",
        "azul" => "#7CC3DF",
        "roxo" => "#C7A7DC",
        "ouro" => "#E5C98C",
        _ => "#A5A1E2",
    }
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

fn chip(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, "fx-chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_valign(Align::Center);
    l
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// An icon on its tinted tile (the rows' left).
pub(crate) fn tile(glyph: &str, tone: &str, size: i32) -> gtk::Box {
    let t = caixa(Orientation::Horizontal, 0, "fx-tile");
    t.add_css_class(tone);
    t.set_size_request(size, size);
    t.set_valign(Align::Start);
    t.set_hexpand(false);
    let g: gtk::Widget = if glyph == "spark" { estrela(13, tone_hex(tone)).upcast() } else { ui_icon(glyph, tone_hex(tone), 15).upcast() };
    g.set_hexpand(true);
    t.append(&g);
    t
}

/// A section's name in capitals.
pub(crate) fn section(t: &str) -> gtk::Label {
    let l = left(t, "fx-secao");
    l.set_margin_top(14);
    l.set_margin_bottom(8);
    l
}

// ---------------------------------------------------------------- the two columns, the right one under the list when narrow

/// The right column of a tab: beside the list (in the Ajustes side stack) or, below ~900 px, at the end of the list.
pub(crate) struct Columns {
    /// what goes in the Ajustes side stack
    pub side: gtk::Box,
    scroller: gtk::ScrolledWindow,
    /// the column's content (moves between `scroller` and `below`)
    pub content: gtk::Box,
    below: gtk::Box,
    width: i32,
    narrow: Cell<bool>,
}

impl Columns {
    /// `below`: the place at the end of the list; `measure_in`: the list's column (its width decides).
    pub fn new(width: i32, below: &gtk::Box, measure_in: &gtk::Box) -> Rc<Columns> {
        let side = caixa(Orientation::Vertical, 0, "central-painel");
        side.add_css_class("fx-lado");
        side.set_size_request(width, -1);
        side.set_hexpand(false);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        let content = caixa(Orientation::Vertical, 0, "");
        content.set_vexpand(true);
        scroller.set_child(Some(&content));
        side.append(&scroller);
        let c = Rc::new(Columns { side, scroller, content, below: below.clone(), width, narrow: Cell::new(false) });
        let medidor = gtk::DrawingArea::new();
        medidor.set_can_target(false);
        medidor.set_hexpand(true);
        measure_in.prepend(&medidor);
        let w = Rc::downgrade(&c);
        medidor.connect_resize(move |m, largura, _| {
            let (w, m) = (w.clone(), m.clone());
            glib::idle_add_local_once(move || {
                let Some(c) = w.upgrade() else { return };
                // the window and the screen it is on: a window wider than the screen is one GTK could not shrink
                let janela = m.root().map(|r| r.width()).unwrap_or(0);
                let tela = m
                    .native()
                    .and_then(|n| n.surface())
                    .and_then(|s| WidgetExt::display(&m).monitor_at_surface(&s))
                    .map(|mo| mo.geometry().width())
                    .unwrap_or(0);
                c.decide(largura, janela, tela);
            });
        });
        c
    }

    fn decide(&self, list_width: i32, window_width: i32, screen_width: i32) {
        let n = narrow_next(self.narrow.get(), list_width, self.width, window_width, screen_width);
        if n != self.narrow.get() {
            self.set_narrow(n);
        }
    }

    fn set_narrow(&self, narrow: bool) {
        self.narrow.set(narrow);
        if narrow {
            self.scroller.set_child(None::<&gtk::Widget>);
            self.below.append(&self.content);
            self.content.add_css_class("fx-lado-abaixo");
            self.scroller.set_visible(false);
            self.side.remove_css_class("central-painel");
            self.side.set_size_request(0, -1);
        } else {
            self.below.remove(&self.content);
            self.content.remove_css_class("fx-lado-abaixo");
            self.scroller.set_child(Some(&self.content));
            self.scroller.set_visible(true);
            self.side.add_css_class("central-painel");
            self.side.set_size_request(self.width, -1);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug ajustes: coluna direita {}", if narrow { "abaixo" } else { "ao lado" });
        }
    }
}

// ---------------------------------------------------------------- the tab

#[derive(Default)]
struct Data {
    local: Vec<Value>,
    providers: Vec<Value>,
    routing: Value,
    usage: Vec<Value>,
    agents: BTreeMap<String, String>,
    /// the agents as the service keeps them (for their monthly budget)
    agent_items: BTreeMap<String, Value>,
    /// `models.usage` since the first day of this month
    usage_month: Vec<Value>,
    currency: Currency,
    secrets: Vec<String>,
}

pub struct ModelsTab {
    pub body: gtk::Box,
    pub cols: Rc<Columns>,
    screen: estados::Screen,
    numeros: [(gtk::Label, gtk::Label); 4],
    mem_title: gtk::Label,
    locais: gtk::Box,
    decisao: gtk::Box,
    nuvem: gtk::Box,
    rota: gtk::Box,
    dados: gtk::Switch,
    tentar: gtk::Switch,
    tentar_sub: gtk::Label,
    uso: gtk::Box,
    uso_titulo: gtk::Label,
    // the form of a new provider
    form: gtk::Revealer,
    f_nome: gtk::Entry,
    f_url: gtk::Entry,
    f_modelo: gtk::Entry,
    f_token: gtk::Entry,
    f_token_label: gtk::Label,
    f_parallel: gtk::Entry,
    f_per_minute: gtk::Entry,
    f_price_in: gtk::Entry,
    f_price_out: gtk::Entry,
    f_title: gtk::Label,
    /// the provider being edited (None: a new one)
    editing: RefCell<Option<String>>,
    f_erro: gtk::Label,
    f_salvar: gtk::Button,
    data: RefCell<Data>,
    filling: Cell<bool>,
    loaded: Cell<bool>,
    remove_armed: RefCell<Option<String>>,
}

impl ModelsTab {
    pub fn new(side_width: i32) -> Rc<ModelsTab> {
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_margin_bottom(16);
        // the intro and "Adicionar provedor"
        let topo = caixa(Orientation::Horizontal, 16, "");
        topo.set_margin_top(16);
        let exp = left(tr("The local model runs on this computer and is the default. Cloud providers only come in when you keep a token and an agent may use them."), "central-sub");
        exp.set_wrap(true);
        exp.set_max_width_chars(1);
        exp.set_hexpand(true);
        topo.append(&exp);
        let adicionar = gtk::Button::new();
        adicionar.add_css_class("rotinas-nova");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.append(&ui_icon("mais", "#1C1D24", 14));
        ac.append(&rotulo(tr("Add provider"), ""));
        adicionar.set_child(Some(&ac));
        adicionar.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes adicionar provedor", adicionar.upcast_ref());
        topo.append(&adicionar);
        col.append(&topo);

        // the new provider (inline, under the intro)
        let form = gtk::Revealer::new();
        form.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        form.set_transition_duration(160);
        let fb = caixa(Orientation::Vertical, 8, "fx-form");
        fb.set_margin_top(14);
        let fh = caixa(Orientation::Horizontal, 8, "");
        fh.append(&ui_icon("globo", "#A5A1E2", 14));
        let f_title = left(tr("NEW CLOUD PROVIDER"), "central-painel-agente");
        fh.append(&f_title);
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_hexpand(true);
        fh.append(&sp);
        fh.append(&chip(tr("OpenAI-compatible"), "info"));
        fb.append(&fh);
        let campo = |rotulo_t: &str, dica: &str, mono: bool, nome_debug: &str| {
            let l = left(rotulo_t, "fx-campo-rotulo");
            l.set_margin_top(4);
            fb.append(&l);
            let e = gtk::Entry::new();
            e.add_css_class("fx-campo");
            if mono {
                e.add_css_class("mono");
            }
            e.set_placeholder_text(Some(dica));
            crate::central::posicao_debug_sempre(&format!("ajustes provedor {nome_debug}"), e.upcast_ref());
            fb.append(&e);
            e
        };
        let f_nome = campo(tr("Name"), "OpenRouter", false, "nome");
        let f_url = campo(tr("Base address (OpenAI-compatible API)"), "https://openrouter.ai/api/v1", true, "url");
        let f_modelo = campo(tr("Model"), "meta-llama/llama-3.3-70b-instruct", true, "modelo");
        let f_token = campo(tr("Token (kept in the vault; leave empty for a server without a key)"), "sk-…", true, "token");
        let f_token_label = f_token.prev_sibling().and_downcast::<gtk::Label>().unwrap_or_else(|| left("", ""));
        f_token.set_visibility(false);
        f_token.set_input_purpose(gtk::InputPurpose::Password);
        f_token.set_secondary_icon_paintable(None::<&gdk::Paintable>);
        let olho = gtk::ToggleButton::new();
        olho.add_css_class("fx-olho");
        olho.set_child(Some(&ui_icon("olho", "#8E8F99", 14)));
        olho.set_tooltip_text(Some(tr("Show what was typed")));
        let ft = f_token.clone();
        olho.connect_toggled(move |o| ft.set_visibility(o.is_active()));
        // the eye beside the field (only before saving: once kept, the value never comes back)
        if let Some(p) = f_token.parent().and_downcast::<gtk::Box>() {
            p.remove(&f_token);
            let linha = caixa(Orientation::Horizontal, 6, "");
            f_token.set_hexpand(true);
            linha.append(&f_token);
            olho.set_valign(Align::Center);
            linha.append(&olho);
            p.append(&linha);
        }
        let nota = left(tr("The token is kept as “NAME.token” in the KalangoOS vault; the service only keeps its name."), "central-agente-sub");
        nota.set_wrap(true);
        nota.set_max_width_chars(1);
        fb.append(&nota);
        // the limits it is called within (the service's Provider: max_parallel, per_minute, price_in, price_out)
        let limites = caixa(Orientation::Horizontal, 10, "");
        limites.set_homogeneous(true);
        limites.set_margin_top(4);
        let pequeno = |rotulo_t: &str, dica: &str, nome_debug: &str| {
            let c = caixa(Orientation::Vertical, 4, "");
            let l = left(rotulo_t, "fx-campo-rotulo");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            l.set_tooltip_text(Some(rotulo_t));
            c.append(&l);
            let e = gtk::Entry::new();
            e.add_css_class("fx-campo");
            e.set_width_chars(4);
            e.set_placeholder_text(Some(dica));
            crate::central::posicao_debug_sempre(&format!("ajustes provedor {nome_debug}"), e.upcast_ref());
            c.append(&e);
            limites.append(&c);
            e
        };
        let f_parallel = pequeno(tr("At the same time"), "2", "simultaneos");
        let f_per_minute = pequeno(tr("Per minute (0: no limit)"), "0", "por minuto");
        let f_price_in = pequeno(tr("US$ per million in"), "0,00", "preco entrada");
        let f_price_out = pequeno(tr("US$ per million out"), "0,00", "preco saida");
        fb.append(&limites);
        let f_erro = left("", "rotinas-erro");
        f_erro.set_wrap(true);
        f_erro.set_visible(false);
        fb.append(&f_erro);
        let fbs = caixa(Orientation::Horizontal, 8, "");
        fbs.set_halign(Align::End);
        fbs.set_margin_top(4);
        let f_cancelar = action_button(tr("Cancel"), "", "");
        let f_salvar = action_button(tr("Add provider"), "mais", "principal");
        crate::central::posicao_debug_sempre("ajustes provedor salvar", f_salvar.upcast_ref());
        fbs.append(&f_cancelar);
        fbs.append(&f_salvar);
        fb.append(&fbs);
        form.set_child(Some(&fb));
        col.append(&form);

        // the numbers
        let stats = caixa(Orientation::Horizontal, 12, "");
        stats.set_homogeneous(true);
        stats.set_margin_top(14);
        let stat = |c: &str| {
            let b = caixa(Orientation::Vertical, 4, "linha-stat");
            b.add_css_class("fx-stat");
            let n = left("", "linha-stat-numero");
            if !c.is_empty() {
                n.add_css_class(c);
            }
            b.append(&n);
            let l = left("", "central-agente-sub");
            l.set_wrap(true);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            b.append(&l);
            stats.append(&b);
            (n, l)
        };
        let numeros = [stat("teal"), stat(""), stat("teal"), stat("gold")];
        col.append(&stats);

        // the memory
        let mem = caixa(Orientation::Horizontal, 12, "fx-memoria");
        mem.set_margin_top(14);
        mem.append(&ui_icon("cpu", "#7CC3DF", 16));
        let mc = caixa(Orientation::Vertical, 2, "");
        mc.set_hexpand(true);
        let mem_title = left("", "fx-memoria-titulo");
        mem_title.set_wrap(true);
        mc.append(&mem_title);
        let ms = left(tr("With little memory, the system unloads the idle model before loading another (it takes a few seconds)."), "central-agente-sub");
        ms.set_wrap(true);
        ms.set_max_width_chars(1);
        mc.append(&ms);
        mem.append(&mc);
        col.append(&mem);

        col.append(&section(tr("ON THE COMPUTER")));
        let locais = caixa(Orientation::Vertical, 8, "");
        col.append(&locais);
        col.append(&section(tr("DECISION (NO TEXT GENERATED)")));
        let decisao = caixa(Orientation::Vertical, 8, "");
        col.append(&decisao);
        col.append(&section(tr("IN THE CLOUD")));
        let nuvem = caixa(Orientation::Vertical, 8, "");
        col.append(&nuvem);
        let abaixo = caixa(Orientation::Vertical, 0, "");
        col.append(&abaixo);

        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&col));

        // the right column
        let cols = Columns::new(side_width, &abaixo, &col);
        let lado = &cols.content;
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("compartilhar", "#A5A1E2", 14));
        cab.append(&rotulo(tr("ROUTING"), "central-painel-agente"));
        lado.append(&cab);
        let rs = left(tr("The order in which the system tries the models when the chosen one fails or is busy."), "central-agente-sub");
        rs.set_wrap(true);
        rs.set_max_width_chars(1);
        rs.set_margin_top(10);
        lado.append(&rs);
        let rota = caixa(Orientation::Vertical, 4, "fx-rota");
        rota.set_margin_top(14);
        lado.append(&rota);
        // "Automático pela entrada": the agents' default mode (model_mode "auto") picks by what comes in
        let auto = caixa(Orientation::Horizontal, 8, "");
        auto.set_margin_top(10);
        let ai = estrela(11, "#A5A1E2");
        ai.set_valign(Align::Start);
        auto.append(&ai);
        let at = left(tr("Automatic by the input: an image goes to LFM3-VL, audio to LFM3-Audio; text follows this order."), "central-agente-sub");
        at.set_wrap(true);
        at.set_max_width_chars(1);
        at.set_hexpand(true);
        auto.append(&at);
        lado.append(&auto);
        let opcao = |t: &str, s: &str, nome: &str| {
            let r = caixa(Orientation::Horizontal, 12, "");
            r.set_margin_top(14);
            let c = caixa(Orientation::Vertical, 2, "");
            c.set_hexpand(true);
            c.append(&left(t, "fx-opcao-titulo"));
            let sl = left(s, "central-agente-sub");
            sl.set_wrap(true);
            sl.set_max_width_chars(1);
            c.append(&sl);
            r.append(&c);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_valign(Align::Center);
            crate::central::posicao_debug_sempre(&format!("ajustes {nome}"), sw.upcast_ref());
            r.append(&sw);
            lado.append(&r);
            (sw, sl)
        };
        let (dados, _) = opcao(tr("Personal data never leaves the computer"), tr("files, e-mails and memory only go to the local model"), "dados locais");
        let (tentar, tentar_sub) = opcao(tr("Try again on error"), "", "tentar de novo");
        let uc = caixa(Orientation::Horizontal, 8, "");
        uc.set_margin_top(20);
        uc.append(&ui_icon("cpu", "#A5A1E2", 14));
        let uso_titulo = rotulo(tr("USAGE AND COST PER AGENT"), "central-painel-agente");
        uso_titulo.set_wrap(true);
        uso_titulo.set_xalign(0.0);
        uso_titulo.set_hexpand(true);
        uso_titulo.set_max_width_chars(1);
        uc.append(&uso_titulo);
        lado.append(&uc);
        let uso = caixa(Orientation::Vertical, 0, "fx-uso");
        uso.set_margin_top(14);
        lado.append(&uso);
        let vao = caixa(Orientation::Vertical, 0, "");
        vao.set_vexpand(true);
        lado.append(&vao);
        let rod = caixa(Orientation::Horizontal, 8, "");
        rod.set_margin_top(18);
        let ri = ui_icon("info", "#8E8F99", 14);
        ri.set_valign(Align::Start);
        rod.append(&ri);
        let rt = left(tr("The models are called from inside the flows service (ADK-Rust's model layer, no separate gateway): LFM3 through the local socket; the cloud only with a token kept in the KalangoOS vault."), "central-agente-sub");
        rt.set_wrap(true);
        rt.set_max_width_chars(1);
        rt.set_hexpand(true);
        rod.append(&rt);
        lado.append(&rod);

        let body = caixa(Orientation::Vertical, 0, "");
        body.set_vexpand(true);
        let tab_weak: Rc<RefCell<Option<std::rc::Weak<ModelsTab>>>> = Rc::default();
        let tw = tab_weak.clone();
        let screen = estados::Screen::new(&rolagem, tr("Reading the models and providers…"), 4, move || {
            if let Some(t) = tw.borrow().as_ref().and_then(|w| w.upgrade()) {
                t.reload();
            }
        });
        body.append(&screen.stack);

        let t = Rc::new(ModelsTab {
            body, cols, screen, numeros, mem_title, locais, decisao, nuvem, rota, dados, tentar, tentar_sub, uso, uso_titulo, form,
            f_nome, f_url, f_modelo, f_token, f_token_label, f_parallel, f_per_minute, f_price_in, f_price_out, f_title, editing: RefCell::default(), f_erro, f_salvar: f_salvar.clone(),
            data: RefCell::default(), filling: Cell::new(false), loaded: Cell::new(false), remove_armed: RefCell::default(),
        });
        *tab_weak.borrow_mut() = Some(Rc::downgrade(&t));
        let w = Rc::downgrade(&t);
        adicionar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                let open = !t.form.reveals_child() || t.editing.borrow().is_some();
                t.set_editing(None);
                t.form.set_reveal_child(open);
                if open {
                    t.f_erro.set_visible(false);
                    t.f_nome.grab_focus();
                }
            }
        });
        let w = Rc::downgrade(&t);
        f_cancelar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.clear_form();
            }
        });
        let w = Rc::downgrade(&t);
        f_salvar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.add_provider();
            }
        });
        let w = Rc::downgrade(&t);
        t.dados.connect_active_notify(move |s| {
            if let Some(t) = w.upgrade().filter(|t| !t.filling.get()) {
                let mut r = t.data.borrow().routing.clone();
                r["personal_data_stays_local"] = json!(s.is_active());
                t.save_routing(r);
            }
        });
        let w = Rc::downgrade(&t);
        t.tentar.connect_active_notify(move |s| {
            if let Some(t) = w.upgrade().filter(|t| !t.filling.get()) {
                let mut r = t.data.borrow().routing.clone();
                r["retries"] = json!(if s.is_active() { 2 } else { 0 });
                t.save_routing(r);
            }
        });
        t
    }

    fn clear_form(&self) {
        self.set_editing(None);
        self.form.set_reveal_child(false);
    }

    /// The form for a new provider (None) or filled with one to change (its token stays unless a new one is typed).
    fn set_editing(&self, p: Option<&Value>) {
        for e in [&self.f_nome, &self.f_url, &self.f_modelo, &self.f_token, &self.f_parallel, &self.f_per_minute, &self.f_price_in, &self.f_price_out] {
            e.set_text("");
        }
        self.f_erro.set_visible(false);
        self.f_salvar.set_sensitive(true);
        let id = p.and_then(|p| p["id"].as_str()).map(str::to_string);
        let editing = id.is_some();
        self.f_title.set_text(if editing { tr("CHANGE CLOUD PROVIDER") } else { tr("NEW CLOUD PROVIDER") });
        self.f_token_label.set_text(if editing { tr("New token (empty: keeps the one in the vault)") } else { tr("Token (kept in the vault; leave empty for a server without a key)") });
        estados::set_button_label(&self.f_salvar, if editing { tr("Save changes") } else { tr("Add provider") });
        if let (Some(p), Some(id)) = (p, id.as_deref()) {
            self.f_nome.set_text(p["name"].as_str().unwrap_or(""));
            self.f_url.set_text(p["base_url"].as_str().unwrap_or(""));
            let modelo = self.data.borrow().routing["order"].as_array().into_iter().flatten().find(|m| m["provider"] == id).and_then(|m| m["model"].as_str().map(str::to_string)).unwrap_or_default();
            self.f_modelo.set_text(&modelo);
            self.f_parallel.set_text(&p["max_parallel"].as_u64().unwrap_or(2).to_string());
            self.f_per_minute.set_text(&p["per_minute"].as_u64().unwrap_or(0).to_string());
            self.f_price_in.set_text(&price_field(p["price_in"].as_u64().unwrap_or(0)));
            self.f_price_out.set_text(&price_field(p["price_out"].as_u64().unwrap_or(0)));
        }
        *self.editing.borrow_mut() = id;
    }

    fn fail_form(&self, why: &str) {
        self.f_erro.set_text(why);
        self.f_erro.set_visible(true);
        self.f_salvar.set_sensitive(true);
    }

    /// "Adicionar provedor": the token into the vault, the provider into the list, its model at the end of the order.
    fn add_provider(self: &Rc<Self>) {
        let (nome, url, modelo, token) = (self.f_nome.text().trim().to_string(), self.f_url.text().trim().trim_end_matches('/').to_string(), self.f_modelo.text().trim().to_string(), self.f_token.text().to_string());
        if let Err(why) = check_provider(&nome, &url, &modelo) {
            self.fail_form(why);
            return;
        }
        let (par, pm, pi, po) = match parse_limits(&self.f_parallel.text(), &self.f_per_minute.text(), &self.f_price_in.text(), &self.f_price_out.text()) {
            Ok(l) => l,
            Err(why) => {
                self.fail_form(why);
                return;
            }
        };
        let editing = self.editing.borrow().clone();
        let mut providers = self.data.borrow().providers.clone();
        let mut routing = self.data.borrow().routing.clone();
        let (id, segredo, novo_token) = match &editing {
            Some(id) => {
                let antigo = providers.iter().find(|p| p["id"] == id.as_str()).and_then(|p| p["secret"].as_str()).unwrap_or("").to_string();
                let s = if token.trim().is_empty() { antigo } else if antigo.is_empty() { format!("{id}.token") } else { antigo };
                (id.clone(), s, !token.trim().is_empty())
            }
            None => {
                let taken: Vec<String> = providers.iter().filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
                let id = provider_id(&nome, &taken);
                let s = if token.trim().is_empty() { String::new() } else { format!("{id}.token") };
                (id, s, !token.trim().is_empty())
            }
        };
        let limits = json!({"max_parallel": par, "per_minute": pm, "price_in": pi, "price_out": po});
        match providers.iter_mut().find(|p| p["id"] == id.as_str()) {
            Some(p) => {
                p["name"] = json!(nome);
                p["base_url"] = json!(url);
                p["secret"] = json!(segredo);
                for (k, v) in limits.as_object().into_iter().flatten() {
                    p[k] = v.clone();
                }
            }
            None => {
                let mut p = json!({"id": id, "kind": "openai_compatible", "name": nome, "base_url": url, "secret": segredo, "enabled": true});
                for (k, v) in limits.as_object().into_iter().flatten() {
                    p[k] = v.clone();
                }
                providers.push(p);
            }
        }
        if let Some(o) = routing["order"].as_array_mut() {
            match o.iter_mut().find(|m| m["provider"] == id.as_str()) {
                Some(m) => m["model"] = json!(modelo),
                None => o.push(json!({"provider": id, "model": modelo})),
            }
        }
        self.f_salvar.set_sensitive(false);
        let w = Rc::downgrade(self);
        let depois = move |r: Result<Value, String>| {
            let Some(t) = w.upgrade() else { return };
            if let Err(e) = r {
                t.fail_form(&trf("Not added: {why}", &[("why", e)]));
                return;
            }
            let w = Rc::downgrade(&t);
            crate::fluxos::call("models.set_providers", json!({"providers": providers}), move |r| {
                let Some(t) = w.upgrade() else { return };
                if let Err(e) = r {
                    t.fail_form(&trf("Not added: {why}", &[("why", e)]));
                    return;
                }
                let w = Rc::downgrade(&t);
                crate::fluxos::call("models.set_routing", json!({"routing": routing}), move |r| {
                    let Some(t) = w.upgrade() else { return };
                    t.f_salvar.set_sensitive(true);
                    match r {
                        Ok(_) => {
                            t.clear_form();
                            t.reload();
                        }
                        Err(e) => t.fail_form(&trf("Not added: {why}", &[("why", e)])),
                    }
                });
            });
        };
        if !novo_token {
            depois(Ok(Value::Null));
        } else {
            crate::fluxos::call("secrets.store", json!({"name": segredo, "value": token}), depois);
        }
    }

    fn save_routing(self: &Rc<Self>, routing: Value) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("models.set_routing", json!({"routing": routing}), move |r| {
            if let Some(t) = w.upgrade() {
                if let Err(e) = r {
                    eprintln!("ajustes: roteamento não salvo: {e}");
                }
                t.reload();
            }
        });
    }

    fn save_providers(self: &Rc<Self>, providers: Vec<Value>, routing: Option<Value>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("models.set_providers", json!({"providers": providers}), move |r| {
            let Some(t) = w.upgrade() else { return };
            if let Err(e) = r {
                eprintln!("ajustes: provedores não salvos: {e}");
            }
            match routing {
                Some(r) => t.save_routing(r),
                None => t.reload(),
            }
        });
    }

    /// Reads everything again (when the tab shows and after each change).
    pub fn reload(self: &Rc<Self>) {
        if !self.loaded.get() {
            self.screen.show_loading();
            self.cols.content.set_visible(false);
        }
        let w = Rc::downgrade(self);
        estados::call_all(
            vec![("models.providers", json!({})), ("models.usage", json!({})), ("agents.list", json!({})), ("secrets.list", json!({})), ("models.local", json!({})), ("models.usage", json!({"since": month_start()})), ("models.currency", json!({}))],
            move |r| {
                let Some(t) = w.upgrade() else { return };
                if let Some(Err(e)) = r.iter().find(|x| x.is_err()) {
                    if estados::is_offline_error(e) {
                        t.loaded.set(false);
                        t.screen.show_offline();
                        t.cols.content.set_visible(false);
                    } else {
                        let (c, _) = estados::notice(estados::Tone::Error, "alerta", tr("The models could not be read"), "", Some(e));
                        t.screen.show_error(&c);
                        t.cols.content.set_visible(false);
                    }
                    return;
                }
                let v: Vec<Value> = r.into_iter().map(|x| x.unwrap_or_default()).collect();
                {
                    let mut d = t.data.borrow_mut();
                    d.providers = v[0]["providers"].as_array().cloned().unwrap_or_default();
                    d.routing = v[0]["routing"].clone();
                    d.usage = v[1].as_array().cloned().unwrap_or_default();
                    d.agents = v[2].as_array().into_iter().flatten().filter_map(|a| Some((a["agent"]["id"].as_str()?.to_string(), a["agent"]["name"].as_str()?.to_string()))).collect();
                    d.agent_items = v[2].as_array().into_iter().flatten().filter_map(|a| Some((a["agent"]["id"].as_str()?.to_string(), a["agent"].clone()))).collect();
                    d.usage_month = v[5].as_array().cloned().unwrap_or_default();
                    d.currency = Currency::from_value(&v[6]);
                    d.secrets = v[3].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
                    d.local = v[4].as_array().cloned().unwrap_or_default();
                }
                t.loaded.set(true);
                t.render();
                t.screen.show_content();
                t.cols.content.set_visible(true);
            },
        );
    }

    fn render(self: &Rc<Self>) {
        self.filling.set(true);
        let d = self.data.borrow();
        let local_of = |id: &str| d.local.iter().find(|m| m["tipo"] == id).cloned().unwrap_or_default();
        // the numbers
        let instalados: Vec<&Kind> = LOCAL.iter().filter(|k| local_of(k.id)["instalado"].as_bool().unwrap_or(false)).collect();
        let nomes: Vec<&str> = instalados.iter().map(|k| tr(k.short)).collect();
        self.numeros[0].0.set_text(&instalados.len().to_string());
        self.numeros[0].1.set_text(&if nomes.is_empty() { tr("local models installed").to_string() } else { trf("local models ({list})", &[("list", nomes.join(", "))]) });
        let nuvem: Vec<&Value> = d.providers.iter().filter(|p| p["kind"] != "local").collect();
        self.numeros[1].0.set_text(&nuvem.len().to_string());
        self.numeros[1].1.set_text(tr("cloud providers"));
        match local_share(&d.usage) {
            Some(p) => {
                self.numeros[2].0.set_text(&format!("{p}%"));
                self.numeros[2].1.set_text(tr("of the calls on this computer"));
            }
            None => {
                self.numeros[2].0.set_text("—");
                self.numeros[2].1.set_text(tr("no calls counted yet"));
            }
        }
        self.numeros[3].0.set_text(&d.currency.money(cloud_cost(&d.usage_month)));
        self.numeros[3].1.set_text(tr("on the cloud this month"));
        // the memory
        let (livre, total) = mem_info(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
        let maior = d.local.iter().filter(|m| m["tipo"] != "texto" && m["instalado"].as_bool().unwrap_or(false)).filter_map(|m| m["bytes"].as_u64()).max().unwrap_or(0);
        self.mem_title.set_text(&memory_headline(livre, total, maior));
        // the local models
        let local_on = d.providers.iter().find(|p| p["id"] == "local").map(|p| p["enabled"].as_bool().unwrap_or(true)).unwrap_or(true);
        for (caixa_k, kinds) in [(&self.locais, LOCAL), (&self.decisao, DECISION)] {
            clear(caixa_k);
            for k in kinds {
                caixa_k.append(&self.model_row(k, &local_of(k.id), local_on));
            }
        }
        // the cloud
        clear(&self.nuvem);
        if nuvem.is_empty() {
            let (e, acoes) = estados::empty("globo", tr("No cloud provider"), tr("Everything runs on this computer. Add an OpenAI-compatible provider (OpenRouter, an Ollama of the home network…) to use it when an agent may."), "");
            e.add_css_class("compacto");
            let b = action_button(tr("Add provider"), "mais", "");
            let f = self.form.clone();
            b.connect_clicked(move |_| f.set_reveal_child(true));
            acoes.append(&b);
            self.nuvem.append(&e);
        }
        for p in nuvem {
            self.nuvem.append(&self.provider_row(p, &d.secrets));
        }
        // the routing
        clear(&self.rota);
        let order: Vec<Value> = d.routing["order"].as_array().cloned().unwrap_or_default();
        for (i, m) in order.iter().enumerate() {
            self.rota.append(&self.route_row(i, m, &d.providers));
        }
        self.rota.set_visible(!order.is_empty());
        self.dados.set_active(d.routing["personal_data_stays_local"].as_bool().unwrap_or(true));
        let n = d.routing["retries"].as_u64().unwrap_or(0);
        self.tentar.set_active(n > 0);
        self.tentar_sub.set_text(&if n > 0 { trf("up to {n} more tries of the same model before the next one", &[("n", n.to_string())]) } else { tr("goes straight to the next model").to_string() });
        // usage
        clear(&self.uso);
        let (ano, mes) = glib::DateTime::now_local().map(|n| (n.year(), n.month())).unwrap_or((1970, 1));
        let _ = ano;
        self.uso_titulo.set_text(&format!("{} · {}", tr("USAGE AND COST PER AGENT"), crate::calendar_app::month_name(mes).to_uppercase()));
        let mut linhas = usage_by_agent(&d.usage_month, &d.agents);
        // the agents with a budget and nothing spent yet this month also show their limit
        for (id, a) in &d.agent_items {
            if agent_budget(a) > 0 && !linhas.iter().any(|u| &u.id == id) {
                linhas.push(AgentUsage { id: id.clone(), name: a["name"].as_str().unwrap_or(id).to_string(), tokens: 0, cost: 0, cloud: false });
            }
        }
        if linhas.is_empty() {
            let l = left(tr("Nothing counted this month yet: each call of an agent appears here with its tokens and, in the cloud, its cost."), "central-agente-sub");
            l.set_wrap(true);
            l.set_max_width_chars(1);
            l.set_margin_top(10);
            l.set_margin_bottom(10);
            self.uso.append(&l);
        }
        for u in linhas {
            let item = d.agent_items.get(&u.id).cloned();
            let orcamento = item.as_ref().map(agent_budget).unwrap_or(0);
            let assistente = item.as_ref().is_some_and(|a| a["kind"] == "assistant");
            self.uso.append(&self.usage_row(&u, orcamento, assistente, &d.currency));
        }
        self.uso.append(&self.rate_row(&d.currency));
        drop(d);
        self.filling.set(false);
    }

    /// One agent's line of usage: its cost (or "local"), its monthly limit with a bar, its tokens. An assistant's
    /// line opens a small editor of the limit (kept in its `config.monthly_budget_micro_usd`).
    /// "câmbio R$ 5,40/US$ · editar": the rate the costs are shown with (the service counts US$).
    fn rate_row(self: &Rc<Self>, cur: &Currency) -> gtk::Widget {
        let b = gtk::MenuButton::new();
        b.add_css_class("fx-uso-botao");
        b.set_halign(Align::Start);
        b.set_margin_top(6);
        b.set_margin_bottom(4);
        let t = left(&trf("exchange {rate} · edit", &[("rate", cur.rate_text())]), "central-agente-sub");
        b.set_child(Some(&t));
        b.set_tooltip_text(Some(tr("The service counts the costs in US$; this rate shows them in your currency")));
        crate::central::posicao_debug_sempre("ajustes cambio", b.upcast_ref());
        let pop = gtk::Popover::new();
        pop.add_css_class("fx-pop");
        let pc = caixa(Orientation::Vertical, 8, "");
        pc.set_margin_top(8);
        pc.set_margin_bottom(8);
        pc.set_margin_start(8);
        pc.set_margin_end(8);
        pc.append(&left(&trf("{symbol} per US$", &[("symbol", cur.symbol())]), "fx-campo-rotulo"));
        let e = gtk::Entry::new();
        e.add_css_class("fx-campo");
        e.set_text(&format!("{:.2}", cur.per_usd).replace('.', ","));
        crate::central::posicao_debug_sempre("ajustes cambio valor", e.upcast_ref());
        pc.append(&e);
        let erro = left("", "rotinas-erro");
        erro.set_visible(false);
        pc.append(&erro);
        let salvar = action_button(tr("Save"), "", "principal");
        salvar.set_halign(Align::End);
        crate::central::posicao_debug_sempre("ajustes cambio salvar", salvar.upcast_ref());
        pc.append(&salvar);
        pop.set_child(Some(&pc));
        b.set_popover(Some(&pop));
        let (w, code, pop2, e2) = (Rc::downgrade(self), cur.code.clone(), pop.clone(), e.clone());
        let enviar = Rc::new(move || {
            let Some(t) = w.upgrade() else { return };
            let Some(v) = parse_price(&e2.text()).filter(|v| *v > 0) else {
                erro.set_text(tr("The rate is a positive number, like 5,40."));
                erro.set_visible(true);
                return;
            };
            pop2.popdown();
            let w = Rc::downgrade(&t);
            crate::fluxos::call("models.set_currency", json!({"code": code, "per_usd": v as f64 / 1e6}), move |r| {
                if let Some(t) = w.upgrade() {
                    if let Err(e) = r {
                        eprintln!("ajustes: câmbio não salvo: {e}");
                    }
                    t.reload();
                }
            });
        });
        let en = enviar.clone();
        salvar.connect_clicked(move |_| en());
        e.connect_activate(move |_| enviar());
        b.upcast()
    }

    fn usage_row(self: &Rc<Self>, u: &AgentUsage, budget: u64, editable: bool, cur: &Currency) -> gtk::Widget {
        let linha = caixa(Orientation::Vertical, 6, "fx-uso-linha");
        let r = caixa(Orientation::Horizontal, 8, "");
        let n = left(&u.name, "fx-uso-nome");
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        r.append(&n);
        if u.cost > 0 || budget > 0 {
            r.append(&rotulo(&cur.money(u.cost), "fx-uso-custo"));
        } else if !u.cloud {
            r.append(&rotulo(tr("local"), "fx-uso-local"));
        }
        if budget > 0 {
            r.append(&rotulo(&trf("limit {value}/month", &[("value", cur.money(budget as i64))]), "central-agente-sub"));
        } else {
            r.append(&rotulo(&tokens_text(u.tokens), "central-agente-sub"));
        }
        linha.append(&r);
        if budget > 0 {
            let trilho = caixa(Orientation::Horizontal, 0, "fx-barra");
            let cheio = caixa(Orientation::Horizontal, 0, "fx-barra-cheia");
            let f = budget_fraction(u.cost, budget);
            cheio.set_visible(f > 0.0);
            trilho.append(&cheio);
            let resto = caixa(Orientation::Horizontal, 0, "");
            resto.set_hexpand(true);
            trilho.append(&resto);
            // the filled part keeps its share of the rail's width
            let (c2, t2) = (cheio.clone(), trilho.clone());
            trilho.add_tick_callback(move |_, _| {
                let w = (t2.width() as f64 * f).round() as i32;
                if c2.width_request() != w {
                    c2.set_size_request(w, -1);
                }
                glib::ControlFlow::Continue
            });
            linha.append(&trilho);
        }
        if !editable {
            return linha.upcast();
        }
        // the limit: a click on the line
        let b = gtk::MenuButton::new();
        b.add_css_class("fx-uso-botao");
        b.set_child(Some(&linha));
        b.set_tooltip_text(Some(tr("Monthly limit on the cloud")));
        crate::central::posicao_debug_sempre(&format!("ajustes limite {}", u.id), b.upcast_ref());
        let pop = gtk::Popover::new();
        pop.add_css_class("fx-pop");
        let pc = caixa(Orientation::Vertical, 8, "");
        pc.set_margin_top(8);
        pc.set_margin_bottom(8);
        pc.set_margin_start(8);
        pc.set_margin_end(8);
        pc.append(&left(&trf("Limit on the cloud per month ({symbol})", &[("symbol", cur.symbol())]), "fx-campo-rotulo"));
        let e = gtk::Entry::new();
        e.add_css_class("fx-campo");
        e.set_placeholder_text(Some(tr("no limit")));
        e.set_text(&cur.field(budget));
        crate::central::posicao_debug_sempre(&format!("ajustes limite {} valor", u.id), e.upcast_ref());
        pc.append(&e);
        let nota = left(tr("When the month's cost reaches it, the agent runs on the local models until the month turns."), "central-agente-sub");
        nota.set_wrap(true);
        nota.set_max_width_chars(34);
        pc.append(&nota);
        let erro = left("", "rotinas-erro");
        erro.set_visible(false);
        pc.append(&erro);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_halign(Align::End);
        let salvar = action_button(tr("Save"), "", "principal");
        crate::central::posicao_debug_sempre(&format!("ajustes limite {} salvar", u.id), salvar.upcast_ref());
        bs.append(&salvar);
        pc.append(&bs);
        pop.set_child(Some(&pc));
        b.set_popover(Some(&pop));
        let (w, id, pop2, e2, cur) = (Rc::downgrade(self), u.id.clone(), pop.clone(), e.clone(), cur.clone());
        let enviar = Rc::new(move || {
            let Some(t) = w.upgrade() else { return };
            let Some(v) = cur.to_micro_usd(&e2.text()) else {
                erro.set_text(tr("The limit is an amount, like 10 or 2,50."));
                erro.set_visible(true);
                return;
            };
            let Some(mut a) = t.data.borrow().agent_items.get(&id).cloned() else { return };
            a["config"]["monthly_budget_micro_usd"] = if v == 0 { Value::Null } else { json!(v) };
            pop2.popdown();
            let w = Rc::downgrade(&t);
            crate::fluxos::call("agents.save", json!({"agent": a}), move |r| {
                if let Some(t) = w.upgrade() {
                    if let Err(e) = r {
                        eprintln!("ajustes: limite não salvo: {e}");
                    }
                    t.reload();
                }
            });
        });
        let en = enviar.clone();
        salvar.connect_clicked(move |_| en());
        e.connect_activate(move |_| enviar());
        b.upcast()
    }

    fn model_row(self: &Rc<Self>, k: &Kind, m: &Value, local_on: bool) -> gtk::Box {
        let instalado = m["instalado"].as_bool().unwrap_or(false);
        let rodando = m["rodando"].as_bool().unwrap_or(false);
        let r = caixa(Orientation::Horizontal, 12, "fx-modelo");
        if k.id == "texto" && instalado && local_on {
            r.add_css_class("padrao");
        }
        r.append(&tile(k.glyph, k.tone, 36));
        let c = caixa(Orientation::Vertical, 5, "");
        c.set_hexpand(true);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        l1.append(&rotulo(k.name, "fx-modelo-nome"));
        l1.append(&chip(tr(k.chip), ""));
        c.append(&l1);
        let l2 = caixa(Orientation::Horizontal, 6, "");
        for cap in k.caps {
            l2.append(&chip(tr(cap), if instalado { "teal" } else { "" }));
        }
        c.append(&l2);
        let l3 = caixa(Orientation::Horizontal, 0, "");
        let (estado, classe) = match (instalado, rodando) {
            (false, _) => (tr("not installed"), "atencao"),
            (true, true) => (tr("loaded"), "ok"),
            (true, false) if k.id == "texto" && !local_on => (tr("turned off"), ""),
            (true, false) if k.id != "texto" && m["enabled"] == false => (tr("turned off"), ""),
            (true, false) => (tr("loads when asked"), ""),
        };
        let e = rotulo(estado, "ajustes-estado");
        if !classe.is_empty() {
            e.add_css_class(classe);
        }
        l3.append(&e);
        let mut resto: Vec<String> = Vec::new();
        if let Some(b) = m["bytes"].as_u64().filter(|b| *b > 0) {
            resto.push(crate::app_store_data::gb(b as f64));
        }
        let det = rotulo(&resto.iter().map(|x| format!(" · {x}")).collect::<String>(), "ajustes-estado");
        if !classe.is_empty() {
            det.add_css_class(classe);
        }
        l3.append(&det);
        let mut extra: Vec<&str> = vec![tr(k.extra)];
        if k.lfm {
            extra.push(tr("weights under the LFM Open License v1.0"));
        }
        let x = left(&format!("   ·   {}", extra.join("   ·   ")), "central-agente-sub");
        x.set_ellipsize(gtk::pango::EllipsizeMode::End);
        x.set_max_width_chars(1);
        x.set_hexpand(true);
        x.set_tooltip_text(Some(&extra.join("\n")));
        l3.append(&x);
        c.append(&l3);
        r.append(&c);
        if !instalado {
            // informational only: this page downloads nothing
            let b = caixa(Orientation::Horizontal, 6, "fx-baixar");
            b.append(&ui_icon("baixar", "#8E8F99", 14));
            b.append(&rotulo(tr("Download"), ""));
            b.set_valign(Align::Start);
            b.set_tooltip_text(Some(tr("Not installed. This version does not download models from here.")));
            r.append(&b);
        }
        if k.id == "texto" {
            // the text model is the "local" provider: off, agents go to the next place of the order
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_valign(Align::Start);
            sw.set_active(local_on);
            sw.set_sensitive(instalado);
            sw.set_tooltip_text(Some(tr("Off: agents use the next model of the routing order")));
            crate::central::posicao_debug_sempre("ajustes modelo local", sw.upcast_ref());
            let w = Rc::downgrade(self);
            sw.connect_active_notify(move |s| {
                let Some(t) = w.upgrade().filter(|t| !t.filling.get()) else { return };
                let mut ps = t.data.borrow().providers.clone();
                match ps.iter_mut().find(|p| p["id"] == "local") {
                    Some(p) => p["enabled"] = json!(s.is_active()),
                    None => ps.insert(0, json!({"id": "local", "kind": "local", "name": "LFM3 1.7B", "enabled": s.is_active(), "max_parallel": 1})),
                }
                t.save_providers(ps, None);
            });
            r.append(&sw);
        } else {
            // the service cannot turn one local model off on its own yet (kalango-fluxos has no such setting): the
            // switch says whether it can be used, and does not pretend to change it
            let ligado = instalado && m["enabled"].as_bool().unwrap_or(true);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_valign(Align::Start);
            sw.set_active(ligado);
            sw.set_sensitive(instalado);
            sw.set_tooltip_text(Some(if !instalado { tr("Not installed") } else { tr("Off: agents and blocks do not use this model") }));
            crate::central::posicao_debug_sempre(&format!("ajustes modelo {}", k.id), sw.upcast_ref());
            let (w, tipo) = (Rc::downgrade(self), k.id);
            sw.connect_active_notify(move |s| {
                let Some(t) = w.upgrade().filter(|t| !t.filling.get()) else { return };
                let w = Rc::downgrade(&t);
                crate::fluxos::call("models.set_local", json!({"tipo": tipo, "enabled": s.is_active()}), move |r| {
                    if let Some(t) = w.upgrade() {
                        if let Err(e) = r {
                            eprintln!("ajustes: modelo local não mudou: {e}");
                        }
                        t.reload();
                    }
                });
            });
            r.append(&sw);
        }
        r
    }

    fn provider_row(self: &Rc<Self>, p: &Value, secrets: &[String]) -> gtk::Box {
        let id = p["id"].as_str().unwrap_or("").to_string();
        let r = caixa(Orientation::Horizontal, 12, "fx-modelo");
        r.append(&tile("globo", "lilas", 36));
        let c = caixa(Orientation::Vertical, 5, "");
        c.set_hexpand(true);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        let nome = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id);
        l1.append(&rotulo(nome, "fx-modelo-nome"));
        l1.append(&chip(tr("OpenAI-compatible"), ""));
        c.append(&l1);
        let modelos: Vec<String> = self.data.borrow().routing["order"].as_array().into_iter().flatten().filter(|m| m["provider"] == id.as_str()).filter_map(|m| m["model"].as_str().map(str::to_string)).collect();
        if !modelos.is_empty() {
            let l2 = caixa(Orientation::Horizontal, 6, "");
            for m in &modelos {
                let ch = chip(m, "teal");
                ch.set_ellipsize(gtk::pango::EllipsizeMode::End);
                ch.set_max_width_chars(36);
                l2.append(&ch);
            }
            c.append(&l2);
        }
        let l3 = caixa(Orientation::Horizontal, 0, "");
        let ligado = p["enabled"].as_bool().unwrap_or(true);
        let segredo = p["secret"].as_str().unwrap_or("");
        let (estado, classe) = if !ligado {
            (tr("turned off").to_string(), "")
        } else if segredo.is_empty() {
            (tr("no key").to_string(), "")
        } else if secrets.iter().any(|s| s == segredo) {
            (trf("key {name}", &[("name", segredo.to_string())]), "ok")
        } else {
            (trf("key {name} is not in the vault", &[("name", segredo.to_string())]), "atencao")
        };
        let e = rotulo(&estado, "ajustes-estado");
        if !classe.is_empty() {
            e.add_css_class(classe);
        }
        l3.append(&e);
        let u = left(&format!("   ·   {}", p["base_url"].as_str().unwrap_or("")), "central-agente-sub");
        u.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        u.set_max_width_chars(1);
        u.set_hexpand(true);
        l3.append(&u);
        c.append(&l3);
        // its limits (what "Adicionar provedor" / "Alterar" set)
        let lim = left(&limits_text(p), "central-agente-sub");
        lim.set_ellipsize(gtk::pango::EllipsizeMode::End);
        lim.set_max_width_chars(1);
        lim.set_tooltip_text(Some(&limits_text(p)));
        c.append(&lim);
        r.append(&c);
        let editar = action_button(tr("Change"), "", "");
        editar.add_css_class("pequeno");
        editar.set_valign(Align::Start);
        crate::central::posicao_debug_sempre(&format!("ajustes alterar provedor {id}"), editar.upcast_ref());
        let (w, pv) = (Rc::downgrade(self), p.clone());
        editar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.set_editing(Some(&pv));
                t.form.set_reveal_child(true);
                t.f_nome.grab_focus();
            }
        });
        r.append(&editar);
        let armado = self.remove_armed.borrow().as_deref() == Some(id.as_str());
        let remover = action_button(if armado { tr("Click again to remove") } else { tr("Remove") }, "lixeira", "perigo");
        remover.add_css_class("pequeno");
        remover.set_valign(Align::Start);
        crate::central::posicao_debug_sempre(&format!("ajustes remover provedor {id}"), remover.upcast_ref());
        let (w, idc) = (Rc::downgrade(self), id.clone());
        remover.connect_clicked(move |_| {
            let Some(t) = w.upgrade() else { return };
            if t.remove_armed.borrow().as_deref() != Some(idc.as_str()) {
                *t.remove_armed.borrow_mut() = Some(idc.clone());
                t.render();
                let (w, idc) = (Rc::downgrade(&t), idc.clone());
                glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                    if let Some(t) = w.upgrade().filter(|t| t.remove_armed.borrow().as_deref() == Some(idc.as_str())) {
                        *t.remove_armed.borrow_mut() = None;
                        t.render();
                    }
                });
                return;
            }
            *t.remove_armed.borrow_mut() = None;
            // the provider and its places in the order go; its key stays in the vault (Ajustes › Chaves removes it)
            let ps: Vec<Value> = t.data.borrow().providers.iter().filter(|p| p["id"] != idc.as_str()).cloned().collect();
            let mut routing = t.data.borrow().routing.clone();
            if let Some(o) = routing["order"].as_array_mut() {
                o.retain(|m| m["provider"] != idc.as_str());
            }
            t.save_providers(ps, Some(routing));
        });
        r.append(&remover);
        let sw = gtk::Switch::new();
        sw.add_css_class("central-switch");
        sw.set_valign(Align::Start);
        sw.set_active(ligado);
        crate::central::posicao_debug_sempre(&format!("ajustes provedor ligado {id}"), sw.upcast_ref());
        let w = Rc::downgrade(self);
        sw.connect_active_notify(move |s| {
            let Some(t) = w.upgrade().filter(|t| !t.filling.get()) else { return };
            let mut ps = t.data.borrow().providers.clone();
            if let Some(p) = ps.iter_mut().find(|p| p["id"] == id.as_str()) {
                p["enabled"] = json!(s.is_active());
            }
            t.save_providers(ps, None);
        });
        r.append(&sw);
        r
    }

    fn move_route(self: &Rc<Self>, from: usize, to: usize) {
        let mut routing = self.data.borrow().routing.clone();
        if let Some(o) = routing["order"].as_array_mut() {
            move_item(o, from, to);
        }
        self.data.borrow_mut().routing = routing.clone();
        self.render();
        self.save_routing(routing);
    }

    fn route_row(self: &Rc<Self>, i: usize, m: &Value, providers: &[Value]) -> gtk::Box {
        let pid = m["provider"].as_str().unwrap_or("");
        let p = providers.iter().find(|p| p["id"] == pid);
        let local = pid == "local";
        let nome = match (local, p) {
            (true, _) => "LFM3 1.7B".to_string(),
            (false, Some(p)) => p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(pid).to_string(),
            (false, None) => pid.to_string(),
        };
        let modelo = if local { tr("local").to_string() } else { m["model"].as_str().unwrap_or("").to_string() };
        let ligado = p.map(|p| p["enabled"].as_bool().unwrap_or(true)).unwrap_or(local);
        let r = caixa(Orientation::Horizontal, 10, "fx-rota-linha");
        let n = rotulo(&(i + 1).to_string(), "fx-rota-numero");
        n.set_valign(Align::Center);
        r.append(&n);
        let c = caixa(Orientation::Vertical, 1, "");
        c.set_hexpand(true);
        let t = left(&format!("{nome} · {modelo}"), "fx-rota-nome");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(1);
        c.append(&t);
        let hint = if p.is_none() && !local { tr("provider removed: skipped") } else { routing_hint(i, local, ligado) };
        c.append(&left(hint, "central-agente-sub"));
        r.append(&c);
        // the handle: drag it, or click it for "up / down"
        let alca = gtk::MenuButton::new();
        alca.add_css_class("fx-alca");
        alca.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        alca.set_valign(Align::Center);
        alca.set_tooltip_text(Some(tr("Drag to change the order")));
        crate::central::posicao_debug_sempre(&format!("ajustes rota {i}"), alca.upcast_ref());
        let pop = gtk::Popover::new();
        pop.add_css_class("fx-pop");
        let pc = caixa(Orientation::Vertical, 2, "");
        let total = self.data.borrow().routing["order"].as_array().map(Vec::len).unwrap_or(0);
        for (rot, glifo, para, nome_d) in [(tr("Move up"), "seta-cima", i.checked_sub(1), "acima"), (tr("Move down"), "seta-baixo", (i + 1 < total).then_some(i + 1), "abaixo")] {
            let b = action_button(rot, glifo, "texto");
            b.set_sensitive(para.is_some());
            crate::central::posicao_debug_sempre(&format!("ajustes rota {i} {nome_d}"), b.upcast_ref());
            let (w, pop2) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop2.popdown();
                if let (Some(t), Some(to)) = (w.upgrade(), para) {
                    let t2 = t.clone();
                    glib::idle_add_local_once(move || t2.move_route(i, to));
                    let _ = t;
                }
            });
            pc.append(&b);
        }
        pop.set_child(Some(&pc));
        alca.set_popover(Some(&pop));
        r.append(&alca);
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::MOVE);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(i as u32).to_value())));
        r.add_controller(drag);
        let drop = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, v, _, _| {
            let (Some(t), Ok(from)) = (w.upgrade(), v.get::<u32>()) else { return false };
            let t2 = t.clone();
            glib::idle_add_local_once(move || t2.move_route(from as usize, i));
            true
        });
        r.add_controller(drop);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyphs drawn in Rust (these tabs and fluxos_estados.rs) use token colours, which the light theme swaps.
    #[test]
    fn glyph_colours_follow_the_light_theme() {
        let m = crate::tema::mapa(&crate::tema::Tema::default(), true);
        let src = [include_str!("ajustes_modelos.rs"), include_str!("ajustes_chaves.rs"), include_str!("fluxos_estados.rs")].concat();
        let mut unknown = Vec::new();
        for (i, _) in src.match_indices("\"#") {
            let c = &src[i + 1..(i + 8).min(src.len())];
            if c.len() == 7 && c[1..].chars().all(|x| x.is_ascii_hexdigit()) && !m.contains_key(&c.to_uppercase()) {
                unknown.push(c.to_string());
            }
        }
        assert!(unknown.is_empty(), "colours the light theme does not map: {unknown:?}");
    }

    #[test]
    fn the_column_goes_under_when_it_does_not_fit() {
        // wide screen, room: beside
        assert!(!narrow_next(false, 900, 474, 1520, 1920));
        // the list and the column under 900 px
        assert!(narrow_next(false, 420, 474, 960, 1920));
        // a window GTK could not shrink to the screen
        assert!(narrow_next(false, 620, 474, 1195, 1100));
        // under it, it stays while the column beside would not fit the screen
        assert!(narrow_next(true, 1000, 474, 1050, 1100));
        // and comes back when it does
        assert!(!narrow_next(true, 1100, 474, 1180, 1920));
        // unknown screen: by the widths only
        assert!(!narrow_next(true, 1100, 474, 1180, 0));
        // a wide window that was narrow before: beside again (the window does not grow for it)
        assert!(!narrow_next(true, 1406, 474, 1520, 1920));
    }

    #[test]
    fn memory_is_read_and_judged_like_the_engine() {
        let m = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    7800000 kB\n";
        let (a, t) = mem_info(m);
        assert_eq!((a, t), (7_800_000 * 1024, 16_000_000 * 1024));
        // 1.6 GB of files: twice that and 1 GiB more fit in 7.9 GB, not in 3 GB
        assert!(fits(a, 1_600_000_000));
        assert!(!fits(3_000_000_000, 1_600_000_000));
        assert!(memory_headline(3_000_000_000, t, 1_600_000_000).ends_with(tr("one large model at a time")));
        assert!(memory_headline(a, t, 1_600_000_000).ends_with(tr("room for more than one model")));
        assert_eq!(mem_info(""), (0, 0));
    }

    #[test]
    fn routing_hints_follow_the_place() {
        assert_eq!(routing_hint(0, true, true), tr("always first"));
        assert_eq!(routing_hint(1, false, true), tr("if the agent allows the cloud"));
        assert_eq!(routing_hint(2, false, true), tr("only if the ones above fail"));
        assert_eq!(routing_hint(2, false, false), tr("turned off: skipped"));
    }

    #[test]
    fn usage_is_summed_per_agent_costliest_first() {
        let rows = vec![
            json!({"agent_id": "a1", "provider": "local", "tokens": 1000, "cost_micro_usd": 0, "calls": 8}),
            json!({"agent_id": "a1", "provider": "openrouter", "tokens": 500, "cost_micro_usd": 3_100_000, "calls": 1}),
            json!({"agent_id": "a2", "provider": "local", "tokens": 182_000, "cost_micro_usd": 0, "calls": 10}),
            json!({"agent_id": null, "provider": "local", "tokens": 10, "cost_micro_usd": 0, "calls": 1}),
        ];
        let names: BTreeMap<String, String> = [("a1".to_string(), "Planejador".to_string()), ("a2".to_string(), "Organizador".to_string())].into();
        let u = usage_by_agent(&rows, &names);
        assert_eq!(u[0], AgentUsage { id: "a1".into(), name: "Planejador".into(), tokens: 1500, cost: 3_100_000, cloud: true });
        assert_eq!(u[1].name, "Organizador");
        assert!(!u[1].cloud);
        assert_eq!(u.len(), 3);
        assert_eq!(local_share(&rows), Some(95));
        assert_eq!(local_share(&[]), None);
    }

    #[test]
    fn money_and_tokens_read_in_portuguese_numbers() {
        assert_eq!(Currency::default().money(3_100_000), "US$ 3,10");
        assert_eq!(Currency::default().money(0), "US$ 0,00");
        assert!(tokens_text(182_400).starts_with("182"));
        assert!(tokens_text(1_200_000).starts_with("1,2"));
        assert!(tokens_text(12).starts_with("12"));
    }

    #[test]
    fn provider_ids_are_new_and_never_local() {
        let taken = vec!["openrouter".to_string(), "local".to_string()];
        assert_eq!(provider_id("Open Router", &taken), "open-router");
        assert_eq!(provider_id("OpenRouter", &taken), "openrouter-2");
        assert_eq!(provider_id("Local", &taken), "provedor");
        assert_eq!(provider_id("  !!  ", &[]), "provedor");
    }

    #[test]
    fn the_order_moves_and_a_provider_is_checked() {
        let mut v = vec![1, 2, 3];
        move_item(&mut v, 2, 0);
        assert_eq!(v, vec![3, 1, 2]);
        move_item(&mut v, 0, 9);
        assert_eq!(v, vec![3, 1, 2]);
        assert!(check_provider("OpenRouter", "https://openrouter.ai/api/v1", "llama").is_ok());
        assert!(check_provider("", "https://openrouter.ai/api/v1", "llama").is_err());
        assert!(check_provider("X", "openrouter.ai", "llama").is_err());
        assert!(check_provider("X", "https://openrouter.ai/api/v1", " ").is_err());
    }

    #[test]
    fn limits_and_prices_read_like_the_service_keeps_them() {
        assert_eq!(parse_price(""), Some(0));
        assert_eq!(parse_price("0,50"), Some(500_000));
        assert_eq!(parse_price("US$ 1.5"), Some(1_500_000));
        assert_eq!(parse_price("-1"), None);
        assert_eq!(parse_price("abc"), None);
        assert_eq!(price_field(500_000), "0,5");
        assert_eq!(price_field(0), "");
        assert_eq!(parse_limits("", "", "", ""), Ok((2, 0, 0, 0)));
        assert_eq!(parse_limits("3", "20", "0,15", "0,60"), Ok((3, 20, 150_000, 600_000)));
        assert!(parse_limits("0", "", "", "").is_err());
        assert!(parse_limits("2", "x", "", "").is_err());
        let p = json!({"max_parallel": 2, "per_minute": 20, "price_in": 500_000, "price_out": 1_500_000});
        assert!(limits_text(&p).contains("20"));
        assert!(limits_text(&p).contains("0,50"));
        assert!(limits_text(&json!({})).contains('2'));
    }

    #[test]
    fn the_month_counts_only_the_cloud_against_a_budget() {
        let rows = vec![
            json!({"agent_id": "a1", "provider": "local", "cost_micro_usd": 0}),
            json!({"agent_id": "a1", "provider": "openrouter", "cost_micro_usd": 3_100_000}),
            json!({"agent_id": "a2", "provider": "anthropic", "cost_micro_usd": 1_020_000}),
        ];
        assert_eq!(cloud_cost(&rows), 4_120_000);
        assert_eq!(agent_budget(&json!({"config": {"monthly_budget_micro_usd": 10_000_000}})), 10_000_000);
        assert_eq!(agent_budget(&json!({"config": {}})), 0);
        assert!((budget_fraction(3_100_000, 10_000_000) - 0.31).abs() < 1e-9);
        assert_eq!(budget_fraction(20, 10), 1.0);
        assert_eq!(budget_fraction(5, 0), 0.0);
        assert!(month_start() <= glib::DateTime::now_local().unwrap().to_unix());
    }

    #[test]
    fn costs_read_in_the_person_s_currency() {
        let brl = Currency::from_value(&json!({"code": "BRL", "per_usd": 5.4, "set_by_person": false}));
        assert_eq!(brl.money(1_000_000), "R$ 5,40");
        assert_eq!(brl.rate_text(), "R$ 5,40/US$");
        assert_eq!(brl.to_micro_usd("54"), Some(10_000_000));
        assert_eq!(brl.to_micro_usd("R$ 5,40"), Some(1_000_000));
        assert_eq!(brl.to_micro_usd("x"), None);
        assert_eq!(brl.field(10_000_000), "54");
        assert_eq!(Currency::from_value(&json!(null)), Currency::default());
        assert_eq!(Currency::default().money(3_100_000), "US$ 3,10");
    }
}
