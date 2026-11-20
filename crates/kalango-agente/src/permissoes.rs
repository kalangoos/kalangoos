//! The person's permissions for the agent (task 3.2): free, ask, never — per tool and per column of the agents
//! center (read, write, delete, network, terminal, screen, spend) —, the folders it may act in, its limits (tokens a
//! day, hours) and whether it is paused. Read on every request, so a change applies at once. Stored in
//! `~/.config/kalango/agente-permissoes.json` (only what differs from the defaults).
//!
//! Defaults: reading is free; changing the person's things (files, connections…) asks in the conversation
//! first; system and full-level actions are free here because kalangod already asks on the system approval
//! screen (which an agent cannot imitate), so the person is not asked twice.

use crate::kalangod::Tool;
use crate::motor::INTERNAL;
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perm {
    Free,
    Ask,
    Never,
}

impl Perm {
    pub fn name(self) -> &'static str {
        match self {
            Perm::Free => "livre",
            Perm::Ask => "perguntar",
            Perm::Never => "nunca",
        }
    }

    pub fn parse(s: &str) -> Option<Perm> {
        match s {
            "livre" | "free" => Some(Perm::Free),
            "perguntar" | "ask" => Some(Perm::Ask),
            "nunca" | "never" => Some(Perm::Never),
            _ => None,
        }
    }
}

pub fn default_for(tool: &Tool) -> Perm {
    // kalangod asks before these itself (the levels; for vm.*, what the person chose in VirtualDesk): asking here
    // too would be the same question twice
    if tool.read_only || matches!(tool.level.as_str(), "sistema" | "total") || tool.name.starts_with("vm.") {
        Perm::Free
    } else {
        Perm::Ask
    }
}

/// The columns of the agents center's permissions table: each tool belongs to one. Setting a column sets every
/// tool in it (and the tools that come later).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Read,
    Write,
    Delete,
    Network,
    Terminal,
    Screen,
    Spend,
}

impl Category {
    pub const ALL: [Category; 7] = [Category::Read, Category::Write, Category::Delete, Category::Network, Category::Terminal, Category::Screen, Category::Spend];

    pub fn id(self) -> &'static str {
        match self {
            Category::Read => "ler",
            Category::Write => "escrever",
            Category::Delete => "apagar",
            Category::Network => "rede",
            Category::Terminal => "terminal",
            Category::Screen => "tela",
            Category::Spend => "gastar",
        }
    }

    pub fn parse(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.id() == s)
    }

    /// For a column with no tool yet (seeing the screen, spending money): asks.
    fn default_empty(self) -> Perm {
        Perm::Ask
    }
}

pub fn category(t: &Tool) -> Category {
    if t.name.starts_with("web.") {
        Category::Network
    } else if matches!(t.name.as_str(), "sistema.executar" | "tarefas.executar") {
        Category::Terminal
    } else if t.name.starts_with("tela.") {
        Category::Screen
    } else if t.destructive || t.name == "arquivos.lixeira" {
        Category::Delete
    } else if t.read_only {
        Category::Read
    } else {
        Category::Write
    }
}

/// A folder in "where it can act": on or off, and whether the agent may change things in it or only read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub on: bool,
    pub write: bool,
}

/// Everything the person set for the agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// per tool (from the conversation's "Always", or `kalango-agente permissao`)
    pub tools: BTreeMap<String, Perm>,
    /// per column
    pub categories: BTreeMap<&'static str, Perm>,
    /// None: no list (the agent goes wherever kalangod lets it: the home folder)
    pub folders: Option<Vec<Folder>>,
    pub tokens_per_day: Option<u64>,
    /// "sempre", "dia" (8h–18h) or "noite" (18h–8h)
    pub hours: String,
    pub paused: bool,
    /// "Aumentar só hoje": extra tokens for one day (the start of that day, the tokens)
    pub extra_hoje: Option<(i64, u64)>,
    /// sites the person blocked after one tried to give orders to the agent
    pub sites_bloqueados: Vec<String>,
}

/// An error with a code the shell understands (shown as the failure card of that kind): \u{1}code\u{1}message.
pub fn com_codigo(codigo: &str, msg: impl Into<String>) -> String {
    format!("\u{1}{codigo}\u{1}{}", msg.into())
}

