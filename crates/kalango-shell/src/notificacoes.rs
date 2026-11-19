//! Notificações: o shell é o servidor `org.freedesktop.Notifications` da sessão (o mesmo que `notify-send`,
//! navegadores e apps GTK/Qt usam). Banners centered under the island, 12 px below the top bar (design frame
//! "Notificação (banner)"), or in the corner the person chose; the app's actions plus "Mark as read". What each
//! notification does follows the person's rules (Settings › Notifications, `notification_policy`): banner and
//! sound, banner only, only in the list (Do not disturb), held for the agent's digest, or nothing.
//!
//! Design (página "03 · Componentes"): cartão de 380 px; cabeçalho com ícone, app e hora, ponto de não lida e
//! fechar; título, texto, imagem opcional, barra de progresso (dica `value`) e ações (a primeira em destaque).
//! Movimento: entra deslizando 24 px + opacidade (mola suave), sai com a mola rápida; quando um banner entra ou
//! sai, os outros deslizam para o novo lugar (só transformação: medem antes e depois e animam a diferença).

use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use kalango_i18n::tr;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_movimento::{self as mv, Animavel, Canal};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const XML: &str = r#"<node><interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="Notify">
  <arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/>
  <arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
  <arg type="u" direction="out"/>
</method>
<method name="CloseNotification"><arg type="u" direction="in"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
</interface></node>"#;

const MAX_VISIVEIS: usize = 4;
const PADRAO_MS: u32 = 6000;

/// Motivos de fechamento (especificação freedesktop).
#[derive(Clone, Copy)]
enum Motivo {
    Expirou = 1,
    Dispensada = 2,
    Fechada = 3,
}

struct Banner {
    id: u32,
    raiz: Animavel,
    titulo: gtk::Label,
    texto: gtk::Label,
    progresso: gtk::ProgressBar,
    temporizador: RefCell<Option<glib::SourceId>>,
}

/// Um aviso guardado na central (barra lateral, modo Avisos).
#[derive(Clone)]
pub struct Guardado {
    pub id: u32,
    pub app: String,
    pub icone: String,
    pub titulo: String,
    pub corpo: String,
    pub imagem: Option<String>,
    pub acoes: Vec<(String, String)>,
    pub quando: i64,
    /// read: the person marked it (banner or list); unread ones keep the teal dot in the notices list
    pub lida: bool,
    /// held for the agent's digest (12:00 and 18:00), not yet in one
    pub digest: bool,
}

pub struct Notificacoes {
    pub historico: RefCell<Vec<Guardado>>,
    pub nao_perturbe: Cell<bool>,
    /// right side bar state: (open, showing the notification list)
    side_bar: Cell<(bool, bool)>,
    ouvintes: RefCell<Vec<Box<dyn Fn()>>>,
    janela: gtk::ApplicationWindow,
    pilha: gtk::Box,
    banners: RefCell<Vec<Rc<Banner>>>,
    proximo: Cell<u32>,
    conexao: RefCell<Option<gio::DBusConnection>>,
    /// the shell's own notices with actions: what each one does (by notice)
    locais: RefCell<HashMap<u32, AcaoLocal>>,
}

type AcaoLocal = Box<dyn Fn(&str)>;

#[derive(Default)]
struct Pedido {
    app: String,
    substitui: u32,
    icone: String,
    titulo: String,
    corpo: String,
    acoes: Vec<(String, String)>,
    imagem: Option<String>,
    progresso: Option<i32>,
    critica: bool,
    expira: i32,
    /// an approval the agent waits for (category "x-kalango.approval")
    approval: bool,
    /// straight into the list, no banner (an agent's finished task, when the person chose "Only in the list")
    so_lista: bool,
}

