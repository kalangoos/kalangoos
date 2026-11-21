//! KalangoOS · tela de aprovação do sistema.
//!
//! Fica ligada na sessão e espera pedidos do kalangod (`kalango/aguardar`). Cada pedido de nível sistema ou total
//! aparece numa superfície layer-shell na camada mais alta, com teclado exclusivo, mostrando quem pede, a ferramenta,
//! o que ela faz, o efeito, se é reversível e os argumentos exatos. Só esta tela (executável confiável, de dono root)
//! e um terminal aprovam sem `kalangod aprovar` — o kalangod confere pelo caminho do executável que o kernel informa.
//!
//! Proteções da própria tela:
//! - não pode ser inspecionada nem controlada por outro processo do mesmo usuário (PR_SET_DUMPABLE = 0);
//! - os botões só respondem 0,7 s depois de o pedido aparecer (contra clique acidental ou induzido);
//! - Enter sozinho não aprova: Ctrl+Enter aprova, Esc nega.
//!
//! Movimento (página "11 · Motion"): o pedido nasce da pílula de agentes, no alto à direita. O fundo escurece e o
//! cartão cresce de lá com a mola suave; decidido, volta com a mola rápida. Um pedido seguinte troca o conteúdo
//! com um toque curto, sem a tela sumir.
//!
//! Uso: `kalango-aprovacao` (sessão Wayland). `KALANGO_APROVACAO_JANELA=1` abre como janela comum (X11, testes).

use gtk::prelude::*;
use kalango_i18n::{tr, trf};
use gtk::{gdk, gio, glib, Align, Application, ApplicationWindow, Box as GBox, Button, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use kalango_movimento::{self as mv, Animavel, Canal};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ESPERA_BOTOES: Duration = Duration::from_millis(700);

fn sock() -> PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
}

fn rpc_on(s: &UnixStream, method: &str, params: Value) -> Result<Value, String> {
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    if line.is_empty() {
        return Err("kalangod fechou a conexão".into());
    }
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("erro").to_string()),
        None => Ok(v["result"].clone()),
    }
}

fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let s = UnixStream::connect(sock()).map_err(|e| format!("kalangod indisponível: {e}"))?;
    rpc_on(&s, method, params)
}

