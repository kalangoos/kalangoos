//! The agent's answers (markdown from the model) as compact chat blocks (design "21 · Chat do agente", compact
//! version for the side bar): paragraphs, headings, lists, code and tables. Inline: **bold**, *italic*, `code`.
//! Text is always escaped before markup, so an answer can never inject Pango markup.

use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::tr;

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// Pairs of `delim` become `open…close`; an unpaired one stays as text.
fn pairs(s: &str, delim: &str, open: &str, close: &str) -> String {
    let parts: Vec<&str> = s.split(delim).collect();
    if parts.len() < 3 {
        return s.to_string();
    }
    let mut out = String::new();
    let closed = (parts.len() - 1) / 2 * 2;
    for (i, p) in parts.iter().enumerate() {
        out.push_str(p);
        if i < parts.len() - 1 {
            if i < closed {
                out.push_str(if i % 2 == 0 { open } else { close });
            } else {
                out.push_str(delim);
            }
        }
    }
    out
}

/// Private-use characters around a link's number while the rest of the line is escaped and formatted.
const LINK_MARK: char = '\u{E000}';
const LINK_END: char = '\u{E001}';

/// Whether an address can be opened from a chat (the web and e-mail; never file:, javascript: or other schemes).
fn is_link(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")) && !url.chars().any(char::is_whitespace)
}

/// The text with each link replaced by a marker, and the links (text, address) in order.
fn take_links(s: &str) -> (String, Vec<(String, String)>) {
    let mut out = String::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut push = |out: &mut String, text: &str, url: &str| {
        out.push_str(&format!("{LINK_MARK}{}{LINK_END}", links.len()));
        links.push((text.to_string(), url.to_string()));
    };
    let mut rest = s;
    while !rest.is_empty() {
        // [text](url)
        if rest.starts_with('[') {
            if let Some(b) = rest.find("](") {
                if let Some(c) = rest[b..].find(')') {
                    let (text, url) = (&rest[1..b], &rest[b + 2..b + c]);
                    if !text.contains('[') {
                        if is_link(url) {
                            // a text that looks like another address shows the real one instead
                            let looks_like_address = text.contains("://") || text.starts_with("www.");
                            push(&mut out, if looks_like_address && text != url { url } else { text }, url);
                        } else {
                            // (not something to open: its text alone)
                            out.push_str(text);
                        }
                        rest = &rest[b + c + 1..];
                        continue;
                    }
                }
            }
        }
        // <https://…>
        if rest.starts_with('<') {
            if let Some(c) = rest.find('>') {
                let url = &rest[1..c];
                if is_link(url) {
                    push(&mut out, url.trim_start_matches("mailto:"), url);
                    rest = &rest[c + 1..];
                    continue;
                }
            }
        }
        // a bare address (the punctuation that ends a sentence is not part of it)
        if rest.starts_with("https://") || rest.starts_with("http://") {
            let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '*']);
            if url.len() > "https://".len() {
                push(&mut out, url, url);
                rest = &rest[url.len()..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    (out, links)
}

/// One line of markdown as Pango markup.
pub fn inline(s: &str) -> String {
    let mut out = String::new();
    // code spans first: their content is not formatted
    for (i, part) in s.split('`').enumerate() {
        if i % 2 == 1 {
            // inline code as the chat's design writes it: the text's own font, in the accent's text colour (no block
            // behind it: a run of names stays a sentence, on either theme)
            out.push_str(&format!("<span foreground=\"{}\">{}</span>", crate::tema::destaque_texto(), escape(part)));
        } else {
            // links — [text](url), <https://…> and bare addresses — leave a marker; the text around them is escaped and
            // formatted, then each marker becomes a clickable <a> (so **[x](y)** stays bold)
            let (t, links) = take_links(part);
            let e = escape(&t);
            // bold is **…** (as the models write it): "__" inside names — processos__listar — stays text
            let e = pairs(&e, "**", "<b>", "</b>");
            let e = pairs(&e, "*", "<i>", "</i>");
            let mut e = e;
            for (k, (text, url)) in links.iter().enumerate() {
                // (the real address on hover: the text never hides where it goes)
                e = e.replace(&format!("{LINK_MARK}{k}{LINK_END}"), &format!("<a href=\"{0}\" title=\"{0}\">{1}</a>", escape(url), escape(text)));
            }
            out.push_str(&e);
        }
    }
    if s.matches('`').count() % 2 == 1 {
        // an odd backtick: show the text plainly
        return escape(s);
    }
    out
}

/// A paragraph, heading or list item held to the prose measure: `widths::PROSE_CHARS` characters of its own font (it
/// follows the font size and scale), at the start of the column — the column grows with the window, and code, tables
/// and diagrams take all of it (design "Central · conversa responsiva"). (A label's own `max-width-chars` only bounds
/// its natural width: GTK still gives a wrapped label the whole row once it is laid out for a height.)
fn prose(l: &gtk::Label) -> crate::limite::Limite {
    crate::limite::Limite::measure_in_chars(l, crate::widths::PROSE_CHARS)
}

fn label(markup: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(markup);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_halign(Align::Fill);
    l.set_selectable(true);
    l.set_can_focus(false);
    l.add_css_class(class);
    if markup.contains("<a href=") {
        open_links(&l);
    }
    l
}

/// A label's links open as every address in the shell does: in the default browser (or mail app). Only what
/// `is_link` accepts is ever opened.
pub fn open_links(l: &gtk::Label) {
    l.set_has_tooltip(true);
    l.connect_activate_link(|_, uri| {
        if is_link(uri) {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug link: {uri}");
            }
            let ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
            let _ = gtk::gio::AppInfo::launch_default_for_uri(uri, ctx.as_ref());
        }
        glib::Propagation::Stop
    });
}

/// A table: one block with its table and chart views (table_block.rs).
fn table(rows: &[Vec<String>]) -> gtk::Widget {
    crate::table_block::new(rows)
}

fn is_table_line(l: &str) -> bool {
    let t = l.trim();
    t.starts_with('|') && t.len() > 1
}

fn cells(l: &str) -> Vec<String> {
    let t = l.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_separator(l: &str) -> bool {
    cells(l).iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

/// List marker: ("•" or "3."), rest of the line.
fn list_item(l: &str) -> Option<(String, &str)> {
    let t = l.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            return Some(("•".into(), r));
        }
    }
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let r = &t[digits.len()..];
        if let Some(r) = r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")) {
            return Some((format!("{digits}."), r));
        }
    }
    None
}

