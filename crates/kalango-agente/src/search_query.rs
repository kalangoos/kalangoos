//! A web search in the request's language. A small model asked in Portuguese searches in English ("Corinthians game
//! yesterday result goals", 19/11/2026) and the engine answers with the club's generic pages; asked in the person's own
//! words, it finds the match. The instruction says so; this makes it so.

const PT: &[&str] = &["o", "a", "os", "as", "de", "do", "da", "dos", "das", "que", "qual", "quais", "quem", "quando", "onde", "ontem", "hoje", "jogo",
    "resultado", "é", "foi", "fez", "no", "na", "para", "com", "um", "uma", "e", "não", "nao", "como", "quanto", "preço", "previsão", "notícias",
    "gols", "placar", "time", "último", "ultimo", "amanhã", "semana"];
const EN: &[&str] = &["the", "yesterday", "today", "tomorrow", "result", "results", "game", "match", "goals", "goal", "score", "scorers", "who",
    "what", "when", "where", "latest", "news", "price", "weather", "scored", "of", "in", "on", "for", "last", "night", "lineup", "lineups",
    "starters", "substitutes", "substitution", "substitutions", "players", "stats", "highlights", "schedule", "standings", "table", "how",
    "is", "was", "did", "and", "with", "vs", "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "player", "team", "coach"];
/// Words of a request that say nothing to a search engine.
const ASKING: &[&str] = &["qual", "quais", "quem", "o", "a", "os", "as", "é", "foi", "me", "diga", "diz", "por", "favor", "sabe", "você", "vc",
    "voce", "pode", "poderia", "quero", "saber", "pesquise", "pesquisa", "procure", "busque", "na", "web", "internet", "e"];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '/' && c != '-').filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The search to make instead, when the request is in Portuguese and the model's search is in English.
pub fn in_request_language(request: &str, query: &str) -> Option<String> {
    let req = words(request);
    let q = words(query);
    let pt_request = req.iter().filter(|w| PT.contains(&w.as_str())).count() >= 2;
    // (one English word is enough when there is no Portuguese one: "Arthur Chaves goal Corinthians")
    let en_query = q.iter().any(|w| EN.contains(&w.as_str())) && !q.iter().any(|w| PT.contains(&w.as_str()) && !EN.contains(&w.as_str()));
    if !pt_request || !en_query {
        return None;
    }
    let mut out: Vec<String> = req.iter().filter(|w| !ASKING.contains(&w.as_str())).cloned().collect();
    // the names the model added (the opponent, a player) stay
    for w in query.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| w.chars().next().is_some_and(char::is_uppercase)) {
        let l = w.to_lowercase();
        if !out.contains(&l) && !EN.contains(&l.as_str()) && !req.contains(&l) {
            out.push(w.to_string());
        }
    }
    out.dedup();
    out.truncate(12);
    (!out.is_empty()).then(|| out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_english_search_for_a_portuguese_request_is_redone() {
        assert_eq!(in_request_language("qual o resultado do jogo de ontem do corinthians? quem fez os gols?", "Corinthians game yesterday result goals").as_deref(),
            Some("resultado do jogo de ontem do corinthians fez gols"));
        let r = in_request_language("alex teles e carilho e huiguinho nao foram as subistituicao ?", "Corinthians Botafogo 18/11/2026 lineup starters").unwrap();
        assert!(r.starts_with("alex teles carilho huiguinho") && r.contains("Corinthians") && r.contains("Botafogo"), "{r}");
        let r = in_request_language("o André Carrillo fez gol?", "Arthur Chaves goal Corinthians Botafogo 18 November 2026").unwrap();
        assert!(r.starts_with("andré carrillo fez gol") && !r.contains("November") && r.contains("Arthur"), "{r}");
    }

    #[test]
    fn the_right_language_or_an_english_request_stays() {
        assert_eq!(in_request_language("qual o resultado do jogo de ontem do corinthians", "resultado do jogo do Corinthians ontem"), None);
        assert_eq!(in_request_language("what was the score of the game yesterday", "Corinthians game yesterday result"), None);
        assert_eq!(in_request_language("qual o preço do bitcoin hoje", "bitcoin"), None);
    }
}
