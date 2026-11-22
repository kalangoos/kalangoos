mod common;

use common::*;
use kalango_modelos::catalog::{ModelEntry, ModelFile};
use kalango_modelos::download::{Control, Options, Source};
use kalango_modelos::net::{Client, NetError};
use kalango_modelos::probe::{probe_folder, probe_server, ModelState};
use kalango_modelos::queue::{Failure, Phase, Queue};
use kalango_modelos::urlpolicy::parse_server;
use kalango_modelos::Secret;
use std::time::Duration;

fn entry(id: &str, files: &[(&str, &[u8])]) -> ModelEntry {
    ModelEntry {
        id: id.into(),
        nome: id.into(),
        descricao: String::new(),
        tipo: "texto".into(),
        arquivos: files.iter().map(|(n, b)| ModelFile { url: String::new(), nome: n.to_string(), bytes: b.len() as u64, sha256: sha256(b) }).collect(),
    }
}

fn client(s: &Server, token: Option<&str>) -> (Client, kalango_modelos::urlpolicy::ServerUrl) {
    let u = parse_server(&format!("{}/k", s.base()), false).unwrap();
    (Client::with_timeouts(Some(u.clone()), token.map(Secret::new), Duration::from_secs(2), Duration::from_secs(5)).with_online(|| true), u)
}

#[test]
fn the_server_test_says_per_model_what_is_there_and_whether_the_sum_matches() {
    let s = Server::start();
    let (a, b, c, d) = (bytes(1000, 1), bytes(2000, 2), bytes(3000, 3), bytes(4000, 4));
    let cat = vec![entry("ok", &[("a.gguf", &a)]), entry("bad", &[("b.gguf", &b)]), entry("gone", &[("gone.gguf", &c)]), entry("nosum", &[("d.gguf", &d)])];
    s.put("k/a.gguf", a.clone());
    s.put("k/b.gguf", b.clone());
    s.put("k/d.gguf", d.clone());
    let sums = format!("{}  a.gguf\n{}  b.gguf\n", sha256(&a), sha256(b"another content"));
    s.put("k/SHA256SUMS", sums.into_bytes());
    let (cl, u) = client(&s, None);
    let p = probe_server(&cl, &u, &cat).unwrap();
    assert_eq!(p.state("ok"), Some(ModelState::Verified));
    assert_eq!(p.state("bad"), Some(ModelState::SumMismatch));
    assert_eq!(p.state("gone"), Some(ModelState::Absent));
    assert_eq!(p.state("nosum"), Some(ModelState::Unverified));
    assert!(p.sums_found);
    assert!(!ModelState::SumMismatch.usable() && !ModelState::Absent.usable());
    assert_eq!(p.found(), 2);
    // no HEAD downloaded a body: only SHA256SUMS was a GET
    assert!(s.seen().iter().filter(|r| r.method == "GET").all(|r| r.path.ends_with("SHA256SUMS")));
}

#[test]
fn a_size_other_than_the_catalogs_is_flagged_and_a_missing_sums_file_is_tolerated() {
    let s = Server::start();
    let a = bytes(1000, 1);
    s.put("k/a.gguf", bytes(999, 1));
    let (cl, u) = client(&s, None);
    let p = probe_server(&cl, &u, &[entry("m", &[("a.gguf", &a)])]).unwrap();
    assert_eq!(p.state("m"), Some(ModelState::WrongSize));
    assert!(!p.sums_found);
}

#[test]
fn the_server_test_reports_denied_unreachable_and_never_sends_the_token_elsewhere() {
    let s = Server::start();
    *s.b.token.lock().unwrap() = Some("right".into());
    let cat = vec![entry("m", &[("a.gguf", b"x")])];
    let (cl, u) = client(&s, None);
    assert_eq!(probe_server(&cl, &u, &cat), Err(NetError::Denied(401)));
    let (cl, u) = client(&s, Some("wrong"));
    assert_eq!(probe_server(&cl, &u, &cat), Err(NetError::Denied(403)));
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let u = parse_server(&format!("http://127.0.0.1:{port}/k"), false).unwrap();
    let cl = Client::with_timeouts(Some(u.clone()), None, Duration::from_secs(1), Duration::from_secs(1)).with_online(|| true);
    assert!(matches!(probe_server(&cl, &u, &cat), Err(NetError::Unreachable(_))));
    let cl = Client::with_timeouts(Some(u.clone()), None, Duration::from_secs(1), Duration::from_secs(1)).with_online(|| false);
    assert_eq!(probe_server(&cl, &u, &cat), Err(NetError::Offline));
}

#[test]
fn a_model_whose_catalog_entry_has_no_sum_cannot_come_from_a_server() {
    let s = Server::start();
    let mut e = entry("m", &[("a.gguf", b"x")]);
    e.arquivos[0].sha256.clear();
    let (cl, u) = client(&s, None);
    assert_eq!(probe_server(&cl, &u, &[e]).unwrap().state("m"), Some(ModelState::NoChecksum));
}

#[test]
fn the_folder_test_hashes_the_files() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (bytes(5000, 1), bytes(5000, 2));
    std::fs::write(d.path().join("a.gguf"), &a).unwrap();
    std::fs::write(d.path().join("b.gguf"), bytes(5000, 9)).unwrap(); // same size, other content
    let cat = vec![entry("a", &[("a.gguf", &a)]), entry("b", &[("b.gguf", &b)]), entry("c", &[("c.gguf", &a)])];
    let p = probe_folder(d.path(), &cat, &Control::default());
    assert_eq!(p.state("a"), Some(ModelState::Verified));
    assert_eq!(p.state("b"), Some(ModelState::SumMismatch));
    assert_eq!(p.state("c"), Some(ModelState::Absent));
}

