//! The networks where the person's agents may not use the internet (Configurações › Rede › "Permitir agentes
//! usarem esta rede"): kept for each person in kalangod's state, and checked by `web.ler` against the connections
//! active now (NetworkManager's). A metered phone hotspot, a café's network: the person decides, the agents follow.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn arquivo(state_dir: &Path) -> PathBuf {
    state_dir.join("redes-sem-agentes.json")
}

fn tudo(state_dir: &Path) -> Value {
    std::fs::read(arquivo(state_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

/// The networks this person keeps the agents off.
pub fn lista(state_dir: &Path, uid: u32) -> Vec<String> {
    tudo(state_dir)[uid.to_string()].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Allows or keeps the agents off a network (by its connection's name); the list as it is now.
pub fn definir(state_dir: &Path, uid: u32, rede: &str, permitir: bool) -> Result<Vec<String>, String> {
    if rede.is_empty() || rede.len() > 256 || rede.contains('\n') {
        return Err(kalango_i18n::tr("invalid network name").into());
    }
    let mut v = tudo(state_dir);
    let mut l = lista(state_dir, uid);
    l.retain(|x| x != rede);
    let ids_key = format!("{uid}_identidades");
    if !v[&ids_key].is_object() {
        v[&ids_key] = json!({});
    }
    if let Some(o) = v[&ids_key].as_object_mut() {
        o.remove(rede);
    }
    if !permitir {
        l.push(rede.to_string());
        // what else tells this network apart, while it is on: its profile's id, the Wi-Fi's name, the router's
        // address — so another profile for the same network (a second "Casa", the cable) is kept off too
        if let Some(id) = ativas_ou_nada().unwrap_or_default().into_iter().find(|a| a.nome == rede) {
            v[&ids_key][rede] = json!({"uuid": id.uuid, "ssid": id.ssid, "gw_mac": id.gw_mac});
        } else if let Some((uuid, ssid)) = perfil(rede) {
            // not on it now: its profile's id and the Wi-Fi it is for, from NetworkManager's settings (the router's
            // address is only known while on it)
            v[&ids_key][rede] = json!({"uuid": uuid, "ssid": ssid, "gw_mac": ""});
        }
    }
    v[uid.to_string()] = json!(l);
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let tmp = arquivo(state_dir).with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, arquivo(state_dir)).map_err(|e| e.to_string())?;
    Ok(l)
}

/// NetworkManager's command (a stand-in in the tests).
pub fn nmcli() -> String {
    std::env::var("KALANGO_NMCLI").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "nmcli".into())
}

/// One active connection, with what tells its network apart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rede {
    pub nome: String,
    pub uuid: String,
    /// the Wi-Fi network's name (empty for a cable)
    pub ssid: String,
    /// the router's hardware address on that connection (from the default route and the ARP table)
    pub gw_mac: String,
}

/// One line of nmcli's terse output: fields split at the colons that are not escaped (`\:`), with `\\` a backslash.
pub fn campos(linha: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut it = linha.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => {
                if let Some(n) = it.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

/// Runs nmcli, waiting three seconds at most; tried twice (a slow NetworkManager is not yet a missing one).
fn nmcli_saida(args: &[&str]) -> Option<String> {
    use std::io::Read;
    for _ in 0..2 {
        let Ok(mut c) = std::process::Command::new(nmcli())
            .args(args)
            .env("LC_ALL", "C")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return None;
        };
        let t0 = std::time::Instant::now();
        let st = loop {
            match c.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if t0.elapsed() < std::time::Duration::from_secs(3) => std::thread::sleep(std::time::Duration::from_millis(20)),
                _ => {
                    let _ = c.kill();
                    let _ = c.wait();
                    break None;
                }
            }
        };
        if let Some(st) = st {
            let mut out = String::new();
            c.stdout.take()?.read_to_string(&mut out).ok()?;
            return st.success().then_some(out);
        }
    }
    None
}

/// The SSID a Wi-Fi profile is for (NetworkManager's stored setting), empty for other kinds.
fn ssid_do_perfil(uuid: &str) -> String {
    nmcli_saida(&["-g", "802-11-wireless.ssid", "connection", "show", uuid])
        .and_then(|o| o.lines().next().map(|l| campos(l).join(":")))
        .unwrap_or_default()
}

/// A saved profile by its name (active or not): its id and, for a Wi-Fi, the network it is for.
pub fn perfil(nome: &str) -> Option<(String, String)> {
    let out = nmcli_saida(&["-t", "-f", "NAME,UUID,TYPE", "connection", "show"])?;
    out.lines().map(campos).find(|f| f.first().map(String::as_str) == Some(nome)).map(|f| {
        let uuid = f.get(1).cloned().unwrap_or_default();
        let ssid = if f.get(2).map(String::as_str) == Some("802-11-wireless") { ssid_do_perfil(&uuid) } else { String::new() };
        (uuid, ssid)
    })
}

/// Where the route and ARP tables are read (/proc/net; a folder of the test's in the tests).
fn proc_net() -> PathBuf {
    std::env::var_os("KALANGO_REDES_PROC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/proc/net"))
}

/// The hardware address of the default gateway on `device`, from the route and ARP tables.
pub fn gateway_mac(route: &str, arp: &str, device: &str) -> String {
    let gw = route.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() > 7 && f[0] == device && f[1] == "00000000").then(|| u32::from_str_radix(f[2], 16).ok()).flatten()
    });
    let Some(gw) = gw.map(|v| std::net::Ipv4Addr::from(v.to_le_bytes()).to_string()) else { return String::new() };
    arp.lines()
        .skip(1)
        .find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() > 5 && f[0] == gw && f[5] == device && f[3] != "00:00:00:00:00:00").then(|| f[3].to_lowercase())
        })
        .unwrap_or_default()
}

