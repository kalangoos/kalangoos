//! A command the agent ran (design "Chat · blocos de mensagem" › Terminal): a header with "Terminal", the folder it
//! ran in and its exit code and time (teal when it worked, coral when not), copy and fold; under it the command and
//! what it printed.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::Value;

/// "4,2 s", "850 ms", "1 min 12 s" (the decimal comma of pt-BR).
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0).replace('.', ",")
    } else {
        format!("{} min {} s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// The command as a shell line (arguments with spaces quoted).
pub fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|a| if a.is_empty() || a.contains(|c: char| c.is_whitespace() || "'\"$`\\".contains(c)) { format!("'{}'", a.replace('\'', "'\\''")) } else { a.clone() }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A check's result in a few words (design "Chat · código" › Verificações): tests passed and failed with the time
/// (cargo test, pytest), a linter's warnings (clippy), else the time — or, when it failed, the last line it printed.
pub fn summary(command: &str, code: Option<i64>, ms: u64, output: &str) -> String {
    let time = duration(ms);
    let count = |line: &str, word: &str| -> Option<u64> {
        let i = line.find(word)?;
        line[..i].trim_end().rsplit(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    };
    // cargo test: "test result: ok. 24 passed; 0 failed; …" (one per test binary, summed)
    let results: Vec<&str> = output.lines().filter(|l| l.contains("test result:")).collect();
    if !results.is_empty() {
        let passed: u64 = results.iter().filter_map(|l| count(l, " passed")).sum();
        let failed: u64 = results.iter().filter_map(|l| count(l, " failed")).sum();
        return trf("{passed} passed · {failed} failed · {time}", &[("passed", passed.to_string()), ("failed", failed.to_string()), ("time", time)]);
    }
    // pytest: "===== 3 passed, 1 failed in 0.12s ====="
    if let Some(l) = output.lines().rev().find(|l| l.contains(" passed") && l.contains(" in ") && l.trim_start().starts_with('=')) {
        let passed = count(l, " passed").unwrap_or(0);
        let failed = count(l, " failed").unwrap_or(0);
        return trf("{passed} passed · {failed} failed · {time}", &[("passed", passed.to_string()), ("failed", failed.to_string()), ("time", time)]);
    }
    if command.contains("clippy") {
        let n = output.lines().filter(|l| l.starts_with("warning:") && !l.contains("generated")).count();
        return trf("{n} warnings", &[("n", n.to_string())]);
    }
    if code == Some(0) {
        return time;
    }
    let last = output.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let last: String = last.chars().take(80).collect();
    if last.is_empty() { time } else { last }
}

/// The block for a finished `tarefas.executar` / `sistema.executar`: its arguments and the daemon's result (`codigo`,
/// `saida`, `erros`, `tempo_ms`). None when the result is not one (a refusal's text).
pub fn from_result(args: &Value, result: &str) -> Option<gtk::Widget> {
    let r: Value = serde_json::from_str(result).ok()?;
    if !r.is_object() || r.get("tempo_ms").is_none() {
        return None;
    }
    let program = args["programa"].as_str().unwrap_or("");
    let a: Vec<String> = args["argumentos"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
    let folder = args["pasta"].as_str().unwrap_or("~");
    let mut out = r["saida"].as_str().unwrap_or("").trim_end().to_string();
    let err = r["erros"].as_str().unwrap_or("").trim_end();
    if !err.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(err);
    }
    Some(new(&command_line(program, &a), folder, r["codigo"].as_i64(), r["tempo_ms"].as_u64().unwrap_or(0), &out))
}

/// The block. `code`: the exit code (None: stopped by a signal).
pub fn new(command: &str, folder: &str, code: Option<i64>, ms: u64, output: &str) -> gtk::Widget {
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    bloco.add_css_class("terminal-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("terminal", "#8E8F99", 16));
    cab.append(&rotulo(tr("Terminal"), "md-bloco-titulo"));
    let d = rotulo(folder, "md-bloco-detalhe");
    d.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    d.set_max_width_chars(1);
    d.set_xalign(0.0);
    cab.append(&d);
    let ok = code == Some(0);
    let estado = rotulo(&format!("{} · {}", code.map(|c| format!("exit {c}")).unwrap_or_else(|| tr("interrupted").to_string()), duration(ms)), if ok { "diff-mais" } else { "diff-menos" });
    estado.set_valign(Align::Center);
    cab.append(&estado);
    let e = caixa(Orientation::Horizontal, 0, "");
    e.set_hexpand(true);
    cab.append(&e);
    let texto = format!("$ {command}\n{output}");
    let copiar = gtk::Button::new();
    copiar.add_css_class("md-bloco-icone");
    copiar.set_child(Some(&ui_icon("copiar", "#8E8F99", 16)));
    copiar.set_tooltip_text(Some(tr("Copy")));
    copiar.set_valign(Align::Center);
    let t = texto.clone();
    copiar.connect_clicked(move |b| b.clipboard().set_text(&t));
    cab.append(&copiar);
    let dobrar = gtk::ToggleButton::new();
    dobrar.add_css_class("md-bloco-icone");
    dobrar.set_child(Some(&ui_icon("chevron-baixo", "#8E8F99", 16)));
    dobrar.set_tooltip_text(Some(tr("Fold")));
    dobrar.set_valign(Align::Center);
    cab.append(&dobrar);
    bloco.append(&cab);
    let div = caixa(Orientation::Horizontal, 0, "md-bloco-div");
    bloco.append(&div);
    // (a long output: its last 200 lines)
    let linhas: Vec<&str> = output.lines().collect();
    let corte = linhas.len().saturating_sub(200);
    let mostrado = if corte > 0 { format!("$ {command}\n⋯\n{}", linhas[corte..].join("\n")) } else { texto };
    let saida = rotulo(&mostrado, "md-bloco-codigo");
    saida.set_xalign(0.0);
    saida.set_valign(Align::Start);
    saida.set_selectable(true);
    let corpo = caixa(Orientation::Horizontal, 0, "md-bloco-corpo");
    corpo.append(&saida);
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    rol.set_propagate_natural_height(true);
    rol.set_max_content_height(280);
    rol.set_child(Some(&corpo));
    rol.add_css_class("md-bloco-rolagem");
    bloco.append(&rol);
    let (r, dv) = (rol.clone(), div.clone());
    dobrar.connect_toggled(move |b| {
        r.set_visible(!b.is_active());
        dv.set_visible(!b.is_active());
    });
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug codigo: terminal «{command}» ({}, {} linhas)", code.map(|c| format!("exit {c}")).unwrap_or_default(), linhas.len());
    }
    bloco.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_summaries() {
        let cargo = "   Compiling x v0.1.0\n    Finished release\ntest result: ok. 24 passed; 0 failed; 0 ignored\ntest result: ok. 3 passed; 1 failed; 0 ignored";
        let tests = |p: &str, f: &str, t: &str| trf("{passed} passed · {failed} failed · {time}", &[("passed", p.into()), ("failed", f.into()), ("time", t.into())]);
        assert_eq!(summary("cargo test --release", Some(101), 4200, cargo), tests("27", "1", "4,2 s"));
        assert_eq!(summary("pytest", Some(0), 1200, "===== 3 passed, 1 failed in 0.12s ====="), tests("3", "1", "1,2 s"));
        let clippy = "warning: unused variable\nwarning: needless return\nwarning: `x` (lib) generated 2 warnings";
        assert_eq!(summary("cargo clippy", Some(0), 900, clippy), trf("{n} warnings", &[("n", "2".into())]));
        assert_eq!(summary("cargo fmt --check", Some(0), 300, ""), "300 ms");
        assert_eq!(summary("make", Some(2), 300, "cc -c a.c\nmake: *** [all] Error 1\n"), "make: *** [all] Error 1");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(850), "850 ms");
        assert_eq!(duration(4210), "4,2 s");
        assert_eq!(duration(72_000), "1 min 12 s");
    }

    #[test]
    fn command_quoting() {
        assert_eq!(command_line("cargo", &["test".into(), "--release".into()]), "cargo test --release");
        assert_eq!(command_line("echo", &["oi mundo".into()]), "echo 'oi mundo'");
        assert_eq!(command_line("echo", &["it's".into()]), "echo 'it'\\''s'");
    }

    #[test]
    fn only_a_run_result() {
        let a = json!({"programa": "ls"});
        assert!(from_result(&a, "you did not allow it").is_none());
        assert!(from_result(&a, "{\"pontos\": []}").is_none());
    }
}
