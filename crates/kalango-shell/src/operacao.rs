//! Long file operations (design "Operação de arquivos (cópia em andamento)"): a copy or a move that takes a while
//! lives in the island — collapsed, a bar in it; clicked, a card opens under the top bar with what goes where, how
//! much is done, the speed and the time left (with the speed's chart), the file being copied, and Pause, Details
//! and Cancel. Closing Files does not stop it. When it ends: a notice with "Show in the folder".
//!
//! The copying itself runs in a thread (by blocks, so it can pause and stop at any point; a stopped copy leaves
//! no half file behind); the interface reads its progress twice a second.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a running copy has done so far (shared with its thread).
#[derive(Default)]
pub struct Progresso {
    pub total_bytes: AtomicU64,
    pub feitos_bytes: AtomicU64,
    pub total_itens: AtomicU64,
    pub feitos_itens: AtomicU64,
    pub atual: Mutex<String>,
    pub pausado: AtomicBool,
    pub cancelado: AtomicBool,
    pub terminou: AtomicBool,
    pub erros: Mutex<Vec<String>>,
}

/// A count with the thousands marked as the design writes it ("1.284").
pub(crate) fn numero(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// A free name for `nome` in `pasta` ("x.pdf", "x (2).pdf"…).
pub(crate) fn nome_livre(pasta: &Path, nome: &str) -> PathBuf {
    let p = pasta.join(nome);
    if !p.exists() {
        return p;
    }
    let (raiz, ext) = match nome.rfind('.').filter(|i| *i > 0) {
        Some(i) => (&nome[..i], &nome[i..]),
        None => (nome, ""),
    };
    (2..).map(|n| pasta.join(format!("{raiz} ({n}){ext}"))).find(|p| !p.exists()).unwrap()
}

pub(crate) fn contar(p: &Path, pr: &Progresso) {
    let Ok(md) = std::fs::symlink_metadata(p) else { return };
    pr.total_itens.fetch_add(1, Ordering::Relaxed);
    if md.is_dir() {
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            contar(&e.path(), pr);
        }
    } else {
        pr.total_bytes.fetch_add(md.len(), Ordering::Relaxed);
    }
}

/// Copies one file by blocks, waiting while paused; stopped, the half-copied file is removed.
fn copiar_arquivo(de: &Path, para: &Path, pr: &Progresso) -> std::io::Result<bool> {
    let mut r = std::fs::File::open(de)?;
    let mut w = std::fs::File::create(para)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        while pr.pausado.load(Ordering::Relaxed) && !pr.cancelado.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if pr.cancelado.load(Ordering::Relaxed) {
            drop(w);
            let _ = std::fs::remove_file(para);
            return Ok(false);
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        pr.feitos_bytes.fetch_add(n as u64, Ordering::Relaxed);
        // (the session test's stand-in for a slow disk: bytes per second)
        if let Some(bps) = std::env::var("KALANGO_TESTE_COPIA_LENTA").ok().and_then(|v| v.parse::<u64>().ok()).filter(|b| *b > 0) {
            std::thread::sleep(Duration::from_secs_f64(n as f64 / bps as f64));
        }
    }
    if let Ok(md) = std::fs::metadata(de) {
        let _ = std::fs::set_permissions(para, md.permissions());
        if let Ok(t) = md.modified() {
            let _ = w.set_modified(t);
        }
    }
    Ok(true)
}

fn copiar_arvore(de: &Path, para: &Path, pr: &Progresso) -> std::io::Result<bool> {
    *pr.atual.lock().unwrap() = de.display().to_string();
    let md = std::fs::symlink_metadata(de)?;
    let ok = if md.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(de)?, para)?;
        true
    } else if md.is_dir() {
        std::fs::create_dir_all(para)?;
        let mut ok = true;
        for e in std::fs::read_dir(de)?.flatten() {
            if !copiar_arvore(&e.path(), &para.join(e.file_name()), pr)? {
                ok = false;
                break;
            }
        }
        ok
    } else {
        copiar_arquivo(de, para, pr)?
    };
    pr.feitos_itens.fetch_add(1, Ordering::Relaxed);
    Ok(ok)
}

