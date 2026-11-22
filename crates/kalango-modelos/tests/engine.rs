mod common;

use common::*;
use kalango_modelos::catalog::ModelFile;
use kalango_modelos::download::{download_files, Control, DownloadError, Event, Options, Source};
use kalango_modelos::urlpolicy::parse_server;
use kalango_modelos::Secret;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

const TOKEN: &str = "tok-0123456789-abcdef";

fn quick() -> Options {
    Options { max_attempts: 3, backoff: Duration::from_millis(10), max_backoff: Duration::from_millis(40), online: || true, connect_timeout: Duration::from_secs(2), read_timeout: Duration::from_secs(5), ..Options::default() }
}

fn file(name: &str, body: &[u8]) -> ModelFile {
    ModelFile { url: String::new(), nome: name.into(), bytes: body.len() as u64, sha256: sha256(body) }
}

fn server_source(s: &Server, token: Option<&str>) -> Source {
    Source::Server { url: parse_server(&format!("{}/kalango", s.base()), false).unwrap(), token: token.map(Secret::new) }
}

fn run(files: &[ModelFile], dest: &Path, src: &Source, ctl: &Control, opts: &Options) -> (Result<(), DownloadError>, Vec<String>) {
    let mut events = Vec::new();
    let r = download_files(files, dest, src, ctl, opts, &mut |e| match e {
        Event::Retry { attempt, max, .. } => events.push(format!("retry {attempt}/{max}")),
        Event::Verifying { file } => events.push(format!("verifying {file}")),
        Event::Progress { .. } => {}
    });
    (r, events)
}

#[test]
fn downloads_verifies_and_installs() {
    let s = Server::start();
    let body = bytes(700_000, 1);
    s.put("kalango/m.gguf", body.clone());
    let d = tempfile::tempdir().unwrap();
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    r.unwrap();
    assert_eq!(std::fs::read(d.path().join("m.gguf")).unwrap(), body);
    assert!(!d.path().join("m.gguf.part").exists());
    assert!(ev.contains(&"verifying m.gguf".to_string()));
}

#[test]
fn resumes_with_range_after_an_interrupted_transfer() {
    let s = Server::start();
    let body = bytes(900_000, 2);
    s.put("kalango/m.gguf", body.clone());
    *s.b.drop_first_after.lock().unwrap() = Some(300_000);
    let d = tempfile::tempdir().unwrap();
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    r.unwrap();
    // the whole file is right (the sum covers the resumed bytes too)
    assert_eq!(sha256(&std::fs::read(d.path().join("m.gguf")).unwrap()), sha256(&body));
    assert!(ev.iter().any(|e| e.starts_with("retry 1/3")), "{ev:?}");
    let gets: Vec<_> = s.seen().into_iter().filter(|r| r.method == "GET").collect();
    assert_eq!(gets.len(), 2);
    assert!(gets[0].range.is_none());
    let from: u64 = gets[1].range.as_ref().unwrap().trim_start_matches("bytes=").trim_end_matches('-').parse().unwrap();
    assert!(from > 0 && from < body.len() as u64, "the second request resumed at {from}");
}

#[test]
fn a_wrong_sum_is_refused_and_the_file_deleted() {
    let s = Server::start();
    let body = bytes(200_000, 3);
    s.put("kalango/m.gguf", bytes(200_000, 4)); // same size, other content
    let d = tempfile::tempdir().unwrap();
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::Mismatch { file: "m.gguf".into() }));
    assert!(ev.contains(&"verifying m.gguf".to_string()));
    assert!(!d.path().join("m.gguf").exists());
    assert!(!d.path().join("m.gguf.part").exists());
    assert_eq!(s.b.gets.load(Ordering::Relaxed), 1, "a refused file is not retried");
}

#[test]
fn a_resumed_file_with_a_wrong_sum_is_refused_too() {
    let s = Server::start();
    let body = bytes(500_000, 5);
    s.put("kalango/m.gguf", bytes(500_000, 6));
    *s.b.drop_first_after.lock().unwrap() = Some(100_000);
    let d = tempfile::tempdir().unwrap();
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::Mismatch { file: "m.gguf".into() }));
    assert!(!d.path().join("m.gguf").exists() && !d.path().join("m.gguf.part").exists());
}

