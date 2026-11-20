//! GPUs para máquinas virtuais (Kalango VirtualDesk › Ajustes › Vídeo e GPU): entregar uma GPU inteira ao vfio-pci
//! (passagem para uma máquina), devolvê-la ao driver de antes e ligar o IOMMU na linha do kernel.
//!
//! Tudo é nível sistema: o kalangod pede aprovação, cria o ponto de restauração e registra na linha do tempo com o
//! inverso. A proteção que não se negocia: **a GPU que mostra a tela deste computador nunca é entregue** (uma saída
//! conectada, a placa de boot ou a única GPU) — tirá-la do driver apagaria a tela de quem aprovou. Como o grupo IOMMU
//! inteiro vai junto, a mesma conferência vale para cada dispositivo do grupo, e só as funções da própria placa (mesmo
//! barramento e dispositivo PCI: áudio, USB, serial) podem acompanhá-la; outra GPU ou qualquer outra coisa no grupo
//! faz recusar.

use super::{n_, obj, s, tr, trf, Ctx, Out, Tool};
use crate::policy::{Effect, Level};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "gpu.listar", description: n_("GPUs of this computer for virtual machines: driver, IOMMU group, whether it shows the screen, and whether the IOMMU is on"),
            level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: super::none, run: listar },
        Tool { name: "gpu.vfio_reservar", description: n_("Gives a whole GPU (and the rest of its IOMMU group) to vfio-pci so a virtual machine can use it; never the GPU that shows the screen"),
            level: Level::Sistema, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"endereco": {"type": "string", "description": "endereço PCI, ex.: 0000:0c:00.0"}}), &["endereco"]), run: reservar },
        Tool { name: "gpu.vfio_devolver", description: n_("Gives a GPU back from vfio-pci to this computer's drivers"),
            level: Level::Sistema, effect: Effect::Escreve, reversible: false,
            schema: || obj(json!({"endereco": {"type": "string"}}), &["endereco"]), run: devolver },
        Tool { name: "gpu.iommu_ligar", description: n_("Turns the IOMMU on in the kernel line (GRUB); takes effect after a restart"),
            level: Level::Sistema, effect: Effect::Escreve, reversible: true, schema: super::none, run: iommu_ligar },
        Tool { name: "gpu.iommu_desligar", description: n_("Takes the IOMMU options out of the kernel line (GRUB); takes effect after a restart"),
            level: Level::Sistema, effect: Effect::Escreve, reversible: true, schema: super::none, run: iommu_desligar },
    ]
}

/// Uma GPU como o /sys mostra.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub addr: String,
    pub driver: String,
    pub group: Option<String>,
    /// os outros dispositivos do grupo IOMMU (vão junto), com a classe
    pub group_devices: Vec<(String, String)>,
    pub boot_vga: bool,
    pub connected_outputs: usize,
    /// algum outro dispositivo do grupo mostra a tela (saída conectada ou placa de boot)
    pub group_drives_display: bool,
}

/// O dispositivo mostra a tela agora: uma saída conectada (drm/cardN/cardN-*/status) ou a placa de boot.
fn drives_display(dev: &Path) -> (bool, usize) {
    let mut connected = 0;
    for card in std::fs::read_dir(dev.join("drm")).map(|r| r.flatten().collect::<Vec<_>>()).unwrap_or_default() {
        for o in std::fs::read_dir(card.path()).map(|r| r.flatten().collect::<Vec<_>>()).unwrap_or_default() {
            if read(&o.path().join("status")) == "connected" {
                connected += 1;
            }
        }
    }
    (read(&dev.join("boot_vga")) == "1", connected)
}

