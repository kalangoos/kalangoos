//! Formulas in an answer (design "Chat · diagramas e fórmulas"): a ```latex block or $$…$$ lines typeset in the
//! conversation, one equation per line, numbered at the right, with "Copiar TeX". A small math layout of its own —
//! fractions, roots, super- and subscripts, big operators with their limits (∑ ∏ ∫ lim), \left…\right, Greek letters
//! and the usual symbols — drawn with pango on cairo in the theme's text colour; anything unknown shows as written.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use std::rc::Rc;

/// A piece of a formula as read.
#[derive(Clone, Debug, PartialEq)]
pub enum Atom {
    /// a symbol (a letter is italic, a digit or an operator upright)
    Sym(String, bool),
    Row(Vec<Atom>),
    Frac(Box<Atom>, Box<Atom>),
    Sqrt(Box<Atom>),
    /// a base with its superscript and subscript
    Scripts(Box<Atom>, Option<Box<Atom>>, Option<Box<Atom>>),
    /// a big operator (∑ ∏ ∫ lim) with its limits under and over it
    Big(String, Option<Box<Atom>>, Option<Box<Atom>>),
    /// \left( … \right)
    Fence(String, Box<Atom>, String),
}

fn symbol(cmd: &str) -> Option<(&'static str, bool)> {
    Some(match cmd {
        "alpha" => ("α", true), "beta" => ("β", true), "gamma" => ("γ", true), "delta" => ("δ", true), "epsilon" | "varepsilon" => ("ε", true),
        "zeta" => ("ζ", true), "eta" => ("η", true), "theta" => ("θ", true), "lambda" => ("λ", true), "mu" => ("μ", true), "nu" => ("ν", true),
        "xi" => ("ξ", true), "pi" => ("π", true), "rho" => ("ρ", true), "sigma" => ("σ", true), "tau" => ("τ", true), "phi" | "varphi" => ("φ", true),
        "chi" => ("χ", true), "psi" => ("ψ", true), "omega" => ("ω", true), "Gamma" => ("Γ", false), "Delta" => ("Δ", false), "Theta" => ("Θ", false),
        "Lambda" => ("Λ", false), "Pi" => ("Π", false), "Sigma" => ("Σ", false), "Phi" => ("Φ", false), "Psi" => ("Ψ", false), "Omega" => ("Ω", false),
        "pm" => ("±", false), "mp" => ("∓", false), "cdot" => ("·", false), "times" => ("×", false), "div" => ("÷", false), "infty" => ("∞", false),
        "leq" | "le" => ("≤", false), "geq" | "ge" => ("≥", false), "neq" | "ne" => ("≠", false), "approx" => ("≈", false), "equiv" => ("≡", false),
        "to" | "rightarrow" => ("→", false), "leftarrow" => ("←", false), "Rightarrow" => ("⇒", false), "iff" | "Leftrightarrow" => ("⇔", false),
        "in" => ("∈", false), "notin" => ("∉", false), "subset" => ("⊂", false), "cup" => ("∪", false), "cap" => ("∩", false), "forall" => ("∀", false),
        "exists" => ("∃", false), "partial" => ("∂", false), "nabla" => ("∇", false), "ldots" | "dots" | "cdots" => ("…", false), "circ" => ("∘", false),
        "sin" => ("sin", false), "cos" => ("cos", false), "tan" => ("tan", false), "log" => ("log", false), "ln" => ("ln", false), "exp" => ("exp", false),
        "max" => ("max", false), "min" => ("min", false), "quad" => ("  ", false), "qquad" => ("    ", false), "," => (" ", false), ";" => (" ", false),
        "!" => ("", false), " " => (" ", false), "{" => ("{", false), "}" => ("}", false), "%" => ("%", false),
        _ => return None,
    })
}

