//! A LaTeX document in an answer, drawn (design "Chat · diagramas, documentos e realce de sintaxe" › Documento LaTeX
//! desenhado): with a TeX engine, the document is compiled in the background — walled in (latex_pdf.rs), at most
//! [`crate::latex_pdf::PARALLEL`] at a time, once per text (the cache keeps the PDF, or the document's own error, by its
//! hash) — and its pages show as white paper on the panel, the width of the column. Poppler reads and draws them in a
//! thread; the conversation paints the pictures. "Documento / Código" switch the views, "Abrir no Canvas" shows the
//! pages large in the side panel, Copiar and "Gerar PDF" as in the code block. While it compiles: "Gerando a
//! prévia…"; when it fails: the error, the code and "Tentar de novo". Without an engine: the code, with a line saying
//! how to see it drawn.

use crate::conversa::ui_icon;
use crate::latex_pdf::{Cached, Compiled, Engine, Enqueued, Failure, Progress, Queue};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The pages shown in the conversation (the rest in the Canvas).
const PAGES_INLINE: usize = 3;

type Done = Box<dyn FnOnce(Result<Compiled, Failure>)>;
type Step = Box<dyn Fn(&Progress)>;

/// The compilations: the queue, and who hears each one (one run per key, however many blocks show the same text).
struct Jobs {
    queue: Queue,
    done: HashMap<String, Vec<Done>>,
    steps: HashMap<String, Vec<Step>>,
    /// what a waiting key will compile: (engine, text, output folder, output name)
    work: HashMap<String, (Engine, String, PathBuf, String)>,
}

thread_local! {
    static JOBS: RefCell<Jobs> = RefCell::new(Jobs { queue: Queue::new(crate::latex_pdf::PARALLEL), done: HashMap::new(), steps: HashMap::new(), work: HashMap::new() });
    /// who opens a document's pages in the Canvas (the agents center)
    static OPENERS: RefCell<Vec<Box<dyn Fn(PathBuf)>>> = const { RefCell::new(Vec::new()) };
}

