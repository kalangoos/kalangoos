//! The queue and the network coming and going. `KALANGO_REDE` is process-wide, so this file has one test.
mod common;

use common::*;
use kalango_modelos::catalog::{ModelEntry, ModelFile};
use kalango_modelos::download::{Options, Source};
use kalango_modelos::queue::{Phase, Queue};
use kalango_modelos::urlpolicy::parse_server;
use std::time::{Duration, Instant};

#[test]
fn without_network_it_waits_and_goes_on_by_itself_when_the_network_returns() {
    let a = bytes(80_000, 1);
    let e = ModelEntry { id: "a".into(), nome: "a".into(), descricao: String::new(), tipo: "texto".into(), arquivos: vec![ModelFile { url: String::new(), nome: "a.gguf".into(), bytes: a.len() as u64, sha256: sha256(&a) }] };
    let s = Server::start();
    s.put("k/a.gguf", a.clone());
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    std::env::set_var("KALANGO_REDE", "down");
    let d = tempfile::tempdir().unwrap();
    let src = Source::Server { url: parse_server(&format!("http://127.0.0.1:{dead}/k"), false).unwrap(), token: None };
    let q = Queue::start(d.path().to_path_buf(), vec![e], src, || Options { max_attempts: 2, backoff: Duration::from_millis(10), connect_timeout: Duration::from_secs(1), ..Options::default() });
    q.enqueue("a");
    let t0 = Instant::now();
    while q.item("a").unwrap().phase != Phase::WaitingNetwork && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(q.item("a").unwrap().phase, Phase::WaitingNetwork);
    // the network returns (and the server answers)
    q.set_source(Source::Server { url: parse_server(&format!("{}/k", s.base()), false).unwrap(), token: None });
    std::env::set_var("KALANGO_REDE", "up");
    let t0 = Instant::now();
    while !matches!(q.item("a").unwrap().phase, Phase::Installed { .. }) && t0.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: true });
    q.stop();
}
