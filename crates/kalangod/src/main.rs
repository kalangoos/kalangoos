//! kalangod · o MCP do sistema KalangoOS.
//!
//! Uso:
//!   kalangod servir [--config ARQ] [--dev PASTA] [--simular]   inicia o serviço (como root em produção)
//!   kalangod mcp                                              ponte stdio ⇄ soquete, para clientes MCP (ex.: Claude Code)
//!   kalangod pendentes                                        aprovações esperando você
//!   kalangod aprovar ID | negar ID                            decide uma aprovação (num terminal: presencial)
//!   kalangod quem                                             como o kalangod enxerga este processo
//!   kalangod ferramentas                                      ferramentas disponíveis para este processo
//!   kalangod conector testar|rodar|entrar MANIFESTO           conector MCP de terceiros (local isolado ou remoto)
//!   kalangod cofre guardar|apagar NOME | cofre listar         segredos dos conectores (chaveiro do sistema)
//!
//! O soquete padrão é /run/kalango/mcp.sock; KALANGOD_SOCKET muda isso.

mod approval;
mod cofre;
mod remoto;
mod conector;
mod guard;
mod limits;
mod peer;
mod policy;
mod redes;
mod remote_control;
mod restore;
mod sandbox;
mod server;
mod timeline;
mod tools;

use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
}

fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let s = UnixStream::connect(socket_path()).map_err(|e| trf("kalangod is not running at {socket}: {error}", &[("socket", socket_path().display().to_string()), ("error", e.to_string())]))?;
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if let Some(e) = v.get("error") {
        return Err(e["message"].as_str().unwrap_or("erro").to_string());
    }
    Ok(v["result"].clone())
}

/// Ponte para clientes MCP que falam por stdio.
fn bridge() -> std::io::Result<()> {
    let s = UnixStream::connect(socket_path())?;
    let mut to_sock = s.try_clone()?;
    let from_sock = s;
    let t = std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for line in BufReader::new(from_sock).lines().map_while(Result::ok) {
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    });
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        writeln!(to_sock, "{line}")?;
    }
    let _ = to_sock.shutdown(std::net::Shutdown::Write);
    let _ = t.join();
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let res: Result<(), String> = match a.as_slice() {
        ["__auxiliar"] => sandbox::helper_main(),
        ["__tarefa", plano] => limits::task_main(plano),
        ["servir", rest @ ..] => {
            let mut cfg = policy::Config::system_default();
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--config" => {
                        cfg = policy::Config::load(std::path::Path::new(rest.get(i + 1).copied().unwrap_or(""))).unwrap_or_else(|e| {
                            eprintln!("kalangod: {e}");
                            std::process::exit(2)
                        });
                        i += 1;
                    }
                    "--dev" => {
                        cfg = policy::Config::dev(std::path::Path::new(rest.get(i + 1).copied().unwrap_or("/tmp/kalangod")));
                        i += 1;
                    }
                    "--simular" => cfg.dry_run = true,
                    "--sem-terminal" => cfg.approval_requires_tty = false,
                    o => {
                        eprintln!("{}", trf("unknown option: {option}", &[("option", o.to_string())]));
                        std::process::exit(2)
                    }
                }
                i += 1;
            }
            server::serve(cfg).map_err(|e| e.to_string())
        }
        ["mcp"] => bridge().map_err(|e| trf("kalangod is not running at {socket}: {error}", &[("socket", socket_path().display().to_string()), ("error", e.to_string())])),
        ["pendentes"] => rpc("kalango/pendentes", json!({})).map(|v| {
            let list = v.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("{}", tr("no approvals pending"));
            }
            for p in list {
                println!("{}", trf("#{id}  {caller}  asks for {tool} (level {level})  {args}", &[("id", p["id"].to_string()), ("caller", p["caller"].as_str().unwrap_or("").to_string()), ("tool", p["tool"].as_str().unwrap_or("").to_string()), ("level", p["level"].as_str().unwrap_or("").to_string()), ("args", p["args"].to_string())]));
            }
        }),
        ["aprovar", id] | ["negar", id] | ["aprovar", id, "--sempre"] => {
            let ok = a[0] == "aprovar";
            let always = a.get(2) == Some(&"--sempre");
            id.trim_start_matches('#').parse::<u64>().map_err(|_| "id inválido".to_string())
                .and_then(|n| rpc("kalango/decidir", json!({"id": n, "aprovar": ok, "sempre": always})))
                .map(|v| println!("{}", v["mensagem"].as_str().unwrap_or("")))
        }
        ["conector", "testar", m] => conector::test(std::path::Path::new(m)).map(|v| println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default())),
        ["conector", "rodar", m] => match conector::run(std::path::Path::new(m)) {
            Ok(code) => std::process::exit(code),
            Err(e) => Err(e),
        },
        ["conector", "entrar", m] => conector::Manifesto::load(std::path::Path::new(m)).and_then(|m| remoto::login(&m))
            .map(|v| println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default())),
        ["cofre", "guardar", nome] => cofre::read_secret_stdin().and_then(|v| cofre::put(nome, &v)).map(|onde| println!("{}", trf("'{name}' saved ({place})", &[("name", nome.to_string()), ("place", onde.to_string())]))),
        ["cofre", "apagar", nome] => cofre::delete(nome).map(|ok| println!("{}", if ok { trf("'{name}' deleted", &[("name", nome.to_string())]) } else { trf("'{name}' did not exist", &[("name", nome.to_string())]) })),
        ["cofre", "listar"] => {
            for n in cofre::list() {
                println!("{n}");
            }
            Ok(())
        }
        ["quem"] => rpc("kalango/quem", json!({})).map(|v| println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default())),
        ["ferramentas"] => rpc("tools/list", json!({})).map(|v| {
            for t in v["tools"].as_array().cloned().unwrap_or_default() {
                println!("{:<26} {}", t["name"].as_str().unwrap_or(""), t["description"].as_str().unwrap_or(""));
            }
        }),
        _ => {
            eprintln!("{}", include_str!("main.rs").lines().skip(2).take(11).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
            std::process::exit(2)
        }
    };
    if let Err(e) = res {
        eprintln!("kalangod: {e}");
        std::process::exit(1);
    }
}
