//! Agents center › Linha do tempo (design "Linha do tempo (desfazer o que os agentes fizeram)"): every action an agent
//! took through kalangod is a recorded transaction — what changed, why, with which permissions — and has an undo when
//! it is reversible. Shown: every change, every refusal, every undo, and the reads made while working on a request of
//! the person or a routine (the reads of background summaries are left out). Requests waiting for the person come
//! first ("aguarda você"). On the right, the chosen action: before/after, why it was done (the request or routine
//! running then), the permissions it used, undo it, or undo everything since a moment (the rewind bar picks it; one
//! approval for all).
//!
//! Undoing goes through kalangod like any system change: it asks for approval on the system screen and makes a restore
//! point first.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

const FILTROS: [&str; 3] = [n_("Today"), n_("7 days"), n_("All")];

fn hora(t: i64) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn inicio_do_dia() -> i64 {
    glib::DateTime::now_local().ok().and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0)
}

fn agora() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// The permission column of a tool (the same columns as the Permissions page).
fn coluna(tool: &str, leitura: bool) -> (&'static str, &'static str) {
    if tool.starts_with("web.") {
        (n_("network"), "globo")
    } else if matches!(tool, "sistema.executar" | "tarefas.executar") {
        (n_("terminal"), "terminal")
    } else if matches!(tool, "arquivos.lixeira" | "pacotes.remover" | "processos.sinal" | "usuarios.remover") {
        (n_("delete"), "lixeira")
    } else if leitura {
        (n_("read"), "olho")
    } else {
        (n_("write"), "doc")
    }
}

/// A request or routine running when a record was made.
struct Janela {
    inicio: i64,
    fim: i64,
    pedido: String,
    rotina: bool,
}

pub struct Linha {
    pub raiz: gtk::Box,
    shell: Weak<Shell>,
    filtros: Vec<gtk::ToggleButton>,
    filtro: Cell<usize>,
    numeros: [gtk::Label; 3],
    lista: gtk::Box,
    escala: gtk::Scale,
    escala_hora: gtk::Label,
    escala_marcas: gtk::Box,
    // the chosen action
    titulo: gtk::Label,
    meta: gtk::Label,
    /// "Recomendação do d1" (design page 31): shown for an action the d1 approved by itself
    d1_caixa: gtk::Box,
    antes_depois: gtk::Box,
    porque: gtk::Label,
    permissoes: gtk::Box,
    explica: gtk::Label,
    desfazer: gtk::Button,
    desfazer_tudo: gtk::Button,
    registros: RefCell<Vec<Value>>,
    pendentes: RefCell<Vec<Value>>,
    janelas: RefCell<Vec<Janela>>,
    escolhido: Cell<u64>,
    busca: RefCell<String>,
    ocupado: Cell<bool>,
    preenchendo: Cell<bool>,
    /// screen actions' thumbnails already read from kalangod, by record
    miniaturas: RefCell<std::collections::HashMap<u64, gtk::gdk::Texture>>,
}

