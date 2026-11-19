//! The calculator's arithmetic (KalangoOS's Calculadora): numbers as people in Brazil write them ("1.240,5"), the four
//! operations with precedence, parentheses, and percent as calculators do it — "a + b%" is a plus b percent of a,
//! "a × b%" is b percent of a.

/// A number as the calculator shows it: thousands with ".", decimals with "," (up to 10 places, no trailing zeros).
pub fn format(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let neg = v < 0.0;
    let a = v.abs();
    let rounded = (a * 1e10).round() / 1e10;
    let int = rounded.trunc();
    let mut frac = format!("{:.10}", rounded - int);
    frac = frac.trim_start_matches('0').trim_start_matches('.').trim_end_matches('0').to_string();
    let digits = format!("{int:.0}");
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    let mut s = if neg && (int > 0.0 || !frac.is_empty()) { format!("-{grouped}") } else { grouped };
    if !frac.is_empty() {
        s.push(',');
        s.push_str(&frac);
    }
    s
}

/// A number typed the Brazilian way ("1.240,5", "15") or with a decimal point when there is no comma ("2.5").
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let normal = if t.contains(',') {
        t.replace('.', "").replace(',', ".")
    } else if t.matches('.').count() == 1 && t.split('.').nth(1).is_some_and(|d| d.len() != 3) {
        t.to_string()
    } else {
        t.replace('.', "")
    };
    normal.parse().ok()
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    Pct,
    /// "²" after a number
    Sq,
    Open,
    Close,
    /// sin, cos, tan (degrees), ln, log (base 10), sqrt
    Func(&'static str),
}

fn tokens(expr: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let cs: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' => i += 1,
            '0'..='9' | ',' | '.' => {
                let start = i;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == ',' || cs[i] == '.') {
                    i += 1;
                }
                out.push(Tok::Num(parse_number(&cs[start..i].iter().collect::<String>())?));
            }
            '+' => {
                out.push(Tok::Op('+'));
                i += 1;
            }
            '-' | '−' => {
                out.push(Tok::Op('-'));
                i += 1;
            }
            '*' | '×' => {
                out.push(Tok::Op('*'));
                i += 1;
            }
            '/' | '÷' | ':' => {
                out.push(Tok::Op('/'));
                i += 1;
            }
            '%' => {
                out.push(Tok::Pct);
                i += 1;
            }
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            '^' => {
                out.push(Tok::Op('^'));
                i += 1;
            }
            '²' => {
                out.push(Tok::Sq);
                i += 1;
            }
            'π' => {
                out.push(Tok::Num(std::f64::consts::PI));
                i += 1;
            }
            '√' => {
                out.push(Tok::Func("sqrt"));
                i += 1;
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < cs.len() && cs[i].is_alphabetic() {
                    i += 1;
                }
                let w: String = cs[start..i].iter().collect::<String>().to_lowercase();
                out.push(match w.as_str() {
                    "sin" | "sen" => Tok::Func("sin"),
                    "cos" => Tok::Func("cos"),
                    "tan" | "tg" => Tok::Func("tan"),
                    "ln" => Tok::Func("ln"),
                    "log" => Tok::Func("log"),
                    "sqrt" | "raiz" => Tok::Func("sqrt"),
                    "pi" => Tok::Num(std::f64::consts::PI),
                    "e" => Tok::Num(std::f64::consts::E),
                    "x" => Tok::Op('*'),
                    _ => return None,
                });
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }
    /// sum := term (('+'|'-') term[%])*  — "a + b%" is a + a·b/100
    fn sum(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            if op != '+' && op != '-' {
                break;
            }
            self.next();
            let (r, pct) = self.term_pct()?;
            let r = if pct { v * r / 100.0 } else { r };
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    /// a term that may end in %, telling so (for the sum's rule)
    fn term_pct(&mut self) -> Option<(f64, bool)> {
        let start = self.i;
        let v = self.term()?;
        // a single factor with % right after it: the sum takes it as a percent of what came before
        let simple = self.i == start + 2 && matches!(self.t.get(start + 1), Some(Tok::Pct));
        if simple {
            return Some((v * 100.0, true));
        }
        Some((v, false))
    }
    /// term := power (('*'|'/') power)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            if op != '*' && op != '/' {
                break;
            }
            self.next();
            let r = self.power()?;
            v = if op == '*' { v * r } else { v / r };
        }
        Some(v)
    }

    /// power := factor ['^' power]  (right to left: 2^3^2 is 2^9)
    fn power(&mut self) -> Option<f64> {
        let b = self.factor()?;
        if matches!(self.peek(), Some(Tok::Op('^'))) {
            self.next();
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    /// factor := ['-'] (number | '(' sum ')') ['%']
    fn factor(&mut self) -> Option<f64> {
        let neg = if matches!(self.peek(), Some(Tok::Op('-'))) {
            self.next();
            true
        } else {
            false
        };
        let mut v = self.primary()?;
        loop {
            match self.peek() {
                Some(Tok::Pct) => {
                    self.next();
                    v /= 100.0;
                }
                Some(Tok::Sq) => {
                    self.next();
                    v *= v;
                }
                _ => break,
            }
        }
        Some(if neg { -v } else { v })
    }

    /// a number, a parenthesis, or a function of one ("sin(30)", "√9")
    fn primary(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(n) => Some(n),
            Tok::Open => {
                let v = self.sum()?;
                // (a parenthesis left open at the end closes itself, as on a calculator)
                if self.peek().is_some() && self.next()? != Tok::Close {
                    return None;
                }
                Some(v)
            }
            Tok::Func(f) => {
                let x = self.factor()?;
                let rad = x.to_radians();
                Some(match f {
                    "sin" => rad.sin(),
                    "cos" => rad.cos(),
                    "tan" => rad.tan(),
                    "ln" => x.ln(),
                    "log" => x.log10(),
                    _ => x.sqrt(),
                })
            }
            _ => None,
        }
    }
}

