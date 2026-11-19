//! AI widgets (design "Widgets › Inteligência artificial"): "Tokens hoje" (S, today's use against the daily budget,
//! cloud and local) and "Modelo local" (S, the model, whether the server has it loaded, VRAM and speed). The engine
//! writes every reply's use to ~/.local/state/kalango/uso-tokens.jsonl; the budget is the person's
//! (~/.config/kalango/orcamento.json, "tokens_por_dia"; 500 thousand when not set).

use super::comum::{barra, decimal, Anel, Suave, Texto, GIB, GOLD, TEAL};
use super::dados::{self, Amostra};
use super::{quadro, Tam};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::Value;
use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

pub const ORCAMENTO_PADRAO: u64 = 500_000;

fn uso_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/uso-tokens.jsonl")
}

/// The day's token budget: the limit the agent enforces (Agents › Permissions or Usage), else the widget's own setting,
/// else the default.
pub fn orcamento() -> u64 {
    if let Some(n) = crate::central_uso::limite_diario() {
        return n;
    }
    std::fs::read(glib::user_config_dir().join("kalango/orcamento.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["tokens_por_dia"].as_u64())
        .filter(|n| *n > 0).unwrap_or(ORCAMENTO_PADRAO)
}

/// Today's use, read from where the last reading stopped (the log only grows).
#[derive(Default, Clone)]
pub struct Uso {
    dia: String,
    pos: u64,
    pub local: u64,
    pub nuvem: u64,
    pub tok_s: Option<f64>,
    pub modelo: Option<String>,
}

impl Uso {
    pub fn atualizar(&mut self) {
        let hoje = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default();
        let inicio_do_dia = glib::DateTime::now_local().ok()
            .and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
        if hoje != self.dia {
            *self = Uso { dia: hoje, modelo: self.modelo.take(), tok_s: self.tok_s, ..Default::default() };
        }
        let Ok(mut f) = std::fs::File::open(uso_path()) else { return };
        let tam = f.metadata().map(|m| m.len()).unwrap_or(0);
        if tam < self.pos {
            // a new file (rotated): from its start
            self.pos = 0;
        }
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return;
        }
        let mut s = String::new();
        if f.read_to_string(&mut s).is_err() {
            return;
        }
        // only whole lines (an engine may be writing the next one)
        let fim = s.rfind('\n').map(|i| i + 1).unwrap_or(0);
        for l in s[..fim].lines() {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            if let Some(s) = v["tok_s"].as_f64() {
                self.tok_s = Some(s);
            }
            if let Some(m) = v["modelo"].as_str() {
                self.modelo = Some(m.to_string());
            }
            if v["t"].as_i64().unwrap_or(0) < inicio_do_dia {
                continue;
            }
            let n = v["tokens"].as_u64().unwrap_or(0);
            if v["tipo"] == "nuvem" {
                self.nuvem += n;
            } else {
                self.local += n;
            }
        }
        self.pos += fim as u64;
    }
}

/// "182k", "640".
pub fn mil(n: u64) -> String {
    if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

pub fn tokens() -> gtk::Widget {
    let q = quadro(Tam::P.px().0, Tam::P.px().1);
    q.append(&super::comum::cabecalho("raio", tr("TOKENS TODAY")).0);
    let anel = Anel::new(96, GOLD);
    let numero = Texto::new("desk-anel-numero");
    numero.alinhar(0.5);
    anel.centro.append(&numero.stack);
    let de = rotulo("", "desk-metrica-sub");
    anel.centro.append(&de);
    q.append(&anel.widget);
    let partes = Texto::new("desk-metrica-sub");
    q.append(&partes.stack);
    let uso: Rc<RefCell<Uso>> = Rc::default();
    let atualizar = {
        let q = q.downgrade();
        move || {
            if q.upgrade().is_none() {
                return false;
            }
            uso.borrow_mut().atualizar();
            let u = uso.borrow();
            let total = u.local + u.nuvem;
            let orc = orcamento();
            anel.ir(total as f64 / orc as f64);
            numero.definir(&mil(total));
            de.set_text(&trf("of {n}", &[("n", mil(orc))]));
            let nuvem = if total > 0 { (u.nuvem as f64 / total as f64 * 100.0).round() as u64 } else { 0 };
            partes.definir(&trf("Cloud {c}% · Local {l}%", &[("c", nuvem.to_string()), ("l", if total > 0 { (100 - nuvem).to_string() } else { "0".into() })]));
            true
        }
    };
    atualizar();
    glib::timeout_add_local(Duration::from_secs(10), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    q.upcast()
}

/// Does llama-server answer on its socket (the model is loaded)?
fn servidor_pronto() -> bool {
    use std::io::Write;
    let sock = std::env::var_os("KALANGO_MODELO_SOCKET").map(PathBuf::from).unwrap_or_else(|| glib::user_runtime_dir().join("kalango/modelo.sock"));
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(sock) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n").is_err() {
        return false;
    }
    let mut r = [0u8; 64];
    let n = s.read(&mut r).unwrap_or(0);
    String::from_utf8_lossy(&r[..n]).contains(" 200")
}

/// Speed bar scale: 200 tok/s fills it.
const VELOCIDADE_CHEIA: f64 = 200.0;

pub fn modelo() -> gtk::Widget {
    let (w, h) = Tam::P.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("engrenagem", tr("LOCAL MODEL")).0);
    let nome = rotulo("", "desk-modelo-nome");
    nome.set_xalign(0.0);
    nome.set_ellipsize(gtk::pango::EllipsizeMode::End);
    nome.set_max_width_chars(1);
    nome.set_hexpand(true);
    q.append(&nome);
    let estado = rotulo("", "desk-modelo-estado");
    estado.set_xalign(0.0);
    q.append(&estado);
    let corpo = caixa(Orientation::Vertical, 8, "");
    let linha = |t: &str| {
        let l = caixa(Orientation::Horizontal, 0, "");
        let a = rotulo(t, "desk-metrica-sub");
        a.set_xalign(0.0);
        a.set_hexpand(true);
        l.append(&a);
        let b = rotulo("—", "desk-metrica-nome");
        l.append(&b);
        (l, b)
    };
    let (l1, vram) = linha(tr("VRAM"));
    let (b1, s1): (gtk::DrawingArea, Suave) = barra(160, TEAL);
    let (l2, vel) = linha(tr("Speed"));
    let (b2, s2) = barra(160, TEAL);
    for x in [l1.upcast_ref::<gtk::Widget>(), b1.upcast_ref(), l2.upcast_ref(), b2.upcast_ref()] {
        corpo.append(x);
    }
    q.append(&corpo);
    {
        let (b1, s1) = (b1.clone(), s1.clone());
        dados::ouvir(&q, move |a: &Amostra| match a.gpu.as_ref().and_then(|g| g.vram) {
            Some((u, t)) => {
                vram.set_text(&format!("{} / {:.0} GB", decimal(u / GIB), t / GIB));
                s1.ir(u / t, &b1);
            }
            None => vram.set_text("—"),
        });
    }
    let uso: Rc<RefCell<Uso>> = Rc::default();
    let atualizar = {
        let q = q.downgrade();
        move || {
            if q.upgrade().is_none() {
                return false;
            }
            uso.borrow_mut().atualizar();
            let u = uso.borrow().clone();
            nome.set_text(&u.modelo.as_deref().map(crate::conversa::model_label).unwrap_or_else(crate::conversa::guess_model));
            match u.tok_s {
                Some(v) => {
                    vel.set_text(&trf("{n} tok/s", &[("n", format!("{v:.0}"))]));
                    s2.ir(v / VELOCIDADE_CHEIA, &b2);
                }
                None => vel.set_text("—"),
            }
            let estado = estado.clone();
            crate::lateral::em_segundo_plano(servidor_pronto, move |ok| {
                estado.set_text(&if ok { trf("{runtime} · loaded", &[("runtime", "llama.cpp".into())]) } else { trf("{runtime} · stopped", &[("runtime", "llama.cpp".into())]) });
                if ok { estado.remove_css_class("parado") } else { estado.add_css_class("parado") }
            });
            true
        }
    };
    atualizar();
    glib::timeout_add_local(Duration::from_secs(5), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    q.upcast()
}

/// Who is capturing now, from PipeWire's graph (`pw-dump`): apps recording audio (the microphone) and apps reading
/// the compositor's screen-share source (the screen).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observadores {
    pub tela: Vec<String>,
    pub microfone: Vec<String>,
}

pub fn observadores(dump: &str) -> Observadores {
    let Ok(Value::Array(objs)) = serde_json::from_str::<Value>(dump) else { return Observadores::default() };
    let props = |o: &Value| o["info"]["props"].clone();
    let nome = |p: &Value| p["application.name"].as_str().or(p["node.name"].as_str()).unwrap_or("").to_string();
    let nos: std::collections::HashMap<u64, &Value> = objs.iter().filter(|o| o["type"] == "PipeWire:Interface:Node").filter_map(|o| Some((o["id"].as_u64()?, o))).collect();
    let mut r = Observadores::default();
    for o in nos.values() {
        let p = props(o);
        let rodando = o["info"]["state"] == "running";
        if p["media.class"] == "Stream/Input/Audio" && rodando {
            let n = nome(&p);
            if !n.is_empty() && !r.microfone.contains(&n) {
                r.microfone.push(n);
            }
        }
    }
    // links from the compositor's screen-share source to someone else's video stream
    for l in objs.iter().filter(|o| o["type"] == "PipeWire:Interface:Link") {
        let (Some(saida), Some(entrada)) = (l["info"]["output-node-id"].as_u64(), l["info"]["input-node-id"].as_u64()) else { continue };
        let (Some(fonte), Some(destino)) = (nos.get(&saida), nos.get(&entrada)) else { continue };
        let pf = props(fonte);
        let tela = pf["media.class"] == "Video/Source" && [pf["node.name"].as_str(), pf["application.name"].as_str()].iter().flatten()
            .any(|n| n.to_lowercase().contains("niri") || n.to_lowercase().contains("screen"));
        if tela {
            let n = nome(&props(destino));
            if !n.is_empty() && !r.tela.contains(&n) {
                r.tela.push(n);
            }
        }
    }
    r
}

/// Bytes the agents sent to cloud models today (the engine writes them with each cloud reply; local ones send none).
fn enviado_hoje() -> u64 {
    let inicio = glib::DateTime::now_local().ok()
        .and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
    std::fs::read_to_string(uso_path()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["t"].as_i64().unwrap_or(0) >= inicio && v["tipo"] == "nuvem").map(|v| v["bytes_enviados"].as_u64().unwrap_or(0)).sum()
}

fn bytes(n: u64) -> String {
    if n >= 1_000_000 { format!("{} MB", super::comum::decimal(n as f64 / 1e6)) } else if n >= 1000 { format!("{} KB", n / 1000) } else { format!("{n} B") }
}

pub fn privacidade() -> gtk::Widget {
    let (w, h) = Tam::P.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("escudo", tr("PRIVACY")).0);
    let titulo = rotulo("", "desk-privacidade-titulo");
    titulo.set_xalign(0.0);
    titulo.set_wrap(true);
    titulo.set_max_width_chars(16);
    q.append(&titulo);
    let corpo = caixa(Orientation::Vertical, 6, "");
    corpo.set_vexpand(true);
    corpo.set_valign(gtk::Align::End);
    let linha = |t: &str| {
        let l = caixa(Orientation::Horizontal, 0, "");
        let a = rotulo(t, "desk-metrica-sub");
        a.set_xalign(0.0);
        a.set_hexpand(true);
        l.append(&a);
        let b = rotulo("", "desk-metrica-nome");
        b.set_ellipsize(gtk::pango::EllipsizeMode::End);
        b.set_max_width_chars(10);
        l.append(&b);
        corpo.append(&l);
        b
    };
    let (tela, mic, env) = (linha(tr("Screen")), linha(tr("Microphone")), linha(tr("Sent today")));
    q.append(&corpo);
    let vivo = q.downgrade();
    let atualizar = move || {
        if vivo.upgrade().is_none() {
            return false;
        }
        let (titulo, tela, mic, env) = (titulo.clone(), tela.clone(), mic.clone(), env.clone());
        crate::lateral::em_segundo_plano(|| {
            let o = std::process::Command::new("pw-dump").output().ok().map(|o| observadores(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
            (o, enviado_hoje())
        }, move |(o, enviado): (Observadores, u64)| {
            titulo.set_text(match (o.tela.is_empty(), o.microfone.is_empty()) {
                (true, true) => tr("Nothing is being watched"),
                (false, _) => tr("The screen is being shared"),
                (true, false) => tr("The microphone is in use"),
            });
            tela.set_text(&if o.tela.is_empty() { tr("nobody").to_string() } else { o.tela.join(", ") });
            mic.set_text(&if o.microfone.is_empty() { tr("off").to_string() } else { o.microfone.join(", ") });
            for (l, ativo) in [(&tela, !o.tela.is_empty()), (&mic, !o.microfone.is_empty())] {
                if ativo { l.add_css_class("alerta") } else { l.remove_css_class("alerta") }
            }
            env.set_text(&bytes(enviado));
        });
        true
    };
    atualizar();
    glib::timeout_add_local(Duration::from_secs(5), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    q.upcast()
}

/// "Agentes" (S): how many agents are working, which, and how many requests wait for the person.
pub fn agentes(shell: &std::rc::Rc<crate::Shell>) -> gtk::Widget {
    let (w, h) = Tam::P.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("spark", tr("AGENTS")).0);
    let n = caixa(Orientation::Horizontal, 6, "");
    let numero = rotulo("0", "desk-agentes-numero");
    n.append(&numero);
    let ativos = rotulo("", "desk-metrica-sub");
    ativos.set_valign(gtk::Align::End);
    ativos.set_margin_bottom(8);
    n.append(&ativos);
    q.append(&n);
    let lista = caixa(Orientation::Vertical, 4, "");
    q.append(&lista);
    let espera = rotulo("", "desk-agentes-espera");
    espera.set_xalign(0.0);
    q.append(&espera);
    let s = std::rc::Rc::downgrade(shell);
    let vivo = q.downgrade();
    let atualizar = move || {
        let (Some(s), true) = (s.upgrade(), vivo.upgrade().is_some()) else { return false };
        let pids = crate::sistema::agent_pids().len();
        numero.set_text(&pids.to_string());
        ativos.set_text(if pids == 1 { tr("agent active") } else { tr("agents active") });
        while let Some(c) = lista.first_child() {
            lista.remove(&c);
        }
        if let Some((pedido, _)) = s.estado.agent_task.borrow().as_ref() {
            let l = caixa(Orientation::Horizontal, 6, "");
            let p = caixa(Orientation::Vertical, 0, "desk-ponto");
            p.add_css_class("estado-verde");
            p.set_valign(gtk::Align::Center);
            l.append(&p);
            let t = rotulo(&format!("{} · {pedido}", tr("Agent")), "desk-agentes-nome");
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.set_max_width_chars(1);
            t.set_hexpand(true);
            t.set_xalign(0.0);
            l.append(&t);
            lista.append(&l);
        }
        let pend = s.estado.pendentes.get().unwrap_or(0);
        espera.set_text(&match pend {
            0 => String::new(),
            1 => tr("1 waits for you").to_string(),
            n => trf("{n} wait for you", &[("n", n.to_string())]),
        });
        true
    };
    atualizar();
    glib::timeout_add_local(Duration::from_secs(2), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    q.upcast()
}

/// "Agentes ativos" (L): the agent at work with its last steps, the requests waiting with what they ask; "Revisar"
/// opens the request (approving stays on the approval screen) and "Central de agentes" the agents' bar.
pub fn agentes_detalhe(shell: &std::rc::Rc<crate::Shell>) -> gtk::Widget {
    let (w, h) = Tam::G.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("spark", tr("ACTIVE AGENTS")).0);
    let lista = caixa(Orientation::Vertical, 12, "");
    lista.set_vexpand(true);
    q.append(&lista);
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_homogeneous(true);
    let revisar = gtk::Button::with_label(tr("Review the request"));
    revisar.add_css_class("desk-botao-principal");
    let central = gtk::Button::with_label(tr("Agents center"));
    central.add_css_class("desk-botao");
    let s = std::rc::Rc::downgrade(shell);
    revisar.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.right_bar().abrir(crate::lateral::Modo::Agente);
        }
    });
    let s = std::rc::Rc::downgrade(shell);
    central.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.abrir_central(None);
        }
    });
    acoes.append(&revisar);
    acoes.append(&central);
    q.append(&acoes);
    let bloco = |nome: &str, estado: &str, cor: &str, progresso: f64, linhas: &[(char, String)]| {
        let b = caixa(Orientation::Vertical, 6, "");
        let l = caixa(Orientation::Horizontal, 8, "");
        let p = caixa(Orientation::Vertical, 0, "desk-ponto");
        p.add_css_class(if cor == GOLD { "estado-ouro" } else { "estado-verde" });
        p.set_valign(gtk::Align::Center);
        l.append(&p);
        l.append(&rotulo(nome, "desk-agentes-forte"));
        let e = rotulo(estado, "desk-metrica-sub");
        e.set_ellipsize(gtk::pango::EllipsizeMode::End);
        e.set_max_width_chars(1);
        e.set_hexpand(true);
        e.set_xalign(0.0);
        l.append(&e);
        b.append(&l);
        let (bar, v) = barra(344, cor);
        b.append(&bar);
        v.ir(progresso, &bar);
        for (m, t) in linhas {
            let r = rotulo(&format!("{m} {t}"), "desk-passo");
            if *m == '!' {
                r.add_css_class("espera");
            }
            r.set_xalign(0.0);
            r.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.set_max_width_chars(1);
            r.set_hexpand(true);
            b.append(&r);
        }
        b
    };
    let s = std::rc::Rc::downgrade(shell);
    let vivo = q.downgrade();
    let ultimo = RefCell::new(String::new());
    let atualizar = move || {
        let (Some(s), true) = (s.upgrade(), vivo.upgrade().is_some()) else { return false };
        let conv = s.right_bar().conversa.clone();
        let passos: Vec<(char, String)> = conv.passos(3).into_iter().map(|(m, t, _)| (m, t)).collect();
        let tarefa = s.estado.agent_task.borrow().clone();
        let pendentes = s.estado.pendentes.get().unwrap_or(0);
        let chave = format!("{tarefa:?}|{passos:?}|{pendentes}");
        if *ultimo.borrow() == chave {
            return true;
        }
        *ultimo.borrow_mut() = chave;
        while let Some(c) = lista.first_child() {
            lista.remove(&c);
        }
        match &tarefa {
            Some((pedido, n)) => lista.append(&bloco(tr("Agent"), pedido, TEAL, (*n as f64 + 1.0) / (*n as f64 + 3.0), &passos)),
            None => lista.append(&bloco(tr("Agent"), tr("Ready · Super+G"), TEAL, 0.0, &[])),
        }
        if pendentes > 0 {
            let linhas = [('!', if pendentes == 1 { tr("1 request needs you").to_string() } else { trf("{n} requests need you", &[("n", pendentes.to_string())]) })];
            lista.append(&bloco(tr("Approvals"), tr("Waiting for approval"), GOLD, 1.0, &linhas));
        }
        revisar.set_sensitive(pendentes > 0);
        true
    };
    atualizar();
    glib::timeout_add_local(Duration::from_secs(1), move || if atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    q.upcast()
}

