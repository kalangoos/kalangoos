//! Routines (the agents center's "Rotinas"): things that run by themselves — at a time (every day, on weekdays or on
//! some days of the week), when a USB device is connected, or when the person enters a space. A routine either gives
//! the agent a request (it runs alone: what is set to "ask" is not done, nobody is there to answer; kalangod still
//! asks for system changes on its screen) or opens apps.
//!
//! The shell is the scheduler (it is always running in the session). Routines keep the agent's permissions and limits
//! (the agent applies them) and never break focus mode: a routine due while the focus timer runs waits for it to end.
//! Each run is recorded on the routine (its last result) and in the agents' task history ("Em execução").
//!
//! Stored in `~/.config/kalango/rotinas.json`.

use crate::Shell;
use gtk::prelude::*;
use gtk::{gio, glib};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};

#[derive(Clone, Debug, PartialEq)]
pub enum Gatilho {
    /// days 1 (Monday) … 7 (Sunday); empty: every day
    Horario { dias: Vec<u8>, hora: u8, minuto: u8 },
    Usb,
    Espaco(String),
    /// only when the person runs it ("Rodar agora"): an agent made with no time
    Manual,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Acao {
    Pedido(String),
    /// app ids
    Abrir(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rotina {
    pub id: u64,
    pub nome: String,
    pub acao: Acao,
    pub gatilho: Gatilho,
    pub ativa: bool,
    pub criado: i64,
    /// (when, "ok" | "falhou", summary or why)
    pub ultima: Option<(i64, String, String)>,
    /// the time slot of the last scheduled run (so a slot runs once)
    pub disparo: i64,
    /// made by description ("Criar agente"): what it was asked, its steps and the scope it runs with
    pub agente: Option<AgenteConfig>,
}

/// An agent made by description: the person's sentence, its steps (text, kind of action), how much it asks, the
/// permission columns it runs without (the minimal ones: what its steps do not need) and the model it thinks with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgenteConfig {
    pub descricao: String,
    pub passos: Vec<(String, String)>,
    pub autonomia: String,
    pub desligadas: Vec<String>,
    pub modelo: String,
    /// an installed agent's scope (its manifest, as reduced before installing): folders, programs, sites
    pub pastas: Vec<String>,
    pub programas: Vec<String>,
    pub dominios: Vec<String>,
    /// where it came from (the manifest's id), for an installed one
    pub origem: String,
}

fn lista(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

impl AgenteConfig {
    fn from_json(v: &Value) -> Option<AgenteConfig> {
        Some(AgenteConfig {
            descricao: v["descricao"].as_str()?.to_string(),
            passos: v["passos"].as_array().into_iter().flatten().filter_map(|p| Some((p["texto"].as_str()?.to_string(), p["acao"].as_str().unwrap_or("ler").to_string()))).collect(),
            autonomia: v["autonomia"].as_str().unwrap_or("risco").to_string(),
            desligadas: v["desligadas"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            modelo: v["modelo"].as_str().unwrap_or("").to_string(),
            pastas: lista(&v["pastas"]),
            programas: lista(&v["programas"]),
            dominios: lista(&v["dominios"]),
            origem: v["origem"].as_str().unwrap_or("").to_string(),
        })
    }

    fn to_json(&self) -> Value {
        json!({"descricao": self.descricao, "passos": self.passos.iter().map(|(t, a)| json!({"texto": t, "acao": a})).collect::<Vec<_>>(),
            "autonomia": self.autonomia, "desligadas": self.desligadas, "modelo": self.modelo, "pastas": self.pastas, "programas": self.programas,
            "dominios": self.dominios, "origem": self.origem})
    }

    /// What the agent is asked each run: the person's sentence and the steps it was shown.
    pub fn pedido(&self, nome: &str) -> String {
        let mut p = trf("You are the agent “{name}”. {description}", &[("name", nome.to_string()), ("description", self.descricao.clone())]);
        if !self.passos.is_empty() {
            p.push('\n');
            p += tr("Steps:");
            for (i, (t, _)) in self.passos.iter().enumerate() {
                p += &format!("\n{}. {t}", i + 1);
            }
        }
        p
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rotinas {
    pub lista: Vec<Rotina>,
    pub proximo: u64,
    /// suggestions the person does not want again
    pub dispensadas: Vec<String>,
}

const DIAS: [&str; 7] = [n_("every Monday at {time}"), n_("every Tuesday at {time}"), n_("every Wednesday at {time}"), n_("every Thursday at {time}"),
    n_("every Friday at {time}"), n_("every Saturday at {time}"), n_("every Sunday at {time}")];

impl Gatilho {
    fn from_json(v: &Value) -> Option<Gatilho> {
        Some(match v["tipo"].as_str()? {
            "horario" => Gatilho::Horario {
                dias: v["dias"].as_array().map(|a| a.iter().filter_map(|d| d.as_u64().filter(|d| (1..=7).contains(d)).map(|d| d as u8)).collect()).unwrap_or_default(),
                hora: v["hora"].as_u64().unwrap_or(9).min(23) as u8, minuto: v["minuto"].as_u64().unwrap_or(0).min(59) as u8,
            },
            "usb" => Gatilho::Usb,
            "espaco" => Gatilho::Espaco(v["nome"].as_str()?.to_string()),
            "manual" => Gatilho::Manual,
            _ => return None,
        })
    }

    fn to_json(&self) -> Value {
        match self {
            Gatilho::Horario { dias, hora, minuto } => json!({"tipo": "horario", "dias": dias, "hora": hora, "minuto": minuto}),
            Gatilho::Usb => json!({"tipo": "usb"}),
            Gatilho::Espaco(n) => json!({"tipo": "espaco", "nome": n}),
            Gatilho::Manual => json!({"tipo": "manual"}),
        }
    }

    /// The chip next to the name: "toda sexta às 18:00", "ao conectar um dispositivo USB"…
    pub fn texto(&self) -> String {
        match self {
            Gatilho::Horario { dias, hora, minuto } => {
                let h = format!("{hora:02}:{minuto:02}");
                match dias.as_slice() {
                    [] => trf("every day at {time}", &[("time", h)]),
                    [1, 2, 3, 4, 5] => trf("weekdays at {time}", &[("time", h)]),
                    [d] => trf(DIAS[(*d as usize - 1).min(6)], &[("time", h)]),
                    ds => trf("{days} at {time}", &[("days", ds.iter().map(|d| dia_curto(*d)).collect::<Vec<_>>().join(", ")), ("time", h)]),
                }
            }
            Gatilho::Usb => tr("when a USB device is connected").to_string(),
            Gatilho::Espaco(n) => trf("when entering the space {name}", &[("name", n.clone())]),
            Gatilho::Manual => tr("when you ask").to_string(),
        }
    }
}

/// "seg", "ter"… (the system's language), from a Monday.
fn dia_curto(d: u8) -> String {
    glib::DateTime::from_local(2024, 1, d as i32, 12, 0, 0.0).ok().and_then(|x| x.format("%a").ok()).map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default()
}

impl Rotina {
    fn from_json(v: &Value) -> Option<Rotina> {
        let a = &v["acao"];
        Some(Rotina {
            id: v["id"].as_u64()?,
            nome: v["nome"].as_str()?.to_string(),
            acao: match a["tipo"].as_str()? {
                "pedido" => Acao::Pedido(a["texto"].as_str()?.to_string()),
                "abrir" => Acao::Abrir(a["apps"].as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
                _ => return None,
            },
            gatilho: Gatilho::from_json(&v["gatilho"])?,
            ativa: v["ativa"].as_bool().unwrap_or(true),
            criado: v["criado"].as_i64().unwrap_or(0),
            ultima: v["ultima"].as_object().map(|u| (u["quando"].as_i64().unwrap_or(0), u["estado"].as_str().unwrap_or("").to_string(), u["resumo"].as_str().unwrap_or("").to_string())),
            disparo: v["disparo"].as_i64().unwrap_or(0),
            agente: v.get("agente").and_then(AgenteConfig::from_json),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id, "nome": self.nome, "ativa": self.ativa, "criado": self.criado, "disparo": self.disparo, "gatilho": self.gatilho.to_json(),
            "acao": match &self.acao { Acao::Pedido(t) => json!({"tipo": "pedido", "texto": t}), Acao::Abrir(a) => json!({"tipo": "abrir", "apps": a}) },
            "ultima": self.ultima.as_ref().map(|(q, e, r)| json!({"quando": q, "estado": e, "resumo": r})),
            "agente": self.agente.as_ref().map(AgenteConfig::to_json),
        })
    }

    /// The time slot due now (the latest scheduled time ≤ `agora`, at most 15 minutes ago, not run yet).
    pub fn vencida(&self, agora: &glib::DateTime) -> Option<i64> {
        let Gatilho::Horario { dias, hora, minuto } = &self.gatilho else { return None };
        let d = agora.add_days(0).ok()?;
        let slot = glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), *hora as i32, *minuto as i32, 0.0).ok()?;
        let dia_ok = dias.is_empty() || dias.contains(&(slot.day_of_week() as u8));
        let t = slot.to_unix();
        let n = agora.to_unix();
        (self.ativa && dia_ok && t <= n && n - t <= 15 * 60 && t > self.criado && t != self.disparo).then_some(t)
    }

    /// The next scheduled time (within a week).
    pub fn proxima(&self, agora: &glib::DateTime) -> Option<glib::DateTime> {
        let Gatilho::Horario { dias, hora, minuto } = &self.gatilho else { return None };
        (0..8).filter_map(|k| {
            let d = agora.add_days(k).ok()?;
            glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), *hora as i32, *minuto as i32, 0.0).ok()
        }).find(|s| s.to_unix() > agora.to_unix() && (dias.is_empty() || dias.contains(&(s.day_of_week() as u8))))
    }
}

fn arquivo() -> PathBuf {
    glib::user_config_dir().join("kalango/rotinas.json")
}

impl Rotinas {
    pub fn carregar() -> Rotinas {
        let v: Value = std::fs::read_to_string(arquivo()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
        Rotinas {
            lista: v["rotinas"].as_array().into_iter().flatten().filter_map(Rotina::from_json).collect(),
            proximo: v["proximo"].as_u64().unwrap_or(1),
            dispensadas: v["dispensadas"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        }
    }

    pub fn salvar(&self) {
        let p = arquivo();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let v = json!({"rotinas": self.lista.iter().map(Rotina::to_json).collect::<Vec<_>>(), "proximo": self.proximo, "dispensadas": self.dispensadas});
        let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    pub fn adicionar(&mut self, nome: &str, acao: Acao, gatilho: Gatilho) -> u64 {
        let id = self.proximo.max(1);
        self.proximo = id + 1;
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        self.lista.push(Rotina { id, nome: nome.trim().to_string(), acao, gatilho, ativa: true, criado: agora, ultima: None, disparo: 0, agente: None });
        id
    }
}

// ------------------------------------------------------------------ the scheduler

pub struct Agendador {
    shell: Weak<Shell>,
    /// routines running now: id → (start, the process, when it is the agent)
    pub rodando: RefCell<BTreeMap<u64, (i64, Option<u32>)>>,
    /// due while focus mode ran: (id, context)
    adiadas: RefCell<Vec<(u64, String)>>,
    espaco: RefCell<Option<String>>,
    _volumes: gio::VolumeMonitor,
    /// the agents center's page listens (redraws when a run starts or ends)
    pub ao_mudar: RefCell<Vec<Box<dyn Fn()>>>,
    iniciado: Cell<bool>,
}

impl Agendador {
    pub fn new(shell: &Rc<Shell>) -> Rc<Agendador> {
        let a = Rc::new(Agendador {
            shell: Rc::downgrade(shell), rodando: RefCell::default(), adiadas: RefCell::default(), espaco: RefCell::default(),
            _volumes: gio::VolumeMonitor::get(), ao_mudar: RefCell::default(), iniciado: Cell::new(false),
        });
        // a USB device (a drive) mounted
        let w = Rc::downgrade(&a);
        a._volumes.connect_mount_added(move |_, m| {
            let Some(a) = w.upgrade() else { return };
            let caminho = m.root().path().map(|p| p.display().to_string()).unwrap_or_default();
            let contexto = trf("(the device “{name}” was connected; it is mounted at {path})", &[("name", m.name().to_string()), ("path", caminho)]);
            a.disparar_todas(|g| *g == Gatilho::Usb, &contexto);
        });
        // entering a space (by its name)
        let w = Rc::downgrade(&a);
        shell.estado.ouvir(move || {
            let Some(a) = w.upgrade() else { return };
            let Some(s) = a.shell.upgrade() else { return };
            let nome = s.estado.espacos.borrow().iter().find(|e| e.focado).and_then(|e| e.nome.clone());
            if *a.espaco.borrow() == nome {
                return;
            }
            *a.espaco.borrow_mut() = nome.clone();
            // (the first reading at start is where the person already is, not an entry)
            if !a.iniciado.get() {
                return;
            }
            if let Some(n) = nome {
                a.disparar_todas(|g| matches!(g, Gatilho::Espaco(x) if x.eq_ignore_ascii_case(&n)), "");
            }
        });
        // the clock: every 20 s
        let w = Rc::downgrade(&a);
        glib::timeout_add_seconds_local(20, move || {
            let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
            a.relogio();
            glib::ControlFlow::Continue
        });
        let w = Rc::downgrade(&a);
        glib::timeout_add_seconds_local_once(3, move || {
            if let Some(a) = w.upgrade() {
                a.iniciado.set(true);
                a.relogio();
            }
        });
        a
    }

    fn avisar(&self) {
        for f in self.ao_mudar.borrow().iter() {
            f();
        }
    }

    fn foco_ativo(&self) -> bool {
        self.shell.upgrade().is_some_and(|s| s.today_bar().foco_estado().2)
    }

    fn relogio(self: &Rc<Self>) {
        // what waited for focus mode to end
        if !self.foco_ativo() {
            let adiadas: Vec<(u64, String)> = self.adiadas.borrow_mut().drain(..).collect();
            for (id, contexto) in adiadas {
                self.disparar(id, &contexto);
            }
        }
        let Ok(agora) = glib::DateTime::now_local() else { return };
        let mut r = Rotinas::carregar();
        let vencidas: Vec<(u64, i64)> = r.lista.iter().filter_map(|x| x.vencida(&agora).map(|t| (x.id, t))).collect();
        if vencidas.is_empty() {
            return;
        }
        for (id, t) in &vencidas {
            if let Some(x) = r.lista.iter_mut().find(|x| x.id == *id) {
                x.disparo = *t;
            }
        }
        r.salvar();
        for (id, _) in vencidas {
            self.disparar(id, "");
        }
    }

    fn disparar_todas(self: &Rc<Self>, f: impl Fn(&Gatilho) -> bool, contexto: &str) {
        let ids: Vec<u64> = Rotinas::carregar().lista.iter().filter(|r| r.ativa && f(&r.gatilho)).map(|r| r.id).collect();
        for id in ids {
            self.disparar(id, contexto);
        }
    }

    /// Runs a routine now (from its trigger, or "Rodar agora"). Waits while focus mode runs; one run at a time each.
    pub fn disparar(self: &Rc<Self>, id: u64, contexto: &str) {
        let Some(r) = Rotinas::carregar().lista.into_iter().find(|r| r.id == id) else { return };
        if self.rodando.borrow().contains_key(&id) {
            return;
        }
        if self.foco_ativo() {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug rotinas: {} adiada (modo foco)", r.nome);
            }
            self.adiadas.borrow_mut().push((id, contexto.to_string()));
            return;
        }
        let inicio = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug rotinas: {} começou", r.nome);
        }
        match r.acao.clone() {
            Acao::Abrir(ids) => {
                let Some(s) = self.shell.upgrade() else { return };
                let mut abertos = 0;
                let mut erro = None;
                for app in ids.iter().filter_map(|i| s.todos.iter().find(|a| a.id == *i)) {
                    match crate::apps::abrir(app) {
                        Ok(()) => abertos += 1,
                        Err(e) => erro = Some(e),
                    }
                }
                let (estado, resumo) = match erro {
                    Some(e) => ("falhou", e),
                    None => ("ok", if abertos == 1 { tr("opened 1 app").to_string() } else { trf("opened {n} apps", &[("n", abertos.to_string())]) }),
                };
                self.terminou(&r, inicio, estado, &resumo, 0, vec![]);
            }
            Acao::Pedido(texto) => {
                let pedido = if contexto.is_empty() { texto } else { format!("{texto}\n{contexto}") };
                let bin = crate::voz::agent_binary();
                let mut cmd = Command::new(&bin);
                cmd.arg("--json");
                if let Some(a) = &r.agente {
                    cmd.arg(format!("--desligadas={}", a.desligadas.join(","))).arg(format!("--autonomia={}", a.autonomia));
                    for (opcao, l) in [("pastas", &a.pastas), ("programas", &a.programas), ("dominios", &a.dominios)] {
                        if !l.is_empty() {
                            cmd.arg(format!("--{opcao}={}", l.join(",")));
                        }
                    }
                    // (the model it was made to think with, when not the one the agent uses anyway)
                    if !a.modelo.is_empty() && std::path::Path::new(&a.modelo).is_file() {
                        cmd.env("KALANGO_MODELO", &a.modelo);
                    }
                }
                let child = cmd.arg(&pedido).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
                let mut child = match child {
                    Ok(c) => c,
                    Err(e) => return self.terminou(&r, inicio, "falhou", &e.to_string(), 0, vec![]),
                };
                self.rodando.borrow_mut().insert(id, (inicio, Some(child.id())));
                self.avisar();
                let w = Rc::downgrade(self);
                let out = child.stdout.take();
                crate::lateral::em_segundo_plano(move || {
                    // the agent's events: the answer, a stop or an error, the tokens, the steps with their times
                    let mut resposta = String::new();
                    let mut falha: Option<String> = None;
                    let mut pediu: Vec<String> = Vec::new();
                    let mut tokens = 0u64;
                    let mut passos: Vec<Value> = Vec::new();
                    let agora = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                    for l in std::io::BufReader::new(out.unwrap()).lines().map_while(Result::ok) {
                        let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
                        match v["tipo"].as_str().unwrap_or("") {
                            "resposta" => resposta = v["texto"].as_str().unwrap_or("").to_string(),
                            "erro" => falha = Some(v["texto"].as_str().unwrap_or("").to_string()),
                            "parado" => falha = Some(v["motivo"].as_str().unwrap_or("").to_string()),
                            "pedido" => pediu.push(v["descricao"].as_str().unwrap_or("").to_string()),
                            "uso" => tokens = v["tokens"].as_u64().unwrap_or(tokens),
                            "ferramenta" => passos.push(json!({"marca": "◌", "texto": v["descricao"].as_str().unwrap_or(""), "t": agora()})),
                            "resultado" => if let Some(p) = passos.last_mut() {
                                p["marca"] = json!(if v["ok"].as_bool().unwrap_or(false) { "✓" } else { "✗" });
                            },
                            _ => {}
                        }
                    }
                    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
                    (ok, resposta, falha, pediu, tokens, passos)
                }, move |(ok, resposta, falha, pediu, tokens, passos)| {
                    let Some(a) = w.upgrade() else { return };
                    a.rodando.borrow_mut().remove(&id);
                    let (estado, resumo) = match falha {
                        Some(f) => ("falhou", f),
                        None if !ok => ("falhou", tr("the agent ended with an error").to_string()),
                        // asked for something nobody was there to allow: done without it
                        None if !pediu.is_empty() => ("falhou", trf("needed approval: {what}", &[("what", pediu.join("; "))])),
                        None => ("ok", resposta.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(90).collect()),
                    };
                    a.terminou(&r, inicio, estado, &resumo, tokens, passos);
                });
            }
        }
    }

    /// Stops a running routine (the agents center's stop button).
    pub fn parar(&self, id: u64) {
        if let Some((_, Some(pid))) = self.rodando.borrow().get(&id).cloned() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }

    fn terminou(self: &Rc<Self>, r: &Rotina, inicio: i64, estado: &str, resumo: &str, tokens: u64, passos: Vec<Value>) {
        let fim = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let mut todas = Rotinas::carregar();
        if let Some(x) = todas.lista.iter_mut().find(|x| x.id == r.id) {
            x.ultima = Some((fim, estado.to_string(), resumo.to_string()));
        }
        todas.salvar();
        // the agents' task history (Em execução › done or failed)
        crate::conversa::registrar_tarefa(&json!({
            "pedido": r.nome, "rotina": r.nome, "inicio": inicio, "fim": fim, "estado": if estado == "ok" { "concluido" } else { "falhou" },
            "motivo": if estado == "ok" { "" } else { resumo }, "tokens": tokens, "passos": passos,
        }));
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug rotinas: {} terminou ({estado}: {resumo})", r.nome);
        }
        if estado != "ok" {
            if let Some(s) = self.shell.upgrade() {
                s.notificar(tr("Routines"), &trf("“{name}” did not finish", &[("name", r.nome.clone())]), resumo);
            }
        } else if crate::agent_prefs::load().routine_notice == "sempre" {
            // (Agents › Settings › Notifications › Routines: "Always")
            if let Some(s) = self.shell.upgrade() {
                s.notificar(tr("Routines"), &trf("“{name}” ran", &[("name", r.nome.clone())]), resumo);
            }
        }
        self.avisar();
    }
}

// ------------------------------------------------------------------ suggestions (patterns in what the person does)

pub struct Sugestao {
    pub chave: String,
    pub titulo: String,
    pub texto: String,
    pub nome: String,
    pub acao: Acao,
    pub gatilho: Gatilho,
}

/// From the apps the person opens (the last 30 days): one opened at a regular time every day, and two opened
/// together (within 3 minutes, on 4 different days or more). Not the ones dismissed or already a routine.
pub fn sugestoes(shell: &Shell, r: &Rotinas) -> Vec<Sugestao> {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let usos: Vec<(String, i64)> = crate::apps::uses().into_iter().filter(|(_, t)| agora - t <= 30 * 86_400).collect();
    let nome = |id: &str| shell.todos.iter().find(|a| a.id == id).map(|a| a.nome.clone());
    let hora = |t: i64| glib::DateTime::from_unix_local(t).map(|d| d.hour() as u32).unwrap_or(0);
    let dia = |t: i64| glib::DateTime::from_unix_local(t).map(|d| d.day_of_year()).unwrap_or(0);
    let ja = |apps: &[String]| r.lista.iter().any(|x| matches!(&x.acao, Acao::Abrir(a) if apps.iter().all(|i| a.contains(i))));
    let mut v = Vec::new();
    let mut por_app: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (id, t) in &usos {
        por_app.entry(id).or_default().push(*t);
    }
    for (id, ts) in &por_app {
        if ts.len() < 6 {
            continue;
        }
        let horas: Vec<u32> = ts.iter().map(|t| hora(*t)).collect();
        let Some((ini, len)) = crate::central_memoria::janela_de_horas(&horas) else { continue };
        if len > 3 {
            continue;
        }
        let (Some(n), chave) = (nome(id), format!("habito:{id}")) else { continue };
        if r.dispensadas.contains(&chave) || ja(&[id.to_string()]) {
            continue;
        }
        v.push(Sugestao {
            chave, titulo: trf("You open {app} every day around {hour}h", &[("app", n.clone()), ("hour", ini.to_string())]),
            texto: trf("Automate: open {app} on weekdays at {hour}:00.", &[("app", n.clone()), ("hour", format!("{ini:02}"))]),
            nome: trf("Open {app}", &[("app", n)]), acao: Acao::Abrir(vec![id.to_string()]),
            gatilho: Gatilho::Horario { dias: vec![1, 2, 3, 4, 5], hora: ini as u8, minuto: 0 },
        });
    }
    // pairs opened together
    let mut pares: BTreeMap<(String, String), (Vec<i32>, Vec<u32>)> = BTreeMap::new();
    for (i, (a, ta)) in usos.iter().enumerate() {
        for (b, tb) in usos[i + 1..].iter().take_while(|(_, tb)| tb - ta <= 180) {
            if a != b {
                let k = if a < b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
                let e = pares.entry(k).or_default();
                e.0.push(dia(*ta));
                e.1.push(hora(*ta));
                let _ = tb;
            }
        }
    }
    for ((a, b), (dias, horas)) in pares {
        let mut d = dias.clone();
        d.sort();
        d.dedup();
        if d.len() < 4 {
            continue;
        }
        let (Some(na), Some(nb)) = (nome(&a), nome(&b)) else { continue };
        let chave = format!("par:{a}:{b}");
        if r.dispensadas.contains(&chave) || ja(&[a.clone(), b.clone()]) {
            continue;
        }
        let mut hs = horas.clone();
        hs.sort();
        let h = hs[hs.len() / 2];
        v.push(Sugestao {
            chave, titulo: trf("You open {a} and {b} together", &[("a", na.clone()), ("b", nb.clone())]),
            texto: trf("Create a routine that opens both on weekdays at {hour}:00.", &[("hour", format!("{h:02}"))]),
            nome: trf("Open {a} and {b}", &[("a", na), ("b", nb)]), acao: Acao::Abrir(vec![a, b]),
            gatilho: Gatilho::Horario { dias: vec![1, 2, 3, 4, 5], hora: h as u8, minuto: 0 },
        });
    }
    // an app that is part of a pair is suggested only in the pair
    let em_par: Vec<String> = v.iter().filter(|x| x.chave.starts_with("par:")).flat_map(|x| match &x.acao { Acao::Abrir(a) => a.clone(), _ => vec![] }).collect();
    v.retain(|x| !x.chave.starts_with("habito:") || !matches!(&x.acao, Acao::Abrir(a) if a.iter().any(|i| em_par.contains(i))));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rotina(dias: Vec<u8>, hora: u8, minuto: u8) -> Rotina {
        Rotina { id: 1, nome: "x".into(), acao: Acao::Pedido("p".into()), gatilho: Gatilho::Horario { dias, hora, minuto }, ativa: true, criado: 0, ultima: None, disparo: 0, agente: None }
    }

    #[test]
    fn a_slot_runs_once_and_only_on_its_days() {
        // Friday 15 Nov 2024, 18:05
        let agora = glib::DateTime::from_local(2024, 11, 15, 18, 5, 0.0).unwrap();
        let mut r = rotina(vec![5], 18, 0);
        let slot = r.vencida(&agora).expect("due five minutes ago");
        r.disparo = slot;
        assert!(r.vencida(&agora).is_none(), "the same slot runs once");
        assert!(rotina(vec![1], 18, 0).vencida(&agora).is_none(), "not on Monday");
        assert!(rotina(vec![], 17, 0).vencida(&agora).is_none(), "an hour ago is too late");
        let mut parada = rotina(vec![], 18, 0);
        parada.ativa = false;
        assert!(parada.vencida(&agora).is_none());
        // next: Monday 9:00 from Friday evening
        let p = rotina(vec![1], 9, 0).proxima(&agora).unwrap();
        assert_eq!((p.day_of_week(), p.day_of_month(), p.hour()), (1, 18, 9));
    }

    #[test]
    fn stored_and_read_back() {
        let mut r = Rotinas::default();
        r.adicionar("Faxina", Acao::Pedido("Organize ~/Downloads".into()), Gatilho::Horario { dias: vec![5], hora: 18, minuto: 0 });
        r.adicionar("Pendrive", Acao::Pedido("Confira".into()), Gatilho::Usb);
        r.adicionar("Dev", Acao::Abrir(vec!["a".into(), "b".into()]), Gatilho::Espaco("Desenvolver".into()));
        let v = json!({"rotinas": r.lista.iter().map(Rotina::to_json).collect::<Vec<_>>()});
        let lidas: Vec<Rotina> = v["rotinas"].as_array().unwrap().iter().filter_map(Rotina::from_json).collect();
        assert_eq!(lidas, r.lista);
    }
}