/// The answer as a column of blocks.
pub fn render(md: &str) -> gtk::Box {
    render_with(md, false)
}

/// The agent's answer in its conversation: a code block that names its file can be applied ("Aplicar no arquivo").
pub fn render_agent(md: &str) -> gtk::Box {
    // (a model asked for a formula sometimes answers with the bare expression — no "$$": it is still the formula)
    if let Some(src) = bare_latex(md) {
        if let Some(w) = crate::math_block::new(&src) {
            let out = gtk::Box::new(Orientation::Vertical, 8);
            out.add_css_class("md");
            out.append(&w);
            return out;
        }
    }
    render_with(md, true)
}

/// An answer that is only a LaTeX expression (a command such as \frac or \sqrt, no words of a sentence), without the
/// "{{ }}" or "\[ \]" some models wrap it in.
fn bare_latex(md: &str) -> Option<String> {
    let t = md.trim();
    if t.is_empty() || t.contains("$$") || t.contains("```") || t.lines().filter(|l| !l.trim().is_empty()).count() > 3 {
        return None;
    }
    const COMANDOS: [&str; 12] = ["\\frac", "\\sqrt", "\\pm", "\\sum", "\\int", "\\cdot", "\\times", "\\alpha", "\\beta", "\\pi", "\\Delta", "\\lim"];
    if !COMANDOS.iter().any(|c| t.contains(c)) {
        return None;
    }
    // words of a sentence: 4 letters or more, not a command's name
    let palavras = t.split(|c: char| !c.is_alphabetic() && c != '\\').filter(|w| !w.starts_with('\\') && w.chars().count() >= 4).count();
    if palavras > 0 {
        return None;
    }
    let mut src = t.to_string();
    for (a, b) in [("{{", "}}"), ("\\[", "\\]"), ("\\(", "\\)")] {
        if src.starts_with(a) && src.ends_with(b) {
            src = src[a.len()..src.len() - b.len()].trim().to_string();
        }
    }
    Some(src)
}