/// (code, message) of an error made by `com_codigo`; plain errors have no code.
pub fn separar(e: &str) -> (Option<&str>, &str) {
    match e.strip_prefix('\u{1}').and_then(|r| r.split_once('\u{1}')) {
        Some((c, m)) => (Some(c), m),
        None => (None, e),
    }
}

fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
        .join("kalango/agente-permissoes.json")
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

impl Settings {
    /// From the file's JSON. The first version of the file was only {tool: permission}.
    pub fn parse(v: &Value) -> Settings {
        let mut s = Settings { hours: "sempre".into(), ..Default::default() };
        let Some(o) = v.as_object() else { return s };
        let tools = if o.contains_key("ferramentas") { v["ferramentas"].as_object() } else { Some(o) };
        for (k, p) in tools.into_iter().flatten() {
            if let Some(p) = p.as_str().and_then(Perm::parse) {
                s.tools.insert(k.clone(), p);
            }
        }
        for (k, p) in v["categorias"].as_object().into_iter().flatten() {
            if let (Some(c), Some(p)) = (Category::parse(k), p.as_str().and_then(Perm::parse)) {
                s.categories.insert(c.id(), p);
            }
        }
        s.folders = v["pastas"].as_array().map(|a| a.iter().filter_map(|f| Some(Folder {
            path: f["caminho"].as_str()?.to_string(), on: f["ativa"].as_bool().unwrap_or(true), write: f["modo"].as_str() != Some("ler"),
        })).collect());
        s.tokens_per_day = v["tokens_dia"].as_u64().filter(|n| *n > 0);
        if let Some(h) = v["horario"].as_str().filter(|h| matches!(*h, "sempre" | "dia" | "noite")) {
            s.hours = h.to_string();
        }
        s.paused = v["pausado"].as_bool().unwrap_or(false);
        s.extra_hoje = v["extra_hoje"]["dia"].as_i64().zip(v["extra_hoje"]["tokens"].as_u64());
        s.sites_bloqueados = v["sites_bloqueados"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        s
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "ferramentas": self.tools.iter().map(|(k, p)| (k.clone(), json!(p.name()))).collect::<serde_json::Map<_, _>>(),
            "categorias": self.categories.iter().map(|(k, p)| (k.to_string(), json!(p.name()))).collect::<serde_json::Map<_, _>>(),
            "horario": self.hours, "pausado": self.paused,
        });
        if let Some(f) = &self.folders {
            v["pastas"] = json!(f.iter().map(|f| json!({"caminho": f.path, "ativa": f.on, "modo": if f.write { "mexer" } else { "ler" }})).collect::<Vec<_>>());
        }
        if let Some(n) = self.tokens_per_day {
            v["tokens_dia"] = json!(n);
        }
        if let Some((d, n)) = self.extra_hoje {
            v["extra_hoje"] = json!({"dia": d, "tokens": n});
        }
        if !self.sites_bloqueados.is_empty() {
            v["sites_bloqueados"] = json!(self.sites_bloqueados);
        }
        v
    }

    /// The permission in force for a tool: the tool's own, else its column's, else the default.
    pub fn get(&self, tool: &Tool) -> Perm {
        self.tools.get(&tool.name).or_else(|| self.categories.get(category(tool).id())).copied().unwrap_or_else(|| default_for(tool))
    }

    /// What a column shows: the most open permission among its tools (never safer than the truth), or the
    /// column's own setting when it has no tool yet.
    /// A system or full-level change left at its default is free here only because kalangod asks the person on the
    /// system approval screen: for the person it asks, and the column shows that.
    pub fn column(&self, tools: &[Tool], c: Category) -> Perm {
        let open = |p: Perm| match p { Perm::Free => 2, Perm::Ask => 1, Perm::Never => 0 };
        let shown = |t: &Tool| self.tools.get(&t.name).or_else(|| self.categories.get(category(t).id())).copied().unwrap_or_else(|| {
            if !t.read_only && matches!(t.level.as_str(), "sistema" | "total") { Perm::Ask } else { default_for(t) }
        });
        tools.iter().filter(|t| !INTERNAL.contains(&t.name.as_str()) && category(t) == c).map(shown).max_by_key(|p| open(*p))
            .unwrap_or_else(|| self.categories.get(c.id()).copied().unwrap_or(c.default_empty()))
    }

    /// Sets a column: its tools lose their own settings, so the column holds for all of them.
    pub fn set_column(&mut self, tools: &[Tool], c: Category, p: Perm) {
        self.categories.insert(c.id(), p);
        self.tools.retain(|name, _| tools.iter().find(|t| &t.name == name).is_none_or(|t| category(t) != c));
    }

    /// "Revogar tudo": every column never, no folder.
    pub fn revoke_all(&mut self, tools: &[Tool]) {
        for c in Category::ALL {
            self.set_column(tools, c, Perm::Never);
        }
        self.tools.clear();
        let mut f = self.shown_folders();
        f.iter_mut().for_each(|f| f.on = false);
        self.folders = Some(f);
    }

    /// The folders as the agents center shows them (with no list: the home folder, whole).
    pub fn shown_folders(&self) -> Vec<Folder> {
        self.folders.clone().unwrap_or_else(|| vec![Folder { path: "~".into(), on: true, write: true }])
    }

    /// Whether a tool may touch the paths in its arguments, by the folder list. Err: why not.
    pub fn folder_check(&self, tool: &Tool, args: &Value) -> Result<(), String> {
        let Some(folders) = &self.folders else { return Ok(()) };
        if !tool.name.starts_with("arquivos.") {
            return Ok(());
        }
        let home = home();
        let expand = |p: &str| -> PathBuf {
            let p = if p == "~" { home.clone() } else if let Some(r) = p.strip_prefix("~/") { home.join(r) } else { PathBuf::from(p) };
            normalize(&p)
        };
        for k in ["caminho", "de", "para"] {
            let Some(raw) = args.get(k).and_then(Value::as_str) else { continue };
            let p = expand(raw);
            let inside: Vec<&Folder> = folders.iter().filter(|f| f.on && p.starts_with(expand(&f.path))).collect();
            if inside.is_empty() {
                return Err(trf("{path} is outside the folders the agent may use", &[("path", raw.to_string())]));
            }
            if !tool.read_only && !inside.iter().any(|f| f.write) {
                return Err(trf("the agent may only read {path}", &[("path", raw.to_string())]));
            }
        }
        Ok(())
    }
}

