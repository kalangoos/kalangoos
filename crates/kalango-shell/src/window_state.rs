//! What an app window holds that the person can see or left there, written down so a close and reopen can be compared
//! with what was there before (`kalango-shell estado …`; sessao/testar-janelas.sh): every widget's type, whether it
//! shows, its place, and its state — text and cursor of a field, the page of a stack, the scroll, a toggle, a value, a
//! label's words, the terminal's text —, and the window's own: title, size, focus, whether it is realized and how many
//! native surfaces it holds.
//!
//! The windows are found by their widget name, their app id (`os.kalango.Calculadora`): superficies::app_window names
//! each app's window.
//!
//! `estado gravar APP FILE`  the window's state to FILE
//! `estado prazo MS`          the delay before a window closed from now on is unrealized (a huge one: never)
//! `estado texto APP PATH TEXT` puts TEXT in the field at PATH (as the dump numbers the widgets)
//! `estado rolar APP PATH N`  scrolls the scrolled window at PATH to N
//! `estado pagina APP PATH NAME` shows the page NAME of the stack at PATH
//! `estado ativar APP PATH`   what a click does to the button, toggle or list row at PATH

use gtk::prelude::*;
use gtk::gdk;

/// The app window named `app`, among the shell's windows (hidden or not).
pub(crate) fn window_of(app: &str) -> Option<gtk::Window> {
    gtk::Window::list_toplevels().into_iter().filter_map(|w| w.downcast::<gtk::Window>().ok()).find(|w| w.widget_name() == app)
}

/// Every realized native under `root` (itself included): each one holds a surface and a renderer.
pub(crate) fn realized_natives(root: &gtk::Widget) -> usize {
    let mut n = usize::from(root.is_realized() && root.dynamic_cast_ref::<gtk::Native>().is_some());
    let mut c = root.first_child();
    while let Some(k) = c {
        n += realized_natives(&k);
        c = k.next_sibling();
    }
    n
}

