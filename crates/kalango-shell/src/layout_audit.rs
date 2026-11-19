//! `kalango-shell auditar-layout`: checks the open windows' layout at the size they have now, so a screen does not
//! "burst" on a bigger or a smaller screen. It reports (in the log, "debug layout: …"):
//!
//! - a part made to a fixed width (a side bar, a details pane, a search field: `set_size_request(w, -1)`) that grew
//!   because something inside it asks to expand — GTK passes `hexpand` up to every ancestor that did not say otherwise,
//!   so a label that fills its row makes the whole side bar take half of a wide window. The same for heights.
//! - a part that goes past its window's edges (outside anything that scrolls or clips).
//!
//! The suite resizes the windows to several screen sizes and runs it after each one.

use gtk::prelude::*;

/// What the audit found in one window.
struct Finding {
    window: String,
    text: String,
}

/// A widget as the log names it: its type, then its CSS classes or name ("GtkBox.arq-lateral").
fn name_of(w: &gtk::Widget) -> String {
    let classes: Vec<String> = w.css_classes().iter().map(|c| c.to_string()).collect();
    let mut n = w.type_().name().to_string();
    if !classes.is_empty() {
        n.push('.');
        n.push_str(&classes.join("."));
    } else if !w.widget_name().is_empty() && w.widget_name() != w.type_().name() {
        n.push('#');
        n.push_str(&w.widget_name());
    }
    n
}

/// The path from the window to a widget, by the nearest ancestors that have a class (short enough to read).
fn path_of(w: &gtk::Widget) -> String {
    let mut parts = vec![name_of(w)];
    let mut p = w.parent();
    while let Some(x) = p {
        if x.is::<gtk::Window>() {
            break;
        }
        if !x.css_classes().is_empty() && parts.len() < 4 {
            parts.push(name_of(&x));
        }
        p = x.parent();
    }
    parts.reverse();
    parts.join(" › ")
}

/// The first descendant that asks to expand on its own (the cause of a fixed part growing).
fn who_expands(w: &gtk::Widget, o: gtk::Orientation) -> Option<gtk::Widget> {
    let mut c = w.first_child();
    while let Some(x) = c {
        if x.is_visible() {
            let (set, on) = match o {
                gtk::Orientation::Horizontal => (x.is_hexpand_set(), x.hexpands()),
                _ => (x.is_vexpand_set(), x.vexpands()),
            };
            if set && on {
                return Some(x);
            }
            if !set || on {
                if let Some(d) = who_expands(&x, o) {
                    return Some(d);
                }
            }
        }
        c = x.next_sibling();
    }
    None
}

/// The label inside that asks the most width (natural), when it asks more than `limit`.
fn widest_label(w: &gtk::Widget, limit: i32) -> Option<(gtk::Widget, i32)> {
    let mut best: Option<(gtk::Widget, i32)> = None;
    let mut stack = vec![w.clone()];
    while let Some(x) = stack.pop() {
        if !x.is_visible() {
            continue;
        }
        if x.is::<gtk::Label>() {
            let (_, nat, _, _) = x.measure(gtk::Orientation::Horizontal, -1);
            if nat > limit / 2 && best.as_ref().is_none_or(|(_, n)| nat > *n) {
                best = Some((x.clone(), nat));
            }
        }
        let mut c = x.first_child();
        while let Some(y) = c {
            stack.push(y.clone());
            c = y.next_sibling();
        }
    }
    best
}

/// Whether something between the widget and the window scrolls or clips (what is past an edge there is meant to be).
fn clipped(w: &gtk::Widget) -> bool {
    let mut p = w.parent();
    while let Some(x) = p {
        if x.is::<gtk::Window>() {
            return false;
        }
        if x.is::<gtk::Viewport>() || x.is::<gtk::ScrolledWindow>() || x.is::<gtk::Stack>() || x.overflow() == gtk::Overflow::Hidden {
            return true;
        }
        p = x.parent();
    }
    false
}

