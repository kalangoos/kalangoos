//! KalangoOS's Text Editor (design "Editor de texto", app id os.kalango.Editor; `kalango-shell editor [FILE]`), in
//! place of GNOME's.
//! - **Tabs:** one per file (× closes, asking first when it has changes), "+" a new one; Ctrl+N, Ctrl+O, Ctrl+S,
//!   Ctrl+Shift+S and Ctrl+W.
//! - **Source:** with its line numbers and the markdown's marks in colour.
//! - **Preview:** the same text as a page, as it is typed (markdown).
//! - **Sugestões:** the local model reads the text and offers what the agent could do with it; a click (or "Pedir ao
//!   agente", Ctrl+K) sends the request to the agent's bar with the text.

use crate::arquivos::seletor::{self, Filtro};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, CString};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A file open in a tab.
struct Doc {
    path: RefCell<Option<PathBuf>>,
    buffer: gtk::TextBuffer,
    /// what was read or last saved (changes are anything else)
    saved: RefCell<String>,
    /// when it was saved (the file's time, for one opened), in seconds
    saved_at: Cell<Option<i64>>,
    tab: gtk::Box,
    name: gtk::Label,
}

impl Doc {
    fn title(&self) -> String {
        self.path.borrow().as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("Untitled").to_string())
    }
    fn text(&self) -> String {
        let (a, b) = self.buffer.bounds();
        self.buffer.text(&a, &b, false).to_string()
    }
    fn changed(&self) -> bool {
        self.text() != *self.saved.borrow()
    }
    /// What would be lost: the lines new, changed and gone since it was saved (counted, no model involved).
    fn what_changed(&self) -> String {
        let now = self.text();
        let Some(at) = self.saved_at.get().filter(|_| self.path.borrow().is_some()) else {
            let n = now.lines().count().max(1);
            return if n == 1 { tr("It has 1 line and has not been saved yet.").to_string() } else { trf("It has {n} lines and has not been saved yet.", &[("n", n.to_string())]) };
        };
        let (came, changed, gone) = line_changes(&self.saved.borrow(), &now);
        let mut parts = Vec::new();
        if came > 0 {
            parts.push(if came == 1 { tr("1 new line").to_string() } else { trf("{n} new lines", &[("n", came.to_string())]) });
        }
        if changed > 0 {
            parts.push(if changed == 1 { tr("1 changed").to_string() } else { trf("{n} changed", &[("n", changed.to_string())]) });
        }
        if gone > 0 {
            parts.push(if gone == 1 { tr("1 removed").to_string() } else { trf("{n} removed", &[("n", gone.to_string())]) });
        }
        let changes = match parts.as_slice() {
            [] => return tr("Only the line breaks changed since you saved.").to_string(),
            [a] => a.clone(),
            [a, b] => trf("{a} and {b}", &[("a", a.clone()), ("b", b.clone())]),
            [a, b, c, ..] => trf("{a}, {b} and {c}", &[("a", a.clone()), ("b", b.clone()), ("c", c.clone())]),
        };
        let when = glib::DateTime::from_unix_local(at).ok();
        let today = glib::DateTime::now_local().ok().zip(when.as_ref()).is_some_and(|(n, w)| n.ymd() == w.ymd());
        match when {
            Some(w) if today => trf("Since you saved, at {time}: {changes}.", &[("time", w.format("%H:%M").map(|t| t.to_string()).unwrap_or_default()), ("changes", changes)]),
            Some(w) => trf("Since you saved, on {date}: {changes}.", &[("date", w.format("%d/%m").map(|t| t.to_string()).unwrap_or_default()), ("changes", changes)]),
            None => changes,
        }
    }
    /// The name a file without one is offered under: its first words ("relatorio de novembro.md").
    fn suggested_name(&self) -> String {
        if self.path.borrow().is_some() {
            return self.title();
        }
        let first = self.text().lines().map(|l| l.trim_start_matches(['#', '-', '*', '>', ' ']).trim().to_string()).find(|l| !l.is_empty()).unwrap_or_default();
        let name: String = first.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).take(40).collect();
        let name = name.trim().trim_end_matches('.');
        format!("{}.md", if name.is_empty() { tr("notes") } else { name })
    }
}