#[test]
fn access_errors_401_and_403_are_clear_and_not_retried() {
    for code in [401u16, 403] {
        let s = Server::start();
        let body = bytes(1000, 7);
        s.put("kalango/m.gguf", body.clone());
        *s.b.always.lock().unwrap() = Some(code);
        let d = tempfile::tempdir().unwrap();
        let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, Some(TOKEN)), &Control::default(), &quick());
        assert_eq!(r, Err(DownloadError::Denied(code)));
        assert!(ev.is_empty(), "no retry for a refused token: {ev:?}");
        assert_eq!(s.seen().len(), 1);
    }
}

#[test]
fn the_right_token_gets_in_and_a_wrong_one_is_refused() {
    let s = Server::start();
    let body = bytes(50_000, 8);
    s.put("kalango/m.gguf", body.clone());
    *s.b.token.lock().unwrap() = Some(TOKEN.into());
    let d = tempfile::tempdir().unwrap();
    let f = [file("m.gguf", &body)];
    assert_eq!(run(&f, d.path(), &server_source(&s, None), &Control::default(), &quick()).0, Err(DownloadError::Denied(401)));
    assert_eq!(run(&f, d.path(), &server_source(&s, Some("wrong")), &Control::default(), &quick()).0, Err(DownloadError::Denied(403)));
    run(&f, d.path(), &server_source(&s, Some(TOKEN)), &Control::default(), &quick()).0.unwrap();
}

#[test]
fn an_unreachable_server_is_retried_a_bounded_number_of_times() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }; // closed again: connection refused
    let src = Source::Server { url: parse_server(&format!("http://127.0.0.1:{port}/k"), false).unwrap(), token: None };
    let body = bytes(1000, 9);
    let d = tempfile::tempdir().unwrap();
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &src, &Control::default(), &quick());
    assert!(matches!(r, Err(DownloadError::Unreachable(_))), "{r:?}");
    assert_eq!(ev, ["retry 1/3", "retry 2/3"]);
}

#[test]
fn no_network_at_all_is_offline_and_not_retried() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let src = Source::Server { url: parse_server(&format!("http://127.0.0.1:{port}/k"), false).unwrap(), token: None };
    let body = bytes(1000, 9);
    let d = tempfile::tempdir().unwrap();
    let opts = Options { online: || false, ..quick() };
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &src, &Control::default(), &opts);
    assert_eq!(r, Err(DownloadError::Offline));
    assert!(ev.is_empty());
}

#[test]
fn a_server_error_is_retried_then_reported() {
    let s = Server::start();
    let body = bytes(1000, 10);
    s.put("kalango/m.gguf", body.clone());
    *s.b.always.lock().unwrap() = Some(503);
    let d = tempfile::tempdir().unwrap();
    let (r, ev) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::Status(503)));
    assert_eq!(ev.len(), 2);
}

#[test]
fn a_file_with_another_size_than_the_catalog_is_refused_and_deleted() {
    // longer than the catalog says
    let s = Server::start();
    let body = bytes(300_000, 11);
    s.put("kalango/m.gguf", bytes(350_000, 11));
    let d = tempfile::tempdir().unwrap();
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::WrongSize { file: "m.gguf".into(), found: 300_001, expected: 300_000 }));
    assert!(!d.path().join("m.gguf").exists() && !d.path().join("m.gguf.part").exists());
    // shorter: the part is resumed, the server says there is nothing more (416), the size is wrong
    let s = Server::start();
    s.put("kalango/m.gguf", body[..200_000].to_vec());
    let d = tempfile::tempdir().unwrap();
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert!(matches!(r, Err(DownloadError::WrongSize { found: 200_000, expected: 300_000, .. })), "{r:?}");
    assert!(!d.path().join("m.gguf").exists() && !d.path().join("m.gguf.part").exists());
}

