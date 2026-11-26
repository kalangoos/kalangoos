//! Central de Apps › Apps criativos (design "36 · Central · Apps criativos": coleção 345270, detalhe 345764,
//! atualizações 346139, janela estreita 347389): os 17 apps do projeto Craft, instalados e atualizados pela Central a
//! partir dos lançamentos assinados do próprio projeto (`kalango_craft`).
//! - Coleção: as categorias com as contagens, um cartão por app (marcador neutro com as iniciais até o app instalado
//!   trazer o próprio ícone, "Alfa", a categoria e o tamanho), "Instalar todos" e a atualização automática geral.
//! - Detalhe: versão fixada, "verificado: assinatura do projeto", Sobre (versão, fonte, tamanho, licença), a
//!   atualização automática do app; ao lado, as pastas que ele vê, a rede (desligada por padrão) e se os agentes podem
//!   usá-lo, com os três níveis.
//! - Atualizações: instalando (com pausa), pronta (aplica ao fechar o app; "Reiniciar app agora"), bloqueada pelo
//!   KalangoOS, assinatura que não confere (nada muda no sistema), sem espaço; e as recentes, cada uma com Desfazer
//!   (volta ao ponto de restauração: a versão anterior guardada).
//!
//! A atualização automática roda em segundo plano (`agendar`): procura a cada 6 h e aplica quando o app está fechado.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_craft::instalar::{self, Erro};
use kalango_craft::verificar::{self, Andamento, Anuncio, Falha};
use kalango_craft::{ajustes, catalogo, App};
use kalango_i18n::{n_, tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// As categorias (id do catálogo, rótulo) na ordem do design.
const CATEGORIAS: &[(&str, &str)] = &[("", n_("All of them")), ("imagem", n_("Image")), ("video", n_("Video")), ("audio", n_("Audio")), ("documentos", n_("Documents")), ("3d", n_("3D")), ("ferramentas", n_("Tools"))];
/// A cor do ponto de cada categoria (design: lilás imagem, dourado vídeo, teal áudio, azul documentos, dourado 3D).
fn cor_da_categoria(c: &str) -> &'static str {
    match c {
        "imagem" => "#A5A1E2",
        "video" | "3d" => "#E5C98C",
        "audio" => "#8ED2C3",
        "documentos" => "#7CC3DF",
        _ => "#8ED2C3",
    }
}

fn nome_da_categoria(c: &str) -> &'static str {
    CATEGORIAS.iter().find(|x| x.0 == c).map(|x| tr(x.1)).unwrap_or("")
}

/// "1,4 GB", "118 MB".
pub fn tamanho(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9).replace('.', ",")
    } else {
        format!("{} MB", (bytes as f64 / 1e6).round() as u64)
    }
}

/// "há 2 h", "ontem", "3 dias".
fn quando(seg: u64) -> String {
    let d = instalar::agora().saturating_sub(seg);
    if d < 3600 {
        trf("{n} min ago", &[("n", (d / 60).max(1).to_string())])
    } else if d < 86_400 {
        trf("{n} h ago", &[("n", (d / 3600).to_string())])
    } else if d < 2 * 86_400 {
        tr("yesterday").to_string()
    } else {
        trf("{n} days", &[("n", (d / 86_400).to_string())])
    }
}

// ------------------------------------------------------------------------------------------------ the jobs

/// O que acontece com um app agora (a linha da tela de atualizações e o botão do cartão).
#[derive(Clone, Debug, PartialEq)]
pub enum Fase {
    Verificando,
    Baixando,
    Instalando,
    /// baixada e verificada, esperando o app fechar
    Pronta { versao: String },
    Feito,
    Falhou(Erro),
}

#[derive(Clone)]
struct Trabalho {
    fase: Arc<Mutex<Fase>>,
    andamento: Andamento,
    atualizacao: bool,
}

/// O que é só do agendador: imagens verificadas e baixadas esperando o app fechar.
/// (commit verificado pelo kalangod, versão, digest, repositório protegido)
type Prontas = Arc<Mutex<HashMap<String, Preparado>>>;

#[derive(Clone, Debug)]
pub struct Preparado {
    commit: String,
    versao: String,
    digest: String,
    repo: PathBuf,
}

pub struct Criativos {
    pub colecao: gtk::Box,
    pub detalhe: gtk::Box,
    pub atualizacoes: gtk::Box,
    /// a chamada da coleção nas páginas Descobrir e Apps
    pub chamada: gtk::Box,
    grade: gtk::FlowBox,
    chips: Vec<gtk::ToggleButton>,
    instalar_todos: gtk::Button,
    acoes_topo: gtk::Box,
    linha_topo: gtk::Box,
    titulo_bloco: gtk::Box,
    auto: gtk::Switch,
    categoria: Cell<usize>,
    largura: Cell<i32>,
    pub atual: RefCell<Option<String>>,
    anuncios: RefCell<HashMap<String, Anuncio>>,
    indice_estado: Cell<u8>,
    instalados: RefCell<Vec<String>>,
    trabalhos: RefCell<HashMap<String, Trabalho>>,
    prontas: Prontas,
    acompanhando: Cell<bool>,
    agentes: RefCell<serde_json::Value>,
    /// a página pede para ser mostrada de novo (o AppStore liga: refresh)
    pub ao_mudar: RefCell<Option<Box<dyn Fn()>>>,
    /// abrir o detalhe de um app
    pub ao_abrir: RefCell<Option<Box<dyn Fn(&str)>>>,
    pub ao_voltar: RefCell<Option<Box<dyn Fn()>>>,
    barras: RefCell<HashMap<String, (glib::WeakRef<gtk::LevelBar>, glib::WeakRef<gtk::Label>)>>,
}

thread_local! {
    /// a única: a Central e o agendador usam a mesma (os trabalhos e as prontas são um só)
    static MEU: RefCell<Option<Rc<Criativos>>> = const { RefCell::new(None) };
}

/// A coleção desta sessão (criada na primeira vez).
pub fn compartilhado() -> Rc<Criativos> {
    if let Some(s) = MEU.with(|m| m.borrow().clone()) {
        return s;
    }
    let s = Criativos::new();
    MEU.with(|m| *m.borrow_mut() = Some(s.clone()));
    s
}

