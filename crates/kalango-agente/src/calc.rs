//! The agent's calculator (`calculadora.calcular`): a small model gets arithmetic wrong, so a sum, a percentage or a
//! square root is worked out here. An expression with + − × ÷ ^ and %, parentheses, a decimal comma or point, and
//! the usual functions (raiz/sqrt, sin, cos, tan, log, ln, exp, abs, round, floor, ceil) and constants (pi, e).

/// The value of an expression, or why it has none.
pub fn eval(expr: &str) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut p = Parser { t: tokens, i: 0 };
    let v = p.sum()?;
    if p.i < p.t.len() {
        return Err(format!("sobrou “{}” na expressão", p.t[p.i].text()));
    }
    if !v.is_finite() {
        return Err("o resultado não é um número (divisão por zero?)".into());
    }
    Ok(v)
}

/// A value as people read it: no ".0", at most 12 significant digits, the decimal comma when `comma`.
pub fn format(v: f64, comma: bool) -> String {
    let mut s = if v != 0.0 && (v.abs() >= 1e15 || v.abs() < 1e-9) {
        format!("{v:e}")
    } else {
        let digits = (12 - v.abs().log10().floor().max(0.0) as i32).clamp(0, 12) as usize;
        let s = format!("{v:.digits$}");
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    };
    if s == "-0" {
        s = "0".into();
    }
    if comma { s.replace('.', ",") } else { s }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    Open,
    Close,
    Name(String),
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Tok::Num(n) => n.to_string(),
            Tok::Op(c) => c.to_string(),
            Tok::Open => "(".into(),
            Tok::Close => ")".into(),
            Tok::Name(n) => n.clone(),
        }
    }
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let s = s.trim().trim_end_matches('=').trim();
    // (a decimal comma when there is no point: "2,5"; "1.234,56" is a thousands point and a decimal comma)
    let normal: String = if s.contains(',') && s.contains('.') {
        s.replace('.', "").replace(',', ".")
    } else {
        s.replace(',', ".")
    };
    let cs: Vec<char> = normal.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            let n: String = cs[start..i].iter().collect();
            out.push(Tok::Num(n.parse().map_err(|_| format!("número inválido: {n}"))?));
        } else if c.is_alphabetic() {
            let start = i;
            while i < cs.len() && cs[i].is_alphabetic() {
                i += 1;
            }
            let w: String = cs[start..i].iter().collect::<String>().to_lowercase();
            // operations in words: "3 x 4", "15% de 230", "3 vezes 4", "10 mais 2", "10 menos 2"
            match w.as_str() {
                "x" | "de" | "of" | "vezes" | "times" => out.push(Tok::Op('*')),
                "mais" | "plus" => out.push(Tok::Op('+')),
                "menos" | "minus" => out.push(Tok::Op('-')),
                _ => out.push(Tok::Name(w)),
            }
        } else {
            out.push(match c {
                '+' | '-' | '%' | '^' => Tok::Op(c),
                '*' | '×' | '·' => {
                    // "**" is a power
                    if c == '*' && cs.get(i + 1) == Some(&'*') {
                        i += 1;
                        Tok::Op('^')
                    } else {
                        Tok::Op('*')
                    }
                }
                '/' | '÷' | ':' => Tok::Op('/'),
                '−' => Tok::Op('-'),
                '(' | '[' => Tok::Open,
                ')' | ']' => Tok::Close,
                _ => return Err(format!("símbolo desconhecido: {c}")),
            });
            i += 1;
        }
    }
    if out.is_empty() {
        return Err("expressão vazia".into());
    }
    Ok(out)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut v = self.product()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.i += 1;
            let r = self.product()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut v = self.power()?;
        loop {
            match self.peek().cloned() {
                Some(Tok::Op(c @ ('*' | '/'))) => {
                    self.i += 1;
                    let r = self.power()?;
                    v = if c == '*' { v * r } else { v / r };
                }
                // an implicit product: "2(3+4)", "2pi"
                Some(Tok::Open) | Some(Tok::Name(_)) => v *= self.power()?,
                _ => break,
            }
        }
        Ok(v)
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.unary()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.i += 1;
            // (right-associative: 2^3^2 = 2^9)
            let e = self.power()?;
            return Ok(base.powf(e));
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Op('-')) => {
                self.i += 1;
                Ok(-self.unary()?)
            }
            Some(Tok::Op('+')) => {
                self.i += 1;
                self.unary()
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<f64, String> {
        let mut v = self.atom()?;
        // "15%": a hundredth; "200 + 10%" stays a hundredth (said plainly, not a shop's markup)
        while let Some(Tok::Op('%')) = self.peek() {
            self.i += 1;
            v /= 100.0;
        }
        Ok(v)
    }

    fn atom(&mut self) -> Result<f64, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(n)
            }
            Some(Tok::Open) => {
                self.i += 1;
                let v = self.sum()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err("falta fechar um parêntese".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                match n.as_str() {
                    "pi" | "π" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    f => {
                        let arg = self.postfix()?;
                        Ok(match f {
                            "sqrt" | "raiz" | "rq" => arg.sqrt(),
                            "sin" | "sen" => arg.sin(),
                            "cos" => arg.cos(),
                            "tan" | "tg" => arg.tan(),
                            "log" => arg.log10(),
                            "ln" => arg.ln(),
                            "exp" => arg.exp(),
                            "abs" => arg.abs(),
                            "round" | "arredondar" => arg.round(),
                            "floor" => arg.floor(),
                            "ceil" => arg.ceil(),
                            _ => return Err(format!("função desconhecida: {f}")),
                        })
                    }
                }
            }
            Some(t) => Err(format!("esperava um número e veio “{}”", t.text())),
            None => Err("a expressão terminou cedo".into()),
        }
    }
}

