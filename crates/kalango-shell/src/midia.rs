//! Mídia tocando agora (MPRIS no D-Bus da sessão): navegadores, Spotify, players de vídeo. Alimenta a ilha, os
//! Controles e o "Hoje". Escolhe o player que está tocando; se nenhum estiver, o último que tocou.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Midia {
    pub titulo: String,
    pub artista: String,
    pub arte: Option<String>,
    pub tocando: bool,
    pub app: String,
}

type MediaListener = Box<dyn Fn(Option<Midia>)>;

pub struct Players {
    proxies: RefCell<HashMap<String, gio::DBusProxy>>,
    ultimo: RefCell<Option<String>>,
    ouvintes: RefCell<Vec<MediaListener>>,
}

const PREFIXO: &str = "org.mpris.MediaPlayer2.";

/// A value of the metadata's a{sv}, out of its box when it comes in one ("v"); as it is otherwise. (Asking for the
/// box of a value that has none is a GLib-CRITICAL on every MPRIS change.)
fn sem_caixa(v: &glib::Variant) -> glib::Variant {
    if v.type_() == glib::VariantTy::VARIANT {
        v.as_variant().unwrap_or_else(|| v.clone())
    } else {
        v.clone()
    }
}

fn ler(p: &gio::DBusProxy) -> Option<Midia> {
    let meta = p.cached_property("Metadata")?;
    let meta: HashMap<String, glib::Variant> = meta.get()?;
    let s = |k: &str| meta.get(k).and_then(|v| sem_caixa(v).get::<String>());
    let artista = meta.get("xesam:artist").and_then(|v| {
        let v = sem_caixa(v);
        v.get::<Vec<String>>().map(|a| a.join(", ")).or_else(|| v.get::<String>())
    }).unwrap_or_default();
    let titulo = s("xesam:title").unwrap_or_default();
    if titulo.is_empty() {
        return None;
    }
    let tocando = p.cached_property("PlaybackStatus").and_then(|v| v.get::<String>()).map(|x| x == "Playing").unwrap_or(false);
    let nome = p.name().map(|n| n.to_string()).unwrap_or_default();
    let app = nome.trim_start_matches(PREFIXO).split('.').next().unwrap_or("").to_string();
    Some(Midia { titulo, artista, arte: s("mpris:artUrl"), tocando, app })
}