fn ellipsized(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

fn quebra(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

fn botao(texto: &str, principal: bool) -> gtk::Button {
    let b = gtk::Button::with_label(texto);
    b.add_css_class("loja-acao");
    if principal {
        b.add_css_class("principal");
    }
    b.set_valign(Align::Center);
    b
}

fn botao_icone(glyph: &str, cor: &str, texto: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    let r = caixa(Orientation::Horizontal, 6, "");
    r.set_halign(Align::Center);
    r.append(&ui_icon(glyph, cor, 14));
    r.append(&rotulo(texto, ""));
    b.set_child(Some(&r));
    b.set_valign(Align::Center);
    b
}

/// As pastas de dados do XDG (a da pessoa e as do sistema), onde ficam os `.desktop` exportados.
fn pastas_de_dados() -> Vec<PathBuf> {
    std::iter::once(glib::user_data_dir()).chain(glib::system_data_dirs()).collect()
}

/// O `.desktop` de um app, se algum destes lugares o tem (num sistema limpo, só depois de instalado pelo Flatpak).
fn desktop_do_app(id: &str, dirs: &[PathBuf]) -> Option<gio::DesktopAppInfo> {
    dirs.iter().map(|d| d.join("applications").join(format!("{id}.desktop"))).find(|p| p.is_file()).and_then(gio::DesktopAppInfo::from_filename)
}

/// O marcador do app: o ícone do app instalado (o do pacote), senão as iniciais num quadrado neutro.
fn marcador(app: &App, lado: i32) -> gtk::Widget {
    if let Some(d) = desktop_do_app(&app.id, &pastas_de_dados()) {
        if let Some(i) = d.icon().filter(|i| gdk::Display::default().is_some_and(|dp| gtk::IconTheme::for_display(&dp).has_gicon(i))) {
            let img = gtk::Image::from_gicon(&i);
            img.set_pixel_size(lado * 3 / 4);
            img.set_size_request(lado, lado);
            img.set_valign(Align::Center);
            return img.upcast();
        }
    }
    let b = caixa(Orientation::Horizontal, 0, "cr-marcador");
    b.set_size_request(lado, lado);
    b.set_valign(Align::Center);
    b.set_halign(Align::Start);
    let l = rotulo(&app.iniciais(), if lado >= 64 { "cr-iniciais-grande" } else { "cr-iniciais" });
    l.set_hexpand(true);
    b.append(&l);
    b.upcast()
}

fn alfa() -> gtk::Label {
    let l = rotulo(tr("Alpha"), "cr-alfa");
    l.set_valign(Align::Center);
    l
}

impl Criativos {
    fn new() -> Rc<Criativos> {
        // ---- the collection
        let colecao = caixa(Orientation::Vertical, 18, "");
        let linha_topo = caixa(Orientation::Horizontal, 12, "");
        let titulo_bloco = caixa(Orientation::Vertical, 4, "");
        titulo_bloco.set_hexpand(true);
        let h = rotulo(tr("Creative apps"), "cr-titulo");
        h.set_xalign(0.0);
        titulo_bloco.append(&h);
        let sub = quebra(tr("17 free apps made in Rust, maintained by the Craft project · installed and updated by the App Center"), "loja-sub");
        titulo_bloco.append(&sub);
        linha_topo.append(&titulo_bloco);
        let acoes_topo = caixa(Orientation::Horizontal, 8, "");
        acoes_topo.set_valign(Align::Start);
        let auto_box = caixa(Orientation::Horizontal, 10, "cr-auto");
        auto_box.append(&rotulo(tr("Automatic updates"), "cr-auto-texto"));
        let auto = gtk::Switch::new();
        auto.add_css_class("central-switch");
        auto.set_valign(Align::Center);
        auto.set_active(ajustes::ler().atualizar);
        crate::central::posicao_debug_sempre("criativos atualizacao automatica", auto.upcast_ref());
        auto_box.append(&auto);
        acoes_topo.append(&auto_box);
        let instalar_todos = botao_icone("download", "#1C1D24", "", "cr-instalar-todos");
        crate::central::posicao_debug_sempre("criativos instalar todos", instalar_todos.upcast_ref());
        acoes_topo.append(&instalar_todos);
        linha_topo.append(&acoes_topo);
        colecao.append(&linha_topo);
        let chips_row = caixa(Orientation::Horizontal, 6, "");
        let mut chips: Vec<gtk::ToggleButton> = Vec::new();
        for (id, _) in CATEGORIAS {
            let c = gtk::ToggleButton::new();
            c.add_css_class("loja-chip");
            c.add_css_class("cr-filtro");
            if let Some(f) = chips.first() {
                c.set_group(Some(f));
            }
            crate::central::posicao_debug_sempre(&format!("criativos categoria {}", if id.is_empty() { "todos" } else { id }), c.upcast_ref());
            chips_row.append(&c);
            chips.push(c);
        }
        chips[0].set_active(true);
        let chips_scroll = gtk::ScrolledWindow::new();
        chips_scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
        chips_scroll.set_hscrollbar_policy(gtk::PolicyType::External);
        chips_scroll.set_child(Some(&chips_row));
        colecao.append(&chips_scroll);
        let grade = gtk::FlowBox::new();
        grade.set_selection_mode(gtk::SelectionMode::None);
        grade.set_homogeneous(true);
        grade.set_column_spacing(12);
        grade.set_row_spacing(12);
        grade.set_valign(Align::Start);
        colecao.append(&grade);
        let nota = quebra(tr("The final icons come with each app (its Flatpak package); until then, neutral markers with the name."), "cr-nota");
        nota.set_margin_top(12);
        colecao.append(&nota);
        // ---- the call on Discover and Apps
        let chamada = caixa(Orientation::Horizontal, 14, "loja-cartao");
        chamada.add_css_class("cr-chamada");
        let ct = caixa(Orientation::Vertical, 2, "");
        ct.set_hexpand(true);
        let t1 = caixa(Orientation::Horizontal, 8, "");
        t1.append(&rotulo(tr("Creative apps"), "loja-app-nome"));
        t1.append(&alfa());
        ct.append(&t1);
        ct.append(&quebra(tr("Photo, video, audio, documents and 3D: 17 free apps of the Craft project, verified by their signature"), "loja-sub"));
        chamada.append(&ct);
        let ver = botao(tr("See collection"), false);
        crate::central::posicao_debug_sempre("criativos ver colecao", ver.upcast_ref());
        chamada.append(&ver);

        let s = Rc::new(Criativos {
            colecao, detalhe: caixa(Orientation::Vertical, 18, ""), atualizacoes: caixa(Orientation::Vertical, 10, ""), chamada, grade, chips, instalar_todos, acoes_topo, linha_topo, titulo_bloco, auto,
            categoria: Cell::new(0), largura: Cell::new(1400), atual: RefCell::default(), anuncios: RefCell::default(), indice_estado: Cell::new(0), instalados: RefCell::default(),
            trabalhos: RefCell::default(), prontas: Arc::default(), acompanhando: Cell::new(false), agentes: RefCell::new(serde_json::json!({})),
            ao_mudar: RefCell::default(), ao_abrir: RefCell::default(), ao_voltar: RefCell::default(), barras: RefCell::default(),
        });
        // (KALANGO_CRIATIVOS_DEMO=1: the update page's states, as in design 346139, for the session's screenshots)
        if std::env::var_os("KALANGO_CRIATIVOS_DEMO").is_some() {
            let f = |id: &str, fase: Fase, feito: u64, total: u64| {
                let t = Trabalho { fase: Arc::new(Mutex::new(fase)), andamento: Andamento::default(), atualizacao: true };
                t.andamento.feito.store(feito, Ordering::Relaxed);
                t.andamento.total.store(total, Ordering::Relaxed);
                s.trabalhos.borrow_mut().insert(id.to_string(), t);
            };
            f("ai.storyteller.photocraft", Fase::Baixando, 59_000_000, 96_000_000);
            f("ai.storyteller.filmcraft", Fase::Pronta { versao: "0.24.0".into() }, 0, 0);
            f("ai.storyteller.effectcraft", Fase::Falhou(Erro::Verificacao(Falha::Bloqueada { versao: "0.20.0".into(), motivo: "falha conhecida ao exportar".into() })), 0, 0);
            f("ai.storyteller.vectorcraft", Fase::Falhou(Erro::Verificacao(Falha::Assinatura("demo".into()))), 0, 0);
            f("ai.storyteller.cadcraft", Fase::Falhou(Erro::SemEspaco { faltam: 210_000_000 }), 0, 0);
        }
        for (i, c) in s.chips.iter().enumerate() {
            let w = Rc::downgrade(&s);
            c.connect_toggled(move |c| {
                if let (Some(s), true) = (w.upgrade(), c.is_active()) {
                    s.categoria.set(i);
                    s.mostrar_colecao();
                }
            });
        }
        let w = Rc::downgrade(&s);
        s.auto.connect_state_set(move |_, on| {
            let mut a = ajustes::ler();
            a.atualizar = on;
            if let Err(e) = ajustes::gravar(&a) {
                eprintln!("kalango-shell: apps criativos: {e}");
            }
            if debug() {
                eprintln!("debug criativos: atualização automática {}", if on { "ligada" } else { "desligada" });
            }
            if let Some(s) = w.upgrade() {
                s.mudou();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&s);
        s.instalar_todos.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                let faltam: Vec<String> = catalogo().apps.iter().filter(|a| !s.instalados.borrow().contains(&a.id)).map(|a| a.id.clone()).collect();
                if debug() {
                    eprintln!("debug criativos: instalar todos ({} apps)", faltam.len());
                }
                for id in faltam {
                    s.instalar(&id, false);
                }
            }
        });
        let w = Rc::downgrade(&s);
        ver.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                if let Some(f) = s.ao_abrir.borrow().as_ref() {
                    f("");
                }
            }
        });
        s
    }

    fn mudou(&self) {
        if let Some(f) = self.ao_mudar.borrow().as_ref() {
            f();
        }
    }

    /// O que muda entre uma vez e outra: o que está instalado, e o índice do projeto (uma vez, fora da interface).
    pub fn recarregar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (inst, agentes) = gio::spawn_blocking(|| (instalar::instalados(), crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "ler"})).unwrap_or(serde_json::json!({}))))
                .await.unwrap_or_default();
            let Some(s) = w.upgrade() else { return };
            *s.instalados.borrow_mut() = inst;
            *s.agentes.borrow_mut() = agentes;
            s.mudou();
        });
        if self.indice_estado.get() == 0 {
            self.indice_estado.set(1);
            let w = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let r = gio::spawn_blocking(|| verificar::indice(catalogo())).await.unwrap_or_else(|_| Err(Falha::Rede("?".into())));
                let Some(s) = w.upgrade() else { return };
                match r {
                    Ok(v) => {
                        if debug() {
                            eprintln!("debug criativos: índice com {} apps", v.len());
                        }
                        *s.anuncios.borrow_mut() = v.into_iter().collect();
                        s.indice_estado.set(2);
                    }
                    Err(e) => {
                        eprintln!("kalango-shell: apps criativos: índice: {e}");
                        s.indice_estado.set(0);
                    }
                }
                s.mudou();
            });
        }
    }

    /// A largura do conteúdo: as colunas e a janela estreita (design 347389) seguem ela.
    pub fn ajustar(&self, largura: i32) {
        self.largura.set(largura);
        let col = ((largura + 12) / (300 + 12)).clamp(1, 4) as u32;
        // (no mínimo uma: a grade não segura a janela larga; no máximo as que cabem)
        if self.grade.max_children_per_line() != col {
            self.grade.set_min_children_per_line(1);
            self.grade.set_max_children_per_line(col);
        }
        // estreita: "Instalar todos" desce para baixo do subtítulo
        let estreita = largura < 1000;
        let pai = self.instalar_todos.parent();
        if estreita && pai.as_ref() != Some(self.titulo_bloco.upcast_ref()) {
            self.acoes_topo.remove(&self.instalar_todos);
            self.instalar_todos.set_halign(Align::Start);
            self.instalar_todos.set_margin_top(6);
            self.titulo_bloco.append(&self.instalar_todos);
        } else if !estreita && pai.as_ref() == Some(self.titulo_bloco.upcast_ref()) {
            self.titulo_bloco.remove(&self.instalar_todos);
            self.instalar_todos.set_margin_top(0);
            self.acoes_topo.append(&self.instalar_todos);
        }
        let _ = &self.linha_topo;
    }

    fn bytes_de(&self, app: &App) -> u64 {
        self.anuncios.borrow().get(&app.id).map(|a| a.tamanho_download).filter(|b| *b > 0).unwrap_or(app.tamanho_mb * 1_000_000)
    }

    /// Os apps criativos instalados (o que a Central viu por último).
    pub fn instalados_agora(&self) -> Vec<String> {
        if self.instalados.borrow().is_empty() {
            *self.instalados.borrow_mut() = instalar::instalados();
        }
        self.instalados.borrow().clone()
    }

    fn instalado(&self, id: &str) -> bool {
        self.instalados.borrow().iter().any(|x| x == id)
    }

    // ------------------------------------------------------------------------------------------ the collection

    pub fn mostrar_colecao(self: &Rc<Self>) {
        while let Some(c) = self.grade.first_child() {
            self.grade.remove(&c);
        }
        let cat = catalogo();
        for (i, (id, nome)) in CATEGORIAS.iter().enumerate() {
            let n = cat.apps.iter().filter(|a| id.is_empty() || a.categoria == *id).count();
            self.chips[i].set_label(&format!("{} · {n}", tr(nome)));
            self.chips[i].set_visible(n > 0);
        }
        let filtro = CATEGORIAS[self.categoria.get()].0;
        let mut anims = Vec::new();
        for app in cat.apps.iter().filter(|a| filtro.is_empty() || a.categoria == filtro) {
            let c = self.cartao(app);
            self.grade.append(&c);
            anims.push(c);
        }
        kalango_movimento::cascata(&anims, 8.0);
        let faltam: Vec<&App> = cat.apps.iter().filter(|a| !self.instalado(&a.id)).collect();
        let total: u64 = faltam.iter().map(|a| self.bytes_de(a)).sum();
        self.instalar_todos.set_visible(!faltam.is_empty());
        if let Some(l) = self.instalar_todos.child().and_then(|r| r.last_child()).and_downcast::<gtk::Label>() {
            l.set_text(&trf("Install all · {size}", &[("size", tamanho(total))]));
        }
        if self.auto.is_active() != ajustes::ler().atualizar {
            self.auto.set_active(ajustes::ler().atualizar);
        }
        if debug() {
            eprintln!("debug criativos: coleção {} · {} instalados · faltam {}", if filtro.is_empty() { "todos" } else { filtro }, self.instalados.borrow().len(), tamanho(total));
        }
    }

    fn cartao(self: &Rc<Self>, app: &App) -> kalango_movimento::Animavel {
        let card = caixa(Orientation::Horizontal, 12, "loja-cartao");
        card.add_css_class("cr-cartao");
        card.append(&marcador(app, 48));
        let t = caixa(Orientation::Vertical, 2, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        let linha = caixa(Orientation::Horizontal, 8, "");
        let nome = rotulo(&app.nome, "loja-app-nome");
        nome.set_xalign(0.0);
        linha.append(&nome);
        linha.append(&alfa());
        t.append(&linha);
        let d = quebra(&app.descricao, "loja-sub");
        d.set_lines(2);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.append(&d);
        let pe = caixa(Orientation::Horizontal, 6, "");
        pe.set_margin_top(6);
        let dot = caixa(Orientation::Horizontal, 0, "cr-ponto");
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        let tint = gtk::CssProvider::new();
        tint.load_from_string(&format!(".cr-ponto {{ background: {}; }}", crate::tema::cor(cor_da_categoria(&app.categoria))));
        #[allow(deprecated)]
        dot.style_context().add_provider(&tint, gtk::STYLE_PROVIDER_PRIORITY_USER + 2);
        pe.append(&dot);
        let sub = ellipsized(&format!("{} · {}", nome_da_categoria(&app.categoria), tamanho(self.bytes_de(app))), "cr-meta");
        pe.append(&sub);
        t.append(&pe);
        card.append(&t);
        card.append(&self.acao(app, Some(&sub)));
        // (clicar no cartão abre o detalhe)
        let click = gtk::GestureClick::new();
        let (w, id) = (Rc::downgrade(self), app.id.clone());
        click.connect_released(move |g, _, x, y| {
            // (um clique no botão do cartão é do botão)
            let alvo = g.widget().and_then(|c| c.pick(x, y, gtk::PickFlags::DEFAULT));
            if alvo.as_ref().is_some_and(|a| a.is::<gtk::Button>() || a.ancestor(gtk::Button::static_type()).is_some()) {
                return;
            }
            if let Some(s) = w.upgrade() {
                if let Some(f) = s.ao_abrir.borrow().as_ref() {
                    f(&id);
                }
            }
        });
        card.add_controller(click);
        crate::central::posicao_debug_sempre(&format!("criativos cartao {}", app.imagem()), card.upcast_ref());
        kalango_movimento::Animavel::new(&card)
    }

    /// O botão de um app: Instalar, "Instalado", o progresso, ou o erro.
    fn acao(self: &Rc<Self>, app: &App, texto: Option<&gtk::Label>) -> gtk::Widget {
        if let Some(tr_) = self.trabalhos.borrow().get(&app.id).cloned() {
            let fase = tr_.fase.lock().map(|f| f.clone()).unwrap_or(Fase::Verificando);
            match fase {
                Fase::Falhou(e) => {
                    let b = botao_icone("alerta", "#EFA2A6", tr("Try again"), "loja-acao");
                    b.set_tooltip_text(Some(&texto_do_erro(app, &e)));
                    let (w, id) = (Rc::downgrade(self), app.id.clone());
                    b.connect_clicked(move |_| {
                        if let Some(s) = w.upgrade() {
                            s.instalar(&id, false);
                        }
                    });
                    crate::central::posicao_debug_sempre(&format!("criativos tentar {}", app.imagem()), b.upcast_ref());
                    return b.upcast();
                }
                Fase::Feito => {}
                Fase::Pronta { .. } => {}
                _ => return self.barra(&app.id, texto),
            }
        }
        if self.instalado(&app.id) {
            let p = caixa(Orientation::Horizontal, 6, "cr-instalado");
            p.set_valign(Align::Center);
            let dot = caixa(Orientation::Horizontal, 0, "cr-ponto-teal");
            dot.set_size_request(6, 6);
            dot.set_valign(Align::Center);
            p.append(&dot);
            p.append(&rotulo(tr("Installed (one app)"), ""));
            return p.upcast();
        }
        let b = botao(tr("Install"), false);
        crate::central::posicao_debug_sempre(&format!("criativos instalar {}", app.imagem()), b.upcast_ref());
        let (w, id) = (Rc::downgrade(self), app.id.clone());
        b.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.instalar(&id, false);
            }
        });
        b.upcast()
    }

    fn barra(self: &Rc<Self>, id: &str, texto: Option<&gtk::Label>) -> gtk::Widget {
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.add_css_class("plano-barra");
        bar.set_size_request(96, -1);
        bar.set_valign(Align::Center);
        let label = texto.cloned().unwrap_or_else(|| rotulo("", ""));
        self.barras.borrow_mut().insert(id.to_string(), (bar.downgrade(), label.downgrade()));
        self.acompanhar();
        bar.upcast()
    }

    // ------------------------------------------------------------------------------------------ installing

    /// Instala (ou atualiza) um app: verifica, baixa, importa e instala numa thread; o progresso vai para a tela.
    pub fn instalar(self: &Rc<Self>, id: &str, atualizacao: bool) {
        if self.trabalhos.borrow().get(id).is_some_and(|t| !matches!(*t.fase.lock().unwrap(), Fase::Falhou(_) | Fase::Feito | Fase::Pronta { .. })) {
            return;
        }
        let Some(app) = catalogo().app(id).cloned() else { return };
        let t = Trabalho { fase: Arc::new(Mutex::new(Fase::Verificando)), andamento: Andamento::default(), atualizacao };
        self.trabalhos.borrow_mut().insert(id.to_string(), t.clone());
        if debug() {
            eprintln!("debug criativos: {} {id}", if atualizacao { "atualizando" } else { "instalando" });
        }
        let anuncio = self.anuncios.borrow().get(id).cloned();
        let prontas = self.prontas.clone();
        std::thread::spawn(move || {
            let r = trabalhar(&app, anuncio, &t, &prontas);
            if let Ok(mut f) = t.fase.lock() {
                *f = match r {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("kalango-shell: apps criativos: {}: {e}", app.id);
                        Fase::Falhou(e)
                    }
                };
            }
        });
        self.mudou();
        self.acompanhar();
    }

    /// Segue os trabalhos enquanto houver: as barras, e a página de novo quando um termina.
    fn acompanhar(self: &Rc<Self>) {
        if self.acompanhando.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(150), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            let mut acabou = false;
            let mut correndo = false;
            for (id, t) in s.trabalhos.borrow().iter() {
                let fase = t.fase.lock().map(|f| f.clone()).unwrap_or(Fase::Verificando);
                let (feito, total) = (t.andamento.feito.load(Ordering::Relaxed), t.andamento.total.load(Ordering::Relaxed));
                if let Some((b, l)) = s.barras.borrow().get(id) {
                    if let Some(b) = b.upgrade() {
                        b.set_value(if total > 0 { feito as f64 / total as f64 } else { 0.0 });
                    }
                    if let Some(l) = l.upgrade() {
                        l.set_text(&texto_da_fase(&fase, feito, total));
                    }
                }
                match fase {
                    Fase::Verificando | Fase::Baixando | Fase::Instalando => correndo = true,
                    _ => {}
                }
                if matches!(fase, Fase::Feito | Fase::Falhou(_) | Fase::Pronta { .. }) && s.barras.borrow().contains_key(id) {
                    acabou = true;
                }
            }
            if acabou {
                let feitos: Vec<String> = s.trabalhos.borrow().iter().filter(|(_, t)| matches!(*t.fase.lock().unwrap(), Fase::Feito) && !t.atualizacao).map(|(k, _)| k.clone()).collect();
                for id in feitos {
                    associar(&id);
                }
                s.barras.borrow_mut().clear();
                s.trabalhos.borrow_mut().retain(|_, t| !matches!(*t.fase.lock().unwrap(), Fase::Feito));
                *s.instalados.borrow_mut() = instalar::instalados();
                s.mudou();
            }
            if !correndo && !acabou {
                s.acompanhando.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn remover(self: &Rc<Self>, id: &str) {
        let Some(app) = catalogo().app(id).cloned() else { return };
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || instalar::remover(&app, false)).await.unwrap_or_else(|_| Err("?".into()));
            if let Err(e) = &r {
                eprintln!("kalango-shell: apps criativos: remover: {e}");
            }
            if let Some(s) = w.upgrade() {
                *s.instalados.borrow_mut() = instalar::instalados();
                s.mudou();
            }
        });
    }

    /// "Desfazer": volta à versão anterior — o commit que o kalangod verificou e guardou como ponto de restauração.
    pub fn voltar(self: &Rc<Self>, id: &str) {
        let Some(app) = catalogo().app(id).cloned() else { return };
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let id = app.id.clone();
            let r = gio::spawn_blocking(move || voltar_ao_anterior(&app)).await.unwrap_or_else(|_| Err("?".into()));
            match r {
                Ok(versao) => {
                    // (a atualização automática deste app espera a pessoa ligar de novo: senão ele voltaria para a nova)
                    let mut a = ajustes::ler();
                    a.apps.entry(id.clone()).or_default().voltou = true;
                    let _ = ajustes::gravar(&a);
                    if debug() {
                        eprintln!("debug criativos: {id} voltou para {versao}");
                    }
                }
                Err(e) => eprintln!("kalango-shell: apps criativos: voltar {id}: {e}"),
            }
            if let Some(s) = w.upgrade() {
                *s.instalados.borrow_mut() = instalar::instalados();
                s.mudou();
            }
        });
    }

    // ------------------------------------------------------------------------------------------ the detail

    pub fn mostrar_detalhe(self: &Rc<Self>, id: &str) {
        let d = &self.detalhe;
        while let Some(c) = d.first_child() {
            d.remove(&c);
        }
        let Some(app) = catalogo().app(id) else { return };
        *self.atual.borrow_mut() = Some(app.id.clone());
        let inst = self.instalado(&app.id);
        let estado = instalar::ler_estado().apps.get(&app.id).cloned();
        let aj = ajustes::ler();
        let anuncio = self.anuncios.borrow().get(&app.id).cloned();
        let voltar = gtk::Button::with_label(&format!("‹ {}", tr("Creative apps")));
        voltar.add_css_class("cr-voltar");
        voltar.set_halign(Align::Start);
        crate::central::posicao_debug_sempre("criativos voltar", voltar.upcast_ref());
        let w = Rc::downgrade(self);
        voltar.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                if let Some(g) = s.ao_voltar.borrow().as_ref() {
                    g();
                }
            }
        });
        d.append(&voltar);
        // the header
        let topo = caixa(Orientation::Horizontal, 16, "");
        topo.append(&marcador(app, 72));
        let tt = caixa(Orientation::Vertical, 8, "");
        tt.set_hexpand(true);
        tt.append(&{
            let l = rotulo(&app.nome, "cr-nome-grande");
            l.set_xalign(0.0);
            l
        });
        let chips = caixa(Orientation::Horizontal, 6, "");
        let chip = |t: &str, class: &str, glyph: Option<&str>| {
            let c = caixa(Orientation::Horizontal, 5, "cr-chip");
            if !class.is_empty() {
                c.add_css_class(class);
            }
            if let Some(g) = glyph {
                c.append(&ui_icon(g, "#8ED2C3", 12));
            }
            c.append(&rotulo(t, ""));
            c
        };
        chips.append(&chip(tr("Alpha"), "cr-chip-alfa", None));
        chips.append(&chip(nome_da_categoria(&app.categoria), "cr-chip-cat", None));
        let versao = estado.as_ref().map(|e| e.versao.clone()).filter(|v| !v.is_empty()).or_else(|| anuncio.as_ref().map(|a| a.versao.clone())).unwrap_or_default();
        if !versao.is_empty() {
            chips.append(&chip(&if inst { trf("v{v} pinned", &[("v", versao.clone())]) } else { format!("v{versao}") }, "", None));
        }
        if inst {
            chips.append(&chip(tr("verified: the project's signature"), "cr-chip-ok", Some("escudo")));
        }
        tt.append(&chips);
        let sobre = quebra(&app.sobre, "cr-sobre");
        tt.append(&sobre);
        topo.append(&tt);
        let bt = caixa(Orientation::Horizontal, 8, "");
        bt.set_valign(Align::Start);
        if inst {
            let rm = botao_icone("lixeira", "#C9CAD1", tr("Remove"), "cr-remover");
            crate::central::posicao_debug_sempre("criativos remover", rm.upcast_ref());
            let (w, idr) = (Rc::downgrade(self), app.id.clone());
            rm.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.remover(&idr);
                }
            });
            bt.append(&rm);
            let ab = botao(tr("Open"), true);
            ab.add_css_class("cr-cheio");
            crate::central::posicao_debug_sempre("criativos abrir", ab.upcast_ref());
            let ida = app.id.clone();
            ab.connect_clicked(move |_| open_creative(&ida, Vec::new()));
            bt.append(&ab);
        } else {
            bt.append(&self.acao(app, None));
        }
        topo.append(&bt);
        d.append(&topo);
        // the columns (side by side when wide, one under the other when narrow)
        let estreita = self.largura.get() < 1000;
        let corpo = caixa(if estreita { Orientation::Vertical } else { Orientation::Horizontal }, 20, "");
        let esq = caixa(Orientation::Vertical, 16, "");
        esq.set_hexpand(true);
        // screenshots: the project's (placeholders here, as in the design)
        let capt = caixa(Orientation::Horizontal, 16, "cr-capturas");
        for (i, cores) in [["#A5A1E2", "#8ED2C3", "#EFA2A6", "#7CC3DF", "#E5C98C"], ["#8ED2C3", "#EFA2A6", "#7CC3DF", "#E5C98C", "#A5A1E2"], ["#EFA2A6", "#7CC3DF", "#E5C98C", "#A5A1E2", "#8ED2C3"]].iter().enumerate() {
            if estreita && i > 0 {
                break;
            }
            let p = caixa(Orientation::Vertical, 8, "cr-captura");
            p.set_hexpand(true);
            let tela = caixa(Orientation::Vertical, 0, "cr-captura-tela");
            tela.set_size_request(-1, 120);
            p.append(&tela);
            for (j, c) in cores.iter().enumerate() {
                let b = caixa(Orientation::Horizontal, 0, "cr-barrinha");
                let tint = gtk::CssProvider::new();
                tint.load_from_string(&format!(".cr-barrinha {{ background: alpha({}, 0.8); }}", crate::tema::cor(c)));
                #[allow(deprecated)]
                b.style_context().add_provider(&tint, gtk::STYLE_PROVIDER_PRIORITY_USER + 2);
                b.set_size_request([100, 130, 160, 190, 220][(i + j) % 5], 12);
                b.set_halign(Align::Start);
                p.append(&b);
            }
            capt.append(&p);
        }
        esq.append(&capt);
        esq.append(&{
            let l = rotulo(tr("Screenshots provided by the project (markers here)"), "cr-nota");
            l.set_xalign(0.0);
            l
        });
        esq.append(&{
            let l = rotulo(tr("About"), "cr-secao");
            l.set_xalign(0.0);
            l
        });
        let g = gtk::Grid::new();
        g.set_column_spacing(40);
        g.set_row_spacing(10);
        let instalado_bytes = anuncio.as_ref().map(|a| a.tamanho_instalado).unwrap_or(0);
        let linhas: Vec<(&str, String)> = vec![
            (tr("Version"), if inst { trf("{v} · pinned by the Center (does not change without you or without automatic updates)", &[("v", versao.clone())]) } else if versao.is_empty() { "—".into() } else { versao.clone() }),
            (tr("Source"), tr("the project's signed releases · getartcraft.com, checked with the key KalangoOS pins").into()),
            (tr("Size"), trf("{size} to download · {installed} installed · user data in ~/.var/app", &[("size", tamanho(self.bytes_de(app))), ("installed", if instalado_bytes > 0 { tamanho(instalado_bytes) } else { "—".into() })])),
            (tr("Licence"), trf("{lic} (the text is in the app itself)", &[("lic", app.licenca.clone())])),
        ];
        for (i, (k, v)) in linhas.iter().enumerate() {
            let kl = rotulo(k, "cr-chave");
            kl.set_xalign(0.0);
            kl.set_valign(Align::Start);
            g.attach(&kl, 0, i as i32, 1, 1);
            g.attach(&quebra(v, "cr-valor"), 1, i as i32, 1, 1);
        }
        esq.append(&g);
        // automatic updates of this app
        let au = caixa(Orientation::Vertical, 8, "loja-cartao");
        au.add_css_class("cr-painel");
        let l1 = caixa(Orientation::Horizontal, 8, "");
        let tl = caixa(Orientation::Vertical, 2, "");
        tl.set_hexpand(true);
        tl.append(&{
            let l = rotulo(tr("Automatic updates"), "cr-painel-titulo");
            l.set_xalign(0.0);
            l
        });
        let ultima = estado.as_ref().map(|e| trf("Last update {when} · v{v}", &[("when", quando(e.quando)), ("v", e.versao.clone())])).unwrap_or_else(|| tr("Not installed").to_string());
        tl.append(&{
            let l = rotulo(&ultima, "loja-sub");
            l.set_xalign(0.0);
            l
        });
        // (o ponto de restauração do app, dito onde a pessoa liga a atualização)
        tl.append(&{
            let l = quebra(tr("Before each update the version in use is kept (the last 2); Undo goes back to it"), "cr-nota");
            l.set_margin_top(2);
            l
        });
        l1.append(&tl);
        let sw = gtk::Switch::new();
        sw.add_css_class("central-switch");
        sw.set_valign(Align::Center);
        sw.set_active(aj.atualiza(&app.id));
        crate::central::posicao_debug_sempre("criativos atualizacao do app", sw.upcast_ref());
        let ids = app.id.clone();
        sw.connect_state_set(move |_, on| {
            let mut a = ajustes::ler();
            let x = a.apps.entry(ids.clone()).or_default();
            x.atualizar = Some(on);
            if on {
                x.voltou = false;
            }
            let _ = ajustes::gravar(&a);
            glib::Propagation::Proceed
        });
        l1.append(&sw);
        au.append(&l1);
        if let Some(Fase::Pronta { versao }) = self.trabalhos.borrow().get(&app.id).map(|t| t.fase.lock().unwrap().clone()) {
            let p = caixa(Orientation::Horizontal, 6, "cr-chip");
            p.set_halign(Align::Start);
            p.append(&ui_icon("relogio", "#8ED2C3", 12));
            p.append(&rotulo(&trf("v{v} downloaded and verified · updates when you close the app (restore point first)", &[("v", versao)]), ""));
            au.append(&p);
        }
        esq.append(&au);
        corpo.append(&esq);
        // the right column
        let dir = caixa(Orientation::Vertical, 12, "");
        if !estreita {
            dir.set_size_request(460, -1);
        }
        // folders
        let fa = caixa(Orientation::Vertical, 8, "loja-cartao");
        fa.add_css_class("cr-painel");
        fa.append(&{
            let l = rotulo(tr("Files"), "cr-painel-titulo");
            l.set_xalign(0.0);
            l
        });
        fa.append(&{
            let l = rotulo(tr("Only the folders you chose:"), "loja-sub");
            l.set_xalign(0.0);
            l
        });
        let pastas_box = gtk::FlowBox::new();
        pastas_box.set_selection_mode(gtk::SelectionMode::None);
        pastas_box.set_max_children_per_line(6);
        pastas_box.set_column_spacing(6);
        pastas_box.set_row_spacing(6);
        // (as pastas e a rede são do kalangod: a escolha fica no estado dele, conferida; sem ele, nenhuma)
        let do_kalangod = self.agentes.borrow()[&app.id].clone();
        let pastas: Vec<String> = do_kalangod["pastas"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_else(|| app.pastas.clone());
        for p in &pastas {
            let c = caixa(Orientation::Horizontal, 5, "cr-chip");
            c.append(&ui_icon("pasta", "#8E8F99", 12));
            c.append(&rotulo(&pasta_legivel(p), ""));
            let x = gtk::Button::new();
            x.add_css_class("cr-x");
            x.set_child(Some(&rotulo("×", "")));
            x.set_tooltip_text(Some(tr("Remove this folder")));
            let (w, idp, pp) = (Rc::downgrade(self), app.id.clone(), p.clone());
            x.connect_clicked(move |_| {
                if let Some(s) = w.upgrade() {
                    s.mudar_pastas(&idp, |v| v.retain(|y| *y != pp));
                }
            });
            c.append(&x);
            pastas_box.append(&c);
        }
        let mais = gtk::Button::with_label(tr("+ folder"));
        mais.add_css_class("cr-chip");
        mais.add_css_class("cr-mais");
        crate::central::posicao_debug_sempre("criativos mais pasta", mais.upcast_ref());
        let (w, idm) = (Rc::downgrade(self), app.id.clone());
        mais.connect_clicked(move |b| {
            let Some(s) = w.upgrade() else { return };
            let dlg = gtk::FileDialog::builder().title(tr("Folder the app may see")).modal(true).build();
            let win = b.root().and_downcast::<gtk::Window>();
            let (w2, id2) = (Rc::downgrade(&s), idm.clone());
            dlg.select_folder(win.as_ref(), gio::Cancellable::NONE, move |r| {
                if let (Ok(f), Some(s)) = (r, w2.upgrade()) {
                    if let Some(p) = f.path() {
                        let casa = glib::home_dir();
                        // (só pastas dentro da casa; "~/" no lugar dela)
                        if let Ok(rel) = p.strip_prefix(&casa) {
                            let v = format!("~/{}", rel.display());
                            s.mudar_pastas(&id2, |l| if !l.contains(&v) { l.push(v.clone()) });
                        }
                    }
                }
            });
        });
        pastas_box.append(&mais);
        fa.append(&pastas_box);
        dir.append(&fa);
        // network
        let re = caixa(Orientation::Vertical, 8, "loja-cartao");
        re.add_css_class("cr-painel");
        let rl = caixa(Orientation::Horizontal, 8, "");
        let rt = rotulo(tr("Network"), "cr-painel-titulo");
        rt.set_xalign(0.0);
        rt.set_hexpand(true);
        rl.append(&rt);
        rl.append(&rotulo(tr("Allow network"), "loja-sub"));
        let rs = gtk::Switch::new();
        rs.add_css_class("central-switch");
        rs.set_valign(Align::Center);
        rs.set_active(do_kalangod["rede"].as_bool().unwrap_or(false));
        crate::central::posicao_debug_sempre("criativos rede", rs.upcast_ref());
        let (w, idn) = (Rc::downgrade(self), app.id.clone());
        rs.connect_state_set(move |sw, on| {
            match crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "definir", "app": idn, "rede": on})) {
                Ok(v) => {
                    if let Some(s) = w.upgrade() {
                        *s.agentes.borrow_mut() = v;
                    }
                }
                Err(e) => {
                    eprintln!("kalango-shell: apps criativos: rede: {e}");
                    let sw = sw.clone();
                    glib::idle_add_local_once(move || sw.set_active(!on));
                }
            }
            glib::Propagation::Proceed
        });
        rl.append(&rs);
        re.append(&rl);
        re.append(&quebra(&trf("Off by default. {app} does not need the internet to edit; turn it on only to fetch online media. The App Center keeps updating the app even with its network off.", &[("app", app.nome.clone())]), "loja-sub"));
        dir.append(&re);
        // agents
        let ag = caixa(Orientation::Vertical, 8, "loja-cartao");
        ag.add_css_class("cr-painel");
        ag.add_css_class("cr-agentes");
        let al = caixa(Orientation::Horizontal, 8, "");
        al.append(&ui_icon("spark", "#A5A1E2", 14));
        let at = rotulo(tr("Agents can use this app"), "cr-painel-titulo");
        at.set_xalign(0.0);
        at.set_hexpand(true);
        al.append(&at);
        let asw = gtk::Switch::new();
        asw.add_css_class("central-switch");
        asw.set_valign(Align::Center);
        asw.set_active(self.agentes.borrow()[&app.id]["agentes"].as_bool().unwrap_or(true));
        crate::central::posicao_debug_sempre("criativos agentes", asw.upcast_ref());
        let (w, idg) = (Rc::downgrade(self), app.id.clone());
        asw.connect_state_set(move |sw, on| {
            let r = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "definir", "app": idg, "agentes": on}));
            match r {
                Ok(v) => {
                    if let Some(s) = w.upgrade() {
                        *s.agentes.borrow_mut() = v;
                    }
                }
                Err(e) => {
                    eprintln!("kalango-shell: apps criativos: agentes: {e}");
                    // (sem o kalangod, nada muda: o interruptor volta)
                    let sw = sw.clone();
                    glib::idle_add_local_once(move || sw.set_active(!on));
                }
            }
            glib::Propagation::Proceed
        });
        al.append(&asw);
        ag.append(&al);
        for (cor, o_que, nivel, class) in [("#8ED2C3", tr("Read projects and media"), tr("free"), "cr-nivel-livre"), ("#E5C98C", tr("Save and export"), tr("asks to confirm"), "cr-nivel-confirmar"), ("#EFA2A6", tr("Drive the window"), tr("asks for approval (d1)"), "cr-nivel-aprovar")] {
            let l = caixa(Orientation::Horizontal, 8, "");
            let dot = caixa(Orientation::Horizontal, 0, "cr-ponto");
            dot.set_size_request(6, 6);
            dot.set_valign(Align::Center);
            let tint = gtk::CssProvider::new();
            tint.load_from_string(&format!(".cr-ponto {{ background: {}; }}", crate::tema::cor(cor)));
            #[allow(deprecated)]
            dot.style_context().add_provider(&tint, gtk::STYLE_PROVIDER_PRIORITY_USER + 2);
            l.append(&dot);
            let t = rotulo(o_que, "cr-valor");
            t.set_xalign(0.0);
            t.set_hexpand(true);
            l.append(&t);
            l.append(&rotulo(nivel, class));
            ag.append(&l);
        }
        ag.append(&quebra(tr("Agents' permissions live in the Agents app; here, only what applies to this app. A tool KalangoOS does not know yet always asks for approval."), "cr-nota"));
        dir.append(&ag);
        corpo.append(&dir);
        d.append(&corpo);
        if debug() {
            eprintln!("debug criativos: detalhe {} ({}{})", app.id, if inst { "instalado" } else { "não instalado" }, if estreita { ", estreita" } else { "" });
        }
    }

    fn mudar_pastas(self: &Rc<Self>, id: &str, f: impl FnOnce(&mut Vec<String>)) {
        let Some(app) = catalogo().app(id) else { return };
        let mut v: Vec<String> = self.agentes.borrow()[&app.id]["pastas"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_else(|| app.pastas.clone());
        f(&mut v);
        // (o kalangod confere, guarda e refaz as permissões; uma pasta recusada, ou sem o kalangod, nada muda)
        match crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "definir", "app": id, "pastas": v})) {
            Ok(x) => *self.agentes.borrow_mut() = x,
            Err(e) => {
                // (why the folder was refused, said to the person: kalangod's own words)
                eprintln!("kalango-shell: apps criativos: pastas não mudaram: {e}");
                if let Some(sh) = crate::shell_atual() {
                    sh.notificar(tr("App Center"), tr("The folders did not change"), &e);
                }
            }
        }
        self.mostrar_detalhe(id);
    }

    // ------------------------------------------------------------------------------------------ updates

    /// A parte dos Apps criativos na página Atualizações.
    pub fn mostrar_atualizacoes(self: &Rc<Self>) {
        let b = &self.atualizacoes;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let topo = caixa(Orientation::Horizontal, 10, "");
        let t = rotulo(tr("Creative apps"), "loja-secao");
        t.set_xalign(0.0);
        topo.append(&t);
        let on = ajustes::ler().atualizar;
        let pill = caixa(Orientation::Horizontal, 6, if on { "cr-pilula-ok" } else { "cr-pilula" });
        let dot = caixa(Orientation::Horizontal, 0, if on { "cr-ponto-teal" } else { "cr-ponto-cinza" });
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        pill.append(&dot);
        pill.append(&rotulo(if on { tr("Automatic updates on") } else { tr("Automatic updates off") }, ""));
        pill.set_valign(Align::Center);
        topo.append(&pill);
        let mola = caixa(Orientation::Horizontal, 0, "");
        mola.set_hexpand(true);
        topo.append(&mola);
        let procurar = botao_icone("reiniciar", "#C9CAD1", tr("Check now"), "loja-acao");
        crate::central::posicao_debug_sempre("criativos procurar agora", procurar.upcast_ref());
        let w = Rc::downgrade(self);
        procurar.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.procurar_agora();
            }
        });
        topo.append(&procurar);
        b.append(&topo);
        let estado = instalar::ler_estado();
        let mut linhas = 0;
        let trabalhos: Vec<(String, Trabalho)> = self.trabalhos.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut ordem: Vec<&(String, Trabalho)> = trabalhos.iter().collect();
        // (como no design: o que corre, o que espera, o que o KalangoOS segurou, o que falhou)
        let peso = |f: &Fase| match f {
            Fase::Verificando | Fase::Baixando | Fase::Instalando => 0,
            Fase::Pronta { .. } => 1,
            Fase::Falhou(Erro::Verificacao(Falha::Bloqueada { .. })) => 2,
            Fase::Falhou(Erro::SemEspaco { .. }) => 4,
            _ => 3,
        };
        ordem.sort_by_key(|(id, t)| (peso(&t.fase.lock().map(|f| f.clone()).unwrap_or(Fase::Feito)), catalogo().apps.iter().position(|a| &a.id == id).unwrap_or(99)));
        for (id, t) in ordem {
            let Some(app) = catalogo().app(id) else { continue };
            let fase = t.fase.lock().map(|f| f.clone()).unwrap_or(Fase::Verificando);
            if fase == Fase::Feito {
                continue;
            }
            b.append(&self.linha(app, t, &fase));
            linhas += 1;
        }
        let com_linha: Vec<String> = self.trabalhos.borrow().iter().filter(|(_, t)| !matches!(*t.fase.lock().unwrap(), Fase::Feito)).map(|(k, _)| k.clone()).collect();
        let instalados = self.instalados.borrow().len();
        // the recent ones, each with Undo
        let rec = caixa(Orientation::Horizontal, 10, "");
        rec.set_margin_top(10);
        let rt = rotulo(tr("Recent"), "cr-secao");
        rt.set_xalign(0.0);
        rec.append(&rt);
        let _ = linhas;
        let em_dia = self.instalados.borrow().iter().filter(|i| !com_linha.contains(i)).count();
        if instalados > 0 {
            let p = caixa(Orientation::Horizontal, 6, "cr-chip");
            p.append(&ui_icon("check", "#8ED2C3", 12));
            let ver = if estado.verificado_em > 0 { trf(" · checked {when}", &[("when", quando(estado.verificado_em))]) } else { String::new() };
            p.append(&rotulo(&format!("{}{ver}", trf("the other {n} apps: all up to date", &[("n", em_dia.to_string())])), ""));
            p.set_valign(Align::Center);
            rec.append(&p);
        }
        b.append(&rec);
        let g = gtk::Grid::new();
        g.set_column_spacing(40);
        g.set_row_spacing(8);
        let mut hist: Vec<(String, String, String, u64)> = estado.apps.iter().flat_map(|(id, i)| i.historico.iter().take(1).map(move |(a, b, q)| (id.clone(), a.clone(), b.clone(), *q))).collect();
        hist.sort_by_key(|x| std::cmp::Reverse(x.3));
        for (n, (id, de, para, q)) in hist.iter().take(6).enumerate() {
            let Some(app) = catalogo().app(id) else { continue };
            let nm = rotulo(&app.nome, "cr-valor");
            nm.set_xalign(0.0);
            g.attach(&nm, 0, n as i32, 1, 1);
            g.attach(&rotulo(&format!("v{de} → v{para}"), "cr-mono"), 1, n as i32, 1, 1);
            g.attach(&rotulo(&quando(*q), "loja-sub"), 2, n as i32, 1, 1);
            let tem_ponto = estado.apps.get(id).is_some_and(|i| !i.historico.is_empty());
            if tem_ponto {
                let d = gtk::Button::with_label(tr("Undo (back to the restore point)"));
                d.add_css_class("cr-link");
                crate::central::posicao_debug_sempre(&format!("criativos desfazer {}", app.imagem()), d.upcast_ref());
                let (w, idv) = (Rc::downgrade(self), id.clone());
                d.connect_clicked(move |_| {
                    if let Some(s) = w.upgrade() {
                        s.voltar(&idv);
                    }
                });
                g.attach(&d, 3, n as i32, 1, 1);
            }
        }
        if hist.is_empty() {
            let l = rotulo(tr("No update of a creative app yet."), "loja-sub");
            l.set_xalign(0.0);
            g.attach(&l, 0, 0, 1, 1);
        }
        b.append(&g);
    }

    fn linha(self: &Rc<Self>, app: &App, t: &Trabalho, fase: &Fase) -> gtk::Box {
        let card = caixa(Orientation::Horizontal, 12, "loja-cartao");
        card.add_css_class("cr-linha");
        card.append(&marcador(app, 48));
        let tx = caixa(Orientation::Vertical, 4, "");
        tx.set_hexpand(true);
        tx.set_valign(Align::Center);
        tx.append(&{
            let l = rotulo(&app.nome, "loja-app-nome");
            l.set_xalign(0.0);
            l
        });
        let status = caixa(Orientation::Horizontal, 6, "");
        let (glyph, cor, texto, class) = match fase {
            Fase::Verificando | Fase::Baixando | Fase::Instalando => (None, "", texto_da_fase(fase, t.andamento.feito.load(Ordering::Relaxed), t.andamento.total.load(Ordering::Relaxed)), "loja-sub"),
            Fase::Pronta { versao } => (Some("relogio"), "#8ED2C3", trf("v{v} ready · updates when you close the app · restore point kept", &[("v", versao.clone())]), "loja-sub"),
            Fase::Falhou(Erro::Verificacao(Falha::Bloqueada { versao, motivo })) => {
                let ficou = instalar::ler_estado().apps.get(&app.id).map(|i| i.versao.clone()).unwrap_or_default();
                let texto = if ficou.is_empty() {
                    trf("Version {v} is blocked by KalangoOS: {why}", &[("v", versao.clone()), ("why", motivo.clone())])
                } else {
                    trf("This version ({v}) was blocked by KalangoOS: it stayed on the previous one (v{old}). Reason: {why}.", &[("v", versao.clone()), ("old", ficou), ("why", motivo.clone())])
                };
                (Some("escudo"), "#E5C98C", texto, "cr-aviso")
            }
            Fase::Falhou(Erro::Verificacao(Falha::Assinatura(_) | Falha::Formato(_))) => (Some("alerta"), "#EFA2A6", tr("Signature does not match: installation refused. Nothing was changed on the system.").to_string(), "cr-erro"),
            Fase::Falhou(Erro::SemEspaco { faltam }) => (Some("alerta"), "#E5C98C", trf("No space: {size} missing on the system disk.", &[("size", tamanho(*faltam))]), "cr-aviso"),
            Fase::Falhou(e) => (Some("alerta"), "#EFA2A6", texto_do_erro(app, e), "cr-erro"),
            Fase::Feito => (None, "", String::new(), "loja-sub"),
        };
        if let Some(g) = glyph {
            status.append(&ui_icon(g, cor, 12));
        }
        let sl = quebra(&texto, class);
        status.append(&sl);
        tx.append(&status);
        if matches!(fase, Fase::Verificando | Fase::Baixando | Fase::Instalando) {
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.add_css_class("plano-barra");
            bar.set_hexpand(true);
            self.barras.borrow_mut().insert(app.id.clone(), (bar.downgrade(), sl.downgrade()));
            tx.append(&bar);
            self.acompanhar();
        }
        card.append(&tx);
        let (rotulo_b, principal, acao): (&str, bool, u8) = match fase {
            Fase::Verificando | Fase::Baixando | Fase::Instalando => (tr("Pause"), false, 0),
            Fase::Pronta { .. } => (tr("Restart app now"), true, 1),
            Fase::Falhou(Erro::Verificacao(Falha::Bloqueada { .. })) => (tr("Details"), false, 2),
            Fase::Falhou(Erro::SemEspaco { .. }) => (tr("Free up space"), false, 3),
            _ => (tr("Try again"), false, 4),
        };
        if matches!(fase, Fase::Falhou(Erro::Verificacao(Falha::Assinatura(_) | Falha::Formato(_)))) {
            card.add_css_class("cr-linha-erro");
        }
        let bt = botao(rotulo_b, principal);
        if principal {
            bt.add_css_class("cr-cheio");
        }
        crate::central::posicao_debug_sempre(&format!("criativos linha {} {}", app.imagem(), acao), bt.upcast_ref());
        let (w, id, andamento) = (Rc::downgrade(self), app.id.clone(), t.andamento.clone());
        bt.connect_clicked(move |_| {
            let Some(s) = w.upgrade() else { return };
            match acao {
                0 => andamento.cancelar.store(true, Ordering::Relaxed),
                1 => {
                    // (fecha o app; a atualização entra e ele abre de novo)
                    let _ = std::process::Command::new(instalar::flatpak()).args(["kill", &id]).status();
                    s.aplicar_prontas(true);
                }
                2 => {
                    if let Some(f) = s.ao_abrir.borrow().as_ref() {
                        f(&id);
                    }
                }
                3 => crate::configuracoes::abrir_painel("sistema"),
                _ => s.instalar(&id, true),
            }
        });
        card.append(&bt);
        card
    }

    /// "Procurar agora": o índice de novo e, para cada app instalado com versão nova, a atualização (automática ou não:
    /// a pessoa pediu).
    pub fn procurar_agora(self: &Rc<Self>) {
        self.indice_estado.set(0);
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(|| verificar::indice(catalogo())).await.unwrap_or_else(|_| Err(Falha::Rede("?".into())));
            let Some(s) = w.upgrade() else { return };
            if let Ok(v) = r {
                *s.anuncios.borrow_mut() = v.into_iter().collect();
                s.indice_estado.set(2);
                let mut e = instalar::ler_estado();
                e.verificado_em = instalar::agora();
                let _ = instalar::gravar_estado(&e);
                s.atualizar_o_que_mudou(false);
            }
            s.mudou();
        });
    }

    /// Para cada app instalado cuja versão anunciada é outra: atualiza (só os de atualização automática ligada, se
    /// `so_automaticos`).
    fn atualizar_o_que_mudou(self: &Rc<Self>, so_automaticos: bool) {
        let estado = instalar::ler_estado();
        let aj = ajustes::ler();
        let inst = instalar::instalados();
        *self.instalados.borrow_mut() = inst.clone();
        let anuncios = self.anuncios.borrow().clone();
        for id in inst {
            let Some(a) = anuncios.get(&id) else { continue };
            let atual = estado.apps.get(&id).map(|i| i.digest.clone()).unwrap_or_default();
            if a.digest == atual || (so_automaticos && !aj.atualiza(&id)) {
                continue;
            }
            if debug() {
                eprintln!("debug criativos: {id}: versão nova {}", a.versao);
            }
            self.instalar(&id, true);
        }
    }

    /// Aplica as atualizações prontas cujos apps estão fechados.
    fn aplicar_prontas(self: &Rc<Self>, logo: bool) {
        let prontas = self.prontas.clone();
        let trabalhos: Vec<(String, Trabalho)> = self.trabalhos.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        std::thread::spawn(move || {
            if logo {
                std::thread::sleep(Duration::from_secs(2));
            }
            let abertos = instalar::em_execucao();
            let ids: Vec<String> = prontas.lock().map(|p| p.keys().cloned().collect()).unwrap_or_default();
            for id in ids {
                if abertos.contains(&id) {
                    continue;
                }
                let Some(pr) = prontas.lock().ok().and_then(|mut p| p.remove(&id)) else { continue };
                let Some(app) = catalogo().app(&id) else { continue };
                let t = trabalhos.iter().find(|x| x.0 == id).map(|x| x.1.clone());
                if let Some(t) = &t {
                    *t.fase.lock().unwrap() = Fase::Instalando;
                }
                let r = instalar_preparado(app, &pr);
                // (opened only once it installed and kalangod made its permissions again: a failed update keeps it closed)
                let installed = r.is_ok();
                if let Some(t) = &t {
                    *t.fase.lock().unwrap() = match r {
                        Ok(_) => Fase::Feito,
                        Err(e) => Fase::Falhou(e),
                    };
                }
                if installed && logo && abertos.is_empty() {
                    let _ = std::process::Command::new(instalar::flatpak()).args(["run", &id]).spawn();
                }
            }
        });
        self.acompanhar();
    }
}