/// The value of an expression ("1.240 × 15%", "(2+3)÷4", "200 + 10%"), or None when it does not make sense.
pub fn eval(expr: &str) -> Option<f64> {
    let t = tokens(expr)?;
    if t.is_empty() {
        return None;
    }
    let mut p = Parser { t, i: 0 };
    let v = p.sum()?;
    if p.i != p.t.len() {
        return None;
    }
    Some(v)
}

/// "15% de 1.240" (and "15 por cento de 1240"): the percent of a number, without the model.
pub fn percent_of(text: &str) -> Option<f64> {
    let t = text.to_lowercase().replace("por cento", "%");
    let (a, b) = t.split_once("% de ")?;
    let a = parse_number(a.trim().rsplit(' ').next()?)?;
    let b = parse_number(b.trim().split(' ').next()?)?;
    Some(a * b / 100.0)
}

// ------------------------------------------------------------------------------------------------- conversions

/// A unit: id, its name (plural, singular) and how many of the category's base unit one is.
pub struct Unit {
    pub id: &'static str,
    pub plural: &'static str,
    pub singular: &'static str,
    pub factor: f64,
    /// what people write for it ("km", "milhas")
    pub aliases: &'static [&'static str],
}

const fn u(id: &'static str, plural: &'static str, singular: &'static str, factor: f64, aliases: &'static [&'static str]) -> Unit {
    Unit { id, plural, singular, factor, aliases }
}

/// The categories (design "Calculadora · Conversões") and their units; money's factors come from the day's rates.
pub const CATEGORIES: &[(&str, &str, &[Unit])] = &[
    ("moeda", "Moeda", &[
        u("BRL", "Reais", "real", 0.0, &["real", "reais", "brl", "r$"]), u("USD", "Dólares", "dólar", 0.0, &["dólar", "dólares", "dolar", "dolares", "usd", "us$"]),
        u("EUR", "Euros", "euro", 0.0, &["euro", "euros", "eur", "€"]), u("GBP", "Libras", "libra", 0.0, &["libra", "libras", "gbp", "£"]),
        u("ARS", "Pesos argentinos", "peso argentino", 0.0, &["ars"]), u("JPY", "Ienes", "iene", 0.0, &["iene", "ienes", "jpy"]),
    ]),
    ("comprimento", "Comprimento", &[
        u("mm", "Milímetros", "milímetro", 0.001, &["mm", "milímetro", "milímetros"]), u("cm", "Centímetros", "centímetro", 0.01, &["cm", "centímetro", "centímetros"]),
        u("m", "Metros", "metro", 1.0, &["m", "metro", "metros"]), u("km", "Quilômetros", "quilômetro", 1000.0, &["km", "quilômetro", "quilômetros", "quilometro", "quilometros"]),
        u("in", "Polegadas", "polegada", 0.0254, &["pol", "polegada", "polegadas", "in"]), u("ft", "Pés", "pé", 0.3048, &["pé", "pés", "ft"]),
        u("mi", "Milhas", "milha", 1609.344, &["mi", "milha", "milhas"]),
    ]),
    ("massa", "Massa", &[
        u("g", "Gramas", "grama", 0.001, &["g", "grama", "gramas"]), u("kg", "Quilos", "quilo", 1.0, &["kg", "quilo", "quilos", "quilograma", "quilogramas"]),
        u("t", "Toneladas", "tonelada", 1000.0, &["t", "tonelada", "toneladas"]), u("oz", "Onças", "onça", 0.028349523125, &["oz", "onça", "onças"]),
        u("lb", "Libras-peso", "libra-peso", 0.45359237, &["lb", "lbs", "libra-peso"]),
    ]),
    ("temperatura", "Temperatura", &[
        u("C", "Graus Celsius", "grau Celsius", 0.0, &["°c", "c", "celsius"]), u("F", "Graus Fahrenheit", "grau Fahrenheit", 0.0, &["°f", "f", "fahrenheit"]),
        u("K", "Kelvin", "kelvin", 0.0, &["k", "kelvin"]),
    ]),
    ("volume", "Volume", &[
        u("ml", "Mililitros", "mililitro", 0.001, &["ml", "mililitro", "mililitros"]), u("l", "Litros", "litro", 1.0, &["l", "litro", "litros"]),
        u("m3", "Metros cúbicos", "metro cúbico", 1000.0, &["m3", "m³"]), u("gal", "Galões (EUA)", "galão", 3.785411784, &["gal", "galão", "galões"]),
        u("xic", "Xícaras", "xícara", 0.24, &["xícara", "xícaras", "xicara", "xicaras"]),
    ]),
    ("dados", "Dados", &[
        u("B", "Bytes", "byte", 1.0, &["b", "byte", "bytes"]), u("KB", "Kilobytes", "kilobyte", 1e3, &["kb", "kilobyte", "kilobytes"]),
        u("MB", "Megabytes", "megabyte", 1e6, &["mb", "megabyte", "megabytes"]), u("GB", "Gigabytes", "gigabyte", 1e9, &["gb", "gigabyte", "gigabytes"]),
        u("TB", "Terabytes", "terabyte", 1e12, &["tb", "terabyte", "terabytes"]), u("GiB", "Gibibytes", "gibibyte", 1073741824.0, &["gib"]),
    ]),
];

impl Unit {
    /// its name for an amount other than one (the plural, lower case)
    pub fn singular_or_plural(&self) -> String {
        self.plural.to_lowercase()
    }
}

pub fn units(cat: &str) -> &'static [Unit] {
    CATEGORIES.iter().find(|(c, _, _)| *c == cat).map(|(_, _, u)| *u).unwrap_or(&[])
}

