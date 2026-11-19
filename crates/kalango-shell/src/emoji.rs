//! Emoji and symbols (design "Emoji e símbolos (Super+.)"): a panel over the window in use. The search finds emoji by
//! their Portuguese names and keywords (Unicode CLDR, in assets/emoji-pt.tsv, made from cldr-json and emoji-test.txt),
//! the tabs show a category (the recent ones first), the symbols below; arrows choose, ↵ puts the chosen one where the
//! text cursor is (the clipboard and the virtual keyboard's Ctrl+V, as the clipboard history pastes). With nothing
//! found, ↵ asks the local model for the words of what was described ("gato rindo") and searches again.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const TABELA: &str = include_str!("../assets/emoji-pt.tsv");
const POR_LINHA: usize = 8;
const MOSTRADOS: usize = 32;

/// The tabs: (category, glyph, name).
const CATEGORIAS: &[(&str, &str, &str)] = &[
    ("recentes", "relogio", n_("Recent")),
    ("rostos", "estrela-n", n_("Faces")),
    ("pessoas", "usuario", n_("People")),
    ("natureza", "sol", n_("Nature")),
    ("atividades", "musica", n_("Activities")),
    ("objetos", "codigo", n_("Objects")),
    ("viagens", "aviao", n_("Travel")),
];

#[derive(Clone)]
struct Emoji {
    simbolo: String,
    grupo: String,
    nome: String,
    palavras: String,
}

fn tabela() -> Vec<Emoji> {
    TABELA.lines().filter_map(|l| {
        let mut c = l.split('\t');
        Some(Emoji { simbolo: c.next()?.to_string(), grupo: c.next()?.to_string(), nome: c.next()?.to_string(), palavras: c.next().unwrap_or("").to_string() })
    }).collect()
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Lower case without accents (the search finds "coracao" for "coração").
fn dobrar(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' => 'a',
        'é' | 'ê' => 'e',
        'í' => 'i',
        'ó' | 'ô' | 'õ' => 'o',
        'ú' | 'ü' => 'u',
        'ç' => 'c',
        c => c,
    }).collect()
}

/// The emoji that match every word of the search: the name first, then the keywords.
fn buscar(t: &[Emoji], q: &str) -> Vec<Emoji> {
    let palavras: Vec<String> = dobrar(q).split_whitespace().map(str::to_string).collect();
    if palavras.is_empty() {
        return vec![];
    }
    let mut v: Vec<(u8, Emoji)> = t.iter().filter(|e| e.grupo != "sinais").filter_map(|e| {
        let (nome, kw) = (dobrar(&e.nome), dobrar(&e.palavras));
        if !palavras.iter().all(|p| nome.contains(p.as_str()) || kw.contains(p.as_str())) {
            return None;
        }
        let peso = if palavras.iter().all(|p| nome.split_whitespace().any(|n| n.starts_with(p.as_str()))) { 0 } else { 1 };
        Some((peso, e.clone()))
    }).collect();
    v.sort_by_key(|(p, _)| *p);
    v.into_iter().map(|(_, e)| e).collect()
}

fn recentes_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/emoji-recentes.txt")
}

fn recentes() -> Vec<String> {
    std::fs::read_to_string(recentes_path()).unwrap_or_default().lines().map(str::to_string).collect()
}

fn usar(simbolo: &str) {
    let mut r = recentes();
    r.retain(|x| x != simbolo);
    r.insert(0, simbolo.to_string());
    r.truncate(MOSTRADOS);
    let p = recentes_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, r.join("\n"));
}