/// Called with a PDF when "Abrir no Canvas" is pressed.
pub fn on_open(f: impl Fn(PathBuf) + 'static) {
    OPENERS.with(|o| o.borrow_mut().push(Box::new(f)));
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Compiles `src` into `out_dir/out_name` (queued; a key already running is joined); `step` hears the progress, `done`
/// the result, on the main loop.
pub fn request(key: &str, e: &Engine, src: &str, out_dir: &Path, out_name: &str, step: impl Fn(&Progress) + 'static, done: impl FnOnce(Result<Compiled, Failure>) + 'static) {
    let what = JOBS.with(|j| {
        let mut j = j.borrow_mut();
        j.done.entry(key.to_string()).or_default().push(Box::new(done));
        j.steps.entry(key.to_string()).or_default().push(Box::new(step));
        let what = j.queue.request(key);
        if what != Enqueued::Joined {
            j.work.insert(key.to_string(), (e.clone(), src.to_string(), out_dir.to_path_buf(), out_name.to_string()));
        }
        what
    });
    if debug() {
        eprintln!("debug latex: pedido {key}: {what:?}");
    }
    if what == Enqueued::Start {
        start(key.to_string());
    }
}

/// Runs a key's compilation in a thread and hands out its result; then the next waiting one.
fn start(key: String) {
    let Some((e, src, out_dir, out_name)) = JOBS.with(|j| j.borrow_mut().work.remove(&key)) else { return };
    enum Msg {
        Step(Progress),
        Done(Result<Compiled, Failure>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let tx2 = std::sync::Mutex::new(tx.clone());
    std::thread::spawn(move || {
        let r = crate::latex_pdf::compile(&e, &src, &out_dir, &out_name, move |p| {
            if let Ok(t) = tx2.lock() {
                let _ = t.send(Msg::Step(p));
            }
        });
        let _ = tx.send(Msg::Done(r));
    });
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        for m in rx.try_iter() {
            match m {
                Msg::Step(p) => JOBS.with(|j| j.borrow().steps.get(&key).into_iter().flatten().for_each(|s| s(&p))),
                Msg::Done(r) => {
                    let (done, next) = JOBS.with(|j| {
                        let mut j = j.borrow_mut();
                        j.steps.remove(&key);
                        (j.done.remove(&key).unwrap_or_default(), j.queue.finished(&key))
                    });
                    for d in done {
                        d(r.clone());
                    }
                    crate::latex_pdf::prune();
                    if let Some(n) = next {
                        start(n);
                    }
                    return glib::ControlFlow::Break;
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

/// How a block starts, from what is installed and what the cache holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// no TeX engine: the code, and how to see it drawn
    NoEngine,
    /// drawn before: its pages
    Show(Compiled),
    /// failed before (the document's own error): the error, the code and "Tentar de novo"
    Failed(String),
    /// compile it: "Gerando a prévia…"
    Compile,
}

pub fn start_of(engine: bool, cached: Cached) -> Start {
    match (engine, cached) {
        (false, _) => Start::NoEngine,
        (true, Cached::Pdf(c)) => Start::Show(c),
        (true, Cached::Failed(f)) => Start::Failed(f),
        (true, Cached::Nothing) => Start::Compile,
    }
}

/// What a start shows: the view ("documento" or "codigo"), whether the line over it shows, and whether "Tentar de
/// novo" does.
pub fn shows(s: &Start) -> (&'static str, bool, bool) {
    match s {
        Start::NoEngine => ("codigo", true, false),
        Start::Show(c) => ("documento", c.warning.is_some(), false),
        Start::Failed(_) => ("codigo", true, true),
        Start::Compile => ("documento", true, false),
    }
}

/// The pages a view keeps drawn (the least recently used picture goes first): a few, however long the document.
const PAGES_KEPT: usize = 6;
/// The pages the Canvas shows (the rest: "Abrir no Documentos").
pub const PAGES_CANVAS: usize = 30;

/// Views of pages alive (each frees its pictures when its widget goes: a debug count, and the tests').
static LIVE_VIEWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn live_views() -> usize {
    LIVE_VIEWS.load(std::sync::atomic::Ordering::SeqCst)
}

/// A page's pixels, drawn by Poppler in a thread (nothing of GTK there).
struct PagePixels {
    width: i32,
    height: i32,
    stride: i32,
    data: Vec<u8>,
}

/// The pages' sizes (points), at most `max`, and how many there are.
fn page_sizes(pdf: &Path, max: usize) -> Option<(usize, Vec<(f64, f64)>)> {
    let doc = poppler::Document::from_file(&gtk::prelude::FileExt::uri(&gtk::gio::File::for_path(pdf)), None).ok()?;
    let n = doc.n_pages().max(0) as usize;
    Some((n, (0..n.min(max)).filter_map(|k| doc.page(k as i32).map(|p| p.size())).collect()))
}

/// Draws page `k` `width` pixels wide.
fn render_page(pdf: &Path, k: usize, width: i32) -> Option<PagePixels> {
    let doc = poppler::Document::from_file(&gtk::prelude::FileExt::uri(&gtk::gio::File::for_path(pdf)), None).ok()?;
    let page = doc.page(k as i32)?;
    let (pw, ph) = page.size();
    let s = f64::from(width) / pw;
    let h = (ph * s).ceil() as i32;
    let mut img = gtk::cairo::ImageSurface::create(gtk::cairo::Format::Rgb24, width, h).ok()?;
    {
        let cr = gtk::cairo::Context::new(&img).ok()?;
        // (paper is white in either theme: it is the document's own page)
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        cr.scale(s, s);
        page.render(&cr);
    }
    img.flush();
    let stride = img.stride();
    let data = img.data().ok()?.to_vec();
    Some(PagePixels { width, height: h, stride, data })
}

/// A view of a PDF's pages. Owned by its widget (set on it as data); the drawings and the threads' answers hold it
/// weakly, so it goes with the widget, its pictures with it.
struct PagesView {
    pdf: PathBuf,
    areas: RefCell<Vec<gtk::DrawingArea>>,
    /// the pictures kept: (page, width, picture), the most recently used first
    kept: RefCell<Vec<(usize, i32, gtk::cairo::ImageSurface)>>,
    asked: RefCell<Vec<(usize, i32)>>,
    /// the scrolled window it is in (the Canvas): only the pages in sight are drawn
    viewport: RefCell<Option<glib::WeakRef<gtk::ScrolledWindow>>>,
}

impl Drop for PagesView {
    fn drop(&mut self) {
        LIVE_VIEWS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        if debug() {
            eprintln!("debug latex: vista de páginas liberada ({} vivas)", live_views());
        }
    }
}

impl PagesView {
    /// The width a page is drawn at for `w` pixels shown: twice (sharp on a scaled screen), in steps of 256.
    fn bucket(w: i32) -> i32 {
        ((w.max(64) * 2 + 255) / 256) * 256
    }

    fn in_sight(&self, area: &gtk::DrawingArea) -> bool {
        let Some(vp) = self.viewport.borrow().as_ref().and_then(|w| w.upgrade()) else { return true };
        match area.compute_bounds(&vp) {
            Some(b) => b.y() + b.height() >= -200.0 && b.y() <= vp.height() as f32 + 200.0,
            None => false,
        }
    }

    /// Paints page `k` from its picture; asks for one when there is none at this width (a page in sight only).
    fn paint(self: &Rc<Self>, k: usize, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32) {
        let want = Self::bucket(w * area.scale_factor().max(1) / 2);
        let pic = {
            let mut kept = self.kept.borrow_mut();
            let at = kept.iter().position(|(p, pw, _)| *p == k && *pw == want).or_else(|| kept.iter().position(|(p, _, _)| *p == k));
            at.map(|i| {
                let e = kept.remove(i);
                let s = e.2.clone();
                let exact = e.1 == want;
                kept.insert(0, e);
                (s, exact)
            })
        };
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        if let Some((img, _)) = &pic {
            let s = f64::from(w) / f64::from(img.width());
            cr.scale(s, s);
            let _ = cr.set_source_surface(img, 0.0, 0.0);
            cr.source().set_filter(gtk::cairo::Filter::Good);
            let _ = cr.paint();
        }
        if !pic.as_ref().is_some_and(|p| p.1) && self.in_sight(area) {
            self.ask(k, want);
        }
    }

    /// Draws page `k` `width` wide in a thread; keeps it (the least recently used out past [`PAGES_KEPT`]).
    fn ask(self: &Rc<Self>, k: usize, width: i32) {
        if self.asked.borrow().contains(&(k, width)) {
            return;
        }
        self.asked.borrow_mut().push((k, width));
        let (tx, rx) = std::sync::mpsc::channel();
        let pdf = self.pdf.clone();
        let t = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(render_page(&pdf, k, width));
        });
        let me = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
            let Ok(r) = rx.try_recv() else { return glib::ControlFlow::Continue };
            let Some(v) = me.upgrade() else { return glib::ControlFlow::Break };
            v.asked.borrow_mut().retain(|a| *a != (k, width));
            let Some(px) = r else { return glib::ControlFlow::Break };
            let Ok(img) = gtk::cairo::ImageSurface::create_for_data(px.data, gtk::cairo::Format::Rgb24, px.width, px.height, px.stride) else { return glib::ControlFlow::Break };
            {
                let mut kept = v.kept.borrow_mut();
                kept.retain(|(p, _, _)| *p != k);
                kept.insert(0, (k, width, img));
                kept.truncate(PAGES_KEPT);
            }
            if debug() {
                eprintln!("debug latex: página {} desenhada ({width} px, {} ms, {} guardadas)", k + 1, t.elapsed().as_millis(), v.kept.borrow().len());
            }
            if let Some(a) = v.areas.borrow().get(k) {
                a.queue_draw();
            }
            glib::ControlFlow::Break
        });
    }
}

/// A PDF's pages as paper, at most `max`, and a line under them ("2 páginas · documento.pdf"). Poppler reads and draws
/// them in threads; only the pages in sight are drawn, at the width shown, and only a few pictures are kept. In the
/// Canvas (`viewport`: the scrolled window), the pages past `max` are left to Documentos.
pub fn pages(pdf: &Path, max: usize, viewport: Option<&gtk::ScrolledWindow>) -> gtk::Box {
    let col = caixa(Orientation::Vertical, 16, "latex-paginas");
    let view = Rc::new(PagesView { pdf: pdf.to_path_buf(), areas: RefCell::default(), kept: RefCell::default(), asked: RefCell::default(), viewport: RefCell::new(viewport.map(|v| v.downgrade())) });
    LIVE_VIEWS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(vp) = viewport {
        let w = Rc::downgrade(&view);
        vp.vadjustment().connect_value_changed(move |_| {
            if let Some(v) = w.upgrade() {
                v.areas.borrow().iter().for_each(|a| a.queue_draw());
            }
        });
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let p = pdf.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(page_sizes(&p, max));
    });
    let w = Rc::downgrade(&view);
    let col2 = col.downgrade();
    let canvas = viewport.is_some();
    glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
        let Ok(r) = rx.try_recv() else { return glib::ControlFlow::Continue };
        let (Some(v), Some(col)) = (w.upgrade(), col2.upgrade()) else { return glib::ControlFlow::Break };
        let Some((n, sizes)) = r else { return glib::ControlFlow::Break };
        for (k, (pw, ph)) in sizes.into_iter().enumerate() {
            let aspect = ph / pw;
            let area = gtk::DrawingArea::new();
            area.set_hexpand(true);
            area.add_css_class("latex-pagina");
            area.set_content_height((640.0 * aspect) as i32);
            area.connect_resize(move |a, w, _| {
                let h = (f64::from(w) * aspect).round() as i32;
                if a.content_height() != h {
                    let a = a.clone();
                    glib::idle_add_local_once(move || a.set_content_height(h));
                }
            });
            let wv = Rc::downgrade(&v);
            area.set_draw_func(move |a, cr, w, _| {
                if let Some(v) = wv.upgrade() {
                    v.paint(k, a, cr, w);
                }
            });
            col.append(&area);
            v.areas.borrow_mut().push(area);
        }
        let name = v.pdf.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let count = if n == 1 { tr("1 page").to_string() } else { trf("{n} pages", &[("n", n.to_string())]) };
        let rest = n.saturating_sub(max);
        let more = match (rest, canvas) {
            (0, _) => String::new(),
            (r, false) => format!(" · {}", trf("{n} more in the Canvas", &[("n", r.to_string())])),
            (r, true) => format!(" · {}", trf("{n} more in Documents", &[("n", r.to_string())])),
        };
        let l = rotulo(&format!("{count} · {name}{more}"), "latex-paginas-legenda");
        l.set_halign(Align::Center);
        col.append(&l);
        glib::ControlFlow::Break
    });
    // (the widget owns the view)
    unsafe {
        col.set_data("paginas", view);
    }
    col
}