/// LaTeX's own display delimiters ("\\[ … \\]", which models write as often as "$$") as "$$", outside code blocks.
fn display_math_delims(md: &str) -> String {
    let mut out = Vec::new();
    let mut codigo = false;
    for l in md.lines() {
        let t = l.trim();
        if t.starts_with("```") {
            codigo = !codigo;
            out.push(l.to_string());
            continue;
        }
        if codigo {
            out.push(l.to_string());
            continue;
        }
        let abre = t.starts_with("\\[");
        let fecha = t.ends_with("\\]");
        out.push(match (abre, fecha) {
            (true, true) if t.len() > 4 => format!("$${}$$", &t[2..t.len() - 2]),
            (true, _) if t.len() == 2 => "$$".to_string(),
            (_, true) if t.len() == 2 => "$$".to_string(),
            (true, false) => format!("$${}", &t[2..]),
            (false, true) => format!("{}$$", &t[..t.len() - 2]),
            _ => l.to_string(),
        });
    }
    out.join("\n")
}

fn render_with(md: &str, apply: bool) -> gtk::Box {
    let out = gtk::Box::new(Orientation::Vertical, 8);
    out.add_css_class("md");
    // the fenced blocks first (crate::fences: nested fences, "~~~", an answer still arriving), the text between them
    for piece in crate::fences::split(md) {
        match piece {
            crate::fences::Piece::Text(lines) => render_text(&out, &display_math_delims(&lines.join("\n")), apply),
            crate::fences::Piece::Code(f) => out.append(&fenced_block(&f, apply)),
        }
    }
    out
}

/// A fenced block: a mermaid diagram, a formula, a plan or code. A block still open (the answer is arriving) is code
/// until its closing fence comes.
fn fenced_block(f: &crate::fences::Fence, apply: bool) -> gtk::Widget {
    let (lang, _) = crate::code_block::parse_info(&f.info);
    if !f.closed {
        return crate::code_block::new(&f.info, &f.body, apply);
    }
    let src = &f.body;
    let drawn = match lang.as_str() {
        // a mermaid diagram is drawn (the source one click away); one it cannot draw is its code, with a note
        "mermaid" => crate::diagram_block::new(src, apply)
            .or_else(|| Some(crate::code_block::with_detail(&f.info, src, apply, None, Some(tr("I couldn't draw this diagram. The code is below.")), false))),
        // (a whole document, or text with \item and \textbf, is source: typeset as equations it lost its spaces)
        "latex" | "tex" if crate::math_block::is_document(src) => Some(crate::latex_block::new(&f.info, src, apply)),
        "latex" | "tex" | "math" | "katex" => crate::math_block::new(src),
        // a markdown block that only wraps another block (the model showing "its" diagram) is that block
        "markdown" | "md" => match crate::fences::only_block(src) {
            Some(inner) => Some(fenced_block(&inner, apply)),
            // (a model's plan fenced as markdown — a task list and nothing else — is the plan, not its source)
            None if apply && is_task_list(src) => Some(render_with(src, apply).upcast()),
            None => None,
        },
        _ => None,
    };
    drawn.unwrap_or_else(|| crate::code_block::new(&f.info, src, apply))
}

