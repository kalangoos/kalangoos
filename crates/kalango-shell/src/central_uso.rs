//! Agents center › Uso (design "Uso e custos dos agentes"): how much the agents worked and what it cost — in plain
//! words, not only tokens. Tokens per day (local and cloud) from the token log, tasks done and undone, an estimate of
//! the time saved (its rule in the tooltip), a table per agent (each request or routine gets the tokens logged while
//! it ran; the rest are the system's own summaries), the limits, where the cloud went, and a suggestion.
//!
//! This build has no cloud provider: cloud cost is zero and nothing left the computer — the page says so, and the
//! cloud controls stay off with the reason.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::process::Command;
use std::rc::{Rc, Weak};

const PERIODOS: [&str; 3] = [n_("Today"), n_("7 days"), n_("30 days")];

fn agora() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

fn inicio_do_dia(t: i64) -> i64 {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0)
}

pub(crate) fn mil(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1} M", n as f64 / 1e6).replace('.', ",") } else if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

/// The tokens the agents used today (all of them, local and cloud).
pub(crate) fn tokens_hoje() -> u64 {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    log_de_tokens(inicio_do_dia(agora)).iter().map(|(_, n, _)| n).sum()
}

/// The token log: (time, tokens, cloud?).
fn log_de_tokens(desde: i64) -> Vec<(i64, u64, bool)> {
    let p = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/uso-tokens.jsonl");
    std::fs::read_to_string(p).unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["t"].as_i64()?, v["tokens"].as_u64()?, v["tipo"] == "nuvem"))).filter(|(t, _, _)| *t >= desde).collect()
}

/// The usage settings of the shell (warn at 80% of the daily limit).
fn ajustes_path() -> std::path::PathBuf {
    glib::user_config_dir().join("kalango/uso.json")
}

pub fn avisar_80() -> bool {
    std::fs::read(ajustes_path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["avisar_80"].as_bool()).unwrap_or(true)
}

fn set_avisar_80(on: bool) {
    let p = ajustes_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, serde_json::json!({"avisar_80": on}).to_string());
}

