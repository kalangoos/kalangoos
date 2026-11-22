//! KalangoOS's own Disks app (design "Discos", 59940; app id os.kalango.Discos; `kalango-shell discos`): the drives
//! (disks_data.rs: UDisks2) on the left with their health; the one chosen on the right — its partitions as a bar
//! (click one), its SMART health, its read and write rates live, and the actions (check, make an image, resize,
//! format…), each confirmed with exactly what it does. The agent watches: a drive wearing out that holds the
//! backups gets a card offering to copy them to a healthy one; and a request ("clone o NVMe para o SSD") becomes
//! the action it means, shown to confirm.

use crate::conversa::ui_icon;
use crate::disks_data::{self as data, Action, Drive, Status};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Seconds of rates kept for the sparkline.
const HISTORY: usize = 60;

pub struct DisksApp {
    window: gtk::ApplicationWindow,
    list: gtk::Box,
    title: gtk::Label,
    model: gtk::Label,
    bar: gtk::DrawingArea,
    sel_line: gtk::Label,
    used_line: gtk::Label,
    h_state: gtk::Label,
    h_wear: gtk::Label,
    h_temp: gtk::Label,
    h_hours: gtk::Label,
    r_read: gtk::Label,
    r_write: gtk::Label,
    spark: gtk::DrawingArea,
    alert: gtk::Box,
    alert_text: gtk::Label,
    alert_b: gtk::Button,
    confirm: gtk::Box,
    confirm_title: gtk::Label,
    confirm_text: gtk::Label,
    confirm_go: gtk::Button,
    status: gtk::Label,
    ask: gtk::Entry,
    empty: gtk::Label,
    drives: RefCell<Vec<Drive>>,
    chosen: Cell<usize>,
    part: Cell<usize>,
    pending: RefCell<Option<Action>>,
    rates: RefCell<HashMap<String, VecDeque<(f64, f64)>>>,
    last_stats: RefCell<HashMap<String, (u64, u64)>>,
    busy: Cell<bool>,
    rate_seen: Cell<bool>,
    shell: Weak<Shell>,
}

thread_local! {
    static APP: RefCell<Option<Rc<DisksApp>>> = const { RefCell::new(None) };
}

pub fn open(shell: &Rc<Shell>) {
    let a = APP.with(|a| a.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let a = DisksApp::new(&app, shell);
        APP.with(|x| *x.borrow_mut() = Some(a.clone()));
        a
    });
    if !a.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        a.window.set_default_size(w, h);
    }
    a.window.present();
    a.reload();
}

fn row(name: &str, value_class: &str) -> (gtk::Box, gtk::Label) {
    let r = caixa(Orientation::Horizontal, 8, "dsc-linha");
    let n = rotulo(name, "dsc-nome");
    n.set_hexpand(true);
    n.set_xalign(0.0);
    let v = rotulo("—", value_class);
    v.set_xalign(1.0);
    v.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    r.append(&n);
    r.append(&v);
    (r, v)
}

fn card(class: &str) -> gtk::Box {
    caixa(Orientation::Vertical, 10, class)
}

fn heading(glyph: Option<&str>, text: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 8, "");
    if let Some(g) = glyph {
        r.append(&ui_icon(g, "#A5A1E2", 14));
    }
    let l = rotulo(text, "dsc-secao");
    l.set_xalign(0.0);
    r.append(&l);
    r
}

fn action_button(glyph: &str, label: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    let r = caixa(Orientation::Horizontal, 8, "");
    if !glyph.is_empty() {
        r.append(&ui_icon(glyph, "#E2E2E6", 14));
    }
    r.append(&rotulo(label, ""));
    b.set_child(Some(&r));
    b
}

