//! One mould for every context menu (design "Menus de contexto"): 264 px wide, 32 px rows with the glyph, the text
//! and the shortcut at the right, the agent's actions grouped and in the accent, a destructive one in coral at the
//! end; a submenu slides in with a row to go back. Files' menus are built the same way (`arquivos`).

use crate::arquivos::item_menu_completo;
use crate::conversa::ui_icon;
use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use std::rc::Rc;

pub const WIDTH: i32 = 264;

pub struct Menu {
    pub pop: gtk::Popover,
    pages: gtk::Stack,
    /// the first page (the menu itself)
    pub main: gtk::Box,
}

impl Menu {
    fn new(w: &gtk::Widget) -> Rc<Menu> {
        let pop = gtk::Popover::new();
        pop.set_parent(w);
        pop.set_has_arrow(false);
        pop.add_css_class("arq-menu");
        // (a menu of the dock: Escape closes it too)
        crate::superficies::keyboard_while_open(&pop);
        let pages = gtk::Stack::new();
        pages.set_vhomogeneous(false);
        pages.set_hhomogeneous(false);
        pages.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        pages.set_transition_duration(180);
        let main = caixa(Orientation::Vertical, 2, "");
        main.set_size_request(WIDTH, -1);
        pages.add_named(&main, Some("menu"));
        pop.set_child(Some(&pages));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        Rc::new(Menu { pop, pages, main })
    }

    /// At the pointer (`x`, `y` in `w`), as a context menu opens: its corner there.
    pub fn at(w: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Rc<Menu> {
        let m = Menu::new(w.upcast_ref());
        m.pop.set_position(gtk::PositionType::Bottom);
        m.pop.set_halign(Align::Start);
        m.pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        m
    }

    /// Beside a widget, on that side (the dock's icons: above them, or to the right of a side dock).
    pub fn beside(w: &impl IsA<gtk::Widget>, side: gtk::PositionType) -> Rc<Menu> {
        let m = Menu::new(w.upcast_ref());
        m.pop.set_position(side);
        m
    }

    /// A row: closes the menu, then does `f`.
    pub fn item(self: &Rc<Self>, c: &gtk::Box, glyph: &str, text: &str, shortcut: &str, class: &str, f: impl Fn() + 'static) -> gtk::Button {
        let cor = match class {
            "perigo" => "#EFA2A6",
            "agente" => "#A5A1E2",
            _ => "#E2E2E6",
        };
        let b = item_menu_completo(c, glyph, cor, text, shortcut, class);
        let p = self.pop.downgrade();
        b.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.popdown();
            }
            f();
        });
        b
    }

    /// A row that opens a submenu; returns the submenu's page (its first row goes back).
    pub fn sub(self: &Rc<Self>, c: &gtk::Box, glyph: &str, text: &str, name: &str) -> gtk::Box {
        // (the agent's submenus — its star — in the accent, as its other actions)
        let agente = glyph == "spark";
        let b = item_menu_completo(c, glyph, if agente { "#A5A1E2" } else { "#E2E2E6" }, text, "", if agente { "agente" } else { "" });
        if let Some(l) = b.child().and_downcast::<gtk::Box>() {
            let ch = ui_icon("seta-direita", "#7A7B88", 12);
            ch.set_margin_end(2);
            l.append(&ch);
        }
        let page = caixa(Orientation::Vertical, 2, "");
        page.set_size_request(WIDTH, -1);
        let voltar = item_menu_completo(&page, "seta-esquerda", "#8E8F99", text, "", "arq-menu-voltar");
        let pg = self.pages.downgrade();
        voltar.connect_clicked(move |_| {
            if let Some(p) = pg.upgrade() {
                p.set_visible_child_name("menu");
            }
        });
        self.pages.add_named(&page, Some(name));
        let (pg, n) = (self.pages.downgrade(), name.to_string());
        b.connect_clicked(move |_| {
            if let Some(p) = pg.upgrade() {
                p.set_visible_child_name(&n);
            }
        });
        crate::central::posicao_debug(&format!("menu {name}"), b.upcast_ref());
        page
    }

    pub fn divider(c: &gtk::Box) {
        let d = caixa(Orientation::Horizontal, 0, "arq-menu-divisor");
        d.set_size_request(-1, 1);
        c.append(&d);
    }

    pub fn show(&self) {
        self.pop.popup();
    }
}