/// A request that needs arithmetic: numbers with an operation, or the words of a calculation.
pub fn wanted(request: &str) -> bool {
    let r = request.to_lowercase();
    let tem_numero = r.chars().any(|c| c.is_ascii_digit());
    let palavras = ["calcul", "quanto é", "quanto e ", "quanto dá", "quanto da ", "soma", "some ", "multiplic", "divid", "porcent", "raiz", "média", "media",
        "total", "juros", "desconto", "subtra", "vezes", "elevado", "calculate", "how much is", "percent", "sum", "average"];
    tem_numero && (palavras.iter().any(|w| r.contains(w)) || r.chars().any(|c| "+*/×÷^%=".contains(c)) || r.contains(" x ") || r.contains(" - "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> f64 {
        eval(s).unwrap()
    }

    #[test]
    fn arithmetic_as_people_write_it() {
        assert_eq!(v("2 + 3 * 4"), 14.0);
        assert_eq!(v("(2 + 3) * 4"), 20.0);
        assert_eq!(v("2^3^2"), 512.0);
        assert_eq!(v("2**10"), 1024.0);
        assert_eq!(v("-3 + 5"), 2.0);
        assert_eq!(v("1234,56 * 2"), 2469.12);
        assert_eq!(v("1.234,50 + 0,5"), 1235.0);
        assert_eq!(v("15% * 200"), 30.0);
        assert_eq!(v("3 x 4"), 12.0);
        assert_eq!(v("15% de 230"), 34.5);
        assert_eq!(v("3 vezes 4 mais 1"), 13.0);
        assert_eq!(v("10 ÷ 4"), 2.5);
        assert_eq!(v("raiz(16) + sqrt 9"), 7.0);
        assert_eq!(v("2pi"), 2.0 * std::f64::consts::PI);
        assert_eq!(v("2(3+4)"), 14.0);
        // powf and powi round differently in the last bits: compare with a tolerance
        assert!((v("1500 * (1 + 0,02)^12 =") - 1500.0 * 1.02f64.powi(12)).abs() < 1e-9);
    }

    #[test]
    fn what_is_not_an_expression_is_said() {
        assert!(eval("2 +").is_err());
        assert!(eval("(2 + 3").is_err());
        assert!(eval("1 / 0").is_err());
        assert!(eval("2 $ 3").is_err());
        assert!(eval("").is_err());
    }

    #[test]
    fn values_read_well() {
        assert_eq!(format(14.0, true), "14");
        assert_eq!(format(2469.12, true), "2469,12");
        assert_eq!(format(0.1 + 0.2, false), "0.3");
        assert_eq!(format(1.0 / 3.0, true), "0,333333333333");
        assert_eq!(format(-0.0, true), "0");
    }

    #[test]
    fn requests_that_need_it() {
        assert!(wanted("quanto é 15% de 230?"));
        assert!(wanted("some 12 e 30"));
        assert!(wanted("2+2"));
        assert!(!wanted("que horas são"));
        assert!(!wanted("organize os downloads"));
    }
}
