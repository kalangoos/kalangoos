//! The blocks of an agent conversation that more than one screen shows: the agent's header over an answer, the
//! person's message and a step (a tool call). The main chat (conversa.rs) and the agent editor's "Testar agora"
//! (agent_editor.rs) build their messages with these, so both look the same in either theme (classes in style.css:
//! agente-*, mensagem-pessoa*, passo*). The answer's text itself is markdown.rs.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use serde_json::Value;

/// The accent of the agent's spark (the token's dark value; the light theme swaps it by value).
const ACCENT: &str = "#A5A1E2";

/// The agent's header above an answer: spark tile, the agent's name, its model chip(s) and when.
pub(crate) fn agent_header(name: &str, models: &[String], when: &str) -> gtk::Box {
    let h = caixa(Orientation::Horizontal, 8, "agente-cabecalho");
    let tile = caixa(Orientation::Horizontal, 0, "agente-tile");
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let sp = estrela(12, ACCENT);
    sp.set_margin_start(5);
    sp.set_margin_end(5);
    tile.append(&sp);
    h.append(&tile);
    h.append(&rotulo(name, "agente-nome"));
    for m in models {
        h.append(&rotulo(m, "chip-modelo"));
    }
    h.append(&rotulo(when, "agente-hora"));
    h
}

/// The person's message: a bubble as wide as the column, its text selectable.
pub(crate) fn person_message(text: &str) -> gtk::Box {
    let msg = caixa(Orientation::Horizontal, 0, "mensagem-pessoa");
    msg.set_margin_start(20);
    let l = rotulo(text, "mensagem-pessoa-texto");
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_selectable(true);
    msg.append(&l);
    msg
}

/// One step of a turn (a tool call): the rail, its icon, what it is, a detail in mono and its time. `title_class` is
/// one of passo-titulo-ativo / -feito / -espera / -falhou. Returns the row and the parts that change later.
pub(crate) fn step_row(glyph: &str, color: &str, title: &str, title_class: &str, detail: &str, time: &str) -> (gtk::Box, gtk::Image, gtk::Label, gtk::Label, gtk::Label) {
    let row = caixa(Orientation::Horizontal, 10, "passo");
    row.append(&caixa(Orientation::Vertical, 0, "passo-trilho"));
    let icon = ui_icon(glyph, color, 16);
    icon.set_valign(Align::Center);
    row.append(&icon);
    let title = rotulo(title, title_class);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_xalign(0.0);
    row.append(&title);
    let detail = rotulo(detail, "passo-detalhe");
    detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    detail.set_hexpand(true);
    detail.set_xalign(0.0);
    row.append(&detail);
    let time = rotulo(time, "passo-tempo");
    row.append(&time);
    (row, icon, title, detail, time)
}

/// A path with the home folder as "~".
pub(crate) fn short_path(p: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if h.len() > 1 => {
            let h = h.trim_end_matches('/');
            let s = p.replace(&format!("{h}/"), "~/");
            if s.ends_with(h) { format!("{}~", &s[..s.len() - h.len()]) } else { s }
        }
        _ => p.to_string(),
    }
}

/// The argument that says what the step works on (path, packages, program…), short.
pub(crate) fn main_argument(args: &Value) -> String {
    for k in ["caminho", "de", "programa", "nome", "servico", "chave", "dispositivo", "conexao", "id"] {
        if let Some(v) = args.get(k) {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            return short_path(&s);
        }
    }
    if let Some(a) = args.get("nomes").and_then(Value::as_array) {
        return a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
    }
    String::new()
}

pub(crate) fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
