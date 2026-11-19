//! KalangoOS · movimento, a página "11 · Motion" do Design System em código.
//!
//! - **Molas, não cronômetro.** Toda transição é uma mola (massa, rigidez, amortecimento). Ela pode ser
//!   interrompida no meio e segue da velocidade atual, sem "pulo".
//! - **Quatro molas:** rápida (hover, toque, fechar), suave (abrir painéis e menus), elástica (ilha, app abrindo
//!   na dock) e lenta (trocar de espaço, páginas, progresso).
//! - **Só transformação e opacidade.** [`Animavel`] desenha o filho deslocado, escalado e com opacidade no
//!   `snapshot`: nada de layout a cada quadro. O relógio é o do quadro (`add_tick_callback`), então acompanha
//!   60, 120 ou 144 Hz.
//! - **Reduzir animações** (configuração do GTK ou `KALANGO_REDUZIR_MOVIMENTO=1`): nada desliza nem escala; a
//!   opacidade dissolve em 90 ms.

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

/// Parâmetros de uma mola (os mesmos tokens do Design System).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tipo {
    pub rigidez: f64,
    pub amortecimento: f64,
    pub massa: f64,
}

/// Respostas diretas: hover, toque, fechar, encaixe de janela (≈ 210 ms, ζ 0,83).
pub const RAPIDA: Tipo = Tipo { rigidez: 520.0, amortecimento: 38.0, massa: 1.0 };
/// Padrão: abrir janelas, painéis, menus, visão geral (≈ 270 ms, ζ 0,87).
pub const SUAVE: Tipo = Tipo { rigidez: 300.0, amortecimento: 30.0, massa: 1.0 };
/// Retorno com vida: ilha, interruptor, app abrindo na dock (≈ 400 ms, ζ 0,51).
pub const ELASTICA: Tipo = Tipo { rigidez: 380.0, amortecimento: 20.0, massa: 1.0 };
/// Percursos longos: trocar de espaço, páginas, progresso (≈ 310 ms, ζ 0,97).
pub const LENTA: Tipo = Tipo { rigidez: 180.0, amortecimento: 26.0, massa: 1.0 };

/// Intervalo da cascata de listas e quantos itens animam (o resto aparece junto).
pub const CASCATA_MS: u32 = 24;
pub const CASCATA_MAX: usize = 8;
/// Duração do dissolve com "reduzir animações".
pub const DISSOLVE_MS: f64 = 90.0;

impl Tipo {
    /// Razão de amortecimento ζ.
    pub fn zeta(&self) -> f64 {
        self.amortecimento / (2.0 * (self.rigidez * self.massa).sqrt())
    }

