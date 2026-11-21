//! A table in an answer (design "Chat · dados (tabela e gráfico)"): one block, two views. The table — numbers at the
//! right, the earlier period quieter, a change column coloured (a fall in teal, a rise in coral; the biggest rise's row
//! marked) — and the chart of its numeric columns, grouped bars per row with a legend. "↓ CSV" keeps it in Downloads
//! (where the Shelf picks it up).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use std::rc::Rc;

/// A cell's number: "R$ 1.240,00", "1,240.50", "−4,8%", "+12" (pt-BR or en separators; a sign of either dash).
pub fn number(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-' | '+' | '−')).collect();
    if !t.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    // (text with numbers inside is not a number: "Mercado 2")
    let letters = s.chars().filter(|c| c.is_alphabetic()).count();
    if letters > 3 {
        return None;
    }
    let neg = t.starts_with('-') || t.starts_with('−');
    let body: String = t.chars().filter(|c| c.is_ascii_digit() || matches!(c, ',' | '.')).collect();
    let (last_c, last_d) = (body.rfind(','), body.rfind('.'));
    let normal = match (last_c, last_d) {
        (Some(c), Some(d)) if c > d => body.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => body.replace(',', ""),
        // a lone comma: decimal when 1–2 digits follow ("4,8"), thousands otherwise ("1,240")
        (Some(c), None) => if body.len() - c - 1 <= 2 { body.replace(',', ".") } else { body.replace(',', "") },
        (None, Some(d)) => if body.len() - d - 1 == 3 && body.matches('.').count() >= 1 && !body.starts_with("0.") { body.replace('.', "") } else { body },
        (None, None) => body,
    };
    normal.parse::<f64>().ok().map(|v| if neg { -v } else { v })
}

/// A column of changes: its header says so ("Variação", "%", "Δ") or its values are signed percentages.
fn is_change(header: &str, values: &[&str]) -> bool {
    let h = header.to_lowercase();
    h.contains("varia") || h.contains('%') || h.contains('δ') || h.contains("change") || h.contains("diferen")
        || (!values.is_empty() && values.iter().all(|v| v.trim().ends_with('%')))
}

/// What the table is: which columns are numbers, which one is the change, the row with the biggest rise.
pub struct Shape {
    pub numeric: Vec<bool>,
    pub change: Option<usize>,
    pub top_rise: Option<usize>,
}

