//! Text input for windows that are hidden and shown again (the apps that hide on close, the Editor, the panels).
//!
//! GTK 4.14 on Wayland keeps a single "current" text field per display for the compositor's text-input, moved by the
//! focus-in and focus-out of text widgets. A window hidden while it is active never gets its focus-out (the
//! keyboard's leave reaches a window that is no longer mapped and is dropped), so when it is shown again GTK still
//! takes it for active and its focused field gets no focus-in. If another field took the text input meanwhile (the
//! print dialog's page range, then closed), the compositor never hears of this field again: no input method, no
//! on-screen keyboard rising for it. Shown again, such a window's focus is given anew, which tells GTK.

use gtk::prelude::*;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Every window of the application, as it is added.
pub fn watch(app: &gtk::Application) {
    app.connect_window_added(|_, window| {
        window.connect_map(refocus);
    });
}

fn refocus(window: &gtk::Window) {
    // (a window shown for the first time, or hidden while another had the keyboard, gets its focus-in from GTK)
    if !window.is_active() {
        return;
    }
    let Some(focus) = GtkWindowExt::focus(window) else { return };
    GtkWindowExt::set_focus(window, None::<&gtk::Widget>);
    // (an entry's text keeps its selection: grabbing the focus would select it all)
    match focus.downcast_ref::<gtk::Text>() {
        Some(text) => {
            text.grab_focus_without_selecting();
        }
        None => {
            focus.grab_focus();
        }
    }
    if debug() {
        eprintln!("debug foco: {} mostrada de novo, foco devolvido a {}", window.title().map(|t| t.to_string()).unwrap_or_default(), focus.type_().name());
    }
}
