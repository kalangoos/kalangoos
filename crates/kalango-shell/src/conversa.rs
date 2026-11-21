//! The agent conversation in the right side bar, Agent mode (design "22 · Barras laterais", frame "Barra lateral
//! direita · Agente", with the compact chat blocks of "21 · Chat do agente").
//!
//! The engine is `kalango-agente --json`, one process per conversation (the model keeps the context). It talks to
//! the local model (LFM3 via llama.cpp) and acts on the system only through kalangod: levels, the approval screen
//! and the timeline stay where they are. This module only shows what happens and sends what the person writes.
//!
//! Layout: agent header (spark, name, model chip, time) · turns (person's message card, steps with a rail,
//! answer as markdown, stop/error cards) · composer (text, attach, model chip, send/stop).
//! Motion: a new message enters with the quick spring, the answer with the gentle one; the steps title pulses
//! while the model thinks.

use crate::agent_picker::{self, Picked, Picker};
use crate::chat_blocks::{agent_header, answer_footer, first_upper, main_argument, person_message, short_path, StepCall, StepState, StepsList};
use crate::markdown;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------------------------------ icons

/// Line icons exported from the design (page 22, 16×16 glyphs), recoloured per state.
fn svg_source(name: &str) -> &'static str {
    match name {
        "raio" => include_str!("../assets/ui/raio.svg"),
        "seta-baixo" => include_str!("../assets/ui/seta-baixo.svg"),
        "olho" => include_str!("../assets/ui/olho.svg"),
        "pasta" => include_str!("../assets/ui/pasta.svg"),
        "lixeira" => include_str!("../assets/ui/lixeira.svg"),
        "mais" => include_str!("../assets/ui/mais.svg"),
        "enviar" => include_str!("../assets/ui/enviar.svg"),
        "parar" => include_str!("../assets/ui/parar.svg"),
        "baixar" => include_str!("../assets/ui/baixar.svg"),
        "ejetar" => include_str!("../assets/ui/ejetar.svg"),
        "local" => include_str!("../assets/ui/local.svg"),
        "recortar" => include_str!("../assets/ui/recortar.svg"),
        "ponteiro" => include_str!("../assets/ui/ponteiro.svg"),
        "desfazer" => include_str!("../assets/ui/desfazer.svg"),
        "info" => include_str!("../assets/ui/info.svg"),
        "menu" => include_str!("../assets/ui/menu.svg"),
        "minimizar" => include_str!("../assets/ui/minimizar.svg"),
        "maximizar" => include_str!("../assets/ui/maximizar.svg"),
        "mouse" => include_str!("../assets/ui/mouse.svg"),
        "teclado" => include_str!("../assets/ui/teclado.svg"),
        "camera" => include_str!("../assets/ui/camera.svg"),
        "musica" => include_str!("../assets/ui/musica.svg"),
        "sino" => include_str!("../assets/ui/sino.svg"),
        "anterior" => include_str!("../assets/ui/anterior.svg"),
        "pausa" => include_str!("../assets/ui/pausa.svg"),
        "proxima" => include_str!("../assets/ui/proxima.svg"),
        "tocar" => include_str!("../assets/ui/tocar.svg"),
        "escudo" => include_str!("../assets/ui/escudo.svg"),
        "feito" => include_str!("../assets/ui/feito.svg"),
        "globo" => include_str!("../assets/ui/globo.svg"),
        "layout" => include_str!("../assets/ui/layout.svg"),
        "camadas" => include_str!("../assets/ui/camadas.svg"),
        "microfone" => include_str!("../assets/ui/microfone.svg"),
        "ajustes" => include_str!("../assets/ui/ajustes.svg"),
        "wifi" => include_str!("../assets/ui/wifi.svg"),
        "wifi-fraco" => include_str!("../assets/ui/wifi-fraco.svg"),
        "wifi-sem" => include_str!("../assets/ui/wifi-sem.svg"),
        "cabo" => include_str!("../assets/ui/cabo.svg"),
        "aviao" => include_str!("../assets/ui/aviao.svg"),
        "bluetooth" => include_str!("../assets/ui/bluetooth.svg"),
        "bluetooth-off" => include_str!("../assets/ui/bluetooth-off.svg"),
        "volume" => include_str!("../assets/ui/volume.svg"),
        "volume-baixo" => include_str!("../assets/ui/volume-baixo.svg"),
        "volume-mudo" => include_str!("../assets/ui/volume-mudo.svg"),
        "energia" => include_str!("../assets/ui/energia.svg"),
        "check" => include_str!("../assets/ui/check.svg"),
        "download" => include_str!("../assets/ui/download.svg"),
        "paleta" => include_str!("../assets/ui/paleta.svg"),
        "raio-e" => include_str!("../assets/ui/raio-e.svg"),
        "sino-n" => include_str!("../assets/ui/sino-n.svg"),
        "sino-mudo-n" => include_str!("../assets/ui/sino-mudo-n.svg"),
        "apps-n" => include_str!("../assets/ui/apps-n.svg"),
        "estrela-n" => include_str!("../assets/ui/estrela-n.svg"),
        "chevron-n" => include_str!("../assets/ui/chevron-n.svg"),
        "bateria-d" => include_str!("../assets/ui/bateria-d.svg"),
        "lua-d" => include_str!("../assets/ui/lua-d.svg"),
        "cpu-d" => include_str!("../assets/ui/cpu-d.svg"),
        "microfone-d" => include_str!("../assets/ui/microfone-d.svg"),
        "ajustes-d" => include_str!("../assets/ui/ajustes-d.svg"),
        "chevron-baixo" => include_str!("../assets/ui/chevron-baixo.svg"),
        "video-d" => include_str!("../assets/ui/video-d.svg"),
        "anel" => include_str!("../assets/ui/anel.svg"),
        "busca-d" => include_str!("../assets/ui/busca-d.svg"),
        "chevron" => include_str!("../assets/ui/chevron.svg"),
        "celular-d" => include_str!("../assets/ui/celular-d.svg"),
        "rede-cabo" => include_str!("../assets/ui/rede-cabo.svg"),
        "olho-d" => include_str!("../assets/ui/olho-d.svg"),
        "painel-direita" => include_str!("../assets/ui/painel-direita.svg"),
        "expandir" => include_str!("../assets/ui/expandir.svg"),
        "recolher" => include_str!("../assets/ui/recolher.svg"),
        "painel-esquerda" => include_str!("../assets/ui/painel-esquerda.svg"),
        "clipe" => include_str!("../assets/ui/clipe.svg"),
        "apps-pontos" => include_str!("../assets/ui/apps-pontos.svg"),
        "sino-d" => include_str!("../assets/ui/sino-d.svg"),
        "teclado-d" => include_str!("../assets/ui/teclado-d.svg"),
        "cadeado-d" => include_str!("../assets/ui/cadeado-d.svg"),
        "info-d" => include_str!("../assets/ui/info-d.svg"),
        "desligar-d" => include_str!("../assets/ui/desligar-d.svg"),
        "mouse-d" => include_str!("../assets/ui/mouse-d.svg"),
        "noite-d" => include_str!("../assets/ui/noite-d.svg"),
        "configuracoes" => include_str!("../assets/ui/configuracoes.svg"),
        "casa" => include_str!("../assets/ui/casa.svg"),
        "imagem" => include_str!("../assets/ui/imagem.svg"),
        "codigo" => include_str!("../assets/ui/codigo.svg"),
        "grade" => include_str!("../assets/ui/grade.svg"),
        "lista" => include_str!("../assets/ui/lista.svg"),
        "compartilhar" => include_str!("../assets/ui/compartilhar.svg"),
        "alerta" => include_str!("../assets/ui/alerta.svg"),
        "relogio" => include_str!("../assets/ui/relogio.svg"),
        "doc" => include_str!("../assets/ui/doc.svg"),
        "terminal" => include_str!("../assets/ui/terminal.svg"),
        "ramo" => include_str!("../assets/ui/ramo.svg"),
        "cerebro" => include_str!("../assets/ui/cerebro.svg"),
        "spark" => include_str!("../assets/ui/spark.svg"),
        "cadeado" => include_str!("../assets/ui/cadeado.svg"),
        "lua" => include_str!("../assets/ui/lua.svg"),
        "usuario" => include_str!("../assets/ui/usuario.svg"),
        "reiniciar" => include_str!("../assets/ui/reiniciar.svg"),
        "desligar" => include_str!("../assets/ui/desligar.svg"),
        "sair" => include_str!("../assets/ui/sair.svg"),
        "sino-mudo" => include_str!("../assets/ui/sino-mudo.svg"),
        "sol" => include_str!("../assets/ui/sol.svg"),
        "fone" => include_str!("../assets/ui/fone.svg"),
        "celular" => include_str!("../assets/ui/celular.svg"),
        "monitor" => include_str!("../assets/ui/monitor.svg"),
        "seta-direita" => include_str!("../assets/ui/seta-direita.svg"),
        "alfinete" => include_str!("../assets/ui/alfinete.svg"),
        "expandir-altura" => include_str!("../assets/ui/expandir-altura.svg"),
        "quadrado" => include_str!("../assets/ui/quadrado.svg"),
        "fixar" => include_str!("../assets/ui/fixar.svg"),
        "teclado-p" => include_str!("../assets/ui/teclado-p.svg"),
        "engrenagem" => include_str!("../assets/ui/engrenagem.svg"),
        "seta-esquerda" => include_str!("../assets/ui/seta-esquerda.svg"),
        "seta-cima" => include_str!("../assets/ui/seta-cima.svg"),
        "lapis" => include_str!("../assets/ui/lapis.svg"),
        "lua-pequena" => include_str!("../assets/ui/lua-pequena.svg"),
        "cpu" => include_str!("../assets/ui/cpu.svg"),
        "fechar" => include_str!("../assets/ui/fechar.svg"),
        "tesoura" => include_str!("../assets/ui/tesoura.svg"),
        "copiar" => include_str!("../assets/ui/copiar.svg"),
        "colar" => include_str!("../assets/ui/colar.svg"),
        "ordenar" => include_str!("../assets/ui/ordenar.svg"),
        "agrupar" => include_str!("../assets/ui/agrupar.svg"),
        "pontos-v" => include_str!("../assets/ui/pontos-v.svg"),
        "menos" => include_str!("../assets/ui/menos.svg"),
        "apagar-tecla" => include_str!("../assets/ui/apagar-tecla.svg"),
        "enter" => include_str!("../assets/ui/enter.svg"),
        "shift" => include_str!("../assets/ui/shift.svg"),
        "impressora" => include_str!("../assets/ui/impressora.svg"),
        "colunas" => include_str!("../assets/ui/colunas.svg"),
        "galeria" => include_str!("../assets/ui/galeria.svg"),
        "acima" => include_str!("../assets/ui/acima.svg"),
        "disco" => include_str!("../assets/ui/disco.svg"),
        "busca" => include_str!("../assets/ui/busca.svg"),
        _ => include_str!("../assets/ui/erro.svg"),
    }
}

/// Whether the shell has a glyph of this name.
pub(crate) fn has_icon(name: &str) -> bool {
    name == "erro" || svg_source(name) != include_str!("../assets/ui/erro.svg")
}

/// An SVG in one colour (an agent's imported monochrome icon).
/// (its fill and stroke values that are colours — not "none" nor a gradient — become `color`; with no colour at all,
/// the root fills with it)
pub(crate) fn recolor_svg(svg: &str, color: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    let mut any = false;
    while let Some(i) = ["fill=\"", "stroke=\""].iter().filter_map(|a| rest.find(a).map(|i| i + a.len())).min() {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find('"').unwrap_or(rest.len());
        let v = &rest[..end];
        if v == "none" || v.starts_with("url(") {
            out.push_str(v);
        } else {
            out.push_str(color);
            any = true;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    if !any {
        if let Some(i) = out.find("<svg") {
            out.insert_str(i + 4, &format!(" fill=\"{color}\""));
        }
    }
    out
}

/// Every colour of the glyph (strokes and fills, not "none") becomes `color`.
fn recolor(svg: &str, color: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find("=\"#") {
        out.push_str(&rest[..i + 2]);
        out.push_str(color);
        rest = &rest[i + 2 + 7..];
    }
    out.push_str(rest);
    out
}

/// The glyph in `color` as the theme shows it (the colours are the dark sheet's).
fn texture(name: &str, color: &str) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(recolor(svg_source(name), &crate::tema::cor(color)).into_bytes())).ok()
}

/// Draws it again when the theme changes.
fn seguir_tema(img: &gtk::Image, name: &str, color: &str) {
    let (name, color) = (name.to_string(), color.to_string());
    crate::tema::registrar(img, move |w| {
        if let Some(i) = w.downcast_ref::<gtk::Image>() {
            i.set_paintable(texture(&name, &color).as_ref());
        }
    });
}

pub fn ui_icon(name: &str, color: &str, px: i32) -> gtk::Image {
    // a paintable fills its allocation: centred and with a fixed size it stays at the glyph size
    let img = gtk::Image::from_paintable(texture(name, color).as_ref());
    seguir_tema(&img, name, color);
    img.set_pixel_size(px);
    img.set_size_request(px, px);
    img.set_halign(Align::Center);
    img.set_valign(Align::Center);
    img
}

pub(crate) fn set_icon(img: &gtk::Image, name: &str, color: &str) {
    img.set_paintable(texture(name, color).as_ref());
    seguir_tema(img, name, color);
}

/// The numbers of an svg attribute on its root element (`viewBox="0 0 16 16"` → [0, 0, 16, 16]).
fn root_numbers(svg: &str, attr: &str) -> Vec<f64> {
    let raiz = &svg[..svg.find('>').unwrap_or(svg.len())];
    raiz.find(&format!(" {attr}=\"")).and_then(|i| {
        let resto = &raiz[i + attr.len() + 3..];
        resto.find('"').map(|j| resto[..j].split([' ', ',']).filter_map(|n| n.parse().ok()).collect())
    }).unwrap_or_default()
}

thread_local! {
    static BOUNDS: RefCell<std::collections::HashMap<String, [f64; 5]>> = RefCell::default();
}

/// Where a glyph's vector is inside its box — x, y, width, height and the box's side, in the svg's units — as the
/// designs measure it: the drawn shape less the stroke's half (round caps and joins reach that far on every side).
/// Measured once by drawing it large.
pub(crate) fn glyph_bounds(name: &str) -> [f64; 5] {
    if let Some(b) = BOUNDS.with(|c| c.borrow().get(name).copied()) {
        return b;
    }
    let svg = svg_source(name);
    let lado = root_numbers(svg, "viewBox").get(2).copied().unwrap_or(16.0);
    let traco = if svg.contains("stroke=\"#") {
        svg.find("stroke-width=\"").and_then(|i| svg[i + 14..].split('"').next()?.parse::<f64>().ok()).unwrap_or(1.0)
    } else {
        0.0
    };
    const K: f64 = 8.0;
    let px = (lado * K) as i32;
    let fluxo = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(recolor(svg, "#000000").into_bytes()));
    let mut b = [0.0, 0.0, lado, lado, lado];
    if let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&fluxo, px, px, false, None::<&gtk::gio::Cancellable>) {
        if pb.has_alpha() && pb.n_channels() == 4 {
            let (w, h, rs) = (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize);
            let bytes = pb.read_pixel_bytes();
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            for y in 0..h {
                for x in 0..w {
                    // half covered or more: the edge within 1/16 of a unit
                    if bytes[y * rs + x * 4 + 3] >= 128 {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x + 1);
                        y1 = y1.max(y + 1);
                    }
                }
            }
            if x0 < x1 {
                let m = traco / 2.0;
                let (gx, gy) = (x0 as f64 / K + m, y0 as f64 / K + m);
                b = [gx, gy, (x1 as f64 / K - m - gx).max(0.0), (y1 as f64 / K - m - gy).max(0.0), lado];
            }
        }
    }
    BOUNDS.with(|c| c.borrow_mut().insert(name.to_string(), b));
    b
}

/// A glyph cut to its drawn shape (the vector plus half a pixel around it, where the stroke reaches), drawn at the
/// screen's scale: in the designs an auto layout measures the vector itself, so a row of glyph and text puts the
/// text `gap` after the vector — here `gap - 1` after this widget (as after the spark). `px` is the box's size on
/// screen (16 for the 16 px glyphs).
pub(crate) fn ui_icon_fitted(name: &str, color: &str, px: i32) -> gtk::DrawingArea {
    let [gx, gy, gw, gh, lado] = glyph_bounds(name);
    let f = f64::from(px) / lado;
    let (w, h) = ((gw * f + 1.0).round().max(1.0), (gh * f + 1.0).round().max(1.0));
    // the cut, centred on the vector, in the svg's units
    let (vw, vh) = (w / f, h / f);
    let (vx, vy) = (gx + gw / 2.0 - vw / 2.0, gy + gh / 2.0 - vh / 2.0);
    let mut svg = svg_source(name).to_string();
    let fim = svg.find('>').unwrap_or(0);
    let mut raiz = svg[..fim].to_string();
    for (attr, valor) in [("width", format!("{w}")), ("height", format!("{h}")), ("viewBox", format!("{vx} {vy} {vw} {vh}"))] {
        let chave = format!(" {attr}=\"");
        match raiz.find(&chave) {
            Some(i) => {
                let ini = i + chave.len();
                let j = ini + raiz[ini..].find('"').unwrap_or(0);
                raiz.replace_range(ini..j, &valor);
            }
            None => raiz += &format!("{chave}{valor}\""),
        }
    }
    svg.replace_range(..fim, &raiz);
    let da = gtk::DrawingArea::new();
    da.set_content_width(w as i32);
    da.set_content_height(h as i32);
    da.set_halign(Align::Center);
    da.set_valign(Align::Center);
    // drawn at the screen's scale, in the theme's colour (again when either changes)
    let feito: RefCell<Option<(i32, u64, gtk::gdk_pixbuf::Pixbuf)>> = RefCell::default();
    let color = color.to_string();
    da.set_draw_func(move |da, cr, _, _| {
        use gtk::gdk::prelude::GdkCairoContextExt;
        let s = da.scale_factor().max(1);
        let g = crate::tema::geracao();
        if feito.borrow().as_ref().is_none_or(|(e, eg, _)| *e != s || *eg != g) {
            let fonte = recolor(&svg, &crate::tema::cor(&color));
            let fluxo = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(fonte.into_bytes()));
            let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&fluxo, w as i32 * s, h as i32 * s, false, None::<&gtk::gio::Cancellable>) else { return };
            *feito.borrow_mut() = Some((s, g, pb));
        }
        if let Some((s, _, pb)) = feito.borrow().as_ref() {
            cr.scale(1.0 / f64::from(*s), 1.0 / f64::from(*s));
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            let _ = cr.paint();
        }
    });
    crate::tema::registrar(&da, |w| w.queue_draw());
    da
}

