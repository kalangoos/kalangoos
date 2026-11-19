//! Configurações › Notificações (design "Configurações · Notificações"): Do not disturb (by hand; by itself at night
//! and while the focus timer runs) and the agent's digest (what is not urgent waits for 12:00 and 18:00); each app
//! that sent notifications this week with its rule (off, banners and sound, banners only, only in the digest); and in
//! the panel how they show (where the banners go — drawn in the preview —, long activities in the island, the lock
//! screen, the sound) and the priority (important people, what breaks through Do not disturb). The rules themselves
//! are `notification_policy`; the notification server follows them.

use crate::configuracoes::{depurando, interruptor};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::notification_policy::{self as policy, AppRule, LockScreen, Mode, Position};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

const POSITIONS: &[(Position, &str)] = &[
    (Position::Island, n_("Under the island")),
    (Position::TopRight, n_("Top right corner")),
    (Position::TopLeft, n_("Top left corner")),
    (Position::BottomRight, n_("Bottom right corner")),
];

const LOCK_SCREEN: &[(LockScreen, &str)] = &[
    (LockScreen::All, n_("Show everything")),
    (LockScreen::NoContent, n_("Without the content")),
    (LockScreen::Nothing, n_("Nothing")),
];

const MODES: &[(Mode, &str)] = &[
    (Mode::BannersSound, n_("Banners and sound")),
    (Mode::Banners, n_("Banners only")),
    (Mode::DigestOnly, n_("Only in the agent's digest")),
];

/// The agents' notices come as this app (the shell names it in the system's language).
fn agents() -> &'static str {
    tr("Agents")
}

fn left_label(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

fn label16(glyph: &str, text: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    r.append(&ui_icon(glyph, "#8E8F99", 16));
    r.append(&rotulo(text, "cfg-rotulo"));
    r
}

/// The title and the line under it (the part of a row that opens its choices when clicked).
fn texts(title: &str, sub: &gtk::Label) -> gtk::Box {
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left_label(title, "cfg-texto-13"));
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.append(sub);
    t
}

fn selector(items: &[&str]) -> gtk::DropDown {
    let d = gtk::DropDown::new(Some(gtk::StringList::new(items)), None::<gtk::Expression>);
    d.add_css_class("cfg-seletor");
    d.set_size_request(200, 34);
    d.set_valign(Align::Center);
    d
}

/// What an app's rule means, in a few words.
pub fn rule_text(app: &str, rule: AppRule, p: &policy::Policy) -> String {
    if !rule.on {
        return if app == agents() { tr("only approvals").to_string() } else { tr("off").to_string() };
    }
    let mode = match rule.mode {
        Mode::BannersSound => tr("banners and sound").to_string(),
        Mode::Banners => tr("banners only").to_string(),
        Mode::DigestOnly if !p.agent_digest => tr("banners only").to_string(),
        Mode::DigestOnly if !p.people.is_empty() => tr("only from important people · the rest in the digest").to_string(),
        Mode::DigestOnly => tr("only in the digest").to_string(),
    };
    if app == agents() && p.breaks.approvals {
        format!("{} · {mode}", tr("approvals always show"))
    } else {
        mode
    }
}

/// A popover of choices under a row, gone when closed.
fn choices(parent: &impl IsA<gtk::Widget>, child: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let p = gtk::Popover::new();
    p.add_css_class("cfg-escolhas");
    p.set_child(Some(child));
    p.set_parent(parent);
    p.set_position(gtk::PositionType::Bottom);
    p.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    p.popup();
    p
}

