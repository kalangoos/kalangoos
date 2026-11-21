//! The blocks of an agent conversation that more than one screen shows: the agent's header over an answer, the
//! person's message, the steps (the tools it used) and the line under the answer. The main chat (conversa.rs) and the agent editor's "Testar agora"
//! (agent_editor.rs) build their messages with these, so both look the same in either theme (classes in style.css:
//! agente-*, mensagem-pessoa*, passo*). The answer's text itself is markdown.rs.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use serde_json::Value;

/// The accent of the agent's spark (the token's dark value; the light theme swaps it by value).
const ACCENT: &str = "#A5A1E2";

/// The agent's header above an answer: spark tile, the agent's name, its model chip(s) and when.
pub(crate) fn agent_header(name: &str, models: &[String], when: &str) -> gtk::Box {
    let h = caixa(Orientation::Horizontal, 8, "agente-cabecalho");
    let tile = caixa(Orientation::Horizontal, 0, "agente-tile");
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let sp = estrela(12, ACCENT);
    sp.set_margin_start(5);
    sp.set_margin_end(5);
    tile.append(&sp);
    h.append(&tile);
    h.append(&rotulo(name, "agente-nome"));
    for m in models {
        h.append(&rotulo(m, "chip-modelo"));
    }
    h.append(&rotulo(when, "agente-hora"));
    h
}

/// The person's message: a bubble as wide as the column, its text selectable.
pub(crate) fn person_message(text: &str) -> gtk::Box {
    let msg = caixa(Orientation::Horizontal, 0, "mensagem-pessoa");
    msg.set_margin_start(20);
    let l = rotulo(text, "mensagem-pessoa-texto");
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_selectable(true);
    msg.append(&l);
    msg
}

/// A path with the home folder as "~".
pub(crate) fn short_path(p: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if h.len() > 1 => {
            let h = h.trim_end_matches('/');
            let s = p.replace(&format!("{h}/"), "~/");
            if s.ends_with(h) { format!("{}~", &s[..s.len() - h.len()]) } else { s }
        }
        _ => p.to_string(),
    }
}

/// The argument that says what the step works on (path, packages, program…), short.
pub(crate) fn main_argument(args: &Value) -> String {
    for k in ["caminho", "de", "programa", "comando", "nome", "servico", "chave", "dispositivo", "conexao", "id"] {
        if let Some(v) = args.get(k) {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            return short_path(&s);
        }
    }
    if let Some(a) = args.get("nomes").and_then(Value::as_array) {
        return a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    }
    String::new()
}

pub(crate) fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ------------------------------------------------------------------------------------- the agent's steps (53/54)
//
// Design "Chat · passos do agente (movimento)": the tools an agent used, as steps of a list in a card. Each step has
// a rail mark (done ✓ teal, running accent ring with a dot, waiting gold shield, failed ×), a tile with the tool's
// icon, a human label, a mono detail, a count chip, the time and a chevron that opens the arguments and results.
// Consecutive calls of one tool are one step ("Pesquisou na web · 6 buscas"). Motion: only opacity and transforms
// (Animavel): the running mark and the progress line's shine are drawn once and only fade and slide, so a frame
// redraws no drawing and lays nothing out; one tick per list, stopped when nothing runs.

use gtk::glib;
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{Animavel, Canal, RAPIDA, SUAVE};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    /// waiting for the person (approval), or — in a test — what would ask
    Waiting,
    Failed,
}

/// One call of a tool, as the list keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct StepCall {
    pub tool: String,
    /// what it works on, short (a path, the search's terms)
    pub arg: String,
    /// what came back, short ("8 resultados"), or why it failed
    pub result: String,
    pub state: StepState,
    /// the arguments as JSON (the technical details; a web result reads its address)
    pub args_json: String,
    /// how long it took, when known
    pub took: Option<Duration>,
    /// a member of a team or a task of a list that answered (a row of its own: "Tarefa 1 · Buscador")
    pub member: Option<Member>,
}

impl StepCall {
    pub fn new(tool: &str, arg: &str, state: StepState) -> StepCall {
        StepCall { tool: tool.into(), arg: arg.into(), result: String::new(), state, args_json: String::new(), took: None, member: None }
    }
}

/// Who answered in a step of a team or a task list: the mono mark before the name ("Tarefa 1", "Volta 2", or none
/// for a team's member), the agent's name and what it answered (the opened row shows it).
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub tag: String,
    pub name: String,
    pub text: String,
}

/// The tool name of a member's row (no tool: it never joins another row).
pub const MEMBER_TOOL: &str = "agente.membro";

/// The agent's name in the engine's events (its id with anything not ASCII alphanumeric as "_"; kalango-fluxos'
/// `adk_name`).
pub fn adk_name(id: &str) -> String {
    let base: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if base.starts_with(|c: char| c.is_ascii_digit()) { format!("a_{base}") } else { base }
}

/// The task number and the assistant of a task step's author ("<assistant>_tarefa_<n>").
pub fn task_of(author: &str) -> Option<(&str, u32)> {
    let (who, n) = author.rsplit_once("_tarefa_")?;
    let n: u32 = n.parse().ok()?;
    (!who.is_empty()).then_some((who, n))
}

/// What an answer is, short, for the row's right side: "tabela com 7 linhas", "3 parágrafos", or its first words.
pub fn answer_shape(text: &str) -> String {
    let t = text.trim();
    let rows = t.lines().filter(|l| l.trim_start().starts_with('|')).count();
    // (a markdown table: its header and the |---| line are not rows)
    if rows >= 3 {
        let n = rows - 2;
        return if n == 1 { tr("table with 1 row").to_string() } else { trf("table with {n} rows", &[("n", n.to_string())]) };
    }
    let paragraphs = t.split("\n\n").filter(|p| !p.trim().is_empty()).count();
    if paragraphs >= 2 {
        return trf("{n} paragraphs", &[("n", paragraphs.to_string())]);
    }
    let first = t.lines().next().unwrap_or("").trim().trim_start_matches(['#', '*', '-', ' ']);
    let words: String = first.chars().take(28).collect();
    if first.chars().count() > 28 { format!("{}…", words.trim_end()) } else { words }
}

/// The rows of the members and tasks that answered, in order: `answers` are (author, text, how long it took); the
/// root agent's own answers have no row (an assistant alone shows only its tools). `agents` are (id, name); a loop
/// numbers its turns.
pub fn member_calls(answers: &[(String, String, Option<Duration>)], root_id: &str, kind: &str, agents: &[(String, String)]) -> Vec<StepCall> {
    let root = adk_name(root_id);
    let name_of = |adk: &str| agents.iter().find(|(id, _)| adk_name(id) == adk).map(|(_, n)| n.clone()).unwrap_or_else(|| adk.to_string());
    let mut turns: Vec<(String, u32)> = Vec::new();
    let mut out = Vec::new();
    for (author, text, took) in answers {
        if author.is_empty() || *author == root || author == "user" {
            continue;
        }
        let (tag, name) = match task_of(author) {
            Some((who, n)) => (trf("Task {n}", &[("n", n.to_string())]), name_of(who)),
            None if kind == "loop" => {
                let n = match turns.iter_mut().find(|(a, _)| a == author) {
                    Some((_, n)) => {
                        *n += 1;
                        *n
                    }
                    None => {
                        turns.push((author.clone(), 1));
                        1
                    }
                };
                (trf("Turn {n}", &[("n", n.to_string())]), name_of(author))
            }
            None => (String::new(), name_of(author)),
        };
        let mut c = StepCall::new(MEMBER_TOOL, "", StepState::Done);
        c.result = answer_shape(text);
        c.took = *took;
        c.member = Some(Member { tag, name, text: text.clone() });
        out.push(c);
    }
    out
}

/// How long each answer took, from when the live events came (`arrivals`: author and time since the request was
/// sent, in order): a parallel team's members all started at once, the others one after the previous. Each answer
/// takes the next live event of its author. The last answer's event comes with the whole answer (on another
/// connection, often after it): with none, it ended when the whole request did (`total`). Another one with no event
/// (lost) has no time.
pub fn answer_times(kind: &str, arrivals: &[(String, Duration)], answers: &[(String, String)], total: Duration) -> Vec<Option<Duration>> {
    let mut next = 0;
    let mut prev = Duration::ZERO;
    let last = answers.len().saturating_sub(1);
    answers
        .iter()
        .enumerate()
        .map(|(n, (author, _))| {
            let t = match arrivals[next..].iter().position(|(a, _)| a == author) {
                Some(i) => {
                    next += i + 1;
                    arrivals[next - 1].1
                }
                None if n == last => total.max(prev),
                None => return None,
            };
            let took = if kind == "parallel" { t } else { t.saturating_sub(prev) };
            prev = t;
            Some(took)
        })
        .collect()
}

