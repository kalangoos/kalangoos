//! Um cliente MCP mínimo (stdio: JSON-RPC 2.0, uma mensagem por linha) para o `<app>-cli mcp` de um app criativo.
//! Quem o usa é o kalangod: ele inicia o processo, guarda a sessão (o documento do servidor vive entre as chamadas) e
//! o encerra quando a sessão fica parada. Nada do que o app responde vira instrução: volta ao agente como dado.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PROTOCOLO: &str = "2025-06-18";

/// O que encerra uma sessão de fora, sem esperar a sessão: quem a usa segura o cadeado dela durante a chamada (que pode
/// durar minutos), e o interruptor "Agentes podem usar este app" não pode esperar por isso.
///
/// O sinal vai por um pidfd aberto logo depois de iniciar o filho: ele fica preso àquele processo, então, depois de o
/// filho ser recolhido, o número dele pode ser de outro processo e o sinal não vai a lugar nenhum. Sem pidfd (um
/// kernel antigo), vale um cadeado: quem recolhe o filho marca `recolhido` sob ele, e quem mata confere e envia sob ele.
#[derive(Clone)]
pub struct Controle {
    pid: u32,
    pidfd: Option<Arc<OwnedFd>>,
    cancelado: Arc<AtomicBool>,
    recolhido: Arc<Mutex<bool>>,
}

impl Controle {
    /// Mata o servidor e faz a chamada em curso voltar já com erro. Devolve se um sinal foi enviado (não é, se o filho já
    /// tinha sido recolhido).
    pub fn encerrar(&self) -> bool {
        self.cancelado.store(true, Ordering::SeqCst);
        if let Some(fd) = &self.pidfd {
            // (um processo que já acabou responde ESRCH: nada é enviado a ninguém)
            return unsafe { libc::syscall(libc::SYS_pidfd_send_signal, fd.as_raw_fd(), libc::SIGKILL, std::ptr::null::<libc::c_void>(), 0u32) } == 0;
        }
        let recolhido = self.recolhido.lock().unwrap_or_else(|e| e.into_inner());
        if *recolhido {
            return false;
        }
        unsafe { libc::kill(self.pid as i32, libc::SIGKILL) == 0 }
    }

    pub fn encerrada(&self) -> bool {
        self.cancelado.load(Ordering::SeqCst)
    }
}

