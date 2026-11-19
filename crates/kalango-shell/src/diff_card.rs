//! An edit before it is written (design "Chat · blocos de mensagem" › Diff): when the agent asks to write a file, the
//! conversation shows what changes — the file's lines as they are now against the ones it would write, removed in
//! coral, added in teal, a little context around each change — with "Rejeitar" and "Aceitar". Accepting answers the
//! approval: the daemon writes it and the Timeline keeps the old content (Desfazer).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::tr;

/// One line of the comparison: (line number in the old file, in the new one, text). A removed line has no new
/// number, an added one no old number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
}

impl Line {
    pub fn removed(&self) -> bool {
        self.new.is_none()
    }
    pub fn added(&self) -> bool {
        self.old.is_none()
    }
}

/// Above this many line pairs the comparison is not worked out line by line (the whole file is new).
const LIMIT: usize = 4_000_000;

/// The old and new text line by line (the longest common run of lines kept, the rest removed or added).
pub fn diff(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    // the common start and end first (most edits touch a few lines in the middle)
    let mut pre = 0;
    while pre < n && pre < m && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < n - pre && suf < m - pre && a[n - 1 - suf] == b[m - 1 - suf] {
        suf += 1;
    }
    let (a_mid, b_mid) = (&a[pre..n - suf], &b[pre..m - suf]);
    let mut out: Vec<Line> = (0..pre).map(|i| Line { old: Some(i + 1), new: Some(i + 1), text: a[i].to_string() }).collect();
    let (x, y) = (a_mid.len(), b_mid.len());
    if x * y > LIMIT {
        out.extend(a_mid.iter().enumerate().map(|(i, t)| Line { old: Some(pre + i + 1), new: None, text: t.to_string() }));
        out.extend(b_mid.iter().enumerate().map(|(j, t)| Line { old: None, new: Some(pre + j + 1), text: t.to_string() }));
    } else {
        // lcs[i][j]: the longest common run of a_mid[i..] and b_mid[j..]
        let mut lcs = vec![0u32; (x + 1) * (y + 1)];
        let at = |i: usize, j: usize| i * (y + 1) + j;
        for i in (0..x).rev() {
            for j in (0..y).rev() {
                lcs[at(i, j)] = if a_mid[i] == b_mid[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < x || j < y {
            if i < x && j < y && a_mid[i] == b_mid[j] {
                out.push(Line { old: Some(pre + i + 1), new: Some(pre + j + 1), text: a_mid[i].to_string() });
                i += 1;
                j += 1;
            } else if j < y && (i == x || lcs[at(i, j + 1)] >= lcs[at(i + 1, j)]) {
                // (removals before additions at the same place: "−" then "+", as the design shows them)
                if i < x && lcs[at(i + 1, j)] == lcs[at(i, j + 1)] {
                    out.push(Line { old: Some(pre + i + 1), new: None, text: a_mid[i].to_string() });
                    i += 1;
                } else {
                    out.push(Line { old: None, new: Some(pre + j + 1), text: b_mid[j].to_string() });
                    j += 1;
                }
            } else {
                out.push(Line { old: Some(pre + i + 1), new: None, text: a_mid[i].to_string() });
                i += 1;
            }
        }
    }
    out.extend((0..suf).map(|k| Line { old: Some(n - suf + k + 1), new: Some(m - suf + k + 1), text: a[n - suf + k].to_string() }));
    out
}

/// Lines added and removed.
pub fn counts(lines: &[Line]) -> (usize, usize) {
    (lines.iter().filter(|l| l.added()).count(), lines.iter().filter(|l| l.removed()).count())
}

/// The changes with `context` unchanged lines around each; `None` between runs that are apart (a gap).
pub fn hunks(lines: &[Line], context: usize) -> Vec<Option<Line>> {
    let changed: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.added() || l.removed()).map(|(i, _)| i).collect();
    let mut keep = vec![false; lines.len()];
    for &c in &changed {
        for k in c.saturating_sub(context)..=(c + context).min(lines.len().saturating_sub(1)) {
            keep[k] = true;
        }
    }
    let mut out = Vec::new();
    let mut last: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        if last.is_some_and(|p| p + 1 != i) {
            out.push(None);
        }
        out.push(Some(l.clone()));
        last = Some(i);
    }
    out
}

/// The lines of a change (context around each part, "⋯" between): whole when short, scrolling inside when long.
fn body(lines: &[Line]) -> gtk::ScrolledWindow {
    let corpo = caixa(Orientation::Vertical, 0, "diff-linhas");
    let partes = hunks(&lines, 1);
    // (a long change: its first 400 lines here, the rest summed up)
    let mostrar = 400;
    for l in partes.iter().take(mostrar) {
        let linha = caixa(Orientation::Horizontal, 0, "diff-linha");
        let Some(l) = l else {
            linha.add_css_class("diff-intervalo");
            linha.append(&rotulo("⋯", "diff-num"));
            corpo.append(&linha);
            continue;
        };
        let (classe, sinal) = if l.removed() { ("removida", "-") } else if l.added() { ("adicionada", "+") } else { ("", " ") };
        if !classe.is_empty() {
            linha.add_css_class(classe);
        }
        let marca = caixa(Orientation::Vertical, 0, "diff-marca");
        marca.set_size_request(3, -1);
        linha.append(&marca);
        let num = rotulo(&l.new.or(l.old).map(|n| n.to_string()).unwrap_or_default(), "diff-num");
        num.set_xalign(1.0);
        num.set_size_request(26, -1);
        linha.append(&num);
        linha.append(&rotulo(sinal, "diff-sinal"));
        let txt = rotulo(&l.text, "diff-texto");
        txt.set_xalign(0.0);
        txt.set_selectable(true);
        linha.append(&txt);
        corpo.append(&linha);
    }
    if partes.len() > mostrar {
        let r = rotulo(&format!("⋯ +{}", partes.len() - mostrar), "diff-num");
        r.set_xalign(0.0);
        r.set_margin_start(13);
        corpo.append(&r);
    }
    let rol = gtk::ScrolledWindow::new();
    // (a short change shows whole — no vertical scroll that a relayout could squeeze to two lines —, a long one
    // scrolls inside at least 240 px)
    let rows = partes.len().min(mostrar + 1);
    let longa = rows > 16;
    rol.set_policy(gtk::PolicyType::Automatic, if longa { gtk::PolicyType::Automatic } else { gtk::PolicyType::Never });
    rol.set_propagate_natural_height(true);
    if longa {
        rol.set_min_content_height(240);
        rol.set_max_content_height(360);
    }
    rol.set_child(Some(&corpo));
    rol.add_css_class("md-bloco-rolagem");
    rol
}

/// The same change to look at only (no Accept/Reject): a file's diff in the Changes tab.
pub fn view(file: &str, old: &str, new: &str) -> gtk::Widget {
    let lines = diff(old, new);
    let (mais, menos) = counts(&lines);
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    bloco.add_css_class("diff");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("doc", "#8E8F99", 16));
    let t = rotulo(file, "md-bloco-titulo");
    t.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    t.set_xalign(0.0);
    t.set_hexpand(true);
    cab.append(&t);
    for (txt, cl) in [(format!("+{mais}"), "diff-mais"), (format!("−{menos}"), "diff-menos")] {
        let l = rotulo(&txt, cl);
        l.set_valign(Align::Center);
        cab.append(&l);
    }
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    bloco.append(&body(&lines));
    bloco.upcast()
}

