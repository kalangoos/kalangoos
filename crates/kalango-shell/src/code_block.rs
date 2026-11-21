//! A code block in an answer (design "Chat · blocos de mensagem" › Código): a header with its language and file,
//! "Copiar" and — in the agent's conversation, when the block names its file — "Aplicar no arquivo", which asks the
//! agent to apply it (the change then shows as a diff to accept before it is written); under it the lines, numbered,
//! with the theme's syntax colours (keywords accent, strings teal, numbers gold, functions blue, types coral).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use crate::syntax::Kind;
use kalango_i18n::{tr, trf};
use std::time::Duration;

/// A fence's info line: "rust", "rust src/ui/mod.rs", "rust:src/ui/mod.rs", "src/ui/mod.rs" → (language, file).
pub fn parse_info(info: &str) -> (String, Option<String>) {
    let info = info.trim();
    let (a, b) = match info.split_once(|c: char| c == ':' || c.is_whitespace()) {
        Some((a, b)) => (a.trim(), Some(b.trim().trim_matches(|c| c == '"' || c == '\'')).filter(|b| !b.is_empty())),
        None => (info, None),
    };
    // (a lone path: its extension says the language)
    if b.is_none() && (a.contains('/') || a.contains('.')) {
        let ext = a.rsplit('.').next().unwrap_or("");
        return (ext.to_lowercase(), Some(a.to_string()));
    }
    let b = b.map(|b| b.strip_prefix("title=").unwrap_or(b).trim_matches('"').to_string());
    (a.to_lowercase(), b)
}

/// The language as its header names it.
pub fn language_name(lang: &str) -> String {
    match lang {
        "rs" | "rust" => "Rust",
        "py" | "python" => "Python",
        "js" | "javascript" | "mjs" => "JavaScript",
        "ts" | "typescript" => "TypeScript",
        "sh" | "bash" | "shell" | "zsh" | "console" => "Shell",
        "toml" => "TOML",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "c" | "h" => "C",
        "cpp" | "c++" | "cc" | "hpp" => "C++",
        "go" => "Go",
        "html" => "HTML",
        "css" => "CSS",
        "md" | "markdown" => "Markdown",
        "latex" | "tex" => "LaTeX",
        "mermaid" => "Mermaid",
        "sql" => "SQL",
        "diff" | "patch" => "Diff",
        "" | "text" | "txt" | "plain" => return tr("Code").to_string(),
        other => return other.to_string(),
    }
    .to_string()
}

/// The colour of a kind of code (design "Chat · diagramas, documentos e realce de sintaxe"), in the theme shown now.
/// (Keywords take the accent for text: in the light theme it is a darker green than the accent's, and the two share
/// their dark value, so the light one is read by name.)
fn colour(k: Kind) -> String {
    let name = match k {
        Kind::Plain => "k_text2",
        Kind::Keyword => "k_accent_text",
        Kind::Text => "k_teal",
        Kind::Number | Kind::Attribute => "k_gold",
        Kind::Function => "k_blue",
        Kind::Type | Kind::Tag => "k_coral",
        Kind::Comment => "k_dim",
        Kind::Punct => "k_muted",
    };
    if crate::tema::e_claro() {
        crate::tema::token_claro(name).unwrap_or_else(|| crate::tema::token(name))
    } else {
        crate::tema::token(name)
    }
}

