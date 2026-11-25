//! A real creative app through kalangod (ignored by default: it needs an app installed from the project's signed
//! images into a test Flatpak installation). Run it with
//!   KALANGO_CRAFT_FLATPAK_USER_DIR=<a flatpak user dir with ai.storyteller.anicraft> \
//!   cargo test -p kalangod --test craft_real -- --ignored --nocapture
//! It lists the installed creative apps and their tools with the levels, runs a reading tool (free: no approval), and
//! checks that an unknown tool and a window tool are not run without approval.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_kalangod");

fn rpc(w: &mut UnixStream, r: &mut BufReader<UnixStream>, n: &mut u64, method: &str, params: Value) -> Value {
    *n += 1;
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": *n, "method": method, "params": params})).unwrap();
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
#[ignore]
fn a_real_creative_app_answers_an_inspect_tool_through_kalangod() {
    let Some(fdir) = std::env::var_os("KALANGO_CRAFT_FLATPAK_USER_DIR") else {
        panic!("set KALANGO_CRAFT_FLATPAK_USER_DIR to a Flatpak user installation with AniCraft");
    };
    let dir = tempfile::tempdir().unwrap();
    let casa = dir.path().join("casa");
    // the app's folders in the test home (AniCraft sees Videos, Pictures, Documents by default)
    for p in ["Videos", "Pictures", "Documents", "Documentos"] {
        std::fs::create_dir_all(casa.join(p)).unwrap();
    }
    let sock = dir.path().join("mcp.sock");
    let cfg = format!(r#"home = "{casa}"
socket = "{sock}"
state_dir = "{state}"
protected_paths = ["~/.ssh"]
approval_requires_tty = false
approval_timeout_s = 3
total_grant_s = 900
dry_run = true
restore_root = "{raiz}"
trash_dir = "{raiz}/lixeira"
[[callers]]
name = "teste"
max_level = "usuario"
allowed_paths = ["{casa}"]
"#, casa = casa.display(), sock = sock.display(), state = dir.path().join("estado").display(), raiz = dir.path().join("raiz").display());
    std::fs::create_dir_all(dir.path().join("raiz/etc")).unwrap();
    let cfg_path = dir.path().join("kalangod.toml");
    std::fs::write(&cfg_path, cfg).unwrap();
    let mut child = Command::new(BIN).env("LANGUAGE", "pt_BR").env("KALANGO_CRAFT_FLATPAK_USER_DIR", &fdir).args(["servir", "--config", cfg_path.to_str().unwrap()]).spawn().unwrap();
    let s = (0..200).find_map(|_| {
        std::thread::sleep(Duration::from_millis(50));
        UnixStream::connect(&sock).ok()
    }).expect("kalangod listening");
    let (mut w, mut r, mut n) = (s.try_clone().unwrap(), BufReader::new(s), 0u64);
    rpc(&mut w, &mut r, &mut n, "initialize", json!({"protocolVersion": "2025-06-18"}));
    // the tools are there
    let list = rpc(&mut w, &mut r, &mut n, "tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"craft.apps") && names.contains(&"craft.chamar"), "{names:?}");
    // the installed apps, with each tool's level
    let t0 = std::time::Instant::now();
    let apps = rpc(&mut w, &mut r, &mut n, "tools/call", json!({"name": "craft.apps", "arguments": {"app": "anicraft", "ferramentas": true}}));
    let v = &apps["result"]["structuredContent"];
    println!("craft.apps ({} ms): {}", t0.elapsed().as_millis(), serde_json::to_string_pretty(v).unwrap().chars().take(3000).collect::<String>());
    let ani = v["apps"].as_array().unwrap().iter().find(|a| a["id"] == "ai.storyteller.anicraft").expect("AniCraft installed");
    assert!(ani["erro"].is_null(), "{}", ani["erro"]);
    let tools = ani["ferramentas"].as_array().unwrap();
    let nivel = |n: &str| tools.iter().find(|t| t["nome"] == n).map(|t| t["nivel"].as_str().unwrap().to_string());
    assert_eq!(nivel("anicraft_document").as_deref(), Some("livre"));
    // an inspect tool: free, it runs at once and answers from the app
    let t1 = std::time::Instant::now();
    let doc = rpc(&mut w, &mut r, &mut n, "tools/call", json!({"name": "craft.chamar", "arguments": {"app": "anicraft", "ferramenta": "anicraft_document", "argumentos": {}}}));
    println!("anicraft_document ({} ms): {}", t1.elapsed().as_millis(), serde_json::to_string(&doc["result"]).unwrap().chars().take(1500).collect::<String>());
    assert_eq!(doc["result"]["isError"], false, "{doc}");
    assert_eq!(doc["result"]["structuredContent"]["nivel"], "livre");
    // a tool KalangoOS does not know: asks for approval (nobody answers here: refused, never run)
    let unk = rpc(&mut w, &mut r, &mut n, "tools/call", json!({"name": "craft.chamar", "arguments": {"app": "anicraft", "ferramenta": "anicraft_new_tool", "argumentos": {}}}));
    assert_eq!(unk["result"]["isError"], true, "{unk}");
    // a command the catalog has not checked: approval too
    let run = rpc(&mut w, &mut r, &mut n, "tools/call", json!({"name": "craft.chamar", "arguments": {"app": "anicraft", "ferramenta": "anicraft_run", "argumentos": {"cmd": "file.export_png", "params": {"path": "/tmp/x.png"}}}}));
    assert_eq!(run["result"]["isError"], true, "{run}");
    // all in the timeline
    let tl = rpc(&mut w, &mut r, &mut n, "tools/call", json!({"name": "linha_do_tempo.listar", "arguments": {}}));
    let text = tl.to_string();
    assert!(text.contains("craft.chamar"), "{}", text.chars().take(800).collect::<String>());
    let _ = child.kill();
    let _ = child.wait();
}