/// Copies (or moves) `origens` into `destino`; runs in a thread.
pub fn executar(pr: &Progresso, origens: &[PathBuf], destino: &Path, mover: bool) {
    for o in origens {
        contar(o, pr);
    }
    for o in origens {
        if pr.cancelado.load(Ordering::Relaxed) {
            break;
        }
        let Some(nome) = o.file_name() else { continue };
        if destino.starts_with(o) {
            continue;
        }
        let alvo = nome_livre(destino, &nome.to_string_lossy());
        // a move in the same disk is a rename: instant
        if mover && std::fs::rename(o, &alvo).is_ok() {
            let so_este = Progresso::default();
            contar(&alvo, &so_este);
            pr.feitos_bytes.fetch_add(so_este.total_bytes.load(Ordering::Relaxed), Ordering::Relaxed);
            pr.feitos_itens.fetch_add(so_este.total_itens.load(Ordering::Relaxed), Ordering::Relaxed);
            continue;
        }
        match copiar_arvore(o, &alvo, pr) {
            Ok(true) if mover => {
                let r = if o.is_dir() { std::fs::remove_dir_all(o) } else { std::fs::remove_file(o) };
                if let Err(e) = r {
                    pr.erros.lock().unwrap().push(format!("{}: {e}", o.display()));
                }
            }
            Ok(_) => {}
            Err(e) => pr.erros.lock().unwrap().push(format!("{}: {e}", o.display())),
        }
    }
    pr.terminou.store(true, Ordering::Relaxed);
}

/// One copy the island follows.
pub struct Operacao {
    pub progresso: Arc<Progresso>,
    pub origens: Vec<PathBuf>,
    pub destino: PathBuf,
    pub mover: bool,
    inicio: Instant,
    /// speeds (bytes/s), the newest last: the card's chart
    velocidades: RefCell<VecDeque<f64>>,
    ultimo: RefCell<(Instant, u64)>,
}

impl Operacao {
    pub(crate) fn titulo(&self) -> String {
        let n = self.progresso.total_itens.load(Ordering::Relaxed);
        let para = self.destino.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.destino.display().to_string());
        if n == 1 {
            return if self.mover { trf("Moving 1 item to {folder}", &[("folder", para)]) } else { trf("Copying 1 item to {folder}", &[("folder", para)]) };
        }
        let n = numero(n);
        if self.mover {
            trf("Moving {n} items to {folder}", &[("n", n), ("folder", para)])
        } else {
            trf("Copying {n} items to {folder}", &[("n", n), ("folder", para)])
        }
    }

    fn de(&self) -> String {
        let pai = self.origens.first().and_then(|o| o.parent()).map(Path::to_path_buf).unwrap_or_default();
        let h = glib::home_dir();
        let s = pai.strip_prefix(&h).map(|r| r.display().to_string()).unwrap_or_else(|_| pai.display().to_string());
        trf("from {folder}", &[("folder", if s.is_empty() { "~".into() } else { s })])
    }

    /// The speed over the last half second (kept for the chart), and the time left.
    fn amostrar(&self) -> (f64, Option<u64>) {
        let agora = Instant::now();
        let feitos = self.progresso.feitos_bytes.load(Ordering::Relaxed);
        let (t0, b0) = *self.ultimo.borrow();
        let dt = agora.duration_since(t0).as_secs_f64().max(0.001);
        let v = if self.progresso.pausado.load(Ordering::Relaxed) { 0.0 } else { (feitos.saturating_sub(b0)) as f64 / dt };
        *self.ultimo.borrow_mut() = (agora, feitos);
        let mut vs = self.velocidades.borrow_mut();
        vs.push_back(v);
        while vs.len() > 40 {
            vs.pop_front();
        }
        // the time left from the average of the recent speeds (steadier than the last one)
        let media = vs.iter().rev().take(8).sum::<f64>() / vs.len().clamp(1, 8) as f64;
        let falta = self.progresso.total_bytes.load(Ordering::Relaxed).saturating_sub(feitos);
        (v, (media > 1.0).then(|| (falta as f64 / media) as u64))
    }

    /// The recent speed (bytes/s) and the time left, as the card last measured them (without a new sample).
    pub(crate) fn ritmo(&self) -> (f64, Option<u64>) {
        let vs = self.velocidades.borrow();
        let v = vs.back().copied().unwrap_or(0.0);
        let media = vs.iter().rev().take(8).sum::<f64>() / vs.len().clamp(1, 8) as f64;
        let falta = self.progresso.total_bytes.load(Ordering::Relaxed).saturating_sub(self.progresso.feitos_bytes.load(Ordering::Relaxed));
        (v, (media > 1.0).then(|| (falta as f64 / media) as u64))
    }

    pub fn fracao(&self) -> f64 {
        let t = self.progresso.total_bytes.load(Ordering::Relaxed);
        if t == 0 {
            return if self.progresso.terminou.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
        }
        (self.progresso.feitos_bytes.load(Ordering::Relaxed) as f64 / t as f64).min(1.0)
    }
}