/// Abre um pidfd do filho (o filho ainda não foi recolhido: o número é dele).
fn abrir_pidfd(pid: u32) -> Option<Arc<OwnedFd>> {
    #[cfg(test)]
    if SEM_PIDFD.load(Ordering::SeqCst) {
        return None;
    }
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    (fd >= 0).then(|| Arc::new(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
}

#[cfg(test)]
static SEM_PIDFD: AtomicBool = AtomicBool::new(false);

pub struct Sessao {
    filho: Child,
    entrada: Option<ChildStdin>,
    cancelado: Arc<AtomicBool>,
    recolhido: Arc<Mutex<bool>>,
    pidfd: Option<Arc<OwnedFd>>,
    linhas: Receiver<String>,
    proximo: u64,
    pub ferramentas: Vec<Value>,
    pub instrucoes: String,
    pub usada: Instant,
}

impl Sessao {
    /// Inicia o servidor e faz o aperto de mão (initialize, initialized, tools/list).
    pub fn iniciar(mut cmd: Command, prazo: Duration) -> Result<Sessao, String> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut filho = cmd.spawn().map_err(|e| format!("não iniciou: {e}"))?;
        let entrada = filho.stdin.take().ok_or("sem entrada")?;
        let saida = filho.stdout.take().ok_or("sem saída")?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for l in BufReader::new(saida).lines() {
                let Ok(l) = l else { break };
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        let pidfd = abrir_pidfd(filho.id());
        let mut s = Sessao {
            filho,
            entrada: Some(entrada),
            cancelado: Arc::new(AtomicBool::new(false)),
            recolhido: Arc::new(Mutex::new(false)),
            pidfd,
            linhas: rx,
            proximo: 1,
            ferramentas: Vec::new(),
            instrucoes: String::new(),
            usada: Instant::now(),
        };
        let r = s.pedir(
            "initialize",
            json!({"protocolVersion": PROTOCOLO, "capabilities": {}, "clientInfo": {"name": "kalangod", "version": "1"}}),
            prazo,
        )?;
        s.instrucoes = r["instructions"].as_str().unwrap_or("").to_string();
        s.avisar("notifications/initialized", json!({}))?;
        let t = s.pedir("tools/list", json!({}), prazo)?;
        s.ferramentas = t["tools"].as_array().cloned().unwrap_or_default();
        Ok(s)
    }

    fn escrever(&mut self, v: &Value) -> Result<(), String> {
        let mut l = serde_json::to_string(v).map_err(|e| e.to_string())?;
        l.push('\n');
        let e = self.entrada.as_mut().ok_or("o servidor fechou")?;
        e.write_all(l.as_bytes()).and_then(|_| e.flush()).map_err(|e| format!("o servidor fechou: {e}"))
    }

    pub fn avisar(&mut self, metodo: &str, params: Value) -> Result<(), String> {
        self.escrever(&json!({"jsonrpc": "2.0", "method": metodo, "params": params}))
    }

    /// Um pedido e a sua resposta (as notificações do servidor no meio são ignoradas).
    pub fn pedir(&mut self, metodo: &str, params: Value, prazo: Duration) -> Result<Value, String> {
        if self.cancelado.load(Ordering::SeqCst) {
            return Err("a sessão com o app foi encerrada".into());
        }
        let id = self.proximo;
        self.proximo += 1;
        self.escrever(&json!({"jsonrpc": "2.0", "id": id, "method": metodo, "params": params}))?;
        let fim = Instant::now() + prazo;
        loop {
            if self.cancelado.load(Ordering::SeqCst) {
                return Err("a sessão com o app foi encerrada".into());
            }
            let resta = fim.saturating_duration_since(Instant::now());
            // (em fatias: quem encerra a sessão de fora é atendido logo, mesmo com o servidor mudo)
            let l = match self.linhas.recv_timeout(resta.min(Duration::from_millis(100))) {
                Ok(l) => l,
                Err(RecvTimeoutError::Timeout) if !resta.is_zero() => continue,
                Err(RecvTimeoutError::Timeout) => return Err(format!("sem resposta do app em {} s", prazo.as_secs())),
                Err(RecvTimeoutError::Disconnected) => return Err("o servidor do app terminou".into()),
            };
            let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            self.usada = Instant::now();
            if let Some(e) = v.get("error") {
                return Err(e["message"].as_str().unwrap_or("erro do app").to_string());
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// `tools/call`.
    pub fn chamar(&mut self, ferramenta: &str, argumentos: Value, prazo: Duration) -> Result<Value, String> {
        self.pedir("tools/call", json!({"name": ferramenta, "arguments": argumentos}), prazo)
    }

    /// Se o filho acabou, recolhe-o (e marca, sob o cadeado, que o número dele já não é dele).
    fn recolher_se_saiu(&mut self) -> bool {
        let mut recolhido = self.recolhido.lock().unwrap_or_else(|e| e.into_inner());
        match self.filho.try_wait() {
            Ok(None) => false,
            _ => {
                *recolhido = true;
                true
            }
        }
    }

    pub fn viva(&mut self) -> bool {
        !self.recolher_se_saiu()
    }

    pub fn controle(&self) -> Controle {
        Controle { pid: self.filho.id(), pidfd: self.pidfd.clone(), cancelado: self.cancelado.clone(), recolhido: self.recolhido.clone() }
    }

    pub fn pid(&self) -> u32 {
        self.filho.id()
    }
}

impl Drop for Sessao {
    fn drop(&mut self) {
        // (fechar a entrada encerra um servidor MCP; o que não sair em 2 s é morto; uma sessão encerrada de fora já foi)
        self.entrada.take();
        let fim = Instant::now() + if self.cancelado.load(Ordering::SeqCst) { Duration::ZERO } else { Duration::from_secs(2) };
        while Instant::now() < fim {
            if self.recolher_se_saiu() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut recolhido = self.recolhido.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.filho.kill();
        let _ = self.filho.wait();
        *recolhido = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny MCP server in the shell: answers initialize, tools/list and tools/call by id.
    const FALSO: &str = r#"while IFS= read -r l; do
  id=$(printf '%s' "$l" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$l" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","instructions":"oi"}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","method":"notifications/message","params":{}}\n{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"doc_inspect"},{"name":"ui_click"}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"ok"}]}}\n' "$id";;
  esac
done"#;

    #[test]
    fn the_client_does_the_handshake_and_calls_a_tool() {
        let mut c = Command::new("sh");
        c.args(["-c", FALSO]);
        let mut s = Sessao::iniciar(c, Duration::from_secs(5)).unwrap();
        assert_eq!(s.instrucoes, "oi");
        let nomes: Vec<&str> = s.ferramentas.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(nomes, ["doc_inspect", "ui_click"]);
        let r = s.chamar("doc_inspect", json!({}), Duration::from_secs(5)).unwrap();
        assert_eq!(r["content"][0]["text"], "ok");
        assert!(s.viva());
    }

    #[test]
    fn a_silent_server_times_out() {
        let mut c = Command::new("sh");
        c.args(["-c", "cat >/dev/null"]);
        let e = Sessao::iniciar(c, Duration::from_millis(300)).err().unwrap();
        assert!(e.contains("sem resposta"), "{e}");
    }

    #[test]
    fn closing_from_outside_ends_a_call_that_is_stuck() {
        // answers the handshake, then never answers tools/call
        let preso = FALSO.replace(r#"*'"tools/call"'*) printf"#, r#"*'"tools/call"'*) exec sleep 300; printf"#);
        assert_ne!(preso, FALSO);
        let mut c = Command::new("sh");
        c.args(["-c", &preso]);
        let mut s = Sessao::iniciar(c, Duration::from_secs(5)).unwrap();
        let ctl = s.controle();
        let t = Instant::now();
        let h = std::thread::spawn(move || s.chamar("doc_inspect", json!({}), Duration::from_secs(120)));
        std::thread::sleep(Duration::from_millis(300));
        ctl.encerrar();
        let r = h.join().unwrap();
        assert!(r.is_err() && ctl.encerrada(), "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(5), "the stuck call ended at once, not after its 120 s: {:?}", t.elapsed());
    }

    /// A session whose server exits on its first tool call.
    fn sessao_que_sai() -> Sessao {
        let sai = FALSO.replace(r#"*'"tools/call"'*) printf"#, r#"*'"tools/call"'*) exit 0; printf"#);
        assert_ne!(sai, FALSO);
        let mut c = Command::new("sh");
        c.args(["-c", &sai]);
        Sessao::iniciar(c, Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn killing_after_the_child_was_reaped_signals_nobody() {
        for sem_pidfd in [false, true] {
            SEM_PIDFD.store(sem_pidfd, Ordering::SeqCst);
            // a live server: the signal goes out
            let vivo = sessao_que_sai();
            assert_eq!(vivo.pidfd.is_none(), sem_pidfd);
            assert!(vivo.controle().encerrar(), "live (sem_pidfd={sem_pidfd})");
            drop(vivo);
            // the server exits, the session reaps it (viva) and then kills from outside: nothing is sent
            let mut s = sessao_que_sai();
            let ctl = s.controle();
            let _ = s.chamar("doc_inspect", json!({}), Duration::from_secs(5));
            let t = Instant::now();
            while s.viva() && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!s.viva());
            assert!(!ctl.encerrar(), "reaped by viva (sem_pidfd={sem_pidfd})");
            // reaped by the drop: a control that outlives the session signals nobody either
            let s2 = sessao_que_sai();
            let ctl2 = s2.controle();
            drop(s2);
            assert!(!ctl2.encerrar(), "reaped by drop (sem_pidfd={sem_pidfd})");
        }
        SEM_PIDFD.store(false, Ordering::SeqCst);
    }
}
