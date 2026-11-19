//! Configurações › Tela (design "Configurações · Tela"): the monitors as the compositor (niri) has them — drawn in
//! their arrangement, dragged to another place (on release a monitor snaps to the nearest edge of another), one only
//! or all of them —, and the chosen one's resolution, refresh rate, scale, orientation and variable refresh rate,
//! and whether the top bar and the dock live on it. A change of mode, scale or orientation asks "Manter estas
//! configurações?" and goes back by itself after 15 s, as a screen left unreadable could not be answered.
//!
//! niri applies these at once and forgets them when it starts again: the shell keeps them (telas.json) and applies
//! them again when the session starts. What niri cannot do is said so: mirroring the screens and HDR.

use crate::configuracoes::{depurando, interruptor};
use crate::configuracoes_moldura::{linha_interruptor, Moldura};
use crate::conversa::ui_icon;
use crate::servicos::niri_pedido;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const ABAS: &[(&str, &str)] = &[("monitores", n_("Monitors")), ("noturna", n_("Night light")), ("cor", n_("Colour"))];

#[derive(Clone, Debug, PartialEq)]
pub struct Modo {
    pub w: u32,
    pub h: u32,
    /// millihertz
    pub mhz: u32,
    pub preferido: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Saida {
    pub nome: String,
    pub fabricante: String,
    pub modelo: String,
    pub serie: String,
    pub fisico: Option<(u32, u32)>,
    pub modos: Vec<Modo>,
    pub atual: Option<usize>,
    pub vrr_suportado: bool,
    pub vrr: bool,
    /// x, y, width, height (logical), scale, transform; none: off
    pub logico: Option<(i32, i32, i32, i32, f64, String)>,
}

impl Saida {
    /// Who it is across plugging and unplugging (its connector may change).
    pub fn id(&self) -> String {
        format!("{} {} {}", self.fabricante, self.modelo, self.serie).trim().to_string()
    }

