//! The greetd protocol (the login daemon the greeter talks to, on GREETD_SOCK): JSON messages, each with its length
//! as a 4-byte native-endian prefix. One session at a time: create it for a person, answer what PAM asks (the
//! password), start it with the chosen session's command. Blocking: used off the interface thread.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

/// What greetd answered.
#[derive(Debug, PartialEq)]
pub enum Resposta {
    /// The step is done (authenticated, or the session will start).
    Sucesso,
    /// PAM asks something: (secret?, the question). Secret: the password.
    Pergunta { segredo: bool, texto: String },
    /// PAM says something to show (info, or an error message).
    Mensagem { erro: bool, texto: String },
    /// The authentication failed (wrong password) or something else went wrong.
    Erro { autenticacao: bool, texto: String },
}

pub struct Greetd {
    s: UnixStream,
}

impl Greetd {
    pub fn conectar(caminho: &Path) -> Result<Greetd, String> {
        UnixStream::connect(caminho).map(|s| Greetd { s }).map_err(|e| format!("greetd ({}): {e}", caminho.display()))
    }

    #[cfg(test)]
    pub fn de(s: UnixStream) -> Greetd {
        Greetd { s }
    }

    fn pedir(&mut self, v: Value) -> Result<Resposta, String> {
        let corpo = v.to_string();
        self.s.write_all(&(corpo.len() as u32).to_ne_bytes()).map_err(|e| e.to_string())?;
        self.s.write_all(corpo.as_bytes()).map_err(|e| e.to_string())?;
        let mut n = [0u8; 4];
        self.s.read_exact(&mut n).map_err(|e| e.to_string())?;
        let n = u32::from_ne_bytes(n) as usize;
        if n > 1 << 20 {
            return Err("greetd: resposta grande demais".into());
        }
        let mut b = vec![0u8; n];
        self.s.read_exact(&mut b).map_err(|e| e.to_string())?;
        let r: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        Ok(match r["type"].as_str() {
            Some("success") => Resposta::Sucesso,
            Some("auth_message") => {
                let texto = r["auth_message"].as_str().unwrap_or("").to_string();
                match r["auth_message_type"].as_str() {
                    Some("secret") => Resposta::Pergunta { segredo: true, texto },
                    Some("visible") => Resposta::Pergunta { segredo: false, texto },
                    Some("error") => Resposta::Mensagem { erro: true, texto },
                    _ => Resposta::Mensagem { erro: false, texto },
                }
            }
            _ => Resposta::Erro { autenticacao: r["error_type"] == "auth_error", texto: r["description"].as_str().unwrap_or("").to_string() },
        })
    }

    pub fn criar_sessao(&mut self, usuario: &str) -> Result<Resposta, String> {
        self.pedir(json!({"type": "create_session", "username": usuario}))
    }

    /// The answer to PAM's question (None for an informational message).
    pub fn responder(&mut self, resposta: Option<&str>) -> Result<Resposta, String> {
        self.pedir(json!({"type": "post_auth_message_response", "response": resposta}))
    }

    pub fn iniciar(&mut self, cmd: &[String], env: &[String]) -> Result<Resposta, String> {
        self.pedir(json!({"type": "start_session", "cmd": cmd, "env": env}))
    }

    pub fn cancelar(&mut self) -> Result<Resposta, String> {
        self.pedir(json!({"type": "cancel_session"}))
    }
}

/// The whole login: a session for `usuario`, `senha` for every secret question (greetd may ask more than once).
/// Ok(()) when greetd accepted it; Err((wrong_password, message)) otherwise (the session is cancelled).
pub fn autenticar(g: &mut Greetd, usuario: &str, senha: &str) -> Result<(), (bool, String)> {
    let mut r = g.criar_sessao(usuario).map_err(|e| (false, e))?;
    for _ in 0..8 {
        r = match r {
            Resposta::Sucesso => return Ok(()),
            Resposta::Pergunta { segredo: true, .. } => g.responder(Some(senha)).map_err(|e| (false, e))?,
            // a visible question (rare: an OTP prompt, a username): the password is not the answer
            Resposta::Pergunta { segredo: false, texto } => {
                let _ = g.cancelar();
                return Err((false, texto));
            }
            Resposta::Mensagem { .. } => g.responder(None).map_err(|e| (false, e))?,
            Resposta::Erro { autenticacao, texto } => {
                let _ = g.cancelar();
                return Err((autenticacao, texto));
            }
        };
    }
    let _ = g.cancelar();
    Err((false, "greetd: autenticação longa demais".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// A tiny greetd: a secret question, the right password succeeds, a wrong one is an auth_error.
    fn servidor(s: UnixStream, senha: &'static str) -> thread::JoinHandle<Vec<Value>> {
        thread::spawn(move || {
            let mut s = s;
            let mut visto = Vec::new();
            loop {
                let mut n = [0u8; 4];
                if s.read_exact(&mut n).is_err() {
                    return visto;
                }
                let mut b = vec![0u8; u32::from_ne_bytes(n) as usize];
                s.read_exact(&mut b).unwrap();
                let v: Value = serde_json::from_slice(&b).unwrap();
                let r = match v["type"].as_str().unwrap() {
                    "create_session" => json!({"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password: "}),
                    "post_auth_message_response" if v["response"] == senha => json!({"type": "success"}),
                    "post_auth_message_response" => json!({"type": "error", "error_type": "auth_error", "description": "pam_authenticate"}),
                    _ => json!({"type": "success"}),
                };
                visto.push(v);
                let c = r.to_string();
                s.write_all(&(c.len() as u32).to_ne_bytes()).unwrap();
                s.write_all(c.as_bytes()).unwrap();
            }
        })
    }

    #[test]
    fn logs_in_and_refuses_a_wrong_password() {
        let (a, b) = UnixStream::pair().unwrap();
        let t = servidor(b, "certa");
        let mut g = Greetd::de(a);
        assert_eq!(autenticar(&mut g, "ana", "errada"), Err((true, "pam_authenticate".into())));
        assert_eq!(autenticar(&mut g, "ana", "certa"), Ok(()));
        assert_eq!(g.iniciar(&["niri-session".into()], &["XDG_SESSION_TYPE=wayland".into()]), Ok(Resposta::Sucesso));
        drop(g);
        let visto = t.join().unwrap();
        // after the wrong password the session was cancelled before the next one
        let tipos: Vec<&str> = visto.iter().map(|v| v["type"].as_str().unwrap()).collect();
        assert_eq!(tipos, ["create_session", "post_auth_message_response", "cancel_session", "create_session", "post_auth_message_response", "start_session"]);
        assert_eq!(visto[5]["cmd"][0], "niri-session");
    }
}