fn normalize(p: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn load() -> Settings {
    Settings::parse(&std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({})))
}

pub fn save(s: &Settings) -> Result<(), String> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&s.to_json()).unwrap_or_default()).map_err(|e| e.to_string())
}

/// The permission in force for a tool.
pub fn get(tool: &Tool) -> Perm {
    load().get(tool)
}

/// Changes a tool's permission (from the conversation's "Always" or the settings).
pub fn set(tool: &str, perm: Perm) -> Result<(), String> {
    let mut s = load();
    s.tools.insert(tool.to_string(), perm);
    save(&s)
}

/// Every tool with its permission and default (for the settings in the shell).
pub fn report(tools: &[Tool]) -> Value {
    let s = load();
    Value::Array(tools.iter().map(|t| json!({
        "nome": t.name,
        "descricao": t.description.split(" [").next().unwrap_or(""),
        "nivel": t.level,
        "leitura": t.read_only,
        "permissao": s.get(t).name(),
        "padrao": default_for(t).name(),
        "categoria": category(t).id(),
    })).collect())
}

// ------------------------------------------------------------------ when the agent may work

/// Local time (hour, start of today as unix time).
fn local_now() -> (u32, i64) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = now as libc::time_t;
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    let since_midnight = (tm.tm_hour * 3600 + tm.tm_min * 60 + tm.tm_sec) as i64;
    (tm.tm_hour as u32, now - since_midnight)
}

/// Today's tokens (the local model, every use by the agent runtime), from the usage log.
pub fn tokens_today() -> u64 {
    let (_, day) = local_now();
    std::fs::read_to_string(crate::modelo::uso_path()).unwrap_or_default().lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["t"].as_i64().unwrap_or(0) >= day).map(|v| v["tokens"].as_u64().unwrap_or(0)).sum()
}

