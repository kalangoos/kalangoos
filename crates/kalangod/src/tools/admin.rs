//! Administração do sistema: rede, firewall, discos, usuários, atualizações, tarefas limitadas e pontos de
//! restauração. Cada ferramenta declara o contrato (nível, efeito, reversível) e, quando muda algo, o inverso.

use super::*;
use crate::limits::{self, Landlock, Pedido};
use crate::restore;

pub fn tools() -> Vec<Tool> {
    let t = |name, description, level, effect, reversible, schema: fn() -> Value, run: Run| Tool { name, description, level, effect, reversible, schema, run };
    use Effect::*;
    use Level::*;
    vec![
        // ---------------------------------------------------------------- rede
        t("rede.conexoes", n_("Saved and active network connections (NetworkManager)"), Observar, Leitura, false, none, net_list),
        t("rede.wifi", n_("Visible Wi-Fi networks, with signal and security"), Observar, Leitura, false, none, wifi_list),
        t("rede.conexao", n_("Turns a saved network connection on or off"), Sistema, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "acao": {"type": "string", "enum": ["ativar", "desativar"]}}), &["nome", "acao"]), net_conn),
        t("firewall.estado", n_("Firewall (ufw) state and numbered rules"), Observar, Leitura, false, none, fw_status),
        t("firewall.regra", n_("Allows or blocks a port in the firewall, or removes that rule"), Sistema, Escreve, true,
            || obj(json!({"acao": {"type": "string", "enum": ["permitir", "bloquear"]}, "porta": {"type": "string", "description": "ex.: 22/tcp, 8080, ssh"}, "remover": {"type": "boolean"}}), &["acao", "porta"]), fw_rule),
        t("firewall.ligar", n_("Turns the firewall on or off"), Sistema, Escreve, true, || obj(json!({"ligado": {"type": "boolean"}}), &["ligado"]), fw_toggle),
        // ---------------------------------------------------------------- placa de vídeo
        t("grafico.modo", n_("Which graphics card the system uses (hybrid, dedicated or integrated); takes effect after restarting"), Sistema, Escreve, true,
            || obj(json!({"modo": {"type": "string", "enum": ["hibrido", "dedicado", "integrado"]}}), &["modo"]), gpu_mode),
        // ---------------------------------------------------------------- bateria e tampa
        t("energia.limite_carga", n_("Stops charging the battery at 80% (it lasts more years) or charges it full"), Sistema, Escreve, true,
            || obj(json!({"ligado": {"type": "boolean"}}), &["ligado"]), charge_limit),
        t("energia.tampa", n_("What closing the laptop's lid does: suspend, power off or nothing"), Sistema, Escreve, true,
            || obj(json!({"acao": {"type": "string", "enum": ["suspender", "desligar", "nada"]}}), &["acao"]), lid_action),
        // ---------------------------------------------------------------- o computador, o relógio e o idioma
        t("sistema.nome", n_("The computer's name on the network (hostname)"), Sistema, Escreve, true,
            || obj(json!({"nome": {"type": "string", "description": "letras, números e hífens"}}), &["nome"]), host_name),
        t("relogio.fuso", n_("The time zone (for example America/Sao_Paulo)"), Sistema, Escreve, true,
            || obj(json!({"fuso": {"type": "string"}}), &["fuso"]), time_zone),
        t("relogio.automatico", n_("Sets the time by itself from the network (NTP), or not"), Sistema, Escreve, true,
            || obj(json!({"ligado": {"type": "boolean"}}), &["ligado"]), time_ntp),
        t("idioma.sistema", n_("The system's language and formats (a locale such as pt_BR.UTF-8)"), Sistema, Escreve, true,
            || obj(json!({"idioma": {"type": "string"}, "formatos": {"type": "string"}, "outros": {"type": "array", "items": {"type": "string"}, "maxItems": 6, "description": "os idiomas seguintes, quando falta uma tradução"}}), &["idioma"]), system_locale),
        t("usuarios.foto", n_("Shows the account's photo (~/.face) on the login screen, or takes it away; only the account that asks"), Sistema, Escreve, false,
            || obj(json!({"limpar": {"type": "boolean", "description": "tira a foto da tela de entrada"}}), &[]), user_photo),
        t("usuarios.convidado", n_("Lets a guest use the computer from the login screen (its files are wiped when it leaves), or not"), Sistema, Escreve, true,
            || obj(json!({"ligado": {"type": "boolean"}}), &["ligado"]), guest_toggle),
        t("sessao.entrar_sem_senha", n_("Signs a person in without a password when the computer starts (empty: always ask)"), Sistema, Escreve, true,
            || obj(json!({"usuario": {"type": "string"}}), &["usuario"]), autologin),
        // ---------------------------------------------------------------- discos
        t("discos.dispositivos", n_("Disks and partitions: size, file system, where they are mounted"), Observar, Leitura, false, none, disk_devices),
        t("discos.saude", n_("Health of a disk (SMART)"), Observar, Leitura, false, || obj(json!({"dispositivo": {"type": "string"}}), &["dispositivo"]), disk_health),
        t("discos.montar", n_("Mounts a partition (udisks)"), Sistema, Escreve, true, || obj(json!({"dispositivo": {"type": "string"}}), &["dispositivo"]), disk_mount),
        t("discos.desmontar", n_("Unmounts a partition (udisks)"), Sistema, Escreve, true, || obj(json!({"dispositivo": {"type": "string"}}), &["dispositivo"]), disk_unmount),
        // ---------------------------------------------------------------- usuários
        t("usuarios.listar", n_("People's accounts (uid ≥ 1000) and their groups"), Observar, Leitura, false, none, users_list),
        t("usuarios.criar", n_("Creates a person's account, with a home folder"), Sistema, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "nome_completo": {"type": "string"}, "administrador": {"type": "boolean"}}), &["nome"]), user_create),
        t("usuarios.remover", n_("Removes an account (the restore point brings the account back; the home folder only if it was not deleted)"), Sistema, Destrutiva, false,
            || obj(json!({"nome": {"type": "string"}, "apagar_casa": {"type": "boolean"}}), &["nome"]), user_remove),
        t("usuarios.grupo", n_("Adds an account to a group or removes it"), Sistema, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "grupo": {"type": "string"}, "acao": {"type": "string", "enum": ["adicionar", "remover"]}}), &["nome", "grupo", "acao"]), user_group),
        // ---------------------------------------------------------------- atualizações
        t("atualizacoes.listar", n_("Packages with an update available (current version → new)"), Observar, Leitura, false, none, upd_list),
        t("atualizacoes.verificar", n_("Refreshes the package index from the repositories"), Sistema, Escreve, false, none, upd_check),
        t("atualizacoes.aplicar", n_("Applies updates (all or only the given ones); the inverse goes back to the previous versions"), Sistema, Escreve, true,
            || obj(json!({"pacotes": {"type": "array", "items": {"type": "string"}, "maxItems": 200}}), &[]), upd_apply),
        t("pacotes.versoes", n_("Installs exact package versions (including older ones)"), Sistema, Escreve, true,
            || obj(json!({"versoes": {"type": "array", "items": {"type": "string", "description": "pacote=versão"}, "minItems": 1, "maxItems": 500}}), &["versoes"]), pkg_versions),
        // ---------------------------------------------------------------- tarefas
        t("tarefas.executar", n_("Runs a program as you, with memory, CPU, process and time limits; it reads your files (not the protected ones) and writes only in the work folders (Documentos, Downloads…), the project folders, its own TMPDIR, its own build caches (cargo, npm, Go, pip) and its own shared memory (/dev/shm, only when kalangod runs as root)"), Usuario, Destrutiva, false,
            || obj(json!({
                "programa": {"type": "string"}, "argumentos": {"type": "array", "items": {"type": "string"}}, "pasta": {"type": "string"},
                "memoria_mb": {"type": "integer", "minimum": 16}, "cpu_pct": {"type": "integer", "minimum": 1}, "processos": {"type": "integer", "minimum": 1}, "tempo_s": {"type": "integer", "minimum": 1}
            }), &["programa"]), task_run),
        t("tarefas.pedir_pasta", n_("Asks the person to let your commands write in a folder (a project); outside the work folders and the approved ones, commands only read"), Usuario, Leitura, false,
            || obj(json!({"pasta": {"type": "string", "description": "the folder, e.g. ~/Projetos/site"}}), &["pasta"]), folder_request),
        // ---------------------------------------------------------------- pontos de restauração
        t("pontos.listar", n_("System restore points"), Observar, Leitura, false, none, points_list),
        t("pontos.criar", n_("Creates a restore point now"), Sistema, Escreve, false, || obj(json!({"descricao": {"type": "string"}}), &[]), points_create),
        t("pontos.restaurar", n_("Brings the configuration (/etc) and the packages back to a restore point; creates a new point first, so it can be undone too"), Sistema, Destrutiva, true,
            || obj(json!({"id": {"type": "string"}, "so_plano": {"type": "boolean", "description": "só mostra o que mudaria"}}), &["id"]), points_restore),
    ]
}