/// The lines new, changed and gone between two texts (a changed line is one gone where another came).
pub(crate) fn line_changes(old: &str, new: &str) -> (usize, usize, usize) {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // the longest run in common (a rewrite past 2000 × 2000 lines counts as it stands)
    let common = if a.len().saturating_mul(b.len()) > 4_000_000 {
        0
    } else {
        let mut row = vec![0usize; b.len() + 1];
        for x in a {
            let mut diag = 0;
            for (j, y) in b.iter().enumerate() {
                let up = row[j + 1];
                row[j + 1] = if x == y { diag + 1 } else { row[j + 1].max(row[j]) };
                diag = up;
            }
        }
        row[b.len()]
    };
    let (gone, came) = (a.len() - common, b.len() - common);
    let changed = gone.min(came);
    (came - changed, changed, gone - changed)
}

pub struct Editor {
    pub window: gtk::ApplicationWindow,
    tabs: gtk::Box,
    view: gtk::TextView,
    gutter: gtk::DrawingArea,
    preview: gtk::ScrolledWindow,
    suggestions: gtk::Box,
    ask: gtk::Entry,
    docs: RefCell<Vec<Rc<Doc>>>,
    current: Cell<usize>,
    shell: Weak<Shell>,
    preview_timer: RefCell<Option<glib::SourceId>>,
    suggest_timer: RefCell<Option<glib::SourceId>>,
    /// the window's layers: its questions come over it
    over: gtk::Overlay,
    /// the panes that give way on a narrow window (half the screen): the suggestions, then the preview
    side: gtk::Box,
    divider: gtk::Box,
    /// the bar's free stretch (where the window is dragged)
    bar_gap: gtk::Box,
    /// the question open ("Salvar as alterações?"), and how it closes
    asking: RefCell<Option<Rc<dyn Fn()>>>,
}

thread_local! {
    static EDITOR: RefCell<Option<Rc<Editor>>> = const { RefCell::new(None) };
}

/// Opens the editor (a file in a tab of its own, or a new one).
pub fn open(shell: &Rc<Shell>, path: Option<PathBuf>) {
    let e = EDITOR.with(|e| e.borrow().clone()).unwrap_or_else(|| {
        let app = gtk::gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let e = Editor::new(&app, shell);
        EDITOR.with(|x| *x.borrow_mut() = Some(e.clone()));
        e
    });
    // (closed and opened again: the screen may have changed meanwhile)
    if !e.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1400, 880);
        e.window.set_default_size(w, h);
    }
    match path {
        Some(p) => e.open_file(&p),
        None if e.docs.borrow().is_empty() => e.new_doc(),
        None => {}
    }
    e.window.present();
    if debug() {
        eprintln!("debug editor: aberto ({} abas)", e.docs.borrow().len());
    }
}

/// Puts text at the cursor of the document open in the Editor (a diagram from the conversation); false when no
/// document is open.
pub fn insert_in_open(text: &str) -> bool {
    let Some(e) = EDITOR.with(|e| e.borrow().clone()).filter(|e| e.window.is_visible()) else { return false };
    let Some(d) = e.docs.borrow().get(e.current.get()).cloned() else { return false };
    let b = &d.buffer;
    b.begin_user_action();
    let mut at = b.iter_at_mark(&b.get_insert());
    let antes = if at.starts_line() { "" } else { "\n" };
    b.insert(&mut at, &format!("{antes}{text}\n"));
    b.end_user_action();
    e.window.present();
    if debug() {
        eprintln!("debug editor: inserido ({} caracteres)", text.chars().count());
    }
    true
}

fn tag_table() -> gtk::TextTagTable {
    let t = gtk::TextTagTable::new();
    let add = |name: &str, f: &dyn Fn(&gtk::TextTag)| {
        let tag = gtk::TextTag::new(Some(name));
        f(&tag);
        t.add(&tag);
    };
    add("titulo", &|g| {
        g.set_foreground(Some("#A5A1E2"));
        g.set_weight(600);
    });
    add("marca", &|g| g.set_foreground(Some("#8E8F99")));
    add("feito", &|g| g.set_foreground(Some("#8E8F99")));
    add("codigo", &|g| g.set_foreground(Some("#8ED2C3")));
    add("forte", &|g| g.set_weight(700));
    t
}

