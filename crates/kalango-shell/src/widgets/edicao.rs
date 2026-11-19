//! Editing the desktop (design "Editar widgets (galeria)"): the gallery comes in from the right (gentle spring) with
//! every widget, live, by group; a click adds one in the first free cell. "Buscar ou pedir…" filters by name, and
//! "Monte para mim" asks the local model for a whole desktop from a sentence ("quero ver meus agentes, a GPU e a
//! agenda de hoje"). At the bottom, the edit bar with "Concluir".

use super::{Area, Tam, TIPOS};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

const GRUPOS: &[(&str, &str)] = &[("todos", kalango_i18n::n_("All widgets")), ("ia", kalango_i18n::n_("AI")), ("sistema", kalango_i18n::n_("System")), ("metricas", kalango_i18n::n_("Metrics"))];

pub struct Galeria {
    pub painel: Animavel,
    pub barra: Animavel,
    area: Weak<Area>,
    busca: gtk::Entry,
    chips: gtk::Box,
    lista: gtk::Box,
    grupo: Cell<&'static str>,
    monte_estado: gtk::Label,
    gerar: gtk::Button,
    /// widget kinds whose previews are built (the gallery builds them while open)
    aberta: Cell<bool>,
    /// on the right (the design's side) or on the left, whichever covers fewer widgets
    right: Cell<bool>,
    /// put away by the person (the bar's button) or while a widget is dragged
    hidden: Cell<bool>,
    giving_way: Cell<bool>,
    toggle: gtk::Button,
    sequencia: RefCell<u32>,
}