#[cfg(test)]
mod tests {
    /// PipeWire: an app recording the microphone and one reading niri's screen-share source are named; idle
    /// streams are not.
    #[test]
    fn finds_who_is_watching() {
        let dump = serde_json::json!([
            {"id": 40, "type": "PipeWire:Interface:Node", "info": {"state": "running", "props": {"media.class": "Stream/Input/Audio", "application.name": "Gravador"}}},
            {"id": 41, "type": "PipeWire:Interface:Node", "info": {"state": "idle", "props": {"media.class": "Stream/Input/Audio", "application.name": "Parado"}}},
            {"id": 50, "type": "PipeWire:Interface:Node", "info": {"state": "running", "props": {"media.class": "Video/Source", "node.name": "niri"}}},
            {"id": 51, "type": "PipeWire:Interface:Node", "info": {"state": "running", "props": {"media.class": "Stream/Input/Video", "application.name": "OBS"}}},
            {"id": 60, "type": "PipeWire:Interface:Link", "info": {"output-node-id": 50, "input-node-id": 51}}
        ]).to_string();
        let o = super::observadores(&dump);
        assert_eq!(o.microfone, vec!["Gravador".to_string()]);
        assert_eq!(o.tela, vec!["OBS".to_string()]);
        assert_eq!(super::observadores("[]"), super::Observadores::default());
    }
}

