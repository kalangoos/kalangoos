//! The engine: `kalango-instalador --instalar PLAN.json` (the password on stdin). It prepares the disk, copies the
//! system, adds the graphics drivers and the local model, and finishes (fstab, the account, language, keyboard, the
//! boot menu), telling the window how it goes, one JSON line at a time:
//!   {"etapa": "copiar", "estado": "fazendo", "progresso": 0.42, "texto": "4,1 GB de 6,6 GB"}
//!   {"etapa": "copiar", "estado": "feito", "texto": "6,6 GB"}         ·  {"fim": true}  ·  {"erro": "..."}
//!
//! It only touches a disk when it runs as root with KALANGO_INSTALADOR_REAL=1 (the installation image's launcher sets
//! it). Otherwise it simulates: every command it would run comes out as {"comando": "..."} and nothing is changed — the
//! session test and anyone curious can see exactly what an installation does.

use crate::discos::{self, Mode, Plan};
use kalango_i18n::{tr, trf};
use serde_json::json;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

pub const ETAPAS: &[&str] = &["disco", "copiar", "drivers", "modelo", "final"];

fn say(v: serde_json::Value) {
    println!("{v}");
    let _ = std::io::stdout().flush();
}

struct Ctx {
    real: bool,
    /// a short simulation (tests)
    fast: bool,
}