/// The card. `answer(true)` accepts, `answer(false)` rejects; returns the card and its "Aceitar" button (an
/// approval from the island clicks it).
pub fn card(file: &str, old: &str, new: &str, answer: impl Fn(bool) + 'static) -> (gtk::Widget, gtk::Button) {
    let lines = diff(old, new);
    let (mais, menos) = counts(&lines);
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    bloco.add_css_class("diff");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("doc", "#8E8F99", 16));
    let t = rotulo(file, "md-bloco-titulo");
    // (the path's end shows when it does not fit: its file name)
    t.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    t.set_xalign(0.0);
    cab.append(&t);
    let pm = rotulo(&format!("+{mais}"), "diff-mais");
    pm.set_valign(Align::Center);
    cab.append(&pm);
    let pn = rotulo(&format!("−{menos}"), "diff-menos");
    pn.set_valign(Align::Center);
    cab.append(&pn);
    let e = caixa(Orientation::Horizontal, 0, "");
    e.set_hexpand(true);
    cab.append(&e);
    let rejeitar = gtk::Button::with_label(tr("Reject"));
    rejeitar.add_css_class("md-bloco-botao");
    rejeitar.set_valign(Align::Center);
    let aceitar = gtk::Button::with_label(tr("Accept"));
    aceitar.add_css_class("diff-aceitar");
    aceitar.set_valign(Align::Center);
    cab.append(&rejeitar);
    cab.append(&aceitar);
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    bloco.append(&body(&lines));
    let resultado = rotulo("", "cartao-rodape");
    resultado.set_xalign(0.0);
    resultado.set_margin_start(12);
    resultado.set_margin_bottom(8);
    resultado.set_margin_top(8);
    resultado.set_visible(false);
    bloco.append(&resultado);
    let answer = std::rc::Rc::new(answer);
    for (b, sim) in [(&rejeitar, false), (&aceitar, true)] {
        let (answer, rej, ace, res) = (answer.clone(), rejeitar.downgrade(), aceitar.downgrade(), resultado.clone());
        b.connect_clicked(move |_| {
            answer(sim);
            for b in [rej.upgrade(), ace.upgrade()].into_iter().flatten() {
                b.set_visible(false);
            }
            res.set_text(if sim { tr("Accepted: written (Undo in the Timeline).") } else { tr("Rejected: the file stays as it was.") });
            res.set_visible(true);
        });
    }
    crate::central::posicao_debug("diff aceitar", aceitar.upcast_ref());
    crate::central::posicao_debug("diff rejeitar", rejeitar.upcast_ref());
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug codigo: diff de {file} (+{mais} −{menos})");
    }
    (bloco.upcast(), aceitar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_changed() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nd\n";
        let l = diff(old, new);
        assert_eq!(counts(&l), (1, 1));
        let i = l.iter().position(|x| x.removed()).unwrap();
        assert_eq!(l[i].text, "b");
        assert_eq!(l[i + 1].text, "B");
        assert!(l[i + 1].added());
        assert_eq!(l[i + 1].new, Some(2));
    }

    #[test]
    fn added_and_new_file() {
        let l = diff("", "x\ny\n");
        assert_eq!(counts(&l), (2, 0));
        let l = diff("a\nc\n", "a\nb\nc\n");
        assert_eq!(counts(&l), (1, 0));
        assert_eq!(l[1], Line { old: None, new: Some(2), text: "b".into() });
        assert_eq!(l[2], Line { old: Some(2), new: Some(3), text: "c".into() });
    }

    #[test]
    fn hunks_keep_context_and_gaps() {
        let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l3\n", "L3\n").replace("l15\n", "L15\n");
        let h = hunks(&diff(&old, &new), 1);
        // two runs apart: a gap between them
        assert_eq!(h.iter().filter(|x| x.is_none()).count(), 1);
        assert!(h.iter().flatten().any(|l| l.text == "l2"));
        assert!(!h.iter().flatten().any(|l| l.text == "l9"));
    }
}