// ------------------------------------------------------------------------------------------------ Ajustes › Apps

/// Ajustes › Apps › um app criativo (design 346774), na Central de agentes: o que ele vê (pastas), rede, câmera e
/// microfone, se os agentes podem usá-lo, atualização automática, o ponto de restauração ("Voltar para…", o Desfazer)
/// e Remover. Os controles são os mesmos do detalhe na Central de Apps (mesmo estado: o do kalangod e o da Central).
pub struct AjustesApp {
    pub raiz: gtk::Box,
    atual: RefCell<String>,
    remover_aberto: Cell<bool>,
}

thread_local! {
    static AJUSTES: RefCell<Option<Rc<AjustesApp>>> = const { RefCell::new(None) };
}

pub fn ajustes_app() -> Rc<AjustesApp> {
    if let Some(s) = AJUSTES.with(|m| m.borrow().clone()) {
        return s;
    }
    let s = Rc::new(AjustesApp { raiz: caixa(Orientation::Vertical, 16, "cr-ajustes"), atual: RefCell::default(), remover_aberto: Cell::new(false) });
    AJUSTES.with(|m| *m.borrow_mut() = Some(s.clone()));
    s
}

fn linha_ajuste(glyph: &str, titulo: &str, sub: &str, controle: &gtk::Widget) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "loja-cartao");
    r.add_css_class("cr-ajuste");
    let ic = ui_icon(glyph, "#8E8F99", 14);
    ic.set_valign(Align::Center);
    r.append(&ic);
    let t = caixa(Orientation::Vertical, 2, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&ellipsized(titulo, "cr-painel-titulo"));
    t.append(&quebra(sub, "loja-sub"));
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

