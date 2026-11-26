//! Guard for what the AI model reads. Text that goes into the model's `messages` or system prompt is written with
//! `tr_model`/`trf_model` (English outside the languages tested with the real model), never with `tr`/`trf`: a
//! refusal note in Japanese is a refusal the small model may not obey. The files below build that text, so in them a
//! plain `tr(`/`trf(` is allowed only on a line that is visibly for the person: a status or progress line, a tool's
//! summary, a tool description (all of them go through `for_model` when offered to the model), or a line marked `// ui` (for text the person sees that is also handed to the model through
//! `kalango_i18n::for_model`, which turns it back into English). A new `tr(` in these files without one of those
//! fails this test, which is what makes the choice explicit. It reads the sources, so it cannot follow a translated
//! string through a variable: a `// ui` line whose value reaches `messages` must pass through `for_model`.

#[cfg(test)]
mod tests {
    /// Files that build model messages or prompts.
    const FILES: &[&str] = &["motor.rs", "remote.rs"];
    /// What marks a plain translation call as for the person.
    const UI_MARKERS: &[&str] = &["Event::Status(", "on_status(", "summary:", "description:", "// ui"];

    /// The plain `tr(`/`trf(` calls (not `tr_model`, not `str(`) on a line, ignoring comments.
    fn plain_calls(line: &str) -> usize {
        let code = line.split("//").next().unwrap_or("");
        let b = code.as_bytes();
        let mut n = 0;
        for (i, _) in code.match_indices(&format!("{}(", "tr")[..]).chain(code.match_indices(&format!("{}(", "trf")[..])) {
            if i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn plain_call_detection() {
        assert_eq!(plain_calls("x = tr(\"a\");"), 1);
        assert_eq!(plain_calls("x = trf(\"a\", &[]);"), 1);
        assert_eq!(plain_calls("x = tr_model(\"a\");"), 0);
        assert_eq!(plain_calls("x = trf_model(\"a\", &[]);"), 0);
        assert_eq!(plain_calls("s.push_str(\"a\"); // tr(\"b\")"), 0);
        assert_eq!(plain_calls("format!(\"{}\", tr(\"a\"), trf(\"b\", &[]))"), 2);
    }

    #[test]
    fn model_text_never_uses_the_person_translation() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = Vec::new();
        for f in FILES {
            let text = std::fs::read_to_string(dir.join(f)).unwrap();
            for (i, line) in text.lines().enumerate() {
                if line.starts_with("mod tests") {
                    break;
                }
                if plain_calls(line) > 0 && !UI_MARKERS.iter().any(|m| line.contains(m)) {
                    bad.push(format!("{f}:{}: {}", i + 1, line.trim()));
                }
            }
        }
        assert!(bad.is_empty(), "text for the model must use tr_model/trf_model (or mark the line `// ui` and pass it through for_model):\n{}", bad.join("\n"));
    }
}
