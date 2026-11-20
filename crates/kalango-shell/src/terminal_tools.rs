//! What every tab of the Terminal has (design "Kalango Remote · SSH abre no Terminal do sistema"):
//! - **Split** (Ctrl+Shift+D, or the toolbar's columns): a second pane beside the first — the same host again on an
//!   SSH tab (through the session's connection, no second login), the person's shell in the same folder otherwise.
//! - **Search** (Ctrl+Shift+F): a bar at the top right of the panes, "1 de 3", Enter for the next older one.
//! - **The selection's menu**: "Explicar erro" (to the agent's bar, with secrets taken out), "Copiar", "Buscar".
//! - **The agent's command on the line**: its card under the prompt — Enter runs it, Esc erases it, "Explicar".
//! - **Approvals**: the agent's command for a production host, or one that erases or restarts something, waits for
//!   the person's yes before going to the line; on a production host, Enter on such a line asks first.

use super::*;
use crate::ssh_hosts::{is_destructive, redact_secrets};

/// PCRE2's flags for a search: UTF, caseless, multiline.
const SEARCH_FLAGS: u32 = 0x0008_0000 | 0x0000_0008 | 0x0000_0400;

/// A terminal as the Terminal's tabs have it.
pub(super) fn new_vte() -> vte4::Terminal {
    let term = vte4::Terminal::new();
    term.add_css_class("term-vte");
    // (rows 28 px apart, as drawn)
    term.set_cell_height_scale(1.4);
    term.set_bold_is_bright(false);
    term.set_mouse_autohide(true);
    style_terminal(&term, &crate::terminal_prefs::load());
    term.set_vexpand(true);
    term.set_hexpand(true);
    term
}

pub(super) fn scrolled(term: &vte4::Terminal) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(term));
    scroll
}