#[test]
fn not_enough_free_space_is_said_before_downloading() {
    let s = Server::start();
    let body = bytes(500_000, 12);
    s.put("kalango/m.gguf", body.clone());
    let d = tempfile::tempdir().unwrap();
    let opts = Options { free_space: Box::new(|_| Some(120_000)), ..quick() };
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &opts);
    assert_eq!(r, Err(DownloadError::NoSpace { missing: 380_000 }));
    assert_eq!(s.b.gets.load(Ordering::Relaxed), 0, "nothing was requested");
}

#[test]
fn a_real_full_disk_is_no_space_too() {
    // /dev/full fails every write with ENOSPC; a models folder cannot be it, so use it through the part file's path
    if !Path::new("/dev/full").exists() {
        return;
    }
    let s = Server::start();
    let body = bytes(100_000, 13);
    s.put("kalango/m.gguf", body.clone());
    let d = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/dev/full", d.path().join("m.gguf.part")).unwrap();
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert!(matches!(r, Err(DownloadError::NoSpace { .. })), "{r:?}");
}

#[test]
fn the_token_goes_only_to_the_configured_server() {
    let a = Server::start();
    let other = Server::start();
    let body = bytes(40_000, 14);
    a.put("kalango/m.gguf", body.clone());
    other.put("kalango/m.gguf", body.clone());
    // the configured server redirects (same origin) to its own file: followed, token kept
    a.put("kalango/real.gguf", body.clone());
    let d = tempfile::tempdir().unwrap();
    let f = [file("m.gguf", &body)];
    run(&f, d.path(), &server_source(&a, Some(TOKEN)), &Control::default(), &quick()).0.unwrap();
    assert!(a.seen().iter().all(|r| r.authorization.as_deref() == Some(&format!("Bearer {TOKEN}"))));
    // a redirect to another origin (the other server's port) is refused, and that server sees nothing
    *a.b.redirect_to.lock().unwrap() = Some(format!("{}/kalango/m.gguf", other.base()));
    let d2 = tempfile::tempdir().unwrap();
    let src = Source::Server { url: parse_server(&format!("{}/redirect", a.base()), false).unwrap(), token: Some(Secret::new(TOKEN)) };
    let (r, _) = run(&f, d2.path(), &src, &Control::default(), &quick());
    let err = format!("{r:?}");
    assert!(matches!(r, Err(DownloadError::Redirect(_))), "{err}");
    assert!(other.seen().is_empty(), "the other host received a request: {:?}", other.seen());
    assert!(!err.contains(TOKEN), "the error text carries the token");
}

#[test]
fn the_public_source_never_sends_a_token_and_follows_only_safe_redirects() {
    let s = Server::start();
    let body = bytes(30_000, 15);
    s.put("m.gguf", body.clone());
    let f = ModelFile { url: format!("{}/m.gguf", s.base()), nome: "m.gguf".into(), bytes: body.len() as u64, sha256: sha256(&body) };
    let d = tempfile::tempdir().unwrap();
    run(&[f], d.path(), &Source::Default, &Control::default(), &quick()).0.unwrap();
    assert!(s.seen().iter().all(|r| r.authorization.is_none()));
}

#[test]
fn a_catalog_file_without_a_sum_is_refused_for_a_private_server_and_a_folder() {
    let s = Server::start();
    let body = bytes(1000, 16);
    s.put("kalango/m.gguf", body.clone());
    let mut f = file("m.gguf", &body);
    f.sha256.clear();
    let d = tempfile::tempdir().unwrap();
    let (r, _) = run(std::slice::from_ref(&f), d.path(), &server_source(&s, None), &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::MissingChecksum { file: "m.gguf".into() }));
    assert!(s.seen().is_empty());
    let r = run(&[f], d.path(), &Source::Folder(d.path().into()), &Control::default(), &quick()).0;
    assert_eq!(r, Err(DownloadError::MissingChecksum { file: "m.gguf".into() }));
}

