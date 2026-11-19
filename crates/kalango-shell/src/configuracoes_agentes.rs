//! Configurações › Agentes e IA (design "Configurações · Agentes e IA"): Settings looks after the models only — which
//! one, where it runs, what is used for each kind of task; permissions, memory and usage have one place, the Agents
//! app (the panel leads there).
//!
//! - Models: the providers on this computer and their state — KalangoOS's own server (llama.cpp with LFM3, on a Unix
//!   socket: loaded or not), and Ollama and LM Studio when they answer on their usual local ports; the model for each
//!   kind of task (conversation and code share the text model, chosen here as in System › Default apps — a change
//!   restarts the server; voice and seeing images use LFM3's audio and vision models).
//! - Cloud and limits: no cloud provider — nothing leaves the computer — and the agents' daily token limit.
//! - Voice: speech to text (LFM3-Audio, local) and reading aloud (speech-dispatcher).
//! - Panel: where to manage the rest (the Agents app's pages) and how the local model runs: loaded when the session
//!   starts, and paused while a game is open (the graphics card for the game).

use crate::configuracoes::{ajustes, depurando, interruptor, salvar_ajuste};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const TABS: &[(&str, &str)] = &[("modelos", n_("Models")), ("nuvem", n_("Cloud and limits")), ("voz", n_("Voice"))];

/// Where the person's models are (and the system's).
fn models_dirs() -> Vec<PathBuf> {
    std::env::var_os("KALANGO_MODELOS").map(PathBuf::from).into_iter()
        .chain([glib::user_data_dir().join("kalango/modelos"), PathBuf::from("/usr/share/kalango/modelos")]).collect()
}

fn installed(file_part: &str) -> Option<PathBuf> {
    models_dirs().into_iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "gguf") && p.to_string_lossy().contains(file_part) && !p.to_string_lossy().contains("mmproj"))
}

/// The text models installed (not the audio, vision or projector files).
pub fn text_models() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = models_dirs().into_iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| {
            let s = p.to_string_lossy();
            p.extension().is_some_and(|x| x == "gguf") && !s.contains("mmproj") && !s.contains("Audio") && !s.contains("-VL-")
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// A model file's name as people read it ("LFM3 1.7B").
pub fn model_name(p: &Path) -> String {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // (the quantization at the end says nothing to a person)
    let base = stem.split("-Q").next().unwrap_or(&stem).to_string();
    base.replace(['-', '_'], " ")
}

/// The local model servers' sockets (as kalango-agente starts them).
fn runtime_dir() -> PathBuf {
    glib::user_runtime_dir().join("kalango")
}

fn text_socket() -> PathBuf {
    std::env::var_os("KALANGO_MODELO_SOCKET").map(PathBuf::from).unwrap_or_else(|| runtime_dir().join("modelo.sock"))
}

/// One HTTP GET (a model server's own API), with a short timeout: (status, body).
fn http_get(addr: &str, path: &str) -> Option<(u16, String)> {
    use std::io::{Read, Write};
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let mut raw = Vec::new();
    if let Some(sock) = addr.strip_prefix("unix:") {
        let mut s = std::os::unix::net::UnixStream::connect(sock).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(800))).ok();
        s.write_all(req.as_bytes()).ok()?;
        s.read_to_end(&mut raw).ok()?;
    } else {
        let a: std::net::SocketAddr = addr.parse().ok()?;
        let mut s = std::net::TcpStream::connect_timeout(&a, Duration::from_millis(400)).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(800))).ok();
        s.write_all(req.as_bytes()).ok()?;
        s.read_to_end(&mut raw).ok()?;
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, body.to_string()))
}

