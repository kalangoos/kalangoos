//! Agents center › Rotinas (design "Rotinas (agentes agendados e gatilhos)"): the routines with their trigger, state
//! and last result, a switch each; "Nova rotina" (an inline form: name, what the agent does, when); a click on a
//! routine runs it now or deletes it. On the right, the next scheduled runs and the patterns the system noticed in
//! what the person does ("Criar rotina", "Não sugerir de novo"). The scheduler is `rotinas::Agendador`.

use crate::conversa::ui_icon;
use crate::rotinas::{Acao, Gatilho, Rotinas};
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The form's "when": (label, days or a trigger without time).
fn quandos() -> Vec<(String, Option<Vec<u8>>)> {
    let mut v = vec![(tr("every day").to_string(), Some(vec![])), (tr("weekdays").to_string(), Some(vec![1, 2, 3, 4, 5]))];
    for d in 1..=7u8 {
        let nome = glib::DateTime::from_local(2024, 1, d as i32, 12, 0, 0.0).ok().and_then(|x| x.format("%A").ok()).map(|s| s.to_string()).unwrap_or_default();
        v.push((trf("every {day}", &[("day", nome)]), Some(vec![d])));
    }
    v.push((tr("when a USB device is connected").to_string(), None));
    v.push((tr("when entering a space…").to_string(), None));
    v
}

/// "18:00", "18h", "18h30", "18.30", "1830" or "18".
fn hora_valida(t: &str) -> Option<(u8, u8)> {
    let t = t.trim().to_lowercase();
    let (h, m) = match t.split_once([':', 'h', '.']) {
        Some((h, m)) => (h.to_string(), if m.trim().is_empty() { "0".to_string() } else { m.to_string() }),
        None if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) => (t[..2].to_string(), t[2..].to_string()),
        None => (t.clone(), "0".to_string()),
    };
    let (h, m) = (h.trim().parse::<u8>().ok()?, m.trim().parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_as_people_write_them() {
        for (t, h) in [("18:00", (18, 0)), ("18h", (18, 0)), ("18h30", (18, 30)), ("7.05", (7, 5)), ("1830", (18, 30)), ("9", (9, 0))] {
            assert_eq!(super::hora_valida(t), Some(h), "{t}");
        }
        assert_eq!(super::hora_valida("25:00"), None);
        assert_eq!(super::hora_valida("amanhã"), None);
    }
}

pub struct PaginaRotinas {
    pub raiz: gtk::Box,
    shell: Weak<Shell>,
    lista: gtk::Box,
    nova: gtk::Button,
    form: gtk::Revealer,
    nome: gtk::Entry,
    pedido: gtk::Entry,
    quando: gtk::DropDown,
    hora: gtk::Entry,
    espaco: gtk::Entry,
    form_erro: gtk::Label,
    proximas: gtk::Box,
    sugestoes: gtk::Box,
    preenchendo: Cell<bool>,
    /// the flows of kalango-fluxos, above the simple routines
    flows: gtk::Box,
    open_flow: RefCell<Option<Rc<dyn Fn(&str)>>>,
}