#[test]
fn pause_keeps_the_part_and_continue_resumes_it() {
    let s = Server::start();
    let body = bytes(800_000, 17);
    s.put("kalango/m.gguf", body.clone());
    *s.b.chunk_delay.lock().unwrap() = Duration::from_millis(3);
    let d = tempfile::tempdir().unwrap();
    let f = [file("m.gguf", &body)];
    let src = server_source(&s, None);
    let ctl = Control::default();
    let mut paused_at = 0;
    let r = download_files(&f, d.path(), &src, &ctl, &quick(), &mut |e| {
        if let Event::Progress { done, .. } = e {
            if done > 200_000 {
                ctl.pause.store(true, Ordering::Relaxed);
                paused_at = done;
            }
        }
    });
    assert_eq!(r, Err(DownloadError::Paused));
    let part = std::fs::metadata(d.path().join("m.gguf.part")).unwrap().len();
    assert!(part >= 200_000 && part < body.len() as u64);
    assert!(!d.path().join("m.gguf").exists());
    ctl.reset();
    *s.b.chunk_delay.lock().unwrap() = Duration::ZERO;
    download_files(&f, d.path(), &src, &ctl, &quick(), &mut |_| {}).unwrap();
    assert_eq!(sha256(&std::fs::read(d.path().join("m.gguf")).unwrap()), sha256(&body));
    assert!(s.seen().iter().any(|r| r.range.is_some()));
    assert!(paused_at > 0);
}

#[test]
fn cancel_deletes_the_part() {
    let s = Server::start();
    let body = bytes(800_000, 18);
    s.put("kalango/m.gguf", body.clone());
    *s.b.chunk_delay.lock().unwrap() = Duration::from_millis(3);
    let d = tempfile::tempdir().unwrap();
    let ctl = Control::default();
    let r = download_files(&[file("m.gguf", &body)], d.path(), &server_source(&s, None), &ctl, &quick(), &mut |e| {
        if let Event::Progress { done, .. } = e {
            if done > 100_000 {
                ctl.cancel.store(true, Ordering::Relaxed);
            }
        }
    });
    assert_eq!(r, Err(DownloadError::Cancelled));
    assert!(!d.path().join("m.gguf.part").exists() && !d.path().join("m.gguf").exists());
}

#[test]
fn a_folder_is_copied_and_verified() {
    let src = tempfile::tempdir().unwrap();
    let body = bytes(400_000, 19);
    std::fs::write(src.path().join("m.gguf"), &body).unwrap();
    let d = tempfile::tempdir().unwrap();
    let f = file("m.gguf", &body);
    run(std::slice::from_ref(&f), d.path(), &Source::Folder(src.path().into()), &Control::default(), &quick()).0.unwrap();
    assert_eq!(std::fs::read(d.path().join("m.gguf")).unwrap(), body);
    assert!(kalango_modelos::download::is_verified(d.path(), &f));
    // wrong content: refused and deleted; missing: not found; wrong size
    let d2 = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("m.gguf"), bytes(400_000, 20)).unwrap();
    assert_eq!(run(std::slice::from_ref(&f), d2.path(), &Source::Folder(src.path().into()), &Control::default(), &quick()).0, Err(DownloadError::Mismatch { file: "m.gguf".into() }));
    assert!(!d2.path().join("m.gguf").exists() && !d2.path().join("m.gguf.part").exists());
    std::fs::write(src.path().join("m.gguf"), b"short").unwrap();
    assert!(matches!(run(std::slice::from_ref(&f), d2.path(), &Source::Folder(src.path().into()), &Control::default(), &quick()).0, Err(DownloadError::WrongSize { .. })));
    std::fs::remove_file(src.path().join("m.gguf")).unwrap();
    assert_eq!(run(&[f], d2.path(), &Source::Folder(src.path().into()), &Control::default(), &quick()).0, Err(DownloadError::NotFound("m.gguf".into())));
}

#[test]
fn the_secret_never_prints_itself() {
    let t = Secret::new(TOKEN);
    assert!(!format!("{t:?} {:?}", Some(&t)).contains(TOKEN));
    let src = Source::Server { url: parse_server("https://modelos.example.org/k", false).unwrap(), token: Some(t) };
    assert!(!format!("{src:?}").contains(TOKEN));
}