fn pedido(params: &glib::Variant) -> Pedido {
    let get_s = |i| params.child_value(i).get::<String>().unwrap_or_default();
    let acoes: Vec<String> = params.child_value(5).get().unwrap_or_default();
    let dicas: HashMap<String, glib::Variant> = params.child_value(6).get().unwrap_or_default();
    let dica_s = |k: &str| dicas.get(k).and_then(|v| v.get::<String>());
    Pedido {
        app: get_s(0),
        substitui: params.child_value(1).get().unwrap_or(0),
        icone: get_s(2),
        titulo: get_s(3),
        corpo: get_s(4),
        acoes: acoes.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect(),
        imagem: dica_s("image-path").or_else(|| dica_s("image_path")),
        progresso: dicas.get("value").and_then(|v| v.get::<i32>()),
        critica: dicas.get("urgency").and_then(|v| v.get::<u8>()).map(|u| u == 2).unwrap_or(false),
        expira: params.child_value(7).get().unwrap_or(-1),
        approval: dica_s("category").is_some_and(|c| c == "x-kalango.approval"),
        so_lista: false,
    }
}

/// Ícone por nome do tema, caminho de arquivo ou URI.
pub fn imagem(nome: &str, tamanho: i32) -> Option<gtk::Image> {
    if nome.is_empty() {
        return None;
    }
    let caminho = nome.strip_prefix("file://").unwrap_or(nome);
    let img = if caminho.starts_with('/') {
        if !std::path::Path::new(caminho).exists() {
            return None;
        }
        gtk::Image::from_file(caminho)
    } else {
        gtk::Image::from_icon_name(nome)
    };
    img.set_pixel_size(tamanho);
    Some(img)
}

impl Notificacoes {
    pub fn iniciar(app: &gtk::Application) -> Rc<Notificacoes> {
        // anchored to the top only: centered under the island
        let janela = camada(app, "kalango-notificacoes", Layer::Overlay, &[Edge::Top]);
        // the top bar's reserved zone already pushes the layer below it: 12 px more, as in the design
        janela.set_margin(Edge::Top, 12);
        // the surface follows the stack, shrinking too: a window keeps its biggest size otherwise, and the empty part
        // left when a banner goes would take the clicks meant for the windows under it
        janela.set_resizable(false);
        let pilha = caixa(Orientation::Vertical, 8, "pilha-avisos");
        pilha.set_size_request(380, -1);
        pilha.set_valign(Align::Start);
        janela.set_child(Some(&pilha));
        let n = Rc::new(Notificacoes { historico: RefCell::default(), nao_perturbe: Cell::new(false), side_bar: Cell::new((false, false)), ouvintes: RefCell::default(), janela, pilha, banners: RefCell::default(), proximo: Cell::new(1), conexao: RefCell::default(), locais: RefCell::default() });
        n.apply_position();
        let fraco = Rc::downgrade(&n);
        let fraco2 = fraco.clone();
        gio::bus_own_name(
            gio::BusType::Session,
            "org.freedesktop.Notifications",
            // na sessão de teste nunca tenta tomar o serviço de outro (o desktop de quem está testando)
            if std::env::var_os("KALANGO_SHELL_DEV").is_some() { gio::BusNameOwnerFlags::ALLOW_REPLACEMENT } else { gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT },
            move |c, _| {
                let Some(n) = fraco.upgrade() else { return };
                *n.conexao.borrow_mut() = Some(c.clone());
                let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
                let iface = info.lookup_interface("org.freedesktop.Notifications").unwrap();
                let w = Rc::downgrade(&n);
                let r = c.register_object("/org/freedesktop/Notifications", &iface)
                    .method_call(move |_, _, _, _, metodo, params, inv| {
                        let Some(n) = w.upgrade() else { return };
                        match metodo {
                            "GetCapabilities" => inv.return_value(Some(&(vec!["actions", "body", "icon-static"],).to_variant())),
                            "GetServerInformation" => inv.return_value(Some(&("KalangoOS", "KalangoOS", env!("CARGO_PKG_VERSION"), "1.2").to_variant())),
                            "Notify" => {
                                let p = pedido(&params);
                                // the screen picture taken to describe the screen (Accessibility) is not news to anyone
                                if p.app == "niri" && crate::descrever::captura_propria() {
                                    inv.return_value(Some(&(0u32,).to_variant()));
                                    return;
                                }
                                let route = n.route_for(&p);
                                crate::notification_policy::log(&p.app, &p.icone, route);
                                // its sound (at the notifications' volume) when the person's rules give it one, unless
                                // the app asks for silence
                                let dicas = glib::VariantDict::new(Some(&params.child_value(6)));
                                let mudo = dicas.lookup_value("suppress-sound", None).and_then(|v| v.get::<bool>()).unwrap_or(false);
                                if route == (crate::notification_policy::Route::Banner { sound: true }) && !mudo {
                                    let som = dicas.lookup_value("sound-name", None).and_then(|v| v.get::<String>()).unwrap_or_else(|| "message-new-instant".into());
                                    crate::configuracoes_som::play_at(&som, crate::notification_policy::read().volume);
                                }
                                let id = n.mostrar_rota(p, route);
                                inv.return_value(Some(&(id,).to_variant()));
                            }
                            "CloseNotification" => {
                                let id: u32 = params.child_value(0).get().unwrap_or(0);
                                n.historico.borrow_mut().retain(|g| g.id != id);
                                n.fechar(id, Motivo::Fechada);
                                n.avisar();
                                inv.return_value(None);
                            }
                            _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", metodo),
                        }
                    })
                    .build();
                if let Err(e) = r {
                    eprintln!("kalango-shell: notificações: {e}");
                }
            },
            |_, _| eprintln!("kalango-shell: servidor de notificações ativo"),
            move |_, _| {
                if fraco2.upgrade().is_some() {
                    eprintln!("kalango-shell: outro programa é o servidor de notificações desta sessão");
                }
            },
        );
        n
    }

