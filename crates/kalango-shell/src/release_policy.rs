//! When a closed app window may let go of what it holds of the display (superficies::release_resources_when_hidden
//! asks after its delay). Letting go unrealizes the window: its surface, its renderer and its popovers' surfaces go;
//! every widget and the app itself stay. So only what depends on the window's own surface waits; answering false keeps
//! it realized a while longer, and it is asked again later. (A dialog open over the window holds it too:
//! superficies checks that for every window.)
//!
//! What does not hold a window, and why:
//! - Music with a track, even playing: its GtkMediaFile is never given the window's surface (no GtkVideo shows it), so
//!   playback, MPRIS and the island go on (sessao/testar-memoria.sh plays it behind the closed window);
//! - an install in the App Center, an operation in Disks, an agent's work in Files, an answer the Calendar asked for,
//!   a summary in Documents, an edit being made in Images: threads and callbacks that write into widgets, which stay;
//! - the Terminal: closing it hangs up its shells first (as it always did), so no VTE with a live shell is let go.

/// Video keeps a video playing: its frames keep coming into the window's picture.
pub fn video(playing: bool) -> bool {
    !playing
}

/// Documents keeps a print going: the print dialog (through the portal) knows the window by its surface.
pub fn documents(printing: bool) -> bool {
    !printing
}

/// A player's progress timer runs while its window shows or something plays (else it sleeps until either).
pub fn keep_ticking(window_visible: bool, playing: bool) -> bool {
    window_visible || playing
}

/// The delay before a closed window lets go: `KALANGO_SOLTAR_JANELA_MS` when set (the tests), else a minute.
pub fn delay_ms(env: Option<&str>) -> u64 {
    env.and_then(|v| v.trim().parse().ok()).unwrap_or(60_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_depends_on_the_surface_waits() {
        assert!(!video(true));
        assert!(video(false));
        assert!(!documents(true));
        assert!(documents(false));
        assert!(keep_ticking(true, false));
        assert!(keep_ticking(false, true));
        assert!(!keep_ticking(false, false));
    }

    #[test]
    fn the_delay_is_a_minute_unless_set() {
        assert_eq!(delay_ms(None), 60_000);
        assert_eq!(delay_ms(Some("1500")), 1500);
        assert_eq!(delay_ms(Some(" 800 ")), 800);
        assert_eq!(delay_ms(Some("x")), 60_000);
    }

    /// The rule, read from the sources (the apps need the whole shell to be built): every app window lets go of its
    /// surface after the delay, once, with its guard where it has one — and none is destroyed or dropped to be built
    /// anew (what is in a window lives in its widgets: a window built again would come back empty).
    #[test]
    fn every_app_window_is_unrealized_and_none_destroyed() {
        let apps: [(&str, &str, &str, Option<&str>); 15] = [
            ("calculadora.rs", include_str!("calculadora.rs"), "Calculadora", None),
            ("help_app.rs", include_str!("help_app.rs"), "Ajuda", None),
            ("disks_app.rs", include_str!("disks_app.rs"), "Discos", None),
            ("app_store.rs", include_str!("app_store.rs"), "Apps", None),
            ("calendar_app.rs", include_str!("calendar_app.rs"), "Calendario", None),
            ("music_player.rs", include_str!("music_player.rs"), "Musica", None),
            ("video_player.rs", include_str!("video_player.rs"), "Video", Some("release_policy::video(PLAYER.with(|x| x.borrow().as_ref().is_some_and(|p| p.playing.get())))")),
            ("image_viewer.rs", include_str!("image_viewer.rs"), "Imagens", None),
            ("arquivos.rs", include_str!("arquivos.rs"), "Arquivos", None),
            ("central.rs", include_str!("central.rs"), "Agentes", None),
            ("configuracoes.rs", include_str!("configuracoes.rs"), "Configuracoes", None),
            ("doc_reader.rs", include_str!("doc_reader.rs"), "Documentos", Some("release_policy::documents(r.printing.get())")),
            ("editor.rs", include_str!("editor.rs"), "Editor", None),
            ("terminal.rs", include_str!("terminal.rs"), "Terminal", None),
            ("monitor.rs", include_str!("monitor.rs"), "Monitor", None),
        ];
        for (name, src, id, guard) in apps {
            assert_eq!(src.matches("release_resources_when_hidden(").count(), 1, "{name}: its window must let go of its surface once");
            assert!(src.contains(&format!("set_widget_name(\"os.kalango.{id}\")")), "{name}: its window is named by its app id");
            // (the call itself, up to its end: the guard must be in it, not anywhere in the file)
            let at = src.find("release_resources_when_hidden(").unwrap_or_default();
            let call = &src[at..at + src[at..].find(");").map_or(0, |e| e + 2)];
            match guard {
                Some(g) => assert!(call.contains(g), "{name}: its guard «{g}», in «{call}»"),
                None => assert!(call.starts_with("release_resources_when_hidden(&") && call.ends_with(", || true);"), "{name}: no guard, in «{call}»"),
            }
            // (the app's own reference dropped, to be built again: the old way, which lost what was in the window)
            assert!(!src.contains("x.borrow_mut().take()));"), "{name}: its singleton must not be dropped");
        }
        let sup = include_str!("superficies.rs");
        assert!(sup.contains("gtk::prelude::WidgetExt::unrealize(&win);") && !sup.contains("win.destroy();"), "the windows are unrealized, not destroyed");
        assert!(!include_str!("main.rs").contains("release_resources_when_hidden("), "main.rs hooks no window of its own");
    }
}