/// Whether the agent may start working now: not paused, within its hours. Err: why not (for the person).
pub fn may_run(s: &Settings) -> Result<(), String> {
    if s.paused {
        return Err(com_codigo("pausado", tr("the agent is paused in its permissions (Agents › Permissions › Resume agent)")));
    }
    let (h, _) = local_now();
    let ok = match s.hours.as_str() {
        "dia" => (8..18).contains(&h),
        "noite" => !(8..18).contains(&h),
        _ => true,
    };
    if !ok {
        let when = if s.hours == "dia" { tr("from 8 am to 6 pm") } else { tr("from 6 pm to 8 am") };
        return Err(com_codigo("horario", trf("the agent may only work {when} (Agents › Permissions)", &[("when", when.to_string())])));
    }
    Ok(())
}

/// Whether the agent may use the model once more today. Err: why not.
pub fn may_spend_tokens(s: &Settings) -> Result<(), String> {
    let Some(n) = s.tokens_per_day else { return Ok(()) };
    // "Aumentar só hoje" counts only on its day
    let (_, hoje) = local_now();
    let extra = s.extra_hoje.filter(|(d, _)| *d == hoje).map(|(_, x)| x).unwrap_or(0);
    let usados = tokens_today();
    if usados >= n + extra {
        return Err(com_codigo("orcamento", trf("today's token limit ({n}) was reached (Agents › Permissions)", &[("n", (n + extra).to_string())])));
    }
    Ok(())
}

/// The start of today (local), for "Aumentar só hoje".
pub fn hoje() -> i64 {
    local_now().1
}

// ------------------------------------------------------------------ when each column was last used

fn use_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/state")).join("kalango/agente-uso-permissoes.json")
}

/// {"desde": first time recorded, "ultimo": {column: last time used}}
pub fn uses() -> Value {
    std::fs::read_to_string(use_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}))
}

/// The agent used a tool of this column now (the agents center suggests "never" for columns long unused).
pub fn record_use(c: Category) {
    if cfg!(test) {
        return;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut v = uses();
    if v["desde"].as_u64().is_none() {
        v["desde"] = json!(now);
    }
    v["ultimo"][c.id()] = json!(now);
    let p = use_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, v.to_string());
}

/// Everything for the agents center's permissions page.
pub fn profile(tools: &[Tool], protected: &[String]) -> Value {
    let s = load();
    let mut u = uses();
    // the first time the page is shown, counting starts (a column never used is suggested only after a while)
    if u["desde"].as_u64().is_none() && !cfg!(test) {
        record_since();
        u = uses();
    }
    json!({
        "colunas": Category::ALL.iter().map(|c| json!({
            "id": c.id(), "valor": s.column(tools, *c).name(),
            "ferramentas": tools.iter().filter(|t| !INTERNAL.contains(&t.name.as_str()) && category(t) == *c).count(),
            "sistema": tools.iter().any(|t| category(t) == *c && !t.read_only && matches!(t.level.as_str(), "sistema" | "total")),
        })).collect::<Vec<_>>(),
        "pastas": s.shown_folders().iter().map(|f| json!({"caminho": f.path, "ativa": f.on, "modo": if f.write { "mexer" } else { "ler" }})).collect::<Vec<_>>(),
        "protegidas": protected,
        "tokens_dia": s.tokens_per_day, "tokens_hoje": tokens_today(), "horario": s.hours, "pausado": s.paused,
        "uso": u,
    })
}

fn record_since() {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut v = uses();
    v["desde"] = json!(now);
    let p = use_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, v.to_string());
}

/// What the person answered when asked in the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Once,
    Always,
    Deny,
}