    pub fn ouvir(&self, f: impl Fn() + 'static) {
        self.ouvintes.borrow_mut().push(Box::new(f));
    }

    fn avisar(&self) {
        for f in self.ouvintes.borrow().iter() {
            f();
        }
    }

    /// Tira um aviso da central (e da tela, se ainda estiver lá).
    pub fn dispensar(self: &Rc<Self>, id: u32) {
        self.historico.borrow_mut().retain(|g| g.id != id);
        self.fechar(id, Motivo::Dispensada);
        self.avisar();
    }

    /// "Mark all as read" (the notices summary).
    pub fn marcar_todas_lidas(self: &Rc<Self>) {
        let ids: Vec<u32> = self.historico.borrow().iter().filter(|g| !g.lida).map(|g| g.id).collect();
        for g in self.historico.borrow_mut().iter_mut() {
            g.lida = true;
        }
        for id in ids {
            self.fechar(id, Motivo::Dispensada);
        }
        self.avisar();
    }

    pub fn limpar(self: &Rc<Self>) {
        let ids: Vec<u32> = self.historico.borrow().iter().map(|g| g.id).collect();
        self.historico.borrow_mut().clear();
        for id in ids {
            self.fechar(id, Motivo::Dispensada);
        }
        self.avisar();
    }

    /// Ação escolhida na central: o app recebe ActionInvoked e o aviso sai.
    pub fn acionar(self: &Rc<Self>, id: u32, chave: &str) {
        let local = self.locais.borrow_mut().remove(&id);
        match local {
            Some(f) => f(chave),
            None => self.sinal("ActionInvoked", (id, chave).to_variant()),
        }
        self.dispensar(id);
    }

    /// A notice of the shell itself with actions ((key, label)); `f` gets the key chosen.
    pub fn notify_local_acoes(self: &Rc<Self>, app: &str, title: &str, body: &str, acoes: &[(&str, &str)], f: impl Fn(&str) + 'static) -> u32 {
        let id = self.mostrar(Pedido { app: app.into(), titulo: title.into(), corpo: body.into(), expira: -1,
            acoes: acoes.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect(), ..Default::default() });
        self.locais.borrow_mut().insert(id, Box::new(f));
        id
    }

    /// A critical notice of the shell's (battery, disk): it stays until answered and gets past Do not disturb.
    pub fn notify_critico(self: &Rc<Self>, app: &str, title: &str, body: &str, acoes: &[(&str, &str)], f: impl Fn(&str) + 'static) -> u32 {
        let id = self.mostrar(Pedido { app: app.into(), titulo: title.into(), corpo: body.into(), expira: 0, critica: true,
            acoes: acoes.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect(), ..Default::default() });
        self.locais.borrow_mut().insert(id, Box::new(f));
        id
    }

