//! KalangoOS's print dialog (design "Imprimir", 62187): what every app prints comes here through the Print portal
//! (portal.rs): the document's real pages in the preview, the printer with its state, copies, pages (all, the one in
//! view, a range), both sides, colour, orientation, paper and scale; "Salvar como PDF". It prints with CUPS (`lp`;
//! the printers from `lpstat`). In the session test both are stand-ins (`KALANGO_LPSTAT`, `KALANGO_LP`): nothing is
//! ever printed by a test.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn lpstat() -> String {
    std::env::var("KALANGO_LPSTAT").unwrap_or_else(|_| "lpstat".into())
}

fn lp() -> String {
    std::env::var("KALANGO_LP").unwrap_or_else(|_| "lp".into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Printer {
    pub name: String,
    pub ready: bool,
    /// why it is stopped, when it says
    pub reason: String,
    pub default: bool,
}

impl Printer {
    /// "HP LaserJet M110 · pronta".
    pub fn label(&self) -> String {
        let state = if self.ready { tr("ready to print").to_string() } else if self.reason.is_empty() { tr("stopped").to_string() } else { self.reason.to_lowercase() };
        format!("{} · {state}", self.name.replace('_', " "))
    }
}

/// The printers CUPS knows (`lpstat -p -d`), the default first.
pub fn printers() -> Vec<Printer> {
    let out = std::process::Command::new(lpstat()).args(["-p", "-d"]).env("LC_ALL", "C").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    parse_lpstat(&out)
}

pub fn parse_lpstat(out: &str) -> Vec<Printer> {
    let mut v: Vec<Printer> = Vec::new();
    let mut default = String::new();
    for l in out.lines() {
        if let Some(rest) = l.strip_prefix("printer ") {
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            let ready = rest.contains(" is idle") || rest.contains(" now printing") || (rest.contains("enabled") && !rest.contains("disabled"));
            v.push(Printer { name, ready, reason: String::new(), default: false });
        } else if l.starts_with('\t') || l.starts_with("  ") {
            if let Some(p) = v.last_mut() {
                if p.reason.is_empty() {
                    p.reason = l.trim().to_string();
                }
            }
        } else if let Some(d) = l.strip_prefix("system default destination: ") {
            default = d.trim().to_string();
        }
    }
    for p in &mut v {
        p.default = p.name == default;
    }
    v.sort_by_key(|p| !p.default);
    v
}

/// What will be printed, as CUPS options.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub printer: String,
    pub copies: u32,
    /// None: every page; else "1-3,5"
    pub pages: Option<String>,
    pub duplex: bool,
    pub color: bool,
    pub landscape: bool,
    pub paper: String,
    pub fit: bool,
}

pub fn lp_args(c: &Choice, title: &str, file: &Path) -> Vec<String> {
    let mut a = vec!["-d".into(), c.printer.clone(), "-n".into(), c.copies.to_string(), "-t".into(), title.to_string()];
    let mut o = |k: &str| {
        a.push("-o".into());
        a.push(k.to_string());
    };
    o(if c.duplex { "sides=two-sided-long-edge" } else { "sides=one-sided" });
    o(if c.color { "print-color-mode=color" } else { "print-color-mode=monochrome" });
    o(if c.landscape { "orientation-requested=4" } else { "orientation-requested=3" });
    o(&format!("media={}", c.paper));
    if c.fit {
        o("fit-to-page");
    }
    if let Some(p) = &c.pages {
        a.push("-P".into());
        a.push(p.clone());
    }
    a.push(file.display().to_string());
    a
}

/// "1-3, 5" as written by a person → "1-3,5" (only digits, dashes and commas; pages within the document).
pub fn clean_range(s: &str, n: i32) -> Option<String> {
    let parts: Vec<String> = s.split([',', ';']).map(|p| p.trim().replace(' ', "")).filter(|p| !p.is_empty()).collect();
    let ok = !parts.is_empty() && parts.iter().all(|p| {
        let nums: Vec<i32> = p.split('-').filter_map(|x| x.parse().ok()).collect();
        let dashes = p.matches('-').count();
        (dashes == 0 && nums.len() == 1 || dashes == 1 && nums.len() == 2 && nums[0] <= nums[1]) && nums.iter().all(|x| *x >= 1 && *x <= n)
    });
    ok.then(|| parts.join(","))
}

/// How many pages a range has.
pub fn count_range(r: &str) -> i32 {
    r.split(',').map(|p| match p.split_once('-') {
        Some((a, b)) => b.parse::<i32>().unwrap_or(0) - a.parse::<i32>().unwrap_or(0) + 1,
        None => 1,
    }).sum()
}

/// What the person did with the dialog.
pub enum Outcome {
    Printed,
    Saved(PathBuf),
    Cancelled,
}

struct Dialog {
    doc: Option<poppler::Document>,
    page: Cell<i32>,
    pages: i32,
    preview: gtk::DrawingArea,
    page_label: gtk::Label,
    copies: Cell<u32>,
    copies_label: gtk::Label,
    range_mode: Cell<u8>,
    range: gtk::Entry,
    print_b: gtk::Button,
    status: gtk::Label,
}

fn segmented(options: &[&str], id: &str) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let b = caixa(Orientation::Horizontal, 0, "imp-segmentos");
    b.set_halign(Align::End);
    let mut v: Vec<gtk::ToggleButton> = Vec::new();
    for (i, o) in options.iter().enumerate() {
        let t = gtk::ToggleButton::with_label(o);
        t.add_css_class("imp-segmento");
        if let Some(first) = v.first() {
            t.set_group(Some(first));
        }
        crate::central::posicao_debug_sempre(&format!("imprimir {id} {i}"), t.upcast_ref());
        b.append(&t);
        v.push(t);
    }
    if let Some(f) = v.first() {
        f.set_active(true);
    }
    (b, v)
}