// ------------------------------------------------------------------ validação
fn device(v: &str) -> Result<&str, String> {
    let ok = v.starts_with("/dev/") && !v.contains("..") && v.len() < 100 && v[5..].chars().all(|c| c.is_ascii_alphanumeric() || "/_-".contains(c));
    if ok { Ok(v) } else { Err(kalango_i18n::trf("invalid device: {v}", &[("v", format!("{v:?}"))])) }
}

fn user_name(v: &str) -> Result<&str, String> {
    let mut ch = v.chars();
    let ok = v.len() <= 32 && ch.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && ch.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c));
    if ok { Ok(v) } else { Err(kalango_i18n::trf("invalid account name: {v}", &[("v", format!("{v:?}"))])) }
}

fn port(v: &str) -> Result<&str, String> {
    let (p, proto) = v.split_once('/').unwrap_or((v, ""));
    let num = p.split(':').all(|x| x.parse::<u16>().is_ok_and(|n| n > 0));
    let svc = !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !p.starts_with('-');
    if (num || (svc && proto.is_empty())) && ["", "tcp", "udp"].contains(&proto) { Ok(v) } else { Err(kalango_i18n::trf("invalid port: {v}", &[("v", format!("{v:?}"))])) }
}

fn conn_name(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.len() > 128 || v.contains('\n') { Err(kalango_i18n::trf("invalid connection: {v}", &[("v", format!("{v:?}"))])) } else { Ok(v) }
}

/// Campos de `nmcli -t` (separados por ':' com '\:' escapado).
fn nm_fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for c in line.chars() {
        match (esc, c) {
            (true, _) => {
                out.last_mut().unwrap().push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, ':') => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

// ------------------------------------------------------------------ rede
fn net_list(_: &Ctx, _: &Value) -> Result<Out, String> {
    let raw = exec("nmcli", &["-t", "-f", "NAME,UUID,TYPE,DEVICE,STATE", "connection", "show"])?;
    let list: Vec<Value> = raw.lines().map(nm_fields).filter(|f| f.len() >= 5)
        .map(|f| json!({"nome": f[0], "uuid": f[1], "tipo": f[2], "dispositivo": f[3], "estado": f[4]})).collect();
    let ativas = list.iter().filter(|c| c["estado"] == "activated").count();
    Ok(Out::read(json!({"conexoes": list}), trf("{n} connections, {active} active", &[("n", list.len().to_string()), ("active", ativas.to_string())])))
}

fn wifi_list(_: &Ctx, _: &Value) -> Result<Out, String> {
    let raw = exec("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"])?;
    let list: Vec<Value> = raw.lines().map(nm_fields).filter(|f| f.len() >= 4)
        .map(|f| json!({"em_uso": f[0] == "*", "ssid": f[1], "sinal": f[2].parse::<u32>().unwrap_or(0), "seguranca": f[3]})).collect();
    Ok(Out::read(json!({"redes": list}), trf("{n} Wi-Fi networks", &[("n", list.len().to_string())])))
}

fn net_conn(c: &Ctx, a: &Value) -> Result<Out, String> {
    let name = conn_name(s(a, "nome")?)?;
    let (verb, inv) = match s(a, "acao")? {
        "ativar" => ("up", "desativar"),
        "desativar" => ("down", "ativar"),
        o => return Err(kalango_i18n::trf("unknown action: {o}", &[("o", o.to_string())])),
    };
    let r = act(c, "nmcli", &["connection", verb, "id", name])?;
    Ok(Out { value: json!({"conexao": name, "acao": verb, "saida": r.trim()}), summary: trf("connection {name}: {action}", &[("name", name.to_string()), ("action", verb.to_string())]),
        inverse: Some(json!({"ferramenta": "rede.conexao", "argumentos": {"nome": name, "acao": inv}})) })
}

fn fw_status(_: &Ctx, _: &Value) -> Result<Out, String> {
    let raw = exec("ufw", &["status", "numbered"])?;
    let ativo = raw.lines().next().is_some_and(|l| l.contains("active") && !l.contains("inactive"));
    let regras: Vec<&str> = raw.lines().filter(|l| l.trim_start().starts_with('[')).map(str::trim).collect();
    Ok(Out::read(json!({"ligado": ativo, "regras": regras}), trf("firewall {state}, {n} rules", &[("state", if ativo { tr("on") } else { tr("off") }.to_string()), ("n", regras.len().to_string())])))
}

fn fw_rule(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = port(s(a, "porta")?)?;
    let verb = match s(a, "acao")? {
        "permitir" => "allow",
        "bloquear" => "deny",
        o => return Err(kalango_i18n::trf("unknown action: {o}", &[("o", o.to_string())])),
    };
    let remover = a.get("remover").and_then(Value::as_bool).unwrap_or(false);
    let r = if remover { act(c, "ufw", &["delete", verb, p])? } else { act(c, "ufw", &[verb, p])? };
    Ok(Out { value: json!({"porta": p, "acao": verb, "removida": remover, "saida": r.trim()}),
        summary: if remover { trf("removed firewall rule: {action} {port}", &[("action", verb.to_string()), ("port", p.to_string())]) } else { trf("firewall {action} {port}", &[("action", verb.to_string()), ("port", p.to_string())]) },
        inverse: Some(json!({"ferramenta": "firewall.regra", "argumentos": {"acao": s(a, "acao")?, "porta": p, "remover": !remover}})) })
}

/// prime-select's names for the modes, and back.
fn prime_name(modo: &str) -> Option<&'static str> {
    match modo {
        "hibrido" => Some("on-demand"),
        "dedicado" => Some("nvidia"),
        "integrado" => Some("intel"),
        _ => None,
    }
}