/// Markdown without fenced blocks: paragraphs, headings, lists, tables, display maths.
fn render_text(out: &gtk::Box, md: &str, apply: bool) {
    let lines: Vec<&str> = md.lines().collect();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |p: &mut Vec<&str>, out: &gtk::Box| {
        if !p.is_empty() {
            let text = p.iter().map(|l| inline(l.trim())).collect::<Vec<_>>().join("\n");
            out.append(&prose(&label(&text, "md-p")));
            p.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim();
        if t.starts_with("$$") {
            // display math: "$$…$$" on one line, or lines between two "$$"
            flush(&mut paragraph, out);
            let mut src = vec![t.trim_start_matches("$$").to_string()];
            let mut fechou = t.len() > 2 && t.ends_with("$$");
            while !fechou && i + 1 < lines.len() {
                i += 1;
                let x = lines[i].trim();
                fechou = x.ends_with("$$");
                src.push(x.to_string());
            }
            let mut src = src.join("\n").trim_end_matches("$$").to_string();
            // (the display equations that follow, blank lines between, go in the same block: numbered together)
            loop {
                let mut k = i + 1;
                while k < lines.len() && lines[k].trim().is_empty() {
                    k += 1;
                }
                if k >= lines.len() || !lines[k].trim().starts_with("$$") {
                    break;
                }
                i = k;
                let x = lines[i].trim();
                let mut eq = vec![x.trim_start_matches("$$").to_string()];
                let mut fim = x.len() > 2 && x.ends_with("$$");
                while !fim && i + 1 < lines.len() {
                    i += 1;
                    let y = lines[i].trim();
                    fim = y.ends_with("$$");
                    eq.push(y.to_string());
                }
                src.push('\n');
                src.push_str(eq.join(" ").trim_end_matches("$$"));
            }
            if let Some(w) = crate::math_block::new(&src) {
                out.append(&w);
            }
        } else if is_table_line(l) {
            flush(&mut paragraph, out);
            let mut rows = Vec::new();
            while i < lines.len() && is_table_line(lines[i]) {
                if !is_separator(lines[i]) {
                    rows.push(cells(lines[i]));
                }
                i += 1;
            }
            out.append(&table(&rows));
            continue;
        } else if let Some(h) = t.strip_prefix("#") {
            flush(&mut paragraph, out);
            // (its level as a class too: a document's title and its sections look different)
            let nivel = 1 + h.chars().take_while(|c| *c == '#').count();
            let l = label(&inline(h.trim_start_matches('#').trim()), "md-h");
            l.add_css_class(&format!("md-h{}", nivel.min(3)));
            out.append(&prose(&l));
        } else if apply && task_item(l).is_some() && lines.get(i + 1).is_some_and(|n| task_item(n).is_some()) {
            // the agent's plan: its steps in one block, how many are done (design "Chat · código (plano, diff e terminal)")
            flush(&mut paragraph, out);
            let mut passos = Vec::new();
            while i < lines.len() {
                let Some((feito, texto)) = task_item(lines[i]) else { break };
                passos.push((feito, texto.to_string()));
                i += 1;
            }
            out.append(&plan(&passos, false));
            continue;
        } else if let Some((feito, texto)) = task_item(l) {
            // a task: its box, ticked or not (a ticked one's text is quieter)
            flush(&mut paragraph, out);
            let row = gtk::Box::new(Orientation::Horizontal, 8);
            let caixa = gtk::Box::new(Orientation::Horizontal, 0);
            caixa.add_css_class("md-tarefa-caixa");
            caixa.set_size_request(16, 16);
            caixa.set_valign(Align::Center);
            caixa.set_halign(Align::Start);
            caixa.set_hexpand(false);
            if feito {
                caixa.add_css_class("feita");
                let v = gtk::Label::new(Some("✓"));
                v.add_css_class("md-tarefa-visto");
                v.set_halign(Align::Center);
                v.set_valign(Align::Center);
                caixa.set_homogeneous(true);
                caixa.append(&v);
            }
            row.append(&caixa);
            let body = label(&inline(texto), "md-p");
            if feito {
                body.add_css_class("md-tarefa-feita");
            }
            body.set_hexpand(true);
            row.append(&prose(&body));
            out.append(&row);
        } else if t == "---" || t == "***" {
            flush(&mut paragraph, out);
            out.append(&gtk::Separator::new(Orientation::Horizontal));
        } else if let Some((mark, rest)) = list_item(l) {
            flush(&mut paragraph, out);
            let row = gtk::Box::new(Orientation::Horizontal, 8);
            let indent = (l.len() - l.trim_start().len()) as i32;
            row.set_margin_start(4 + indent * 6);
            let m = gtk::Label::new(Some(&mark));
            m.add_css_class("md-marca");
            m.set_valign(Align::Start);
            row.append(&m);
            let body = label(&inline(rest), "md-p");
            body.set_hexpand(true);
            row.append(&prose(&body));
            out.append(&row);
        } else if t.is_empty() {
            flush(&mut paragraph, out);
        } else {
            paragraph.push(l);
        }
        i += 1;
    }
    flush(&mut paragraph, out);
}

/// The agent's plan: "Plano · 3 de 5 passos" with its bar; the steps ticked, the next one marked, the rest quieter.
/// The agent's own plan as it works (design "Chat · código"): the steps done ticked, the one in progress with a
/// spinner while `running`.
pub fn plan_live(steps: &[String], done: usize, running: bool) -> gtk::Widget {
    let passos: Vec<(bool, String)> = steps.iter().enumerate().map(|(i, s)| (i < done, s.clone())).collect();
    plan(&passos, running)
}

fn plan(passos: &[(bool, String)], running: bool) -> gtk::Widget {
    use kalango_i18n::{tr, trf};
    let feitos = passos.iter().filter(|p| p.0).count();
    let bloco = gtk::Box::new(Orientation::Vertical, 0);
    bloco.add_css_class("md-bloco");
    bloco.add_css_class("plano");
    let cab = gtk::Box::new(Orientation::Horizontal, 8);
    cab.add_css_class("md-bloco-cab");
    cab.append(&crate::conversa::ui_icon("lista", "#8E8F99", 16));
    let t = gtk::Label::new(Some(tr("Plan")));
    t.add_css_class("md-bloco-titulo");
    cab.append(&t);
    let d = gtk::Label::new(Some(&trf("{done} of {n} steps", &[("done", feitos.to_string()), ("n", passos.len().to_string())])));
    d.add_css_class("md-bloco-detalhe");
    d.set_hexpand(true);
    d.set_xalign(0.0);
    cab.append(&d);
    let barra = gtk::LevelBar::for_interval(0.0, 1.0);
    barra.add_css_class("plano-barra");
    barra.set_value(feitos as f64 / passos.len().max(1) as f64);
    barra.set_size_request(120, -1);
    barra.set_valign(Align::Center);
    cab.append(&barra);
    bloco.append(&cab);
    bloco.append(&{
        let div = gtk::Box::new(Orientation::Horizontal, 0);
        div.add_css_class("md-bloco-div");
        div
    });
    let corpo = gtk::Box::new(Orientation::Vertical, 6);
    corpo.add_css_class("plano-passos");
    let proximo = passos.iter().position(|p| !p.0);
    for (k, (feito, texto)) in passos.iter().enumerate() {
        let row = gtk::Box::new(Orientation::Horizontal, 10);
        let caixa = gtk::Box::new(Orientation::Horizontal, 0);
        caixa.add_css_class("md-tarefa-caixa");
        caixa.set_size_request(16, 16);
        caixa.set_valign(Align::Center);
        caixa.set_hexpand(false);
        if *feito {
            caixa.add_css_class("feita");
            let v = gtk::Label::new(Some("✓"));
            v.add_css_class("md-tarefa-visto");
            caixa.set_homogeneous(true);
            caixa.append(&v);
        }
        if running && !*feito && Some(k) == proximo {
            // (the step in progress: turning)
            let sp = gtk::Spinner::new();
            sp.start();
            sp.set_size_request(16, 16);
            sp.set_valign(Align::Center);
            row.append(&sp);
        } else {
            row.append(&caixa);
        }
        let body = label(&inline(texto), "md-p");
        body.set_hexpand(true);
        if *feito {
            body.add_css_class("plano-feito");
        }
        if !*feito && Some(k) == proximo {
            body.add_css_class("plano-atual");
        } else if !*feito {
            body.add_css_class("plano-depois");
        }
        row.append(&body);
        corpo.append(&row);
    }
    bloco.append(&corpo);
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug plano: {feitos} de {} passos", passos.len());
    }
    bloco.upcast()
}