thread_local! {
    /// the second panes' processes (hung up with their tab)
    static PANE_PIDS: RefCell<Vec<(glib::WeakRef<vte4::Terminal>, i32)>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn hang_up_pane(term: &vte4::Terminal) {
    PANE_PIDS.with(|p| {
        p.borrow_mut().retain(|(w, pid)| {
            if w.upgrade().is_some_and(|t| &t == term) {
                unsafe { kill(*pid, 1) };
                false
            } else {
                w.upgrade().is_some()
            }
        })
    });
}

/// "\.*" and such, literal in a regex.
pub(super) fn regex_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// How many times `needle` is in `hay`, ignoring case.
pub(super) fn count_matches(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    hay.to_lowercase().matches(&needle.to_lowercase()).count()
}

/// The cursor's row on the screen (0: the top one). VTE's own rows count from the start of its history, and after a
/// `clear` its scroll position does not say where that start is: the screen's text does (the cursor's line is its last
/// written one when a command waits on the line).
pub(super) fn cursor_screen_row(term: &vte4::Terminal) -> i64 {
    let shown = term.text_format(vte4::Format::Text).map(|s| s.to_string()).unwrap_or_default();
    let lines = shown.trim_end().lines().count() as i64;
    let top = term.vadjustment().map(|a| a.value()).unwrap_or(0.0) as i64;
    let by_rows = term.cursor_position().1 - top;
    if lines > 0 { (lines - 1).min(by_rows) } else { by_rows }
}

/// The command on the cursor's row (after the prompt's "$ " or "# ", when there is one).
pub(super) fn line_command(row: &str) -> String {
    let r = row.trim_end();
    let cut = ["$ ", "# ", "➜ ", "% "].iter().filter_map(|m| r.rfind(m).map(|i| i + m.len())).max();
    match cut {
        Some(i) => r[i..].trim().to_string(),
        None => r.trim().to_string(),
    }
}

/// The search bar of a tab.
pub(super) struct Search {
    bar: gtk::Box,
    entry: gtk::Entry,
    count: gtk::Label,
    index: Cell<usize>,
    total: Cell<usize>,
}

impl Terminal {
    /// The pane's controllers: the pane in use, the selection's menu, the agent's card's keys, production's guard.
    pub(super) fn wire_pane(self: &Rc<Self>, x: &Rc<Tab>, term: &vte4::Terminal) {
        let focus = gtk::EventControllerFocus::new();
        let (w, xw, tw) = (Rc::downgrade(self), Rc::downgrade(x), term.downgrade());
        focus.connect_enter(move |_| {
            let (Some(t), Some(x), Some(term)) = (w.upgrade(), xw.upgrade(), tw.upgrade()) else { return };
            *x.focused.borrow_mut() = term;
            t.update_status();
        });
        term.add_controller(focus);
        // a selection made: its menu where the pointer let go (VTE keeps the drag's own clicks: the menu waits for the
        // selection to settle with the button up)
        let at: Rc<Cell<(f64, f64)>> = Rc::default();
        let motion = gtk::EventControllerMotion::new();
        let a2 = at.clone();
        motion.connect_motion(move |_, px, py| a2.set((px, py)));
        term.add_controller(motion);
        let pending = Rc::new(Cell::new(0u32));
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        term.connect_selection_changed(move |term| {
            let n = pending.get() + 1;
            pending.set(n);
            let (w, xw, tw, at, pending) = (w.clone(), xw.clone(), term.downgrade(), at.clone(), pending.clone());
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                let (Some(t), Some(x), Some(term)) = (w.upgrade(), xw.upgrade(), tw.upgrade()) else { return glib::ControlFlow::Break };
                // (the search selects what it finds: no menu for that)
                if pending.get() != n || x.search.borrow().is_some() {
                    return glib::ControlFlow::Break;
                }
                let held = term.display().default_seat().and_then(|s| s.pointer()).is_some_and(|p| p.modifier_state().contains(gdk::ModifierType::BUTTON1_MASK));
                if held {
                    return glib::ControlFlow::Continue;
                }
                if term.has_selection() && term.text_selected(vte4::Format::Text).is_some_and(|s| !s.trim().is_empty()) {
                    let (px, py) = at.get();
                    t.selection_menu(&x, &term, px, py);
                }
                glib::ControlFlow::Break
            });
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (w, xw, tw) = (Rc::downgrade(self), Rc::downgrade(x), term.downgrade());
        keys.connect_key_pressed(move |_, k, _, m| {
            let (Some(t), Some(x), Some(term)) = (w.upgrade(), xw.upgrade(), tw.upgrade()) else { return glib::Propagation::Proceed };
            if m.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            let card = x.agent_card.borrow().is_some();
            match k {
                gdk::Key::Escape if card => {
                    // the agent's command goes from the line
                    term.feed_child(b"\x15");
                    t.hide_agent_card(&x);
                    t.st_hint.set_text("");
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if card {
                        t.hide_agent_card(&x);
                        t.st_hint.set_text("");
                    }
                    // production (or a host that asks before connecting): a command that erases or restarts something
                    // asks first
                    let production = x.remote.as_ref().is_some_and(|r| r.target.production || r.target.confirm);
                    if production {
                        let row = term.cursor_position().1;
                        let (text, _) = term.text_range_format(vte4::Format::Text, row, 0, row, term.column_count());
                        let cmd = line_command(text.as_deref().unwrap_or(""));
                        if is_destructive(&cmd) {
                            t.confirm_in_production(&x, &term, &cmd);
                            return glib::Propagation::Stop;
                        }
                    }
                    glib::Propagation::Proceed
                }
                _ => glib::Propagation::Proceed,
            }
        });
        term.add_controller(keys);
        // what the session shows (ssh's questions, mosh's notice, the folder for SFTP)
        if x.remote.is_some() {
            let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
            term.connect_contents_changed(move |_| {
                if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                    t.screen_changed(&x);
                }
            });
        }
    }

    /// The menu of a selection (design: "Explicar erro · Copiar · Buscar" by the selected lines).
    fn selection_menu(self: &Rc<Self>, x: &Rc<Tab>, term: &vte4::Terminal, px: f64, py: f64) {
        let pop = gtk::Popover::new();
        pop.add_css_class("term-selecao");
        pop.set_has_arrow(false);
        pop.set_autohide(true);
        pop.set_position(gtk::PositionType::Bottom);
        pop.set_parent(term);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(px as i32, py as i32 + 6, 1, 1)));
        let row = caixa(Orientation::Horizontal, 2, "");
        let item = |icon: &str, color: &str, text: &str| {
            let b = gtk::Button::new();
            b.add_css_class("term-selecao-item");
            let c = caixa(Orientation::Horizontal, 6, "");
            c.append(&ui_icon(icon, color, 13));
            c.append(&rotulo(text, ""));
            b.set_child(Some(&c));
            row.append(&b);
            b
        };
        let explain = item("spark", "#A5A1E2", tr("Explain error"));
        let copy = item("copiar", "#B9BAC4", tr("Copy"));
        let find = item("busca", "#B9BAC4", tr("Search"));
        pop.set_child(Some(&row));
        let text = term.text_selected(vte4::Format::Text).map(|s| s.to_string()).unwrap_or_default();
        let (w, xw, pw) = (Rc::downgrade(self), Rc::downgrade(x), pop.downgrade());
        let t2 = text.clone();
        explain.connect_clicked(move |_| {
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            let Some(s) = t.shell.upgrade() else { return };
            // (the agent never reads secrets: keys and values after "password=" and such are taken out)
            let clean = redact_secrets(t2.trim());
            let head = match &x.remote {
                Some(r) => trf("Explain this error from the terminal (on the server {host}):", &[("host", r.target.label())]),
                None => trf("Explain this error from the terminal (in {folder}):", &[("folder", x.cwd.borrow().display().to_string())]),
            };
            s.right_bar().ask_agent(&format!("{head}\n{clean}"));
            if debug() {
                eprintln!("debug terminal: explicar seleção ({} caracteres)", clean.len());
            }
        });
        let (tw, pw) = (term.downgrade(), pop.downgrade());
        copy.connect_clicked(move |_| {
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
            if let Some(term) = tw.upgrade() {
                term.copy_clipboard_format(vte4::Format::Text);
            }
        });
        let (w, xw, pw) = (Rc::downgrade(self), Rc::downgrade(x), pop.downgrade());
        find.connect_clicked(move |_| {
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                let first = text.lines().next().unwrap_or("").trim().to_string();
                t.open_search(&x, Some(&first));
            }
        });
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        crate::central::posicao_debug("terminal explicar erro", explain.upcast_ref());
        pop.popup();
        if debug() {
            eprintln!("debug terminal: menu da seleção");
        }
    }

    /// The search bar (Ctrl+Shift+F): over the panes, at the top right.
    pub(super) fn open_search(self: &Rc<Self>, x: &Rc<Tab>, text: Option<&str>) {
        let existing = x.search.borrow().clone();
        let s = match existing {
            Some(s) => s,
            None => {
                let bar = caixa(Orientation::Horizontal, 8, "term-busca");
                bar.set_halign(Align::End);
                bar.set_valign(Align::Start);
                bar.set_margin_top(10);
                bar.set_margin_end(16);
                bar.append(&ui_icon("busca", "#8E8F99", 13));
                let entry = gtk::Entry::new();
                entry.add_css_class("term-busca-campo");
                entry.set_width_chars(16);
                entry.set_placeholder_text(Some(tr("Search")));
                bar.append(&entry);
                let count = rotulo("", "term-busca-conta");
                bar.append(&count);
                let next = gtk::Button::new();
                next.add_css_class("term-busca-botao");
                next.set_child(Some(&ui_icon("chevron-baixo", "#8E8F99", 12)));
                next.set_tooltip_text(Some(tr("Next (Enter)")));
                bar.append(&next);
                let close = gtk::Button::new();
                close.add_css_class("term-busca-botao");
                close.set_child(Some(&ui_icon("fechar", "#8E8F99", 10)));
                close.set_tooltip_text(Some(tr("Close (Esc)")));
                bar.append(&close);
                x.page.add_overlay(&bar);
                let s = Rc::new(Search { bar, entry: entry.clone(), count, index: Cell::new(0), total: Cell::new(0) });
                *x.search.borrow_mut() = Some(s.clone());
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
                entry.connect_changed(move |e| {
                    if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                        t.search_for(&x, &e.text());
                    }
                });
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
                entry.connect_activate(move |_| {
                    if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                        t.search_next(&x);
                    }
                });
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
                next.connect_clicked(move |_| {
                    if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                        t.search_next(&x);
                    }
                });
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
                close.connect_clicked(move |_| {
                    if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                        t.close_search(&x);
                    }
                });
                let keys = gtk::EventControllerKey::new();
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
                keys.connect_key_pressed(move |_, k, _, _| {
                    if k == gdk::Key::Escape {
                        if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                            t.close_search(&x);
                        }
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                entry.add_controller(keys);
                crate::central::posicao_debug("terminal busca", entry.upcast_ref());
                s
            }
        };
        if let Some(t) = text {
            s.entry.set_text(t);
        }
        s.entry.grab_focus();
        s.entry.select_region(0, -1);
    }

    fn search_for(&self, x: &Rc<Tab>, text: &str) {
        let Some(s) = x.search.borrow().clone() else { return };
        let term = x.active();
        if text.is_empty() {
            term.search_set_regex(None, 0);
            s.count.set_text("");
            return;
        }
        let Ok(re) = vte4::Regex::for_search(&regex_escape(text), SEARCH_FLAGS) else { return };
        term.search_set_regex(Some(&re), 0);
        term.search_set_wrap_around(true);
        // (how many: the whole scrollback's text)
        let (lo, hi) = term.vadjustment().map(|a| (a.lower() as i64, a.upper() as i64)).unwrap_or((0, term.row_count()));
        let (all, _) = term.text_range_format(vte4::Format::Text, lo, 0, hi, term.column_count());
        let n = count_matches(all.as_deref().unwrap_or(""), text);
        s.total.set(n);
        s.index.set(0);
        term.unselect_all();
        if n > 0 {
            let found = term.search_find_previous();
            s.index.set(1);
            if debug() {
                eprintln!("debug terminal: busca achou {found}: «{}»", term.text_selected(vte4::Format::Text).unwrap_or_default());
            }
        }
        self.search_count(&s);
    }

    fn search_next(&self, x: &Rc<Tab>) {
        let Some(s) = x.search.borrow().clone() else { return };
        if s.total.get() == 0 {
            return;
        }
        x.active().search_find_previous();
        s.index.set(s.index.get() % s.total.get() + 1);
        self.search_count(&s);
    }

    fn search_count(&self, s: &Search) {
        s.count.set_text(&if s.total.get() == 0 { tr("none").to_string() } else { trf("{i} of {n}", &[("i", s.index.get().to_string()), ("n", s.total.get().to_string())]) });
        if debug() {
            eprintln!("debug terminal: busca «{}» {}", s.entry.text(), s.count.text());
        }
    }

    fn close_search(&self, x: &Rc<Tab>) {
        if let Some(s) = x.search.borrow_mut().take() {
            x.page.remove_overlay(&s.bar);
        }
        let term = x.active();
        term.search_set_regex(None, 0);
        term.unselect_all();
        term.grab_focus();
    }

    /// Splits the tab in two (or closes the second pane).
    pub(super) fn toggle_split(self: &Rc<Self>, x: &Rc<Tab>) {
        let old = x.split.borrow_mut().take();
        if let Some(s) = old {
            hang_up_pane(&s);
            x.panes.set_end_child(gtk::Widget::NONE);
            *x.focused.borrow_mut() = x.term.clone();
            x.term.grab_focus();
            self.update_status();
            return;
        }
        let term = new_vte();
        term.set_font_scale(x.term.font_scale());
        let scroll = scrolled(&term);
        scroll.add_css_class("term-painel-dois");
        // (design: the new pane slides in from the edge — the soft spring)
        let slide = kalango_movimento::Animavel::new(&scroll);
        x.panes.set_end_child(Some(&slide));
        if !kalango_movimento::reduzido() {
            slide.entrar((48.0, 0.0), 1.0);
        }
        x.panes.set_resize_end_child(true);
        x.panes.set_shrink_end_child(false);
        let width = x.panes.width();
        if width > 0 {
            x.panes.set_position(width / 2);
        }
        *x.split.borrow_mut() = Some(term.clone());
        self.wire_pane(x, &term);
        // the pane's end: it closes
        let (w, xw, tw) = (Rc::downgrade(self), Rc::downgrade(x), term.downgrade());
        term.connect_child_exited(move |_, _| {
            let (Some(t), Some(x), Some(term)) = (w.upgrade(), xw.upgrade(), tw.upgrade()) else { return };
            if x.split.borrow().as_ref() == Some(&term) {
                t.toggle_split(&x);
            }
        });
        let (argv, dir): (Vec<String>, PathBuf) = match &x.remote {
            // (the same host, through the session's connection: no second login)
            Some(r) => (r.argv(), glib::home_dir()),
            None => (vec![user_shell()], x.cwd.borrow().clone()),
        };
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let env = ["COLORTERM=truecolor", "TERM_PROGRAM=KalangoOS-Terminal"];
        let tw = term.downgrade();
        term.spawn_async(vte4::PtyFlags::DEFAULT, Some(&dir.display().to_string()), &argv_refs, &env, glib::SpawnFlags::DEFAULT, || {}, -1,
            gio::Cancellable::NONE, move |r| {
                if let (Ok(pid), Some(term)) = (r, tw.upgrade()) {
                    PANE_PIDS.with(|p| p.borrow_mut().push((term.downgrade(), pid.0)));
                }
            });
        *x.focused.borrow_mut() = term.clone();
        term.grab_focus();
        self.update_status();
        if debug() {
            eprintln!("debug terminal: dividida ({})", argv.first().map(String::as_str).unwrap_or(""));
        }
    }

    /// The card under the prompt while the agent's command waits on the line (design: "Comando do agente na linha,
    /// ainda não rodou · Enter roda · Esc apaga · Explicar").
    pub(super) fn show_agent_card(self: &Rc<Self>, x: &Rc<Tab>, cmd: &str) {
        self.hide_agent_card(x);
        x.hide_hint();
        let card = caixa(Orientation::Horizontal, 10, "term-agente");
        card.set_halign(Align::Start);
        card.set_valign(Align::Start);
        card.append(&ui_icon("spark", "#A5A1E2", 13));
        card.append(&rotulo(tr("The agent's command is on the line, not run yet"), "term-agente-texto"));
        for key in [tr("Enter runs it"), tr("Esc erases it")] {
            card.append(&rotulo(key, "term-agente-tecla"));
        }
        let explain = gtk::Button::with_label(tr("Explain"));
        explain.add_css_class("term-agente-explicar");
        card.append(&explain);
        let term = x.active();
        let cell = term.char_height().max(1) as i32;
        let row = cursor_screen_row(&term);
        let below = PAD + (row as i32 + 1) * cell + 8;
        let room = x.page.height();
        // (the second pane: the card under its own prompt)
        let left = if x.split.borrow().as_ref() == Some(&term) { x.panes.position() + PAD } else { PAD - 6 };
        card.set_margin_start(left);
        card.set_margin_top(if room > 0 && below + 48 > room { (PAD + row as i32 * cell - 48).max(8) } else { below });
        x.page.add_overlay(&card);
        *x.agent_card.borrow_mut() = Some(card);
        let (w, xw, cmd) = (Rc::downgrade(self), Rc::downgrade(x), cmd.to_string());
        explain.connect_clicked(move |_| {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            if let Some(s) = t.shell.upgrade() {
                s.right_bar().ask_agent(&trf("Explain what this terminal command does before I run it: {command}", &[("command", cmd.clone())]));
            }
            x.active().grab_focus();
        });
        crate::central::posicao_debug("terminal agente explicar", explain.upcast_ref());
        if debug() {
            eprintln!("debug terminal: comando do agente na linha (linha {row} da tela)");
        }
    }

    pub(super) fn hide_agent_card(&self, x: &Rc<Tab>) {
        if let Some(c) = x.agent_card.borrow_mut().take() {
            x.page.remove_overlay(&c);
        }
    }

    /// A dialog inside the Terminal's window (as "Fechar a aba?"): the screen dims, Esc or a click outside cancels.
    /// Returns what closes it.
    pub(super) fn show_dialog(self: &Rc<Self>, card: &gtk::Box, focus: &impl IsA<gtk::Widget>) -> Rc<dyn Fn()> {
        let old = self.asking.borrow_mut().take();
        if let Some(close) = old {
            close();
        }
        let dim = caixa(Orientation::Vertical, 0, "arq-escurecer");
        dim.set_hexpand(true);
        dim.set_vexpand(true);
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        self.over.add_overlay(&dim);
        self.over.add_overlay(card);
        let (ow, dw, cw, w) = (self.over.downgrade(), dim.downgrade(), card.downgrade(), Rc::downgrade(self));
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if let (Some(o), Some(dm), Some(c)) = (ow.upgrade(), dw.upgrade(), cw.upgrade()) {
                if c.parent().is_some() {
                    o.remove_overlay(&c);
                    o.remove_overlay(&dm);
                }
            }
            if let Some(t) = w.upgrade() {
                t.asking.borrow_mut().take();
                if let Some(x) = t.tabs.borrow().get(t.current.get()) {
                    x.active().grab_focus();
                }
            }
        });
        *self.asking.borrow_mut() = Some(close.clone());
        let outside = gtk::GestureClick::new();
        let c = close.clone();
        outside.connect_released(move |_, _, _, _| c());
        dim.add_controller(outside);
        focus.grab_focus();
        close
    }

    /// The dialog's title row: an icon in its square, the title and a line under it.
    pub(super) fn dialog_head(icon: &str, color: &str, square: &str, title: &str, sub: &str) -> gtk::Box {
        let top = caixa(Orientation::Horizontal, 14, "");
        let alert = caixa(Orientation::Horizontal, 0, square);
        alert.set_size_request(44, 44);
        alert.set_valign(Align::Start);
        alert.set_hexpand(false);
        let i = ui_icon(icon, color, 20);
        i.set_hexpand(true);
        alert.append(&i);
        top.append(&alert);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let t = rotulo(title, "arq-dialogo-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(1);
        col.append(&t);
        let s = rotulo(sub, "arq-sub");
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_max_width_chars(1);
        col.append(&s);
        top.append(&col);
        top
    }

    /// The agent's command for a production host, or one that erases or restarts something: the person says yes
    /// before it goes to the line (the buttons answer after 0.7 s, as the system's approval screen).
    pub(super) fn approve_agent_command(self: &Rc<Self>, x: &Rc<Tab>, cmd: String, why: String) {
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.set_size_request(560, -1);
        let sub = match x.remote.as_ref().filter(|r| r.target.production) {
            Some(r) => trf("{host} is a production server: the command goes to the line only with your approval.", &[("host", r.target.label())]),
            None => tr("This command erases or restarts something: it goes to the line only with your approval.").to_string(),
        };
        card.append(&Self::dialog_head("spark", "#A5A1E2", "term-dialogo-agente", tr("The agent wants to put a command on the line"), &sub));
        let line = caixa(Orientation::Vertical, 6, "ed-mudou");
        let c = rotulo(&cmd, "term-dialogo-comando");
        c.set_xalign(0.0);
        c.set_wrap(true);
        c.set_wrap_mode(pango::WrapMode::WordChar);
        c.set_selectable(true);
        line.append(&c);
        if !why.is_empty() {
            let y = rotulo(&why, "arq-sub");
            y.set_xalign(0.0);
            y.set_wrap(true);
            line.append(&y);
        }
        card.append(&line);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_halign(Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("arq-acao-texto");
        let put = gtk::Button::with_label(tr("Put it on the line"));
        put.add_css_class("arq-acao-principal");
        put.set_sensitive(false);
        buttons.append(&cancel);
        buttons.append(&put);
        card.append(&buttons);
        let close = self.show_dialog(&card, &cancel);
        let pw = put.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
            if let Some(p) = pw.upgrade() {
                p.set_sensitive(true);
            }
        });
        let c = close.clone();
        cancel.connect_clicked(move |_| c());
        let (c, w, xw) = (close.clone(), Rc::downgrade(self), Rc::downgrade(x));
        put.connect_clicked(move |_| {
            c();
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                t.put_agent_command(&x, &cmd, &why);
            }
        });
        crate::central::posicao_debug("terminal aprovar comando", put.upcast_ref());
        crate::central::posicao_debug("terminal cancelar comando", cancel.upcast_ref());
        if debug() {
            eprintln!("debug terminal: aprovação do comando do agente");
        }
    }

    /// Enter on a production host, for a command that erases or restarts something.
    fn confirm_in_production(self: &Rc<Self>, x: &Rc<Tab>, term: &vte4::Terminal, cmd: &str) {
        let Some(r) = &x.remote else { return };
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.set_size_request(540, -1);
        card.append(&Self::dialog_head("alerta", "#EFA2A6", "term-dialogo-perigo", &trf("Run on {host}?", &[("host", r.target.name.clone())]),
            &trf("{host} is a production server, and this command erases or restarts something.", &[("host", r.target.label())])));
        let line = caixa(Orientation::Horizontal, 0, "ed-mudou");
        let c = rotulo(cmd, "term-dialogo-comando");
        c.set_xalign(0.0);
        c.set_wrap(true);
        c.set_wrap_mode(pango::WrapMode::WordChar);
        line.append(&c);
        card.append(&line);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_halign(Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("arq-acao-texto");
        let run = gtk::Button::with_label(tr("Run"));
        run.add_css_class("arq-acao-perigo");
        buttons.append(&cancel);
        buttons.append(&run);
        card.append(&buttons);
        let close = self.show_dialog(&card, &cancel);
        let c = close.clone();
        cancel.connect_clicked(move |_| c());
        let (c, tw) = (close.clone(), term.downgrade());
        run.connect_clicked(move |_| {
            c();
            if let Some(term) = tw.upgrade() {
                term.feed_child(b"\r");
            }
        });
        crate::central::posicao_debug("terminal rodar em producao", run.upcast_ref());
        crate::central::posicao_debug("terminal cancelar em producao", cancel.upcast_ref());
        if debug() {
            eprintln!("debug terminal: confirmar em produção «{cmd}»");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_and_line_helpers() {
        assert_eq!(regex_escape("a.b*(c)"), "a\\.b\\*\\(c\\)");
        assert_eq!(count_matches("ERROR timeout\nINFO\nerror again TIMEOUT", "timeout"), 2);
        assert_eq!(count_matches("x", ""), 0);
        assert_eq!(line_command("deploy@app-prod-01:~/app$ sudo rm -rf /var/log/app   "), "sudo rm -rf /var/log/app");
        assert_eq!(line_command("➜ ~/KalangoOS ls"), "~/KalangoOS ls");
        assert_eq!(line_command("reboot"), "reboot");
    }
}