/// `v` from one unit to another; money with the rates (units per euro), temperature by its formulas.
pub fn convert(cat: &str, from: &str, to: &str, v: f64, rates: &std::collections::HashMap<String, f64>) -> Option<f64> {
    if cat == "temperatura" {
        let c = match from { "C" => v, "F" => (v - 32.0) * 5.0 / 9.0, "K" => v - 273.15, _ => return None };
        return Some(match to { "C" => c, "F" => c * 9.0 / 5.0 + 32.0, "K" => c + 273.15, _ => return None });
    }
    if cat == "moeda" {
        let r = |c: &str| if c == "EUR" { Some(1.0) } else { rates.get(c).copied() };
        return Some(v / r(from)? * r(to)?);
    }
    let us = units(cat);
    let f = us.iter().find(|u| u.id == from)?.factor;
    let t = us.iter().find(|u| u.id == to)?.factor;
    Some(v * f / t)
}

/// "100 km em milhas", "5 dólares para reais": (category, from, to, value).
pub fn parse_conversion(text: &str) -> Option<(&'static str, &'static str, &'static str, f64)> {
    let t = text.to_lowercase().replace(" para ", " em ").replace(" in ", " em ");
    let (left, right) = t.rsplit_once(" em ")?;
    let left = left.trim();
    let num_end = left.find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.' || c == '-')).unwrap_or(left.len());
    let v = parse_number(&left[..num_end])?;
    let from_w = left[num_end..].trim();
    let to_w = right.trim().trim_end_matches(['.', '?', '!']);
    for (cat, _, us) in CATEGORIES {
        let find = |w: &str| us.iter().find(|u| u.aliases.contains(&w) || u.id.to_lowercase() == w || u.plural.to_lowercase() == w).map(|u| u.id);
        if let (Some(f), Some(to)) = (find(from_w), find(to_w)) {
            return Some((cat, f, to, v));
        }
    }
    None
}