fn walk(w: &gtk::Widget, win: &gtk::Window, title: &str, out: &mut Vec<Finding>) {
    if !w.is_visible() || !w.is_mapped() {
        return;
    }
    let (rw, rh) = w.size_request();
    // a fixed part that grew with a child's expand: in a box that lays it out in that direction
    let in_box = w.parent().and_downcast::<gtk::Box>();
    for (o, req, got, set) in [(gtk::Orientation::Horizontal, rw, w.width(), w.is_hexpand_set()), (gtk::Orientation::Vertical, rh, w.height(), w.is_vexpand_set())] {
        let along = in_box.as_ref().is_some_and(|b| b.orientation() == o);
        let grows = w.compute_expand(o) && !(set && !w.compute_expand(o));
        if req > 0 && along && !set && grows && got > req + 2 {
            let cause = who_expands(w, o).map(|c| path_of(&c)).unwrap_or_default();
            out.push(Finding {
                window: title.to_string(),
                text: format!("{} pedia {req} px de {}, ficou com {got} (cresce com {cause})", path_of(w), if o == gtk::Orientation::Horizontal { "largura" } else { "altura" }),
            });
        } else if o == gtk::Orientation::Horizontal && req >= 160 && along && !grows && got > req + 2 && (w.is::<gtk::Box>() || w.is::<gtk::ScrolledWindow>()) {
            // a pane that does not expand but grew to its content's natural width: a long text that does not wrap or cut
            let cause = widest_label(w, req).map(|(l, n)| {
                let t: String = l.downcast_ref::<gtk::Label>().map(|l| l.text().chars().take(48).collect()).unwrap_or_default();
                format!("{} “{t}” quer {n} px", path_of(&l))
            }).unwrap_or_default();
            out.push(Finding { window: title.to_string(), text: format!("{} pedia {req} px de largura, ficou com {got} pelo conteúdo ({cause}; mínimos: {})", path_of(w), who_asks(w, o)) });
        }
    }
    // past the window's edges
    if let Some(b) = w.compute_bounds(win) {
        let (x0, y0, x1, y1) = (b.x(), b.y(), b.x() + b.width(), b.y() + b.height());
        let (ww, wh) = (win.width() as f32, win.height() as f32);
        if (x0 < -1.0 || y0 < -1.0 || x1 > ww + 1.0 || y1 > wh + 1.0) && b.width() > 1.0 && !clipped(w) {
            out.push(Finding { window: title.to_string(), text: format!("{} passa da janela ({x0:.0},{y0:.0} → {x1:.0},{y1:.0} numa de {ww:.0}×{wh:.0})", path_of(w)) });
        }
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        walk(&x, win, title, out);
        c = x.next_sibling();
    }
}

/// What makes a window ask more than its screen: from the window down, the child that asks the most at each level
/// (a Stack's hidden pages count too, when it keeps one size for all), until a widget asks it by itself.
fn who_asks(w: &gtk::Widget, o: gtk::Orientation) -> String {
    let mut chain = Vec::new();
    let mut cur = w.clone();
    for _ in 0..24 {
        let (min, _, _, _) = cur.measure(o, -1);
        let mut best: Option<(gtk::Widget, i32)> = None;
        let mut c = cur.first_child();
        while let Some(x) = c {
            if x.is_visible() || cur.is::<gtk::Stack>() {
                let (m, _, _, _) = x.measure(o, -1);
                if best.as_ref().is_none_or(|(_, b)| m > *b) {
                    best = Some((x.clone(), m));
                }
            }
            c = x.next_sibling();
        }
        if let Some(st) = cur.downcast_ref::<gtk::Stack>() {
            // a stack: each page's own minimum (the widest page sets it for all)
            let mut pages = Vec::new();
            let mut c = cur.first_child();
            while let Some(x) = c {
                let (m, _, _, _) = x.measure(o, -1);
                pages.push(format!("{}={m}", st.page(&x).name().map(|n| n.to_string()).unwrap_or_default()));
                c = x.next_sibling();
            }
            chain.push(format!("{} {min} [{}]", name_of(&cur), pages.join(" ")));
        } else {
            chain.push(format!("{} {min}", name_of(&cur)));
        }
        match best {
            // a child asks most of it: the cause is further in
            Some((x, m)) if m * 10 >= min * 6 && m > 0 => cur = x,
            _ => break,
        }
    }
    chain.join(" › ")
}

/// Audits every visible window of the shell; prints what it found and how many.
pub fn run() {
    let mut out = Vec::new();
    let mut n = 0;
    for t in gtk::Window::list_toplevels() {
        let Ok(win) = t.downcast::<gtk::Window>() else { continue };
        if !win.is_visible() || !win.is_mapped() || win.width() < 200 {
            continue;
        }
        n += 1;
        let title = win.title().map(|t| t.to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| name_of(win.upcast_ref()));
        if let Some(c) = win.child() {
            walk(&c, &win, &title, &mut out);
            // bigger than its screen: the window itself burst (what asks for it)
            let monitor = win.surface().and_then(|s| WidgetExt::display(&win).monitor_at_surface(&s));
            if let Some(g) = monitor.map(|m| m.geometry()) {
                for (o, got, max, what) in [(gtk::Orientation::Horizontal, win.width(), g.width(), "largura"), (gtk::Orientation::Vertical, win.height(), g.height(), "altura")] {
                    if got > max {
                        out.push(Finding { window: title.clone(), text: format!("a janela tem {got} px de {what} numa tela de {max}: {}", who_asks(&c, o)) });
                    }
                }
            }
        }
        eprintln!("debug layout: janela {title} ({}×{})", win.width(), win.height());
    }
    for f in &out {
        eprintln!("debug layout: [{}] {}", f.window, f.text);
    }
    eprintln!("debug layout: {n} janelas, {} problemas", out.len());
}