impl PaginaRotinas {
    pub fn new(shell: &Rc<Shell>, largura_janela: i32) -> Rc<PaginaRotinas> {
        let compacta = largura_janela < 1480;
        let raiz = caixa(Orientation::Horizontal, 0, "");
        raiz.set_vexpand(true);
        let corpo = caixa(Orientation::Vertical, 0, "central-corpo");
        corpo.set_hexpand(true);
        let topo = caixa(Orientation::Horizontal, 8, "");
        let t = caixa(Orientation::Vertical, 4, "");
        t.set_hexpand(true);
        let h = rotulo(tr("Routines"), "central-titulo");
        h.set_xalign(0.0);
        t.append(&h);
        let sub = rotulo(tr("Agents that run by themselves: at a time or when something happens."), "central-sub");
        sub.set_xalign(0.0);
        t.append(&sub);
        topo.append(&t);
        let nova = gtk::Button::new();
        nova.add_css_class("rotinas-nova");
        let nc = caixa(Orientation::Horizontal, 6, "");
        nc.append(&ui_icon("mais", "#1C1D24", 14));
        nc.append(&rotulo(tr("New routine"), ""));
        nova.set_child(Some(&nc));
        nova.set_valign(Align::Start);
        nova.set_margin_top(6);
        let new_flow = gtk::Button::new();
        new_flow.add_css_class("central-botao");
        new_flow.add_css_class("ao-lado");
        let nfc = caixa(Orientation::Horizontal, 6, "");
        nfc.append(&ui_icon("raio", "#E2E2E6", 14));
        nfc.append(&rotulo(tr("New flow"), ""));
        new_flow.set_child(Some(&nfc));
        new_flow.set_valign(Align::Start);
        new_flow.set_margin_top(6);
        topo.append(&new_flow);
        topo.append(&nova);
        corpo.append(&topo);
        // the inline form
        let form = gtk::Revealer::new();
        form.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        form.set_transition_duration(220);
        let fc = caixa(Orientation::Vertical, 10, "rotinas-form");
        fc.set_margin_top(14);
        let campo = |dica: &str| {
            let e = gtk::Entry::new();
            e.set_placeholder_text(Some(dica));
            e.add_css_class("rotinas-campo");
            e
        };
        let nome = campo(tr("Name (e.g. Friday clean-up)"));
        fc.append(&nome);
        let pedido = campo(tr("What the agent does (e.g. Organize ~/Downloads by type)"));
        fc.append(&pedido);
        let linha = caixa(Orientation::Horizontal, 8, "");
        let rotulos: Vec<String> = quandos().into_iter().map(|(l, _)| l).collect();
        let quando = gtk::DropDown::from_strings(&rotulos.iter().map(String::as_str).collect::<Vec<_>>());
        quando.add_css_class("central-escolha");
        quando.set_valign(Align::Center);
        linha.append(&quando);
        let hora = campo("09:00");
        hora.set_text("09:00");
        hora.set_width_chars(6);
        hora.set_max_width_chars(6);
        linha.append(&hora);
        let espaco = campo(tr("space name"));
        espaco.set_visible(false);
        linha.append(&espaco);
        let vazio = caixa(Orientation::Horizontal, 0, "");
        vazio.set_hexpand(true);
        linha.append(&vazio);
        let cancelar = gtk::Button::with_label(tr("Cancel"));
        cancelar.add_css_class("rotinas-texto");
        linha.append(&cancelar);
        let criar = gtk::Button::with_label(tr("Create"));
        criar.add_css_class("rotinas-criar");
        linha.append(&criar);
        fc.append(&linha);
        let form_erro = rotulo("", "rotinas-erro");
        form_erro.set_xalign(0.0);
        form_erro.set_visible(false);
        fc.append(&form_erro);
        form.set_child(Some(&fc));
        corpo.append(&form);
        let lista = caixa(Orientation::Vertical, 8, "");
        lista.set_margin_top(8);
        let flows = caixa(Orientation::Vertical, 8, "");
        flows.set_margin_top(16);
        let ambas = caixa(Orientation::Vertical, 0, "");
        ambas.append(&flows);
        ambas.append(&lista);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&ambas));
        corpo.append(&rolagem);
        raiz.append(&corpo);

        // next runs and suggestions, on the right
        let painel = caixa(Orientation::Vertical, 0, "central-painel");
        painel.set_size_request(if compacta { 340 } else { 474 }, -1);
        // fixed: what expands inside it must not widen it (GTK passes expand up to the ancestors)
        painel.set_hexpand(false);
        let cab = caixa(Orientation::Horizontal, 8, "");
        cab.append(&ui_icon("relogio", "#A5A1E2", 14));
        cab.append(&rotulo(tr("NEXT RUNS"), "central-painel-agente"));
        painel.append(&cab);
        let proximas = caixa(Orientation::Vertical, 10, "central-cartao");
        proximas.add_css_class("rotinas-proximas");
        proximas.set_margin_top(14);
        painel.append(&proximas);
        let cab2 = caixa(Orientation::Horizontal, 7, "");
        cab2.set_margin_top(18);
        cab2.append(&estrela(13, "#A5A1E2"));
        cab2.append(&rotulo(tr("SUGGESTED — THE AGENT NOTICED A PATTERN"), "central-painel-agente"));
        painel.append(&cab2);
        let sugestoes = caixa(Orientation::Vertical, 12, "");
        sugestoes.set_margin_top(14);
        painel.append(&sugestoes);
        let espaco_v = caixa(Orientation::Vertical, 0, "");
        espaco_v.set_vexpand(true);
        painel.append(&espaco_v);
        let nota = rotulo(tr("Routines keep the agents' permissions and limits and never break focus mode."), "central-agente-sub");
        nota.set_xalign(0.0);
        nota.set_wrap(true);
        painel.append(&nota);
        raiz.append(&painel);

        let p = Rc::new(PaginaRotinas {
            raiz, shell: Rc::downgrade(shell), lista, nova: nova.clone(), form, nome, pedido, quando, hora, espaco, form_erro, proximas, sugestoes, preenchendo: Cell::new(false),
            flows, open_flow: RefCell::default(),
        });
        let w = Rc::downgrade(&p);
        new_flow.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.create_flow();
            }
        });
        let w = Rc::downgrade(&p);
        nova.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let abrir = !p.form.reveals_child();
                p.form.set_reveal_child(abrir);
                if abrir {
                    p.form_erro.set_visible(false);
                }
            }
        });
        // the name field takes the keyboard once the form shows (before, it cannot: what is typed would go to the
        // button)
        let w = Rc::downgrade(&p);
        p.form.connect_child_revealed_notify(move |f| {
            if let Some(p) = w.upgrade().filter(|_| f.is_child_revealed()) {
                p.nome.grab_focus();
                crate::central::posicao_debug("rotina nome", p.nome.upcast_ref());
                // (for the session test: each field is clicked, the typing does not ride on Tab's order)
                crate::central::posicao_debug("rotina pedido", p.pedido.upcast_ref());
                crate::central::posicao_debug("rotina hora", p.hora.upcast_ref());
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug rotinas: formulário aberto");
                }
            }
        });
        let w = Rc::downgrade(&p);
        cancelar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.form.set_reveal_child(false);
            }
        });
        let w = Rc::downgrade(&p);
        p.quando.connect_selected_notify(move |d| {
            if let Some(p) = w.upgrade() {
                let i = d.selected() as usize;
                let n = quandos().len();
                p.hora.set_visible(i < n - 2);
                p.espaco.set_visible(i == n - 1);
            }
        });
        for e in [&p.nome, &p.pedido, &p.hora, &p.espaco] {
            let w = Rc::downgrade(&p);
            e.connect_activate(move |_| {
                if let Some(p) = w.upgrade() {
                    p.criar();
                }
            });
        }
        let w = Rc::downgrade(&p);
        criar.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.criar();
            }
        });
        // the scheduler says when a run starts or ends
        if let Some(a) = shell.agendador() {
            let w = Rc::downgrade(&p);
            // (while this page lives: a page built again — the Central built anew — added one more each time)
            a.ouvir_enquanto(move || {
                let Some(p) = w.upgrade() else { return false };
                p.render();
                true
            });
        }
        p
    }

    fn criar(self: &Rc<Self>) {
        let nome = self.nome.text().trim().to_string();
        let pedido = self.pedido.text().trim().to_string();
        let erro = |t: &str| {
            self.form_erro.set_text(t);
            self.form_erro.set_visible(true);
        };
        if nome.is_empty() || pedido.is_empty() {
            return erro(tr("Give the routine a name and say what the agent does."));
        }
        let qs = quandos();
        let i = (self.quando.selected() as usize).min(qs.len() - 1);
        let gatilho = match &qs[i].1 {
            Some(dias) => match hora_valida(&self.hora.text()) {
                Some((h, m)) => Gatilho::Horario { dias: dias.clone(), hora: h, minuto: m },
                None => return erro(tr("The time is written like 18:00 or 18h.")),
            },
            None if i == qs.len() - 2 => Gatilho::Usb,
            None => {
                let e = self.espaco.text().trim().to_string();
                if e.is_empty() {
                    return erro(tr("Say which space (its name)."));
                }
                Gatilho::Espaco(e)
            }
        };
        let mut r = Rotinas::carregar();
        r.adicionar(&nome, Acao::Pedido(pedido), gatilho);
        r.salvar();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug rotinas: criada {nome}");
        }
        self.nome.set_text("");
        self.pedido.set_text("");
        self.form.set_reveal_child(false);
        self.render();
    }

    /// What opens a flow in the builder (the center sets it).
    pub fn ao_abrir_fluxo(&self, f: impl Fn(&str) + 'static) {
        *self.open_flow.borrow_mut() = Some(Rc::new(f));
    }

    fn open(&self, id: &str) {
        let f = self.open_flow.borrow().clone();
        if let Some(f) = f {
            f(id);
        }
    }

    /// "Novo fluxo": a flow with a first message block, opened in the builder.
    fn create_flow(self: &Rc<Self>) {
        let flow = serde_json::json!({"nodes": [{"id": "message_1", "position": {"x": 40, "y": 60}, "type": "message", "data": {"content": tr("Hello! This flow is new: drag blocks onto the canvas.")}}], "edges": []});
        let w = Rc::downgrade(self);
        crate::fluxos::call("flows.save", serde_json::json!({"name": tr("New flow"), "flow": flow}), move |r| {
            if let (Some(p), Ok(v)) = (w.upgrade(), r) {
                if let Some(id) = v["id"].as_str() {
                    p.open(id);
                }
            }
        });
    }

    /// The flows of kalango-fluxos (each with its trigger, its last run and a switch), above the routines.
    fn render_flows(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::fluxos::call("flows.list", serde_json::json!({}), move |r| {
            let Some(p) = w.upgrade() else { return };
            let flows = match r {
                Ok(v) => v.as_array().cloned().unwrap_or_default(),
                Err(_) => {
                    while let Some(x) = p.flows.first_child() {
                        p.flows.remove(&x);
                    }
                    return;
                }
            };
            let w = Rc::downgrade(&p);
            crate::fluxos::call("triggers.list", serde_json::json!({}), move |t| {
                let Some(p) = w.upgrade() else { return };
                let triggers = t.unwrap_or_default().as_array().cloned().unwrap_or_default();
                let w = Rc::downgrade(&p);
                crate::fluxos::call("runs.list", serde_json::json!({"limit": 200}), move |r| {
                    let Some(p) = w.upgrade() else { return };
                    let runs = r.unwrap_or_default().as_array().cloned().unwrap_or_default();
                    p.fill_flows(&flows, &triggers, &runs);
                });
            });
        });
    }

    fn fill_flows(self: &Rc<Self>, flows: &[serde_json::Value], triggers: &[serde_json::Value], runs: &[serde_json::Value]) {
        while let Some(x) = self.flows.first_child() {
            self.flows.remove(&x);
        }
        if flows.is_empty() {
            return;
        }
        let cab = rotulo(tr("FLOWS"), "central-painel-agente");
        cab.set_xalign(0.0);
        self.flows.append(&cab);
        for f in flows {
            let id = f["id"].as_str().unwrap_or("").to_string();
            let on = f["enabled"].as_bool().unwrap_or(true);
            let card = caixa(Orientation::Horizontal, 14, "rotinas-item");
            card.add_css_class("fluxo-item");
            if !on {
                card.add_css_class("pausada");
            }
            let ic = caixa(Orientation::Horizontal, 0, "rotinas-icone");
            ic.set_size_request(36, 36);
            ic.set_valign(Align::Center);
            ic.set_hexpand(false);
            let g = ui_icon("raio", "#C9CAD1", 16);
            g.set_hexpand(true);
            g.set_halign(Align::Center);
            ic.append(&g);
            card.append(&ic);
            let col = caixa(Orientation::Vertical, 4, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let l1 = caixa(Orientation::Horizontal, 8, "");
            l1.append(&rotulo(f["name"].as_str().unwrap_or(""), "rotinas-nome"));
            let trig = triggers.iter().find(|t| t["flow_id"] == serde_json::json!(id)).map(|t| crate::fluxo_modelo::trigger_summary(&t["config"])).unwrap_or_else(|| tr("by hand").to_string());
            let chip = rotulo(&trig, "rotinas-chip");
            chip.set_valign(Align::Center);
            l1.append(&chip);
            col.append(&l1);
            let l2 = caixa(Orientation::Horizontal, 0, "");
            l2.append(&rotulo(&format!("{}  ·  {}  ·  ", tr("Flow"), if on { tr("active") } else { tr("paused") }), "central-agente-sub"));
            let last = runs.iter().find(|r| r["flow_id"] == serde_json::json!(id));
            let (t, class) = match last {
                Some(r) => {
                    let (st, class) = crate::fluxo_construtor::run_status_text(r["status"].as_str().unwrap_or(""));
                    let st = if class == "ok" { "ok".to_string() } else { st };
                    (format!("{}: {} · {st}", tr("last"), crate::fluxo_construtor::when_text(r["created_at"].as_i64().unwrap_or(0))), class)
                }
                None => (tr("never ran").to_string(), ""),
            };
            let u = rotulo(&t, "rotinas-ultima");
            u.add_css_class(match class {
                "ok" => "ok",
                "falhou" => "falhou",
                "aguarda" => "aguarda",
                _ => "central-agente-sub",
            });
            u.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l2.append(&u);
            col.append(&l2);
            card.append(&col);
            let edit = gtk::Button::new();
            edit.add_css_class("central-botao");
            edit.add_css_class("ao-lado");
            let ec = caixa(Orientation::Horizontal, 6, "");
            ec.append(&ui_icon("lapis", "#E2E2E6", 14));
            ec.append(&rotulo(tr("Edit flow"), ""));
            edit.set_child(Some(&ec));
            edit.set_valign(Align::Center);
            let (w, id2) = (Rc::downgrade(self), id.clone());
            edit.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.open(&id2);
                }
            });
            card.append(&edit);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_active(on);
            sw.set_valign(Align::Center);
            let (w, id2) = (Rc::downgrade(self), id.clone());
            sw.connect_state_set(move |_, v| {
                let w = w.clone();
                crate::fluxos::call("flows.enable", serde_json::json!({"id": id2, "on": v}), move |_| {
                    if let Some(p) = w.upgrade() {
                        p.render_flows();
                    }
                });
                glib::Propagation::Proceed
            });
            card.append(&sw);
            // a click on the card (not on its switch or button) opens it too
            let clique = gtk::GestureClick::new();
            let (w, id2) = (Rc::downgrade(self), id.clone());
            clique.connect_released(move |g, _, x, y| {
                if g.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT)).is_some_and(|p| p.is::<gtk::Switch>() || p.ancestor(gtk::Button::static_type()).is_some() || p.ancestor(gtk::Switch::static_type()).is_some()) {
                    return;
                }
                if let Some(p) = w.upgrade() {
                    p.open(&id2);
                }
            });
            card.add_controller(clique);
            self.flows.append(&card);
            crate::central::posicao_debug(&format!("fluxo {}", f["name"].as_str().unwrap_or("")), edit.upcast_ref());
        }
        let cab = rotulo(tr("SIMPLE ROUTINES"), "central-painel-agente");
        cab.set_xalign(0.0);
        cab.set_margin_top(10);
        self.flows.append(&cab);
    }

    pub fn render(self: &Rc<Self>) {
        self.render_flows();
        let Some(shell) = self.shell.upgrade() else { return };
        let r = Rotinas::carregar();
        let rodando: Vec<u64> = shell.agendador().map(|a| a.rodando.borrow().keys().copied().collect()).unwrap_or_default();
        let agora = glib::DateTime::now_local().ok();
        self.preenchendo.set(true);
        while let Some(x) = self.lista.first_child() {
            self.lista.remove(&x);
        }
        if r.lista.is_empty() {
            let v = rotulo(tr("No routine yet. Create one, or accept a suggestion on the right."), "central-sub");
            v.set_xalign(0.0);
            v.set_wrap(true);
            self.lista.append(&v);
        }
        for x in &r.lista {
            let card = caixa(Orientation::Horizontal, 14, "rotinas-item");
            if !x.ativa {
                card.add_css_class("pausada");
            }
            let ic = caixa(Orientation::Horizontal, 0, "rotinas-icone");
            ic.set_size_request(36, 36);
            ic.set_valign(Align::Center);
            ic.set_hexpand(false);
            let glifo = match &x.gatilho {
                Gatilho::Horario { hora, .. } if *hora >= 20 || *hora < 6 => "lua",
                Gatilho::Horario { .. } => "relogio",
                Gatilho::Usb => "baixar",
                Gatilho::Espaco(_) => "layout",
                Gatilho::Manual => "spark",
            };
            let g = ui_icon(glifo, "#C9CAD1", 16);
            g.set_hexpand(true);
            g.set_halign(Align::Center);
            ic.append(&g);
            card.append(&ic);
            let col = caixa(Orientation::Vertical, 4, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let l1 = caixa(Orientation::Horizontal, 8, "");
            l1.append(&rotulo(&x.nome, "rotinas-nome"));
            let chip = rotulo(&x.gatilho.texto(), "rotinas-chip");
            chip.set_valign(Align::Center);
            l1.append(&chip);
            col.append(&l1);
            let l2 = caixa(Orientation::Horizontal, 0, "");
            let quem = match x.acao { Acao::Pedido(_) => tr("Agent"), Acao::Abrir(_) => tr("System") };
            let estado = if rodando.contains(&x.id) {
                tr("running now").to_string()
            } else if !x.ativa {
                tr("paused").to_string()
            } else if let Some(p) = agora.as_ref().and_then(|a| x.proxima(a)) {
                trf("next: {when}", &[("when", quando_curto(&p))])
            } else {
                tr("waiting for the trigger").to_string()
            };
            l2.append(&rotulo(&format!("{quem}  ·  {estado}  ·  "), "central-agente-sub"));
            let (ultima, classe) = match &x.ultima {
                Some((_, e, resumo)) if e == "ok" => (if resumo.is_empty() { tr("last: ok").to_string() } else { trf("last: ok · {what}", &[("what", resumo.clone())]) }, "ok"),
                Some((_, _, resumo)) => (trf("last: failed · {why}", &[("why", resumo.clone())]), "falhou"),
                None => (tr("never ran").to_string(), ""),
            };
            let u = rotulo(&ultima, "rotinas-ultima");
            if !classe.is_empty() {
                u.add_css_class(classe);
            } else {
                u.add_css_class("central-agente-sub");
            }
            u.set_ellipsize(gtk::pango::EllipsizeMode::End);
            u.set_max_width_chars(60);
            l2.append(&u);
            col.append(&l2);
            card.append(&col);
            let sw = gtk::Switch::new();
            sw.add_css_class("central-switch");
            sw.set_active(x.ativa);
            sw.set_valign(Align::Center);
            let (w, id) = (Rc::downgrade(self), x.id);
            sw.connect_state_set(move |_, on| {
                if let Some(p) = w.upgrade().filter(|p| !p.preenchendo.get()) {
                    let mut r = Rotinas::carregar();
                    if let Some(x) = r.lista.iter_mut().find(|x| x.id == id) {
                        x.ativa = on;
                    }
                    r.salvar();
                    p.render();
                }
                glib::Propagation::Proceed
            });
            card.append(&sw);
            // a click: run it now, or delete it
            let menu = gtk::Popover::new();
            menu.set_parent(&card);
            menu.set_has_arrow(false);
            let mc = caixa(Orientation::Vertical, 2, "");
            let rodar = gtk::Button::with_label(tr("Run now"));
            rodar.add_css_class("central-menu-item");
            let apagar = gtk::Button::with_label(tr("Delete routine"));
            apagar.add_css_class("central-menu-item");
            mc.append(&rodar);
            mc.append(&apagar);
            menu.set_child(Some(&mc));
            let (sh, m2) = (self.shell.clone(), menu.downgrade());
            rodar.connect_clicked(move |_| {
                if let Some(m) = m2.upgrade() {
                    m.popdown();
                }
                if let Some(a) = sh.upgrade().and_then(|s| s.agendador()) {
                    a.disparar(id, "");
                }
            });
            let (w, m2) = (Rc::downgrade(self), menu.downgrade());
            apagar.connect_clicked(move |_| {
                if let Some(m) = m2.upgrade() {
                    m.popdown();
                }
                let mut r = Rotinas::carregar();
                r.lista.retain(|x| x.id != id);
                r.salvar();
                if let Some(p) = w.upgrade() {
                    glib::idle_add_local_once(move || p.render());
                }
            });
            let clique = gtk::GestureClick::new();
            let m2 = menu.downgrade();
            clique.connect_released(move |g, _, x, y| {
                // (not on the switch)
                if g.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT)).is_some_and(|p| p.is::<gtk::Switch>() || p.ancestor(gtk::Switch::static_type()).is_some()) {
                    return;
                }
                if let Some(m) = m2.upgrade() {
                    m.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    m.popup();
                }
            });
            card.add_controller(clique);
            let mw = menu.downgrade();
            card.connect_destroy(move |_| {
                if let Some(m) = mw.upgrade() {
                    m.unparent();
                }
            });
            self.lista.append(&card);
        }

        // the next scheduled runs
        while let Some(x) = self.proximas.first_child() {
            self.proximas.remove(&x);
        }
        let mut prox: Vec<(glib::DateTime, String, bool)> = agora.as_ref().map(|a| r.lista.iter().filter_map(|x| {
            let mut y = x.clone();
            y.ativa = true;
            y.proxima(a).map(|p| (p, x.nome.clone(), x.ativa))
        }).collect()).unwrap_or_default();
        prox.sort_by_key(|(p, _, _)| p.to_unix());
        if prox.is_empty() {
            let v = rotulo(tr("No routine at a set time."), "central-agente-sub");
            v.set_xalign(0.0);
            self.proximas.append(&v);
        }
        for (p, nome, ativa) in prox.into_iter().take(3) {
            let e = caixa(Orientation::Horizontal, 10, "");
            let barra = caixa(Orientation::Vertical, 0, "rotinas-barra");
            barra.set_size_request(2, -1);
            e.append(&barra);
            let c = caixa(Orientation::Vertical, 2, "");
            let q = rotulo(&quando_curto(&p).replace(", ", " · "), "rotinas-quando");
            q.set_xalign(0.0);
            c.append(&q);
            let n = rotulo(&if ativa { nome } else { format!("{nome} ({})", tr("paused")) }, "central-limite-nome");
            n.set_xalign(0.0);
            c.append(&n);
            e.append(&c);
            self.proximas.append(&e);
        }

        // patterns
        while let Some(x) = self.sugestoes.first_child() {
            self.sugestoes.remove(&x);
        }
        let sugs = crate::rotinas::sugestoes(&shell, &r);
        if sugs.is_empty() {
            let v = rotulo(tr("Nothing noticed yet: suggestions come from the apps you open at regular times or together."), "central-agente-sub");
            v.set_xalign(0.0);
            v.set_wrap(true);
            self.sugestoes.append(&v);
        }
        for s in sugs.into_iter().take(3) {
            let c = caixa(Orientation::Vertical, 4, "rotinas-sugestao");
            let t = rotulo(&s.titulo, "rotinas-sugestao-titulo");
            t.set_xalign(0.0);
            t.set_wrap(true);
            c.append(&t);
            let d = rotulo(&s.texto, "central-agente-sub");
            d.set_xalign(0.0);
            d.set_wrap(true);
            c.append(&d);
            let b = caixa(Orientation::Horizontal, 6, "");
            b.set_margin_top(6);
            let criar = gtk::Button::with_label(tr("Create routine"));
            criar.add_css_class("rotinas-sugestao-criar");
            let nao = gtk::Button::with_label(tr("Do not suggest again"));
            nao.add_css_class("rotinas-texto");
            b.append(&criar);
            b.append(&nao);
            c.append(&b);
            let w = Rc::downgrade(self);
            let (nome, acao, gatilho) = (s.nome.clone(), s.acao.clone(), s.gatilho.clone());
            criar.connect_clicked(move |_| {
                let mut r = Rotinas::carregar();
                r.adicionar(&nome, acao.clone(), gatilho.clone());
                r.salvar();
                if let Some(p) = w.upgrade() {
                    glib::idle_add_local_once(move || p.render());
                }
            });
            let (w, chave) = (Rc::downgrade(self), s.chave.clone());
            nao.connect_clicked(move |_| {
                let mut r = Rotinas::carregar();
                if !r.dispensadas.contains(&chave) {
                    r.dispensadas.push(chave.clone());
                }
                r.salvar();
                if let Some(p) = w.upgrade() {
                    glib::idle_add_local_once(move || p.render());
                }
            });
            self.sugestoes.append(&c);
        }
        self.preenchendo.set(false);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: rotinas ({} rotinas, {} rodando)", r.lista.len(), rodando.len());
        }
    }

    /// Where "Nova rotina" is, for the session test (once the page shows: a hidden page has no place yet).
    pub fn posicoes_debug(&self) {
        crate::central::posicao_debug("nova rotina", self.nova.upcast_ref());
    }
}

/// "hoje, 23:00", "amanhã, 09:00", "sex, 18:00".
fn quando_curto(p: &glib::DateTime) -> String {
    let h = p.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    let hoje = glib::DateTime::now_local().map(|d| d.day_of_year()).unwrap_or(0);
    let dia = match p.day_of_year() - hoje {
        0 => tr("today").to_string(),
        1 => tr("tomorrow").to_string(),
        _ => p.format("%a").map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default(),
    };
    format!("{dia}, {h}")
}