/// "0000:10:00.1" → "0000:10:00" (barramento e dispositivo; o que muda entre as funções de uma placa é o ".N").
fn slot(addr: &str) -> &str {
    addr.rsplit_once('.').map(|(a, _)| a).unwrap_or(addr)
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn link_name(p: &Path) -> String {
    std::fs::read_link(p).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default()
}

/// As GPUs (classe 0x03) sob uma raiz /sys.
pub fn cards(sys: &Path) -> Vec<Card> {
    let pci = sys.join("bus/pci/devices");
    let mut out = vec![];
    let mut all: Vec<PathBuf> = std::fs::read_dir(&pci).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    all.sort();
    for d in all {
        if !read(&d.join("class")).starts_with("0x03") {
            continue;
        }
        let addr = d.file_name().unwrap_or_default().to_string_lossy().to_string();
        let group = Some(link_name(&d.join("iommu_group"))).filter(|g| !g.is_empty());
        let mut group_devices = vec![];
        if let Some(g) = &group {
            for m in std::fs::read_dir(sys.join(format!("kernel/iommu_groups/{g}/devices"))).map(|r| r.flatten().collect::<Vec<_>>()).unwrap_or_default() {
                let a = m.file_name().to_string_lossy().to_string();
                if a != addr {
                    group_devices.push((a.clone(), read(&pci.join(&a).join("class"))));
                }
            }
            group_devices.sort();
        }
        let (boot_vga, connected) = drives_display(&d);
        let group_drives_display = group_devices.iter().any(|(a, _)| {
            let (b, n) = drives_display(&pci.join(a));
            b || n > 0
        });
        out.push(Card { driver: link_name(&d.join("driver")), group, group_devices, boot_vga, connected_outputs: connected, group_drives_display, addr });
    }
    out
}

/// Por que esta GPU não pode ser entregue (None: pode). A da tela nunca, nem por estar no grupo de outra.
pub fn refuse(all: &[Card], c: &Card, iommu_on: bool) -> Option<String> {
    // cada dispositivo que seria tirado do driver: a placa e o grupo inteiro
    if c.connected_outputs > 0 || c.boot_vga || all.len() == 1 || c.group_drives_display {
        return Some(tr("this GPU shows this computer's screen: giving it to a machine would turn the screen off").into());
    }
    if !iommu_on || c.group.is_none() {
        return Some(tr("the IOMMU is off").into());
    }
    // só as funções da própria placa (mesmo barramento e dispositivo) vão junto: áudio, USB e serial dela
    let own = |a: &str, k: &str| slot(a) == slot(&c.addr) && (k.starts_with("0x0403") || k.starts_with("0x0c03") || k.starts_with("0x0c80"));
    if let Some((a, k)) = c.group_devices.iter().find(|(a, k)| !own(a, k)) {
        if k.starts_with("0x03") {
            return Some(trf("its IOMMU group also has the GPU {a}, which would go along", &[("a", a.clone())]));
        }
        return Some(trf("its IOMMU group also has {a}, which is not part of the GPU", &[("a", a.clone())]));
    }
    None
}

fn valid_addr(a: &str) -> Result<&str, String> {
    let ok = a.len() == 12 && a.chars().enumerate().all(|(i, ch)| match i {
        4 | 7 => ch == ':',
        10 => ch == '.',
        _ => ch.is_ascii_hexdigit(),
    });
    if ok { Ok(a) } else { Err(tr("invalid PCI address").into()) }
}

fn iommu_on(sys: &Path) -> bool {
    std::fs::read_dir(sys.join("kernel/iommu_groups")).map(|r| r.count() > 0).unwrap_or(false)
}

fn listar(_: &Ctx, _: &Value) -> Result<Out, String> {
    let sys = Path::new("/sys");
    let all = cards(sys);
    let on = iommu_on(sys);
    let v: Vec<Value> = all.iter().map(|c| json!({
        "endereco": c.addr, "driver": c.driver, "grupo_iommu": c.group, "grupo": c.group_devices, "mostra_a_tela": c.connected_outputs > 0 || c.boot_vga || all.len() == 1 || c.group_drives_display,
        "pode_ser_reservada": refuse(&all, c, on).is_none(), "motivo": refuse(&all, c, on),
    })).collect();
    let n = v.len();
    Ok(Out::read(json!({"gpus": v, "iommu_ligado": on, "linha_do_kernel": read(Path::new("/proc/cmdline"))}), trf("{n} GPUs", &[("n", n.to_string())])))
}

/// Liga um dispositivo a um driver pelo driver_override (vazio: o driver padrão do kernel).
fn rebind(sys: &Path, addr: &str, driver: &str) -> Result<(), String> {
    let d = sys.join("bus/pci/devices").join(addr);
    std::fs::write(d.join("driver_override"), if driver.is_empty() { "\n" } else { driver }).map_err(|e| format!("{addr}: {e}"))?;
    if d.join("driver").exists() {
        std::fs::write(d.join("driver/unbind"), addr).map_err(|e| format!("{addr}: {e}"))?;
    }
    std::fs::write(sys.join("bus/pci/drivers_probe"), addr).map_err(|e| format!("{addr}: {e}"))?;
    Ok(())
}

fn reservar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let addr = valid_addr(s(a, "endereco")?)?.to_string();
    let sys = Path::new("/sys");
    let all = cards(sys);
    let card = all.iter().find(|x| x.addr == addr).ok_or_else(|| trf("there is no GPU at {a}", &[("a", addr.clone())]))?;
    if let Some(why) = refuse(&all, card, iommu_on(sys)) {
        return Err(why);
    }
    let mut devices = vec![(card.addr.clone(), card.driver.clone())];
    devices.extend(card.group_devices.iter().map(|(d, _)| (d.clone(), link_name(&sys.join("bus/pci/devices").join(d).join("driver")))));
    if !c.cfg.dry_run {
        let _ = std::process::Command::new("modprobe").arg("vfio-pci").status();
        for (d, _) in &devices {
            rebind(sys, d, "vfio-pci")?;
        }
    }
    let before: serde_json::Map<String, Value> = devices.iter().map(|(d, drv)| (d.clone(), json!(drv))).collect();
    Ok(Out {
        value: json!({"endereco": addr, "dispositivos": before, "simulado": c.cfg.dry_run}),
        summary: trf("GPU {a} given to vfio-pci ({n} devices)", &[("a", addr.clone()), ("n", devices.len().to_string())]),
        inverse: Some(json!({"ferramenta": "gpu.vfio_devolver", "argumentos": {"endereco": addr}})),
    })
}