/// O tamanho de uma pasta (os dados do app em ~/.var/app), sem seguir links.
fn tamanho_da_pasta(p: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut pilha = vec![p.to_path_buf()];
    while let Some(d) = pilha.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                pilha.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

impl AjustesApp {
    pub fn mostrar(self: &Rc<Self>, id: &str) {
        let Some(app) = catalogo().app(id) else { return };
        if *self.atual.borrow() != app.id {
            self.remover_aberto.set(false);
        }
        *self.atual.borrow_mut() = app.id.clone();
        let r = &self.raiz;
        while let Some(c) = r.first_child() {
            r.remove(&c);
        }
        let central = compartilhado();
        let estado = instalar::ler_estado().apps.get(&app.id).cloned();
        let do_kalangod = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "ler"})).map(|v| v[&app.id].clone()).unwrap_or(serde_json::json!({}));
        let migalha = rotulo(&format!("{} › {}", tr("Agent settings"), tr("Apps")), "cr-migalha");
        migalha.set_xalign(0.0);
        r.append(&migalha);
        // the header
        let topo = caixa(Orientation::Horizontal, 14, "");
        topo.append(&marcador(app, 56));
        let tt = caixa(Orientation::Vertical, 2, "");
        tt.set_hexpand(true);
        tt.set_valign(Align::Center);
        let n = rotulo(&app.nome, "cr-nome-grande");
        n.set_xalign(0.0);
        tt.append(&n);
        let dados = tamanho_da_pasta(&glib::home_dir().join(".var/app").join(&app.id));
        let instalado = central.anuncios.borrow().get(&app.id).map(|a| a.tamanho_instalado).filter(|b| *b > 0)
            .or_else(|| instalar::rodar(&["--user", "info", "--show-size", &app.id]).ok().and_then(|s| s.trim().parse().ok())).unwrap_or(0);
        let versao = estado.as_ref().map(|e| e.versao.clone()).unwrap_or_default();
        let mut partes = vec![tr("Installed by the App Center").to_string()];
        if !versao.is_empty() {
            partes.push(format!("v{versao}"));
        }
        if instalado > 0 {
            partes.push(trf("{size} + {data} of data", &[("size", tamanho(instalado)), ("data", tamanho(dados))]));
        }
        let sub = rotulo(&partes.join(" · "), "loja-sub");
        sub.set_xalign(0.0);
        tt.append(&sub);
        topo.append(&tt);
        let na_central = gtk::Button::with_label(tr("Open in the App Center"));
        na_central.add_css_class("cr-link-texto");
        na_central.set_valign(Align::Center);
        crate::central::posicao_debug_sempre("ajustes app abrir na central", na_central.upcast_ref());
        let idc = app.id.clone();
        na_central.connect_clicked(move |b| {
            if let Some(sh) = b.root().and_then(|_| crate::shell_atual()) {
                crate::app_store::open_page(&sh, &format!("criativo:{idc}"));
            }
        });
        topo.append(&na_central);
        let rm = botao_icone("lixeira", "#1C1D24", tr("Remove"), "cr-remover-cheio");
        crate::central::posicao_debug_sempre("ajustes app remover", rm.upcast_ref());
        let w = Rc::downgrade(self);
        rm.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.remover_aberto.set(true);
                let id = s.atual.borrow().clone();
                s.mostrar(&id);
            }
        });
        topo.append(&rm);
        r.append(&topo);
        let estreita = central.largura.get() < 1000;
        let corpo = caixa(if estreita { Orientation::Vertical } else { Orientation::Horizontal }, 20, "");
        let esq = caixa(Orientation::Vertical, 10, "");
        esq.set_hexpand(true);
        // files
        let nomes: Vec<String> = do_kalangod["pastas"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_else(|| app.pastas.clone());
        let alterar = botao(tr("Change (a setting)"), false);
        crate::central::posicao_debug_sempre("ajustes app pastas", alterar.upcast_ref());
        let ida = app.id.clone();
        alterar.connect_clicked(move |_| {
            if let Some(sh) = crate::shell_atual() {
                crate::app_store::open_page(&sh, &format!("criativo:{ida}"));
            }
        });
        let lista: Vec<String> = nomes.iter().map(|p| pasta_legivel(p)).collect();
        esq.append(&linha_ajuste("pasta", tr("Files"), &if lista.is_empty() { tr("No folder: the app sees only what you open in it").to_string() } else { lista.join(", ") }, alterar.upcast_ref()));
        // network
        let rede_on = do_kalangod["rede"].as_bool().unwrap_or(false);
        let rs = gtk::Switch::new();
        rs.add_css_class("central-switch");
        rs.set_active(rede_on);
        crate::central::posicao_debug_sempre("ajustes app rede", rs.upcast_ref());
        let idn = app.id.clone();
        let w = Rc::downgrade(self);
        rs.connect_state_set(move |sw, on| {
            match crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "definir", "app": idn, "rede": on})) {
                Ok(_) => {
                    if let Some(s) = w.upgrade() {
                        let id = s.atual.borrow().clone();
                        glib::idle_add_local_once(move || ajustes_app().mostrar(&id));
                    }
                }
                Err(e) => {
                    eprintln!("kalango-shell: apps criativos: rede: {e}");
                    let sw = sw.clone();
                    glib::idle_add_local_once(move || sw.set_active(!on));
                }
            }
            glib::Propagation::Proceed
        });
        esq.append(&linha_ajuste("globo", tr("Network"), if rede_on { tr("On · the app can reach the internet") } else { tr("Off · the app does not reach the internet") }, rs.upcast_ref()));
        // camera and microphone: the portals ask when the app wants them (Privacy decides)
        let cam = botao(tr("Change (a setting)"), false);
        crate::central::posicao_debug_sempre("ajustes app camera", cam.upcast_ref());
        cam.connect_clicked(|_| crate::configuracoes::abrir_painel("privacidade"));
        esq.append(&linha_ajuste("camera", tr("Camera and microphone"), tr("Asks when the app wants them"), cam.upcast_ref()));
        // agents
        let asw = gtk::Switch::new();
        asw.add_css_class("central-switch");
        asw.set_active(do_kalangod["agentes"].as_bool().unwrap_or(true));
        crate::central::posicao_debug_sempre("ajustes app agentes", asw.upcast_ref());
        let idg = app.id.clone();
        asw.connect_state_set(move |sw, on| {
            if let Err(e) = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "definir", "app": idg, "agentes": on})) {
                eprintln!("kalango-shell: apps criativos: agentes: {e}");
                let sw = sw.clone();
                glib::idle_add_local_once(move || sw.set_active(!on));
            }
            glib::Propagation::Proceed
        });
        esq.append(&linha_ajuste("spark", tr("Agents can use it"), tr("Read: free · Save/export: confirmation · Window: approval"), asw.upcast_ref()));
        // automatic updates
        let aj = ajustes::ler();
        let usw = gtk::Switch::new();
        usw.add_css_class("central-switch");
        usw.set_active(aj.atualiza(&app.id));
        crate::central::posicao_debug_sempre("ajustes app atualizacao", usw.upcast_ref());
        let idu = app.id.clone();
        usw.connect_state_set(move |_, on| {
            let mut a = ajustes::ler();
            let x = a.apps.entry(idu.clone()).or_default();
            x.atualizar = Some(on);
            if on {
                x.voltou = false;
            }
            let _ = ajustes::gravar(&a);
            glib::Propagation::Proceed
        });
        let ultima = estado.as_ref().map(|e| trf("On · last {when} · applied when the app closes", &[("when", quando(e.quando))])).unwrap_or_else(|| tr("Applied when the app closes").to_string());
        esq.append(&linha_ajuste("download", tr("Automatic updates"), &if aj.atualiza(&app.id) { ultima } else { tr("Off").to_string() }, usw.upcast_ref()));
        // the restore point: the previous version kalangod keeps (the Undo)
        let anterior = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "anterior", "app": app.id})).ok();
        let volta: gtk::Widget = match anterior.as_ref().and_then(|v| v["versao"].as_str().map(str::to_string)) {
            Some(v) => {
                let b = botao(&trf("Back to v{v}", &[("v", v)]), false);
                crate::central::posicao_debug_sempre("ajustes app voltar", b.upcast_ref());
                let idv = app.id.clone();
                b.connect_clicked(move |_| compartilhado().voltar(&idv));
                b.upcast()
            }
            None => rotulo(tr("No earlier version yet"), "loja-sub").upcast(),
        };
        esq.append(&linha_ajuste("desfazer", tr("Restore point"), tr("Before each update the version in use is kept (the last 2); Undo goes back to it"), &volta));
        corpo.append(&esq);
        // the removal panel
        let dir = caixa(Orientation::Vertical, 8, "loja-cartao");
        dir.add_css_class("cr-painel");
        if !estreita {
            dir.set_size_request(480, -1);
        }
        dir.set_valign(Align::Start);
        let rt = rotulo(tr("Remove"), "cr-remover-titulo");
        rt.set_xalign(0.0);
        dir.append(&rt);
        for l in [tr("Removes the app and, if you want, its data (~/.var/app)."), tr("Projects in your folders are never deleted.")] {
            dir.append(&quebra(l, "cr-valor"));
        }
        dir.append(&quebra(&trf("Agents that used {app} will ask for another app or say it is missing.", &[("app", app.nome.clone())]), "cr-valor"));
        if self.remover_aberto.get() {
            let acoes = caixa(Orientation::Horizontal, 8, "");
            acoes.set_margin_top(6);
            for (rotulo_b, dados, chave) in [(tr("Remove the app only"), false, "so"), (tr("Remove with its data"), true, "dados")] {
                let b = botao(rotulo_b, false);
                crate::central::posicao_debug_sempre(&format!("ajustes app remover {chave}"), b.upcast_ref());
                let (idr, w) = (app.id.clone(), Rc::downgrade(self));
                b.connect_clicked(move |_| {
                    let Some(app) = catalogo().app(&idr).cloned() else { return };
                    let w = w.clone();
                    glib::spawn_future_local(async move {
                        let r = gio::spawn_blocking(move || instalar::remover(&app, dados)).await.unwrap_or_else(|_| Err("?".into()));
                        if let Err(e) = r {
                            eprintln!("kalango-shell: apps criativos: remover: {e}");
                        }
                        compartilhado().recarregar();
                        if let Some(s) = w.upgrade() {
                            s.remover_aberto.set(false);
                        }
                        if let Some(sh) = crate::shell_atual() {
                            sh.abrir_central(Some("ajustes"));
                        }
                    });
                });
                acoes.append(&b);
            }
            dir.append(&acoes);
        }
        corpo.append(&dir);
        r.append(&corpo);
        if debug() {
            eprintln!("debug criativos: ajustes de {} ({})", app.id, if self.remover_aberto.get() { "remover aberto" } else { "normal" });
        }
    }
}