impl Editor {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Editor> {
        // the design's 1400 × 880; on a smaller screen almost all of it (the bars and the dock stay in view)
        let (w, h) = crate::superficies::tamanho_de_janela(1400, 880);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Text Editor")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Editor");
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden)
        crate::superficies::release_resources_when_hidden(&window, || true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        // the toolbar: the app, its tabs, "+", the agent's field, the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        if let Some(a) = shell.todos.iter().find(|a| a.id == "os.kalango.Editor") {
            bar.append(&crate::apps::imagem(a, 24));
        }
        let tabs = caixa(Orientation::Horizontal, 4, "");
        tabs.set_valign(Align::Center);
        bar.append(&tabs);
        let new_tab = gtk::Button::new();
        new_tab.add_css_class("ed-nova-aba");
        new_tab.set_child(Some(&ui_icon("mais", "#8E8F99", 14)));
        new_tab.set_tooltip_text(Some(tr("New tab")));
        new_tab.set_valign(Align::Center);
        bar.append(&new_tab);
        let gap = caixa(Orientation::Horizontal, 0, "");
        gap.set_hexpand(true);
        bar.append(&gap);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(300, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Ask the agent: “summarize this text”")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ask_box = gtk::Overlay::new();
        ask_box.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ask_box.add_overlay(&key);
        bar.append(&ask_box);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        pop.set_child(Some(&items));
        menu.set_popover(Some(&pop));
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));

        // the source, its numbers in a gutter
        let view = gtk::TextView::new();
        view.add_css_class("ed-fonte");
        // the selected text's menu (copy, search, the agent explains, rewrites or translates it in place, read aloud)
        crate::context_menu::text_selection(&view, Rc::downgrade(shell));
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(28);
        view.set_bottom_margin(28);
        view.set_left_margin(14);
        view.set_right_margin(32);
        view.set_pixels_below_lines(6);
        let gutter = gtk::DrawingArea::new();
        gutter.set_content_width(40);
        gutter.set_vexpand(true);
        view.set_gutter(gtk::TextWindowType::Left, Some(&gutter));
        let src = gtk::ScrolledWindow::new();
        src.set_hscrollbar_policy(gtk::PolicyType::Never);
        src.set_child(Some(&view));
        src.set_hexpand(true);
        let divider = caixa(Orientation::Vertical, 0, "ed-divisor");
        divider.set_size_request(1, -1);
        let preview = gtk::ScrolledWindow::new();
        preview.set_hscrollbar_policy(gtk::PolicyType::Never);
        preview.add_css_class("ed-previa");
        preview.set_hexpand(true);
        // the agent's suggestions
        let side = caixa(Orientation::Vertical, 10, "ed-agente");
        side.set_size_request(340, -1);
        // (340 px as in the design: its texts wrap inside it instead of widening it)
        side.set_hexpand(false);
        let head = caixa(Orientation::Horizontal, 6, "");
        head.append(&ui_icon("spark", "#A5A1E2", 13));
        head.append(&rotulo(tr("Suggestions"), "ed-agente-titulo"));
        side.append(&head);
        let suggestions = caixa(Orientation::Vertical, 10, "");
        side.append(&suggestions);
        let body = caixa(Orientation::Horizontal, 0, "");
        body.set_vexpand(true);
        body.append(&src);
        body.append(&divider);
        body.append(&preview);
        body.append(&side);
        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&handle);
        root.append(&body);
        let over = gtk::Overlay::new();
        over.set_child(Some(&root));
        // (measures the window as it changes — a tiled half, a quarter — and nothing else)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        over.add_overlay(&meter);
        window.set_child(Some(&over));

        let e = Rc::new(Editor {
            window, tabs, view, gutter, preview, suggestions, ask, docs: RefCell::default(), current: Cell::new(0), shell: Rc::downgrade(shell),
            preview_timer: RefCell::default(), suggest_timer: RefCell::default(), over, asking: RefCell::default(), side, divider,
            bar_gap: gap,
        });
        let w = Rc::downgrade(&e);
        meter.connect_resize(move |_, width, _| {
            if let Some(e) = w.upgrade() {
                e.fit(width);
            }
        });
        // the line numbers of the lines in view
        let w = Rc::downgrade(&e);
        e.gutter.set_draw_func(move |_, cr, width, _| {
            let Some(e) = w.upgrade() else { return };
            let v = &e.view;
            let rect = v.visible_rect();
            // (the view's top margin makes the visible rectangle start above the first line)
            // (the line at the top of the view, whatever the x: x 0 is the text's margin, where there is no text)
            let (it, _) = v.line_at_y(rect.y().max(0));
            // (13 px JetBrains Mono, #7A7B88, right-aligned, on the first row of each line)
            cr.select_font_face("JetBrains Mono", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(13.0);
            cr.set_source_rgb(0x7A as f64 / 255.0, 0x7B as f64 / 255.0, 0x88 as f64 / 255.0);
            let row = cr.font_extents().map(|f| f.ascent()).unwrap_or(11.0);
            // (by the buffer's count: the empty line after a final line break has its number too)
            let b = v.buffer();
            for line in it.line()..b.line_count() {
                let Some(li) = b.iter_at_line(line) else { break };
                let (y, _) = v.line_yrange(&li);
                if y > rect.y() + rect.height() {
                    break;
                }
                let (_, wy) = v.buffer_to_window_coords(gtk::TextWindowType::Left, 0, y);
                let n = (line + 1).to_string();
                let lw = cr.text_extents(&n).map(|e| e.x_advance()).unwrap_or(8.0);
                cr.move_to(f64::from(width) - lw - 8.0, f64::from(wy) + row + 2.0);
                let _ = cr.show_text(&n);
            }
        });
        let g = e.gutter.downgrade();
        src.vadjustment().connect_value_changed(move |_| {
            if let Some(g) = g.upgrade() {
                g.queue_draw();
            }
        });
        let w = Rc::downgrade(&e);
        new_tab.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.new_doc();
            }
        });
        for (label, action) in [(tr("Open…"), "abrir"), (tr("Save"), "salvar"), (tr("Save as…"), "salvar-como")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            let (w, p) = (Rc::downgrade(&e), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                if let Some(e) = w.upgrade() {
                    e.action(action);
                }
            });
            items.append(&b);
        }
        let w = Rc::downgrade(&e);
        e.ask.connect_activate(move |a| {
            if let Some(e) = w.upgrade() {
                let q = a.text().trim().to_string();
                if !q.is_empty() {
                    e.to_agent(&q);
                    a.set_text("");
                }
            }
        });
        // the keys
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&e);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(e) = w.upgrade() else { return glib::Propagation::Proceed };
            // a question open: Esc is "Cancelar" (↵ is its focused "Salvar"); the shortcuts wait for it
            let asking = e.asking.borrow().clone();
            if let Some(close) = asking {
                if k == gdk::Key::Escape {
                    close();
                    return glib::Propagation::Stop;
                }
                return if m.contains(gdk::ModifierType::CONTROL_MASK) { glib::Propagation::Stop } else { glib::Propagation::Proceed };
            }
            if !m.contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
            match k.to_lower() {
                gdk::Key::s if shift => e.action("salvar-como"),
                gdk::Key::s => e.action("salvar"),
                gdk::Key::o => e.action("abrir"),
                gdk::Key::n | gdk::Key::t => e.new_doc(),
                gdk::Key::w => e.close_tab(e.current.get()),
                gdk::Key::k => {
                    e.ask.grab_focus();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        e.window.add_controller(keys);
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = CString::new("os.kalango.Editor").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        e.window.connect_realize(move |j| app_id(j));
        e.window.connect_map(move |j| app_id(j));
        // closing the window: each tab with changes asks first
        let w = Rc::downgrade(&e);
        e.window.connect_close_request(move |win| {
            let Some(e) = w.upgrade() else { return glib::Propagation::Proceed };
            if let Some(i) = e.docs.borrow().iter().position(|d| d.changed()) {
                e.select(i);
                e.ask_to_save(i, true);
                return glib::Propagation::Stop;
            }
            e.docs.borrow_mut().clear();
            while let Some(c) = e.tabs.first_child() {
                e.tabs.remove(&c);
            }
            win.set_visible(false);
            if debug() {
                eprintln!("debug editor: fechado");
            }
            glib::Propagation::Stop
        });
        e
    }

    fn add_doc(self: &Rc<Self>, path: Option<PathBuf>, text: &str) {
        let buffer = gtk::TextBuffer::new(Some(&tag_table()));
        buffer.set_text(text);
        let tab = caixa(Orientation::Horizontal, 8, "ed-aba");
        let name = rotulo("", "ed-aba-nome");
        let click = gtk::GestureClick::new();
        tab.append(&name);
        let close = gtk::Button::new();
        close.add_css_class("ed-aba-fechar");
        close.set_child(Some(&ui_icon("fechar", "#8E8F99", 10)));
        close.set_tooltip_text(Some(tr("Close tab")));
        tab.append(&close);
        let saved_at = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        let doc = Rc::new(Doc { path: RefCell::new(path), buffer: buffer.clone(), saved: RefCell::new(text.to_string()), saved_at: Cell::new(saved_at), tab: tab.clone(), name });
        let (w, d) = (Rc::downgrade(self), Rc::downgrade(&doc));
        click.connect_released(move |_, _, _, _| {
            if let (Some(e), Some(d)) = (w.upgrade(), d.upgrade()) {
                if let Some(i) = e.docs.borrow().iter().position(|x| Rc::ptr_eq(x, &d)) {
                    e.select(i);
                }
            }
        });
        tab.add_controller(click);
        let (w, d) = (Rc::downgrade(self), Rc::downgrade(&doc));
        close.connect_clicked(move |_| {
            if let (Some(e), Some(d)) = (w.upgrade(), d.upgrade()) {
                if let Some(i) = e.docs.borrow().iter().position(|x| Rc::ptr_eq(x, &d)) {
                    e.close_tab(i);
                }
            }
        });
        let w = Rc::downgrade(self);
        buffer.connect_changed(move |b| {
            if let Some(e) = w.upgrade() {
                color(b);
                e.update_titles();
                e.gutter.queue_draw();
                e.schedule_preview();
                e.schedule_suggestions();
            }
        });
        color(&buffer);
        self.tabs.append(&tab);
        self.docs.borrow_mut().push(doc);
        let n = self.docs.borrow().len() - 1;
        self.select(n);
    }

    fn new_doc(self: &Rc<Self>) {
        self.add_doc(None, "");
    }

    fn open_file(self: &Rc<Self>, p: &std::path::Path) {
        // the same file again: its tab
        if let Some(i) = self.docs.borrow().iter().position(|d| d.path.borrow().as_deref() == Some(p)) {
            self.select(i);
            return;
        }
        // a file not there yet (`kalango-shell editor novo.txt`): an empty tab that saving creates
        let read = match std::fs::read(p) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && p.parent().is_some_and(|d| d.is_dir()) => Ok(Vec::new()),
            r => r,
        };
        match read {
            Ok(b) => {
                let text = String::from_utf8_lossy(&b).into_owned();
                // a new, empty tab is replaced by the file
                let blank = self.docs.borrow().get(self.current.get()).is_some_and(|d| d.path.borrow().is_none() && d.text().is_empty());
                if blank {
                    let i = self.current.get();
                    self.close_tab_now(i);
                }
                self.add_doc(Some(p.to_path_buf()), &text);
                if debug() {
                    eprintln!("debug editor: abriu {} ({} linhas)", p.display(), text.lines().count());
                }
            }
            Err(err) => eprintln!("kalango-shell: editor: {}: {err}", p.display()),
        }
    }

    fn select(self: &Rc<Self>, i: usize) {
        let Some(d) = self.docs.borrow().get(i).cloned() else { return };
        self.current.set(i);
        self.view.set_buffer(Some(&d.buffer));
        for (n, x) in self.docs.borrow().iter().enumerate() {
            if n == i { x.tab.add_css_class("ativa") } else { x.tab.remove_css_class("ativa") }
        }
        self.update_titles();
        self.render_preview();
        self.schedule_suggestions();
        self.gutter.queue_draw();
        self.view.grab_focus();
    }

    fn update_titles(&self) {
        for d in self.docs.borrow().iter() {
            // (a dot before the name: changes not saved)
            d.name.set_text(&if d.changed() { format!("• {}", d.title()) } else { d.title() });
        }
        if let Some(d) = self.docs.borrow().get(self.current.get()) {
            self.window.set_title(Some(&format!("{} — {}", d.title(), tr("Text Editor"))));
        }
    }

    fn schedule_preview(self: &Rc<Self>) {
        if let Some(t) = self.preview_timer.borrow_mut().take() {
            t.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            if let Some(e) = w.upgrade() {
                e.preview_timer.borrow_mut().take();
                e.render_preview();
            }
        });
        *self.preview_timer.borrow_mut() = Some(id);
    }

    /// The text as a page (markdown), keeping where the person was reading.
    fn render_preview(&self) {
        let Some(d) = self.docs.borrow().get(self.current.get()).cloned() else { return };
        let pos = self.preview.vadjustment().value();
        let page = crate::markdown::render(&d.text());
        page.add_css_class("ed-pagina");
        // (the design's rhythm: 10 px between a page's lines)
        page.set_spacing(7);
        self.preview.set_child(Some(&page));
        let adj = self.preview.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(pos));
    }

    /// The local model's suggestions for the text, a moment after the person stops typing.
    /// The window's width decides the panes: all three from 1180 px (as drawn), the source and its preview below that
    /// (the agent stays one Ctrl+K away), the source alone below 760.
    fn fit(self: &Rc<Self>, width: i32) {
        let (side, preview) = (width >= 1180, width >= 760);
        // "Pedir ao agente" gives way too (300 px as drawn, down to 160): the bar keeps room to drag the window by
        self.ask.set_size_request((width - 600).clamp(160, 300), 32);
        if debug() {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                let Some(e) = w.upgrade() else { return };
                if let Some(b) = e.bar_gap.compute_bounds(&e.window) {
                    // (near its start: the middle of the screen is where notices come down)
                    eprintln!("debug editor: barra livre em {} {}", (b.x() + (b.width() / 2.).min(60.)) as i32, (b.y() + b.height() / 2.) as i32);
                }
            });
        }
        if self.side.is_visible() != side {
            self.side.set_visible(side);
            if side {
                self.schedule_suggestions();
            }
            if debug() {
                eprintln!("debug editor: {width} px — sugestões {}, prévia {}", if side { "à vista" } else { "recolhidas" }, if preview { "à vista" } else { "recolhida" });
            }
        }
        self.preview.set_visible(preview);
        self.divider.set_visible(preview);
    }

    fn schedule_suggestions(self: &Rc<Self>) {
        if let Some(t) = self.suggest_timer.borrow_mut().take() {
            t.remove();
        }
        // (the panel out of view: no model at work for it)
        if !self.side.is_visible() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if let Some(e) = w.upgrade() {
                e.suggest_timer.borrow_mut().take();
                e.ask_suggestions();
            }
        });
        *self.suggest_timer.borrow_mut() = Some(id);
    }

    fn ask_suggestions(self: &Rc<Self>) {
        let Some(d) = self.docs.borrow().get(self.current.get()).cloned() else { return };
        let text = d.text();
        // (Agents › Settings › General › Suggestions while you work: off, the model is not asked)
        if text.trim().len() < 12 || !crate::agent_prefs::load().suggestions {
            self.show_suggestions(Vec::new());
            return;
        }
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("sugestoes-texto").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(text.chars().take(4000).collect::<String>().as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            Some(v["sugestoes"].as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).take(4).collect::<Vec<_>>())
        }, move |s: Option<Vec<String>>| {
            if let Some(e) = w.upgrade() {
                e.show_suggestions(s.unwrap_or_default());
            }
        });
    }

    fn show_suggestions(self: &Rc<Self>, list: Vec<String>) {
        while let Some(c) = self.suggestions.first_child() {
            self.suggestions.remove(&c);
        }
        for s in &list {
            let b = gtk::Button::new();
            b.add_css_class("ed-sugestao");
            let row = caixa(Orientation::Horizontal, 8, "");
            let l = rotulo(s, "ed-sugestao-texto");
            l.set_hexpand(true);
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_max_width_chars(36);
            l.set_width_chars(1);
            row.append(&l);
            row.append(&ui_icon("chevron", "#8E8F99", 10));
            b.set_child(Some(&row));
            let (w, q) = (Rc::downgrade(self), s.clone());
            b.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.to_agent(&q);
                }
            });
            self.suggestions.append(&b);
        }
        if debug() {
            eprintln!("debug editor: {} sugestões", list.len());
        }
    }

    /// A request about this text, to the agent's bar (the text goes with it).
    fn to_agent(&self, request: &str) {
        let Some(d) = self.docs.borrow().get(self.current.get()).cloned() else { return };
        let Some(s) = self.shell.upgrade() else { return };
        let pedido = format!("{request}\n\n{}\n{}", trf("The text ({file}):", &[("file", d.title())]), d.text());
        s.right_bar().ask_agent(&pedido);
        if debug() {
            eprintln!("debug editor: ao agente «{request}»");
        }
    }

    fn action(self: &Rc<Self>, a: &str) {
        match a {
            "abrir" => {
                // the system's chooser, in the folder of the file in view
                let pasta = self.docs.borrow().get(self.current.get()).and_then(|d| d.path.borrow().as_ref().and_then(|p| p.parent().map(|x| x.to_path_buf())));
                let filtros = vec![Filtro::de(tr("Text files"), &["text/plain", "text/*"]), Filtro::de(tr("All files"), &["*"])];
                let w = Rc::downgrade(self);
                seletor::abrir_arquivo(tr("Text Editor"), tr("Open file"), None, filtros, pasta, move |p| {
                    if let Some(e) = w.upgrade() {
                        e.open_file(&p);
                    }
                });
            }
            "salvar" => {
                let i = self.current.get();
                if self.docs.borrow().get(i).is_some_and(|d| d.path.borrow().is_some()) {
                    self.save(i);
                } else {
                    self.save_as(i, false, false);
                }
            }
            _ => self.save_as(self.current.get(), false, false),
        }
    }

    fn save(&self, i: usize) -> bool {
        let Some(d) = self.docs.borrow().get(i).cloned() else { return false };
        let Some(p) = d.path.borrow().clone() else { return false };
        let text = d.text();
        match std::fs::write(&p, &text) {
            Ok(()) => {
                *d.saved.borrow_mut() = text;
                d.saved_at.set(glib::DateTime::now_local().ok().map(|t| t.to_unix()));
                self.update_titles();
                if debug() {
                    eprintln!("debug editor: salvou {}", p.display());
                }
                true
            }
            Err(err) => {
                eprintln!("kalango-shell: editor: salvar {}: {err}", p.display());
                false
            }
        }
    }

    /// Saving to a place chosen in the system's chooser; then, if asked, the tab closes (and the window).
    fn save_as(self: &Rc<Self>, i: usize, then_close: bool, then_close_window: bool) {
        let Some(d) = self.docs.borrow().get(i).cloned() else { return };
        let pasta = d.path.borrow().as_ref().and_then(|p| p.parent().map(|x| x.to_path_buf()));
        let w = Rc::downgrade(self);
        seletor::salvar_arquivo(tr("Text Editor"), tr("Save as"), &d.suggested_name(), pasta, move |p| {
            let Some(e) = w.upgrade() else { return };
            *d.path.borrow_mut() = Some(p);
            let Some(i) = e.docs.borrow().iter().position(|x| Rc::ptr_eq(x, &d)) else { return };
            if e.save(i) && then_close {
                e.close_tab_now(i);
                if then_close_window {
                    e.window.close();
                }
            }
        });
    }

    fn close_tab(self: &Rc<Self>, i: usize) {
        let changed = self.docs.borrow().get(i).is_some_and(|d| d.changed());
        if changed {
            self.select(i);
            self.ask_to_save(i, false);
        } else {
            self.close_tab_now(i);
        }
    }

    /// "Salvar as alterações?" (design "Editor de texto · salvar alterações"): inside the editor's window — the
    /// question is the app's, so only its window dims; what would be lost is said in lines. Salvar (↵, focused), Não
    /// salvar, Cancelar (Esc, or a click outside).
    fn ask_to_save(self: &Rc<Self>, i: usize, closing_window: bool) {
        let Some(d) = self.docs.borrow().get(i).cloned() else { return };
        let old = self.asking.borrow_mut().take();
        if let Some(close) = old {
            close();
        }
        let dim = caixa(Orientation::Vertical, 0, "arq-escurecer");
        dim.set_hexpand(true);
        dim.set_vexpand(true);
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.set_size_request(520, -1);
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        let top = caixa(Orientation::Horizontal, 14, "");
        let alert = caixa(Orientation::Horizontal, 0, "arq-dialogo-alerta");
        alert.set_size_request(44, 44);
        alert.set_valign(Align::Start);
        // (44 × 44 as drawn: the icon centred inside does not widen it)
        alert.set_hexpand(false);
        alert.set_halign(Align::Start);
        let icon = ui_icon("alerta", "#E5C98C", 20);
        icon.set_hexpand(true);
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);
        alert.append(&icon);
        top.append(&alert);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let t = rotulo(&trf("Save the changes to “{file}”?", &[("file", d.title())]), "arq-dialogo-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(1);
        col.append(&t);
        let sub = rotulo(tr("If you don't save, they are lost."), "arq-sub");
        sub.set_xalign(0.0);
        col.append(&sub);
        top.append(&col);
        card.append(&top);
        let what = d.what_changed();
        let changed = caixa(Orientation::Horizontal, 0, "ed-mudou");
        let cl = rotulo(&what, "ed-mudou-texto");
        cl.set_xalign(0.0);
        cl.set_wrap(true);
        cl.set_max_width_chars(1);
        cl.set_hexpand(true);
        changed.append(&cl);
        card.append(&changed);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_halign(Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("arq-acao-texto");
        let discard = gtk::Button::with_label(tr("Don't save"));
        discard.add_css_class("arq-acao");
        let save = gtk::Button::with_label(tr("Save"));
        save.add_css_class("arq-acao-principal");
        buttons.append(&cancel);
        buttons.append(&discard);
        buttons.append(&save);
        card.append(&buttons);
        self.over.add_overlay(&dim);
        self.over.add_overlay(&card);
        let (ow, dw, cw, w) = (self.over.downgrade(), dim.downgrade(), card.downgrade(), Rc::downgrade(self));
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if let (Some(o), Some(dm), Some(c)) = (ow.upgrade(), dw.upgrade(), cw.upgrade()) {
                if c.parent().is_some() {
                    o.remove_overlay(&c);
                    o.remove_overlay(&dm);
                }
            }
            if let Some(e) = w.upgrade() {
                e.asking.borrow_mut().take();
                e.view.grab_focus();
            }
        });
        *self.asking.borrow_mut() = Some(close.clone());
        let outside = gtk::GestureClick::new();
        let c = close.clone();
        outside.connect_released(move |_, _, _, _| c());
        dim.add_controller(outside);
        let c = close.clone();
        cancel.connect_clicked(move |_| c());
        let (c, w, dd) = (close.clone(), Rc::downgrade(self), d.clone());
        discard.connect_clicked(move |_| {
            c();
            let Some(e) = w.upgrade() else { return };
            if debug() {
                eprintln!("debug editor: não salvou «{}»", dd.title());
            }
            let i = e.docs.borrow().iter().position(|x| Rc::ptr_eq(x, &dd));
            if let Some(i) = i {
                e.close_tab_now(i);
                if closing_window {
                    e.window.close();
                }
            }
        });
        let (c, w, dd) = (close.clone(), Rc::downgrade(self), d.clone());
        save.connect_clicked(move |_| {
            c();
            let Some(e) = w.upgrade() else { return };
            let Some(i) = e.docs.borrow().iter().position(|x| Rc::ptr_eq(x, &dd)) else { return };
            if dd.path.borrow().is_some() {
                if e.save(i) {
                    e.close_tab_now(i);
                    if closing_window {
                        e.window.close();
                    }
                }
            } else {
                e.save_as(i, true, closing_window);
            }
        });
        crate::central::posicao_debug("editor cancelar", cancel.upcast_ref());
        crate::central::posicao_debug("editor descartar", discard.upcast_ref());
        crate::central::posicao_debug("editor salvar", save.upcast_ref());
        save.grab_focus();
        if debug() {
            eprintln!("debug editor: pergunta salvar «{}» — {what}", d.title());
        }
    }

    fn close_tab_now(self: &Rc<Self>, i: usize) {
        let d = {
            let mut docs = self.docs.borrow_mut();
            if i >= docs.len() {
                return;
            }
            docs.remove(i)
        };
        self.tabs.remove(&d.tab);
        let n = self.docs.borrow().len();
        if n == 0 {
            self.new_doc();
        } else {
            self.select(i.min(n - 1));
        }
    }
}

