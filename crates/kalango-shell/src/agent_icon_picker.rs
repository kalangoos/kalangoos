//! The agent's icon picker (design 44, light 50): one of the shell's icons on a tile of a token colour, or an SVG the
//! person imports. kalango-fluxos cleans an imported SVG (`icons.clean`) and the agent keeps only what comes back,
//! so exporting the agent carries the drawing and nothing points at the person's file. Without a choice the icon
//! comes from the agent's name and kind (agent_picker::default_icon).

use crate::agent_picker;
use crate::fluxos_ui as ui;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{n_, tr};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// The shell's icons an agent can take: (name kept in the agent, what it shows). The names are kalango-fluxos's
/// (agent_picker::glyph_of turns them into the shell's glyphs).
pub const ICONS: [(&str, &str); 24] = [
    ("search", n_("Magnifier")),
    ("folder", n_("Folder")),
    ("doc", n_("Document")),
    ("list", n_("List")),
    ("image", n_("Picture")),
    ("play", n_("Play")),
    ("code", n_("Code")),
    ("clock", n_("Clock")),
    ("bell", n_("Bell")),
    ("send", n_("Paper plane")),
    ("shield", n_("Shield")),
    ("cpu", n_("Chip")),
    ("bolt", n_("Lightning")),
    ("brain", n_("Brain")),
    ("star", n_("Star")),
    ("download", n_("Download")),
    ("network", n_("Network")),
    ("terminal", n_("Terminal")),
    ("eye", n_("Eye")),
    ("gear", n_("Gear")),
    ("spark", n_("Spark")),
    ("home", n_("House")),
    ("keyboard", n_("Keyboard")),
    ("apps", n_("Grid")),
];

/// The tile colours (kalango-fluxos's `icon::COLORS`), in the design's order.
pub const COLORS: [&str; 7] = ["accent", "teal", "blue", "gold", "coral", "purple", "grey"];

/// Words people type that find an icon besides its name (the same words that give an agent its default icon:
/// "jogo" finds the play icon).
fn icon_words(name: &str) -> &'static [&'static str] {
    match name {
        "search" => &["busca", "pesquis", "procura", "lupa"],
        "folder" => &["arquivo", "pasta", "download", "organiz"],
        "image" => &["foto", "imagem"],
        "send" => &["e-mail", "email", "correio", "caixa", "enviar"],
        "doc" => &["texto", "revis", "resum", "redat", "escrev", "traduz"],
        "code" => &["código", "codigo", "teste", "program", "corrig"],
        "clock" => &["agenda", "plano", "lembrete", "calend", "hora"],
        "download" => &["backup", "cópia", "copia", "baixar"],
        "shield" => &["segur", "senha", "proteg"],
        "cpu" => &["sistema", "disco", "memória", "processador"],
        "play" => &["jogo", "game", "vídeo", "video", "música"],
        "bell" => &["notific", "aviso", "alerta"],
        "network" => &["rede", "servidor", "conex"],
        "home" => &["casa", "início"],
        "brain" => &["ideia", "pensa", "memória"],
        _ => &[],
    }
}

/// Whether an icon answers the search (its translated name, its own name or its words; empty finds all).
pub fn icon_matches(name: &str, label: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    label.to_lowercase().contains(&q) || name.contains(&q) || icon_words(name).iter().any(|w| w.starts_with(&q) || q.starts_with(w))
}

/// Why an SVG was not taken, from the code kalango-fluxos answers.
pub fn svg_error_text(error: &str) -> String {
    let t = if error.contains("too_big") {
        tr("This SVG is over 64 KB.")
    } else if error.contains("not_svg") {
        tr("This file is not an SVG.")
    } else if error.contains("too_complex") {
        tr("This SVG is too detailed for an icon.")
    } else {
        tr("Could not read this file.")
    };
    t.to_string()
}

/// The icon after a choice: an icon of the list keeps the tile's colour; a colour keeps the icon (or the SVG).
pub fn with_icon(current: &Value, name: &str) -> Value {
    json!({"name": name, "color": current["color"].as_str().unwrap_or("accent")})
}

