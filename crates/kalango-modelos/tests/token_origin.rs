//! A token kept for one server never reaches another: change the address in the configuration and the token stays
//! behind. Real local servers; the vault is a stand-in `kalangod` (a script keeping what it is given in a folder).
//! Environment variables are process-wide: this file has one test.
mod common;

use common::*;
use kalango_modelos::catalog::ModelFile;
use kalango_modelos::config::{Config, SourceKind};
use kalango_modelos::download::{download_files, Control, Options, Source};
use kalango_modelos::urlpolicy::parse_server;
use kalango_modelos::{vault, Secret};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn the_token_kept_for_one_server_does_not_follow_a_changed_address() {
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join("vault");
    std::fs::create_dir(&store).unwrap();
    let script = t.path().join("kalangod");
    std::fs::write(&script, "#!/bin/sh\ncase \"$2\" in\n guardar) cat > \"$VAULT_DIR/$3\" ;;\n ler) cat \"$VAULT_DIR/$3\" ;;\n apagar) rm -f \"$VAULT_DIR/$3\" ;;\nesac\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("KALANGOD", &script);
    std::env::set_var("VAULT_DIR", &store);
    std::env::set_var("XDG_DATA_HOME", t.path().join("data"));

    let good = Server::start();
    let evil = Server::start();
    let body = bytes(30_000, 5);
    for s in [&good, &evil] {
        s.put("k/m.gguf", body.clone());
    }
    let f = ModelFile { url: String::new(), nome: "m.gguf".into(), bytes: body.len() as u64, sha256: sha256(&body) };
    let opts = || Options { max_attempts: 1, online: || true, connect_timeout: Duration::from_secs(2), ..Options::default() };

    // the token is kept for the good server
    let good_url = parse_server(&format!("{}/k", good.base()), false).unwrap();
    vault::store(&good_url.origin(), &Secret::new("tok-for-the-good-server")).unwrap();
    let cfg_good = Config { fonte: SourceKind::Servidor, servidor: Some(format!("{}/k", good.base())), ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    download_files(std::slice::from_ref(&f), d.path(), &cfg_good.source(), &Control::default(), &opts(), &mut |_| {}).unwrap();
    assert!(good.seen().iter().all(|r| r.authorization.as_deref() == Some("Bearer tok-for-the-good-server")));

    // the address is changed to another server, the token is not typed again: nothing is sent to it
    let cfg_evil = Config { fonte: SourceKind::Servidor, servidor: Some(format!("{}/k", evil.base())), ..Default::default() };
    assert!(matches!(cfg_evil.source(), Source::Server { token: None, .. }));
    let d2 = tempfile::tempdir().unwrap();
    download_files(std::slice::from_ref(&f), d2.path(), &cfg_evil.source(), &Control::default(), &opts(), &mut |_| {}).unwrap();
    assert!(!evil.seen().is_empty());
    assert!(evil.seen().iter().all(|r| r.authorization.is_none()), "the token reached another server: {:?}", evil.seen());
    // and an invalid configuration has no source at all
    assert!(matches!(Config { fonte: SourceKind::Servidor, servidor: Some("http://modelos.example.org".into()), ..Default::default() }.source(), Source::Disabled));
}