const ACCENT: &str = "#A5A1E2";
const GOLD: &str = "#E5C98C";
const CORAL: &str = "#EFA2A6";
const MUTED: &str = "#8E8F99";
const ON_ACCENT: &str = "#1C1D24";

// ------------------------------------------------------------------------------------------------ helpers

/// 2310 → "2.310".
fn thousands(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn decimal(v: f64) -> String {
    let s = format!("{v:.1}");
    if kalango_i18n::system_locale().is_some_and(|l| l.starts_with("pt")) {
        s.replace('.', ",")
    } else {
        s
    }
}

/// The sound when a request ends: it failed (unless a block already said so), or it was answered out of sight. Stopped
/// requests, and answers the person watched, are quiet.
fn turn_sound(outcome: Option<&str>, answered: bool, failed_already: bool, out_of_sight: bool) -> Option<&'static str> {
    match outcome {
        Some("falhou") if !failed_already => Some("kalango-agent-failed"),
        None if answered && out_of_sight => Some("kalango-agent-done"),
        _ => None,
    }
}

fn elapsed(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        trf("{s} s", &[("s", decimal(s))])
    } else {
        trf("{m} min", &[("m", ((s / 60.0).round() as u64).to_string())])
    }
}

/// Paths under the person's home as "~/…", anywhere in the text.
/// The first widget under `w` with that CSS class.
pub(crate) fn descendant_with_class(w: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    let mut c = w.as_ref().first_child();
    while let Some(x) = c {
        if x.has_css_class(class) {
            return Some(x);
        }
        if let Some(d) = descendant_with_class(&x, class) {
            return Some(d);
        }
        c = x.next_sibling();
    }
    None
}

/// A file's chip (its kind's glyph and name; `removable`: a × to take it out of the next message).
fn attachment_chip(p: &std::path::Path, removable: bool) -> gtk::Box {
    let chip = caixa(Orientation::Horizontal, 6, "anexo-chip");
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let glifo = match ext.as_str() {
        "csv" | "tsv" | "xlsx" | "ods" => "grade",
        "png" | "jpg" | "jpeg" | "webp" | "gif" => "imagem",
        "mp3" | "ogg" | "flac" | "wav" => "musica",
        "mp4" | "mkv" | "webm" => "video",
        _ => "doc",
    };
    chip.append(&ui_icon(glifo, MUTED, 14));
    let n = rotulo(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "anexo-nome");
    n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    n.set_max_width_chars(28);
    chip.append(&n);
    chip.set_tooltip_text(Some(&p.display().to_string()));
    if removable {
        let x = gtk::Button::new();
        x.add_css_class("anexo-tirar");
        x.set_child(Some(&ui_icon("fechar", MUTED, 12)));
        x.set_tooltip_text(Some(tr("Remove")));
        chip.append(&x);
    }
    chip
}

fn first_lower(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Tools that exist only for kalangod's undo (never listed for the person).
const INTERNAL_TOOLS: &[&str] = &["arquivos.restaurar", "arquivos.apagar_criado"];

/// The area of a tool (its name prefix) in the person's words.
fn area_name(prefix: &str) -> &'static str {
    match prefix {
        "arquivos" => tr("Files"),
        "rede" => tr("Network"),
        "firewall" => tr("Firewall"),
        "discos" => tr("Disks"),
        "usuarios" => tr("Accounts"),
        "atualizacoes" => tr("Updates"),
        "pacotes" => tr("Packages"),
        "tarefas" => tr("Tasks"),
        "pontos" => tr("Restore points"),
        "servicos" => tr("Services"),
        "processos" => tr("Processes"),
        "registros" => tr("Logs"),
        "sysctl" | "kernel" => tr("Kernel"),
        "linha_do_tempo" => tr("Timeline"),
        "web" => tr("Web"),
        _ => tr("System"),
    }
}

/// "LFM3-1.7B-Q4_K_M" → "LFM3 1.7B".
pub(crate) fn model_label(file_stem: &str) -> String {
    file_stem.split('-').take_while(|p| !(p.starts_with('Q') && p.len() > 1 && p.as_bytes()[1].is_ascii_digit()) && *p != "BF16" && *p != "F16")
        .collect::<Vec<_>>().join(" ")
}

/// The model the engine will use (the same rule: KALANGO_MODELO or the first GGUF in the models folder).
pub(crate) fn guess_model() -> String {
    if let Some(p) = std::env::var_os("KALANGO_MODELO") {
        return model_label(&PathBuf::from(p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    }
    let dir = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("kalango/modelos");
    std::fs::read_dir(dir).ok().and_then(|it| it.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "gguf")).min())
        .and_then(|p| p.file_stem().map(|s| model_label(&s.to_string_lossy())))
        .unwrap_or_else(|| "LFM3".into())
}

fn agent_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_AGENTE") {
        return p.into();
    }
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|p| p.is_file()) {
        return p;
    }
    let sys = PathBuf::from("/usr/bin/kalango-agente");
    if sys.is_file() { sys } else { "kalango-agente".into() }
}

// ------------------------------------------------------------------------------------------------ state

struct Engine {
    child: Child,
    stdin: ChildStdin,
}

use crate::voz::{self, Recording};

/// The longest recording (it stops by itself).
const MAX_RECORDING: Duration = Duration::from_secs(120);

/// A request as it was sent (see `Conversa::retry`).
#[derive(Clone)]
enum LastSend {
    /// to the system's agent: the message
    System(String),
    /// to one of the person's agents
    Platform { shown: String, text: String, agent: Value, once: bool, input: Option<Value> },
}

struct Turn {
    /// the tools it used, as steps (chat_blocks.rs; design 53/54), and its place in the list
    steps: Rc<StepsList>,
    steps_holder: Animavel,
    body: gtk::Box,
    started: Instant,
    answered: bool,
    /// the answer's text (a notice says it when the person was not looking)
    answer: String,
    /// the person saw this conversation while the agent worked (then left it: the end is worth a notice)
    seen: bool,
    /// how it ended when not with an answer: ("falhou" | "parado", why)
    desfecho: Option<(&'static str, String)>,
    /// the agent's plan for this request (its widget, steps and how many are done), redrawn as it advances
    plan: Option<(gtk::Widget, Vec<String>, usize)>,
    /// the failure's sound played already (a block)
    failed_sound: bool,
}

/// The agents' task history (the agents center's "Concluído" and "Falhou"): one JSON line per request that ended,
/// in ~/.local/state/kalango/tarefas.jsonl, the last 200 kept.
pub(crate) fn tarefas_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/tarefas.jsonl")
}

pub(crate) fn registrar_tarefa(v: &Value) {
    let p = tarefas_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut linhas: Vec<String> = std::fs::read_to_string(&p).unwrap_or_default().lines().map(str::to_string).collect();
    linhas.push(v.to_string());
    let k = linhas.len().saturating_sub(200);
    let _ = std::fs::write(&p, linhas[k..].join("\n") + "\n");
}

/// The tasks that ended, most recent first.
pub(crate) fn tarefas(n: usize) -> Vec<Value> {
    std::fs::read_to_string(tarefas_path()).unwrap_or_default().lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(n).collect()
}

/// Told when the agent starts or stops working (the top bar pill follows it).
/// What the agent is doing: Some((request, steps so far)) while it works, None when it stops.
type BusyListener = Box<dyn Fn(Option<(String, usize)>)>;

pub struct Conversa {
    pub raiz: gtk::Box,
    /// the files added to the next message (chips above the text) and where they show
    anexos: RefCell<Vec<PathBuf>>,
    /// the attached recordings that are voice messages: the agent gets their words (the transcript), not the file
    voz_anexos: RefCell<Vec<PathBuf>>,
    anexos_box: gtk::Box,
    /// what the bar shows before the first message (pending requests, timeline): filled by the side bar
    pub vazio: gtk::Box,
    intro: gtk::Box,
    lista: gtk::Box,
    rolagem: gtk::ScrolledWindow,
    texto: gtk::TextView,
    placeholder: gtk::Label,
    enviar: gtk::Button,
    enviar_icone: gtk::Image,
    microfone: gtk::Button,
    voz_estado: gtk::Label,
    recording: RefCell<Option<Recording>>,
    modelo: gtk::Label,
    hora: gtk::Label,
    engine: RefCell<Option<Engine>>,
    turn: RefCell<Option<Turn>>,
    busy: Cell<bool>,
    /// the agent may read web pages (globe)
    web: Cell<bool>,
    turns: Cell<usize>,
    last_message: Cell<i64>,
    on_busy: RefCell<Option<BusyListener>>,
    /// the request being worked on (for the "Agentes ativos" widget and the top bar)
    current: RefCell<String>,
    /// the context the conversation uses (the engine's "uso" events)
    tokens: Cell<u64>,
    /// the engine process is frozen (the overview's pause)
    paused: Cell<bool>,
    /// the conversation's id: its history (the engine's messages and these events) is kept on disk
    id: RefCell<String>,
    /// rebuilding a kept conversation from its events: nothing is recorded or asked again
    replaying: Cell<bool>,
    /// the tool groups this conversation turned off (sent to the engine)
    desligadas: RefCell<Vec<String>>,
    /// while rebuilding: when the turn's message was sent and its last event (its real duration)
    replay_tempo: Cell<(i64, i64)>,
    /// the agent header (the agents center hides it: its messages carry their own)
    cabecalho: gtk::Box,
    /// the requests waiting for the person: their "allow once" button (the approval notice answers through it)
    pedidos: RefCell<std::collections::BTreeMap<u64, gtk::Button>>,
    ao_pedir: RefCell<Option<AoPedir>>,
    /// whether the person sees this conversation now (its owner knows: the bar open in Agent mode, the agents
    /// center in front on Conversas)
    visivel: RefCell<Option<Visivel>>,
    /// action buttons whose place the session test reads (said again when the turn ends: the answer that follows
    /// moves them)
    posicoes: RefCell<Vec<(String, glib::WeakRef<gtk::Button>)>>,
    /// others that follow what the agent does in this conversation (the Files window marks the files it touches)
    ouvintes: RefCell<Vec<Ouvinte>>,
    /// the pages read for this request (the answer's "Fontes"), and where a page opens: the agent's browser in the
    /// side bar (with the person in control, when it asks for a login)
    fontes: RefCell<Vec<(String, String)>>,
    pub on_page: RefCell<Option<Box<dyn Fn(&str, bool)>>>,
    // ---- the person's own agents (kalango-fluxos), in the agents center
    /// the agent this conversation talks to now (None: the system's own, through the engine)
    agent: RefCell<Option<Value>>,
    /// the agent of the turn on screen (its answer's header and footer)
    turn_agent: RefCell<Option<Value>>,
    /// the last request, for "tentar de novo" on a failed step; the input a resend reuses
    last_send: RefCell<Option<LastSend>>,
    retry_input: RefCell<Option<Option<Value>>>,
    /// the composer's row above the text: the agent's chip and its input fields
    agent_row: gtk::Box,
    agent_chip: gtk::Button,
    pub input_form: Rc<agent_picker::InputForm>,
    input_error: gtk::Label,
    picker: RefCell<Option<Rc<Picker>>>,
    /// the picker opened on "@" typed in the composer (the text keeps the focus)
    mention_mode: Cell<bool>,
    /// (side bar) the header's chip is the agent's (the narrow picker), "@nome" suggests the agents and calls one for
    /// that message only, and every answer says which agent gave it
    side: Cell<bool>,
    head_tile: gtk::Box,
    head_name: gtk::Label,
    head_chip: gtk::Button,
    /// (side bar) the "@nome" suggestions over the composer
    suggest: RefCell<Option<Rc<Picker>>>,
    /// told when the agent changes (the header's chip), and what the picker leaves to the page (run a flow, create
    /// or manage agents)
    on_agent: RefCell<Option<Box<dyn Fn(Option<&Value>)>>>,
    pub on_picked_other: RefCell<Option<Box<dyn Fn(Picked)>>>,
    /// the request to kalango-fluxos in progress: its reference (events and the answer are matched by it) and run
    client_ref: RefCell<String>,
    run_id: RefCell<Option<String>>,
    /// approvals of kalango-fluxos on screen: the card's id → the service's approval id
    remote_approvals: RefCell<std::collections::HashMap<u64, (String, String)>>,
    /// the tools of this request the person denied (their steps end as denied)
    remote_denied: RefCell<Vec<String>>,
    remote_seq: Cell<u64>,
    /// events shown but not kept (the live steps of a request to kalango-fluxos, kept whole when it ends)
    not_kept: Cell<bool>,
    /// the request to kalango-fluxos: when it was sent, and each answer of a member or task as it came (author,
    /// time since sent) — the rows' times
    platform_started: Cell<Option<Instant>>,
    arrivals: RefCell<Vec<(String, Duration)>>,
}

type Ouvinte = Box<dyn Fn(&Value)>;

type Visivel = Box<dyn Fn() -> bool>;

/// A request appeared (Some(what, where)) while the conversation is not on screen, or it was answered (None).
type AoPedir = Box<dyn Fn(u64, Option<(String, String)>)>;

/// Where the conversations are kept (one folder each: meta.json, eventos.jsonl, and the engine's mensagens.json).
pub(crate) fn conversas_dir() -> PathBuf {
    glib::user_data_dir().join("kalango/conversas")
}

fn novo_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{t:x}-{:x}", std::process::id())
}

/// The badge of an approval card that is not high risk: what kind of thing the tool does (a web search said
/// "Muda arquivos" before).
fn approval_badge(tool: &str) -> &'static str {
    match tool.split('.').next().unwrap_or("") {
        "arquivos" | "documentos" => kalango_i18n::n_("Changes files"),
        "web" | "http" | "navegador" => kalango_i18n::n_("Uses the internet"),
        _ => kalango_i18n::n_("Acts on the system"),
    }
}

/// The pictures and sounds of a message, for kalango-fluxos ({path, mime}); the rest (and a voice message's
/// recording) only goes by its path in the text.
fn media_attachments(files: &[PathBuf], voice: &[PathBuf]) -> Vec<Value> {
    files
        .iter()
        .filter(|p| !voice.contains(p))
        .filter_map(|p| {
            // (by the extension first: the MIME database may be missing, and a guess with no data gave
            // "application/octet-stream" for a .png)
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let mime = match ext.as_str() {
                "png" | "gif" | "webp" | "bmp" => format!("image/{ext}"),
                "jpg" | "jpeg" => "image/jpeg".to_string(),
                "wav" => "audio/wav".to_string(),
                "mp3" => "audio/mpeg".to_string(),
                "ogg" | "oga" | "opus" => "audio/ogg".to_string(),
                "flac" => "audio/flac".to_string(),
                "m4a" => "audio/mp4".to_string(),
                _ => {
                    let (ct, _) = gio::content_type_guess(Some(p), &[]);
                    gio::content_type_get_mime_type(&ct).map(|m| m.to_string()).unwrap_or_default()
                }
            };
            (mime.starts_with("image/") || mime.starts_with("audio/")).then(|| json!({"path": p.display().to_string(), "mime": mime}))
        })
        .collect()
}

/// A member's row as the conversation keeps it (its words already in the person's language).
fn member_event(c: &StepCall) -> Value {
    let m = c.member.clone().unwrap_or(crate::chat_blocks::Member { tag: String::new(), name: String::new(), text: String::new() });
    json!({"tipo": "membro", "marca": m.tag, "nome": m.name, "texto": m.text, "resumo": c.result, "segundos": c.took.map(|d| d.as_secs_f64())})
}

/// Every kept conversation: (id, meta), the latest first.
pub(crate) fn conversas() -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = std::fs::read_dir(conversas_dir()).into_iter().flatten().filter_map(|e| e.ok()).filter_map(|e| {
        let id = e.file_name().to_string_lossy().into_owned();
        let meta: Value = serde_json::from_str(&std::fs::read_to_string(e.path().join("meta.json")).ok()?).ok()?;
        Some((id, meta))
    }).collect();
    v.sort_by_key(|(_, m)| std::cmp::Reverse(m["atualizado"].as_i64().unwrap_or(0)));
    v
}