/// The block's lines as Pango markup, in the theme's colours now (crate::syntax; a language it does not know: plain).
pub fn markup(lang: &str, code: &str) -> String {
    crate::syntax::highlight(crate::syntax::lang(lang), code)
        .into_iter()
        .map(|pieces| pieces.into_iter().map(|(k, s)| format!("<span foreground=\"{}\">{}</span>", colour(k), glib::markup_escape_text(&s))).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The folder of the conversation a widget is in (its root's "conversa_pasta"), or the documents' own when it is in
/// none (a preview).
fn conversation_folder(w: &gtk::Widget) -> std::path::PathBuf {
    let mut x = Some(w.clone());
    while let Some(c) = x {
        if let Some(f) = unsafe { c.data::<crate::latex_pdf::FolderFn>("conversa_pasta") } {
            return unsafe { f.as_ref() }();
        }
        x = c.parent();
    }
    glib::user_data_dir().join("kalango").join("latex")
}

/// The text for why LaTeX failed, for the person.
pub(crate) fn failure_text(f: &crate::latex_pdf::Failure) -> String {
    use crate::latex_pdf::Failure;
    match f {
        Failure::Offline => tr("No connection: the first time, LaTeX downloads the packages the document uses. Connect and try again.").to_string(),
        Failure::TimedOut => tr("LaTeX took too long and was stopped.").to_string(),
        Failure::Sandbox(_) => tr("This system cannot wall LaTeX in (Landlock), so the document is not compiled.").to_string(),
        Failure::TooBig => tr("The document wrote too much and was stopped.").to_string(),
        Failure::CacheFull => tr("The LaTeX package cache is full; the document needs packages that are not there.").to_string(),
        Failure::Missing(out) => match crate::latex_pdf::failure(out) {
            Failure::Document(why) if !why.is_empty() => trf("LaTeX could not make the PDF: {reason}", &[("reason", why)]),
            _ => tr("LaTeX could not make the PDF.").to_string(),
        },
        Failure::Document(why) if why.is_empty() => tr("LaTeX could not make the PDF.").to_string(),
        Failure::Document(why) => trf("LaTeX could not make the PDF: {reason}", &[("reason", why.clone())]),
    }
}

/// What a progress step says.
pub(crate) fn progress_text(p: &crate::latex_pdf::Progress) -> String {
    use crate::latex_pdf::Progress;
    match p {
        Progress::Preparing => tr("Preparing LaTeX (only the first time): downloading its packages…").to_string(),
        Progress::Downloading(name) => trf("Downloading LaTeX packages (only the first time)… {name}", &[("name", name.clone())]),
        Progress::Typesetting => tr("Typesetting the document…").to_string(),
        Progress::Writing => tr("Writing the PDF…").to_string(),
    }
}

/// "Gerar PDF": the document next to the conversation, compiled in the background with its progress (the first
/// run downloads LaTeX packages), then opened in Documentos.
pub(crate) fn generate_pdf(b: &gtk::Button, e: &crate::latex_pdf::Engine, tex: &str, aviso: &gtk::Box, div: &gtk::Box, estado: &gtk::Label) {
    let debug = std::env::var_os("KALANGO_SHELL_DEBUG").is_some();
    let dir = conversation_folder(b.upcast_ref());
    let _ = std::fs::create_dir_all(&dir);
    let file = crate::latex_pdf::free_name(&dir);
    if let Err(err) = std::fs::write(&file, tex) {
        estado.set_text(&trf("The PDF could not be made: {reason}", &[("reason", err.to_string())]));
        aviso.set_visible(true);
        div.set_visible(true);
        return;
    }
    b.set_sensitive(false);
    b.set_label(tr("Generating PDF…"));
    estado.set_text(tr("Starting LaTeX…"));
    aviso.set_visible(true);
    div.set_visible(true);
    if debug {
        eprintln!("debug latex: gerando {} com {}", file.display(), e.path.display());
    }
    let st = estado.clone();
    let (b, estado) = (b.clone(), estado.clone());
    let name = file.with_extension("pdf").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "documento.pdf".into());
    crate::latex_block::request(&format!("pdf:{}", file.display()), e, tex, &dir, &name, move |p| {
        st.set_text(&progress_text(p));
        if debug {
            if let crate::latex_pdf::Progress::Downloading(n) = p {
                eprintln!("debug latex: baixando {n}");
            }
        }
    }, move |r| {
        b.set_sensitive(true);
        b.set_label(tr("Generate PDF"));
        match r {
            Ok(c) => {
                let name = c.pdf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                estado.set_text(&match &c.warning {
                    Some(w) => trf("PDF ready: {file} (LaTeX went past an error: {reason})", &[("file", name), ("reason", w.clone())]),
                    None => trf("PDF ready: {file}", &[("file", name)]),
                });
                if debug {
                    eprintln!("debug latex: pdf em {}", c.pdf.display());
                }
                // (KalangoOS's reader, in this same shell)
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("documentos").arg(&c.pdf).spawn();
                }
            }
            Err(f) => {
                estado.set_text(&failure_text(&f));
                if debug {
                    eprintln!("debug latex: falhou: {f:?}");
                }
            }
        }
    });
}

/// "Copiar" for a block's text ("Copiado" for a moment).
pub fn copy_button(code: &str) -> gtk::Button {
    let copiar = gtk::Button::new();
    copiar.add_css_class("md-bloco-botao");
    let cc = caixa(Orientation::Horizontal, 6, "");
    cc.append(&ui_icon("copiar", "#8E8F99", 16));
    let cl = rotulo(tr("Copy"), "");
    cc.append(&cl);
    copiar.set_child(Some(&cc));
    copiar.set_valign(Align::Center);
    let texto = code.to_string();
    let n_linhas = code.lines().count();
    copiar.connect_clicked(move |b| {
        b.clipboard().set_text(&texto);
        cl.set_text(tr("Copied"));
        let l = cl.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(l) = l.upgrade() {
                l.set_text(tr("Copy"));
            }
        });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug codigo: copiado ({n_linhas} linhas)");
        }
    });
    copiar
}

