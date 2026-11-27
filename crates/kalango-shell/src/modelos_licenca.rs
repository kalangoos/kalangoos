//! The licence of a model, before it downloads (design "Licença do modelo", kalangoos/design page 35): a dialog "Baixar
//! {modelo}" with the model, the licence card (summary in plain language and "Ler a licença completa"), the required
//! "Li e aceito" box, and "Baixar" only after it. "Cancelar" is "Pular: baixar depois". The acceptance is recorded per model,
//! licence and licence version in `~/.config/kalango/modelos-licencas.json`; the download queue refuses without it
//! whatever asked (`kalango-modelos::licenca`). Settings shows "Licença aceita em … · ver" and the way to revoke it.

use crate::app_store_data::{self as data, ModelEntry};
use crate::conversa::ui_icon;
use crate::fluxos_estados::action_button;
use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_modelos::licenca::{self, licenca_do_modelo, textos, textos::Marca, Aceites, Licenca};

fn rotulo(t: &str, class: &str) -> gtk::Label {
    let l = crate::superficies::rotulo(t, class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

/// Does downloading `m` need an acceptance the person has not given?
pub fn precisa(m: &ModelEntry) -> bool {
    licenca_do_modelo(&m.id).is_some_and(|l| l.exige_aceite) && licenca::falta_aceite(&licenca::aceites_path(), &m.id)
}

/// The full text (shipped) or the official link, for the reader.
fn texto_para_ler(l: &Licenca) -> String {
    licenca::texto_completo(l).unwrap_or_else(|| textos::so_o_link(l))
}

fn janela(from: &gtk::Widget, titulo: &str, largura: i32) -> gtk::Window {
    let parent = from.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    // (the width follows the window it opens over: a narrow window gets a narrow dialog, not a fixed one)
    let largura = parent.as_ref().map(|p| (p.width() - 48).clamp(300, largura)).unwrap_or(largura);
    let win = gtk::Window::builder().modal(true).title(titulo).default_width(largura).resizable(false).build();
    win.add_css_class("mf-dialogo");
    if let Some(p) = &parent {
        win.set_transient_for(Some(p));
    }
    win
}

fn cabecalho(titulo: &str, sub: &str) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 2, "");
    c.append(&rotulo(titulo, "mf-titulo"));
    if !sub.is_empty() {
        c.append(&rotulo(sub, "mf-sub"));
    }
    c
}

/// The sheet with the licence's full text, as it is: "Copiar texto" and "Fechar".
pub fn folha(from: &gtk::Widget, l: &'static Licenca) {
    let win = janela(from, l.nome, 640);
    let col = caixa(Orientation::Vertical, 12, "mf-dialogo-corpo");
    col.append(&cabecalho(l.nome, &textos::folha_subtitulo(l)));
    col.append(&rotulo(l.url, "mf-sub"));
    let tv = gtk::TextView::new();
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_wrap_mode(gtk::WrapMode::Word);
    tv.set_monospace(true);
    let texto = texto_para_ler(l);
    tv.buffer().set_text(&texto);
    let rolagem = gtk::ScrolledWindow::builder().min_content_height(280).max_content_height(380).child(&tv).vexpand(true).build();
    col.append(&rolagem);
    let rodape = caixa(Orientation::Horizontal, 10, "");
    rodape.set_halign(Align::End);
    let copiar = action_button(textos::copiar(), "", "");
    let fechar = action_button(tr("Close"), "", "principal");
    rodape.append(&copiar);
    rodape.append(&fechar);
    col.append(&rodape);
    win.set_child(Some(&col));
    let from2 = win.clone();
    copiar.connect_clicked(move |_| from2.clipboard().set_text(&texto));
    let w = win.clone();
    fechar.connect_clicked(move |_| w.close());
    win.present();
}