/// While the first preview is made: a dimmed blank page and "1 página · documento.pdf" (design "… › Gerando a prévia").
pub fn placeholder() -> gtk::Box {
    let col = caixa(Orientation::Vertical, 16, "latex-paginas");
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.add_css_class("latex-pagina");
    // (an A4 page's shape)
    area.set_content_height(905);
    area.connect_resize(|a, w, _| {
        let h = (f64::from(w) * 1.414).round() as i32;
        if a.content_height() != h {
            let a = a.clone();
            glib::idle_add_local_once(move || a.set_content_height(h));
        }
    });
    area.set_draw_func(|_, cr, _, _| {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        let _ = cr.paint();
    });
    col.append(&area);
    let l = rotulo(&format!("{} · documento.pdf", tr("1 page")), "latex-paginas-legenda");
    l.set_halign(Align::Center);
    col.append(&l);
    col
}

/// The block for a LaTeX document.
pub fn new(info: &str, src: &str, apply: bool) -> gtk::Widget {
    new_with(info, src, apply, crate::latex_pdf::engine())
}

/// [`new`] with the engine given (the tests' stand-in).
pub fn new_with(info: &str, src: &str, apply: bool, engine: Option<Engine>) -> gtk::Widget {
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("codigo", "#8E8F99", 16));
    cab.append(&rotulo("LaTeX", "md-bloco-titulo"));
    let det = rotulo(tr("document"), "md-bloco-detalhe");
    det.set_hexpand(true);
    det.set_xalign(0.0);
    det.set_ellipsize(gtk::pango::EllipsizeMode::End);
    det.set_max_width_chars(1);
    cab.append(&det);
    let vistas = gtk::Stack::new();
    vistas.set_vhomogeneous(false);
    vistas.set_interpolate_size(true);
    let seg = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
    seg.set_valign(Align::Center);
    let b_doc = gtk::ToggleButton::with_label(tr("Document"));
    let b_code = gtk::ToggleButton::with_label(tr("Code"));
    for b in [&b_doc, &b_code] {
        b.add_css_class("tabela-aba");
        seg.append(b);
    }
    b_code.set_group(Some(&b_doc));
    let canvas = gtk::Button::with_label(tr("Open in the Canvas"));
    canvas.add_css_class("md-bloco-botao");
    canvas.set_valign(Align::Center);
    canvas.set_sensitive(false);
    if engine.is_some() {
        cab.append(&seg);
        cab.append(&canvas);
    }
    cab.append(&crate::code_block::copy_button(src));
    // the line over the views: progress, an error (with "Tentar de novo"), the missing engine
    let aviso = caixa(Orientation::Horizontal, 8, "md-bloco-aviso");
    let ponto = caixa(Orientation::Horizontal, 0, "md-bloco-aviso-ponto");
    ponto.set_valign(Align::Center);
    ponto.set_hexpand(false);
    aviso.append(&ponto);
    let estado = rotulo("", "md-bloco-aviso-texto");
    estado.set_wrap(true);
    estado.set_xalign(0.0);
    estado.set_hexpand(true);
    aviso.append(&estado);
    let retry = gtk::Button::with_label(tr("Try again"));
    retry.add_css_class("md-bloco-botao");
    retry.set_valign(Align::Center);
    retry.set_visible(false);
    aviso.append(&retry);
    let div = caixa(Orientation::Horizontal, 0, "md-bloco-div");
    if let Some(e) = &engine {
        let gerar = gtk::Button::with_label(tr("Generate PDF"));
        gerar.add_css_class("md-bloco-aplicar");
        gerar.set_valign(Align::Center);
        cab.append(&gerar);
        crate::central::posicao_debug("latex gerar pdf", gerar.upcast_ref());
        let (e, tex, av, dv, st) = (e.clone(), src.to_string(), aviso.clone(), div.clone(), estado.clone());
        gerar.connect_clicked(move |b| crate::code_block::generate_pdf(b, &e, &tex, &av, &dv, &st));
    }
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    bloco.append(&aviso);
    bloco.append(&div);
    let (lang, _) = crate::code_block::parse_info(info);
    let doc_view = caixa(Orientation::Vertical, 0, "latex-documento");
    vistas.add_named(&doc_view, Some("documento"));
    vistas.add_named(&crate::code_block::body(if lang.is_empty() { "latex" } else { &lang }, src, 1, None), Some("codigo"));
    bloco.append(&vistas);
    let v = vistas.clone();
    b_code.connect_toggled(move |b| v.set_visible_child_name(if b.is_active() { "codigo" } else { "documento" }));
    crate::central::posicao_debug("latex documento", b_doc.upcast_ref());
    crate::central::posicao_debug("latex abrir no canvas", canvas.upcast_ref());
    let _ = apply;
    let key = engine.as_ref().map(|e| crate::latex_pdf::cache_key(src, e)).unwrap_or_default();
    let dir = crate::latex_pdf::cache_root().join(&key);
    let first = start_of(engine.is_some(), if engine.is_some() { crate::latex_pdf::cached(&dir) } else { Cached::Nothing });
    let pdf_now: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let p2 = pdf_now.clone();
    canvas.connect_clicked(move |_| {
        if let Some(p) = p2.borrow().clone() {
            if debug() {
                eprintln!("debug latex: abrir no canvas {}", p.display());
            }
            OPENERS.with(|o| o.borrow().iter().for_each(|f| f(p.clone())));
        }
    });
    // shows a state: its view, the line, the pages
    let apply_state = {
        // (weakly: the buttons of the block hold this, the block must still be free to go)
        let (doc_view, estado, aviso, div, canvas, b_doc, b_code, retry) = (doc_view.downgrade(), estado.downgrade(), aviso.downgrade(), div.downgrade(), canvas.downgrade(), b_doc.downgrade(), b_code.downgrade(), retry.downgrade());
        move |s: &Start| {
            let (Some(doc_view), Some(estado), Some(aviso), Some(div), Some(canvas), Some(b_doc), Some(b_code), Some(retry)) =
                (doc_view.upgrade(), estado.upgrade(), aviso.upgrade(), div.upgrade(), canvas.upgrade(), b_doc.upgrade(), b_code.upgrade(), retry.upgrade())
            else {
                return;
            };
            let (view, line, again) = shows(s);
            aviso.set_visible(line);
            div.set_visible(line);
            retry.set_visible(again);
            if view == "codigo" { b_code.set_active(true) } else { b_doc.set_active(true) }
            match s {
                Start::NoEngine => estado.set_text(tr("Without a TeX engine installed, the document stays as code. Install tectonic to see it drawn.")),
                Start::Failed(f) => estado.set_text(f),
                Start::Compile => {
                    estado.set_text(tr("Generating the preview…"));
                    while let Some(x) = doc_view.first_child() {
                        doc_view.remove(&x);
                    }
                    doc_view.append(&placeholder());
                }
                Start::Show(c) => {
                    while let Some(x) = doc_view.first_child() {
                        doc_view.remove(&x);
                    }
                    doc_view.append(&pages(&c.pdf, PAGES_INLINE, None));
                    *pdf_now.borrow_mut() = Some(c.pdf.clone());
                    canvas.set_sensitive(true);
                    if let Some(w) = &c.warning {
                        estado.set_text(&trf("LaTeX went past an error: {reason}", &[("reason", w.clone())]));
                    }
                }
            }
        }
    };
    let apply_state = Rc::new(apply_state);
    if debug() {
        eprintln!("debug latex: bloco começa em {}", match &first { Start::NoEngine => "sem motor", Start::Show(_) => "cache", Start::Failed(_) => "falha guardada", Start::Compile => "compilar" });
    }
    apply_state(&first);
    let Some(e) = engine else { return bloco.upcast() };
    // compiles (queued), then shows the result and keeps it
    let compile: Rc<dyn Fn()> = {
        let (e, src, dir, key, apply_state, estado) = (e.clone(), src.to_string(), dir.clone(), key.clone(), apply_state.clone(), estado.downgrade());
        Rc::new(move || {
            apply_state(&Start::Compile);
            let started = std::time::Instant::now();
            let (dir2, apply2, st) = (dir.clone(), apply_state.clone(), estado.clone());
            request(&key, &e, &src, &dir, "documento.pdf", move |p| {
                if let Some(st) = st.upgrade() {
                    st.set_text(&format!("{} {}", tr("Generating the preview…"), crate::code_block::progress_text(p)));
                }
            }, move |r| {
                let text = r.as_ref().err().map(crate::code_block::failure_text).unwrap_or_default();
                crate::latex_pdf::remember(&dir2, &r, &text);
                if debug() {
                    match &r {
                        Ok(c) => eprintln!("debug latex: prévia pronta (compilado, {} ms){}", started.elapsed().as_millis(), c.warning.as_ref().map(|w| format!(" com aviso: {w}")).unwrap_or_default()),
                        Err(f) => eprintln!("debug latex: prévia falhou: {f:?}"),
                    }
                }
                apply2(&match r {
                    Ok(c) => Start::Show(c),
                    Err(_) => Start::Failed(text),
                });
            });
        })
    };
    let c2 = compile.clone();
    let dir2 = dir.clone();
    retry.connect_clicked(move |_| {
        let _ = std::fs::remove_file(dir2.join("falha.txt"));
        c2();
    });
    crate::central::posicao_debug("latex tentar de novo", retry.upcast_ref());
    if first == Start::Compile {
        compile();
    }
    bloco.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn how_a_block_starts_and_what_it_shows() {
        let pdf = Compiled { pdf: "/c/documento.pdf".into(), warning: None };
        let warned = Compiled { pdf: "/c/documento.pdf".into(), warning: Some("x".into()) };
        // no engine: the code with its note, whatever the cache holds
        assert_eq!(start_of(false, Cached::Pdf(pdf.clone())), Start::NoEngine);
        assert_eq!(shows(&Start::NoEngine), ("codigo", true, false));
        // drawn before (a cache hit): the pages, the line only for an error LaTeX went past
        assert_eq!(start_of(true, Cached::Pdf(pdf.clone())), Start::Show(pdf.clone()));
        assert_eq!(shows(&Start::Show(pdf)), ("documento", false, false));
        assert_eq!(shows(&Start::Show(warned)), ("documento", true, false));
        // failed before: the error, the code and "Tentar de novo" (not compiled again on every reopening)
        assert_eq!(start_of(true, Cached::Failed("e".into())), Start::Failed("e".into()));
        assert_eq!(shows(&Start::Failed("e".into())), ("codigo", true, true));
        // nothing yet: "Gerando a prévia…" over the document view
        assert_eq!(start_of(true, Cached::Nothing), Start::Compile);
        assert_eq!(shows(&Start::Compile), ("documento", true, false));
    }

    /// Runs the main loop until `done` (at most 20 s).
    fn until(done: impl Fn() -> bool) -> bool {
        let t = std::time::Instant::now();
        while !done() && t.elapsed() < std::time::Duration::from_secs(20) {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        done()
    }

    fn find_button(w: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(b) = w.downcast_ref::<gtk::Button>().filter(|b| b.label().as_deref() == Some(label)) {
            return Some(b.clone());
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            if let Some(b) = find_button(&x, label) {
                return Some(b);
            }
            c = x.next_sibling();
        }
        None
    }

    /// The wiring around the compilations, with a stand-in engine (a script: it writes a real one-page PDF, or fails;
    /// each run counted): a text asked for twice compiles once; a failure is kept (the next block with the same text
    /// does not compile again) and "Tentar de novo" compiles it again; the pages' views go with their widgets. Needs a
    /// display for GTK (skipped without one).
    #[test]
    fn requests_retry_and_pages_freed() {
        let _g = crate::latex_pdf::TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if gtk::init().is_err() {
            eprintln!("(sem tela: o teste do GTK não roda)");
            return;
        }
        let dir = crate::latex_pdf::tests::temp("blocos");
        let cache = dir.join("cache");
        *crate::latex_pdf::TEST_CACHE.lock().unwrap() = Some(cache.clone());
        *crate::latex_pdf::TEST_BUNDLES.lock().unwrap() = Some(dir.join("pacotes"));
        // a real one-page PDF for the stand-in to hand out
        let sample = dir.join("amostra.pdf");
        {
            let s = gtk::cairo::PdfSurface::new(595.0, 842.0, &sample).unwrap();
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.move_to(50.0, 50.0);
            cr.show_text("ok").unwrap();
            drop(cr);
            s.finish();
        }
        let runs = dir.join("execucoes");
        let fail = dir.join("falhar");
        let (sample2, runs2, fail2) = (sample.clone(), runs.clone(), fail.clone());
        *crate::latex_pdf::TEST_COMMAND.lock().unwrap() = Some(std::sync::Arc::new(move |s: &crate::latex_pdf::Sandbox, _m: &crate::latex_pdf::Mode| {
            let script = format!("sleep 0.3; echo x >> '{}'; if [ -e '{}' ]; then echo '! LaTeX Error: Undefined control sequence.' >&2; else cp '{}' documento.pdf; fi",
                runs2.display(), fail2.display(), sample2.display());
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg(script).current_dir(&s.work).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            c
        }));
        let count = || std::fs::read_to_string(&runs).map(|t| t.lines().count()).unwrap_or(0);
        let e = Engine { path: "/bin/true".into(), tectonic: false };
        // the same text asked for twice: one run, both told
        let heard = Rc::new(Cell::new(0));
        for _ in 0..2 {
            let h = heard.clone();
            request("k1", &e, "x", &dir.join("o1"), "documento.pdf", |_| {}, move |r| {
                assert!(r.is_ok());
                h.set(h.get() + 1);
            });
        }
        assert!(until(|| heard.get() == 2));
        assert_eq!(count(), 1);
        // three texts at once: all done (two at a time)
        let heard = Rc::new(Cell::new(0));
        for k in ["k2", "k3", "k4"] {
            let h = heard.clone();
            request(k, &e, k, &dir.join(k), "documento.pdf", |_| {}, move |_| h.set(h.get() + 1));
        }
        assert!(until(|| heard.get() == 3));
        assert_eq!(count(), 4);
        // a block whose document fails: the error kept, "Tentar de novo" there
        std::fs::write(&fail, b"").unwrap();
        let src = "\\documentclass{article}\\begin{document}falha\\end{document}";
        let b1 = new_with("latex", src, true, Some(e.clone()));
        let key = crate::latex_pdf::cache_key(src, &e);
        assert!(until(|| matches!(crate::latex_pdf::cached(&cache.join(&key)), Cached::Failed(_))));
        assert_eq!(count(), 5);
        let retry = find_button(&b1, tr("Try again")).expect("Tentar de novo");
        assert!(until(|| retry.is_visible()));
        // another block with the same text: no new run
        let b2 = new_with("latex", src, true, Some(e.clone()));
        while glib::MainContext::default().iteration(false) {}
        assert_eq!(count(), 5);
        drop(b2);
        // fixed: "Tentar de novo" compiles again and the pages show
        std::fs::remove_file(&fail).unwrap();
        let before = live_views();
        retry.emit_clicked();
        assert!(until(|| matches!(crate::latex_pdf::cached(&cache.join(&key)), Cached::Pdf(_))));
        assert_eq!(count(), 6);
        assert!(until(|| live_views() == before + 1));
        // the block goes: its pages' view and pictures go with it
        drop(retry);
        drop(b1);
        assert!(until(|| live_views() == before), "a pages view stayed alive: {}", live_views());
        *crate::latex_pdf::TEST_COMMAND.lock().unwrap() = None;
        *crate::latex_pdf::TEST_CACHE.lock().unwrap() = None;
        *crate::latex_pdf::TEST_BUNDLES.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