type AoTerminar = Box<dyn Fn(&Operacao)>;
type Ilha = Box<dyn Fn(Option<f64>)>;

/// The card under the top bar and the island's bar, for the copy running now.
pub struct Operacoes {
    janela: gtk::Window,
    /// the windows behind the open card, blurred by the compositor (layer rule "kalango-operacao-fundo": one layer
    /// over each window, the desktop stays sharp, as in the design); a click on them folds the card back
    fundos: RefCell<Vec<gtk::Window>>,
    app: gtk::Application,
    titulo: gtk::Label,
    de: gtk::Label,
    pct: gtk::Label,
    barra: gtk::LevelBar,
    feitos: gtk::Label,
    ritmo: gtk::Label,
    grafico: gtk::DrawingArea,
    atual: gtk::Label,
    pausar_txt: gtk::Label,
    pausar_ic: gtk::Image,
    detalhes: gtk::Label,
    atual_op: RefCell<Option<Rc<Operacao>>>,
    ilha: RefCell<Option<Ilha>>,
    ao_terminar: RefCell<Option<AoTerminar>>,
    /// (for the session test: where its buttons are on the screen)
    botoes: RefCell<Vec<(&'static str, gtk::Widget)>>,
}

pub(crate) fn tempo(s: u64) -> String {
    if s < 60 {
        trf("{n} s left", &[("n", s.max(1).to_string())])
    } else if s < 3600 {
        trf("{n} min left", &[("n", s.div_ceil(60).to_string())])
    } else {
        trf("{h} h {m} min left", &[("h", (s / 3600).to_string()), ("m", ((s % 3600) / 60).to_string())])
    }
}

impl Operacoes {
    pub fn new(app: &gtk::Application) -> Rc<Operacoes> {
        let janela = gtk::Window::new();
        janela.set_application(Some(app));
        janela.init_layer_shell();
        janela.set_layer(Layer::Top);
        janela.set_namespace(Some("kalango-operacao"));
        janela.set_anchor(Edge::Top, true);
        janela.set_margin(Edge::Top, 4);
        // over the top bar, opening from the island (not below the bar's reserved room)
        janela.set_exclusive_zone(-1);
        janela.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
        janela.add_css_class("pedido-aviso-janela");
        let c = caixa(Orientation::Vertical, 12, "op-cartao");
        c.set_size_request(540, -1);
        // the header: the folder's tile, what goes where, the percentage
        let cab = caixa(Orientation::Horizontal, 12, "");
        let tile = caixa(Orientation::Horizontal, 0, "op-tile");
        tile.set_size_request(36, 36);
        tile.set_valign(Align::Center);
        tile.append(&ui_icon("pasta", "#7CC3DF", 16));
        cab.append(&tile);
        let t = caixa(Orientation::Vertical, 2, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        // (long texts shorten: the card keeps the design's 540 px)
        let titulo = rotulo("", "op-titulo");
        titulo.set_xalign(0.0);
        titulo.set_max_width_chars(1);
        titulo.set_hexpand(true);
        titulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.append(&titulo);
        let de = rotulo("", "op-sub");
        de.set_xalign(0.0);
        de.set_max_width_chars(1);
        de.set_hexpand(true);
        de.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        t.append(&de);
        cab.append(&t);
        let pct = rotulo("", "op-pct");
        cab.append(&pct);
        c.append(&cab);
        let barra = gtk::LevelBar::for_interval(0.0, 1.0);
        barra.add_css_class("op-barra");
        c.append(&barra);
        let nums = caixa(Orientation::Horizontal, 8, "");
        let feitos = rotulo("", "op-feitos");
        feitos.set_hexpand(true);
        feitos.set_xalign(0.0);
        nums.append(&feitos);
        let ritmo = rotulo("", "op-sub");
        nums.append(&ritmo);
        c.append(&nums);
        let grafico = gtk::DrawingArea::new();
        grafico.set_content_height(46);
        grafico.set_hexpand(true);
        c.append(&grafico);
        let atual = rotulo("", "op-atual");
        atual.set_xalign(0.0);
        atual.set_max_width_chars(1);
        atual.set_hexpand(true);
        atual.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        c.append(&atual);
        let detalhes = rotulo("", "op-sub");
        detalhes.set_xalign(0.0);
        detalhes.set_wrap(true);
        detalhes.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        detalhes.set_max_width_chars(1);
        detalhes.set_hexpand(true);
        detalhes.set_visible(false);
        c.append(&detalhes);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        let pausar = gtk::Button::new();
        pausar.add_css_class("op-pausar");
        let pc = caixa(Orientation::Horizontal, 6, "");
        let pausar_ic = ui_icon("pausa", "#E2E2E6", 16);
        pc.append(&pausar_ic);
        let pausar_txt = rotulo(tr("Pause"), "");
        pc.append(&pausar_txt);
        pausar.set_child(Some(&pc));
        acoes.append(&pausar);
        let det = gtk::ToggleButton::new();
        det.add_css_class("op-detalhes");
        let dc = caixa(Orientation::Horizontal, 6, "");
        dc.append(&ui_icon("seta-baixo", "#8E8F99", 16));
        dc.append(&rotulo(tr("Details"), ""));
        det.set_child(Some(&dc));
        acoes.append(&det);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        acoes.append(&esp);
        let cancelar = gtk::Button::with_label(tr("Cancel"));
        cancelar.add_css_class("op-cancelar");
        acoes.append(&cancelar);
        c.append(&acoes);
        janela.set_child(Some(&c));
        let o = Rc::new(Operacoes { janela, fundos: RefCell::default(), app: app.clone(), titulo, de, pct, barra, feitos, ritmo, grafico, atual, pausar_txt, pausar_ic, detalhes, atual_op: RefCell::default(),
            ilha: RefCell::default(), ao_terminar: RefCell::default(), botoes: RefCell::default() });
        let w = Rc::downgrade(&o);
        o.grafico.set_draw_func(move |_, cr, larg, alt| {
            let Some(o) = w.upgrade() else { return };
            let Some(op) = o.atual_op.borrow().clone() else { return };
            let vs = op.velocidades.borrow();
            if vs.len() < 2 {
                return;
            }
            let max = vs.iter().cloned().fold(1.0, f64::max) * 1.15;
            let n = 40.0;
            let px = |i: usize| larg as f64 * (i as f64 + (n - vs.len() as f64)) / (n - 1.0);
            let py = |v: f64| alt as f64 - 2.0 - (alt as f64 - 6.0) * v / max;
            cr.move_to(px(0), py(vs[0]));
            for (i, v) in vs.iter().enumerate().skip(1) {
                cr.line_to(px(i), py(*v));
            }
            let linha = cr.copy_path().ok();
            // the area under the line (12 %), then the line
            cr.line_to(px(vs.len() - 1), alt as f64);
            cr.line_to(px(0), alt as f64);
            cr.close_path();
            let (r, g, b) = crate::tema::rgb("#7CC3DF");
            cr.set_source_rgba(r, g, b, 0.12);
            let _ = cr.fill();
            if let Some(l) = linha {
                cr.new_path();
                cr.append_path(&l);
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(1.5);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                let _ = cr.stroke();
            }
        });
        let w = Rc::downgrade(&o);
        pausar.connect_clicked(move |_| {
            if let Some(op) = w.upgrade().and_then(|o| o.atual_op.borrow().clone()) {
                let p = !op.progresso.pausado.load(Ordering::Relaxed);
                op.progresso.pausado.store(p, Ordering::Relaxed);
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug operacao: {}", if p { "pausada" } else { "retomada" });
                }
            }
        });
        let w = Rc::downgrade(&o);
        det.connect_toggled(move |b| {
            if let Some(o) = w.upgrade() {
                o.detalhes.set_visible(b.is_active());
            }
        });
        let w = Rc::downgrade(&o);
        cancelar.connect_clicked(move |_| {
            if let Some(o) = w.upgrade() {
                if let Some(op) = o.atual_op.borrow().clone() {
                    op.progresso.cancelado.store(true, Ordering::Relaxed);
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug operacao: cancelada");
                    }
                }
            }
        });
        // a click outside (Esc) folds it back into the island
        let k = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&o);
        k.connect_key_pressed(move |_, tecla, _, _| {
            if tecla == gtk::gdk::Key::Escape {
                if let Some(o) = w.upgrade() {
                    o.recolher();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        o.janela.add_controller(k);
        *o.botoes.borrow_mut() = vec![("operacao pausar", pausar.upcast()), ("operacao cancelar", cancelar.upcast())];
        o
    }

    pub fn set_ilha(&self, f: impl Fn(Option<f64>) + 'static) {
        *self.ilha.borrow_mut() = Some(Box::new(f));
    }

    pub fn ao_terminar(&self, f: impl Fn(&Operacao) + 'static) {
        *self.ao_terminar.borrow_mut() = Some(Box::new(f));
    }

    /// The copy running now.
    pub fn atual(&self) -> Option<Rc<Operacao>> {
        self.atual_op.borrow().clone()
    }

    pub fn ativa(&self) -> bool {
        self.atual_op.borrow().is_some()
    }

    /// Starts a copy (or move) and follows it: the island's bar at once; the card when the island is clicked.
    pub fn iniciar(self: &Rc<Self>, origens: Vec<PathBuf>, destino: PathBuf, mover: bool) {
        let progresso = Arc::new(Progresso::default());
        let op = Rc::new(Operacao { progresso: progresso.clone(), origens: origens.clone(), destino: destino.clone(), mover, inicio: Instant::now(),
            velocidades: RefCell::default(), ultimo: RefCell::new((Instant::now(), 0)) });
        std::thread::spawn(move || executar(&progresso, &origens, &destino, mover));
        *self.atual_op.borrow_mut() = Some(op);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug operacao: começou");
        }
        self.atualizar();
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(o) = w.upgrade() else { return glib::ControlFlow::Break };
            if o.atualizar() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
        });
    }

