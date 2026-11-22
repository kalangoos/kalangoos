//! The "AI models" step (design "Instalador · Modelos de IA"): where the models come from — the public catalog, a
//! private server (address and token, with "Test connection"), a folder or USB drive, or later — and which to
//! download. The installer only records the choice; the downloads start in the installed system.

use crate::modelos_logica::{Chip, Fail, Notice, State, Test};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_modelos::catalog::SLOTS;
use kalango_modelos::config::{Config, SourceKind};
use kalango_modelos::download::Control;
use kalango_modelos::net::{Client, NetError};
use kalango_modelos::probe::{probe_folder, probe_server};
use kalango_modelos::urlpolicy::{parse_server, UrlError};
use kalango_modelos::Secret;
use std::cell::RefCell;
use std::rc::Rc;

const KINDS: [&str; 5] = [kalango_i18n::n_("Text"), kalango_i18n::n_("Voice"), kalango_i18n::n_("Vision"), kalango_i18n::n_("Omni"), kalango_i18n::n_("Decision d1")];
const MODELS: [&str; 5] = ["LFM3 1.7B", "LFM3-Audio 2B", "LFM3-VL 2B", "LFM3-Omni", "d1-2B"];
const ABOUT: [&str; 5] = [kalango_i18n::n_("required by the agents"), kalango_i18n::n_("dictation and reading aloud"), kalango_i18n::n_("images and screen"), kalango_i18n::n_("all in one"), kalango_i18n::n_("decides with probabilities")];

fn caixa(o: Orientation, gap: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(o, gap);
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b
}

fn rotulo(t: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(t));
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_xalign(0.0);
    l
}

/// A line icon (SVG with its stroke written as #8E8F99) in the given colour of the dark sheet, in the theme.
fn svg_textura(svg: &str, cor: &str) -> Option<gtk::gdk::Texture> {
    let hex = crate::tema_hex(cor);
    gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(svg.replace("#8E8F99", &hex).into_bytes())).ok()
}

fn svg_icone(svg: &str, cor: &str, px: i32) -> gtk::Image {
    let i = gtk::Image::from_paintable(svg_textura(svg, cor).as_ref());
    i.set_pixel_size(px);
    i.set_size_request(px, px);
    // (a paintable fills its allocation: centred, it stays at the glyph's size)
    i.set_halign(Align::Center);
    i.set_valign(Align::Center);
    i
}

fn icone(nome: &str, px: i32) -> gtk::Image {
    let i = gtk::Image::from_icon_name(nome);
    i.set_pixel_size(px);
    i
}

/// GB with a decimal comma, as the disks' step writes ("3,9 GB").
fn human(bytes: u64) -> String {
    crate::discos::human(bytes)
}

/// A banner: an icon, a title, a detail and maybe an action. Its class says the tone (ok, erro, aviso, info).
struct Banner {
    root: gtk::Box,
    icone: gtk::Image,
    titulo: gtk::Label,
    detalhe: gtk::Label,
    acao: gtk::Button,
    acao_txt: gtk::Label,
}

impl Banner {
    fn new() -> Banner {
        let root = caixa(Orientation::Horizontal, 12, "inst-aviso");
        root.set_hexpand(true);
        let icone = icone("dialog-information-symbolic", 16);
        icone.set_valign(Align::Center);
        root.append(&icone);
        let textos = caixa(Orientation::Vertical, 1, "");
        textos.set_hexpand(true);
        textos.set_valign(Align::Center);
        let titulo = rotulo("", "inst-aviso-titulo");
        titulo.set_wrap(true);
        titulo.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        let detalhe = rotulo("", "inst-aviso-detalhe");
        detalhe.set_wrap(true);
        detalhe.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        detalhe.set_max_width_chars(60);
        textos.append(&titulo);
        textos.append(&detalhe);
        root.append(&textos);
        let acao = gtk::Button::new();
        acao.add_css_class("inst-aviso-acao");
        acao.set_valign(Align::Center);
        let acao_txt = rotulo("", "");
        acao.set_child(Some(&acao_txt));
        root.append(&acao);
        root.set_visible(false);
        Banner { root, icone, titulo, detalhe, acao, acao_txt }
    }

