//! Critical alerts (design "Alertas críticos (bateria e disco)"): notices that stay until answered, with what to do —
//! the battery at 5% on battery power (maximum saving: the power-saver mode; or suspend now), and the system disk
//! almost full (free what can go without touching the person's files: caches and the trash; or see what fills it in
//! Settings › System › Storage). Checked with the power samples (each minute; seconds in the test session).

use kalango_i18n::{tr, trf};
use std::cell::Cell;
use std::path::Path;

thread_local! {
    /// the battery's alert was given in this discharge (again only after charging)
    static BATERIA_AVISADA: Cell<bool> = const { Cell::new(false) };
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What can be freed without touching anything of the person's: the caches (not KalangoOS's own) and the trash.
fn liberavel() -> u64 {
    let du = |p: &Path| -> u64 {
        std::process::Command::new("du").args(["-sb", &p.display().to_string()]).output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().and_then(|n| n.parse().ok())).unwrap_or(0)
    };
    let cache = gtk::glib::user_cache_dir();
    let proprio = du(&cache.join("kalango"));
    du(&cache).saturating_sub(proprio) + du(&gtk::glib::user_data_dir().join("Trash/files"))
}

pub fn vigiar() {
    bateria();
    disco();
}

fn bateria() {
    crate::lateral::em_segundo_plano(crate::configuracoes_energia::read_battery, |b| {
        let Some(b) = b else { return };
        if b.charging {
            BATERIA_AVISADA.with(|a| a.set(false));
            return;
        }
        if b.pct > 5.0 || BATERIA_AVISADA.with(|a| a.replace(true)) {
            return;
        }
        let Some(s) = crate::shell_atual() else { return };
        let titulo = match b.minutes {
            Some(m) => trf("Battery at {pct}% — about {min} minutes", &[("pct", format!("{:.0}", b.pct)), ("min", m.to_string())]),
            None => trf("Battery at {pct}%", &[("pct", format!("{:.0}", b.pct))]),
        };
        if debug() {
            eprintln!("debug alerta: {titulo}");
        }
        s.notificacoes().notify_critico(tr("Power"), &titulo, tr("Plug in the charger. Suspended, what is open stays as it is."),
            &[("economia", tr("Maximum saving")), ("suspender", tr("Suspend now"))], |acao| {
                if debug() {
                    eprintln!("debug alerta: bateria → {acao}");
                }
                match acao {
                    "economia" => {
                        std::thread::spawn(|| crate::sistema::perfil("power-saver"));
                    }
                    "suspender" => crate::energia::Energia::fazer("suspender", false),
                    _ => {}
                }
            });
    });
}

fn disco() {
    let today = gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default();
    if crate::configuracoes::ajustes()["alerta_disco_dia"].as_str() == Some(today.as_str()) {
        return;
    }
    // (the session test: a file named by KALANGO_DISCO_TESTE, when it exists, holds the "free bytes")
    let teste = std::env::var_os("KALANGO_DISCO_TESTE").and_then(|f| std::fs::read_to_string(f).ok()).and_then(|v| v.trim().parse::<u64>().ok());
    let Some((total, livre)) = crate::configuracoes_sistema::statvfs(Path::new("/")).map(|(t, l)| (t, teste.unwrap_or(l))) else { return };
    // almost full: under 5 GB, or under 3% of the disk
    if total == 0 || (livre >= 5 << 30 && livre * 100 / total >= 3) {
        return;
    }
    crate::configuracoes::salvar_ajuste("alerta_disco_dia", serde_json::json!(today));
    crate::lateral::em_segundo_plano(liberavel, move |pode| {
        let Some(s) = crate::shell_atual() else { return };
        let livre_h = crate::configuracoes_sistema::human(livre);
        let pode_h = crate::configuracoes_sistema::human(pode);
        let titulo = trf("Disk almost full: {free} left", &[("free", livre_h)]);
        let corpo = if pode > 0 {
            trf("Updates and AI models may fail. {size} can be freed without deleting anything of yours.", &[("size", pode_h.clone())])
        } else {
            tr("Updates and AI models may fail. See what fills the disk.").to_string()
        };
        if debug() {
            eprintln!("debug alerta: {titulo} (liberável {pode_h})");
        }
        let liberar = trf("Free {size}", &[("size", pode_h)]);
        let mut acoes: Vec<(&str, &str)> = Vec::new();
        if pode > 0 {
            acoes.push(("liberar", liberar.as_str()));
        }
        acoes.push(("ver", tr("See what fills it")));
        s.notificacoes().notify_critico(tr("System"), &titulo, &corpo, &acoes, |acao| {
            if debug() {
                eprintln!("debug alerta: disco → {acao}");
            }
            match acao {
                "liberar" => crate::lateral::em_segundo_plano(|| (crate::configuracoes_sistema::clear_cache(), crate::configuracoes_sistema::empty_trash()), |(c, t)| {
                    if debug() {
                        eprintln!("debug alerta: liberado ({c} caches, {t} na lixeira)");
                    }
                    if let Some(s) = crate::shell_atual() {
                        s.notificacoes().notify_local(tr("System"), tr("Space freed"), &trf("{c} caches and {t} items of the trash are gone.", &[("c", c.to_string()), ("t", t.to_string())]));
                    }
                }),
                "ver" => crate::configuracoes::abrir_painel("armazenamento"),
                _ => {}
            }
        });
    });
}