fn devolver(c: &Ctx, a: &Value) -> Result<Out, String> {
    let addr = valid_addr(s(a, "endereco")?)?.to_string();
    let sys = Path::new("/sys");
    let card = cards(sys).into_iter().find(|x| x.addr == addr).ok_or_else(|| trf("there is no GPU at {a}", &[("a", addr.clone())]))?;
    let mut devices = vec![card.addr.clone()];
    devices.extend(card.group_devices.iter().map(|(d, _)| d.clone()));
    if !c.cfg.dry_run {
        for d in &devices {
            rebind(sys, d, "")?;
        }
    }
    Ok(Out { value: json!({"endereco": addr, "dispositivos": devices, "simulado": c.cfg.dry_run}), summary: trf("GPU {a} back to this computer", &[("a", addr)]), inverse: None })
}

// ------------------------------------------------------------------ IOMMU na linha do kernel

/// Um arquivo só nosso em grub.d: os arquivos de /etc/default/grub.d são lidos depois de /etc/default/grub (o
/// instalador grava ali 90-kalangoos.cfg com a linha do kernel), então editar /etc/default/grub não valeria nada.
/// Este, lido depois do 90, acrescenta ao que já estiver definido; desligar é apagá-lo.
const IOMMU_CFG: &str = "/etc/default/grub.d/95-kalangoos-iommu.cfg";

pub fn iommu_args(cpuinfo: &str) -> [&'static str; 2] {
    if cpuinfo.contains("GenuineIntel") { ["intel_iommu=on", "iommu=pt"] } else { ["amd_iommu=on", "iommu=pt"] }
}