pub fn with_color(current: &Value, color: &str) -> Value {
    let mut v = current.clone();
    if !v.is_object() {
        v = json!({"name": "spark"});
    }
    v["color"] = json!(color);
    v
}

/// The picker: the agent's tile as a button that opens it.
pub struct IconPicker {
    pub button: gtk::MenuButton,
    popover: gtk::Popover,
    icons: Vec<(&'static str, gtk::Button)>,
    swatches: Vec<(&'static str, gtk::Button)>,
    error: gtk::Label,
    /// the agent as the tile shows it (name, description, kind, icon)
    agent: RefCell<Value>,
    on_change: Box<dyn Fn(Value)>,
}

impl IconPicker {
    /// `on_change` gets the agent's new "icon".
    pub fn new(on_change: impl Fn(Value) + 'static) -> Rc<IconPicker> {
        let button = gtk::MenuButton::new();
        button.add_css_class("fx-icone-botao");
        button.set_valign(Align::Start);
        button.set_tooltip_text(Some(tr("Choose the icon")));
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");
        popover.add_css_class("fx-icones");
        let col = caixa(Orientation::Vertical, 0, "fx-icones-corpo");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(tr("Search icon: magnifier, folder, game…")));
        search.add_css_class("fx-busca-campo");
        col.append(&search);
        let s1 = ui::section(tr("SYSTEM ICONS"));
        s1.set_margin_top(12);
        col.append(&s1);
        let grid = gtk::FlowBox::new();
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.set_max_children_per_line(8);
        grid.set_min_children_per_line(8);
        grid.set_homogeneous(true);
        grid.set_column_spacing(4);
        grid.set_row_spacing(4);
        grid.set_margin_top(8);
        let mut icons = Vec::new();
        for (name, label) in ICONS {
            let b = gtk::Button::new();
            b.add_css_class("fx-icone-opcao");
            b.set_tooltip_text(Some(tr(label)));
            b.set_child(Some(&ui::glyph(&agent_picker::glyph_of(name), ui::MUTED, 16)));
            grid.insert(&b, -1);
            icons.push((name, b));
        }
        col.append(&grid);
        let s2 = ui::section(tr("BACKGROUND COLOUR"));
        s2.set_margin_top(14);
        col.append(&s2);
        let row = caixa(Orientation::Horizontal, 8, "");
        row.set_margin_top(8);
        let mut swatches = Vec::new();
        for c in COLORS {
            let b = gtk::Button::new();
            b.add_css_class("fx-icone-cor");
            b.add_css_class(agent_picker::tone_of(c));
            b.set_tooltip_text(Some(c));
            row.append(&b);
            swatches.push((c, b));
        }
        col.append(&row);
        let svg_row = caixa(Orientation::Horizontal, 10, "fx-icones-svg");
        let svg = ui::button(Some("download"), tr("Use an SVG…"), false);
        svg_row.append(&svg);
        let limit = rotulo(tr("up to 64 KB · cleaned on import"), "fx-nota");
        limit.set_hexpand(true);
        limit.set_xalign(1.0);
        svg_row.append(&limit);
        col.append(&svg_row);
        let error = ui::error_label();
        col.append(&error);
        let note = ui::note("info", tr("With no choice, the icon comes from the name and the kind (search → magnifier)."));
        note.set_margin_top(10);
        col.append(&note);
        popover.set_child(Some(&col));
        button.set_popover(Some(&popover));
        let p = Rc::new(IconPicker { button, popover, icons, swatches, error, agent: RefCell::new(json!({})), on_change: Box::new(on_change) });
        for (name, b) in &p.icons {
            let (w, name) = (Rc::downgrade(&p), *name);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    let cur = agent_picker::icon_spec(&p.agent.borrow());
                    p.choose(with_icon(&cur, name));
                }
            });
        }
        for (c, b) in &p.swatches {
            let (w, c) = (Rc::downgrade(&p), *c);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    let cur = agent_picker::icon_spec(&p.agent.borrow());
                    p.choose(with_color(&cur, c));
                }
            });
        }
        {
            let icons: Vec<(&'static str, gtk::Button)> = p.icons.clone();
            search.connect_search_changed(move |s| {
                let q = s.text().to_string();
                for ((name, label), (_, b)) in ICONS.iter().zip(&icons) {
                    if let Some(child) = b.parent() {
                        child.set_visible(icon_matches(name, tr(label), &q));
                    }
                }
            });
        }
        let w = Rc::downgrade(&p);
        svg.connect_clicked(move |b| {
            if let Some(p) = w.upgrade() {
                p.pick_svg(b);
            }
        });
        p
    }

    /// Shows this agent's icon on the button and marks its choice in the picker.
    pub fn set(&self, agent: &Value) {
        *self.agent.borrow_mut() = agent.clone();
        let spec = agent_picker::icon_spec(agent);
        self.button.set_child(Some(&agent_picker::icon_tile(agent, 44)));
        let own = agent["icon"].is_object();
        for (name, b) in &self.icons {
            let on = own && spec["svg"].is_null() && spec["name"].as_str() == Some(name);
            if on { b.add_css_class("ativo") } else { b.remove_css_class("ativo") }
        }
        for (c, b) in &self.swatches {
            if spec["color"].as_str() == Some(c) { b.add_css_class("ativo") } else { b.remove_css_class("ativo") }
        }
    }

    fn choose(&self, icon: Value) {
        ui::show_error(&self.error, "");
        let mut a = self.agent.borrow().clone();
        a["icon"] = icon.clone();
        self.set(&a);
        (self.on_change)(icon);
    }

    /// "Usar um SVG…": the file goes to kalango-fluxos, which cleans it; the agent keeps only the clean drawing.
    fn pick_svg(self: &Rc<Self>, _b: &gtk::Button) {
        self.popover.popdown();
        // (the system's chooser, straight: no round trip through the portal)
        let filters = vec![crate::arquivos::seletor::Filtro::de("SVG", &["*.svg", "image/svg+xml"])];
        let w = Rc::downgrade(self);
        crate::arquivos::seletor::abrir_arquivo(tr("Agents"), tr("Use an SVG…"), None, filters, None, move |path| {
            let Some(p) = w.upgrade() else { return };
            let w = Rc::downgrade(&p);
            crate::fluxos::call("icons.clean", json!({"path": path.to_string_lossy()}), move |r| {
                let Some(p) = w.upgrade() else { return };
                match r {
                    Ok(v) => {
                        let color = agent_picker::icon_spec(&p.agent.borrow())["color"].as_str().unwrap_or("accent").to_string();
                        p.choose(json!({"svg": v["svg"], "monochrome": v["monochrome"].as_bool().unwrap_or(false), "color": color}));
                    }
                    Err(e) => {
                        ui::show_error(&p.error, &svg_error_text(&e));
                        p.button.popup();
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_found_by_what_people_type() {
        assert!(icon_matches("play", tr("Play"), "jogo"));
        assert!(icon_matches("search", tr("Magnifier"), "lupa"));
        assert!(icon_matches("folder", tr("Folder"), "pas"));
        assert!(icon_matches("folder", tr("Folder"), ""));
        assert!(!icon_matches("bell", tr("Bell"), "jogo"));
        // every icon of the list is one the shell can draw
        for (name, _) in ICONS.iter().filter(|(n, _)| *n != "spark") {
            assert_ne!(agent_picker::glyph_of(name), "spark", "{name}");
        }
    }

    #[test]
    fn a_choice_keeps_the_other_half() {
        let cur = json!({"name": "folder", "color": "gold"});
        assert_eq!(with_icon(&cur, "bell"), json!({"name": "bell", "color": "gold"}));
        assert_eq!(with_color(&cur, "teal"), json!({"name": "folder", "color": "teal"}));
        // an imported SVG keeps its drawing when the colour changes
        let svg = json!({"svg": "<svg/>", "monochrome": true, "color": "teal"});
        assert_eq!(with_color(&svg, "coral")["svg"], "<svg/>");
        assert_eq!(svg_error_text("icons.clean: too_big"), tr("This SVG is over 64 KB."));
        assert_eq!(svg_error_text("not_svg"), tr("This file is not an SVG."));
    }
}