/// A conversation's events (what the agents center's context panel reads).
pub(crate) fn eventos(id: &str) -> Vec<Value> {
    std::fs::read_to_string(conversas_dir().join(id).join("eventos.jsonl")).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

pub(crate) fn meta(id: &str) -> Value {
    std::fs::read_to_string(conversas_dir().join(id).join("meta.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}))
}

pub(crate) fn salvar_meta(id: &str, m: &Value) {
    let d = conversas_dir().join(id);
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join("meta.json"), m.to_string());
}

impl Conversa {
    pub fn new() -> Rc<Conversa> {
        let raiz = caixa(Orientation::Vertical, 12, "conversa");
        raiz.set_vexpand(true);

        // agent header: spark tile, name, model chip, time
        let cab = caixa(Orientation::Horizontal, 8, "agente-cabecalho");
        let tile = caixa(Orientation::Horizontal, 0, "agente-tile");
        tile.set_hexpand(false);
        tile.set_halign(Align::Start);
        tile.set_valign(Align::Center);
        let sp = estrela(13, ACCENT);
        sp.set_halign(Align::Center);
        sp.set_valign(Align::Center);
        sp.set_margin_start(5);
        sp.set_margin_end(5);
        tile.append(&sp);
        cab.append(&tile);
        let head_name = rotulo(tr("Agent"), "agente-nome");
        cab.append(&head_name);
        // (side bar) the agent's chip in their place: it opens the picker
        let head_chip = gtk::Button::new();
        head_chip.add_css_class("agente-chip-cabecalho");
        head_chip.set_tooltip_text(Some(tr("Choose the agent (or type @)")));
        head_chip.set_valign(Align::Center);
        head_chip.set_visible(false);
        cab.append(&head_chip);
        let head_tile = tile.clone();
        let modelo = rotulo(&format!("{} · {}", guess_model(), tr("local")), "chip-modelo");
        cab.append(&modelo);
        let hora = rotulo("", "agente-hora");
        hora.set_hexpand(true);
        hora.set_halign(Align::Start);
        cab.append(&hora);
        let nova = gtk::Button::new();
        nova.set_child(Some(&ui_icon("mais", MUTED, 16)));
        nova.add_css_class("fantasma");
        nova.set_tooltip_text(Some(tr("New conversation (/nova)")));
        cab.append(&nova);
        raiz.append(&cab);
        let cabecalho = cab.clone();

        // turns (and the intro/empty state before the first one)
        let lista = caixa(Orientation::Vertical, 12, "");
        let intro = caixa(Orientation::Vertical, 12, "");
        let vazio = caixa(Orientation::Vertical, 12, "");
        intro.append(&vazio);
        lista.append(&intro);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&lista));
        raiz.append(&rolagem);

        // composer: the files added, the text, then tools (attach · model · send/stop)
        let caixa_c = caixa(Orientation::Vertical, 8, "compositor");
        // (agents center) the agent's chip — the picker — and the fields its input asks for
        let agent_row = caixa(Orientation::Horizontal, 8, "");
        agent_row.set_visible(false);
        let agent_chip = gtk::Button::new();
        agent_chip.add_css_class("agente-chip-compositor");
        agent_chip.set_tooltip_text(Some(tr("Choose the agent (or type @)")));
        agent_chip.set_valign(Align::Center);
        agent_row.append(&agent_chip);
        let input_form = agent_picker::InputForm::new();
        input_form.root.set_visible(false);
        let campos = gtk::ScrolledWindow::new();
        campos.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        campos.set_hexpand(true);
        campos.set_propagate_natural_height(true);
        campos.set_child(Some(&input_form.root));
        agent_row.append(&campos);
        caixa_c.append(&agent_row);
        let input_error = crate::fluxos_ui::error_label();
        caixa_c.append(&input_error);
        let anexos_box = caixa(Orientation::Horizontal, 6, "");
        anexos_box.set_visible(false);
        caixa_c.append(&anexos_box);
        let sobre = gtk::Overlay::new();
        let texto = gtk::TextView::new();
        texto.set_wrap_mode(gtk::WrapMode::WordChar);
        texto.add_css_class("compositor-texto");
        texto.set_accepts_tab(false);
        // directly in the box: the text view is measured with its real width and grows line by line
        // (a ScrolledWindow would measure it as one long line)
        texto.set_size_request(-1, 24);
        sobre.set_child(Some(&texto));
        let placeholder = rotulo(tr("Ask anything · / commands"), "compositor-dica");
        placeholder.set_halign(Align::Start);
        placeholder.set_valign(Align::Start);
        placeholder.set_can_target(false);
        sobre.add_overlay(&placeholder);
        caixa_c.append(&sobre);
        let ferramentas = caixa(Orientation::Horizontal, 6, "");
        let anexar = gtk::Button::new();
        anexar.set_child(Some(&ui_icon("mais", MUTED, 16)));
        anexar.add_css_class("botao-compositor");
        anexar.set_tooltip_text(Some(tr("Add a file to the message")));
        ferramentas.append(&anexar);
        // the globe: reading web pages in this conversation (on by default, accent tint when on)
        let web = gtk::ToggleButton::new();
        let web_icone = ui_icon("globo", ACCENT, 16);
        web.set_child(Some(&web_icone));
        web.add_css_class("botao-compositor");
        web.add_css_class("alternavel");
        web.set_active(true);
        web.set_tooltip_text(Some(tr("Web pages: the agent may read them")));
        ferramentas.append(&web);
        let espaco = gtk::Box::new(Orientation::Horizontal, 0);
        espaco.set_hexpand(true);
        ferramentas.append(&espaco);
        let chip = caixa(Orientation::Horizontal, 6, "chip-modelo-compositor");
        let ponto = caixa(Orientation::Horizontal, 0, "ponto-modelo");
        ponto.set_valign(Align::Center);
        chip.append(&ponto);
        let modelo_c = rotulo(&guess_model(), "");
        chip.append(&modelo_c);
        chip.set_tooltip_text(Some(tr("Local model: nothing leaves this computer")));
        ferramentas.append(&chip);
        // settings: the agent's permissions per tool (free / ask / never)
        let ajustes = gtk::MenuButton::new();
        ajustes.set_child(Some(&ui_icon("ajustes", MUTED, 16)));
        ajustes.add_css_class("botao-compositor");
        ajustes.set_tooltip_text(Some(tr("Agent permissions")));
        ajustes.set_direction(gtk::ArrowType::Up);
        ferramentas.insert_child_after(&ajustes, Some(&web));
        // voice: record, then the local audio model turns it into text in the composer
        let voz_estado = rotulo("", "voz-estado");
        voz_estado.set_visible(false);
        // it must not widen the composer (and the bar): cut with an ellipsis, full text in the tooltip
        voz_estado.set_ellipsize(gtk::pango::EllipsizeMode::End);
        voz_estado.set_width_chars(4);
        voz_estado.set_max_width_chars(14);
        ferramentas.insert_child_after(&voz_estado, Some(&ajustes));
        let microfone = gtk::Button::new();
        microfone.set_child(Some(&ui_icon("microfone", MUTED, 16)));
        microfone.add_css_class("botao-compositor");
        microfone.set_tooltip_text(Some(tr("Dictate (the local voice model writes it here)")));
        ferramentas.append(&microfone);
        let enviar = gtk::Button::new();
        let enviar_icone = ui_icon("enviar", ON_ACCENT, 16);
        enviar.set_child(Some(&enviar_icone));
        enviar.add_css_class("botao-enviar");
        enviar.set_tooltip_text(Some(tr("Send (Enter)")));
        ferramentas.append(&enviar);
        caixa_c.append(&ferramentas);
        raiz.append(&caixa_c);

        let c = Rc::new(Conversa {
            anexos: RefCell::default(), voz_anexos: RefCell::default(), anexos_box,
            raiz, vazio, intro, lista, rolagem, texto, placeholder, enviar, enviar_icone, microfone, voz_estado,
            recording: RefCell::default(), modelo, hora,
            engine: RefCell::default(), turn: RefCell::default(), busy: Cell::new(false), web: Cell::new(true), turns: Cell::new(0),
            last_message: Cell::new(0), on_busy: RefCell::default(), current: RefCell::default(), tokens: Cell::new(0), paused: Cell::new(false), id: RefCell::new(novo_id()), replaying: Cell::new(false), desligadas: RefCell::default(), replay_tempo: Cell::new((0, 0)), pedidos: RefCell::default(), ao_pedir: RefCell::default(), visivel: RefCell::default(), posicoes: RefCell::default(), ouvintes: RefCell::default(), fontes: RefCell::default(), on_page: RefCell::default(), cabecalho,
            agent: RefCell::default(), turn_agent: RefCell::default(), last_send: RefCell::default(), retry_input: RefCell::default(), agent_row, agent_chip, input_form, input_error, picker: RefCell::default(), mention_mode: Cell::new(false),
            side: Cell::new(false), head_tile, head_name, head_chip, suggest: RefCell::default(),
            on_agent: RefCell::default(), on_picked_other: RefCell::default(), client_ref: RefCell::default(), run_id: RefCell::default(), remote_approvals: RefCell::default(), remote_denied: RefCell::default(),
            remote_seq: Cell::new(1 << 40), not_kept: Cell::new(false), platform_started: Cell::new(None), arrivals: RefCell::default(),
        });
        c.show_agent_chip();
        let w = Rc::downgrade(&c);
        c.agent_chip.connect_clicked(move |b| {
            if let Some(c) = w.upgrade() {
                c.mention_mode.set(false);
                let p = c.picker();
                p.popover.set_autohide(true);
                let cur = c.agent.borrow().as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
                p.open(b, cur.as_deref(), "", true);
            }
        });
        let w = Rc::downgrade(&c);
        c.head_chip.connect_clicked(move |b| {
            if let Some(c) = w.upgrade() {
                c.close_picker();
                let p = c.picker();
                p.popover.set_autohide(true);
                let cur = c.agent.borrow().as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
                p.open(b, cur.as_deref(), "", true);
            }
        });
        // the chip in the composer follows the header's
        {
            let mc = modelo_c.clone();
            c.modelo.connect_label_notify(move |l| mc.set_text(l.text().split(" · ").next().unwrap_or("")));
        }
        c.build_intro();
        // a code block's "Aplicar no arquivo": the agent applies it (the change shows for approval before it is written)
        let acoes = gio::SimpleActionGroup::new();
        let aplicar = gio::SimpleAction::new("aplicar", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(&c);
        aplicar.connect_activate(move |_, p| {
            let (Some(c), Some(p)) = (w.upgrade(), p.and_then(|p| p.str().map(str::to_string))) else { return };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&p) else { return };
            let (f, lang, code) = (v["file"].as_str().unwrap_or(""), v["lang"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
            if c.busy.get() || f.is_empty() {
                return;
            }
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug codigo: aplicar em {f}");
            }
            let pedido = trf("Apply this code to the file {file} (change only what it changes):", &[("file", f.to_string())]);
            c.ask(&format!("{pedido}\n```{lang}\n{code}\n```"));
        });
        acoes.add_action(&aplicar);
        c.raiz.insert_action_group("conversa", Some(&acoes));
        // (where its blocks keep their files: a LaTeX document's PDF)
        let w = Rc::downgrade(&c);
        let pasta: crate::latex_pdf::FolderFn = Rc::new(move || w.upgrade().map(|c| conversas_dir().join(c.id())).unwrap_or_else(conversas_dir));
        unsafe {
            c.raiz.set_data("conversa_pasta", pasta);
        }

        let w = Rc::downgrade(&c);
        c.texto.buffer().connect_changed(move |b| {
            if let Some(c) = w.upgrade() {
                c.placeholder.set_visible(b.char_count() == 0);
                c.follow_mention(&b.text(&b.start_iter(), &b.end_iter(), false));
            }
        });
        // Enter sends, Shift+Enter breaks the line
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&c);
        keys.connect_key_pressed(move |_, k, _, m| {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug conversa tecla: {k:?} {m:?}");
            }
            if let Some(c) = w.upgrade().filter(|c| c.mention_mode.get() && c.mention_picker().is_some_and(|p| p.is_open())) {
                // "@nome": the arrows move the highlight, Enter picks it, Escape closes the list
                if matches!(k, gdk::Key::Up | gdk::Key::Down) {
                    if let Some(p) = c.mention_picker() {
                        p.move_focus(if k == gdk::Key::Up { -1 } else { 1 });
                    }
                    return glib::Propagation::Stop;
                }
                if matches!(k, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab) && c.mention_picker().is_some_and(|p| p.pick_first()) {
                    return glib::Propagation::Stop;
                }
                if k == gdk::Key::Escape {
                    c.close_picker();
                    return glib::Propagation::Stop;
                }
            }
            if matches!(k, gdk::Key::Return | gdk::Key::KP_Enter) && !m.contains(gdk::ModifierType::SHIFT_MASK) {
                if let Some(c) = w.upgrade() {
                    c.submit();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        c.texto.add_controller(keys);
        let w = Rc::downgrade(&c);
        c.enviar.connect_clicked(move |_| {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug conversa: botão enviar/parar");
            }
            if let Some(c) = w.upgrade() {
                if c.busy.get() { c.stop(true) } else { c.submit() }
            }
        });
        let w = Rc::downgrade(&c);
        nova.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.new_conversation();
            }
        });
        let w = Rc::downgrade(&c);
        web.connect_toggled(move |b| {
            let Some(c) = w.upgrade() else { return };
            let on = b.is_active();
            c.web.set(on);
            set_icon(&web_icone, "globo", if on { ACCENT } else { MUTED });
            b.set_tooltip_text(Some(if on { tr("Web pages: the agent may read them") } else { tr("Web pages: off in this conversation") }));
            // a running engine learns it at once; a new one gets it when it starts
            let mut eng = c.engine.borrow_mut();
            if let Some(e) = eng.as_mut() {
                let _ = writeln!(e.stdin, "{}", json!({"web": on})).and_then(|_| e.stdin.flush());
            }
        });
        let w = Rc::downgrade(&c);
        let pop = gtk::Popover::new();
        pop.add_css_class("permissoes");
        ajustes.set_popover(Some(&pop));
        pop.connect_show(move |p| {
            if let Some(c) = w.upgrade() {
                c.fill_permissions(p);
            }
        });
        crate::central::posicao_debug_sempre("conversa microfone", c.microfone.upcast_ref());
        let w = Rc::downgrade(&c);
        c.microfone.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                if c.recording.borrow().is_some() { c.stop_recording() } else { c.start_recording() }
            }
        });
        let w = Rc::downgrade(&c);
        anexar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.attach();
            }
        });
        // files dropped on the conversation (from the Shelf, Files…) go with the next message
        let soltar = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = Rc::downgrade(&c);
        soltar.connect_drop(move |_, v, _, _| {
            let (Some(c), Ok(l)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
            for f in l.files() {
                if let Some(p) = f.path() {
                    c.add_attachment(p);
                }
            }
            c.focus();
            true
        });
        c.raiz.add_controller(soltar);
        // the time of the last message ("now", "3 min") stays current
        let w = Rc::downgrade(&c);
        glib::timeout_add_seconds_local(30, move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            c.update_time();
            glib::ControlFlow::Continue
        });
        c
    }

    pub fn on_busy(&self, f: impl Fn(Option<(String, usize)>) + 'static) {
        *self.on_busy.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_empty(&self) -> bool {
        self.turns.get() == 0
    }

    /// The agent is working on a request.
    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    pub fn focus(&self) {
        self.texto.grab_focus();
        // for the session test: where the composer buttons are (centres, in output coordinates)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let row = self.microfone.parent();
            let root = self.raiz.root();
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                let (Some(row), Some(root)) = (row, root) else { return };
                // the bar sits at the right edge with an 8 px margin: surface → output coordinates
                let out_w = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>()
                    .map(|m| m.geometry().width()).unwrap_or(1920);
                let dx = f64::from(out_w - 8 - root.width());
                let mut c = row.first_child();
                let mut i = 0;
                while let Some(w) = c {
                    if let Some(b) = w.compute_bounds(&root) {
                        eprintln!("debug compositor botão {i}: {} {} {}", w.css_classes().join("."), (dx + f64::from(b.x() + b.width() / 2.)) as i32, (b.y() + b.height() / 2.) as i32);
                    }
                    i += 1;
                    c = w.next_sibling();
                }
            });
        }
    }

    fn set_busy(&self, b: bool) {
        self.busy.set(b);
        if b {
            set_icon(&self.enviar_icone, "parar", ON_ACCENT);
            self.enviar.add_css_class("parar");
            self.enviar.set_tooltip_text(Some(tr("Stop")));
        } else {
            set_icon(&self.enviar_icone, "enviar", ON_ACCENT);
            self.enviar.remove_css_class("parar");
            self.enviar.set_tooltip_text(Some(tr("Send (Enter)")));
        }
        self.tell_activity(if b { Some(0) } else { None });
    }

    fn tell_activity(&self, steps: Option<usize>) {
        if let Some(f) = self.on_busy.borrow().as_ref() {
            f(steps.map(|n| (self.current.borrow().clone(), n)));
        }
    }

    fn update_time(&self) {
        let t = self.last_message.get();
        self.hora.set_text(&if t == 0 { String::new() } else { crate::lateral::tempo_relativo(t) });
    }

    /// Greeting and suggestions above the empty state.
    fn build_intro(self: &Rc<Self>) {
        let ola = caixa(Orientation::Vertical, 6, "agente-ola");
        let t = rotulo(tr("What can I do for you?"), "agente-ola-titulo");
        t.set_halign(Align::Start);
        ola.append(&t);
        let s = rotulo(tr("The agent runs on the local model and acts on this computer through kalangod: you approve system changes and everything goes into the timeline, where it can be undone."), "agente-ola-texto");
        s.set_wrap(true);
        s.set_xalign(0.0);
        ola.append(&s);
        let sugestoes = gtk::FlowBox::new();
        sugestoes.set_selection_mode(gtk::SelectionMode::None);
        sugestoes.set_max_children_per_line(2);
        sugestoes.set_column_spacing(6);
        sugestoes.set_row_spacing(6);
        for q in [tr("How much free space do I have?"), tr("What is using the most memory?"), tr("Which services failed?"), tr("What did the agents do today?")] {
            let b = gtk::Button::with_label(q);
            b.add_css_class("sugestao");
            if let Some(l) = b.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                l.set_wrap(true);
                l.set_xalign(0.0);
            }
            let w = Rc::downgrade(self);
            let q = q.to_string();
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.send(&q);
                }
            });
            sugestoes.append(&b);
        }
        ola.append(&sugestoes);
        self.intro.prepend(&ola);
    }

    fn scroll_to_end(&self) {
        let r = self.rolagem.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(r) = r.upgrade() {
                let a = r.vadjustment();
                a.set_value(a.upper() - a.page_size());
            }
        });
    }

    fn submit(self: &Rc<Self>) {
        let b = self.texto.buffer();
        let text = b.text(&b.start_iter(), &b.end_iter(), false).trim().to_string();
        if self.busy.get() && matches!(text.as_str(), "/parar" | "/stop") {
            b.set_text("");
            return self.stop(true);
        }
        if text.is_empty() || self.busy.get() {
            return;
        }
        b.set_text("");
        self.send(&text);
    }

    // ---------------------------------------------------------------------------------------------- sending

    /// A request from outside the bar (the command bar): sent now, or left in the composer while the agent
    /// is busy with another one.
    pub fn ask(self: &Rc<Self>, text: &str) {
        if self.busy.get() {
            self.texto.buffer().set_text(text);
            self.focus();
        } else {
            self.send(text);
        }
    }

    fn send(self: &Rc<Self>, text: &str) {
        if self.busy.get() {
            return;
        }
        match text.trim() {
            "/nova" | "/new" => return self.new_conversation(),
            "/parar" | "/stop" => return self.stop(true),
            t if t.starts_with('/') => {
                self.note(&trf("Commands: {list}", &[("list", "/nova, /parar".into())]));
                return;
            }
            _ => {}
        }
        // "@Nome" calls one of the person's agents; the message goes on without the mention. In the agents center the
        // conversation goes on with it; in the side bar only this message goes to it (the person's text keeps the
        // mention)
        let shown = text.to_string();
        let mut text = text.to_string();
        let mut once: Option<Option<Value>> = None;
        if self.agents_on() && text.contains('@') {
            let agents = agent_picker::cached_agents();
            let mut names: Vec<(String, String)> = agents.iter().filter_map(|a| Some((a["id"].as_str()?.to_string(), a["name"].as_str()?.to_string()))).collect();
            names.push((String::new(), agent_picker::SYSTEM_AGENT.to_string()));
            if let Some((id, rest)) = agent_picker::resolve_mention(&text, &names) {
                let a = agents.into_iter().find(|a| a["id"].as_str() == Some(id.as_str()));
                if let Some(flow) = a.clone().filter(agent_picker::is_flow) {
                    self.picked(Picked::Run(flow));
                    return;
                }
                let needs_input = a.as_ref().is_some_and(|a| agent_picker::fields(&a["input_fields"]).iter().any(|f| f.required));
                if self.side.get() && !needs_input {
                    if rest.is_empty() {
                        return;
                    }
                    once = Some(a);
                } else {
                    // (one that asks for fields takes the conversation: its fields show in the composer)
                    self.set_agent(a);
                    if rest.is_empty() && self.input_form.is_empty() {
                        return;
                    }
                    if self.side.get() && !self.input_form.is_empty() {
                        self.texto.buffer().set_text(&rest);
                        return;
                    }
                }
                text = rest;
            }
        }
        let shown = if once.is_some() { shown } else { text.clone() };
        let text = text.as_str();
        let target = match once.clone() {
            Some(a) => a,
            None => self.agent.borrow().clone(),
        };
        if let Some(a) = target {
            return self.send_platform(&shown, text, a, once.is_some());
        }
        *self.turn_agent.borrow_mut() = None;
        *self.last_send.borrow_mut() = Some(LastSend::System(text.to_string()));
        self.intro.set_visible(false);
        self.turns.set(self.turns.get() + 1);
        self.last_message.set(glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0));
        self.update_time();
        // kept: the person's message in the log, the conversation's title and time
        self.registrar(&json!({"tipo": "pessoa", "texto": shown, "anexos": self.anexos.borrow().iter().map(|p| p.display().to_string()).collect::<Vec<_>>()}));
        let id = self.id.borrow().clone();
        let mut m = meta(&id);
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        if m["titulo"].as_str().is_none_or(|t| t.is_empty()) {
            let t: String = shown.lines().next().unwrap_or("").chars().take(60).collect();
            m["titulo"] = json!(t);
            m["criado"] = json!(agora);
        }
        m["atualizado"] = json!(agora);
        salvar_meta(&id, &m);

        // the person's message (the files added to it above)
        let anexos: Vec<PathBuf> = self.anexos.borrow_mut().drain(..).collect();
        while let Some(ch) = self.anexos_box.first_child() {
            self.anexos_box.remove(&ch);
        }
        self.anexos_box.set_visible(false);
        self.bolha_pessoa_com(&shown, &anexos);
        // (the agent reads them with its own tools: their paths go with the request — not a voice message's
        // recording, whose words are the text)
        let voz: Vec<PathBuf> = self.voz_anexos.borrow_mut().drain(..).collect();
        let para_o_agente: Vec<&PathBuf> = anexos.iter().filter(|p| !voz.contains(p)).collect();
        let text = &if para_o_agente.is_empty() {
            text.to_string()
        } else {
            format!("{text}\n\n{} {}", tr("Attached files:"), para_o_agente.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
        };

        *self.current.borrow_mut() = text.to_string();
        // steps block (shown on the first event) and the answer area
        let turn = self.new_turn();
        *self.turn.borrow_mut() = Some(turn);
        self.set_busy(true);
        crate::system_sounds::play_agent("kalango-agent-start");
        self.scroll_to_end();

        if let Err(e) = self.write_line(text) {
            self.cartao_erro(None, &trf("Could not start the agent: {error}", &[("error", e)]));
            self.finish_turn();
        }
    }

    fn new_turn(self: &Rc<Self>) -> Turn {
        self.fontes.borrow_mut().clear();
        // in the agents center each answer carries its header (agent, model, when): the bar has one for all
        if self.raiz.has_css_class("conversa-app") {
            let t = if self.replaying.get() { self.replay_tempo.get().0 } else { glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) };
            let quando = glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M").ok()).map(|x| x.to_string()).unwrap_or_default();
            let platform = self.turn_agent.borrow().clone();
            let name = platform.as_ref().and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| tr("Agent").to_string());
            let h = agent_header(&name, &[self.modelo.text().to_string()], &quando);
            if let Some(a) = platform.as_ref() {
                // one of the person's agents: its own icon in place of the system's ✦
                if let Some(first) = h.first_child() {
                    h.remove(&first);
                }
                h.prepend(&agent_picker::icon_tile(a, 24));
            }
            self.lista.append(&h);
        }
        let steps = StepsList::new(false);
        steps.set_instant(self.replaying.get());
        // "tentar de novo" on a failed step sends the request again (not in a kept conversation being rebuilt)
        if !self.replaying.get() {
            let w = Rc::downgrade(self);
            steps.set_on_retry(move || {
                if let Some(c) = w.upgrade() {
                    c.retry();
                }
            });
        }
        let a = Animavel::new(&steps.widget());
        self.lista.append(&a);
        a.entrar((0.0, 8.0), 1.0);
        let body = caixa(Orientation::Vertical, 10, "");
        if self.side.get() {
            // (side bar) who answers: the agent's icon and name, and when
            let who = caixa(Orientation::Horizontal, 6, "resposta-agente");
            let a = self.turn_agent.borrow().clone();
            who.append(&agent_picker::tile_for(a.as_ref(), 18));
            let name = a.as_ref().and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| agent_picker::SYSTEM_AGENT.to_string());
            let n = rotulo(&name, "resposta-agente-nome");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            who.append(&n);
            let t = if self.replaying.get() { self.replay_tempo.get().0 } else { glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) };
            who.append(&rotulo(&format!("· {}", crate::lateral::tempo_relativo(t)), "agente-hora"));
            body.append(&who);
        }
        self.lista.append(&body);
        Turn { steps, steps_holder: a, body, started: Instant::now(), answered: false, answer: String::new(), seen: false, desfecho: None, plan: None, failed_sound: false }
    }

    /// Sends a line to the engine, starting it first if needed (one process per conversation).
    fn write_line(self: &Rc<Self>, text: &str) -> Result<(), String> {
        let line = format!("{}\n", serde_json::to_string(text).unwrap_or_default());
        if let Some(e) = self.engine.borrow_mut().as_mut() {
            if e.stdin.write_all(line.as_bytes()).and_then(|_| e.stdin.flush()).is_ok() {
                return Ok(());
            }
        }
        self.start_engine()?;
        let mut eng = self.engine.borrow_mut();
        let e = eng.as_mut().ok_or("no engine")?;
        e.stdin.write_all(line.as_bytes()).and_then(|_| e.stdin.flush()).map_err(|e| e.to_string())
    }

    fn start_engine(self: &Rc<Self>) -> Result<(), String> {
        let bin = agent_binary();
        let mut cmd = Command::new(&bin);
        cmd.args(["--json", "--conversa", &self.id.borrow()]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        // the engine dies with the shell (a crash or a restart of the shell leaves no agent behind)
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                extern "C" {
                    fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
                }
                const PR_SET_PDEATHSIG: i32 = 1;
                const SIGTERM: u64 = 15;
                prctl(PR_SET_PDEATHSIG, SIGTERM, 0, 0, 0);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
        let stdin = child.stdin.take().ok_or("stdin")?;
        let stdout = child.stdout.take().ok_or("stdout")?;
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(json!({"tipo": "saiu"}));
        });
        let w = Rc::downgrade(self);
        let pid = child.id();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            loop {
                match rx.try_recv() {
                    Ok(v) => {
                        // events of an engine that was replaced (stop, new conversation) are dropped
                        let current = c.engine.borrow().as_ref().map(|e| e.child.id()) == Some(pid);
                        if current {
                            c.handle(&v);
                        }
                        if v["tipo"] == "saiu" {
                            return glib::ControlFlow::Break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
        });
        let mut stdin = stdin;
        if !self.web.get() {
            writeln!(stdin, "{}", json!({"web": false})).map_err(|e| e.to_string())?;
        }
        if !self.desligadas.borrow().is_empty() {
            writeln!(stdin, "{}", json!({"desligadas": *self.desligadas.borrow()})).map_err(|e| e.to_string())?;
        }
        *self.engine.borrow_mut() = Some(Engine { child, stdin });
        Ok(())
    }

    fn kill_engine(&self) {
        self.tokens.set(0);
        self.paused.set(false);
        if let Some(mut e) = self.engine.borrow_mut().take() {
            let _ = e.child.kill();
            std::thread::spawn(move || {
                let _ = e.child.wait();
            });
        }
    }

    /// An event in the conversation's log (with its time); not while rebuilding from the log.
    fn registrar(&self, v: &Value) {
        if self.replaying.get() || self.not_kept.get() {
            return;
        }
        let d = conversas_dir().join(&*self.id.borrow());
        let _ = std::fs::create_dir_all(&d);
        let mut v = v.clone();
        v["t"] = json!(glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(d.join("eventos.jsonl")) {
            let _ = writeln!(f, "{v}");
        }
    }

    pub fn id(&self) -> String {
        self.id.borrow().clone()
    }

    /// Opens a kept conversation: its events rebuild what was said and done; the next message goes on from there
    /// (the engine loads its history).
    pub fn abrir(self: &Rc<Self>, id: &str) {
        self.new_conversation();
        *self.id.borrow_mut() = id.to_string();
        self.replaying.set(true);
        for v in eventos(id) {
            let t = v["t"].as_i64().unwrap_or(0);
            if v["tipo"] == "pessoa" {
                self.intro.set_visible(false);
                self.turns.set(self.turns.get() + 1);
                self.last_message.set(v["t"].as_i64().unwrap_or(0));
                let anexos: Vec<PathBuf> = v["anexos"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(PathBuf::from)).collect();
                if self.turn.borrow().is_some() {
                    self.finish_turn();
                }
                // (a message to one of the person's agents: its input above, its agent on the answer)
                let agent = v["agent"].as_object().map(|_| v["agent"].clone());
                match (&agent, v["input"].as_object()) {
                    (Some(a), Some(_)) => {
                        let fields = agent_picker::fields(if v["fields"].is_array() { &v["fields"] } else { &a["input_fields"] });
                        let fields = if fields.is_empty() { v["input"].as_object().map(|o| o.keys().map(|k| agent_picker::Field { name: k.clone(), kind: String::new(), required: false, description: String::new() }).collect()).unwrap_or_default() } else { fields };
                        self.person_bubble_with_input(v["texto"].as_str().unwrap_or(""), &anexos, &fields, Some(&v["input"]));
                    }
                    _ => self.bolha_pessoa_com(v["texto"].as_str().unwrap_or(""), &anexos),
                }
                *self.turn_agent.borrow_mut() = agent;
                self.replay_tempo.set((t, t));
                let turno = self.new_turn();
                *self.turn.borrow_mut() = Some(turno);
            } else {
                self.replay_tempo.set((self.replay_tempo.get().0, t));
                self.handle(&v);
            }
        }
        if self.turn.borrow().is_some() {
            self.finish_turn();
        }
        self.replaying.set(false);
        self.set_busy(false);
        // the agent it talked to last goes on (the same session in kalango-fluxos); its fields come with the list
        let m = meta(id);
        let kept = m["agent"].as_object().map(|_| m["agent"].clone());
        self.set_agent(kept.clone());
        if let Some(k) = kept {
            let w = Rc::downgrade(self);
            let id = id.to_string();
            agent_picker::load_agents(move |r| {
                let Some(c) = w.upgrade().filter(|c| *c.id.borrow() == id) else { return };
                if let Some(a) = r.ok().into_iter().flatten().find(|a| a["id"] == k["id"]) {
                    c.set_agent(Some(a));
                    // (the input the last message took, ready to change)
                    if let Some(last) = eventos(&id).iter().rev().find(|e| e["tipo"] == "pessoa" && e["input"].is_object()) {
                        c.input_form.fill(&last["input"]);
                    }
                }
            });
        }
        self.update_time();
        self.scroll_to_end();
    }

    /// A new, empty conversation.
    pub fn nova(self: &Rc<Self>) {
        self.new_conversation();
    }

    /// The tool groups this conversation turns off (kept for its engine; sent now when it runs).
    pub fn set_desligadas(&self, v: Vec<String>) {
        if let Some(e) = self.engine.borrow_mut().as_mut() {
            let _ = writeln!(e.stdin, "{}", json!({"desligadas": v})).and_then(|_| e.stdin.flush());
        }
        *self.desligadas.borrow_mut() = v;
    }

    pub fn desligadas(&self) -> Vec<String> {
        self.desligadas.borrow().clone()
    }

    /// In the agents center: no agent header (the messages carry their own), wider measures.
    pub fn modo_app(&self) {
        self.cabecalho.set_visible(false);
        self.raiz.add_css_class("conversa-app");
        self.agent_row.set_visible(true);
        self.show_agent_chip();
    }

    /// Adds text to what is being written (e.g. a file the person picked as context).
    pub fn inserir(&self, t: &str) {
        let b = self.texto.buffer();
        let mut fim = b.end_iter();
        b.insert(&mut fim, t);
        self.focus();
    }

    /// While the agent works: when the request started, the context in use (tokens) and whether it is paused.
    pub fn work(&self) -> Option<(Instant, u64, bool)> {
        if !self.busy.get() {
            return None;
        }
        let started = self.turn.borrow().as_ref().map(|t| t.started)?;
        Some((started, self.tokens.get(), self.paused.get()))
    }

    /// The last steps of the request in progress, as the desktop shows them: (mark, text). ✓ done, ◌ running,
    /// ! waiting for the person, ✗ failed.
    pub fn passos(&self, n: usize) -> Vec<(char, String, i64)> {
        let tb = self.turn.borrow();
        let Some(t) = tb.as_ref() else { return vec![] };
        t.steps.marks(n)
    }

    /// Freezes or resumes the engine process: the step in progress waits where it is (the model may finish the
    /// reply it was writing; nothing else happens until it resumes).
    pub fn pause(&self, on: bool) {
        if let Some(e) = self.engine.borrow().as_ref() {
            crate::sistema::pausar_processo(e.child.id() as i32, on);
            self.paused.set(on);
        }
    }

    /// Stops the running request (the engine process ends; the next message starts a new one).
    pub(crate) fn stop(self: &Rc<Self>, by_person: bool) {
        if !self.busy.get() {
            return;
        }
        self.kill_engine();
        // (a request to kalango-fluxos: its answer is dropped and its run cancelled)
        self.client_ref.borrow_mut().clear();
        if let Some(r) = self.run_id.borrow_mut().take() {
            crate::fluxos::call("runs.cancel", json!({"id": r}), |_| {});
        }
        if let Some(t) = self.turn.borrow_mut().as_mut() {
            t.desfecho.get_or_insert(("parado", if by_person { tr("stopped by you").to_string() } else { String::new() }));
        }
        if by_person {
            self.note(tr("Stopped by you. Actions already done stay in the timeline."));
        }
        self.finish_turn();
    }

    fn new_conversation(self: &Rc<Self>) {
        self.kill_engine();
        self.client_ref.borrow_mut().clear();
        self.run_id.borrow_mut().take();
        *self.turn_agent.borrow_mut() = None;
        if self.agent.borrow().is_some() {
            self.set_agent(None);
        }
        *self.id.borrow_mut() = novo_id();
        *self.turn.borrow_mut() = None;
        self.set_busy(false);
        while let Some(w) = self.lista.last_child() {
            if w == self.intro.clone().upcast::<gtk::Widget>() {
                break;
            }
            self.lista.remove(&w);
        }
        self.turns.set(0);
        self.last_message.set(0);
        self.update_time();
        self.intro.set_visible(true);
        self.focus();
    }

    // ---------------------------------------------------------------------------------------------- the person's agents

    /// Whether the person's agents are called here (the agents center, or the side bar).
    fn agents_on(&self) -> bool {
        self.side.get() || self.raiz.has_css_class("conversa-app")
    }

    /// The picker "@nome" opens: the side bar's suggestions, or the agents center's picker.
    fn mention_picker(&self) -> Option<Rc<Picker>> {
        if self.side.get() { self.suggest.borrow().clone() } else { self.picker.borrow().clone() }
    }

    /// (side bar) The "@nome" suggestions over the composer (made once): the agent chosen is written in the text,
    /// and that message goes to it; a flow runs.
    fn suggest(self: &Rc<Self>) -> Rc<Picker> {
        if let Some(p) = self.suggest.borrow().as_ref() {
            return p.clone();
        }
        let w = Rc::downgrade(self);
        let p = Picker::with_style(agent_picker::Style::Suggest, move |x| {
            let Some(c) = w.upgrade() else { return };
            c.mention_mode.set(false);
            let b = c.texto.buffer();
            let t = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            let before = agent_picker::strip_mention_query(&t);
            match x {
                Picked::Agent(Some(a)) => {
                    let name = a["name"].as_str().unwrap_or("");
                    b.set_text(&format!("{before}{}@{name} ", if before.is_empty() { "" } else { " " }));
                }
                other => {
                    b.set_text(&before);
                    c.picked(other);
                }
            }
            c.texto.grab_focus();
        });
        p.popover.set_position(gtk::PositionType::Top);
        *self.suggest.borrow_mut() = Some(p.clone());
        p
    }

    /// The side bar's conversation: the header's chip is the agent's, "@nome" calls an agent for one message and
    /// every answer says who gave it.
    pub fn modo_lateral(self: &Rc<Self>) {
        self.side.set(true);
        self.head_tile.set_visible(false);
        self.head_name.set_visible(false);
        self.head_chip.set_visible(true);
        // (no chip in the composer: only the fields of an agent that asks for them)
        self.agent_chip.set_visible(false);
        self.show_agent_chip();
    }

    /// The picker under the composer's chip, or the header's in the side bar (made once).
    fn picker(self: &Rc<Self>) -> Rc<Picker> {
        if let Some(p) = self.picker.borrow().as_ref() {
            return p.clone();
        }
        let w = Rc::downgrade(self);
        let style = if self.side.get() { agent_picker::Style::Narrow } else { agent_picker::Style::Full };
        let p = Picker::with_style(style, move |x| {
            let Some(c) = w.upgrade() else { return };
            if c.mention_mode.replace(false) {
                let b = c.texto.buffer();
                let t = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                b.set_text(&agent_picker::strip_mention_query(&t));
            }
            c.picked(x);
            c.texto.grab_focus();
        });
        *self.picker.borrow_mut() = Some(p.clone());
        p
    }

    fn picked(self: &Rc<Self>, x: Picked) {
        match x {
            Picked::Agent(a) => self.set_agent(a),
            other => {
                if let Some(f) = self.on_picked_other.borrow().as_ref() {
                    f(other);
                }
            }
        }
    }

    fn close_picker(&self) {
        self.mention_mode.set(false);
        for p in [&self.picker, &self.suggest] {
            if let Some(p) = p.borrow().as_ref() {
                p.close();
            }
        }
    }

    /// "@query" typed at the end of the composer opens the picker on it (the text keeps the focus).
    fn follow_mention(self: &Rc<Self>, text: &str) {
        if !self.agents_on() {
            return;
        }
        // (a name already chosen, followed by a space, is not being typed any more)
        let chosen = self.side.get()
            && agent_picker::mention_query(text).is_some_and(|q| q.ends_with(' ') && agent_picker::cached_agents().iter().any(|a| a["name"].as_str().is_some_and(|n| crate::fluxos_ui::fold(n) == crate::fluxos_ui::fold(q.trim_end()))));
        match agent_picker::mention_query(text).filter(|_| !chosen) {
            Some(q) => {
                let p = if self.side.get() { self.suggest() } else { self.picker() };
                if p.is_open() && self.mention_mode.get() {
                    p.set_query(&q);
                    if !q.is_empty() && !p.has_matches(&q) {
                        self.close_picker();
                    }
                } else if !p.is_open() && (q.is_empty() || p.has_matches(&q) || agent_picker::cached_agents().is_empty()) {
                    self.mention_mode.set(true);
                    p.popover.set_autohide(false);
                    let cur = self.agent.borrow().as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
                    if self.side.get() {
                        // (over the composer, from its left edge)
                        p.popover.set_pointing_to(Some(&gdk::Rectangle::new(290 / 2 + 8, 0, 1, 1)));
                        p.open(&self.texto, cur.as_deref(), &q, false);
                    } else {
                        p.open(&self.agent_chip, cur.as_deref(), &q, false);
                    }
                }
            }
            None if self.mention_mode.get() => self.close_picker(),
            None => {}
        }
    }

    /// The agent this conversation talks to from now on (None: the system's own).
    pub fn set_agent(self: &Rc<Self>, agent: Option<Value>) {
        let agent = agent.filter(|a| a["id"].is_string() && !agent_picker::is_flow(a));
        let same = self.agent.borrow().as_ref().and_then(|a| a["id"].as_str().map(str::to_string)) == agent.as_ref().and_then(|a| a["id"].as_str().map(str::to_string));
        let fields = agent.as_ref().map(|a| agent_picker::fields(&a["input_fields"])).unwrap_or_default();
        *self.agent.borrow_mut() = agent;
        if !same || fields.is_empty() != self.input_form.is_empty() {
            self.input_form.set_fields(fields);
        }
        crate::fluxos_ui::show_error(&self.input_error, "");
        self.show_agent_chip();
        if let Some(f) = self.on_agent.borrow().as_ref() {
            f(self.agent.borrow().as_ref());
        }
    }

    pub fn agent(&self) -> Option<Value> {
        self.agent.borrow().clone()
    }

    pub fn on_agent(&self, f: impl Fn(Option<&Value>) + 'static) {
        *self.on_agent.borrow_mut() = Some(Box::new(f));
    }

    fn show_agent_chip(&self) {
        let a = self.agent.borrow().clone();
        let b = caixa(Orientation::Horizontal, 7, "");
        b.append(&agent_picker::tile_for(a.as_ref(), 20));
        let name = a.as_ref().and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| agent_picker::SYSTEM_AGENT.to_string());
        let l = rotulo(&name, "agente-chip-nome");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(22);
        b.append(&l);
        b.append(&ui_icon("chevron-baixo", MUTED, 12));
        self.agent_chip.set_child(Some(&b));
        // (side bar) the header's chip
        let h = caixa(Orientation::Horizontal, 6, "");
        h.append(&agent_picker::tile_for(a.as_ref(), 18));
        let l = rotulo(&name, "agente-chip-nome");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(18);
        h.append(&l);
        h.append(&ui_icon("chevron-baixo", MUTED, 12));
        self.head_chip.set_child(Some(&h));
        if self.side.get() {
            self.placeholder.set_text(if a.is_some() { tr("Ask the agent · @ calls another") } else { tr("Ask anything · / commands · @ mention") });
            self.agent_row.set_visible(!self.input_form.is_empty());
        }
        if self.raiz.has_css_class("conversa-app") {
            self.placeholder.set_text(if a.is_some() { tr("Ask the agent · @ calls another") } else { tr("Ask anything · / commands · @ calls an agent") });
        }
        crate::central::posicao_debug("conversa chip agente", self.agent_chip.upcast_ref());
    }

    /// A request to one of the person's agents (kalango-fluxos `chat.send`): its input checked first, the steps live,
    /// approvals inline, then the answer with the same blocks as the system's agent.
    /// `shown` is the person's text as it shows (with an "@nome" that called the agent `once`, for this message only:
    /// the conversation's own agent and fields stay).
    fn send_platform(self: &Rc<Self>, shown: &str, text: &str, agent: Value, once: bool) {
        let fields = if once { Vec::new() } else { agent_picker::fields(&agent["input_fields"]) };
        let resend = self.retry_input.borrow_mut().take();
        let input = if let Some(i) = resend {
            i
        } else if once || self.input_form.is_empty() {
            None
        } else {
            match self.input_form.value() {
                Ok(v) => Some(v),
                Err(e) => {
                    crate::fluxos_ui::show_error(&self.input_error, &e);
                    let b = self.texto.buffer();
                    if b.char_count() == 0 {
                        b.set_text(text);
                    }
                    return;
                }
            }
        };
        crate::fluxos_ui::show_error(&self.input_error, "");
        *self.last_send.borrow_mut() = Some(LastSend::Platform { shown: shown.to_string(), text: text.to_string(), agent: agent.clone(), once, input: input.clone() });
        self.intro.set_visible(false);
        self.turns.set(self.turns.get() + 1);
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        self.last_message.set(agora);
        self.update_time();
        let summary = input.as_ref().map(|i| agent_picker::input_summary(&fields, i)).unwrap_or_default();
        let anexos: Vec<PathBuf> = self.anexos.borrow_mut().drain(..).collect();
        let voz: Vec<PathBuf> = self.voz_anexos.borrow_mut().drain(..).collect();
        // pictures and sounds go to the service as such: it reads them with LFM3-VL / LFM3-Audio before the agent
        // answers ("Automático pela entrada"); before, only their paths went, and the text model guessed from the
        // file's name (a voice message's recording is its words already)
        let media = media_attachments(&anexos, &voz);
        while let Some(ch) = self.anexos_box.first_child() {
            self.anexos_box.remove(&ch);
        }
        self.anexos_box.set_visible(false);
        let aref = agent_picker::agent_ref(&agent);
        self.registrar(&json!({"tipo": "pessoa", "texto": shown, "anexos": anexos.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), "agent": aref, "input": input, "input_summary": summary, "fields": agent["input_fields"], "once": once}));
        let id = self.id.borrow().clone();
        let mut m = meta(&id);
        if m["titulo"].as_str().is_none_or(|t| t.is_empty()) {
            let first = if shown.trim().is_empty() { summary.clone() } else { shown.to_string() };
            m["titulo"] = json!(first.lines().next().unwrap_or("").chars().take(60).collect::<String>());
            m["criado"] = json!(agora);
        }
        m["atualizado"] = json!(agora);
        if !once {
            m["agent"] = aref;
        }
        m["session"] = json!(id);
        salvar_meta(&id, &m);
        self.person_bubble_with_input(shown, &anexos, &fields, input.as_ref());
        // (the fields keep their values for the next message, as the design shows)
        let message = if anexos.is_empty() {
            text.to_string()
        } else {
            format!("{text}\n\n{} {}", tr("Attached files:"), anexos.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
        };
        *self.current.borrow_mut() = if text.trim().is_empty() { summary } else { text.to_string() };
        *self.turn_agent.borrow_mut() = Some(agent.clone());
        let turn = self.new_turn();
        *self.turn.borrow_mut() = Some(turn);
        self.set_busy(true);
        self.scroll_to_end();
        let reference = novo_id();
        *self.client_ref.borrow_mut() = reference.clone();
        *self.run_id.borrow_mut() = None;
        let started = Instant::now();
        self.platform_started.set(Some(started));
        self.arrivals.borrow_mut().clear();
        // (the members' and tasks' names: the agents' list, read now if it is old; then the first task's line)
        let steps = self.turn.borrow().as_ref().map(|t| Rc::downgrade(&t.steps));
        let tasks = (agent["kind"] == "tasks").then(|| agent["config"]["tasks"].clone());
        agent_picker::load_agents(move |_| {
            let first = tasks.and_then(|t| crate::chat_blocks::next_task_text(&t, 0, &crate::chat_blocks::agent_names()));
            if let (Some(s), Some(first)) = (steps.and_then(|s| s.upgrade()), first) {
                if s.is_empty() {
                    s.show_thinking(&first);
                }
            }
        });
        // its run's events: the steps as they happen, the approvals it asks for, who answered, a coordinator's route
        let w = Rc::downgrade(self);
        let r2 = reference.clone();
        crate::fluxos::subscribe(&["chat.", "run.", "route."], move |ev| {
            let Some(c) = w.upgrade() else { return false };
            if *c.client_ref.borrow() != r2 {
                return false;
            }
            c.platform_event(&ev);
            true
        });
        let mut params = json!({"agent_id": agent["id"], "message": message, "input": input, "session": id, "client_ref": reference});
        if !media.is_empty() {
            params["attachments"] = json!(media);
        }
        let w = Rc::downgrade(self);
        // (a moment for the subscription to be in place: "chat.started" names the run the steps come from)
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            crate::fluxos::call("chat.send", params, move |r| {
                if let Some(c) = w.upgrade() {
                    c.platform_result(&reference, r, &agent, started.elapsed());
                }
            });
        });
    }

    /// An event of kalango-fluxos while a request of this conversation runs.
    fn platform_event(self: &Rc<Self>, ev: &Value) {
        let topic = ev["topic"].as_str().unwrap_or("");
        let d = &ev["data"];
        if topic == "chat.started" {
            if d["client_ref"].as_str() == Some(self.client_ref.borrow().as_str()) {
                *self.run_id.borrow_mut() = d["run_id"].as_str().map(str::to_string);
            }
            return;
        }
        // a coordinator's route (the event names only the agent): the line above its steps
        if topic == "route.decided" {
            let mine = self.turn_agent.borrow().as_ref().is_some_and(|a| a["id"].as_str().is_some_and(|id| d["agent_id"].as_str() == Some(id)));
            let busy = !self.client_ref.borrow().is_empty();
            if mine && busy && d["client_ref"].as_str().is_none_or(|r| r == self.client_ref.borrow().as_str()) {
                let threshold = self.turn_agent.borrow().as_ref().map(|a| crate::chat_blocks::route_threshold(&a["config"])).unwrap_or(0.7);
                if let Some(l) = crate::chat_blocks::route_line(&json!({"agent": d["choice"], "confidence": d["confidence"]}), &crate::chat_blocks::agent_names(), threshold) {
                    self.not_kept.set(true);
                    self.handle(&json!({"tipo": "rota", "texto": l}));
                    self.not_kept.set(false);
                }
            }
            return;
        }
        let ours = self.run_id.borrow().as_deref().is_some_and(|r| d["run_id"].as_str() == Some(r))
            || (self.run_id.borrow().is_none() && d["session"].as_str() == Some(self.id.borrow().as_str()));
        if !ours {
            return;
        }
        match topic {
            "run.tool" => {
                let tool = d["tool"].as_str().unwrap_or("").replace("__", ".");
                let args: Value = d["args"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or_else(|| d["args"].clone());
                // (the service tells when a tool starts, not when it ends: a new one means the one still running is
                // over — it kept "Olhando uma pasta…" spinning while the next asked for approval)
                if let Some(t) = self.turn.borrow().as_ref() {
                    t.steps.settle_running();
                }
                self.not_kept.set(true);
                self.handle(&json!({"tipo": "ferramenta", "nome": tool, "descricao": tool, "argumentos": args}));
                self.not_kept.set(false);
            }
            "run.waiting" => {
                let sid = self.remote_seq.get() + 1;
                self.remote_seq.set(sid);
                self.remote_approvals.borrow_mut().insert(sid, (d["approval_id"].as_str().unwrap_or("").to_string(), d["tool"].as_str().unwrap_or("").replace("__", ".")));
                let details = d["details"].as_str().and_then(|t| serde_json::from_str::<Value>(t).ok()).or_else(|| d["details"].as_object().map(|_| d["details"].clone()));
                // (the service's details are {tool, arguments}: the card shows the arguments, as the system's does)
                let args = match details {
                    Some(v) if v["arguments"].is_object() => v["arguments"].clone(),
                    Some(v) if v.is_object() => v,
                    _ => json!({"detalhes": d["details"]}),
                };
                let tool = d["tool"].as_str().unwrap_or("").replace("__", ".");
                // ("Use X?" says nothing the tool's name does not)
                let question = d["question"].as_str().filter(|q| !q.is_empty() && !q.starts_with("Use ")).unwrap_or(&tool);
                self.handle(&json!({"tipo": "pedido", "id": sid, "nome": tool, "descricao": question, "argumentos": args, "agente": self.turn_agent.borrow().as_ref().and_then(|a| a["name"].as_str().map(str::to_string))}));
            }
            "run.approval_failed" => {
                let tool = d["tool"].as_str().unwrap_or("").replace("__", ".");
                self.handle(&json!({"tipo": "aviso", "texto": trf("{tool}: could not ask for your approval ({error}). The agent stopped.", &[("tool", tool), ("error", d["error"].as_str().unwrap_or("").to_string())])}));
            }
            "run.answer" => {
                // a member of a team or a task of a list finished: its row, live ("Tarefa 1 · Buscador · 3,1 s")
                let author = d["author"].as_str().unwrap_or("").to_string();
                let at = self.platform_started.get().map(|s| s.elapsed()).unwrap_or_default();
                let prev = self.arrivals.borrow().last().map(|x| x.1).unwrap_or_default();
                self.arrivals.borrow_mut().push((author.clone(), at));
                let agent = self.turn_agent.borrow().clone().unwrap_or_default();
                let kind = agent["kind"].as_str().unwrap_or("assistant").to_string();
                let took = if kind == "parallel" { at } else { at.saturating_sub(prev) };
                let names = crate::chat_blocks::agent_names();
                let rows = crate::chat_blocks::member_calls(&[(author.clone(), d["text"].as_str().unwrap_or("").to_string(), Some(took))], agent["id"].as_str().unwrap_or(""), &kind, &names);
                // (the member answered: the tools it used are over)
                if let Some(t) = self.turn.borrow().as_ref() {
                    t.steps.settle_running();
                }
                self.not_kept.set(true);
                for c in rows {
                    self.handle(&member_event(&c));
                }
                self.not_kept.set(false);
                let next = crate::chat_blocks::task_of(&author).and_then(|(_, n)| crate::chat_blocks::next_task_text(&agent["config"]["tasks"], n as usize, &names));
                let steps = self.turn.borrow().as_ref().map(|t| t.steps.clone());
                if let (Some(s), Some(next)) = (steps, next) {
                    s.show_thinking(&next);
                }
            }
            "run.decided" => {
                // (decided: the step waiting for the person runs, or failed; the agent goes on thinking)
                if let Some(t) = self.turn.borrow().as_ref().filter(|t| !t.steps.is_empty()) {
                    t.steps.decide_waiting(d["decision"].as_bool().unwrap_or(true));
                    t.steps.show_thinking(tr("Thinking…"));
                }
            }
            _ => {}
        }
    }

    /// The answer of kalango-fluxos: the tools it used, the structured output, the text and the footer.
    fn platform_result(self: &Rc<Self>, reference: &str, r: Result<Value, String>, agent: &Value, took: Duration) {
        if *self.client_ref.borrow() != reference {
            return;
        }
        self.client_ref.borrow_mut().clear();
        self.run_id.borrow_mut().take();
        let v = match r {
            Ok(v) => v,
            Err(e) => {
                self.handle(&json!({"tipo": "erro", "texto": e}));
                return;
            }
        };
        // the live steps go, the whole ones (with how each ended) are kept; with no list from the service, the live
        // ones are kept as they ran (a denied one ends as denied)
        let denied: Vec<String> = self.remote_denied.borrow_mut().drain(..).collect();
        let live: Vec<(String, String)> =
            self.turn.borrow().as_ref().map(|t| t.steps.clear().into_iter().filter(|c| c.member.is_none()).map(|c| (c.tool, c.args_json)).collect()).unwrap_or_default();
        // (the whole steps take the live ones' place at once: they already came in)
        if let Some(t) = self.turn.borrow().as_ref() {
            t.steps.set_instant(true);
        }
        // a coordinator's route, then who answered (each member or task, with how long it took), then the tools
        let names = crate::chat_blocks::agent_names();
        if let Some(l) = crate::chat_blocks::route_line(&v["route"], &names, crate::chat_blocks::route_threshold(&agent["config"])) {
            self.handle(&json!({"tipo": "rota", "texto": l}));
        }
        let answers: Vec<(String, String)> =
            v["answers"].as_array().into_iter().flatten().map(|a| (a["author"].as_str().unwrap_or("").to_string(), a["text"].as_str().unwrap_or("").to_string())).collect();
        let kind = agent["kind"].as_str().unwrap_or("assistant");
        let times = crate::chat_blocks::answer_times(kind, &self.arrivals.borrow_mut().drain(..).collect::<Vec<_>>(), &answers, took);
        let timed: Vec<(String, String, Option<Duration>)> = answers.into_iter().zip(times).map(|((a, t), x)| (a, t, x)).collect();
        for c in crate::chat_blocks::member_calls(&timed, agent["id"].as_str().unwrap_or(""), kind, &names) {
            self.handle(&member_event(&c));
        }
        let mut tools: Vec<(String, bool, String, Value)> = v["tools"].as_array().into_iter().flatten().map(|tc| (tc[0].as_str().unwrap_or("").replace("__", "."), tc[1].as_bool().unwrap_or(false), tc[2].as_str().unwrap_or("").to_string(), json!({}))).collect();
        if tools.is_empty() {
            tools = live.into_iter().map(|(name, args)| {
                let no = denied.contains(&name);
                (name, !no, if no { tr("denied by you").to_string() } else { String::new() }, serde_json::from_str(&args).unwrap_or(json!({})))
            }).collect();
        }
        for (name, ok, summary, args) in tools {
            self.handle(&json!({"tipo": "ferramenta", "nome": name, "descricao": name, "argumentos": args}));
            // (a failure without a reason keeps the tool's name as its title)
            let texto = if summary.is_empty() && !ok { name.clone() } else { summary.clone() };
            self.handle(&json!({"tipo": "resultado", "ok": ok, "texto": texto, "resumo": "", "detalhe": summary}));
        }
        if let Some(t) = self.turn.borrow().as_ref() {
            t.steps.set_instant(self.replaying.get());
        }
        let output = v["output"].clone();
        let has_output = output.is_object();
        if has_output {
            self.handle(&json!({"tipo": "saida", "valor": output, "campos": agent["output_fields"]}));
        }
        let text = v["text"].as_str().unwrap_or("").trim().to_string();
        let text_is_output = has_output && serde_json::from_str::<Value>(text.trim_start_matches("```json").trim_end_matches("```").trim()).ok().as_ref() == Some(&output);
        if !text.is_empty() && !text_is_output {
            self.handle(&json!({"tipo": "resposta", "texto": text}));
        }
        if let Some(why) = v["stopped"].as_str().filter(|s| !s.is_empty()) {
            self.handle(&json!({"tipo": "aviso", "texto": trf("The agent stopped: {why}", &[("why", why.to_string())])}));
        }
        let models: Vec<String> = v["models"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        self.handle(&json!({"tipo": "rodape", "modelo": self.modelo.text().as_str(), "segundos": took.as_secs_f64(), "modelos": models}));
        self.handle(&json!({"tipo": "fim"}));
    }

    /// The person's message to one of their agents: the input as a chip above ("Entrada · pasta, dias") and its
    /// values on the first line.
    fn person_bubble_with_input(&self, text: &str, anexos: &[PathBuf], fields: &[agent_picker::Field], input: Option<&Value>) {
        let Some(i) = input.filter(|i| i.as_object().is_some_and(|o| !o.is_empty())) else {
            return self.bolha_pessoa_com(text, anexos);
        };
        let names: Vec<String> = fields.iter().filter(|f| i.get(&f.name).is_some()).map(|f| f.name.clone()).collect();
        let chip = caixa(Orientation::Horizontal, 6, "entrada-chip");
        chip.append(&ui_icon("codigo", MUTED, 12));
        chip.append(&rotulo(&format!("{} · {}", tr("Input"), names.join(", ")), ""));
        chip.set_halign(Align::End);
        self.lista.append(&chip);
        let summary = agent_picker::input_summary(fields, i);
        let full = if text.trim().is_empty() { summary } else { format!("{summary}\n{text}") };
        self.bolha_pessoa_com(&full, anexos);
    }

    /// The footer under an answer of one of the person's agents: model, where it ran and how long it took.
    fn answer_footer(&self, v: &Value) {
        let mut parts = vec![v["modelo"].as_str().unwrap_or("").to_string()];
        for m in v["modelos"].as_array().into_iter().flatten().filter_map(|m| m.as_str()) {
            parts.push(m.to_string());
        }
        parts.retain(|p| !p.is_empty());
        if let Some(sec) = v["segundos"].as_f64() {
            parts.push(elapsed(Duration::from_secs_f64(sec.max(0.0))));
        }
        let l = rotulo(&parts.join(" · "), "resposta-rodape");
        l.set_xalign(1.0);
        l.set_halign(Align::End);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        self.put(&l);
    }

    // ---------------------------------------------------------------------------------------------- permissions

    /// Sends the person's answer to a tool that asks first.
    fn answer(&self, id: u64, answer: &str) {
        crate::system_sounds::play_agent(if answer == "negar" { "kalango-agent-denied" } else { "kalango-agent-approved" });
        // (an approval of kalango-fluxos is decided there)
        let remote = self.remote_approvals.borrow_mut().remove(&id);
        if let Some((approval, tool)) = remote {
            if answer == "negar" {
                self.remote_denied.borrow_mut().push(tool);
            }
            let decision = match answer {
                "negar" => "deny",
                "sempre" => "allow_always",
                _ => "allow_once",
            };
            crate::fluxos::call("runs.decide", json!({"approval_id": approval, "decision": decision}), |r| {
                if let Err(e) = r {
                    eprintln!("kalango-shell: runs.decide: {e}");
                }
            });
            return;
        }
        let mut eng = self.engine.borrow_mut();
        if let Some(e) = eng.as_mut() {
            let _ = writeln!(e.stdin, "{}", json!({"resposta": answer, "id": id})).and_then(|_| e.stdin.flush());
        }
    }

    /// Design "Permissão necessária": what the agent wants to do, its arguments, Deny / Allow once / Always.
    fn permission_card(self: &Rc<Self>, v: &Value) {
        let id = v["id"].as_u64().unwrap_or(0);
        let tool = v["nome"].as_str().unwrap_or("").to_string();
        // writing a file: what changes, as a diff to accept or reject (design "Chat · blocos de mensagem" › Diff)
        if tool == "arquivos.escrever" && self.diff_card(id, v) {
            return;
        }
        let verb = tool.rsplit('.').next().unwrap_or("");
        let high_risk = matches!(verb, "lixeira" | "apagar_criado" | "executar" | "remover");
        let c = caixa(Orientation::Vertical, 10, "cartao-permissao");
        let h = caixa(Orientation::Horizontal, 8, "");
        h.append(&ui_icon("escudo", GOLD, 16));
        let t = rotulo(tr("Permission needed"), "cartao-titulo");
        t.set_hexpand(true);
        t.set_xalign(0.0);
        h.append(&t);
        h.append(&rotulo(if high_risk { tr("High risk") } else { tr(approval_badge(&tool)) }, if high_risk { "selo-risco" } else { "selo-aviso" }));
        c.append(&h);
        let desc = v["descricao"].as_str().unwrap_or(&tool);
        let d = rotulo(&match v["agente"].as_str() {
            // (one of the person's agents: kalango-fluxos writes the whole sentence)
            Some(_) => first_upper(desc),
            None => trf("The agent wants to: {action}", &[("action", first_lower(desc))]),
        }, "cartao-texto");
        d.set_wrap(true);
        d.set_xalign(0.0);
        c.append(&d);
        // the exact arguments, as in the design's command box
        let lines: Vec<String> = v["argumentos"].as_object().map(|o| o.iter().map(|(k, x)| {
            let val = x.as_str().map(short_path).unwrap_or_else(|| short_path(&x.to_string()));
            format!("{k}: {val}")
        }).collect()).unwrap_or_default();
        if !lines.is_empty() {
            let code = rotulo(&lines.join("\n"), "cartao-codigo");
            code.set_wrap(true);
            code.set_wrap_mode(gtk::pango::WrapMode::Char);
            code.set_xalign(0.0);
            code.set_selectable(true);
            c.append(&code);
        }
        let scope = main_argument(&v["argumentos"]);
        let s = rotulo(&if scope.is_empty() { tr("Only this time").to_string() } else { trf("Scope: {scope} · only this time", &[("scope", scope)]) }, "cartao-rodape");
        s.set_xalign(0.0);
        s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        c.append(&s);
        let actions = caixa(Orientation::Horizontal, 8, "");
        actions.set_homogeneous(true);
        let result = rotulo("", "cartao-rodape");
        result.set_xalign(0.0);
        result.set_visible(false);
        for (label, kind, class) in [(tr("Deny"), "negar", "botao-negar"), (tr("Allow once"), "uma-vez", "botao-permitir"), (tr("Always"), "sempre", "botao-sempre")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class(class);
            let w = Rc::downgrade(self);
            let (row, result) = (actions.clone(), result.clone());
            let tool = tool.clone();
            if kind == "uma-vez" {
                self.pedidos.borrow_mut().insert(id, b.clone());
            }
            b.connect_clicked(move |_| {
                let Some(c) = w.upgrade() else { return };
                c.answer(id, kind);
                c.pedidos.borrow_mut().remove(&id);
                if let Some(f) = c.ao_pedir.borrow().as_ref() {
                    f(id, None);
                }
                row.set_visible(false);
                result.set_text(&match kind {
                    "negar" => tr("Denied: the agent did not do it.").to_string(),
                    "sempre" => trf("Allowed always for “{tool}” (you can change it in the permissions).", &[("tool", tool.clone())]),
                    _ => tr("Allowed this time.").to_string(),
                });
                result.set_visible(true);
            });
            actions.append(&b);
        }
        c.append(&actions);
        c.append(&result);
        self.put(&c);
        // nobody sees this conversation now: the island and a notice ask for it
        if !self.na_tela() {
            if let Some(f) = self.ao_pedir.borrow().as_ref() {
                f(id, Some((desc.to_string(), main_argument(&v["argumentos"]))));
            }
        }
    }

    /// The diff card for an approval to write a file (its content now against the new one); false when it cannot be
    /// shown that way (no text, a file too big to compare here) and the plain card asks instead.
    fn diff_card(self: &Rc<Self>, id: u64, v: &Value) -> bool {
        let (Some(f), Some(novo)) = (v["argumentos"]["caminho"].as_str(), v["argumentos"]["conteudo"].as_str()) else { return false };
        let caminho = match f.strip_prefix("~/") {
            Some(r) => glib::home_dir().join(r),
            None => PathBuf::from(f),
        };
        if std::fs::metadata(&caminho).is_ok_and(|m| m.len() > 2_000_000) {
            return false;
        }
        let antigo = match std::fs::read(&caminho) {
            Ok(b) => match String::from_utf8(b) {
                Ok(t) => t,
                Err(_) => return false,
            },
            Err(_) => String::new(),
        };
        let w = Rc::downgrade(self);
        let (card, aceitar) = crate::diff_card::card(&short_path(f), &antigo, novo, move |sim| {
            let Some(c) = w.upgrade() else { return };
            c.answer(id, if sim { "uma-vez" } else { "negar" });
            c.pedidos.borrow_mut().remove(&id);
            if let Some(f) = c.ao_pedir.borrow().as_ref() {
                f(id, None);
            };
        });
        // (for the session test: where "Aceitar" is once the conversation scrolled to it)
        let b = aceitar.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if let Some(b) = b.upgrade() {
                crate::central::posicao_debug("diff aceitar", b.upcast_ref());
            }
        });
        self.pedidos.borrow_mut().insert(id, aceitar);
        self.put(&card);
        if !self.na_tela() {
            if let Some(f) = self.ao_pedir.borrow().as_ref() {
                let desc = v["descricao"].as_str().unwrap_or("arquivos.escrever");
                f(id, Some((desc.to_string(), main_argument(&v["argumentos"]))));
            }
        }
        true
    }

    /// The agent needs the person (an approval, a question): a sound, if they want it (Agents › Settings ›
    /// Notifications).
    /// The agent needs the person: an approval (`kalango-agent-approval`) or a question (`kalango-agent-request`).
    fn chamar_atencao(&self, approval: bool) {
        if !self.replaying.get() {
            crate::system_sounds::play_agent(if approval { "kalango-agent-approval" } else { "kalango-agent-request" });
        }
    }

    /// The conversation is where the person looks: shown, and in the active window (a layer surface — the side
    /// bar — has no "active": shown is enough).
    fn na_tela(&self) -> bool {
        if let Some(f) = self.visivel.borrow().as_ref() {
            return f();
        }
        self.raiz.is_mapped() && self.raiz.root().and_downcast::<gtk::Window>().is_some_and(|w| w.is_active())
    }

    /// Follows the agent's events in this conversation (not the replay of a kept one).
    pub fn ao_evento(&self, f: impl Fn(&Value) + 'static) {
        self.ouvintes.borrow_mut().push(Box::new(f));
    }

    pub fn set_visivel(&self, f: impl Fn() -> bool + 'static) {
        *self.visivel.borrow_mut() = Some(Box::new(f));
    }

    pub fn ao_pedir(&self, f: impl Fn(u64, Option<(String, String)>) + 'static) {
        *self.ao_pedir.borrow_mut() = Some(Box::new(f));
    }

    /// Answers a waiting request "allow once" (the approval notice), as its own button.
    pub fn permitir_pedido(&self, id: u64) {
        let b = self.pedidos.borrow().get(&id).cloned();
        if let Some(b) = b {
            b.emit_clicked();
        }
    }

    /// The permissions list in the settings popover, grouped by area.
    fn fill_permissions(self: &Rc<Self>, pop: &gtk::Popover) {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug permissões abertas");
        }
        let wait = rotulo(tr("Loading…"), "fraco");
        pop.set_child(Some(&wait));
        let bin = agent_binary();
        let pop_w = pop.downgrade();
        crate::lateral::em_segundo_plano(move || {
            Command::new(&bin).arg("permissoes").output().ok().and_then(|o| serde_json::from_slice::<Vec<Value>>(&o.stdout).ok())
        }, move |list: Option<Vec<Value>>| {
            let Some(pop) = pop_w.upgrade() else { return };
            let Some(list) = list else {
                pop.set_child(Some(&rotulo(tr("kalangod is not responding."), "fraco")));
                return;
            };
            let col = caixa(Orientation::Vertical, 4, "");
            col.set_margin_start(6);
            col.set_margin_end(6);
            let t = rotulo(tr("Agent permissions"), "cartao-titulo");
            t.set_xalign(0.0);
            col.append(&t);
            let sub = rotulo(tr("Free: used without asking · Ask: asks in the conversation first · Never: not used. System changes always go through the approval screen."), "fraco");
            sub.set_wrap(true);
            sub.set_xalign(0.0);
            sub.set_max_width_chars(46);
            col.append(&sub);
            let mut last_area = String::new();
            for item in list.iter().filter(|x| !INTERNAL_TOOLS.contains(&x["nome"].as_str().unwrap_or(""))) {
                let name = item["nome"].as_str().unwrap_or("").to_string();
                let area = name.split('.').next().unwrap_or("").to_string();
                if area != last_area {
                    let a = rotulo(&area_name(&area).to_uppercase(), "caps");
                    a.set_xalign(0.0);
                    a.set_margin_top(8);
                    col.append(&a);
                    last_area = area;
                }
                let row = caixa(Orientation::Horizontal, 8, "permissao-linha");
                let d = rotulo(item["descricao"].as_str().unwrap_or(&name), "permissao-desc");
                d.set_xalign(0.0);
                d.set_hexpand(true);
                d.set_ellipsize(gtk::pango::EllipsizeMode::End);
                d.set_max_width_chars(30);
                d.set_tooltip_text(Some(&name));
                row.append(&d);
                let options = ["livre", "perguntar", "nunca"];
                let dd = gtk::DropDown::from_strings(&[tr("Free"), tr("Ask"), tr("Never")]);
                dd.add_css_class("permissao-escolha");
                let current = item["permissao"].as_str().unwrap_or("livre");
                dd.set_selected(options.iter().position(|o| *o == current).unwrap_or(0) as u32);
                let bin = agent_binary();
                dd.connect_selected_notify(move |dd| {
                    let p = options[(dd.selected() as usize).min(2)];
                    let _ = Command::new(&bin).args(["permissao", &name, p]).status();
                });
                row.append(&dd);
                col.append(&row);
            }
            let s = gtk::ScrolledWindow::new();
            s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            s.set_propagate_natural_height(true);
            s.set_max_content_height(440);
            s.set_min_content_width(340);
            s.set_child(Some(&col));
            pop.set_child(Some(&s));
        });
    }

    // ---------------------------------------------------------------------------------------------- voice

    fn start_recording(self: &Rc<Self>) {
        let rec = match voz::start("conversa") {
            Ok(r) => r,
            Err(e) => {
                self.voice_status(&trf("Could not record: {error}", &[("error", e)]), true);
                return;
            }
        };
        *self.recording.borrow_mut() = Some(rec);
        self.microfone.add_css_class("gravando");
        self.microfone.set_tooltip_text(Some(tr("Stop and transcribe")));
        self.voice_status("● 0:00", false);
        self.voz_estado.add_css_class("gravando");
        // the time recorded, and the automatic stop
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            let started = match c.recording.borrow().as_ref() {
                Some(r) => r.started,
                None => return glib::ControlFlow::Break,
            };
            let s = started.elapsed().as_secs();
            let time = format!("{}:{:02}", s / 60, s % 60);
            c.voz_estado.set_text(&format!("● {time}"));
            c.voz_estado.set_tooltip_text(Some(&trf("Recording {time}", &[("time", time)])));
            if started.elapsed() >= MAX_RECORDING {
                c.stop_recording();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn stop_recording(self: &Rc<Self>) {
        let Some(rec) = self.recording.borrow_mut().take() else { return };
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug voz: parando a gravação");
        }
        self.microfone.remove_css_class("gravando");
        self.microfone.set_tooltip_text(Some(tr("Dictate (the local voice model writes it here)")));
        self.voz_estado.remove_css_class("gravando");
        self.voice_status(tr("Transcribing…"), false);
        self.microfone.set_sensitive(false);
        // (with nothing written yet it is a voice message: the recording goes with it, kept with the conversation;
        // into a draft it is dictation, the words only)
        let b = self.texto.buffer();
        let guardar = (b.char_count() == 0).then(|| conversas_dir().join(self.id.borrow().as_str()));
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || voz::finish_keeping(rec, guardar), move |r: Result<(String, Option<PathBuf>), String>| {
            let Some(c) = w.upgrade() else { return };
            c.microfone.set_sensitive(true);
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug voz: {r:?}");
            }
            let r = r.map(|(t, arquivo)| {
                if let Some(p) = arquivo.filter(|_| !t.trim().is_empty()) {
                    c.voz_anexos.borrow_mut().push(p.clone());
                    c.add_attachment(p);
                }
                t
            });
            match r {
                Ok(t) if !t.trim().is_empty() => {
                    c.voz_estado.set_visible(false);
                    let buf = c.texto.buffer();
                    let mut end = buf.end_iter();
                    let sep = if buf.char_count() > 0 { " " } else { "" };
                    buf.insert(&mut end, &format!("{sep}{}", t.trim()));
                    c.focus();
                }
                Ok(_) => c.voice_status(tr("Nothing was heard."), true),
                Err(e) => c.voice_status(&trf("Could not transcribe: {error}", &[("error", e)]), true),
            }
        });
    }

    /// The voice line next to the composer buttons; errors go away after a few seconds.
    fn voice_status(&self, text: &str, transient: bool) {
        self.voz_estado.set_text(text);
        self.voz_estado.set_tooltip_text(Some(text));
        self.voz_estado.set_visible(true);
        if transient {
            let l = self.voz_estado.downgrade();
            let t = text.to_string();
            glib::timeout_add_local_once(Duration::from_secs(6), move || {
                if let Some(l) = l.upgrade() {
                    if l.text() == t {
                        l.set_visible(false);
                    }
                }
            });
        }
    }

    /// Adds a file to the message, from the system's chooser: a chip above the text (× takes it out).
    fn attach(self: &Rc<Self>) {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug anexar: abrindo o seletor de arquivo");
        }
        let w = Rc::downgrade(self);
        crate::arquivos::seletor::abrir_arquivo("", tr("Add a file to the message"), Some(tr("Add")), Vec::new(), None, move |p| {
            if let Some(c) = w.upgrade() {
                c.add_attachment(p);
                c.focus();
            }
        });
    }

    /// A file for the next message (from the chooser, or dropped on the composer).
    pub fn add_attachment(self: &Rc<Self>, p: PathBuf) {
        if self.anexos.borrow().contains(&p) {
            return;
        }
        self.anexos.borrow_mut().push(p.clone());
        let chip = attachment_chip(&p, true);
        let (w, pp) = (Rc::downgrade(self), p.clone());
        if let Some(x) = chip.last_child().and_downcast::<gtk::Button>() {
            x.connect_clicked(move |b| {
                let Some(c) = w.upgrade() else { return };
                c.anexos.borrow_mut().retain(|q| *q != pp);
                if let Some(ch) = b.parent() {
                    c.anexos_box.remove(&ch);
                }
                c.anexos_box.set_visible(!c.anexos.borrow().is_empty());
            });
        }
        self.anexos_box.append(&chip);
        self.anexos_box.set_visible(true);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug anexar: {} ({} na mensagem)", p.display(), self.anexos.borrow().len());
        }
    }

    // ---------------------------------------------------------------------------------------------- events

    fn handle(self: &Rc<Self>, v: &Value) {
        let kind = v["tipo"].as_str().unwrap_or("");
        let text = v["texto"].as_str().unwrap_or("");
        if !self.replaying.get() && self.na_tela() {
            if let Some(t) = self.turn.borrow_mut().as_mut() {
                t.seen = true;
            }
        }
        if !matches!(kind, "estado" | "pronto" | "saiu") {
            self.registrar(v);
        }
        if !self.replaying.get() {
            for f in self.ouvintes.borrow().iter() {
                f(v);
            }
        }
        match kind {
            // a kept conversation: an ask already answered is only said
            "pedido" if self.replaying.get() => {
                self.note(&trf("Asked for permission: {what}", &[("what", v["descricao"].as_str().unwrap_or(v["nome"].as_str().unwrap_or("")).to_string())]));
            }
            "pronto" => {
                if let Some(m) = v["modelo"].as_str() {
                    self.modelo.set_text(&format!("{} · {}", model_label(m), tr("local")));
                }
            }
            "estado" => {
                if let Some(t) = self.turn.borrow().as_ref() {
                    // before the first step the header says what is happening (thinking, starting the model);
                    // after, a running row at the end of the steps says it (design 53: "Escrevendo o resumo…")
                    if t.steps.is_empty() {
                        t.steps.set_status(&first_upper(text));
                    } else {
                        t.steps.show_thinking(&first_upper(text));
                    }
                }
            }
            "ferramenta" => self.step_start(v),
            "membro" => {
                if let Some(t) = self.turn.borrow().as_ref() {
                    let mut c = StepCall::new(crate::chat_blocks::MEMBER_TOOL, "", StepState::Done);
                    c.result = v["resumo"].as_str().unwrap_or("").to_string();
                    c.took = v["segundos"].as_f64().map(|s| Duration::from_secs_f64(s.max(0.0)));
                    c.member = Some(crate::chat_blocks::Member {
                        tag: v["marca"].as_str().unwrap_or("").to_string(),
                        name: v["nome"].as_str().unwrap_or("").to_string(),
                        text: v["texto"].as_str().unwrap_or("").to_string(),
                    });
                    t.steps.push(c);
                }
                self.scroll_to_end();
            }
            "rota" => {
                if let Some(t) = self.turn.borrow().as_ref() {
                    t.steps.set_route(text);
                }
            }
            "pedido" => {
                if let Some(t) = self.turn.borrow().as_ref() {
                    t.steps.wait_last();
                }
                self.chamar_atencao(true);
                self.permission_card(v)
            }
            "uso" => self.tokens.set(v["tokens"].as_u64().unwrap_or(0)),
            "resultado" => {
                self.step_done(v);
                self.web_result(v);
            }
            "plano" => {
                let steps: Vec<String> = v["passos"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
                let done = v["feitos"].as_u64().unwrap_or(0) as usize;
                self.show_plan(steps, done, !v["fim"].as_bool().unwrap_or(false));
            }
            "resposta" => {
                let w = markdown::render_agent(text);
                let a = Animavel::new(&w);
                if let Some(t) = self.turn.borrow_mut().as_mut() {
                    t.body.append(&a);
                    // under it: the model, how long and the tokens (design 53)
                    let tokens = self.tokens.get();
                    let took = if self.replaying.get() {
                        let (a, b) = self.replay_tempo.get();
                        Duration::from_secs((b - a).max(0) as u64)
                    } else {
                        t.started.elapsed()
                    };
                    // (an answer of one of the person's agents has its own footer: "rodape")
                    if self.turn_agent.borrow().is_none() {
                        let parts = [self.modelo.text().to_string(), crate::chat_blocks::seconds(took), if tokens > 0 { trf("{n} tokens", &[("n", thousands(tokens))]) } else { String::new() }];
                        t.body.append(&answer_footer(&parts));
                    }
                    t.answered = true;
                    t.answer.push_str(text);
                }
                a.entrar((0.0, 8.0), 1.0);
                self.sources_row();
                self.scroll_to_end();
            }
            "parado" => {
                if let Some(t) = self.turn.borrow_mut().as_mut() {
                    t.desfecho = Some(("falhou", v["motivo"].as_str().unwrap_or("").to_string()));
                }
                self.cartao_suspeito(v["motivo"].as_str().unwrap_or(""), v["trecho"].as_str().unwrap_or(""), v["fonte"].as_str().unwrap_or(""));
            }
            "saida" => {
                let fields = agent_picker::fields(&v["campos"]);
                if let Some(t) = agent_picker::output_table(&fields, &v["valor"]) {
                    if let Some(turn) = self.turn.borrow_mut().as_mut() {
                        turn.answered = true;
                        turn.answer.push_str(&agent_picker::output_rows(&fields, &v["valor"]).iter().map(|r| format!("{}: {}", r.0, r.2)).collect::<Vec<_>>().join("; "));
                    }
                    self.put(&t);
                }
            }
            "aviso" => self.note(text),
            "rodape" => self.answer_footer(v),
            "bloqueio" => {
                // (one sound a request, however many times the kernel's rules stop it)
                if !self.replaying.get() && self.turn.borrow_mut().as_mut().is_some_and(|t| !std::mem::replace(&mut t.failed_sound, true)) {
                    crate::system_sounds::play_agent("kalango-agent-failed");
                }
                self.cartao_bloqueio(v["alvo"].as_str().unwrap_or(""), v["regra"].as_str().unwrap_or(""))
            }
            "sem_rede" => self.cartao_sem_rede(v["url"].as_str().unwrap_or("")),
            "duvida" => {
                self.chamar_atencao(false);
                if let Some(t) = self.turn.borrow_mut().as_mut() {
                    t.answered = true;
                }
                let opcoes: Vec<String> = v["opcoes"].as_array().into_iter().flatten().filter_map(|o| o.as_str().map(str::to_string)).collect();
                self.cartao_duvida(v["pergunta"].as_str().unwrap_or(""), &opcoes);
            }
            "erro" => {
                if let Some(t) = self.turn.borrow_mut().as_mut() {
                    t.desfecho = Some(("falhou", text.to_string()));
                }
                self.cartao_erro(v["codigo"].as_str(), text);
                // an error before "pronto" ends the process; the next message starts again
                self.finish_turn();
            }
            "fim" => self.finish_turn(),
            "saiu" => {
                let was_busy = self.busy.get();
                self.engine.borrow_mut().take();
                if was_busy {
                    self.cartao_erro(None, tr("The agent stopped unexpectedly. Send the message again to restart it."));
                    self.finish_turn();
                }
            }
            _ => {}
        }
    }

    fn step_start(self: &Rc<Self>, v: &Value) {
        let steps = {
            let tb = self.turn.borrow();
            let Some(t) = tb.as_ref() else { return };
            let tool = v["nome"].as_str().unwrap_or("");
            // (a system-level tool asks first: it waits for the person)
            let needs_approval = matches!(v["nivel"].as_str().unwrap_or(""), "sistema" | "total");
            let mut call = StepCall::new(tool, &main_argument(&v["argumentos"]), if needs_approval { StepState::Waiting } else { StepState::Running });
            call.args_json = serde_json::to_string(&v["argumentos"]).unwrap_or_default();
            t.steps.push(call);
            t.steps.len()
        };
        self.tell_activity(Some(steps));
        self.scroll_to_end();
    }

    fn step_done(&self, v: &Value) {
        let tb = self.turn.borrow();
        let Some(t) = tb.as_ref() else { return };
        let Some(last) = t.steps.last_call() else { return };
        let ok = v["ok"].as_bool().unwrap_or(false);
        // a program the agent ran: its command and output in a terminal block (design "Chat · blocos de mensagem")
        if ok && matches!(last.tool.as_str(), "tarefas.executar" | "sistema.executar") {
            let args: Value = serde_json::from_str(&last.args_json).unwrap_or_default();
            if let Some(b) = crate::terminal_block::from_result(&args, v["texto"].as_str().unwrap_or("")) {
                t.body.append(&b);
            }
        }
        // what came back (its detail or summary), or — failed — the reason
        let result = if ok {
            short_path(v["detalhe"].as_str().or(v["resumo"].as_str()).unwrap_or(""))
        } else {
            first_upper(v["texto"].as_str().unwrap_or("").lines().next().unwrap_or(""))
        };
        t.steps.finish_last(ok, &result, None);
    }

    /// The plan at the top of the turn's answer, redrawn where it was (the spinner on the step in progress while
    /// `running`).
    fn show_plan(&self, steps: Vec<String>, done: usize, running: bool) {
        if steps.len() < 2 {
            return;
        }
        let mut turn = self.turn.borrow_mut();
        let Some(t) = turn.as_mut() else { return };
        let w = markdown::plan_live(&steps, done, running);
        match t.plan.take() {
            Some((old, _, _)) if old.parent().as_ref() == Some(t.body.upcast_ref()) => {
                let prev = old.prev_sibling();
                t.body.remove(&old);
                t.body.insert_child_after(&w, prev.as_ref());
            }
            _ => t.body.prepend(&w),
        }
        t.plan = Some((w, steps, done));
    }

    fn finish_turn(&self) {
        // (a plan still turning stops: the work ended)
        let parado = self.turn.borrow().as_ref().and_then(|t| t.plan.as_ref().map(|(_, s, d)| (s.clone(), *d)));
        if let Some((s, d)) = parado {
            self.show_plan(s, d, false);
        }
        for (nome, b) in self.posicoes.borrow_mut().drain(..) {
            if let Some(b) = b.upgrade() {
                let nome = format!("{nome} final");
                glib::timeout_add_local_once(Duration::from_millis(300), move || crate::central::posicao_debug(&nome, b.upcast_ref()));
            }
        }
        let fim = self.turn.borrow().as_ref().map(|t| {
            let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
            let (estado, motivo) = match &t.desfecho {
                Some((e, m)) => (*e, m.clone()),
                None if t.answered => ("concluido", String::new()),
                None => ("parado", String::new()),
            };
            serde_json::json!({
                "pedido": self.current.borrow().clone(), "inicio": agora - t.started.elapsed().as_secs() as i64, "fim": agora,
                "estado": estado, "motivo": motivo, "tokens": self.tokens.get(),
            })
        });
        let passos = self.passos(usize::MAX);
        // (a kept conversation being rebuilt was recorded when it happened)
        if let Some(mut r) = fim.filter(|_| !self.replaying.get()) {
            r["passos"] = serde_json::json!(passos.iter().map(|(m, t, h)| serde_json::json!({"marca": m.to_string(), "texto": t, "t": h})).collect::<Vec<_>>());
            registrar_tarefa(&r);
        }
        if let Some(t) = self.turn.borrow_mut().take() {
            let n = t.steps.len();
            if n == 0 {
                // no tool was used: the steps block has nothing to say
                t.steps_holder.set_visible(false);
            } else {
                let took = if self.replaying.get() {
                    let (a, b) = self.replay_tempo.get();
                    Duration::from_secs((b - a).max(0) as u64)
                } else {
                    t.started.elapsed()
                };
                // answered: the steps fold under "Trabalhou por…" (Agents › Settings › General › Show the steps open)
                t.steps.finish(took, t.answered && !crate::agent_prefs::load().steps_open);
            }
            // done out of sight — the person watched it and left, or it took long — a notice (Agents › Settings ›
            // Notifications › When a task finishes); a request made from another screen (the memory's field) is
            // answered where it was made
            let out_of_sight = !self.replaying.get() && !self.na_tela() && (t.seen || t.started.elapsed() >= Duration::from_secs(20));
            if !self.replaying.get() {
                if let Some(id) = turn_sound(t.desfecho.as_ref().map(|d| d.0), t.answered, t.failed_sound, out_of_sight) {
                    crate::system_sounds::play_agent(id);
                }
            }
            if t.answered && out_of_sight {
                let pedido: String = self.current.borrow().chars().take(60).collect();
                let resposta: String = t.answer.replace("**", "").replace('`', "").lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect();
                if let Some(s) = crate::shell_atual() {
                    s.notificacoes().notify_agent_task(&trf("The agent finished “{request}”", &[("request", pedido)]), &resposta);
                }
            }
        }
        self.set_busy(false);
        self.scroll_to_end();
    }

    // ---------------------------------------------------------------------------------------------- cards

    fn target(&self) -> gtk::Box {
        self.turn.borrow().as_ref().map(|t| t.body.clone()).unwrap_or_else(|| self.lista.clone())
    }

    fn put(&self, w: &impl IsA<gtk::Widget>) {
        let a = Animavel::new(w);
        self.target().append(&a);
        a.entrar((0.0, 8.0), 1.0);
        self.scroll_to_end();
    }

    /// The person's message (also for requests answered outside the engine, like "Resumir esta tela").
    pub(crate) fn bolha_pessoa(&self, text: &str) {
        self.bolha_pessoa_com(text, &[]);
    }

    /// The person's message, with the files that went with it as chips above it (at the right, as the bubble).
    fn bolha_pessoa_com(&self, text: &str, anexos: &[PathBuf]) {
        // pictures, sounds and videos as themselves (design "Chat · mídia"); other files as chips
        let imagens: Vec<PathBuf> = anexos.iter().filter(|p| crate::media_block::is_image(p)).cloned().collect();
        if !imagens.is_empty() {
            self.lista.append(&crate::media_block::images(&imagens));
        }
        for p in anexos.iter().filter(|p| crate::media_block::is_audio(p)) {
            let card = crate::media_block::audio(p);
            // (for the session test: the play button's place, again when the turn ends and the list has scrolled)
            if let Some(b) = descendant_with_class(&card, "midia-tocar").and_downcast::<gtk::Button>() {
                self.marcar_posicao("midia tocar", &b);
            }
            if let Some(b) = descendant_with_class(&card, "midia-transcrever").and_downcast::<gtk::Button>() {
                self.marcar_posicao("midia transcrever", &b);
            }
            self.lista.append(&card);
        }
        for p in anexos.iter().filter(|p| crate::media_block::is_video(p)) {
            let card = crate::media_block::video(p);
            for (class, nome) in [("midia-video-ir", "midia ir"), ("midia-video-cortar", "midia cortar")] {
                if let Some(b) = descendant_with_class(&card, class).and_downcast::<gtk::Button>() {
                    self.marcar_posicao(nome, &b);
                }
            }
            self.lista.append(&card);
        }
        let outros: Vec<&PathBuf> = anexos.iter().filter(|p| !crate::media_block::is_image(p) && !crate::media_block::is_audio(p) && !crate::media_block::is_video(p)).collect();
        if !outros.is_empty() {
            let fila = caixa(Orientation::Horizontal, 6, "");
            fila.set_halign(Align::End);
            for p in outros {
                fila.append(&attachment_chip(p, false));
            }
            self.lista.append(&fila);
        }
        self.intro.set_visible(false);
        let msg = person_message(text);
        let a = Animavel::new(&msg);
        self.lista.append(&a);
        a.entrar((0.0, 10.0), 1.0);
        self.scroll_to_end();
    }

    /// An answer that came from outside the engine (the vision model's description).
    pub(crate) fn resposta(&self, text: &str) {
        self.put(&markdown::render_agent(text));
    }

    pub(crate) fn nota(&self, text: &str) {
        self.note(text);
    }

    fn note(&self, text: &str) {
        let l = rotulo(text, "conversa-nota");
        l.set_wrap(true);
        l.set_xalign(0.0);
        self.put(&l);
    }

    /// The defence stopped the task (design: approval card family, coral).
    /// The failure card (design "Agente: erros, limites e bloqueios"): one template for every kind — what kind, what
    /// happened, what can be done, and what was kept.
    fn cartao_falha(&self, cor: &str, glifo: &str, tipo: &str, titulo: &str, texto: &str, rodape: &str) -> (gtk::Box, gtk::Box) {
        let c = caixa(Orientation::Vertical, 10, "falha");
        c.add_css_class(cor);
        let h = caixa(Orientation::Horizontal, 12, "");
        let ic = caixa(Orientation::Horizontal, 0, "falha-icone");
        ic.add_css_class(cor);
        ic.set_size_request(34, 34);
        ic.set_hexpand(false);
        ic.set_valign(Align::Start);
        let cor_hex = match cor { "coral" => CORAL, "gold" => GOLD, "azul" => "#7CC3DF", _ => ACCENT };
        let g = ui_icon(glifo, cor_hex, 16);
        g.set_hexpand(true);
        ic.append(&g);
        h.append(&ic);
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        let k = rotulo(tipo, "falha-tipo");
        k.add_css_class(cor);
        k.set_xalign(0.0);
        col.append(&k);
        let t = rotulo(titulo, "falha-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        col.append(&t);
        if !texto.is_empty() {
            let x = rotulo(texto, "falha-texto");
            x.set_xalign(0.0);
            x.set_wrap(true);
            x.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            x.set_selectable(true);
            x.set_margin_top(4);
            col.append(&x);
        }
        h.append(&col);
        c.append(&h);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        acoes.set_margin_start(46);
        c.append(&acoes);
        if !rodape.is_empty() {
            let r = caixa(Orientation::Horizontal, 6, "");
            r.set_margin_start(46);
            r.append(&ui_icon("check", MUTED, 13));
            let l = rotulo(rodape, "falha-rodape");
            l.set_xalign(0.0);
            l.set_wrap(true);
            r.append(&l);
            c.append(&r);
        }
        self.put(&c);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug conversa: falha {glifo}");
        }
        (c, acoes)
    }

    fn botao_falha(acoes: &gtk::Box, t: &str, classe: &str) -> gtk::Button {
        let b = gtk::Button::with_label(t);
        b.add_css_class("falha-botao");
        if !classe.is_empty() {
            b.add_css_class(classe);
        }
        acoes.append(&b);
        b
    }

    /// For the session test: where an action button is (now, and again when the turn ends).
    fn marcar_posicao(&self, nome: &str, b: &gtk::Button) {
        crate::central::posicao_debug(nome, b.upcast_ref());
        self.posicoes.borrow_mut().push((nome.to_string(), b.downgrade()));
    }

    /// The request of the turn that failed, again (after the fix), saying what changed: the conversation still holds
    /// the refusal, and the model would answer from it without trying.
    /// "Tentar de novo" on a failed step: the last request again, to the same agent, with the same message and input.
    fn retry(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let last = self.last_send.borrow().clone();
        match last {
            Some(LastSend::System(text)) => {
                // (to the system's agent, even if the conversation has one of the person's agents chosen now)
                let chosen = self.agent.replace(None);
                self.send(&text);
                *self.agent.borrow_mut() = chosen;
            }
            Some(LastSend::Platform { shown, text, agent, once, input }) => {
                *self.retry_input.borrow_mut() = Some(input);
                self.send_platform(&shown, &text, agent, once);
            }
            None => {}
        }
    }

    fn de_novo(self: &Rc<Self>, mudou: &str) {
        let p = self.current.borrow().clone();
        // (the request of a resend is the original one)
        let original = p.split_once(" — ").map(|(_, r)| r.to_string()).unwrap_or(p);
        if !original.is_empty() && !self.busy.get() {
            self.send(&format!("{mudou} — {original}"));
        }
    }

    /// Runs `kalango-agente ARGS` in the background, then (when it worked) sends the request again saying what changed.
    fn corrigir_e_repetir(self: &Rc<Self>, args: Vec<String>, b: &gtk::Button, mudou: String) {
        b.set_sensitive(false);
        let bin = agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || Command::new(&bin).args(&args).stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false), move |ok| {
            if let Some(c) = w.upgrade() {
                if ok {
                    c.de_novo(&mudou);
                } else {
                    c.note(tr("That did not work; see the agent's log."));
                }
            }
        });
    }

    fn cartao_erro(self: &Rc<Self>, codigo: Option<&str>, text: &str) {
        match codigo {
            Some("modelo") => {
                let (_, a) = self.cartao_falha("gold", "cpu", tr("MODEL UNAVAILABLE"), tr("The local model is not running"), text, tr("nothing was lost · it goes on from where it stopped"));
                let b = Self::botao_falha(&a, tr("Start the model"), "principal");
                let w = Rc::downgrade(self);
                b.connect_clicked(move |b| {
                    if let Some(c) = w.upgrade() {
                        c.corrigir_e_repetir(vec!["modelo".into()], b, tr("The model is running again; go on").to_string());
                    }
                });
            }
            Some("orcamento") => {
                let mut u = crate::widgets::ia::Uso::default();
                u.atualizar();
                let lim = crate::central_uso::limite_diario().unwrap_or(0);
                let usados = u.local + u.nuvem;
                let mil = |n: u64| if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() };
                let (_, a) = self.cartao_falha("gold", "raio", tr("BUDGET REACHED"), &trf("Today's limit ran out: {used} of {limit} tokens", &[("used", mil(usados)), ("limit", mil(lim))]),
                    tr("The agents wait. The limit comes back at midnight."), tr("the limit comes back at midnight"));
                let b = Self::botao_falha(&a, tr("Raise it only today"), "principal");
                self.marcar_posicao("aumentar so hoje", &b);
                let extra = (lim / 2).max(20_000);
                let w = Rc::downgrade(self);
                b.connect_clicked(move |b| {
                    if let Some(c) = w.upgrade() {
                        c.corrigir_e_repetir(vec!["limite".into(), "extra-hoje".into(), (usados.saturating_sub(lim) + extra).to_string()], b, tr("I raised today's limit; go on").to_string());
                    }
                });
            }
            Some("pausado") => {
                let (_, a) = self.cartao_falha("gold", "pausa", tr("AGENT PAUSED"), tr("The agent is paused"), tr("It does not start new work until it is resumed."), tr("nothing was done"));
                let b = Self::botao_falha(&a, tr("Resume the agent"), "principal");
                let w = Rc::downgrade(self);
                b.connect_clicked(move |b| {
                    if let Some(c) = w.upgrade() {
                        c.corrigir_e_repetir(vec!["pausar".into(), "nao".into()], b, tr("I resumed the agent; go on").to_string());
                    }
                });
            }
            Some("horario") => {
                let (_, a) = self.cartao_falha("gold", "relogio", tr("OUTSIDE ITS HOURS"), tr("The agent does not work at this hour"), text, tr("nothing was done"));
                let b = Self::botao_falha(&a, tr("Open the permissions"), "");
                b.connect_clicked(|_| {
                    if let Ok(me) = std::env::current_exe() {
                        let _ = Command::new(me).args(["central", "permissoes"]).spawn();
                    }
                });
            }
            _ => {
                self.cartao_falha("coral", "erro", tr("ERROR"), tr("Something went wrong"), text, "");
            }
        }
    }

    fn cartao_suspeito(self: &Rc<Self>, reason: &str, excerpt: &str, fonte: &str) {
        let pagina = fonte.starts_with("http");
        let titulo = if pagina { tr("A page tried to give orders to the agent") } else if fonte.starts_with('/') || fonte.starts_with('~') { tr("A file tried to give orders to the agent") } else { tr("Something read tried to steer an action") };
        let texto = format!("{}. {}", first_upper(reason), tr("The agent ignored it and stopped the task."));
        let (c, a) = self.cartao_falha("coral", "alerta", tr("SUSPICIOUS INSTRUCTION"), titulo, &texto, tr("outside content never becomes a command"));
        let r = gtk::Revealer::new();
        let e = rotulo(excerpt, "cartao-codigo");
        e.set_wrap(true);
        e.set_wrap_mode(gtk::pango::WrapMode::Char);
        e.set_xalign(0.0);
        e.set_selectable(true);
        e.set_margin_start(46);
        r.set_child(Some(&e));
        c.append(&r);
        if !excerpt.is_empty() {
            let b = Self::botao_falha(&a, tr("See the excerpt"), "");
            b.connect_clicked(move |_| r.set_reveal_child(!r.reveals_child()));
        }
        if pagina {
            let host = fonte.split('/').nth(2).unwrap_or("").trim_start_matches("www.").to_string();
            let b = Self::botao_falha(&a, tr("Block this site"), "perigo");
            let bin = agent_binary();
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let _ = Command::new(&bin).args(["bloquear-site", &host]).status();
                b.set_label(tr("Site blocked"));
            });
        }
    }

    /// A page the agent read: one of the answer's sources. One that asks for a login (401/403, even through the
    /// browser): a card — go on without it, or log in yourself in the agent's browser (design "Chat · navegador do
    /// agente").
    fn web_result(self: &Rc<Self>, v: &Value) {
        if v["nome"] != "web.ler" {
            return;
        }
        let ok = v["ok"].as_bool().unwrap_or(false);
        let texto = v["texto"].as_str().unwrap_or("");
        if ok {
            if let Ok(p) = serde_json::from_str::<Value>(texto) {
                if let Some(url) = p["url"].as_str() {
                    let mut f = self.fontes.borrow_mut();
                    if !f.iter().any(|(u, _)| u == url) {
                        f.push((url.to_string(), p["titulo"].as_str().unwrap_or("").to_string()));
                    }
                }
            }
            return;
        }
        if ![" 401", " 403"].iter().any(|c| texto.contains(c)) {
            return;
        }
        let url = self.turn.borrow().as_ref().and_then(|t| t.steps.last_call().and_then(|c| serde_json::from_str::<Value>(&c.args_json).ok()))
            .and_then(|a| a["url"].as_str().map(str::to_string)).unwrap_or_default();
        if url.is_empty() {
            return;
        }
        let site = url.split('/').nth(2).unwrap_or(&url).trim_start_matches("www.").to_string();
        let (c, a) = self.cartao_falha("azul", "cadeado", tr("LOGIN NEEDED"), &trf("{site} asks for a login to show this page", &[("site", site.clone())]),
            tr("I can go on without logging in, with what is public. Or log in yourself in the agent's browser: an isolated session, your login stays there."),
            tr("nothing of yours is used without you"));
        let sem = Self::botao_falha(&a, tr("Go on without logging in"), "principal");
        let cw = c.downgrade();
        sem.connect_clicked(move |_| {
            if let Some(c) = cw.upgrade() {
                c.add_css_class("resolvida");
            }
        });
        let entrar = Self::botao_falha(&a, tr("Log in with my account"), "");
        self.marcar_posicao("login entrar", &entrar);
        let w = Rc::downgrade(self);
        entrar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                if let Some(f) = c.on_page.borrow().as_ref() {
                    f(&url, true);
                }
            }
        });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug conversa: {site} pede login");
        }
    }

    /// The answer's sources: the pages read for it, numbered; each opens in the agent's browser.
    fn sources_row(self: &Rc<Self>) {
        let fontes = self.fontes.borrow().clone();
        if fontes.is_empty() {
            return;
        }
        let row = caixa(Orientation::Horizontal, 6, "conversa-fontes");
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_max_children_per_line(6);
        flow.set_column_spacing(6);
        flow.set_row_spacing(6);
        flow.set_hexpand(true);
        let titulo = rotulo(tr("Sources"), "conversa-fontes-titulo");
        titulo.set_valign(Align::Start);
        titulo.set_margin_top(5);
        row.append(&titulo);
        for (i, (url, title)) in fontes.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("conversa-fonte");
            let bx = caixa(Orientation::Horizontal, 6, "");
            bx.append(&rotulo(&(i + 1).to_string(), "conversa-fonte-n"));
            let site = url.split('/').nth(2).unwrap_or(url).trim_start_matches("www.").to_string();
            bx.append(&rotulo(&site, "conversa-fonte-site"));
            if !title.is_empty() {
                let t = rotulo(title, "conversa-fonte-titulo");
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(22);
                bx.append(&t);
            }
            b.set_child(Some(&bx));
            b.set_tooltip_text(Some(url));
            self.marcar_posicao(&format!("fonte {}", i + 1), &b);
            let (w, u) = (Rc::downgrade(self), url.clone());
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    if let Some(f) = c.on_page.borrow().as_ref() {
                        f(&u, false);
                    }
                }
            });
            flow.insert(&b, -1);
        }
        row.append(&flow);
        if let Some(t) = self.turn.borrow().as_ref() {
            t.body.append(&row);
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug conversa: fontes {}", fontes.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>().join(" · "));
        }
    }

    fn cartao_bloqueio(self: &Rc<Self>, alvo: &str, regra: &str) {
        let home = glib::home_dir().display().to_string();
        let curto = alvo.replacen(&home, "~", 1);
        let protegida = regra == "protegida";
        // (an installed agent's own limits: what it was installed with, not the person's permissions)
        let escopo = regra == "escopo";
        let (texto, rodape) = if protegida {
            (trf("Target: {target}. Rule: protected folder — no agent reads it.", &[("target", curto.clone())]), tr("recorded in the Timeline"))
        } else if escopo {
            (trf("Target: {target}. Rule: outside the limits this agent was installed with.", &[("target", curto.clone())]), tr("nothing was done outside its limits"))
        } else {
            (trf("Target: {target}. Rule: outside the folders the agent may use (Permissions › Where it can act).", &[("target", curto.clone())]), tr("nothing was done outside the folders"))
        };
        let (c, a) = self.cartao_falha("coral", "escudo", tr("ACTION BLOCKED"), tr("The agent tried to act outside the allowed folders"), &texto, rodape);
        let manter = Self::botao_falha(&a, tr("Keep it blocked"), "principal");
        let cw = c.downgrade();
        manter.connect_clicked(move |_| {
            if let Some(c) = cw.upgrade() {
                c.add_css_class("resolvida");
            }
        });
        if !protegida && !escopo {
            // the folder itself (a folder was the target) or the file's folder
            let pasta = if std::path::Path::new(alvo).is_dir() { curto.clone() } else { std::path::Path::new(&curto).parent().map(|p| p.display().to_string()).filter(|p| !p.is_empty()).unwrap_or(curto) };
            let nome = std::path::Path::new(&pasta).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pasta.clone());
            let b = Self::botao_falha(&a, &trf("Allow {folder}", &[("folder", nome)]), "");
            b.set_tooltip_text(Some(&pasta));
            self.marcar_posicao("liberar", &b);
            let w = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                if let Some(c) = w.upgrade() {
                    c.corrigir_e_repetir(vec!["pasta".into(), pasta.clone(), "adicionar".into()], b, trf("I allowed the folder {folder}; try again", &[("folder", pasta.clone())]));
                }
            });
        }
    }

    fn cartao_sem_rede(self: &Rc<Self>, url: &str) {
        let host = url.split('/').nth(2).unwrap_or(url).to_string();
        let (c, a) = self.cartao_falha("azul", "wifi", tr("NO CONNECTION"), tr("No internet: the task goes on when the network is back"),
            &trf("{site} did not answer: this computer is offline.", &[("site", host)]), tr("trying again when the network is back"));
        let agora = Self::botao_falha(&a, tr("Try now"), "principal");
        let w = Rc::downgrade(self);
        agora.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.de_novo(tr("Try again now"));
            }
        });
        let cancelar = Self::botao_falha(&a, tr("Cancel the task"), "");
        let cw = c.downgrade();
        let cancelada = Rc::new(Cell::new(false));
        let k2 = cancelada.clone();
        cancelar.connect_clicked(move |_| {
            k2.set(true);
            if let Some(c) = cw.upgrade() {
                c.add_css_class("resolvida");
            }
        });
        // once, when the network is back (not when rebuilding a kept conversation)
        if !self.replaying.get() {
            let w = Rc::downgrade(self);
            let feito = Rc::new(Cell::new(false));
            gtk::gio::NetworkMonitor::default().connect_network_changed(move |m, disponivel| {
                if disponivel && m.connectivity() == gtk::gio::NetworkConnectivity::Full && !feito.replace(true) && !cancelada.get() {
                    if let Some(c) = w.upgrade() {
                        c.de_novo(tr("The network is back; try again"));
                    }
                }
            });
        }
    }

    fn cartao_duvida(self: &Rc<Self>, pergunta: &str, opcoes: &[String]) {
        let (_, a) = self.cartao_falha("accent", "info", tr("I NEED YOU"), pergunta, "", tr("I would rather ask than guess"));
        for (i, o) in opcoes.iter().enumerate() {
            let b = Self::botao_falha(&a, o, if i == 0 { "principal" } else { "" });
            let (w, o) = (Rc::downgrade(self), o.clone());
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.ask(&o);
                }
            });
        }
    }
}