/// A block's line numbers, from `first` (one line at least), one a line: the gutter, right-aligned by its label.
pub fn gutter(first: usize, lines: usize) -> String {
    let first = first.max(1);
    (first..first + lines.max(1)).map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
}

/// The coloured stretches of an editable code field: (line, first char, char after, kind), in chars (a TextBuffer's
/// offsets), plain text left out.
pub fn spans(lang: &str, text: &str) -> Vec<(i32, i32, i32, Kind)> {
    let mut out = Vec::new();
    for (line, pieces) in crate::syntax::highlight(crate::syntax::lang(lang), text).into_iter().enumerate() {
        let mut at = 0i32;
        for (k, piece) in pieces {
            let n = piece.chars().count() as i32;
            if k != Kind::Plain {
                out.push((line as i32, at, at + n, k));
            }
            at += n;
        }
    }
    out
}

/// The lines of a block: numbered from `first` in an aligned gutter, coloured by syntax.rs, in the code font; long
/// lines scroll sideways, and past `max_height` (when given; 0: as tall as its place) the lines scroll too.
pub fn body(lang: &str, code: &str, first: usize, max_height: Option<i32>) -> gtk::ScrolledWindow {
    let corpo = caixa(Orientation::Horizontal, 14, "md-bloco-corpo");
    let nums = rotulo(&gutter(first, code.lines().count()), "md-bloco-num");
    nums.set_xalign(1.0);
    nums.set_valign(Align::Start);
    corpo.append(&nums);
    let linhas = gtk::Label::new(None);
    linhas.add_css_class("md-bloco-codigo");
    linhas.set_xalign(0.0);
    linhas.set_valign(Align::Start);
    linhas.set_selectable(true);
    linhas.set_markup(&markup(lang, code));
    // (the theme changes: the colours again)
    let (lg, cd) = (lang.to_string(), code.to_string());
    crate::tema::registrar(&linhas, move |w| {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            l.set_markup(&markup(&lg, &cd));
        }
    });
    corpo.append(&linhas);
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, if max_height.is_some() { gtk::PolicyType::Automatic } else { gtk::PolicyType::Never });
    match max_height {
        // (0: as tall as its place, the lines scroll inside)
        Some(0) => rol.set_vexpand(true),
        Some(h) => {
            rol.set_propagate_natural_height(true);
            rol.set_max_content_height(h);
        }
        None => rol.set_propagate_natural_height(true),
    }
    rol.set_child(Some(&corpo));
    rol.add_css_class("md-bloco-rolagem");
    rol
}

/// A code block to look at outside a conversation (the Canvas's code): its language, a detail ("pagina.html · linhas
/// 1–8"), Copiar, and the lines numbered from `first`.
pub fn view(lang: &str, code: &str, detail: &str, first: usize, max_height: Option<i32>) -> gtk::Widget {
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("codigo", "#8E8F99", 16));
    cab.append(&rotulo(&language_name(lang), "md-bloco-titulo"));
    let d = rotulo(detail, "md-bloco-detalhe");
    d.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    d.set_max_width_chars(1);
    d.set_hexpand(true);
    d.set_xalign(0.0);
    cab.append(&d);
    let copiar = copy_button(code);
    cab.append(&copiar);
    crate::central::posicao_debug("canvas codigo copiar", copiar.upcast_ref());
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    bloco.append(&body(lang, code, first, max_height));
    bloco.upcast()
}