pub fn shape(rows: &[Vec<String>]) -> Shape {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let body = &rows[1.min(rows.len())..];
    let numeric: Vec<bool> = (0..cols).map(|c| {
        let vals: Vec<&str> = body.iter().filter_map(|r| r.get(c).map(String::as_str)).filter(|v| !v.trim().is_empty()).collect();
        c > 0 && !vals.is_empty() && vals.iter().all(|v| number(v).is_some())
    }).collect();
    let change = (0..cols).rev().find(|&c| {
        let vals: Vec<&str> = body.iter().filter_map(|r| r.get(c).map(String::as_str)).collect();
        numeric[c] && is_change(rows.first().and_then(|h| h.get(c)).map(String::as_str).unwrap_or(""), &vals)
    });
    let top_rise = change.and_then(|c| {
        body.iter().enumerate().filter_map(|(i, r)| r.get(c).and_then(|v| number(v)).map(|n| (i, n))).filter(|(_, n)| *n > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    });
    Shape { numeric, change, top_rise }
}

/// The table as CSV (quoted where it has to be).
pub fn csv(rows: &[Vec<String>]) -> String {
    rows.iter().map(|r| r.iter().map(|c| {
        let c = c.replace("**", "");
        if c.contains([',', '"', '\n']) { format!("\"{}\"", c.replace('"', "\"\"")) } else { c }
    }).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n") + "\n"
}

fn plain(s: &str) -> String {
    s.replace("**", "").replace('`', "")
}

/// The block (the header row first in `rows`).
pub fn new(rows: &[Vec<String>]) -> gtk::Widget {
    let raw = rows;
    let rows: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| plain(c)).collect()).collect();
    let sh = shape(&rows);
    let n_rows = rows.len().saturating_sub(1);
    let n_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let series: Vec<usize> = (0..n_cols).filter(|&c| sh.numeric[c] && Some(c) != sh.change).collect();
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    bloco.add_css_class("tabela-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("grade", "#8E8F99", 16));
    let titulo = rotulo(tr("Table"), "md-bloco-titulo");
    cab.append(&titulo);
    let det = rotulo(&format!("{} · {}", trf("{n} rows", &[("n", n_rows.to_string())]), trf("{n} columns", &[("n", n_cols.to_string())])), "md-bloco-detalhe");
    det.set_hexpand(true);
    det.set_xalign(0.0);
    det.set_ellipsize(gtk::pango::EllipsizeMode::End);
    cab.append(&det);
    // the two views (a chart only when there are numbers to draw)
    let seg = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
    seg.set_valign(Align::Center);
    let b_tab = gtk::ToggleButton::with_label(tr("Table"));
    let b_graf = gtk::ToggleButton::with_label(tr("Chart"));
    for b in [&b_tab, &b_graf] {
        b.add_css_class("tabela-aba");
        seg.append(b);
    }
    b_graf.set_group(Some(&b_tab));
    b_tab.set_active(true);
    seg.set_visible(!series.is_empty() && n_rows > 0);
    cab.append(&seg);
    let baixar = gtk::Button::new();
    baixar.add_css_class("md-bloco-botao");
    let bc = caixa(Orientation::Horizontal, 6, "");
    bc.append(&ui_icon("download", "#8E8F99", 16));
    bc.append(&rotulo("CSV", ""));
    baixar.set_child(Some(&bc));
    baixar.set_valign(Align::Center);
    baixar.set_tooltip_text(Some(tr("Save as CSV in Downloads")));
    cab.append(&baixar);
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    let vistas = gtk::Stack::new();
    vistas.set_vhomogeneous(false);
    vistas.set_interpolate_size(true);
    vistas.set_transition_type(gtk::StackTransitionType::Crossfade);
    // ---- the table
    let grid = gtk::Grid::new();
    grid.add_css_class("tabela-grade");
    for (r, row) in rows.iter().enumerate() {
        let alta = r > 0 && sh.top_rise == Some(r - 1);
        for (c, cell) in row.iter().enumerate() {
            let class = if r == 0 { "tabela-th" } else if c == 0 { "tabela-nome" } else { "tabela-td" };
            let l = rotulo(cell, class);
            // a cell with an address: its link opens (the rest of the table is plain text)
            if let Some(src) = raw.get(r).and_then(|x| x.get(c)).filter(|x| x.contains("http://") || x.contains("https://") || x.contains("mailto:")) {
                l.set_markup(&crate::markdown::inline(src));
                crate::markdown::open_links(&l);
            }
            let num = c > 0 && sh.numeric.get(c).copied().unwrap_or(false);
            l.set_xalign(if num { 1.0 } else { 0.0 });
            l.set_hexpand(c == 0);
            l.set_wrap(c == 0);
            if r > 0 {
                l.add_css_class("tabela-linha");
                if alta {
                    l.add_css_class("alta");
                }
                // the earlier period quieter than the latest (the first of two or more series)
                if series.len() >= 2 && series.first() == Some(&c) {
                    l.add_css_class("antes");
                }
                if Some(c) == sh.change {
                    match number(cell) {
                        Some(v) if v > 0.0 => l.add_css_class("subiu"),
                        Some(v) if v < 0.0 => l.add_css_class("caiu"),
                        _ => l.add_css_class("antes"),
                    }
                }
            } else {
                l.add_css_class("tabela-cabecalho");
            }
            grid.attach(&l, c as i32, r as i32, 1, 1);
        }
    }
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    rol.set_propagate_natural_height(true);
    rol.set_child(Some(&grid));
    vistas.add_named(&rol, Some("tabela"));
    // ---- the chart: grouped bars, one group per row, one bar per numeric column
    let area = gtk::DrawingArea::new();
    area.set_content_height(200);
    area.set_hexpand(true);
    let data: Rc<(Vec<String>, Vec<String>, Vec<Vec<f64>>, Option<usize>)> = Rc::new((
        rows.iter().skip(1).map(|r| r.first().cloned().unwrap_or_default()).collect(),
        series.iter().map(|&c| rows[0].get(c).cloned().unwrap_or_default()).collect(),
        rows.iter().skip(1).map(|r| series.iter().map(|&c| r.get(c).and_then(|v| number(v)).unwrap_or(0.0)).collect()).collect(),
        sh.top_rise,
    ));
    let d2 = data.clone();
    area.set_draw_func(move |a, cr, w, h| draw_chart(a, cr, w, h, &d2));
    vistas.add_named(&area, Some("grafico"));
    bloco.append(&vistas);
    let v = vistas.clone();
    b_graf.connect_toggled(move |b| {
        v.set_visible_child_name(if b.is_active() { "grafico" } else { "tabela" });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug dados: vista {}", if b.is_active() { "gráfico" } else { "tabela" });
        }
    });
    crate::central::posicao_debug("tabela grafico", b_graf.upcast_ref());
    crate::central::posicao_debug("tabela csv", baixar.upcast_ref());
    let rows_csv = rows.clone();
    baixar.connect_clicked(move |b| {
        let dir = crate::pastas::downloads();
        let _ = std::fs::create_dir_all(&dir);
        let p = crate::widgets::nome_livre(&dir, tr("table"), "csv");
        match std::fs::write(&p, csv(&rows_csv)) {
            Ok(()) => {
                b.set_tooltip_text(Some(&trf("Saved in {folder}", &[("folder", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())])));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug dados: csv salvo {}", p.display());
                }
            }
            Err(e) => eprintln!("kalango-shell: tabela em CSV: {e}"),
        }
    });
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug dados: tabela {n_rows}×{n_cols} ({} séries{})", series.len(), sh.change.map(|c| format!(", variação na coluna {}", c + 1)).unwrap_or_default());
    }
    bloco.upcast()
}

