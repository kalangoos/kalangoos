//! Fenced code blocks in a model's markdown, read the way models write them.
//!
//! The rules (CommonMark's, plus one for nesting):
//! - A fence opens with three or more "`" or "~" (any indentation), then an optional info string (the language and,
//!   for code, a file). A backtick fence's info cannot hold a backtick ("``` x ```" on one line is inline code).
//! - It closes only with a bare fence (no info) of the same character, at least as long as the opening one.
//! - **Nesting.** Inside a block, a fence line *with* an info string never closes it (CommonMark says the same): it
//!   opens a nested level, and the next bare fence closes that level, not the outer block. Models wrap the diagram they
//!   show in a markdown block ("```markdown", "```mermaid", the diagram, "```", "```"), or a LaTeX document holds a
//!   fenced example in a `verbatim`: both stay one block with their inner fences as text. (Plain CommonMark closes the
//!   outer block at the inner "```": the first half rendered empty and the rest leaked out as paragraphs.)
//! - A block still open at the end of the text (an answer still arriving) is returned with `closed: false`: it shows
//!   as code that grows, and is drawn (diagram, formula) once its closing fence arrives.
//! - Windows line ends ("\r\n") count as line ends; the opening fence's indentation is taken off its lines.

/// A fenced block: its info string, its text (without the fences) and whether its closing fence was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fence {
    pub info: String,
    pub body: String,
    pub closed: bool,
}

/// The markdown cut at its fenced blocks: runs of plain lines and the blocks between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(Vec<&'a str>),
    Code(Fence),
}

/// A fence line: its character, how many, its indentation and its info string.
struct FenceLine<'a> {
    ch: char,
    len: usize,
    indent: usize,
    info: &'a str,
}

fn fence_line(line: &str) -> Option<FenceLine<'_>> {
    let t = line.trim_start();
    let indent = line.len() - t.len();
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    let info = t[len..].trim();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some(FenceLine { ch, len, indent, info })
}

/// A line without up to `n` leading spaces (the opening fence's indentation).
fn dedent(line: &str, n: usize) -> &str {
    let k = line.bytes().take(n).take_while(|b| *b == b' ' || *b == b'\t').count();
    &line[k..]
}

/// Cuts `md` at its fenced blocks (see the module's rules).
pub fn split(md: &str) -> Vec<Piece<'_>> {
    let lines: Vec<&str> = md.lines().map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut out = Vec::new();
    let mut text: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(open) = fence_line(lines[i]) else {
            text.push(lines[i]);
            i += 1;
            continue;
        };
        if !text.is_empty() {
            out.push(Piece::Text(std::mem::take(&mut text)));
        }
        // the levels open: the block's own fence, then any nested one
        let mut levels = vec![(open.ch, open.len)];
        let mut body = Vec::new();
        let mut closed = false;
        i += 1;
        while i < lines.len() {
            let l = lines[i];
            i += 1;
            if let Some(f) = fence_line(l) {
                if !f.info.is_empty() {
                    levels.push((f.ch, f.len));
                } else if levels.last().is_some_and(|(c, n)| f.ch == *c && f.len >= *n) {
                    levels.pop();
                    if levels.is_empty() {
                        closed = true;
                        break;
                    }
                }
            }
            body.push(dedent(l, open.indent));
        }
        out.push(Piece::Code(Fence { info: open.info.to_string(), body: body.join("\n"), closed }));
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    out
}

