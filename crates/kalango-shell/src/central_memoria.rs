//! Agents center › Memória (design "Memória (o que o sistema sabe de você)"): what the agent remembers, as a list
//! the person reads, corrects and deletes; each memory says where it came from (you said it, observed, you marked
//! it, from a folder). On the right: where it is kept, its size, the switches (memory on, learn from what I do,
//! cloud, apart by space, anonymous mode), export and forget everything.
//!
//! The memory belongs to the agent runtime (`kalango-agente memoria …`), which keeps it (encrypted when the system
//! keyring is there) and uses it. "Observed" memories come from here: `observar_habitos` turns the apps the person
//! opens (the launch log of "Sugeridos agora") into habits ("usually opens X between 20h and 2h").

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

const CATEGORIAS: &[(&str, &str)] = &[
    ("preferencias", n_("PREFERENCES")),
    ("rotinas", n_("ROUTINES")),
    ("projetos", n_("PROJECTS")),
    ("pessoas", n_("PEOPLE")),
];

/// Requests typed in the search field that go to the agent (the rest only filters the list).
const VERBOS: &[&str] = &["esqueç", "esquece", "lembr", "anote", "guarde", "forget", "remember"];

pub fn e_pedido(texto: &str) -> bool {
    let t = texto.trim().to_lowercase();
    VERBOS.iter().any(|v| t.starts_with(v))
}

fn quando(t: i64) -> String {
    let Ok(agora) = glib::DateTime::now_local() else { return String::new() };
    let Ok(d) = glib::DateTime::from_unix_local(t) else { return String::new() };
    let dia = |x: &glib::DateTime| glib::DateTime::from_local(x.year(), x.month(), x.day_of_month(), 0, 0, 0.0).map(|d| d.to_unix()).unwrap_or(0);
    let dias = (dia(&agora) - dia(&d)) / 86_400;
    match dias {
        ..=0 => tr("today").to_string(),
        1 => tr("yesterday").to_string(),
        2..=6 => trf("{n} days ago", &[("n", dias.to_string())]),
        _ => d.format("%-d %b").map(|s| s.to_lowercase()).unwrap_or_default(),
    }
}

fn tamanho(n: u64) -> String {
    if n < 1024 { format!("{n} B") } else if n < 1024 * 1024 { format!("{} KB", n.div_ceil(1024)) } else { format!("{:.1} MB", n as f64 / 1048576.0) }
}

pub struct Memoria {
    pub raiz: gtk::Box,
    lista: gtk::Box,
    resumo: gtk::Label,
    filtros: Vec<gtk::ToggleButton>,
    filtro: Cell<usize>,
    busca: RefCell<String>,
    dados: RefCell<Value>,
    onde: gtk::Label,
    tamanho: gtk::Label,
    chaves: Vec<(&'static str, gtk::Switch)>,
    anonimo: gtk::Switch,
    esquecer_rotulo: gtk::Label,
    confirmando: Cell<bool>,
    editando: Cell<Option<u64>>,
    preenchendo: Cell<bool>,
    /// the memory file as last read (modified time, size): a change elsewhere (the agent) reloads the list
    visto: Cell<(i64, u64)>,
}

/// The memory file's (modified time, size), whichever of the two forms exists.
fn marca_do_arquivo() -> (i64, u64) {
    let pasta = glib::user_data_dir().join("kalango");
    ["memoria.bin", "memoria.json"].iter().filter_map(|n| std::fs::metadata(pasta.join(n)).ok())
        .map(|m| (m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0), m.len()))
        .next().unwrap_or((0, 0))
}