/// The licence card: heading, plain-language lines, "Ler a licença completa" and the original text's version.
fn cartao(l: &'static Licenca) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 8, "mf-licenca");
    let topo = caixa(Orientation::Horizontal, 8, "");
    topo.append(&ui_icon("escudo", "#E5C98C", 16));
    topo.append(&cabecalho(&textos::titulo(l), textos::subtitulo()));
    c.append(&topo);
    for (marca, texto) in textos::topicos(l) {
        let (g, cor) = match marca {
            Marca::Ok => ("check", "#8ED2C3"),
            Marca::Aviso => ("alerta", "#E5C98C"),
            Marca::Info => ("info", "#8E8F99"),
        };
        let linha = caixa(Orientation::Horizontal, 8, "");
        linha.append(&ui_icon(g, cor, 13));
        linha.append(&rotulo(texto, "mf-sub"));
        c.append(&linha);
    }
    let ler = caixa(Orientation::Horizontal, 10, "");
    let b = action_button(textos::ler(), "", "");
    b.connect_clicked(move |b| folha(b.upcast_ref(), l));
    ler.append(&b);
    ler.append(&rotulo(&textos::original(l), "mf-sub"));
    c.append(&ler);
    c
}

/// Asks the person to accept the licence of `m`; `depois` runs once it is accepted (the download starts).
pub fn pedir(from: &gtk::Widget, m: &ModelEntry, depois: impl Fn() + 'static) {
    let Some(l) = licenca_do_modelo(&m.id) else { return depois() };
    let win = janela(from, &trf("Download {model}", &[("model", m.nome.clone())]), 620);
    let col = caixa(Orientation::Vertical, 12, "mf-dialogo-corpo");
    let fonte = match crate::modelos_fonte::config() {
        Ok(c) if c.fonte != kalango_modelos::config::SourceKind::Padrao => crate::modelos_fonte::source_name(&c),
        _ => tr("KalangoOS public catalog"),
    };
    col.append(&cabecalho(&trf("Download {model}", &[("model", m.nome.clone())]), &trf("Source: {source} · checked with SHA-256", &[("source", fonte.to_string())])));
    // the model
    let modelo = caixa(Orientation::Horizontal, 10, "mf-opcao");
    modelo.append(&ui_icon(crate::modelos_fonte::kind_glyph(&m.tipo), "#8E8F99", 16));
    let nomes = caixa(Orientation::Vertical, 1, "");
    nomes.set_hexpand(true);
    nomes.append(&rotulo(&m.nome, "mf-nome"));
    nomes.append(&rotulo(&format!("{} · {}", m.descricao, l.nome), "mf-sub"));
    modelo.append(&nomes);
    modelo.append(&rotulo(&data::gb(m.bytes() as f64), "mf-nome"));
    col.append(&modelo);
    col.append(&cartao(l));
    // the required box
    let aceito = gtk::CheckButton::new();
    let opcao = caixa(Orientation::Horizontal, 10, "mf-opcao");
    opcao.append(&aceito);
    let tx = caixa(Orientation::Vertical, 1, "");
    tx.append(&rotulo(&textos::aceito(l), "mf-nome"));
    tx.append(&rotulo(&textos::aceito_detalhe(l), "mf-sub"));
    opcao.append(&tx);
    col.append(&opcao);
    let rodape = caixa(Orientation::Horizontal, 10, "");
    let nota = rotulo(textos::rodape_sem_modelo(), "mf-sub");
    nota.set_hexpand(true);
    rodape.append(&nota);
    let pular = action_button(tr("Cancel"), "", "");
    pular.set_tooltip_text(Some(textos::pular()));
    let baixar = action_button(&trf("Download {size}", &[("size", data::gb(m.bytes() as f64))]), "baixar", "principal");
    baixar.set_sensitive(false);
    rodape.append(&pular);
    rodape.append(&baixar);
    col.append(&rodape);
    win.set_child(Some(&col));
    let b2 = baixar.clone();
    aceito.connect_toggled(move |c| b2.set_sensitive(c.is_active()));
    let w = win.clone();
    pular.connect_clicked(move |_| w.close());
    let (w, id) = (win.clone(), m.id.clone());
    baixar.connect_clicked(move |_| {
        if licenca::aceitar_em(&licenca::aceites_path(), &id, &licenca::hoje()).is_ok() {
            depois();
        }
        w.close();
    });
    win.present();
}

