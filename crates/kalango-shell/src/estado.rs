//! Estado do shell, alimentado pelo niri (espaços e janelas) e pelo kalangod (pedidos de aprovação).
//! Tudo roda na thread da interface; as threads de leitura mandam mensagens por canal.

use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Espaco {
    pub id: u64,
    pub idx: u64,
    pub saida: String,
    pub ativo: bool,
    pub focado: bool,
    /// the name the person gave it (niri's named workspaces)
    pub nome: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Janela {
    pub id: u64,
    pub app_id: String,
    pub titulo: String,
    pub focada: bool,
    /// quando recebeu o foco pela última vez (para escolher qual janela do app focar)
    pub foco_em: u64,
    /// the workspace it is on
    pub espaco: Option<u64>,
    /// the window's size and, when it is in view, where its tile is on the workspace (logical pixels)
    pub tamanho: (i32, i32),
    pub pos: Option<(f64, f64)>,
    /// tiled windows: column and place in it (1-based), and the tile's size (niri gives no position for them)
    pub coluna: Option<(usize, usize)>,
    pub tile: (f64, f64),
    /// the client's process (the System Monitor groups processes by the app whose window they belong to)
    pub pid: Option<i32>,
    /// minimized (KalangoOS's niri): open, out of sight until focused again
    pub minimizada: bool,
}

#[derive(Default)]
pub struct Estado {
    pub espacos: RefCell<Vec<Espaco>>,
    pub janelas: RefCell<BTreeMap<u64, Janela>>,
    /// niri respondendo
    pub niri_ok: Cell<bool>,
    /// pedidos esperando aprovação no kalangod (None = kalangod indisponível)
    pub pendentes: Cell<Option<usize>>,
    /// the agent in the side bar is working on a request
    pub agent_working: Cell<bool>,
    /// what the agent works on and how many steps it took so far
    pub agent_task: RefCell<Option<(String, usize)>>,
    /// the latest day summary from `kalango-agente resumo` (the desktop widget and the "Hoje" bar share it)
    pub resumo_dia: RefCell<Option<serde_json::Value>>,
    ouvintes: RefCell<Vec<Box<dyn Fn()>>>,
    /// told of each window that opens (its id and app id)
    ao_abrir: RefCell<Vec<Box<dyn Fn(u64, &str)>>>,
    seq: Cell<u64>,
}

impl Estado {
    pub fn ouvir(&self, f: impl Fn() + 'static) {
        self.ouvintes.borrow_mut().push(Box::new(f));
    }

    /// Called for each window that opens from now on (not the ones already open).
    pub fn ao_abrir_janela(&self, f: impl Fn(u64, &str) + 'static) {
        self.ao_abrir.borrow_mut().push(Box::new(f));
    }

    pub fn avisar(&self) {
        for f in self.ouvintes.borrow().iter() {
            f();
        }
    }

    fn espaco(v: &Value) -> Espaco {
        Espaco {
            id: v["id"].as_u64().unwrap_or(0),
            idx: v["idx"].as_u64().unwrap_or(0),
            saida: v["output"].as_str().unwrap_or("").into(),
            ativo: v["is_active"].as_bool().unwrap_or(false),
            focado: v["is_focused"].as_bool().unwrap_or(false),
            nome: v["name"].as_str().map(str::to_string),
        }
    }

    fn janela(&self, v: &Value) -> Janela {
        let n = self.seq.get() + 1;
        self.seq.set(n);
        let focada = v["is_focused"].as_bool().unwrap_or(false);
        Janela {
            id: v["id"].as_u64().unwrap_or(0),
            app_id: v["app_id"].as_str().unwrap_or("").into(),
            titulo: v["title"].as_str().unwrap_or("").into(),
            focada,
            foco_em: if focada { n } else { 0 },
            espaco: v["workspace_id"].as_u64(),
            tamanho: (
                v["layout"]["window_size"][0].as_i64().unwrap_or(0) as i32,
                v["layout"]["window_size"][1].as_i64().unwrap_or(0) as i32,
            ),
            pos: v["layout"]["tile_pos_in_workspace_view"].as_array().and_then(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?))),
            coluna: v["layout"]["pos_in_scrolling_layout"].as_array().and_then(|p| Some((p.first()?.as_u64()? as usize, p.get(1)?.as_u64()? as usize))),
            tile: (v["layout"]["tile_size"][0].as_f64().unwrap_or(0.0), v["layout"]["tile_size"][1].as_f64().unwrap_or(0.0)),
            pid: v["pid"].as_i64().map(|p| p as i32),
            minimizada: v["is_minimized"].as_bool().unwrap_or(false),
        }
    }

    /// Aplica um evento do niri (formato do `niri msg --json event-stream`).
    pub fn evento_niri(&self, ev: &Value) {
        let Some((nome, d)) = ev.as_object().and_then(|o| o.iter().next()) else { return };
        match nome.as_str() {
            "WorkspacesChanged" => {
                let mut v: Vec<Espaco> = d["workspaces"].as_array().into_iter().flatten().map(Self::espaco).collect();
                v.sort_by_key(|e| (e.saida.clone(), e.idx));
                *self.espacos.borrow_mut() = v;
            }
            "WorkspaceActivated" => {
                let id = d["id"].as_u64().unwrap_or(0);
                let focado = d["focused"].as_bool().unwrap_or(false);
                let mut es = self.espacos.borrow_mut();
                let saida = es.iter().find(|e| e.id == id).map(|e| e.saida.clone()).unwrap_or_default();
                for e in es.iter_mut() {
                    if e.saida == saida {
                        e.ativo = e.id == id;
                    }
                    if focado {
                        e.focado = e.id == id;
                    }
                }
            }
            "WindowsChanged" => {
                let mut m = BTreeMap::new();
                for w in d["windows"].as_array().into_iter().flatten() {
                    let j = self.janela(w);
                    m.insert(j.id, j);
                }
                *self.janelas.borrow_mut() = m;
            }
            "WindowOpenedOrChanged" => {
                let j = self.janela(&d["window"]);
                let mut js = self.janelas.borrow_mut();
                if j.focada {
                    for o in js.values_mut() {
                        o.focada = false;
                    }
                }
                let antigo = js.get(&j.id).map(|o| o.foco_em).unwrap_or(0);
                let nova = !js.contains_key(&j.id);
                let mut j = j;
                j.foco_em = j.foco_em.max(antigo);
                let (id, app_id) = (j.id, j.app_id.clone());
                js.insert(j.id, j);
                drop(js);
                if nova {
                    for f in self.ao_abrir.borrow().iter() {
                        f(id, &app_id);
                    }
                }
            }
            // sizes and places only (frequent while windows move): kept quietly, nobody redraws for them
            "WindowLayoutsChanged" => {
                let mut js = self.janelas.borrow_mut();
                for c in d["changes"].as_array().into_iter().flatten() {
                    let (Some(id), l) = (c[0].as_u64(), &c[1]) else { continue };
                    if let Some(j) = js.get_mut(&id) {
                        j.tamanho = (l["window_size"][0].as_i64().unwrap_or(0) as i32, l["window_size"][1].as_i64().unwrap_or(0) as i32);
                        j.pos = l["tile_pos_in_workspace_view"].as_array().and_then(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?)));
                        j.coluna = l["pos_in_scrolling_layout"].as_array().and_then(|p| Some((p.first()?.as_u64()? as usize, p.get(1)?.as_u64()? as usize)));
                        j.tile = (l["tile_size"][0].as_f64().unwrap_or(0.0), l["tile_size"][1].as_f64().unwrap_or(0.0));
                    }
                }
                return;
            }
            "WindowClosed" => {
                self.janelas.borrow_mut().remove(&d["id"].as_u64().unwrap_or(0));
            }
            "WindowFocusChanged" => {
                let id = d["id"].as_u64();
                let n = self.seq.get() + 1;
                self.seq.set(n);
                for j in self.janelas.borrow_mut().values_mut() {
                    j.focada = Some(j.id) == id;
                    if j.focada {
                        j.foco_em = n;
                    }
                }
            }
            // the keyboard's layouts (niri's): the on-screen indicator says the switch
            "KeyboardLayoutsChanged" => {
                let k = &d["keyboard_layouts"];
                crate::osd::layouts(k["names"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)).collect(), k["current_idx"].as_u64().unwrap_or(0) as usize);
                return;
            }
            "KeyboardLayoutSwitched" => {
                crate::osd::layout_trocado(d["idx"].as_u64().unwrap_or(0) as usize);
                return;
            }
            _ => return,
        }
        self.avisar();
    }
}