fn quick() -> Options {
    Options { max_attempts: 2, backoff: Duration::from_millis(10), max_backoff: Duration::from_millis(20), online: || true, connect_timeout: Duration::from_secs(2), ..Options::default() }
}

#[test]
fn the_queue_downloads_one_model_at_a_time_and_reports_each_outcome() {
    let s = Server::start();
    let (a, b, c) = (bytes(300_000, 1), bytes(200_000, 2), bytes(100_000, 3));
    s.put("k/a.gguf", a.clone());
    s.put("k/b.gguf", bytes(200_000, 99)); // wrong content: refused
    s.put("k/c.gguf", c.clone());
    let cat = vec![entry("a", &[("a.gguf", &a)]), entry("b", &[("b.gguf", &b)]), entry("c", &[("c.gguf", &c)])];
    let d = tempfile::tempdir().unwrap();
    let src = Source::Server { url: parse_server(&format!("{}/k", s.base()), false).unwrap(), token: None };
    let q = Queue::start(d.path().to_path_buf(), cat, src, quick);
    for id in ["a", "b", "c"] {
        q.enqueue(id);
    }
    assert!(q.wait_idle(Duration::from_secs(20)));
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: true });
    assert_eq!(q.item("b").unwrap().phase, Phase::Failed(Failure::Mismatch));
    assert_eq!(q.item("c").unwrap().phase, Phase::Installed { verified: true });
    assert!(!d.path().join("b.gguf").exists());
    assert_eq!(q.item("a").unwrap().done, 300_000);
    // retry after fixing the server
    s.put("k/b.gguf", b.clone());
    q.retry("b");
    assert!(q.wait_idle(Duration::from_secs(20)));
    assert_eq!(q.item("b").unwrap().phase, Phase::Installed { verified: true });
    // asking again for an installed model changes nothing
    q.enqueue("a");
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: true });
    q.stop();
}

#[test]
fn the_queue_pauses_continues_and_cancels() {
    let s = Server::start();
    let a = bytes(2_000_000, 1);
    s.put("k/a.gguf", a.clone());
    *s.b.chunk_delay.lock().unwrap() = Duration::from_millis(4);
    let d = tempfile::tempdir().unwrap();
    let src = Source::Server { url: parse_server(&format!("{}/k", s.base()), false).unwrap(), token: None };
    let q = Queue::start(d.path().to_path_buf(), vec![entry("a", &[("a.gguf", &a)])], src, quick);
    q.enqueue("a");
    let t0 = std::time::Instant::now();
    while q.item("a").map(|i| i.done).unwrap_or(0) < 200_000 && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    q.pause("a");
    let t0 = std::time::Instant::now();
    while q.item("a").unwrap().phase != Phase::Paused && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(q.item("a").unwrap().phase, Phase::Paused);
    assert!(d.path().join("a.gguf.part").exists());
    *s.b.chunk_delay.lock().unwrap() = Duration::ZERO;
    q.resume("a");
    assert!(q.wait_idle(Duration::from_secs(20)));
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: true });
    // remove deletes the files
    q.remove("a");
    assert!(!d.path().join("a.gguf").exists() && q.item("a").is_none());
    // cancel while queued behind a slow one leaves no part
    *s.b.chunk_delay.lock().unwrap() = Duration::from_millis(4);
    q.enqueue("a");
    std::thread::sleep(Duration::from_millis(200));
    q.cancel("a");
    let t0 = std::time::Instant::now();
    while q.item("a").is_some() && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(q.item("a").is_none() && !d.path().join("a.gguf.part").exists());
    q.stop();
}

#[test]
fn an_installed_file_never_checked_is_verified_by_the_queue() {
    let s = Server::start();
    let a = bytes(120_000, 1);
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.gguf"), &a).unwrap();
    let src = Source::Server { url: parse_server(&format!("{}/k", s.base()), false).unwrap(), token: None };
    let q = Queue::start(d.path().to_path_buf(), vec![entry("a", &[("a.gguf", &a)])], src, quick);
    q.enqueue("a");
    assert!(q.wait_idle(Duration::from_secs(10)));
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: true });
    assert!(s.seen().is_empty(), "nothing was downloaded for a file that was already right");
    q.stop();
}

#[test]
fn a_read_only_folder_keeps_the_file_installed_unverified_and_does_not_hash_it_again() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { geteuid() } == 0 {
        return;
    }
    let s = Server::start();
    let a = bytes(120_000, 7);
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.gguf"), &a).unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let src = Source::Server { url: parse_server(&format!("{}/k", s.base()), false).unwrap(), token: None };
    let q = Queue::start(d.path().to_path_buf(), vec![entry("a", &[("a.gguf", &a)])], src, quick);
    q.enqueue("a");
    assert!(q.wait_idle(Duration::from_secs(10)));
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: false });
    // asking again in the same session leaves it as it is (no second hashing, no download)
    q.enqueue("a");
    assert_eq!(q.item("a").unwrap().phase, Phase::Installed { verified: false });
    assert!(s.seen().is_empty());
    q.stop();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}

extern "C" {
    fn geteuid() -> u32;
}
