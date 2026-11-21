//! Agents center › Ajustes › Chaves (design "Ajustes · Chaves e cofre", dark and light): the keys kept in the
//! KalangoOS vault, by name only — what kind each is and who uses it is worked out from what refers to it (a cloud
//! provider's `secret`, a custom tool's `{cofre:NAME}`, a connector manifest's `cofre:NAME`, a remote agent's key) —
//! and the form that keeps a new one (`secrets.store`; its value is shown, with the eye, only before saving; never
//! again). Deleting asks inline first. "Chave da API local" shows where the service listens; reaching it from the home
//! network is not in this version (the service only listens on its local socket), so that switch stays off.

use crate::ajustes_modelos::Columns;
use crate::conversa::ui_icon;
use crate::fluxos_estados::{self as estados, action_button};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

// ---------------------------------------------------------------- pure pieces (tested below)

/// The vault's rule for names (kalango-fluxos vault.rs `valid_name`): letters, digits and `._:-`, up to 128.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c))
}

/// Every vault name a text refers to (`cofre:NAME`, `{cofre:NAME}`, `cofre://NAME`).
pub(crate) fn vault_refs(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("cofre:") {
        rest = &rest[i + 6..];
        let r = rest.trim_start_matches("//");
        let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || "._:-".contains(*c)).collect();
        let name = name.trim_end_matches(['.', ':']).to_string();
        if valid_name(&name) && !v.contains(&name) {
            v.push(name);
        }
    }
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UseKind {
    Provider,
    Connector,
    Tool,
    RemoteAgent,
    /// the key other agents of the home network call with (kalango-fluxos a2a_listener.rs `KEY`)
    LocalApi,
}

/// One use of a key: its kind and who (a provider with the agents that call it, a tool, a connector, an agent).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeyUse {
    pub kind: UseKind,
    pub who: String,
}

/// What uses each key, from the providers, the custom tools, the agents and the connector manifests
/// ((name, text) pairs).
pub(crate) fn key_uses(name: &str, providers: &[Value], tools: &[Value], agents: &[Value], connectors: &[(String, String)]) -> Vec<KeyUse> {
    let mut v = Vec::new();
    for p in providers.iter().filter(|p| p["secret"] == name) {
        let id = p["id"].as_str().unwrap_or("");
        let pname = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
        // the agents whose model or fallback is this provider
        let quem: Vec<&str> = agents.iter().filter(|a| {
            let c = &a["agent"]["config"];
            c["model"]["provider"] == id || c["fallback"].as_array().is_some_and(|f| f.iter().any(|m| m["provider"] == id))
        }).filter_map(|a| a["agent"]["name"].as_str()).collect();
        let who = if quem.is_empty() { trf("{provider} (no agent chose it)", &[("provider", pname.to_string())]) } else { format!("{pname}: {}", quem.join(", ")) };
        v.push(KeyUse { kind: UseKind::Provider, who });
    }
    for (c, text) in connectors {
        if vault_refs(text).iter().any(|r| r == name) {
            v.push(KeyUse { kind: UseKind::Connector, who: c.clone() });
        }
    }
    for t in tools {
        if vault_refs(&t["config"].to_string()).iter().any(|r| r == name) {
            v.push(KeyUse { kind: UseKind::Tool, who: t["name"].as_str().unwrap_or("").to_string() });
        }
    }
    for a in agents.iter().filter(|a| a["agent"]["kind"] == "remote" && a["agent"]["config"]["key_secret"] == name) {
        v.push(KeyUse { kind: UseKind::RemoteAgent, who: a["agent"]["name"].as_str().unwrap_or("").to_string() });
    }
    if name == crate::ajustes_rede::KEY_NAME {
        v.push(KeyUse { kind: UseKind::LocalApi, who: tr("A2A of the home network").to_string() });
    }
    v.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.who.cmp(&b.who)));
    v
}

/// Whether an agent calls a provider (its model or one of its fallbacks).
pub(crate) fn agent_reaches(agent: &Value, provider: &str) -> bool {
    let c = &agent["config"];
    c["model"]["provider"] == provider || c["fallback"].as_array().is_some_and(|f| f.iter().any(|m| m["provider"] == provider))
}

/// An assistant given (or refused) the providers of a key: given, it may use the cloud and gets the first provider as
/// a fallback when it reaches none of them; refused, its model and fallbacks on those providers go (its model back to
/// automatic).
pub(crate) fn with_access(agent: &Value, providers: &[String], model_of: &dyn Fn(&str) -> String, give: bool) -> Value {
    let mut a = agent.clone();
    let c = &mut a["config"];
    if give {
        c["allow_cloud"] = json!(true);
        if !providers.iter().any(|p| agent_reaches(agent, p)) {
            if let Some(p) = providers.first() {
                let mut f = c["fallback"].as_array().cloned().unwrap_or_default();
                f.push(json!({"provider": p, "model": model_of(p)}));
                c["fallback"] = json!(f);
            }
        }
    } else {
        if providers.iter().any(|p| c["model"]["provider"] == p.as_str()) {
            c["model"] = Value::Null;
        }
        if let Some(f) = c["fallback"].as_array_mut() {
            f.retain(|m| !providers.iter().any(|p| m["provider"] == p.as_str()));
        }
    }
    a
}

fn kind_label(k: Option<UseKind>) -> (&'static str, &'static str) {
    match k {
        Some(UseKind::Provider) => (tr("provider"), "lilas"),
        Some(UseKind::Connector) => (tr("connector"), "azul"),
        Some(UseKind::Tool) => (tr("tool"), "ouro"),
        Some(UseKind::RemoteAgent) => (tr("remote agent"), "teal"),
        Some(UseKind::LocalApi) => (tr("local API"), "teal"),
        None => (tr("unused"), ""),
    }
}

/// The filters over the table: (id, name).
const FILTERS: [(&str, &str); 5] = [
    ("todas", kalango_i18n::n_("All of them")),
    ("provedores", kalango_i18n::n_("Providers")),
    ("conectores", kalango_i18n::n_("Connectors")),
    ("ferramentas", kalango_i18n::n_("Tools")),
    ("sem-uso", kalango_i18n::n_("Unused")),
];

pub(crate) fn passes(filter: &str, uses: &[KeyUse]) -> bool {
    match filter {
        "provedores" => uses.iter().any(|u| u.kind == UseKind::Provider),
        "conectores" => uses.iter().any(|u| u.kind == UseKind::Connector),
        "ferramentas" => uses.iter().any(|u| u.kind == UseKind::Tool),
        "sem-uso" => uses.is_empty(),
        _ => true,
    }
}

/// A minimal request to a provider with this token (the models list of an OpenAI-compatible API), as a
/// kalango-fluxos HTTP tool for `tools.test`.
pub(crate) fn provider_probe(base_url: &str, token: &str) -> Value {
    json!({
        "name": "teste_de_chave",
        "description": "Ajustes › Chaves: testa a chave antes de guardar",
        "method": "GET",
        "endpoint": format!("{}/models", base_url.trim().trim_end_matches('/')),
        "headers": {"Authorization": format!("Bearer {token}")},
    })
}