impl Galeria {
    pub fn new(area: &Rc<Area>) -> Rc<Galeria> {
        // the panel (400 wide at the right, 12 px under the bar)
        let p = caixa(Orientation::Vertical, 14, "galeria");
        p.set_size_request(400, -1);
        let topo = caixa(Orientation::Horizontal, 0, "");
        let t = rotulo(tr("Widget gallery"), "galeria-titulo");
        t.set_xalign(0.0);
        t.set_hexpand(true);
        topo.append(&t);
        let fechar = gtk::Button::new();
        fechar.set_child(Some(&ui_icon("fechar", "#8E8F99", 16)));
        fechar.add_css_class("fantasma");
        fechar.set_tooltip_text(Some(tr("Done")));
        topo.append(&fechar);
        p.append(&topo);
        let busca_caixa = caixa(Orientation::Horizontal, 8, "galeria-busca");
        busca_caixa.append(&estrela(13, "#A5A1E2"));
        let busca = gtk::Entry::new();
        busca.set_hexpand(true);
        busca.add_css_class("galeria-busca-campo");
        busca.set_placeholder_text(Some(tr("Search or ask…")));
        busca_caixa.append(&busca);
        let atalho = rotulo("Ctrl K", "visao-atalho");
        atalho.set_valign(Align::Center);
        busca_caixa.append(&atalho);
        p.append(&busca_caixa);
        let chips = caixa(Orientation::Horizontal, 6, "");
        p.append(&chips);
        // rows: two small cards side by side (176 + 12 + 176), a wide or large one alone
        let lista = caixa(Orientation::Vertical, 12, "");
        lista.set_valign(Align::Start);
        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        rolagem.set_child(Some(&lista));
        p.append(&rolagem);
        // "Monte para mim"
        let m = caixa(Orientation::Vertical, 8, "galeria-monte");
        let mt = caixa(Orientation::Horizontal, 6, "");
        mt.append(&estrela(13, "#A5A1E2"));
        mt.append(&rotulo(tr("Build it for me"), "galeria-monte-titulo"));
        m.append(&mt);
        let exemplo = rotulo(tr("“I want to see my agents, the GPU and today's agenda”"), "desk-metrica-sub");
        exemplo.set_xalign(0.0);
        exemplo.set_wrap(true);
        m.append(&exemplo);
        let monte_estado = rotulo("", "desk-metrica-sub");
        monte_estado.set_xalign(0.0);
        monte_estado.set_wrap(true);
        monte_estado.set_visible(false);
        m.append(&monte_estado);
        let gerar = gtk::Button::with_label(tr("Generate layout"));
        gerar.add_css_class("galeria-gerar");
        m.append(&gerar);
        p.append(&m);
        let painel = Animavel::new(&p);
        painel.set_halign(Align::End);
        painel.set_valign(Align::Fill);
        painel.set_margin_end(16);
        painel.set_margin_top(12);
        painel.set_margin_bottom(18);
        painel.set_visible(false);

        // the edit bar (bottom centre)
        let b = caixa(Orientation::Horizontal, 10, "galeria-barra");
        b.append(&ui_icon("layout", "#A5A1E2", 16));
        b.append(&rotulo(tr("Editing widgets"), "galeria-barra-titulo"));
        let dica = rotulo(tr("drag to move · handle to resize"), "desk-metrica-sub");
        dica.set_hexpand(true);
        dica.set_xalign(0.0);
        b.append(&dica);
        // the gallery can go away (and come back) to reach the widgets under it
        let toggle = gtk::Button::with_label(tr("Hide gallery"));
        toggle.add_css_class("galeria-alternar");
        b.append(&toggle);
        let concluir = gtk::Button::with_label(tr("Done"));
        concluir.add_css_class("galeria-concluir");
        b.append(&concluir);
        let barra = Animavel::new(&b);
        barra.set_halign(Align::Center);
        barra.set_valign(Align::End);
        barra.set_margin_bottom(26);
        barra.set_visible(false);

        let g = Rc::new(Galeria {
            painel, barra, area: Rc::downgrade(area), busca, chips, lista, grupo: Cell::new("todos"), monte_estado, gerar: gerar.clone(),
            aberta: Cell::new(false), sequencia: RefCell::new(0), right: Cell::new(true), hidden: Cell::new(false), giving_way: Cell::new(false),
            toggle: toggle.clone(),
        });
        let w = Rc::downgrade(&g);
        toggle.connect_clicked(move |_| {
            if let Some(g) = w.upgrade() {
                g.set_hidden(!g.hidden.get());
            }
        });

        for bt in [&fechar, &concluir] {
            let a = Rc::downgrade(area);
            bt.connect_clicked(move |_| {
                if let Some(a) = a.upgrade() {
                    a.sair_da_edicao();
                }
            });
        }
        let w = Rc::downgrade(&g);
        g.busca.connect_changed(move |_| {
            if let Some(g) = w.upgrade() {
                g.preencher();
            }
        });
        // Enter: a name adds that widget; a sentence builds a desktop
        let w = Rc::downgrade(&g);
        g.busca.connect_activate(move |e| {
            let Some(g) = w.upgrade() else { return };
            let q = e.text().trim().to_string();
            if q.is_empty() {
                return;
            }
            match g.filtrados().first() {
                Some(t) if g.filtrados().len() == 1 => {
                    if let Some(a) = g.area.upgrade() {
                        a.adicionar(t.id, t.tamanhos[0]);
                    }
                }
                _ => g.monte_para_mim(&q),
            }
        });
        let w = Rc::downgrade(&g);
        gerar.connect_clicked(move |_| {
            let Some(g) = w.upgrade() else { return };
            let q = g.busca.text().trim().to_string();
            if q.is_empty() {
                g.monte_estado.set_text(tr("Write in the field above what you want to see."));
                g.monte_estado.set_visible(true);
                g.busca.grab_focus();
                return;
            }
            g.monte_para_mim(&q);
        });
        g
    }