/// What each provider is now: (local loaded, Ollama's models, LM Studio's models) — None where it does not answer.
fn providers_now() -> (bool, Option<usize>, Option<usize>) {
    let local = http_get(&format!("unix:{}", text_socket().display()), "/health").is_some_and(|(s, _)| s == 200);
    let count = |addr: &str, path: &str, key: &str| -> Option<usize> {
        let (s, b) = http_get(addr, path)?;
        if s != 200 {
            return None;
        }
        // (a chunked body: the JSON is between the first "{" and the last "}")
        let j = b.find('{').and_then(|i| b.rfind('}').map(|k| &b[i..=k]))?;
        serde_json::from_str::<Value>(j).ok().map(|v| v[key].as_array().map(Vec::len).unwrap_or(0))
    };
    (local, count("127.0.0.1:11434", "/api/tags", "models"), count("127.0.0.1:1234", "/v1/models", "data"))
}

/// Ends the local model servers (the text one, and voice and vision with `todos`): the next request starts them again
/// (with the model chosen now). Only KalangoOS's own llama-server processes, by the socket in their command line.
pub fn stop_model_servers(todos: bool) -> usize {
    let mut socks = vec![text_socket()];
    if todos {
        socks.push(runtime_dir().join("modelo-audio.sock"));
        socks.push(runtime_dir().join("modelo-visao.sock"));
    }
    let mut n = 0;
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
        if cmd.contains("llama-server") && socks.iter().any(|s| cmd.contains(&s.display().to_string())) {
            extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            unsafe { kill(pid, 15) };
            n += 1;
        }
    }
    if depurando() {
        eprintln!("debug agentes-ia: {n} servidores do modelo parados");
    }
    n
}

/// A game opened and the person wants the graphics card for it: the local models pause (the next request starts them).
pub fn game_opened(app: &crate::apps::App) {
    if ajustes()["liberar_gpu_jogos"].as_bool() == Some(false) {
        return;
    }
    let jogo = app.info.categories().is_some_and(|c| c.split(';').any(|x| x == "Game"));
    if jogo {
        let n = stop_model_servers(true);
        if depurando() {
            eprintln!("debug agentes-ia: jogo aberto ({}), {n} modelos pausados", app.id);
        }
    }
}