    fn set(&self, tom: &str, icone: &str, titulo: &str, detalhe: &str, acao: Option<&str>) {
        for c in ["ok", "erro", "aviso", "info", "seguro"] {
            self.root.remove_css_class(c);
        }
        self.root.add_css_class(tom);
        self.icone.set_icon_name(Some(icone));
        self.titulo.set_text(titulo);
        self.detalhe.set_text(detalhe);
        self.detalhe.set_visible(!detalhe.is_empty());
        // (a banner with only a sentence: that sentence in the plain, muted style)
        if detalhe.is_empty() { self.titulo.add_css_class("simples") } else { self.titulo.remove_css_class("simples") }
        self.acao.set_visible(acao.is_some());
        self.acao_txt.set_text(acao.unwrap_or(""));
        self.root.set_visible(true);
    }

    fn hide(&self) {
        self.root.set_visible(false);
    }
}

struct Row {
    check: gtk::CheckButton,
    nome: gtk::Label,
    modelo: gtk::Label,
    sobre: gtk::Label,
    chip: gtk::Label,
    tamanho: gtk::Label,
}

pub struct Modelos {
    pub root: gtk::ScrolledWindow,
    st: RefCell<State>,
    fontes: Vec<gtk::ToggleButton>,
    detalhe: gtk::Stack,
    endereco: gtk::Entry,
    token: gtk::Entry,
    lan: gtk::CheckButton,
    testar: gtk::Button,
    testar_txt: gtk::Label,
    dica: gtk::Label,
    pasta: gtk::Entry,
    banners: [Banner; 4],
    sem_espaco: Banner,
    linhas: Vec<Row>,
    total: gtk::Label,
    livre: gtk::Label,
    info: gtk::Label,
    ocupado: RefCell<u32>,
    /// told when the choice changes (the footer's "Continue")
    pub mudou: RefCell<Option<Box<dyn Fn()>>>,
    /// "Back to Disk"
    pub voltar_disco: RefCell<Option<Box<dyn Fn()>>>,
    janela: gtk::ApplicationWindow,
}

const ORDEM: [SourceKind; 4] = [SourceKind::Padrao, SourceKind::Servidor, SourceKind::Pasta, SourceKind::Depois];