/// Two tasks or more and nothing else (blank lines aside).
fn is_task_list(src: &str) -> bool {
    let linhas: Vec<&str> = src.lines().filter(|l| !l.trim().is_empty()).collect();
    linhas.len() >= 2 && linhas.iter().all(|l| task_item(l).is_some())
}

/// "- [x] text" / "- [ ] text": (done, text).
pub fn task_item(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
        return Some((true, r));
    }
    rest.strip_prefix("[ ] ").map(|r| (false, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_markup_is_escaped_and_formatted() {
        assert_eq!(inline("a **b** c"), "a <b>b</b> c");
        assert_eq!(inline("<script> & *x*"), "&lt;script&gt; &amp; <i>x</i>");
        assert!(inline("rode `rm -rf <x>`").contains("rm -rf &lt;x&gt;"));
        assert_eq!(inline("veja [o site](http://x.org)"), "veja <a href=\"http://x.org\" title=\"http://x.org\">o site</a>");
        assert_eq!(inline("fonte: <https://a.b/c?d=1&e=2>."), "fonte: <a href=\"https://a.b/c?d=1&amp;e=2\" title=\"https://a.b/c?d=1&amp;e=2\">https://a.b/c?d=1&amp;e=2</a>.");
        assert_eq!(inline("em https://x.org/a, depois"), "em <a href=\"https://x.org/a\" title=\"https://x.org/a\">https://x.org/a</a>, depois");
        assert_eq!(inline("**[forte](https://x.org)**"), "<b><a href=\"https://x.org\" title=\"https://x.org\">forte</a></b>");
        // only the web and e-mail open: other schemes are their text
        assert_eq!(inline("[aqui](javascript:alert(1))"), "aqui)");
        assert_eq!(inline("[a](file:///etc/passwd)"), "a");
        // a text that looks like an address cannot hide another one
        assert!(inline("[https://banco.com.br](https://outro.example)").contains(">https://outro.example</a>"));
        // a quote in the address cannot break out of the attribute
        assert!(!inline("<https://x.org/\"onclick>").contains("\"onclick"));
        assert_eq!(inline("2 * 3 = 6"), "2 * 3 = 6");
        assert_eq!(inline("um ` solto"), "um ` solto");
        // underscores inside names are not bold
        assert_eq!(inline("processos__listar e usuarios__listar"), "processos__listar e usuarios__listar");
        // inline code is coloured text, no block behind it
        assert!(!inline("`x`").contains("background"));
        // markup injection attempt stays text
        assert_eq!(inline("<span foreground='red'>x</span>"), "&lt;span foreground=&apos;red&apos;&gt;x&lt;/span&gt;");
    }

    #[test]
    fn tasks_are_recognized() {
        assert_eq!(super::task_item("- [x] tema e ícones"), Some((true, "tema e ícones")));
        assert_eq!(super::task_item("- [ ] ISO instalável"), Some((false, "ISO instalável")));
        assert_eq!(super::task_item("- lista comum"), None);
        assert!(super::is_task_list("- [x] um\n\n- [ ] dois"));
        assert!(!super::is_task_list("- [x] só um"));
        assert!(!super::is_task_list("# Título\n- [x] um\n- [ ] dois"));
        assert_eq!(super::bare_latex("{{ \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a} }}").as_deref(), Some("\\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}"));
        assert_eq!(super::bare_latex("x = \\frac{1}{2}").as_deref(), Some("x = \\frac{1}{2}"));
        assert_eq!(super::bare_latex("A fórmula é \\frac{1}{2} para metade"), None);
        assert_eq!(super::bare_latex("$$\\frac{1}{2}$$"), None);
        // LaTeX's display delimiters become "$$" (not inside code)
        assert_eq!(super::display_math_delims("A fórmula:\n\n\\[\nx = \\frac{1}{2}\n\\]"), "A fórmula:\n\n$$\nx = \\frac{1}{2}\n$$");
        assert_eq!(super::display_math_delims("\\[ a^2 \\]"), "$$ a^2 $$");
        assert_eq!(super::display_math_delims("```\n\\[\n```"), "```\n\\[\n```");
    }

    #[test]
    fn tables_and_lists_are_recognized() {
        assert!(is_separator("|---|:--:|"));
        assert_eq!(cells("| a | **b** |"), vec!["a", "**b**"]);
        assert_eq!(list_item("- x").unwrap().0, "•");
        assert_eq!(list_item("12. y").unwrap(), ("12.".to_string(), "y"));
        assert!(list_item("-x").is_none());
    }
}