struct Parser<'a> {
    s: Vec<char>,
    i: usize,
    _src: &'a str,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn command(&mut self) -> String {
        // after the backslash: a word, or one character
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i == start {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    /// One argument: a {group} or a single atom.
    fn argument(&mut self) -> Atom {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
        if self.peek() == Some('{') {
            self.i += 1;
            let r = self.row(Some('}'));
            self.i += 1;
            r
        } else {
            self.atom().unwrap_or(Atom::Row(vec![]))
        }
    }

    fn atom(&mut self) -> Option<Atom> {
        let c = self.peek()?;
        self.i += 1;
        Some(match c {
            '\\' => {
                let cmd = self.command();
                match cmd.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let a = self.argument();
                        let b = self.argument();
                        Atom::Frac(Box::new(a), Box::new(b))
                    }
                    "sqrt" => Atom::Sqrt(Box::new(self.argument())),
                    "sum" => Atom::Big("∑".into(), None, None),
                    "prod" => Atom::Big("∏".into(), None, None),
                    "int" => Atom::Big("∫".into(), None, None),
                    "lim" => Atom::Big("lim".into(), None, None),
                    "left" => {
                        let open = self.fence_char();
                        let inner = self.row_until_right();
                        let close = self.fence_char();
                        Atom::Fence(open, Box::new(inner), close)
                    }
                    "text" | "mathrm" | "operatorname" => {
                        let a = self.argument();
                        Atom::Sym(flat(&a), false)
                    }
                    "mathbf" | "mathit" | "boldsymbol" => self.argument(),
                    other => match symbol(other) {
                        Some((t, it)) => Atom::Sym(t.into(), it),
                        None => Atom::Sym(format!("\\{other}"), false),
                    },
                }
            }
            '{' => {
                let r = self.row(Some('}'));
                self.i += 1;
                r
            }
            c if c.is_ascii_alphabetic() => Atom::Sym(c.to_string(), true),
            c if c.is_ascii_digit() || c == '.' => {
                let mut t = c.to_string();
                while self.peek().is_some_and(|d| d.is_ascii_digit() || d == '.' || d == ',') {
                    t.push(self.peek().unwrap());
                    self.i += 1;
                }
                Atom::Sym(t, false)
            }
            '-' => Atom::Sym("−".into(), false),
            '*' => Atom::Sym("∗".into(), false),
            '\'' => Atom::Sym("′".into(), false),
            c => Atom::Sym(c.to_string(), false),
        })
    }

    fn fence_char(&mut self) -> String {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
        match self.peek() {
            Some('\\') => {
                self.i += 1;
                let c = self.command();
                match c.as_str() {
                    "{" => "{".into(),
                    "}" => "}".into(),
                    "|" => "‖".into(),
                    "langle" => "⟨".into(),
                    "rangle" => "⟩".into(),
                    _ => String::new(),
                }
            }
            Some('.') => {
                self.i += 1;
                String::new()
            }
            Some(c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    fn row_until_right(&mut self) -> Atom {
        let mut items = Vec::new();
        while self.i < self.s.len() {
            let rest: String = self.s[self.i..].iter().take(6).collect();
            if rest.starts_with("\\right") {
                self.i += 6;
                break;
            }
            if let Some(a) = self.scripted() {
                items.push(a);
            }
        }
        Atom::Row(items)
    }

    /// An atom with its ^ and _ (a big operator takes them as its limits).
    fn scripted(&mut self) -> Option<Atom> {
        while self.peek() == Some(' ') {
            self.i += 1;
        }
        if self.peek().is_none() || self.peek() == Some('}') {
            return None;
        }
        let base = self.atom()?;
        let (mut sup, mut sub) = (None, None);
        loop {
            while self.peek() == Some(' ') {
                self.i += 1;
            }
            match self.peek() {
                Some('^') => {
                    self.i += 1;
                    sup = Some(Box::new(self.argument()));
                }
                Some('_') => {
                    self.i += 1;
                    sub = Some(Box::new(self.argument()));
                }
                _ => break,
            }
        }
        Some(match base {
            Atom::Big(op, _, _) => Atom::Big(op, sub, sup),
            b if sup.is_some() || sub.is_some() => Atom::Scripts(Box::new(b), sup, sub),
            b => b,
        })
    }

    fn row(&mut self, until: Option<char>) -> Atom {
        let mut items = Vec::new();
        while self.i < self.s.len() && self.peek() != until {
            match self.scripted() {
                Some(a) => items.push(a),
                None => {
                    if self.peek() == Some('}') && until.is_none() {
                        self.i += 1;
                    } else if self.peek().is_some() && self.peek() != until {
                        self.i += 1;
                    }
                }
            }
        }
        Atom::Row(items)
    }
}

fn flat(a: &Atom) -> String {
    match a {
        Atom::Sym(t, _) => t.clone(),
        Atom::Row(v) => v.iter().map(flat).collect(),
        _ => String::new(),
    }
}

pub fn parse(src: &str) -> Atom {
    let mut p = Parser { s: src.trim().chars().collect(), i: 0, _src: src };
    p.row(None)
}

/// A laid-out piece: its width, how far it rises above the baseline and drops under it, and how to draw it.
struct Laid {
    w: f64,
    up: f64,
    down: f64,
    draw: Rc<dyn Fn(&gtk::cairo::Context, f64, f64)>,
}

fn text(cr: &gtk::cairo::Context, t: &str, size: f64, italic: bool) -> Laid {
    let l = pangocairo::functions::create_layout(cr);
    let mut f = gtk::pango::FontDescription::from_string("Noto Serif");
    f.set_absolute_size(size * f64::from(gtk::pango::SCALE));
    if italic {
        f.set_style(gtk::pango::Style::Italic);
    }
    l.set_font_description(Some(&f));
    l.set_text(t);
    let (_, log) = l.pixel_extents();
    let base = f64::from(l.baseline()) / f64::from(gtk::pango::SCALE);
    let (w, h) = (f64::from(log.width()), f64::from(log.height()));
    // (operators get a little room on both sides)
    let pad = if !italic && t.chars().count() == 1 && "=+−±∓×÷·<>≤≥≠≈≡→⇒⇔∈".contains(t) { size * 0.22 } else { 0.0 };
    Laid {
        w: w + 2.0 * pad,
        up: base,
        down: h - base,
        draw: Rc::new(move |cr, x, y| {
            cr.move_to(x + pad, y - base);
            pangocairo::functions::show_layout(cr, &l);
        }),
    }
}

fn lay(cr: &gtk::cairo::Context, a: &Atom, size: f64) -> Laid {
    match a {
        Atom::Sym(t, it) => text(cr, t, size, *it),
        Atom::Row(v) => {
            let parts: Vec<Laid> = v.iter().map(|x| lay(cr, x, size)).collect();
            let w = parts.iter().map(|p| p.w).sum();
            let up = parts.iter().map(|p| p.up).fold(size * 0.7, f64::max);
            let down = parts.iter().map(|p| p.down).fold(size * 0.25, f64::max);
            let parts = Rc::new(parts);
            Laid { w, up, down, draw: Rc::new(move |cr, x, y| {
                let mut xx = x;
                for p in parts.iter() {
                    (p.draw)(cr, xx, y);
                    xx += p.w;
                }
            }) }
        }
        Atom::Frac(n, d) => {
            let (n, d) = (lay(cr, n, size * 0.9), lay(cr, d, size * 0.9));
            let w = n.w.max(d.w) + size * 0.3;
            let axis = size * 0.3;
            let gap = size * 0.15;
            let (up, down) = (axis + gap + n.up + n.down, -axis + gap + d.up + d.down);
            let (n, d) = (Rc::new(n), Rc::new(d));
            Laid { w, up, down: down.max(0.0), draw: Rc::new(move |cr, x, y| {
                (n.draw)(cr, x + (w - n.w) / 2.0, y - axis - gap - n.down);
                (d.draw)(cr, x + (w - d.w) / 2.0, y - axis + gap + d.up);
                cr.set_line_width(1.0);
                cr.move_to(x + 1.0, y - axis);
                cr.line_to(x + w - 1.0, y - axis);
                let _ = cr.stroke();
            }) }
        }
        Atom::Sqrt(inner) => {
            let i = lay(cr, inner, size);
            let lead = size * 0.6;
            let (up, down) = (i.up + size * 0.18, i.down + 1.0);
            let w = lead + i.w + size * 0.1;
            let i = Rc::new(i);
            Laid { w, up, down, draw: Rc::new(move |cr, x, y| {
                cr.set_line_width(1.0);
                cr.move_to(x, y - up * 0.35);
                cr.line_to(x + lead * 0.3, y - up * 0.45);
                cr.line_to(x + lead * 0.6, y + down);
                cr.line_to(x + lead, y - up);
                cr.line_to(x + w, y - up);
                let _ = cr.stroke();
                (i.draw)(cr, x + lead, y);
            }) }
        }
        Atom::Scripts(b, sup, sub) => {
            let b = lay(cr, b, size);
            let sup = sup.as_ref().map(|s| lay(cr, s, size * 0.7));
            let sub = sub.as_ref().map(|s| lay(cr, s, size * 0.7));
            let sw = sup.as_ref().map(|s| s.w).unwrap_or(0.0).max(sub.as_ref().map(|s| s.w).unwrap_or(0.0));
            let rise = b.up * 0.55;
            let drop_ = size * 0.25;
            let up = b.up.max(sup.as_ref().map(|s| rise + s.up).unwrap_or(0.0));
            let down = b.down.max(sub.as_ref().map(|s| drop_ + s.down).unwrap_or(0.0));
            let w = b.w + sw + size * 0.05;
            let (b, sup, sub) = (Rc::new(b), sup.map(Rc::new), sub.map(Rc::new));
            Laid { w, up, down, draw: Rc::new(move |cr, x, y| {
                (b.draw)(cr, x, y);
                if let Some(s) = &sup {
                    (s.draw)(cr, x + b.w + 1.0, y - rise);
                }
                if let Some(s) = &sub {
                    (s.draw)(cr, x + b.w + 1.0, y + drop_);
                }
            }) }
        }
        Atom::Big(op, under, over) => {
            let o = text(cr, op, if op.chars().count() == 1 { size * 1.6 } else { size }, false);
            let under = under.as_ref().map(|s| lay(cr, s, size * 0.7));
            let over = over.as_ref().map(|s| lay(cr, s, size * 0.7));
            let w = o.w.max(under.as_ref().map(|s| s.w).unwrap_or(0.0)).max(over.as_ref().map(|s| s.w).unwrap_or(0.0)) + size * 0.3;
            let lift = (o.up - o.down) / 2.0 - size * 0.3;
            let up = o.up - lift + over.as_ref().map(|s| s.up + s.down + 2.0).unwrap_or(0.0);
            let down = o.down + lift + under.as_ref().map(|s| s.up + s.down + 2.0).unwrap_or(0.0);
            let (o, under, over) = (Rc::new(o), under.map(Rc::new), over.map(Rc::new));
            Laid { w, up, down, draw: Rc::new(move |cr, x, y| {
                let oy = y + lift;
                (o.draw)(cr, x + (w - o.w) / 2.0, oy);
                if let Some(s) = &over {
                    (s.draw)(cr, x + (w - s.w) / 2.0, oy - o.up - 2.0 - s.down);
                }
                if let Some(s) = &under {
                    (s.draw)(cr, x + (w - s.w) / 2.0, oy + o.down + 2.0 + s.up);
                }
            }) }
        }
        Atom::Fence(open, inner, close) => {
            let i = lay(cr, inner, size);
            let scale = ((i.up + i.down) / (size * 1.2)).max(1.0);
            let o = text(cr, open, size * scale, false);
            let c = text(cr, close, size * scale, false);
            let w = o.w + i.w + c.w;
            let up = i.up.max(o.up - (o.up + o.down - i.up - i.down).max(0.0) / 2.0);
            let down = i.down;
            let shift = ((o.up + o.down) - (i.up + i.down)) / 2.0;
            let (o, i, c) = (Rc::new(o), Rc::new(i), Rc::new(c));
            Laid { w, up, down, draw: Rc::new(move |cr, x, y| {
                let fy = y + i.down - o.down + shift.max(0.0);
                (o.draw)(cr, x, fy);
                (i.draw)(cr, x + o.w, y);
                (c.draw)(cr, x + o.w + i.w, fy);
            }) }
        }
    }
}

/// The equations of a block: one per line (an `\\` or a blank line between), each with its own optional `name:`.
pub fn equations(src: &str) -> Vec<String> {
    src.split("\\\\").flat_map(|p| p.lines().map(str::to_string).collect::<Vec<_>>())
        .map(|l| l.trim().trim_matches('$').trim().trim_start_matches("\\[").trim_end_matches("\\]").trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("\\begin") && !l.starts_with("\\end")).collect()
}

/// The block for a LaTeX source (None when there is no equation in it).
pub fn new(src: &str) -> Option<gtk::Widget> {
    let eqs = equations(src);
    if eqs.is_empty() {
        return None;
    }
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("codigo", "#8E8F99", 16));
    cab.append(&rotulo("LaTeX", "md-bloco-titulo"));
    let det = rotulo(&format!("{} · {}", tr("rendered"), if eqs.len() == 1 { tr("1 equation").to_string() } else { trf("{n} equations", &[("n", eqs.len().to_string())]) }), "md-bloco-detalhe");
    det.set_hexpand(true);
    det.set_xalign(0.0);
    cab.append(&det);
    let copiar = gtk::Button::new();
    copiar.add_css_class("md-bloco-botao");
    let cc = caixa(Orientation::Horizontal, 6, "");
    cc.append(&ui_icon("copiar", "#8E8F99", 16));
    cc.append(&rotulo(tr("Copy TeX"), ""));
    copiar.set_child(Some(&cc));
    copiar.set_valign(Align::Center);
    let tex = src.trim().to_string();
    copiar.connect_clicked(move |b| b.clipboard().set_text(&tex));
    cab.append(&copiar);
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    let corpo = caixa(Orientation::Vertical, 0, "formulas");
    for (k, e) in eqs.iter().enumerate() {
        let linha = caixa(Orientation::Horizontal, 12, "formula-linha");
        if k > 0 {
            linha.add_css_class("seguinte");
        }
        let atom = Rc::new(parse(e));
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        // its height from a first layout
        let (lw, lh) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).ok().and_then(|s| gtk::cairo::Context::new(&s).ok())
            .map(|cr| { let l = lay(&cr, &atom, 20.0); (l.w, l.up + l.down) }).unwrap_or((200.0, 40.0));
        area.set_content_height((lh + 16.0).ceil() as i32);
        area.set_content_width(lw.ceil() as i32);
        let a2 = atom.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let l = lay(cr, &a2, 20.0);
            let t = crate::tema::rgb("#E2E2E6");
            cr.set_source_rgb(t.0, t.1, t.2);
            let x = ((f64::from(w) - l.w) / 2.0).max(0.0);
            let y = (f64::from(h) - l.up - l.down) / 2.0 + l.up;
            (l.draw)(cr, x, y);
        });
        linha.append(&area);
        let n = rotulo(&format!("({})", k + 1), "formula-numero");
        n.set_valign(Align::Center);
        linha.append(&n);
        corpo.append(&linha);
    }
    bloco.append(&corpo);
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug formulas: {} equações", eqs.len());
    }
    Some(bloco.upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bhaskara() {
        let a = parse(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}");
        let Atom::Row(v) = a else { panic!() };
        assert_eq!(v[0], Atom::Sym("x".into(), true));
        assert_eq!(v[1], Atom::Sym("=".into(), false));
        let Atom::Frac(num, den) = &v[2] else { panic!("{:?}", v[2]) };
        let Atom::Row(n) = num.as_ref() else { panic!() };
        assert!(n.iter().any(|x| matches!(x, Atom::Sqrt(_))));
        assert!(n.contains(&Atom::Sym("±".into(), false)));
        assert_eq!(flat(den), "2a");
    }

    #[test]
    fn basel_and_lines() {
        let a = parse(r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}");
        let Atom::Row(v) = a else { panic!() };
        let Atom::Big(op, Some(under), Some(over)) = &v[0] else { panic!("{:?}", v[0]) };
        assert_eq!(op, "∑");
        assert_eq!(flat(under), "n=1");
        assert_eq!(flat(over), "∞");
        assert_eq!(equations("$$a=b$$\n\n$$c=d$$").len(), 2);
        assert_eq!(equations("a = 1 \\\\ b = 2"), vec!["a = 1".to_string(), "b = 2".to_string()]);
    }
}