/// Thread que espera mudanças na fila de pedidos e manda a lista nova para a interface.
fn watcher(tx: mpsc::Sender<Result<Vec<Value>, String>>) {
    std::thread::spawn(move || loop {
        let conn = UnixStream::connect(sock());
        let Ok(s) = conn else {
            let _ = tx.send(Err("kalangod indisponível".into()));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let mut gen = u64::MAX;
        loop {
            match rpc_on(&s, "kalango/aguardar", json!({"geracao": gen, "max_s": 30})) {
                Ok(r) => {
                    gen = r["geracao"].as_u64().unwrap_or(u64::MAX);
                    if tx.send(Ok(r["pendentes"].as_array().cloned().unwrap_or_default())).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    std::thread::sleep(Duration::from_secs(1));
                    break;
                }
            }
        }
    });
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn label(text: &str, class: &str) -> Label {
    let l = Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    // o cartão tem largura fixa: textos longos quebram em vez de alargá-lo
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

struct Ui {
    win: ApplicationWindow,
    risco: Label,
    frase: Label,
    comando: Label,
    escopo: Label,
    rodape: Label,
    erro: Label,
    aprovar: Button,
    negar: Button,
    sempre: Button,
    /// a file that runs at login (design page 30): the warning and the exact diff, shown instead of the command
    aviso: GBox,
    aviso_texto: Label,
    diff: gtk::ScrolledWindow,
    diff_linhas: GBox,
    atual: RefCell<Option<Value>>,
    camada: bool,
    fundo: Animavel,
    cartao: Animavel,
    desde: Cell<Instant>,
    /// how many more requests wait after this one
    waiting: Cell<usize>,
    /// a request was already showing when this one came (the card changes without leaving)
    havia: Cell<bool>,
}

/// The agent's name in "name (/path/to/exe)".
fn agent_name(caller: &str) -> String {
    caller.split(" (").next().unwrap_or(caller).to_string()
}

/// The request as the command it amounts to: "program args…" for a program run, else the tool and its arguments.
fn command_text(tool: &str, args: &Value) -> String {
    let word = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    };
    if let Some(prog) = args["programa"].as_str() {
        let rest = word(&args["argumentos"]);
        return if rest.is_empty() { prog.to_string() } else { format!("{prog} {rest}") };
    }
    let Some(map) = args.as_object() else { return tool.to_string() };
    let mut lines = vec![tool.to_string()];
    for (k, v) in map {
        lines.push(format!("  {k}: {}", word(v)));
    }
    let text = lines.join("\n");
    if text.chars().count() > 900 { format!("{}…", text.chars().take(900).collect::<String>()) } else { text }
}

/// The warning over a change to files that run at login: a shell's file runs in every terminal, the rest at login.
fn startup_warning(files: &[String]) -> String {
    let first = files.first().map(|f| f.rsplit(" → ").next().unwrap_or(f).to_string()).unwrap_or_default();
    let name = first.rsplit('/').next().unwrap_or(&first);
    let terminal = [".bashrc", ".bash_aliases", ".zshrc", ".zshenv"].contains(&name) || first.contains("/.config/fish");
    if terminal {
        trf("{file} runs every time you open a terminal. What is in it runs as you, without asking again.", &[("file", first)])
    } else {
        trf("{file} runs when you log in. What is in it runs as you, without asking again.", &[("file", first)])
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Tools of the agents platform (kalangod's tools/plataforma.rs) whose approval shows a preview.
fn is_platform_tool(tool: &str) -> bool {
    matches!(tool, "agentes.criar" | "agentes.editar" | "fluxos.criar" | "fluxos.editar" | "ferramentas.criar_http" | "mcp.registrar")
}

/// The preview lines as markup: the first line in bold, "· item" lines as they are, and the lines saying what was
/// removed or tightened (everything after the items) in the error colour.
fn platform_markup(reason: &str) -> String {
    let esc = |t: &str| glib::markup_escape_text(t).to_string();
    let mut out = Vec::new();
    let mut seen_item = false;
    for (i, l) in reason.lines().enumerate() {
        if i == 0 {
            out.push(format!("<b>{}</b>", esc(l)));
        } else if l.starts_with("· ") {
            seen_item = true;
            out.push(esc(l));
        } else if seen_item {
            out.push(format!("<span foreground=\"#E06C75\">{}</span>", esc(l)));
        } else {
            out.push(esc(l));
        }
    }
    out.join("\n")
}

impl Ui {
    fn mostrar(self: &Rc<Self>, lista: &[Value]) {
        let Some(p) = lista.first() else {
            if self.atual.borrow_mut().take().is_none() {
                return;
            }
            if self.camada {
                self.win.set_keyboard_mode(KeyboardMode::None);
            }
            // volta para a pílula de agentes com a mola rápida e só então some
            self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
            let me = self.clone();
            self.cartao.sair(self.ate_pilula(), 0.3, move || {
                if me.atual.borrow().is_none() {
                    me.win.set_visible(false);
                    // (for the session test: the screen is free again)
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug aprovação: fechada");
                    }
                }
            });
            return;
        };
        self.waiting.set(lista.len() - 1);
        let mesmo = self.atual.borrow().as_ref().map(|a| a["id"] == p["id"]).unwrap_or(false);
        self.havia.set(self.atual.borrow().is_some());
        if mesmo {
            return;
        }
        *self.atual.borrow_mut() = Some(p.clone());
        let nivel = p["level"].as_str().unwrap_or("");
        let efeito = p["efeito"].as_str().unwrap_or("");
        let reversivel = p["reversivel"] == true;
        let alto = nivel == "total" || efeito == "destrutiva" || !reversivel;
        self.risco.set_text(if alto { tr("High risk") } else { tr("Medium risk") });
        self.risco.set_css_classes(&["risco", if alto { "alto" } else { "medio" }]);
        let agente = agent_name(p["caller"].as_str().unwrap_or("?"));
        let tool = p["tool"].as_str().unwrap_or("");
        self.aviso.set_visible(false);
        self.diff.set_visible(false);
        self.comando.set_visible(true);
        // a file that runs at login (design page 30): high risk, what it does, the exact diff; never "always"
        if nivel == "inicio" {
            self.risco.set_text(tr("High risk"));
            self.risco.set_css_classes(&["risco", "alto"]);
            self.frase.set_text(&trf("The agent {agent} wants to change a file that runs when you log in:", &[("agent", agente)]));
            let st = &p["startup"];
            let files: Vec<String> = st["arquivos"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
            self.aviso_texto.set_text(&if st["outro_nome"] == true {
                trf("{file} has another name (a hard link) and may be a file that runs when you log in: what is in it may run as you.", &[("file", files.first().cloned().unwrap_or_default())])
            } else {
                startup_warning(&files)
            });
            self.aviso.set_visible(true);
            self.comando.set_visible(false);
            self.sempre.set_visible(false);
            self.show_diff(&files, st);
            let what = st["o_que"].as_str().map(capitalized).unwrap_or_default();
            self.escopo.set_text(&[what, tr("can be undone in the timeline").into(), tr("startup files have no “Always”: each change asks").into()]
                .into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "));
            self.aprovar.set_label(tr("Allow once"));
            self.sempre.set_visible(false);
            self.mostrar_comum(p);
            return;
        }
        // controlling the screen (design page 29): the window, the time and what the agent may do; never "always"
        if tool == "tela.controlar" {
            self.risco.set_text(tr("Medium risk"));
            self.risco.set_css_classes(&["risco", "medio"]);
            let a = &p["args"];
            let quem = a["agente"].as_str().map(str::to_string).unwrap_or(agente);
            self.frase.set_text(&trf("The agent {agent} wants to control the screen:", &[("agent", quem)]));
            let minutos = a["minutos"].as_u64().unwrap_or(10).to_string();
            self.comando.set_text(&[
                trf("Window: {w}", &[("w", a["janela"].as_str().unwrap_or("").to_string())]),
                trf("For {n} min · up to {max} actions", &[("n", minutos.clone()), ("max", a["acoes"].as_u64().unwrap_or(0).to_string())]),
                tr("Click, type, shortcuts, scroll and drag").to_string(),
            ].join("\n"));
            self.escopo.set_text(tr("Never: approvals, passwords, the lock screen, Settings › Privacy and Users · moving your mouse or typing stops it"));
            self.aprovar.set_label(&trf("Allow for {n} min", &[("n", minutos)]));
            self.sempre.set_visible(false);
            self.mostrar_comum(p);
            return;
        }
        self.aprovar.set_label(tr("Allow once"));
        self.frase.set_text(&if nivel == "total" {
            trf("The agent {agent} asks for full access to use “{tool}”:", &[("agent", agente), ("tool", tool.to_string())])
        } else {
            trf("The agent {agent} wants to change the system with “{tool}”:", &[("agent", agente), ("tool", tool.to_string())])
        });
        // the agents platform: kalangod's reason is the exact preview of what will exist (already fitted to the
        // agent's ceiling): it is shown instead of the raw arguments, with what was removed in red
        let plataforma = is_platform_tool(tool);
        if plataforma {
            self.comando.set_markup(&platform_markup(p["reason"].as_str().unwrap_or("")));
        } else {
            self.comando.set_text(&command_text(tool, &p["args"]));
        }
        let descricao = p["descricao"].as_str().unwrap_or("").to_string();
        let mut escopo = vec![descricao];
        if plataforma {
            escopo.push(tr("marked “created by the agent”").into());
            escopo.push(tr("can be undone from the timeline").into());
        } else if nivel == "total" {
            escopo.push(trf("full access for {n} min", &[("n", "15".into())]));
            escopo.push(tr("protected folders stay out of reach").into());
        } else {
            escopo.push(if reversivel { tr("can be undone").into() } else { tr("cannot be undone").into() });
            escopo.push(tr("restore point first").into());
        }
        self.escopo.set_text(&escopo.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "));
        // "always" covers only this same request, and never full access (it is always temporary)
        // (and never for what an agent creates: each agent, flow or connector is seen before it exists)
        self.sempre.set_visible(nivel == "sistema" && !plataforma);
        self.mostrar_comum(p);
    }

    /// The files, then each line of the diff: kept, removed (coral) or added (teal); a gap as "⋯".
    fn show_diff(&self, files: &[String], st: &Value) {
        while let Some(c) = self.diff_linhas.first_child() {
            self.diff_linhas.remove(&c);
        }
        let head = Label::new(Some(&format!("{} · {}", files.join(", "), st["resumo"].as_str().unwrap_or(""))));
        head.add_css_class("diff-arquivo");
        head.set_xalign(0.0);
        head.set_wrap(true);
        head.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        head.set_max_width_chars(1);
        head.set_hexpand(true);
        self.diff_linhas.append(&head);
        let lines = st["linhas"].as_array().cloned().unwrap_or_default();
        for l in &lines {
            let kind = l["t"].as_str().unwrap_or(" ");
            let row = GBox::new(Orientation::Horizontal, 8);
            row.add_css_class("diff-linha");
            let (sign, class) = match kind {
                "+" => ("+", Some("mais")),
                "-" => ("-", Some("menos")),
                "@" => ("⋯", Some("intervalo")),
                _ => (" ", None),
            };
            if let Some(c) = class {
                row.add_css_class(c);
            }
            let a = Label::new(Some(sign));
            a.add_css_class("diff-sinal");
            row.append(&a);
            let b = Label::new(Some(l["l"].as_str().unwrap_or("")));
            b.add_css_class("diff-texto");
            b.set_xalign(0.0);
            b.set_hexpand(true);
            b.set_wrap(true);
            b.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            b.set_max_width_chars(1);
            row.append(&b);
            self.diff_linhas.append(&row);
        }
        self.diff.set_visible(true);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() || std::env::var_os("KALANGO_APROVACAO_DEBUG").is_some() {
            let ch: Vec<String> = lines.iter().filter(|l| l["t"] != " ").map(|l| format!("{}{}", l["t"].as_str().unwrap_or(""), l["l"].as_str().unwrap_or(""))).collect();
            eprintln!("debug aprovação: arquivo de início {} · {} · sempre visível {}", files.join(", "), ch.join(" | "), self.sempre.get_visible());
        }
    }

    /// What every request shares: the error line, the deadline, the buttons held for a moment, the card's motion.
    fn mostrar_comum(self: &Rc<Self>, p: &Value) {
        let havia = self.havia.get();
        self.erro.set_text("");
        self.erro.set_visible(false);
        self.atualizar_prazo();
        // botões travados por um instante: um pedido que aparece debaixo do cursor não é aprovado sem querer
        self.desde.set(Instant::now());
        self.set_buttons(false);
        let me = self.clone();
        let id = p["id"].clone();
        glib::timeout_add_local_once(ESPERA_BOTOES, move || {
            if me.atual.borrow().as_ref().map(|a| a["id"] == id).unwrap_or(false) {
                me.set_buttons(true);
            }
        });
        self.win.present();
        if havia {
            // troca de pedido com a tela aberta: um toque curto no cartão (rápida), sem sumir
            self.cartao.definir(Canal::Escala, 0.985);
            self.cartao.ir(Canal::Escala, 1.0, mv::RAPIDA);
        } else {
            self.fundo.definir(Canal::Opacidade, 0.0);
            self.fundo.ir(Canal::Opacidade, 1.0, mv::SUAVE);
            self.cartao.definir(Canal::Opacidade, 0.0);
            let me = self.clone();
            // o tamanho da tela só existe depois de mapear
            glib::idle_add_local_once(move || me.cartao.entrar(me.ate_pilula(), 0.3));
        }
        if std::env::var_os("KALANGO_APROVACAO_DEBUG").is_some() {
            let me = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(500), move || {
                eprintln!("debug: visível={} mapeada={} {}x{} fundo.op={:.2} cartao.op={:.2} esc={:.2} x={:.0} y={:.0} anim={}",
                    me.win.is_visible(), me.win.is_mapped(), me.win.width(), me.win.height(), me.fundo.valor(Canal::Opacidade),
                    me.cartao.valor(Canal::Opacidade), me.cartao.valor(Canal::Escala), me.cartao.valor(Canal::X), me.cartao.valor(Canal::Y), me.cartao.em_movimento());
            });
        }
        // test aid: where "Always" is on the output (the screen covers it whole)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some_and(|v| !v.is_empty()) || std::env::var_os("KALANGO_APROVACAO_DEBUG").is_some() {
            let me = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                if let Some(r) = me.sempre.compute_bounds(&me.win).filter(|_| me.sempre.is_mapped()) {
                    eprintln!("debug aprovação: sempre em {} {}", (r.x() + r.width() / 2.) as i32, (r.y() + r.height() / 2.) as i32);
                }
            });
        }
        self.negar.grab_focus();
        if self.camada {
            // pede o teclado exclusivo de novo depois de mapear: o compositor reavalia o foco
            let me = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(60), move || me.win.set_keyboard_mode(KeyboardMode::Exclusive));
        }
    }

    /// Deslocamento do centro da tela até a pílula de agentes (barra superior, à direita).
    fn ate_pilula(&self) -> (f64, f64) {
        let (w, h) = (self.win.width() as f64, self.win.height() as f64);
        if w < 10.0 {
            return (0.0, -40.0);
        }
        (w / 2.0 - 200.0, -(h / 2.0 - 16.0))
    }

    fn set_buttons(&self, on: bool) {
        for b in [&self.aprovar, &self.negar, &self.sempre] {
            b.set_sensitive(on);
        }
    }

    /// The small line under the buttons: when no answer becomes "no", and how many more requests wait.
    fn atualizar_prazo(&self) {
        if let Some(p) = self.atual.borrow().as_ref() {
            let fim = p["criado"].as_u64().unwrap_or(0) + p["prazo_s"].as_u64().unwrap_or(0);
            let resta = fim.saturating_sub(now());
            let mut t = trf("no answer means denied in {s} s", &[("s", resta.to_string())]);
            match self.waiting.get() {
                0 => {}
                1 => t = format!("{t} · {}", tr("+1 request waiting")),
                n => t = format!("{t} · {}", trf("+{n} requests waiting", &[("n", n.to_string())])),
            }
            self.rodape.set_text(&t);
        }
    }

    fn decidir(&self, aprovar: bool, sempre: bool) {
        if self.desde.get().elapsed() < ESPERA_BOTOES {
            eprintln!("kalango-aprovacao: ignorado: o pedido acabou de aparecer");
            return;
        }
        let Some(id) = self.atual.borrow().as_ref().and_then(|p| p["id"].as_u64()) else { return };
        if std::env::var_os("KALANGO_APROVACAO_DEMO").is_some() {
            eprintln!("demo: #{id} {}", if sempre { "aprovado (sempre)" } else if aprovar { "aprovado" } else { "negado" });
            decision_sound(aprovar);
            // some e volta com o próximo pedido, como acontece de verdade
            let me: Rc<Ui> = DEMO.with(|d| d.borrow().clone().unwrap());
            me.mostrar(&[]);
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                let mut p = demo_pedido();
                p["id"] = json!(id + 1);
                me.mostrar(&[p]);
            });
            return;
        }
        match rpc("kalango/decidir", json!({"id": id, "aprovar": aprovar, "sempre": sempre})) {
            Ok(_) => {
                self.set_buttons(false);
                decision_sound(aprovar);
            }
            Err(e) => {
                eprintln!("kalango-aprovacao: decisão recusada: {e}");
                self.erro.set_text(&e);
                self.erro.set_visible(true);
            }
        }
    }
}