/// Colours an editable code field (a TextView: the Canvas's diagram source, a pasted configuration) as the blocks
/// are, again after each change (on idle: typing stays quick) and when the theme changes.
pub fn highlight_editable(tv: &gtk::TextView, lang: &str) {
    tv.add_css_class("codigo-editavel");
    let paint = {
        let lang = lang.to_string();
        move |b: &gtk::TextBuffer| {
            let (start, end) = b.bounds();
            let text = b.text(&start, &end, false).to_string();
            let table = b.tag_table();
            for k in ["Plain", "Keyword", "Text", "Number", "Function", "Type", "Comment", "Tag", "Attribute", "Punct"] {
                if let Some(t) = table.lookup(k) {
                    b.remove_tag(&t, &start, &end);
                }
            }
            for (line, from, to, k) in spans(&lang, &text) {
                let (Some(a), Some(z)) = (b.iter_at_line_offset(line, from), b.iter_at_line_offset(line, to)) else { continue };
                let name = format!("{k:?}");
                let tag = table.lookup(&name).unwrap_or_else(|| {
                    let t = gtk::TextTag::new(Some(&name));
                    table.add(&t);
                    t
                });
                tag.set_foreground(Some(&colour(k)));
                b.apply_tag(&tag, &a, &z);
            }
        }
    };
    let paint = std::rc::Rc::new(paint);
    paint(&tv.buffer());
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    let p2 = paint.clone();
    tv.buffer().connect_changed(move |b| {
        if pending.replace(true) {
            return;
        }
        let (b, p, pending) = (b.clone(), p2.clone(), pending.clone());
        glib::idle_add_local_once(move || {
            pending.set(false);
            p(&b);
        });
    });
    crate::tema::registrar(tv, move |w| {
        if let Some(tv) = w.downcast_ref::<gtk::TextView>() {
            paint(&tv.buffer());
        }
    });
}

/// The block. `apply`: show "Aplicar no arquivo" (the agent's conversation, a block that names its file); the
/// button runs the widget action `conversa.aplicar` with `{"file", "lang", "code"}`.
pub fn new(info: &str, code: &str, apply: bool) -> gtk::Widget {
    with_detail(info, code, apply, None, None, false)
}