fn short(s: &str) -> String {
    s.chars().take(160).collect::<String>().replace('\n', "\\n")
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn hash_bytes(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

fn paintable_note(p: Option<gdk::Paintable>) -> String {
    match p {
        None => "paintable=none".into(),
        Some(p) => match p.downcast_ref::<gdk::Texture>() {
            // (an icon's pixels too, hashed: which icon a button shows — "play" or "pause" — is the same size)
            Some(t) if t.width() * t.height() <= 256 * 256 => {
                let mut px = vec![0u8; (t.width() * t.height() * 4) as usize];
                t.download(&mut px, t.width() as usize * 4);
                format!("texture={}x{}#{:x}", t.width(), t.height(), hash_bytes(&px))
            }
            Some(t) => format!("texture={}x{}", t.width(), t.height()),
            None => format!("paintable={}", p.type_().name()),
        },
    }
}

/// What a widget holds that the person can see or left there.
fn state_of(w: &gtk::Widget) -> String {
    let mut s = Vec::new();
    if let Some(t) = w.downcast_ref::<gtk::TextView>() {
        let b = t.buffer();
        let text = b.text(&b.start_iter(), &b.end_iter(), false);
        s.push(format!("text={:?} cursor={} undo={} redo={} sel={}", short(&text), b.cursor_position(), b.can_undo(), b.can_redo(), b.has_selection()));
    } else if let Some(e) = w.dynamic_cast_ref::<gtk::Editable>() {
        s.push(format!("text={:?} pos={} sel={:?}", short(&e.text()), e.position(), e.selection_bounds()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Label>() {
        s.push(format!("label={:?}", short(&x.text())));
    }
    if let Some(x) = w.downcast_ref::<gtk::Stack>() {
        s.push(format!("page={:?}", x.visible_child_name().map(|n| n.to_string())));
    }
    if let Some(x) = w.downcast_ref::<gtk::ScrolledWindow>() {
        let (v, h) = (x.vadjustment(), x.hadjustment());
        s.push(format!("v={:.0}/{:.0} h={:.0}/{:.0}", v.value(), v.upper(), h.value(), h.upper()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Popover>() {
        s.push(format!("popover open={}", x.get_visible()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Revealer>() {
        s.push(format!("reveal={}", x.reveals_child()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Switch>() {
        s.push(format!("active={}", x.is_active()));
    }
    if let Some(x) = w.downcast_ref::<gtk::CheckButton>() {
        s.push(format!("active={}", x.is_active()));
    }
    if let Some(x) = w.downcast_ref::<gtk::ToggleButton>() {
        s.push(format!("active={}", x.is_active()));
    }
    if let Some(x) = w.dynamic_cast_ref::<gtk::Range>() {
        s.push(format!("value={:.3}", x.value()));
    }
    if let Some(x) = w.downcast_ref::<gtk::DropDown>() {
        s.push(format!("selected={}", x.selected()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Paned>() {
        s.push(format!("position={}", x.position()));
    }
    if let Some(x) = w.downcast_ref::<gtk::ListBox>() {
        let sel: Vec<i32> = x.selected_rows().iter().map(|r| r.index()).collect();
        s.push(format!("selected={sel:?}"));
    }
    if let Some(x) = w.downcast_ref::<gtk::Picture>() {
        s.push(paintable_note(x.paintable()));
    }
    if let Some(x) = w.downcast_ref::<gtk::Image>() {
        s.push(paintable_note(x.paintable()));
    }
    if let Some(x) = w.downcast_ref::<gtk::DrawingArea>() {
        s.push(format!("content={}x{}", x.content_width(), x.content_height()));
    }
    if let Some(x) = w.downcast_ref::<vte4::Terminal>() {
        use vte4::prelude::*;
        let text = x.text_format(vte4::Format::Text).map(|t| t.to_string()).unwrap_or_default();
        let last: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = last.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join(" ⏎ ");
        s.push(format!("terminal text={:x} ({} chars) tail={:?} cursor={:?}", hash(&text), text.chars().count(), short(&tail), x.cursor_position()));
    }
    s.join(" ")
}

fn walk(w: &gtk::Widget, root: &gtk::Widget, focus: Option<&gtk::Widget>, path: &str, out: &mut String, widgets: &mut usize) {
    *widgets += 1;
    let b = w.compute_bounds(root).map(|b| format!("{:.0},{:.0} {:.0}x{:.0}", b.x(), b.y(), b.width(), b.height())).unwrap_or_else(|| "-".into());
    let classes: Vec<String> = w.css_classes().iter().map(|c| c.to_string()).collect();
    let state = state_of(w);
    out.push_str(&format!(
        "{path} {} v={} s={} [{b}] .{}{}{}\n",
        w.type_().name(),
        u8::from(w.get_visible()),
        u8::from(w.is_sensitive()),
        classes.join("."),
        if state.is_empty() { String::new() } else { format!(" | {state}") },
        if focus == Some(w) { " FOCUS" } else { "" },
    ));
    let (mut c, mut i) = (w.first_child(), 0);
    while let Some(k) = c {
        // (GTK's tooltip window is rebuilt each time a tooltip shows: not the person's)
        if k.type_().name() != "GtkTooltipWindow" {
            walk(&k, root, focus, &format!("{path}/{i}"), out, widgets);
        }
        c = k.next_sibling();
        i += 1;
    }
}

/// The window's state, as text (none when the app's window was never built).
pub(crate) fn dump(app: &str) -> String {
    let Some(win) = window_of(app) else { return format!("WINDOW {app} none\n") };
    let focus = GtkWindowExt::focus(&win);
    let (mut body, mut widgets) = (String::new(), 0);
    walk(win.upcast_ref(), win.upcast_ref(), focus.as_ref(), "w", &mut body, &mut widgets);
    let (dw, dh) = win.default_size();
    format!(
        "WINDOW {app} title={:?} visible={} realized={} natives={} size={}x{} default={}x{} maximized={} widgets={widgets} focus={}\n{body}",
        win.title().map(|t| t.to_string()).unwrap_or_default(),
        win.is_visible(),
        win.is_realized(),
        realized_natives(win.upcast_ref()),
        win.width(),
        win.height(),
        dw,
        dh,
        win.is_maximized(),
        focus.map(|f| f.type_().name().to_string()).unwrap_or_else(|| "none".into()),
    )
}

/// The widget at `path` ("w/0/3/1", as the dump numbers them) under the window.
fn at_path(win: &gtk::Window, path: &str) -> Option<gtk::Widget> {
    let mut w: gtk::Widget = win.clone().upcast();
    for part in path.split('/').skip(1).filter(|p| !p.is_empty()) {
        let n: usize = part.parse().ok()?;
        let mut c = w.first_child();
        for _ in 0..n {
            c = c?.next_sibling();
        }
        w = c?;
    }
    Some(w)
}

pub(crate) fn command(args: &[String]) {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    let (op, app) = (arg(0), arg(1));
    if op == "prazo" {
        crate::superficies::set_release_delay(app.parse().unwrap_or(u64::from(u32::MAX)));
        eprintln!("debug estado: prazo {app} ms");
        return;
    }
    if op == "gravar" {
        match std::fs::write(arg(2), dump(app)) {
            Ok(()) => eprintln!("debug estado: {app} gravado em {}", arg(2)),
            Err(e) => eprintln!("debug estado: {app} não gravado: {e}"),
        }
        return;
    }
    let Some(w) = window_of(app).and_then(|win| at_path(&win, arg(2))) else {
        eprintln!("debug estado: {op}: nada em {app} {}", arg(2));
        return;
    };
    match op {
        "texto" => {
            if let Some(e) = w.dynamic_cast_ref::<gtk::Editable>() {
                e.set_text(arg(3));
            } else if let Some(t) = w.downcast_ref::<gtk::TextView>() {
                t.buffer().set_text(arg(3));
            }
        }
        "rolar" => {
            if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
                sw.vadjustment().set_value(arg(3).parse().unwrap_or(0.0));
            }
        }
        // what a click does: a button clicked, a toggle switched on, a row of a list chosen
        "ativar" => {
            if let Some(t) = w.downcast_ref::<gtk::ToggleButton>() {
                t.set_active(true);
            } else if let Some(b) = w.downcast_ref::<gtk::Button>() {
                b.emit_clicked();
            } else if let Some(r) = w.downcast_ref::<gtk::ListBoxRow>() {
                if let Some(l) = r.parent().and_downcast::<gtk::ListBox>() {
                    l.select_row(Some(r));
                }
                r.activate();
            } else if let Some(c) = w.downcast_ref::<gtk::CheckButton>() {
                c.set_active(!c.is_active());
            } else {
                w.activate();
            }
        }
        "pagina" => {
            if let Some(s) = w.downcast_ref::<gtk::Stack>() {
                s.set_visible_child_name(arg(3));
            }
        }
        o => {
            eprintln!("debug estado: comando desconhecido: {o}");
            return;
        }
    }
    eprintln!("debug estado: {op} em {app} {}", arg(2));
}
