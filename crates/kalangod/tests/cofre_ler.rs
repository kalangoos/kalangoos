//! `kalangod cofre guardar|ler|apagar`: o caminho que o download de modelos usa para guardar e ler o token do
//! servidor privado. Usa o modo arquivo (KALANGO_COFRE=arquivo) numa pasta do teste, nunca o cofre real.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_kalangod");

fn kalangod(data: &std::path::Path, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut c = Command::new(BIN);
    c.args(args).env("KALANGO_COFRE", "arquivo").env("XDG_DATA_HOME", data).env("HOME", data).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut ch = c.spawn().unwrap();
    if let Some(i) = stdin {
        ch.stdin.take().unwrap().write_all(i.as_bytes()).unwrap();
    }
    ch.wait_with_output().unwrap()
}

#[test]
fn guarda_le_e_apaga_um_segredo_sem_mostrar_o_valor_ao_guardar() {
    let d = tempfile::tempdir().unwrap();
    let nome = "kalango.modelos.token";
    let o = kalangod(d.path(), &["cofre", "guardar", nome], Some("tok-0123456789\n"));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!String::from_utf8_lossy(&o.stdout).contains("tok-0123456789"), "guardar não repete o valor");
    let f = d.path().join("kalango/cofre.json");
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    let o = kalangod(d.path(), &["cofre", "ler", nome], None);
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), "tok-0123456789");
    // o que não existe sai com erro (e nada no stdout)
    let o = kalangod(d.path(), &["cofre", "ler", "nao.existe"], None);
    assert!(!o.status.success() && o.stdout.is_empty());
    // another secret, even one that exists, is not handed out
    kalangod(d.path(), &["cofre", "guardar", "chave.da.api"], Some("segredo-de-outra-coisa\n"));
    let o = kalangod(d.path(), &["cofre", "ler", "chave.da.api"], None);
    assert!(!o.status.success() && o.stdout.is_empty(), "o cofre entregou um segredo que não é do download de modelos");
    kalangod(d.path(), &["cofre", "apagar", nome], None);
    assert!(!kalangod(d.path(), &["cofre", "ler", nome], None).status.success());
}