pub struct EmojiPainel {
    janela: gtk::ApplicationWindow,
    busca: gtk::Entry,
    abas: Vec<(&'static str, gtk::ToggleButton)>,
    grade: gtk::Grid,
    resultados_t: gtk::Label,
    sinais: gtk::Grid,
    descrever: gtk::Label,
    tabela: Vec<Emoji>,
    mostrados: RefCell<Vec<Emoji>>,
    escolhido: Cell<usize>,
    alvo: RefCell<String>,
    aberto: Cell<bool>,
    perguntando: Cell<bool>,
}

thread_local! {
    static PAINEL: RefCell<Option<Rc<EmojiPainel>>> = const { RefCell::new(None) };
}

/// Opens the panel over the window in use (Super+.), or closes it.
pub fn alternar(app: &gtk::Application) {
    let p = PAINEL.with(|p| p.borrow().clone()).unwrap_or_else(|| {
        let p = EmojiPainel::new(app);
        PAINEL.with(|x| *x.borrow_mut() = Some(p.clone()));
        p
    });
    if p.aberto.get() { p.fechar() } else { p.abrir() }
}

impl EmojiPainel {
    fn new(app: &gtk::Application) -> Rc<EmojiPainel> {
        let janela = camada(app, "kalango-emoji", Layer::Overlay, &[Edge::Top, Edge::Left]);
        janela.add_css_class("transparente");
        janela.set_resizable(false);
        let painel = caixa(Orientation::Vertical, 10, "emoji-painel");
        painel.set_size_request(392, -1);
        let bc = caixa(Orientation::Horizontal, 8, "emoji-busca");
        bc.append(&ui_icon("busca", "#8E8F99", 14));
        let busca = gtk::Entry::new();
        busca.add_css_class("emoji-campo");
        busca.set_hexpand(true);
        busca.set_placeholder_text(Some(tr("Search emoji and symbols")));
        bc.append(&busca);
        painel.append(&bc);
        let tabs = caixa(Orientation::Horizontal, 2, "");
        let mut abas = Vec::new();
        let mut g: Option<gtk::ToggleButton> = None;
        for (id, glifo, nome) in CATEGORIAS {
            let b = gtk::ToggleButton::new();
            b.add_css_class("emoji-aba");
            b.set_child(Some(&ui_icon(glifo, "#8E8F99", 15)));
            b.set_tooltip_text(Some(tr(nome)));
            b.set_focusable(false);
            if let Some(g) = &g {
                b.set_group(Some(g));
            } else {
                g = Some(b.clone());
            }
            tabs.append(&b);
            abas.push((*id, b));
        }
        painel.append(&tabs);
        let resultados_t = rotulo(tr("RESULTS"), "emoji-rotulo");
        resultados_t.set_xalign(0.0);
        painel.append(&resultados_t);
        let grade = gtk::Grid::new();
        grade.set_row_spacing(4);
        grade.set_column_spacing(4);
        painel.append(&grade);
        let st = rotulo(tr("SYMBOLS"), "emoji-rotulo");
        st.set_xalign(0.0);
        painel.append(&st);
        let sinais = gtk::Grid::new();
        sinais.set_row_spacing(4);
        sinais.set_column_spacing(4);
        painel.append(&sinais);
        let rodape = caixa(Orientation::Horizontal, 8, "emoji-rodape");
        rodape.append(&crate::superficies::estrela(12, "#A5A1E2"));
        let descrever = rotulo(tr("or describe it: “cat laughing”"), "emoji-sub");
        descrever.set_xalign(0.0);
        descrever.set_hexpand(true);
        descrever.set_ellipsize(gtk::pango::EllipsizeMode::End);
        rodape.append(&descrever);
        rodape.append(&rotulo("↵", "clip-tecla"));
        painel.append(&rodape);
        janela.set_child(Some(&painel));
        let p = Rc::new(EmojiPainel {
            janela, busca, abas, grade, resultados_t, sinais, descrever, tabela: tabela(), mostrados: RefCell::default(), escolhido: Cell::new(0),
            alvo: RefCell::default(), aberto: Cell::new(false), perguntando: Cell::new(false),
        });
        p.encher_sinais();
        p.ligar();
        p
    }