/// As permissões de um app instalado: quem as faz é o kalangod, do estado dele (pastas conferidas, rede escolhida). Sem
/// o kalangod, só as restrições: sem rede, sem o serviço do Flatpak, sem pasta nenhuma — e o erro volta (a tela mostra).
/// Devolve as pastas que o kalangod acabou de deixar de fora (com o porquê), para a pessoa ficar sabendo.
pub fn permissoes(id: &str) -> Result<Vec<(String, String)>, String> {
    let r = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "aplicar", "app": id}));
    sem_kalangod(id, r)
}

/// Whether a desktop id is one of the creative apps of the catalog (exactly its id).
pub fn is_creative(id: &str) -> bool {
    catalogo().apps.iter().any(|a| a.id == id)
}

/// Opens an installed creative app from the shell, with `files` if any (the App Center's Open, the launcher, the dock,
/// Files and the desktop, "Open with"): first kalangod checks its folders again and makes its Flatpak permissions again
/// (`permissoes`: a folder swapped since the person chose it is dropped there, and the person is told why; without
/// kalangod the app sees no folder), then the app starts.
pub fn open_creative(id: &str, files: Vec<gio::File>) {
    let id = id.to_string();
    glib::spawn_future_local(async move {
        let i = id.clone();
        match gio::spawn_blocking(move || check_before_opening(&i)).await {
            Ok(Ok(dropped)) => tell_dropped(&id, &dropped),
            Ok(Err(e)) => eprintln!("kalango-shell: {id}: {e}"),
            Err(_) => eprintln!("kalango-shell: {id}: the permissions were not checked"),
        }
        let r = match gio::DesktopAppInfo::new(&format!("{id}.desktop")) {
            Some(d) => crate::apps::launch_info(&d, &files),
            None => std::process::Command::new(instalar::flatpak())
                .args(["run", &id])
                .args(files.iter().filter_map(|f| f.path()))
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string()),
        };
        if let Err(e) = r {
            eprintln!("kalango-shell: abrir {id}: {e}");
        }
    });
}