/// The daily token limit the agent enforces (Permissions › Tokens a day), if one is set.
pub fn limite_diario() -> Option<u64> {
    std::fs::read(glib::user_config_dir().join("kalango/agente-permissoes.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["tokens_dia"].as_u64()).filter(|n| *n > 0)
}

/// Warns once a day when today's tokens pass 80% of the daily limit, and again at the limit (the shell calls this
/// every minute).
pub fn vigiar_limite(shell: &Shell, avisado: &Cell<(i64, u8)>) {
    let Some(lim) = limite_diario() else { return };
    if !avisar_80() {
        return;
    }
    let hoje = inicio_do_dia(agora());
    let usados: u64 = log_de_tokens(hoje).iter().map(|(_, n, _)| n).sum();
    let nivel = if usados >= lim { 2 } else if usados * 10 >= lim * 8 { 1 } else { 0 };
    let (dia, ja) = avisado.get();
    let ja = if dia == hoje { ja } else { 0 };
    if nivel > ja {
        avisado.set((hoje, nivel));
        let (titulo, corpo) = if nivel == 2 {
            (tr("Daily token limit reached"), trf("The agents used {used} of {limit} tokens today and stop until tomorrow (or until the limit changes).", &[("used", mil(usados)), ("limit", mil(lim))]))
        } else {
            (tr("80% of the daily token limit"), trf("The agents used {used} of {limit} tokens today.", &[("used", mil(usados)), ("limit", mil(lim))]))
        };
        shell.notificar(tr("Agents"), titulo, &corpo);
    }
}

pub struct PaginaUso {
    pub raiz: gtk::Box,
    shell: Weak<Shell>,
    periodos: Vec<gtk::ToggleButton>,
    periodo: Cell<usize>,
    cartoes: [(gtk::Label, gtk::Label, gtk::Label); 4],
    grafico: gtk::DrawingArea,
    barras: Rc<RefCell<Vec<(String, u64, u64)>>>,
    tabela: gtk::Box,
    tokens: gtk::Scale,
    tokens_v: gtk::Label,
    avisar: gtk::Switch,
    sugestao: gtk::Box,
    sugestao_texto: gtk::Label,
    sugestao_valor: Cell<u64>,
    aplicar: gtk::Button,
    preenchendo: Cell<bool>,
}

impl PaginaUso {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<PaginaUso> {
        let compacta = largura_janela < 1480;
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let topo = caixa(Orientation::Horizontal, 8, "");
        let t = caixa(Orientation::Vertical, 4, "");
        t.set_hexpand(true);
        let h = rotulo(tr("Usage"), "central-titulo");
        h.set_xalign(0.0);
        t.append(&h);
        let sub = rotulo(tr("How much the agents worked and what it cost."), "central-sub");
        sub.set_xalign(0.0);
        t.append(&sub);
        topo.append(&t);
        let seg = caixa(Orientation::Horizontal, 0, "uso-segmentos");
        seg.set_valign(Align::Start);
        seg.set_margin_top(6);
        let mut periodos: Vec<gtk::ToggleButton> = Vec::new();
        for p in PERIODOS {
            let b = gtk::ToggleButton::with_label(tr(p));
            b.add_css_class("uso-segmento");
            if let Some(f) = periodos.first() {
                b.set_group(Some(f));
            }
            seg.append(&b);
            periodos.push(b);
        }
        topo.append(&seg);
        corpo.append(&topo);
        let linha = caixa(Orientation::Horizontal, 12, "");
        linha.set_homogeneous(true);
        linha.set_margin_top(14);
        let cartao = |classe: &str| {
            let c = caixa(Orientation::Vertical, 2, "linha-stat");
            let n = rotulo("", "uso-numero");
            if !classe.is_empty() {
                n.add_css_class(classe);
            }
            n.set_xalign(0.0);
            c.append(&n);
            let a = rotulo("", "uso-cartao-nome");
            a.set_xalign(0.0);
            a.set_margin_top(4);
            c.append(&a);
            let b = rotulo("", "central-agente-sub");
            b.set_xalign(0.0);
            c.append(&b);
            linha.append(&c);
            (n, a, b)
        };
        let cartoes = [cartao(""), cartao("gold"), cartao("teal"), cartao("accent")];
        corpo.append(&linha);
        // tokens per day
        let gc = caixa(Orientation::Vertical, 8, "uso-grafico");
        gc.set_margin_top(14);
        let gcab = caixa(Orientation::Horizontal, 12, "");
        let gt = rotulo(tr("TOKENS PER DAY"), "central-painel-secao");
        gt.set_hexpand(true);
        gt.set_xalign(0.0);
        gcab.append(&gt);
        for (cor, nome) in [("local", tr("local")), ("nuvem", tr("cloud"))] {
            let l = caixa(Orientation::Horizontal, 5, "");
            let p = caixa(Orientation::Vertical, 0, "uso-legenda");
            p.add_css_class(cor);
            p.set_valign(Align::Center);
            l.append(&p);
            l.append(&rotulo(nome, "central-agente-sub"));
            gcab.append(&l);
        }
        gc.append(&gcab);
        let grafico = gtk::DrawingArea::new();
        grafico.set_content_height(150);
        grafico.set_hexpand(true);
        gc.append(&grafico);
        corpo.append(&gc);
        // per agent
        let tabela = caixa(Orientation::Vertical, 0, "central-tabela");
        tabela.set_margin_top(14);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&tabela));
        corpo.append(&rolagem);
        raiz.append(&corpo);

        // limits, where the cloud went, a suggestion
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("raio", "#A5A1E2", 14));
        cab.append(&rotulo(tr("LIMITS"), "central-painel-agente"));
        painel.append(&cab);
        let lim = caixa(Orientation::Vertical, 0, "central-cartao");
        lim.set_margin_top(14);
        let linha_lim = |nome: &str, explica: &str, controle: &gtk::Widget| {
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
            controle.set_valign(Align::Center);
            l.append(controle);
            lim.append(&l);
            l
        };
        let nuvem = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 5.0);
        nuvem.set_draw_value(false);
        nuvem.set_size_request(if compacta { 120 } else { 190 }, -1);
        nuvem.add_css_class("central-escala");
        nuvem.set_sensitive(false);
        let nl = linha_lim(tr("Cloud per month"), "", nuvem.upcast_ref());
        nl.set_tooltip_text(Some(tr("No cloud provider is set up, so nothing is spent.")));
        let tc = caixa(Orientation::Horizontal, 10, "");
        let tokens = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 1_000_000.0, 10_000.0);
        tokens.set_draw_value(false);
        tokens.set_size_request(if compacta { 120 } else { 190 }, -1);
        tokens.add_css_class("central-escala");
        tc.append(&tokens);
        let tokens_v = rotulo("", "central-limite-valor");
        tokens_v.set_size_request(56, -1);
        tokens_v.set_xalign(1.0);
        tc.append(&tokens_v);
        linha_lim(tr("Tokens a day"), "", tc.upcast_ref());
        let avisar = gtk::Switch::new();
        avisar.add_css_class("central-switch");
        linha_lim(tr("Warn at 80% of the limit"), "", avisar.upcast_ref());
        let preferir = gtk::Switch::new();
        preferir.add_css_class("central-switch");
        preferir.set_active(true);
        preferir.set_sensitive(false);
        let pl = linha_lim(tr("Prefer the local model"), tr("cloud only when the local one cannot do it"), preferir.upcast_ref());
        pl.set_tooltip_text(Some(tr("No cloud provider is set up: everything runs on the local model.")));
        painel.append(&lim);
        let cab2 = caixa(Orientation::Horizontal, 8, "");
        cab2.set_margin_top(18);
        cab2.append(&ui_icon("globo", "#A5A1E2", 14));
        cab2.append(&rotulo(tr("WHERE THE CLOUD WENT"), "central-painel-agente"));
        painel.append(&cab2);
        let nuvem_c = caixa(Orientation::Vertical, 8, "central-cartao");
        nuvem_c.add_css_class("memoria-info");
        nuvem_c.set_margin_top(12);
        let nada = rotulo(tr("No cloud provider is set up: nothing left this computer."), "central-agente-sub");
        nada.set_xalign(0.0);
        nada.set_wrap(true);
        nuvem_c.append(&nada);
        let env = caixa(Orientation::Horizontal, 8, "");
        let el = rotulo(tr("Files sent"), "central-agente-sub");
        el.set_hexpand(true);
        el.set_xalign(0.0);
        env.append(&el);
        env.append(&rotulo(tr("none"), "uso-nenhum"));
        nuvem_c.append(&env);
        painel.append(&nuvem_c);
        let sugestao = caixa(Orientation::Vertical, 6, "rotinas-sugestao");
        sugestao.set_margin_top(14);
        let sc = caixa(Orientation::Horizontal, 7, "");
        sc.append(&estrela(13, "#A5A1E2"));
        sc.append(&rotulo(tr("SUGGESTION"), "central-painel-agente"));
        sugestao.append(&sc);
        let sugestao_texto = rotulo("", "central-limite-nome");
        sugestao_texto.set_xalign(0.0);
        sugestao_texto.set_wrap(true);
        sugestao.append(&sugestao_texto);
        let aplicar = gtk::Button::with_label(tr("Apply"));
        aplicar.add_css_class("rotinas-sugestao-criar");
        aplicar.set_halign(Align::Start);
        aplicar.set_margin_top(4);
        sugestao.append(&aplicar);
        sugestao.set_visible(false);
        painel.append(&sugestao);
        raiz.append(&painel);

        let p = Rc::new(PaginaUso {
            raiz, shell: Rc::downgrade(shell), periodos, periodo: Cell::new(1), cartoes, grafico, barras: Rc::default(), tabela, tokens, tokens_v, avisar,
            sugestao, sugestao_texto, sugestao_valor: Cell::new(0), aplicar: aplicar.clone(), preenchendo: Cell::new(false),
        });
        p.periodos[1].set_active(true);
        for (i, b) in p.periodos.iter().enumerate() {
            let w = Rc::downgrade(&p);
            b.connect_toggled(move |b| {
                if let Some(p) = w.upgrade().filter(|_| b.is_active()) {
                    p.periodo.set(i);
                    p.render();
                }
            });
        }
        let barras = p.barras.clone();
        p.grafico.set_draw_func(move |_, cr, w, h| desenhar(cr, w as f64, h as f64, &barras.borrow()));
        let espera: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let w = Rc::downgrade(&p);
        p.tokens.connect_value_changed(move |s| {
            let Some(p) = w.upgrade() else { return };
            let n = s.value().round() as u64;
            p.tokens_v.set_text(&if n == 0 { tr("no limit").to_string() } else { mil(n) });
            if p.preenchendo.get() {
                return;
            }
            if let Some(id) = espera.borrow_mut().take() {
                id.remove();
            }
            let (w, e2) = (Rc::downgrade(&p), espera.clone());
            *espera.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(450), move || {
                e2.borrow_mut().take();
                if let Some(p) = w.upgrade() {
                    p.limite(n);
                }
            }));
        });
        let w = Rc::downgrade(&p);
        p.avisar.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|p| !p.preenchendo.get()) {
                set_avisar_80(on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&p);
        aplicar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let n = p.sugestao_valor.get();
                p.limite(n);
            }
        });
        p
    }

    /// The daily token limit (the same as Permissions › Tokens a day: the agent enforces it).
    fn limite(self: &Rc<Self>, n: u64) {
        let w = Rc::downgrade(self);
        let bin = crate::voz::agent_binary();
        crate::lateral::em_segundo_plano(move || Command::new(&bin).args(["limite", "tokens", &n.to_string()]).status().map(|s| s.success()).unwrap_or(false), move |ok| {
            if !ok {
                eprintln!("kalango-shell: limite de tokens não foi salvo");
            }
            if let Some(p) = w.upgrade() {
                p.render();
            }
        });
    }

    pub fn render(self: &Rc<Self>) {
        let n = agora();
        let hoje = inicio_do_dia(n);
        let (desde, dias) = match self.periodo.get() { 0 => (hoje, 1), 1 => (hoje - 6 * 86_400, 7), _ => (hoje - 29 * 86_400, 30) };
        let log = log_de_tokens(desde);
        let total: u64 = log.iter().map(|(_, k, _)| k).sum();
        let nuvem: u64 = log.iter().filter(|(_, _, c)| *c).map(|(_, k, _)| k).sum();
        let tarefas: Vec<Value> = crate::conversa::tarefas(2000).into_iter().filter(|t| t["fim"].as_i64().unwrap_or(0) >= desde).collect();
        let concluidas: Vec<&Value> = tarefas.iter().filter(|t| t["estado"] == "concluido").collect();
        let periodo = match self.periodo.get() { 0 => tr("today").to_string(), 1 => tr("in 7 days").to_string(), _ => tr("in 30 days").to_string() };
        let c = &self.cartoes;
        c[0].0.set_text(&mil(total));
        c[0].1.set_text(&trf("tokens {period}", &[("period", periodo.clone())]));
        c[0].2.set_text(&if total == 0 { tr("nothing yet").to_string() } else { trf("{n}% on the local model", &[("n", (100 - nuvem * 100 / total.max(1)).to_string())]) });
        c[1].0.set_text("R$ 0,00");
        c[1].1.set_text(tr("spent on the cloud"));
        c[1].2.set_text(tr("no cloud provider set up"));
        c[2].0.set_text(&concluidas.len().to_string());
        c[2].1.set_text(tr("tasks done"));
        // undone by the person: from kalangod's records (in the background)
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_registros(desde, 5000).map(|r| r.iter().filter(|e| e["status"] == "desfeito").count()).unwrap_or(0),
            move |k| {
                if let Some(p) = w.upgrade() {
                    p.cartoes[2].2.set_text(&if k == 1 { tr("1 undone by you").to_string() } else { trf("{n} undone by you", &[("n", k.to_string())]) });
                }
            });
        let acoes: usize = concluidas.iter().map(|t| t["passos"].as_array().map(|a| a.len()).unwrap_or(0)).sum();
        let min = acoes as i64;
        c[3].0.set_text(&if min >= 60 { format!("{} h {:02}", min / 60, min % 60) } else { format!("{min} min") });
        c[3].1.set_text(tr("time saved"));
        c[3].2.set_text(tr("system estimate"));
        if let Some(x) = c[3].0.parent() {
            x.set_tooltip_text(Some(tr("Estimate: one minute for each action the agent took in a finished task.")));
        }
        // the chart: hours today, else days
        let mut barras: Vec<(String, u64, u64)> = Vec::new();
        if dias == 1 {
            for h in 0..24 {
                let (a, b) = (hoje + h * 3600, hoje + (h + 1) * 3600);
                let (l, nv) = log.iter().filter(|(t, _, _)| *t >= a && *t < b).fold((0, 0), |(l, nv), (_, k, c)| if *c { (l, nv + k) } else { (l + k, nv) });
                barras.push((if h % 3 == 0 { format!("{h}h") } else { String::new() }, l, nv));
            }
        } else {
            for d in 0..dias {
                let a = desde + d * 86_400;
                let (l, nv) = log.iter().filter(|(t, _, _)| *t >= a && *t < a + 86_400).fold((0, 0), |(l, nv), (_, k, c)| if *c { (l, nv + k) } else { (l + k, nv) });
                let nome = glib::DateTime::from_unix_local(a + 43_200).ok().and_then(|x| x.format(if dias == 7 { "%a" } else { "%d" }).ok())
                    .map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default();
                barras.push((if dias == 7 || d % 5 == 0 || d == dias - 1 { nome } else { String::new() }, l, nv));
            }
        }
        *self.barras.borrow_mut() = barras;
        self.grafico.queue_draw();
        // per agent: the tokens logged while each request or routine ran
        let mut por: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        let mut atribuido = vec![false; log.len()];
        for t in &tarefas {
            let (a, b) = (t["inicio"].as_i64().unwrap_or(0) - 2, t["fim"].as_i64().unwrap_or(0) + 2);
            let nome = t["rotina"].as_str().map(|r| trf("Routine {name}", &[("name", r.to_string())])).unwrap_or_else(|| tr("Agent").to_string());
            let e = por.entry(nome).or_default();
            e.0 += 1;
            for (i, (tt, k, _)) in log.iter().enumerate() {
                if *tt >= a && *tt <= b && !atribuido[i] {
                    atribuido[i] = true;
                    e.1 += k;
                }
            }
        }
        let resto: u64 = log.iter().zip(&atribuido).filter(|(_, a)| !**a).map(|((_, k, _), _)| k).sum();
        while let Some(x) = self.tabela.first_child() {
            self.tabela.remove(&x);
        }
        let cab = caixa(Orientation::Horizontal, 0, "uso-tabela-cab");
        for (t, larg, x) in [(tr("Agent"), -1, 0.0), (tr("Tasks"), 90, 1.0), (tr("Tokens"), 110, 1.0), (tr("Model"), 110, 1.0), (tr("Cost"), 110, 1.0)] {
            let l = rotulo(t, "central-tabela-titulo");
            l.set_xalign(x);
            if larg < 0 { l.set_hexpand(true) } else { l.set_size_request(larg, -1) }
            cab.append(&l);
        }
        self.tabela.append(&cab);
        let mut linhas: Vec<(String, String, u64, &str)> = por.into_iter().map(|(n, (k, t))| (n, k.to_string(), t, "agente")).collect();
        linhas.sort_by_key(|(_, _, t, _)| std::cmp::Reverse(*t));
        if resto > 0 {
            linhas.push((tr("System summaries").to_string(), "—".into(), resto, "sistema"));
        }
        if linhas.is_empty() {
            let v = rotulo(tr("No agent worked in this period."), "central-sub");
            v.set_xalign(0.0);
            v.set_margin_top(12);
            v.set_margin_bottom(12);
            v.set_margin_start(16);
            self.tabela.append(&v);
        }
        for (nome, k, t, tipo) in linhas {
            let l = caixa(Orientation::Horizontal, 0, "uso-tabela-linha");
            let q = caixa(Orientation::Horizontal, 10, "");
            q.set_hexpand(true);
            let av = caixa(Orientation::Horizontal, 0, "central-avatar");
            av.set_size_request(22, 22);
            av.set_valign(Align::Center);
            av.set_hexpand(false);
            let g: gtk::Widget = if tipo == "sistema" { ui_icon("cpu", "#7CC3DF", 12).upcast() } else { estrela(11, "#A5A1E2").upcast() };
            g.set_hexpand(true);
            g.set_halign(Align::Center);
            g.set_valign(Align::Center);
            av.append(&g);
            q.append(&av);
            q.append(&rotulo(&nome, "central-limite-nome"));
            l.append(&q);
            for (txt, larg) in [(k, 90), (mil(t), 110), (tr("local").to_string(), 110), ("—".to_string(), 110)] {
                let x = rotulo(&txt, "uso-mono");
                x.set_xalign(1.0);
                x.set_size_request(larg, -1);
                l.append(&x);
            }
            self.tabela.append(&l);
        }
        // limits
        self.preenchendo.set(true);
        let lim = limite_diario();
        self.tokens.set_value(lim.unwrap_or(0) as f64);
        self.tokens_v.set_text(&lim.map(mil).unwrap_or_else(|| tr("no limit").to_string()));
        self.avisar.set_active(avisar_80());
        self.preenchendo.set(false);
        // a suggestion from the use: a daily limit when there is none (about twice a normal day), or a higher one when
        // today is near it
        let semana = log_de_tokens(hoje - 6 * 86_400);
        let dia_medio = semana.iter().map(|(_, k, _)| k).sum::<u64>() / 7;
        // never below the busiest day (today included): a suggested limit must not stop the agents right away
        let dia_maximo = (0..7).map(|d| {
            let a = hoje - d * 86_400;
            semana.iter().filter(|(t, _, _)| *t >= a && *t < a + 86_400).map(|(_, k, _)| k).sum::<u64>()
        }).max().unwrap_or(0);
        let hoje_t: u64 = log_de_tokens(hoje).iter().map(|(_, k, _)| k).sum();
        let arred = |x: u64| (x.div_ceil(10_000) * 10_000).max(20_000);
        let sugerido = arred((dia_medio * 2).max(dia_maximo * 3 / 2));
        let sug = match lim {
            None if dia_medio > 0 => Some((sugerido, trf("No daily token limit is set. A normal day here uses about {avg}: a limit of {limit} keeps a runaway agent in check.", &[("avg", mil(dia_medio)), ("limit", mil(sugerido))]))),
            Some(l) if hoje_t * 10 >= l * 8 => Some((arred(l * 3 / 2), trf("Today is already at {used} of {limit}. Raise the limit to {new}?", &[("used", mil(hoje_t)), ("limit", mil(l)), ("new", mil(arred(l * 3 / 2)))]))),
            _ => None,
        };
        match sug {
            Some((v, t)) => {
                self.sugestao_valor.set(v);
                self.sugestao_texto.set_text(&t);
                self.sugestao.set_visible(true);
                crate::central::posicao_debug("aplicar", self.aplicar.upcast_ref());
            }
            None => self.sugestao.set_visible(false),
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: uso ({total} tokens, {} tarefas concluídas, limite {:?})", concluidas.len(), lim);
        }
        let _ = self.shell.upgrade();
    }
}