type ChartData = (Vec<String>, Vec<String>, Vec<Vec<f64>>, Option<usize>);

fn draw_chart(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, d: &ChartData) {
    let (labels, names, vals, top) = d;
    if labels.is_empty() || names.is_empty() {
        return;
    }
    let (w, h) = (f64::from(w), f64::from(h));
    let accent = crate::tema::rgb("#A5A1E2");
    let coral = crate::tema::rgb("#EFA2A6");
    let others = [crate::tema::rgb("#8ED2C3"), crate::tema::rgb("#E5C98C"), crate::tema::rgb("#7CC3DF")];
    // a series' colour: the earlier one faint, the latest in the accent, more in the theme's others
    let colour = |s: usize| -> ((f64, f64, f64), f64) {
        match (names.len(), s) {
            (1, _) => (accent, 1.0),
            (_, 0) => (accent, 0.4),
            (_, 1) => (accent, 1.0),
            (_, k) => (others[(k - 2) % others.len()], 1.0),
        }
    };
    let max = vals.iter().flatten().fold(0.0f64, |m, v| m.max(*v)).max(1e-9);
    let (top_m, legend_h, label_h) = (14.0, 24.0, 18.0);
    let plot_h = (h - top_m - legend_h - label_h).max(10.0);
    let group_w = (w - 32.0) / labels.len() as f64;
    let bar_w = (group_w * 0.5 / names.len() as f64).clamp(6.0, 26.0);
    let muted = crate::tema::rgb("#8E8F99");
    let text = crate::tema::rgb("#E2E2E6");
    let pango = |cr: &gtk::cairo::Context, t: &str, size: f64| {
        let l = pangocairo::functions::create_layout(cr);
        let mut f = gtk::pango::FontDescription::from_string(crate::tema::fonte_ui());
        f.set_absolute_size(size * f64::from(gtk::pango::SCALE));
        l.set_font_description(Some(&f));
        l.set_text(t);
        l
    };
    for (i, (label, row)) in labels.iter().zip(vals).enumerate() {
        let gx = 16.0 + group_w * i as f64 + (group_w - bar_w * names.len() as f64 - 2.0 * (names.len() - 1) as f64) / 2.0;
        for (s, v) in row.iter().enumerate() {
            let bh = (v.max(0.0) / max) * plot_h;
            let x = gx + s as f64 * (bar_w + 2.0);
            let y = top_m + plot_h - bh;
            let ((r, g, b), alpha) = if *top == Some(i) && s + 1 == names.len() && names.len() > 1 { (coral, 1.0) } else { colour(s) };
            cr.set_source_rgba(r, g, b, alpha);
            let rad = 2.0f64.min(bar_w / 2.0).min(bh / 2.0);
            cr.new_sub_path();
            cr.arc(x + bar_w - rad, y + rad, rad, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.line_to(x + bar_w, y + bh);
            cr.line_to(x, y + bh);
            cr.arc(x + rad, y + rad, rad, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
            let _ = cr.fill();
        }
        let l = pango(cr, label, 11.0);
        let (lw, _) = l.pixel_size();
        let (r, g, b) = if *top == Some(i) { text } else { muted };
        cr.set_source_rgb(r, g, b);
        cr.move_to(16.0 + group_w * i as f64 + (group_w - f64::from(lw)) / 2.0, top_m + plot_h + 4.0);
        pangocairo::functions::show_layout(cr, &l);
    }
    // the legend, centred under the bars
    let mut items: Vec<(String, (f64, f64, f64), f64)> = names.iter().enumerate().map(|(s, n)| { let (c, a) = colour(s); (n.clone(), c, a) }).collect();
    if top.is_some() && names.len() > 1 {
        items.push((tr("biggest rise").to_string(), coral, 1.0));
    }
    let widths: Vec<f64> = items.iter().map(|(n, ..)| f64::from(pango(cr, n, 11.0).pixel_size().0) + 14.0 + 16.0).collect();
    let mut x = (w - widths.iter().sum::<f64>()) / 2.0;
    let y = h - legend_h + 8.0;
    for ((n, (r, g, b), alpha), wd) in items.iter().zip(&widths) {
        cr.set_source_rgba(*r, *g, *b, *alpha);
        cr.rectangle(x, y + 2.0, 8.0, 8.0);
        let _ = cr.fill();
        cr.set_source_rgb(muted.0, muted.1, muted.2);
        cr.move_to(x + 14.0, y);
        pangocairo::functions::show_layout(cr, &pango(cr, n, 11.0));
        x += wd;
    }
    let _ = a;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_both_ways() {
        assert_eq!(number("R$ 1.240,00"), Some(1240.0));
        assert_eq!(number("1,240.50"), Some(1240.5));
        assert_eq!(number("−4,8%"), Some(-4.8));
        assert_eq!(number("+12,8%"), Some(12.8));
        assert_eq!(number("0%"), Some(0.0));
        assert_eq!(number("Mercado"), None);
        assert_eq!(number("3.5"), Some(3.5));
        assert_eq!(number("1.240"), Some(1240.0));
    }

    #[test]
    fn the_table_shape() {
        let t = |v: &[&[&str]]| v.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect::<Vec<Vec<String>>>();
        let rows = t(&[
            &["Categoria", "Outubro", "Novembro", "Variação"],
            &["Mercado", "R$ 1.240,00", "R$ 1.180,00", "−4,8%"],
            &["Energia", "R$ 189,90", "R$ 214,30", "+12,8%"],
            &["Assinaturas", "R$ 96,70", "R$ 142,60", "+47,5%"],
        ]);
        let s = shape(&rows);
        assert_eq!(s.numeric, vec![false, true, true, true]);
        assert_eq!(s.change, Some(3));
        assert_eq!(s.top_rise, Some(2));
        assert_eq!(csv(&rows[..2]).lines().nth(1), Some("Mercado,\"R$ 1.240,00\",\"R$ 1.180,00\",\"−4,8%\""));
    }
}