/// At the start of the session kalangod checks every installed creative app's folders again and makes its permissions
/// again, so an override written before (by an older KalangoOS, say) never outlives the rules, whatever opens the app
/// later; the person is told of each folder left out. Kalangod not answering yet: tried again a little later, and nothing
/// is changed meanwhile.
pub fn refresh_permissions_at_start() {
    glib::spawn_future_local(async {
        for _ in 0..10 {
            // (kalangod does each app on its own: one it cannot do is said in "erro" and does not stop the others)
            let r = gio::spawn_blocking(|| crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "reaplicar"}))).await;
            if let Ok(Ok(v)) = r {
                for a in v["apps"].as_array().into_iter().flatten() {
                    let id = a["app"].as_str().unwrap_or("");
                    if let Some(e) = a["erro"].as_str() {
                        eprintln!("kalango-shell: apps criativos: {id}: {e}");
                    }
                    tell_dropped(id, &newly_dropped(a));
                }
                return;
            }
            // (kalangod is not answering yet: again in a while)
            glib::timeout_future_seconds(30).await;
        }
    });
}

/// Right before the shell opens a creative app: kalangod checks its folders again and makes its permissions again
/// (`abrir`; without kalangod, as `permissoes`, the app sees no folder). Returns the folders it has just left out.
fn check_before_opening(id: &str) -> Result<Vec<(String, String)>, String> {
    let r = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "abrir", "app": id}));
    sem_kalangod(id, r)
}