    fn ligar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.busca.connect_changed(move |_| if let Some(p) = w.upgrade() { p.encher() });
        for (id, b) in &self.abas {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|_| b.is_active()) {
                    if !p.busca.text().is_empty() {
                        p.busca.set_text("");
                    }
                    p.categoria(id);
                }
            });
        }
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        teclas.connect_key_pressed(move |_, k, _, _| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            let n = p.mostrados.borrow().len();
            let i = p.escolhido.get();
            match k {
                gdk::Key::Escape => p.fechar(),
                gdk::Key::Right if n > 0 => p.escolher((i + 1).min(n - 1)),
                gdk::Key::Left => p.escolher(i.saturating_sub(1)),
                gdk::Key::Down if n > 0 => p.escolher((i + POR_LINHA).min(n - 1)),
                gdk::Key::Up => p.escolher(i.saturating_sub(POR_LINHA)),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let e = p.mostrados.borrow().get(i).cloned();
                    match e {
                        Some(e) => p.colar(&e.simbolo),
                        None => p.perguntar(),
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.janela.add_controller(teclas);
    }

    fn abrir(self: &Rc<Self>) {
        let v = crate::servicos::niri_pedido(serde_json::json!("FocusedWindow")).ok().map(|v| v["FocusedWindow"].clone()).unwrap_or_default();
        *self.alvo.borrow_mut() = v["app_id"].as_str().unwrap_or("").to_string();
        let tela = crate::configuracoes_tela::monitor_principal().map(|m| (m.geometry().width(), m.geometry().height())).unwrap_or((1920, 1080));
        let l = &v["layout"];
        let (x, y) = match (l["tile_pos_in_workspace_view"][0].as_f64(), l["tile_pos_in_workspace_view"][1].as_f64(), l["window_size"][0].as_f64(), l["window_size"][1].as_f64()) {
            (Some(px), Some(py), Some(w), Some(h)) => ((px + w / 2.0 - 196.0) as i32, (py + h / 2.0 - 150.0) as i32),
            _ => (tela.0 / 2 - 196, tela.1 / 2 - 200),
        };
        self.janela.set_margin(Edge::Left, x.clamp(12, (tela.0 - 404).max(12)));
        self.janela.set_margin(Edge::Top, (y - 32).clamp(8, (tela.1 - 480).max(8)));
        self.busca.set_text("");
        let tem_recentes = !recentes().is_empty();
        let aba = if tem_recentes { "recentes" } else { "rostos" };
        if let Some((_, b)) = self.abas.iter().find(|(id, _)| *id == aba) {
            b.set_active(true);
        }
        self.categoria(aba);
        self.aberto.set(true);
        self.janela.present();
        self.busca.grab_focus();
        let jw = self.janela.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(j) = jw.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
        if debug() {
            eprintln!("debug emoji: aberto sobre {}", self.alvo.borrow());
        }
    }

    fn fechar(&self) {
        if !self.aberto.replace(false) {
            return;
        }
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.janela.set_visible(false);
    }

    fn categoria(self: &Rc<Self>, id: &str) {
        let v: Vec<Emoji> = if id == "recentes" {
            recentes().iter().filter_map(|s| self.tabela.iter().find(|e| &e.simbolo == s).cloned()).collect()
        } else {
            self.tabela.iter().filter(|e| e.grupo == id).take(MOSTRADOS * 4).cloned().collect()
        };
        self.resultados_t.set_text(&CATEGORIAS.iter().find(|c| c.0 == id).map(|c| tr(c.2).to_uppercase()).unwrap_or_default());
        self.mostrar(v);
    }

    fn encher(self: &Rc<Self>) {
        let q = self.busca.text().to_string();
        if q.trim().is_empty() {
            let aba = self.abas.iter().find(|(_, b)| b.is_active()).map(|(id, _)| *id).unwrap_or("rostos");
            self.categoria(aba);
            return;
        }
        self.resultados_t.set_text(tr("RESULTS"));
        let v = buscar(&self.tabela, &q);
        if debug() {
            eprintln!("debug emoji: «{q}» {} resultados", v.len());
        }
        self.descrever.set_text(&if v.is_empty() { format!("{} «{q}» ↵", tr("nothing with these words · describe to the agent:")) } else { tr("or describe it: “cat laughing”").to_string() });
        self.mostrar(v);
    }

    fn mostrar(self: &Rc<Self>, v: Vec<Emoji>) {
        while let Some(c) = self.grade.first_child() {
            self.grade.remove(&c);
        }
        for (k, e) in v.iter().take(MOSTRADOS).enumerate() {
            let b = gtk::Button::with_label(&e.simbolo);
            b.add_css_class("emoji-celula");
            b.set_tooltip_text(Some(&e.nome));
            b.set_focusable(false);
            let (w, s) = (Rc::downgrade(self), e.simbolo.clone());
            b.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.colar(&s) });
            self.grade.attach(&b, (k % POR_LINHA) as i32, (k / POR_LINHA) as i32, 1, 1);
        }
        *self.mostrados.borrow_mut() = v.into_iter().take(MOSTRADOS).collect();
        self.escolher(0);
    }

    fn escolher(&self, i: usize) {
        self.escolhido.set(i);
        let mut c = self.grade.first_child();
        let mut k = 0;
        while let Some(x) = c {
            if k == i { x.add_css_class("escolhido") } else { x.remove_css_class("escolhido") }
            k += 1;
            c = x.next_sibling();
        }
    }

    fn encher_sinais(self: &Rc<Self>) {
        for (k, e) in self.tabela.iter().filter(|e| e.grupo == "sinais").take(16).enumerate() {
            let b = gtk::Button::with_label(&e.simbolo);
            b.add_css_class("emoji-sinal");
            b.set_tooltip_text(Some(&e.nome));
            b.set_focusable(false);
            let (w, s) = (Rc::downgrade(self), e.simbolo.clone());
            b.connect_clicked(move |_| if let Some(p) = w.upgrade() { p.colar(&s) });
            self.sinais.attach(&b, (k % POR_LINHA) as i32, (k / POR_LINHA) as i32, 1, 1);
        }
    }

    /// Into the window in use: the clipboard and the virtual keyboard's Ctrl+V (Ctrl+Shift+V in a terminal).
    fn colar(&self, s: &str) {
        usar(s);
        if let Err(e) = crate::clipboard::copy_text(s, false) {
            eprintln!("kalango-shell: emoji: {e}");
            return;
        }
        let alvo = self.alvo.borrow().to_lowercase();
        let shift = ["terminal", "console", "kitty", "alacritty", "foot", "ptyxis", "wezterm", "konsole", "tilix"].iter().any(|t| alvo.contains(t));
        self.fechar();
        if debug() {
            eprintln!("debug emoji: colado {s}");
        }
        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            std::thread::spawn(move || {
                if let Err(e) = crate::paste_keys::paste(shift) {
                    eprintln!("kalango-shell: emoji: colar: {e}");
                }
            });
        });
    }

    /// Nothing found: the local model gives the words of the description, and they are searched.
    fn perguntar(self: &Rc<Self>) {
        let q = self.busca.text().to_string();
        if q.trim().is_empty() || self.perguntando.replace(true) {
            return;
        }
        self.descrever.set_text(tr("asking the local model…"));
        let bin = crate::voz::agent_binary();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            std::process::Command::new(&bin).args(["emoji-palavras", &q]).stdin(std::process::Stdio::null()).output().ok()
                .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
        }, move |palavras: String| {
            let Some(p) = w.upgrade() else { return };
            p.perguntando.set(false);
            let mut v = Vec::new();
            for termo in palavras.lines().filter(|l| !l.trim().is_empty()) {
                for e in buscar(&p.tabela, termo) {
                    if !v.iter().any(|x: &Emoji| x.simbolo == e.simbolo) {
                        v.push(e);
                    }
                }
            }
            if debug() {
                eprintln!("debug emoji: o modelo deu «{}» → {} resultados", palavras.replace('\n', ", "), v.len());
            }
            p.descrever.set_text(if v.is_empty() { tr("the agent found nothing either") } else { tr("found by the description") });
            p.mostrar(v);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busca_em_portugues() {
        let t = tabela();
        assert!(t.len() > 1500);
        let r = buscar(&t, "sorri");
        assert!(r.iter().any(|e| e.simbolo == "😀"), "{:?}", r.iter().map(|e| e.simbolo.clone()).collect::<Vec<_>>());
        assert!(buscar(&t, "coracao vermelho").iter().any(|e| e.simbolo == "❤️"));
        assert!(buscar(&t, "gato").iter().any(|e| e.simbolo == "🐱" || e.simbolo == "🐈"));
    }
}