/// The accepted licence of a model (Settings › "ver"): the acceptance, the full text, and how to revoke it.
pub fn mostrar(from: &gtk::Widget, m: &ModelEntry) {
    let Some(l) = licenca_do_modelo(&m.id) else { return };
    let registro = Aceites::carregar(&licenca::aceites_path()).registro(&m.id).cloned();
    let win = janela(from, &m.nome, 620);
    let col = caixa(Orientation::Vertical, 12, "mf-dialogo-corpo");
    col.append(&cabecalho(&m.nome, &m.descricao));
    // what the model is on this computer (installed and verified, or not)
    let v = crate::modelos_fonte::view_of(m);
    let estado = caixa(Orientation::Horizontal, 10, "mf-opcao");
    estado.append(&ui_icon(if v.detail_tone == "ok" { "check" } else { "info" }, "#8ED2C3", 16));
    let et = caixa(Orientation::Vertical, 1, "");
    et.append(&rotulo(&v.chip.0, "mf-nome"));
    et.append(&rotulo(&v.detail, "mf-sub"));
    estado.append(&et);
    col.append(&estado);
    let aceite = caixa(Orientation::Horizontal, 10, "mf-opcao");
    aceite.append(&ui_icon("escudo", "#8ED2C3", 16));
    let tx = caixa(Orientation::Vertical, 1, "");
    tx.set_hexpand(true);
    tx.append(&rotulo(&textos::aceita_em(registro.as_ref().map(|r| r.data.as_str()).unwrap_or("")), "mf-nome"));
    tx.append(&rotulo(&textos::aceita_detalhe(l), "mf-sub"));
    aceite.append(&tx);
    let ver = action_button(textos::ver_licenca(), "", "");
    ver.connect_clicked(move |b| folha(b.upcast_ref(), l));
    aceite.append(&ver);
    col.append(&aceite);
    // revoking: the models under this licence are removed (and the acceptance is dropped: it is asked again)
    let revogar = caixa(Orientation::Vertical, 6, "mf-licenca");
    revogar.add_css_class("ruim");
    revogar.append(&rotulo(textos::revogar_titulo(), "mf-nome"));
    revogar.append(&rotulo(&textos::revogar_texto(l), "mf-sub"));
    let botao = action_button(&textos::revogar_botao(l), "lixeira", "");
    botao.set_halign(Align::Start);
    revogar.append(&botao);
    col.append(&revogar);
    let fechar = action_button(tr("Close"), "", "principal");
    fechar.set_halign(Align::End);
    col.append(&fechar);
    win.set_child(Some(&col));
    let w = win.clone();
    fechar.connect_clicked(move |_| w.close());
    let (w, id) = (win.clone(), m.id.clone());
    botao.connect_clicked(move |_| {
        revogar_modelos(&id);
        w.close();
    });
    win.present();
}

/// Removes every catalog model under the same licence as `id` (the ones in the person's folder) and revokes the acceptance.
pub fn revogar_modelos(id: &str) {
    let Some(l) = licenca_do_modelo(id) else { return };
    for m in data::model_catalog().iter().filter(|m| licenca_do_modelo(&m.id).is_some_and(|x| x.id == l.id)) {
        crate::modelos_fonte::queue().remove(&m.id);
    }
    let _ = licenca::revogar_em(&licenca::aceites_path(), id);
}

/// Settings: "Licença aceita em 2026-11-24 · ver", for a model whose licence was accepted (nothing for the others).
pub fn linha_aceite(m: &ModelEntry) -> Option<gtk::Box> {
    let a = Aceites::carregar(&licenca::aceites_path());
    let registro = a.registro(&m.id)?;
    let linha = caixa(Orientation::Horizontal, 6, "");
    linha.append(&rotulo(&textos::aceita_em(&registro.data), "mf-sub"));
    let ver = action_button(textos::ver(), "", "");
    ver.add_css_class("flat");
    let m = m.clone();
    ver.connect_clicked(move |b| mostrar(b.upcast_ref(), &m));
    linha.append(&ver);
    Some(linha)
}