/// `tell_dropped` from another thread (the install and the restore run outside the interface's thread).
fn tell_dropped_later(id: &str, dropped: Vec<(String, String)>) {
    if dropped.is_empty() {
        return;
    }
    let id = id.to_string();
    glib::MainContext::default().invoke(move || tell_dropped(&id, &dropped));
}

/// The folders kalangod has just left out of an app's permissions (`novas_recusas` of `aplicar`): (folder, why).
fn newly_dropped(v: &serde_json::Value) -> Vec<(String, String)> {
    v["novas_recusas"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| Some((x["pasta"].as_str()?.to_string(), x["motivo"].as_str()?.to_string())))
        .collect()
}

/// Tells the person, in a notification, of each folder kalangod has just left out of a creative app, and why (also in
/// the Timeline, from kalangod).
fn tell_dropped(id: &str, dropped: &[(String, String)]) {
    let (Some(app), Some(sh)) = (catalogo().app(id), crate::shell_atual()) else { return };
    for (_, why) in dropped {
        sh.notificar(tr("App Center"), &trf("{app} no longer sees a folder", &[("app", app.nome.clone())]), why);
    }
}

/// O que fazer com a resposta do kalangod (separado para o teste).
fn sem_kalangod(id: &str, r: Result<serde_json::Value, String>) -> Result<Vec<(String, String)>, String> {
    match r {
        Ok(v) => Ok(newly_dropped(&v)),
        Err(e) => {
            let _ = instalar::aplicar_permissoes(id, false, &[]);
            Err(trf("the folders and the network were not granted ({e}): the app sees no folder of yours until KalangoOS's service answers", &[("e", e)]))
        }
    }
}

/// Os tipos de arquivo de um app recém-instalado (do `.desktop` dele, que o Flatpak exporta): um tipo que um app do
/// KalangoOS abre continua com o app do KalangoOS como padrão (escrito, para o app novo não tomar o lugar); um tipo que
/// só os apps criativos abrem fica com o app novo se ainda não houver padrão.
pub fn associar(id: &str) {
    let Some(d) = gio::DesktopAppInfo::new(&format!("{id}.desktop")) else { return };
    let tipos: Vec<String> = d.supported_types().iter().map(|s| s.to_string()).collect();
    let nosso = |a: &gio::AppInfo| a.id().is_some_and(|i| i.starts_with("os.kalango."));
    let mut n = (0, 0);
    for t in &tipos {
        let todos = gio::AppInfo::all_for_type(t);
        let padrao = gio::AppInfo::default_for_type(t, false);
        if let Some(k) = todos.iter().find(|a| nosso(a)) {
            if !padrao.as_ref().is_some_and(nosso) {
                let _ = k.set_as_default_for_type(t);
                n.0 += 1;
            }
        } else if padrao.is_none() {
            let _ = d.set_as_default_for_type(t);
            n.1 += 1;
        }
    }
    if debug() {
        eprintln!("debug criativos: {id}: {} tipos; {} ficaram com o app do KalangoOS, {} com o app novo", tipos.len(), n.0, n.1);
    }
}