    /// The card, opened from the island.
    pub fn expandir(self: &Rc<Self>) {
        if !self.ativa() {
            return;
        }
        // a second click on the island folds it back
        if self.janela.is_visible() {
            self.recolher();
            return;
        }
        self.desfocar_janelas();
        self.janela.present();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug operacao: cartão aberto");
            // its buttons on the screen (the card is centred under the top, 4 px down)
            let (j, bs) = (self.janela.clone(), self.botoes.borrow().clone());
            glib::timeout_add_local_once(Duration::from_millis(400), move || {
                let Some(m) = gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>() else { return };
                for (nome, b) in bs {
                    if let Some(r) = b.compute_bounds(&j) {
                        let x0 = (m.geometry().width() - j.width()) as f32 / 2.0;
                        eprintln!("debug operacao: {nome} na tela {:.0} {:.0}", x0 + r.x() + r.width() / 2.0, 4.0 + r.y() + r.height() / 2.0);
                    }
                }
            });
        }
    }

    pub fn recolher(&self) {
        self.janela.set_visible(false);
        for f in self.fundos.borrow_mut().drain(..) {
            f.destroy();
        }
    }

    /// A blurred layer over each window of the screen (from niri: where each one is).
    fn desfocar_janelas(self: &Rc<Self>) {
        for w in crate::encaixe::workspace_windows() {
            let l = &w["layout"];
            let num = |v: &serde_json::Value, i: usize| v[i].as_f64().unwrap_or(0.0);
            let (x, y) = (num(&l["tile_pos_in_workspace_view"], 0) + num(&l["window_offset_in_tile"], 0), num(&l["tile_pos_in_workspace_view"], 1) + num(&l["window_offset_in_tile"], 1));
            let (lw, lh) = (num(&l["window_size"], 0) as i32, num(&l["window_size"], 1) as i32);
            if lw <= 0 || lh <= 0 {
                continue;
            }
            let f = gtk::Window::new();
            f.set_application(Some(&self.app));
            f.init_layer_shell();
            f.set_layer(Layer::Top);
            f.set_namespace(Some("kalango-operacao-fundo"));
            f.set_anchor(Edge::Top, true);
            f.set_anchor(Edge::Left, true);
            f.set_exclusive_zone(-1);
            f.set_margin(Edge::Left, x.round() as i32);
            f.set_margin(Edge::Top, y.round() as i32);
            f.set_default_size(lw, lh);
            f.set_size_request(lw, lh);
            f.add_css_class("op-fundo");
            f.set_child(Some(&caixa(Orientation::Vertical, 0, "")));
            let clique = gtk::GestureClick::new();
            let o = Rc::downgrade(self);
            clique.connect_released(move |_, _, _, _| {
                if let Some(o) = o.upgrade() {
                    o.recolher();
                }
            });
            f.add_controller(clique);
            f.present();
            self.fundos.borrow_mut().push(f);
        }
    }

    /// Reads the progress; false when it ended.
    fn atualizar(self: &Rc<Self>) -> bool {
        let Some(op) = self.atual_op.borrow().clone() else { return false };
        let p = &op.progresso;
        let (v, falta) = op.amostrar();
        let f = op.fracao();
        self.titulo.set_text(&op.titulo());
        self.de.set_text(&op.de());
        self.pct.set_text(&format!("{}%", (f * 100.0).floor() as u32));
        self.barra.set_value(f);
        let (feito, total) = (p.feitos_bytes.load(Ordering::Relaxed), p.total_bytes.load(Ordering::Relaxed));
        self.feitos.set_text(&trf("{done} of {total}", &[("done", crate::arquivos::tamanho(feito)), ("total", crate::arquivos::tamanho(total))]));
        let pausado = p.pausado.load(Ordering::Relaxed);
        self.ritmo.set_text(&if pausado {
            tr("paused").to_string()
        } else {
            match falta {
                Some(s) => format!("{}/s · {}", crate::arquivos::tamanho(v as u64), tempo(s)),
                None => format!("{}/s", crate::arquivos::tamanho(v as u64)),
            }
        });
        self.atual.set_text(&p.atual.lock().map(|a| a.clone()).unwrap_or_default());
        crate::conversa::set_icon(&self.pausar_ic, if pausado { "tocar" } else { "pausa" }, "#E2E2E6");
        self.pausar_txt.set_text(if pausado { tr("Resume") } else { tr("Pause") });
        let erros = p.erros.lock().map(|e| e.clone()).unwrap_or_default();
        self.detalhes.set_text(&format!("{}\n{}{}", trf("{done} of {total} items", &[("done", numero(p.feitos_itens.load(Ordering::Relaxed))),
            ("total", numero(p.total_itens.load(Ordering::Relaxed)))]), trf("to {folder}", &[("folder", op.destino.display().to_string())]),
            erros.iter().take(5).map(|e| format!("\n{e}")).collect::<String>()));
        self.grafico.queue_draw();
        if let Some(i) = self.ilha.borrow().as_ref() {
            i(Some(f));
        }
        if p.terminou.load(Ordering::Relaxed) {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug operacao: terminou ({} erros, {})", erros.len(), if p.cancelado.load(Ordering::Relaxed) { "cancelada" } else { "completa" });
            }
            // done (a copy that took a while, so the person may be elsewhere), or with errors; cancelled is quiet
            if !p.cancelado.load(Ordering::Relaxed) {
                crate::configuracoes_som::tocar(if erros.is_empty() { "complete" } else { "dialog-error" });
            }
            self.atual_op.borrow_mut().take();
            self.recolher();
            if let Some(i) = self.ilha.borrow().as_ref() {
                i(None);
            }
            if let Some(f) = self.ao_terminar.borrow().as_ref() {
                f(&op);
            }
            return false;
        }
        let _ = op.inicio;
        true
    }
}

