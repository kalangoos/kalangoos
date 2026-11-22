//! What the shell holds in memory, for finding leaks (`kalango-shell memoria`, `memoria aparar`): the process's RSS,
//! the C heap in use and free (glibc's mallinfo2), and the GObject types with the most live instances. The counts
//! need `GOBJECT_DEBUG=instance-count` in the shell's environment (glib counts nothing without it).

use gtk::glib;
use gtk::glib::gobject_ffi;
use gtk::prelude::*;

#[repr(C)]
#[derive(Default)]
struct MallInfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

extern "C" {
    fn mallinfo2() -> MallInfo2;
    fn malloc_trim(pad: usize) -> i32;
}

/// The resident set in MB, from /proc.
pub fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| parse_rss_kb(&s))
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

/// VmRSS (kB) from the text of /proc/PID/status.
pub fn parse_rss_kb(status: &str) -> Option<u64> {
    status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.split_whitespace().next()).and_then(|n| n.parse().ok())
}

/// Every type derived from `t` (itself included), depth first.
fn all_types(t: glib::ffi::GType, out: &mut Vec<glib::ffi::GType>) {
    out.push(t);
    let mut n = 0u32;
    // SAFETY: g_type_children returns a newly allocated array of n GTypes, freed below with g_free
    unsafe {
        let kids = gobject_ffi::g_type_children(t, &mut n);
        for i in 0..n as usize {
            all_types(*kids.add(i), out);
        }
        glib::ffi::g_free(kids as *mut _);
    }
}

/// (type name, live instances), the biggest first; empty without GOBJECT_DEBUG=instance-count.
pub fn instance_counts() -> Vec<(String, i32)> {
    let mut types = Vec::new();
    all_types(gobject_ffi::G_TYPE_OBJECT, &mut types);
    let mut counts: Vec<(String, i32)> = types
        .into_iter()
        .filter_map(|t| {
            // SAFETY: t is a registered type; both calls only read glib's type tables
            let n = unsafe { gobject_ffi::g_type_get_instance_count(t) };
            // SAFETY: t comes from glib's own type tree
            (n > 0).then(|| (unsafe { glib::translate::from_glib::<_, glib::Type>(t) }.name().to_string(), n))
        })
        .collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}

/// Widgets under `w`, itself included.
fn widget_count(w: &gtk::Widget) -> usize {
    let mut n = 1;
    let mut c = w.first_child();
    while let Some(k) = c {
        n += widget_count(&k);
        c = k.next_sibling();
    }
    n
}

/// One line for the log: RSS, heap in use / free, the windows and their widgets (the `top` biggest), and the live
/// GObjects by type when glib counts them.
pub fn report(top: usize) -> String {
    // SAFETY: mallinfo2 only reads the allocator's statistics
    let m = unsafe { mallinfo2() };
    let wins = gtk::Window::list_toplevels();
    let mut per: Vec<(String, usize, bool)> = wins
        .iter()
        .map(|w| {
            let name = w.downcast_ref::<gtk::Window>().and_then(|w| w.title()).map(|t| t.to_string()).filter(|t| !t.is_empty());
            // (a layer surface has no title: its own style class names it, past GTK's generic ones)
            let class = || w.css_classes().iter().map(|c| c.to_string()).find(|c| !["background", "csd", "solid-csd"].contains(&c.as_str()));
            (name.or_else(class).unwrap_or_else(|| w.type_().name().into()), widget_count(w), w.is_visible())
        })
        .collect();
    let widgets: usize = per.iter().map(|p| p.1).sum();
    let natives: usize = wins.iter().map(crate::window_state::realized_natives).sum();
    per.sort_by_key(|a| std::cmp::Reverse(a.1));
    let tops: Vec<String> = per.iter().take(top).map(|(n, c, v)| format!("{n}{}={c}", if *v { "" } else { "(oculta)" })).collect();
    let counts = instance_counts();
    let objects: i64 = counts.iter().map(|c| c.1 as i64).sum();
    let types: Vec<String> = counts.iter().take(top).map(|(n, c)| format!("{n}={c}")).collect();
    format!(
        "rss {} MB, heap em uso {} MB, livre {} MB, janelas {} ({} visíveis), nativos realizados {natives}, widgets {widgets}: {} | objetos {objects} {}",
        rss_mb(),
        (m.uordblks + m.hblkhd) >> 20,
        m.fordblks >> 20,
        per.len(),
        per.iter().filter(|p| p.2).count(),
        tops.join(" "),
        types.join(" ")
    )
}

/// Gives the heap's free pages back to the system (to tell fragmentation from live memory; and after an app's
/// window lets go of its surface).
pub fn trim() {
    // SAFETY: malloc_trim is safe to call at any time on glibc
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rss_from_status() {
        let s = "Name:\tkalango-shell\nVmPeak:\t  999 kB\nVmRSS:\t  786432 kB\nRssAnon:\t 1 kB\n";
        assert_eq!(parse_rss_kb(s), Some(786432));
        assert_eq!(parse_rss_kb("Name: x\n"), None);
    }
}