/// The currency a sentence asks the result in ("… em dólares" → "US$ "), to show with it.
pub fn currency_prefix(text: &str) -> &'static str {
    let t = text.to_lowercase();
    let Some((_, after)) = t.rsplit_once(" em ") else { return "" };
    match after.trim().trim_end_matches(['.', '?', '!']) {
        a if a.starts_with("dólar") || a.starts_with("dolar") || a == "usd" => "US$ ",
        a if a.starts_with("euro") || a == "eur" => "€ ",
        a if a.starts_with("rea") || a == "brl" => "R$ ",
        a if a.starts_with("libra") || a == "gbp" => "£ ",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brazilian_numbers() {
        assert_eq!(format(1240.0), "1.240");
        assert_eq!(format(186.0), "186");
        assert_eq!(format(-1234567.5), "-1.234.567,5");
        assert_eq!(format(0.1 + 0.2), "0,3");
        assert_eq!(parse_number("1.240,5"), Some(1240.5));
        assert_eq!(parse_number("1.240"), Some(1240.0));
        assert_eq!(parse_number("2.5"), Some(2.5));
    }

    #[test]
    fn operations_and_percent() {
        assert_eq!(eval("1.240 × 15%"), Some(186.0));
        assert_eq!(eval("200 + 10%"), Some(220.0));
        assert_eq!(eval("200 − 10%"), Some(180.0));
        assert_eq!(eval("2 + 3 × 4"), Some(14.0));
        assert_eq!(eval("(2 + 3) ÷ 4"), Some(1.25));
        assert_eq!(eval("-5 + 2"), Some(-3.0));
        assert_eq!(eval("2 +"), None);
        assert!(eval("1 ÷ 0").is_some_and(|v| v.is_infinite()));
        assert_eq!(percent_of("15% de 1.240 em dólares"), Some(186.0));
        assert_eq!(currency_prefix("15% de 1.240 em dólares"), "US$ ");
        assert_eq!(currency_prefix("15% de 1.240"), "");
    }

    #[test]
    fn scientific() {
        let close = |a: Option<f64>, b: f64| a.is_some_and(|v| (v - b).abs() < 1e-9);
        assert!(close(eval("sin(30)"), 0.5));
        assert!(close(eval("√2 × π"), 2f64.sqrt() * std::f64::consts::PI));
        assert!(close(eval("2^3^2"), 512.0));
        assert!(close(eval("3² + 4²"), 25.0));
        assert!(close(eval("log(1000)"), 3.0));
        assert!(close(eval("ln(e)"), 1.0));
        assert!(close(eval("2 × (3 + 4"), 14.0));
    }

    #[test]
    fn conversions() {
        let r: std::collections::HashMap<String, f64> = [("USD".to_string(), 1.1), ("BRL".to_string(), 5.5)].into_iter().collect();
        let close = |a: Option<f64>, b: f64| a.is_some_and(|v| (v - b).abs() < 1e-3);
        assert!(close(convert("comprimento", "km", "mi", 100.0, &r), 62.1371));
        assert!(close(convert("temperatura", "C", "F", 100.0, &r), 212.0));
        assert!(close(convert("moeda", "BRL", "USD", 5.0, &r), 1.0));
        assert_eq!(parse_conversion("100 km em milhas"), Some(("comprimento", "km", "mi", 100.0)));
        assert_eq!(parse_conversion("5 dólares para reais").map(|x| (x.0, x.1, x.2)), Some(("moeda", "USD", "BRL")));
    }
}