/// Stacked bars (local, then cloud on top), the labels under them.
fn desenhar(cr: &gtk::cairo::Context, w: f64, h: f64, barras: &[(String, u64, u64)]) {
    if barras.is_empty() {
        return;
    }
    let max = barras.iter().map(|(_, l, n)| l + n).max().unwrap_or(0).max(1) as f64;
    let base = h - 20.0;
    let n = barras.len() as f64;
    let largura = (w / n * 0.62).min(56.0);
    let passo = (w - 32.0) / n;
    // (in the theme's colours)
    let cor = |cr: &gtk::cairo::Context, (r, g, b): (u8, u8, u8)| {
        let (r, g, b) = crate::tema::rgb(&format!("#{r:02X}{g:02X}{b:02X}"));
        cr.set_source_rgb(r, g, b)
    };
    for (i, (nome, l, nv)) in barras.iter().enumerate() {
        let x = 16.0 + passo * i as f64 + (passo - largura) / 2.0;
        let hl = (*l as f64 / max) * (base - 8.0);
        let hn = (*nv as f64 / max) * (base - 8.0);
        if hl > 0.0 {
            cor(cr, (0x8E, 0xD2, 0xC3));
            cr.rectangle(x, base - hl, largura, hl);
            let _ = cr.fill();
        } else {
            cor(cr, (0x26, 0x27, 0x2F));
            cr.rectangle(x, base - 2.0, largura, 2.0);
            let _ = cr.fill();
        }
        if hn > 0.0 {
            cor(cr, (0xE5, 0xC9, 0x8C));
            cr.rectangle(x, base - hl - hn - 2.0, largura, hn);
            let _ = cr.fill();
        }
        if !nome.is_empty() {
            cor(cr, (0x8E, 0x8F, 0x99));
            cr.select_font_face(crate::tema::fonte_ui(), gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(11.0);
            if let Ok(ext) = cr.text_extents(nome) {
                cr.move_to(x + largura / 2.0 - ext.width() / 2.0, h - 4.0);
                let _ = cr.show_text(nome);
            }
        }
    }
}