impl Memoria {
    pub fn new(_shell: &Rc<Shell>, largura_janela: i32) -> Rc<Memoria> {
        let compacta = largura_janela < 1480;
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        // ---- the list
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let topo = caixa(Orientation::Horizontal, 8, "");
        let t = caixa(Orientation::Vertical, 4, "");
        t.set_hexpand(true);
        let h = rotulo(tr("Memory"), "central-titulo");
        h.set_xalign(0.0);
        t.append(&h);
        let resumo = rotulo("", "central-sub");
        resumo.set_xalign(0.0);
        t.append(&resumo);
        topo.append(&t);
        let mut filtros = Vec::new();
        for nome in [tr("All"), tr("You said"), tr("Observed")] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("memoria-filtro");
            b.set_valign(Align::Start);
            b.set_margin_top(6);
            if let Some(p) = filtros.first() {
                b.set_group(Some(p));
            }
            topo.append(&b);
            filtros.push(b);
        }
        corpo.append(&topo);
        let lista = caixa(Orientation::Vertical, 0, "");
        lista.set_margin_top(8);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&lista));
        corpo.append(&rolagem);
        raiz.append(&corpo);

        // ---- how it works, on the right
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("cerebro", "#A5A1E2", 14));
        cab.append(&rotulo(tr("HOW MEMORY WORKS"), "central-painel-agente"));
        painel.append(&cab);
        let info = caixa(Orientation::Vertical, 6, "central-cartao");
        info.add_css_class("memoria-info");
        info.set_margin_top(14);
        let linha_info = |nome: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            let n = rotulo(nome, "central-agente-sub");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            l.append(&n);
            let v = rotulo("", "memoria-info-valor");
            v.set_xalign(1.0);
            v.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            v.set_max_width_chars(24);
            l.append(&v);
            info.append(&l);
            v
        };
        let onde = linha_info(tr("Where it is kept"));
        let tamanho_v = linha_info(tr("Size"));
        let quem = linha_info(tr("Who uses it"));
        quem.set_text(tr("agents of this account"));
        painel.append(&info);
        let chaves_cartao = caixa(Orientation::Vertical, 0, "central-cartao");
        chaves_cartao.set_margin_top(12);
        let mut chaves = Vec::new();
        for (id, nome, explica) in [
            ("ligada", tr("Memory on"), ""),
            ("aprender", tr("Learn from what I do"), tr("besides what I say")),
            ("nuvem", tr("Use the memory with cloud models"), tr("only the part needed, with a notice")),
            ("por_espaco", tr("Memory kept apart by space"), tr("work does not mix with personal")),
        ] {
            let (l, sw) = Self::linha_chave(nome, explica);
            if id == "nuvem" {
                // no cloud provider in this build: nothing would go to one
                sw.set_sensitive(false);
                l.set_tooltip_text(Some(tr("No cloud provider is set up: the memory only goes to the local model.")));
            }
            chaves_cartao.append(&l);
            chaves.push((id, sw));
        }
        painel.append(&chaves_cartao);
        let anon = caixa(Orientation::Horizontal, 12, "central-cartao");
        anon.set_margin_top(12);
        let olho = ui_icon("olho", "#8E8F99", 16);
        olho.set_valign(Align::Center);
        anon.append(&olho);
        let (l, anonimo) = Self::linha_chave(tr("Anonymous mode"), tr("nothing from this session is remembered"));
        l.set_hexpand(true);
        anon.append(&l);
        painel.append(&anon);
        let espaco = caixa(Orientation::Vertical, 0, "");
        espaco.set_vexpand(true);
        painel.append(&espaco);
        let nota = rotulo(tr("You can ask “why did you do that?”: the agent says which memories it used."), "central-agente-sub");
        nota.set_xalign(0.0);
        nota.set_wrap(true);
        nota.set_margin_bottom(10);
        painel.append(&nota);
        let botoes = caixa(Orientation::Horizontal, 8, "");
        botoes.set_homogeneous(true);
        let exportar = gtk::Button::new();
        exportar.add_css_class("central-botao");
        let ec = caixa(Orientation::Horizontal, 8, "");
        ec.set_halign(Align::Center);
        ec.append(&ui_icon("baixar", "#E2E2E6", 14));
        ec.append(&rotulo(tr("Export"), ""));
        exportar.set_child(Some(&ec));
        botoes.append(&exportar);
        let esquecer = gtk::Button::new();
        esquecer.add_css_class("central-botao");
        esquecer.add_css_class("perigo");
        let fc = caixa(Orientation::Horizontal, 8, "");
        fc.set_halign(Align::Center);
        fc.append(&ui_icon("lixeira", "#EFA2A6", 14));
        let esquecer_rotulo = rotulo(tr("Forget everything"), "");
        fc.append(&esquecer_rotulo);
        esquecer.set_child(Some(&fc));
        botoes.append(&esquecer);
        painel.append(&botoes);
        raiz.append(&painel);

        let m = Rc::new(Memoria {
            raiz, lista, resumo, filtros, filtro: Cell::new(0), busca: RefCell::default(), dados: RefCell::new(Value::Null), onde, tamanho: tamanho_v,
            chaves, anonimo, esquecer_rotulo, confirmando: Cell::new(false), editando: Cell::new(None), preenchendo: Cell::new(false),
            visto: Cell::new((0, 0)),
        });
        m.filtros[0].set_active(true);
        for (i, b) in m.filtros.iter().enumerate() {
            let w = Rc::downgrade(&m);
            b.connect_toggled(move |b| {
                if let Some(m) = w.upgrade().filter(|_| b.is_active()) {
                    m.filtro.set(i);
                    m.render();
                }
            });
        }
        for (id, sw) in &m.chaves {
            let (w, id) = (Rc::downgrade(&m), *id);
            sw.connect_state_set(move |_, on| {
                if let Some(m) = w.upgrade().filter(|m| !m.preenchendo.get()) {
                    m.executar(vec!["ajuste".into(), id.into(), if on { "sim" } else { "nao" }.into()]);
                }
                glib::Propagation::Proceed
            });
        }
        let w = Rc::downgrade(&m);
        m.anonimo.connect_state_set(move |_, on| {
            if let Some(m) = w.upgrade().filter(|m| !m.preenchendo.get()) {
                m.executar(vec!["anonimo".into(), if on { "sim" } else { "nao" }.into()]);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&m);
        exportar.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            let w = Rc::downgrade(&m);
            crate::arquivos::seletor::salvar_arquivo(tr("Agents"), tr("Export the memory"), "memoria-kalango.json", None, move |p| {
                if let Some(m) = w.upgrade() {
                    m.executar(vec!["exportar".into(), p.display().to_string()]);
                }
            });
        });
        // forgetting everything takes a second click (within 4 s)
        let w = Rc::downgrade(&m);
        esquecer.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            if m.confirmando.replace(false) {
                m.esquecer_rotulo.set_text(tr("Forget everything"));
                m.executar(vec!["esquecer-tudo".into()]);
                return;
            }
            m.confirmando.set(true);
            m.esquecer_rotulo.set_text(tr("Click again to forget"));
            let w = Rc::downgrade(&m);
            glib::timeout_add_local_once(Duration::from_secs(4), move || {
                if let Some(m) = w.upgrade().filter(|m| m.confirmando.replace(false)) {
                    m.esquecer_rotulo.set_text(tr("Forget everything"));
                }
            });
        });
        m
    }

    fn linha_chave(nome: &str, explica: &str) -> (gtk::Box, gtk::Switch) {
        let l = caixa(Orientation::Horizontal, 12, "memoria-chave");
        let col = caixa(Orientation::Vertical, 1, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let n = rotulo(nome, "central-limite-nome");
        n.set_xalign(0.0);
        col.append(&n);
        if !explica.is_empty() {
            let e = rotulo(explica, "central-agente-sub");
            e.set_xalign(0.0);
            col.append(&e);
        }
        l.append(&col);
        let sw = gtk::Switch::new();
        sw.add_css_class("central-switch");
        sw.set_valign(Align::Center);
        l.append(&sw);
        (l, sw)
    }

    /// The search field of the toolbar: the list shows only what contains the text.
    pub fn filtrar(self: &Rc<Self>, texto: &str) {
        *self.busca.borrow_mut() = texto.trim().to_lowercase();
        self.render();
    }

    /// Reloads when the memory file changed since it was read (the agent remembered or forgot something).
    pub fn verificar(self: &Rc<Self>) {
        if marca_do_arquivo() != self.visto.get() {
            self.recarregar();
        }
    }

    pub fn recarregar(self: &Rc<Self>) {
        self.visto.set(marca_do_arquivo());
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || {
            Command::new(&bin).args(["memoria", "listar"]).output().ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        }, move |v| {
            let Some(m) = w.upgrade() else { return };
            match v {
                Some(v) => {
                    *m.dados.borrow_mut() = v;
                    m.render();
                }
                None => eprintln!("kalango-shell: kalango-agente memoria listar não respondeu"),
            }
        });
    }

    fn executar(self: &Rc<Self>, args: Vec<String>) {
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || {
            let mut a = vec!["memoria".to_string()];
            a.extend(args);
            Command::new(&bin).args(&a).output().map_err(|e| e.to_string())
                .and_then(|o| if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) })
        }, move |r| {
            if let Err(e) = r {
                eprintln!("kalango-shell: memória do agente: {e}");
            }
            if let Some(m) = w.upgrade() {
                m.recarregar();
            }
        });
    }

    fn render(self: &Rc<Self>) {
        let v = self.dados.borrow().clone();
        self.preenchendo.set(true);
        let todas = v["lembrancas"].as_array().cloned().unwrap_or_default();
        let n = todas.len();
        let quantas = if n == 1 { tr("1 memory").to_string() } else { trf("{n} memories", &[("n", n.to_string())]) };
        self.resumo.set_text(&format!("{quantas} · {}", tr("kept only on this computer")));
        while let Some(x) = self.lista.first_child() {
            self.lista.remove(&x);
        }
        if let Some(e) = v["erro"].as_str() {
            let l = rotulo(e, "central-sub");
            l.set_xalign(0.0);
            l.set_wrap(true);
            self.lista.append(&l);
        }
        let busca = self.busca.borrow().clone();
        let filtro = self.filtro.get();
        let mut mostradas = 0;
        for (cat, nome) in CATEGORIAS {
            let mut itens: Vec<&Value> = todas.iter().filter(|l| l["categoria"] == *cat)
                .filter(|l| match filtro { 1 => l["origem"] == "voce", 2 => l["origem"] == "observado", _ => true })
                .filter(|l| busca.is_empty() || l["texto"].as_str().unwrap_or("").to_lowercase().contains(&busca)).collect();
            if itens.is_empty() {
                continue;
            }
            itens.sort_by_key(|l| -l["atualizado"].as_i64().unwrap_or(0));
            let s = rotulo(tr(nome), "central-painel-secao");
            s.set_xalign(0.0);
            s.set_margin_top(if mostradas == 0 { 12 } else { 18 });
            s.set_margin_bottom(10);
            self.lista.append(&s);
            for l in itens {
                self.lista.append(&self.item(l));
                mostradas += 1;
            }
        }
        if mostradas == 0 && v["erro"].is_null() {
            let vazio = rotulo(if n == 0 {
                tr("Nothing remembered yet. Tell the agent something about you (“remember that…”) and it shows up here.")
            } else {
                tr("No memory matches.")
            }, "central-sub");
            vazio.set_xalign(0.0);
            vazio.set_wrap(true);
            vazio.set_margin_top(16);
            self.lista.append(&vazio);
        }
        // how it works
        let cifrada = v["cifrada"].as_bool().unwrap_or(false);
        self.onde.set_text(if cifrada { tr("on this computer, encrypted") } else { tr("on this computer") });
        self.onde.set_tooltip_text((!cifrada).then_some(tr("A file only you can read. It is encrypted when the system keyring is available.")));
        self.tamanho.set_text(&tamanho(v["tamanho"].as_u64().unwrap_or(0)));
        for (id, sw) in &self.chaves {
            let padrao = matches!(*id, "ligada" | "aprender");
            sw.set_active(v["ajustes"][*id].as_bool().unwrap_or(padrao));
        }
        self.anonimo.set_active(v["anonimo"].as_bool().unwrap_or(false));
        self.preenchendo.set(false);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: memória ({n} lembranças, {mostradas} mostradas, cifrada {cifrada}, anônimo {})", v["anonimo"].as_bool().unwrap_or(false));
        }
    }

    /// One memory: its colour bar, the text (or the field while it is edited), where it came from and when, edit and
    /// delete. Deleting slides it out first.
    fn item(self: &Rc<Self>, l: &Value) -> gtk::Revealer {
        let id = l["id"].as_u64().unwrap_or(0);
        let texto = l["texto"].as_str().unwrap_or("").to_string();
        let origem = l["origem"].as_str().unwrap_or("voce");
        let r = gtk::Revealer::new();
        r.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        r.set_transition_duration(220);
        r.set_reveal_child(true);
        let card = caixa(Orientation::Horizontal, 12, "memoria-item");
        card.set_margin_bottom(6);
        let barra = caixa(Orientation::Vertical, 0, "memoria-barra");
        barra.add_css_class(l["categoria"].as_str().unwrap_or("preferencias"));
        barra.set_size_request(3, 28);
        barra.set_valign(Align::Center);
        card.append(&barra);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        if self.editando.get() == Some(id) {
            let e = gtk::Entry::new();
            e.set_text(&texto);
            e.add_css_class("memoria-edicao");
            let w = Rc::downgrade(self);
            e.connect_activate(move |e| {
                if let Some(m) = w.upgrade() {
                    m.editando.set(None);
                    let t = e.text().trim().to_string();
                    if t.is_empty() {
                        m.render();
                    } else {
                        m.executar(vec!["editar".into(), id.to_string(), t]);
                    }
                }
            });
            let k = gtk::EventControllerKey::new();
            let w = Rc::downgrade(self);
            k.connect_key_pressed(move |_, tecla, _, _| {
                if tecla == gtk::gdk::Key::Escape {
                    if let Some(m) = w.upgrade() {
                        m.editando.set(None);
                        m.render();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            e.add_controller(k);
            col.append(&e);
            glib::idle_add_local_once(move || {
                e.grab_focus();
            });
        } else {
            let t = rotulo(&texto, "memoria-texto");
            t.set_xalign(0.0);
            t.set_wrap(true);
            col.append(&t);
        }
        let meta = caixa(Orientation::Horizontal, 8, "");
        let chip = rotulo(&match origem {
            "observado" => tr("observed").to_string(),
            "marcado" => tr("you marked").to_string(),
            "arquivo" => trf("from {folder}", &[("folder", l["fonte"].as_str().unwrap_or("").to_string())]),
            _ => tr("you said").to_string(),
        }, "memoria-origem");
        chip.add_css_class(origem);
        meta.append(&chip);
        let data = if origem == "observado" {
            trf("updated {when}", &[("when", quando(l["atualizado"].as_i64().unwrap_or(0)))])
        } else {
            quando(l["criado"].as_i64().unwrap_or(0))
        };
        meta.append(&rotulo(&data, "central-agente-sub"));
        col.append(&meta);
        card.append(&col);
        let editar = gtk::Button::new();
        editar.add_css_class("memoria-acao");
        editar.set_child(Some(&ui_icon("doc", "#8E8F99", 14)));
        editar.set_tooltip_text(Some(tr("Correct")));
        editar.set_valign(Align::Center);
        let w = Rc::downgrade(self);
        editar.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.editando.set(Some(id));
                m.render();
            }
        });
        card.append(&editar);
        let apagar = gtk::Button::new();
        apagar.add_css_class("memoria-acao");
        apagar.add_css_class("perigo");
        apagar.set_child(Some(&ui_icon("lixeira", "#EFA2A6", 14)));
        apagar.set_tooltip_text(Some(tr("Delete")));
        apagar.set_valign(Align::Center);
        crate::central::posicao_debug(&format!("apagar lembrança {id}"), apagar.upcast_ref());
        let (w, rw) = (Rc::downgrade(self), r.downgrade());
        apagar.connect_clicked(move |_| {
            if let Some(r) = rw.upgrade() {
                r.set_reveal_child(false);
            }
            let w = w.clone();
            glib::timeout_add_local_once(Duration::from_millis(240), move || {
                if let Some(m) = w.upgrade() {
                    m.executar(vec!["apagar".into(), id.to_string()]);
                }
            });
        });
        card.append(&apagar);
        r.set_child(Some(&card));
        r
    }
}

