//! A request as the shell sends it to the engine, with where each part of it came from. A line of the conversation is
//! a JSON string (the whole request typed) or an object `{"parts": [{"text": "…", "origin": "typed|pasted|dictated"}]}`.
//! What the person pasted came from somewhere else (a page, a mail, a chat): the traces say so, and a turn that has it
//! is not taken for the person's own words.

use serde_json::Value;

/// The request's text, and which parts of it (character ranges, half open) were pasted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Request {
    pub text: String,
    pub pasted: Vec<(usize, usize)>,
}

/// The request in a `{"parts": …}` object; None for any other line. `typed` and `dictated` are the person's own words;
/// any other origin, and a part with none, is not (a newer shell may name more kinds: when in doubt, it is external).
/// Adjacent pasted parts are one range.
pub fn from_parts(line: &Value) -> Option<Request> {
    let parts = line.get("parts")?.as_array()?;
    let mut req = Request::default();
    let mut at = 0usize;
    for p in parts {
        let text = p.get("text").and_then(Value::as_str).unwrap_or("");
        let own = matches!(p.get("origin").and_then(Value::as_str), Some("typed" | "dictated"));
        let len = text.chars().count();
        if !own && len > 0 {
            match req.pasted.last_mut() {
                Some(last) if last.1 == at => last.1 = at + len,
                _ => req.pasted.push((at, at + len)),
            }
        }
        req.text.push_str(text);
        at += len;
    }
    Some(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parts_become_the_text_and_the_pasted_ranges() {
        let r = from_parts(&json!({"parts": [
            {"text": "resuma: ", "origin": "typed"},
            {"text": "texto colado", "origin": "pasted"},
            {"text": " e depois ", "origin": "dictated"},
            {"text": "mais um", "origin": "pasted"},
        ]}))
        .unwrap();
        assert_eq!(r.text, "resuma: texto colado e depois mais um");
        assert_eq!(r.pasted, [(8, 20), (30, 37)]);
        assert_eq!(&r.text.chars().skip(8).take(12).collect::<String>(), "texto colado");
    }

    #[test]
    fn ranges_count_characters_not_bytes_and_adjacent_pastes_join() {
        let r = from_parts(
            &json!({"parts": [{"text": "é ", "origin": "typed"}, {"text": "ção", "origin": "pasted"}, {"text": "ão", "origin": "pasted"}]}),
        )
        .unwrap();
        assert_eq!(r.text, "é çãoão");
        assert_eq!(r.pasted, [(2, 7)]);
    }

    #[test]
    fn an_unknown_or_missing_origin_is_not_the_persons_own() {
        let r = from_parts(&json!({"parts": [{"text": "a", "origin": "typed"}, {"text": "b", "origin": "dropped"}, {"text": "c"}]})).unwrap();
        assert_eq!(r.pasted, [(1, 3)]);
    }

    #[test]
    fn other_lines_are_not_parts() {
        assert!(from_parts(&json!("um pedido")).is_none());
        assert!(from_parts(&json!({"web": false})).is_none());
        assert!(from_parts(&json!({"parts": "x"})).is_none());
        // an empty list is a request with no text and nothing pasted
        assert_eq!(from_parts(&json!({"parts": []})).unwrap(), Request::default());
    }
}
