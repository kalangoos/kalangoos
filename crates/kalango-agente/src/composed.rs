//! An action the person turned off cannot be rebuilt out of others. Today that is moving files: with
//! `arquivos.mover` set to "never" the agent may still read, write and trash files, but not chain them into a
//! move, in either order: trashing a file whose content it already wrote somewhere else, or writing out the
//! content of a file it already trashed. The prompt asks the model not to try; this makes it so.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct MoveGuard {
    /// path → content, from what the agent read in this conversation
    read: HashMap<String, String>,
    /// sources whose content was written to another path
    copied: HashSet<String>,
    /// paths sent to the trash
    trashed: HashSet<String>,
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The written text carries the file's content (the same, or the whole of it inside).
fn carries(written: &str, content: &str) -> bool {
    let (w, c) = (norm(written), norm(content));
    !c.is_empty() && (w == c || (c.chars().count() >= 16 && w.contains(&c)))
}

fn path_arg(args: &Value) -> Option<&str> {
    args["caminho"].as_str()
}

/// The content of an `arquivos.ler` result (its JSON, or the plain text).
fn read_content(result: &str) -> String {
    serde_json::from_str::<Value>(result).ok().and_then(|v| v["conteudo"].as_str().map(str::to_string)).unwrap_or_else(|| result.to_string())
}

impl MoveGuard {
    /// Before a call. `Some(path)` refuses it: going ahead would move that file.
    pub fn check(&self, tool: &str, args: &Value, move_off: bool) -> Option<String> {
        if !move_off {
            return None;
        }
        match tool {
            "arquivos.lixeira" => path_arg(args).filter(|p| self.copied.contains(*p)).map(str::to_string),
            "arquivos.escrever" => {
                let dest = path_arg(args)?;
                let written = args["conteudo"].as_str()?;
                self.trashed.iter().find(|src| *src != dest && self.read.get(*src).is_some_and(|c| carries(written, c))).cloned()
            }
            _ => None,
        }
    }

    /// After a call that worked: what it means for the next ones.
    pub fn record(&mut self, tool: &str, args: &Value, result: &str) {
        let Some(path) = path_arg(args) else { return };
        match tool {
            "arquivos.ler" => {
                self.read.insert(path.to_string(), read_content(result));
            }
            "arquivos.escrever" => {
                if let Some(written) = args["conteudo"].as_str() {
                    let sources: Vec<String> = self.read.iter().filter(|(src, c)| *src != path && carries(written, c)).map(|(s, _)| s.clone()).collect();
                    self.copied.extend(sources);
                }
            }
            "arquivos.lixeira" => {
                self.trashed.insert(path.to_string());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn copy_then_trash_is_refused_when_moving_is_off() {
        let mut g = MoveGuard::default();
        g.record("arquivos.ler", &json!({"caminho": "/a/lista.txt"}), &json!({"conteudo": "pão, leite, café"}).to_string());
        g.record("arquivos.escrever", &json!({"caminho": "/a/x.txt", "conteudo": "pão, leite, café\n"}), "{}");
        let trash = json!({"caminho": "/a/lista.txt"});
        assert_eq!(g.check("arquivos.lixeira", &trash, true).as_deref(), Some("/a/lista.txt"));
        assert_eq!(g.check("arquivos.lixeira", &trash, false), None, "moving allowed: nothing to refuse");
        // trashing another file is fine
        assert_eq!(g.check("arquivos.lixeira", &json!({"caminho": "/a/outro.txt"}), true), None);
    }

    #[test]
    fn trash_then_rewrite_is_refused_when_moving_is_off() {
        let mut g = MoveGuard::default();
        g.record("arquivos.ler", &json!({"caminho": "/a/nota.md"}), &json!({"conteudo": "Reunião às 15h com a equipe de design."}).to_string());
        g.record("arquivos.lixeira", &json!({"caminho": "/a/nota.md"}), "{}");
        let write = json!({"caminho": "/b/nota.md", "conteudo": "# Nota\nReunião às 15h com a equipe de design."});
        assert_eq!(g.check("arquivos.escrever", &write, true).as_deref(), Some("/a/nota.md"));
        // writing something else is fine
        assert_eq!(g.check("arquivos.escrever", &json!({"caminho": "/b/outra.md", "conteudo": "outra coisa"}), true), None);
    }
}