impl Drop for Conversa {
    fn drop(&mut self) {
        self.kill_engine();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pictures_and_sounds_go_to_the_service_as_media() {
        let f = |n: &str| std::path::PathBuf::from(format!("/tmp/teste/{n}"));
        let m = super::media_attachments(&[f("cartaz.png"), f("nota.wav"), f("texto.txt"), f("voz.ogg")], &[f("voz.ogg")]);
        let mimes: Vec<&str> = m.iter().filter_map(|a| a["mime"].as_str()).collect();
        assert_eq!(mimes.len(), 2, "{m:?}");
        assert!(mimes[0].starts_with("image/") && mimes[1].starts_with("audio/"), "{mimes:?}");
    }

    #[test]
    fn an_approval_badge_says_what_the_tool_does() {
        assert_eq!(super::approval_badge("web.buscar"), "Uses the internet");
        assert_eq!(super::approval_badge("arquivos.mover"), "Changes files");
        assert_eq!(super::approval_badge("notificar"), "Acts on the system");
    }

    use super::*;

    #[test]
    fn a_request_s_end_sounds_as_it_ended() {
        assert_eq!(turn_sound(None, true, false, true), Some("kalango-agent-done"));
        assert_eq!(turn_sound(None, true, false, false), None);
        assert_eq!(turn_sound(Some("falhou"), false, false, false), Some("kalango-agent-failed"));
        assert_eq!(turn_sound(Some("falhou"), true, true, true), None);
        assert_eq!(turn_sound(Some("parado"), false, false, true), None);
        assert_eq!(turn_sound(None, false, false, true), None);
    }

    #[test]
    fn model_names_and_arguments() {
        assert_eq!(model_label("LFM3-1.7B-Q4_K_M"), "LFM3 1.7B");
        assert_eq!(model_label("LFM3-VL-2B-Q8_0"), "LFM3 VL 2B");
        assert_eq!(model_label("LFM3-1.7B-BF16"), "LFM3 1.7B");
        assert_eq!(main_argument(&json!({"nomes": ["htop", "jq"]})), "htop, jq");
        assert_eq!(recolor("<path stroke=\"#8E8F99\" fill=\"none\"/>", "#FFFFFF"), "<path stroke=\"#FFFFFF\" fill=\"none\"/>");
    }
}