impl Players {
    pub fn iniciar() -> Rc<Players> {
        let p = Rc::new(Players { proxies: RefCell::default(), ultimo: RefCell::default(), ouvintes: RefCell::default() });
        let Ok(con) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return p };
        // players que já existem
        if let Ok(r) = con.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", None,
            Some(glib::VariantTy::new("(as)").unwrap()), gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE) {
            let nomes: Vec<String> = r.child_value(0).get().unwrap_or_default();
            for n in nomes.into_iter().filter(|n| n.starts_with(PREFIXO)) {
                p.adicionar(&con, &n);
            }
        }
        // players que aparecem e somem
        let w = Rc::downgrade(&p);
        let c2 = con.clone();
        con.signal_subscribe(Some("org.freedesktop.DBus"), Some("org.freedesktop.DBus"), Some("NameOwnerChanged"), Some("/org/freedesktop/DBus"), None,
            gio::DBusSignalFlags::NONE, move |_, _, _, _, _, args| {
                let Some(p) = w.upgrade() else { return };
                let (nome, _, novo): (String, String, String) = args.get().unwrap_or_default();
                if !nome.starts_with(PREFIXO) {
                    return;
                }
                if novo.is_empty() {
                    p.proxies.borrow_mut().remove(&nome);
                    p.avisar();
                } else {
                    p.adicionar(&c2, &nome);
                }
            });
        p
    }

    fn adicionar(self: &Rc<Self>, con: &gio::DBusConnection, nome: &str) {
        // (made without waiting: a player in this same process — the Music app — answers the proxy's first
        // question on this thread, which a synchronous call would keep stuck until its 25 s timeout)
        let w = Rc::downgrade(self);
        let n = nome.to_string();
        gio::DBusProxy::new(con, gio::DBusProxyFlags::NONE, None, Some(nome), "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player",
            gio::Cancellable::NONE, move |r| {
                match (w.upgrade(), r) {
                    (Some(s), Ok(proxy)) => s.guardar(&n, proxy),
                    (_, Err(e)) => eprintln!("kalango-shell: mídia: {n}: {e}"),
                    _ => {}
                }
            });
    }

    fn guardar(self: &Rc<Self>, nome: &str, proxy: gio::DBusProxy) {
        let w = Rc::downgrade(self);
        let n = nome.to_string();
        // "g-properties-changed": the player changed track, state or cover
        proxy.connect_local("g-properties-changed", false, move |args| {
            let p = args[0].get::<gio::DBusProxy>().ok()?;
            if let Some(s) = w.upgrade() {
                if ler(&p).map(|m| m.tocando).unwrap_or(false) {
                    *s.ultimo.borrow_mut() = Some(n.clone());
                }
                s.avisar();
            }
            None
        });
        if ler(&proxy).map(|m| m.tocando).unwrap_or(false) {
            *self.ultimo.borrow_mut() = Some(nome.to_string());
        }
        self.proxies.borrow_mut().insert(nome.to_string(), proxy);
        self.avisar();
    }

    fn escolhido(&self) -> Option<gio::DBusProxy> {
        let ps = self.proxies.borrow();
        ps.values().find(|p| ler(p).map(|m| m.tocando).unwrap_or(false)).cloned()
            .or_else(|| self.ultimo.borrow().as_ref().and_then(|n| ps.get(n).cloned()))
            .or_else(|| ps.values().find(|p| ler(p).is_some()).cloned())
    }

    pub fn atual(&self) -> Option<Midia> {
        self.escolhido().and_then(|p| ler(&p))
    }

    pub fn ouvir(&self, f: impl Fn(Option<Midia>) + 'static) {
        f(self.atual());
        self.ouvintes.borrow_mut().push(Box::new(f));
    }

    fn avisar(&self) {
        let m = self.atual();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            if let Some(m) = &m {
                eprintln!("debug midia: {} «{}» tocando {}{}", m.app, m.titulo, m.tocando, if m.arte.is_some() { " (capa)" } else { "" });
            }
        }
        for f in self.ouvintes.borrow().iter() {
            f(m.clone());
        }
    }

    /// Where the track is and how long it is (µs). Players do not announce the position as it moves: it is asked.
    pub fn posicao(&self, f: impl FnOnce(Option<(i64, i64)>) + 'static) {
        let Some(p) = self.escolhido() else { return f(None) };
        let comprimento = p.cached_property("Metadata").and_then(|m| glib::VariantDict::new(Some(&m)).lookup_value("mpris:length", None))
            .and_then(|v| v.get::<i64>().or_else(|| v.get::<u64>().map(|x| x as i64)));
        let (Some(nome), Some(con)) = (p.name(), Some(p.connection())) else { return f(None) };
        con.call(Some(&nome), "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties", "Get",
            Some(&("org.mpris.MediaPlayer2.Player", "Position").to_variant()), None, gio::DBusCallFlags::NONE, 1000, gio::Cancellable::NONE,
            move |r| {
                let pos = r.ok().and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.get::<i64>());
                f(pos.zip(comprimento));
            });
    }

    /// The players playing now and their volume (MPRIS `Volume`, 0 – 1): (bus name, volume).
    pub fn volumes_tocando(&self) -> Vec<(String, f64)> {
        self.proxies.borrow().iter().filter(|(_, p)| ler(p).is_some_and(|m| m.tocando))
            .filter_map(|(n, p)| Some((n.clone(), p.cached_property("Volume")?.get::<f64>()?))).collect()
    }

    /// Sets a player's volume (0 – 1).
    pub fn definir_volume(&self, nome: &str, v: f64) {
        let Some(p) = self.proxies.borrow().get(nome).cloned() else { return };
        let (Some(dono), con) = (p.name(), p.connection()) else { return };
        con.call(Some(&dono), "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties", "Set",
            Some(&("org.mpris.MediaPlayer2.Player", "Volume", v.clamp(0.0, 1.0).to_variant()).to_variant()), None, gio::DBusCallFlags::NONE, 1000,
            gio::Cancellable::NONE, |_| {});
    }

    /// `PlayPause`, `Next` ou `Previous`.
    pub fn comando(&self, metodo: &str) {
        if let Some(p) = self.escolhido() {
            p.call(metodo, None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE, |_| {});
        }
    }
}