/// The hours window (start hour, length) holding at least 70% of the launches, the shortest one (2 to 8 hours).
pub(crate) fn janela_de_horas(horas: &[u32]) -> Option<(u32, u32)> {
    let mut hist = [0usize; 24];
    for h in horas {
        hist[(*h % 24) as usize] += 1;
    }
    let total = horas.len();
    (2..=8u32).find_map(|len| {
        (0..24u32).map(|ini| (ini, (0..len).map(|k| hist[((ini + k) % 24) as usize]).sum::<usize>()))
            .filter(|(_, n)| *n * 10 >= total * 7).max_by_key(|(_, n)| *n).map(|(ini, _)| (ini, len))
    })
}

/// "Learn from what I do": the apps opened at a regular time of day (at least 6 times in the last 30 days, 70% of
/// them within a few hours) become observed memories. The agent runtime keeps them only while learning is on.
pub fn observar_habitos(shell: &Rc<Shell>) {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let mut por_app: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    for (id, t) in crate::apps::uses() {
        if agora - t <= 30 * 86_400 {
            if let Ok(d) = glib::DateTime::from_unix_local(t) {
                por_app.entry(id).or_default().push(d.hour() as u32);
            }
        }
    }
    let fatos: Vec<Value> = por_app.iter().filter(|(_, h)| h.len() >= 6).filter_map(|(id, horas)| {
        let (ini, len) = janela_de_horas(horas)?;
        let nome = shell.todos.iter().find(|a| a.id == *id).map(|a| a.nome.clone())?;
        Some(serde_json::json!({
            "chave": format!("app-horario:{id}"), "categoria": "rotinas",
            "texto": trf("Usually opens {app} between {from}h and {to}h", &[("app", nome), ("from", ini.to_string()), ("to", ((ini + len) % 24).to_string())]),
        }))
    }).collect();
    let bin = crate::voz::agent_binary();
    let entrada = Value::Array(fatos).to_string();
    crate::lateral::em_segundo_plano(move || {
        let mut c = Command::new(&bin).args(["memoria", "observar"]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().ok()?;
        use std::io::Write;
        c.stdin.take()?.write_all(entrada.as_bytes()).ok()?;
        c.wait().ok()
    }, |_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn habits_need_a_regular_time() {
        // evenings, across midnight
        let (ini, len) = janela_de_horas(&[20, 21, 22, 23, 0, 1, 21, 22]).unwrap();
        assert!(len <= 4 && (20..24).contains(&ini), "{ini} {len}");
        assert_eq!(janela_de_horas(&[9, 9, 10, 9, 10, 9]), Some((9, 2)));
        // spread over the day: no habit
        assert_eq!(janela_de_horas(&[1, 4, 7, 10, 13, 16, 19, 22]), None);
    }

    #[test]
    fn requests_from_the_search_field() {
        assert!(e_pedido("Esqueça que eu moro em Recife"));
        assert!(e_pedido("lembre que prefiro tema escuro"));
        assert!(!e_pedido("tema escuro"));
    }
}