    /// The kinds for the group and the search: the small ones first (as the design's gallery), the clock among the
    /// last of them.
    fn filtrados(&self) -> Vec<&'static super::Tipo> {
        let q = crate::indice::fold(self.busca.text().trim());
        const PRIMEIRO: &[&str] = &["privacidade", "agentes-ativos", "tokens", "memoria", "dispositivos", "modelo", "cpu", "gpu", "clima", "foco"];
        let mut v: Vec<&'static super::Tipo> = TIPOS.iter().filter(|t| self.grupo.get() == "todos" || t.grupo == self.grupo.get())
            .filter(|t| q.is_empty() || crate::indice::fold(tr(t.nome)).contains(&q) || t.id.contains(&q)).collect();
        let rank = |t: &super::Tipo| (PRIMEIRO.iter().position(|p| *p == t.id).unwrap_or(PRIMEIRO.len()), match t.tamanhos[0] { Tam::P => 0, Tam::M => 1, Tam::G => 2 });
        v.sort_by_key(|t| rank(t));
        v
    }

    fn render_chips(self: &Rc<Self>) {
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        for (id, nome) in GRUPOS {
            let b = gtk::Button::with_label(tr(nome));
            b.add_css_class("grade-chip");
            if *id == self.grupo.get() {
                b.add_css_class("ativo");
            }
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| {
                if let Some(g) = w.upgrade() {
                    g.grupo.set(id);
                    g.render_chips();
                    g.preencher();
                }
            });
            self.chips.append(&b);
        }
    }

    /// The widgets, live, in cards (a click adds one).
    fn preencher(self: &Rc<Self>) {
        while let Some(c) = self.lista.first_child() {
            self.lista.remove(&c);
        }
        let Some(area) = self.area.upgrade() else { return };
        let mut par: Option<gtk::Box> = None;
        for t in self.filtrados() {
            let tam = t.tamanhos[0];
            let (w, h) = if t.id == "relogio" { (168, 188) } else { tam.px() };
            let card = gtk::Button::new();
            card.add_css_class("galeria-cartao");
            card.set_widget_name(t.id);
            let col = caixa(Orientation::Vertical, 8, "");
            if let Some(prev) = area.widget(t.id, tam, true) {
                prev.set_can_target(false);
                prev.set_size_request(w, h);
                col.append(&prev);
            }
            let rodape = caixa(Orientation::Horizontal, 0, "");
            let n = rotulo(tr(t.nome), "desk-metrica-nome");
            n.set_xalign(0.0);
            n.set_hexpand(true);
            rodape.append(&n);
            let tams: Vec<&str> = t.tamanhos.iter().map(|s| match s { Tam::P => "P", Tam::M => "M", Tam::G => "G" }).collect();
            rodape.append(&rotulo(&tams.join(" · "), "desk-metrica-sub"));
            col.append(&rodape);
            card.set_child(Some(&col));
            let (a, id) = (self.area.clone(), t.id);
            card.connect_clicked(move |_| {
                if let Some(a) = a.upgrade() {
                    if !a.adicionar(id, tam) && std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug widgets: sem lugar para {id}");
                    }
                }
            });
            if tam == Tam::P {
                match par.take() {
                    Some(linha) => linha.append(&card),
                    None => {
                        let linha = caixa(Orientation::Horizontal, 12, "");
                        linha.append(&card);
                        self.lista.append(&linha);
                        par = Some(linha);
                    }
                }
            } else {
                par = None;
                let linha = caixa(Orientation::Horizontal, 12, "");
                linha.append(&card);
                self.lista.append(&linha);
            }
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let w = Rc::downgrade(self);
            let seq = {
                let mut s = self.sequencia.borrow_mut();
                *s += 1;
                *s
            };
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                let Some(g) = w.upgrade() else { return };
                if *g.sequencia.borrow() != seq {
                    return;
                }
                g.registrar_cartoes();
            });
        }
    }

    /// Debug: where each card is (the session test clicks one).
    fn registrar_cartoes(&self) {
        let Some(a) = self.area.upgrade() else { return };
        let mut linha = self.lista.first_child();
        while let Some(l) = linha {
            let mut c = l.first_child();
            while let Some(card) = c {
                if let Some(b) = card.compute_bounds(&a.janela) {
                    eprintln!("debug widgets: cartão {} em {} {}", card.widget_name(), (b.x() + b.width() / 2.0) as i32,
                        (f64::from(crate::superficies::BARRA_ALTURA) + f64::from(b.y() + b.height() / 2.0)) as i32);
                }
                c = card.next_sibling();
            }
            linha = l.next_sibling();
        }
    }

    /// The bar's "hide gallery" button (the session test clicks it).
    pub(crate) fn toggle_button(&self) -> gtk::Button {
        self.toggle.clone()
    }

    /// Where the panel slides from, and back to.
    fn offset(&self) -> (f64, f64) {
        (if self.right.get() { 60.0 } else { -60.0 }, 0.0)
    }

    fn place(&self) {
        let p = &self.painel;
        if self.right.get() {
            p.set_halign(Align::End);
            p.set_margin_end(16);
            p.set_margin_start(0);
        } else {
            p.set_halign(Align::Start);
            p.set_margin_start(16);
            p.set_margin_end(0);
        }
    }

    /// The person puts the gallery away (to reach the widgets under it) or brings it back.
    pub fn set_hidden(&self, hidden: bool) {
        self.hidden.set(hidden);
        self.toggle.set_label(if hidden { tr("Show gallery") } else { tr("Hide gallery") });
        self.slide(!hidden);
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug widgets: galeria {}", if hidden { "escondida" } else { "de volta" });
        }
    }

    /// While a widget is dragged the gallery steps aside, then comes back (unless the person put it away).
    pub fn give_way(&self, yes: bool) {
        if self.giving_way.replace(yes) == yes || self.hidden.get() {
            return;
        }
        self.slide(!yes);
    }

    fn slide(&self, show: bool) {
        let p = self.painel.clone();
        if show {
            p.set_visible(true);
            p.entrar(self.offset(), 1.0);
        } else {
            let p2 = p.clone();
            p.sair(self.offset(), 1.0, move || p2.set_visible(false));
        }
    }

    pub fn abrir(self: &Rc<Self>, right: bool) {
        self.right.set(right);
        self.hidden.set(false);
        self.giving_way.set(false);
        self.toggle.set_label(tr("Hide gallery"));
        self.place();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug widgets: galeria à {}", if right { "direita" } else { "esquerda" });
        }
        self.aberta.set(true);
        self.busca.set_text("");
        self.grupo.set("todos");
        self.monte_estado.set_visible(false);
        self.render_chips();
        self.preencher();
        self.painel.set_visible(true);
        self.barra.set_visible(true);
        self.painel.entrar(self.offset(), 1.0);
        self.barra.entrar((0.0, 16.0), 1.0);
    }

    pub fn fechar(&self) {
        self.aberta.set(false);
        let (p, b, l) = (self.painel.clone(), self.barra.clone(), self.lista.clone());
        self.painel.sair(self.offset(), 1.0, move || {
            p.set_visible(false);
            // the live previews end with the gallery
            while let Some(c) = l.first_child() {
                l.remove(&c);
            }
        });
        let b2 = b.clone();
        b.sair((0.0, 16.0), 1.0, move || b2.set_visible(false));
    }

    /// A whole desktop from a sentence: the local model picks from the catalogue, the shell lays it out.
    fn monte_para_mim(self: &Rc<Self>, pedido: &str) {
        let catalogo: Vec<Value> = TIPOS.iter().map(|t| json!({"id": t.id, "nome": tr(t.nome),
            "tamanhos": t.tamanhos.iter().map(|s| match s { Tam::P => "P", Tam::M => "M", Tam::G => "G" }).collect::<Vec<_>>()})).collect();
        let entrada = json!({"pedido": pedido, "widgets": catalogo}).to_string();
        self.monte_estado.set_text(tr("Building it with the local model…"));
        self.monte_estado.set_visible(true);
        self.gerar.set_sensitive(false);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let bin = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|b| b.is_file())
                .unwrap_or_else(|| "kalango-agente".into());
            let mut c = std::process::Command::new(bin).arg("widgets").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(entrada.as_bytes()).ok()?;
            serde_json::from_slice::<Value>(&c.wait_with_output().ok()?.stdout).ok()
        }, move |v: Option<Value>| {
            let Some(g) = w.upgrade() else { return };
            g.gerar.set_sensitive(true);
            let escolhidos: Vec<(String, Tam)> = v.as_ref().map(|v| v["widgets"].as_array().cloned().unwrap_or_default()).unwrap_or_default().iter()
                .filter_map(|w| {
                    let id = w["id"].as_str()?;
                    let t = TIPOS.iter().find(|t| t.id == id)?;
                    let tam = match w["tam"].as_str() { Some("M") => Tam::M, Some("G") => Tam::G, _ => Tam::P };
                    Some((id.to_string(), if t.tamanhos.contains(&tam) { tam } else { t.tamanhos[0] }))
                }).collect();
            if escolhidos.is_empty() {
                g.monte_estado.set_text(if v.as_ref().is_some_and(|v| v["gerado"] == false) {
                    tr("The local model is not available now.")
                } else {
                    tr("No widget matches what you asked. Try other words.")
                });
                return;
            }
            g.monte_estado.set_text(&trf("{n} widgets placed. Move or resize them, then “Done”.", &[("n", escolhidos.len().to_string())]));
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug widgets: monte para mim: {}", escolhidos.iter().map(|(i, t)| format!("{i} {t:?}")).collect::<Vec<_>>().join(", "));
            }
            if let Some(a) = g.area.upgrade() {
                a.aplicar(&escolhidos);
            }
        });
    }
}