impl Modelos {
    pub fn new(janela: &gtk::ApplicationWindow, estreito: bool) -> Rc<Modelos> {
        let catalog = kalango_modelos::catalog::parse_catalog(&std::fs::read_to_string(catalog_path()).unwrap_or_default()).unwrap_or_default();
        let st = State::new(catalog, 0, kalango_modelos::net::network_up());
        let pagina = caixa(Orientation::Vertical, 12, "inst-modelos-pagina");

        // the four sources
        let grade = gtk::Grid::new();
        grade.set_row_spacing(10);
        grade.set_column_spacing(10);
        grade.set_column_homogeneous(true);
        let mut fontes: Vec<gtk::ToggleButton> = Vec::new();
        // (the design's own line icons: an SVG each, drawn in the muted colour, and in the accent when chosen)
        let dados = [
            (include_str!("../assets/icones/rede.svg"), tr("Default source"), tr("KalangoOS public catalog")),
            (include_str!("../assets/icones/servidor.svg"), tr("Private server"), tr("your HTTPS address")),
            (include_str!("../assets/icones/pendrive.svg"), tr("Folder or USB drive"), tr("files you already have")),
            (include_str!("../assets/icones/relogio.svg"), tr("Download it later"), tr("download later, in the App Center")),
        ];
        let mut glifos: Vec<(gtk::Image, &'static str)> = Vec::new();
        for (k, (svg, t, s)) in dados.iter().enumerate() {
            let b = gtk::ToggleButton::new();
            b.add_css_class("inst-fonte");
            let c = caixa(Orientation::Horizontal, 12, "");
            let ib = caixa(Orientation::Horizontal, 0, "inst-escolha-icone");
            ib.set_size_request(32, 32);
            ib.set_homogeneous(true);
            ib.set_valign(Align::Center);
            let gi = svg_icone(svg, "#8E8F99", 16);
            ib.append(&gi);
            glifos.push((gi, svg));
            c.append(&ib);
            let tx = caixa(Orientation::Vertical, 1, "");
            tx.set_valign(Align::Center);
            tx.append(&rotulo(t, "inst-escolha-titulo"));
            let sl = rotulo(s, "inst-sub");
            sl.set_wrap(true);
            sl.set_max_width_chars(30);
            tx.append(&sl);
            c.append(&tx);
            b.set_child(Some(&c));
            if let Some(first) = fontes.first() {
                b.set_group(Some(first));
            }
            grade.attach(&b, (k % 2) as i32, (k / 2) as i32, 1, 1);
            fontes.push(b);
        }
        fontes[0].set_active(true);
        // the chosen card's glyph takes the accent
        for (b, (img, svg)) in fontes.iter().zip(glifos) {
            let pinta = move |b: &gtk::ToggleButton| img.set_paintable(svg_textura(svg, if b.is_active() { "#A5A1E2" } else { "#8E8F99" }).as_ref());
            pinta(b);
            b.connect_toggled(pinta);
        }
        pagina.append(&grade);

        // the area of each source
        let detalhe = gtk::Stack::new();
        detalhe.set_vhomogeneous(false);
        detalhe.set_hhomogeneous(false);
        detalhe.set_interpolate_size(false);
        let banners = [Banner::new(), Banner::new(), Banner::new(), Banner::new()];
        // 0 · the public catalog
        let p0 = caixa(Orientation::Vertical, 10, "");
        p0.append(&banners[0].root);
        detalhe.add_named(&p0, Some("padrao"));
        // 1 · a private server
        let p1 = caixa(Orientation::Vertical, 10, "");
        let campos = caixa(if estreito { Orientation::Vertical } else { Orientation::Horizontal }, 10, "");
        campos.set_homogeneous(!estreito);
        let campo = |rot: &str, ic: &str| -> (gtk::Box, gtk::Entry) {
            let c = caixa(Orientation::Vertical, 6, "");
            c.set_hexpand(true);
            c.append(&rotulo(rot, "inst-sub-rot"));
            let e = gtk::Entry::new();
            e.add_css_class("inst-campo");
            e.set_primary_icon_name(Some(ic));
            c.append(&e);
            (c, e)
        };
        let (c_end, endereco) = campo(tr("Server address (HTTPS)"), "network-server-symbolic");
        endereco.set_placeholder_text(Some("https://modelos.example.org/kalango"));
        endereco.set_input_purpose(gtk::InputPurpose::Url);
        let (c_tok, token) = campo(tr("Access token (optional)"), "dialog-password-symbolic");
        token.set_visibility(false);
        token.set_input_purpose(gtk::InputPurpose::Password);
        campos.append(&c_end);
        campos.append(&c_tok);
        p1.append(&campos);
        let lan = gtk::CheckButton::with_label(tr("This server is on my private network and has no HTTPS"));
        lan.add_css_class("inst-radio");
        lan.set_visible(false);
        p1.append(&lan);
        let linha_teste = caixa(Orientation::Horizontal, 12, "");
        let testar = gtk::Button::new();
        testar.add_css_class("inst-botao");
        testar.set_valign(Align::Start);
        let tb = caixa(Orientation::Horizontal, 6, "");
        tb.append(&icone("network-workgroup-symbolic", 14));
        let testar_txt = rotulo(tr("Test connection"), "");
        tb.append(&testar_txt);
        testar.set_child(Some(&tb));
        linha_teste.append(&testar);
        let dica = rotulo(tr("Lists the server's models, sizes and sums."), "inst-sub");
        dica.set_valign(Align::Center);
        dica.set_wrap(true);
        dica.set_hexpand(true);
        linha_teste.append(&dica);
        linha_teste.append(&banners[1].root);
        p1.append(&linha_teste);
        detalhe.add_named(&p1, Some("servidor"));
        // 2 · a folder or USB drive
        let p2 = caixa(Orientation::Vertical, 6, "");
        p2.append(&rotulo(tr("Folder with the models"), "inst-sub-rot"));
        let linha_pasta = caixa(Orientation::Horizontal, 10, "");
        let pasta = gtk::Entry::new();
        pasta.add_css_class("inst-campo");
        pasta.add_css_class("inst-mono");
        pasta.set_hexpand(true);
        pasta.set_primary_icon_name(Some("drive-removable-media-symbolic"));
        pasta.set_placeholder_text(Some("/media/…/modelos"));
        linha_pasta.append(&pasta);
        let escolher = gtk::Button::new();
        escolher.add_css_class("inst-botao");
        let eb = caixa(Orientation::Horizontal, 6, "");
        eb.append(&icone("folder-symbolic", 14));
        eb.append(&rotulo(tr("Choose folder…"), ""));
        escolher.set_child(Some(&eb));
        linha_pasta.append(&escolher);
        p2.append(&linha_pasta);
        let esp = caixa(Orientation::Vertical, 0, "");
        esp.set_margin_top(4);
        esp.append(&banners[2].root);
        p2.append(&esp);
        detalhe.add_named(&p2, Some("pasta"));
        // 3 · later
        let p3 = caixa(Orientation::Vertical, 10, "");
        p3.append(&banners[3].root);
        detalhe.add_named(&p3, Some("depois"));
        pagina.append(&detalhe);
        let sem_espaco = Banner::new();
        pagina.append(&sem_espaco.root);

        // the five models
        let lista = caixa(Orientation::Vertical, 0, "inst-cartao");
        lista.add_css_class("inst-modelos");
        let mut linhas = Vec::new();
        for i in 0..SLOTS.len() {
            let r = caixa(Orientation::Horizontal, 8, "inst-linha-modelo");
            let check = gtk::CheckButton::new();
            check.add_css_class("inst-radio");
            check.set_valign(Align::Center);
            r.append(&check);
            let nome = rotulo(tr(KINDS[i]), "inst-texto");
            nome.set_width_chars(if estreito { 8 } else { 14 });
            r.append(&nome);
            let modelo = rotulo(MODELS[i], "inst-modelo");
            modelo.set_width_chars(13);
            r.append(&modelo);
            let sobre = rotulo(tr(ABOUT[i]), "inst-sub");
            sobre.set_hexpand(true);
            sobre.set_ellipsize(gtk::pango::EllipsizeMode::End);
            sobre.set_visible(!estreito);
            r.append(&sobre);
            let espaco = caixa(Orientation::Horizontal, 0, "");
            espaco.set_hexpand(estreito);
            r.append(&espaco);
            let chip = rotulo("", "inst-chip");
            chip.set_valign(Align::Center);
            chip.set_visible(false);
            r.append(&chip);
            let tamanho = rotulo("", "inst-sub");
            tamanho.set_xalign(1.0);
            tamanho.set_width_chars(6);
            r.append(&tamanho);
            lista.append(&r);
            linhas.push(Row { check, nome, modelo, sobre, chip, tamanho });
        }
        pagina.append(&lista);
        let tot = caixa(Orientation::Horizontal, 8, "inst-totais");
        tot.append(&rotulo(tr("Total to download"), "inst-sub"));
        let total = rotulo("", "inst-forte");
        tot.append(&total);
        let e = caixa(Orientation::Horizontal, 0, "");
        e.set_hexpand(true);
        tot.append(&e);
        tot.append(&rotulo(tr("Free after installation"), "inst-sub"));
        let livre = rotulo("", "inst-forte");
        tot.append(&livre);
        pagina.append(&tot);
        let il = caixa(Orientation::Horizontal, 8, "");
        il.append(&icone("dialog-information-symbolic", 14));
        let info = rotulo("", "inst-sub");
        info.set_wrap(true);
        info.set_hexpand(true);
        il.append(&info);
        pagina.append(&il);

        let rolagem = gtk::ScrolledWindow::new();
        rolagem.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rolagem.set_vexpand(true);
        // (the content never ends under the footer: it scrolls, with room below the last line)
        rolagem.set_margin_bottom(10);
        rolagem.add_css_class("inst-rolagem");
        rolagem.set_child(Some(&pagina));
        let m = Rc::new(Modelos { root: rolagem, st: RefCell::new(st), fontes, detalhe, endereco, token, lan, testar, testar_txt, dica, pasta, banners, sem_espaco, linhas, total, livre, info,
            ocupado: RefCell::new(0), mudou: RefCell::new(None), voltar_disco: RefCell::new(None), janela: janela.clone() });
        m.ligar(&escolher);
        m.desenhar();
        m
    }

    fn ligar(self: &Rc<Self>, escolher: &gtk::Button) {
        for (k, b) in self.fontes.iter().enumerate() {
            let w = Rc::downgrade(self);
            b.connect_toggled(move |b| {
                if let (true, Some(m)) = (b.is_active(), w.upgrade()) {
                    m.st.borrow_mut().source = ORDEM[k];
                    m.st.borrow_mut().invalidate();
                    // the ticks come back to what the person chose, for the sources that can have them
                    let marcas: Vec<bool> = m.linhas.iter().map(|r| r.check.is_active()).collect();
                    if m.st.borrow().source != SourceKind::Depois && marcas.iter().all(|x| !x) && m.st.borrow().ticked.iter().all(|x| !x) {
                        m.st.borrow_mut().ticked = SLOTS.iter().map(|(_, id)| kalango_modelos::catalog::DEFAULT_CHOSEN.contains(id)).collect();
                    }
                    m.desenhar();
                    m.avisar();
                }
            });
        }
        for (i, r) in self.linhas.iter().enumerate() {
            let w = Rc::downgrade(self);
            r.check.connect_toggled(move |c| {
                if let Some(m) = w.upgrade() {
                    if *m.ocupado.borrow() > 0 {
                        return;
                    }
                    if let Some(t) = m.st.borrow_mut().ticked.get_mut(i) {
                        *t = c.is_active();
                    }
                    m.desenhar();
                    m.avisar();
                }
            });
        }
        let w = Rc::downgrade(self);
        self.endereco.connect_changed(move |e| {
            if let Some(m) = w.upgrade() {
                m.st.borrow_mut().server = e.text().to_string();
                m.st.borrow_mut().invalidate();
                m.desenhar();
                m.avisar();
            }
        });
        let w = Rc::downgrade(self);
        self.token.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.st.borrow_mut().invalidate();
                m.desenhar();
            }
        });
        let w = Rc::downgrade(self);
        self.lan.connect_toggled(move |c| {
            if let Some(m) = w.upgrade() {
                if *m.ocupado.borrow() == 0 {
                    m.st.borrow_mut().lan_ok = c.is_active();
                    m.st.borrow_mut().invalidate();
                    m.desenhar();
                    m.avisar();
                }
            }
        });
        let w = Rc::downgrade(self);
        self.testar.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                m.testar_servidor();
            }
        });
        let w = Rc::downgrade(self);
        self.pasta.connect_changed(move |e| {
            if let Some(m) = w.upgrade() {
                m.st.borrow_mut().folder = e.text().to_string();
                m.st.borrow_mut().invalidate();
                m.desenhar();
                m.avisar();
            }
        });
        let w = Rc::downgrade(self);
        self.pasta.connect_activate(move |_| {
            if let Some(m) = w.upgrade() {
                m.testar_pasta();
            }
        });
        let w = Rc::downgrade(self);
        escolher.connect_clicked(move |_| {
            let Some(m) = w.upgrade() else { return };
            let d = gtk::FileDialog::new();
            let w2 = Rc::downgrade(&m);
            d.select_folder(Some(&m.janela), gtk::gio::Cancellable::NONE, move |r| {
                if let (Some(m), Ok(f)) = (w2.upgrade(), r) {
                    if let Some(p) = f.path() {
                        m.pasta.set_text(&p.display().to_string());
                        m.testar_pasta();
                    }
                }
            });
        });
        let w = Rc::downgrade(self);
        self.sem_espaco.acao.connect_clicked(move |_| {
            if let Some(m) = w.upgrade() {
                if let Some(f) = m.voltar_disco.borrow().as_ref() {
                    f();
                }
            }
        });
        let w = Rc::downgrade(self);
        self.banners[0].acao.connect_clicked(|_| open_networks());
        let _ = w;
    }

    fn avisar(&self) {
        if let Some(f) = self.mudou.borrow().as_ref() {
            f();
        }
    }

    // ------------------------------------------------------------------------------------------ outside

    /// The disk step's choice: the bytes KalangoOS will have (the disk, or the part given to it).
    pub fn definir_capacidade(&self, bytes: u64) {
        self.st.borrow_mut().capacity = bytes;
        self.st.borrow_mut().online = kalango_modelos::net::network_up();
        self.desenhar();
    }

    pub fn pode_continuar(&self) -> bool {
        self.st.borrow().can_continue()
    }

    /// What the installer records, and the token for the vault (None: no token).
    pub fn escolha(&self) -> (Config, Option<Secret>) {
        let st = self.st.borrow();
        let t = self.token.text().to_string();
        let token = (st.source == SourceKind::Servidor && !t.is_empty()).then(|| Secret::new(t));
        (st.config(), token)
    }

    /// Session tests: a token typed into the field (shown masked).
    pub fn depurar_token(&self, t: &str) {
        self.token.set_text(t);
    }

    /// Session tests: the ticks as a list.
    pub fn depurar_marcar(&self, v: &[bool]) {
        for (i, t) in self.st.borrow_mut().ticked.iter_mut().enumerate() {
            *t = v.get(i).copied().unwrap_or(false);
        }
        self.desenhar();
        self.avisar();
    }

    /// Session tests: runs the test of the chosen source.
    pub fn depurar_testar(self: &Rc<Self>) {
        let fonte = self.st.borrow().source;
        match fonte {
            SourceKind::Servidor => self.testar_servidor(),
            SourceKind::Pasta => self.testar_pasta(),
            _ => {}
        }
    }

    /// Session tests: sets the source and its fields as a person would.
    pub fn depurar_preencher(&self, fonte: usize, valor: &str) {
        if let Some(b) = self.fontes.get(fonte) {
            b.set_active(true);
        }
        match fonte {
            1 => self.endereco.set_text(valor),
            2 => self.pasta.set_text(valor),
            _ => {}
        }
    }

    // ------------------------------------------------------------------------------------------ tests of the source

    fn testar_servidor(self: &Rc<Self>) {
        let (server, lan, catalog) = {
            let s = self.st.borrow();
            (s.server.clone(), s.lan_ok, s.catalog.clone())
        };
        let Ok(url) = parse_server(&server, lan) else { return };
        let token = Some(self.token.text().to_string()).filter(|t| !t.is_empty()).map(Secret::new);
        self.st.borrow_mut().set_test(Test::Running);
        self.desenhar();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let c = Client::new(Some(url.clone()), token);
            let _ = tx.send(match probe_server(&c, &url, &catalog) {
                Ok(p) => Test::Done(p),
                Err(NetError::Denied(code)) => Test::Failed(Fail::Denied(code)),
                Err(NetError::Offline) => Test::Failed(Fail::Offline),
                Err(NetError::Timeout) => Test::Failed(Fail::Timeout),
                Err(NetError::Status(c)) => Test::Failed(Fail::Http(c)),
                Err(_) => Test::Failed(Fail::Unreachable),
            });
        });
        self.esperar(rx);
    }

    fn testar_pasta(self: &Rc<Self>) {
        let (dir, catalog) = {
            let s = self.st.borrow();
            (s.folder.clone(), s.catalog.clone())
        };
        if !dir.starts_with('/') {
            return;
        }
        self.st.borrow_mut().set_test(Test::Running);
        self.desenhar();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Test::Done(probe_folder(std::path::Path::new(&dir), &catalog, &Control::default())));
        });
        self.esperar(rx);
    }

    fn esperar(self: &Rc<Self>, rx: std::sync::mpsc::Receiver<Test>) {
        let w = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(80), move || match rx.try_recv() {
            Ok(t) => {
                if let Some(m) = w.upgrade() {
                    // (only when the person is still on the same running test)
                    if m.st.borrow().test == Test::Running {
                        m.st.borrow_mut().set_test(t);
                        m.desenhar();
                        m.avisar();
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    // ------------------------------------------------------------------------------------------ drawing

    fn aplicar_aviso(&self, b: &Banner, n: &Notice) {
        match n {
            Notice::None => b.hide(),
            Notice::Public => b.set("seguro", "security-high-symbolic", tr("KalangoOS public catalog. Size and checksum are checked at every download."), "", None),
            Notice::PublicOffline => b.set("aviso", "network-wireless-offline-symbolic", tr("No internet connection"), tr("The default source needs a network. Connect, use a server on your network or a folder."), Some(tr("See networks"))),
            Notice::Later => b.set("info", "dialog-information-symbolic", tr("Nothing will be downloaded now"), tr("The agents only work after the Text model is downloaded, in the App Center."), None),
            Notice::Testing { host } => b.set("info", "content-loading-symbolic", &if host.is_empty() { tr("Checking the files and their sums…").to_string() } else { trf("Connecting to {host} and checking the sums…", &[("host", host.clone())]) }, "", None),
            Notice::Connected { host, found, of, all_sums } => b.set("ok", "object-select-symbolic", &trf("Connected to {host}", &[("host", host.clone())]),
                &format!("{} · {}", trf("{found} of {of} models found", &[("found", found.to_string()), ("of", of.to_string())]), if *all_sums { tr("all sums match") } else { tr("the sums are checked at download") }), None),
            Notice::SumMismatch { model } => b.set("erro", "dialog-warning-symbolic", &trf("The sum of {model} does not match", &[("model", SLOTS.iter().position(|(_, id)| self.st.borrow().entry(SLOTS.iter().position(|(_, i)| i == id).unwrap_or(0)).is_some_and(|e| e.nome == *model && e.id == *id)).map(|i| tr(KINDS[i]).to_string()).unwrap_or_else(|| model.clone()))]), tr("SHA-256 different from the catalog: the file was refused."), None),
            Notice::Unreachable { host, timeout } => b.set("erro", "dialog-warning-symbolic", &trf("Could not reach {host}", &[("host", host.clone())]),
                if *timeout { tr("Timed out after 10 s. Check the address and the network.") } else { tr("The server did not answer. Check the address and the network.") }, None),
            Notice::ServerError { host, code } => b.set("erro", "dialog-warning-symbolic", &trf("{host} answered with an error (HTTP {code})", &[("host", host.clone()), ("code", code.to_string())]), tr("The server is there but could not give the list. Ask whoever administers it."), None),
            Notice::Denied { code } => b.set("erro", "changes-prevent-symbolic", &trf("Access denied (error {code})", &[("code", code.to_string())]), tr("The server refused the token. Ask for a new one from whoever administers it."), None),
            Notice::FolderOk { found, of, all_sums } => {
                let (nome, livre) = self.volume();
                b.set("ok", "object-select-symbolic", &trf("{volume} · {free} free", &[("volume", nome), ("free", livre)]),
                    &format!("{} · {}", trf("{found} of {of} models found", &[("found", found.to_string()), ("of", of.to_string())]), if *all_sums { tr("all sums match") } else { tr("the sums are checked at download") }), None)
            }
            Notice::FolderEmpty => b.set("aviso", "dialog-warning-symbolic", tr("No model found in this folder"), tr("Choose the folder that has the model files (for example LFM3-1.7B-Q4_K_M.gguf)."), None),
        }
    }

    /// The volume's name and its free space, for the folder's banner.
    fn volume(&self) -> (String, String) {
        let p = self.st.borrow().folder.clone();
        let nome = crate::volume::label_of(std::path::Path::new(&p)).unwrap_or_else(|| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let livre = kalango_modelos::download::free_space(std::path::Path::new(&p)).map(human).unwrap_or_default();
        (nome, livre)
    }

    fn chip_texto(c: Chip) -> &'static str {
        match c {
            Chip::None => "",
            Chip::Verifying => tr("checking…"),
            Chip::NoNetwork => tr("no network"),
            Chip::NotVerified => tr("not verified"),
            Chip::OnServerOk => tr("on the server · sum matches"),
            Chip::OnServer => tr("on the server"),
            Chip::AbsentOnServer => tr("missing on the server"),
            Chip::InFolderOk => tr("in the folder · sum matches"),
            Chip::InFolder => tr("in the folder"),
            Chip::NotInFolder => tr("not in the folder"),
            Chip::SumMismatch => tr("sum does not match"),
            Chip::WrongSize => tr("different size"),
            Chip::NoChecksum => tr("no sum in the catalog"),
        }
    }

    pub fn desenhar(&self) {
        *self.ocupado.borrow_mut() += 1;
        let st = self.st.borrow();
        let k = ORDEM.iter().position(|s| *s == st.source).unwrap_or(0);
        self.detalhe.set_visible_child_name(["padrao", "servidor", "pasta", "depois"][k]);
        let n = st.notice();
        // each source's banner: the one of the visible area shows the notice
        for (j, b) in self.banners.iter().enumerate() {
            if j == k { self.aplicar_aviso(b, &n) } else { b.hide() }
        }
        // the server's test row: a hint while there is nothing to say, the banner otherwise
        self.dica.set_visible(k == 1 && n == Notice::None);
        // the address and the token
        let erro_end = st.server_error();
        if erro_end.is_some() { self.endereco.add_css_class("erro") } else { self.endereco.remove_css_class("erro") }
        if matches!(n, Notice::Denied { .. }) { self.token.add_css_class("erro") } else { self.token.remove_css_class("erro") }
        self.lan.set_visible(st.needs_lan_confirmation() || (self.lan.is_active() && k == 1));
        if erro_end == Some(UrlError::PlainHttp) && !st.needs_lan_confirmation() {
            self.endereco.set_tooltip_text(Some(tr("Use an https:// address.")));
        } else {
            self.endereco.set_tooltip_text(None);
        }
        let (rotulo_teste, ocupado) = match (&st.test, &n) {
            (Test::Running, _) => (tr("Testing…"), true),
            (Test::Done(_), _) => (tr("Test again"), false),
            (Test::Failed(Fail::Denied(_)), _) => (tr("Test connection"), false),
            (Test::Failed(_), _) => (tr("Try again"), false),
            _ => (tr("Test connection"), false),
        };
        self.testar_txt.set_text(rotulo_teste);
        self.testar.set_sensitive(!ocupado && st.server_valid());
        // the rows
        for (i, r) in self.linhas.iter().enumerate() {
            let pode = st.tickable(i);
            r.check.set_active(st.effective(i));
            r.check.set_sensitive(pode);
            for w in [r.nome.upcast_ref::<gtk::Widget>(), r.modelo.upcast_ref(), r.sobre.upcast_ref(), r.tamanho.upcast_ref()] {
                if pode { w.remove_css_class("apagado") } else { w.add_css_class("apagado") }
            }
            let c = st.chip(i);
            r.chip.set_visible(c != Chip::None);
            r.chip.set_text(Self::chip_texto(c));
            for t in ["bom", "aviso", "ruim", "neutro"] {
                r.chip.remove_css_class(t);
            }
            r.chip.add_css_class(c.tone());
            r.tamanho.set_text(&st.entry(i).map(|m| human(m.bytes())).unwrap_or_default());
        }
        // the totals, and the disk
        self.total.set_text(&human(st.total()));
        let livre = st.free_after().max(0) as u64;
        self.livre.set_text(&trf("{free} of {total}", &[("free", human(livre)), ("total", human(st.capacity))]));
        if st.enough_space() { self.livre.remove_css_class("ruim") } else { self.livre.add_css_class("ruim") }
        if st.enough_space() {
            self.sem_espaco.hide();
        } else {
            self.sem_espaco.set("erro", "dialog-warning-symbolic", tr("Not enough space on the chosen disk"),
                &trf("{missing} missing. Untick models or choose a larger partition in the Disk step.", &[("missing", human(st.missing()))]), Some(tr("Back to Disk")));
        }
        self.info.set_text(match st.source {
            SourceKind::Servidor => tr("The choice is recorded in the installed system. The token goes to the key vault, never as plain text."),
            SourceKind::Depois => tr("You can choose the source later in Settings › Models and providers."),
            _ => tr("The choice is recorded in the installed system and can be changed later in Settings."),
        });
        drop(st);
        *self.ocupado.borrow_mut() -= 1;
    }
}

fn open_networks() {
    let shell = glib::find_program_in_path("kalango-shell").is_some();
    let _ = if shell { std::process::Command::new("kalango-shell").args(["configuracoes", "rede"]).spawn() } else { std::process::Command::new("nm-connection-editor").spawn() };
}

/// The catalog the session or the image ships.
pub fn catalog_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_LOJA_DADOS") {
        return std::path::PathBuf::from(p).join("modelos.json");
    }
    ["/usr/share/kalango/loja/modelos.json", "sessao/dist/share/kalango/loja/modelos.json"].iter().map(std::path::PathBuf::from).find(|p| p.is_file()).unwrap_or_default()
}
