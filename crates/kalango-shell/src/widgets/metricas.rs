//! Metric widgets (design "Widgets › Métricas"): CPU, GPU and memory (S, a ring), network (M, both directions and
//! the download over the last minutes), disks (M, one bar each) and "Sistema · últimos 5 min" (L, three charts).
//! Everything comes from the sampler in dados.rs.

use super::comum::{decimal, tamanho, taxa, Anel, Grafico, Texto, ACCENT, BLUE, GIB, GOLD, TEAL};
use super::dados::{self, Amostra, HISTORICO};
use super::{quadro, Tam};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::Orientation;
use kalango_i18n::{tr, trf};

fn linha_info(esq: &str, dir: &str, classe_dir: &str) -> (gtk::Box, gtk::Label, gtk::Label) {
    let l = caixa(Orientation::Horizontal, 0, "");
    let a = rotulo(esq, "desk-metrica-nome");
    a.set_xalign(0.0);
    a.set_hexpand(true);
    l.append(&a);
    let b = rotulo(dir, classe_dir);
    b.set_xalign(1.0);
    l.append(&b);
    (l, a, b)
}

/// A ring widget: title, ring with the number in the middle, a line under it.
fn anel_widget(glifo: &str, titulo: &str, cor: &str) -> (gtk::Box, Anel, Texto, Texto) {
    let q = quadro(Tam::P.px().0, Tam::P.px().1);
    q.append(&super::comum::cabecalho(glifo, titulo).0);
    let anel = Anel::new(92, cor);
    let numero = Texto::new("desk-anel-numero");
    numero.alinhar(0.5);
    anel.centro.append(&numero.stack);
    q.append(&anel.widget);
    let sub = Texto::new("desk-metrica-sub");
    q.append(&sub.stack);
    (q, anel, numero, sub)
}

pub fn cpu() -> gtk::Widget {
    let (q, anel, numero, sub) = anel_widget("cpu", tr("CPU"), TEAL);
    dados::ouvir(&q, move |a: &Amostra| {
        anel.ir(a.cpu);
        numero.definir(&format!("{:.0}%", a.cpu * 100.0));
        let n = if a.nucleos == 1 { tr("1 core").to_string() } else { trf("{n} cores", &[("n", a.nucleos.to_string())]) };
        sub.definir(&match a.ghz {
            Some(g) => format!("{n} · {} GHz", decimal(g)),
            None => n,
        });
    });
    q.upcast()
}

pub fn gpu() -> gtk::Widget {
    let (q, anel, numero, sub) = anel_widget("monitor", tr("GPU"), ACCENT);
    dados::ouvir(&q, move |a: &Amostra| match &a.gpu {
        Some(g) => {
            anel.ir(g.uso);
            numero.definir(&format!("{:.0}%", g.uso * 100.0));
            sub.definir(&match g.temp {
                Some(t) => format!("{} · {t:.0} °C", g.nome),
                None => g.nome.clone(),
            });
        }
        None => {
            anel.ir(0.0);
            numero.definir("—");
            sub.definir(tr("No dedicated GPU"));
        }
    });
    q.upcast()
}

pub fn memoria() -> gtk::Widget {
    let (q, anel, numero, sub) = anel_widget("layout", tr("MEMORY"), BLUE);
    dados::ouvir(&q, move |a: &Amostra| {
        let (u, t) = a.mem;
        anel.ir(if t > 0.0 { u / t } else { 0.0 });
        numero.definir(&decimal(u / GIB));
        sub.definir(&trf("of {n} GB", &[("n", format!("{:.0}", t / GIB))]));
    });
    q.upcast()
}

pub fn rede() -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    let (cab, titulo) = super::comum::cabecalho("wifi", tr("NETWORK"));
    q.append(&cab);
    let valores = caixa(Orientation::Horizontal, 24, "");
    let par = |seta: &str, cor: &str| {
        let b = caixa(Orientation::Horizontal, 6, "");
        let s = rotulo(seta, "desk-rede-seta");
        s.add_css_class(cor);
        b.append(&s);
        let v = Texto::new("desk-rede-valor");
        b.append(&v.stack);
        (b, v)
    };
    let (b1, baixa) = par("↓", "teal");
    let (b2, sobe) = par("↑", "coral");
    valores.append(&b1);
    valores.append(&b2);
    q.append(&valores);
    let g = Grafico::new(344, 64, 60, TEAL);
    q.append(&g.area);
    dados::ouvir(&q, move |a: &Amostra| match &a.rede {
        Some(r) => {
            titulo.set_text(&match r.tipo.as_str() {
                "wifi" => trf("NETWORK · WI-FI {name}", &[("name", r.nome.to_uppercase())]),
                "cabo" => trf("NETWORK · CABLE {name}", &[("name", r.nome.to_uppercase())]),
                _ => tr("NETWORK").to_string(),
            });
            baixa.definir(&taxa(r.rx));
            sobe.definir(&taxa(r.tx));
            // the download of the last 2 minutes, against its own peak
            let ultimos: Vec<f64> = a.hist_rx.iter().rev().take(60).rev().copied().collect();
            let pico = ultimos.iter().copied().fold(1.0, f64::max);
            g.mostrar(&ultimos.iter().map(|v| v / pico).collect::<Vec<_>>());
        }
        None => {
            titulo.set_text(tr("NETWORK · OFFLINE"));
            baixa.definir("—");
            sobe.definir("—");
        }
    });
    q.upcast()
}

