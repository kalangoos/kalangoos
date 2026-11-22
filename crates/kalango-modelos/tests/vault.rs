//! The token's hand-off from the installer to the vault. A stand-in `kalangod` (a shell script that keeps what it is
//! given in a folder of the test) plays the vault; the real one is tested in the kalangod crate.
//! Environment variables are process-wide: this file has one test.
use kalango_modelos::vault;
use kalango_modelos::Secret;

const ORIGIN: &str = "https://modelos.example.org:443";
use std::os::unix::fs::PermissionsExt;

#[test]
fn the_installers_token_ends_in_the_vault_and_the_hand_off_file_is_gone() {
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join("vault");
    std::fs::create_dir(&store).unwrap();
    let script = t.path().join("kalangod");
    std::fs::write(&script, "#!/bin/sh\ncase \"$2\" in\n guardar) cat > \"$VAULT_DIR/$3\" ;;\n ler) cat \"$VAULT_DIR/$3\" ;;\n apagar) rm -f \"$VAULT_DIR/$3\" ;;\nesac\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = t.path().join("home/ana");
    std::env::set_var("KALANGOD", &script);
    std::env::set_var("VAULT_DIR", &store);
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));

    // the installer's side
    let token = Secret::new("tok-abcdef-0123456789");
    let staged = vault::write_staging(&home, ORIGIN, &token).unwrap();
    assert_eq!(std::fs::metadata(&staged).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(staged.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(staged, vault::staging_path());
    // before adoption the token is still readable (from the hand-off file)
    assert_eq!(vault::read_for(ORIGIN).unwrap().expose(), "tok-abcdef-0123456789");

    // the first session's side
    assert_eq!(vault::adopt_staged(), Ok(true));
    assert!(!staged.exists(), "the hand-off file is deleted");
    let kept = std::fs::read_to_string(store.join(vault::VAULT_NAME)).unwrap();
    assert!(kept.contains("tok-abcdef-0123456789") && kept.contains(ORIGIN));
    // the token belongs to that server: another origin gets nothing
    assert!(vault::read_for("https://evil.example:443").is_none());
    assert_eq!(vault::bound_origin().as_deref(), Some(ORIGIN));
    assert_eq!(vault::read_for(ORIGIN).unwrap().expose(), "tok-abcdef-0123456789");
    assert_eq!(vault::adopt_staged(), Ok(false));

    // a vault that refuses keeps the file (private to the user) and says so
    vault::write_staging(&home, ORIGIN, &token).unwrap();
    std::env::set_var("KALANGOD", t.path().join("not-there"));
    assert!(vault::adopt_staged().is_err());
    assert!(staged.exists());

    // forgetting removes both
    std::env::set_var("KALANGOD", &script);
    vault::forget();
    assert!(!staged.exists() && !store.join(vault::VAULT_NAME).exists());
    assert!(vault::read_for(ORIGIN).is_none());
}
