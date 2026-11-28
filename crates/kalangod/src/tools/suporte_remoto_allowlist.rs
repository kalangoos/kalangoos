//! The read-only command allowlist for agents' `consult` calls. An ALLOWLIST of command shapes, not a
//! deny-list: a command is accepted only when it parses into one of the shapes below, word by word.
//!
//! This file is byte-for-byte the same as `crates/kalangod/src/tools/suporte_remoto_allowlist.rs` in the
//! KalangoOS repository (it uses only `std`), and both run the same `allowlist-cases.json`; a test in
//! KalangoOS compares the two files when `KALANGO_REMOTE_REPO` points at this repository. Change both.
//!
//! What it does not do: it cannot know what a path is on the other machine (a symlink under /var/log can
//! point anywhere). The server-side option is a restricted user or an SSH forced command
//! (docs/SUPORTE-REMOTO.md).

/// Longest N accepted for `-n N`.
pub const MAX_LINES: u32 = 500;

const FORBIDDEN_CHARS: &str = ";&|<>$`\\\"'(){}[]*?!#~%^=,:\n\r\t\0";

/// `Ok(())` when `cmd` is one of the allowed shapes; otherwise why not (a short English reason).
pub fn check(cmd: &str) -> Result<(), String> {
    if cmd.is_empty() || cmd.len() > 512 {
        return Err("empty or too long".into());
    }
    if !cmd.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Err("only printable ASCII is allowed".into());
    }
    if let Some(c) = cmd.chars().find(|c| FORBIDDEN_CHARS.contains(*c)) {
        return Err(format!("character {c:?} is not allowed"));
    }
    if cmd.starts_with(' ') || cmd.ends_with(' ') || cmd.contains("  ") {
        return Err("words must be separated by single spaces".into());
    }
    let words: Vec<&str> = cmd.split(' ').collect();
    let args = &words[1..];
    let ok = match words[0] {
        "uptime" | "lsblk" | "hostname" | "whoami" | "id" | "date" => args.is_empty(),
        "df" => matches!(args, [] | ["-h"] | ["-hT"] | ["-i"]),
        "free" => matches!(args, [] | ["-m"] | ["-h"]),
        "uname" => matches!(args, ["-a"] | ["-r"] | []),
        "ip" => matches!(args, ["-br", "addr"] | ["-br", "link"]),
        "ss" => matches!(args, ["-tulpn"] | ["-tuln"]),
        "cat" => matches!(args, ["/etc/os-release"] | ["/etc/hostname"]),
        "apt" => matches!(args, ["list", "--upgradable"]),
        "systemctl" => systemctl(args),
        "journalctl" => journalctl(args),
        "du" => matches!(args, ["-sh", p] if path_ok(p)),
        "ls" => ls(args),
        "tail" => matches!(args, ["-n", n, f] if lines_ok(n) && log_file_ok(f)),
        _ => return Err("the command is not in the read-only allowlist".into()),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("the arguments of {} are not in the read-only allowlist", words[0]))
    }
}

/// The services whose logs and status an agent may read (and, in kalangod, act on): the same closed list. `ssh`, `sshd`,
/// `systemd-logind`, `sudo`, `auditd` and session scopes are not in it: their journal is the login log, which the log
/// files list already keeps out (`log_holds_secrets`).
pub const SERVICES: &[&str] = &[
    "nginx", "apache2", "httpd", "caddy", "php-fpm", "postgresql", "mysql", "mariadb", "redis-server", "redis", "mongod", "memcached", "docker", "containerd", "cups", "smbd", "nmbd", "bluetooth", "cron", "crond", "rsyslog",
    "unattended-upgrades", "chrony", "ntp",
];

/// `NAME` or `NAME.service` of one of `SERVICES`.
pub fn service_ok(u: &str) -> bool {
    SERVICES.contains(&u.strip_suffix(".service").unwrap_or(u))
}

fn unit_ok(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 64
        && !u.starts_with(['-', '.'])
        && u.chars().all(|c| c.is_ascii_alphanumeric() || "_.@-".contains(c))
        && !u.contains("..")
}

fn lines_ok(n: &str) -> bool {
    n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<u32>().is_ok_and(|v| (1..=MAX_LINES).contains(&v))
}

fn systemctl(args: &[&str]) -> bool {
    match args {
        ["is-active" | "is-enabled", u] => unit_ok(u),
        // (`status` prints the last lines of the unit's journal: only the closed list of services)
        ["status", u] | ["status", u, "--no-pager"] => service_ok(u),
        ["--failed", "--no-pager"] | ["list-units", "--failed", "--no-pager"] => true,
        _ => false,
    }
}

fn journalctl(args: &[&str]) -> bool {
    let (mut unit, mut lines, mut pager, mut prio) = (false, false, false, false);
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-u" if !unit && args.get(i + 1).is_some_and(|u| service_ok(u)) => {
                unit = true;
                i += 2;
            }
            "-n" if !lines && args.get(i + 1).is_some_and(|n| lines_ok(n)) => {
                lines = true;
                i += 2;
            }
            "-p" if !prio && args.get(i + 1).is_some_and(|p| ["emerg", "alert", "crit", "err", "warning", "notice", "info", "debug"].contains(p)) => {
                prio = true;
                i += 2;
            }
            "--no-pager" if !pager => {
                pager = true;
                i += 1;
            }
            _ => return false,
        }
    }
    // a unit is required: the whole journal holds what other programs logged, secrets typed on a command line included
    unit && lines
}