/// "Último uso": "há 12 min", "há 2 h", "hoje 08:00", "ontem", "há 9 dias", a date, or "nunca" (times in seconds).
pub(crate) fn last_use_text(now: &glib::DateTime, then_s: Option<i64>) -> String {
    let Some(then_s) = then_s.filter(|t| *t > 0) else { return tr("never").to_string() };
    let Ok(then) = glib::DateTime::from_unix_local(then_s) else { return tr("never").to_string() };
    let ago = (now.to_unix() - then_s).max(0);
    let day = |d: &glib::DateTime| (d.year(), d.day_of_year());
    let yesterday = now.add_days(-1).map(|d| day(&d)).ok();
    if ago < 3600 {
        trf("{n} min ago", &[("n", (ago / 60).max(1).to_string())])
    } else if ago < 3 * 3600 {
        trf("{n} h ago", &[("n", (ago / 3600).to_string())])
    } else if day(&then) == day(now) {
        trf("today {time}", &[("time", then.format("%H:%M").map(|s| s.to_string()).unwrap_or_default())])
    } else if Some(day(&then)) == yesterday {
        tr("yesterday").to_string()
    } else if ago < 31 * 86400 {
        trf("{n} days ago", &[("n", ((ago + 43_200) / 86400).max(2).to_string())])
    } else {
        then.format("%d/%m/%Y").map(|s| s.to_string()).unwrap_or_default()
    }
}

/// The line under a key's name when something needs the person: a failure on its last use (coral), an expiry within
/// 14 days or past (gold / coral; the form says "avisamos 14 dias antes"). None: the dots.
pub(crate) fn key_note(now_s: i64, expires_s: Option<i64>, failed: bool) -> Option<(String, &'static str)> {
    if failed {
        return Some((tr("failed on its last use").to_string(), "erro"));
    }
    let e = expires_s?;
    if e <= now_s {
        return Some((tr("expired").to_string(), "erro"));
    }
    let days = (e - now_s + 86_399) / 86_400;
    (days <= 14).then(|| (if days == 1 { tr("expires tomorrow").to_string() } else { trf("expires in {n} days", &[("n", days.to_string())]) }, "atencao"))
}

/// A date typed as "31/03/2027" (or "31/3/27"), as the end of that day on this computer's clock, in unix seconds.
/// Some(None): empty (no expiry). None: not a date.
pub(crate) fn parse_expiry(text: &str) -> Option<Option<i64>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    // (any separator: "31/03/2027", "31-03-2027", "31.03.27")
    let p: Vec<&str> = t.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).collect();
    let [d, m, y] = p.as_slice() else { return None };
    let (d, m, mut y) = (d.parse::<i32>().ok()?, m.parse::<i32>().ok()?, y.parse::<i32>().ok()?);
    if y < 100 {
        y += 2000;
    }
    let dt = glib::DateTime::from_local(y, m, d, 23, 59, 59.0).ok()?;
    // (glib accepts 31/02 as an error, so a date that exists is what comes back)
    (dt.day_of_month() == d && dt.month() == m).then_some(Some(dt.to_unix()))
}

