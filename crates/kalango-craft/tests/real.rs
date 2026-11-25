//! The real chain against the project's servers (ignored by default: network, and a Flatpak installation it changes).
//! Run with a throwaway installation and folder:
//!   FLATPAK_USER_DIR=/tmp/x/flatpak KALANGO_CRAFT_DIR=/tmp/x/craft KALANGO_CRAFT_AJUSTES=/tmp/x/ajustes.json \
//!   cargo test -p kalango-craft --test real -- --ignored --nocapture instalar
//! Installs AniCraft (the smallest app) from its signed image, checks the installed commit and the permissions, and
//! leaves it installed for kalangod's test (`kalangod/tests/craft_real.rs`); `remover_anicraft` takes it away.

use kalango_craft::{catalogo, instalar, verificar};

fn guard() {
    for k in ["FLATPAK_USER_DIR", "KALANGO_CRAFT_DIR"] {
        let v = std::env::var(k).unwrap_or_else(|_| panic!("set {k} to a throwaway folder"));
        let home = std::env::var("HOME").unwrap_or_default();
        assert!(!v.starts_with(&home), "{k} must not be in the real home");
    }
}

#[test]
#[ignore]
fn instalar_anicraft_verificado() {
    guard();
    let cat = catalogo();
    let app = cat.app("anicraft").unwrap();
    let t = std::time::Instant::now();
    let idx = verificar::indice(cat).expect("index");
    assert_eq!(idx.len(), 17, "the index has every app of the catalog");
    let an = idx.iter().find(|x| x.0 == app.id).unwrap().1.clone();
    println!("index: {} apps ({} ms); AniCraft {} {} ({} bytes)", idx.len(), t.elapsed().as_millis(), an.versao, an.digest, an.tamanho_download);
    // the protected side (kalangod): verify, download, import into the repository only it writes
    let repo = instalar::pasta().join("repo-protegido");
    let a = verificar::Andamento::default();
    let t = std::time::Instant::now();
    let (commit, versao, digest) =
        instalar::preparar(cat, app, &repo, &instalar::pasta().join("baixando"), None, &a).expect("verifies, downloads and imports");
    println!("prepared commit {commit} v{versao} ({} bytes) in {} ms", a.feito.load(std::sync::atomic::Ordering::Relaxed), t.elapsed().as_millis());
    // the person's side: install exactly that commit
    let t = std::time::Instant::now();
    let i = instalar::instalar_commit(app, &repo, &commit, &versao, &digest).expect("installs");
    println!("installed commit {} v{} in {} ms", i.commit, i.versao, t.elapsed().as_millis());
    // a commit nobody verified is refused (not in the repository: flatpak cannot deploy it)
    assert!(instalar::instalar_commit(app, &repo, &"0".repeat(64), "9.9.9", "").is_err());
    assert_eq!(instalar::commit_instalado(&app.id).as_deref(), Some(commit.as_str()), "the verified one stays");
    assert_eq!(instalar::commit_instalado(&app.id).as_deref(), Some(i.commit.as_str()));
    assert_eq!(instalar::origem(&app.id).as_deref(), Some(instalar::REMOTO));
    instalar::aplicar_permissoes(&app.id, false, &app.pastas).expect("permissions");
    let o = instalar::rodar(&["override", "--user", "--show", &app.id]).unwrap();
    println!("override:\n{o}");
    assert!(o.contains("!network"), "{o}");
    assert!(o.contains("org.freedesktop.Flatpak=none"), "{o}");
    assert!(instalar::instalados().contains(&app.id));
}

/// Only the protected side: verify the signature, download, import into a scratch repository and check that the commit
/// the import produced carries the metadata the signed config names (`importar_em`); no runtime and no installation needed.
#[test]
#[ignore]
fn preparar_anicraft_sem_instalar() {
    guard();
    let cat = catalogo();
    let app = cat.app("anicraft").unwrap();
    let idx = verificar::indice(cat).expect("index");
    let an = idx.iter().find(|x| x.0 == app.id).unwrap().1.clone();
    println!("AniCraft {} runtime {} download {} bytes installed {} bytes", an.versao, an.runtime, an.tamanho_download, an.tamanho_instalado);
    let repo = instalar::pasta().join("repo-protegido");
    let a = verificar::Andamento::default();
    let (commit, versao, _) = instalar::preparar(cat, app, &repo, &instalar::pasta().join("baixando"), None, &a)
        .expect("verifies, downloads, imports and the commit matches the signed one");
    println!("prepared commit {commit} v{versao}: the imported commit carries the signed config's metadata (the commit id itself differs from org.flatpak.commit: the import re-commits)");
    assert_eq!(commit.len(), 64);
}

#[test]
#[ignore]
fn remover_anicraft() {
    guard();
    let app = catalogo().app("anicraft").unwrap();
    instalar::remover(app, true).expect("removes");
    assert!(!instalar::instalados().contains(&app.id));
}