/// The title bar's menu (design "Menus de contexto", BARRA DE TÍTULO) for a KalangoOS app's window: a right click on
/// its header (a window handle, not a field in it) opens it. Installed once per window.
pub fn title_bar(win: &gtk::Window) {
    if win.has_css_class("com-menu-titulo") {
        return;
    }
    win.add_css_class("com-menu-titulo");
    let g = gtk::GestureClick::new();
    g.set_button(3);
    g.set_propagation_phase(gtk::PropagationPhase::Capture);
    g.connect_pressed(|g, _, x, y| {
        let Some(w) = g.widget().and_downcast::<gtk::Window>() else { return };
        let Some(alvo) = w.pick(x, y, gtk::PickFlags::DEFAULT) else { return };
        let Some(cab) = alvo.downcast_ref::<gtk::WindowHandle>().map(|h| h.clone().upcast::<gtk::Widget>()).or_else(|| alvo.ancestor(gtk::WindowHandle::static_type())) else { return };
        // (a field in the header keeps its own menu)
        if alvo.is::<gtk::Text>() || alvo.ancestor(gtk::Text::static_type()).is_some() || alvo.ancestor(gtk::TextView::static_type()).is_some() {
            return;
        }
        g.set_state(gtk::EventSequenceState::Claimed);
        let (cx, cy) = w.compute_point(&cab, &gtk::graphene::Point::new(x as f32, y as f32)).map(|p| (f64::from(p.x()), f64::from(p.y()))).unwrap_or((x, y));
        show_title_menu(&w, &cab, cx, cy);
    });
    win.add_controller(g);
}

fn niri(acao: serde_json::Value) {
    let _ = crate::servicos::niri_acao(acao);
}

/// The focused window's niri id (the one right-clicked: the click focused it).
fn focused_id() -> Option<u64> {
    let v = crate::servicos::niri_pedido(serde_json::json!("FocusedWindow")).ok()?;
    v["FocusedWindow"]["id"].as_u64()
}