pub struct NotificationsPage {
    pub frame: Rc<Moldura>,
    settings: Weak<crate::configuracoes::Configuracoes>,
    dnd: gtk::Switch,
    dnd_sub: gtk::Label,
    dnd_texts: gtk::Box,
    digest: gtk::Switch,
    digest_sub: gtk::Label,
    apps: gtk::Box,
    apps_signature: RefCell<String>,
    /// the apps' order while the page is open: a notification that arrives does not move the rows under the pointer
    /// (a new app goes at the end); opened again, the page sorts them again
    apps_order: RefCell<Vec<String>>,
    preview: gtk::DrawingArea,
    position: gtk::DropDown,
    in_island: gtk::Switch,
    lock_screen: gtk::DropDown,
    volume: gtk::Scale,
    people_sub: gtk::Label,
    people_row: gtk::Box,
    breaks_sub: gtk::Label,
    breaks_row: gtk::Box,
    loading: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl NotificationsPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<NotificationsPage> {
        let m = Moldura::new(tr("Notifications"), &[], tr("Appearance and priority"));
        m.busca_grande();
        // ---- the two modes, side by side
        let modes = caixa(Orientation::Horizontal, 16, "");
        modes.set_homogeneous(true);
        let mode_card = |glyph: Option<&str>, title: &str, sub: &gtk::Label, sw: &gtk::Switch| -> (gtk::Box, gtk::Box) {
            let card = caixa(Orientation::Vertical, 10, "cfg-cartao");
            card.add_css_class("estreito");
            let r = caixa(Orientation::Horizontal, 12, "cfg-notif-modo");
            let icon = caixa(Orientation::Horizontal, 0, "");
            icon.set_size_request(16, 16);
            icon.set_valign(Align::Center);
            match glyph {
                Some(g) => icon.append(&ui_icon(g, "#8E8F99", 16)),
                None => {
                    // (the design's spark sits a pixel in from its frame)
                    let s = estrela(13, "#A5A1E2");
                    s.set_halign(Align::Start);
                    s.set_valign(Align::Center);
                    s.set_margin_start(1);
                    icon.append(&s);
                }
            }
            r.append(&icon);
            let t = texts(title, sub);
            r.append(&t);
            r.append(sw);
            card.append(&r);
            (card, t)
        };
        let dnd = interruptor();
        let dnd_sub = rotulo("", "cfg-sub");
        let (dc, dnd_texts) = mode_card(Some("sino-mudo-n"), tr("Do not disturb"), &dnd_sub, &dnd);
        modes.append(&dc);
        let digest = interruptor();
        let digest_sub = rotulo("", "cfg-sub");
        let (gc, _) = mode_card(None, tr("Agent's digest"), &digest_sub, &digest);
        modes.append(&gc);
        m.corpo.append(&modes);
        // ---- per app
        let pa = caixa(Orientation::Vertical, 2, "cfg-cartao");
        pa.append(&label16("apps-n", tr("PER APP")));
        let apps = caixa(Orientation::Vertical, 2, "");
        pa.append(&apps);
        m.corpo.append(&pa);
        // ---- the panel: how they show
        let rules = caixa(Orientation::Vertical, 16, "");
        m.painel.append(&rules);
        let how = caixa(Orientation::Vertical, 2, "cfg-cartao");
        how.append(&label16("sino-n", tr("HOW THEY SHOW")));
        let preview = gtk::DrawingArea::new();
        preview.set_content_height(120);
        preview.set_hexpand(true);
        how.append(&preview);
        let row = |title: &str, sub: Option<&str>, control: &gtk::Widget| -> gtk::Box {
            let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
            let t = caixa(Orientation::Vertical, 1, "");
            t.set_hexpand(true);
            t.set_valign(Align::Center);
            t.append(&left_label(title, "cfg-texto-13"));
            if let Some(s) = sub {
                t.append(&left_label(s, "cfg-sub"));
                r.add_css_class("alta");
            }
            r.append(&t);
            control.set_valign(Align::Center);
            r.append(control);
            r
        };
        let position = selector(&POSITIONS.iter().map(|(_, n)| tr(n)).collect::<Vec<_>>());
        how.append(&row(tr("Where the banners show"), None, position.upcast_ref()));
        let in_island = interruptor();
        how.append(&row(tr("Show in the island"), Some(tr("long activities become a card in the bar")), in_island.upcast_ref()));
        let lock_screen = selector(&LOCK_SCREEN.iter().map(|(_, n)| tr(n)).collect::<Vec<_>>());
        how.append(&row(tr("On the lock screen"), None, lock_screen.upcast_ref()));
        let volume = crate::configuracoes::deslizador();
        volume.set_hexpand(false);
        volume.set_size_request(200, -1);
        how.append(&row(tr("Sound"), None, volume.upcast_ref()));
        rules.append(&how);
        // ---- priority
        let pr = caixa(Orientation::Vertical, 2, "cfg-cartao");
        pr.append(&label16("estrela-n", tr("PRIORITY")));
        let chevron_row = |title: &str, sub: &gtk::Label| -> gtk::Box {
            let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
            r.add_css_class("alta");
            r.add_css_class("cfg-linha-clicavel");
            r.append(&texts(title, sub));
            let ch = ui_icon("chevron-n", "#7A7B88", 16);
            ch.set_valign(Align::Center);
            r.append(&ch);
            r
        };
        let people_sub = rotulo("", "cfg-sub");
        let people_row = chevron_row(tr("Important people"), &people_sub);
        pr.append(&people_row);
        let breaks_sub = rotulo("", "cfg-sub");
        let breaks_row = chevron_row(tr("What breaks through Do not disturb"), &breaks_sub);
        pr.append(&breaks_row);
        rules.append(&pr);
        let page = Rc::new(NotificationsPage { frame: m, settings: Rc::downgrade(c), dnd, dnd_sub, dnd_texts, digest, digest_sub, apps, apps_signature: RefCell::default(), apps_order: RefCell::default(),
            preview, position, in_island, lock_screen, volume, people_sub, people_row, breaks_sub, breaks_row, loading: Cell::new(false), timer: RefCell::default() });
        page.connect_signals(c);
        page
    }