#[cfg(test)]
mod tests_uso {
    use super::*;

    /// Today's lines are added up (cloud and local apart), older days are not, and reading again only reads what
    /// was appended.
    #[test]
    fn adds_up_todays_use() {
        let dir = std::env::temp_dir().join(format!("kalango-uso-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("kalango")).unwrap();
        std::env::set_var("XDG_STATE_HOME", &dir);
        let agora = glib::DateTime::now_local().unwrap().to_unix();
        let p = dir.join("kalango/uso-tokens.jsonl");
        let linhas = [
            serde_json::json!({"t": agora - 3 * 86400, "tipo": "local", "tokens": 999_999}),
            serde_json::json!({"t": agora, "tipo": "local", "tokens": 1200, "tok_s": 41.5, "modelo": "LFM3-1.7B-Q4_K_M"}),
            serde_json::json!({"t": agora, "tipo": "nuvem", "tokens": 800}),
        ];
        std::fs::write(&p, linhas.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        let mut u = Uso::default();
        u.atualizar();
        assert_eq!((u.local, u.nuvem), (1200, 800));
        assert_eq!(u.tok_s, Some(41.5));
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(format!("{}\n", serde_json::json!({"t": agora, "tipo": "local", "tokens": 100})).as_bytes()).unwrap();
        u.atualizar();
        assert_eq!(u.local, 1300);
        assert_eq!(mil(182_400), "182k");
        assert_eq!(mil(640), "640");
        std::env::remove_var("XDG_STATE_HOME");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