fn mode_from_prime(p: &str) -> &'static str {
    match p.trim() {
        "nvidia" => "dedicado",
        "intel" => "integrado",
        _ => "hibrido",
    }
}

fn gpu_mode(c: &Ctx, a: &Value) -> Result<Out, String> {
    let modo = s(a, "modo")?;
    let p = prime_name(modo).ok_or(kalango_i18n::tr("unknown graphics mode"))?;
    let antes = exec("prime-select", &["query"]).map(|q| mode_from_prime(&q)).unwrap_or("hibrido");
    let r = act(c, "prime-select", &[p])?;
    Ok(Out { value: json!({"modo": modo, "antes": antes, "saida": r.trim(), "reiniciar": true}), summary: trf("graphics card: {mode} (after restarting)", &[("mode", modo.to_string())]),
        inverse: Some(json!({"ferramenta": "grafico.modo", "argumentos": {"modo": antes}})) })
}

/// The batteries' charge thresholds (sysfs; KALANGO_POWER_SUPPLY points elsewhere in the session test).
fn thresholds() -> Vec<std::path::PathBuf> {
    let base = std::env::var("KALANGO_POWER_SUPPLY").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/sys/class/power_supply".into());
    let mut v: Vec<_> = std::fs::read_dir(base).map(|d| d.flatten().map(|e| e.path().join("charge_control_end_threshold")).filter(|p| p.exists()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn charge_limit(c: &Ctx, a: &Value) -> Result<Out, String> {
    let on = a.get("ligado").and_then(Value::as_bool).ok_or(kalango_i18n::tr("argument 'ligado' is required"))?;
    let alvos = thresholds();
    if alvos.is_empty() {
        return Err(kalango_i18n::tr("this battery has no charge limit").to_string());
    }
    let antes = std::fs::read_to_string(&alvos[0]).ok().and_then(|t| t.trim().parse::<u32>().ok()).unwrap_or(100);
    let valor = if on { "80" } else { "100" };
    let saida = if c.cfg.dry_run {
        format!("[simulado] {valor} > {}", alvos[0].display())
    } else {
        for p in &alvos {
            std::fs::write(p, valor).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        valor.to_string()
    };
    Ok(Out { value: json!({"ligado": on, "antes": antes, "saida": saida}), summary: if on { tr("battery charge limited to 80%") } else { tr("battery charges full") }.to_string(),
        inverse: Some(json!({"ferramenta": "energia.limite_carga", "argumentos": {"ligado": antes < 100}})) })
}

const LID_CONF: &str = "/etc/systemd/logind.conf.d/50-kalango-tampa.conf";

/// logind's name for each action, and back.
fn logind_lid(acao: &str) -> Option<&'static str> {
    match acao {
        "suspender" => Some("suspend"),
        "desligar" => Some("poweroff"),
        "nada" => Some("ignore"),
        _ => None,
    }
}

pub fn lid_now() -> &'static str {
    let atual = std::fs::read_to_string(LID_CONF).ok().and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("HandleLidSwitch=").map(str::to_string)));
    match atual.as_deref() {
        Some("poweroff") => "desligar",
        Some("ignore") => "nada",
        _ => "suspender",
    }
}

fn lid_action(c: &Ctx, a: &Value) -> Result<Out, String> {
    let acao = s(a, "acao")?;
    let nome = logind_lid(acao).ok_or(kalango_i18n::tr("unknown lid action"))?;
    let antes = lid_now();
    let texto = format!("# KalangoOS: Configurações › Energia\n[Login]\nHandleLidSwitch={nome}\n");
    let saida = if c.cfg.dry_run {
        format!("[simulado] HandleLidSwitch={nome} > {LID_CONF}")
    } else {
        std::fs::create_dir_all("/etc/systemd/logind.conf.d").map_err(|e| e.to_string())?;
        std::fs::write(LID_CONF, texto).map_err(|e| format!("{LID_CONF}: {e}"))?;
        // logind reads its configuration again on SIGHUP
        exec("systemctl", &["kill", "-s", "HUP", "systemd-logind"])?;
        nome.to_string()
    };
    Ok(Out { value: json!({"acao": acao, "antes": antes, "saida": saida}), summary: trf("closing the lid: {action}", &[("action", acao.to_string())]),
        inverse: Some(json!({"ferramenta": "energia.tampa", "argumentos": {"acao": antes}})) })
}

// ------------------------------------------------------------------ o convidado e a entrada automática
const GREETD_CONF: &str = "/etc/greetd/config.toml";

fn guest_toggle(c: &Ctx, a: &Value) -> Result<Out, String> {
    let on = a.get("ligado").and_then(Value::as_bool).ok_or(tr("argument 'ligado' is required"))?;
    let shell_now = std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().find(|l| l.starts_with("convidado:")).and_then(|l| l.rsplit(':').next().map(str::to_string));
    let Some(antes) = shell_now else { return Err(tr("this system has no guest account").to_string()) };
    let novo = if on { "/bin/bash" } else { "/usr/sbin/nologin" };
    let saida = act(c, "usermod", &["-s", novo, "convidado"])?;
    Ok(Out { value: json!({"ligado": on, "saida": saida.trim()}), summary: if on { tr("the guest can sign in").to_string() } else { tr("the guest is off").to_string() },
        inverse: Some(json!({"ferramenta": "usuarios.convidado", "argumentos": {"ligado": !antes.ends_with("nologin") && !antes.ends_with("false")}})) })
}

const ICONS: &str = "/var/lib/AccountsService/icons";

/// The bytes of a photo kalangod (root) may copy out of a person's home: a regular file of theirs (never a link: a
/// ~/.face pointing at /etc/shadow would be copied where everyone reads it), at most 5 MB, a PNG or a JPEG.
fn photo_bytes(path: &std::path::Path, owner: u32) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new().read(true).custom_flags(nix::libc::O_NOFOLLOW).open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.file_type().is_file() || m.uid() != owner {
        return Err(tr("the photo is not a file of this account").to_string());
    }
    if m.len() > 5 << 20 {
        return Err(tr("the photo is too big (at most 5 MB)").to_string());
    }
    let mut b = Vec::new();
    f.take(5 << 20).read_to_end(&mut b).map_err(|e| e.to_string())?;
    let png = b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let jpeg = b.starts_with(&[0xFF, 0xD8, 0xFF]);
    if !png && !jpeg {
        return Err(tr("the photo is not an image (PNG or JPEG)").to_string());
    }
    Ok(b)
}