/// The connections active now, or None when NetworkManager could not tell (nmcli missing, failing, or taking more
/// than three seconds twice).
pub fn ativas_ou_nada() -> Option<Vec<Rede>> {
    let out = nmcli_saida(&["-t", "-f", "NAME,UUID,TYPE,DEVICE", "connection", "show", "--active"])?;
    let route = std::fs::read_to_string(proc_net().join("route")).unwrap_or_default();
    let arp = std::fs::read_to_string(proc_net().join("arp")).unwrap_or_default();
    Some(
        out.lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let f = campos(l);
                let get = |i: usize| f.get(i).cloned().unwrap_or_default();
                let (nome, uuid, tipo, dev) = (get(0), get(1), get(2), get(3));
                let ssid = if tipo == "802-11-wireless" && !uuid.is_empty() { ssid_do_perfil(&uuid) } else { String::new() };
                Rede { nome, uuid, ssid, gw_mac: gateway_mac(&route, &arp, &dev) }
            })
            .collect(),
    )
}

/// Whether an active connection is one of the networks kept off the agents: by its name, or by what was recorded of
/// the network when the person kept it off (the same profile, the same Wi-Fi, the same router).
pub fn corresponde(a: &Rede, nome: &str, id: &Value) -> bool {
    let same = |k: &str, v: &str| !v.is_empty() && id[k].as_str() == Some(v);
    a.nome == nome || same("uuid", &a.uuid) || same("ssid", &a.ssid) || same("gw_mac", &a.gw_mac)
}

/// The people (uids) who keep the agents off some network.
pub fn pessoas(state_dir: &Path) -> Vec<u32> {
    tudo(state_dir)
        .as_object()
        .map(|o| o.iter().filter(|(_, v)| v.as_array().is_some_and(|a| !a.is_empty())).filter_map(|(k, _)| k.parse().ok()).collect())
        .unwrap_or_default()
}