pub fn armazenamento() -> gtk::Widget {
    let (w, h) = Tam::M.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("pasta", tr("STORAGE")).0);
    let lista = caixa(Orientation::Vertical, 10, "");
    q.append(&lista);
    let ultimo: std::rc::Rc<std::cell::RefCell<Vec<(String, u64, u64)>>> = Default::default();
    dados::ouvir(&q, move |a: &Amostra| {
        let chave: Vec<(String, u64, u64)> = a.discos.iter().map(|d| (d.nome.clone(), (d.usado / 1e9) as u64, (d.total / 1e9) as u64)).collect();
        if *ultimo.borrow() == chave {
            return;
        }
        *ultimo.borrow_mut() = chave;
        while let Some(c) = lista.first_child() {
            lista.remove(&c);
        }
        for (i, d) in a.discos.iter().enumerate() {
            let bloco = caixa(Orientation::Vertical, 8, "");
            let (l, _, _) = linha_info(&d.nome, &trf("{used} of {total}", &[("used", tamanho(d.usado)), ("total", tamanho(d.total))]), "desk-metrica-sub");
            bloco.append(&l);
            let (b, v) = super::comum::barra(344, [ACCENT, GOLD, TEAL][i % 3]);
            bloco.append(&b);
            v.ir(if d.total > 0.0 { d.usado / d.total } else { 0.0 }, &b);
            lista.append(&bloco);
        }
        if a.discos.is_empty() {
            lista.append(&rotulo(tr("No disk found"), "desk-metrica-sub"));
        }
    });
    q.upcast()
}

/// "Ligado há 3 d 4 h".
fn ligado(s: u64) -> String {
    let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
    if d > 0 {
        trf("On for {d} d {h} h", &[("d", d.to_string()), ("h", h.to_string())])
    } else if h > 0 {
        trf("On for {h} h {m} min", &[("h", h.to_string()), ("m", m.to_string())])
    } else {
        trf("On for {m} min", &[("m", m.to_string())])
    }
}

pub fn sistema() -> gtk::Widget {
    let (w, h) = Tam::G.px();
    let q = quadro(w, h);
    q.append(&super::comum::cabecalho("cpu", tr("SYSTEM · LAST 5 MIN")).0);
    let serie = |nome: &str, cor: &'static str| {
        let (l, _, valor) = linha_info(nome, "", "desk-metrica-valor");
        valor.add_css_class(match cor { TEAL => "teal", ACCENT => "acento", _ => "azul" });
        let g = Grafico::new(344, 52, HISTORICO, cor);
        (l, valor, g)
    };
    let (l1, v1, g1) = serie(tr("CPU"), TEAL);
    let (l2, v2, g2) = serie(tr("GPU"), ACCENT);
    let (l3, v3, g3) = serie(tr("Memory"), BLUE);
    for (l, g) in [(&l1, &g1), (&l2, &g2), (&l3, &g3)] {
        q.append(l);
        q.append(&g.area);
    }
    let (rodape, ligado_l, temp) = linha_info("", "", "desk-metrica-sub");
    ligado_l.remove_css_class("desk-metrica-nome");
    ligado_l.add_css_class("desk-metrica-sub");
    q.append(&rodape);
    dados::ouvir(&q, move |a: &Amostra| {
        v1.set_text(&format!("{:.0}%", a.cpu * 100.0));
        g1.mostrar(&a.hist_cpu.iter().copied().collect::<Vec<_>>());
        match &a.gpu {
            Some(g) => {
                v2.set_text(&format!("{:.0}%", g.uso * 100.0));
                g2.mostrar(&a.hist_gpu.iter().copied().collect::<Vec<_>>());
            }
            None => v2.set_text("—"),
        }
        v3.set_text(&format!("{:.0}%", if a.mem.1 > 0.0 { a.mem.0 / a.mem.1 * 100.0 } else { 0.0 }));
        g3.mostrar(&a.hist_mem.iter().copied().collect::<Vec<_>>());
        ligado_l.set_text(&ligado(a.ligado_s));
        temp.set_text(&a.temp.map(|t| trf("Temp. {t} °C", &[("t", format!("{t:.0}"))])).unwrap_or_default());
    });
    q.upcast()
}

#[cfg(test)]
mod tests {
    #[test]
    fn uptime_reads_naturally() {
        // (the words follow the system language; the numbers are what matters here)
        let a = super::ligado(3 * 86400 + 4 * 3600 + 120);
        assert!(a.contains("3 d 4 h"), "{a}");
        let b = super::ligado(2 * 3600 + 5 * 60);
        assert!(b.contains("2 h 5 min"), "{b}");
    }
}