    /// "MSI 34″ ultrawide": the maker, the diagonal from the physical size, and the shape.
    pub fn descricao(&self) -> String {
        let mut d = if self.fabricante.is_empty() { self.modelo.clone() } else { self.fabricante.clone() };
        if let Some((w, h)) = self.fisico.filter(|(w, h)| *w > 0 && *h > 0) {
            let pol = ((f64::from(w * w + h * h)).sqrt() / 25.4).round();
            d += &format!(" {pol}″");
            if f64::from(w) / f64::from(h) > 2.0 {
                d += &format!(" {}", tr("ultrawide"));
            }
        }
        d
    }
}

/// niri's outputs (`"Outputs"`), the stable order: by position, then by name.
pub fn ler_saidas(v: &Value) -> Vec<Saida> {
    let mut l: Vec<Saida> = v.as_object().map(|o| o.values().filter_map(|s| {
        let modos = s["modes"].as_array()?.iter().map(|m| Modo {
            w: m["width"].as_u64().unwrap_or(0) as u32,
            h: m["height"].as_u64().unwrap_or(0) as u32,
            mhz: m["refresh_rate"].as_u64().unwrap_or(0) as u32,
            preferido: m["is_preferred"].as_bool().unwrap_or(false),
        }).collect();
        let lg = &s["logical"];
        Some(Saida {
            nome: s["name"].as_str()?.to_string(),
            fabricante: s["make"].as_str().unwrap_or("").to_string(),
            modelo: s["model"].as_str().unwrap_or("").to_string(),
            serie: s["serial"].as_str().unwrap_or("").to_string(),
            fisico: s["physical_size"].as_array().and_then(|a| Some((a.first()?.as_u64()? as u32, a.get(1)?.as_u64()? as u32))),
            modos,
            atual: s["current_mode"].as_u64().map(|i| i as usize),
            vrr_suportado: s["vrr_supported"].as_bool().unwrap_or(false),
            vrr: s["vrr_enabled"].as_bool().unwrap_or(false),
            logico: lg.is_object().then(|| (lg["x"].as_i64().unwrap_or(0) as i32, lg["y"].as_i64().unwrap_or(0) as i32, lg["width"].as_i64().unwrap_or(0) as i32,
                lg["height"].as_i64().unwrap_or(0) as i32, lg["scale"].as_f64().unwrap_or(1.0), lg["transform"].as_str().unwrap_or("Normal").to_string())),
        })
    }).collect()).unwrap_or_default();
    l.sort_by_key(|s| (s.logico.as_ref().map(|g| (g.0, g.1)).unwrap_or((i32::MAX, 0)), s.nome.clone()));
    l
}

fn saidas() -> Vec<Saida> {
    niri_pedido(json!("Outputs")).map(|v| ler_saidas(&v["Outputs"])).unwrap_or_default()
}

/// A request to change an output (niri's `Output` request).
fn mudar(nome: &str, acao: Value) -> Result<(), String> {
    let r = niri_pedido(json!({"Output": {"output": nome, "action": acao}}))?;
    if r["OutputConfigChanged"] == "OutputWasMissing" {
        return Err(tr("This monitor is no longer connected").into());
    }
    Ok(())
}

fn acao_modo(m: &Modo) -> Value {
    json!({"Mode": {"mode": {"Specific": {"width": m.w, "height": m.h, "refresh": f64::from(m.mhz) / 1000.0}}}})
}

/// A dragged monitor's place on release: snapped to the nearest edge of another (sides touching, tops or bottoms
/// lined up when close), so the screens have no gap between them.
pub fn encaixar(r: (i32, i32, i32, i32), outros: &[(i32, i32, i32, i32)]) -> (i32, i32) {
    let (mut x, mut y, w, h) = r;
    let mut melhor = i64::MAX;
    let mut alvo = (x, y);
    for &(ox, oy, ow, oh) in outros {
        // touching on the left or the right, the top lined up with the other's (or as dropped, within reach)
        for (nx, ny) in [(ox + ow, y), (ox - w, y), (x, oy + oh), (x, oy - h)] {
            let d = i64::from(nx - x).pow(2) + i64::from(ny - y).pow(2);
            if d < melhor {
                melhor = d;
                alvo = (nx, ny);
            }
        }
    }
    if melhor == i64::MAX {
        return (0, 0);
    }
    (x, y) = alvo;
    // the other axis: lined up with an edge of the other when close (64 px)
    for &(ox, oy, ow, oh) in outros {
        if x == ox + ow || x + w == ox {
            for a in [oy, oy + oh - h] {
                if (y - a).abs() <= 64 {
                    y = a;
                }
            }
        } else if y == oy + oh || y + h == oy {
            for a in [ox, ox + ow - w] {
                if (x - a).abs() <= 64 {
                    x = a;
                }
            }
        }
    }
    (x, y)
}

// ------------------------------------------------------------ kept settings (applied again when the session starts)

fn telas_path() -> std::path::PathBuf {
    glib::user_config_dir().join("kalango/telas.json")
}

fn guardadas() -> Value {
    std::fs::read(telas_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

fn guardar(f: impl FnOnce(&mut Value)) {
    let mut v = guardadas();
    f(&mut v);
    if let Some(d) = telas_path().parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(telas_path(), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Keeps how an output is now.
fn guardar_saida(s: &Saida) {
    let modo = s.atual.and_then(|i| s.modos.get(i)).map(|m| json!({"w": m.w, "h": m.h, "mhz": m.mhz}));
    let v = match &s.logico {
        Some((x, y, _, _, escala, tf)) => json!({"ligada": true, "modo": modo, "x": x, "y": y, "escala": escala, "transform": tf, "vrr": s.vrr}),
        None => json!({"ligada": false}),
    };
    let id = s.id();
    guardar(move |t| t["saidas"][id] = v);
}

/// At the session's start: each known output as the person left it (in the order that cannot overlap: modes and
/// scales first, then the places).
pub fn reaplicar() {
    std::thread::spawn(|| {
        let g = guardadas();
        let lista = saidas();
        for s in &lista {
            let k = &g["saidas"][s.id()];
            if !k.is_object() {
                continue;
            }
            if k["ligada"] == false {
                let _ = mudar(&s.nome, json!("Off"));
                continue;
            }
            if let Some(m) = k["modo"].as_object() {
                let alvo = Modo { w: m["w"].as_u64().unwrap_or(0) as u32, h: m["h"].as_u64().unwrap_or(0) as u32, mhz: m["mhz"].as_u64().unwrap_or(0) as u32, preferido: false };
                if s.modos.iter().any(|x| x.w == alvo.w && x.h == alvo.h && x.mhz == alvo.mhz) {
                    let _ = mudar(&s.nome, acao_modo(&alvo));
                }
            }
            if let Some(e) = k["escala"].as_f64() {
                let _ = mudar(&s.nome, json!({"Scale": {"scale": {"Specific": e}}}));
            }
            if let Some(t) = k["transform"].as_str() {
                let _ = mudar(&s.nome, json!({"Transform": {"transform": t}}));
            }
            if s.vrr_suportado {
                let _ = mudar(&s.nome, json!({"Vrr": {"vrr": {"vrr": k["vrr"].as_bool().unwrap_or(false), "on_demand": false}}}));
            }
        }
        for s in &lista {
            let k = &g["saidas"][s.id()];
            if let (Some(x), Some(y)) = (k["x"].as_i64(), k["y"].as_i64()) {
                let _ = mudar(&s.nome, json!({"Position": {"position": {"Specific": {"x": x, "y": y}}}}));
            }
        }
    });
}

/// The monitor the top bar and the dock live on (the person's choice; the first otherwise).
pub fn monitor_principal() -> Option<gdk::Monitor> {
    let d = gdk::Display::default()?;
    let ms = d.monitors();
    let todos: Vec<gdk::Monitor> = (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()).collect();
    let escolhido = guardadas()["principal"].as_str().map(str::to_string);
    escolhido.and_then(|c| todos.iter().find(|m| m.connector().is_some_and(|x| x == c)).cloned()).or_else(|| todos.first().cloned())
}

/// The top bar and the dock on the main monitor.
pub fn posicionar_barra_e_dock(shell: &crate::Shell) {
    use gtk4_layer_shell::LayerShell;
    let m = monitor_principal();
    for w in [shell.barra.borrow().clone(), shell.dock.borrow().clone()].into_iter().flatten() {
        w.set_monitor(m.as_ref());
    }
}

// ------------------------------------------------------------ the page

pub struct Tela {
    pub moldura: Rc<Moldura>,
    corpo: gtk::Stack,
    mesa: gtk::DrawingArea,
    arranjo: Vec<(&'static str, gtk::ToggleButton)>,
    so_um: gtk::Box,
    sugestao: gtk::Box,
    sugestao_txt: gtk::Label,
    manter: gtk::Box,
    manter_txt: gtk::Label,
    p_rotulo: gtk::Label,
    p_resolucao: gtk::DropDown,
    p_taxa: gtk::DropDown,
    p_escalas: Vec<(f64, gtk::ToggleButton)>,
    p_orientacao: gtk::DropDown,
    p_vrr: gtk::Switch,
    p_vrr_linha: gtk::Box,
    p_principal: gtk::Switch,
    p_erro: gtk::Label,
    // night light tab
    n_agora: gtk::Switch,
    n_agenda: gtk::Switch,
    n_temp: gtk::Scale,
    n_temp_txt: gtk::Label,
    n_aviso: gtk::Label,
    lista: RefCell<Vec<Saida>>,
    escolhida: RefCell<Option<String>>,
    /// a monitor being dragged on the board: its name, the pointer's start, its logical place at the start
    arrasto: RefCell<Option<(String, (f64, f64), (i32, i32))>>,
    /// where it is while dragged (logical)
    arrastando: Cell<Option<(i32, i32)>>,
    /// the change waiting for "Manter": how to undo it, and the countdown
    desfazer: RefCell<Option<(Vec<(String, Value)>, glib::SourceId)>>,
    segundos: Cell<u32>,
    lendo: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
}

fn esquerda(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

/// The design's option line in the panel: the name at the left, its control at the right (40 px, 10 px in).
fn linha_opcao(nome: &str, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = esquerda(nome, "cfg-texto-13");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn seletor(itens: &[String]) -> gtk::DropDown {
    let lista = gtk::StringList::new(&itens.iter().map(String::as_str).collect::<Vec<_>>());
    let d = gtk::DropDown::new(Some(lista), None::<gtk::Expression>);
    d.add_css_class("cfg-seletor");
    d.set_size_request(230, 34);
    d
}

fn trocar_itens(d: &gtk::DropDown, itens: &[String]) {
    let lista = gtk::StringList::new(&itens.iter().map(String::as_str).collect::<Vec<_>>());
    d.set_model(Some(&lista));
}

/// Orientations: (niri's transform, name).
fn orientacoes() -> [(&'static str, &'static str); 4] {
    [("Normal", tr("Landscape")), ("90", tr("Portrait")), ("180", tr("Landscape (flipped)")), ("270", tr("Portrait (flipped)"))]
}

impl Tela {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<Tela> {
        let m = Moldura::new(tr("Display"), ABAS, tr("Selected monitor"));
        let corpo = gtk::Stack::new();
        corpo.set_vhomogeneous(false);
        corpo.set_hhomogeneous(false);
        m.corpo.append(&corpo);
        // ---- monitors: the board
        let mon = caixa(Orientation::Vertical, 16, "");
        let cd = caixa(Orientation::Vertical, 12, "cfg-cartao");
        let r = caixa(Orientation::Horizontal, 6, "");
        r.append(&ui_icon("monitor", "#8E8F99", 16));
        r.append(&rotulo(tr("MONITOR ARRANGEMENT"), "cfg-rotulo"));
        cd.append(&r);
        let mesa = gtk::DrawingArea::new();
        mesa.set_content_height(300);
        mesa.set_hexpand(true);
        mesa.add_css_class("cfg-tela-mesa");
        cd.append(&mesa);
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.add_css_class("escuro");
        seg.set_homogeneous(true);
        let mut arranjo: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("estender", tr("Extend")), ("espelhar", tr("Mirror"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = arranjo.first() {
                b.set_group(Some(p));
            }
            seg.append(&b);
            arranjo.push((id, b));
        }
        // (niri shows each screen on its own: mirroring is not something it does)
        arranjo[1].1.set_sensitive(false);
        arranjo[1].1.set_tooltip_text(Some(tr("The compositor does not mirror screens yet")));
        let so_um = caixa(Orientation::Horizontal, 2, "");
        so_um.set_homogeneous(true);
        seg.append(&so_um);
        cd.append(&seg);
        mon.append(&cd);
        let manter = caixa(Orientation::Horizontal, 10, "cfg-tela-aviso");
        manter.append(&ui_icon("monitor", "#A5A1E2", 16));
        let manter_txt = esquerda("", "cfg-texto");
        manter_txt.set_hexpand(true);
        manter.append(&manter_txt);
        let ficar = gtk::Button::with_label(tr("Keep"));
        ficar.add_css_class("cfg-botao-principal");
        ficar.add_css_class("pequeno");
        manter.append(&ficar);
        let voltar = gtk::Button::with_label(tr("Undo"));
        voltar.add_css_class("cfg-botao-texto");
        manter.append(&voltar);
        manter.set_visible(false);
        // (at the top of the page: after a new scale the window may reach past the screen, and the question must be in
        // sight to be answered)
        mon.prepend(&manter);
        let sugestao = caixa(Orientation::Horizontal, 10, "cfg-tela-aviso");
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_size_request(16, 16);
        sp.set_valign(Align::Center);
        let est = estrela(13, "#A5A1E2");
        est.set_hexpand(true);
        sp.append(&est);
        sugestao.append(&sp);
        let sugestao_txt = esquerda("", "cfg-texto");
        sugestao_txt.set_hexpand(true);
        sugestao_txt.set_wrap(true);
        sugestao.append(&sugestao_txt);
        let aplicar = gtk::Button::with_label(tr("Apply"));
        aplicar.add_css_class("cfg-botao-principal");
        aplicar.add_css_class("pequeno");
        sugestao.append(&aplicar);
        let dispensar = gtk::Button::with_label(tr("Dismiss"));
        dispensar.add_css_class("cfg-botao-texto");
        sugestao.append(&dispensar);
        sugestao.set_visible(false);
        mon.append(&sugestao);
        corpo.add_named(&mon, Some("monitores"));
        // ---- night light
        let nc = caixa(Orientation::Vertical, 12, "cfg-cartao");
        nc.set_valign(Align::Start);
        let r = caixa(Orientation::Horizontal, 6, "");
        r.append(&ui_icon("noite-d", "#8E8F99", 16));
        r.append(&rotulo(tr("NIGHT LIGHT"), "cfg-rotulo"));
        nc.append(&r);
        let (l, n_agora) = linha_interruptor(tr("On now"));
        nc.append(&l);
        let (l, n_agenda) = linha_interruptor(tr("Every night, from 20h to 6h"));
        nc.append(&l);
        let tl = caixa(Orientation::Horizontal, 0, "");
        let t = esquerda(tr("Warmth"), "cfg-texto");
        t.set_hexpand(true);
        tl.append(&t);
        let n_temp_txt = rotulo("", "cfg-valor-pequeno");
        tl.append(&n_temp_txt);
        nc.append(&tl);
        // (the colour temperature: lower is warmer)
        let n_temp = gtk::Scale::with_range(Orientation::Horizontal, 2500.0, 5500.0, 100.0);
        n_temp.add_css_class("cfg-escala");
        n_temp.set_draw_value(false);
        n_temp.set_inverted(true);
        nc.append(&n_temp);
        let n_aviso = esquerda("", "cfg-sub");
        n_aviso.set_wrap(true);
        nc.append(&n_aviso);
        corpo.add_named(&nc, Some("noturna"));
        // ---- colour
        let cc = caixa(Orientation::Vertical, 10, "cfg-cartao");
        cc.set_valign(Align::Start);
        let r = caixa(Orientation::Horizontal, 6, "");
        r.append(&ui_icon("paleta", "#8E8F99", 16));
        r.append(&rotulo(tr("COLOUR PROFILES"), "cfg-rotulo"));
        cc.append(&r);
        let ct = esquerda(tr("The compositor shows colours as each monitor sends them: colour profiles (ICC) and HDR are not applied yet. The night light warms the colours at night."), "cfg-sub");
        ct.set_wrap(true);
        cc.append(&ct);
        corpo.add_named(&cc, Some("cor"));
        // ---- the panel
        let r = caixa(Orientation::Horizontal, 6, "");
        r.append(&ui_icon("monitor", "#8E8F99", 16));
        let p_rotulo = rotulo("", "cfg-rotulo");
        r.append(&p_rotulo);
        m.painel.set_spacing(4);
        m.painel.append(&r);
        let p_resolucao = seletor(&[]);
        m.painel.append(&linha_opcao(tr("Resolution"), &p_resolucao));
        let p_taxa = seletor(&[]);
        m.painel.append(&linha_opcao(tr("Refresh rate"), &p_taxa));
        let esc = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        esc.add_css_class("escuro");
        esc.set_size_request(256, 34);
        esc.set_homogeneous(true);
        let mut p_escalas: Vec<(f64, gtk::ToggleButton)> = Vec::new();
        for e in [1.0, 1.25, 1.5, 2.0] {
            let b = gtk::ToggleButton::with_label(&format!("{}%", (e * 100.0) as i32));
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = p_escalas.first() {
                b.set_group(Some(p));
            }
            esc.append(&b);
            p_escalas.push((e, b));
        }
        m.painel.append(&linha_opcao(tr("Scale"), &esc));
        let p_orientacao = seletor(&orientacoes().iter().map(|(_, n)| n.to_string()).collect::<Vec<_>>());
        m.painel.append(&linha_opcao(tr("Orientation"), &p_orientacao));
        let div = caixa(Orientation::Horizontal, 0, "cfg-div");
        div.set_size_request(-1, 1);
        div.set_margin_top(4);
        div.set_margin_bottom(4);
        m.painel.append(&div);
        let p_vrr_linha = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
        let vt = caixa(Orientation::Vertical, 1, "");
        vt.set_hexpand(true);
        vt.set_valign(Align::Center);
        vt.append(&esquerda(tr("Variable refresh rate (VRR)"), "cfg-texto-13"));
        vt.append(&esquerda(tr("avoids tearing in games and video"), "cfg-sub"));
        p_vrr_linha.append(&vt);
        let p_vrr = interruptor();
        p_vrr_linha.append(&p_vrr);
        m.painel.append(&p_vrr_linha);
        let p_principal = interruptor();
        m.painel.append(&linha_opcao(tr("Bar and dock on this monitor"), &p_principal));
        let p_erro = esquerda("", "cfg-erro");
        p_erro.set_wrap(true);
        p_erro.set_visible(false);
        m.painel.append(&p_erro);
        let t = Rc::new(Tela { moldura: m, corpo, mesa, arranjo, so_um, sugestao, sugestao_txt, manter, manter_txt, p_rotulo, p_resolucao, p_taxa, p_escalas, p_orientacao,
            p_vrr, p_vrr_linha, p_principal, p_erro, n_agora, n_agenda, n_temp, n_temp_txt, n_aviso, lista: RefCell::default(), escolhida: RefCell::default(),
            arrasto: RefCell::default(), arrastando: Cell::new(None), desfazer: RefCell::default(), segundos: Cell::new(0), lendo: Cell::new(false), timer: RefCell::default() });
        t.ligar(c, &ficar, &voltar, &aplicar, &dispensar);
        t
    }

    fn ligar(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>, ficar: &gtk::Button, voltar: &gtk::Button, aplicar: &gtk::Button, dispensar: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.moldura.ao_trocar_aba(move |id| if let Some(t) = w.upgrade() { t.aba(id) });
        // the board: drawn, clicked to choose, dragged to move
        let w = Rc::downgrade(self);
        self.mesa.set_draw_func(move |_, cr, w_, h_| if let Some(t) = w.upgrade() { t.desenhar_mesa(cr, f64::from(w_), f64::from(h_)) });
        let arrastar = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        arrastar.connect_drag_begin(move |_, x, y| {
            let Some(t) = w.upgrade() else { return };
            if let Some(s) = t.saida_em(x, y) {
                *t.escolhida.borrow_mut() = Some(s.nome.clone());
                if let Some((lx, ly, ..)) = s.logico {
                    *t.arrasto.borrow_mut() = Some((s.nome.clone(), (x, y), (lx, ly)));
                }
                t.painel_render();
                t.mesa.queue_draw();
            }
        });
        let w = Rc::downgrade(self);
        arrastar.connect_drag_update(move |_, dx, dy| {
            let Some(t) = w.upgrade() else { return };
            let Some((_, _, (lx, ly))) = t.arrasto.borrow().clone() else { return };
            let k = t.escala_mesa();
            t.arrastando.set(Some((lx + (dx / k) as i32, ly + (dy / k) as i32)));
            t.mesa.queue_draw();
        });
        let w = Rc::downgrade(self);
        arrastar.connect_drag_end(move |_, dx, dy| {
            let Some(t) = w.upgrade() else { return };
            let Some((nome, _, (lx, ly))) = t.arrasto.borrow_mut().take() else { return };
            t.arrastando.set(None);
            if dx.abs() + dy.abs() < 4.0 {
                t.mesa.queue_draw();
                return;
            }
            let k = t.escala_mesa();
            t.mover(&nome, (lx + (dx / k) as i32, ly + (dy / k) as i32));
        });
        self.mesa.add_controller(arrastar);
        let w = Rc::downgrade(self);
        self.arranjo[0].1.connect_toggled(move |b| {
            if b.is_active() {
                if let Some(t) = w.upgrade().filter(|t| !t.lendo.get()) {
                    t.so_o_monitor(None);
                }
            }
        });
        // the chosen monitor
        let w = Rc::downgrade(self);
        self.p_resolucao.connect_selected_notify(move |d| if let Some(t) = w.upgrade() { t.resolucao(d.selected()) });
        let w = Rc::downgrade(self);
        self.p_taxa.connect_selected_notify(move |d| if let Some(t) = w.upgrade() { t.taxa(d.selected()) });
        for (e, b) in &self.p_escalas {
            let (w, e) = (Rc::downgrade(self), *e);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(t) = w.upgrade() {
                        t.com_desfazer(json!({"Scale": {"scale": {"Specific": e}}}), "escala");
                    }
                }
            });
        }
        let w = Rc::downgrade(self);
        self.p_orientacao.connect_selected_notify(move |d| {
            let Some(t) = w.upgrade() else { return };
            let Some((tf, _)) = orientacoes().get(d.selected() as usize).copied() else { return };
            t.com_desfazer(json!({"Transform": {"transform": tf}}), "orientação");
        });
        let w = Rc::downgrade(self);
        self.p_vrr.connect_state_set(move |_, on| {
            if let Some(t) = w.upgrade().filter(|t| !t.lendo.get()) {
                t.aplicar(vec![(t.escolhida_nome(), json!({"Vrr": {"vrr": {"vrr": on, "on_demand": false}}}))], "vrr");
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.p_principal.connect_state_set(move |_, on| {
            if let Some(t) = w.upgrade().filter(|t| !t.lendo.get()) {
                if on {
                    let nome = t.escolhida_nome();
                    guardar(|v| v["principal"] = json!(nome));
                    if let Some(s) = crate::shell_atual() {
                        posicionar_barra_e_dock(&s);
                    }
                    if depurando() {
                        eprintln!("debug tela: barra e dock em {}", t.escolhida_nome());
                    }
                    t.mesa.queue_draw();
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        ficar.connect_clicked(move |_| if let Some(t) = w.upgrade() { t.manter_agora() });
        let w = Rc::downgrade(self);
        voltar.connect_clicked(move |_| if let Some(t) = w.upgrade() { t.voltar_agora() });
        let w = Rc::downgrade(self);
        aplicar.connect_clicked(move |_| {
            let Some(t) = w.upgrade() else { return };
            if let Some(nome) = t.sugerida() {
                *t.escolhida.borrow_mut() = Some(nome.clone());
                t.sugestao.set_visible(false);
                t.aplicar_com_desfazer(&nome, json!({"Scale": {"scale": {"Specific": 1.25}}}), "escala sugerida");
            }
        });
        let w = Rc::downgrade(self);
        dispensar.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                if let Some(n) = t.sugerida() {
                    guardar(|v| v["sugestao_dispensada"] = json!(n));
                }
                t.sugestao.set_visible(false);
            }
        });
        // night light
        let w = Rc::downgrade(self);
        self.n_agora.connect_state_set(move |_, on| {
            if let Some(t) = w.upgrade().filter(|t| !t.lendo.get()) {
                let temp = t.n_temp.value() as u32;
                std::thread::spawn(move || crate::sistema::luz_noturna_com(on, temp));
                let w = Rc::downgrade(&t);
                glib::timeout_add_local_once(Duration::from_millis(900), move || if let Some(t) = w.upgrade() { t.noturna_render() });
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.n_agenda.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|t| !t.lendo.get()) {
                crate::configuracoes::salvar_ajuste("luz_noturna_auto", json!(on));
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.n_temp.connect_value_changed(move |s| {
            let Some(t) = w.upgrade() else { return };
            let v = s.value() as u32;
            t.n_temp_txt.set_text(&format!("{v} K"));
            if !t.lendo.get() {
                crate::configuracoes::salvar_ajuste("luz_noturna_temperatura", json!(v));
                if t.n_agora.is_active() {
                    std::thread::spawn(move || crate::sistema::luz_noturna_com(true, v));
                }
            }
        });
        for (n, wdg) in [("cfg tela estender", self.arranjo[0].1.upcast_ref::<gtk::Widget>()), ("cfg tela mesa", self.mesa.upcast_ref()),
            ("cfg tela vrr", self.p_vrr.upcast_ref()), ("cfg tela principal", self.p_principal.upcast_ref())] {
            crate::configuracoes::registrar_posicao(c, n, wdg);
        }
        for (e, b) in &self.p_escalas {
            crate::configuracoes::registrar_posicao(c, &format!("cfg tela escala {}", (e * 100.0) as i32), b.upcast_ref());
        }
        crate::configuracoes::registrar_posicao(c, "cfg tela manter", ficar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg tela desfazer", voltar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg tela aplicar", aplicar.upcast_ref());
    }

    pub fn compactar(&self, c: bool) {
        self.moldura.compactar(c);
    }

    fn aba(self: &Rc<Self>, id: &str) {
        self.corpo.set_visible_child_name(id);
        if id == "noturna" {
            self.noturna_render();
        }
    }

    pub fn mostrar(self: &Rc<Self>) {
        self.atualizar();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(t) = w.upgrade() else { return glib::ControlFlow::Break };
            if !t.moldura.raiz.is_mapped() {
                t.timer.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            if t.arrasto.borrow().is_none() {
                t.atualizar();
            }
            glib::ControlFlow::Continue
        }));
    }

    fn atualizar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(saidas, move |l| {
            if let Some(t) = w.upgrade() {
                t.mostrar_saidas(l);
            }
        });
    }

    fn mostrar_saidas(self: &Rc<Self>, l: Vec<Saida>) {
        let mudou = *self.lista.borrow() != l;
        let ligadas = l.iter().filter(|s| s.logico.is_some()).count();
        self.moldura.sub.set_text(&match (l.len(), ligadas) {
            (0, _) => tr("No monitor found").to_string(),
            (1, _) => tr("1 monitor").to_string(),
            (n, k) if k == n => trf("{n} monitors · extended area", &[("n", n.to_string())]),
            (n, _) => trf("{n} monitors · one showing", &[("n", n.to_string())]),
        });
        if self.escolhida.borrow().as_ref().is_none_or(|e| !l.iter().any(|s| s.nome == *e)) {
            *self.escolhida.borrow_mut() = l.first().map(|s| s.nome.clone());
        }
        // "Só o monitor N", one a monitor
        if mudou {
            let antes = self.lista.borrow().len();
            if antes != l.len() || self.so_um.first_child().is_none() {
                while let Some(x) = self.so_um.first_child() {
                    self.so_um.remove(&x);
                }
                for (i, s) in l.iter().enumerate() {
                    let b = gtk::ToggleButton::with_label(&trf("Only monitor {n}", &[("n", (i + 1).to_string())]));
                    b.add_css_class("cfg-seg");
                    b.set_group(Some(&self.arranjo[0].1));
                    b.set_widget_name(&s.nome);
                    let (w, nome) = (Rc::downgrade(self), s.nome.clone());
                    b.connect_toggled(move |b| {
                        if b.is_active() {
                            if let Some(t) = w.upgrade().filter(|t| !t.lendo.get()) {
                                t.so_o_monitor(Some(&nome));
                            }
                        }
                    });
                    self.so_um.append(&b);
                    if depurando() {
                        crate::central::posicao_debug(&format!("cfg tela so {}", i + 1), b.upcast_ref());
                    }
                }
            }
        }
        *self.lista.borrow_mut() = l;
        self.lendo.set(true);
        let todas = { let l = self.lista.borrow(); l.iter().all(|s| s.logico.is_some()) };
        if todas {
            self.arranjo[0].1.set_active(true);
        } else {
            let mut x = self.so_um.first_child();
            while let Some(b) = x {
                if let Some(b) = b.downcast_ref::<gtk::ToggleButton>() {
                    let ligada = self.lista.borrow().iter().any(|s| s.nome == b.widget_name() && s.logico.is_some());
                    if ligada {
                        b.set_active(true);
                    }
                }
                x = b.next_sibling();
            }
        }
        self.lendo.set(false);
        self.painel_render();
        self.sugestao_render();
        self.mesa.queue_draw();
        if depurando() && mudou {
            let l = self.lista.borrow();
            eprintln!("debug tela: {}", l.iter().map(|s| match &s.logico {
                Some((x, y, w, h, e, tf)) => format!("{} {w}x{h}+{x}+{y} escala {e} {tf}", s.nome),
                None => format!("{} desligada", s.nome),
            }).collect::<Vec<_>>().join("; "));
        }
    }

    fn escolhida_nome(&self) -> String {
        self.escolhida.borrow().clone().unwrap_or_default()
    }

    fn escolhida(&self) -> Option<Saida> {
        let e = self.escolhida.borrow().clone()?;
        self.lista.borrow().iter().find(|s| s.nome == e).cloned()
    }

    // ------------------------------------------------------------ the board

    /// The scale from logical pixels to the board (all the monitors fit with a margin).
    fn escala_mesa(&self) -> f64 {
        let (x0, y0, x1, y1) = self.limites();
        let (w, h) = (f64::from(self.mesa.width().max(1)), f64::from(self.mesa.height().max(1)));
        ((w - 120.0) / f64::from((x1 - x0).max(1))).min((h - 110.0) / f64::from((y1 - y0).max(1))).max(0.01)
    }

    fn limites(&self) -> (i32, i32, i32, i32) {
        let l = self.lista.borrow();
        let g: Vec<_> = l.iter().filter_map(|s| s.logico.as_ref().map(|g| (g.0, g.1, g.0 + g.2, g.1 + g.3))).collect();
        if g.is_empty() {
            return (0, 0, 1920, 1080);
        }
        (g.iter().map(|r| r.0).min().unwrap(), g.iter().map(|r| r.1).min().unwrap(), g.iter().map(|r| r.2).max().unwrap(), g.iter().map(|r| r.3).max().unwrap())
    }

    /// A monitor's rectangle on the board (the dragged one where it is being dragged).
    fn retangulo(&self, s: &Saida) -> Option<(f64, f64, f64, f64)> {
        let (x, y, w, h, ..) = s.logico.clone()?;
        let (x, y) = match (self.arrasto.borrow().as_ref(), self.arrastando.get()) {
            (Some((n, _, _)), Some(p)) if *n == s.nome => p,
            _ => (x, y),
        };
        let (x0, y0, x1, y1) = self.limites();
        let k = self.escala_mesa();
        let (mw, mh) = (f64::from(self.mesa.width()), f64::from(self.mesa.height()));
        let ox = (mw - f64::from(x1 - x0) * k) / 2.0 - f64::from(x0) * k;
        let oy = (mh - 30.0 - f64::from(y1 - y0) * k) / 2.0 - f64::from(y0) * k;
        Some((ox + f64::from(x) * k, oy + f64::from(y) * k, f64::from(w) * k, f64::from(h) * k))
    }

    fn saida_em(&self, px: f64, py: f64) -> Option<Saida> {
        let l = self.lista.borrow();
        l.iter().find(|s| self.retangulo(s).is_some_and(|(x, y, w, h)| px >= x && px <= x + w && py >= y && py <= y + h)).cloned()
    }

    fn desenhar_mesa(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let rr = |x: f64, y: f64, w: f64, h: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
        };
        let cor = |hex: &str, a: f64| {
            let (r, g, b) = crate::tema::rgb(hex);
            cr.set_source_rgba(r, g, b, a);
        };
        rr(0.5, 0.5, w - 1.0, h - 1.0, 4.0);
        cor("#101114", 1.0);
        let _ = cr.fill_preserve();
        cor("#26272F", 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let principal = monitor_principal().and_then(|m| m.connector()).map(|c| c.to_string());
        let escolhida = self.escolhida_nome();
        let fonte = |cr: &gtk::cairo::Context, familia: &str, peso: gtk::cairo::FontWeight, px: f64| {
            cr.select_font_face(familia, gtk::cairo::FontSlant::Normal, peso);
            cr.set_font_size(px);
        };
        for (i, s) in self.lista.borrow().iter().enumerate() {
            let Some((x, y, rw, rh)) = self.retangulo(s) else { continue };
            let sel = s.nome == escolhida;
            rr(x + 1.0, y + 1.0, rw - 2.0, rh - 2.0, 4.0);
            if sel {
                cor(crate::tema::ACENTO, 0.16);
                let _ = cr.fill_preserve();
                cor(crate::tema::ACENTO, 1.0);
                cr.set_line_width(2.0);
            } else {
                cor("#FFFFFF", 0.05);
                let _ = cr.fill_preserve();
                cor("#FFFFFF", 0.5);
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();
            // its number, and on the main one what lives there
            let n = (i + 1).to_string();
            fonte(cr, crate::tema::fonte_display(), gtk::cairo::FontWeight::Bold, if sel { 34.0 } else { 28.0 });
            if let Ok(e) = cr.text_extents(&n) {
                cor(if sel { crate::tema::ACENTO } else { "#8E8F99" }, 1.0);
                cr.move_to(x + rw / 2.0 - e.width() / 2.0 - e.x_bearing(), y + rh / 2.0 + e.height() / 2.0 - 6.0);
                let _ = cr.show_text(&n);
            }
            if principal.as_deref() == Some(s.nome.as_str()) && rw > 130.0 {
                let t = tr("main · bar and dock");
                fonte(cr, crate::tema::fonte_ui(), gtk::cairo::FontWeight::Normal, 11.0);
                if let Ok(e) = cr.text_extents(t) {
                    cor("#8E8F99", 1.0);
                    cr.move_to(x + rw / 2.0 - e.width() / 2.0, y + rh - 14.0);
                    let _ = cr.show_text(t);
                }
            }
        }
        fonte(cr, crate::tema::fonte_ui(), gtk::cairo::FontWeight::Normal, 12.0);
        cor("#7A7B88", 1.0);
        cr.move_to(16.0, h - 16.0);
        let _ = cr.show_text(tr("Drag to rearrange · the edges snap by themselves"));
    }

    /// The dragged monitor goes where it was dropped, snapped to the nearest edge of another.
    fn mover(self: &Rc<Self>, nome: &str, (x, y): (i32, i32)) {
        let l = self.lista.borrow().clone();
        let Some(s) = l.iter().find(|s| s.nome == nome) else { return };
        let Some((_, _, w, h, ..)) = s.logico else { return };
        let outros: Vec<_> = l.iter().filter(|o| o.nome != nome).filter_map(|o| o.logico.as_ref().map(|g| (g.0, g.1, g.2, g.3))).collect();
        let (nx, ny) = encaixar((x, y, w, h), &outros);
        self.aplicar(vec![(nome.to_string(), json!({"Position": {"position": {"Specific": {"x": nx, "y": ny}}}}))], &format!("posição {nx},{ny}"));
    }

    // ------------------------------------------------------------ the panel

    fn painel_render(&self) {
        let Some(s) = self.escolhida() else { return };
        self.lendo.set(true);
        let n = self.lista.borrow().iter().position(|x| x.nome == s.nome).unwrap_or(0) + 1;
        self.p_rotulo.set_text(&format!("{} · {}", trf("MONITOR {n}", &[("n", n.to_string())]), s.descricao().to_uppercase()));
        // resolutions (the largest first), and the rates of the one in use
        let atual = s.atual.and_then(|i| s.modos.get(i)).cloned();
        let mut res: Vec<(u32, u32)> = Vec::new();
        for m in &s.modos {
            if !res.contains(&(m.w, m.h)) {
                res.push((m.w, m.h));
            }
        }
        res.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
        let preferida = s.modos.iter().find(|m| m.preferido).map(|m| (m.w, m.h));
        let nomes: Vec<String> = res.iter().map(|(w, h)| if Some((*w, *h)) == preferida { trf("{w} × {h} (recommended)", &[("w", w.to_string()), ("h", h.to_string())]) } else { format!("{w} × {h}") }).collect();
        trocar_itens(&self.p_resolucao, &nomes);
        if let Some(a) = &atual {
            if let Some(i) = res.iter().position(|r| *r == (a.w, a.h)) {
                {
                    // (only a change: setting it again would close its list while the person is choosing)
                    let i = i as u32;
                    if self.p_resolucao.selected() != i {
                        self.p_resolucao.set_selected(i);
                    }
                }
            }
        }
        let taxas = self.taxas(&s);
        trocar_itens(&self.p_taxa, &taxas.iter().map(|m| format!("{} Hz", (f64::from(*m) / 1000.0).round())).collect::<Vec<_>>());
        if let Some(a) = &atual {
            if let Some(i) = taxas.iter().position(|m| *m == a.mhz) {
                {
                    // (only a change: setting it again would close its list while the person is choosing)
                    let i = i as u32;
                    if self.p_taxa.selected() != i {
                        self.p_taxa.set_selected(i);
                    }
                }
            }
        }
        let ligada = s.logico.is_some();
        if let Some((_, _, _, _, e, tf)) = &s.logico {
            for (x, b) in &self.p_escalas {
                if (x - e).abs() < 0.01 {
                    b.set_active(true);
                }
            }
            if let Some(i) = orientacoes().iter().position(|(t, _)| t == tf) {
                {
                    // (only a change: setting it again would close its list while the person is choosing)
                    let i = i as u32;
                    if self.p_orientacao.selected() != i {
                        self.p_orientacao.set_selected(i);
                    }
                }
            }
        }
        for w in [self.p_resolucao.upcast_ref::<gtk::Widget>(), self.p_taxa.upcast_ref(), self.p_orientacao.upcast_ref()] {
            w.set_sensitive(ligada);
        }
        for (_, b) in &self.p_escalas {
            b.set_sensitive(ligada);
        }
        self.p_vrr_linha.set_visible(s.vrr_suportado);
        self.p_vrr.set_active(s.vrr);
        self.p_vrr.set_state(s.vrr);
        let principal = monitor_principal().and_then(|m| m.connector()).is_some_and(|c| c == s.nome);
        self.p_principal.set_active(principal);
        self.p_principal.set_state(principal);
        // (the main one stays main until another is chosen)
        self.p_principal.set_sensitive(!principal && ligada);
        self.lendo.set(false);
    }

    /// The refresh rates of the chosen resolution.
    fn taxas(&self, s: &Saida) -> Vec<u32> {
        let atual = s.atual.and_then(|i| s.modos.get(i)).cloned();
        let (w, h) = atual.map(|m| (m.w, m.h)).unwrap_or((0, 0));
        let mut t: Vec<u32> = s.modos.iter().filter(|m| m.w == w && m.h == h).map(|m| m.mhz).collect();
        t.sort_unstable_by(|a, b| b.cmp(a));
        t.dedup();
        t
    }

    fn resolucao(self: &Rc<Self>, i: u32) {
        if self.lendo.get() {
            return;
        }
        let Some(s) = self.escolhida() else { return };
        let mut res: Vec<(u32, u32)> = Vec::new();
        for m in &s.modos {
            if !res.contains(&(m.w, m.h)) {
                res.push((m.w, m.h));
            }
        }
        res.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
        let Some((w, h)) = res.get(i as usize).copied() else { return };
        // the fastest rate of that resolution
        let Some(m) = s.modos.iter().filter(|m| m.w == w && m.h == h).max_by_key(|m| m.mhz).cloned() else { return };
        if s.atual.and_then(|i| s.modos.get(i)) == Some(&m) {
            return;
        }
        self.com_desfazer(acao_modo(&m), "resolução");
    }

    fn taxa(self: &Rc<Self>, i: u32) {
        if self.lendo.get() {
            return;
        }
        let Some(s) = self.escolhida() else { return };
        let Some(a) = s.atual.and_then(|i| s.modos.get(i)).cloned() else { return };
        let Some(mhz) = self.taxas(&s).get(i as usize).copied() else { return };
        if mhz == a.mhz {
            return;
        }
        self.com_desfazer(acao_modo(&Modo { mhz, ..a }), "taxa");
    }

    /// "Estender" (every monitor on, where they were) or "Só o monitor N" (that one; the others off).
    fn so_o_monitor(self: &Rc<Self>, nome: Option<&str>) {
        let l = self.lista.borrow().clone();
        let mut pedidos = Vec::new();
        match nome {
            None => pedidos.extend(l.iter().filter(|s| s.logico.is_none()).map(|s| (s.nome.clone(), json!("On")))),
            Some(n) => {
                pedidos.push((n.to_string(), json!("On")));
                pedidos.extend(l.iter().filter(|s| s.nome != n && s.logico.is_some()).map(|s| (s.nome.clone(), json!("Off"))));
            }
        }
        if pedidos.is_empty() {
            return;
        }
        // (turning screens off can leave the person without one: it asks, and goes back by itself)
        let desfazer: Vec<(String, Value)> = l.iter().map(|s| (s.nome.clone(), json!(if s.logico.is_some() { "On" } else { "Off" }))).collect();
        self.aplicar(pedidos, &format!("arranjo {}", nome.unwrap_or("estender")));
        self.perguntar(desfazer);
    }

    /// A change that may leave the screen unreadable: applied, then "Manter?" for 15 s.
    fn com_desfazer(self: &Rc<Self>, acao: Value, o_que: &str) {
        if self.lendo.get() {
            return;
        }
        let nome = self.escolhida_nome();
        self.aplicar_com_desfazer(&nome, acao, o_que);
    }

    fn aplicar_com_desfazer(self: &Rc<Self>, nome: &str, acao: Value, o_que: &str) {
        let Some(s) = self.lista.borrow().iter().find(|s| s.nome == nome).cloned() else { return };
        let mut volta = Vec::new();
        if let Some(m) = s.atual.and_then(|i| s.modos.get(i)) {
            volta.push((s.nome.clone(), acao_modo(m)));
        }
        if let Some((_, _, _, _, e, tf)) = &s.logico {
            volta.push((s.nome.clone(), json!({"Scale": {"scale": {"Specific": e}}})));
            volta.push((s.nome.clone(), json!({"Transform": {"transform": tf}})));
        }
        self.aplicar(vec![(nome.to_string(), acao)], o_que);
        self.perguntar(volta);
    }

    /// "Manter estas configurações?" with the seconds left; at 0 it goes back.
    fn perguntar(self: &Rc<Self>, volta: Vec<(String, Value)>) {
        if let Some((_, t)) = self.desfazer.borrow_mut().take() {
            t.remove();
        }
        self.segundos.set(15);
        self.manter_txt.set_text(&trf("Keep these settings? Back as they were in {n} s", &[("n", "15".into())]));
        self.manter.set_visible(true);
        let w = Rc::downgrade(self);
        let t = glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(t) = w.upgrade() else { return glib::ControlFlow::Break };
            let n = t.segundos.get().saturating_sub(1);
            t.segundos.set(n);
            if n == 0 {
                // (the source ends by returning Break: it is taken out without removing it)
                let volta = t.desfazer.borrow_mut().take().map(|(v, _)| v);
                t.manter.set_visible(false);
                if let Some(v) = volta {
                    t.aplicar(v, "voltou sozinho");
                }
                return glib::ControlFlow::Break;
            }
            t.manter_txt.set_text(&trf("Keep these settings? Back as they were in {n} s", &[("n", n.to_string())]));
            glib::ControlFlow::Continue
        });
        *self.desfazer.borrow_mut() = Some((volta, t));
        // the question comes into view (the page's body scrolls) and Enter keeps the settings; the places the session
        // test reads are taken once it is in view
        let manter = self.manter.clone();
        glib::idle_add_local_once(move || {
            if let Some(b) = manter.last_child().and_then(|x| x.prev_sibling()) {
                b.grab_focus();
            }
            if depurando() {
                glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                    crate::central::posicao_debug("cfg tela manter", manter.last_child().and_then(|x| x.prev_sibling()).as_ref().unwrap_or(manter.upcast_ref()));
                    if let Some(v) = manter.last_child() {
                        crate::central::posicao_debug("cfg tela desfazer", &v);
                    }
                });
            }
        });
    }

    fn manter_agora(self: &Rc<Self>) {
        if let Some((_, t)) = self.desfazer.borrow_mut().take() {
            t.remove();
        }
        self.manter.set_visible(false);
        for s in self.lista.borrow().iter() {
            guardar_saida(s);
        }
        if depurando() {
            eprintln!("debug tela: mantido");
        }
    }

    fn voltar_agora(self: &Rc<Self>) {
        let volta = self.desfazer.borrow_mut().take().map(|(v, t)| {
            t.remove();
            v
        });
        self.manter.set_visible(false);
        if let Some(v) = volta {
            self.aplicar(v, "desfeito");
        }
    }

    /// Sends the changes to niri off the interface thread, keeps them (unless waiting for "Manter") and reads again.
    fn aplicar(self: &Rc<Self>, pedidos: Vec<(String, Value)>, o_que: &str) {
        self.p_erro.set_visible(false);
        let o_que = o_que.to_string();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            for (nome, a) in &pedidos {
                mudar(nome, a.clone())?;
            }
            Ok::<_, String>(())
        }, move |r| {
            let Some(t) = w.upgrade() else { return };
            match r {
                Ok(()) => {
                    if depurando() {
                        eprintln!("debug tela: {o_que}");
                    }
                    // the screen changed (a scale, a resolution): the window fits it again once niri has it (asked twice:
                    // niri can take a moment to give the new size)
                    for ms in [400, 1500] {
                        glib::timeout_add_local_once(std::time::Duration::from_millis(ms), || {
                            if let Some(c) = crate::shell_atual().and_then(|s| s.configuracoes_atual()) {
                                c.caber_na_tela("tela");
                            }
                        });
                    }
                }
                Err(e) => {
                    t.p_erro.set_text(&e);
                    t.p_erro.set_visible(true);
                }
            }
            let w = Rc::downgrade(&t);
            crate::lateral::em_segundo_plano(saidas, move |l| {
                let Some(t) = w.upgrade() else { return };
                t.mostrar_saidas(l);
                // kept at once when nothing waits for "Manter" (a place, the variable rate)
                if t.desfazer.borrow().is_none() {
                    for s in t.lista.borrow().iter() {
                        guardar_saida(s);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ the agent's suggestion

    /// A monitor turned to portrait, dense (above 110 dpi) and at 100 %: its text comes out small.
    fn sugerida(&self) -> Option<String> {
        let dispensada = guardadas()["sugestao_dispensada"].as_str().map(str::to_string);
        self.lista.borrow().iter().find(|s| {
            let Some((_, _, w, _, e, tf)) = &s.logico else { return false };
            let retrato = tf == "90" || tf == "270";
            let dpi = s.fisico.filter(|(fw, fh)| *fw > 0 && *fh > 0).map(|(fw, fh)| {
                let mm = if retrato { fh } else { fw };
                f64::from(*w) * e / (f64::from(mm) / 25.4)
            });
            retrato && *e < 1.2 && dpi.is_some_and(|d| d > 110.0) && dispensada.as_deref() != Some(s.nome.as_str())
        }).map(|s| s.nome.clone())
    }

    fn sugestao_render(&self) {
        let Some(nome) = self.sugerida() else {
            self.sugestao.set_visible(false);
            return;
        };
        let n = self.lista.borrow().iter().position(|s| s.nome == nome).unwrap_or(0) + 1;
        self.sugestao_txt.set_text(&trf("Monitor {n} is in portrait and its text comes out small. I can apply 125% scale on it only.", &[("n", n.to_string())]));
        self.sugestao.set_visible(true);
    }

    // ------------------------------------------------------------ night light

    fn noturna_render(&self) {
        self.lendo.set(true);
        let indisponivel = crate::sistema::night_light_unavailable();
        let ligada = crate::sistema::luz_noturna_ligada();
        self.n_agora.set_active(ligada);
        self.n_agora.set_state(ligada);
        self.n_agora.set_sensitive(!indisponivel || ligada);
        let auto = crate::configuracoes::ajustes()["luz_noturna_auto"].as_bool().unwrap_or(false);
        self.n_agenda.set_active(auto);
        self.n_agenda.set_state(auto);
        let temp = crate::configuracoes::ajustes()["luz_noturna_temperatura"].as_u64().unwrap_or(3500) as f64;
        self.n_temp.set_value(temp);
        self.n_temp_txt.set_text(&format!("{temp} K"));
        self.n_aviso.set_text(if indisponivel { tr("This screen does not let the colours change (no gamma control): the night light cannot turn on here.") } else { "" });
        self.n_aviso.set_visible(indisponivel);
        self.lendo.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_outputs_become_monitors_in_their_order() {
        let v: Value = serde_json::from_str(r#"{
            "DP-2": {"name": "DP-2", "make": "Dell", "model": "U2723QE", "serial": "X1", "physical_size": [597, 336], "current_mode": 0, "vrr_supported": false, "vrr_enabled": false,
                     "modes": [{"width": 3840, "height": 2160, "refresh_rate": 60000, "is_preferred": true}],
                     "logical": {"x": 3440, "y": 0, "width": 1080, "height": 1920, "scale": 2.0, "transform": "90"}},
            "DP-1": {"name": "DP-1", "make": "MSI", "model": "MAG341CQ", "serial": null, "physical_size": [800, 335], "current_mode": 1, "vrr_supported": true, "vrr_enabled": true,
                     "modes": [{"width": 3440, "height": 1440, "refresh_rate": 59973, "is_preferred": false}, {"width": 3440, "height": 1440, "refresh_rate": 143973, "is_preferred": true}],
                     "logical": {"x": 0, "y": 0, "width": 3440, "height": 1440, "scale": 1.0, "transform": "Normal"}},
            "HDMI-A-1": {"name": "HDMI-A-1", "make": "LG", "model": "TV", "serial": null, "physical_size": null, "current_mode": null, "vrr_supported": false, "vrr_enabled": false, "modes": [], "logical": null}
        }"#).unwrap();
        let l = ler_saidas(&v);
        assert_eq!(l.iter().map(|s| s.nome.as_str()).collect::<Vec<_>>(), vec!["DP-1", "DP-2", "HDMI-A-1"]);
        assert_eq!(l[0].descricao(), "MSI 34″ ultrawide");
        assert!(l[0].vrr_suportado && l[0].vrr);
        assert_eq!(l[0].atual.map(|i| l[0].modos[i].mhz), Some(143_973));
        assert!(l[2].logico.is_none());
    }

    #[test]
    fn a_dropped_monitor_snaps_to_the_nearest_edge() {
        let um = (0, 0, 3440, 1440);
        // dropped a bit off the right edge, a bit low: it touches the right edge and lines up with the top
        assert_eq!(encaixar((3500, 40, 1080, 1920), &[um]), (3440, 0));
        // dropped on the left, 40 px low: lined up with the top; 300 px low: it stays that low (an offset on purpose)
        assert_eq!(encaixar((-1200, 40, 1080, 1920), &[um]), (-1080, 0));
        assert_eq!(encaixar((-1200, 300, 1080, 1920), &[um]), (-1080, 300));
        // dropped under it, a bit to the right: it touches the bottom, lined up with the left
        assert_eq!(encaixar((50, 1500, 1920, 1080), &[um]), (0, 1440));
        // alone: at the origin
        assert_eq!(encaixar((300, 300, 1920, 1080), &[]), (0, 0));
    }
}