impl Linha {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<Linha> {
        let compacta = largura_janela < 1480;
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let topo = caixa(Orientation::Horizontal, 6, "");
        let h = rotulo(tr("Timeline"), "central-titulo");
        h.set_xalign(0.0);
        h.set_hexpand(true);
        topo.append(&h);
        let mut filtros: Vec<gtk::ToggleButton> = Vec::new();
        for f in FILTROS {
            let b = gtk::ToggleButton::with_label(tr(f));
            b.add_css_class("memoria-filtro");
            b.set_valign(Align::Center);
            if let Some(p) = filtros.first() {
                b.set_group(Some(p));
            }
            topo.append(&b);
            filtros.push(b);
        }
        corpo.append(&topo);
        let stats = caixa(Orientation::Horizontal, 12, "");
        stats.set_homogeneous(true);
        stats.set_margin_top(14);
        let numero = |rotulo_t: &str, classe: &str| {
            let c = caixa(Orientation::Vertical, 4, "linha-stat");
            let n = rotulo("0", "linha-stat-numero");
            if !classe.is_empty() {
                n.add_css_class(classe);
            }
            n.set_xalign(0.0);
            c.append(&n);
            let l = rotulo(rotulo_t, "central-agente-sub");
            l.set_xalign(0.0);
            c.append(&l);
            stats.append(&c);
            n
        };
        let numeros = [numero(tr("agent actions"), ""), numero(tr("undone by you"), "gold"), numero(tr("outside the permissions"), "teal")];
        corpo.append(&stats);
        let lista = caixa(Orientation::Vertical, 0, "");
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&lista));
        corpo.append(&rolagem);
        // rewind to a moment
        let reb = caixa(Orientation::Vertical, 6, "linha-rebobinar");
        reb.set_margin_top(12);
        let rc = caixa(Orientation::Horizontal, 8, "");
        rc.append(&ui_icon("relogio", "#C9CAD1", 14));
        let rt = rotulo(tr("Rewind the system to a moment"), "central-limite-nome");
        rt.set_hexpand(true);
        rt.set_xalign(0.0);
        rc.append(&rt);
        let escala_hora = rotulo("", "linha-rebobinar-hora");
        rc.append(&escala_hora);
        reb.append(&rc);
        let escala = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 60.0);
        escala.set_draw_value(false);
        escala.add_css_class("central-escala");
        reb.append(&escala);
        let escala_marcas = caixa(Orientation::Horizontal, 0, "");
        reb.append(&escala_marcas);
        corpo.append(&reb);
        raiz.append(&corpo);

        // the chosen action
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 7, "");
        cab.append(&estrela(13, "#A5A1E2"));
        cab.append(&rotulo(tr("CHOSEN ACTION"), "central-painel-agente"));
        painel.append(&cab);
        let titulo = rotulo("", "linha-titulo");
        titulo.set_xalign(0.0);
        titulo.set_wrap(true);
        titulo.set_margin_top(12);
        painel.append(&titulo);
        let meta = rotulo("", "central-agente-sub");
        meta.set_xalign(0.0);
        meta.set_margin_top(6);
        painel.append(&meta);
        let d1_caixa = caixa(Orientation::Horizontal, 8, "linha-d1");
        d1_caixa.set_margin_top(14);
        d1_caixa.set_visible(false);
        painel.append(&d1_caixa);
        let antes_depois = caixa(Orientation::Horizontal, 12, "");
        antes_depois.set_homogeneous(true);
        antes_depois.set_margin_top(14);
        painel.append(&antes_depois);
        let secao = |t: &str| {
            let l = rotulo(t, "central-painel-secao");
            l.set_xalign(0.0);
            l.set_margin_top(18);
            l
        };
        painel.append(&secao(tr("WHY IT WAS DONE")));
        let porque = rotulo("", "linha-porque");
        porque.set_xalign(0.0);
        porque.set_wrap(true);
        porque.set_margin_top(8);
        painel.append(&porque);
        painel.append(&secao(tr("PERMISSIONS USED")));
        let permissoes = caixa(Orientation::Horizontal, 6, "");
        permissoes.set_margin_top(10);
        painel.append(&permissoes);
        let espaco = caixa(Orientation::Vertical, 0, "");
        espaco.set_vexpand(true);
        painel.append(&espaco);
        let explica = rotulo("", "central-agente-sub");
        explica.set_xalign(0.0);
        explica.set_wrap(true);
        explica.set_margin_bottom(10);
        painel.append(&explica);
        let desfazer = gtk::Button::new();
        desfazer.add_css_class("linha-desfazer");
        painel.append(&desfazer);
        let desfazer_tudo = gtk::Button::new();
        desfazer_tudo.add_css_class("central-botao");
        desfazer_tudo.set_margin_top(10);
        painel.append(&desfazer_tudo);
        raiz.append(&painel);

        let l = Rc::new(Linha {
            raiz, shell: Rc::downgrade(shell), filtros, filtro: Cell::new(0), numeros, lista, escala, escala_hora, escala_marcas, titulo, meta,
            d1_caixa, antes_depois, porque, permissoes, explica, desfazer, desfazer_tudo, registros: RefCell::default(), pendentes: RefCell::default(),
            janelas: RefCell::default(), escolhido: Cell::new(0), busca: RefCell::default(), ocupado: Cell::new(false), preenchendo: Cell::new(false), miniaturas: RefCell::default(),
        });
        l.filtros[0].set_active(true);
        for (i, b) in l.filtros.iter().enumerate() {
            let w = Rc::downgrade(&l);
            b.connect_toggled(move |b| {
                if let Some(l) = w.upgrade().filter(|_| b.is_active()) {
                    l.filtro.set(i);
                    l.recarregar();
                }
            });
        }
        let w = Rc::downgrade(&l);
        l.escala.connect_value_changed(move |s| {
            if let Some(l) = w.upgrade().filter(|l| !l.preenchendo.get()) {
                l.escala_hora.set_text(&hora(s.value() as i64));
                l.botoes();
            }
        });
        let w = Rc::downgrade(&l);
        l.desfazer.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                let id = l.escolhido.get();
                l.executar("linha_do_tempo.desfazer", json!({"id": id}));
            }
        });
        let w = Rc::downgrade(&l);
        l.desfazer_tudo.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                let desde = l.escala.value() as i64;
                l.executar("linha_do_tempo.desfazer_desde", json!({"desde": desde}));
            }
        });
        l
    }

    pub fn filtrar(self: &Rc<Self>, texto: &str) {
        *self.busca.borrow_mut() = texto.trim().to_lowercase();
        self.render();
    }

    pub fn recarregar(self: &Rc<Self>) {
        let desde = match self.filtro.get() { 0 => inicio_do_dia(), 1 => agora() - 7 * 86_400, _ => 0 };
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            (crate::servicos::kalangod_registros(desde, 3000), crate::servicos::kalangod_pendentes().unwrap_or_default())
        }, move |(r, p)| {
            let Some(l) = w.upgrade() else { return };
            match r {
                Ok(r) => *l.registros.borrow_mut() = r,
                Err(e) => eprintln!("kalango-shell: linha do tempo: {e}"),
            }
            *l.pendentes.borrow_mut() = p;
            *l.janelas.borrow_mut() = crate::conversa::tarefas(400).iter().map(|t| Janela {
                inicio: t["inicio"].as_i64().unwrap_or(0), fim: t["fim"].as_i64().unwrap_or(0),
                pedido: t["pedido"].as_str().unwrap_or("").to_string(), rotina: t["rotina"].is_string(),
            }).collect();
            l.render();
        });
    }

    /// Undo (one action, or everything since a moment): kalangod asks for approval on the system screen.
    fn executar(self: &Rc<Self>, ferramenta: &'static str, args: Value) {
        if self.ocupado.replace(true) {
            return;
        }
        self.botoes();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar(ferramenta, args), move |r| {
            let Some(l) = w.upgrade() else { return };
            l.ocupado.set(false);
            match &r {
                Ok(_) => {
                    crate::configuracoes_som::tocar("kalango-undo");
                }
                Err(e) => l.explica.set_text(&trf("Not undone: {why}", &[("why", e.clone())])),
            }
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug central: desfazer ({ferramenta}) {}", if r.is_ok() { "ok" } else { "recusado" });
            }
            l.recarregar();
        });
    }

    fn janela_de(&self, t: i64) -> Option<(String, bool)> {
        self.janelas.borrow().iter().find(|j| t >= j.inicio - 2 && t <= j.fim + 2).map(|j| (j.pedido.clone(), j.rotina))
    }

    /// Who did it, as the person reads it.
    fn quem(&self, e: &Value) -> (String, &'static str) {
        // (an action on the screen says the agent's own name, as the screen's banner did)
        if let Some(n) = e["args"]["agente"].as_str().filter(|n| !n.is_empty() && e["tool"].as_str().is_some_and(|t| t.starts_with("tela."))) {
            return (n.to_string(), "agente");
        }
        let exe = e["caller"].as_str().unwrap_or("").rsplit(['/', '(']).next().unwrap_or("").trim_end_matches(')').to_string();
        match exe.as_str() {
            "kalango-agente" => match self.janela_de(e["time"].as_i64().unwrap_or(0)) {
                Some((nome, true)) => (trf("Routine {name}", &[("name", nome)]), "rotina"),
                _ => (tr("Agent").to_string(), "agente"),
            },
            "kalango-shell" | "kalango-aprovacao" => (tr("You").to_string(), "voce"),
            "" => (tr("System").to_string(), "sistema"),
            x => (x.to_string(), "outro"),
        }
    }

    /// The records shown (see the module) and the ones undone by the person.
    fn visiveis(&self) -> (Vec<Value>, usize, usize) {
        let regs = self.registros.borrow();
        let busca = self.busca.borrow().clone();
        let mut desfeitas = 0;
        let mut negadas = 0;
        let v: Vec<Value> = regs.iter().filter(|e| {
            let status = e["status"].as_str().unwrap_or("");
            if status == "desfeito" {
                desfeitas += 1;
                return false;
            }
            if status == "negado" {
                negadas += 1;
            }
            let leitura = e["leitura"].as_bool().unwrap_or(e["level"] == "observar");
            // the shell's own reads are housekeeping (its widgets and bars), not actions
            if leitura && self.quem(e).1 == "voce" {
                return false;
            }
            status != "ok" || !leitura || self.janela_de(e["time"].as_i64().unwrap_or(0)).is_some()
        }).filter(|e| busca.is_empty() || format!("{} {} {} {}", e["summary"], e["tool"], e["descricao"], e["args"]).to_lowercase().contains(&busca)).cloned().collect();
        (v, desfeitas, negadas)
    }

    fn render(self: &Rc<Self>) {
        let (v, desfeitas, negadas) = self.visiveis();
        self.numeros[0].set_text(&v.len().to_string());
        self.numeros[1].set_text(&desfeitas.to_string());
        self.numeros[2].set_text(&negadas.to_string());
        while let Some(x) = self.lista.first_child() {
            self.lista.remove(&x);
        }
        let n = agora();
        // waiting for the person
        let pend = self.pendentes.borrow().clone();
        if !pend.is_empty() {
            self.lista.append(&self.secao(tr("WAITING FOR YOU")));
            for p in &pend {
                let titulo = p["descricao"].as_str().unwrap_or(p["tool"].as_str().unwrap_or("")).split(['.', '\n']).next().unwrap_or("").to_string();
                let card = self.cartao(p["criado"].as_i64().unwrap_or(n), tr("Agent"), "agente", &titulo,
                    tr("nothing was done — it waits for your decision"), &[(tr("waits for you"), "espera")], false);
                let b = gtk::Button::with_label(tr("Review"));
                b.add_css_class("linha-revisar");
                b.set_valign(Align::Center);
                let sh = self.shell.clone();
                b.connect_clicked(move |_| {
                    if let Some(s) = sh.upgrade() {
                        s.right_bar().abrir(crate::lateral::Modo::Agente);
                    }
                });
                card.1.append(&b);
                self.lista.append(&card.0);
            }
        }
        if v.is_empty() && pend.is_empty() {
            let vazio = rotulo(tr("No agent action in this period."), "central-sub");
            vazio.set_xalign(0.0);
            vazio.set_margin_top(16);
            self.lista.append(&vazio);
        }
        // an agent changed a file that runs at login, let through by the person's choice (design page 30): said
        // first, with undo and the way to the setting
        if let Some(e) = v.iter().rev().find(|e| e["startup"]["automatico"] == true && e["status"] == "ok" && e["desfeito"] != true) {
            self.lista.append(&self.startup_notice(e));
        }
        let mut secao_atual = "";
        for e in v.iter().rev() {
            let t = e["time"].as_i64().unwrap_or(0);
            let s = if n - t < 3600 { "recente" } else { "cedo" };
            if s != secao_atual {
                self.lista.append(&self.secao(if s == "recente" { tr("JUST NOW") } else { tr("EARLIER") }));
                secao_atual = s;
            }
            let id = e["id"].as_u64().unwrap_or(0);
            let status = e["status"].as_str().unwrap_or("");
            let leitura = e["leitura"].as_bool().unwrap_or(false);
            let desfeito = e["desfeito"].as_bool().unwrap_or(false);
            let reversivel = !e["inverse"].is_null() && status == "ok" && !desfeito;
            let (quem, classe) = self.quem(e);
            let titulo = titulo_de(e);
            let sub = match status {
                "negado" => trf("refused: {why}", &[("why", e["summary"].as_str().unwrap_or("").to_string())]),
                "erro" => trf("failed: {why}", &[("why", e["summary"].as_str().unwrap_or("").to_string())]),
                _ if e["tool"].as_str().is_some_and(|t| t.starts_with("tela.")) => screen_sub(e),
                _ if e["startup"].is_object() => startup_sub(e),
                _ => resumo_args(&e["args"]),
            };
            let mut chips: Vec<(&str, &str)> = Vec::new();
            let d1_chip = e["d1"]["confianca"].as_u64().map(|n| trf("approved by the d1 · {n} %", &[("n", n.to_string())]));
            if let Some(c) = &d1_chip {
                chips.push((c, "d1"));
            }
            if e["startup"].is_object() {
                chips.push((tr("startup file"), "inicio"));
            }
            if e["miniatura"].is_string() && status == "ok" {
                chips.push((tr("on the screen"), "tela"));
            }
            if desfeito {
                chips.push((tr("undone"), "desfeito"));
            } else if reversivel {
                chips.push((tr("reversible"), "reversivel"));
            }
            if leitura {
                chips.push((tr("read only"), "leitura"));
            }
            match status {
                "negado" => chips.push((tr("refused"), "negado")),
                "simulado" => chips.push((tr("simulated"), "simulado")),
                "erro" => chips.push((tr("failed"), "negado")),
                _ => {}
            }
            if !e["restore_point"].is_null() {
                chips.push((tr("restore point"), "leitura"));
            }
            let (linha, card) = self.cartao(t, &quem, classe, &titulo, &sub, &chips, id == self.escolhido.get());
            // an action on the screen: the screen after it, small (design page 29)
            if e["miniatura"].is_string() {
                card.append(&self.miniatura(id, 96, 54));
            }
            if reversivel {
                let b = gtk::Button::new();
                b.add_css_class("linha-acao");
                let bc = caixa(Orientation::Horizontal, 6, "");
                bc.append(&ui_icon("desfazer", if id == self.escolhido.get() { "#A5A1E2" } else { "#E2E2E6" }, 14));
                bc.append(&rotulo(tr("Undo"), ""));
                b.set_child(Some(&bc));
                b.set_valign(Align::Center);
                let w = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(l) = w.upgrade() {
                        l.escolher(id);
                        l.executar("linha_do_tempo.desfazer", json!({"id": id}));
                    }
                });
                crate::central::posicao_debug(&format!("desfazer registro {id}"), b.upcast_ref());
                card.append(&b);
            }
            let clique = gtk::GestureClick::new();
            let w = Rc::downgrade(self);
            clique.connect_released(move |_, _, _, _| {
                if let Some(l) = w.upgrade() {
                    l.escolher(id);
                }
            });
            card.add_controller(clique);
            self.lista.append(&linha);
        }
        // the rewind bar: from the first record shown to now, a mark at each reversible action
        self.preenchendo.set(true);
        let inicio = v.first().and_then(|e| e["time"].as_i64()).unwrap_or(n - 3600).min(n - 60);
        self.escala.set_range(inicio as f64, n as f64);
        self.escala.clear_marks();
        for e in v.iter().filter(|e| !e["inverse"].is_null() && e["status"] == "ok" && e["desfeito"] != true) {
            self.escala.add_mark(e["time"].as_f64().unwrap_or(0.0), gtk::PositionType::Bottom, None);
        }
        while let Some(x) = self.escala_marcas.first_child() {
            self.escala_marcas.remove(&x);
        }
        for (i, t) in [inicio, inicio + (n - inicio) / 3, inicio + 2 * (n - inicio) / 3, n].iter().enumerate() {
            let l = rotulo(&if i == 3 { tr("now").to_string() } else { hora(*t) }, "linha-rebobinar-marca");
            l.set_hexpand(true);
            l.set_xalign(match i { 0 => 0.0, 3 => 1.0, _ => 0.5 });
            self.escala_marcas.append(&l);
        }
        if self.escolhido.get() == 0 || !v.iter().any(|e| e["id"].as_u64() == Some(self.escolhido.get())) {
            self.escolhido.set(v.last().and_then(|e| e["id"].as_u64()).unwrap_or(0));
        }
        let t_esc = v.iter().find(|e| e["id"].as_u64() == Some(self.escolhido.get())).and_then(|e| e["time"].as_i64()).unwrap_or(n);
        self.escala.set_value(t_esc as f64);
        self.escala_hora.set_text(&hora(t_esc));
        self.preenchendo.set(false);
        self.detalhe();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: linha do tempo ({} ações, {desfeitas} desfeitas, {negadas} fora das permissões, {} esperando)", v.len(), pend.len());
        }
    }

    /// The notice over the list: which file, who, when; undo, and the setting (Permissions).
    fn startup_notice(self: &Rc<Self>, e: &Value) -> gtk::Box {
        let id = e["id"].as_u64().unwrap_or(0);
        let b = caixa(Orientation::Horizontal, 12, "linha-aviso-inicio");
        b.set_margin_bottom(4);
        let ic = ui_icon("alerta", "#E5C98C", 16);
        ic.set_valign(Align::Center);
        b.append(&ic);
        let col = caixa(Orientation::Vertical, 3, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let t = rotulo(tr("An agent changed a file that runs at login"), "linha-aviso-titulo");
        t.set_xalign(0.0);
        col.append(&t);
        let (quem, _) = self.quem(e);
        let file = e["startup"]["arquivos"][0].as_str().unwrap_or("").to_string();
        let sub = rotulo(&trf("{agent} changed {file} at {time}. You set these changes to “Allow automatically” (Permissions).",
            &[("agent", quem), ("file", file), ("time", hora(e["time"].as_i64().unwrap_or(0)))]), "linha-aviso-texto");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(1);
        col.append(&sub);
        b.append(&col);
        let desfazer = gtk::Button::new();
        desfazer.add_css_class("linha-acao");
        let bc = caixa(Orientation::Horizontal, 6, "");
        bc.append(&ui_icon("desfazer", "#A5A1E2", 14));
        bc.append(&rotulo(tr("Undo"), ""));
        desfazer.set_child(Some(&bc));
        desfazer.set_valign(Align::Center);
        let w = Rc::downgrade(self);
        desfazer.connect_clicked(move |_| {
            if let Some(l) = w.upgrade() {
                l.escolher(id);
                l.executar("linha_do_tempo.desfazer", json!({"id": id}));
            }
        });
        crate::central::posicao_debug("aviso de início: desfazer", desfazer.upcast_ref());
        b.append(&desfazer);
        let ajuste = gtk::Button::with_label(tr("Permissions"));
        ajuste.add_css_class("linha-revisar");
        ajuste.set_valign(Align::Center);
        let sh = self.shell.clone();
        ajuste.connect_clicked(move |_| {
            if let Some(s) = sh.upgrade() {
                s.abrir_central(Some("permissoes"));
            }
        });
        b.append(&ajuste);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: aviso de arquivo de início (registro {id}, {})", e["startup"]["arquivos"]);
        }
        b
    }

    fn secao(&self, t: &str) -> gtk::Label {
        let s = rotulo(t, "central-painel-secao");
        s.set_xalign(0.0);
        s.set_margin_top(18);
        s.set_margin_bottom(10);
        s
    }

    /// One record: time, the dot on the line, and the card (who, what, details, chips). Returns (row, card).
    #[allow(clippy::too_many_arguments)]
    fn cartao(&self, t: i64, quem: &str, classe: &str, titulo: &str, sub: &str, chips: &[(&str, &str)], escolhido: bool) -> (gtk::Box, gtk::Box) {
        let linha = caixa(Orientation::Horizontal, 12, "");
        linha.set_margin_bottom(10);
        // (the line beside the dot fills the row's height; the row itself does not take the list's spare height —
        // a lone record would stretch to the whole list, its buttons floating in the middle)
        linha.set_vexpand(false);
        let h = rotulo(&hora(t), "linha-hora");
        h.set_valign(Align::Start);
        h.set_margin_top(4);
        h.set_size_request(44, -1);
        h.set_xalign(0.0);
        linha.append(&h);
        let trilho = caixa(Orientation::Vertical, 0, "");
        let ponto = caixa(Orientation::Vertical, 0, "linha-ponto");
        if !classe.is_empty() {
            ponto.add_css_class(classe);
        }
        ponto.set_halign(Align::Center);
        ponto.set_margin_top(7);
        trilho.append(&ponto);
        let fio = caixa(Orientation::Vertical, 0, "linha-fio");
        fio.set_vexpand(true);
        fio.set_halign(Align::Center);
        trilho.append(&fio);
        linha.append(&trilho);
        let card = caixa(Orientation::Horizontal, 12, "linha-cartao");
        if escolhido {
            card.add_css_class("escolhido");
        }
        card.set_hexpand(true);
        let col = caixa(Orientation::Vertical, 5, "");
        col.set_hexpand(true);
        let l1 = caixa(Orientation::Horizontal, 8, "");
        let q = rotulo(quem, "linha-quem");
        if !classe.is_empty() {
            q.add_css_class(classe);
        }
        l1.append(&q);
        let tl = rotulo(titulo, "linha-texto");
        tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tl.set_max_width_chars(1);
        tl.set_hexpand(true);
        tl.set_xalign(0.0);
        l1.append(&tl);
        col.append(&l1);
        if !sub.is_empty() {
            let s = rotulo(sub, "central-agente-sub");
            s.set_xalign(0.0);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            s.set_max_width_chars(1);
            col.append(&s);
        }
        if !chips.is_empty() {
            let cs = caixa(Orientation::Horizontal, 6, "");
            for (t, c) in chips {
                let chip = rotulo(t, "linha-chip");
                chip.add_css_class(c);
                cs.append(&chip);
            }
            col.append(&cs);
        }
        card.append(&col);
        linha.append(&card);
        (linha, card)
    }

    /// A screen action's thumbnail (kalangod keeps it; read once, in the background).
    fn miniatura(self: &Rc<Self>, id: u64, w: i32, h: i32) -> gtk::Picture {
        let p = gtk::Picture::new();
        p.add_css_class("linha-miniatura");
        p.set_size_request(w, h);
        p.set_can_shrink(true);
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_valign(Align::Center);
        p.set_halign(Align::Start);
        if let Some(t) = self.miniaturas.borrow().get(&id) {
            p.set_paintable(Some(t));
            return p;
        }
        let (pw, lw) = (p.downgrade(), Rc::downgrade(self));
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_rpc("kalango/miniatura", json!({"id": id})).ok()
            .and_then(|v| v["png"].as_str().map(str::to_string)), move |b64| {
            let Some(bytes) = b64.map(|b| glib::base64_decode(&b)).filter(|b| !b.is_empty()) else { return };
            let Ok(t) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) else { return };
            if let Some(l) = lw.upgrade() {
                l.miniaturas.borrow_mut().insert(id, t.clone());
            }
            if let Some(p) = pw.upgrade() {
                p.set_paintable(Some(&t));
            }
        });
        p
    }

    fn escolher(self: &Rc<Self>, id: u64) {
        self.escolhido.set(id);
        let t = self.registros.borrow().iter().find(|e| e["id"].as_u64() == Some(id)).and_then(|e| e["time"].as_i64());
        if let Some(t) = t {
            self.preenchendo.set(true);
            self.escala.set_value(t as f64);
            self.escala_hora.set_text(&hora(t));
            self.preenchendo.set(false);
        }
        self.render();
    }

    fn botoes(&self) {
        let e = self.registros.borrow().iter().find(|e| e["id"].as_u64() == Some(self.escolhido.get())).cloned();
        let reversivel = e.as_ref().is_some_and(|e| !e["inverse"].is_null() && e["status"] == "ok" && e["desfeito"] != true);
        let ocupado = self.ocupado.get();
        let rotulo_b = |b: &gtk::Button, glifo: &str, cor: &str, t: &str| {
            let c = caixa(Orientation::Horizontal, 8, "");
            c.set_halign(Align::Center);
            c.append(&ui_icon(glifo, cor, 14));
            c.append(&rotulo(t, ""));
            b.set_child(Some(&c));
        };
        rotulo_b(&self.desfazer, "desfazer", "#1C1D24", if ocupado { tr("Waiting for your approval…") } else { tr("Undo this action") });
        self.desfazer.set_sensitive(reversivel && !ocupado);
        let desde = self.escala.value() as i64;
        let ha = self.registros.borrow().iter().any(|e| e["time"].as_i64().unwrap_or(0) >= desde && !e["inverse"].is_null() && e["status"] == "ok" && e["desfeito"] != true);
        rotulo_b(&self.desfazer_tudo, "relogio", "#E2E2E6", &trf("Undo everything since {time}", &[("time", hora(desde))]));
        self.desfazer_tudo.set_sensitive(ha && !ocupado);
    }

    /// The d1's recommendation in the detail (design page 31): "d1 seems safe · 94 %", why, and why it went ahead alone.
    fn render_d1(&self, d1: Option<&Value>) {
        while let Some(x) = self.d1_caixa.first_child() {
            self.d1_caixa.remove(&x);
        }
        let Some(d) = d1 else {
            self.d1_caixa.set_visible(false);
            return;
        };
        let spark = ui_icon("spark", "#8ED2C3", 13);
        spark.set_valign(Align::Start);
        self.d1_caixa.append(&spark);
        let col = caixa(Orientation::Vertical, 3, "");
        col.set_hexpand(true);
        let linha = caixa(Orientation::Horizontal, 6, "");
        linha.append(&rotulo("d1", "linha-d1-nome"));
        linha.append(&rotulo(tr("seems safe"), "linha-d1-veredito"));
        if let Some(n) = d["confianca"].as_u64() {
            linha.append(&rotulo(&format!("· {n} %"), "linha-d1-confianca"));
        }
        col.append(&linha);
        // (the reason is one of the d1's closed list: nothing the model or the action wrote is shown)
        if let Some(r) = d["motivo"].as_str().and_then(kalango_d1::Reason::from_key) {
            let m = rotulo(tr(r.text()), "central-agente-sub");
            m.set_xalign(0.0);
            m.set_wrap(true);
            m.set_max_width_chars(1);
            col.append(&m);
        }
        if let Some(min) = d["minimo"].as_u64() {
            let l = rotulo(&trf("approved by itself: confidence above {min} % (the minimum you chose)", &[("min", min.to_string())]), "central-agente-sub");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_max_width_chars(1);
            col.append(&l);
        }
        self.d1_caixa.append(&col);
        self.d1_caixa.set_visible(true);
    }

    fn detalhe(self: &Rc<Self>) {
        let e = self.registros.borrow().iter().find(|e| e["id"].as_u64() == Some(self.escolhido.get())).cloned();
        while let Some(x) = self.antes_depois.first_child() {
            self.antes_depois.remove(&x);
        }
        while let Some(x) = self.permissoes.first_child() {
            self.permissoes.remove(&x);
        }
        let Some(e) = e else {
            self.titulo.set_text(tr("Nothing chosen"));
            self.render_d1(None);
            self.meta.set_text("");
            self.porque.set_text("");
            self.explica.set_text("");
            self.botoes();
            return;
        };
        let t = e["time"].as_i64().unwrap_or(0);
        let (quem, _) = self.quem(&e);
        self.titulo.set_text(&titulo_de(&e));
        let d1 = e["d1"].as_object().is_some().then(|| e["d1"].clone());
        self.render_d1(d1.as_ref());
        if let Some(d) = &d1 {
            let _ = d;
            self.meta.set_text(&format!("{quem} · {} · {} · {}", hora(t), e["tool"].as_str().unwrap_or(""), tr("approved by the d1")));
        } else if e["startup"].is_object() {
            self.meta.set_text(&format!("{quem} · {} · {} · {}", hora(t), e["tool"].as_str().unwrap_or(""), tr("file that runs at login")));
        } else {
            self.meta.set_text(&format!("{quem} · {} · {}", hora(t), trf("level {level}", &[("level", e["level"].as_str().unwrap_or("").to_string())])));
        }
        // before / after
        let a = &e["args"];
        let caixa_ad = |rotulo_t: &str, linhas: Vec<String>, depois: bool| {
            let b = caixa(Orientation::Vertical, 6, "linha-ad");
            if depois {
                b.add_css_class("depois");
            }
            let r = rotulo(rotulo_t, "central-painel-secao");
            r.set_xalign(0.0);
            b.append(&r);
            for l in linhas {
                let x = caixa(Orientation::Horizontal, 8, "");
                x.append(&ui_icon("pasta", "#8E8F99", 14));
                let lt = rotulo(&l, "linha-ad-texto");
                lt.set_xalign(0.0);
                lt.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                lt.set_max_width_chars(1);
                lt.set_hexpand(true);
                x.append(&lt);
                b.append(&x);
            }
            b
        };
        if e["startup"]["linhas"].is_array() {
            // a file that runs at login: what changed in it, line by line
            self.antes_depois.append(&diff_box(&e["startup"]));
        } else if e["miniatura"].is_string() {
            // the screen before (the previous action's picture) and after this one
            let id = e["id"].as_u64().unwrap_or(0);
            let antes = self.registros.borrow().iter().rev().filter(|x| x["id"].as_u64().unwrap_or(0) < id && x["miniatura"].is_string())
                .find_map(|x| x["id"].as_u64());
            for (rotulo_t, rid, depois) in [(tr("BEFORE"), antes, false), (tr("AFTER"), Some(id), true)] {
                let b = caixa(Orientation::Vertical, 6, "linha-ad");
                if depois {
                    b.add_css_class("depois");
                }
                let r = rotulo(rotulo_t, "central-painel-secao");
                r.set_xalign(0.0);
                b.append(&r);
                match rid {
                    Some(rid) => b.append(&self.miniatura(rid, 202, 112)),
                    None => b.append(&rotulo(tr("the first action of the session"), "linha-ad-texto")),
                }
                self.antes_depois.append(&b);
            }
        } else if let (Some(de), Some(para)) = (a["de"].as_str(), a["para"].as_str()) {
            self.antes_depois.append(&caixa_ad(tr("BEFORE"), vec![de.to_string()], false));
            self.antes_depois.append(&caixa_ad(tr("AFTER"), vec![para.to_string()], true));
        } else if let Some(p) = a["caminho"].as_str() {
            let antes = if e["inverse"]["ferramenta"] == "arquivos.apagar_criado" { tr("did not exist").to_string() } else { p.to_string() };
            self.antes_depois.append(&caixa_ad(tr("BEFORE"), vec![antes], false));
            self.antes_depois.append(&caixa_ad(tr("AFTER"), vec![if e["tool"] == "arquivos.lixeira" { tr("in the trash").to_string() } else { p.to_string() }], true));
        }
        self.antes_depois.set_visible(self.antes_depois.first_child().is_some());
        // why
        let porque = match self.janela_de(t) {
            Some((nome, true)) => trf("Routine “{name}”.", &[("name", nome)]),
            Some((pedido, false)) => {
                let j = self.janelas.borrow().iter().find(|j| t >= j.inicio - 2 && t <= j.fim + 2).map(|j| j.inicio).unwrap_or(t);
                trf("Your request at {time}: “{request}”.", &[("time", hora(j)), ("request", pedido)])
            }
            None if self.quem(&e).1 == "voce" => tr("You did it, in the system.").to_string(),
            None => tr("No request of yours is linked to it (a summary in the background, or another program).").to_string(),
        };
        let porque = match e["startup"]["automatico"].as_bool() {
            Some(true) => format!("{porque} {}", tr("Files that run at login are set to “Allow automatically” (Permissions), so the agent did not ask for approval.")),
            Some(false) if e["status"] == "ok" => format!("{porque} {}", tr("It changes a file that runs at login: you approved it on the system screen.")),
            _ => porque,
        };
        let porque = if d1.is_some() { format!("{porque} {}", tr("Approval by the d1 was on (Permissions) and this is one of the low-risk actions it may approve.")) } else { porque };
        self.porque.set_text(&porque);
        // permissions used
        let startup_file = e["startup"]["arquivos"][0].as_str().map(str::to_string);
        let leitura = e["leitura"].as_bool().unwrap_or(false);
        let tool = e["tool"].as_str().unwrap_or("");
        let (col, glifo) = coluna(tool, leitura);
        let mut chips: Vec<(String, &str)> = vec![(tr(col).to_string(), glifo), (trf("level {level}", &[("level", e["level"].as_str().unwrap_or("").to_string())]), "escudo")];
        if let Some(f) = &startup_file {
            chips = vec![(f.clone(), "olho"), (tr(col).to_string(), glifo),
                (if e["startup"]["automatico"] == true { tr("automatic") } else { tr("approved by you") }.to_string(), "cadeado")];
        } else if let Some(p) = a["caminho"].as_str().or(a["de"].as_str()) {
            let pasta = std::path::Path::new(p).parent().map(|x| x.display().to_string()).unwrap_or_default();
            let home = glib::home_dir().display().to_string();
            chips.push((pasta.replacen(&home, "~", 1), "pasta"));
        }
        if let Some(n) = d1.as_ref().and_then(|d| d["confianca"].as_u64()) {
            chips.push((trf("d1 · {n} %", &[("n", n.to_string())]), "cadeado"));
        }
        for (t, g) in chips {
            let c = caixa(Orientation::Horizontal, 6, "linha-permissao");
            c.append(&ui_icon(g, "#C9CAD1", 13));
            let l = rotulo(&t, "");
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_max_width_chars(22);
            c.append(&l);
            self.permissoes.append(&c);
        }
        // what undoing does
        let explica = if let (Some(f), true) = (&startup_file, !e["inverse"].is_null() && e["status"] == "ok" && e["desfeito"] != true) {
            trf("Undoing puts {file} back as it was before the change.", &[("file", f.clone())])
        } else if e["miniatura"].is_string() {
            tr("An action on the screen is not undone from here: the app itself undoes it (Ctrl+Z). The thumbnails stay on this computer and go with the record.").to_string()
        } else if e["desfeito"] == true {
            tr("This action was already undone.").to_string()
        } else if !e["inverse"].is_null() && e["status"] == "ok" {
            trf("Undoing runs the recorded inverse ({tool}). It asks for your approval on the system screen and makes a restore point first.",
                &[("tool", e["inverse"]["ferramenta"].as_str().unwrap_or("").to_string())])
        } else if leitura {
            tr("It only read: there is nothing to undo.").to_string()
        } else if e["status"] != "ok" {
            tr("It was not done: there is nothing to undo.").to_string()
        } else {
            tr("This action has no recorded inverse; a restore point (Settings › Restore points) can bring the system back.").to_string()
        };
        self.explica.set_text(&explica);
        self.botoes();
    }
}