    fn register(&self, name: &str, w: &gtk::Widget) {
        if let Some(c) = self.settings.upgrade() {
            crate::configuracoes::registrar_posicao(&c, name, w);
        }
    }

    fn connect_signals(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>) {
        // Do not disturb by hand; its row opens "by itself at night and in focus"
        let w = Rc::downgrade(self);
        self.dnd.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                if let Some(s) = crate::shell_atual() {
                    s.notificacoes().definir_nao_perturbe(on);
                }
                if depurando() {
                    eprintln!("debug notif: não perturbe {on}");
                }
                p.refresh();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let Some(p) = w.upgrade() else { return };
            let auto = gtk::CheckButton::with_label(tr("By itself from 22:00 to 07:00 and in focus mode"));
            auto.set_active(policy::read().dnd_auto);
            let w = Rc::downgrade(&p);
            auto.connect_toggled(move |b| {
                let on = b.is_active();
                policy::update(|x| x.dnd_auto = on);
                if depurando() {
                    eprintln!("debug notif: não perturbe automático {on}");
                }
                if let Some(p) = w.upgrade() {
                    p.refresh();
                }
            });
            let pop = choices(&p.dnd_texts, &auto);
            if depurando() {
                let a = auto.clone();
                glib::timeout_add_local_once(Duration::from_millis(300), move || crate::central::posicao_debug("cfg notif automatico", a.upcast_ref()));
            }
            let _ = pop;
        });
        self.dnd_texts.add_controller(click);
        let w = Rc::downgrade(self);
        self.digest.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                policy::update(|x| x.agent_digest = on);
                if depurando() {
                    eprintln!("debug notif: resumo do agente {on}");
                }
                p.refresh();
            }
            glib::Propagation::Proceed
        });
        // how they show
        let w = Rc::downgrade(self);
        self.position.connect_selected_notify(move |d| {
            let Some(p) = w.upgrade().filter(|p| !p.loading.get()) else { return };
            let pos = POSITIONS.get(d.selected() as usize).map(|(x, _)| *x).unwrap_or_default();
            policy::update(|x| x.position = pos);
            if let Some(s) = crate::shell_atual() {
                s.notificacoes().apply_position();
            }
            p.preview.queue_draw();
        });
        let w = Rc::downgrade(self);
        self.in_island.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                policy::update(|x| x.in_island = on);
                if depurando() {
                    eprintln!("debug notif: na ilha {on}");
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.lock_screen.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                let l = LOCK_SCREEN.get(d.selected() as usize).map(|(x, _)| *x).unwrap_or_default();
                policy::update(|x| x.lock_screen = l);
                if depurando() {
                    eprintln!("debug notif: tela de bloqueio {l:?}");
                }
            }
        });
        let w = Rc::downgrade(self);
        self.volume.connect_value_changed(move |s| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                let v = (s.value() * 100.0).round() / 100.0;
                policy::update(|x| x.volume = v);
            }
        });
        // a sample at the volume chosen, when the slider is let go
        let release = gtk::GestureClick::new();
        release.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        release.connect_released(move |_, _, _, _| {
            if let Some(p) = w.upgrade() {
                crate::configuracoes_som::play_at("message-new-instant", p.volume.value());
            }
        });
        self.volume.add_controller(release);
        // priority
        let w = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            if let Some(p) = w.upgrade() {
                p.edit_people();
            }
        });
        self.people_row.add_controller(click);
        let w = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            if let Some(p) = w.upgrade() {
                p.edit_breaks();
            }
        });
        self.breaks_row.add_controller(click);
        let w = Rc::downgrade(self);
        self.preview.set_draw_func(move |_, cr, w_, h_| {
            if let Some(p) = w.upgrade() {
                p.draw_preview(cr, f64::from(w_), f64::from(h_));
            }
        });
        crate::configuracoes::registrar_posicao(c, "cfg notif nao perturbe", self.dnd.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif nao perturbe texto", self.dnd_texts.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif resumo", self.digest.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif posicao", self.position.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif ilha", self.in_island.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif bloqueio", self.lock_screen.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif pessoas", self.people_row.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg notif furam", self.breaks_row.upcast_ref());
    }

    /// Important people: each one with a way out, and a field to add one more.
    fn edit_people(self: &Rc<Self>) {
        let b = caixa(Orientation::Vertical, 6, "cfg-escolhas-lista");
        let list = caixa(Orientation::Vertical, 2, "");
        b.append(&list);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(tr("Add a person (name as notifications show it)")));
        b.append(&entry);
        let fill: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::default();
        let (l2, f2, w) = (list.clone(), fill.clone(), Rc::downgrade(self));
        *fill.borrow_mut() = Some(Box::new(move || {
            while let Some(x) = l2.first_child() {
                l2.remove(&x);
            }
            for name in policy::read().people {
                let r = caixa(Orientation::Horizontal, 8, "");
                let n = left_label(&name, "cfg-texto-13");
                n.set_hexpand(true);
                r.append(&n);
                let x = gtk::Button::with_label(tr("Remove"));
                x.add_css_class("cfg-botao-pequeno");
                let (f3, w3) = (f2.clone(), w.clone());
                x.connect_clicked(move |_| {
                    policy::update(|p| p.people.retain(|q| *q != name));
                    if let Some(f) = f3.borrow().as_ref() {
                        f();
                    }
                    if let Some(p) = w3.upgrade() {
                        p.refresh();
                    }
                });
                r.append(&x);
                l2.append(&r);
            }
        }));
        if let Some(f) = fill.borrow().as_ref() {
            f();
        }
        let (f2, w) = (fill.clone(), Rc::downgrade(self));
        entry.connect_activate(move |e| {
            let name = e.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            policy::update(|p| {
                if !p.people.contains(&name) {
                    p.people.push(name.clone());
                }
            });
            if depurando() {
                eprintln!("debug notif: pessoa importante {name}");
            }
            e.set_text("");
            if let Some(f) = f2.borrow().as_ref() {
                f();
            }
            if let Some(p) = w.upgrade() {
                p.refresh();
            }
        });
        let pop = choices(&self.people_row, &b);
        pop.set_size_request(320, -1);
        entry.grab_focus();
        if depurando() {
            let e = entry.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || crate::central::posicao_debug("cfg notif pessoa nova", e.upcast_ref()));
        }
    }

    /// What breaks through Do not disturb: alarms, high-risk approvals, important people.
    fn edit_breaks(self: &Rc<Self>) {
        let b = caixa(Orientation::Vertical, 4, "cfg-escolhas-lista");
        let p = policy::read();
        for (key, label, on) in [("alarms", tr("Alarms"), p.breaks.alarms), ("approvals", tr("High-risk approvals"), p.breaks.approvals), ("people", tr("Important people"), p.breaks.people)] {
            let c = gtk::CheckButton::with_label(label);
            c.set_active(on);
            let w = Rc::downgrade(self);
            c.connect_toggled(move |c| {
                let on = c.is_active();
                policy::update(|p| match key {
                    "alarms" => p.breaks.alarms = on,
                    "approvals" => p.breaks.approvals = on,
                    _ => p.breaks.people = on,
                });
                if depurando() {
                    eprintln!("debug notif: fura o não perturbe: {key} {on}");
                }
                if let Some(p) = w.upgrade() {
                    p.refresh();
                }
            });
            if depurando() {
                let c2 = c.clone();
                glib::timeout_add_local_once(Duration::from_millis(300), move || crate::central::posicao_debug(&format!("cfg notif fura {key}"), c2.upcast_ref()));
            }
            b.append(&c);
        }
        choices(&self.breaks_row, &b);
    }

    /// An app's choices: its mode.
    fn edit_app(self: &Rc<Self>, app: &str, anchor: &gtk::Box) {
        let b = caixa(Orientation::Vertical, 4, "cfg-escolhas-lista");
        let rule = policy::read().apps.get(app).copied().unwrap_or_default();
        let mut first: Option<gtk::CheckButton> = None;
        for (mode, label) in MODES {
            let c = gtk::CheckButton::with_label(tr(label));
            if let Some(f) = &first {
                c.set_group(Some(f));
            }
            c.set_active(rule.mode == *mode);
            let (w, app, mode) = (Rc::downgrade(self), app.to_string(), *mode);
            c.connect_toggled(move |c| {
                if !c.is_active() {
                    return;
                }
                policy::update(|p| p.apps.entry(app.clone()).or_default().mode = mode);
                if depurando() {
                    eprintln!("debug notif: {app} {mode:?}");
                }
                if let Some(p) = w.upgrade() {
                    p.refresh();
                }
            });
            if depurando() {
                let (c2, n) = (c.clone(), format!("cfg notif modo {mode:?}"));
                glib::timeout_add_local_once(Duration::from_millis(300), move || crate::central::posicao_debug(&n, c2.upcast_ref()));
            }
            first.get_or_insert(c.clone());
            b.append(&c);
        }
        choices(anchor, &b);
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }

    pub fn mostrar(self: &Rc<Self>) {
        self.apps_order.borrow_mut().clear();
        self.refresh();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
            if !p.frame.raiz.is_mapped() {
                p.timer.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            p.refresh();
            glib::ControlFlow::Continue
        }));
    }

    fn refresh(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(policy::counts, move |counts| {
            if let Some(p) = w.upgrade() {
                p.show(counts);
            }
        });
    }

    fn show(self: &Rc<Self>, counts: policy::Counts) {
        let p = policy::read();
        self.loading.set(true);
        self.frame.sub.set_text(&trf("{n} today · {m} summed up by the agent", &[("n", counts.today.to_string()), ("m", counts.digested.to_string())]));
        let manual = crate::shell_atual().is_some_and(|s| s.notificacoes().nao_perturbe.get());
        let now = crate::shell_atual().is_some_and(|s| s.notificacoes().dnd_now());
        self.dnd.set_active(manual);
        self.dnd.set_state(manual);
        self.dnd_sub.set_text(&match (p.dnd_auto, now && !manual) {
            (true, true) => tr("on by itself now · at night and in focus mode").to_string(),
            (true, false) => tr("by itself from 22:00 to 07:00 and during focus mode").to_string(),
            (false, _) => tr("only when you turn it on").to_string(),
        });
        self.digest.set_active(p.agent_digest);
        self.digest.set_state(p.agent_digest);
        self.digest_sub.set_text(tr("gathers what is not urgent and delivers it at 12:00 and 18:00"));
        {
            // (only a change: setting it again would close its list while the person is choosing)
            let i = POSITIONS.iter().position(|(x, _)| *x == p.position).unwrap_or(0) as u32;
            if self.position.selected() != i {
                self.position.set_selected(i);
            }
        }
        self.in_island.set_active(p.in_island);
        self.in_island.set_state(p.in_island);
        {
            // (only a change: setting it again would close its list while the person is choosing)
            let i = LOCK_SCREEN.iter().position(|(x, _)| *x == p.lock_screen).unwrap_or(1) as u32;
            if self.lock_screen.selected() != i {
                self.lock_screen.set_selected(i);
            }
        }
        if !self.volume.has_css_class("arrastando") {
            self.volume.set_value(p.volume);
        }
        self.people_sub.set_text(&if p.people.is_empty() { tr("nobody yet").to_string() } else { p.people.join(", ") });
        let breaks: Vec<&str> = [(p.breaks.alarms, tr("alarms")), (p.breaks.approvals, tr("high-risk approvals")), (p.breaks.people, tr("important people"))]
            .into_iter().filter(|(on, _)| *on).map(|(_, t)| t).collect();
        self.breaks_sub.set_text(&if breaks.is_empty() { tr("nothing").to_string() } else { breaks.join(", ") });
        self.show_apps(&counts, &p);
        self.preview.queue_draw();
        self.loading.set(false);
        if depurando() {
            eprintln!("debug notif: {} hoje, {} resumidas; apps {}", counts.today, counts.digested,
                self.app_names(&counts, &p).join(","));
        }
    }

    /// The agent first (its approvals), then the apps that sent notifications this week, most first, and the ones
    /// the person set a rule for.
    fn app_names(&self, counts: &policy::Counts, p: &policy::Policy) -> Vec<String> {
        let mut v = vec![agents().to_string()];
        for (a, _, _) in &counts.apps {
            if !v.contains(a) {
                v.push(a.clone());
            }
        }
        for a in p.apps.keys() {
            if !v.contains(a) {
                v.push(a.clone());
            }
        }
        let mut order = self.apps_order.borrow_mut();
        if !order.is_empty() {
            let mut kept: Vec<String> = order.iter().filter(|a| v.contains(a)).cloned().collect();
            kept.extend(v.into_iter().filter(|a| !order.contains(a)));
            v = kept;
        }
        *order = v.clone();
        v
    }

    fn show_apps(self: &Rc<Self>, counts: &policy::Counts, p: &policy::Policy) {
        let names = self.app_names(counts, p);
        let signature = format!("{names:?}{:?}{:?}{}{:?}", p.apps, p.people, p.agent_digest, p.breaks);
        if *self.apps_signature.borrow() == signature {
            return;
        }
        *self.apps_signature.borrow_mut() = signature;
        while let Some(x) = self.apps.first_child() {
            self.apps.remove(&x);
        }
        let all = crate::apps::todos();
        for app in names {
            let rule = p.apps.get(&app).copied().unwrap_or_default();
            let r = caixa(Orientation::Horizontal, 12, "cfg-notif-app");
            // the app's icon: 21 in a 26 frame, as the design's tiles
            let ib = caixa(Orientation::Horizontal, 0, "");
            ib.set_size_request(26, 26);
            ib.set_valign(Align::Center);
            ib.set_hexpand(false);
            let lower = app.to_lowercase();
            let icon_name = counts.apps.iter().find(|(a, _, _)| *a == app).map(|(_, i, _)| i.clone()).unwrap_or_default();
            let img = if app == agents() {
                crate::apps::imagem_kalango("agentes", 21)
            } else if let Some(a) = all.iter().find(|a| a.nome.to_lowercase() == lower || a.id.to_lowercase().contains(&lower)) {
                crate::apps::imagem(a, 21)
            } else if let Some(i) = crate::notificacoes::imagem(&icon_name, 21) {
                i
            } else {
                let i = gtk::Image::from_icon_name("application-x-executable");
                i.set_pixel_size(21);
                i
            };
            img.set_halign(Align::Center);
            img.set_valign(Align::Center);
            img.set_hexpand(true);
            ib.append(&img);
            r.append(&ib);
            let sub = rotulo(&rule_text(&app, rule, p), "cfg-sub");
            let t = texts(&app, &sub);
            r.append(&t);
            let sw = interruptor();
            sw.set_active(rule.on);
            sw.set_state(rule.on);
            let (w, a2) = (Rc::downgrade(self), app.clone());
            sw.connect_state_set(move |_, on| {
                if w.upgrade().is_some_and(|p| !p.loading.get()) {
                    policy::update(|p| p.apps.entry(a2.clone()).or_default().on = on);
                    if depurando() {
                        eprintln!("debug notif: {a2} {}", if on { "ligado" } else { "desligado" });
                    }
                    if let Some(p) = w.upgrade() {
                        p.refresh();
                    }
                }
                glib::Propagation::Proceed
            });
            r.append(&sw);
            // its name opens the choices of how it shows
            let click = gtk::GestureClick::new();
            let (w, a3, tw) = (Rc::downgrade(self), app.clone(), t.downgrade());
            click.connect_released(move |_, _, _, _| {
                if let (Some(p), Some(t)) = (w.upgrade(), tw.upgrade()) {
                    p.edit_app(&a3, &t);
                }
            });
            t.add_controller(click);
            self.register(&format!("cfg notif app {app}"), sw.upcast_ref());
            self.register(&format!("cfg notif app texto {app}"), t.upcast_ref());
            self.apps.append(&r);
        }
    }

    /// A small screen: the top bar, the island, and the banners where they show.
    fn draw_preview(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let rect = |x: f64, y: f64, ww: f64, hh: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + ww - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(x + ww - r, y + hh - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(x + r, y + hh - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
        };
        let set = |hex: &str, a: f64| {
            let (r, g, b) = crate::tema::rgb(hex);
            cr.set_source_rgba(r, g, b, a);
        };
        // the screen, its border, the top bar
        rect(0.5, 0.5, w - 1.0, h - 1.0, 4.0);
        set("#101114", 1.0);
        let _ = cr.fill_preserve();
        cr.save().ok();
        cr.clip();
        set("#1A1B21", 1.0);
        cr.rectangle(0.0, 0.0, w, 12.0);
        let _ = cr.fill();
        cr.restore().ok();
        rect(0.5, 0.5, w - 1.0, h - 1.0, 4.0);
        set("#26272F", 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        set("#FFFFFF", 0.2);
        rect((w - 60.0) / 2.0, 3.0, 60.0, 6.0, 3.0);
        let _ = cr.fill();
        // the banners: the newest outlined, the one before under it
        let (bw, bh, b2h) = (126.0, 34.0, 24.0);
        let pos = policy::read().position;
        let x = match pos {
            Position::Island => (w - bw) / 2.0,
            Position::TopLeft => 12.0,
            Position::TopRight | Position::BottomRight => w - bw - 12.0,
        };
        let (y1, y2) = if pos == Position::BottomRight { (h - 12.0 - bh, h - 12.0 - bh - 6.0 - b2h) } else { (22.0, 22.0 + bh + 6.0) };
        set("#FFFFFF", 0.06);
        rect(x, y2, bw, b2h, 4.0);
        let _ = cr.fill();
        set("#2A2B35", 1.0);
        rect(x + 0.5, y1 + 0.5, bw - 1.0, bh - 1.0, 4.0);
        let _ = cr.fill_preserve();
        set(crate::tema::ACENTO, 1.0);
        let _ = cr.stroke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apps_rule_in_a_few_words() {
        let mut p = policy::Policy::default();
        let on = AppRule { on: true, mode: Mode::BannersSound };
        assert_eq!(rule_text("Discord", on, &p), tr("banners and sound"));
        assert_eq!(rule_text("Música", AppRule { on: false, ..on }, &p), tr("off"));
        assert_eq!(rule_text("Correio", AppRule { on: true, mode: Mode::DigestOnly }, &p), tr("only in the digest"));
        p.people.push("Marina".into());
        assert_eq!(rule_text("Correio", AppRule { on: true, mode: Mode::DigestOnly }, &p), tr("only from important people · the rest in the digest"));
        assert_eq!(rule_text(agents(), AppRule { on: true, mode: Mode::Banners }, &p), format!("{} · {}", tr("approvals always show"), tr("banners only")));
        // without the agent's digest, "only in the digest" shows as banners
        p.agent_digest = false;
        assert_eq!(rule_text("Correio", AppRule { on: true, mode: Mode::DigestOnly }, &p), tr("banners only"));
    }
}