impl DisksApp {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<DisksApp> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Disks")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Discos");
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden)
        crate::superficies::release_resources_when_hidden(&window, || true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        bar.append(&crate::apps::imagem_kalango("discos", 24));
        let name = rotulo(tr("Disks"), "img-titulo");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        bar.append(&name);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(340, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Search or ask: “clone the NVMe to the SSD”")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ab = gtk::Overlay::new();
        ab.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ab.add_overlay(&key);
        bar.append(&ab);
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
        let line = gtk::Separator::new(Orientation::Horizontal);
        line.add_css_class("vid-linha");

        // the drives
        let left = caixa(Orientation::Vertical, 10, "dsc-unidades");
        left.set_size_request(380, -1);
        let lt = rotulo(tr("DRIVES"), "dsc-secao");
        lt.set_xalign(0.0);
        left.append(&lt);
        let list = caixa(Orientation::Vertical, 10, "");
        let ls = gtk::ScrolledWindow::new();
        ls.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        ls.set_vexpand(true);
        ls.set_child(Some(&list));
        left.append(&ls);

        // the one chosen
        let detail = caixa(Orientation::Vertical, 14, "dsc-detalhe");
        detail.set_hexpand(true);
        let title = rotulo("", "dsc-titulo");
        title.set_xalign(0.0);
        detail.append(&title);
        let model = rotulo("", "dsc-modelo");
        model.set_xalign(0.0);
        model.set_margin_top(-6);
        detail.append(&model);
        let parts = card("dsc-cartao");
        parts.append(&heading(None, tr("PARTITIONS")));
        let pbar = gtk::DrawingArea::new();
        pbar.set_content_height(44);
        pbar.set_hexpand(true);
        pbar.set_cursor_from_name(Some("pointer"));
        pbar.set_has_tooltip(true);
        crate::central::posicao_debug_sempre("discos particoes", pbar.upcast_ref());
        parts.append(&pbar);
        let (r1, sel_line) = row(tr("Selected"), "dsc-valor-forte");
        let (r2, used_line) = row(tr("Used"), "dsc-valor-forte");
        parts.append(&r1);
        parts.append(&r2);
        detail.append(&parts);
        let two = caixa(Orientation::Horizontal, 14, "");
        two.set_homogeneous(true);
        let health = card("dsc-cartao");
        health.append(&heading(Some("escudo"), tr("HEALTH (SMART)")));
        let (a, h_state) = row(tr("State"), "dsc-valor");
        let (b, h_wear) = row(tr("Life used"), "dsc-valor");
        let (c, h_temp) = row(tr("Temperature"), "dsc-valor");
        let (d, h_hours) = row(tr("Hours on"), "dsc-valor");
        for x in [&a, &b, &c, &d] {
            health.append(x);
        }
        let speed = card("dsc-cartao");
        speed.append(&heading(Some("raio"), tr("SPEED")));
        let (e, r_read) = row(tr("Reading"), "dsc-valor-forte");
        let (f, r_write) = row(tr("Writing"), "dsc-valor-forte");
        speed.append(&e);
        speed.append(&f);
        let spark = gtk::DrawingArea::new();
        spark.set_content_height(40);
        spark.set_content_width(300);
        spark.set_halign(Align::Start);
        speed.append(&spark);
        two.append(&health);
        two.append(&speed);
        detail.append(&two);
        // the actions
        let acts = caixa(Orientation::Horizontal, 8, "");
        let check = action_button("escudo", tr("Check"), "dsc-acao");
        let image = action_button("download", tr("Make an image"), "dsc-acao");
        let resize = action_button("layout", tr("Resize"), "dsc-acao");
        let mount = action_button("pasta", tr("Mount"), "dsc-acao");
        let eject = action_button("ejetar", tr("Eject"), "dsc-acao");
        let space = caixa(Orientation::Horizontal, 0, "");
        space.set_hexpand(true);
        let format = action_button("", tr("Format…"), "dsc-perigo");
        for (bt, id) in [(&check, "verificar"), (&image, "imagem"), (&resize, "redimensionar"), (&mount, "montar"), (&eject, "ejetar")] {
            crate::central::posicao_debug_sempre(&format!("discos {id}"), bt.upcast_ref());
            acts.append(bt);
        }
        acts.append(&space);
        crate::central::posicao_debug_sempre("discos formatar", format.upcast_ref());
        acts.append(&format);
        detail.append(&acts);
        // the agent's card
        let alert = caixa(Orientation::Horizontal, 12, "dsc-alerta");
        alert.append(&ui_icon("alerta", "#E5C98C", 16));
        let alert_text = rotulo("", "dsc-alerta-texto");
        alert_text.set_wrap(true);
        alert_text.set_xalign(0.0);
        alert_text.set_hexpand(true);
        alert_text.set_max_width_chars(1);
        alert.append(&alert_text);
        let alert_b = gtk::Button::with_label(tr("Copy the backups"));
        alert_b.add_css_class("dsc-alerta-botao");
        alert_b.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("discos copiar backups", alert_b.upcast_ref());
        alert.append(&alert_b);
        alert.set_visible(false);
        detail.append(&alert);
        // a confirmation: exactly what will be done, before it is
        let confirm = caixa(Orientation::Vertical, 8, "dsc-confirmar");
        let confirm_title = rotulo("", "dsc-confirmar-titulo");
        confirm_title.set_xalign(0.0);
        let confirm_text = rotulo("", "dsc-alerta-texto");
        confirm_text.set_wrap(true);
        confirm_text.set_xalign(0.0);
        confirm_text.set_max_width_chars(1);
        let cr = caixa(Orientation::Horizontal, 8, "");
        cr.set_halign(Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("dsc-acao");
        let confirm_go = gtk::Button::with_label(tr("Do it"));
        confirm_go.add_css_class("dsc-perigo");
        crate::central::posicao_debug_sempre("discos confirmar", confirm_go.upcast_ref());
        cr.append(&cancel);
        cr.append(&confirm_go);
        confirm.append(&confirm_title);
        confirm.append(&confirm_text);
        confirm.append(&cr);
        confirm.set_visible(false);
        // (it floats at the foot of the drive's side: always in view, whatever the window's size)
        confirm.set_valign(Align::End);
        confirm.set_margin_start(18);
        confirm.set_margin_end(24);
        confirm.set_margin_bottom(20);
        let status = rotulo("", "dsc-modelo");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_max_width_chars(1);
        detail.append(&status);
        let ds = gtk::ScrolledWindow::new();
        ds.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        ds.set_hexpand(true);
        ds.set_child(Some(&detail));
        let empty = rotulo(tr("Reading the drives…"), "img-vazio");
        empty.set_halign(Align::Center);
        empty.set_valign(Align::Center);
        let over = gtk::Overlay::new();
        over.set_child(Some(&ds));
        over.add_overlay(&empty);
        over.add_overlay(&confirm);
        over.set_hexpand(true);
        let body = caixa(Orientation::Horizontal, 0, "dsc-corpo");
        body.set_vexpand(true);
        body.append(&left);
        body.append(&over);
        let root = caixa(Orientation::Vertical, 0, "doc-fundo");
        root.append(&handle);
        root.append(&line);
        root.append(&body);
        window.set_child(Some(&root));
        let a = Rc::new(DisksApp {
            window, list, title, model, bar: pbar, sel_line, used_line, h_state, h_wear, h_temp, h_hours, r_read, r_write, spark, alert, alert_text,
            alert_b, confirm, confirm_title, confirm_text, confirm_go, status, ask, empty, drives: RefCell::default(), chosen: Cell::new(0),
            part: Cell::new(usize::MAX), pending: RefCell::default(), rates: RefCell::default(), last_stats: RefCell::default(), busy: Cell::new(false),
            rate_seen: Cell::new(false), shell: Rc::downgrade(shell),
        });
        a.wire(&check, &image, &resize, &mount, &eject, &format, &cancel, &items, &pop);
        a
    }

    #[allow(clippy::too_many_arguments)]
    fn wire(self: &Rc<Self>, check: &gtk::Button, image: &gtk::Button, resize: &gtk::Button, mount: &gtk::Button, eject: &gtk::Button,
        format: &gtk::Button, cancel: &gtk::Button, items: &gtk::Box, pop: &gtk::Popover) {
        let w = Rc::downgrade(self);
        self.bar.set_draw_func(move |_, cr, width, height| {
            if let Some(a) = w.upgrade() {
                a.draw_bar(cr, f64::from(width), f64::from(height));
            }
        });
        let w = Rc::downgrade(self);
        self.spark.set_draw_func(move |_, cr, width, height| {
            if let Some(a) = w.upgrade() {
                a.draw_spark(cr, f64::from(width), f64::from(height));
            }
        });
        // a click on the bar chooses that partition
        let g = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        g.connect_released(move |_, _, x, _| {
            if let Some(a) = w.upgrade() {
                if let Some(i) = a.part_at(x) {
                    a.part.set(i);
                    a.show_part();
                    a.bar.queue_draw();
                }
            }
        });
        self.bar.add_controller(g);
        let w = Rc::downgrade(self);
        self.bar.connect_query_tooltip(move |_, x, _, _, tip| {
            let Some(a) = w.upgrade() else { return false };
            let Some(i) = a.part_at(f64::from(x)) else { return false };
            let d = a.drives.borrow().get(a.chosen.get()).cloned();
            let Some(p) = d.and_then(|d| d.parts.get(i).cloned()) else { return false };
            tip.set_text(Some(&format!("{} · {} · {}", p.title(&trf("Partition {n}", &[("n", p.number.to_string())])), p.device, data::amount(p.size))));
            true
        });
        type Handler = Box<dyn Fn(&Rc<DisksApp>)>;
        let pairs: Vec<(&gtk::Button, Handler)> = vec![
            (check, Box::new(|a| a.ask_check())),
            (image, Box::new(|a| a.ask_image())),
            (resize, Box::new(|a| a.ask_resize())),
            (mount, Box::new(|a| a.ask_mount())),
            (eject, Box::new(|a| a.ask_eject())),
            (format, Box::new(|a| a.ask_format())),
            (cancel, Box::new(|a| a.cancel())),
            (&self.confirm_go, Box::new(|a| a.run_pending())),
            (&self.alert_b, Box::new(|a| a.copy_backups())),
        ];
        for (b, f) in pairs {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    f(&a);
                }
            });
        }
        let add = |label: &str, f: Box<dyn Fn(&Rc<DisksApp>)>| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            // (the popover weakly: the button is inside it, and a strong one from its own child kept both for good)
            let (w, pop) = (Rc::downgrade(self), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
                if let Some(a) = w.upgrade() {
                    f(&a);
                }
            });
            items.append(&b);
        };
        add(tr("Read the drives again"), Box::new(|a| a.reload()));
        add(tr("Open in Files"), Box::new(|a| {
            let m = a.selected_part().and_then(|p| p.mount);
            if let (Some(m), Some(s)) = (m, a.shell.upgrade()) {
                s.abrir_arquivos(Some(std::path::Path::new(&m)));
            }
        }));
        // the request: the agent turns it into one of the actions, shown to confirm
        let w = Rc::downgrade(self);
        self.ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(a), false) = (w.upgrade(), t.is_empty()) {
                e.set_text("");
                gtk::prelude::GtkWindowExt::set_focus(&a.window, None::<&gtk::Widget>);
                a.request(t);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(a) = w.upgrade() else { return glib::Propagation::Proceed };
            if m.contains(gdk::ModifierType::CONTROL_MASK) && k == gdk::Key::k {
                a.ask.grab_focus();
                return glib::Propagation::Stop;
            }
            if k == gdk::Key::Escape && a.confirm.is_visible() {
                a.cancel();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Discos").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| {
            app_id(j);
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
        // the rates, every second (while the window is open)
        let w = Rc::downgrade(self);
        crate::superficies::every_while_mapped(&self.window, Duration::from_secs(1), move || {
            let Some(a) = w.upgrade() else { return false };
            if a.window.is_visible() {
                a.sample_rates();
            }
            true
        });
    }

    fn reload(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(data::read, move |r| {
            let Some(a) = w.upgrade() else { return };
            match r {
                Ok(ds) => {
                    if debug() {
                        eprintln!("debug discos: {} unidades: {}", ds.len(), ds.iter().map(|d| format!("{} ({:?})", d.title(), d.status())).collect::<Vec<_>>().join("; "));
                    }
                    a.empty.set_visible(ds.is_empty());
                    a.empty.set_text(tr("No drives found."));
                    // (the drive and partition in view stay in view)
                    let keep = a.drives.borrow().get(a.chosen.get()).map(|d| d.object.clone());
                    let keep_part = a.selected_part().map(|p| p.object);
                    *a.drives.borrow_mut() = ds;
                    let i = keep.and_then(|k| a.drives.borrow().iter().position(|d| d.object == k)).unwrap_or(0);
                    a.fill_list();
                    a.choose(i);
                    let j = keep_part.and_then(|k| a.drive().and_then(|d| d.parts.iter().position(|p| p.object == k)));
                    if let Some(j) = j.filter(|j| *j != a.part.get()) {
                        a.part.set(j);
                        a.show_part();
                        a.bar.queue_draw();
                    }
                    a.watch_health();
                }
                Err(e) => {
                    a.empty.set_text(&trf("The drives could not be read: {error}", &[("error", e)]));
                    a.empty.set_visible(true);
                }
            }
        });
    }

    fn fill_list(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let drives = self.drives.borrow().clone();
        for (i, d) in drives.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("dsc-unidade");
            let r = caixa(Orientation::Horizontal, 12, "");
            let ic = caixa(Orientation::Horizontal, 0, "dsc-unidade-icone");
            let g = ui_icon("pasta", "#8E8F99", 16);
            g.set_hexpand(true);
            ic.append(&g);
            ic.set_valign(Align::Center);
            ic.set_hexpand(false);
            r.append(&ic);
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let t = rotulo(&d.title(), "dsc-unidade-nome");
            t.set_xalign(0.0);
            let s = rotulo(&self.drive_line(d), "dsc-unidade-linha");
            s.set_xalign(0.0);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            s.set_max_width_chars(1);
            col.append(&t);
            col.append(&s);
            r.append(&col);
            let (chip, class) = match d.status() {
                Status::Healthy => (tr("healthy"), "dsc-chip-ok"),
                Status::Attention => (tr("attention"), "dsc-chip-atencao"),
                Status::Removable => (tr("removable"), "dsc-chip-removivel"),
            };
            let c = rotulo(chip, class);
            c.set_valign(Align::Center);
            r.append(&c);
            b.set_child(Some(&r));
            crate::central::posicao_debug_sempre(&format!("discos unidade {}", i + 1), b.upcast_ref());
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    a.choose(i);
                }
            });
            self.list.append(&b);
        }
    }

    /// "sistema · criptografado", "dados · 80% cheio", "backup · 14.200 h de uso", "pendrive · FAT32".
    fn drive_line(&self, d: &Drive) -> String {
        if d.removable {
            let fs = d.parts.iter().map(|p| fs_name(&p.fs)).find(|f| !f.is_empty()).unwrap_or_default();
            return [tr("pen drive"), fs.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
        }
        if d.is_system() {
            return if d.encrypted() { format!("{} · {}", tr("system"), tr("encrypted")) } else { tr("system").to_string() };
        }
        if d.backup_mount().is_some() {
            return match d.health.hours {
                Some(h) => format!("{} · {}", tr("backup"), trf("{h} h of use", &[("h", data::thousands(h))])),
                None => tr("backup").to_string(),
            };
        }
        match d.fullness() {
            Some(f) => format!("{} · {}", tr("data"), trf("{n}% full", &[("n", ((f * 100.0).round() as u32).to_string())])),
            None => tr("data").to_string(),
        }
    }

    fn choose(self: &Rc<Self>, i: usize) {
        let Some(d) = self.drives.borrow().get(i).cloned() else { return };
        self.chosen.set(i);
        let mut k = 0;
        while let Some(c) = self.list.observe_children().item(k).and_downcast::<gtk::Widget>() {
            if k as usize == i {
                c.add_css_class("atual");
            } else {
                c.remove_css_class("atual");
            }
            k += 1;
        }
        self.title.set_text(&d.title());
        self.model.set_text(&d.model_line(tr("serial number hidden")));
        // the partition in use by the system (or the biggest) is the one chosen at first
        let p = d.parts.iter().position(|p| p.mount.as_deref() == Some("/")).or_else(|| (0..d.parts.len()).max_by_key(|&j| d.parts[j].size)).unwrap_or(usize::MAX);
        self.part.set(p);
        self.show_part();
        let h = &d.health;
        let (st, class) = match d.status() {
            Status::Attention => (tr("attention"), "dsc-valor-atencao"),
            _ if !h.smart => (tr("no SMART"), "dsc-valor"),
            _ => (tr("healthy"), "dsc-valor-ok"),
        };
        self.h_state.set_text(st);
        for c in ["dsc-valor", "dsc-valor-ok", "dsc-valor-atencao"] {
            self.h_state.remove_css_class(c);
        }
        self.h_state.add_css_class(class);
        self.h_wear.set_text(&h.wear.map(|w| format!("{w}%")).unwrap_or_else(|| "—".into()));
        self.h_temp.set_text(&h.temp_c.map(|t| format!("{} °C", t.round())).unwrap_or_else(|| "—".into()));
        self.h_hours.set_text(&h.hours.map(|x| format!("{} h", data::thousands(x))).unwrap_or_else(|| "—".into()));
        self.bar.queue_draw();
        self.spark.queue_draw();
        self.rate_seen.set(false);
        self.cancel();
        if debug() {
            eprintln!("debug discos: escolhida «{}» ({} partições)", d.title(), d.parts.len());
        }
    }

    fn selected_part(&self) -> Option<data::Part> {
        self.drives.borrow().get(self.chosen.get()).and_then(|d| d.parts.get(self.part.get()).cloned())
    }

    fn show_part(&self) {
        let Some(p) = self.selected_part() else {
            self.sel_line.set_text("—");
            self.used_line.set_text("—");
            return;
        };
        let nth = trf("Partition {n}", &[("n", p.number.to_string())]);
        let mut bits = vec![p.title(&nth)];
        if !p.fs.is_empty() {
            bits.push(fs_name(&p.fs));
        }
        if !p.encrypted.is_empty() {
            bits.push(p.encrypted.clone());
        }
        bits.push(match &p.mount {
            Some(m) => trf("mounted at {path}", &[("path", m.clone())]),
            None => tr("not mounted").to_string(),
        });
        self.sel_line.set_text(&bits.join(" · "));
        self.used_line.set_text(&match p.used {
            Some(u) => trf("{used} of {total}", &[("used", data::amount(u)), ("total", data::amount(p.size))]),
            None => data::amount(p.size),
        });
        if debug() {
            eprintln!("debug discos: partição «{}»", bits.join(" · "));
        }
    }

    /// The bar's partitions: each as wide as its share, at least 60 px (the EFI's), with 3 px between.
    fn layout_bar(&self, w: f64) -> Vec<(f64, f64)> {
        let d = self.drives.borrow().get(self.chosen.get()).cloned().unwrap_or_default();
        let n = d.parts.len();
        if n == 0 {
            return Vec::new();
        }
        let total: u64 = d.parts.iter().map(|p| p.size).sum::<u64>().max(1);
        let avail = w - 3.0 * (n as f64 - 1.0);
        let min = 60.0f64.min(avail / n as f64);
        let raw: Vec<f64> = d.parts.iter().map(|p| p.size as f64 / total as f64 * avail).collect();
        let small: f64 = raw.iter().filter(|r| **r < min).count() as f64 * min;
        let big_sum: f64 = raw.iter().filter(|r| **r >= min).sum::<f64>().max(1.0);
        let mut x = 0.0;
        raw.iter().map(|r| {
            let width = if *r < min { min } else { r / big_sum * (avail - small) };
            let seg = (x, width);
            x += width + 3.0;
            seg
        }).collect()
    }

    fn part_at(&self, x: f64) -> Option<usize> {
        self.layout_bar(f64::from(self.bar.width())).iter().position(|(a, w)| x >= *a && x <= a + w)
    }

    fn draw_bar(&self, cr: &cairo::Context, w: f64, h: f64) {
        let d = self.drives.borrow().get(self.chosen.get()).cloned().unwrap_or_default();
        let sel = self.part.get();
        let rounded = |x: f64, y: f64, w: f64, h: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
        };
        for (i, (x, pw)) in self.layout_bar(w).iter().enumerate() {
            let p = &d.parts[i];
            let (bg, fg) = if p.efi { ("#E5C98C", "#1C1D24") } else if i == sel { ("#4D4A70", "#E2E2E6") } else { ("#2A2B33", "#E2E2E6") };
            let (r, g, b) = crate::tema::rgb(bg);
            cr.set_source_rgb(r, g, b);
            rounded(*x, 0.0, *pw, h, 3.0);
            let _ = cr.fill();
            if i == sel && !p.efi {
                let (r, g, b) = crate::tema::rgb("#A5A1E2");
                cr.set_source_rgba(r, g, b, 0.9);
                cr.set_line_width(1.0);
                rounded(x + 0.5, 0.5, pw - 1.0, h - 1.0, 3.0);
                let _ = cr.stroke();
            }
            // its name and size, when it fits
            let nth = trf("Partition {n}", &[("n", p.number.to_string())]);
            let label = if p.efi { "EFI".to_string() } else { format!("{} · {}", p.title(&nth), data::amount(p.size)) };
            let layout = pangocairo::functions::create_layout(cr);
            let mut fd = gtk::pango::FontDescription::from_string("Inter 12");
            fd.set_weight(gtk::pango::Weight::Medium);
            layout.set_font_description(Some(&fd));
            layout.set_text(&label);
            layout.set_width(((pw - 12.0).max(1.0) * f64::from(gtk::pango::SCALE)) as i32);
            layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
            layout.set_alignment(gtk::pango::Alignment::Center);
            let (_, lh) = layout.pixel_size();
            let (r, g, b) = crate::tema::rgb(fg);
            cr.set_source_rgb(r, g, b);
            cr.move_to(x + 6.0, (h - f64::from(lh)) / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
    }

    fn sample_rates(&self) {
        let now = data::diskstats();
        let mut last = self.last_stats.borrow_mut();
        let mut rates = self.rates.borrow_mut();
        for (dev, (r, w)) in &now {
            if let Some((r0, w0)) = last.get(dev) {
                let q = rates.entry(dev.clone()).or_default();
                q.push_back(((r.saturating_sub(*r0) * 512) as f64, (w.saturating_sub(*w0) * 512) as f64));
                while q.len() > HISTORY {
                    q.pop_front();
                }
            }
        }
        *last = now;
        drop(last);
        let dev = self.drives.borrow().get(self.chosen.get()).map(|d| d.device.clone()).unwrap_or_default();
        let (r, w) = rates.get(&dev).and_then(|q| q.back().copied()).unwrap_or((0.0, 0.0));
        if debug() && (r > 0.0 || w > 0.0) && !self.rate_seen.replace(true) {
            eprintln!("debug discos: velocidade {dev}: leitura {} MB/s, gravação {} MB/s", (r / 1e6).round(), (w / 1e6).round());
        }
        let mbs = |b: f64| if b >= 1e6 { format!("{} MB/s", data::thousands((b / 1e6).round() as u64)) } else { format!("{} KB/s", (b / 1e3).round()) };
        self.r_read.set_text(&mbs(r));
        self.r_write.set_text(&mbs(w));
        drop(rates);
        self.spark.queue_draw();
    }

    /// The design's sparkline: the reading and writing together, filled under.
    fn draw_spark(&self, cr: &cairo::Context, w: f64, h: f64) {
        let dev = self.drives.borrow().get(self.chosen.get()).map(|d| d.device.clone()).unwrap_or_default();
        let rates = self.rates.borrow();
        let Some(q) = rates.get(&dev).filter(|q| q.len() >= 2) else { return };
        let v: Vec<f64> = q.iter().map(|(r, w)| r + w).collect();
        let max = v.iter().copied().fold(1.0, f64::max);
        let step = w / (HISTORY - 1) as f64;
        let x0 = w - step * (v.len() - 1) as f64;
        let pt = |i: usize, y: f64| (x0 + i as f64 * step, h - 2.0 - y / max * (h - 6.0));
        cr.move_to(x0, h);
        for (i, y) in v.iter().enumerate() {
            let (x, yy) = pt(i, *y);
            cr.line_to(x, yy);
        }
        cr.line_to(w, h);
        cr.close_path();
        let (r, g, b) = crate::tema::rgb("#A5A1E2");
        cr.set_source_rgba(r, g, b, 0.18);
        let _ = cr.fill();
        for (i, y) in v.iter().enumerate() {
            let (x, yy) = pt(i, *y);
            if i == 0 {
                cr.move_to(x, yy);
            } else {
                cr.line_to(x, yy);
            }
        }
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    }

    // ------------------------------------------------------------------------------------------- the agent

    /// A drive wearing out that keeps the backups, and a healthy one with room: the card offering to copy them.
    fn watch_health(&self) {
        let drives = self.drives.borrow().clone();
        let worn = drives.iter().find(|d| d.status() == Status::Attention && d.backup_mount().is_some());
        let dest = worn.and_then(|w| drives.iter().filter(|d| d.object != w.object && d.status() == Status::Healthy && !d.removable && !d.is_system())
            .filter(|d| d.free_space() > w.total_used()).max_by_key(|d| d.free_space()));
        match (worn, dest) {
            (Some(w), Some(d)) => {
                self.alert_text.set_text(&trf("The {worn} shows signs of wear. Your backups are on it — copy them to the {dest}?",
                    &[("worn", w.title()), ("dest", d.title())]));
                self.alert.set_visible(true);
                if debug() {
                    eprintln!("debug discos: aviso: {} gasto, backups → {}", w.title(), d.title());
                }
            }
            _ => self.alert.set_visible(false),
        }
    }

    fn copy_backups(&self) {
        let drives = self.drives.borrow().clone();
        let Some(w) = drives.iter().find(|d| d.status() == Status::Attention && d.backup_mount().is_some()) else { return };
        let Some(dest) = drives.iter().filter(|d| d.object != w.object && d.status() == Status::Healthy && !d.removable && !d.is_system())
            .filter_map(|d| d.parts.iter().filter_map(|p| p.mount.clone().zip(p.used.map(|u| p.size.saturating_sub(u)))).max_by_key(|(_, f)| *f).map(|(m, f)| (d, m, f)))
            .max_by_key(|(_, _, f)| *f) else { return };
        let Some(src) = w.backup_mount() else { return };
        let target = PathBuf::from(&dest.1).join(trf("Backups (from the {drive})", &[("drive", w.title())]));
        let origins: Vec<PathBuf> = std::fs::read_dir(&src).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n != "lost+found")).collect();
        if debug() {
            eprintln!("debug discos: copiar backups {} itens de {src} → {}", origins.len(), target.display());
        }
        let _ = std::fs::create_dir_all(&target);
        if let Some(ops) = self.shell.upgrade().and_then(|s| s.operacoes()) {
            ops.iniciar(origins, target.clone(), false);
        }
        self.alert_text.set_text(&trf("Copying the backups to {path} (the progress is on the island).", &[("path", target.display().to_string())]));
        self.alert_b.set_visible(false);
    }

    /// The drive a request's words name ("o pendrive", "o SSD", "o NVMe"), when only one fits.
    fn drive_named(&self, text: &str) -> Option<usize> {
        let t = text.to_lowercase();
        let drives = self.drives.borrow();
        let fits: Vec<usize> = drives.iter().enumerate().filter(|(_, d)| {
            let words = format!("{} {} {}", d.title(), self.drive_line(d), if d.removable { "pendrive pen drive usb removível" } else { "" }).to_lowercase();
            words.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
                .any(|w| t.contains(w))
        }).map(|(i, _)| i).collect();
        (fits.len() == 1).then(|| fits[0])
    }

    fn request(self: &Rc<Self>, text: String) {
        let drives = self.drives.borrow().clone();
        let input = serde_json::json!({
            "pedido": text,
            "unidades": drives.iter().enumerate().map(|(i, d)| serde_json::json!({
                "id": i, "nome": d.title(), "linha": self.drive_line(d), "removivel": d.removable,
                "particoes": d.parts.iter().enumerate().map(|(j, p)| serde_json::json!({"id": j, "nome": p.title(&format!("{}", p.number)),
                    "sistema_de_arquivos": p.fs, "montada_em": p.mount, "tamanho": data::amount(p.size)})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        self.status.set_text(tr("The agent is reading the request…"));
        let w = Rc::downgrade(self);
        let text = text.clone();
        crate::lateral::em_segundo_plano(move || ask_agent("discos-pedido", &input), move |r| {
            let Some(a) = w.upgrade() else { return };
            a.status.set_text("");
            match r {
                Ok(v) => {
                    let act = v["acao"].as_str().unwrap_or("responder").to_string();
                    // (the drive the person's words name wins — "o pendrive" is the pendrive, whatever index the model
                    // gave —; then the model's partition was for another drive and is dropped)
                    let do_modelo = v["unidade"].as_u64().map(|x| x as usize);
                    let dita = a.drive_named(&text);
                    let unit = dita.or(do_modelo);
                    let part = v["particao"].as_u64().map(|x| x as usize).filter(|_| dita.is_none() || dita == do_modelo);
                    let dest = v["destino"].as_u64().map(|x| x as usize);
                    if debug() {
                        eprintln!("debug discos: pedido → {act} (unidade {unit:?}, partição {part:?}, destino {dest:?})");
                    }
                    if let Some(u) = unit.filter(|u| *u < a.drives.borrow().len()) {
                        a.choose(u);
                        if let Some(p) = part {
                            a.part.set(p);
                            a.show_part();
                            a.bar.queue_draw();
                        }
                    }
                    match act.as_str() {
                        "verificar" => a.ask_check(),
                        "imagem" => a.ask_image(),
                        "formatar" => a.ask_format(),
                        "montar" => a.ask_mount(),
                        "ejetar" => a.ask_eject(),
                        "clonar" => a.ask_clone(dest),
                        _ => a.status.set_text(v["texto"].as_str().unwrap_or("")),
                    }
                }
                Err(e) => a.status.set_text(&trf("The agent could not answer: {error}", &[("error", e)])),
            }
        });
    }

    // ------------------------------------------------------------------------------------------- the actions

    fn show_confirm(&self, title: &str, text: &str, go: &str, danger: bool, a: Action) {
        self.confirm_title.set_text(title);
        self.confirm_text.set_text(text);
        self.confirm_go.set_label(go);
        if danger {
            self.confirm_go.add_css_class("dsc-perigo");
            self.confirm_go.remove_css_class("dsc-acao-forte");
        } else {
            self.confirm_go.remove_css_class("dsc-perigo");
            self.confirm_go.add_css_class("dsc-acao-forte");
        }
        *self.pending.borrow_mut() = Some(a);
        self.confirm.set_visible(true);
        if debug() {
            eprintln!("debug discos: confirmar «{title}»");
        }
    }

    fn cancel(&self) {
        self.pending.borrow_mut().take();
        self.confirm.set_visible(false);
    }

    fn drive(&self) -> Option<Drive> {
        self.drives.borrow().get(self.chosen.get()).cloned()
    }

    fn ask_check(&self) {
        let Some(d) = self.drive() else { return };
        let p = self.selected_part();
        let what = match p.as_ref().filter(|p| p.mount.is_none() && !p.fs.is_empty()) {
            Some(p) => trf("The file system of {part} is checked for errors (it is not changed).", &[("part", p.title(&p.device))]),
            None => trf("The {drive} runs its short self-test (about 2 minutes); the result shows in its health.", &[("drive", d.title())]),
        };
        self.show_confirm(tr("Check"), &what, tr("Check"), false, Action::Check { drive: d.object.clone(), part: p.filter(|p| p.mount.is_none()).map(|p| p.object) });
    }

    fn ask_image(&self) {
        let Some(d) = self.drive() else { return };
        let p = self.selected_part();
        let (block, name, size) = match &p {
            Some(p) => (p.object.clone(), p.title(&p.device), p.size),
            None => (d.object.clone(), d.title(), d.size),
        };
        let dir = crate::pastas::documentos();
        let file = dir.join(format!("{}.img", name.replace(['/', ' '], "-")));
        self.show_confirm(tr("Make an image"), &trf("A copy of {what} ({size}) is saved as {file}. Nothing on the drive changes.",
            &[("what", name), ("size", data::amount(size)), ("file", file.display().to_string())]), tr("Make the image"), false, Action::Image { block, file });
    }

    fn ask_resize(&self) {
        let Some(p) = self.selected_part() else { return };
        let used = p.used.unwrap_or(p.size / 2);
        // (to the space in use plus a fifth: what is kept stays, the rest frees up)
        let size = (used as f64 * 1.2) as u64;
        if size >= p.size {
            self.status.set_text(tr("It is nearly full: there is nothing to give back."));
            return;
        }
        self.show_confirm(tr("Resize"), &trf("{part} goes from {from} to {to}; its files stay, the {free} after it become free space.",
            &[("part", p.title(&p.device)), ("from", data::amount(p.size)), ("to", data::amount(size)), ("free", data::amount(p.size - size))]),
            tr("Resize"), true, Action::Resize { part: p.object, size });
    }

    fn ask_mount(&self) {
        let Some(p) = self.selected_part() else { return };
        if p.fs.is_empty() {
            self.status.set_text(tr("This partition has no file system to mount (or it is locked)."));
            return;
        }
        match p.mount.clone() {
            Some(m) => self.show_confirm(tr("Unmount"), &trf("{part} stops being reachable at {path} until it is mounted again.", &[("part", p.title(&p.device)), ("path", m)]),
                tr("Unmount"), false, Action::Unmount { part: p.object }),
            None => self.show_confirm(tr("Mount"), &trf("{part} becomes reachable in Files.", &[("part", p.title(&p.device))]), tr("Mount"), false, Action::Mount { part: p.object }),
        }
    }

    fn ask_eject(&self) {
        let Some(d) = self.drive() else { return };
        if !d.removable {
            self.status.set_text(tr("Only removable drives are ejected."));
            return;
        }
        self.show_confirm(tr("Eject"), &trf("The {drive} is unmounted and can be taken out.", &[("drive", d.title())]), tr("Eject"), false, Action::Eject { drive: d.object });
    }

    fn ask_format(&self) {
        let Some(p) = self.selected_part() else { return };
        let d = self.drive().unwrap_or_default();
        if p.mount.as_deref() == Some("/") || d.is_system() && p.mount.is_some() {
            self.status.set_text(tr("The system's own partition in use cannot be formatted."));
            return;
        }
        let fs = if d.removable { "exfat" } else { "ext4" };
        let label = p.title(&p.device);
        self.show_confirm(tr("Format…"), &trf("EVERYTHING on {part} ({size}) is erased for good, and it becomes an empty {fs} partition named “{label}”.",
            &[("part", p.title(&p.device)), ("size", data::amount(p.size)), ("fs", fs.to_string()), ("label", label.clone())]),
            tr("Erase and format"), true, Action::Format { part: p.object, fs: fs.into(), label });
    }

    fn ask_clone(&self, dest: Option<usize>) {
        let drives = self.drives.borrow().clone();
        let (Some(src), Some(dst)) = (drives.get(self.chosen.get()), dest.and_then(|i| drives.get(i))) else {
            self.status.set_text(tr("Say which drive to copy and to which one."));
            return;
        };
        // (a clone is an image of the source written over the destination: shown, the person makes it with the
        // two actions — never in one click by a request)
        self.status.set_text(&trf("To clone the {src} to the {dst}: make an image of the {src} here, then restore it onto the {dst}, which erases everything on it ({size}).",
            &[("src", src.title()), ("dst", dst.title()), ("size", data::amount(dst.size))]));
        if debug() {
            eprintln!("debug discos: clonar {} → {} explicado", src.title(), dst.title());
        }
    }

    fn run_pending(self: &Rc<Self>) {
        let Some(a) = self.pending.borrow_mut().take() else { return };
        self.confirm.set_visible(false);
        if self.busy.replace(true) {
            return;
        }
        self.status.set_text(tr("Working… (the system may ask for your password)"));
        if debug() {
            eprintln!("debug discos: fazer {a:?}");
        }
        let w = Rc::downgrade(self);
        let progress = std::sync::Arc::new(std::sync::Mutex::new(0.0f64));
        let p2 = progress.clone();
        let w2 = Rc::downgrade(self);
        let ticker = glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(a) = w2.upgrade() else { return glib::ControlFlow::Break };
            let f = p2.lock().map(|g| *g).unwrap_or(0.0);
            if f > 0.0 && f < 1.0 {
                a.status.set_text(&trf("Working… {n}%", &[("n", ((f * 100.0) as u32).to_string())]));
            }
            glib::ControlFlow::Continue
        });
        let ticker = RefCell::new(Some(ticker));
        crate::lateral::em_segundo_plano(move || data::act(&a, &|f| { if let Ok(mut g) = progress.lock() { *g = f; } }), move |r| {
            if let Some(t) = ticker.borrow_mut().take() {
                t.remove();
            }
            let Some(a) = w.upgrade() else { return };
            a.busy.set(false);
            match r {
                Ok(m) => {
                    if debug() {
                        eprintln!("debug discos: feito ({m})");
                    }
                    a.status.set_text(tr("Done."));
                    a.reload();
                }
                Err(e) => a.status.set_text(&trf("It could not be done: {error}", &[("error", e)])),
            }
        });
    }
}

/// A file system's usual name ("vfat" → "FAT32").
fn fs_name(fs: &str) -> String {
    match fs {
        "vfat" => "FAT32".into(),
        "exfat" => "exFAT".into(),
        "ntfs" => "NTFS".into(),
        f => f.to_string(),
    }
}

fn ask_agent(cmd: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let mut c = std::process::Command::new(crate::voz::agent_binary()).arg(cmd).stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    c.stdin.take().ok_or("stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
}