fn user_photo(c: &Ctx, a: &Value) -> Result<Out, String> {
    let user = c.peer.user.clone();
    if user.is_empty() || !user.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return Err(tr("unknown account").to_string());
    }
    let dest = std::path::Path::new(ICONS).join(&user);
    let clear = a.get("limpar").and_then(Value::as_bool).unwrap_or(false);
    let had = dest.is_file();
    let saida = if clear {
        if c.cfg.dry_run { format!("[simulado] remover {}", dest.display()) } else {
            if had {
                std::fs::remove_file(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
            String::new()
        }
    } else {
        let b = photo_bytes(&c.peer.home.join(".face"), c.peer.uid)?;
        if c.cfg.dry_run { format!("[simulado] {} ({} bytes)", dest.display(), b.len()) } else {
            std::fs::create_dir_all(ICONS).map_err(|e| format!("{ICONS}: {e}"))?;
            // (written beside and then renamed: the login screen never reads half a photo)
            let tmp = dest.with_extension("novo");
            std::fs::write(&tmp, &b).map_err(|e| format!("{}: {e}", tmp.display()))?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            String::new()
        }
    };
    Ok(Out { value: json!({"usuario": user, "limpar": clear, "saida": saida}),
        summary: if clear { tr("the login screen shows the name's initial").to_string() } else { tr("the login screen shows the account's photo").to_string() },
        // (a photo put where there was none comes off again; taking one off has nothing to give back)
        inverse: (!clear && !had).then(|| json!({"ferramenta": "usuarios.foto", "argumentos": {"limpar": true}})) })
}

#[cfg(test)]
mod photo_tests {
    use super::*;

    #[test]
    fn only_an_image_file_of_the_account_is_copied() {
        let d = std::env::temp_dir().join(format!("kalango-foto-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let me = nix::unistd::getuid().as_raw();
        let png = d.join("png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]).unwrap();
        assert!(photo_bytes(&png, me).is_ok());
        let jpg = d.join("jpg");
        std::fs::write(&jpg, [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        assert!(photo_bytes(&jpg, me).is_ok());
        // not an image
        let txt = d.join("txt");
        std::fs::write(&txt, b"root:x:0:0").unwrap();
        assert!(photo_bytes(&txt, me).is_err());
        // a link (to anything) is refused, even to an image
        let link = d.join("link");
        std::os::unix::fs::symlink(&png, &link).unwrap();
        assert!(photo_bytes(&link, me).is_err());
        // another owner's file is refused
        assert!(photo_bytes(&png, me + 1).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}

/// The person greetd signs in by itself (its initial_session), if any.
pub fn autologin_now() -> String {
    let t = std::fs::read_to_string(GREETD_CONF).unwrap_or_default();
    let mut dentro = false;
    for l in t.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            dentro = l == "[initial_session]";
        } else if dentro {
            if let Some(u) = l.strip_prefix("user").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"')) {
                return u.to_string();
            }
        }
    }
    String::new()
}

fn autologin(c: &Ctx, a: &Value) -> Result<Out, String> {
    let usuario = s(a, "usuario")?.trim().to_string();
    if !usuario.is_empty() && (!usuario.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || usuario == "convidado") {
        return Err(tr("unknown account").to_string());
    }
    let antes = autologin_now();
    let atual = std::fs::read_to_string(GREETD_CONF).unwrap_or_default();
    // the file without its initial_session, then the new one
    let mut out = String::new();
    let mut pular = false;
    for l in atual.lines() {
        if l.trim().starts_with('[') {
            pular = l.trim() == "[initial_session]";
        }
        if !pular {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !usuario.is_empty() {
        out.push_str(&format!("\n[initial_session]\ncommand = \"/usr/libexec/kalango/kalango-sessao\"\nuser = \"{usuario}\"\n"));
    }
    let saida = if c.cfg.dry_run {
        format!("[simulado] {GREETD_CONF}: entrar sem senha = {usuario}")
    } else {
        std::fs::write(GREETD_CONF, out).map_err(|e| format!("{GREETD_CONF}: {e}"))?;
        String::new()
    };
    Ok(Out { value: json!({"usuario": usuario, "antes": antes, "saida": saida}),
        summary: if usuario.is_empty() { tr("the computer asks for a password when it starts").to_string() } else { trf("{name} signs in without a password when the computer starts", &[("name", usuario.clone())]) },
        inverse: Some(json!({"ferramenta": "sessao.entrar_sem_senha", "argumentos": {"usuario": antes}})) })
}

// ------------------------------------------------------------------ o computador, o relógio e o idioma
fn valid_hostname(n: &str) -> bool {
    !n.is_empty() && n.len() <= 63 && !n.starts_with('-') && !n.ends_with('-') && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn host_name(c: &Ctx, a: &Value) -> Result<Out, String> {
    let nome = s(a, "nome")?.trim();
    if !valid_hostname(nome) {
        return Err(tr("a computer's name has letters, numbers and hyphens (up to 63)").to_string());
    }
    let antes = std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    let saida = act(c, "hostnamectl", &["set-hostname", nome])?;
    Ok(Out { value: json!({"nome": nome, "antes": antes, "saida": saida.trim()}), summary: trf("the computer is now called {name}", &[("name", nome.to_string())]),
        inverse: Some(json!({"ferramenta": "sistema.nome", "argumentos": {"nome": antes}})) })
}

fn current_zone() -> String {
    std::fs::read_link("/etc/localtime").ok().and_then(|p| p.to_string_lossy().split("zoneinfo/").nth(1).map(str::to_string)).unwrap_or_else(|| "UTC".into())
}

fn time_zone(c: &Ctx, a: &Value) -> Result<Out, String> {
    let fuso = s(a, "fuso")?.trim();
    // only a zone the system has (no path tricks)
    if fuso.contains("..") || !std::path::Path::new("/usr/share/zoneinfo").join(fuso).is_file() {
        return Err(trf("unknown time zone: {zone}", &[("zone", fuso.to_string())]));
    }
    let antes = current_zone();
    let saida = act(c, "timedatectl", &["set-timezone", fuso])?;
    Ok(Out { value: json!({"fuso": fuso, "antes": antes, "saida": saida.trim()}), summary: trf("time zone: {zone}", &[("zone", fuso.to_string())]),
        inverse: Some(json!({"ferramenta": "relogio.fuso", "argumentos": {"fuso": antes}})) })
}

fn time_ntp(c: &Ctx, a: &Value) -> Result<Out, String> {
    let on = a.get("ligado").and_then(Value::as_bool).ok_or(tr("argument 'ligado' is required"))?;
    let antes = exec("timedatectl", &["show", "-p", "NTP", "--value"]).map(|v| v.trim() == "yes").unwrap_or(!on);
    let saida = act(c, "timedatectl", &["set-ntp", if on { "true" } else { "false" }])?;
    Ok(Out { value: json!({"ligado": on, "saida": saida.trim()}), summary: if on { tr("the time is set from the network").to_string() } else { tr("the time is set by hand").to_string() },
        inverse: Some(json!({"ferramenta": "relogio.automatico", "argumentos": {"ligado": antes}})) })
}

fn valid_locale(l: &str) -> bool {
    !l.is_empty() && l.len() <= 40 && l.chars().all(|c| c.is_ascii_alphanumeric() || "_.-@".contains(c))
}

fn system_locale(c: &Ctx, a: &Value) -> Result<Out, String> {
    let idioma = s(a, "idioma")?.trim().to_string();
    let formatos = a.get("formatos").and_then(Value::as_str).map(str::trim).filter(|f| !f.is_empty()).unwrap_or(&idioma).to_string();
    if !valid_locale(&idioma) || !valid_locale(&formatos) {
        return Err(tr("unknown language").to_string());
    }
    let atual = exec("localectl", &["status"]).unwrap_or_default();
    let antes = atual.lines().find_map(|l| l.trim().strip_prefix("System Locale: LANG=")).map(str::to_string).unwrap_or_else(|| "C.UTF-8".into());
    let lang = format!("LANG={idioma}");
    let mut args = vec!["set-locale", lang.as_str()];
    let fmt: Vec<String> = ["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER"].iter().map(|k| format!("{k}={formatos}")).collect();
    args.extend(fmt.iter().map(String::as_str));
    // the languages that come next when a text has no translation (LANGUAGE=pt_BR:en_US)
    let outros = strings(a, "outros");
    if outros.iter().any(|o| !valid_locale(o)) {
        return Err(tr("unknown language").to_string());
    }
    let language = format!("LANGUAGE={}", std::iter::once(idioma.split('.').next().unwrap_or(&idioma).to_string())
        .chain(outros.iter().map(|o| o.split('.').next().unwrap_or(o).to_string())).collect::<Vec<_>>().join(":"));
    if !outros.is_empty() {
        args.push(language.as_str());
    }
    let saida = act(c, "localectl", &args)?;
    Ok(Out { value: json!({"idioma": idioma, "formatos": formatos, "antes": antes, "saida": saida.trim()}),
        summary: trf("system language: {lang} (after signing in again)", &[("lang", idioma.clone())]),
        inverse: Some(json!({"ferramenta": "idioma.sistema", "argumentos": {"idioma": antes}})) })
}

fn fw_toggle(c: &Ctx, a: &Value) -> Result<Out, String> {
    let on = a.get("ligado").and_then(Value::as_bool).ok_or(kalango_i18n::tr("argument 'ligado' is required"))?;
    let r = if on { act(c, "ufw", &["--force", "enable"])? } else { act(c, "ufw", &["disable"])? };
    Ok(Out { value: json!({"ligado": on, "saida": r.trim()}), summary: trf("firewall {state}", &[("state", if on { tr("on") } else { tr("off") }.to_string())]),
        inverse: Some(json!({"ferramenta": "firewall.ligar", "argumentos": {"ligado": !on}})) })
}

// ------------------------------------------------------------------ discos
fn disk_devices(_: &Ctx, _: &Value) -> Result<Out, String> {
    let raw = exec("lsblk", &["-J", "-b", "-o", "NAME,PATH,SIZE,TYPE,FSTYPE,MOUNTPOINTS,LABEL,MODEL,RM"])?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let n = v["blockdevices"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Out::read(v, trf("{n} devices", &[("n", n.to_string())])))
}

fn disk_health(_: &Ctx, a: &Value) -> Result<Out, String> {
    let d = device(s(a, "dispositivo")?)?;
    // o smartctl usa o código de saída como mapa de bits: o JSON vale mesmo com código diferente de zero
    let o = std::process::Command::new("smartctl").args(["-j", "-H", "-i", "-A", d]).env("LC_ALL", "C").output().map_err(|e| format!("smartctl: {e}"))?;
    let v: Value = serde_json::from_slice(&o.stdout).map_err(|_| format!("smartctl: {}", String::from_utf8_lossy(&o.stderr).trim()))?;
    if let Some(m) = v.pointer("/smartctl/messages/0/string").and_then(Value::as_str) {
        if v.get("smart_status").is_none() {
            return Err(format!("smartctl: {m}"));
        }
    }
    let passed = v.pointer("/smart_status/passed").and_then(Value::as_bool);
    let out = json!({
        "dispositivo": d, "modelo": v["model_name"], "serie": v["serial_number"], "saudavel": passed,
        "temperatura_c": v.pointer("/temperature/current"), "horas_ligado": v.pointer("/power_on_time/hours"),
        "nvme": v.get("nvme_smart_health_information_log"),
    });
    Ok(Out::read(out, format!("{d}: {}", match passed { Some(true) => tr("healthy"), Some(false) => tr("FAILING"), None => tr("no SMART") })))
}

fn disk_mount(c: &Ctx, a: &Value) -> Result<Out, String> {
    let d = device(s(a, "dispositivo")?)?;
    let r = act(c, "udisksctl", &["mount", "-b", d, "--no-user-interaction"])?;
    Ok(Out { value: json!({"dispositivo": d, "saida": r.trim()}), summary: trf("mounted {device}", &[("device", d.to_string())]),
        inverse: Some(json!({"ferramenta": "discos.desmontar", "argumentos": {"dispositivo": d}})) })
}

fn disk_unmount(c: &Ctx, a: &Value) -> Result<Out, String> {
    let d = device(s(a, "dispositivo")?)?;
    let r = act(c, "udisksctl", &["unmount", "-b", d, "--no-user-interaction"])?;
    Ok(Out { value: json!({"dispositivo": d, "saida": r.trim()}), summary: trf("unmounted {device}", &[("device", d.to_string())]),
        inverse: Some(json!({"ferramenta": "discos.montar", "argumentos": {"dispositivo": d}})) })
}

// ------------------------------------------------------------------ usuários
fn passwd() -> Vec<(String, u32, u32, String, String)> {
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        Some((f.first()?.to_string(), f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?, f.get(4)?.split(',').next().unwrap_or("").to_string(), f.get(5)?.to_string()))
    }).collect()
}

fn groups_of(user: &str, gid: u32) -> Vec<String> {
    std::fs::read_to_string("/etc/group").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        let g: u32 = f.get(2)?.parse().ok()?;
        (g == gid || f.get(3)?.split(',').any(|m| m == user)).then(|| f[0].to_string())
    }).collect()
}

fn users_list(_: &Ctx, _: &Value) -> Result<Out, String> {
    let list: Vec<Value> = passwd().into_iter().filter(|u| (1000..60000).contains(&u.1))
        .map(|(n, uid, gid, full, home)| json!({"nome": n, "uid": uid, "nome_completo": full, "casa": home, "grupos": groups_of(&n, gid)})).collect();
    Ok(Out::read(json!({"usuarios": list}), trf("{n} accounts", &[("n", list.len().to_string())])))
}

fn user_create(c: &Ctx, a: &Value) -> Result<Out, String> {
    let n = user_name(s(a, "nome")?)?;
    if passwd().iter().any(|u| u.0 == n) {
        return Err(kalango_i18n::trf("the account {n} already exists", &[("n", n.to_string())]));
    }
    let full = a.get("nome_completo").and_then(Value::as_str).unwrap_or("");
    if full.contains([':', '\n']) {
        return Err(kalango_i18n::tr("invalid full name").into());
    }
    let mut args = vec!["-m", "-s", "/bin/bash", "-c", full];
    if a.get("administrador").and_then(Value::as_bool).unwrap_or(false) {
        args.extend(["-G", "sudo"]);
    }
    args.push(n);
    let r = act(c, "useradd", &args)?;
    Ok(Out { value: json!({"nome": n, "saida": r.trim()}), summary: trf("created the account {name}", &[("name", n.to_string())]),
        inverse: Some(json!({"ferramenta": "usuarios.remover", "argumentos": {"nome": n, "apagar_casa": true}})) })
}

fn user_remove(c: &Ctx, a: &Value) -> Result<Out, String> {
    let n = user_name(s(a, "nome")?)?;
    let u = passwd().into_iter().find(|u| u.0 == n).ok_or(kalango_i18n::trf("the account {n} does not exist", &[("n", n.to_string())]))?;
    if u.1 < 1000 {
        return Err(kalango_i18n::trf("{n} is a system account (uid {uid}): it is not removed this way", &[("n", n.to_string()), ("uid", u.1.to_string())]));
    }
    if u.1 == c.peer.uid {
        return Err(kalango_i18n::tr("you cannot remove your own account").into());
    }
    let home = a.get("apagar_casa").and_then(Value::as_bool).unwrap_or(false);
    let r = if home { act(c, "userdel", &["-r", n])? } else { act(c, "userdel", &[n])? };
    Ok(Out { value: json!({"nome": n, "casa_apagada": home, "saida": r.trim()}), summary: if home { trf("removed the account {name} and its home folder", &[("name", n.to_string())]) } else { trf("removed the account {name}", &[("name", n.to_string())]) }, inverse: None })
}

fn user_group(c: &Ctx, a: &Value) -> Result<Out, String> {
    let n = user_name(s(a, "nome")?)?;
    let g = user_name(s(a, "grupo")?)?;
    let (flag, inv) = match s(a, "acao")? {
        "adicionar" => ("-a", "remover"),
        "remover" => ("-d", "adicionar"),
        o => return Err(kalango_i18n::trf("unknown action: {o}", &[("o", o.to_string())])),
    };
    let r = act(c, "gpasswd", &[flag, n, g])?;
    Ok(Out { value: json!({"nome": n, "grupo": g, "saida": r.trim()}), summary: if flag == "-a" { trf("{name} joined {group}", &[("name", n.to_string()), ("group", g.to_string())]) } else { trf("{name} left {group}", &[("name", n.to_string()), ("group", g.to_string())]) },
        inverse: Some(json!({"ferramenta": "usuarios.grupo", "argumentos": {"nome": n, "grupo": g, "acao": inv}})) })
}

// ------------------------------------------------------------------ atualizações
fn upgradable() -> Result<Vec<Value>, String> {
    let raw = exec("apt", &["list", "--upgradable"])?;
    Ok(raw.lines().filter(|l| l.contains("[upgradable from:")).filter_map(|l| {
        let (name, rest) = l.split_once('/')?;
        let f: Vec<&str> = rest.split_whitespace().collect();
        Some(json!({"pacote": name, "nova": f.get(1)?, "atual": l.rsplit_once("from: ")?.1.trim_end_matches(']')}))
    }).collect())
}

fn upd_list(_: &Ctx, _: &Value) -> Result<Out, String> {
    let v = upgradable()?;
    let n = v.len();
    Ok(Out::read(json!({"atualizacoes": v}), trf("{n} updates available", &[("n", n.to_string())])))
}

fn upd_check(c: &Ctx, _: &Value) -> Result<Out, String> {
    let r = act(c, "apt-get", &["update", "-q"])?;
    let n = if c.cfg.dry_run { 0 } else { upgradable().map(|v| v.len()).unwrap_or(0) };
    Ok(Out { value: json!({"saida": r.lines().last().unwrap_or(""), "disponiveis": n}), summary: trf("index refreshed: {n} updates", &[("n", n.to_string())]), inverse: None })
}

fn versions() -> std::collections::BTreeMap<String, String> {
    exec("dpkg-query", &["-W", "-f", "${db:Status-Abbrev}\t${Package}:${Architecture}\t${Version}\n"]).unwrap_or_default().lines()
        .filter(|l| l.starts_with("ii")).filter_map(|l| { let f: Vec<&str> = l.split('\t').collect(); Some((f.get(1)?.to_string(), f.get(2)?.to_string())) }).collect()
}

fn apt_run(c: &Ctx, args: &[&str]) -> Result<String, String> {
    if c.cfg.dry_run {
        return Ok(format!("[simulado] apt-get {}", args.join(" ")));
    }
    let o = std::process::Command::new("apt-get").args(args).env("DEBIAN_FRONTEND", "noninteractive").env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("apt-get {}: {}", args[0], String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Compara versões antes e depois e monta o inverso (voltar às versões anteriores).
fn version_change(before: &std::collections::BTreeMap<String, String>) -> (Vec<Value>, Vec<String>) {
    let after = versions();
    let mut changed = Vec::new();
    let mut back = Vec::new();
    for (k, old) in before {
        if let Some(new) = after.get(k).filter(|n| *n != old) {
            changed.push(json!({"pacote": k, "antes": old, "depois": new}));
            back.push(format!("{k}={old}"));
        }
    }
    (changed, back)
}

fn upd_apply(c: &Ctx, a: &Value) -> Result<Out, String> {
    let pkgs: Vec<String> = strings(a, "pacotes").iter().map(|p| safe_name(p).map(str::to_string)).collect::<Result<_, _>>()?;
    let before = versions();
    let mut args = vec!["-y", "-q"];
    if pkgs.is_empty() {
        args.insert(0, "upgrade");
    } else {
        args.insert(0, "install");
        args.push("--only-upgrade");
        args.extend(pkgs.iter().map(String::as_str));
    }
    let r = apt_run(c, &args)?;
    let (changed, back) = version_change(&before);
    let n = changed.len();
    Ok(Out { value: json!({"atualizados": changed, "saida": r.lines().filter(|l| l.contains("upgraded")).collect::<Vec<_>>()}), summary: trf("{n} packages updated", &[("n", n.to_string())]),
        inverse: (!back.is_empty()).then(|| json!({"ferramenta": "pacotes.versoes", "argumentos": {"versoes": back}})) })
}

fn pkg_versions(c: &Ctx, a: &Value) -> Result<Out, String> {
    let list = strings(a, "versoes");
    for v in &list {
        let (p, ver) = v.split_once('=').ok_or(kalango_i18n::trf("use package=version: {v}", &[("v", format!("{v:?}"))]))?;
        safe_name(p)?;
        if ver.is_empty() || ver.starts_with('-') || !ver.chars().all(|ch| ch.is_ascii_alphanumeric() || ".+-:~_".contains(ch)) {
            return Err(kalango_i18n::trf("invalid version: {v}", &[("v", format!("{v:?}"))]));
        }
    }
    let before = versions();
    let mut args = vec!["install", "-y", "-q", "--allow-downgrades", "--no-install-recommends"];
    args.extend(list.iter().map(String::as_str));
    let r = apt_run(c, &args)?;
    let (changed, back) = version_change(&before);
    Ok(Out { value: json!({"mudancas": changed, "saida": r.lines().filter(|l| l.contains("upgraded") || l.contains("downgraded")).collect::<Vec<_>>()}),
        summary: trf("{n} packages at exact versions", &[("n", changed.len().to_string())]),
        inverse: (!back.is_empty()).then(|| json!({"ferramenta": "pacotes.versoes", "argumentos": {"versoes": back}})) })
}

// ------------------------------------------------------------------ tarefas
pub const SISTEMA_LEITURA: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/opt", "/proc", "/sys", "/dev", "/run", "/var/lib", "/var/cache", "/snap", "/nix"];
pub const SISTEMA_ESCRITA: &[&str] = &["/tmp", "/var/tmp", "/dev/null", "/dev/zero", "/dev/full", "/dev/tty", "/dev/pts", "/dev/shm"];

/// An agent asks for a folder its commands may write in: the person answers on the approval screen (`allowlist.rs`).
fn folder_request(c: &Ctx, a: &Value) -> Result<Out, String> {
    let st = c.state.ok_or(tr("not available here"))?;
    let raw = s(a, "pasta")?;
    let (p, always) = crate::allowlist::ask_folder(st, c.peer, &format!("{} ({})", c.rule.name, c.peer.exe), "tarefas.pedir_pasta", raw, false)?;
    let shown = crate::startup::tilde(&p, &c.peer.home);
    Ok(Out::read(json!({"pasta": shown, "sempre": always}), if always { trf("commands write in {path} from now on", &[("path", shown)]) } else { trf("commands write in {path} during this task", &[("path", shown)]) }))
}

fn task_run(c: &Ctx, a: &Value) -> Result<Out, String> {
    let prog = s(a, "programa")?;
    if prog.is_empty() || prog.starts_with('-') {
        return Err(kalango_i18n::tr("invalid program").into());
    }
    let args = strings(a, "argumentos");
    let home = &c.peer.home;
    let pasta = match a.get("pasta").and_then(Value::as_str) {
        Some(p) => Some(crate::sandbox::check_path(p, c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths)?),
        None => Some(home.clone()),
    };
    // the allowlist (allowlist.rs): the command reads the caller's allowed folders, cut around the protected ones
    // (nothing), and writes only in the work folders, the project folders and the folders approved for this task —
    // narrowed to the allowed ones, cut around the files that run at login and the PATH folders (read only; Landlock
    // grants, it cannot except: startup.rs, `carve`) — and in its own temporary folder (limits.rs)
    let allowed: Vec<std::path::PathBuf> = c.rule.allowed_paths.iter().map(|p| crate::policy::expand(p, home)).collect();
    let protected = crate::allowlist::protected(c.cfg, home);
    let startup: Vec<std::path::PathBuf> = crate::startup::paths(c.cfg, c.peer);
    let auto = crate::startup::mode(c.cfg, c.peer) == crate::startup::Mode::Allow;
    // (Landlock goes by path: a hard link of a startup file in a work folder would be the same file, writable under the
    // other name; while the person wants to be asked, commands wait until it is gone)
    if !auto {
        if let Some(f) = crate::startup::hardlinked(c.peer, c.cfg) {
            return Err(trf("{file} runs when you log in and has another name (a hard link) elsewhere: commands are refused until that link is removed (the file tools still work, asking first)",
                &[("file", crate::startup::tilde(&f, home))]));
        }
    }
    // (every write rule is reached from the real home, without links: allowlist.rs, `pin`)
    let rh = crate::allowlist::real_home(home);
    let candidates = match c.state {
        Some(st) => crate::allowlist::candidates(st, c.peer),
        None => crate::allowlist::work_folders(&rh, &protected),
    };
    let allowed_real: Vec<std::path::PathBuf> = allowed.iter().map(|p| crate::sandbox::resolve_existing(p)).collect();
    let mut write_roots = crate::allowlist::writable(&candidates, &allowed_real);
    let mut read_only: Vec<std::path::PathBuf> = if auto { vec![] } else { startup.clone() };
    // installing for the person writes outside the work folders: asked first, for this command only
    // (the grant, before an install widens it: the agents' cache is this caller's with exactly these folders)
    let scope = crate::allowlist::cache_scope(&c.rule.name, &write_roots);
    let grant_roots = write_roots.clone();
    let mut unset: &[&str] = &[];
    if let Some(i) = crate::allowlist::installs_for_user(prog, &args, &rh) {
        let st = c.state.ok_or(tr("not available here"))?;
        let program_folder = |s: &std::path::Path, w: &std::path::Path| s == w || (crate::policy::is_within(s, w) && s.ends_with("bin"));
        let on_path = i.writes.iter().filter(|w| startup.iter().any(|s| program_folder(s, w))).map(|w| crate::startup::tilde(w, &rh)).collect::<Vec<_>>();
        let writes: Vec<String> = i.writes.iter().map(|w| crate::startup::tilde(w, &rh)).collect();
        let p = crate::approval::Pending {
            id: 0, tool: "tarefas.executar".into(), level: "instalar".into(), args: a.clone(), caller: format!("{} ({})", c.rule.name, c.peer.exe), uid: c.peer.uid, pid: c.peer.pid,
            reason: trf("{what}: writes in {folders}", &[("what", i.what.to_string()), ("folders", writes.join(", "))]),
            descricao: tr("Runs a command that installs for your user, outside the work folders").into(), efeito: "escreve".into(), reversivel: false,
            criado: 0, prazo_s: 0, exe: c.peer.exe.clone(), decision: None, startup: None, no_always: true,
            cartao: Some(json!({"comando": std::iter::once(prog.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "), "escreve": writes, "no_path": on_path, "o_que": i.what})),
        };
        match st.approvals.request(p, std::time::Duration::from_secs(c.cfg.approval_timeout_s)) {
            crate::approval::Outcome::Approved => {}
            crate::approval::Outcome::Denied => return Err(trf("the person did not let {what} write outside the work folders (inside a project folder no approval is needed: python -m venv .venv, npm install without -g, cargo build — with the agents' own caches)", &[("what", i.what.to_string())])),
            crate::approval::Outcome::Timeout => return Err(tr("no approval within the time limit").into()),
        }
        read_only.retain(|s| !i.writes.iter().any(|w| program_folder(s, w)));
        read_only.extend(i.keep);
        // (a folder the card names that is not there yet, ~/go/bin: made now, as the person)
        write_roots.extend(i.writes.iter().filter_map(|w| crate::allowlist::ensure_dir(&rh, w, c.peer.uid, c.peer.gid)));
        unset = i.unset;
    }
    // the agents' own caches (cargo, npm, Go, pip…): never the person's, which feed the person's own builds
    let cache = crate::allowlist::agents_cache(home, &scope, c.peer.uid, c.peer.gid);
    if let (Some(st), Some(_)) = (c.state, &cache) {
        st.caches.touch(c.peer.uid, home, &scope, &grant_roots);
    }
    let mut extra: Vec<(String, String)> = cache.iter().flat_map(|d| crate::allowlist::CACHE_VARS.iter().filter(|(k, _)| !unset.contains(k))
        .map(move |(k, sub)| (k.to_string(), d.join(sub).display().to_string()))).collect();
    // (the person's home as HOME: the tools find their own settings there, read only)
    extra.push(("HOME".into(), rh.display().to_string()));
    extra.extend(unset.iter().map(|k| (k.to_string(), String::new())));
    // the cache's settings files: read only (kalangod writes them)
    if let Some(d) = &cache {
        read_only.extend(crate::allowlist::CACHE_KEEP.iter().map(|k| d.join(k)));
    }
    write_roots.extend(cache);
    let reads = crate::startup::carve(&allowed, &protected, &[]);
    let writes = crate::startup::carve(&write_roots, &protected, &read_only);
    // each write rule pinned to what is there now: one swapped in the meantime (for a link, or another folder) gets no
    // rule (limits.rs); one that already goes through a link here is left out
    // (and only inside the folders asked for: `carve` follows a link at a folder's own place — one swapped in right then
    // would lead elsewhere, and what is there is not what was approved)
    let fixas: Vec<limits::Fixa> = writes.write.iter().filter(|p| write_roots.iter().any(|r| crate::policy::is_within(p, r))).filter_map(|p| {
        let f = crate::allowlist::pin(p, &rh);
        if f.is_none() {
            eprintln!("kalangod: {}: não fixada (fora da pasta pessoal ou por um link): sem escrita ali", p.display());
        }
        f
    }).collect();
    let landlock = Landlock {
        leitura: SISTEMA_LEITURA.iter().map(Into::into).chain(reads.write).chain(reads.read).chain(writes.read).collect(),
        // (no shared /tmp, /var/tmp nor /dev/shm: the command has its own TMPDIR, and nothing it leaves is picked up by
        // the person's programs)
        escrita: SISTEMA_ESCRITA.iter().filter(|p| !p.ends_with("tmp") && !p.ends_with("shm")).map(Into::into).collect(),
        listar: reads.list.into_iter().chain(writes.list).collect(), portas: None, fixas, sockets: vec![],
    };
    let lim = c.cfg.limites.narrowed(a);
    // with the changes let through, what the command changes in them still goes to the timeline, with undo
    let before = auto.then(|| crate::startup::snapshot(c.peer, c.cfg, &write_roots));
    // (no local sockets: nothing the command asks for over D-Bus or systemd runs outside its cgroup — see Seccomp)
    let v = limits::run_task(Pedido { programa: prog, argumentos: &args, uid: c.peer.uid, gid: c.peer.gid, limites: lim, landlock: Some(landlock),
        seccomp: Some(limits::Seccomp { sem_rede: false, sem_unix: true }), ambiente: None, pasta, dry_run: false, tmp_privado: true, extra })?;
    if let Some(before) = before {
        record_startup_changes(c, prog, &before, &crate::startup::snapshot(c.peer, c.cfg, &write_roots));
    }
    // (without root there is no private shared memory for the task, and the shared one is the person's: Python's
    // multiprocessing locks and pools fail — said, so the agent does not guess)
    let mut v = v;
    let erros = v["erros"].as_str().unwrap_or("").to_string();
    if limits::landlock_abi() < 9 && (erros.contains("forkserver") || erros.contains("AF_UNIX")) {
        v["nota"] = json!(tr("this kernel's Landlock cannot keep commands off other programs' local sockets (it needs ABI 9), so commands get no local sockets at all: Python's process pools (forkserver) fail here; use threads, or the \"fork\" start method"));
    }
    if !limits::private_shm() && (erros.contains("SemLock") || erros.contains("/dev/shm")) {
        v["nota"] = json!(tr("this system runs kalangod without root, so commands get no shared memory (/dev/shm): Python's multiprocessing locks and process pools fail here; use threads, or run it as the person"));
    }
    let resumo = if v["estourou_memoria"] == true { tr("stopped: over the memory limit").to_string() }
        else if v["estourou_tempo"] == true { tr("stopped: over the time limit").to_string() }
        else { trf("finished with code {code}", &[("code", v["codigo"].to_string())]) };
    Ok(Out::read(v, format!("{prog}: {resumo}")))
}

/// Each startup file a command changed, as its own timeline record with undo (the earlier text is kept out of the
/// agents' reach, as `arquivos.escrever` keeps it), marked so the agents center tells the person.
fn record_startup_changes(c: &Ctx, prog: &str, before: &std::collections::BTreeMap<String, Option<String>>, after: &std::collections::BTreeMap<String, Option<String>>) {
    for (path, old, new) in crate::startup::changes(before, after) {
        let shown = crate::startup::tilde(std::path::Path::new(&path), &c.peer.home);
        let inverse = match &old {
            // (a link, recorded by where it pointed, is not put back as a file)
            Some(o) if !o.starts_with('→') => {
                let dir = c.cfg.state_dir.join("copias").join(c.peer.uid.to_string());
                static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let copy = dir.join(format!("{}-inicio-{}-{}", crate::timeline::now(), std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                (std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&copy, o).is_ok())
                    .then(|| json!({"ferramenta": "arquivos.restaurar", "argumentos": {"caminho": path, "copia": copy}}))
            }
            Some(_) => None,
            None => Some(json!({"ferramenta": "arquivos.apagar_criado", "argumentos": {"caminho": path}})),
        };
        let summary = match (&old, &new) {
            (None, _) => trf("a command created {path}, which runs when you log in", &[("path", shown.clone())]),
            (_, None) => trf("a command removed {path}, which ran when you logged in", &[("path", shown.clone())]),
            _ => trf("a command changed {path}, which runs when you log in", &[("path", shown.clone())]),
        };
        c.timeline.append(crate::timeline::Entry {
            id: 0, time: 0, caller: format!("{} ({})", c.rule.name, c.peer.exe), uid: c.peer.uid, pid: c.peer.pid, tool: "tarefas.executar".into(), level: "usuario".into(),
            args: json!({"programa": prog, "arquivo": path}), status: "ok".into(), summary, inverse, restore_point: None, miniatura: None,
            startup: Some({
                let d = crate::startup::line_diff(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""), 3, 400);
                json!({"arquivos": [shown], "automatico": true, "comando": true, "resumo": crate::startup::diff_summary(&d),
                    "linhas": d.iter().map(|l| json!({"t": l.kind.to_string(), "l": l.text})).collect::<Vec<_>>()})
            }),
        });
    }
}

/// Usada por `sistema.executar` (nível total): root, sem Landlock, mas ainda com limites.
pub fn run_as_root(c: &Ctx, prog: &str, args: &[String]) -> Result<Value, String> {
    limits::run_task(Pedido { programa: prog, argumentos: args, uid: 0, gid: 0, limites: c.cfg.limites.clone(), landlock: None, seccomp: None, ambiente: None, pasta: None, dry_run: c.cfg.dry_run, tmp_privado: false, extra: vec![] })
}

// ------------------------------------------------------------------ pontos
fn points_list(c: &Ctx, _: &Value) -> Result<Out, String> {
    let list = restore::store(c.cfg).list();
    let n = list.len();
    Ok(Out::read(json!({"pontos": list}), trf("{n} restore points", &[("n", n.to_string())])))
}

fn points_create(c: &Ctx, a: &Value) -> Result<Out, String> {
    let d = a.get("descricao").and_then(Value::as_str).unwrap_or("criado por um agente");
    let p = restore::store(c.cfg).create(d, c.cfg.dry_run)?;
    Ok(Out { summary: trf("point {id} ({kind})", &[("id", p.id.clone()), ("kind", p.tipo.clone())]), value: json!({"ponto": p}), inverse: None })
}

fn points_restore(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    let store = restore::store(c.cfg);
    if a.get("so_plano").and_then(Value::as_bool).unwrap_or(false) || c.cfg.dry_run {
        let v = store.restore(id, true)?;
        return Ok(Out::read(v, trf("plan to go back to point {id}", &[("id", id.to_string())])));
    }
    // a própria restauração vira desfazível: guarda o estado de agora antes
    let antes = store.create(&kalango_i18n::trf("before restoring {id}", &[("id", id.to_string())]), false)?;
    let v = store.restore(id, false)?;
    let mut v = v;
    v["ponto_anterior"] = json!(antes.id);
    Ok(Out { value: v, summary: trf("went back to point {id} (the previous state is in point {previous})", &[("id", id.to_string()), ("previous", antes.id.clone())]),
        inverse: Some(json!({"ferramenta": "pontos.restaurar", "argumentos": {"id": antes.id}})) })
}