/// `t` cut to `max` characters, with "…" when cut.
pub fn short_text(t: &str, max: usize) -> String {
    if t.chars().count() <= max {
        return t.to_string();
    }
    let cut: String = t.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// While a task list runs: the next task's line ("Tarefa 2 · Resumidor…"), after `done` tasks answered.
pub fn next_task_text(tasks: &Value, done: usize, agents: &[(String, String)]) -> Option<String> {
    let t = tasks.as_array()?.get(done)?;
    let id = t["agent"].as_str().unwrap_or("");
    let name = agents.iter().find(|(a, _)| a == id).map(|(_, n)| n.clone()).unwrap_or_else(|| id.to_string());
    Some(trf("Task {n} · {name}…", &[("n", (done + 1).to_string()), ("name", name)]))
}

/// The person's agents read last, as (id, name) (agent_picker's list; for the names of members and routes).
pub fn agent_names() -> Vec<(String, String)> {
    crate::agent_picker::cached_agents()
        .iter()
        .filter_map(|a| Some((a["id"].as_str()?.to_string(), a["name"].as_str().unwrap_or("").to_string())))
        .collect()
}

/// A coordinator's route by the decision model, as the line in its steps: "encaminhado para Tradutor · 92 %"
/// (`agent` is the sub-agent's name; None = it kept the request). `sure`: false when the confidence is under the
/// coordinator's threshold (the coordinator's own model answers, and may still hand it over).
pub fn route_text(agent: Option<&str>, confidence: f64, sure: bool) -> String {
    let p = (confidence.clamp(0.0, 1.0) * 100.0).round() as u32;
    match agent {
        Some(a) if sure => trf("routed to {agent} · {p} %", &[("agent", a.to_string()), ("p", p.to_string())]),
        Some(a) => trf("not sure ({agent} · {p} %): the coordinator answers", &[("agent", a.to_string()), ("p", p.to_string())]),
        None => trf("kept by the coordinator · {p} %", &[("p", p.to_string())]),
    }
}

/// The confidence a coordinator's route needs to go straight to the sub-agent (its config's `route_confidence`;
/// kalango-fluxos' default is 0.7).
pub fn route_threshold(config: &Value) -> f64 {
    config["route_confidence"].as_f64().unwrap_or(0.7)
}

/// The route line of a chat.send answer's "route" ({agent, confidence, millis}): the sub-agent's name from `agents`.
pub fn route_line(route: &Value, agents: &[(String, String)], threshold: f64) -> Option<String> {
    let agent = route["agent"].as_str()?;
    let c = route["confidence"].as_f64()?;
    let name = (agent != "proprio").then(|| agents.iter().find(|(id, _)| id == agent).map(|(_, n)| n.clone()).unwrap_or_else(|| agent.to_string()));
    Some(route_text(name.as_deref(), c, c >= threshold))
}

/// Consecutive calls of the same tool, as one step. `first` is the index of its first call.
#[derive(Clone, Debug, PartialEq)]
pub struct StepGroup {
    pub tool: String,
    pub first: usize,
    pub len: usize,
}

/// The steps of a list of calls: consecutive calls of one tool become one step (the design's "6 buscas"). A call
/// waiting for the person, or one after a failure, starts its own step: it must not hide among the ones that went
/// well.
pub fn group_calls(calls: &[StepCall]) -> Vec<StepGroup> {
    let mut out: Vec<StepGroup> = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        let joins = out.last().is_some_and(|g| {
            let prev = &calls[g.first + g.len - 1];
            let quiet = |s: StepState| !matches!(s, StepState::Waiting | StepState::Failed);
            // (what a test would have asked, one after another, is one step too; so are failures in a row)
            let same = |s: StepState| prev.state == s && c.state == s;
            g.tool == c.tool && prev.member.is_none() && c.member.is_none() && ((quiet(prev.state) && c.state != StepState::Waiting) || same(StepState::Waiting) || same(StepState::Failed))
        });
        if joins {
            out.last_mut().unwrap().len += 1;
        } else {
            out.push(StepGroup { tool: c.tool.clone(), first: i, len: 1 });
        }
    }
    out
}

/// How a step's calls read together: the last one's state, except that a failure anywhere shows.
pub fn group_state(calls: &[StepCall]) -> StepState {
    match calls.last().map(|c| c.state) {
        Some(StepState::Done) if calls.iter().any(|c| c.state == StepState::Failed) => StepState::Failed,
        Some(s) => s,
        None => StepState::Done,
    }
}

/// What the running steps' pulse and shine need, at a moment: whether a step runs, whether motion is on (not
/// reduced, not a kept conversation being rebuilt), whether the card is mapped, whether its window is on screen
/// (shown, not minimized, not suspended by the compositor), whether the compositor still draws it (frames keep coming:
/// a window on a workspace not shown gets none, and not every compositor says "suspended"), and where the card is in
/// the scrolled view around it (its top and bottom against the view's height; no view = in sight).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickView {
    pub running: bool,
    pub motion: bool,
    pub mapped: bool,
    pub window_shown: bool,
    pub drawn: bool,
    pub in_view: Option<(f64, f64, f64)>,
}

/// Whether the pulse and shine timer should run: only while a step runs and someone can see it. Anything else stops
/// it, and it comes back when that changes (a step starts, the window shows, the card scrolls into view).
pub fn should_tick(v: TickView) -> bool {
    let seen = v.in_view.is_none_or(|(top, bottom, height)| bottom > 0.0 && top < height);
    v.running && v.motion && v.mapped && v.window_shown && v.drawn && seen
}

/// Whether the compositor shows a window of ours, from what niri says (its windows: pid, title, workspace, minimized;
/// and the workspaces on screen): `None` when niri does not list it (then GTK's own state decides). niri neither
/// marks a window on a workspace out of sight as suspended nor lets GTK see it minimized, so this is how the shell
/// knows.
pub fn shown_on_compositor<'a>(windows: impl IntoIterator<Item = (Option<i32>, &'a str, Option<u64>, bool)>, active: &[u64], pid: i32, title: &str) -> Option<bool> {
    let (_, _, space, minimized) = windows.into_iter().find(|(p, t, _, _)| *p == Some(pid) && *t == title)?;
    Some(!minimized && space.is_some_and(|s| active.contains(&s)))
}

/// What a tool does, in words: (done, done many with {n}, running, what it wants — infinitive, chip noun with {n},
/// glyph, tone). Unknown tools are their name.
struct ToolWords {
    one: &'static str,
    many: &'static str,
    running: &'static str,
    wants: &'static str,
    chip: Option<&'static str>,
    glyph: &'static str,
    tone: &'static str,
}

fn tool_words(tool: &str) -> Option<ToolWords> {
    let w = |one, many, running, wants, chip, glyph, tone| Some(ToolWords { one, many, running, wants, chip, glyph, tone });
    match tool {
        "web.buscar" => w(n_("Searched the web"), n_("Searched the web"), n_("Searching the web…"), n_("search the web"), Some(n_("{n} searches")), "busca", TEAL),
        "web.ler" => w(n_("Read the page"), n_("Read {n} pages"), n_("Reading a page…"), n_("read a page"), None, "doc", TEAL),
        "arquivos.ler" => w(n_("Read the file"), n_("Read {n} files"), n_("Reading a file…"), n_("read a file"), None, "doc", TEAL),
        "arquivos.listar" => w(n_("Looked in the folder"), n_("Looked in {n} folders"), n_("Looking in a folder…"), n_("look in a folder"), None, "pasta", TEAL),
        "arquivos.mover" => w(n_("Moved"), n_("Moved {n} items"), n_("Moving…"), n_("move files"), None, "pasta", ACCENT),
        "arquivos.lixeira" => w(n_("Sent to the trash"), n_("Sent {n} items to the trash"), n_("Sending to the trash…"), n_("send to the trash"), None, "lixeira", ACCENT),
        "arquivos.escrever" => w(n_("Wrote a file"), n_("Wrote {n} files"), n_("Writing…"), n_("write a file"), None, "doc", ACCENT),
        "terminal.executar" | "sistema.executar" | "tarefas.executar" => {
            w(n_("Ran a command"), n_("Ran {n} commands"), n_("Running a command…"), n_("run a command"), None, "terminal", ACCENT)
        }
        // a sandbox's command (kalangod's tool in the agent's disposable machine)
        "maquina_executar" | "maquina.executar" => {
            w(n_("Ran in the machine"), n_("Ran {n} commands in the machine"), n_("Running in the machine…"), n_("run in the machine"), None, "terminal", ACCENT)
        }
        "linha.registrar" => w(n_("Wrote in the Timeline"), n_("Wrote in the Timeline {n} times"), n_("Writing in the Timeline…"), n_("write in the Timeline"), None, "relogio", TEAL),
        "notificar" => w(n_("Sent a notice"), n_("Sent {n} notices"), n_("Sending a notice…"), n_("send a notice"), None, "sino", TEAL),
        _ => None,
    }
}

/// The step's words: its label for `n` calls in this state.
pub fn step_label(tool: &str, n: usize, state: StepState) -> String {
    let Some(w) = tool_words(tool) else { return tool.to_string() };
    match state {
        StepState::Running => tr(w.running).to_string(),
        StepState::Waiting => trf("Wants to {action}", &[("action", tr(w.wants).to_string())]),
        StepState::Failed => trf("Could not {action}", &[("action", tr(w.wants).to_string())]),
        _ if n > 1 && w.many.contains("{n}") => trf(w.many, &[("n", n.to_string())]),
        _ => tr(w.one).to_string(),
    }
}

/// The count chip of a step of `n` calls ("6 buscas"; only the number in the compact panel).
pub fn step_chip(tool: &str, n: usize, compact: bool) -> Option<String> {
    let chip = tool_words(tool).and_then(|w| w.chip)?;
    (n > 1).then(|| if compact { n.to_string() } else { trf(chip, &[("n", n.to_string())]) })
}

fn step_glyph_tone(tool: &str) -> (&'static str, &'static str) {
    tool_words(tool).map(|w| (w.glyph, w.tone)).unwrap_or(("raio", ACCENT))
}

/// "4,2 s" / "3 min".
pub fn seconds(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        // (the decimal comma where the language writes it)
        let n = format!("{s:.1}");
        let n = if kalango_i18n::system_locale().is_some_and(|l| l.starts_with("pt")) { n.replace('.', ",") } else { n };
        trf("{s} s", &[("s", n)])
    } else {
        trf("{m} min", &[("m", ((s / 60.0).round() as u64).to_string())])
    }
}

const TEAL: &str = "#8ED2C3";
const GOLD: &str = "#E5C98C";
const CORAL: &str = "#EFA2A6";
const MUTED: &str = "#8E8F99";
/// k_divider (dark value; the theme swaps it by value)
const DIVIDER: &str = "#2E2F3A";