/// The one closed block a text is made of, blank lines around it allowed (a markdown block that only wraps a
/// diagram is that diagram).
pub fn only_block(md: &str) -> Option<Fence> {
    let mut found = None;
    for p in split(md) {
        match p {
            Piece::Text(l) if l.iter().all(|l| l.trim().is_empty()) => {}
            Piece::Code(f) if f.closed && found.is_none() => found = Some(f),
            _ => return None,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code<'a>(p: &'a Piece<'a>) -> &'a Fence {
        match p {
            Piece::Code(f) => f,
            Piece::Text(t) => panic!("text, not a block: {t:?}"),
        }
    }

    #[test]
    fn info_string_and_tildes() {
        let p = split("antes\n```mermaid\nflowchart TD\nA-->B\n```\ndepois");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], Piece::Text(vec!["antes"]));
        assert_eq!(code(&p[1]), &Fence { info: "mermaid".into(), body: "flowchart TD\nA-->B".into(), closed: true });
        assert_eq!(p[2], Piece::Text(vec!["depois"]));
        let p = split("~~~python title=\"a.py\"\nprint(1)\n```\n~~~");
        assert_eq!(code(&p[0]).info, "python title=\"a.py\"");
        // a backtick fence does not close a tilde one
        assert_eq!(code(&p[0]).body, "print(1)\n```");
        assert!(code(&p[0]).closed);
    }

    #[test]
    fn closes_with_the_same_char_and_at_least_the_same_length() {
        let p = split("````md\n```\nx\n````");
        assert_eq!(p.len(), 1);
        assert_eq!(code(&p[0]).body, "```\nx");
        let p = split("```\na\n`````\nb");
        assert_eq!(code(&p[0]).body, "a");
        assert_eq!(p[1], Piece::Text(vec!["b"]));
        // "``` inline ```" on a line is not a fence; neither is "``"
        assert!(split("use ``` x ``` aqui\n``x``").iter().all(|p| matches!(p, Piece::Text(_))));
        assert!(split("``` x ```").iter().all(|p| matches!(p, Piece::Text(_))));
    }

    #[test]
    fn nested_mermaid_in_markdown() {
        // what the model wrote (bug of 2026-11-21: an empty "Markdown" block, the diagram as paragraphs)
        let md = "### 2️⃣ Exemplo básico – Fluxograma de tarefas\n```markdown\n```mermaid\nflowchart TD\nA[Iniciar] --> B[Definir requisitos]\n```\n```\n**O que o código faz:**";
        let p = split(md);
        assert_eq!(p.len(), 3, "{p:?}");
        let f = code(&p[1]);
        assert_eq!(f.info, "markdown");
        assert!(f.closed);
        assert_eq!(f.body, "```mermaid\nflowchart TD\nA[Iniciar] --> B[Definir requisitos]\n```");
        assert_eq!(p[2], Piece::Text(vec!["**O que o código faz:**"]));
        let inner = only_block(&f.body).unwrap();
        assert_eq!(inner.info, "mermaid");
        assert_eq!(inner.body, "flowchart TD\nA[Iniciar] --> B[Definir requisitos]");
        // a markdown block with text around its fence is markdown source, not one diagram
        assert!(only_block("Veja:\n```mermaid\nA-->B\n```").is_none());
        // a LaTeX document with a fenced example in a verbatim stays one block
        let p = split("```latex\n\\begin{verbatim}\n```mermaid\nflowchart TD\n```\n\\end{verbatim}\n\\end{document}\n```\nComo compilar");
        assert_eq!(p.len(), 2);
        assert!(code(&p[0]).body.ends_with("\\end{document}"));
    }

    #[test]
    fn streaming_and_line_ends() {
        // an answer still arriving: the open block is everything after its fence
        let p = split("Aqui:\n```mermaid\nflowchart LR\nA --> B");
        assert_eq!(code(&p[1]), &Fence { info: "mermaid".into(), body: "flowchart LR\nA --> B".into(), closed: false });
        // the same answer once its fence arrives
        let p = split("Aqui:\n```mermaid\nflowchart LR\nA --> B\n```");
        assert!(code(&p[1]).closed);
        // Windows line ends, and an indented block (in a list) loses its indentation
        let p = split("```mermaid\r\nflowchart TD\r\nA-->B\r\n```\r\n");
        assert_eq!(code(&p[0]).body, "flowchart TD\nA-->B");
        let p = split("1. Rode:\n   ```bash\n   cargo test\n     --release\n   ```");
        assert_eq!(code(&p[1]).body, "cargo test\n  --release");
        assert!(code(&p[1]).closed);
    }
}