fn row(name: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 10, "imp-linha");
    let n = rotulo(name, "imp-nome");
    n.set_hexpand(true);
    n.set_xalign(0.0);
    r.append(&n);
    r.append(w);
    r
}

/// Shows the dialog for a PDF the app made; `done` gets what the person chose.
pub fn ask(pdf: PathBuf, title: String, app: String, done: impl FnOnce(Outcome) + 'static) {
    let Some(app_) = gio::Application::default().and_downcast::<gtk::Application>() else { return done(Outcome::Cancelled) };
    let doc = poppler::Document::from_file(&gio::File::for_path(&pdf).uri(), None).ok();
    // (the name to save it by: the PDF's own title, else what the app said, else the app and the moment)
    let title = doc.as_ref().and_then(|d| d.title()).map(|t| t.to_string()).filter(|t| !t.trim().is_empty())
        .or_else(|| Some(title.clone()).filter(|t| !t.is_empty() && t != tr("Print") && t != tr("Document")))
        .unwrap_or_else(|| format!("{app} {}", glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H-%M").ok()).map(|s| s.to_string()).unwrap_or_default()));
    let pages = doc.as_ref().map(|d| d.n_pages()).unwrap_or(0);
    let list = printers();
    if debug() {
        eprintln!("debug imprimir: aberto «{title}» de {app} ({pages} páginas, {} impressoras)", list.len());
    }
    let window = gtk::Window::builder().application(&app_).title(tr("Print")).default_width(920).default_height(580).resizable(false).build();
    window.add_css_class("imp-janela");
    let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
    sem_barra.set_visible(false);
    window.set_titlebar(Some(&sem_barra));
    // the preview: the document's own pages
    let left = caixa(Orientation::Vertical, 14, "imp-previa");
    left.set_size_request(430, -1);
    let preview = gtk::DrawingArea::new();
    preview.set_content_width(300);
    preview.set_content_height(424);
    preview.set_halign(Align::Center);
    left.append(&preview);
    let nav = caixa(Orientation::Horizontal, 14, "");
    nav.set_halign(Align::Center);
    let prev = gtk::Button::new();
    prev.add_css_class("imp-seta");
    prev.set_child(Some(&ui_icon("seta-esquerda", "#C9CAD1", 10)));
    let page_label = rotulo("", "imp-pagina");
    let next = gtk::Button::new();
    next.add_css_class("imp-seta");
    next.set_child(Some(&ui_icon("seta-direita", "#C9CAD1", 10)));
    nav.append(&prev);
    nav.append(&page_label);
    nav.append(&next);
    left.append(&nav);
    // the choices
    let right = caixa(Orientation::Vertical, 10, "imp-opcoes");
    right.set_hexpand(true);
    let head = caixa(Orientation::Horizontal, 8, "");
    let t = rotulo(tr("Print"), "imp-titulo");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    head.append(&t);
    let close = gtk::Button::new();
    close.add_css_class("imp-fechar");
    close.set_child(Some(&ui_icon("fechar", "#8E8F99", 10)));
    head.append(&close);
    right.append(&head);
    let pl = rotulo(tr("Printer"), "imp-nome");
    pl.set_xalign(0.0);
    right.append(&pl);
    let names: Vec<String> = if list.is_empty() { vec![tr("No printer — connect one, or save as PDF").to_string()] } else { list.iter().map(|p| p.label()).collect() };
    let printer = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
    printer.add_css_class("imp-impressora");
    crate::central::posicao_debug_sempre("imprimir impressora", printer.upcast_ref());
    right.append(&printer);
    let copies_box = caixa(Orientation::Horizontal, 0, "imp-copias");
    let minus = gtk::Button::with_label("−");
    minus.add_css_class("imp-mais-menos");
    let copies_label = rotulo("1", "imp-copias-n");
    let plus = gtk::Button::with_label("+");
    plus.add_css_class("imp-mais-menos");
    crate::central::posicao_debug_sempre("imprimir copias mais", plus.upcast_ref());
    copies_box.append(&minus);
    copies_box.append(&copies_label);
    copies_box.append(&plus);
    right.append(&row(tr("Copies to print"), &copies_box));
    let (pages_seg, pages_t) = segmented(&[tr("Every page"), tr("Current"), tr("Range")], "paginas");
    right.append(&row(tr("Pages"), &pages_seg));
    let range = gtk::Entry::new();
    range.set_placeholder_text(Some(tr("e.g. 1-3, 5")));
    range.set_visible(false);
    range.set_halign(Align::End);
    range.set_width_chars(14);
    crate::central::posicao_debug_sempre("imprimir intervalo", range.upcast_ref());
    right.append(&range);
    let duplex = gtk::Switch::new();
    duplex.set_valign(Align::Center);
    // (both sides by default, as the design: less paper; a printer that cannot just prints one side)
    duplex.set_active(true);
    right.append(&row(tr("Both sides"), &duplex));
    let (color_seg, color_t) = segmented(&[tr("Black and white"), tr("In colour")], "cor");
    right.append(&row(tr("Colour"), &color_seg));
    let (orient_seg, orient_t) = segmented(&[tr("Portrait"), tr("Landscape")], "orientacao");
    right.append(&row(tr("Orientation"), &orient_seg));
    const PAPERS: [(&str, &str); 4] = [("A4", "A4"), ("Letter", "Carta"), ("Legal", "Ofício"), ("A5", "A5")];
    let paper = gtk::DropDown::from_strings(&PAPERS.iter().map(|(_, n)| *n).collect::<Vec<_>>());
    right.append(&row(tr("Paper"), &paper));
    let scale = gtk::DropDown::from_strings(&[tr("Fit to the page"), tr("Actual size")]);
    right.append(&row(tr("Scale"), &scale));
    let space = caixa(Orientation::Vertical, 0, "");
    space.set_vexpand(true);
    right.append(&space);
    let status = rotulo("", "imp-estado");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_max_width_chars(1);
    right.append(&status);
    let foot = caixa(Orientation::Horizontal, 8, "");
    let save = gtk::Button::new();
    save.add_css_class("imp-salvar");
    let sv = caixa(Orientation::Horizontal, 6, "");
    sv.append(&ui_icon("download", "#C9CAD1", 12));
    sv.append(&rotulo(tr("Save as PDF"), ""));
    save.set_child(Some(&sv));
    crate::central::posicao_debug_sempre("imprimir salvar pdf", save.upcast_ref());
    let gap = caixa(Orientation::Horizontal, 0, "");
    gap.set_hexpand(true);
    let cancel = gtk::Button::with_label(tr("Cancel"));
    cancel.add_css_class("imp-cancelar");
    let print_b = gtk::Button::with_label("");
    print_b.add_css_class("imp-imprimir");
    print_b.set_sensitive(!list.is_empty());
    crate::central::posicao_debug_sempre("imprimir imprimir", print_b.upcast_ref());
    foot.append(&save);
    foot.append(&gap);
    foot.append(&cancel);
    foot.append(&print_b);
    right.append(&foot);
    let root = caixa(Orientation::Horizontal, 0, "imp-corpo");
    root.append(&left);
    root.append(&right);
    window.set_child(Some(&root));

    let d = Rc::new(Dialog { doc, page: Cell::new(0), pages, preview: preview.clone(), page_label, copies: Cell::new(1), copies_label,
        range_mode: Cell::new(0), range: range.clone(), print_b: print_b.clone(), status });
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Outcome)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = {
        let (done, w) = (done.clone(), window.clone());
        move |o: Outcome| {
            if let Some(f) = done.borrow_mut().take() {
                f(o);
            }
            w.close();
        }
    };
    let finish = Rc::new(finish);
    let dd = d.clone();
    preview.set_draw_func(move |_, cr, w, h| dd.draw(cr, f64::from(w), f64::from(h)));
    for (b, k) in [(&prev, -1), (&next, 1)] {
        let d = d.clone();
        b.connect_clicked(move |_| {
            let p = (d.page.get() + k).clamp(0, (d.pages - 1).max(0));
            d.page.set(p);
            d.update();
        });
    }
    for (b, k) in [(&minus, -1i32), (&plus, 1)] {
        let d = d.clone();
        b.connect_clicked(move |_| {
            d.copies.set((d.copies.get() as i32 + k).clamp(1, 99) as u32);
            d.update();
        });
    }
    for (i, t) in pages_t.iter().enumerate() {
        let d = d.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                d.range_mode.set(i as u8);
                d.range.set_visible(i == 2);
                if i == 2 {
                    d.range.grab_focus();
                }
                d.update();
            }
        });
    }
    let dd = d.clone();
    range.connect_changed(move |_| dd.update());
    // closing: the window's ×, Cancel, Esc
    let f = finish.clone();
    close.connect_clicked(move |_| f(Outcome::Cancelled));
    let f = finish.clone();
    cancel.connect_clicked(move |_| f(Outcome::Cancelled));
    let f = finish.clone();
    window.connect_close_request(move |_| {
        f(Outcome::Cancelled);
        glib::Propagation::Proceed
    });
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    // save as PDF: in Documents, with the document's name
    let (f, pdf2, title2) = (finish.clone(), pdf.clone(), title.clone());
    save.connect_clicked(move |_| {
        let dir = crate::pastas::documentos();
        let _ = std::fs::create_dir_all(&dir);
        let base = title2.trim_end_matches(".pdf").replace(['/', '\\'], "-");
        let mut dest = dir.join(format!("{base}.pdf"));
        let mut n = 2;
        while dest.exists() {
            dest = dir.join(format!("{base} ({n}).pdf"));
            n += 1;
        }
        match std::fs::copy(&pdf2, &dest) {
            Ok(_) => {
                if debug() {
                    eprintln!("debug imprimir: salvo como PDF «{}»", dest.display());
                }
                f(Outcome::Saved(dest));
            }
            Err(e) => eprintln!("kalango-shell: imprimir: {e}"),
        }
    });
    // print: CUPS's lp with the choices
    let (f, d2, title3) = (finish.clone(), d.clone(), title.clone());
    print_b.connect_clicked(move |_| {
        let Some(p) = list.get(printer.selected() as usize) else { return };
        let pages = match d2.range_mode.get() {
            1 => Some((d2.page.get() + 1).to_string()),
            2 => match clean_range(&d2.range.text(), d2.pages.max(1)) {
                Some(r) => Some(r),
                None => {
                    d2.status.set_text(&trf("Pages from 1 to {n}, like “1-3, 5”.", &[("n", d2.pages.to_string())]));
                    return;
                }
            },
            _ => None,
        };
        let choice = Choice {
            printer: p.name.clone(), copies: d2.copies.get(), pages, duplex: duplex.is_active(), color: color_t[1].is_active(),
            landscape: orient_t[1].is_active(), paper: PAPERS[paper.selected() as usize % PAPERS.len()].0.to_string(), fit: scale.selected() == 0,
        };
        let args = lp_args(&choice, &title3, &pdf);
        if debug() {
            eprintln!("debug imprimir: lp {}", args.join(" "));
        }
        match std::process::Command::new(lp()).args(&args).output() {
            Ok(o) if o.status.success() => f(Outcome::Printed),
            Ok(o) => d2.status.set_text(&trf("The printer did not take it: {error}", &[("error", String::from_utf8_lossy(&o.stderr).trim().to_string())])),
            Err(e) => d2.status.set_text(&trf("The printer did not take it: {error}", &[("error", e.to_string())])),
        }
    });
    let app_id = |j: &gtk::Window| {
        if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
            let id = std::ffi::CString::new("os.kalango.Imprimir").unwrap();
            unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
        }
    };
    window.connect_realize(move |j| app_id(j));
    window.connect_map(move |j| app_id(j));
    d.update();
    window.present();
}