/// A row of the list (one step) and what its motion needs.
struct StepRow {
    anim: Animavel,
    rail: gtk::DrawingArea,
    shield: gtk::Image,
    tile: gtk::Box,
    icon: gtk::Image,
    /// a member row's mono mark ("Tarefa 1")
    tag: gtk::Label,
    label: gtk::Label,
    detail: gtk::Label,
    chip_anim: Animavel,
    chip: gtk::Label,
    time: gtk::Label,
    /// "tentar de novo", on a failed step when the list can send again
    retry: gtk::Button,
    chevron: gtk::DrawingArea,
    details: gtk::Box,
    revealer: gtk::Revealer,
    /// the running mark (ring and dot), drawn once: it pulses by its opacity
    pulse: Animavel,
    /// the progress line under a running step and its 120 px shine, drawn once: it slides by a translation
    progress: gtk::Overlay,
    shine: Animavel,
    state: Cell<StepState>,
    /// the ✓ being drawn (0 → 1), the ×'s shake start, the chevron's angle (0 → 1 = 90°)
    check: Cell<f64>,
    shake_from: Cell<Option<Instant>>,
    chevron_turn: RefCell<kalango_movimento::Mola>,
    last: Cell<bool>,
    started: Cell<Instant>,
}

/// The steps of one answer (a card): its header, the list and the motion.
pub struct StepsList {
    /// the card (it keeps this list alive: see `new`)
    root: glib::WeakRef<gtk::Box>,
    /// the card until something holds it (a list, a box): then only the card holds this list
    root_hold: RefCell<Option<gtk::Box>>,
    head_icon: gtk::Image,
    head_title: gtk::Label,
    fold: gtk::Button,
    revealer: gtk::Revealer,
    list: gtk::Box,
    rows: RefCell<Vec<Rc<StepRow>>>,
    calls: RefCell<Vec<StepCall>>,
    /// a step with no tool yet ("Pensando…", "Escrevendo…") at the end of the list
    thinking: RefCell<Option<Rc<StepRow>>>,
    compact: bool,
    started: Cell<Instant>,
    /// when the last call started (its time when it ends)
    call_started: Cell<Instant>,
    finished: Cell<Option<Duration>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    /// the running steps' pulse and shine (see `animate`)
    ambient: RefCell<Option<glib::SourceId>>,
    /// the window and the scrolled view are watched (the timer comes back when they change)
    watching: Cell<bool>,
    /// the frame clock's last frame time and when it changed (monotonic µs): frames stop when nothing shows the window
    last_frame: Cell<(i64, i64)>,
    /// waiting for the next frame to come back after frames stopped
    frame_wake: RefCell<Option<(gtk::gdk::FrameClock, glib::SignalHandlerId)>>,
    /// "tentar de novo" on a failed step: sends the last request again
    on_retry: RefCell<Option<Rc<dyn Fn()>>>,
    clock: RefCell<Option<glib::SourceId>>,
    /// rebuilding a kept conversation: nothing moves
    instant: Cell<bool>,
    /// the text after the time of a step waiting for the person ("aguarda você", "teste")
    waiting_text: RefCell<String>,
    /// a coordinator's route ("encaminhado para Tradutor · 92 %"), above the steps
    route: gtk::Label,
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn reduced() -> bool {
    kalango_movimento::reduzido()
}

impl StepsList {
    /// `compact`: the "Testar agora" panel (the chip is only the number).
    pub fn new(compact: bool) -> Rc<StepsList> {
        let root = caixa(Orientation::Vertical, 0, "passos-cartao");
        let head = caixa(Orientation::Horizontal, 8, "passos-cab");
        let head_icon = ui_icon("raio", ACCENT, 14);
        head.append(&head_icon);
        let head_title = rotulo(tr("Thinking…"), "passos-cab-titulo");
        head_title.set_hexpand(true);
        head_title.set_xalign(0.0);
        head_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&head_title);
        let fold = gtk::Button::with_label(tr("collapse"));
        fold.add_css_class("passos-recolher");
        fold.set_valign(Align::Center);
        head.append(&fold);
        root.append(&head);
        let list = caixa(Orientation::Vertical, 0, "passos-linhas");
        let route = rotulo("", "passo-rota");
        route.set_xalign(0.0);
        route.set_ellipsize(gtk::pango::EllipsizeMode::End);
        route.set_visible(false);
        // (outside the folding list: a folded turn still says where the coordinator sent it)
        root.append(&route);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(220);
        revealer.set_reveal_child(true);
        revealer.set_child(Some(&list));
        root.append(&revealer);
        let s = Rc::new(StepsList {
            root: root.downgrade(),
            root_hold: RefCell::new(Some(root.clone())),
            head_icon,
            head_title,
            fold: fold.clone(),
            revealer: revealer.clone(),
            list,
            rows: RefCell::default(),
            calls: RefCell::default(),
            thinking: RefCell::default(),
            compact,
            started: Cell::new(Instant::now()),
            call_started: Cell::new(Instant::now()),
            finished: Cell::new(None),
            tick: RefCell::default(),
            ambient: RefCell::default(),
            watching: Cell::new(false),
            last_frame: Cell::new((0, 0)),
            frame_wake: RefCell::default(),
            on_retry: RefCell::default(),
            clock: RefCell::default(),
            instant: Cell::new(false),
            waiting_text: RefCell::new(tr("waiting for you").to_string()),
            route,
        });
        fold.connect_clicked(move |b| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            b.set_label(if open { tr("collapse") } else { tr("show") });
        });
        s.start_clock();
        // the card owns the list: it lives as long as the card is somewhere (the list only holds it weakly), so the
        // marks keep drawing after the turn that made them is over
        unsafe {
            root.set_data("kalango-passos", s.clone());
        }
        // in sight again (mapped, the window shown, scrolled into view): the motion comes back
        let w = Rc::downgrade(&s);
        root.connect_map(move |_| {
            if let Some(s) = w.upgrade() {
                s.watch();
                s.animate();
            }
        });
        let w = Rc::downgrade(&s);
        root.connect_parent_notify(move |r| {
            if r.parent().is_some() {
                if let Some(s) = w.upgrade() {
                    s.root_hold.borrow_mut().take();
                }
            }
        });
        s
    }

    /// The card, to put in a conversation.
    pub fn widget(&self) -> gtk::Box {
        self.root.upgrade().or_else(|| self.root_hold.borrow().clone()).expect("the card owns its steps list")
    }

    /// What "tentar de novo" on a failed step does: send the last request again (the same agent, message and input).
    pub fn set_on_retry(&self, f: impl Fn() + 'static) {
        *self.on_retry.borrow_mut() = Some(Rc::new(f));
    }

    /// Rebuilding a kept conversation: no motion.
    pub fn set_instant(&self, on: bool) {
        self.instant.set(on);
    }

    pub fn set_waiting_text(&self, t: &str) {
        *self.waiting_text.borrow_mut() = t.to_string();
    }

    /// A coordinator's route above the steps ("encaminhado para Tradutor · 92 %"); empty hides it.
    pub fn set_route(&self, text: &str) {
        self.route.set_text(text);
        self.route.set_visible(!text.is_empty());
        self.fold.set_visible(!self.is_empty() || !text.is_empty());
    }

    /// Before the first step: what is happening ("Pensando…", "Ligando o modelo…").
    pub fn set_status(&self, text: &str) {
        if self.calls.borrow().is_empty() && self.finished.get().is_none() {
            self.head_title.set_text(text);
        }
    }

    pub fn len(&self) -> usize {
        group_calls(&self.calls.borrow()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.borrow().is_empty()
    }

    /// Takes every step out (the calls, to put whole ones in their place).
    pub fn clear(&self) -> Vec<StepCall> {
        self.drop_thinking();
        for r in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r.anim);
        }
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    /// The last call (its tool and arguments: a web result, a terminal block).
    pub fn last_call(&self) -> Option<StepCall> {
        self.calls.borrow().last().cloned()
    }

    /// The last `n` steps as the desktop shows them: (✓ done, ◌ running, ! waiting, ✗ failed; text; when).
    pub fn marks(&self, n: usize) -> Vec<(char, String, i64)> {
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let rows = self.rows.borrow();
        let k = rows.len().saturating_sub(n);
        rows[k..]
            .iter()
            .map(|r| {
                let m = match r.state.get() {
                    StepState::Done => '✓',
                    StepState::Failed => '✗',
                    StepState::Waiting => '!',
                    StepState::Running => '◌',
                };
                (m, r.label.text().to_string(), now - r.started.get().elapsed().as_secs() as i64)
            })
            .collect()
    }

    /// A step with no tool, running ("Escrevendo o resumo…"): it goes when a tool starts or the list finishes.
    pub fn show_thinking(self: &Rc<Self>, text: &str) {
        if let Some(r) = self.thinking.borrow().as_ref() {
            r.label.set_text(text);
            return;
        }
        let r = self.new_row("doc", ACCENT, text, StepState::Running);
        self.list.append(&r.anim);
        self.set_state(&r, StepState::Running);
        r.label.set_text(text);
        r.chevron.set_visible(false);
        *self.thinking.borrow_mut() = Some(r);
        self.update_head();
        self.relink();
        self.animate();
    }

    fn drop_thinking(&self) {
        if let Some(r) = self.thinking.borrow_mut().take() {
            self.list.remove(&r.anim);
        }
    }

    /// A tool started (or, in a test's result, ran: then `state` is its end).
    pub fn push(self: &Rc<Self>, call: StepCall) {
        self.drop_thinking();
        self.call_started.set(Instant::now());
        let (joins, n) = {
            let mut calls = self.calls.borrow_mut();
            calls.push(call.clone());
            let groups = group_calls(&calls);
            let g = groups.last().unwrap();
            (g.len > 1, g.len)
        };
        if joins {
            // the same tool again: the step counts it (the chip's number rolls)
            let r = self.rows.borrow().last().cloned();
            if let Some(r) = r {
                self.fill(&r, n);
                if !self.instant.get() && !reduced() {
                    r.chip_anim.definir(Canal::Y, 6.0).definir(Canal::Opacidade, 0.0);
                    r.chip_anim.ir(Canal::Y, 0.0, RAPIDA).ir(Canal::Opacidade, 1.0, RAPIDA);
                }
                self.set_state(&r, call.state);
            }
        } else {
            let (g, tone) = step_glyph_tone(&call.tool);
            let r = self.new_row(g, tone, "", call.state);
            r.started.set(Instant::now());
            self.fill(&r, 1);
            self.list.append(&r.anim);
            self.rows.borrow_mut().push(r.clone());
            if !self.instant.get() {
                r.anim.definir(Canal::Y, 6.0).definir(Canal::Opacidade, 0.0);
                r.anim.ir(Canal::Y, 0.0, SUAVE).ir(Canal::Opacidade, 1.0, SUAVE);
            }
            self.set_state(&r, call.state);
        }
        self.relink();
        self.update_head();
        self.animate();
    }

    /// The last call ended: how, what came back (or why it failed) and how long it took.
    pub fn finish_last(self: &Rc<Self>, ok: bool, result: &str, took: Option<Duration>) {
        let n = {
            let mut calls = self.calls.borrow_mut();
            let Some(c) = calls.last_mut() else { return };
            c.state = if ok { StepState::Done } else { StepState::Failed };
            c.result = result.to_string();
            // (a kept conversation being rebuilt has no real times)
            c.took = took.or_else(|| (!self.instant.get()).then(|| self.call_started.get().elapsed()));
            group_calls(&calls).last().map(|g| g.len).unwrap_or(1)
        };
        let Some(r) = self.rows.borrow().last().cloned() else { return };
        self.fill(&r, n);
        self.set_state(&r, if ok { StepState::Done } else { StepState::Failed });
        self.update_head();
        self.animate();
    }

    /// The last call is waiting for the person.
    pub fn wait_last(self: &Rc<Self>) {
        let n = {
            let mut calls = self.calls.borrow_mut();
            let Some(c) = calls.last_mut() else { return };
            c.state = StepState::Waiting;
            group_calls(&calls).last().map(|g| g.len).unwrap_or(1)
        };
        if let Some(r) = self.rows.borrow().last().cloned() {
            self.fill(&r, n);
            self.set_state(&r, StepState::Waiting);
        }
    }

    /// Every tool still running is over (the service says when a tool starts, not when it ends: the next tool or a
    /// member's answer means the ones before ended).
    pub fn settle_running(self: &Rc<Self>) {
        let mut any = false;
        for c in self.calls.borrow_mut().iter_mut().filter(|c| c.state == StepState::Running) {
            c.state = StepState::Done;
            any = true;
        }
        if !any {
            return;
        }
        let running: Vec<Rc<StepRow>> = self.rows.borrow().iter().filter(|r| r.state.get() == StepState::Running).cloned().collect();
        for r in running {
            self.set_state(&r, StepState::Done);
        }
        self.update_head();
    }

    /// The person decided on the first step waiting for them (an approval of kalango-fluxos): allowed, it runs
    /// (the answer ends it); denied, it failed ("negado"). Without this the step kept "aguarda você" to the end.
    pub fn decide_waiting(self: &Rc<Self>, allowed: bool) {
        let (before, after, at) = {
            let mut calls = self.calls.borrow_mut();
            let before = group_calls(&calls);
            let Some(i) = calls.iter().position(|c| c.state == StepState::Waiting) else { return };
            calls[i].state = if allowed { StepState::Running } else { StepState::Failed };
            if !allowed {
                calls[i].result = tr("denied").to_string();
            }
            (before, group_calls(&calls), i)
        };
        let same = before.len() == after.len() && before.iter().zip(&after).all(|(a, b)| a.first == b.first && a.len == b.len);
        if !same {
            // (a group split or joined: the rows again, as they are now)
            let calls = self.clear();
            let instant = self.instant.replace(true);
            for c in calls {
                self.push(c);
            }
            self.instant.set(instant);
            return;
        }
        let Some(gi) = after.iter().position(|g| (g.first..g.first + g.len).contains(&at)) else { return };
        let Some(r) = self.rows.borrow().get(gi).cloned() else { return };
        let st = group_state(&self.calls.borrow()[after[gi].first..after[gi].first + after[gi].len]);
        self.fill_group(&r, Some(gi), after[gi].len);
        self.set_state(&r, st);
        self.update_head();
    }

    /// The answer came: "Trabalhou por 14 s · 5 passos"; `fold` closes the list (the person can open it).
    pub fn finish(self: &Rc<Self>, took: Duration, fold: bool) {
        self.drop_thinking();
        // (a call still running when the answer came is over)
        let running: Vec<Rc<StepRow>> = self.rows.borrow().iter().filter(|r| r.state.get() == StepState::Running).cloned().collect();
        for r in running {
            self.set_state(&r, StepState::Done);
        }
        self.finished.set(Some(took));
        if let Some(c) = self.clock.borrow_mut().take() {
            c.remove();
        }
        self.relink();
        self.update_head();
        if fold && !self.is_empty() {
            self.revealer.set_reveal_child(false);
            self.fold.set_label(tr("show"));
        }
    }

    fn update_head(&self) {
        let n = self.len();
        let steps = if n == 1 { tr("1 step").to_string() } else { trf("{n} steps", &[("n", n.to_string())]) };
        match self.finished.get() {
            Some(t) => {
                self.head_title.set_text(&trf("Worked for {time} · {steps}", &[("time", seconds(t)), ("steps", steps)]));
                self.head_icon.remove_css_class("pulsando");
                crate::conversa::set_icon(&self.head_icon, "raio", MUTED);
            }
            None if n == 0 && self.thinking.borrow().is_some() => {
                self.head_title.set_text(&trf("Working · {time}", &[("time", seconds(self.started.get().elapsed()))]));
            }
            None if n > 0 => {
                self.head_title.set_text(&trf("Working · {steps} · {time}", &[("steps", steps), ("time", seconds(self.started.get().elapsed()))]));
            }
            None => {}
        }
        self.fold.set_visible(n > 0);
    }

    /// The header's seconds while it works (once a second; nothing else moves for it).
    fn start_clock(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(1, move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            if s.finished.get().is_some() {
                return glib::ControlFlow::Break;
            }
            s.update_head();
            glib::ControlFlow::Continue
        });
        *self.clock.borrow_mut() = Some(id);
    }

    /// Which row is the last (it draws no connector below its mark).
    fn relink(&self) {
        let rows = self.rows.borrow();
        let thinking = self.thinking.borrow().clone();
        let all: Vec<Rc<StepRow>> = rows.iter().cloned().chain(thinking).collect();
        for (i, r) in all.iter().enumerate() {
            let last = i + 1 == all.len();
            if r.last.get() != last {
                r.last.set(last);
                r.rail.queue_draw();
            }
        }
    }

    fn set_state(&self, r: &StepRow, st: StepState) {
        let before = r.state.get();
        r.state.set(st);
        let (g, tone) = match st {
            StepState::Failed => ("doc", CORAL),
            StepState::Waiting => ("escudo", GOLD),
            _ => {
                let tool = self.rows.borrow().iter().position(|x| std::ptr::eq(x.as_ref(), r)).and_then(|i| group_calls(&self.calls.borrow()).get(i).map(|g| g.tool.clone()));
                let (g, t) = tool.map(|t| step_glyph_tone(&t)).unwrap_or(("doc", ACCENT));
                (g, if st == StepState::Running { ACCENT } else { t })
            }
        };
        crate::conversa::set_icon(&r.icon, g, tone);
        for c in ["teal", "accent", "gold", "coral"] {
            r.tile.remove_css_class(c);
        }
        r.tile.add_css_class(match tone {
            TEAL => "teal",
            GOLD => "gold",
            CORAL => "coral",
            _ => "accent",
        });
        r.shield.set_visible(st == StepState::Waiting);
        r.progress.set_visible(st == StepState::Running);
        r.pulse.set_visible(st == StepState::Running);
        // (with less motion the mark is still and full, and there is no shine)
        r.shine.set_visible(st == StepState::Running && !reduced() && !self.instant.get());
        if st != StepState::Running || reduced() || self.instant.get() {
            r.pulse.definir(Canal::Opacidade, 1.0);
        }
        for c in ["ativo", "feito", "espera", "falhou"] {
            r.label.remove_css_class(c);
        }
        r.label.add_css_class(match st {
            StepState::Running => "ativo",
            StepState::Done => "feito",
            StepState::Waiting => "espera",
            StepState::Failed => "falhou",
        });
        if before != st {
            let moving = !self.instant.get() && !reduced();
            match st {
                // the ✓ draws itself (220 ms); with less motion it is there at once
                StepState::Done => r.check.set(if moving { 0.0 } else { 1.0 }),
                StepState::Failed if moving => r.shake_from.set(Some(Instant::now())),
                _ => {}
            }
            if !moving && !self.instant.get() {
                // state changes are a short fade only
                r.anim.definir(Canal::Opacidade, 0.4);
                r.anim.ir(Canal::Opacidade, 1.0, RAPIDA);
            }
        }
        r.rail.queue_draw();
    }

    /// The row's words for its `n` calls.
    fn fill(&self, r: &StepRow, n: usize) {
        self.fill_group(r, None, n);
    }

    /// The words of the row of group `gi` (None: the last one).
    fn fill_group(&self, r: &StepRow, gi: Option<usize>, n: usize) {
        let calls = self.calls.borrow();
        let groups = group_calls(&calls);
        let Some(g) = (match gi {
            Some(i) => groups.into_iter().nth(i),
            None => groups.into_iter().last(),
        }) else {
            return;
        };
        let mine = &calls[g.first..g.first + g.len];
        let st = group_state(mine);
        let last = mine.last().unwrap();
        if let Some(m) = &last.member {
            return self.fill_member(r, m, last, st);
        }
        r.tag.set_visible(false);
        r.tile.set_visible(true);
        r.label.remove_css_class("membro");
        r.label.set_text(&if st == StepState::Waiting && self.compact { tr("Would ask for approval").to_string() } else { step_label(&g.tool, n, st) });
        // the detail: what it works on (the distinct ones, for a group), or the failure's reason
        let mut args: Vec<&str> = Vec::new();
        for c in mine {
            if !c.arg.is_empty() && !args.contains(&c.arg.as_str()) {
                args.push(&c.arg);
            }
        }
        let detail = if st == StepState::Failed && !last.result.is_empty() {
            last.result.lines().next().unwrap_or("").to_string()
        } else if st == StepState::Waiting && self.compact {
            g.tool.rsplit('.').next().unwrap_or("").to_string()
        } else if g.len > 1 && step_chip(&g.tool, n, self.compact).is_some() {
            String::new()
        } else {
            args.join(", ")
        };
        r.detail.set_text(&detail);
        r.detail.set_visible(!detail.is_empty());
        match step_chip(&g.tool, n, self.compact) {
            Some(c) => {
                r.chip.set_text(&c);
                r.chip_anim.set_visible(true);
            }
            None => r.chip_anim.set_visible(false),
        }
        let took: Option<Duration> = mine.iter().map(|c| c.took).sum();
        r.time.set_text(&match st {
            StepState::Waiting => self.waiting_text.borrow().clone(),
            StepState::Running => String::new(),
            _ => took.map(seconds).unwrap_or_default(),
        });
        // (failed, where the request can be sent again: "tentar de novo" in the time's place)
        let retry = st == StepState::Failed && self.on_retry.borrow().is_some();
        r.retry.set_visible(retry);
        r.time.set_visible(!retry);
        // the opened part: each call's argument and what came back (three, then "+ N")
        crate::fluxos_ui::clear_box(&r.details);
        let technical = crate::agent_prefs::load().technical_details;
        // (the tool's technical name, for who wants to know what ran)
        let name = rotulo(&g.tool, "passo-ferramenta");
        name.set_xalign(0.0);
        name.set_selectable(true);
        r.details.append(&name);
        for c in mine.iter().take(3) {
            let line = caixa(Orientation::Horizontal, 12, "");
            let a = rotulo(if c.arg.is_empty() { &c.tool } else { &c.arg }, "passo-arg");
            a.set_xalign(0.0);
            a.set_hexpand(true);
            a.set_ellipsize(gtk::pango::EllipsizeMode::End);
            a.set_selectable(true);
            line.append(&a);
            let res = rotulo(c.result.lines().next().unwrap_or(""), "passo-res");
            res.set_ellipsize(gtk::pango::EllipsizeMode::End);
            res.set_max_width_chars(28);
            line.append(&res);
            r.details.append(&line);
            if technical && !c.args_json.is_empty() {
                let t = rotulo(&format!("{} {}", c.tool, c.args_json), "passo-tecnico");
                t.set_wrap(true);
                t.set_wrap_mode(gtk::pango::WrapMode::Char);
                t.set_xalign(0.0);
                t.set_selectable(true);
                t.set_max_width_chars(1);
                r.details.append(&t);
            }
        }
        r.chevron.set_visible(!retry && r.details.first_child().is_some());
        if mine.len() > 3 {
            let more = mine.len() - 3;
            let t = match step_chip(&g.tool, more, false) {
                Some(c) if more > 1 => format!("+ {c}"),
                _ => format!("+ {more}"),
            };
            let more = rotulo(&t, "passo-mais");
            more.set_xalign(0.0);
            r.details.append(&more);
        }
    }

    /// A member's row: "Tarefa 1 · Buscador … tabela com 7 linhas · 3,1 s"; opened, what it answered.
    fn fill_member(&self, r: &StepRow, m: &Member, c: &StepCall, st: StepState) {
        r.tile.set_visible(false);
        r.tag.set_text(&m.tag);
        r.tag.set_visible(!m.tag.is_empty());
        r.label.set_text(&m.name);
        r.label.add_css_class("membro");
        r.detail.set_visible(false);
        r.chip_anim.set_visible(false);
        let time = match st {
            StepState::Running => tr("working…").to_string(),
            StepState::Waiting => self.waiting_text.borrow().clone(),
            // (the shape gives way, never the time: "tabela com 3 linh… · 4,0 s")
            _ => [short_text(&c.result, if self.compact { 16 } else { 34 }), c.took.map(seconds).unwrap_or_default()]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        };
        r.time.set_text(&time);
        r.time.set_visible(true);
        // (the name gives way first, then nothing: the row keeps its time)
        r.label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        r.label.set_width_chars(10);
        r.retry.set_visible(false);
        crate::fluxos_ui::clear_box(&r.details);
        let lines: Vec<&str> = m.text.trim().lines().collect();
        let shown = lines.iter().take(8).copied().collect::<Vec<_>>().join("\n");
        if !shown.is_empty() {
            let t = rotulo(&shown, "passo-arg");
            t.set_wrap(true);
            t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            t.set_xalign(0.0);
            t.set_selectable(true);
            t.set_max_width_chars(1);
            t.set_hexpand(true);
            r.details.append(&t);
            if lines.len() > 8 {
                let more = rotulo(&format!("+ {}", lines.len() - 8), "passo-mais");
                more.set_xalign(0.0);
                r.details.append(&more);
            }
        }
        r.chevron.set_visible(r.details.first_child().is_some());
    }

    fn new_row(self: &Rc<Self>, glyph: &str, tone: &str, text: &str, st: StepState) -> Rc<StepRow> {
        let block = caixa(Orientation::Horizontal, 10, "passo-bloco");
        // the rail: the mark and the line to the next one
        let rail = gtk::DrawingArea::new();
        rail.set_content_width(22);
        rail.set_vexpand(true);
        let shield = ui_icon("escudo", GOLD, 12);
        shield.set_halign(Align::Center);
        shield.set_valign(Align::Start);
        shield.set_margin_top(8);
        shield.set_can_target(false);
        shield.set_visible(false);
        let pulse_mark = gtk::DrawingArea::new();
        pulse_mark.set_content_width(22);
        pulse_mark.set_content_height(22);
        pulse_mark.set_draw_func(|_, cr, w, h| draw_running_mark(cr, w, h));
        let pulse = Animavel::new(&pulse_mark);
        pulse.set_halign(Align::Center);
        pulse.set_valign(Align::Start);
        pulse.set_margin_top(3);
        pulse.set_can_target(false);
        pulse.set_visible(false);
        let ro = gtk::Overlay::new();
        ro.set_child(Some(&rail));
        ro.add_overlay(&shield);
        ro.add_overlay(&pulse);
        block.append(&ro);
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_hexpand(true);
        let line = gtk::Button::new();
        line.add_css_class("passo-linha");
        let lb = caixa(Orientation::Horizontal, 8, "");
        let tile = caixa(Orientation::Horizontal, 0, "passo-tile");
        tile.set_valign(Align::Center);
        tile.set_hexpand(false);
        tile.set_homogeneous(true);
        let icon = ui_icon(glyph, tone, 14);
        tile.append(&icon);
        lb.append(&tile);
        let tag = rotulo("", "passo-marca");
        tag.set_visible(false);
        lb.append(&tag);
        let label = rotulo(text, "passo-rotulo");
        label.set_xalign(0.0);
        lb.append(&label);
        let detail = rotulo("", "passo-detalhe");
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_hexpand(true);
        detail.set_width_chars(4);
        lb.append(&detail);
        let chip = rotulo("", "passo-chip");
        let chip_anim = Animavel::new(&chip);
        chip_anim.set_valign(Align::Center);
        chip_anim.set_visible(false);
        lb.append(&chip_anim);
        let filler = caixa(Orientation::Horizontal, 0, "");
        filler.set_hexpand(true);
        lb.append(&filler);
        let time = rotulo("", "passo-tempo");
        lb.append(&time);
        let retry = gtk::Button::with_label(tr("try again"));
        retry.add_css_class("passo-tentar");
        retry.set_valign(Align::Center);
        retry.set_visible(false);
        let ws = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            let f = ws.upgrade().and_then(|s| s.on_retry.borrow().clone());
            if let Some(f) = f {
                f();
            }
        });
        let chevron = gtk::DrawingArea::new();
        chevron.set_content_width(14);
        chevron.set_content_height(14);
        chevron.set_valign(Align::Center);
        lb.append(&chevron);
        line.set_child(Some(&lb));
        // (the link sits beside the row's button, not in it: a button in a button gets no clicks)
        line.set_hexpand(true);
        let line_box = caixa(Orientation::Horizontal, 0, "");
        line_box.append(&line);
        line_box.append(&retry);
        col.append(&line_box);
        let details = caixa(Orientation::Vertical, 2, "passo-detalhes");
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(220);
        revealer.set_child(Some(&details));
        col.append(&revealer);
        // the running step's progress line (a shine slides along it)
        let track = caixa(Orientation::Horizontal, 0, "passo-progresso");
        track.set_hexpand(true);
        let shine_line = gtk::DrawingArea::new();
        shine_line.set_content_width(SHINE);
        shine_line.set_content_height(2);
        shine_line.set_draw_func(|_, cr, w, h| draw_shine(cr, w, h));
        let shine = Animavel::new(&shine_line);
        shine.set_halign(Align::Start);
        shine.set_can_target(false);
        let progress = gtk::Overlay::new();
        progress.set_child(Some(&track));
        progress.add_overlay(&shine);
        progress.set_overflow(gtk::Overflow::Hidden);
        progress.set_margin_top(4);
        progress.set_visible(false);
        col.append(&progress);
        block.append(&col);
        let anim = Animavel::new(&block);
        let r = Rc::new(StepRow {
            anim,
            rail: rail.clone(),
            shield,
            tile,
            icon,
            tag,
            label,
            detail,
            chip_anim,
            chip,
            time,
            retry,
            chevron: chevron.clone(),
            details,
            revealer: revealer.clone(),
            pulse: pulse.clone(),
            progress: progress.clone(),
            shine,
            state: Cell::new(st),
            check: Cell::new(1.0),
            shake_from: Cell::new(None),
            chevron_turn: RefCell::new(kalango_movimento::Mola::new(0.0)),
            last: Cell::new(true),
            started: Cell::new(Instant::now()),
        });
        let wr = Rc::downgrade(&r);
        let ws = Rc::downgrade(self);
        rail.set_draw_func(move |_, cr, w, h| {
            if let (Some(r), Some(s)) = (wr.upgrade(), ws.upgrade()) {
                s.draw_rail(&r, cr, w, h);
            }
        });
        let wr = Rc::downgrade(&r);
        chevron.set_draw_func(move |_, cr, w, h| {
            if let Some(r) = wr.upgrade() {
                draw_chevron(&r, cr, w, h);
            }
        });
        for d in [&rail, &chevron, &pulse_mark, &shine_line] {
            crate::tema::registrar(d, |w| w.queue_draw());
        }
        // a click opens what it got and gave back (the chevron turns 90°)
        let wr = Rc::downgrade(&r);
        let ws: Weak<StepsList> = Rc::downgrade(self);
        line.connect_clicked(move |_| {
            let (Some(r), Some(s)) = (wr.upgrade(), ws.upgrade()) else { return };
            let open = !r.revealer.reveals_child();
            r.revealer.set_transition_duration(if open { 220 } else { 160 });
            r.revealer.set_reveal_child(open);
            let to = if open { 1.0 } else { 0.0 };
            if reduced() {
                r.chevron_turn.borrow_mut().pular(to);
                r.chevron.queue_draw();
            } else {
                r.chevron_turn.borrow_mut().ir(to, SUAVE);
                s.animate();
            }
        });
        r
    }

    /// Whether a one-off motion is under way (a ✓ drawing itself, a shake, a chevron turning): then the tick goes on.
    fn moving(&self) -> bool {
        let still = |r: &StepRow| r.check.get() < 1.0 || r.shake_from.get().is_some() || !r.chevron_turn.borrow().parada();
        self.rows.borrow().iter().any(|r| still(r)) || self.thinking.borrow().as_ref().is_some_and(|r| still(r))
    }

    /// The list as the pulse and shine timer sees it now (see `should_tick`).
    fn tick_view(&self) -> TickView {
        let run = |r: &StepRow| r.state.get() == StepState::Running;
        let running = self.rows.borrow().iter().any(|r| run(r)) || self.thinking.borrow().as_ref().is_some_and(|r| run(r));
        let root = self.root.upgrade();
        let window = root.as_ref().and_then(|r| r.root()).and_downcast::<gtk::Window>();
        let window_shown = window.is_some_and(|w| {
            let minimized = w.surface().and_downcast::<gtk::gdk::Toplevel>().is_some_and(|t| t.state().contains(gtk::gdk::ToplevelState::MINIMIZED));
            // (and what niri says of it: on a workspace in sight, not minimized)
            let niri = crate::shell_atual().and_then(|sh| {
                let spaces: Vec<u64> = sh.estado.espacos.borrow().iter().filter(|e| e.ativo).map(|e| e.id).collect();
                let title = w.title().map(|t| t.to_string()).unwrap_or_default();
                let wins = sh.estado.janelas.borrow();
                shown_on_compositor(wins.values().map(|j| (j.pid, j.titulo.as_str(), j.espaco, j.minimizada)), &spaces, std::process::id() as i32, &title)
            });
            w.is_visible() && !w.is_suspended() && !minimized && niri.unwrap_or(true)
        });
        let in_view = root.as_ref().and_then(|r| {
            let view = r.ancestor(gtk::ScrolledWindow::static_type())?;
            let b = r.compute_bounds(&view)?;
            Some((b.y() as f64, (b.y() + b.height()) as f64, view.height() as f64))
        });
        // (drawn: the last frame came less than half a second ago, or the timer has not run long enough to tell)
        let drawn = glib::monotonic_time() - self.last_frame.get().1 < 500_000;
        TickView { running, motion: !reduced() && !self.instant.get(), mapped: root.is_some_and(|r| r.is_mapped()), window_shown, drawn, in_view }
    }

    fn running(&self) -> bool {
        should_tick(self.tick_view())
    }

    fn frame_time(&self) -> i64 {
        self.root.upgrade().and_then(|r| r.frame_clock()).map(|c| c.frame_time()).unwrap_or(0)
    }

    /// Frames stopped (the window is not shown): the next frame, when the compositor draws the window again, brings
    /// the motion back. Asking for one frame costs nothing while none comes.
    fn wake_on_next_frame(self: &Rc<Self>) {
        let Some(root) = self.root.upgrade() else { return };
        let Some(clock) = root.frame_clock() else { return };
        if self.frame_wake.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = clock.connect_after_paint(move |c| {
            let Some(s) = w.upgrade() else { return };
            if let Some((_, id)) = s.frame_wake.borrow_mut().take() {
                c.disconnect(id);
            }
            if debug() {
                eprintln!("debug passos: quadros voltaram");
            }
            s.animate();
        });
        *self.frame_wake.borrow_mut() = Some((clock, id));
        root.queue_draw();
    }

    /// Watches what brings the motion back once the card is in a window: the window shown or restored, and the
    /// scrolled view around it moving.
    fn watch(self: &Rc<Self>) {
        let Some(root) = self.root.upgrade() else { return };
        let Some(window) = root.root().and_downcast::<gtk::Window>() else { return };
        if self.watching.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        let wake = move || {
            if let Some(s) = w.upgrade() {
                if debug() {
                    eprintln!("debug passos: acordar ({:?})", s.tick_view());
                }
                s.animate();
            }
        };
        // (niri's windows and workspaces changed: a workspace shown, a window minimized or restored; the listener
        // lives as long as the shell, and only wakes a list that still exists)
        if let Some(sh) = crate::shell_atual() {
            sh.estado.ouvir(wake.clone());
        }
        let k = wake.clone();
        window.connect_suspended_notify(move |_| k());
        let k = wake.clone();
        window.connect_visible_notify(move |_| k());
        if let Some(t) = window.surface().and_downcast::<gtk::gdk::Toplevel>() {
            let k = wake.clone();
            t.connect_notify_local(Some("state"), move |_, _| k());
        }
        if let Some(view) = root.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() {
            view.vadjustment().connect_value_changed(move |_| wake());
        }
    }

    /// The motion of the list. One-off motions (the ✓ drawing itself, the × shaking, a chevron turning) follow the
    /// frame clock in one tick for the whole list; the running steps' pulse and shine are slow and endless, so they
    /// step at `AMBIENT_FPS` on one timer for the whole list (each step is a new opacity and translation, nothing
    /// else). Both stop by themselves.
    fn animate(self: &Rc<Self>) {
        if self.instant.get() {
            return;
        }
        if self.ambient.borrow().is_none() {
            // (a fresh start counts as drawn)
            self.last_frame.set((self.frame_time(), glib::monotonic_time()));
        }
        if self.running() && self.ambient.borrow().is_none() {
            let w = Rc::downgrade(self);
            let id = glib::timeout_add_local(Duration::from_millis(1000 / AMBIENT_FPS), move || {
                let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
                let ft = s.frame_time();
                if ft != s.last_frame.get().0 {
                    s.last_frame.set((ft, glib::monotonic_time()));
                }
                // (nothing running, or out of sight: the timer stops; `watch` brings it back)
                if !s.running() {
                    if !s.tick_view().drawn {
                        s.wake_on_next_frame();
                    }
                    if debug() {
                        eprintln!("debug passos: pulso parado ({:?})", s.tick_view());
                    }
                    s.ambient.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                let now = glib::monotonic_time() as f64 / 1e6;
                for r in s.rows.borrow().iter().chain(s.thinking.borrow().as_ref()) {
                    if r.state.get() == StepState::Running {
                        // the mark pulses (0,5 ↔ 1 in 1,2 s) and the shine runs along the line (1,4 s, linear)
                        r.pulse.definir(Canal::Opacidade, 0.75 + 0.25 * (now * std::f64::consts::TAU / 1.2).sin());
                        let w = r.progress.width() as f64;
                        r.shine.definir(Canal::X, (now / 1.4).fract() * (w + SHINE as f64) - SHINE as f64);
                    }
                }
                glib::ControlFlow::Continue
            });
            *self.ambient.borrow_mut() = Some(id);
            if debug() {
                eprintln!("debug passos: pulso ligado");
            }
        }
        if self.tick.borrow().is_some() || !self.moving() {
            return;
        }
        let w = Rc::downgrade(self);
        let last = Cell::new(None::<i64>);
        let Some(root) = self.root.upgrade() else { return };
        let id = root.add_tick_callback(move |_, clock| {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            let t = clock.frame_time();
            let dt = last.get().map(|l| ((t - l) as f64 / 1e6).clamp(0.0, 0.05)).unwrap_or(1.0 / 60.0);
            last.set(Some(t));
            let rows: Vec<Rc<StepRow>> = s.rows.borrow().iter().cloned().chain(s.thinking.borrow().clone()).collect();
            for r in &rows {
                if r.check.get() < 1.0 {
                    r.check.set((r.check.get() + dt / 0.22).min(1.0));
                    r.rail.queue_draw();
                }
                if let Some(from) = r.shake_from.get() {
                    // 2 px, 3 cycles in 240 ms
                    let e = from.elapsed().as_secs_f64();
                    if e >= 0.24 {
                        r.shake_from.set(None);
                        r.anim.definir(Canal::X, 0.0);
                    } else {
                        r.anim.definir(Canal::X, 2.0 * (e / 0.24 * 3.0 * std::f64::consts::TAU).sin() * (1.0 - e / 0.24));
                    }
                }
                if !r.chevron_turn.borrow().parada() {
                    r.chevron_turn.borrow_mut().passo(dt);
                    r.chevron.queue_draw();
                }
            }
            if s.moving() {
                glib::ControlFlow::Continue
            } else {
                s.tick.borrow_mut().take();
                glib::ControlFlow::Break
            }
        });
        *self.tick.borrow_mut() = Some(id);
    }

    fn draw_rail(&self, r: &StepRow, cr: &gtk::cairo::Context, w: i32, h: i32) {
        let rgb = crate::tema::rgb;
        let (cx, cy, rad) = (w as f64 / 2.0, 14.0, 10.0);
        // the line to the next mark
        if !r.last.get() {
            let (a, b, c) = rgb(DIVIDER);
            cr.set_source_rgb(a, b, c);
            cr.set_line_width(1.0);
            cr.move_to(cx.floor() + 0.5, cy + rad + 3.0);
            cr.line_to(cx.floor() + 0.5, h as f64);
            let _ = cr.stroke();
        }
        match r.state.get() {
            // (the running mark is its own drawing, over the rail: it pulses by its opacity)
            StepState::Running => {}
            StepState::Done => {
                let (x, y, z) = rgb(TEAL);
                cr.set_source_rgba(x, y, z, 0.16);
                cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                // the ✓, drawn from 0 to 100 %
                let p = r.check.get().clamp(0.0, 1.0);
                let pts = [(cx - 4.0, cy + 0.2), (cx - 1.2, cy + 3.0), (cx + 4.2, cy - 2.8)];
                let l1 = ((pts[1].0 - pts[0].0).powi(2) + (pts[1].1 - pts[0].1).powi(2)).sqrt();
                let l2 = ((pts[2].0 - pts[1].0).powi(2) + (pts[2].1 - pts[1].1).powi(2)).sqrt();
                let d = p * (l1 + l2);
                cr.set_source_rgb(x, y, z);
                cr.set_line_width(1.6);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                cr.move_to(pts[0].0, pts[0].1);
                if d <= l1 {
                    let f = d / l1;
                    cr.line_to(pts[0].0 + (pts[1].0 - pts[0].0) * f, pts[0].1 + (pts[1].1 - pts[0].1) * f);
                } else {
                    cr.line_to(pts[1].0, pts[1].1);
                    let f = (d - l1) / l2;
                    cr.line_to(pts[1].0 + (pts[2].0 - pts[1].0) * f, pts[1].1 + (pts[2].1 - pts[1].1) * f);
                }
                let _ = cr.stroke();
            }
            StepState::Waiting => {
                let (x, y, z) = rgb(GOLD);
                cr.set_source_rgba(x, y, z, 0.18);
                cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            StepState::Failed => {
                let (x, y, z) = rgb(CORAL);
                cr.set_source_rgba(x, y, z, 0.18);
                cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgb(x, y, z);
                cr.set_line_width(1.6);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                for (a, b) in [((-3.5, -3.5), (3.5, 3.5)), ((3.5, -3.5), (-3.5, 3.5))] {
                    cr.move_to(cx + a.0, cy + a.1);
                    cr.line_to(cx + b.0, cy + b.1);
                }
                let _ = cr.stroke();
            }
        }
    }
}

impl Drop for StepsList {
    fn drop(&mut self) {
        if let Some(c) = self.clock.get_mut().take() {
            c.remove();
        }
        if let Some(c) = self.ambient.get_mut().take() {
            c.remove();
        }
        if let Some((clock, id)) = self.frame_wake.get_mut().take() {
            clock.disconnect(id);
        }
    }
}

/// How often the running steps' pulse and shine step. Each step repaints the window, a fixed cost no matter how
/// little moves (measured in the session with five running steps: about 10 % of a core at 60 per second, about 4 %
/// at 24); slow, endless motion still reads smooth at film rate.
const AMBIENT_FPS: u64 = 24;

/// The width of the progress line's shine.
const SHINE: i32 = 120;

/// The running mark: an accent ring with a dot.
fn draw_running_mark(cr: &gtk::cairo::Context, w: i32, h: i32) {
    let (x, y, z) = crate::tema::rgb(ACCENT);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    cr.set_source_rgb(x, y, z);
    cr.set_line_width(1.5);
    cr.arc(cx, cy, 9.0, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    cr.arc(cx, cy, 3.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

/// The shine: the accent fading in and out along 120 px.
fn draw_shine(cr: &gtk::cairo::Context, w: i32, h: i32) {
    let (x, y, z) = crate::tema::rgb(ACCENT);
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w as f64, 0.0);
    g.add_color_stop_rgba(0.0, x, y, z, 0.0);
    g.add_color_stop_rgba(0.5, x, y, z, 1.0);
    g.add_color_stop_rgba(1.0, x, y, z, 0.0);
    let _ = cr.set_source(&g);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    let _ = cr.fill();
}

fn draw_chevron(r: &StepRow, cr: &gtk::cairo::Context, w: i32, h: i32) {
    let (x, y, z) = crate::tema::rgb(MUTED);
    cr.set_source_rgb(x, y, z);
    cr.set_line_width(1.4);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    cr.translate(w as f64 / 2.0, h as f64 / 2.0);
    cr.rotate(r.chevron_turn.borrow().x * std::f64::consts::FRAC_PI_2);
    cr.move_to(-1.8, -3.6);
    cr.line_to(1.8, 0.0);
    cr.line_to(-1.8, 3.6);
    let _ = cr.stroke();
}

/// The line under an answer: model, how long, tokens ("LFM3 local · 14,2 s · 2.310 tokens").
pub(crate) fn answer_footer(parts: &[String]) -> gtk::Label {
    let l = rotulo(&parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join("  ·  "), "agente-rodape");
    l.set_xalign(0.0);
    l
}

/// A window of steps that move, to measure their cost and record their motion in the development session
/// (`kalango-shell passos-demo [rodando|um|sequencia]`): "rodando" holds five running steps ("um", one), "sequencia" plays a turn like design 53 (searches that count up, pages, an approval, a failure, the answer).
pub(crate) fn demo_window(app: &gtk::Application, mode: &str) {
    let window = gtk::ApplicationWindow::builder().application(app).title("passos-demo").default_width(820).default_height(640).build();
    window.add_css_class("central");
    let col = caixa(Orientation::Vertical, 12, "");
    for m in ["margin-top", "margin-bottom", "margin-start", "margin-end"] {
        col.set_property(m, 24);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&col));
    window.set_child(Some(&scroll));
    col.append(&person_message("Qual é o preço da cadeira Exemplo hoje? Salve o resumo em Documentos."));
    col.append(&agent_header("Pesquisa de preços", &["LFM3 1.7B · local".to_string()], "agora"));
    let steps = StepsList::new(false);
    col.append(&steps.widget());
    window.present();
    let call = |tool: &str, arg: &str, st| {
        let mut c = StepCall::new(tool, arg, st);
        c.args_json = serde_json::json!({ "q": arg }).to_string();
        c
    };
    if mode != "sequencia" {
        for (t, a) in [("web.buscar", "cadeira exemplo preço"), ("web.ler", "loja-a.exemplo"), ("arquivos.listar", "~/Documentos"), ("arquivos.ler", "~/Documentos/notas.md"), ("arquivos.escrever", "~/Documentos/resumo.md")].into_iter().take(if mode == "um" { 1 } else { 5 }) {
            steps.push(call(t, a, StepState::Running));
        }
        return;
    }
    // (time in ms from the start, what happens)
    type Beat = Box<dyn Fn(&Rc<StepsList>, &gtk::Box)>;
    let mut beats: Vec<(u64, Beat)> = Vec::new();
    beats.push((300, Box::new(|s, _| s.show_thinking("Pensando…"))));
    let searches = [("cadeira exemplo preço", "8 resultados"), ("cadeira exemplo promoção", "5 resultados"), ("exemplo frete grátis", "3 resultados"), ("cadeira exemplo loja", "6 resultados"), ("exemplo cupom", "2 resultados"), ("cadeira exemplo avaliação", "9 resultados")];
    let mut t = 1200;
    for (q, r) in searches {
        beats.push((t, Box::new(move |s, _| s.push(call("web.buscar", q, StepState::Running)))));
        beats.push((t + 700, Box::new(move |s, _| s.finish_last(true, r, Some(Duration::from_millis(700))))));
        t += 900;
    }
    for page in ["loja-a.exemplo", "loja-b.exemplo"] {
        beats.push((t, Box::new(move |s, _| s.push(call("web.ler", page, StepState::Running)))));
        beats.push((t + 600, Box::new(move |s, _| s.finish_last(true, "página lida", Some(Duration::from_millis(600))))));
        t += 800;
    }
    beats.push((t, Box::new(move |s, _| s.push(call("arquivos.escrever", "~/Documentos/resumo.md", StepState::Waiting)))));
    t += 1500;
    beats.push((t, Box::new(move |s, _| s.finish_last(true, "resumo salvo", Some(Duration::from_millis(1500))))));
    t += 300;
    beats.push((t, Box::new(move |s, _| s.push(call("web.ler", "loja-c.exemplo", StepState::Running)))));
    t += 900;
    beats.push((t, Box::new(move |s, _| s.finish_last(false, "tempo esgotado", Some(Duration::from_millis(900))))));
    t += 600;
    beats.push((t, Box::new(move |s, _| s.show_thinking("Escrevendo o resumo…"))));
    t += 1800;
    beats.push((t, Box::new(move |s, col| {
        s.finish(Duration::from_millis(14_200), true);
        let a = Animavel::new(&crate::markdown::render("A cadeira Exemplo está em R$ 1.249 na Loja A (frete grátis) e R$ 1.199 na Loja B (+ R$ 60 de frete). A Loja A sai mais em conta."));
        col.append(&a);
        a.entrar((0.0, 8.0), 1.0);
        col.append(&answer_footer(&["LFM3 local".into(), "14,2 s".into(), "2.310 tokens".into()]));
    })));
    for (at, beat) in beats {
        let s = steps.clone();
        let c = col.clone();
        glib::timeout_add_local_once(Duration::from_millis(at), move || beat(&s, &c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, state: StepState) -> StepCall {
        StepCall::new(tool, "x", state)
    }

    #[test]
    fn task_and_member_rows_name_who_answered() {
        assert_eq!(adk_name("relatorio-semana"), "relatorio_semana");
        assert_eq!(adk_name("1abc"), "a_1abc");
        assert_eq!(task_of("buscador_tarefa_2"), Some(("buscador", 2)));
        assert_eq!(task_of("buscador"), None);
        let agents = vec![("buscador".to_string(), "Buscador".to_string()), ("revisor-x".to_string(), "Revisor".to_string())];
        let a = |who: &str, t: &str| (who.to_string(), t.to_string(), None);
        let answers = vec![
            a("buscador_tarefa_1", "| dia | total |\n|---|---|\n| seg | 3 |\n| ter | 4 |"),
            a("revisor_x_tarefa_2", "Um.\n\nDois.\n\nTrês."),
            a("relatorio", "o texto final do próprio agente"),
        ];
        let rows = member_calls(&answers, "relatorio", "tasks", &agents);
        assert_eq!(rows.len(), 2, "the root's own answer has no row");
        let m = rows[0].member.as_ref().unwrap();
        assert!(m.tag.contains('1') && m.name == "Buscador", "{m:?}");
        assert!(rows[0].result.contains('2'), "a table with 2 rows: {}", rows[0].result);
        assert_eq!(rows[1].member.as_ref().unwrap().name, "Revisor");
        assert!(rows[1].result.contains('3'), "3 paragraphs: {}", rows[1].result);
        // members never join one row, even the same agent's turns of a loop (numbered)
        let turns = member_calls(&[a("revisor_x", "a"), a("revisor_x", "b")], "laco", "loop", &agents);
        assert_eq!(group_calls(&turns).len(), 2);
        assert!(turns[1].member.as_ref().unwrap().tag.contains('2'));
        // a team's member: no mark
        let team = member_calls(&[a("buscador", "oi")], "equipe", "sequence", &agents);
        assert_eq!(team[0].member.as_ref().unwrap().tag, "");
        assert_eq!(team[0].result, "oi");
    }

    #[test]
    fn answer_times_follow_the_live_events() {
        let s = Duration::from_secs;
        let arr = vec![("a".to_string(), s(3)), ("b".to_string(), s(7))];
        let ans = vec![("a".to_string(), "x".to_string()), ("b".to_string(), "y".to_string())];
        assert_eq!(answer_times("sequential", &arr, &ans, s(9)), vec![Some(s(3)), Some(s(4))]);
        assert_eq!(answer_times("parallel", &arr, &ans, s(9)), vec![Some(s(3)), Some(s(7))]);
        // the last one's event came after the whole answer: it ended with the request
        assert_eq!(answer_times("tasks", &arr[..1], &ans, s(9)), vec![Some(s(3)), Some(s(6))]);
        assert_eq!(answer_times("tasks", &[], &ans, s(9)), vec![None, Some(s(9))], "an answer with no live event, not the last, has no time");
        let tasks = serde_json::json!([{"agent": "busca"}, {"agent": "resumo"}]);
        let names = vec![("resumo".to_string(), "Resumidor".to_string())];
        let t = next_task_text(&tasks, 1, &names).unwrap();
        assert!(t.contains('2') && t.contains("Resumidor"), "{t}");
        assert!(next_task_text(&tasks, 2, &names).is_none());
    }

    #[test]
    fn a_long_answer_reads_as_its_first_words() {
        assert_eq!(short_text("tabela com 3 linhas", 20), "tabela com 3 linhas");
        assert_eq!(short_text("tabela com 12 linhas e mais", 12), "tabela com…");
        let s = answer_shape("Resumo da semana: as vendas subiram 12 % em relação à semana passada");
        assert!(s.ends_with('…') && s.chars().count() <= 29, "{s}");
    }

    #[test]
    fn the_route_line_names_the_sub_agent_and_its_confidence() {
        let agents = vec![("tradutor".to_string(), "Tradutor".to_string())];
        let l = route_line(&serde_json::json!({"agent": "tradutor", "confidence": 0.917, "millis": 40}), &agents, 0.7).unwrap();
        assert!(l.contains("Tradutor") && l.contains("92 %"), "{l}");
        let own = route_line(&serde_json::json!({"agent": "proprio", "confidence": 0.5, "millis": 40}), &agents, 0.7).unwrap();
        assert!(own.contains("50 %") && !own.contains("proprio"), "{own}");
        assert!(route_line(&Value::Null, &agents, 0.7).is_none());
        // under the threshold the coordinator answers: the line does not say it was handed over
        let unsure = route_line(&serde_json::json!({"agent": "tradutor", "confidence": 0.41}), &agents, 0.7).unwrap();
        assert!(unsure.contains("41 %") && unsure != route_text(Some("Tradutor"), 0.41, true), "{unsure}");
    }

    #[test]
    fn consecutive_calls_of_one_tool_are_one_step() {
        use StepState::*;
        let calls = vec![call("web.buscar", Done), call("web.buscar", Done), call("web.buscar", Running), call("web.ler", Done), call("web.ler", Done), call("web.buscar", Done)];
        let g = group_calls(&calls);
        assert_eq!(g.iter().map(|g| (g.tool.as_str(), g.first, g.len)).collect::<Vec<_>>(), vec![("web.buscar", 0, 3), ("web.ler", 3, 2), ("web.buscar", 5, 1)]);
        assert_eq!(group_state(&calls[0..3]), Running);
        // a call waiting for the person, or one that failed, is a step of its own
        let calls = vec![call("arquivos.mover", Done), call("arquivos.mover", Waiting), call("arquivos.mover", Done)];
        assert_eq!(group_calls(&calls).len(), 3);
        // what a test would have asked, in a row, is one step
        let calls = vec![call("web.buscar", Waiting), call("web.buscar", Waiting), call("web.buscar", Waiting)];
        assert_eq!(group_calls(&calls).len(), 1);
        let calls = vec![call("web.ler", Failed), call("web.ler", Done)];
        assert_eq!(group_calls(&calls).len(), 2);
        // the same failure again and again is one step
        let calls = vec![call("arquivos.listar", Failed), call("arquivos.listar", Failed), call("arquivos.listar", Failed)];
        assert_eq!(group_calls(&calls).len(), 1);
        // …but a failure after good ones joins them and shows
        let calls = vec![call("web.ler", Done), call("web.ler", Failed)];
        assert_eq!(group_calls(&calls).len(), 1);
        assert_eq!(group_state(&calls), Failed);
        assert!(group_calls(&[]).is_empty());
    }

    #[test]
    fn the_pulse_runs_only_while_someone_can_see_a_running_step() {
        let on = TickView { running: true, motion: true, mapped: true, window_shown: true, drawn: true, in_view: Some((40.0, 200.0, 600.0)) };
        assert!(should_tick(on));
        // (no scrolled view around it: in sight)
        assert!(should_tick(TickView { in_view: None, ..on }));
        // nothing running, motion reduced, unmapped (a hidden page or tab), the window hidden/minimized/suspended
        assert!(!should_tick(TickView { running: false, ..on }));
        assert!(!should_tick(TickView { motion: false, ..on }));
        assert!(!should_tick(TickView { mapped: false, ..on }));
        assert!(!should_tick(TickView { window_shown: false, ..on }));
        // the compositor stopped drawing the window (another workspace)
        assert!(!should_tick(TickView { drawn: false, ..on }));
        // scrolled out of view, above or below; a sliver in sight still counts
        assert!(!should_tick(TickView { in_view: Some((-300.0, -10.0, 600.0)), ..on }));
        assert!(!should_tick(TickView { in_view: Some((600.0, 760.0, 600.0)), ..on }));
        assert!(!should_tick(TickView { in_view: Some((-160.0, 0.0, 600.0)), ..on }));
        assert!(should_tick(TickView { in_view: Some((-150.0, 4.0, 600.0)), ..on }));
        assert!(should_tick(TickView { in_view: Some((596.0, 760.0, 600.0)), ..on }));
    }

    #[test]
    fn niri_says_whether_our_window_is_in_sight() {
        let wins = [(Some(7), "Agentes", Some(2), false), (Some(7), "Arquivos", Some(3), true), (Some(9), "Agentes", Some(4), false)];
        // ours (pid 7) on workspace 2, which is on screen
        assert_eq!(shown_on_compositor(wins, &[2, 5], 7, "Agentes"), Some(true));
        // on a workspace out of sight
        assert_eq!(shown_on_compositor(wins, &[4], 7, "Agentes"), Some(false));
        // minimized, even on a workspace in sight
        assert_eq!(shown_on_compositor(wins, &[3], 7, "Arquivos"), Some(false));
        // not listed (another process's window of that name does not count): GTK decides
        assert_eq!(shown_on_compositor(wins, &[4], 7, "Calendário"), None);
        assert_eq!(shown_on_compositor([(Some(9), "Agentes", Some(4), false)], &[4], 7, "Agentes"), None);
    }

    #[test]
    fn steps_read_as_people_say_them() {
        assert_eq!(step_label("web.buscar", 6, StepState::Done), tr("Searched the web"));
        assert_eq!(step_chip("web.buscar", 6, false), Some(trf("{n} searches", &[("n", "6".into())])));
        assert_eq!(step_chip("web.buscar", 6, true).as_deref(), Some("6"));
        assert_eq!(step_chip("web.buscar", 1, false), None);
        assert_eq!(step_label("web.ler", 2, StepState::Done), trf("Read {n} pages", &[("n", "2".into())]));
        assert_eq!(step_label("web.ler", 1, StepState::Done), tr("Read the page"));
        assert_eq!(step_label("arquivos.mover", 1, StepState::Waiting), trf("Wants to {action}", &[("action", tr("move files").into())]));
        assert_eq!(step_label("web.ler", 1, StepState::Failed), trf("Could not {action}", &[("action", tr("read a page").into())]));
        // an unknown tool is its name
        assert_eq!(step_label("meu.conector", 3, StepState::Done), "meu.conector");
        assert!(seconds(Duration::from_millis(4200)).starts_with("4"));
    }
}