/// An expiry as typed back ("31/03/2027").
pub(crate) fn expiry_field(expires_s: i64) -> String {
    glib::DateTime::from_unix_local(expires_s).ok().and_then(|d| d.format("%d/%m/%Y").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// The socket path as shown ("unix:~/…" for the person's own folders).
pub(crate) fn socket_shown(path: &str, home: &str) -> String {
    let p = if !home.is_empty() && path.starts_with(home) { path.replacen(home, "~", 1) } else { path.to_string() };
    format!("unix:{p}")
}

// ---------------------------------------------------------------- the tab

#[derive(Default)]
struct Data {
    secrets: Vec<String>,
    /// `secrets.details` by name: last use, expiry, last failure (never the value)
    details: BTreeMap<String, Value>,
    providers: Vec<Value>,
    tools: Vec<Value>,
    agents: Vec<Value>,
    connectors: Vec<(String, String)>,
    /// (provider, model) of the routing order: the model an agent given a provider calls
    routing_models: Vec<(String, String)>,
}

pub struct KeysTab {
    pub body: gtk::Box,
    pub cols: Rc<Columns>,
    screen: estados::Screen,
    filtros: Vec<(&'static str, gtk::ToggleButton)>,
    filtro: RefCell<&'static str>,
    tabela: gtk::Box,
    linhas: gtk::Box,
    vazio_filtro: gtk::Label,
    // the form
    f_nome: gtk::Entry,
    f_valor: gtk::Entry,
    f_olho: gtk::ToggleButton,
    f_colado: gtk::Label,
    f_validade: gtk::Entry,
    f_quem: gtk::Box,
    f_quem_vazio: gtk::Label,
    f_testar: gtk::Switch,
    f_testar_sub: gtk::Label,
    f_erro: gtk::Label,
    f_ok: gtk::Label,
    f_guardar: gtk::Button,
    escolhidos: RefCell<Vec<String>>,
    /// agents to give (true) or take (false) the chosen providers' access when the key is kept
    agent_access: RefCell<BTreeMap<String, bool>>,
    colado: Cell<bool>,
    data: RefCell<Data>,
    loaded: Cell<bool>,
    confirming: RefCell<Option<String>>,
    home_net: Rc<crate::ajustes_rede::HomeNetwork>,
}

impl KeysTab {
    pub fn new(side_width: i32) -> Rc<KeysTab> {
        let col = caixa(Orientation::Vertical, 0, "");
        col.set_margin_bottom(16);
        let topo = caixa(Orientation::Horizontal, 16, "");
        topo.set_margin_top(16);
        let tc = caixa(Orientation::Vertical, 4, "");
        tc.set_hexpand(true);
        let th = caixa(Orientation::Horizontal, 8, "");
        th.append(&ui_icon("cadeado", "#8ED2C3", 14));
        th.append(&rotulo(tr("Kept in the system vault"), "fx-opcao-titulo"));
        let ch = rotulo(tr("protected"), "fx-chip");
        ch.add_css_class("teal");
        th.append(&ch);
        tc.append(&th);
        let ts = rotulo(tr("Agents and flows use a key by its name; the value never shows on screen, in the logs or in the database."), "central-sub");
        ts.set_xalign(0.0);
        ts.set_wrap(true);
        ts.set_max_width_chars(1);
        tc.append(&ts);
        topo.append(&tc);
        let guardar_b = gtk::Button::new();
        guardar_b.add_css_class("rotinas-nova");
        let gc = caixa(Orientation::Horizontal, 6, "");
        gc.append(&ui_icon("mais", "#1C1D24", 14));
        gc.append(&rotulo(tr("Keep a key"), ""));
        guardar_b.set_child(Some(&gc));
        guardar_b.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes guardar chave", guardar_b.upcast_ref());
        topo.append(&guardar_b);
        col.append(&topo);
        // the filters
        let seg = caixa(Orientation::Horizontal, 0, "uso-segmentos");
        seg.set_halign(Align::Start);
        seg.set_margin_top(16);
        let mut filtros: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in FILTERS {
            let b = gtk::ToggleButton::with_label(tr(nome));
            b.add_css_class("uso-segmento");
            if let Some((_, f)) = filtros.first() {
                b.set_group(Some(f));
            }
            crate::central::posicao_debug_sempre(&format!("ajustes chaves filtro {id}"), b.upcast_ref());
            seg.append(&b);
            filtros.push((id, b));
        }
        col.append(&seg);
        // the table
        let tabela = caixa(Orientation::Vertical, 0, "fx-tabela");
        tabela.set_margin_top(14);
        let cab = caixa(Orientation::Horizontal, 12, "fx-tabela-cab");
        for (t, w) in [(tr("Name"), 220), (tr("Kind"), 130), (tr("Used by"), -1), (tr("Last use"), 110)] {
            let l = rotulo(t, "fx-tabela-col");
            l.set_xalign(0.0);
            if w > 0 {
                l.set_size_request(w, -1);
            } else {
                l.set_hexpand(true);
            }
            cab.append(&l);
        }
        let k = caixa(Orientation::Horizontal, 0, "");
        k.set_size_request(28, -1);
        cab.append(&k);
        tabela.append(&cab);
        let linhas = caixa(Orientation::Vertical, 0, "");
        tabela.append(&linhas);
        let vazio_filtro = rotulo(tr("No key of this kind."), "central-agente-sub");
        vazio_filtro.set_margin_top(16);
        vazio_filtro.set_margin_bottom(16);
        vazio_filtro.set_visible(false);
        tabela.append(&vazio_filtro);
        col.append(&tabela);
        let abaixo = caixa(Orientation::Vertical, 0, "");
        col.append(&abaixo);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&col));

        // the right column: keep a key, then the local API
        let cols = Columns::new(side_width, &abaixo, &col);
        let lado = &cols.content;
        let h = caixa(Orientation::Horizontal, 8, "");
        h.append(&ui_icon("cadeado", "#A5A1E2", 14));
        h.append(&rotulo(tr("KEEP A KEY"), "central-painel-agente"));
        lado.append(&h);
        let rot = |t: &str| {
            let l = rotulo(t, "fx-campo-rotulo");
            l.set_xalign(0.0);
            l.set_margin_top(14);
            l.set_margin_bottom(6);
            l
        };
        lado.append(&rot(tr("Name")));
        let f_nome = gtk::Entry::new();
        f_nome.add_css_class("fx-campo");
        f_nome.add_css_class("mono");
        f_nome.set_placeholder_text(Some("openai.token"));
        crate::central::posicao_debug_sempre("ajustes chave nome", f_nome.upcast_ref());
        lado.append(&f_nome);
        lado.append(&rot(tr("Value")));
        let vl = caixa(Orientation::Horizontal, 0, "fx-campo-com-olho");
        let f_valor = gtk::Entry::new();
        f_valor.add_css_class("fx-campo");
        f_valor.add_css_class("mono");
        f_valor.add_css_class("sem-borda");
        f_valor.set_visibility(false);
        f_valor.set_input_purpose(gtk::InputPurpose::Password);
        f_valor.set_hexpand(true);
        f_valor.set_placeholder_text(Some("sk-…"));
        crate::central::posicao_debug_sempre("ajustes chave valor", f_valor.upcast_ref());
        vl.append(&f_valor);
        let f_olho = gtk::ToggleButton::new();
        f_olho.add_css_class("fx-olho");
        f_olho.set_child(Some(&ui_icon("olho", "#8E8F99", 14)));
        f_olho.set_tooltip_text(Some(tr("Show what was typed (only before saving)")));
        f_olho.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes chave olho", f_olho.upcast_ref());
        vl.append(&f_olho);
        lado.append(&vl);
        let f_colado = rotulo(tr("Pasted from the clipboard: it is cleared after saving."), "central-agente-sub");
        f_colado.set_xalign(0.0);
        f_colado.set_wrap(true);
        f_colado.set_margin_top(6);
        f_colado.set_visible(false);
        lado.append(&f_colado);
        // "Validade (opcional)": from the provider's page; the list warns 14 days before
        lado.append(&rot(tr("Expiry (optional)")));
        let vr = caixa(Orientation::Horizontal, 12, "");
        let vc = caixa(Orientation::Horizontal, 6, "fx-campo-com-olho");
        vc.set_size_request(186, -1);
        let vi = ui_icon("relogio", "#8E8F99", 14);
        vi.set_margin_start(10);
        vi.set_valign(Align::Center);
        vc.append(&vi);
        let f_validade = gtk::Entry::new();
        f_validade.add_css_class("fx-campo");
        f_validade.add_css_class("mono");
        f_validade.add_css_class("sem-borda");
        f_validade.set_hexpand(true);
        f_validade.set_placeholder_text(Some(tr("dd/mm/yyyy")));
        crate::central::posicao_debug_sempre("ajustes chave validade", f_validade.upcast_ref());
        vc.append(&f_validade);
        vc.set_valign(Align::Start);
        vr.append(&vc);
        let vn = rotulo(tr("the provider's page says when the key expires; we warn 14 days before"), "central-agente-sub");
        vn.set_xalign(0.0);
        vn.set_wrap(true);
        vn.set_max_width_chars(1);
        vn.set_hexpand(true);
        vn.set_valign(Align::Center);
        vr.append(&vn);
        lado.append(&vr);
        lado.append(&rot(tr("Who may use it")));
        let f_quem = caixa(Orientation::Horizontal, 6, "");
        lado.append(&f_quem);
        let f_quem_vazio = rotulo(tr("No cloud provider yet: add one in Models and providers, or keep the key for a tool or connector."), "central-agente-sub");
        f_quem_vazio.set_xalign(0.0);
        f_quem_vazio.set_wrap(true);
        f_quem_vazio.set_max_width_chars(1);
        lado.append(&f_quem_vazio);
        let tr_l = caixa(Orientation::Horizontal, 12, "");
        tr_l.set_margin_top(16);
        let tcol = caixa(Orientation::Vertical, 2, "");
        tcol.set_hexpand(true);
        let tt = rotulo(tr("Test before saving"), "fx-opcao-titulo");
        tt.set_xalign(0.0);
        tcol.append(&tt);
        let f_testar_sub = rotulo(tr("makes a minimal request to the provider"), "central-agente-sub");
        f_testar_sub.set_xalign(0.0);
        f_testar_sub.set_wrap(true);
        f_testar_sub.set_max_width_chars(1);
        tcol.append(&f_testar_sub);
        tr_l.append(&tcol);
        let f_testar = gtk::Switch::new();
        f_testar.add_css_class("central-switch");
        f_testar.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes chave testar", f_testar.upcast_ref());
        tr_l.append(&f_testar);
        lado.append(&tr_l);
        let f_erro = rotulo("", "rotinas-erro");
        f_erro.set_xalign(0.0);
        f_erro.set_wrap(true);
        f_erro.set_max_width_chars(1);
        f_erro.set_margin_top(10);
        f_erro.set_visible(false);
        lado.append(&f_erro);
        let f_ok = rotulo("", "fx-ok");
        f_ok.set_xalign(0.0);
        f_ok.set_wrap(true);
        f_ok.set_max_width_chars(1);
        f_ok.set_margin_top(10);
        f_ok.set_visible(false);
        lado.append(&f_ok);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_homogeneous(true);
        bs.set_margin_top(14);
        let f_cancelar = action_button(tr("Cancel"), "", "");
        f_cancelar.add_css_class("alto");
        let f_guardar = action_button(tr("Keep in the vault"), "cadeado", "principal");
        f_guardar.add_css_class("alto");
        crate::central::posicao_debug_sempre("ajustes chave guardar", f_guardar.upcast_ref());
        bs.append(&f_cancelar);
        bs.append(&f_guardar);
        lado.append(&bs);

        // the local API
        let ah = caixa(Orientation::Horizontal, 8, "");
        ah.set_margin_top(28);
        ah.append(&ui_icon("rede-cabo", "#A5A1E2", 14));
        ah.append(&rotulo(tr("LOCAL API KEY"), "central-painel-agente"));
        lado.append(&ah);
        let api = caixa(Orientation::Vertical, 8, "fx-api");
        api.set_margin_top(14);
        let at = rotulo(tr("For apps of this computer and other agents (A2A) to call your agents. It only listens on the local socket; for the home network, turn it on below."), "central-agente-sub");
        at.set_xalign(0.0);
        at.set_wrap(true);
        at.set_max_width_chars(1);
        api.append(&at);
        let el = caixa(Orientation::Horizontal, 12, "");
        el.set_margin_top(4);
        el.append(&rotulo(tr("Address"), "central-agente-sub"));
        let home = glib::home_dir().display().to_string();
        let end = rotulo(&socket_shown(&crate::fluxos::socket_path().display().to_string(), &home), "fx-mono");
        end.set_hexpand(true);
        end.set_xalign(1.0);
        end.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        end.set_max_width_chars(1);
        end.set_selectable(true);
        el.append(&end);
        api.append(&el);
        // "Aceitar da rede de casa" (ajustes_rede.rs)
        let home_net = crate::ajustes_rede::HomeNetwork::new();
        api.append(&home_net.body);
        lado.append(&api);

        let body = caixa(Orientation::Vertical, 0, "");
        body.set_vexpand(true);
        let tab_weak: Rc<RefCell<Option<std::rc::Weak<KeysTab>>>> = Rc::default();
        let tw = tab_weak.clone();
        let screen = estados::Screen::new(&rolagem, tr("Reading the vault's names…"), 4, move || {
            if let Some(t) = tw.borrow().as_ref().and_then(|w| w.upgrade()) {
                t.reload();
            }
        });
        body.append(&screen.stack);

        let t = Rc::new(KeysTab {
            body, cols, screen, filtros, filtro: RefCell::new("todas"), tabela, linhas, vazio_filtro,
            f_nome, f_valor, f_olho, f_colado, f_validade, f_quem, f_quem_vazio, f_testar, f_testar_sub, f_erro, f_ok, f_guardar: f_guardar.clone(),
            escolhidos: RefCell::default(), agent_access: RefCell::default(), colado: Cell::new(false), data: RefCell::default(), loaded: Cell::new(false), confirming: RefCell::default(), home_net,
        });
        *tab_weak.borrow_mut() = Some(Rc::downgrade(&t));
        t.filtros[0].1.set_active(true);
        for (id, b) in &t.filtros {
            let (w, id) = (Rc::downgrade(&t), *id);
            b.connect_toggled(move |b| {
                if let Some(t) = w.upgrade().filter(|_| b.is_active()) {
                    *t.filtro.borrow_mut() = id;
                    t.render();
                }
            });
        }
        let fv = t.f_valor.clone();
        t.f_olho.connect_toggled(move |o| fv.set_visibility(o.is_active()));
        let w = Rc::downgrade(&t);
        // (the entry's text inside it is what pastes)
        if let Some(texto) = t.f_valor.delegate().and_downcast::<gtk::Text>() {
            texto.connect_paste_clipboard(move |_| {
                if let Some(t) = w.upgrade() {
                    t.colado.set(true);
                    t.f_colado.set_visible(true);
                }
            });
        }
        // a name already in the vault: its providers come chosen, and the chips show who uses it
        let w = Rc::downgrade(&t);
        t.f_nome.connect_changed(move |e| {
            let Some(t) = w.upgrade() else { return };
            let nome = e.text().trim().to_string();
            let ps: Vec<String> = t.data.borrow().providers.iter().filter(|p| !nome.is_empty() && p["secret"] == nome.as_str()).filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
            *t.escolhidos.borrow_mut() = ps;
            t.agent_access.borrow_mut().clear();
            // (its expiry too, to keep or change)
            let validade = t.data.borrow().details.get(&nome).and_then(|d| d["expires_s"].as_i64());
            t.f_validade.set_text(&validade.map(expiry_field).unwrap_or_default());
            t.render_who();
        });
        let w = Rc::downgrade(&t);
        t.f_valor.connect_changed(move |e| {
            if let Some(t) = w.upgrade().filter(|_| e.text().is_empty()) {
                t.colado.set(false);
                t.f_colado.set_visible(false);
            }
        });
        let w = Rc::downgrade(&t);
        guardar_b.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.clear_form();
                t.f_nome.grab_focus();
            }
        });
        let w = Rc::downgrade(&t);
        f_cancelar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.clear_form();
            }
        });
        let w = Rc::downgrade(&t);
        f_guardar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.keep();
            }
        });
        t
    }

    fn clear_form(self: &Rc<Self>) {
        self.f_validade.set_text("");
        self.f_nome.set_text("");
        self.f_valor.set_text("");
        self.f_olho.set_active(false);
        self.f_erro.set_visible(false);
        self.colado.set(false);
        self.f_colado.set_visible(false);
        self.escolhidos.borrow_mut().clear();
        self.agent_access.borrow_mut().clear();
        self.f_testar.set_active(false);
        self.render_who();
    }

    fn fail(&self, why: &str) {
        self.f_ok.set_visible(false);
        self.f_erro.set_text(why);
        self.f_erro.set_visible(true);
        self.f_guardar.set_sensitive(true);
    }

    /// "Guardar no cofre": checks, tests when asked, keeps it and points the chosen providers at it.
    fn keep(self: &Rc<Self>) {
        let nome = self.f_nome.text().trim().to_string();
        let valor = self.f_valor.text().to_string();
        if !valid_name(&nome) {
            self.fail(tr("The name takes letters, digits and . _ : - (for example openai.token)."));
            return;
        }
        if valor.trim().is_empty() {
            self.fail(tr("Paste or type the key's value."));
            return;
        }
        let Some(validade) = parse_expiry(&self.f_validade.text()) else {
            self.fail(tr("The expiry is a date like 31/03/2027 (or leave it empty)."));
            return;
        };
        let tinha_validade = self.data.borrow().details.get(&nome).is_some_and(|d| d["expires_s"].is_i64());
        self.f_erro.set_visible(false);
        self.f_guardar.set_sensitive(false);
        let escolhidos = self.escolhidos.borrow().clone();
        let teste = self.f_testar.is_active() && self.f_testar.is_sensitive();
        let base = escolhidos.first().and_then(|id| self.data.borrow().providers.iter().find(|p| p["id"] == id.as_str()).and_then(|p| p["base_url"].as_str().map(str::to_string)));
        let w = Rc::downgrade(self);
        let guardar = move || {
            let Some(t) = w.upgrade() else { return };
            let w = Rc::downgrade(&t);
            let (nome2, escolhidos2) = (nome.clone(), escolhidos.clone());
            let mut pedido = json!({"name": nome, "value": valor});
            if let Some(v) = validade {
                pedido["expires_s"] = json!(v);
            } else if tinha_validade {
                // (keeping it again without a date clears the old one)
                crate::fluxos::call("secrets.set_expiry", json!({"name": nome, "expires_s": null}), |_| {});
            }
            crate::fluxos::call("secrets.store", pedido, move |r| {
                let Some(t) = w.upgrade() else { return };
                if let Err(e) = r {
                    t.fail(&trf("Not kept: {why}", &[("why", e)]));
                    return;
                }
                if t.colado.get() {
                    // the value leaves the clipboard too
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text("");
                    }
                }
                // the agents given or refused the chosen providers
                let access = t.agent_access.borrow().clone();
                let routing_models: Vec<(String, String)> = t.data.borrow().routing_models.clone();
                let model_of = move |p: &str| routing_models.iter().find(|(id, _)| id == p).map(|(_, m)| m.clone()).unwrap_or_default();
                for (id, give) in &access {
                    let Some(a) = t.data.borrow().agents.iter().find(|a| a["agent"]["id"] == id.as_str()).map(|a| a["agent"].clone()) else { continue };
                    let novo = with_access(&a, &escolhidos2, &model_of, *give);
                    if novo != a {
                        crate::fluxos::call("agents.save", json!({"agent": novo}), |r| {
                            if let Err(e) = r {
                                eprintln!("ajustes: agente não mudou: {e}");
                            }
                        });
                    }
                }
                let mut ps = t.data.borrow().providers.clone();
                let mudou = !escolhidos2.is_empty();
                for p in ps.iter_mut().filter(|p| escolhidos2.iter().any(|e| p["id"] == e.as_str())) {
                    p["secret"] = json!(nome2);
                }
                let feito = {
                    let w = Rc::downgrade(&t);
                    let nome3 = nome2.clone();
                    move || {
                        if let Some(t) = w.upgrade() {
                            t.clear_form();
                            t.f_ok.set_text(&trf("{name} is in the vault.", &[("name", nome3)]));
                            t.f_ok.set_visible(true);
                            t.f_guardar.set_sensitive(true);
                            t.reload();
                        }
                    }
                };
                if mudou {
                    let w = Rc::downgrade(&t);
                    crate::fluxos::call("models.set_providers", json!({"providers": ps}), move |r| {
                        if let (Some(t), Err(e)) = (w.upgrade(), &r) {
                            t.fail(&trf("Kept, but the provider was not changed: {why}", &[("why", e.clone())]));
                            t.reload();
                            return;
                        }
                        feito();
                    });
                } else {
                    feito();
                }
            });
        };
        match (teste, base) {
            (true, Some(base)) => {
                let probe = provider_probe(&base, &self.f_valor.text());
                let w = Rc::downgrade(self);
                crate::fluxos::call("tools.test", json!({"tool": probe, "args": {}}), move |r| {
                    let Some(t) = w.upgrade() else { return };
                    let status = r.as_ref().ok().and_then(|x| x["status"].as_u64());
                    match (r, status) {
                        (Ok(_), Some(s)) if (200..300).contains(&s) => guardar(),
                        (Ok(_), Some(s)) => t.fail(&trf("The provider refused this key (HTTP {status}); nothing was kept.", &[("status", s.to_string())])),
                        (Ok(_), None) => t.fail(tr("The provider did not answer; nothing was kept.")),
                        (Err(e), _) => t.fail(&trf("The test did not run: {why}; nothing was kept.", &[("why", e)])),
                    }
                });
            }
            _ => guardar(),
        }
    }

    pub fn reload(self: &Rc<Self>) {
        self.home_net.reload();
        if !self.loaded.get() {
            self.screen.show_loading();
            self.cols.content.set_visible(false);
        }
        let connectors = connector_texts();
        let w = Rc::downgrade(self);
        estados::call_all(vec![("secrets.list", json!({})), ("models.providers", json!({})), ("tools.list", json!({})), ("agents.list", json!({})), ("secrets.details", json!({}))], move |r| {
            let Some(t) = w.upgrade() else { return };
            if let Some(Err(e)) = r.iter().find(|x| x.is_err()) {
                if estados::is_offline_error(e) {
                    t.loaded.set(false);
                    t.screen.show_offline();
                    t.cols.content.set_visible(false);
                } else {
                    // (read again from the start next time: the loading state shows)
                    t.loaded.set(false);
                    let (c, _) = estados::notice(estados::Tone::Error, "alerta", tr("The vault could not be read"), "", Some(e));
                    t.screen.show_error(&c);
                    t.cols.content.set_visible(false);
                }
                return;
            }
            let v: Vec<Value> = r.into_iter().map(|x| x.unwrap_or_default()).collect();
            {
                let mut d = t.data.borrow_mut();
                d.secrets = v[0].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
                d.providers = v[1]["providers"].as_array().cloned().unwrap_or_default();
                d.routing_models = v[1]["routing"]["order"].as_array().into_iter().flatten().filter_map(|m| Some((m["provider"].as_str()?.to_string(), m["model"].as_str()?.to_string()))).collect();
                d.tools = v[2]["custom"].as_array().cloned().unwrap_or_default();
                d.agents = v[3].as_array().cloned().unwrap_or_default();
                d.connectors = connectors;
                d.details = v[4].as_array().into_iter().flatten().filter_map(|x| Some((x["name"].as_str()?.to_string(), x.clone()))).collect();
            }
            t.loaded.set(true);
            t.render();
            t.render_who();
            t.screen.show_content();
            t.cols.content.set_visible(true);
        });
    }

    /// "Quem pode usar": the cloud providers this key is for (chips that toggle), the agents that reach them (chips that
    /// toggle; "+ agente" gives one access) and, read-only, what else uses a key of this name (tools, connectors,
    /// remote agents, the local API).
    fn render_who(self: &Rc<Self>) {
        while let Some(c) = self.f_quem.first_child() {
            self.f_quem.remove(&c);
        }
        let d = self.data.borrow();
        let nuvem: Vec<&Value> = d.providers.iter().filter(|p| p["kind"] != "local").collect();
        let nome = self.f_nome.text().trim().to_string();
        let outros: Vec<KeyUse> = if nome.is_empty() { Vec::new() } else { key_uses(&nome, &d.providers, &d.tools, &d.agents, &d.connectors).into_iter().filter(|u| u.kind != UseKind::Provider).collect() };
        self.f_quem_vazio.set_visible(nuvem.is_empty() && outros.is_empty());
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(6);
        flow.set_row_spacing(6);
        flow.set_max_children_per_line(8);
        flow.set_hexpand(true);
        let put = |w: &gtk::Widget| {
            let fc = gtk::FlowBoxChild::new();
            fc.set_child(Some(w));
            fc.set_focusable(false);
            flow.insert(&fc, -1);
        };
        let chip = |label: &str, active: bool, debug: &str| {
            let b = gtk::ToggleButton::with_label(label);
            b.add_css_class("fx-quem");
            // (the chip keeps its width: the flow box would stretch it over the line)
            b.set_halign(gtk::Align::Start);
            b.set_active(active);
            crate::central::posicao_debug_sempre(debug, b.upcast_ref());
            b
        };
        for p in &nuvem {
            let id = p["id"].as_str().unwrap_or("").to_string();
            let pn = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
            let b = chip(&trf("Provider {name}", &[("name", pn)]), self.escolhidos.borrow().contains(&id), &format!("ajustes chave quem {id}"));
            let w = Rc::downgrade(self);
            b.connect_toggled(move |b| {
                let Some(t) = w.upgrade() else { return };
                let mut e = t.escolhidos.borrow_mut();
                e.retain(|x| x != &id);
                if b.is_active() {
                    e.push(id.clone());
                }
                drop(e);
                t.update_test();
                let t2 = t.clone();
                glib::idle_add_local_once(move || t2.render_who());
            });
            put(b.upcast_ref());
        }
        // the agents of the chosen providers
        let escolhidos = self.escolhidos.borrow().clone();
        if !escolhidos.is_empty() {
            let access = self.agent_access.borrow().clone();
            let assistentes: Vec<(String, String, bool)> = d
                .agents
                .iter()
                .filter(|a| a["agent"]["kind"] == "assistant")
                .filter_map(|a| {
                    let id = a["agent"]["id"].as_str()?.to_string();
                    let reaches = escolhidos.iter().any(|p| agent_reaches(&a["agent"], p));
                    Some((id, a["agent"]["name"].as_str().unwrap_or("").to_string(), reaches))
                })
                .collect();
            let mut fora = Vec::new();
            for (id, an, reaches) in &assistentes {
                let ativo = access.get(id).copied().unwrap_or(*reaches);
                if !ativo && !access.contains_key(id) {
                    fora.push((id.clone(), an.clone()));
                    continue;
                }
                let b = chip(an, ativo, &format!("ajustes chave quem agente {id}"));
                b.set_tooltip_text(Some(tr("Off: this agent stops calling these providers")));
                let (w, id, reaches) = (Rc::downgrade(self), id.clone(), *reaches);
                b.connect_toggled(move |b| {
                    let Some(t) = w.upgrade() else { return };
                    let mut acc = t.agent_access.borrow_mut();
                    if b.is_active() == reaches {
                        acc.remove(&id);
                    } else {
                        acc.insert(id.clone(), b.is_active());
                    }
                });
                put(b.upcast_ref());
            }
            if !fora.is_empty() {
                let mais = gtk::MenuButton::new();
                // (a label child: no arrow, like the other chips)
                mais.set_child(Some(&rotulo(tr("+ agent"), "")));
                mais.add_css_class("fx-quem");
                mais.set_halign(gtk::Align::Start);
                crate::central::posicao_debug_sempre("ajustes chave quem mais agente", mais.upcast_ref());
                let pop = gtk::Popover::new();
                pop.add_css_class("fx-pop");
                let pc = caixa(Orientation::Vertical, 2, "");
                for (id, an) in fora {
                    let b = action_button(&an, "", "texto");
                    b.set_halign(gtk::Align::Fill);
                    crate::central::posicao_debug_sempre(&format!("ajustes chave dar acesso {id}"), b.upcast_ref());
                    let (w, pop2) = (Rc::downgrade(self), pop.clone());
                    b.connect_clicked(move |_| {
                        pop2.popdown();
                        if let Some(t) = w.upgrade() {
                            t.agent_access.borrow_mut().insert(id.clone(), true);
                            let t2 = t.clone();
                            glib::idle_add_local_once(move || t2.render_who());
                        }
                    });
                    pc.append(&b);
                }
                let sc = gtk::ScrolledWindow::new();
                sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
                sc.set_propagate_natural_height(true);
                sc.set_max_content_height(320);
                sc.set_child(Some(&pc));
                pop.set_child(Some(&sc));
                mais.set_popover(Some(&pop));
                put(mais.upcast_ref());
            }
        }
        // what else uses a key of this name: read only
        for u in outros {
            let (k, classe) = kind_label(Some(u.kind));
            let l = rotulo(&format!("{} · {}", u.who, k), "fx-chip");
            if !classe.is_empty() {
                l.add_css_class(classe);
            }
            l.set_halign(gtk::Align::Start);
            l.set_tooltip_text(Some(tr("Uses a key of this name (changed where it is set up)")));
            put(l.upcast_ref());
        }
        self.f_quem.append(&flow);
        drop(d);
        self.update_test();
    }

    fn update_test(&self) {
        let tem = !self.escolhidos.borrow().is_empty();
        self.f_testar.set_sensitive(tem);
        if !tem {
            self.f_testar.set_active(false);
        }
        self.f_testar_sub.set_text(if tem { tr("makes a minimal request to the provider") } else { tr("choose a provider above to test against it") });
    }

    fn render(self: &Rc<Self>) {
        while let Some(c) = self.linhas.first_child() {
            self.linhas.remove(&c);
        }
        let d = self.data.borrow();
        if d.secrets.is_empty() {
            let (e, acoes) = estados::empty("cadeado", tr("No key in the vault"), tr("Keep a cloud provider's token or an API's key here; agents and flows use it by its name."), tr("Nothing is needed to use the local model."));
            e.set_margin_top(28);
            e.set_margin_bottom(28);
            let b = action_button(tr("Keep a key"), "mais", "principal");
            let n = self.f_nome.clone();
            b.connect_clicked(move |_| {
                n.grab_focus();
            });
            acoes.append(&b);
            self.linhas.append(&e);
        }
        let filtro = *self.filtro.borrow();
        let mut mostradas = 0;
        let mut contagem = [0usize; 5];
        for s in &d.secrets {
            let usos = key_uses(s, &d.providers, &d.tools, &d.agents, &d.connectors);
            for (i, (id, _)) in FILTERS.iter().enumerate() {
                if passes(id, &usos) {
                    contagem[i] += 1;
                }
            }
            if !passes(filtro, &usos) {
                continue;
            }
            mostradas += 1;
            let det = d.details.get(s).cloned().unwrap_or_default();
            self.linhas.append(&self.key_row(s, &usos, &det));
        }
        for (i, (_, b)) in self.filtros.iter().enumerate() {
            let nome = tr(FILTERS[i].1);
            b.set_label(&if i == 0 { format!("{nome} {}", contagem[0]) } else { nome.to_string() });
        }
        self.vazio_filtro.set_visible(!d.secrets.is_empty() && mostradas == 0);
        self.tabela.set_visible(true);
    }

    fn key_row(self: &Rc<Self>, name: &str, uses: &[KeyUse], detail: &Value) -> gtk::Box {
        let r = caixa(Orientation::Vertical, 0, "fx-tabela-linha");
        let l = caixa(Orientation::Horizontal, 12, "");
        let nc = caixa(Orientation::Vertical, 2, "");
        nc.set_size_request(220, -1);
        let n = rotulo(name, "fx-mono");
        n.set_xalign(0.0);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        n.set_hexpand(true);
        nc.append(&n);
        let agora = glib::DateTime::now_local().ok();
        let falhou = !detail["last_error"].is_null() && detail.get("last_error").is_some();
        match key_note(agora.as_ref().map(|n| n.to_unix()).unwrap_or(0), detail["expires_s"].as_i64(), falhou) {
            Some((texto, classe)) => {
                let nota = rotulo(&texto, "ajustes-estado");
                nota.add_css_class(classe);
                nota.set_xalign(0.0);
                if falhou {
                    nota.set_tooltip_text(detail["last_error"]["message"].as_str());
                }
                nc.append(&nota);
            }
            None => {
                let pontos = rotulo("••••••••••••", "fx-pontos");
                pontos.set_xalign(0.0);
                nc.append(&pontos);
            }
        }
        nc.set_valign(Align::Center);
        // (the name's label expands inside its fixed column, not the column in the row)
        nc.set_hexpand(false);
        l.append(&nc);
        let (tipo, classe) = kind_label(uses.first().map(|u| u.kind));
        let tc = caixa(Orientation::Horizontal, 0, "");
        tc.set_size_request(130, -1);
        let tch = rotulo(tipo, "fx-chip");
        if !classe.is_empty() {
            tch.add_css_class(classe);
        }
        tch.set_valign(Align::Center);
        tc.append(&tch);
        tc.set_valign(Align::Center);
        tc.set_hexpand(false);
        l.append(&tc);
        let quem = if uses.is_empty() { tr("no one").to_string() } else { uses.iter().map(|u| u.who.clone()).collect::<Vec<_>>().join(" · ") };
        let q = rotulo(&quem, "fx-tabela-texto");
        q.set_xalign(0.0);
        q.set_hexpand(true);
        q.set_wrap(true);
        q.set_max_width_chars(1);
        q.set_valign(Align::Center);
        if uses.is_empty() {
            q.add_css_class("fraco");
        }
        l.append(&q);
        let uso = rotulo(&agora.as_ref().map(|n| last_use_text(n, detail["last_used_s"].as_i64())).unwrap_or_default(), "fx-tabela-texto");
        uso.set_xalign(0.0);
        uso.set_size_request(110, -1);
        uso.set_valign(Align::Center);
        if detail["last_used_s"].as_i64().is_none() {
            uso.add_css_class("fraco");
        }
        l.append(&uso);
        let menu = gtk::ToggleButton::new();
        menu.add_css_class("fx-kebab");
        menu.set_child(Some(&ui_icon("pontos-v", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        menu.set_tooltip_text(Some(tr("Delete…")));
        crate::central::posicao_debug_sempre(&format!("ajustes chave menu {name}"), menu.upcast_ref());
        l.append(&menu);
        r.append(&l);
        // the inline confirmation
        let conf = gtk::Revealer::new();
        conf.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        conf.set_transition_duration(140);
        let cb = caixa(Orientation::Horizontal, 8, "fx-confirmar");
        cb.set_margin_top(10);
        let aviso = if uses.is_empty() {
            trf("Delete {name} from the vault? This cannot be undone.", &[("name", name.to_string())])
        } else {
            trf("Delete {name}? {who} stops working until a new key is kept.", &[("name", name.to_string()), ("who", uses.iter().map(|u| u.who.clone()).collect::<Vec<_>>().join(", "))])
        };
        let at = rotulo(&aviso, "fx-tabela-texto");
        at.set_xalign(0.0);
        at.set_wrap(true);
        at.set_max_width_chars(1);
        at.set_hexpand(true);
        at.set_valign(Align::Center);
        cb.append(&at);
        let nao = action_button(tr("Cancel"), "", "");
        nao.set_valign(Align::Center);
        let sim = action_button(tr("Delete"), "lixeira", "perigo");
        sim.set_valign(Align::Center);
        crate::central::posicao_debug_sempre(&format!("ajustes chave apagar {name}"), sim.upcast_ref());
        cb.append(&nao);
        cb.append(&sim);
        conf.set_child(Some(&cb));
        r.append(&conf);
        let aberto = self.confirming.borrow().as_deref() == Some(name);
        conf.set_reveal_child(aberto);
        menu.set_active(aberto);
        let (w, nm, c2) = (Rc::downgrade(self), name.to_string(), conf.clone());
        menu.connect_toggled(move |m| {
            let Some(t) = w.upgrade() else { return };
            *t.confirming.borrow_mut() = m.is_active().then(|| nm.clone());
            c2.set_reveal_child(m.is_active());
        });
        let m2 = menu.clone();
        nao.connect_clicked(move |_| m2.set_active(false));
        let (w, nm) = (Rc::downgrade(self), name.to_string());
        sim.connect_clicked(move |b| {
            let Some(t) = w.upgrade() else { return };
            b.set_sensitive(false);
            let w = Rc::downgrade(&t);
            crate::fluxos::call("secrets.delete", json!({"name": nm}), move |r| {
                if let Some(t) = w.upgrade() {
                    if let Err(e) = r {
                        eprintln!("ajustes: chave não apagada: {e}");
                    }
                    *t.confirming.borrow_mut() = None;
                    t.reload();
                }
            });
        });
        r
    }
}

/// The connector manifests' text (their `cofre:NAME` say which keys each uses).
fn connector_texts() -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::central_ajustes::pasta()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") {
                if let (Some(n), Ok(t)) = (p.file_stem().and_then(|s| s.to_str()), std::fs::read_to_string(&p)) {
                    v.push((n.to_string(), t));
                }
            }
        }
    }
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_vault() {
        assert!(valid_name("openrouter.token"));
        assert!(valid_name("github:oauth-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("-x"));
        assert!(!valid_name("com espaço"));
        assert!(!valid_name(&"a".repeat(129)));
    }

    #[test]
    fn references_are_found_in_every_form() {
        assert_eq!(vault_refs("Bearer {cofre:clima.token}"), vec!["clima.token"]);
        assert_eq!(vault_refs("TOKEN = \"cofre:github.token\"\nX = \"cofre://correio.oauth\""), vec!["github.token", "correio.oauth"]);
        assert_eq!(vault_refs("cofre:a.b cofre:a.b"), vec!["a.b"]);
        assert!(vault_refs("nada aqui").is_empty());
    }

    #[test]
    fn uses_come_from_providers_tools_connectors_and_agents() {
        let providers = vec![json!({"id": "openrouter", "name": "OpenRouter", "secret": "openrouter.token"}), json!({"id": "local", "kind": "local", "secret": ""})];
        let tools = vec![json!({"name": "clima_hoje", "config": {"headers": {"Authorization": "Bearer {cofre:clima.token}"}}})];
        let agents = vec![
            json!({"agent": {"id": "a1", "name": "Planejador", "kind": "assistant", "config": {"model": {"provider": "openrouter", "model": "llama"}}}}),
            json!({"agent": {"id": "a2", "name": "Pesquisa", "kind": "assistant", "config": {"fallback": [{"provider": "openrouter", "model": "x"}]}}}),
            json!({"agent": {"id": "a3", "name": "Do notebook", "kind": "remote", "config": {"card_url": "https://x", "key_secret": "a2a.notebook"}}}),
        ];
        let connectors = vec![("github".to_string(), "[ambiente]\nGITHUB_TOKEN = \"cofre:github.token\"\n".to_string())];
        let u = key_uses("openrouter.token", &providers, &tools, &agents, &connectors);
        assert_eq!(u, vec![KeyUse { kind: UseKind::Provider, who: "OpenRouter: Planejador, Pesquisa".into() }]);
        assert_eq!(key_uses("clima.token", &providers, &tools, &agents, &connectors)[0].kind, UseKind::Tool);
        assert_eq!(key_uses("github.token", &providers, &tools, &agents, &connectors)[0].who, "github");
        assert_eq!(key_uses("a2a.notebook", &providers, &tools, &agents, &connectors)[0].kind, UseKind::RemoteAgent);
        assert_eq!(key_uses("a2a.chave", &providers, &tools, &agents, &connectors)[0].kind, UseKind::LocalApi);
        assert!(key_uses("solta", &providers, &tools, &agents, &connectors).is_empty());
        assert!(passes("sem-uso", &[]) && !passes("provedores", &[]) && passes("todas", &[]));
    }

    #[test]
    fn the_probe_and_the_address() {
        let p = provider_probe("https://openrouter.ai/api/v1/", "sk-x");
        assert_eq!(p["endpoint"], "https://openrouter.ai/api/v1/models");
        assert_eq!(p["headers"]["Authorization"], "Bearer sk-x");
        assert_eq!(socket_shown("/home/ana/.run/fluxos.sock", "/home/ana"), "unix:~/.run/fluxos.sock");
        assert_eq!(socket_shown("/run/user/1000/kalango/fluxos.sock", "/home/ana"), "unix:/run/user/1000/kalango/fluxos.sock");
    }

    #[test]
    fn last_use_and_notes_read_like_the_design() {
        let now = glib::DateTime::from_local(2026, 11, 15, 12, 0, 0.0).unwrap();
        let n = now.to_unix();
        assert_eq!(last_use_text(&now, None), tr("never"));
        assert_eq!(last_use_text(&now, Some(n - 12 * 60)), trf("{n} min ago", &[("n", "12".to_string())]));
        assert_eq!(last_use_text(&now, Some(n - 2 * 3600)), trf("{n} h ago", &[("n", "2".to_string())]));
        assert_eq!(last_use_text(&now, Some(n - 4 * 3600)), trf("today {time}", &[("time", "08:00".to_string())]));
        assert_eq!(last_use_text(&now, Some(n - 86_400)), tr("yesterday"));
        assert_eq!(last_use_text(&now, Some(n - 9 * 86_400)), trf("{n} days ago", &[("n", "9".to_string())]));
        assert_eq!(key_note(n, None, false), None);
        assert_eq!(key_note(n, None, true).map(|x| x.1), Some("erro"));
        assert_eq!(key_note(n, Some(n + 12 * 86_400), false), Some((trf("expires in {n} days", &[("n", "12".to_string())]), "atencao")));
        assert_eq!(key_note(n, Some(n + 20 * 86_400), false), None);
        assert_eq!(key_note(n, Some(n - 1), false).map(|x| x.1), Some("erro"));
    }

    #[test]
    fn giving_and_taking_a_key_changes_only_the_agent_s_routes() {
        let a = json!({"id": "planejador", "config": {"model": null, "fallback": [], "allow_cloud": false}});
        let model_of = |p: &str| if p == "openrouter" { "llama-3.3-70b".to_string() } else { String::new() };
        let ps = vec!["openrouter".to_string()];
        let dado = with_access(&a, &ps, &model_of, true);
        assert_eq!(dado["config"]["allow_cloud"], true);
        assert_eq!(dado["config"]["fallback"], json!([{"provider": "openrouter", "model": "llama-3.3-70b"}]));
        assert!(agent_reaches(&dado, "openrouter"));
        // given again: nothing more
        assert_eq!(with_access(&dado, &ps, &model_of, true), dado);
        let tirado = with_access(&dado, &ps, &model_of, false);
        assert!(!agent_reaches(&tirado, "openrouter"));
        let fixo = json!({"config": {"model": {"provider": "openrouter", "model": "x"}, "fallback": []}});
        assert_eq!(with_access(&fixo, &ps, &model_of, false)["config"]["model"], Value::Null);
    }

    #[test]
    fn the_expiry_is_a_real_date_at_the_end_of_the_day() {
        assert_eq!(parse_expiry(""), Some(None));
        let v = parse_expiry("31/03/2027").unwrap().unwrap();
        assert_eq!(expiry_field(v), "31/03/2027");
        assert_eq!(parse_expiry("31/3/27"), Some(Some(v)));
        assert_eq!(parse_expiry("31-03-2027"), Some(Some(v)));
        assert_eq!(parse_expiry("31/02/2027"), None);
        assert_eq!(parse_expiry("amanhã"), None);
    }
}