/// The active network this person keeps the agents off, if they are on one.
pub fn bloqueada(state_dir: &Path, uid: u32) -> Option<String> {
    let l = lista(state_dir, uid);
    if l.is_empty() {
        return None;
    }
    let ids = tudo(state_dir)[format!("{uid}_identidades")].clone();
    match ativas_ou_nada() {
        Some(a) => l.iter().find(|n| a.iter().any(|x| corresponde(x, n, &ids[n.as_str()]))).cloned(),
        // the person keeps the agents off some network and which one this computer is on cannot be told: as if it
        // were one of them (never open because NetworkManager did not answer)
        None => Some(kalango_i18n::tr("an unknown network (NetworkManager did not answer)").to_string()),
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn terse_fields_are_unescaped() {
        assert_eq!(campos("Casa\\: 2:abc:802-11-wireless:wlan0"), vec!["Casa: 2", "abc", "802-11-wireless", "wlan0"]);
        assert_eq!(campos("Barra\\\\Invertida:u"), vec!["Barra\\Invertida", "u"]);
    }

    #[test]
    fn the_router_s_address_is_read() {
        let route = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\nwlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\n";
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n192.168.0.1      0x1         0x2         AA:BB:CC:00:11:22     *        wlan0\n";
        assert_eq!(gateway_mac(route, arp, "wlan0"), "aa:bb:cc:00:11:22");
        assert_eq!(gateway_mac(route, arp, "eth0"), "");
    }

    /// A NetworkManager stand-in whose active connection is set by a file (and that answers the SSID query).
    fn stand_in(d: &Path) -> PathBuf {
        let f = d.join("nmcli");
        std::fs::write(&f, format!("#!/bin/sh\ncase \"$*\" in *--active*) cat {0}/ativa;; *802-11-wireless.ssid*) cat {0}/ssid; echo;; *NAME,UUID,TYPE*) cat {0}/perfis;; esac\n", d.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        f
    }

    #[test]
    fn the_same_network_under_another_profile_is_kept_off() {
        let _g = crate::redes::TESTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let st = d.path().join("estado");
        let proc = d.path().join("proc");
        std::fs::create_dir_all(&proc).unwrap();
        std::fs::write(proc.join("route"), "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\nwlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\neth0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\n").unwrap();
        std::fs::write(proc.join("arp"), "IP address HW type Flags HW address Mask Device\n192.168.0.1 0x1 0x2 aa:bb:cc:00:11:22 * wlan0\n192.168.0.1 0x1 0x2 aa:bb:cc:00:11:22 * eth0\n").unwrap();
        std::env::set_var("KALANGO_NMCLI", stand_in(d.path()));
        std::env::set_var("KALANGO_REDES_PROC", &proc);
        std::fs::write(d.path().join("ssid"), "CasaWiFi").unwrap();
        // the person keeps the agents off "Casa" while on it
        std::fs::write(d.path().join("ativa"), "Casa:uuid-1:802-11-wireless:wlan0\n").unwrap();
        definir(&st, 1000, "Casa", false).unwrap();
        assert_eq!(bloqueada(&st, 1000).as_deref(), Some("Casa"));
        // a second profile for the same Wi-Fi
        std::fs::write(d.path().join("ativa"), "Casa 1:uuid-2:802-11-wireless:wlan0\n").unwrap();
        assert_eq!(bloqueada(&st, 1000).as_deref(), Some("Casa"), "the same Wi-Fi under another name");
        // the cable at home (the same router)
        std::fs::write(d.path().join("ativa"), "Cabo:uuid-3:802-3-ethernet:eth0\n").unwrap();
        assert_eq!(bloqueada(&st, 1000).as_deref(), Some("Casa"), "the same router by cable");
        // a name with a backslash or a colon still matches itself
        std::fs::write(d.path().join("ativa"), "Caf\\\\e\\:2:uuid-9:802-3-ethernet:eth9\n").unwrap();
        definir(&st, 1001, "Caf\\e:2", false).unwrap();
        assert_eq!(bloqueada(&st, 1001).as_deref(), Some("Caf\\e:2"));
        // another network (another router, another Wi-Fi) is not
        std::fs::write(d.path().join("ssid"), "Cafe").unwrap();
        std::fs::write(proc.join("arp"), "IP address HW type Flags HW address Mask Device\n192.168.0.1 0x1 0x2 de:ad:be:ef:00:01 * wlan0\n").unwrap();
        std::fs::write(d.path().join("ativa"), "Cafeteria:uuid-4:802-11-wireless:wlan0\n").unwrap();
        assert_eq!(bloqueada(&st, 1000), None);
        // a profile kept off while not on it: its Wi-Fi is known from NetworkManager's settings
        std::fs::write(d.path().join("perfis"), "Escritorio:uuid-7:802-11-wireless\n").unwrap();
        std::fs::write(d.path().join("ssid"), "EscritorioWiFi").unwrap();
        std::fs::write(d.path().join("ativa"), "Outra:uuid-8:802-3-ethernet:eth5\n").unwrap();
        definir(&st, 1002, "Escritorio", false).unwrap();
        std::fs::write(d.path().join("ativa"), "Escritorio 2:uuid-10:802-11-wireless:wlan0\n").unwrap();
        assert_eq!(bloqueada(&st, 1002).as_deref(), Some("Escritorio"), "another profile of that Wi-Fi");
        std::fs::write(d.path().join("ssid"), "Cafe").unwrap();
        // allowing it again forgets what told it apart
        definir(&st, 1000, "Casa", true).unwrap();
        assert!(tudo(&st)["1000_identidades"].get("Casa").is_none());
        std::env::remove_var("KALANGO_NMCLI");
        std::env::remove_var("KALANGO_REDES_PROC");
    }
}

/// The tests that point NetworkManager's stand-in elsewhere (process-wide variables) run one at a time.
#[cfg(test)]
pub static TESTE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod unknown_tests {
    #[test]
    fn an_unknown_network_counts_as_kept_off() {
        let _g = crate::redes::TESTE.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        super::definir(d.path(), 1000, "Celular", false).unwrap();
        // a NetworkManager that fails, and one that hangs
        for script in ["#!/bin/sh\nexit 1\n", "#!/bin/sh\nsleep 30\n"] {
            let f = d.path().join("nmcli");
            std::fs::write(&f, script).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var("KALANGO_NMCLI", &f);
            let t0 = std::time::Instant::now();
            assert!(super::bloqueada(d.path(), 1000).is_some(), "{script}");
            assert!(t0.elapsed() < std::time::Duration::from_secs(8), "tried twice, three seconds each");
        }
        // nobody keeps the agents off anything: nothing is blocked, whatever NetworkManager says
        assert!(super::bloqueada(d.path(), 1001).is_none());
        std::env::remove_var("KALANGO_NMCLI");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_person_keeps_their_own_list() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(super::definir(d.path(), 1000, "Celular", false).unwrap(), vec!["Celular"]);
        super::definir(d.path(), 1001, "Café", false).unwrap();
        assert_eq!(super::lista(d.path(), 1000), vec!["Celular"]);
        assert_eq!(super::definir(d.path(), 1000, "Celular", true).unwrap(), Vec::<String>::new());
        assert_eq!(super::lista(d.path(), 1001), vec!["Café"]);
        assert!(super::definir(d.path(), 1000, "", false).is_err());
    }
}