/// A code block with a word beside its language ("LaTeX documento") and a note over its lines (a diagram that could
/// not be drawn: "Não consegui desenhar este diagrama. O código está abaixo.").
/// `pdf`: a LaTeX document, with "Gerar PDF" when a TeX engine is installed.
pub fn with_detail(info: &str, code: &str, apply: bool, detail: Option<&str>, note: Option<&str>, pdf: bool) -> gtk::Widget {
    let (lang, file) = parse_info(info);
    let side = file.clone().or_else(|| detail.map(str::to_string));
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("codigo", "#8E8F99", 16));
    let t = rotulo(&language_name(&lang), "md-bloco-titulo");
    cab.append(&t);
    if let Some(f) = &side {
        let d = rotulo(f, "md-bloco-detalhe");
        d.set_ellipsize(if file.is_some() { gtk::pango::EllipsizeMode::Start } else { gtk::pango::EllipsizeMode::End });
        d.set_max_width_chars(1);
        d.set_hexpand(true);
        d.set_xalign(0.0);
        cab.append(&d);
    } else {
        let e = caixa(Orientation::Horizontal, 0, "");
        e.set_hexpand(true);
        cab.append(&e);
    }
    let copiar = copy_button(code);
    cab.append(&copiar);
    crate::central::posicao_debug("codigo copiar", copiar.upcast_ref());
    if let (true, Some(f)) = (apply, &file) {
        let aplicar = gtk::Button::with_label(tr("Apply to the file"));
        aplicar.add_css_class("md-bloco-aplicar");
        aplicar.set_valign(Align::Center);
        let p = serde_json::json!({"file": f, "lang": lang, "code": code}).to_string();
        aplicar.connect_clicked(move |b| {
            if b.activate_action("conversa.aplicar", Some(&p.to_variant())).is_err() {
                b.set_sensitive(false);
            }
        });
        crate::central::posicao_debug("codigo aplicar", aplicar.upcast_ref());
        cab.append(&aplicar);
    }
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    if let Some(e) = pdf.then(crate::latex_pdf::engine).flatten() {
        let gerar = gtk::Button::with_label(tr("Generate PDF"));
        gerar.add_css_class("md-bloco-aplicar");
        gerar.set_valign(Align::Center);
        cab.append(&gerar);
        crate::central::posicao_debug("latex gerar pdf", gerar.upcast_ref());
        // the progress and what went wrong: a line over the code, as a diagram's note
        let aviso = caixa(Orientation::Horizontal, 8, "md-bloco-aviso");
        let ponto = caixa(Orientation::Horizontal, 0, "md-bloco-aviso-ponto");
        ponto.set_valign(Align::Center);
        ponto.set_hexpand(false);
        aviso.append(&ponto);
        let estado = rotulo("", "md-bloco-aviso-texto");
        estado.set_wrap(true);
        estado.set_xalign(0.0);
        estado.set_hexpand(true);
        aviso.append(&estado);
        let div = caixa(Orientation::Horizontal, 0, "md-bloco-div");
        aviso.set_visible(false);
        div.set_visible(false);
        let tex = code.to_string();
        let (av, dv) = (aviso.clone(), div.clone());
        gerar.connect_clicked(move |b| generate_pdf(b, &e, &tex, &av, &dv, &estado));
        // (inserted after the header's divider below)
        unsafe {
            bloco.set_data("aviso_pdf", (aviso, div));
        }
    }
    if let Some(n) = note {
        let aviso = caixa(Orientation::Horizontal, 8, "md-bloco-aviso");
        let ponto = caixa(Orientation::Horizontal, 0, "md-bloco-aviso-ponto");
        ponto.set_valign(Align::Center);
        ponto.set_hexpand(false);
        aviso.append(&ponto);
        let l = rotulo(n, "md-bloco-aviso-texto");
        l.set_wrap(true);
        l.set_xalign(0.0);
        aviso.append(&l);
        bloco.append(&aviso);
        bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug codigo: aviso \"{n}\"");
        }
    }
    if let Some((aviso, div)) = unsafe { bloco.steal_data::<(gtk::Box, gtk::Box)>("aviso_pdf") } {
        bloco.append(&aviso);
        bloco.append(&div);
    }
    let n = code.lines().count().max(1);
    let rol = body(&lang, code, 1, None);
    bloco.append(&rol);
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug codigo: bloco {} ({n} linhas){}", language_name(&lang), file.as_ref().map(|f| format!(" de {f}")).unwrap_or_default());
    }
    bloco.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_line() {
        assert_eq!(parse_info("rust"), ("rust".into(), None));
        assert_eq!(parse_info("rust src/ui/mod.rs"), ("rust".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("rust:src/ui/mod.rs"), ("rust".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("src/ui/mod.rs"), ("rs".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("python title=\"app.py\""), ("python".into(), Some("app.py".into())));
        assert_eq!(parse_info(""), ("".into(), None));
    }

    #[test]
    fn gutters_and_editable_spans() {
        // the gutter starts where the shown lines start (the Canvas's "linhas 12–14")
        assert_eq!(gutter(12, 3), "12\n13\n14");
        assert_eq!(gutter(0, 0), "1");
        // an editable field's colours: per line, in chars (accents count one), plain text left out
        let s = spans("json", "{\"nome\": \"São\",\n \"n\": 2}");
        assert!(s.contains(&(0, 1, 7, Kind::Attribute)), "{s:?}");
        assert!(s.contains(&(0, 9, 14, Kind::Text)), "{s:?}");
        assert!(s.contains(&(1, 1, 4, Kind::Attribute)) && s.contains(&(1, 6, 7, Kind::Number)), "{s:?}");
        assert!(spans("mermaid", "flowchart TD\nA --> B").iter().any(|x| x.3 == Kind::Keyword && x.0 == 0));
        assert!(spans("brainfuck", "+.").is_empty());
    }

    #[test]
    fn markup_is_escaped_and_coloured() {
        let m = markup("html", "<a href=\"x\">&</a>");
        assert!(m.contains("&lt;") && m.contains("&amp;") && !m.contains("<a "));
        // a language it does not know: one plain span a line
        assert_eq!(markup("brainfuck", "+.\n-.").matches("<span").count(), 2);
    }

}