fn show_title_menu(w: &gtk::Window, cab: &gtk::Widget, x: f64, y: f64) {
    use kalango_i18n::{tr, trf};
    use serde_json::json;
    let m = Menu::at(cab, x, y);
    let c = m.main.clone();
    let id = focused_id();
    let ww = w.downgrade();
    m.item(&c, "minimizar", tr("Minimize"), "Super+H", "", move || {
        if let Some(w) = ww.upgrade() {
            w.minimize();
        }
    });
    let ww = w.downgrade();
    let max = w.is_maximized();
    m.item(&c, "maximizar", if max { tr("Restore") } else { tr("Maximize") }, "Super+↑", "", move || {
        if let Some(w) = ww.upgrade() {
            if w.is_maximized() { w.unmaximize() } else { w.maximize() }
        }
    });
    // snapped by KalangoOS's niri (thousandths of the working area)
    let enc = m.sub(&c, "colunas", tr("Snap"), "encaixar");
    for (nome, r) in [(tr("Left half"), "0,0,500,1000"), (tr("Right half"), "500,0,500,1000"), (tr("Top half"), "0,0,1000,500"), (tr("Bottom half"), "0,500,1000,500")] {
        let b = m.item(&enc, "nenhum", nome, "", "", move || {
            niri(json!({"SnapWindow": {"id": id, "region": r}}));
        });
        crate::central::posicao_debug(&format!("titulo encaixar {r}"), b.upcast_ref());
    }
    // the spaces of its monitor, and the other monitors
    let espacos = crate::servicos::niri_pedido(json!("Workspaces")).ok().and_then(|v| v["Workspaces"].as_array().cloned()).unwrap_or_default();
    let janelas = crate::servicos::niri_pedido(json!("Windows")).ok().and_then(|v| v["Windows"].as_array().cloned()).unwrap_or_default();
    let ws_atual = janelas.iter().find(|j| j["id"].as_u64() == id).and_then(|j| j["workspace_id"].as_u64());
    let saida = espacos.iter().find(|e| e["id"].as_u64() == ws_atual).and_then(|e| e["output"].as_str()).unwrap_or("").to_string();
    let mut desta: Vec<&serde_json::Value> = espacos.iter().filter(|e| e["output"].as_str() == Some(saida.as_str())).collect();
    desta.sort_by_key(|e| e["idx"].as_u64());
    let mov = m.sub(&c, "camadas", tr("Move to the space"), "mover-espaco");
    for e in desta {
        let idx = e["idx"].as_u64().unwrap_or(0);
        let nome = e["name"].as_str().map(str::to_string).unwrap_or_else(|| trf("Space {n}", &[("n", idx.to_string())]));
        let aqui = e["id"].as_u64() == ws_atual;
        let b = m.item(&mov, if aqui { "check" } else { "nenhum" }, &nome, "", "", move || {
            niri(json!({"MoveWindowToWorkspace": {"window_id": id, "reference": {"Index": idx}, "focus": true}}));
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug titulo: para o espaço {idx}");
            }
        });
        crate::central::posicao_debug(&format!("titulo espaco {idx}"), b.upcast_ref());
    }
    let mut saidas: Vec<String> = espacos.iter().filter_map(|e| e["output"].as_str().map(str::to_string)).filter(|o| *o != saida).collect();
    saidas.sort();
    saidas.dedup();
    for o in saidas {
        let nome = o.clone();
        m.item(&c, "monitor", &trf("Move to the monitor {name}", &[("name", o.clone())]), "", "", move || {
            if let Some(id) = id {
                niri(json!({"FocusWindow": {"id": id}}));
            }
            niri(json!({"MoveWindowToMonitor": {"output": nome}}));
        });
    }
    let topo = item_menu_completo(&c, "alfinete", "#E2E2E6", tr("Always on top"), "", "");
    topo.set_sensitive(false);
    topo.set_tooltip_text(Some(tr("The compositor does not keep windows on top yet")));
    Menu::divider(&c);
    let ww = w.downgrade();
    m.item(&c, "fechar", tr("Close"), "Super+Q", "perigo", move || {
        if let Some(w) = ww.upgrade() {
            w.close();
        }
    });
    m.show();
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug titulo: menu de {}", w.title().map(|t| t.to_string()).unwrap_or_default());
    }
}

/// The menu of selected text in a text view (design "Menus de contexto", TEXTO SELECIONADO): copy, paste, search it;
/// the agent explains it, rewrites it in place (shorter, clearer, more formal, corrected) or translates it in place;
/// and it is read out loud. A right click with nothing selected keeps the view's own menu.
pub fn text_selection(view: &gtk::TextView, shell: std::rc::Weak<crate::Shell>) {
    let g = gtk::GestureClick::new();
    g.set_button(3);
    g.set_propagation_phase(gtk::PropagationPhase::Capture);
    g.connect_pressed(move |g, _, x, y| {
        let Some(v) = g.widget().and_downcast::<gtk::TextView>() else { return };
        let b = v.buffer();
        let Some((ini, fim)) = b.selection_bounds() else { return };
        let sel = b.text(&ini, &fim, false).to_string();
        if sel.trim().is_empty() {
            return;
        }
        g.set_state(gtk::EventSequenceState::Claimed);
        show_text_menu(&v, &sel, &shell, x, y);
    });
    view.add_controller(g);
}