    /// Tempo de assentamento a 2 % pela fórmula clássica 4/(ζ·ωₙ), a que o Design System usa nos rótulos "≈ ms".
    /// Perto do amortecimento crítico (a lenta, ζ 0,97) a mola real demora mais que a fórmula.
    pub fn assentamento_teorico(&self) -> f64 {
        4.0 / (self.zeta() * (self.rigidez / self.massa).sqrt())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mola {
    pub x: f64,
    pub v: f64,
    pub alvo: f64,
    pub tipo: Tipo,
}

impl Mola {
    pub fn new(x: f64) -> Self {
        Mola { x, v: 0.0, alvo: x, tipo: SUAVE }
    }

    /// Novo destino: a velocidade atual é mantida (interrupção sem pulo).
    pub fn ir(&mut self, alvo: f64, tipo: Tipo) {
        self.alvo = alvo;
        self.tipo = tipo;
    }

    pub fn pular(&mut self, x: f64) {
        self.x = x;
        self.alvo = x;
        self.v = 0.0;
    }

    /// Avança `dt` segundos (passos de 1 ms, Euler semi-implícito: estável e fiel à curva).
    pub fn passo(&mut self, dt: f64) {
        let n = (dt / 0.001).ceil().max(1.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            let a = (-self.tipo.rigidez * (self.x - self.alvo) - self.tipo.amortecimento * self.v) / self.tipo.massa;
            self.v += a * h;
            self.x += self.v * h;
        }
        if (self.x - self.alvo).abs() < 0.0005 && self.v.abs() < 0.005 {
            self.pular(self.alvo);
        }
    }

    pub fn parada(&self) -> bool {
        self.x == self.alvo && self.v == 0.0
    }
}

/// Quanto tempo uma mola leva para ficar a menos de `tolerancia` do destino, de 0 a 1 (para conferir os tokens).
pub fn tempo_para_assentar(tipo: Tipo, tolerancia: f64) -> f64 {
    let mut m = Mola::new(0.0);
    m.ir(1.0, tipo);
    let (mut t, mut ultimo_fora) = (0.0, 0.0);
    while t < 3.0 {
        m.passo(0.001);
        t += 0.001;
        if (m.x - 1.0).abs() > tolerancia {
            ultimo_fora = t;
        }
    }
    ultimo_fora
}

pub fn reduzido() -> bool {
    if std::env::var_os("KALANGO_REDUZIR_MOVIMENTO").is_some() {
        return true;
    }
    gtk::Settings::default().map(|s| !s.is_gtk_enable_animations()).unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Canal {
    X,
    Y,
    Escala,
    Opacidade,
}

const CANAIS: usize = 4;

fn idx(c: Canal) -> usize {
    c as usize
}

#[derive(Clone, Copy)]
pub struct Dissolve {
    de: f64,
    para: f64,
    inicio: i64,
}

mod imp {
    use super::*;

    pub struct Animavel {
        pub molas: Cell<[Mola; CANAIS]>,
        pub dissolve: Cell<Option<Dissolve>>,
        pub origem: Cell<(f32, f32)>,
        pub filho: RefCell<Option<gtk::Widget>>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        pub ultimo: Cell<i64>,
        /// vigia: avança a animação se o compositor parar de mandar quadros
        pub vigia: RefCell<Option<glib::SourceId>>,
        pub ultimo_passo: Cell<i64>,
        pub ao_parar: RefCell<Vec<Box<dyn FnOnce()>>>,
    }

    impl Default for Animavel {
        fn default() -> Self {
            Animavel {
                molas: Cell::new([Mola::new(0.0), Mola::new(0.0), Mola::new(1.0), Mola::new(1.0)]),
                dissolve: Cell::new(None),
                origem: Cell::new((0.5, 0.5)),
                filho: RefCell::default(),
                tick: RefCell::default(),
                ultimo: Cell::new(0),
                vigia: RefCell::default(),
                ultimo_passo: Cell::new(0),
                ao_parar: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Animavel {
        const NAME: &'static str = "KalangoAnimavel";
        type Type = super::Animavel;
        type ParentType = gtk::Widget;

        fn class_init(k: &mut Self::Class) {
            k.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Animavel {
        fn dispose(&self) {
            if let Some(c) = self.filho.take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Animavel {
        fn snapshot(&self, s: &gtk::Snapshot) {
            let w = self.obj();
            let Some(c) = self.filho.borrow().clone() else { return };
            let m = self.molas.get();
            let (x, y, e) = (m[idx(Canal::X)].x as f32, m[idx(Canal::Y)].x as f32, m[idx(Canal::Escala)].x as f32);
            let op = match self.dissolve.get() {
                Some(_) => self.opacidade_dissolve(),
                None => m[idx(Canal::Opacidade)].x,
            }
            .clamp(0.0, 1.0);
            if op <= 0.001 {
                return;
            }
            let (ox, oy) = self.origem.get();
            let (ox, oy) = (ox * w.width() as f32, oy * w.height() as f32);
            s.save();
            s.translate(&graphene::Point::new(x + ox, y + oy));
            s.scale(e, e);
            s.translate(&graphene::Point::new(-ox, -oy));
            let opaco = op < 0.999;
            if opaco {
                s.push_opacity(op);
            }
            w.snapshot_child(&c, s);
            if opaco {
                s.pop();
            }
            s.restore();
        }
    }

    impl Animavel {
        pub fn opacidade_dissolve(&self) -> f64 {
            let Some(d) = self.dissolve.get() else { return 1.0 };
            let agora = self.obj().frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let t = ((agora - d.inicio) as f64 / 1000.0 / DISSOLVE_MS).clamp(0.0, 1.0);
            d.de + (d.para - d.de) * t
        }
    }
}

glib::wrapper! {
    /// Envolve um widget e anima deslocamento, escala e opacidade com as molas do Design System.
    pub struct Animavel(ObjectSubclass<imp::Animavel>) @extends gtk::Widget;
}

impl Animavel {
    pub fn new(filho: &impl IsA<gtk::Widget>) -> Self {
        let a: Animavel = glib::Object::new();
        filho.set_parent(&a);
        *a.imp().filho.borrow_mut() = Some(filho.clone().upcast());
        a
    }

    /// Ponto fixo da escala, em fração do tamanho (0,5 · 0,5 = centro).
    pub fn origem(&self, fx: f32, fy: f32) -> &Self {
        self.imp().origem.set((fx, fy));
        self
    }

    pub fn valor(&self, c: Canal) -> f64 {
        self.imp().molas.get()[idx(c)].x
    }

    /// Define sem animar.
    pub fn definir(&self, c: Canal, v: f64) -> &Self {
        let mut m = self.imp().molas.get();
        m[idx(c)].pular(v);
        self.imp().molas.set(m);
        if c == Canal::Opacidade {
            self.imp().dissolve.set(None);
        }
        self.queue_draw();
        self
    }

    /// Anima até `alvo` com a mola `tipo`. Interrompe o que estiver em curso mantendo a velocidade.
    pub fn ir(&self, c: Canal, alvo: f64, tipo: Tipo) -> &Self {
        if reduzido() {
            if c == Canal::Opacidade {
                let de = self.valor(c);
                let inicio = self.frame_clock().map(|k| k.frame_time()).unwrap_or_else(glib::monotonic_time);
                self.imp().dissolve.set(Some(Dissolve { de, para: alvo, inicio }));
                let mut m = self.imp().molas.get();
                m[idx(c)].pular(alvo);
                self.imp().molas.set(m);
            } else {
                // nada desliza nem escala
                self.definir(c, alvo);
            }
        } else {
            let mut m = self.imp().molas.get();
            m[idx(c)].ir(alvo, tipo);
            self.imp().molas.set(m);
        }
        self.animar();
        self
    }

    /// O mesmo que [`ir`](Self::ir), começando depois de `ms` (cascata).
    pub fn ir_apos(&self, ms: u32, c: Canal, alvo: f64, tipo: Tipo) {
        if ms == 0 {
            self.ir(c, alvo, tipo);
            return;
        }
        let w = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(ms as u64), move || {
            if let Some(a) = w.upgrade() {
                a.ir(c, alvo, tipo);
            }
        });
    }

    /// Chamado uma vez, quando todas as molas pararem.
    pub fn ao_parar(&self, f: impl FnOnce() + 'static) {
        self.imp().ao_parar.borrow_mut().push(Box::new(f));
        self.animar();
    }

    pub fn em_movimento(&self) -> bool {
        self.imp().tick.borrow().is_some()
    }

    /// Avança todas as molas `dt` segundos; devolve true quando todas pararam (e chama os `ao_parar`).
    fn avancar(&self, dt: f64, agora_quadro: i64) -> bool {
        let imp = self.imp();
        let mut m = imp.molas.get();
        for mola in m.iter_mut() {
            if !mola.parada() {
                mola.passo(dt);
            }
        }
        imp.molas.set(m);
        imp.ultimo_passo.set(glib::monotonic_time());
        let dissolvendo = imp.dissolve.get().map(|d| (agora_quadro - d.inicio) as f64 / 1000.0 < DISSOLVE_MS).unwrap_or(false);
        if !dissolvendo {
            imp.dissolve.set(None);
        }
        self.queue_draw();
        if m.iter().all(|x| x.parada()) && !dissolvendo {
            if let Some(v) = imp.vigia.borrow_mut().take() {
                v.remove();
            }
            let fs: Vec<_> = imp.ao_parar.borrow_mut().drain(..).collect();
            for f in fs {
                f();
            }
            return true;
        }
        false
    }

    fn animar(&self) {
        if self.imp().tick.borrow().is_some() {
            return;
        }
        self.imp().ultimo.set(0);
        self.imp().ultimo_passo.set(glib::monotonic_time());
        // caminho normal: o relógio do quadro (acompanha 60, 120 ou 144 Hz)
        let id = self.add_tick_callback(|a, relogio| {
            let imp = a.imp();
            let t = relogio.frame_time();
            let dt = if imp.ultimo.get() == 0 { 1.0 / 120.0 } else { ((t - imp.ultimo.get()) as f64 / 1e6).clamp(0.0, 0.05) };
            imp.ultimo.set(t);
            if a.avancar(dt, t) {
                imp.tick.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.imp().tick.borrow_mut() = Some(id);
        // vigia: uma superfície ainda invisível (opacidade 0) não tem o que redesenhar, e alguns compositores
        // (o niri, por exemplo) então não mandam o próximo quadro. Se passar de 50 ms sem quadro, a animação
        // avança pelo relógio do sistema e pede redesenho, o que destrava o compositor.
        if self.imp().vigia.borrow().is_none() {
            let w = self.downgrade();
            let v = glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
                let imp = a.imp();
                let agora = glib::monotonic_time();
                let parado_ha = agora - imp.ultimo_passo.get();
                if parado_ha > 50_000 {
                    let dt = (parado_ha as f64 / 1e6).min(0.05);
                    imp.ultimo.set(0);
                    if a.avancar(dt, agora) {
                        if let Some(id) = imp.tick.borrow_mut().take() {
                            id.remove();
                        }
                        imp.vigia.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    }
                }
                if imp.tick.borrow().is_none() && imp.molas.get().iter().all(|m| m.parada()) {
                    imp.vigia.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
            *self.imp().vigia.borrow_mut() = Some(v);
        }
    }

    // ---------------------------------------------------------------- coreografias prontas

    /// Entrada padrão de painel: nasce de `de` (deslocamento em px, a partir de onde foi tocado), escala `escala0`→1
    /// e opacidade 0→1, com a mola suave.
    pub fn entrar(&self, de: (f64, f64), escala0: f64) {
        self.definir(Canal::X, de.0).definir(Canal::Y, de.1).definir(Canal::Escala, escala0).definir(Canal::Opacidade, 0.0);
        self.ir(Canal::X, 0.0, SUAVE).ir(Canal::Y, 0.0, SUAVE).ir(Canal::Escala, 1.0, SUAVE).ir(Canal::Opacidade, 1.0, SUAVE);
    }

    /// Saída: volta para `para` com a mola rápida (≈ 20 % mais curta que entrar) e chama `depois`.
    pub fn sair(&self, para: (f64, f64), escala1: f64, depois: impl FnOnce() + 'static) {
        self.ir(Canal::X, para.0, RAPIDA).ir(Canal::Y, para.1, RAPIDA).ir(Canal::Escala, escala1, RAPIDA).ir(Canal::Opacidade, 0.0, RAPIDA);
        self.ao_parar(depois);
    }
}

/// Faz uma lista entrar em cascata: 24 ms entre itens, no máximo 8 animados; o resto aparece junto.
pub fn cascata(itens: &[Animavel], deslocamento_y: f64) {
    for (i, a) in itens.iter().enumerate() {
        let atraso = (i.min(CASCATA_MAX) as u32) * CASCATA_MS;
        a.definir(Canal::Opacidade, 0.0).definir(Canal::Y, deslocamento_y);
        a.ir_apos(atraso, Canal::Opacidade, 1.0, SUAVE);
        a.ir_apos(atraso, Canal::Y, 0.0, SUAVE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_batem_com_o_design() {
        // ζ como na página 11 (0,83 · 0,87 · 0,51 · 0,97)
        for (t, z) in [(RAPIDA, 0.83), (SUAVE, 0.87), (ELASTICA, 0.51), (LENTA, 0.97)] {
            assert!((t.zeta() - z).abs() < 0.005, "{t:?}: ζ {} ≠ {z}", t.zeta());
        }
        // os rótulos "≈ 210 · 270 · 400 · 310 ms" do design são 4/(ζ·ωₙ) destes parâmetros
        for (t, rotulo) in [(RAPIDA, 210.0), (SUAVE, 270.0), (ELASTICA, 400.0), (LENTA, 310.0)] {
            let ms = t.assentamento_teorico() * 1000.0;
            assert!((ms - rotulo).abs() / rotulo < 0.02, "{t:?}: {ms:.0} ms ≠ {rotulo}");
        }
        // e a simulação real (a 2 % do destino), para saber o que a pessoa vê
        let ms = |t| tempo_para_assentar(t, 0.02) * 1000.0;
        println!("simulado a 2 %: rápida {:.0} ms · suave {:.0} ms · elástica {:.0} ms · lenta {:.0} ms", ms(RAPIDA), ms(SUAVE), ms(ELASTICA), ms(LENTA));
        assert!(ms(RAPIDA) < ms(SUAVE) && ms(SUAVE) < ms(ELASTICA));
    }

    #[test]
    fn elastica_passa_do_ponto_e_as_outras_nao_tanto() {
        let pico = |t: Tipo| {
            let mut m = Mola::new(0.0);
            m.ir(1.0, t);
            let mut p: f64 = 0.0;
            for _ in 0..1000 {
                m.passo(0.001);
                p = p.max(m.x);
            }
            p
        };
        assert!(pico(ELASTICA) > 1.1, "a elástica tem que ter vida");
        assert!(pico(LENTA) < 1.001, "a lenta quase não passa");
        assert!(pico(RAPIDA) < 1.02);
    }

    #[test]
    fn interromper_mantem_a_velocidade() {
        let mut m = Mola::new(0.0);
        m.ir(1.0, SUAVE);
        for _ in 0..80 {
            m.passo(0.001);
        }
        let (x, v) = (m.x, m.v);
        assert!(v > 0.5);
        m.ir(0.0, RAPIDA); // novo destino no meio do caminho
        assert_eq!((m.x, m.v), (x, v), "sem pulo de posição nem de velocidade");
        m.passo(0.001);
        assert!(m.x > x, "segue pela inércia antes de voltar");
    }
}