impl Ctx {
    /// Runs a command (or, simulating, says it); its output when it succeeds.
    fn run(&self, prog: &str, args: &[&str], input: Option<&str>) -> Result<String, String> {
        let shown = format!("{prog} {}", args.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.to_string() }).collect::<Vec<_>>().join(" "));
        if !self.real {
            say(json!({"comando": shown}));
            return Ok(String::new());
        }
        let mut c = Command::new(prog);
        c.args(args).env("LC_ALL", "C").stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child = c.spawn().map_err(|e| format!("{prog}: {e}"))?;
        if let (Some(i), Some(mut sin)) = (input, child.stdin.take()) {
            let _ = sin.write_all(i.as_bytes());
        }
        let o = child.wait_with_output().map_err(|e| format!("{prog}: {e}"))?;
        if !o.status.success() {
            return Err(format!("{shown}: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    }

    fn chroot(&self, args: &[&str], input: Option<&str>) -> Result<String, String> {
        let mut a = vec![TARGET];
        a.extend_from_slice(args);
        self.run("chroot", &a, input)
    }

    fn pause(&self, ms: u64) {
        if !self.real {
            std::thread::sleep(std::time::Duration::from_millis(if self.fast { ms / 20 } else { ms }));
        }
    }
}

const TARGET: &str = "/target";

fn step(etapa: &str, estado: &str, progresso: f64, texto: &str) {
    say(json!({"etapa": etapa, "estado": estado, "progresso": progresso, "texto": texto}));
}

/// The partition paths of a disk ("/dev/nvme0n1" → "/dev/nvme0n1p2"; "/dev/sda" → "/dev/sda2").
pub fn part(disk: &str, n: u32) -> String {
    if disk.chars().last().is_some_and(|c| c.is_ascii_digit()) { format!("{disk}p{n}") } else { format!("{disk}{n}") }
}

/// The disk ready: (EFI partition, /boot partition when the root is encrypted, the root partition), mounted at /target.
fn prepare_disk(c: &Ctx, p: &Plan) -> Result<(String, Option<String>, String), String> {
    let d = discos::read().into_iter().find(|d| d.path == p.disk);
    let (esp, boot, root) = match p.mode {
        Mode::Erase => {
            step("disco", "fazendo", 0.2, &trf("erasing the {disk}", &[("disk", p.disk_title.clone())]));
            c.run("sgdisk", &["--zap-all", &p.disk], None)?;
            // encrypted, the boot menu reads the kernel from a small /boot outside the encryption
            if p.encrypt {
                c.run("sgdisk", &["-n1:0:+512M", "-t1:EF00", "-c1:EFI", "-n2:0:+1G", "-t2:8300", "-c2:boot", "-n3:0:0", "-t3:8309", "-c3:KalangoOS", &p.disk], None)?;
            } else {
                c.run("sgdisk", &["-n1:0:+512M", "-t1:EF00", "-c1:EFI", "-n2:0:0", "-t2:8300", "-c2:KalangoOS", &p.disk], None)?;
            }
            c.run("partprobe", &[&p.disk], None)?;
            let esp = part(&p.disk, 1);
            c.run("mkfs.vfat", &["-F32", "-n", "EFI", &esp], None)?;
            if p.encrypt { (esp, Some(part(&p.disk, 2)), part(&p.disk, 3)) } else { (esp, None, part(&p.disk, 2)) }
        }
        Mode::Beside => {
            // the system's last partition gives up `size` bytes; KalangoOS gets them, the EFI partition is shared
            let d = d.ok_or_else(|| tr("the disk is not there any more").to_string())?;
            let esp = d.partitions.iter().find(|x| x.fstype == "vfat").map(|x| x.path.clone()).ok_or_else(|| tr("this disk has no EFI partition").to_string())?;
            let last = d.partitions.iter().filter(|x| x.fstype == "ext4" || x.fstype == "ntfs").max_by_key(|x| x.start).cloned().ok_or_else(|| tr("no partition to split").to_string())?;
            if last.start == 0 {
                return Err(tr("the partition's first sector is unknown").to_string());
            }
            let new_size_k = (last.size.saturating_sub(p.size)) / 1024;
            step("disco", "fazendo", 0.15, &trf("making room on {partition}", &[("partition", last.path.clone())]));
            let num: String = last.path.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
            if last.fstype == "ext4" {
                c.run("e2fsck", &["-f", "-y", &last.path], None)?;
                c.run("resize2fs", &[&last.path, &format!("{new_size_k}K")], None)?;
            } else {
                c.run("ntfsresize", &["--force", "--size", &format!("{new_size_k}k"), &last.path], None)?;
            }
            // made again where it starts (never "the first free sector", which may be elsewhere on the disk)
            c.run("sgdisk", &[&format!("-d{num}"), &format!("-n{num}:{}:+{new_size_k}K", last.start), &format!("-t{num}:{}", if last.fstype == "ntfs" { "0700" } else { "8300" }), &p.disk], None)?;
            let n_new = d.partitions.iter().filter(|x| x.path.starts_with(&p.disk)).count() as u32 + 1;
            if p.encrypt {
                c.run("sgdisk", &["-n0:0:+1G", "-t0:8300", "-c0:boot", "-n0:0:0", "-t0:8309", "-c0:KalangoOS", &p.disk], None)?;
            } else {
                c.run("sgdisk", &["-n0:0:0", "-t0:8300", "-c0:KalangoOS", &p.disk], None)?;
            }
            c.run("partprobe", &[&p.disk], None)?;
            if p.encrypt { (esp, Some(part(&p.disk, n_new)), part(&p.disk, n_new + 1)) } else { (esp, None, part(&p.disk, n_new)) }
        }
    };
    // encrypted: LUKS2 with the account's password (the boot asks for it; the design's "TPM" comes later)
    let dev = if p.encrypt {
        step("disco", "fazendo", 0.6, tr("encrypting"));
        c.run("cryptsetup", &["luksFormat", "--type", "luks2", "--batch-mode", "--key-file", "-", &root], Some(&p.password))?;
        c.run("cryptsetup", &["open", "--key-file", "-", &root, "kalango-raiz"], Some(&p.password))?;
        "/dev/mapper/kalango-raiz".to_string()
    } else {
        root.clone()
    };
    c.run("mkfs.ext4", &["-F", "-L", "KalangoOS", &dev], None)?;
    c.run("mkdir", &["-p", TARGET], None)?;
    c.run("mount", &[&dev, TARGET], None)?;
    if let Some(b) = &boot {
        c.run("mkfs.ext4", &["-F", "-L", "boot", b], None)?;
        c.run("mkdir", &["-p", &format!("{TARGET}/boot")], None)?;
        c.run("mount", &[b, &format!("{TARGET}/boot")], None)?;
    }
    c.run("mkdir", &["-p", &format!("{TARGET}/boot/efi")], None)?;
    c.run("mount", &[&esp, &format!("{TARGET}/boot/efi")], None)?;
    Ok((esp, boot, root))
}

fn copy_system(c: &Ctx) -> Result<(), String> {
    // the live image's system: its squashfs when there is one, else the running root
    let squash = ["/run/live/medium/casper/filesystem.squashfs", "/cdrom/casper/filesystem.squashfs", "/run/live/medium/live/filesystem.squashfs"].iter().find(|f| std::path::Path::new(f).is_file());
    if !c.real {
        for (i, feito) in [1.1, 2.8, 4.1, 5.6, 6.6].iter().enumerate() {
            let texto = trf("{done} of {total}", &[("done", discos::human((feito * 1e9) as u64)), ("total", discos::human(6_600_000_000))]);
            step("copiar", "fazendo", (i + 1) as f64 / 5.0, &texto);
            c.pause(1600);
        }
        return c.run("rsync", &["-aAXH", "--info=progress2", "--exclude=/dev/*", "--exclude=/proc/*", "--exclude=/sys/*", "--exclude=/tmp/*", "--exclude=/run/*", "--exclude=/mnt/*", "--exclude=/media/*", "--exclude=/target", "/", &format!("{TARGET}/")], None).map(|_| ());
    }
    let mut cmd = match squash {
        Some(s) => {
            let mut c = Command::new("unsquashfs");
            c.args(["-f", "-percentage", "-d", TARGET, s]);
            c
        }
        None => {
            let mut c = Command::new("rsync");
            c.args(["-aAXH", "--info=progress2", "--exclude=/dev/*", "--exclude=/proc/*", "--exclude=/sys/*", "--exclude=/tmp/*", "--exclude=/run/*", "--exclude=/mnt/*",
                "--exclude=/media/*", "--exclude=/target", "--exclude=/home/*", "/", &format!("{TARGET}/")]);
            c
        }
    };
    let mut child = cmd.env("LC_ALL", "C").stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    // both print a percentage now and then: the window's bar follows it
    let out = child.stdout.take().ok_or("sem saída")?;
    let mut last = -1.0;
    for line in std::io::BufReader::new(out).split(b'\n').flatten() {
        for part in line.split(|b| *b == b'\r') {
            if let Some(pct) = percentage(&String::from_utf8_lossy(part)).filter(|p| *p > last) {
                last = pct;
                step("copiar", "fazendo", pct / 100.0, &format!("{pct:.0}%"));
            }
        }
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err(tr("copying the system failed").to_string());
    }
    Ok(())
}

/// The installed system's boot settings (/etc/default/grub.d), from the live session's kernel command line.
fn grub_defaults(cmdline: &str) -> String {
    let consoles: Vec<&str> = cmdline.split_whitespace().filter(|a| a.starts_with("console=")).collect();
    let mut t = String::from("# KalangoOS (instalador)\nGRUB_DISTRIBUTOR=\"KalangoOS\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\"\n");
    if !consoles.is_empty() {
        t += &format!("GRUB_CMDLINE_LINUX=\"$GRUB_CMDLINE_LINUX {}\"\n", consoles.join(" "));
    }
    t
}

/// The percentage in a line of the copy: rsync's "… 42% …", or unsquashfs -percentage's bare "42".
fn percentage(line: &str) -> Option<f64> {
    let t = line.trim();
    if let Ok(n) = t.parse::<f64>() {
        return (0.0..=100.0).contains(&n).then_some(n);
    }
    t.split_whitespace().find(|w| w.ends_with('%')).and_then(|w| w.trim_end_matches('%').parse::<f64>().ok())
}

fn finish(c: &Ctx, p: &Plan, esp: &str, boot: Option<&str>, root: &str) -> Result<(), String> {
    step("final", "fazendo", 0.1, tr("the disks and the computer's name"));
    let uuid = |dev: &str| -> String {
        if !c.real {
            return format!("<uuid de {dev}>");
        }
        Command::new("blkid").args(["-s", "UUID", "-o", "value", dev]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    };
    let rootdev = if p.encrypt { "/dev/mapper/kalango-raiz".to_string() } else { root.to_string() };
    let mut fstab = format!("# KalangoOS (instalador)\nUUID={} / ext4 defaults,noatime 0 1\n", uuid(&rootdev));
    if let Some(b) = boot {
        fstab += &format!("UUID={} /boot ext4 defaults 0 2\n", uuid(b));
    }
    fstab += &format!("UUID={} /boot/efi vfat umask=0077 0 1\n", uuid(esp));
    write_file(c, "/etc/fstab", &fstab)?;
    if p.encrypt {
        write_file(c, "/etc/crypttab", &format!("kalango-raiz UUID={} none luks,discard\n", uuid(root)))?;
    }
    write_file(c, "/etc/hostname", &format!("{}\n", p.hostname))?;
    write_file(c, "/etc/hosts", &format!("127.0.0.1 localhost\n127.0.1.1 {}\n::1 localhost ip6-localhost ip6-loopback\n", p.hostname))?;
    write_file(c, "/etc/default/locale", &format!("LANG={}\n", p.locale))?;
    let (layout, variant) = p.keyboard.split_once(':').unwrap_or((&p.keyboard, ""));
    write_file(c, "/etc/default/keyboard", &format!("XKBMODEL=\"pc105\"\nXKBLAYOUT=\"{layout}\"\nXKBVARIANT=\"{variant}\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n"))?;
    step("final", "fazendo", 0.4, tr("the account"));
    // the running system's /dev and /sys (grub-install reads the EFI variables there) with what is under them, but
    // without sending mounts back; /proc and /run alone: /run's user folders hold the live session's own mounts (its
    // document portal), which a recursive copy would keep busy at the end (the first ISO's installer stopped there)
    for (how, b) in [("--rbind", "dev"), ("--rbind", "sys"), ("--bind", "proc"), ("--bind", "run")] {
        c.run("mount", &[how, &format!("/{b}"), &format!("{TARGET}/{b}")], None)?;
        if how == "--rbind" {
            c.run("mount", &["--make-rslave", &format!("{TARGET}/{b}")], None)?;
        }
    }
    c.chroot(&["useradd", "-m", "-s", "/bin/bash", "-G", "sudo", "-c", &p.name, &p.user], None)?;
    c.chroot(&["chpasswd"], Some(&format!("{}:{}\n", p.user, p.password)))?;
    if let Some(a) = &p.avatar {
        c.run("cp", &[a, &format!("{TARGET}/home/{}/.face", p.user)], None)?;
    }
    // the first session opens the first use (designs "Primeiro uso"); it also sets up the fingerprint when asked here
    write_file(c, &format!("/home/{}/.config/kalango/primeiro-uso.json", p.user), &format!("{{\"feito\": false, \"impressao_digital\": {}}}\n", p.fingerprint_later))?;
    // the private models server's access token, for the first session to move into the vault
    stage_models_token(c, p)?;
    // the licences accepted in the models step, for the account's download queue (it refuses the others)
    record_licences(c, p)?;
    // the model copied before the account existed, the face, the first use's file: all the person's
    c.chroot(&["chown", "-R", &format!("{0}:{0}", p.user), &format!("/home/{}", p.user)], None)?;
    if p.autologin {
        let greetd = format!("# greetd (/etc/greetd/config.toml)\n[terminal]\nvt = 1\n\n[default_session]\ncommand = \"/usr/libexec/kalango/kalango-entrada-sessao\"\nuser = \"greeter\"\n\n[initial_session]\ncommand = \"/usr/libexec/kalango/kalango-sessao\"\nuser = \"{}\"\n", p.user);
        write_file(c, "/etc/greetd/config.toml", &greetd)?;
    }
    step("final", "fazendo", 0.7, tr("the boot menu"));
    // the live boot (casper) came with the copy: the installed system does not need it, and its initramfs is made again
    // without it (and with the encryption, when there is one)
    if !c.real || std::path::Path::new(&format!("{TARGET}/var/lib/dpkg/info/casper.list")).exists() {
        c.chroot(&["dpkg", "--purge", "casper"], None)?;
    }
    // the boot menu says KalangoOS, and the installed system keeps the consoles it was installed through (a serial
    // console stays one)
    write_file(c, "/etc/default/grub.d/90-kalangoos.cfg", &grub_defaults(&std::fs::read_to_string("/proc/cmdline").unwrap_or_default()))?;
    c.chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--bootloader-id=KalangoOS", "--recheck"], None)?;
    // and the fallback loader (EFI/BOOT/BOOTX64.EFI), which any UEFI finds on its own: without it the disk only boots
    // while the firmware keeps the entry above — lost with a firmware reset, another computer or a VM's new NVRAM
    // (the installed system then fell back to the install media and asked to install again)
    c.chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--removable", "--no-nvram"], None)?;
    c.chroot(&["update-initramfs", "-u", "-k", "all"], None)?;
    c.chroot(&["update-grub"], None)?;
    // a fresh grubenv (the image's /boot/grub/grubenv is not a valid one: grub-editenv refused it, and the boot-success
    // services grub2-common and grub-initrd-fallback failed on every boot of the installed system)
    c.chroot(&["rm", "-f", "/boot/grub/grubenv"], None)?;
    c.chroot(&["grub-editenv", "/boot/grub/grubenv", "create"], None)?;
    // (anything still holding a mount under /target lets go when it ends: the disk is already written and synced)
    c.run("sync", &[], None)?;
    if c.run("umount", &["-R", TARGET], None).is_err() {
        c.run("umount", &["-R", "--lazy", TARGET], None)?;
    }
    if p.encrypt {
        c.run("cryptsetup", &["close", "kalango-raiz"], None)?;
    }
    Ok(())
}

/// `/etc/kalango/modelos.toml`: the source and the chosen models (no secret in it).
fn record_models_source(c: &Ctx, p: &Plan) -> Result<(), String> {
    write_file(c, kalango_modelos::config::SYSTEM_PATH, &p.models.to_toml())
}

/// The chosen models whose licence needs an acceptance the plan does not carry (with "later" nothing is chosen).
fn unaccepted_models(p: &Plan) -> Vec<String> {
    if p.models.fonte == kalango_modelos::config::SourceKind::Depois {
        return Vec::new();
    }
    let a = kalango_modelos::licenca::Aceites { lista: p.licencas.clone() };
    p.models.escolhidos.iter().filter(|id| !a.aceito(id)).cloned().collect()
}

/// `~/.config/kalango/modelos-licencas.json` of the new account: only the records that match the licence and version in
/// force (anything else in the plan is dropped, not trusted).
fn record_licences(c: &Ctx, p: &Plan) -> Result<(), String> {
    let valid: Vec<kalango_modelos::licenca::Aceite> = p.licencas.iter()
        .filter(|a| kalango_modelos::licenca::licenca_do_modelo(&a.modelo).is_some_and(|l| l.exige_aceite && l.id == a.licenca && l.versao == a.versao))
        .cloned().collect();
    if valid.is_empty() {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&json!({ "aceites": valid })).map_err(|e| e.to_string())?;
    write_file(c, &format!("/home/{}/.config/kalango/modelos-licencas.json", p.user), &(text + "\n"))
}

/// The token of a private server: a hand-off file in the new user's own folder (0600, the folder 0700), which the first
/// session's shell moves into the vault and deletes. The root-run engine cannot write the user's vault itself.
fn stage_models_token(c: &Ctx, p: &Plan) -> Result<(), String> {
    if p.models.fonte != kalango_modelos::config::SourceKind::Servidor || p.models_token.is_empty() {
        return Ok(());
    }
    if !c.real {
        say(json!({"comando": format!("guardar o token do servidor de modelos em {TARGET}/home/{}/.local/share/{} (valor omitido)", p.user, kalango_modelos::vault::STAGING_REL)}));
        return Ok(());
    }
    // (bound to the server's origin: the token never goes to another address)
    let origin = p.models.server().map_err(|e| format!("{e:?}"))?.origin();
    kalango_modelos::vault::write_staging(std::path::Path::new(&format!("{TARGET}/home/{}", p.user)), &origin, &kalango_modelos::Secret::new(p.models_token.clone())).map(|_| ()).map_err(|e| e.to_string())
}

fn write_file(c: &Ctx, path: &str, text: &str) -> Result<(), String> {
    if !c.real {
        say(json!({"comando": format!("escrever {TARGET}{path}"), "conteudo": text}));
        return Ok(());
    }
    let full = format!("{TARGET}{path}");
    if let Some(d) = std::path::Path::new(&full).parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&full, text).map_err(|e| format!("{full}: {e}"))
}

/// Runs the plan (the password comes on stdin); what it does goes to stdout.
/// The live session's home (the engine runs as root through pkexec, which says who asked).
fn live_home() -> String {
    if let Ok(uid) = std::env::var("PKEXEC_UID") {
        let out = Command::new("getent").args(["passwd", &uid]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        if let Some(h) = out.trim().split(':').nth(5).filter(|h| !h.is_empty()) {
            return h.to_string();
        }
    }
    std::env::var("SUDO_HOME").or_else(|_| std::env::var("HOME")).unwrap_or_default()
}

pub fn run(plan_path: &str) -> i32 {
    let mut p: Plan = match std::fs::read_to_string(plan_path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(p) => p,
        None => {
            say(json!({"erro": tr("the installation plan could not be read")}));
            return 2;
        }
    };
    let mut pw = String::new();
    let _ = std::io::stdin().read_line(&mut pw);
    p.password = pw.trim_end_matches('\n').to_string();
    // the access token of a private models server, on the second line (never in the plan file): empty when none
    let mut tok = String::new();
    let _ = std::io::stdin().read_line(&mut tok);
    p.models_token = tok.trim_end_matches('\n').to_string();
    // (before anything touches a disk: a source the system would reject is refused here)
    if let Err(e) = p.models.validate() {
        say(json!({"erro": trf("the AI models' source is not valid ({why})", &[("why", format!("{e:?}"))])}));
        return 2;
    }
    // (the licence of what will be downloaded must have been accepted: the same record the installed system asks for)
    let sem_aceite = unaccepted_models(&p);
    if !sem_aceite.is_empty() {
        say(json!({"erro": trf("the licence of these AI models was not accepted: {models}", &[("models", sem_aceite.join(", "))])}));
        return 2;
    }
    let root = std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"));
    let c = Ctx { real: root && std::env::var("KALANGO_INSTALADOR_REAL").as_deref() == Ok("1"), fast: std::env::var_os("KALANGO_INSTALADOR_RAPIDO").is_some() };
    say(json!({"modo": if c.real { "real" } else { "simulado" }, "disco": p.disk, "como": format!("{:?}", p.mode)}));
    let r = (|| -> Result<(), String> {
        step("disco", "fazendo", 0.0, &p.disk_title);
        let (esp, boot, root) = prepare_disk(&c, &p)?;
        c.pause(1200);
        step("disco", "feito", 1.0, &{
            let size = discos::human(if p.mode == Mode::Erase { discos::read().into_iter().find(|d| d.path == p.disk).map(|d| d.size).unwrap_or(p.size) } else { p.size });
            if p.encrypt { trf("{size} encrypted", &[("size", size)]) } else { size }
        });
        step("copiar", "fazendo", 0.0, "");
        copy_system(&c)?;
        step("copiar", "feito", 1.0, "");
        // graphics: NVIDIA's driver when the card is there (Ubuntu's own tool chooses the version)
        step("drivers", "fazendo", 0.0, "");
        let nvidia = Command::new("lspci").output().map(|o| String::from_utf8_lossy(&o.stdout).contains("NVIDIA")).unwrap_or(false);
        if nvidia {
            let _ = c.chroot(&["ubuntu-drivers", "install"], None);
        }
        c.pause(1500);
        step("drivers", "feito", 1.0, if nvidia { tr("NVIDIA driver") } else { tr("none needed") });
        // the local model: the live session's, when it has one (else it can come later)
        step("modelo", "fazendo", 0.0, "");
        let home = live_home();
        let models = std::path::Path::new(&home).join(".local/share/kalango/modelos");
        // (the system's own model, in the copy: the agent works from the first session)
        let in_system = std::fs::read_dir(format!("{TARGET}/usr/share/kalango/modelos")).map(|d| d.flatten().any(|e| e.path().extension().is_some_and(|x| x == "gguf"))).unwrap_or(false);
        if in_system {
            step("modelo", "feito", 1.0, tr("in the system"));
        } else if models.is_dir() {
            c.run("mkdir", &["-p", &format!("{TARGET}/home/{}/.local/share/kalango", p.user)], None)?;
            c.run("cp", &["-r", &models.display().to_string(), &format!("{TARGET}/home/{}/.local/share/kalango/", p.user)], None)?;
            step("modelo", "feito", 1.0, tr("copied"));
        } else {
            step("modelo", "pulado", 1.0, tr("later, from Settings"));
        }
        // where the AI models come from: recorded here, downloaded by the installed system after the first start
        record_models_source(&c, &p)?;
        c.pause(800);
        finish(&c, &p, &esp, boot.as_deref(), &root)?;
        step("final", "feito", 1.0, "");
        Ok(())
    })();
    match r {
        Ok(()) => {
            say(json!({"fim": true}));
            0
        }
        Err(e) => {
            say(json!({"erro": e}));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_source_is_written_without_any_token() {
        use kalango_modelos::config::{Config, SourceKind};
        let p = Plan { models: Config { fonte: SourceKind::Servidor, servidor: Some("https://modelos.example.org/k".into()), escolhidos: vec!["lfm3-1.7b".into()], ..Default::default() }, models_token: "tok-secret-123".into(), ..Default::default() };
        let t = p.models.to_toml();
        assert!(t.contains("fonte = \"servidor\"") && t.contains("escolhidos = [\"lfm3-1.7b\"]"));
        assert!(!t.contains("tok-secret-123"));
        // the plan file the window writes carries neither the password nor the token
        let j = serde_json::to_string(&p).unwrap();
        assert!(!j.contains("tok-secret-123"));
    }

    #[test]
    fn the_installed_system_keeps_the_install_consoles() {
        let g = grub_defaults("BOOT_IMAGE=/casper/vmlinuz boot=casper quiet splash console=ttyS0,115200 console=tty0 ---");
        assert!(g.contains("GRUB_DISTRIBUTOR=\"KalangoOS\""));
        assert!(g.contains("GRUB_CMDLINE_LINUX=\"$GRUB_CMDLINE_LINUX console=ttyS0,115200 console=tty0\""));
        assert!(!g.contains("boot=casper"));
        // (no console given: nothing added)
        assert!(!grub_defaults("boot=casper quiet splash ---").contains("GRUB_CMDLINE_LINUX="));
    }

    #[test]
    fn the_copy_progress_is_read_from_both_tools() {
        assert_eq!(percentage("42"), Some(42.0));
        assert_eq!(percentage("  1,234,567  42%   10.5MB/s    0:01:02"), Some(42.0));
        assert_eq!(percentage("Parallel unsquashfs: Using 4 processors"), None);
        assert_eq!(percentage("123456 inodes"), None);
        assert_eq!(percentage("4096"), None);
    }

    #[test]
    fn partitions_are_named_as_the_kernel_does() {
        assert_eq!(part("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
        assert_eq!(part("/dev/sda", 1), "/dev/sda1");
        assert_eq!(part("/dev/mmcblk0", 3), "/dev/mmcblk0p3");
    }
}