impl Answer {
    pub fn parse(s: &str) -> Answer {
        match s {
            "uma-vez" | "once" | "s" | "sim" | "y" | "yes" => Answer::Once,
            "sempre" | "always" | "S" => Answer::Always,
            _ => Answer::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, level: &str, read_only: bool, destructive: bool) -> Tool {
        Tool { name: name.into(), description: String::new(), schema: json!({}), level: level.into(), read_only, destructive }
    }

    fn tools() -> Vec<Tool> {
        vec![
            tool("arquivos.ler", "usuario", true, false), tool("arquivos.mover", "usuario", false, false), tool("arquivos.lixeira", "usuario", false, false),
            tool("pacotes.remover", "sistema", false, true), tool("web.ler", "usuario", true, false), tool("web.buscar", "usuario", true, false), tool("sistema.executar", "total", false, true),
            tool("processos.listar", "observar", true, false),
        ]
    }

    #[test]
    fn virtual_machine_tools_are_asked_by_kalangod_only() {
        assert_eq!(default_for(&tool("vm.executar", "usuario", false, true)), Perm::Free);
        assert_eq!(default_for(&tool("arquivos.mover", "usuario", false, false)), Perm::Ask);
    }

    #[test]
    fn the_first_file_format_still_reads() {
        let s = Settings::parse(&json!({"arquivos.mover": "nunca"}));
        assert_eq!(s.tools.get("arquivos.mover"), Some(&Perm::Never));
        assert_eq!(s.hours, "sempre");
        assert_eq!(Settings::parse(&s.to_json()), s);
    }

    #[test]
    fn columns_group_the_tools_and_show_the_most_open() {
        let t = tools();
        let cat = |n: &str| category(t.iter().find(|x| x.name == n).unwrap());
        assert_eq!(cat("arquivos.lixeira"), Category::Delete);
        assert_eq!(cat("pacotes.remover"), Category::Delete);
        assert_eq!(cat("web.ler"), Category::Network);
        assert_eq!(cat("sistema.executar"), Category::Terminal);
        assert_eq!(cat("processos.listar"), Category::Read);
        let mut s = Settings::default();
        // writing asks by default; "always" on one tool makes the column show free (never safer than the truth)
        assert_eq!(s.column(&t, Category::Write), Perm::Ask);
        // system changes left at the default ask (on kalangod's screen); reading is free
        assert_eq!(s.column(&t, Category::Delete), Perm::Ask);
        assert_eq!(s.column(&t, Category::Terminal), Perm::Ask);
        assert_eq!(s.column(&t, Category::Read), Perm::Free);
        s.tools.insert("arquivos.mover".into(), Perm::Free);
        assert_eq!(s.column(&t, Category::Write), Perm::Free);
        // setting the column clears the tools' own settings in it
        s.set_column(&t, Category::Write, Perm::Never);
        assert!(s.tools.is_empty());
        assert_eq!(s.get(&t[1]), Perm::Never);
        // a column with no tool keeps its own setting
        assert_eq!(s.column(&t, Category::Screen), Perm::Ask);
        s.set_column(&t, Category::Screen, Perm::Never);
        assert_eq!(s.column(&t, Category::Screen), Perm::Never);
        s.revoke_all(&t);
        assert!(t.iter().all(|x| s.get(x) == Perm::Never));
        assert!(s.shown_folders().iter().all(|f| !f.on));
    }

    #[test]
    fn folders_limit_where_files_are_touched() {
        let t = tools();
        let home = home();
        let mut s = Settings::default();
        let inside = json!({"caminho": format!("{}/Downloads/a.txt", home.display())});
        assert!(s.folder_check(&t[0], &inside).is_ok(), "no list: no limit");
        s.folders = Some(vec![Folder { path: "~/Downloads".into(), on: true, write: false }, Folder { path: "~/Imagens".into(), on: false, write: true }]);
        assert!(s.folder_check(&t[0], &inside).is_ok());
        assert!(s.folder_check(&t[1], &json!({"de": "~/Downloads/a.txt", "para": "~/Downloads/b.txt"})).unwrap_err().contains("Downloads"), "read only");
        assert!(s.folder_check(&t[0], &json!({"caminho": "~/Imagens/x.png"})).is_err(), "folder off");
        assert!(s.folder_check(&t[0], &json!({"caminho": "~/Downloads/../.ssh/id"})).is_err(), "no escaping with ..");
        assert!(s.folder_check(&t[4], &json!({"url": "http://x"})).is_ok(), "only file tools");
    }

    #[test]
    fn hours_and_pause() {
        let mut s = Settings { hours: "sempre".into(), ..Default::default() };
        assert!(may_run(&s).is_ok());
        s.paused = true;
        assert!(may_run(&s).is_err());
        s.paused = false;
        let (h, _) = local_now();
        s.hours = if (8..18).contains(&h) { "noite" } else { "dia" }.into();
        assert!(may_run(&s).is_err());
    }
}
