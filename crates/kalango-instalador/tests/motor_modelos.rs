//! The engine, simulated (it only says the commands it would run): what it records of the AI models' source, and
//! that a private server's token never shows up in its output nor in the plan file.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_kalango-instalador");

fn run(plan: &serde_json::Value, stdin: &str) -> (String, String) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("plano.json");
    std::fs::write(&p, plan.to_string()).unwrap();
    let mut ch = Command::new(BIN).args(["--instalar", p.to_str().unwrap()]).env("KALANGO_INSTALADOR_RAPIDO", "1").env("KALANGO_INSTALADOR_LSBLK", "/nonexistent")
        .env("HOME", d.path()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    ch.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = ch.wait_with_output().unwrap();
    (String::from_utf8_lossy(&o.stdout).into_owned(), std::fs::read_to_string(&p).unwrap())
}

fn base() -> serde_json::Value {
    serde_json::json!({"disk": "/dev/vda", "disk_title": "SSD 20 GB", "mode": "Erase", "size": 20000000000u64, "encrypt": false, "locale": "pt_BR.UTF-8", "keyboard": "br",
        "name": "Ana Lima", "user": "ana", "hostname": "kalango", "autologin": false, "fingerprint_later": false})
}

fn aceite(modelo: &str, versao: &str) -> serde_json::Value {
    serde_json::json!({"modelo": modelo, "licenca": "lfm-open", "versao": versao, "data": "2026-11-24"})
}

fn written(out: &str, path: &str) -> Option<String> {
    out.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).find(|v| v["comando"].as_str() == Some(&format!("escrever /target{path}"))).map(|v| v["conteudo"].as_str().unwrap_or("").to_string())
}

#[test]
fn the_default_choice_is_recorded_in_etc() {
    let mut plan = base();
    plan["licencas"] = serde_json::json!([aceite("lfm3-1.7b", "1.0"), aceite("lfm3-audio-2b", "1.0"), aceite("lfm3-vl-2b", "1.0")]);
    let (out, _) = run(&plan, "senha-de-teste\n\n");
    let t = written(&out, "/etc/kalango/modelos.toml").expect("the engine writes /etc/kalango/modelos.toml");
    assert!(t.contains("fonte = \"padrao\""), "{t}");
    assert!(t.contains("escolhidos = [\"lfm3-1.7b\", \"lfm3-audio-2b\", \"lfm3-vl-2b\"]"), "{t}");
    assert!(out.contains("\"fim\":true"));
    assert!(!out.contains("token do servidor"), "no token, no hand-off");
}

#[test]
fn a_private_server_is_recorded_and_its_token_only_handed_off() {
    let mut plan = base();
    plan["models"] = serde_json::json!({"fonte": "servidor", "servidor": "https://modelos.example.org/kalango", "escolhidos": ["lfm3-1.7b"], "rede_local_confirmada": false});
    plan["licencas"] = serde_json::json!([aceite("lfm3-1.7b", "1.0")]);
    let (out, file) = run(&plan, "senha-de-teste\ntok-0123456789-secret\n");
    let t = written(&out, "/etc/kalango/modelos.toml").unwrap();
    assert!(t.contains("fonte = \"servidor\"") && t.contains("servidor = \"https://modelos.example.org/kalango\"") && t.contains("escolhidos = [\"lfm3-1.7b\"]"), "{t}");
    assert!(out.contains("guardar o token do servidor de modelos em /target/home/ana/.local/share/kalango/modelos-token (valor omitido)"), "{out}");
    for (nome, texto) in [("saída do motor", &out), ("arquivo do plano", &file), ("modelos.toml", &t)] {
        assert!(!texto.contains("tok-0123456789-secret"), "o token vazou em: {nome}");
        assert!(!texto.contains("senha-de-teste"), "a senha vazou em: {nome}");
    }
}

#[test]
fn later_records_that_nothing_is_to_be_downloaded() {
    let mut plan = base();
    plan["models"] = serde_json::json!({"fonte": "depois", "escolhidos": []});
    let (out, _) = run(&plan, "senha\n\n");
    let t = written(&out, "/etc/kalango/modelos.toml").unwrap();
    assert!(t.contains("fonte = \"depois\"") && t.contains("escolhidos = []"), "{t}");
}

#[test]
fn an_invalid_server_in_the_plan_does_not_get_recorded_as_valid() {
    // (the window never produces it; the engine still must not write a configuration the system would reject)
    let mut plan = base();
    plan["models"] = serde_json::json!({"fonte": "servidor", "servidor": "http://modelos.example.org", "escolhidos": ["lfm3-1.7b"]});
    let (out, _) = run(&plan, "senha\ntok\n");
    assert!(out.contains("\"erro\""), "{out}");
    assert!(written(&out, "/etc/kalango/modelos.toml").is_none());
}

#[test]
fn the_engine_refuses_models_whose_licence_was_not_accepted_and_records_the_accepted_ones() {
    // the default choice with no acceptance: nothing is installed
    let (out, _) = run(&base(), "senha\n\n");
    assert!(out.contains("a licença destes modelos") || out.contains("licence of these AI models"), "{out}");
    assert!(out.contains("lfm3-1.7b") && !out.contains("\"fim\":true"), "{out}");
    // a record for an older licence version does not count
    let mut plan = base();
    plan["licencas"] = serde_json::json!([aceite("lfm3-1.7b", "0.9"), aceite("lfm3-audio-2b", "1.0"), aceite("lfm3-vl-2b", "1.0")]);
    let (out, _) = run(&plan, "senha\n\n");
    assert!(!out.contains("\"fim\":true"), "{out}");
    // accepted: the new account gets the record (only what matches the licence in force)
    let mut plan = base();
    plan["models"] = serde_json::json!({"fonte": "padrao", "escolhidos": ["lfm3-1.7b"]});
    plan["licencas"] = serde_json::json!([aceite("lfm3-1.7b", "1.0"), aceite("lfm3-audio-2b", "0.9"), {"modelo": "outro", "licenca": "x", "versao": "1", "data": "d"}]);
    let (out, _) = run(&plan, "senha\n\n");
    assert!(out.contains("\"fim\":true"), "{out}");
    let t = written(&out, "/home/ana/.config/kalango/modelos-licencas.json").expect("the account's acceptance file");
    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["aceites"].as_array().unwrap().len(), 1, "{t}");
    assert_eq!(v["aceites"][0]["modelo"], "lfm3-1.7b");
    assert_eq!(v["aceites"][0]["data"], "2026-11-24");
}