    /// A task the agent finished out of sight (Agents › Settings › Notifications): a banner, only the list, or
    /// nothing at all.
    pub fn notify_agent_task(self: &Rc<Self>, title: &str, body: &str) {
        let how = crate::agent_prefs::load().task_notice;
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug aviso: tarefa do agente ({how}): {title}");
        }
        if how == "nada" {
            return;
        }
        self.mostrar(Pedido { app: tr("Agents").into(), titulo: title.into(), corpo: body.into(), expira: -1, so_lista: how == "lista", ..Default::default() });
    }

    /// Notification raised by the shell itself (e.g. the focus timer).
    pub fn notify_local(self: &Rc<Self>, app: &str, title: &str, body: &str) {
        self.mostrar(Pedido { app: app.into(), titulo: title.into(), corpo: body.into(), expira: -1, ..Default::default() });
    }

    /// The right side bar opened or closed. With the notification list visible, new notifications go straight
    /// into it (no banner); with another mode open, banners move to the left of the bar instead of covering it.
    pub fn set_side_bar(&self, open: bool, showing_list: bool) {
        // banners sit in the middle, clear of the bar: only the "list is showing" case changes anything
        self.side_bar.set((open, showing_list));
    }

    /// "Mark as read": the banner leaves and the notice stays in the list without the unread dot.
    pub fn marcar_lida(self: &Rc<Self>, id: u32) {
        if let Some(g) = self.historico.borrow_mut().iter_mut().find(|g| g.id == id) {
            g.lida = true;
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso: marcada como lida: {}", g.titulo);
            }
        }
        self.fechar(id, Motivo::Dispensada);
        self.avisar();
    }

    pub fn definir_nao_perturbe(&self, v: bool) {
        self.nao_perturbe.set(v);
        self.avisar();
    }

    /// Do not disturb now: by hand (Controls, Settings) or by itself (at night, while the focus timer runs).
    pub fn dnd_now(&self) -> bool {
        // by hand, a focus session holding them (its "Guardar notificações"), or a screen being shared with them hidden
        if self.nao_perturbe.get() || crate::focus_session::holding() || crate::screen_share::hides_notices() {
            return true;
        }
        let hour = glib::DateTime::now_local().map(|d| d.hour() as u32).unwrap_or(12);
        crate::notification_policy::auto_dnd(&crate::notification_policy::read(), hour, false)
    }

    fn route_for(&self, p: &Pedido) -> crate::notification_policy::Route {
        if p.so_lista {
            return crate::notification_policy::Route::ListOnly;
        }
        let n = crate::notification_policy::Notice { app: &p.app, title: &p.titulo, body: &p.corpo, critical: p.critica, approval: p.approval };
        // the agents' notices may pass Do not disturb when the person says so (Agents › Settings › Notifications)
        let dnd = self.dnd_now() && !(p.app == tr("Agents") && !crate::agent_prefs::load().respect_dnd);
        let r = crate::notification_policy::route(&crate::notification_policy::read(), &n, dnd);
        // the focused space's rule ("Espaços por intenção"): what may not interrupt it waits in the list
        if matches!(r, crate::notification_policy::Route::Banner { .. }) && crate::spaces::holds(&p.app, p.approval, p.critica) {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso: guardada pela regra do espaço: {}", p.titulo);
            }
            return crate::notification_policy::Route::ListOnly;
        }
        r
    }

    /// Where the banners show (Settings › Notifications): under the island or in a corner.
    pub fn apply_position(&self) {
        use crate::notification_policy::Position;
        let mut pos = crate::notification_policy::read().position;
        // the taskbar has no island: the banners come above it, at the right (as on such a desktop)
        if pos == crate::notification_policy::Position::Island && crate::superficies::modo_barra_de_tarefas() {
            pos = crate::notification_policy::Position::BottomRight;
        }
        let (top, bottom, left, right) = match pos {
            Position::Island => (true, false, false, false),
            Position::TopRight => (true, false, false, true),
            Position::TopLeft => (true, false, true, false),
            Position::BottomRight => (false, true, false, true),
        };
        self.janela.set_anchor(Edge::Top, top);
        self.janela.set_anchor(Edge::Bottom, bottom);
        self.janela.set_anchor(Edge::Left, left);
        self.janela.set_anchor(Edge::Right, right);
        // 12 px from the top bar (its zone already pushes the layer down) and the edges; above the dock at the bottom
        self.janela.set_margin(Edge::Top, 12);
        self.janela.set_margin(Edge::Left, 12);
        self.janela.set_margin(Edge::Right, 12);
        self.janela.set_margin(Edge::Bottom, 84);
        self.pilha.set_valign(if bottom { Align::End } else { Align::Start });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug aviso: banners em {pos:?}");
        }
    }

    /// The agent's digest: the notifications held for it, summed up by the local model in one banner (each one stays
    /// in the list). At 12:00 and 18:00, or when asked (`kalango-shell avisos resumir`).
    pub fn deliver_digest(self: &Rc<Self>) {
        let held: Vec<Guardado> = self.historico.borrow().iter().filter(|g| g.digest).cloned().collect();
        if held.is_empty() {
            return;
        }
        let ids: Vec<u32> = held.iter().map(|g| g.id).collect();
        for g in self.historico.borrow_mut().iter_mut() {
            if ids.contains(&g.id) {
                g.digest = false;
            }
        }
        let input = serde_json::Value::Array(held.iter().map(|g| serde_json::json!({"app": g.app, "titulo": g.titulo, "corpo": g.corpo})).collect()).to_string();
        let n = held.len();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let bin = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|p| p.is_file())
                .unwrap_or_else(|| "kalango-agente".into());
            let mut c = std::process::Command::new(bin).arg("avisos").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null()).spawn().ok()?;
            if let Some(mut i) = c.stdin.take() {
                let _ = std::io::Write::write_all(&mut i, input.as_bytes());
            }
            let o = c.wait_with_output().ok()?;
            // {"gerado", "pontos": [...]}: the points, one per line
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            let points: Vec<String> = v["pontos"].as_array()?.iter().filter_map(|p| p.as_str()).map(|p| format!("• {p}")).collect();
            Some(points.join("\n")).filter(|s| !s.is_empty())
        }, move |resumo| {
            let Some(me) = w.upgrade() else { return };
            let titulo = if n == 1 { tr("Agent's digest: 1 notification").to_string() } else { kalango_i18n::trf("Agent's digest: {n} notifications", &[("n", n.to_string())]) };
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso: resumo do agente ({n} avisos)");
            }
            me.notify_local(tr("Agents"), &titulo, &resumo.unwrap_or_else(|| tr("They are in the notifications list.").to_string()));
        });
    }

    /// Every minute: the digest at its hours (once each).
    pub fn watch_digest(self: &Rc<Self>) {
        thread_local! { static LAST: Cell<(i32, u32)> = const { Cell::new((0, 0)) }; }
        let Ok(now) = glib::DateTime::now_local() else { return };
        let hour = now.hour() as u32;
        if !crate::notification_policy::DIGEST_HOURS.contains(&hour) || !crate::notification_policy::read().agent_digest {
            return;
        }
        let slot = (now.day_of_year(), hour);
        if LAST.with(|l| l.get()) != slot {
            LAST.with(|l| l.set(slot));
            self.deliver_digest();
        }
    }

    fn sinal(&self, nome: &str, v: glib::Variant) {
        if let Some(c) = self.conexao.borrow().as_ref() {
            let _ = c.emit_signal(None, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", nome, Some(&v));
        }
    }

    /// Posição de cada banner antes de uma mudança na pilha.
    fn medir(&self) -> Vec<(Animavel, f32)> {
        self.banners.borrow().iter().filter_map(|b| b.raiz.compute_bounds(&self.pilha).map(|r| (b.raiz.clone(), r.y()))).collect()
    }

    /// Depois do novo layout, cada banner que mudou de lugar começa no lugar antigo e desliza (mola suave).
    fn reacomodar(&self, antes: Vec<(Animavel, f32)>) {
        let pilha = self.pilha.clone();
        self.pilha.add_tick_callback(move |_, _| {
            for (a, y0) in &antes {
                if let Some(r) = a.compute_bounds(&pilha) {
                    let d = (*y0 - r.y()) as f64 + a.valor(Canal::Y);
                    if d.abs() > 0.5 {
                        a.definir(Canal::Y, d);
                        a.ir(Canal::Y, 0.0, mv::SUAVE);
                    }
                }
            }
            glib::ControlFlow::Break
        });
    }

    /// The shell's own notices go by the same rules as the apps'.
    fn mostrar(self: &Rc<Self>, p: Pedido) -> u32 {
        let route = self.route_for(&p);
        self.mostrar_rota(p, route)
    }

    fn mostrar_rota(self: &Rc<Self>, p: Pedido, route: crate::notification_policy::Route) -> u32 {
        use crate::notification_policy::Route;
        // substituir: atualiza o banner no lugar (progresso de download, por exemplo)
        if p.substitui != 0 {
            if let Some(b) = self.banners.borrow().iter().find(|b| b.id == p.substitui).cloned() {
                b.titulo.set_text(&p.titulo);
                b.texto.set_text(&p.corpo);
                b.texto.set_visible(!p.corpo.is_empty());
                if let Some(v) = p.progresso {
                    b.progresso.set_fraction(v.clamp(0, 100) as f64 / 100.0);
                }
                self.agendar(&b, &p);
                return b.id;
            }
        }
        let id = self.proximo.get();
        self.proximo.set(id + 1);
        // an app turned off: nothing at all
        if route == Route::Drop {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso: descartada ({}): {}", p.app, p.titulo);
            }
            return id;
        }
        self.historico.borrow_mut().insert(0, Guardado {
            id, app: p.app.clone(), icone: p.icone.clone(), titulo: p.titulo.clone(), corpo: p.corpo.clone(), imagem: p.imagem.clone(),
            acoes: p.acoes.clone(), quando: glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0), lida: false,
            digest: route == Route::Digest,
        });
        self.historico.borrow_mut().truncate(50);
        self.avisar();
        // Do not disturb, or held for the agent's digest: straight into the list, no banner
        if matches!(route, Route::ListOnly | Route::Digest) {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug aviso: {} ({}): {}", crate::notification_policy::route_name(route), p.app, p.titulo);
            }
            return id;
        }
        if self.side_bar.get() == (true, true) {
            return id;
        }
        let b = self.banner(id, &p);
        let antes = self.medir();
        self.pilha.prepend(&b.raiz);
        self.banners.borrow_mut().insert(0, b.clone());
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug aviso: {} na tela", self.banners.borrow().len());
        }
        // comes down 12 px from the island with opacity (gentle spring); the others move down to make room
        b.raiz.definir(Canal::Y, -12.0).definir(Canal::Opacidade, 0.0);
        b.raiz.ir(Canal::Y, 0.0, mv::SUAVE).ir(Canal::Opacidade, 1.0, mv::SUAVE);
        self.reacomodar(antes);
        self.janela.present();
        self.agendar(&b, &p);
        // limite de banners na tela: o mais antigo sai
        let excesso: Vec<u32> = self.banners.borrow().iter().skip(MAX_VISIVEIS).map(|b| b.id).collect();
        for e in excesso {
            self.fechar(e, Motivo::Expirou);
        }
        id
    }

    fn agendar(self: &Rc<Self>, b: &Rc<Banner>, p: &Pedido) {
        if let Some(t) = b.temporizador.take() {
            t.remove();
        }
        let ms = match p.expira {
            0 => return,
            _ if p.critica => return,
            -1 => PADRAO_MS,
            n => n.max(1000) as u32,
        };
        let w = Rc::downgrade(self);
        let id = b.id;
        let bf = Rc::downgrade(b);
        *b.temporizador.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(ms as u64), move || {
            if let Some(b) = bf.upgrade() {
                b.temporizador.take();
            }
            if let Some(n) = w.upgrade() {
                n.fechar(id, Motivo::Expirou);
            }
        }));
    }

    fn fechar(self: &Rc<Self>, id: u32, motivo: Motivo) {
        let b = {
            let mut bs = self.banners.borrow_mut();
            let Some(i) = bs.iter().position(|b| b.id == id) else { return };
            bs.remove(i)
        };
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug aviso: {} na tela", self.banners.borrow().len());
        }
        if let Some(t) = b.temporizador.take() {
            t.remove();
        }
        self.sinal("NotificationClosed", (id, motivo as u32).to_variant());
        // goes back up into the island with the quick spring; then the ones below slide up
        b.raiz.set_can_target(false);
        let me = self.clone();
        let raiz = b.raiz.clone();
        b.raiz.sair((0.0, -12.0), 0.98, move || {
            let antes = me.medir();
            me.pilha.remove(&raiz);
            me.reacomodar(antes);
            if me.banners.borrow().is_empty() {
                me.janela.set_visible(false);
            }
        });
    }

    fn banner(self: &Rc<Self>, id: u32, p: &Pedido) -> Rc<Banner> {
        let cartao = caixa(Orientation::Vertical, 10, "aviso");
        cartao.set_size_request(380, -1);
        if p.critica {
            cartao.add_css_class("critico");
        }
        let topo = caixa(Orientation::Horizontal, 8, "");
        if let Some(i) = imagem(&p.icone, 18) {
            topo.append(&i);
        }
        let app = rotulo(if p.app.is_empty() { tr("Notice") } else { &p.app }, "aviso-app");
        topo.append(&app);
        let quando = rotulo(&format!("· {}", tr("now")), "aviso-hora");
        quando.set_hexpand(true);
        quando.set_halign(Align::Start);
        topo.append(&quando);
        let ponto = caixa(Orientation::Horizontal, 0, "aviso-nao-lida");
        ponto.set_valign(Align::Center);
        topo.append(&ponto);
        let x = gtk::Button::new();
        x.add_css_class("aviso-fechar");
        x.set_child(Some(&crate::conversa::ui_icon("fechar", "#8E8F99", 16)));
        x.set_tooltip_text(Some(tr("Dismiss")));
        topo.append(&x);
        cartao.append(&topo);

        let meio = caixa(Orientation::Horizontal, 12, "");
        if let Some(img) = p.imagem.as_deref().and_then(|c| imagem(c, 40)) {
            img.add_css_class("aviso-imagem");
            img.set_valign(Align::Start);
            meio.append(&img);
        }
        let col = caixa(Orientation::Vertical, 3, "");
        col.set_hexpand(true);
        let titulo = rotulo(&p.titulo, "aviso-titulo");
        titulo.set_halign(Align::Fill);
        titulo.set_wrap(true);
        titulo.set_xalign(0.0);
        // fixed 380 px card: a minimal natural width makes the text wrap at the card's real width instead of
        // widening it
        titulo.set_max_width_chars(1);
        titulo.set_hexpand(true);
        let texto = rotulo(&p.corpo, "aviso-texto");
        texto.set_halign(Align::Fill);
        texto.set_wrap(true);
        texto.set_xalign(0.0);
        texto.set_max_width_chars(1);
        texto.set_hexpand(true);
        texto.set_lines(4);
        texto.set_ellipsize(gtk::pango::EllipsizeMode::End);
        texto.set_visible(!p.corpo.is_empty());
        col.append(&titulo);
        col.append(&texto);
        meio.append(&col);
        cartao.append(&meio);
        let progresso = gtk::ProgressBar::new();
        progresso.add_css_class("aviso-progresso");
        progresso.set_visible(p.progresso.is_some());
        progresso.set_fraction(p.progresso.unwrap_or(0).clamp(0, 100) as f64 / 100.0);
        cartao.append(&progresso);

        // the app's actions (the first one highlighted), then "Mark as read"
        let botoes: Vec<_> = p.acoes.iter().filter(|(k, _)| k != "default").cloned().collect();
        let tem_padrao = p.acoes.iter().any(|(k, _)| k == "default");
        {
            let linha = caixa(Orientation::Horizontal, 8, "");
            linha.set_homogeneous(true);
            for (i, (chave, nome)) in botoes.into_iter().enumerate() {
                let b = gtk::Button::with_label(&nome);
                b.add_css_class("aviso-acao");
                if i == 0 {
                    b.add_css_class("primaria");
                }
                // test aid: where the first action is on the output (as "Mark as read" below)
                if i == 0 && std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    let (b, j, titulo) = (b.clone(), self.janela.clone(), p.titulo.clone());
                    glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                        if let Some((x, y)) = on_output(&b, &j) {
                            eprintln!("debug aviso: primeira ação em {x} {y} :: {titulo}");
                            eprintln!("debug aviso: janela dos avisos {}x{}", j.width(), j.height());
                        }
                    });
                }
                let me = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(n) = me.upgrade() {
                        n.acionar(id, &chave);
                    }
                });
                linha.append(&b);
            }
            let lida = gtk::Button::with_label(tr("Mark as read"));
            lida.add_css_class("aviso-acao");
            // test aid: where the button is on the output
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                let (b, j, titulo) = (lida.clone(), self.janela.clone(), p.titulo.clone());
                glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                    if let Some((x, y)) = on_output(&b, &j) {
                        eprintln!("debug aviso: marcar como lida em {x} {y} :: {titulo}");
                    }
                });
            }
            let me = Rc::downgrade(self);
            lida.connect_clicked(move |_| {
                if let Some(n) = me.upgrade() {
                    n.marcar_lida(id);
                }
            });
            linha.append(&lida);
            cartao.append(&linha);
        }
        let raiz = Animavel::new(&cartao);
        raiz.set_halign(Align::Center);
        let banner = Rc::new(Banner { id, raiz, titulo, texto, progresso, temporizador: RefCell::default() });
        {
            let me = Rc::downgrade(self);
            x.connect_clicked(move |_| {
                if let Some(n) = me.upgrade() {
                    n.dispensar(id);
                }
            });
        }
        // clicar no cartão: ação padrão (abrir a conversa, o arquivo…) e fecha
        let clique = gtk::GestureClick::new();
        {
            let me = Rc::downgrade(self);
            clique.connect_released(move |g, _, _, _| {
                if let Some(n) = me.upgrade() {
                    if tem_padrao {
                        n.acionar(id, "default");
                    } else {
                        n.fechar(id, Motivo::Dispensada);
                    }
                }
                g.set_state(gtk::EventSequenceState::Claimed);
            });
        }
        cartao.add_controller(clique);
        // o mouse sobre o banner segura o tempo; ao sair, recomeça
        let mov = gtk::EventControllerMotion::new();
        {
            let (b1, me) = (Rc::downgrade(&banner), Rc::downgrade(self));
            let expira = p.expira;
            let critica = p.critica;
            mov.connect_enter(move |_, _, _| {
                if let Some(b) = b1.upgrade() {
                    if let Some(t) = b.temporizador.take() {
                        t.remove();
                    }
                }
            });
            let b2 = Rc::downgrade(&banner);
            mov.connect_leave(move |_| {
                if let (Some(b), Some(n)) = (b2.upgrade(), me.upgrade()) {
                    n.agendar(&b, &Pedido { expira, critica, ..Default::default() });
                }
            });
        }
        cartao.add_controller(mov);
        banner
    }
}

/// Test aid: a banner button's centre on the output, wherever the person put the banners (the window is anchored as
/// apply_position does: centred under the island, or 12 px from a corner; the top bar's zone pushes it down).
fn on_output(b: &gtk::Button, j: &gtk::ApplicationWindow) -> Option<(i32, i32)> {
    use crate::notification_policy::Position;
    // the output's size from niri (GDK's monitor can keep an old size after a scale change)
    let (largura, altura) = crate::superficies::tamanho_da_saida()?;
    let r = b.compute_bounds(j)?;
    let (cx, cy) = ((r.x() + r.width() / 2.) as i32, (r.y() + r.height() / 2.) as i32);
    let top = crate::superficies::BARRA_ALTURA + 12;
    Some(match crate::notification_policy::read().position {
        Position::Island => ((largura - j.width()) / 2 + cx, top + cy),
        Position::TopLeft => (12 + cx, top + cy),
        Position::TopRight => (largura - 12 - j.width() + cx, top + cy),
        Position::BottomRight => (largura - 12 - j.width() + cx, altura - 84 - j.height() + cy),
    })
}
