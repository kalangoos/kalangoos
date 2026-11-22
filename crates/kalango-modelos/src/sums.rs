//! `SHA256SUMS` files (the `sha256sum` format: `<64 hex>  <name>` or `<64 hex> *<name>`).

use crate::catalog::{is_sha256, safe_file_name};
use std::collections::HashMap;

/// name → lower-case hex. Lines that are not sums, and names with a path, are ignored.
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((sum, rest)) = line.split_once([' ', '\t']) else { continue };
        let sum = sum.to_ascii_lowercase();
        let name = rest.trim_start_matches([' ', '\t', '*']);
        let name = name.strip_prefix("./").unwrap_or(name);
        if is_sha256(&sum) && safe_file_name(name) {
            m.insert(name.to_string(), sum);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h() -> String {
        "3793e1e6".to_string() + &"ab".repeat(28)
    }

    #[test]
    fn reads_both_formats_and_skips_the_rest() {
        let (h, up) = (h(), h().to_uppercase());
        let t = format!("# comment\n{h}  a.gguf\n{up} *./b.gguf\r\nnot a sum\n{h}  sub/c.gguf\n{h}  ../d.gguf\n");
        let m = parse_sums(&t);
        assert_eq!(m.len(), 2);
        assert_eq!(m["a.gguf"], h);
        assert_eq!(m["b.gguf"], h);
    }
}
