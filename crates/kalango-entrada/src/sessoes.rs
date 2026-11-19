//! The sessions there are to start: the desktop files of /usr/share/wayland-sessions (and xsessions, through startx).
//! `KALANGO_ENTRADA_SESSOES` (a folder) replaces them in tests.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Sessao {
    pub nome: String,
    pub cmd: Vec<String>,
    pub wayland: bool,
    pub id: String,
}

fn campo(texto: &str, chave: &str) -> Option<String> {
    let lang = kalango_i18n::system_locale().unwrap_or("en");
    let local = format!("{chave}[{lang}]=");
    let curto = format!("{chave}[{}]=", lang.split('_').next().unwrap_or(lang));
    let geral = format!("{chave}=");
    let linhas: Vec<&str> = texto.lines().map(str::trim).collect();
    [local, curto, geral].iter().find_map(|k| linhas.iter().find_map(|l| l.strip_prefix(k.as_str()).map(str::to_string)))
}

/// Exec, split as a shell would for simple quoting.
pub fn dividir(exec: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut atual = String::new();
    let mut aspas: Option<char> = None;
    for ch in exec.chars() {
        match (ch, aspas) {
            ('"' | '\'', None) => aspas = Some(ch),
            (c, Some(q)) if c == q => aspas = None,
            (' ', None) => {
                if !atual.is_empty() {
                    v.push(std::mem::take(&mut atual));
                }
            }
            (c, _) => atual.push(c),
        }
    }
    if !atual.is_empty() {
        v.push(atual);
    }
    // field codes (%f, %U…) are not for sessions
    v.retain(|a| !(a.len() == 2 && a.starts_with('%')));
    v
}

fn de_pasta(d: &Path, wayland: bool) -> Vec<Sessao> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "desktop") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        if campo(&t, "Hidden").is_some_and(|h| h == "true") || campo(&t, "NoDisplay").is_some_and(|h| h == "true") {
            continue;
        }
        let (Some(nome), Some(exec)) = (campo(&t, "Name"), campo(&t, "Exec")) else { continue };
        let mut cmd = dividir(&exec);
        if cmd.is_empty() {
            continue;
        }
        if !wayland {
            cmd.insert(0, "startx".into());
        }
        let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        v.push(Sessao { nome, cmd, wayland, id });
    }
    v.sort_by(|a, b| a.nome.cmp(&b.nome));
    v
}

pub fn listar() -> Vec<Sessao> {
    if let Some(d) = std::env::var_os("KALANGO_ENTRADA_SESSOES") {
        return de_pasta(&PathBuf::from(d), true);
    }
    let mut v = de_pasta(Path::new("/usr/share/wayland-sessions"), true);
    v.extend(de_pasta(Path::new("/usr/share/xsessions"), false));
    // KalangoOS first
    v.sort_by_key(|s| !s.id.starts_with("kalango"));
    v
}

/// The keyboard layout the system uses (/etc/default/keyboard): "ABNT2", "BR", "US".
pub fn teclado() -> Option<String> {
    let t = std::fs::read_to_string("/etc/default/keyboard").ok()?;
    let v = |k: &str| t.lines().find_map(|l| l.trim().strip_prefix(k)).map(|x| x.trim_matches('"').to_string()).filter(|x| !x.is_empty());
    v("XKBVARIANT=").or_else(|| v("XKBLAYOUT=")).map(|x| x.split(',').next().unwrap_or("").to_uppercase())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_session_files() {
        let d = std::env::temp_dir().join(format!("kalango-sessoes-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("kalango.desktop"), "[Desktop Entry]\nName=KalangoOS\nExec=/usr/libexec/kalango/kalango-sessao --x \"a b\"\n").unwrap();
        std::fs::write(d.join("oculta.desktop"), "[Desktop Entry]\nName=Oculta\nExec=x\nNoDisplay=true\n").unwrap();
        let v = super::de_pasta(&d, true);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].nome, "KalangoOS");
        assert_eq!(v[0].cmd, ["/usr/libexec/kalango/kalango-sessao", "--x", "a b"]);
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(super::dividir("foo %U bar"), ["foo", "bar"]);
    }
}