/// A record's title: for files, short (the file's name and folder, not the whole path); otherwise what kalangod
/// recorded; for a refusal, what the tool does.
fn titulo_de(e: &Value) -> String {
    let a = &e["args"];
    let nome = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string());
    let pasta = |p: &str| std::path::Path::new(p).parent().and_then(|x| x.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let status = e["status"].as_str().unwrap_or("");
    if status == "negado" || status == "erro" {
        return e["descricao"].as_str().unwrap_or(e["tool"].as_str().unwrap_or("")).to_string();
    }
    let caminho = a["caminho"].as_str().unwrap_or("");
    if let Some(f) = e["startup"]["arquivos"][0].as_str() {
        return trf("Changed {file}", &[("file", f.rsplit(" → ").next().unwrap_or(f).to_string())]);
    }
    match e["tool"].as_str().unwrap_or("") {
        "arquivos.mover" => {
            let (de, para) = (a["de"].as_str().unwrap_or(""), a["para"].as_str().unwrap_or(""));
            if pasta(de) == pasta(para) {
                trf("Renamed {from} to {to}", &[("from", nome(de)), ("to", nome(para))])
            } else {
                trf("Moved {file} to {folder}", &[("file", nome(de)), ("folder", pasta(para))])
            }
        }
        "arquivos.escrever" => trf("Wrote {file}", &[("file", nome(caminho))]),
        "arquivos.lixeira" => trf("Put {file} in the trash", &[("file", nome(caminho))]),
        "arquivos.ler" => trf("Read {file}", &[("file", nome(caminho))]),
        "arquivos.listar" => trf("Listed {folder}", &[("folder", nome(caminho))]),
        _ => {
            let s = e["summary"].as_str().unwrap_or("");
            let mut c = s.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// A change to a file that runs at login, in one line: the tool, how much changed, what kind of file it is.
fn startup_sub(e: &Value) -> String {
    [e["tool"].as_str().unwrap_or(""), e["startup"]["resumo"].as_str().unwrap_or(""), tr("file that runs at login")]
        .into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// What changed in a file, line by line (as the approval screen shows it): kept, removed, added, a gap.
fn diff_box(st: &Value) -> gtk::Box {
    let b = caixa(Orientation::Vertical, 4, "linha-diff");
    b.set_hexpand(true);
    let files: Vec<&str> = st["arquivos"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let head = rotulo(&format!("{} · {}", files.join(", "), st["resumo"].as_str().unwrap_or("")), "linha-diff-arquivo");
    head.set_xalign(0.0);
    head.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    head.set_max_width_chars(1);
    b.append(&head);
    let lines = st["linhas"].as_array().cloned().unwrap_or_default();
    for l in lines.iter().take(60) {
        let (sign, class) = match l["t"].as_str().unwrap_or(" ") {
            "+" => ("+", "mais"),
            "-" => ("-", "menos"),
            "@" => ("⋯", "intervalo"),
            _ => (" ", ""),
        };
        let row = caixa(Orientation::Horizontal, 8, "linha-diff-linha");
        if !class.is_empty() {
            row.add_css_class(class);
        }
        row.append(&rotulo(sign, "linha-diff-sinal"));
        let t = rotulo(l["l"].as_str().unwrap_or(""), "linha-diff-texto");
        t.set_xalign(0.0);
        t.set_hexpand(true);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(1);
        row.append(&t);
        b.append(&row);
    }
    b
}

/// What a record touched, in one line: from → to, the path, the packages…
/// A screen action's line: the tool, what it aimed at ("elemento e14", the keys, how many characters).
fn screen_sub(e: &Value) -> String {
    let a = &e["args"];
    let mut parts = vec![e["tool"].as_str().unwrap_or("").to_string()];
    if let Some(el) = a["elemento"].as_str() {
        parts.push(trf("element {e}", &[("e", el.to_string())]));
    } else if let (Some(x), Some(y)) = (a["x"].as_f64(), a["y"].as_f64()) {
        parts.push(format!("({}, {})", x.round(), y.round()));
    }
    if let Some(k) = a["teclas"].as_str() {
        parts.push(k.to_string());
    }
    if let Some(t) = a["texto"].as_str() {
        parts.push(trf("{n} characters", &[("n", t.chars().count().to_string())]));
    }
    if let Some(j) = a["janela"].as_str().filter(|j| !j.is_empty()) {
        parts.push(j.to_string());
    }
    parts.join(" · ")
}

fn resumo_args(a: &Value) -> String {
    let home = glib::home_dir().display().to_string();
    let curto = |s: &str| s.replacen(&home, "~", 1);
    if let (Some(de), Some(para)) = (a["de"].as_str(), a["para"].as_str()) {
        return format!("{} → {}", curto(de), curto(para));
    }
    for k in ["caminho", "url", "programa", "nome", "dispositivo", "chave", "descricao"] {
        if let Some(v) = a[k].as_str() {
            return curto(v);
        }
    }
    if let Some(n) = a["nomes"].as_array() {
        return n.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ");
    }
    String::new()
}