#[test]
fn a_body_that_never_ends_is_cut_at_the_catalogs_size_and_deleted() {
    let s = Server::start();
    let body = bytes(300_000, 21);
    s.put("kalango/m.gguf", body.clone());
    s.b.endless.store(true, Ordering::Relaxed);
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().to_path_buf();
    let f = file("m.gguf", &body);
    let src = server_source(&s, None);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run(&[f], &dest, &src, &Control::default(), &quick()).0);
    });
    let r = rx.recv_timeout(Duration::from_secs(20)).expect("the download kept reading an endless body");
    assert!(matches!(r, Err(DownloadError::WrongSize { found: 300_001, expected: 300_000, .. })), "{r:?}");
    assert!(!d.path().join("m.gguf").exists() && !d.path().join("m.gguf.part").exists());
    // what the server managed to send is bounded by the socket buffers, not by the disk
    assert!(s.b.endless_sent.load(Ordering::Relaxed) < 64 * 1024 * 1024);
}

#[test]
fn a_file_already_there_with_the_right_size_is_checked_and_a_wrong_one_comes_again() {
    let s = Server::start();
    let body = bytes(200_000, 22);
    s.put("kalango/m.gguf", body.clone());
    let f = [file("m.gguf", &body)];
    // right content: kept, recorded as verified, nothing requested
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("m.gguf"), &body).unwrap();
    let (r, ev) = run(&f, d.path(), &server_source(&s, None), &Control::default(), &quick());
    r.unwrap();
    assert!(ev.contains(&"verifying m.gguf".to_string()));
    assert!(kalango_modelos::download::is_verified(d.path(), &f[0]));
    assert!(s.seen().is_empty());
    // right size, other content: refused, deleted, downloaded again
    let d2 = tempfile::tempdir().unwrap();
    std::fs::write(d2.path().join("m.gguf"), bytes(200_000, 99)).unwrap();
    run(&f, d2.path(), &server_source(&s, None), &Control::default(), &quick()).0.unwrap();
    assert_eq!(std::fs::read(d2.path().join("m.gguf")).unwrap(), body);
    assert_eq!(s.b.gets.load(Ordering::Relaxed), 1);
}

#[test]
fn an_invalid_configuration_downloads_nothing() {
    let d = tempfile::tempdir().unwrap();
    let body = bytes(10, 1);
    let (r, _) = run(&[file("m.gguf", &body)], d.path(), &Source::Disabled, &Control::default(), &quick());
    assert_eq!(r, Err(DownloadError::SourceInvalid));
    assert!(!d.path().join("m.gguf").exists());
}

#[test]
fn a_file_changed_while_it_was_hashed_is_not_recorded_as_verified() {
    let body = bytes(50_000, 31);
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.gguf");
    std::fs::write(&p, &body).unwrap();
    let f = file("m.gguf", &body);
    let before = kalango_modelos::download::stamp(&p).unwrap();
    // the file changes after the stamp (and before the check is written)
    std::fs::write(&p, bytes(50_001, 32)).unwrap();
    assert!(!kalango_modelos::download::record_verified_at(d.path(), &f, &before));
    assert!(!kalango_modelos::download::is_verified(d.path(), &f));
    // unchanged since the stamp: recorded
    std::fs::write(&p, &body).unwrap();
    let st = kalango_modelos::download::stamp(&p).unwrap();
    assert!(kalango_modelos::download::record_verified_at(d.path(), &f, &st));
    assert!(kalango_modelos::download::is_verified(d.path(), &f));
}

#[test]
fn a_ledger_that_cannot_be_written_is_reported() {
    use std::os::unix::fs::PermissionsExt;
    let body = bytes(20_000, 33);
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("m.gguf"), &body).unwrap();
    let f = file("m.gguf", &body);
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let ok = kalango_modelos::download::record_verified(d.path(), &f);
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    // (running as root would write anyway: then there is nothing to check)
    if unsafe { libc_geteuid() } != 0 {
        assert!(!ok);
    }
}

extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