fn ls(args: &[&str]) -> bool {
    let (flags, path) = match args {
        [] => return true,
        [f] if f.starts_with('-') => (*f, None),
        [f, p] if f.starts_with('-') => (*f, Some(*p)),
        [p] => ("", Some(*p)),
        _ => return false,
    };
    let flags_ok = flags.is_empty() || ["-l", "-a", "-la", "-al", "-lh", "-lah", "-alh", "-1"].contains(&flags);
    flags_ok && path.is_none_or(path_ok)
}

/// An absolute, already normalised path that is not one of the places that hold secrets.
pub fn path_ok(p: &str) -> bool {
    if !p.starts_with('/') || p.len() > 200 || !p.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+".contains(&b)) {
        return false;
    }
    if p == "/" {
        return true;
    }
    let parts: Vec<&str> = p[1..].trim_end_matches('/').split('/').collect();
    if parts.iter().any(|c| c.is_empty() || *c == "." || *c == ".." || c.starts_with('-')) {
        return false;
    }
    let lower: Vec<String> = parts.iter().map(|c| c.to_ascii_lowercase()).collect();
    let secret_dir = |c: &str| matches!(c, ".ssh" | ".gnupg" | ".aws" | ".kube" | ".docker" | ".config" | "secrets" | "private" | ".password-store" | "keyrings" | ".gnome2" | "gnome-keyring" | ".mozilla" | ".thunderbird");
    let secret_file = |c: &str| {
        c.starts_with("id_")
            || c.starts_with(".env")
            || c.starts_with("shadow")
            || c.starts_with("gshadow")
            || c.starts_with("sudoers")
            || c.starts_with("ssh_host_")
            || c.starts_with("authorized_keys")
            || [".netrc", ".pgpass", ".bash_history", ".zsh_history", ".git-credentials", ".npmrc", ".pypirc"].contains(&c)
            || [".key", ".pem", ".p12", ".pfx", ".kdbx", ".gpg", ".asc"].iter().any(|e| c.ends_with(e))
    };
    if lower.iter().any(|c| secret_dir(c) || secret_file(c)) {
        return false;
    }
    match lower[0].as_str() {
        "root" | "proc" | "boot" => false,
        "etc" => !matches!(
            lower.get(1).map(String::as_str),
            Some("ssh" | "ssl" | "sudoers.d" | "polkit-1" | "security" | "letsencrypt" | "wireguard" | "openvpn" | "krb5.keytab")
        ),
        "var" => {
            !matches!(lower.get(1).map(String::as_str), Some("lib") if matches!(lower.get(2).map(String::as_str), Some("docker" | "kubelet" | "private" | "secrets")))
        }
        _ => true,
    }
}

/// Logs of logins and of what people type (usernames with passwords by mistake, sudo lines, audit records).
fn log_holds_secrets(f: &str) -> bool {
    let lower = f.to_ascii_lowercase();
    lower.split('/').any(|c| {
        ["auth.log", "secure", "btmp", "wtmp", "lastlog", "faillog", "sudo.log", "audit", "keyrings", "syslog", "messages", "daemon.log", "user.log", "kern.log", "debug"].iter().any(|n| c == *n || (c.starts_with(n) && c[n.len()..].starts_with(['.', '-'])))
    })
}

fn log_file_ok(f: &str) -> bool {
    path_ok(f) && f.starts_with("/var/log/") && f.len() > "/var/log/".len() && !f.ends_with('/') && !log_holds_secrets(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Cases {
        allowed: Vec<String>,
        denied: Vec<String>,
    }

    #[test]
    fn shared_cases() {
        let c: Cases = serde_json::from_str(include_str!("allowlist-cases.json")).unwrap();
        for a in &c.allowed {
            assert!(check(a).is_ok(), "{a:?} should be allowed: {:?}", check(a));
        }
        for d in &c.denied {
            assert!(check(d).is_err(), "{d:?} should be denied");
        }
        assert!(c.allowed.len() >= 30 && c.denied.len() >= 120);
    }

    #[test]
    fn the_review_bypasses_are_denied() {
        for d in [
            "rm -rf /tmp/x",
            "find / -delete",
            "systemctl restart nginx",
            "curl http://198.51.100.7/x",
            "wget http://198.51.100.7/x",
            "sh",
            "bash -c id",
            "sudo uptime",
            "python3 -c 1",
            "dd if=/dev/zero of=/dev/sda",
            "kill 1",
            "reboot",
            "shutdown now",
            "chmod 777 /etc",
            "mv a b",
            "cp a b",
            "touch /tmp/x",
            "tee /tmp/x",
            "env",
            "nc -l 1234",
            "apt install x",
            "apt upgrade",
            "systemctl stop sshd",
            "journalctl -u x -n 5 --vacuum-size=1K",
            "tail -f /var/log/syslog",
        ] {
            assert!(check(d).is_err(), "{d}");
        }
    }
}