/// The selection replaced by what `kalango-agente colar-como <mode>` makes of it (one undo step).
fn rewrite(v: &gtk::TextView, mode: &'static str) {
    let b = v.buffer();
    let Some((ini, fim)) = b.selection_bounds() else { return };
    let text = b.text(&ini, &fim, false).to_string();
    // (marks keep the place while the model works: the person may type elsewhere meanwhile)
    let (ma, mb) = (b.create_mark(None, &ini, true), b.create_mark(None, &fim, false));
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug texto: {mode} ({} caracteres)", text.chars().count());
    }
    let bw = b.downgrade();
    crate::lateral::em_segundo_plano(move || {
        let bin = crate::voz::agent_binary();
        let mut c = std::process::Command::new(bin).args(["colar-como", mode]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
        if let Some(mut i) = c.stdin.take() {
            std::io::Write::write_all(&mut i, text.as_bytes()).map_err(|e| e.to_string())?;
        }
        let o = c.wait_with_output().map_err(|e| e.to_string())?;
        let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && !out.is_empty() { Ok(out) } else { Err(kalango_i18n::tr("The local model could not do it now.").to_string()) }
    }, move |r: Result<String, String>| {
        let Some(b) = bw.upgrade() else { return };
        match r {
            Ok(novo) => {
                let (mut i, mut f) = (b.iter_at_mark(&ma), b.iter_at_mark(&mb));
                b.begin_user_action();
                b.delete(&mut i, &mut f);
                let mut i = b.iter_at_mark(&ma);
                b.insert(&mut i, &novo);
                b.end_user_action();
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug texto: {mode} pronto ({} caracteres)", novo.chars().count());
                }
            }
            Err(e) => eprintln!("kalango-shell: {mode}: {e}"),
        }
        b.delete_mark(&ma);
        b.delete_mark(&mb);
    });
}

fn show_text_menu(v: &gtk::TextView, sel: &str, shell: &std::rc::Weak<crate::Shell>, x: f64, y: f64) {
    use kalango_i18n::{tr, trf};
    let m = Menu::at(v, x, y);
    let c = m.main.clone();
    let vw = v.downgrade();
    m.item(&c, "copiar", tr("Copy"), "Ctrl+C", "", move || {
        if let Some(v) = vw.upgrade() {
            v.buffer().copy_clipboard(&v.clipboard());
        }
    });
    let vw = v.downgrade();
    m.item(&c, "colar", tr("Paste"), "Ctrl+V", "", move || {
        if let Some(v) = vw.upgrade() {
            v.buffer().paste_clipboard(&v.clipboard(), None, v.is_editable());
        }
    });
    let curto: String = sel.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(24).collect();
    let (s, q) = (shell.clone(), sel.to_string());
    m.item(&c, "busca", &trf("Search “{text}”", &[("text", curto)]), "", "", move || {
        if let Some(s) = s.upgrade() {
            s.comando().abrir(&q);
        }
    });
    Menu::divider(&c);
    let (s, q) = (shell.clone(), sel.to_string());
    let b = m.item(&c, "spark", tr("Explain"), "", "agente", move || {
        if let Some(s) = s.upgrade() {
            s.right_bar().ask_agent(&trf("Explain this passage: «{text}»", &[("text", q.clone())]));
        }
    });
    crate::central::posicao_debug("texto explicar", b.upcast_ref());
    let editavel = v.is_editable();
    let re = m.sub(&c, "spark", tr("Rewrite"), "reescrever");
    for (nome, modo) in [(tr("Shorter"), "curto"), (tr("Clearer"), "claro"), (tr("More formal"), "formal"), (tr("Fix spelling and grammar"), "corrigir")] {
        let vw = v.downgrade();
        let b = m.item(&re, "spark", nome, "", "agente", move || {
            if let Some(v) = vw.upgrade() {
                rewrite(&v, modo);
            }
        });
        b.set_sensitive(editavel);
        crate::central::posicao_debug(&format!("texto reescrever {modo}"), b.upcast_ref());
    }
    let tr_ = m.sub(&c, "spark", tr("Translate"), "traduzir");
    for (nome, modo) in [(tr("Into English"), "traduzir-en"), (tr("Into Portuguese"), "traduzir-pt"), (tr("Into Spanish"), "traduzir-es")] {
        let vw = v.downgrade();
        let b = m.item(&tr_, "spark", nome, "", "agente", move || {
            if let Some(v) = vw.upgrade() {
                rewrite(&v, modo);
            }
        });
        b.set_sensitive(editavel);
        crate::central::posicao_debug(&format!("texto traduzir {modo}"), b.upcast_ref());
    }
    Menu::divider(&c);
    let q = sel.to_string();
    m.item(&c, "volume", tr("Read aloud"), "", "", move || crate::descrever::falar(&q));
    m.show();
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug texto: menu ({} caracteres)", sel.chars().count());
    }
}