impl Dialog {
    fn update(&self) {
        self.page_label.set_text(&trf("{n} of {total}", &[("n", (self.page.get() + 1).to_string()), ("total", self.pages.max(1).to_string())]));
        self.copies_label.set_text(&self.copies.get().to_string());
        let pages = match self.range_mode.get() {
            1 => 1,
            2 => clean_range(&self.range.text(), self.pages.max(1)).map(|r| count_range(&r)).unwrap_or(0),
            _ => self.pages.max(1),
        };
        let total = pages * self.copies.get() as i32;
        self.print_b.set_label(&if total == 1 { tr("Print 1 page").to_string() } else { trf("Print {n} pages", &[("n", total.to_string())]) });
        self.preview.queue_draw();
    }

    /// The page in view, white paper (rendered by poppler, scaled to fit).
    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        let Some(doc) = &self.doc else { return };
        let Some(p) = doc.page(self.page.get()) else { return };
        let (pw, ph) = p.size();
        let k = (w / pw).min(h / ph);
        let (x, y) = ((w - pw * k) / 2.0, (h - ph * k) / 2.0);
        cr.set_source_rgb(0.957, 0.957, 0.969);
        cr.rectangle(x, y, pw * k, ph * k);
        let _ = cr.fill();
        cr.save().ok();
        cr.translate(x, y);
        cr.scale(k, k);
        p.render(cr);
        cr.restore().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_and_options() {
        let p = parse_lpstat("printer HP_LaserJet_M110 is idle.  enabled since x\nprinter Epson disabled since y -\n\tSem papel\nsystem default destination: HP_LaserJet_M110\n");
        assert_eq!(p.len(), 2);
        assert!(p[0].default && p[0].ready);
        assert!(!p[1].ready);
        assert_eq!(p[1].reason, "Sem papel");
        assert_eq!(clean_range("1-3, 5", 7).as_deref(), Some("1-3,5"));
        assert_eq!(clean_range("2-9", 7), None);
        assert_eq!(clean_range("abc", 7), None);
        assert_eq!(count_range("1-3,5"), 4);
        let c = Choice { printer: "HP".into(), copies: 2, pages: Some("1-2".into()), duplex: true, color: false, landscape: false, paper: "A4".into(), fit: true };
        let a = lp_args(&c, "doc", Path::new("/tmp/x.pdf"));
        assert!(a.windows(2).any(|w| w == ["-n", "2"]) && a.contains(&"sides=two-sided-long-edge".to_string()) && a.contains(&"fit-to-page".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("/tmp/x.pdf"));
    }
}
