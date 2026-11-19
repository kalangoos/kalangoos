//! The shortcuts sheet (design "Folha de atalhos"; Super+/): every shortcut as the person has it now — the system's
//! (with their changes from Settings › Input) and their own made by request — in five groups over the dimmed screen.
//! Typing filters it live; "Não achou?" goes to where the agent makes a shortcut from a request. Esc or Super+/ close.
//!
//! (The design opened it by holding Super; holding Super is the voice mode, so the sheet has its own keys.)

use crate::shortcuts::{self, Shortcut};
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr};
use kalango_movimento::Animavel;
use std::cell::RefCell;
use std::rc::Rc;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The sheet's groups, and which shortcuts (by id) go in each; the ones the registry does not hold are the
/// compositor's own gestures and fixed keys.
const GROUPS: &[(&str, &[&str])] = &[
    (n_("SYSTEM"), &["barra-do-agente", "@busca", "@voz", "bloquear", "energia", "terminal"]),
    (n_("WINDOWS"), &["encaixar", "maximizar", "alternar", "fechar", "layouts", "tela-cheia", "flutuar"]),
    (n_("SPACES AND VIEW"), &["visao", "espaco", "@espacos", "area", "@barras"]),
    (n_("CAPTURE AND PASTE"), &["captura", "gravar", "transferencia", "emoji"]),
    (n_("PRIVACY AND ACCESS"), &["privacidade", "descrever", "ditado"]),
];

/// One line: its name and its keys (as written on the caps).
#[derive(Clone)]
struct Line {
    name: String,
    caps: Vec<String>,
}

fn line_of(id: &str, changes: &std::collections::BTreeMap<String, String>) -> Option<Line> {
    let s = |id: &str| -> Option<&'static Shortcut> { shortcuts::SHORTCUTS.iter().find(|s| s.id == id) };
    Some(match id {
        "@busca" => Line { name: tr("Universal search").into(), caps: vec!["Super".into()] },
        "@voz" => Line { name: tr("Voice mode").into(), caps: vec![tr("hold Super").into()] },
        "@espacos" => Line { name: tr("Go to space 1–4").into(), caps: vec!["Super".into(), "1…4".into()] },
        // the side bars: Today, Notices, Controls, Agent — their letters on one line when they share Super
        "@barras" => {
            let ks: Vec<String> = ["hoje", "avisos", "controles", "agente"].iter().filter_map(|i| s(i)).map(|x| shortcuts::keys_now(x, changes)).collect();
            if ks.iter().all(|k| k.starts_with("Mod+") && k.matches('+').count() == 1) {
                let mut caps = vec!["Super".to_string()];
                caps.extend(ks.iter().map(|k| k.trim_start_matches("Mod+").to_uppercase()));
                Line { name: tr("Side bars").into(), caps }
            } else {
                return None;
            }
        }
        _ => {
            let sc = s(id)?;
            let keys = shortcuts::keys_now(sc, changes);
            if keys.is_empty() {
                return None;
            }
            Line { name: kalango_i18n::tr(sc.name).to_string(), caps: shortcuts::caps_of(&keys) }
        }
    })
}

/// The groups with their lines now (and the person's own shortcuts, when there are any).
fn groups_now() -> Vec<(String, Vec<Line>)> {
    let changes = shortcuts::read_changes();
    let mut out: Vec<(String, Vec<Line>)> = GROUPS.iter().map(|(g, ids)| (kalango_i18n::tr(g).to_string(), ids.iter().filter_map(|i| line_of(i, &changes)).collect())).collect();
    // the side bars are shown together; the ones without a line of their own are not lost
    if !out.iter().any(|(_, ls)| ls.iter().any(|l| l.name == tr("Side bars"))) {
        if let Some((_, ls)) = out.get_mut(2) {
            for id in ["hoje", "avisos", "controles", "agente"] {
                if let Some(l) = line_of(id, &changes) {
                    ls.push(l);
                }
            }
        }
    }
    let proprios: Vec<Line> = shortcuts::read_custom().into_iter().map(|c| Line { name: c.name.clone(), caps: shortcuts::caps_of(&c.keys) }).collect();
    if !proprios.is_empty() {
        out.push((tr("YOURS").to_string(), proprios));
    }
    out
}