/// Uma pasta como a pessoa lê: "xdg-videos" → "~/Vídeos".
fn pasta_legivel(p: &str) -> String {
    let n = p.split(':').next().unwrap_or(p);
    let dir = match n {
        "xdg-videos" => glib::user_special_dir(glib::UserDirectory::Videos),
        "xdg-pictures" => glib::user_special_dir(glib::UserDirectory::Pictures),
        "xdg-documents" => glib::user_special_dir(glib::UserDirectory::Documents),
        "xdg-music" => glib::user_special_dir(glib::UserDirectory::Music),
        "xdg-download" => glib::user_special_dir(glib::UserDirectory::Downloads),
        _ => None,
    };
    // (sem user-dirs, os nomes padrão do XDG)
    let padrao = match n {
        "xdg-videos" => Some("Videos"),
        "xdg-pictures" => Some("Pictures"),
        "xdg-documents" => Some("Documents"),
        "xdg-music" => Some("Music"),
        "xdg-download" => Some("Downloads"),
        _ => None,
    };
    match dir.or_else(|| padrao.map(|p| glib::home_dir().join(p))) {
        Some(d) => d.strip_prefix(glib::home_dir()).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| d.display().to_string()),
        None => n.to_string(),
    }
}

fn texto_da_fase(f: &Fase, feito: u64, total: u64) -> String {
    match f {
        Fase::Verificando => tr("Checking the project's signature…").to_string(),
        Fase::Baixando if total > 0 => trf("Installing · {p} % · {done} of {size}", &[("p", format!("{:.0}", feito as f64 * 100.0 / total as f64)), ("done", tamanho(feito)), ("size", tamanho(total))]),
        Fase::Baixando => tr("Downloading…").to_string(),
        Fase::Instalando => tr("Installing…").to_string(),
        _ => String::new(),
    }
}

fn texto_do_erro(app: &App, e: &Erro) -> String {
    match e {
        Erro::Verificacao(Falha::Assinatura(_) | Falha::Formato(_)) => tr("Signature does not match: installation refused. Nothing was changed on the system.").to_string(),
        Erro::Verificacao(Falha::Bloqueada { versao, motivo }) => trf("Version {v} is blocked by KalangoOS: {why}", &[("v", versao.clone()), ("why", motivo.clone())]),
        Erro::Verificacao(Falha::Rede(_)) => trf("{app}: no connection to the project's server. Try again.", &[("app", app.nome.clone())]),
        Erro::Verificacao(Falha::Cancelada) => tr("Paused.").to_string(),
        Erro::SemEspaco { faltam } => trf("No space: {size} missing on the system disk.", &[("size", tamanho(*faltam))]),
        Erro::EmUso => tr("The app is open: it updates when you close it.").to_string(),
        Erro::Flatpak(x) => trf("Flatpak could not install: {e}", &[("e", x.clone())]),
    }
}

/// O trabalho de instalar ou atualizar, fora da interface: o kalangod verifica, baixa e importa no repositório dele
/// (onde só ele escreve); a Central instala exatamente o commit que ele devolveu e avisa que instalou.
fn trabalhar(app: &App, _anuncio: Option<Anuncio>, t: &Trabalho, prontas: &Prontas) -> Result<Fase, Erro> {
    // (o andamento do download, lido do kalangod enquanto ele trabalha)
    let fim = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (fim, a, id) = (fim.clone(), t.andamento.clone(), app.id.clone());
        std::thread::spawn(move || {
            while !fim.load(Ordering::Relaxed) {
                // ("Pausar": o kalangod para o download dele)
                if a.cancelar.load(Ordering::Relaxed) {
                    let _ = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "cancelar", "app": id}));
                }
                if let Ok(v) = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "andamento", "app": id})) {
                    if let (Some(f), Some(tt)) = (v["feito"].as_u64(), v["total"].as_u64()) {
                        a.feito.store(f, Ordering::Relaxed);
                        a.total.store(tt, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }
    *t.fase.lock().unwrap() = Fase::Baixando;
    let r = crate::servicos::kalangod_rpc_wait("kalango/craft", serde_json::json!({"acao": "preparar", "app": app.id}), Duration::from_secs(3600));
    fim.store(true, Ordering::Relaxed);
    let v = r.map_err(|e| Erro::Flatpak(trf("KalangoOS's service did not verify the app: {e}", &[("e", e)])))?;
    if let Some(e) = v.get("erro") {
        return Err(erro_do_kalangod(e));
    }
    let pr = Preparado {
        commit: v["commit"].as_str().unwrap_or("").into(),
        versao: v["versao"].as_str().unwrap_or("").into(),
        digest: v["digest"].as_str().unwrap_or("").into(),
        repo: PathBuf::from(v["repo"].as_str().unwrap_or("")),
    };
    // uma atualização de um app aberto espera ele fechar (já verificada e importada)
    if t.atualizacao && instalar::em_execucao().contains(&app.id) {
        let versao = pr.versao.clone();
        prontas.lock().unwrap().insert(app.id.clone(), pr);
        return Ok(Fase::Pronta { versao });
    }
    *t.fase.lock().unwrap() = Fase::Instalando;
    instalar_preparado(app, &pr)?;
    Ok(Fase::Feito)
}

/// Instala o preparado, avisa o kalangod (o preparado vira o atual e o de antes, ponto de restauração) e refaz as
/// permissões.
fn instalar_preparado(app: &App, pr: &Preparado) -> Result<(), Erro> {
    instalar::instalar_commit(app, &pr.repo, &pr.commit, &pr.versao, &pr.digest)?;
    crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "instalado", "app": app.id, "commit": pr.commit})).map_err(Erro::Flatpak)?;
    permissoes(&app.id).map(|d| tell_dropped_later(&app.id, d)).map_err(Erro::Flatpak)
}

fn voltar_ao_anterior(app: &App) -> Result<String, String> {
    if instalar::em_execucao().contains(&app.id) {
        return Err(tr("The app is open: it updates when you close it.").into());
    }
    let v = crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "anterior", "app": app.id}))?;
    let (commit, versao, repo) = (v["commit"].as_str().unwrap_or(""), v["versao"].as_str().unwrap_or(""), PathBuf::from(v["repo"].as_str().unwrap_or("")));
    instalar::instalar_commit(app, &repo, commit, versao, "").map_err(|e| e.to_string())?;
    crate::servicos::kalangod_rpc("kalango/craft", serde_json::json!({"acao": "voltou", "app": app.id, "commit": commit}))?;
    if let Ok(d) = permissoes(&app.id) {
        tell_dropped_later(&app.id, d);
    }
    Ok(versao.to_string())
}

/// O erro que o kalangod devolveu, no estado da tela.
fn erro_do_kalangod(e: &serde_json::Value) -> Erro {
    let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
    match e["tipo"].as_str().unwrap_or("") {
        "bloqueada" => Erro::Verificacao(Falha::Bloqueada { versao: s("versao"), motivo: s("motivo") }),
        "assinatura" => Erro::Verificacao(Falha::Assinatura(s("texto"))),
        "rede" => Erro::Verificacao(Falha::Rede(s("texto"))),
        "cancelada" => Erro::Verificacao(Falha::Cancelada),
        "espaco" => Erro::SemEspaco { faltam: e["faltam"].as_u64().unwrap_or(0) },
        "em_uso" => Erro::EmUso,
        _ => Erro::Flatpak(s("texto")),
    }
}

/// A atualização automática em segundo plano: a primeira procura 3 min depois de a sessão abrir, depois a cada 6 h; a
/// cada minuto, as prontas cujos apps fecharam entram.
pub fn agendar() {
    let minutos = Cell::new(0u32);
    glib::timeout_add_seconds_local(60, move || {
        minutos.set(minutos.get() + 1);
        let s = compartilhado();
        if !s.prontas.lock().map(|p| p.is_empty()).unwrap_or(true) {
            s.aplicar_prontas(false);
        }
        let e = instalar::ler_estado();
        let a = ajustes::ler();
        let devido = instalar::agora().saturating_sub(e.verificado_em) >= 6 * 3600 && minutos.get() >= 3;
        if a.atualizar && devido && !e.apps.is_empty() && s.trabalhos.borrow().is_empty() {
            if debug() {
                eprintln!("debug criativos: procura automática");
            }
            let w = Rc::downgrade(&s);
            glib::spawn_future_local(async move {
                let r = gio::spawn_blocking(|| verificar::indice(catalogo())).await.unwrap_or_else(|_| Err(Falha::Rede("?".into())));
                let Some(s) = w.upgrade() else { return };
                let mut e = instalar::ler_estado();
                e.verificado_em = instalar::agora();
                let _ = instalar::gravar_estado(&e);
                if let Ok(v) = r {
                    *s.anuncios.borrow_mut() = v.into_iter().collect();
                    s.atualizar_o_que_mudou(true);
                }
            });
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_catalogs_own_ids_open_through_kalangod() {
        assert!(is_creative("ai.storyteller.filmcraft") && is_creative("ai.storyteller.anicraft"));
        for other in ["filmcraft", "FilmCraft", "ai.storyteller.filmcraft.desktop", "os.kalango.Arquivos", ""] {
            assert!(!is_creative(other), "{other}");
        }
    }

    #[test]
    fn sizes_read_like_the_design() {
        assert_eq!(tamanho(118_000_000), "118 MB");
        assert_eq!(tamanho(1_400_000_000), "1,4 GB");
    }

    #[test]
    fn without_kalangod_no_folder_or_network_is_granted() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("log");
        let fake = d.path().join("flatpak");
        std::fs::write(&fake, format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::env::set_var("KALANGO_FLATPAK", &fake);
        let r = sem_kalangod("ai.storyteller.filmcraft", Err("kalangod: connection refused".into()));
        std::env::remove_var("KALANGO_FLATPAK");
        assert!(r.is_err());
        let l = std::fs::read_to_string(&log).unwrap();
        assert!(l.contains("--unshare=network") && l.contains("--nofilesystem=home"), "{l}");
        assert!(!l.contains("--filesystem="), "no folder granted: {l}");
    }

    #[test]
    fn a_clean_system_shows_the_initials_until_the_app_brings_its_icon() {
        // (empty XDG data folders, as on a fresh KalangoOS: no desktop entry, so the neutral marker with the initials)
        let d = tempfile::tempdir().unwrap();
        let (pessoa, sistema) = (d.path().join("data"), d.path().join("share"));
        for a in &catalogo().apps {
            assert!(desktop_do_app(&a.id, &[pessoa.clone(), sistema.clone()]).is_none(), "{}", a.id);
            assert!(!a.iniciais().is_empty());
        }
        // installed: Flatpak exports its desktop entry, and the app's own icon is used
        std::fs::create_dir_all(pessoa.join("applications")).unwrap();
        std::fs::write(pessoa.join("applications/ai.storyteller.anicraft.desktop"), "[Desktop Entry]\nType=Application\nName=AniCraft\nExec=sh\nIcon=ai.storyteller.anicraft\n").unwrap();
        let info = desktop_do_app("ai.storyteller.anicraft", &[pessoa, sistema]).expect("exported entry");
        assert!(info.icon().is_some());
    }

    #[test]
    fn every_catalog_category_has_a_chip() {
        for a in &catalogo().apps {
            assert!(CATEGORIAS.iter().any(|c| c.0 == a.categoria), "{}", a.categoria);
        }
    }
}