/// O conteúdo do arquivo do IOMMU. Puro.
pub fn iommu_cfg(args: &[&str]) -> String {
    format!(
        "# KalangoOS: IOMMU para passar GPUs às máquinas virtuais (kalangod, gpu.iommu_ligar; gpu.iommu_desligar apaga)\nGRUB_CMDLINE_LINUX_DEFAULT=\"$GRUB_CMDLINE_LINUX_DEFAULT {}\"\n",
        args.join(" ")
    )
}

fn update_grub() -> String {
    std::process::Command::new("update-grub")
        .output()
        .map(|o| if o.status.success() { String::new() } else { String::from_utf8_lossy(&o.stderr).trim().to_string() })
        .unwrap_or_else(|e| e.to_string())
}

fn iommu_change(c: &Ctx, add: bool) -> Result<Out, String> {
    let args = iommu_args(&read(Path::new("/proc/cpuinfo")));
    let path = Path::new(IOMMU_CFG);
    let had = path.exists();
    let mut note = String::new();
    if !c.cfg.dry_run {
        if add {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(path, iommu_cfg(&args)).map_err(|e| format!("{IOMMU_CFG}: {e}"))?;
        } else if had {
            std::fs::remove_file(path).map_err(|e| format!("{IOMMU_CFG}: {e}"))?;
        }
        note = update_grub();
        if !note.is_empty() {
            return Err(trf("update-grub failed: {e}", &[("e", note)]));
        }
    }
    let (summary, inverse) = if add {
        (trf("IOMMU on in the kernel line ({a}); restart to use it", &[("a", args.join(" "))]), "gpu.iommu_desligar")
    } else {
        (tr("IOMMU options out of the kernel line; restart to apply").to_string(), "gpu.iommu_ligar")
    };
    Ok(Out {
        value: json!({"opcoes": args, "arquivo": IOMMU_CFG, "ja_existia": had, "reiniciar": true, "update_grub": note, "simulado": c.cfg.dry_run}),
        summary,
        inverse: Some(json!({"ferramenta": inverse, "argumentos": {}})),
    })
}

fn iommu_ligar(c: &Ctx, _: &Value) -> Result<Out, String> {
    iommu_change(c, true)
}