struct Sheet {
    window: gtk::ApplicationWindow,
    card: Animavel,
    filter: gtk::Text,
    columns: gtk::Box,
    empty: gtk::Label,
    lines: RefCell<Vec<(String, Vec<gtk::Box>, gtk::Box)>>,
}

thread_local! {
    static SHEET: RefCell<Option<Rc<Sheet>>> = const { RefCell::new(None) };
}

fn sheet() -> Option<Rc<Sheet>> {
    if let Some(s) = SHEET.with(|s| s.borrow().clone()) {
        return Some(s);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-atalhos", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    crate::superficies::take_keyboard(&window);
    window.set_exclusive_zone(-1);
    let body = caixa(Orientation::Vertical, 18, "atalhos-cartao");
    body.set_size_request(1420, -1);
    let cab = caixa(Orientation::Horizontal, 12, "");
    let titulo = rotulo(tr("Shortcuts"), "atalhos-titulo");
    titulo.set_hexpand(true);
    titulo.set_xalign(0.0);
    cab.append(&titulo);
    let campo = caixa(Orientation::Horizontal, 8, "atalhos-filtro");
    campo.set_size_request(300, -1);
    campo.append(&crate::conversa::ui_icon("busca", "#8E8F99", 14));
    let filter = gtk::Text::new();
    filter.set_hexpand(true);
    filter.set_placeholder_text(Some(tr("type to filter…")));
    campo.append(&filter);
    cab.append(&campo);
    body.append(&cab);
    let columns = caixa(Orientation::Horizontal, 28, "");
    columns.set_homogeneous(true);
    body.append(&columns);
    let empty = rotulo(tr("No shortcut has that name."), "atalhos-vazio");
    empty.set_visible(false);
    body.append(&empty);
    let rodape = caixa(Orientation::Horizontal, 8, "");
    let pedir = gtk::Button::new();
    pedir.add_css_class("atalhos-pedir");
    let pr = caixa(Orientation::Horizontal, 8, "");
    pr.append(&crate::conversa::ui_icon("spark", "#A5A1E2", 14));
    pr.append(&gtk::Label::new(Some(tr("Not here? Say what you want and the agent makes the shortcut."))));
    pedir.set_child(Some(&pr));
    pedir.connect_clicked(|_| {
        close();
        crate::configuracoes::abrir_painel("entrada");
    });
    rodape.append(&pedir);
    let esp = gtk::Box::new(Orientation::Horizontal, 0);
    esp.set_hexpand(true);
    rodape.append(&esp);
    rodape.append(&rotulo(tr("Esc or Super+/ closes"), "atalhos-dica"));
    body.append(&rodape);
    let card = Animavel::new(&body);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let s = Rc::new(Sheet { window, card, filter: filter.clone(), columns, empty, lines: RefCell::default() });
    let w = Rc::downgrade(&s);
    filter.connect_changed(move |f| {
        if let Some(s) = w.upgrade() {
            s.apply_filter(&f.text());
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    s.window.add_controller(keys);
    if debug() {
        crate::central::posicao_debug("atalhos pedir", pedir.upcast_ref());
    }
    SHEET.with(|x| *x.borrow_mut() = Some(s.clone()));
    Some(s)
}

impl Sheet {
    fn fill(&self) {
        while let Some(c) = self.columns.first_child() {
            self.columns.remove(&c);
        }
        let mut lines = Vec::new();
        let grupos = groups_now();
        // five columns (the person's own shortcuts join the last)
        let mut cols: Vec<gtk::Box> = (0..5).map(|_| caixa(Orientation::Vertical, 6, "")).collect();
        let mut total = 0;
        for (i, (nome, ls)) in grupos.iter().enumerate() {
            let col = &cols[i.min(4)];
            let g = caixa(Orientation::Vertical, 6, "");
            if i > 4 {
                g.set_margin_top(14);
            }
            let cab = rotulo(nome, "atalhos-grupo");
            cab.set_xalign(0.0);
            g.append(&cab);
            let mut linhas = Vec::new();
            for l in ls {
                let r = caixa(Orientation::Horizontal, 8, "atalhos-linha");
                let n = rotulo(&l.name, "atalhos-nome");
                n.set_xalign(0.0);
                n.set_hexpand(true);
                n.set_ellipsize(gtk::pango::EllipsizeMode::End);
                n.set_max_width_chars(1);
                r.append(&n);
                let caps = caixa(Orientation::Horizontal, 4, "");
                for c in &l.caps {
                    caps.append(&rotulo(c, "tecla"));
                }
                r.append(&caps);
                g.append(&r);
                linhas.push(r);
                total += 1;
            }
            col.append(&g);
            lines.push((nome.clone(), linhas, g));
        }
        for c in cols.drain(..) {
            self.columns.append(&c);
        }
        *self.lines.borrow_mut() = lines;
        if debug() {
            eprintln!("debug atalhos: {} atalhos em {} grupos", total, grupos.len());
        }
    }

    fn apply_filter(&self, q: &str) {
        let q = q.trim().to_lowercase();
        let mut visiveis = 0;
        for (_, rs, g) in self.lines.borrow().iter() {
            let mut algum = false;
            for r in rs {
                let texto: String = {
                    let mut t = String::new();
                    let mut c = r.first_child();
                    while let Some(w) = c {
                        if let Some(l) = w.downcast_ref::<gtk::Label>() {
                            t.push_str(&l.text());
                            t.push(' ');
                        } else if let Some(b) = w.downcast_ref::<gtk::Box>() {
                            let mut k = b.first_child();
                            while let Some(x) = k {
                                if let Some(l) = x.downcast_ref::<gtk::Label>() {
                                    t.push_str(&l.text());
                                    t.push(' ');
                                }
                                k = x.next_sibling();
                            }
                        }
                        c = w.next_sibling();
                    }
                    t.to_lowercase()
                };
                let ve = q.is_empty() || q.split_whitespace().all(|p| texto.contains(p));
                r.set_visible(ve);
                algum |= ve;
                if ve {
                    visiveis += 1;
                }
            }
            g.set_visible(algum);
        }
        self.empty.set_visible(visiveis == 0);
        if debug() {
            eprintln!("debug atalhos: filtro «{q}» mostra {visiveis}");
        }
    }
}

/// Super+/: opens the sheet (or closes it).
pub fn toggle() {
    let Some(s) = sheet() else { return };
    if s.window.is_visible() {
        close();
        return;
    }
    // the design's 1420 px, or the screen's width less its margins
    let largura = crate::superficies::tamanho_da_saida().map(|(w, _)| (w - 64).min(1420)).unwrap_or(1420);
    if let Some(c) = s.card.first_child() {
        c.set_size_request(largura, -1);
    }
    s.fill();
    s.filter.set_text("");
    s.apply_filter("");
    s.window.present();
    crate::superficies::take_keyboard(&s.window);
    s.window.add_css_class("aberto");
    s.card.entrar((0.0, 0.0), 0.98);
    s.filter.grab_focus();
    if debug() {
        eprintln!("debug atalhos: folha aberta");
    }
}

fn close() {
    let Some(s) = SHEET.with(|s| s.borrow().clone()) else { return };
    if !s.window.is_visible() {
        return;
    }
    s.window.set_keyboard_mode(KeyboardMode::None);
    s.window.remove_css_class("aberto");
    let w = s.window.downgrade();
    s.card.sair((0.0, 0.0), 0.98, move || {
        if let Some(w) = w.upgrade() {
            w.set_visible(false);
        }
    });
}