/// The island while a copy runs: a folder glyph and the bar (a click opens the card).
pub fn ilha_da_operacao() -> (gtk::Button, Box<dyn Fn(f64)>) {
    let b = gtk::Button::new();
    b.add_css_class("ilha");
    b.add_css_class("ilha-operacao");
    b.set_valign(Align::Center);
    let c = caixa(Orientation::Horizontal, 8, "");
    c.append(&ui_icon("pasta", "#7CC3DF", 16));
    let barra = gtk::LevelBar::for_interval(0.0, 1.0);
    barra.add_css_class("op-barra");
    barra.set_size_request(120, 4);
    barra.set_valign(Align::Center);
    c.append(&barra);
    let pct = rotulo("", "ilha-tarefa-conta");
    c.append(&pct);
    b.set_child(Some(&c));
    let definir = move |f: f64| {
        barra.set_value(f);
        pct.set_text(&format!("{}%", (f * 100.0).floor() as u32));
    };
    (b, Box::new(definir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_a_tree_counting_and_cancels_without_half_files() {
        let d = std::env::temp_dir().join(format!("kalango-op-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("de/sub")).unwrap();
        std::fs::create_dir_all(d.join("para")).unwrap();
        std::fs::write(d.join("de/a.txt"), vec![1u8; 3000]).unwrap();
        std::fs::write(d.join("de/sub/b.txt"), vec![2u8; 5000]).unwrap();
        let pr = Progresso::default();
        executar(&pr, &[d.join("de")], &d.join("para"), false);
        assert_eq!(pr.total_bytes.load(Ordering::Relaxed), 8000);
        assert_eq!(pr.feitos_bytes.load(Ordering::Relaxed), 8000);
        assert_eq!(std::fs::read(d.join("para/de/sub/b.txt")).unwrap(), vec![2u8; 5000]);
        // a second copy goes beside it, with another name
        let pr = Progresso::default();
        executar(&pr, &[d.join("de/a.txt")], &d.join("para/de"), false);
        assert!(d.join("para/de/a (2).txt").exists());
        // cancelled before it starts: nothing half written
        let pr = Progresso::default();
        pr.cancelado.store(true, Ordering::Relaxed);
        executar(&pr, &[d.join("de/sub/b.txt")], &d.join("para"), false);
        assert!(!d.join("para/b.txt").exists());
        // a move removes the origin
        let pr = Progresso::default();
        executar(&pr, &[d.join("de/a.txt")], &d.join("para"), true);
        assert!(!d.join("de/a.txt").exists() && d.join("para/a.txt").exists());
        let _ = std::fs::remove_dir_all(d);
    }
}
