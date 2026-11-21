//! Linha do tempo: registro só de acréscimo de cada chamada, com o inverso quando a ação é reversível.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub time: u64,
    pub caller: String,
    pub uid: u32,
    pub pid: i32,
    pub tool: String,
    pub level: String,
    pub args: Value,
    /// "ok", "negado", "erro", "simulado", "aguardando", "desfeito"
    pub status: String,
    pub summary: String,
    /// Chamada que desfaz esta (ferramenta + argumentos), quando existe.
    pub inverse: Option<Value>,
    pub restore_point: Option<String>,
    /// A small picture of the screen after an action on it (`tela.*`): the file's name in the state folder's
    /// `tela/`, read back through `kalango/miniatura` by its owner only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miniatura: Option<String>,
}

pub struct Timeline {
    path: PathBuf,
    lock: Mutex<u64>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Timeline {
    pub fn open(dir: &Path) -> std::io::Result<Timeline> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("linha-do-tempo.jsonl");
        let last = Self::read_all_from(&path).last().map(|e| e.id).unwrap_or(0);
        Ok(Timeline { path, lock: Mutex::new(last) })
    }

    fn read_all_from(path: &Path) -> Vec<Entry> {
        let Ok(f) = std::fs::File::open(path) else { return vec![] };
        BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
    }

    pub fn append(&self, mut e: Entry) -> u64 {
        let mut last = self.lock.lock().unwrap();
        *last += 1;
        e.id = *last;
        e.time = now();
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default());
        }
        e.id
    }

    pub fn all(&self) -> Vec<Entry> {
        let _g = self.lock.lock().unwrap();
        Self::read_all_from(&self.path)
    }

    pub fn get(&self, id: u64) -> Option<Entry> {
        self.all().into_iter().find(|e| e.id == id)
    }

    /// The ids already undone (each "desfeito" record points at the one it undid).
    pub fn undone_ids(&self) -> std::collections::BTreeSet<u64> {
        self.all().iter().filter(|e| e.status == "desfeito").filter_map(|e| e.args.get("id").and_then(Value::as_u64)).collect()
    }

    pub fn is_undone(&self, id: u64) -> bool {
        self.all().iter().any(|e| e.status == "desfeito" && e.args.get("id").and_then(Value::as_u64) == Some(id))
    }
}