/// The markdown's marks in the source: the headings, and code.
fn color(b: &gtk::TextBuffer) {
    let (start, end) = b.bounds();
    b.remove_all_tags(&start, &end);
    let text = b.text(&start, &end, false).to_string();
    let mut in_code = false;
    for (n, line) in text.lines().enumerate() {
        let Some(ls) = b.iter_at_line(n as i32) else { continue };
        let mut le = ls;
        if !le.ends_line() {
            le.forward_to_line_end();
        }
        let t = line.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            b.apply_tag_by_name("codigo", &ls, &le);
            continue;
        }
        if in_code {
            b.apply_tag_by_name("codigo", &ls, &le);
        } else if t.starts_with('#') {
            // (as in the design: the headings in the accent, the rest of the source in the text's colour)
            b.apply_tag_by_name("titulo", &ls, &le);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::line_changes;

    #[test]
    fn lines_new_changed_and_gone() {
        assert_eq!(line_changes("a\nb\nc", "a\nb\nc"), (0, 0, 0));
        assert_eq!(line_changes("a\nb\nc", "a\nB\nc\nd"), (1, 1, 0));
        assert_eq!(line_changes("# T\n\n- um\n- dois\n", "# T\n\n- um\n- dois\n- três\n- quatro\n"), (2, 0, 0));
        assert_eq!(line_changes("a\nb\nc\nd", "a\nd"), (0, 0, 2));
        assert_eq!(line_changes("", "x"), (1, 0, 0));
    }
}
