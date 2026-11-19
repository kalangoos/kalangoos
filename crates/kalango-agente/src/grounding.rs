//! An answer from the web says only what the pages said. A small model completes lists from memory — a table of six
//! goals for a 4 × 0 game, a club's contract nobody read (19/11/2026). Before an answer goes out, the names and minutes
//! it states are looked for in what the agent read in the conversation; when some are nowhere there, the model is told
//! which, and rewrites it once.

/// The facts of an answer that none of the sources has: people's and places' names (two or more words with capitals)
/// and a match's minutes (37').
pub fn unsupported(answer: &str, sources: &str, asked: &str) -> Vec<String> {
    let norm = |s: &str| {
        s.to_lowercase()
            .replace(['’', '′', '´', '`'], "'")
            .replace(['\u{a0}', '\u{202f}', '\u{2009}'], " ")
            .replace(['‑', '–'], "-")
    };
    let src = norm(sources);
    let asked = norm(asked);
    let known = |f: &str| {
        let f = norm(f);
        src.contains(&f) || asked.contains(&f)
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |f: String| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    let text = answer.replace(['*', '_', '|', '#', '`'], " ");
    // scores: one the pages do not have — or have the other way round ("0 – 4" for a 4 × 0) — is the worst mistake
    let known_scores: Vec<(u32, u32)> = scores(&src).into_iter().chain(scores(&asked)).collect();
    for p in scores(&norm(&text)) {
        if !known_scores.contains(&p) {
            push(trf_score(p));
        }
    }
    // minutes
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() && (i == 0 || !cs[i - 1].is_alphanumeric()) {
            let start = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i - start <= 3 && i < cs.len() && matches!(cs[i], '\'' | '’' | '′') {
                let m: String = cs[start..i].iter().collect();
                if !known(&format!("{m}'")) {
                    push(format!("{m}'"));
                }
            }
        } else {
            i += 1;
        }
    }
    // names: runs of capitalized words (a sentence's first word alone does not make one)
    for line in text.lines() {
        let mut run: Vec<String> = Vec::new();
        let flush = |run: &mut Vec<String>, push: &mut dyn FnMut(String)| {
            if run.len() >= 2 {
                let name = run.join(" ");
                if !known(&name) {
                    push(name);
                }
            }
            run.clear();
        };
        // (punctuation ends a name: "Arena. Gols: Kaio César" is two)
        for token in line.split_whitespace() {
            if token.starts_with(['(', '[', '“', '"', '—', '–']) {
                flush(&mut run, &mut push);
            }
            let core = token.trim_matches(|c: char| !c.is_alphanumeric());
            let capital = core.chars().next().is_some_and(char::is_uppercase) && core.chars().skip(1).any(char::is_lowercase)
                && core.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'');
            if capital {
                run.push(core.to_string());
            } else {
                flush(&mut run, &mut push);
            }
            if token.ends_with(['.', ':', ';', ',', '!', '?', ')', ']', '”', '"', '—']) {
                flush(&mut run, &mut push);
            }
        }
        flush(&mut run, &mut push);
    }
    out
}

fn trf_score(p: (u32, u32)) -> String {
    format!("{} {} × {}", SCORE, p.0, p.1)
}

/// What a web result says — its pages' titles, summaries and text —, never the words the agent searched with: a
/// search for "Botafogo 0 x 4 Corinthians" would make that score look read (19/11/2026).
pub fn what_pages_say(result: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(result) else { return result.to_string() };
    let mut out = String::new();
    if let Some(rs) = v["resultados"].as_array() {
        for r in rs {
            for k in ["titulo", "resumo"] {
                out.push_str(r[k].as_str().unwrap_or(""));
                out.push('\n');
            }
        }
        if let Some(d) = v["destaque"].as_str() {
            out.push_str(d);
        }
        return out;
    }
    for k in ["titulo", "texto"] {
        out.push_str(v[k].as_str().unwrap_or(""));
        out.push('\n');
    }
    out
}

/// How a score is named in the list of what has no source (the engine sends any of them back).
pub const SCORE: &str = "placar";