/// org.gtk.Actions.Activate's arguments for an action with a string parameter: `(sava{sv})`.
fn activate_params(action: &str, param: &str) -> glib::Variant {
    let empty: std::collections::HashMap<String, glib::Variant> = Default::default();
    glib::Variant::tuple_from_iter([
        action.to_variant(),
        glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, [param.to_variant().to_variant()]),
        empty.to_variant(),
    ])
}

/// The decision's sound, played by the shell (this screen loads no audio code: nothing more runs inside it): the
/// shell's app action `play-sound`, over the session bus, fire-and-forget.
fn decision_sound(approved: bool) {
    let id = if approved { "kalango-agent-approved" } else { "kalango-agent-denied" };
    let app = std::env::var("KALANGO_SHELL_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "os.kalango.Shell".into());
    let path = format!("/{}", app.replace('.', "/"));
    let Ok(c) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    c.call(Some(&app), &path, "org.gtk.Actions", "Activate", Some(&activate_params("play-sound", id)), None, gio::DBusCallFlags::NO_AUTO_START, 2000, gio::Cancellable::NONE, |r| {
        if let Err(e) = r {
            eprintln!("kalango-aprovacao: som da decisão: {e}");
        }
    });
}

thread_local! {
    static DEMO: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

#[cfg(test)]
mod platform_tests {
    use super::{is_platform_tool, platform_markup};

    #[test]
    fn previews_of_the_platform() {
        assert!(is_platform_tool("agentes.criar") && !is_platform_tool("pacotes.remover"));
        let m = platform_markup("criar o agente \"A & B\"\ninstruções: x\n· web.ler — rodar livre\narquivos.apagar removida");
        assert!(m.starts_with("<b>criar o agente &quot;A &amp; B&quot;</b>"), "{m}");
        assert!(m.ends_with("<span foreground=\"#E06C75\">arquivos.apagar removida</span>"), "{m}");
    }
}

fn demo_pedido() -> Value {
    // (KALANGO_APROVACAO_DEMO=inicio: a change to a file that runs at login, as in design page 30)
    if std::env::var("KALANGO_APROVACAO_DEMO").as_deref() == Ok("inicio") {
        let l = |t: &str, x: &str| json!({"t": t, "l": x});
        return json!({"id": 7, "tool": "arquivos.escrever", "level": "inicio", "caller": "Assistente (/usr/libexec/kalango/agente)",
            "descricao": "", "efeito": "escreve", "reversivel": true, "no_always": true, "args": {"caminho": "~/.bashrc"}, "reason": "", "criado": now(), "prazo_s": 120,
            "startup": {"arquivos": ["~/.bashrc"], "o_que": tr("Writes a file"), "resumo": tr("1 line changes"),
                "linhas": [l(" ", "# aliases do terminal"), l(" ", "alias la='ls -A'"), l("-", "alias ll='ls -l'"), l("+", "alias ll='ls -alF'"), l(" ", "alias l='ls -CF'")]}});
    }
    json!({"id": 7, "tool": "pacotes.remover", "level": "sistema", "caller": "Organizador (/usr/libexec/kalango/agente)",
        "descricao": tr("Removes packages (apt)"), "efeito": "destrutiva", "reversivel": true,
        "args": {"nomes": ["libreoffice-draw", "hplip"]}, "reason": "mudança no sistema", "criado": now(), "prazo_s": 120})
}

/// Whether the person chose the light theme ("automatico": light from 7 h to 19 h), as the shell decides it.
fn light_theme() -> bool {
    let v: Value = std::fs::read(glib::user_config_dir().join("kalango/tema.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    match v["estilo"].as_str() {
        Some("claro") => true,
        Some("automatico") => glib::DateTime::now_local().map(|d| (7..19).contains(&d.hour())).unwrap_or(false),
        _ => false,
    }
}

fn build(app: &Application) {
    let css = gtk::CssProvider::new();
    // the person's theme (Settings › Appearance, the shell's tema.json): the light tokens on a light theme (design
    // "Aprovação · controlar a tela · claro", page 29)
    let tokens = if light_theme() { concat!(include_str!("../../kalango-shell/assets/tokens-light.css"), "\n.cartao { border-color: @k_border; }") } else { include_str!("../assets/tokens-dark.css") };
    css.load_from_string(&format!("{tokens}\n{}", include_str!("../assets/style.css")));
    // above the person's own ~/.config/gtk-4.0/gtk.css (USER priority): a desktop theme from elsewhere ("button { border-radius: 12px }")
    // must not redraw the system's interface
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);

    let win = ApplicationWindow::builder().application(app).title(tr("Approval · KalangoOS")).build();
    // a lista de protocolos do compositor pode ainda não ter chegado: sincroniza antes de concluir que não há layer-shell
    if let Some(d) = gdk::Display::default() {
        for _ in 0..20 {
            if gtk4_layer_shell::is_supported() {
                break;
            }
            d.sync();
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let janela = std::env::var_os("KALANGO_APROVACAO_JANELA").is_some() || !gtk4_layer_shell::is_supported();
    if janela {
        win.set_default_size(760, 640);
    } else {
        win.init_layer_shell();
        win.set_namespace(Some("kalango-aprovacao"));
        win.set_layer(Layer::Overlay);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            win.set_anchor(e, true);
        }
        win.set_exclusive_zone(-1);
        // teclado só para esta tela enquanto houver pedido: nada digitado vai para outro app
        win.set_keyboard_mode(KeyboardMode::Exclusive);
    }

    let fundo = GBox::new(Orientation::Vertical, 0);
    fundo.add_css_class("fundo");
    // design "Aprovação" (page 06): 460 px card, gold border, the exact command in mono
    let cartao = GBox::new(Orientation::Vertical, 10);
    cartao.add_css_class("cartao");
    cartao.set_size_request(460, -1);

    let topo = GBox::new(Orientation::Horizontal, 8);
    let escudo = gtk::Image::from_paintable(Some(&gdk::Texture::from_bytes(&glib::Bytes::from_owned(
        include_str!("../../kalango-shell/assets/ui/escudo.svg").replace("#8E8F99", if light_theme() { "#7A5611" } else { "#E5C98C" }).into_bytes())).unwrap()));
    escudo.set_pixel_size(16);
    topo.append(&escudo);
    let titulo = label(tr("Permission needed"), "titulo");
    titulo.set_hexpand(true);
    topo.append(&titulo);
    let risco = Label::new(None);
    risco.set_valign(Align::Center);
    topo.append(&risco);
    let frase = label("", "frase");
    let comando = label("", "comando");
    comando.set_selectable(false);
    let escopo = label("", "escopo");
    let erro = label("", "erro");
    erro.set_visible(false);
    // a file that runs at login: the warning, and the diff (scrolls past a few lines)
    let aviso = GBox::new(Orientation::Horizontal, 8);
    aviso.add_css_class("aviso-inicio");
    let escudo_aviso = gtk::Image::from_paintable(Some(&gdk::Texture::from_bytes(&glib::Bytes::from_owned(
        include_str!("../../kalango-shell/assets/ui/escudo.svg").replace("#8E8F99", if light_theme() { "#7A5611" } else { "#E5C98C" }).into_bytes())).unwrap()));
    escudo_aviso.set_pixel_size(16);
    escudo_aviso.set_valign(Align::Start);
    aviso.append(&escudo_aviso);
    let aviso_texto = label("", "aviso-texto");
    aviso.append(&aviso_texto);
    aviso.set_visible(false);
    let diff_linhas = GBox::new(Orientation::Vertical, 4);
    diff_linhas.add_css_class("diff");
    let diff = gtk::ScrolledWindow::new();
    diff.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    diff.set_propagate_natural_height(true);
    diff.set_max_content_height(300);
    diff.set_child(Some(&diff_linhas));
    diff.set_visible(false);

    let acoes = GBox::new(Orientation::Horizontal, 8);
    acoes.set_homogeneous(true);
    let negar = Button::with_label(tr("Deny"));
    negar.set_tooltip_text(Some(tr("Esc")));
    let aprovar = Button::with_label(tr("Allow once"));
    aprovar.set_tooltip_text(Some(tr("Ctrl+Enter (Enter alone does not approve)")));
    aprovar.add_css_class("aprovar");
    let sempre = Button::with_label(tr("Always"));
    sempre.add_css_class("sempre");
    sempre.set_tooltip_text(Some(tr("Allow, and do not ask again for this exact request (it can be taken back in Agents)")));
    acoes.append(&negar);
    acoes.append(&aprovar);
    acoes.append(&sempre);
    let rodape = label("", "prazo");

    for w in [topo.upcast_ref::<gtk::Widget>(), frase.upcast_ref(), aviso.upcast_ref(), comando.upcast_ref(), diff.upcast_ref(), escopo.upcast_ref(), erro.upcast_ref(), acoes.upcast_ref(), rodape.upcast_ref()] {
        cartao.append(w);
    }
    fundo.set_hexpand(true);
    fundo.set_vexpand(true);
    let raiz = gtk::Overlay::new();
    let fundo_a = Animavel::new(&fundo);
    raiz.set_child(Some(&fundo_a));
    let cartao_a = Animavel::new(&cartao);
    cartao_a.set_halign(Align::Center);
    cartao_a.set_valign(Align::Center);
    raiz.add_overlay(&cartao_a);
    win.set_child(Some(&raiz));

    let ui = Rc::new(Ui { win: win.clone(), risco, frase, comando, escopo, rodape, erro, aprovar: aprovar.clone(), negar: negar.clone(), sempre: sempre.clone(),
        aviso, aviso_texto, diff, diff_linhas, atual: RefCell::new(None), camada: !janela, fundo: fundo_a, cartao: cartao_a, desde: Cell::new(Instant::now()), waiting: Cell::new(0), havia: Cell::new(false) });
    aprovar.connect_clicked({ let ui = ui.clone(); move |_| ui.decidir(true, false) });
    negar.connect_clicked({ let ui = ui.clone(); move |_| ui.decidir(false, false) });
    sempre.connect_clicked({ let ui = ui.clone(); move |_| ui.decidir(true, true) });
    let keys = gtk::EventControllerKey::new();
    // fase de captura: o atalho vale mesmo com o foco num botão (que senão consumiria o Enter)
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let debug = std::env::var_os("KALANGO_APROVACAO_DEBUG").is_some();
    keys.connect_key_pressed({
        let ui = ui.clone();
        move |_, key, _, mods| {
            if debug {
                eprintln!("tecla {key:?} {mods:?}");
            }
            match key {
                gdk::Key::Escape => ui.decidir(false, false),
                gdk::Key::Return | gdk::Key::KP_Enter if mods.contains(gdk::ModifierType::CONTROL_MASK) => ui.decidir(true, false),
                // Enter sozinho não faz nada: aprovar sempre é um gesto deliberado
                gdk::Key::Return | gdk::Key::KP_Enter => {}
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    win.add_controller(keys);
    if debug {
        win.connect_is_active_notify(|w| eprintln!("foco: {}", w.is_active()));
    }

    let (tx, rx) = mpsc::channel();
    if std::env::var_os("KALANGO_APROVACAO_DEMO").is_some() {
        // demonstração (capturas de design e testes de teclado), sem kalangod
        DEMO.with(|d| *d.borrow_mut() = Some(ui.clone()));
        let _ = tx.send(Ok(vec![demo_pedido(), json!({"id": 8})]));
        std::mem::forget(tx);
    } else {
        watcher(tx);
    }
    glib::timeout_add_local(Duration::from_millis(80), {
        let ui = ui.clone();
        move || {
            while let Ok(m) = rx.try_recv() {
                match m {
                    Ok(lista) => ui.mostrar(&lista),
                    Err(e) => eprintln!("kalango-aprovacao: {e}"),
                }
            }
            glib::ControlFlow::Continue
        }
    });
    glib::timeout_add_seconds_local(1, { let ui = ui.clone(); move || { ui.atualizar_prazo(); glib::ControlFlow::Continue } });
}

fn main() -> glib::ExitCode {
    // ninguém do mesmo usuário (um agente, por exemplo) pode ler a memória desta tela ou controlá-la por ptrace
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    let args: Vec<String> = std::env::args().collect();
    // o mesmo binário serve de linha de comando para aprovar (útil em testes e scripts presenciais)
    if let [_, cmd, id] = args.as_slice() {
        if cmd == "aprovar" || cmd == "negar" {
            let r = id.parse::<u64>().map_err(|_| "id inválido".to_string()).and_then(|n| rpc("kalango/decidir", json!({"id": n, "aprovar": cmd == "aprovar"})));
            match r {
                Ok(v) => println!("{}", v["mensagem"].as_str().unwrap_or("")),
                Err(e) => eprintln!("{e}"),
            }
            return glib::ExitCode::SUCCESS;
        }
    }
    // uma instância por sessão; testes usam outro id para não conversar com a tela da sessão aberta
    let id = std::env::var("KALANGO_APROVACAO_APP_ID").unwrap_or_else(|_| "os.kalango.Aprovacao".into());
    let app = Application::builder().application_id(id).build();
    app.connect_activate(|app| {
        build(app);
        // a aplicação fica viva mesmo sem pedidos (janela escondida)
        std::mem::forget(app.hold());
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_request_has_the_actions_signature() {
        let v = activate_params("play-sound", "kalango-agent-approved");
        assert_eq!(v.type_().as_str(), "(sava{sv})");
        assert_eq!(v.child_value(1).child_value(0).as_variant().and_then(|x| x.get::<String>()).as_deref(), Some("kalango-agent-approved"));
    }
}