/// Imagem da capa (arquivo local ou URL file://); URLs http ficam de fora (nada é baixado pelo shell).
pub fn capa(m: &Midia, tamanho: i32) -> gtk::Widget {
    if let Some(caminho) = m.arte.as_deref().and_then(|a| a.strip_prefix("file://")) {
        let caminho = glib::Uri::unescape_string(caminho, None::<&str>).map(|s| s.to_string()).unwrap_or(caminho.to_string());
        if std::path::Path::new(&caminho).exists() {
            let p = gtk::Picture::for_filename(&caminho);
            p.set_content_fit(gtk::ContentFit::Cover);
            p.set_size_request(tamanho, tamanho);
            p.add_css_class("capa");
            return p.upcast();
        }
    }
    let i = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    i.set_pixel_size((tamanho / 2).max(12));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("capa");
    b.set_size_request(tamanho, tamanho);
    i.set_vexpand(true);
    b.append(&i);
    b.upcast()
}

/// A cover that slides when the track changes, the same way in the Music app, the island and the desktop's widget
/// (design "Música"): the new one comes in as the old one leaves, easing out; a pause or a play does not move it.
pub struct CapaDeslizante {
    stack: gtk::Stack,
    chave: RefCell<String>,
    tamanho: i32,
    n: Cell<u32>,
}

impl CapaDeslizante {
    pub fn new(tamanho: i32) -> Rc<CapaDeslizante> {
        let stack = gtk::Stack::new();
        stack.add_css_class("capa-deslizante");
        stack.set_size_request(tamanho, tamanho);
        stack.set_overflow(gtk::Overflow::Hidden);
        stack.set_transition_duration(260);
        Rc::new(CapaDeslizante { stack, chave: RefCell::default(), tamanho, n: Cell::new(0) })
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.stack
    }

    pub fn mostrar(&self, m: &Midia) {
        let chave = format!("{}\n{}\n{}", m.titulo, m.artista, m.arte.clone().unwrap_or_default());
        if *self.chave.borrow() == chave {
            return;
        }
        let primeira = self.chave.borrow().is_empty();
        *self.chave.borrow_mut() = chave;
        let nova = capa(m, self.tamanho);
        let nome = format!("capa{}", self.n.get());
        self.n.set(self.n.get() + 1);
        self.stack.add_named(&nova, Some(&nome));
        let antiga = self.stack.visible_child();
        self.stack.set_transition_type(if primeira { gtk::StackTransitionType::None } else { gtk::StackTransitionType::SlideLeft });
        self.stack.set_visible_child(&nova);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && !primeira {
            eprintln!("debug midia: capa desliza «{}» ({} px)", m.titulo, self.tamanho);
        }
        // (the old one goes once it is out of view)
        if let Some(a) = antiga {
            let s = self.stack.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(s) = s.upgrade() {
                    if a.parent().is_some_and(|p| p == *s.upcast_ref::<gtk::Widget>()) && s.visible_child().as_ref() != Some(&a) {
                        s.remove(&a);
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sem_caixa;
    use gtk::glib::{self, prelude::ToVariant};

    #[test]
    fn a_boxed_value_comes_out_and_a_plain_one_stays() {
        let plain = "Faixa".to_variant();
        assert_eq!(sem_caixa(&plain).get::<String>().as_deref(), Some("Faixa"));
        let boxed = glib::Variant::from_variant(&plain);
        assert_eq!(boxed.type_().as_str(), "v");
        assert_eq!(sem_caixa(&boxed).get::<String>().as_deref(), Some("Faixa"));
    }
}