/// The scores in a text: "4 x 0", "4×0", "4 – 0", "4 a 0" (not dates, times or ranges of years).
fn scores(t: &str) -> Vec<(u32, u32)> {
    let cs: Vec<char> = t.chars().collect();
    let num = |i: usize| -> Option<(u32, usize)> {
        let mut j = i;
        while j < cs.len() && cs[j].is_ascii_digit() {
            j += 1;
        }
        (j > i && j - i <= 2).then(|| (cs[i..j].iter().collect::<String>().parse().unwrap_or(0), j))
    };
    let glued = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit() || "/-.:,".contains(*c));
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() && (i == 0 || !glued(cs.get(i - 1))) {
            if let Some((a, mut j)) = num(i) {
                let start = j;
                while j < cs.len() && cs[j] == ' ' {
                    j += 1;
                }
                let sep = cs.get(j).copied();
                let spaced = j > start;
                let ok_sep = match sep {
                    Some('x' | '×' | '-' | '–' | '—') => true,
                    // ("4 a 0": only with spaces around)
                    Some('a') => spaced && cs.get(j + 1) == Some(&' '),
                    _ => false,
                };
                if ok_sep {
                    let mut k = j + 1;
                    while k < cs.len() && cs[k] == ' ' {
                        k += 1;
                    }
                    if let Some((b, end)) = num(k) {
                        let after = cs.get(end);
                        let tail_glued = after.is_some_and(|c| c.is_ascii_digit() || "/-:".contains(*c)) || (after == Some(&'.') && cs.get(end + 1).is_some_and(|c| c.is_ascii_digit()));
                        if !tail_glued && !(sep == Some('x') && !spaced && cs.get(j + 1) != Some(&' ') && !cs.get(j + 1).is_some_and(|c| c.is_ascii_digit())) {
                            out.push((a, b));
                            i = end;
                            continue;
                        }
                    }
                }
                i = start;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The sentences of a page that answer the request (most of its words), in the page's order: put first, a small model
/// reads them instead of looking for the one line in twenty thousand characters.
pub fn excerpts(page: &str, request: &str, max: usize) -> Vec<String> {
    const SKIP: &[&str] = &["qual", "quais", "quem", "quando", "onde", "como", "foram", "sobre", "para", "isso", "esse", "essa", "este", "esta",
        "ontem", "hoje", "what", "which", "when", "where", "with", "that", "this"];
    let keys: Vec<String> = request.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 4 && !SKIP.contains(&w.as_str())).collect();
    if keys.is_empty() {
        return Vec::new();
    }
    let mut sentences: Vec<(usize, usize, String)> = Vec::new();
    for (i, s) in page.split(['\n', '.', '!', '?']).map(str::trim).filter(|s| s.chars().count() >= 25).enumerate() {
        let l = s.to_lowercase();
        let n = keys.iter().filter(|k| l.contains(k.as_str())).count();
        if n >= 2.min(keys.len()) {
            sentences.push((n, i, s.chars().take(300).collect()));
        }
    }
    sentences.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    sentences.truncate(max);
    sentences.sort_by_key(|s| s.1);
    let mut out: Vec<String> = Vec::new();
    for (_, _, s) in sentences {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // (what placardefutebol.com.br said, read with its icons: 19/11/2026)
    const PAGE: &str = "Corinthians 4 x 0 Botafogo\nNeo Química Arena (São Paulo, SP)\nKaio César [Gol]\n28'\nMemphis Depay [Gol]\n34'\n37'\n[yellowcard] Alex Telles\nAndré Carrillo [yellowcard]\n49'\nKaio César [Gol]\n60'\nKaio César [Gol]\n68'";

    #[test]
    fn invented_goals_and_contracts_are_found() {
        let answer = "**Placar:** 4 × 0\n| Memphis Depay | 34' |\n| Kaio César | 64’ |\n| Huguinho | 83' |\nO jogador André está no Wolverhampton Wanderers até 2030.";
        let u = unsupported(answer, PAGE, "qual o resultado do jogo de ontem do corinthians");
        assert!(u.contains(&"64'".to_string()) && u.contains(&"83'".to_string()), "{u:?}");
        assert!(u.contains(&"Wolverhampton Wanderers".to_string()), "{u:?}");
        assert!(!u.iter().any(|f| f == "Memphis Depay" || f == "34'"), "{u:?}");
    }

    #[test]
    fn what_the_page_said_passes() {
        let answer = "Corinthians 4 × 0 Botafogo, na Neo Química Arena. Gols: Kaio César (28', 60', 68') e Memphis Depay (34'). Alex Telles e André Carrillo levaram amarelo.";
        assert!(unsupported(answer, PAGE, "").is_empty(), "{:?}", unsupported(answer, PAGE, ""));
    }

    #[test]
    fn the_sentence_that_answers_comes_first() {
        let page = "Corinthians volta a ganhar por quatro gols após dois anos e meio\nMenu\nPressionado na luta contra o rebaixamento no Campeonato Brasileiro, o Corinthians derrotou o Botafogo do técnico Tite com três gols de Kaio César e um gol contra de Arthur Chaves, em uma finalização de Memphis Depay.\nVeja também os vídeos do jogo de ontem e as notas dos jogadores";
        let e = excerpts(page, "qual o resultado do jogo de ontem do corinthians? quem fez os gols?", 6);
        assert!(e.iter().any(|s| s.contains("três gols de Kaio César e um gol contra de Arthur Chaves")), "{e:?}");
        assert!(!e.iter().any(|s| s == "Menu"));
    }

    #[test]
    fn a_score_the_other_way_round_is_found() {
        // (19/11/2026: "Corinthians × 0 – 4 Botafogo" for a 4 × 0)
        let u = unsupported("**Times:** Corinthians × 0 – 4 Botafogo. Vencedor: Corinthians.", PAGE, "");
        assert!(u.iter().any(|f| f == "placar 0 × 4"), "{u:?}");
        assert!(unsupported("Corinthians 4 a 0 Botafogo, no dia 18/11/2026 às 21:30.", PAGE, "").is_empty(), "{:?}", unsupported("Corinthians 4 a 0 Botafogo, no dia 18/11/2026 às 21:30.", PAGE, ""));
        assert_eq!(scores("4x0, 2 × 1, 3 – 2, 1 a 0, 2026-11-18, 21:30, 10.5 x 2"), vec![(4, 0), (2, 1), (3, 2), (1, 0)]);
    }

    #[test]
    fn the_words_searched_are_not_a_source() {
        let busca = r#"{"consulta": "Botafogo 0 x 4 Corinthians golos", "resultados": [{"titulo": "Corinthians 4 x 0 Botafogo: veja os gols", "url": "https://x", "resumo": "Kaio César marcou três"}]}"#;
        let fontes = what_pages_say(busca);
        assert!(!fontes.contains("0 x 4") && fontes.contains("4 x 0"), "{fontes}");
        assert!(unsupported("Placar: Corinthians 0 × 4 Botafogo", &fontes, "").iter().any(|f| f == "placar 0 × 4"));
    }

    #[test]
    fn a_sentences_first_word_is_not_a_name() {
        assert!(unsupported("Segundo a página, Kaio César marcou três.", PAGE, "").is_empty());
    }
}