/// The session started: the local model loads now when the person asked for it (the first answer comes quicker).
pub fn session_started() {
    if ajustes()["modelo_ao_iniciar"].as_bool() != Some(true) {
        return;
    }
    let _ = std::process::Command::new(crate::voz::agent_binary()).arg("modelo").stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    if depurando() {
        eprintln!("debug agentes-ia: carregando o modelo ao iniciar");
    }
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

fn card_label(glifo: &str, t: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    r.append(&ui_icon(glifo, "#8E8F99", 16));
    r.append(&rotulo(t, "cfg-rotulo"));
    r
}

/// A task row: its glyph, name and what it covers, and the control at the right.
fn task_row(glifo: &str, titulo: &str, sub: &str, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    r.add_css_class("alta");
    let g = ui_icon(glifo, "#8E8F99", 16);
    g.set_valign(Align::Center);
    r.append(&g);
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(titulo, "cfg-texto-13"));
    t.append(&left(sub, "cfg-sub"));
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

/// A value shown where a choice would be (one possibility, or nothing installed).
fn value(t: &str) -> gtk::Label {
    let l = rotulo(t, "cfg-valor-pequeno");
    l.set_xalign(1.0);
    l.set_size_request(220, -1);
    l
}

pub struct AgentsAiPage {
    pub frame: Rc<Moldura>,
    status: Vec<(gtk::Label, gtk::Label, gtk::Box)>,
}

impl AgentsAiPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<AgentsAiPage> {
        let m = Moldura::new(tr("Agents and AI"), TABS, tr("Where to manage"));
        m.busca_grande();
        let mut tabs: Vec<(&'static str, gtk::Widget)> = Vec::new();
        // ---- models: the providers
        // (as many cards in a line as fit, each at least 280 px: one under the other on a narrow window)
        let row = gtk::FlowBox::new();
        row.set_selection_mode(gtk::SelectionMode::None);
        row.set_homogeneous(true);
        row.set_min_children_per_line(1);
        row.set_max_children_per_line(3);
        row.set_column_spacing(12);
        row.set_row_spacing(12);
        let mut status = Vec::new();
        let local_model = crate::configuracoes::ajustes()["modelo_padrao"].as_str().map(PathBuf::from).filter(|p| p.is_file())
            .or_else(|| installed("LFM3-1.7B")).or_else(|| text_models().into_iter().next());
        let local_name = local_model.as_deref().map(model_name).unwrap_or_else(|| tr("no model").to_string());
        for (glifo, nome) in [("cpu", format!("KalangoOS · {local_name}")), ("cpu", "Ollama".to_string()), ("cpu", "LM Studio".to_string())] {
            let b = caixa(Orientation::Horizontal, 12, "cfg-cartao");
            b.add_css_class("cfg-provedor");
            b.set_size_request(280, -1);
            let ic = caixa(Orientation::Horizontal, 0, "cfg-provedor-icone");
            ic.set_size_request(32, 32);
            ic.set_valign(Align::Center);
            ic.set_hexpand(false);
            let g = ui_icon(glifo, "#A5A1E2", 16);
            g.set_hexpand(true);
            ic.append(&g);
            b.append(&ic);
            let t = caixa(Orientation::Vertical, 2, "");
            t.set_hexpand(true);
            t.set_valign(Align::Center);
            let n = left(&nome, "cfg-nome-13");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.append(&n);
            let s = left("", "cfg-sub");
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            t.append(&s);
            b.append(&t);
            let st = caixa(Orientation::Horizontal, 6, "");
            st.set_valign(Align::Start);
            let dot = caixa(Orientation::Horizontal, 0, "cfg-ponto");
            dot.set_size_request(6, 6);
            dot.set_valign(Align::Center);
            st.append(&dot);
            let sl = rotulo("", "cfg-provedor-estado");
            st.append(&sl);
            b.append(&st);
            row.insert(&b, -1);
            status.push((sl, s, dot));
        }
        m.corpo.append(&row);
        tabs.push(("modelos", row.upcast()));
        // ---- the model for each kind of task
        let tasks = caixa(Orientation::Vertical, 2, "cfg-cartao");
        tasks.append(&card_label("camadas", tr("DEFAULT MODEL BY KIND OF TASK")));
        let models = text_models();
        let names: Vec<String> = models.iter().map(|p| format!("{} · {}", model_name(p), tr("local"))).collect();
        let d = gtk::DropDown::new(Some(gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())), None::<gtk::Expression>);
        d.add_css_class("cfg-seletor");
        d.set_size_request(220, 34);
        d.set_sensitive(models.len() > 1);
        if let Some(i) = local_model.as_ref().and_then(|n| models.iter().position(|m| m == n)) {
            d.set_selected(i as u32);
        }
        let ms = models.clone();
        d.connect_selected_notify(move |d| {
            let Some(p) = ms.get(d.selected() as usize) else { return };
            if ajustes()["modelo_padrao"].as_str() == Some(&p.display().to_string()) {
                return;
            }
            salvar_ajuste("modelo_padrao", json!(p.display().to_string()));
            // (the running server keeps its model: it stops, and the next request starts the chosen one)
            stop_model_servers(false);
            if depurando() {
                eprintln!("debug agentes-ia: modelo de texto {}", p.display());
            }
        });
        crate::central::posicao_debug("cfg agentes modelo texto", d.upcast_ref());
        tasks.append(&task_row("enviar", tr("Conversation and writing"), tr("answers, summaries, e-mails"), &d));
        tasks.append(&task_row("codigo", tr("Code"), tr("editing files and running commands"), &value(&if models.is_empty() { tr("not installed").to_string() } else { tr("the conversation's").to_string() })));
        let audio = installed("Audio");
        tasks.append(&task_row("microfone", tr("Voice"), tr("transcription and reading aloud"), &value(&audio.as_deref().map(|p| format!("{} · {}", model_name(p), tr("local"))).unwrap_or_else(|| tr("not installed").to_string()))));
        let vision = installed("-VL-");
        tasks.append(&task_row("imagem", tr("Images"), tr("describing pictures and the screen"), &value(&vision.as_deref().map(|p| format!("{} · {}", model_name(p), tr("local"))).unwrap_or_else(|| tr("not installed").to_string()))));
        tasks.append(&task_row("globo", tr("Hard tasks"), tr("when the local model cannot cope"), &value(tr("local only · it says so"))));
        m.corpo.append(&tasks);
        tabs.push(("modelos", tasks.upcast()));
        // ---- cloud and limits
        let cloud = caixa(Orientation::Vertical, 8, "cfg-cartao");
        cloud.append(&card_label("globo", tr("CLOUD")));
        let ct = left(tr("No cloud provider: the agents use only the models on this computer, and nothing you ask leaves it."), "cfg-texto-13");
        ct.set_wrap(true);
        cloud.append(&ct);
        let limit_v = value("");
        let perms: Value = std::fs::read(glib::user_config_dir().join("kalango/agente-permissoes.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        limit_v.set_text(&perms["tokens_dia"].as_u64().filter(|n| *n > 0).map(|n| trf("{n} tokens a day", &[("n", n.to_string())])).unwrap_or_else(|| tr("no limit").to_string()));
        cloud.append(&task_row("relogio", tr("Daily limit"), tr("tokens the agents may use each day"), &limit_v));
        m.corpo.append(&cloud);
        tabs.push(("nuvem", cloud.upcast()));
        // ---- voice
        let voice = caixa(Orientation::Vertical, 2, "cfg-cartao");
        voice.append(&card_label("microfone", tr("VOICE")));
        voice.append(&task_row("microfone", tr("Speech to text"), tr("the microphone in the conversation and the keyboard's dictation"), &value(&audio.as_deref().map(|p| format!("{} · {}", model_name(p), tr("local"))).unwrap_or_else(|| tr("not installed").to_string()))));
        let spd = glib::find_program_in_path("spd-say").is_some();
        voice.append(&task_row("volume", tr("Reading aloud"), tr("“Read aloud” and the screen description"), &value(if spd { tr("system voice · local") } else { tr("not installed") })));
        m.corpo.append(&voice);
        tabs.push(("voz", voice.upcast()));
        // ---- the panel: where to manage, and how the local model runs
        let manage = caixa(Orientation::Vertical, 2, "cfg-cartao");
        let mh = caixa(Orientation::Horizontal, 5, "");
        mh.append(&crate::superficies::estrela(13, "#A5A1E2"));
        mh.append(&rotulo(tr("MANAGED IN THE AGENTS APP"), "cfg-rotulo"));
        manage.append(&mh);
        for (glifo, page, titulo, sub) in [
            ("escudo", "permissoes", tr("Permissions per agent"), tr("what each one may do without asking")),
            ("cerebro", "memoria", tr("Memory"), tr("what the system knows about you")),
            ("relogio", "uso", tr("Usage and costs"), tr("tokens, time and cloud spending")),
            ("cabo", "ajustes", tr("Connectors (MCP)"), tr("apps and services linked to the agents")),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("cfg-linha-botao");
            let r = caixa(Orientation::Horizontal, 12, "");
            r.append(&ui_icon(glifo, "#8E8F99", 16));
            let t = caixa(Orientation::Vertical, 1, "");
            t.set_hexpand(true);
            t.append(&left(titulo, "cfg-texto-13"));
            t.append(&left(sub, "cfg-sub"));
            r.append(&t);
            r.append(&ui_icon("seta-direita", "#7A7B88", 12));
            b.set_child(Some(&r));
            let cw = Rc::downgrade(c);
            b.connect_clicked(move |_| {
                if let Some(s) = cw.upgrade().and_then(|c| c.shell_publico()) {
                    s.abrir_central(Some(page));
                }
            });
            crate::central::posicao_debug(&format!("cfg agentes {page}"), b.upcast_ref());
            manage.append(&b);
        }
        let open = gtk::Button::new();
        open.add_css_class("cfg-botao-acento");
        let oc = caixa(Orientation::Horizontal, 6, "");
        oc.set_halign(Align::Center);
        oc.append(&crate::superficies::estrela(12, "#A5A1E2"));
        oc.append(&rotulo(tr("Open Agents"), ""));
        open.set_child(Some(&oc));
        let cw = Rc::downgrade(c);
        open.connect_clicked(move |_| {
            if let Some(s) = cw.upgrade().and_then(|c| c.shell_publico()) {
                s.abrir_central(None);
            }
        });
        manage.append(&open);
        m.painel.append(&manage);
        let local = caixa(Orientation::Vertical, 2, "cfg-cartao");
        local.append(&card_label("cpu", tr("LOCAL RUN")));
        for (key, titulo, sub, padrao) in [
            ("modelo_ao_iniciar", tr("Load the model at start"), None, false),
            ("liberar_gpu_jogos", tr("Free the graphics card for games"), Some(tr("pauses the model when a game opens")), true),
        ] {
            let sw = interruptor();
            sw.set_active(ajustes()[key].as_bool().unwrap_or(padrao));
            sw.connect_state_set(move |_, on| {
                salvar_ajuste(key, json!(on));
                if depurando() {
                    eprintln!("debug agentes-ia: {key} {on}");
                }
                glib::Propagation::Proceed
            });
            let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
            let t = caixa(Orientation::Vertical, 1, "");
            t.set_hexpand(true);
            t.set_valign(Align::Center);
            t.append(&left(titulo, "cfg-texto-13"));
            if let Some(s) = sub {
                t.append(&left(s, "cfg-sub"));
                r.add_css_class("alta");
            }
            r.append(&t);
            r.append(&sw);
            local.append(&r);
            crate::central::posicao_debug(&format!("cfg agentes {key}"), sw.upcast_ref());
        }
        m.painel.append(&local);
        // the tabs show their own cards
        let tabs = Rc::new(tabs);
        let t2 = tabs.clone();
        let show = move |id: &str| {
            for (t, w) in t2.iter() {
                w.set_visible(*t == id);
            }
        };
        show("modelos");
        m.ao_trocar_aba(show);
        m.sub.set_text(&trf("{n} providers · local first", &[("n", "3".into())]));
        Rc::new(AgentsAiPage { frame: m, status })
    }

    /// The page shows: the providers' state now (asked off the interface thread).
    pub fn mostrar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(providers_now, move |(local, ollama, lms)| {
            let Some(p) = w.upgrade() else { return };
            let set = |i: usize, on: bool, estado: &str, sub: &str| {
                let (sl, s, dot) = &p.status[i];
                sl.set_text(estado);
                s.set_text(sub);
                for (c, yes) in [("ligado", on), ("desligado", !on)] {
                    if yes { sl.add_css_class(c); dot.add_css_class(c) } else { sl.remove_css_class(c); dot.remove_css_class(c) }
                }
            };
            let installed_local = text_models().is_empty().then(|| tr("no model installed")).unwrap_or(tr("local · private"));
            set(0, local, if local { tr("Loaded") } else { tr("Not loaded") }, installed_local);
            let other = |n: Option<usize>| n.map(|n| trf("local · {n} models", &[("n", n.to_string())])).unwrap_or_else(|| tr("local · —").to_string());
            set(1, ollama.is_some(), if ollama.is_some() { tr("Connected") } else { tr("Disconnected") }, &other(ollama));
            set(2, lms.is_some(), if lms.is_some() { tr("Connected") } else { tr("Disconnected") }, &other(lms));
            if depurando() {
                eprintln!("debug agentes-ia: local {} · ollama {:?} · lm studio {:?}", if local { "carregado" } else { "parado" }, ollama, lms);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_people_read() {
        assert_eq!(model_name(Path::new("/m/LFM3-1.7B-Q4_K_M.gguf")), "LFM3 1.7B");
        assert_eq!(model_name(Path::new("/m/LFM3-Audio-2B-Q4_K_M.gguf")), "LFM3 Audio 2B");
    }
}
