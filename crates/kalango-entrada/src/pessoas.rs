//! Who can log in: the accounts AccountsService knows (name, photo, last login), else the people of /etc/passwd
//! (uid 1000 and up, with a login shell); the guest when the system has the "convidado" account; and who already has
//! a session open (logind), to switch to it. `KALANGO_ENTRADA_PESSOAS` ("login:Nome:foto,…") replaces the list in
//! captures and tests.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Pessoa {
    pub usuario: String,
    pub nome: String,
    pub foto: Option<PathBuf>,
    /// last login (unix seconds), when known
    pub ultimo: Option<i64>,
    /// a session of this person already open (logind's id): logging in goes back to it
    pub sessao: Option<String>,
    pub convidado: bool,
}

pub const CONVIDADO: &str = "convidado";

fn de_teste(v: &str) -> Vec<Pessoa> {
    v.split(',').filter(|x| !x.trim().is_empty()).map(|x| {
        let c: Vec<&str> = x.split(':').collect();
        let usuario = c[0].trim().to_string();
        let nome = c.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or(&usuario).to_string();
        let foto = c.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from);
        let ultimo = c.get(3).and_then(|s| s.trim().parse::<i64>().ok());
        Pessoa { convidado: usuario == CONVIDADO, usuario, nome, foto, ultimo, sessao: None }
    }).collect()
}

fn accounts_service() -> Option<Vec<Pessoa>> {
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok()?;
    let r = bus.call_sync(Some("org.freedesktop.Accounts"), "/org/freedesktop/Accounts", "org.freedesktop.Accounts", "ListCachedUsers",
        None, None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE).ok()?;
    let caminhos: Vec<String> = r.child_value(0).iter().filter_map(|v| v.str().map(str::to_string)).collect();
    let mut v = Vec::new();
    for c in caminhos {
        let Ok(p) = bus.call_sync(Some("org.freedesktop.Accounts"), &c, "org.freedesktop.DBus.Properties", "GetAll",
            Some(&("org.freedesktop.Accounts.User",).to_variant()), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE) else { continue };
        let d = glib::VariantDict::new(Some(&p.child_value(0)));
        let s = |k: &str| d.lookup_value(k, None).and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
        let b = |k: &str| d.lookup_value(k, None).and_then(|v| v.get::<bool>()).unwrap_or(false);
        if b("SystemAccount") || b("Locked") {
            continue;
        }
        let usuario = s("UserName");
        let nome = Some(s("RealName")).filter(|n| !n.is_empty()).unwrap_or_else(|| usuario.clone());
        let foto = Some(PathBuf::from(s("IconFile"))).filter(|f| f.is_file());
        let ultimo = d.lookup_value("LoginTime", None).and_then(|v| v.get::<i64>()).filter(|t| *t > 0);
        v.push(Pessoa { convidado: usuario == CONVIDADO, usuario, nome, foto, ultimo, sessao: None });
    }
    (!v.is_empty()).then_some(v)
}

fn passwd() -> Vec<Pessoa> {
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().filter_map(|l| {
        let c: Vec<&str> = l.split(':').collect();
        let uid: u32 = c.get(2)?.parse().ok()?;
        let shell = c.get(6)?;
        if !(1000..60000).contains(&uid) || shell.ends_with("nologin") || shell.ends_with("false") {
            return None;
        }
        let usuario = c[0].to_string();
        let nome = c.get(4).map(|g| g.split(',').next().unwrap_or("").trim()).filter(|g| !g.is_empty()).unwrap_or(&usuario).to_string();
        let foto = [PathBuf::from(format!("/var/lib/AccountsService/icons/{usuario}")), PathBuf::from(c.get(5)?).join(".face")].into_iter().find(|p| p.is_file());
        Some(Pessoa { convidado: usuario == CONVIDADO, usuario, nome, foto, ultimo: None, sessao: None })
    }).collect()
}

/// Each person's open graphical session (logind), by login.
fn sessoes_abertas() -> Vec<(String, String)> {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else { return vec![] };
    let Ok(r) = bus.call_sync(Some("org.freedesktop.login1"), "/org/freedesktop/login1", "org.freedesktop.login1.Manager", "ListSessions",
        None, None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE) else { return vec![] };
    let mut v = Vec::new();
    for s in r.child_value(0).iter() {
        // (session id, uid, user, seat, object path)
        let (Some(id), Some(user), Some(seat)) = (s.child_value(0).str().map(str::to_string), s.child_value(2).str().map(str::to_string), s.child_value(3).str().map(str::to_string)) else { continue };
        if !seat.is_empty() {
            v.push((user, id));
        }
    }
    v
}

pub fn listar() -> Vec<Pessoa> {
    let mut v = match std::env::var("KALANGO_ENTRADA_PESSOAS") {
        Ok(t) => de_teste(&t),
        Err(_) => accounts_service().unwrap_or_else(passwd),
    };
    if std::env::var_os("KALANGO_ENTRADA_PESSOAS").is_none() {
        for (u, id) in sessoes_abertas() {
            if let Some(p) = v.iter_mut().find(|p| p.usuario == u) {
                p.sessao = Some(id);
            }
        }
    }
    // the guest at the end
    v.sort_by_key(|p| p.convidado);
    v
}

/// "último acesso ontem", "último acesso há 3 dias", "sessão aberta", "sessão temporária".
pub fn detalhe(p: &Pessoa, agora: i64) -> String {
    use kalango_i18n::{tr, trf};
    if p.convidado {
        return tr("temporary session").to_string();
    }
    if p.sessao.is_some() {
        return tr("session open").to_string();
    }
    let Some(t) = p.ultimo else { return String::new() };
    let dias = (agora - t) / 86400;
    match dias {
        d if d <= 0 => tr("last login today").to_string(),
        1 => tr("last login yesterday").to_string(),
        d => trf("last login {n} days ago", &[("n", d.to_string())]),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_test_list_and_writes_the_details() {
        let v = super::de_teste("ana:Ana Lima::1000,convidado:Convidado");
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].usuario.as_str(), v[0].nome.as_str(), v[0].ultimo), ("ana", "Ana Lima", Some(1000)));
        assert!(v[1].convidado);
        // (the words follow the system language; here, that something is said)
        assert!(!super::detalhe(&v[0], 1000 + 86400 + 10).is_empty());
        assert!(super::detalhe(&v[0], 1000 + 3 * 86400).contains('3'));
    }
}