fn iommu_desligar(c: &Ctx, _: &Value) -> Result<Out, String> {
    iommu_change(c, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Um /sys de mentira: `devs` = (endereço, classe, grupo, boot_vga, saída conectada).
    fn fake_sys(name: &str, devs: &[(&str, &str, u32, bool, bool)]) -> PathBuf {
        let s = std::env::temp_dir().join(format!("kalangod-gpu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&s);
        for (a, class, group, boot, connected) in devs {
            let d = s.join("devices").join(a);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("class"), class).unwrap();
            std::fs::write(d.join("boot_vga"), if *boot { "1" } else { "0" }).unwrap();
            let g = s.join(format!("kernel/iommu_groups/{group}/devices"));
            std::fs::create_dir_all(&g).unwrap();
            symlink(&d, g.join(a)).unwrap();
            symlink(s.join(format!("kernel/iommu_groups/{group}")), d.join("iommu_group")).unwrap();
            std::fs::create_dir_all(s.join("bus/pci/devices")).unwrap();
            symlink(&d, s.join("bus/pci/devices").join(a)).unwrap();
            if *connected {
                std::fs::create_dir_all(d.join("drm/card1/card1-DP-2")).unwrap();
                std::fs::write(d.join("drm/card1/card1-DP-2/status"), "connected").unwrap();
            }
        }
        s
    }

    fn verdict(name: &str, devs: &[(&str, &str, u32, bool, bool)], pick: &str) -> Option<String> {
        let s = fake_sys(name, devs);
        let all = cards(&s);
        let c = all.iter().find(|c| c.addr == pick).unwrap().clone();
        let r = refuse(&all, &c, true);
        let _ = std::fs::remove_dir_all(&s);
        r
    }

    const SCREEN: (&str, &str, u32, bool, bool) = ("0000:10:00.0", "0x030000", 9, true, true);

    #[test]
    fn a_card_with_its_own_audio_may_go() {
        let devs = [SCREEN, ("0000:0c:00.0", "0x030000", 15, false, false), ("0000:0c:00.1", "0x040300", 15, false, false), ("0000:0c:00.2", "0x0c0330", 15, false, false)];
        assert_eq!(verdict("propria", &devs, "0000:0c:00.0"), None);
    }

    #[test]
    fn two_gpus_in_one_group_are_refused() {
        // the free card shares the group with the screen's card: both would be unbound
        let devs = [SCREEN, ("0000:10:00.1", "0x040300", 9, false, false), ("0000:11:00.0", "0x030000", 9, false, false)];
        assert!(verdict("duas-tela", &devs, "0000:11:00.0").is_some(), "the screen's card in the group");
        // two free cards in one group, neither on the screen: still refused (the other GPU would go along)
        let devs = [SCREEN, ("0000:20:00.0", "0x030000", 4, false, false), ("0000:21:00.0", "0x030000", 4, false, false)];
        let r = verdict("duas-livres", &devs, "0000:20:00.0").unwrap();
        assert!(r.contains("0000:21:00.0"), "{r}");
    }

    #[test]
    fn a_group_with_a_network_card_is_refused() {
        let devs = [SCREEN, ("0000:30:00.0", "0x030000", 5, false, false), ("0000:30:00.1", "0x020000", 5, false, false)];
        assert!(verdict("rede", &devs, "0000:30:00.0").is_some());
        // audio of another card (other bus:device) is not this card's
        let devs = [SCREEN, ("0000:40:00.0", "0x030000", 6, false, false), ("0000:41:00.1", "0x040300", 6, false, false)];
        assert!(verdict("audio-alheio", &devs, "0000:40:00.0").is_some());
    }

    #[test]
    fn the_screen_s_gpu_and_a_single_gpu_are_refused() {
        let devs = [SCREEN, ("0000:10:00.1", "0x040300", 9, false, false)];
        assert!(verdict("unica", &devs, "0000:10:00.0").is_some(), "the only GPU, on the screen");
        let devs = [("0000:10:00.0", "0x030000", 9, false, false)];
        assert!(verdict("unica-sem-tela", &devs, "0000:10:00.0").is_some(), "the only GPU, even with no output seen");
        let devs = [SCREEN, ("0000:20:00.0", "0x030000", 4, false, true)];
        assert!(verdict("saida", &devs, "0000:20:00.0").is_some(), "a connected output");
    }

    #[test]
    fn the_iommu_file_adds_to_the_line_and_names_the_cpu() {
        let t = iommu_cfg(&iommu_args("vendor_id : GenuineIntel"));
        assert!(t.ends_with("GRUB_CMDLINE_LINUX_DEFAULT=\"$GRUB_CMDLINE_LINUX_DEFAULT intel_iommu=on iommu=pt\"\n"), "{t}");
        assert!(iommu_cfg(&iommu_args("vendor_id : AuthenticAMD")).contains("$GRUB_CMDLINE_LINUX_DEFAULT amd_iommu=on iommu=pt\""));
        // read after the installer's 90-kalangoos.cfg
        assert!(IOMMU_CFG.rsplit('/').next().unwrap() > "90-kalangoos.cfg");
        // what a shell (as grub-mkconfig) makes of it, after the installer's line
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("GRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\"\n{t}echo \"$GRUB_CMDLINE_LINUX_DEFAULT\"")])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "quiet splash intel_iommu=on iommu=pt");
    }

    #[test]
    fn addresses_are_checked() {
        assert!(valid_addr("0000:0c:00.0").is_ok());
        assert!(valid_addr("0c:00.0").is_err());
        assert!(valid_addr("0000:0c:00.0/../x").is_err());
    }
}
