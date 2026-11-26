//! KalangoOS · the installer (designs "Instalador · 1 Boas-vindas", "2 Disco", "3 Conta", "4 Instalando"): a card over
//! the live session's wallpaper. On the left the system's mark and version; on the right the four steps — welcome
//! (try it or install it, the language, the keyboard, the network, help to see and type), the disk (which one, beside
//! the system there or erasing it, how much for KalangoOS, encrypted), the account (name, user, computer, password and
//! its strength) and the installation (the engine's steps, its progress, tips while it works).
//!
//! The engine runs apart (`kalango-instalador --instalar PLAN`, through pkexec when real); the window only follows it.
//! Nothing is erased until "Instalar" on the account's step, and then only after a second, explicit confirmation.
//! Debug (KALANGO_INSTALADOR_DEBUG): "debug instalador: …" lines and the controls' places.

mod discos;
mod modelos_logica;
mod modelos_ui;
mod volume;
mod motor;

use discos::{Disk, Mode, Plan};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, LENTA, SUAVE};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn debug() -> bool {
    std::env::var_os("KALANGO_INSTALADOR_DEBUG").is_some()
}

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

fn icone(nome: &str, px: i32) -> gtk::Image {
    let i = gtk::Image::from_icon_name(nome);
    i.set_pixel_size(px);
    i
}

/// A control's place in the window, for the session test (logged once it is laid out).
fn lugar(nome: &str, w: &impl IsA<gtk::Widget>) {
    if !debug() {
        return;
    }
    let (nome, w) = (nome.to_string(), w.as_ref().clone());
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        if !w.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        if let Some(b) = w.root().and_then(|r| w.compute_bounds(&r.upcast::<gtk::Widget>())) {
            eprintln!("debug instalador: {nome} em {} {}", (b.x() + b.width() / 2.0) as i32, (b.y() + b.height() / 2.0) as i32);
        }
        glib::ControlFlow::Break
    });
}

const PASSOS: &[&str] = &[kalango_i18n::n_("Welcome"), kalango_i18n::n_("Disk"), kalango_i18n::n_("Account"), kalango_i18n::n_("AI models"), kalango_i18n::n_("Installation")];

const DICAS: &[(&str, &str)] = &[
    (kalango_i18n::n_("Ask in your own words"), kalango_i18n::n_("“find the October invoice” — the search bar answers with the file, or does it.")),
    (kalango_i18n::n_("Hold Super and talk"), kalango_i18n::n_("“organise my downloads” — the system understands, shows what it will do and only does it when you confirm.")),
    (kalango_i18n::n_("Everything the agent does can be undone"), kalango_i18n::n_("The timeline keeps each action; one click puts it back.")),
    (kalango_i18n::n_("Your data stays here"), kalango_i18n::n_("The local model runs on this computer; nothing goes to the cloud unless you allow it.")),
    (kalango_i18n::n_("Super+V brings back what you copied"), kalango_i18n::n_("The clipboard keeps your copies, never a password manager's secrets.")),
];

struct Instalador {
    janela: gtk::ApplicationWindow,
    passo: Cell<usize>,
    paginas: Vec<(gtk::Box, Animavel)>,
    modelos: Rc<modelos_ui::Modelos>,
    raiz: gtk::Box,
    estreito: bool,
    marcas: Vec<(gtk::Box, gtk::Label)>,
    titulo: gtk::Label,
    sub: gtk::Label,
    voltar: gtk::Button,
    contagem: gtk::Label,
    seguir: gtk::Button,
    // 1
    instalar: gtk::ToggleButton,
    idioma: gtk::DropDown,
    idiomas: RefCell<Vec<String>>,
    teclado: gtk::DropDown,
    teclados: RefCell<Vec<(String, String)>>,
    rede: gtk::Label,
    // 2
    discos: RefCell<Vec<Disk>>,
    disco: Cell<usize>,
    discos_box: gtk::Box,
    modo_ao_lado: gtk::CheckButton,
    modo_apagar: gtk::CheckButton,
    manual: gtk::CheckButton,
    divisao: gtk::DrawingArea,
    divisao_txt: gtk::Label,
    fracao: Cell<f64>,
    cripto: gtk::Switch,
    // 3
    nome: gtk::Entry,
    usuario: gtk::Entry,
    usuario_tocado: Cell<bool>,
    computador: gtk::Entry,
    senha: gtk::PasswordEntry,
    senha2: gtk::PasswordEntry,
    forca: gtk::DrawingArea,
    forca_txt: gtk::Label,
    confere: gtk::Image,
    sem_senha: gtk::Switch,
    digital: gtk::Switch,
    avatar: RefCell<Option<String>>,
    confirmar: Cell<bool>,
    // 4
    progresso_pct: gtk::Label,
    progresso_txt: gtk::Label,
    barra: gtk::DrawingArea,
    barra_valor: Cell<(f64, f64)>,
    etapas: Vec<(gtk::Box, gtk::Stack, gtk::Label, gtk::Label)>,
    dica_n: gtk::Label,
    dica_t: gtk::Label,
    dica_s: gtk::Label,
    dica: Animavel,
    terminou: Cell<bool>,
}

impl Instalador {
    fn new(app: &gtk::Application) -> Rc<Instalador> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.set_title(Some(tr("Install KalangoOS")));
        let estreito = estreita();
        janela.set_default_size(largura(estreito), 660);
        janela.set_resizable(false);
        janela.add_css_class("instalador");
        let nada = gtk::Box::new(Orientation::Horizontal, 0);
        nada.set_visible(false);
        janela.set_titlebar(Some(&nada));
        let raiz = caixa(Orientation::Horizontal, 0, "");
        // the left: the mark, the version, what it is
        let esq = caixa(Orientation::Vertical, 14, "inst-lateral");
        esq.set_size_request(340, -1);
        esq.set_valign(Align::Fill);
        let topo = caixa(Orientation::Vertical, 0, "");
        topo.set_vexpand(true);
        esq.append(&topo);
        // the design's mark: 264 px (a fixed image; a Picture would ask the texture's own size)
        let logo = gtk::Image::from_paintable(gdk::Texture::from_bytes(&glib::Bytes::from_static(
            // the brand's light-theme logo in the light theme
            if *CLARO.get().unwrap_or(&false) { include_bytes!("../assets/logo-claro.png") } else { include_bytes!("../assets/logo.png") })).ok().as_ref());
        logo.set_pixel_size(264);
        logo.set_halign(Align::Center);
        esq.append(&logo);
        let v = rotulo(&trf("Version {v}", &[("v", env!("CARGO_PKG_VERSION").trim_end_matches(".0").to_string())]), "inst-versao");
        v.set_xalign(0.5);
        v.set_margin_top(18);
        esq.append(&v);
        let t = rotulo(tr("Light, fast and made to work with agents."), "inst-sub");
        t.set_xalign(0.5);
        t.set_justify(gtk::Justification::Center);
        t.set_wrap(true);
        t.set_max_width_chars(26);
        esq.append(&t);
        let base = caixa(Orientation::Vertical, 0, "");
        base.set_vexpand(true);
        esq.append(&base);
        let alca = gtk::WindowHandle::new();
        alca.set_child(Some(&esq));
        // (a narrow window, as the design draws it, has no left side)
        alca.set_visible(!estreito);
        raiz.append(&alca);
        // the right: the steps
        let dir = caixa(Orientation::Vertical, 0, "inst-direita");
        dir.set_hexpand(true);
        let stepper = caixa(Orientation::Horizontal, 8, "inst-passos");
        let mut marcas = Vec::new();
        for (i, p) in PASSOS.iter().enumerate() {
            if i > 0 {
                let l = caixa(Orientation::Horizontal, 0, "inst-passo-linha");
                l.set_size_request(24, 1);
                l.set_valign(Align::Center);
                stepper.append(&l);
            }
            let m = caixa(Orientation::Horizontal, 6, "inst-passo");
            let n = caixa(Orientation::Horizontal, 0, "inst-passo-num");
            n.set_size_request(18, 18);
            n.set_valign(Align::Center);
            n.set_homogeneous(true);
            let nl = gtk::Label::new(Some(&(i + 1).to_string()));
            n.append(&nl);
            m.append(&n);
            let txt = rotulo(tr(p), "");
            m.append(&txt);
            stepper.append(&m);
            marcas.push((n, txt));
        }
        let sh = gtk::WindowHandle::new();
        sh.set_child(Some(&stepper));
        dir.append(&sh);
        let titulo = rotulo("", "inst-titulo");
        titulo.set_margin_top(18);
        dir.append(&titulo);
        let sub = rotulo("", "inst-sub");
        sub.set_margin_top(4);
        dir.append(&sub);
        let palco = caixa(Orientation::Vertical, 0, "");
        palco.set_vexpand(true);
        palco.set_margin_top(18);
        palco.set_overflow(gtk::Overflow::Hidden);
        dir.append(&palco);
        let rodape = caixa(Orientation::Horizontal, 12, "inst-rodape");
        let voltar = gtk::Button::new();
        voltar.add_css_class("inst-voltar");
        let vc = caixa(Orientation::Horizontal, 6, "");
        vc.append(&icone("go-previous-symbolic", 14));
        vc.append(&rotulo(tr("Back"), ""));
        voltar.set_child(Some(&vc));
        rodape.append(&voltar);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        rodape.append(&esp);
        let contagem = rotulo("", "inst-contagem");
        contagem.set_valign(Align::Center);
        rodape.append(&contagem);
        let seguir = gtk::Button::with_label(tr("Continue"));
        seguir.add_css_class("inst-seguir");
        rodape.append(&seguir);
        dir.append(&rodape);
        raiz.append(&dir);
        janela.set_child(Some(&raiz));

        // ---- 1 · welcome
        let p1 = caixa(Orientation::Vertical, 14, "");
        let escolhas = caixa(Orientation::Horizontal, 12, "");
        escolhas.set_homogeneous(true);
        let escolha = |icone_n: &str, t: &str, s: &str| -> gtk::ToggleButton {
            let b = gtk::ToggleButton::new();
            b.add_css_class("inst-escolha");
            let c = caixa(Orientation::Vertical, 8, "");
            let ib = caixa(Orientation::Horizontal, 0, "inst-escolha-icone");
            ib.set_size_request(32, 32);
            ib.set_homogeneous(true);
            ib.set_halign(Align::Start);
            ib.append(&icone(icone_n, 16));
            c.append(&ib);
            c.append(&rotulo(t, "inst-escolha-titulo"));
            let sl = rotulo(s, "inst-sub");
            sl.set_wrap(true);
            sl.set_max_width_chars(30);
            c.append(&sl);
            b.set_child(Some(&c));
            b
        };
        let experimentar = escolha("media-playback-start-symbolic", tr("Try it"), tr("Runs straight from the USB stick. Nothing changes on this computer."));
        let instalar = escolha("document-save-symbolic", tr("Install"), tr("Puts KalangoOS on the disk, alone or beside the system there now."));
        instalar.set_group(Some(&experimentar));
        instalar.set_active(true);
        escolhas.append(&experimentar);
        escolhas.append(&instalar);
        p1.append(&escolhas);
        let opcoes = caixa(Orientation::Vertical, 2, "inst-cartao");
        let linha = |ic: &str, t: &str, controle: &gtk::Widget| {
            let r = caixa(Orientation::Horizontal, 12, "inst-linha");
            r.append(&icone(ic, 16));
            let l = rotulo(t, "inst-texto");
            l.set_hexpand(true);
            r.append(&l);
            controle.set_valign(Align::Center);
            r.append(controle);
            r
        };
        let idioma = gtk::DropDown::new(None::<gtk::StringList>, None::<gtk::Expression>);
        idioma.add_css_class("inst-seletor");
        idioma.set_size_request(220, 32);
        opcoes.append(&linha("preferences-desktop-locale-symbolic", tr("Language"), idioma.upcast_ref()));
        let teclado = gtk::DropDown::new(None::<gtk::StringList>, None::<gtk::Expression>);
        teclado.add_css_class("inst-seletor");
        teclado.set_size_request(220, 32);
        teclado.set_enable_search(true);
        opcoes.append(&linha("input-keyboard-symbolic", tr("Keyboard"), teclado.upcast_ref()));
        let rede_b = gtk::Button::new();
        rede_b.add_css_class("inst-linha-botao");
        let rc = caixa(Orientation::Horizontal, 12, "inst-linha");
        rc.append(&icone("network-wireless-symbolic", 16));
        let rcol = caixa(Orientation::Vertical, 1, "");
        rcol.set_hexpand(true);
        rcol.append(&rotulo(tr("Network"), "inst-texto"));
        let rede = rotulo("", "inst-sub");
        rcol.append(&rede);
        rc.append(&rcol);
        rc.append(&icone("go-next-symbolic", 14));
        rede_b.set_child(Some(&rc));
        opcoes.append(&rede_b);
        p1.append(&opcoes);
        let ajuda = caixa(Orientation::Horizontal, 8, "");
        let al = rotulo(tr("Need help to see or type?"), "inst-sub");
        al.set_valign(Align::Center);
        ajuda.append(&al);
        let contraste = gtk::ToggleButton::new();
        contraste.add_css_class("inst-pilula");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.append(&icone("view-reveal-symbolic", 14));
        cc.append(&rotulo(tr("High contrast"), ""));
        contraste.set_child(Some(&cc));
        ajuda.append(&contraste);
        let leitor = gtk::ToggleButton::new();
        leitor.add_css_class("inst-pilula");
        let lc = caixa(Orientation::Horizontal, 6, "");
        lc.append(&icone("audio-speakers-symbolic", 14));
        lc.append(&rotulo(tr("Screen reader"), ""));
        leitor.set_child(Some(&lc));
        leitor.set_visible(glib::find_program_in_path("orca").is_some());
        ajuda.append(&leitor);
        // an on-screen keyboard for Wayland (wvkbd; squeekboard where it is the one installed)
        let teclado_tela = gtk::ToggleButton::new();
        teclado_tela.add_css_class("inst-pilula");
        let tc = caixa(Orientation::Horizontal, 6, "");
        tc.append(&icone("input-keyboard-symbolic", 14));
        tc.append(&rotulo(tr("Virtual keyboard"), ""));
        teclado_tela.set_child(Some(&tc));
        let programa = ["wvkbd-mobintl", "squeekboard"].into_iter().find(|p| glib::find_program_in_path(p).is_some());
        teclado_tela.set_visible(programa.is_some());
        let filho: Rc<RefCell<Option<std::process::Child>>> = Rc::default();
        teclado_tela.connect_toggled(move |b| {
            if let Some(mut c) = filho.borrow_mut().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            if b.is_active() {
                *filho.borrow_mut() = programa.and_then(|p| std::process::Command::new(p).spawn().ok());
            }
        });
        ajuda.append(&teclado_tela);
        p1.append(&ajuda);

        // ---- 2 · the disk
        let p2 = caixa(Orientation::Vertical, 12, "");
        let discos_box = caixa(Orientation::Horizontal, 12, "");
        discos_box.set_homogeneous(true);
        p2.append(&discos_box);
        let modos = caixa(Orientation::Vertical, 2, "inst-cartao");
        let modo = |t: &str, s: &str| -> (gtk::Box, gtk::CheckButton) {
            let r = caixa(Orientation::Horizontal, 10, "inst-linha");
            let c = gtk::CheckButton::with_label(t);
            c.add_css_class("inst-radio");
            c.set_hexpand(true);
            r.append(&c);
            let sl = rotulo(s, "inst-sub");
            sl.set_xalign(1.0);
            r.append(&sl);
            (r, c)
        };
        let (r_lado, modo_ao_lado) = modo("", tr("both stay in the boot menu"));
        let (r_apagar, modo_apagar) = modo("", "");
        modo_apagar.set_group(Some(&modo_ao_lado));
        let (r_manual, manual) = modo(tr("Partition by hand"), tr("for those who know what they want"));
        manual.set_group(Some(&modo_ao_lado));
        r_manual.set_visible(glib::find_program_in_path("gparted").is_some());
        modos.append(&r_lado);
        modos.append(&r_apagar);
        modos.append(&r_manual);
        p2.append(&modos);
        let divisao = gtk::DrawingArea::new();
        divisao.set_content_height(30);
        divisao.set_hexpand(true);
        divisao.add_css_class("inst-divisao");
        p2.append(&divisao);
        let divisao_txt = rotulo("", "inst-sub");
        p2.append(&divisao_txt);
        let cr = caixa(Orientation::Horizontal, 12, "inst-cartao");
        cr.add_css_class("inst-linha");
        cr.append(&icone("system-lock-screen-symbolic", 16));
        let ccol = caixa(Orientation::Vertical, 1, "");
        ccol.set_hexpand(true);
        ccol.append(&rotulo(tr("Encrypt KalangoOS"), "inst-texto"));
        ccol.append(&rotulo(tr("recommended on laptops"), "inst-sub"));
        cr.append(&ccol);
        let cripto = gtk::Switch::new();
        cripto.add_css_class("inst-switch");
        // on by default where it is recommended: a laptop (one with a battery)
        cripto.set_active(std::fs::read_dir("/sys/class/power_supply").map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("BAT"))).unwrap_or(false));
        cripto.set_valign(Align::Center);
        cr.append(&cripto);
        p2.append(&cr);
        let nota = caixa(Orientation::Horizontal, 8, "");
        nota.append(&icone("security-high-symbolic", 14));
        nota.append(&rotulo(tr("Nothing is erased until you confirm at the last step."), "inst-sub-verde"));
        p2.append(&nota);

        // ---- 3 · the account
        let p3 = caixa(Orientation::Vertical, 12, "");
        let topo3 = caixa(Orientation::Horizontal, 18, "");
        let av_col = caixa(Orientation::Vertical, 8, "");
        let av = caixa(Orientation::Horizontal, 0, "inst-avatar");
        av.set_size_request(72, 72);
        av.set_homogeneous(true);
        let av_letra = rotulo("", "inst-avatar-letra");
        av_letra.set_xalign(0.5);
        av.append(&av_letra);
        av_col.append(&av);
        let escolher_foto = gtk::Button::with_label(tr("Photo…"));
        escolher_foto.add_css_class("inst-pilula");
        av_col.append(&escolher_foto);
        topo3.append(&av_col);
        let campos = caixa(Orientation::Vertical, 8, "");
        campos.set_hexpand(true);
        let campo = |rot: &str| -> (gtk::Box, gtk::Entry) {
            let c = caixa(Orientation::Vertical, 4, "");
            c.append(&rotulo(rot, "inst-sub"));
            let e = gtk::Entry::new();
            e.add_css_class("inst-campo");
            c.append(&e);
            (c, e)
        };
        let (c_nome, nome) = campo(tr("Your name"));
        let (c_user, usuario) = campo(tr("User name"));
        let (c_comp, computador) = campo(tr("Computer's name"));
        computador.set_primary_icon_name(Some("computer-symbolic"));
        campos.append(&c_nome);
        campos.append(&c_user);
        campos.append(&c_comp);
        topo3.append(&campos);
        p3.append(&topo3);
        let senhas = caixa(Orientation::Horizontal, 12, "");
        senhas.set_homogeneous(true);
        let s1 = caixa(Orientation::Vertical, 4, "");
        s1.append(&rotulo(tr("Password"), "inst-sub"));
        let senha = gtk::PasswordEntry::new();
        senha.set_show_peek_icon(true);
        senha.add_css_class("inst-campo");
        s1.append(&senha);
        let forca = gtk::DrawingArea::new();
        forca.set_content_height(3);
        s1.append(&forca);
        let forca_txt = rotulo("", "inst-sub-verde");
        s1.append(&forca_txt);
        senhas.append(&s1);
        let s2 = caixa(Orientation::Vertical, 4, "");
        s2.set_valign(Align::Start);
        s2.append(&rotulo(tr("Repeat the password"), "inst-sub"));
        let s2r = caixa(Orientation::Horizontal, 6, "");
        let senha2 = gtk::PasswordEntry::new();
        senha2.add_css_class("inst-campo");
        senha2.set_hexpand(true);
        s2r.append(&senha2);
        let confere = icone("object-select-symbolic", 16);
        confere.add_css_class("inst-confere");
        confere.set_opacity(0.0);
        s2r.append(&confere);
        s2.append(&s2r);
        senhas.append(&s2);
        p3.append(&senhas);
        let op3 = caixa(Orientation::Vertical, 2, "inst-cartao");
        let sem_senha = gtk::Switch::new();
        sem_senha.add_css_class("inst-switch");
        op3.append(&linha("system-shutdown-symbolic", tr("Sign in without a password at start"), sem_senha.upcast_ref()));
        let digital = gtk::Switch::new();
        digital.add_css_class("inst-switch");
        let dl = linha("avatar-default-symbolic", tr("Set up the fingerprint later"), digital.upcast_ref());
        dl.set_visible(glib::find_program_in_path("fprintd-enroll").is_some());
        op3.append(&dl);
        p3.append(&op3);

        // ---- 4 · installing
        let p4 = caixa(Orientation::Vertical, 12, "");
        let pl = caixa(Orientation::Horizontal, 12, "");
        let progresso_pct = rotulo("0%", "inst-pct");
        pl.append(&progresso_pct);
        let progresso_txt = rotulo("", "inst-sub");
        progresso_txt.set_valign(Align::End);
        progresso_txt.set_margin_bottom(6);
        pl.append(&progresso_txt);
        p4.append(&pl);
        let barra = gtk::DrawingArea::new();
        barra.set_content_height(4);
        barra.set_hexpand(true);
        p4.append(&barra);
        let lista = caixa(Orientation::Vertical, 2, "inst-cartao");
        let mut etapas = Vec::new();
        for (id, nome_e) in [("disco", tr("Prepare the disk")), ("copiar", tr("Copy the system")), ("drivers", tr("Install graphics drivers")), ("modelo", tr("Record where the AI models come from")), ("final", tr("Finish"))] {
            let r = caixa(Orientation::Horizontal, 10, "inst-etapa");
            let marca = gtk::Stack::new();
            marca.set_size_request(16, 16);
            marca.add_named(&rotulo("—", "inst-etapa-espera"), Some("espera"));
            let sp = gtk::Spinner::new();
            sp.set_spinning(true);
            marca.add_named(&sp, Some("fazendo"));
            marca.add_named(&icone("object-select-symbolic", 14), Some("feito"));
            r.append(&marca);
            let n = rotulo(nome_e, "inst-texto");
            n.set_hexpand(true);
            r.append(&n);
            let v = rotulo("", "inst-sub");
            v.set_xalign(1.0);
            r.append(&v);
            lista.append(&r);
            let _ = id;
            etapas.push((r, marca, n, v));
        }
        p4.append(&lista);
        let dica_card = caixa(Orientation::Vertical, 4, "inst-dica");
        let dl1 = caixa(Orientation::Horizontal, 6, "");
        dl1.append(&icone("starred-symbolic", 12));
        let dica_n = rotulo("", "inst-dica-n");
        dl1.append(&dica_n);
        dica_card.append(&dl1);
        let dica_t = rotulo("", "inst-texto-forte");
        dica_card.append(&dica_t);
        let dica_s = rotulo("", "inst-sub");
        dica_s.set_wrap(true);
        dica_s.set_max_width_chars(60);
        dica_card.append(&dica_s);
        let dica = Animavel::new(&dica_card);
        p4.append(&dica);

        // ---- 4 · the AI models' source (the installer only records the choice)
        let modelos = modelos_ui::Modelos::new(&janela, estreito);
        let p_modelos = caixa(Orientation::Vertical, 0, "");
        p_modelos.append(&modelos.root);
        let paginas: Vec<(gtk::Box, Animavel)> = [p1, p2, p3, p_modelos, p4].into_iter().map(|p| {
            let a = Animavel::new(&p);
            a.set_vexpand(true);
            (p, a)
        }).collect();
        for (_, a) in &paginas {
            palco.append(a);
            a.set_visible(false);
        }
        let i = Rc::new(Instalador {
            janela, passo: Cell::new(0), paginas, modelos, raiz: raiz.clone(), estreito, marcas, titulo, sub, voltar, contagem, seguir,
            instalar, idioma, idiomas: RefCell::default(), teclado, teclados: RefCell::default(), rede,
            discos: RefCell::default(), disco: Cell::new(0), discos_box, modo_ao_lado, modo_apagar, manual, divisao, divisao_txt, fracao: Cell::new(0.6), cripto,
            nome, usuario, usuario_tocado: Cell::new(false), computador, senha, senha2, forca, forca_txt, confere, sem_senha, digital, avatar: RefCell::default(),
            confirmar: Cell::new(false),
            progresso_pct, progresso_txt, barra, barra_valor: Cell::new((0.0, 0.0)), etapas, dica_n, dica_t, dica_s, dica, terminou: Cell::new(false),
        });
        i.ligar(&experimentar, &rede_b, &contraste, &leitor, &escolher_foto, &av_letra);
        i.ler_sistema();
        i.ir(0);
        // session tests and captures: start on a step, with a source filled in ("KALANGO_INSTALADOR_MODELOS=1|https://…")
        if debug() {
            if let Some(p) = std::env::var("KALANGO_INSTALADOR_PASSO").ok().and_then(|p| p.parse::<usize>().ok()) {
                i.ir(p.min(3));
            }
            if let Some(dest) = std::env::var_os("KALANGO_INSTALADOR_CAPTURA") {
                // (the window's own pixels to a PNG after a while, then it ends: session checks and design comparisons)
                let w = Rc::downgrade(&i);
                let ms: u64 = std::env::var("KALANGO_INSTALADOR_CAPTURA_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
                glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                    if let Some(i) = w.upgrade() {
                        let j = &i.janela;
                        let pa = gtk::WidgetPaintable::new(Some(j));
                        let s = gtk::Snapshot::new();
                        pa.snapshot(&s, j.width() as f64, j.height() as f64);
                        if let (Some(n), Some(r)) = (s.to_node(), j.native().and_then(|n| n.renderer())) {
                            let _ = r.render_texture(&n, None).save_to_png(&dest);
                        }
                    }
                    std::process::exit(0);
                });
            }
            if let Ok(t) = std::env::var("KALANGO_INSTALADOR_MARCAR") {
                i.modelos.depurar_marcar(&t.split(',').map(|x| x == "1").collect::<Vec<_>>());
            }
            if std::env::var_os("KALANGO_INSTALADOR_TESTAR").is_some() {
                let w = Rc::downgrade(&i);
                glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || if let Some(i) = w.upgrade() { i.modelos.depurar_testar() });
            }
            if let Some((f, v)) = std::env::var("KALANGO_INSTALADOR_MODELOS").ok().as_deref().and_then(|t| t.split_once('|').map(|(a, b)| (a.to_string(), b.to_string()))) {
                i.modelos.depurar_preencher(f.parse().unwrap_or(0), &v);
                if let Ok(t) = std::env::var("KALANGO_INSTALADOR_TOKEN") {
                    i.modelos.depurar_token(&t);
                }
            }
        }
        i
    }

    fn ligar(self: &Rc<Self>, experimentar: &gtk::ToggleButton, rede_b: &gtk::Button, contraste: &gtk::ToggleButton, leitor: &gtk::ToggleButton, foto: &gtk::Button, av_letra: &gtk::Label) {
        let w = Rc::downgrade(self);
        self.seguir.connect_clicked(move |_| if let Some(i) = w.upgrade() { i.continuar() });
        let w = Rc::downgrade(self);
        self.voltar.connect_clicked(move |_| if let Some(i) = w.upgrade() { i.ir(i.passo.get().saturating_sub(1)) });
        let w = Rc::downgrade(self);
        experimentar.connect_toggled(move |_| if let Some(i) = w.upgrade() { i.rodape() });
        lugar("experimentar", experimentar);
        lugar("instalar", &self.instalar);
        lugar("continuar", &self.seguir);
        lugar("voltar", &self.voltar);
        let w = Rc::downgrade(self);
        *self.modelos.mudou.borrow_mut() = Some(Box::new(move || if let Some(i) = w.upgrade() { i.confirmar.set(false); i.rodape() }));
        let w = Rc::downgrade(self);
        *self.modelos.voltar_disco.borrow_mut() = Some(Box::new(move || if let Some(i) = w.upgrade() { i.ir(1) }));
        // the network: the session's settings (or NetworkManager's editor)
        rede_b.connect_clicked(|_| {
            let shell = glib::find_program_in_path("kalango-shell").is_some();
            let _ = if shell { std::process::Command::new("kalango-shell").args(["configuracoes", "rede"]).spawn() } else { std::process::Command::new("nm-connection-editor").spawn() };
        });
        // help to see and type: high contrast here; the screen reader (Orca) for everything
        let jw = self.janela.downgrade();
        contraste.connect_toggled(move |b| if let Some(j) = jw.upgrade() { if b.is_active() { j.add_css_class("alto-contraste") } else { j.remove_css_class("alto-contraste") } });
        leitor.connect_toggled(|b| {
            if b.is_active() {
                let _ = std::process::Command::new("orca").arg("--replace").spawn();
            } else {
                let _ = std::process::Command::new("pkill").args(["-x", "orca"]).status();
            }
        });
        // the keyboard: the live session takes it at once (niri reads the shell's input file)
        let w = Rc::downgrade(self);
        self.teclado.connect_selected_notify(move |d| {
            let Some(i) = w.upgrade() else { return };
            let escolhido = i.teclados.borrow().get(d.selected() as usize).cloned();
            if let Some((l, v)) = escolhido {
                let dir = glib::user_config_dir().join("kalango");
                let _ = std::fs::create_dir_all(&dir);
                let kdl = format!("input {{\n    keyboard {{\n        xkb {{\n            layout \"{l}\"\n{}        }}\n    }}\n}}\n", if v.is_empty() { String::new() } else { format!("            variant \"{v}\"\n") });
                if std::env::var_os("KALANGO_INSTALADOR_DEBUG").is_none() || std::env::var_os("KALANGO_INSTALADOR_TECLADO_VIVO").is_some() {
                    let _ = std::fs::write(dir.join("entrada.kdl"), kdl);
                }
                if debug() {
                    eprintln!("debug instalador: teclado {l} {v}");
                }
            }
        });
        // the disk: the modes, the split
        for c in [&self.modo_ao_lado, &self.modo_apagar, &self.manual] {
            let w = Rc::downgrade(self);
            c.connect_toggled(move |c| {
                if let Some(i) = w.upgrade().filter(|_| c.is_active()) {
                    if c == &i.manual {
                        let _ = std::process::Command::new("gparted").spawn();
                    }
                    i.mostrar_divisao();
                }
            });
        }
        lugar("modo apagar", &self.modo_apagar);
        lugar("modo ao lado", &self.modo_ao_lado);
        let w = Rc::downgrade(self);
        self.divisao.set_draw_func(move |_, cr, wd, h| {
            let Some(i) = w.upgrade() else { return };
            let (wd, h) = (wd as f64, h as f64);
            let f = i.fracao.get();
            let x = (wd * (1.0 - f)).round();
            let discos = i.discos.borrow();
            let d = discos.get(i.disco.get());
            let rr = |cr: &gtk::cairo::Context, x: f64, w: f64| {
                cr.new_sub_path();
                let r = 4.0;
                cr.arc(x + w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
                cr.arc(x + w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
                cr.arc(x + r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
                cr.arc(x + r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
                cr.close_path();
            };
            let (r, g, b) = rgb("#26272F");
            cr.set_source_rgb(r, g, b);
            rr(cr, 0.0, x - 3.0);
            let _ = cr.fill();
            let (r, g, b) = rgb("#A5A1E2");
            cr.set_source_rgba(r, g, b, 0.28);
            rr(cr, x + 3.0, wd - x - 3.0);
            let _ = cr.fill();
            // the handle
            let (r, g, b) = rgb("#A5A1E2");
            cr.set_source_rgb(r, g, b);
            cr.rectangle(x - 1.5, 2.0, 3.0, h - 4.0);
            let _ = cr.fill();
            let sistema = d.and_then(|d| d.system.clone()).unwrap_or_default();
            let total = d.map(|d| d.size).unwrap_or(0) as f64;
            cr.select_font_face("Inter", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(11.0);
            let (r, g, b) = rgb("#8E8F99");
            cr.set_source_rgb(r, g, b);
            let t1 = format!("{sistema} · {}", discos::human((total * (1.0 - f)) as u64));
            cr.move_to(x / 2.0 - t1.len() as f64 * 2.6, h / 2.0 + 4.0);
            let _ = cr.show_text(&t1);
            let (r, g, b) = rgb("#E2E2E6");
            cr.set_source_rgb(r, g, b);
            let t2 = format!("KalangoOS · {}", discos::human((total * f) as u64));
            cr.move_to(x + (wd - x) / 2.0 - t2.len() as f64 * 2.6, h / 2.0 + 4.0);
            let _ = cr.show_text(&t2);
        });
        let arrastar = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        let inicio = Rc::new(Cell::new(0.6));
        let ini = inicio.clone();
        arrastar.connect_drag_begin(move |_, _, _| if let Some(i) = w.upgrade() { ini.set(i.fracao.get()) });
        let w = Rc::downgrade(self);
        arrastar.connect_drag_update(move |g, dx, _| {
            let Some(i) = w.upgrade() else { return };
            let wd = g.widget().map(|w| w.width()).unwrap_or(1).max(1) as f64;
            // KalangoOS gets at least 30 GB, the system there keeps what it uses plus 10%
            let discos = i.discos.borrow();
            let Some(d) = discos.get(i.disco.get()) else { return };
            let min = (30u64 << 30) as f64 / d.size as f64;
            let max = 1.0 - (d.system_used as f64 * 1.1) / d.size as f64;
            i.fracao.set((inicio.get() - dx / wd).clamp(min, max.max(min)));
            drop(discos);
            i.divisao.queue_draw();
            i.texto_divisao();
        });
        self.divisao.add_controller(arrastar);
        // the account: the user from the name (until typed), the computer from the user
        let w = Rc::downgrade(self);
        let al = av_letra.downgrade();
        self.nome.connect_changed(move |e| {
            let Some(i) = w.upgrade() else { return };
            if !i.usuario_tocado.get() {
                i.usuario.set_text(&discos::user_from(&e.text()));
            }
            if let Some(l) = al.upgrade() {
                l.set_text(&e.text().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default());
            }
            i.validar();
        });
        let w = Rc::downgrade(self);
        self.usuario.connect_changed(move |e| {
            let Some(i) = w.upgrade() else { return };
            if e.has_focus() {
                i.usuario_tocado.set(true);
            }
            let model = std::fs::read_to_string("/sys/class/dmi/id/product_name").map(|s| s.trim().split_whitespace().last().unwrap_or("").to_string()).unwrap_or_default();
            if !i.computador.has_focus() {
                let host = format!("{}-{}", e.text(), if model.is_empty() { "kalango".to_string() } else { model });
                i.computador.set_text(&host.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>());
            }
            i.validar();
        });
        for e in [&self.senha, &self.senha2] {
            let w = Rc::downgrade(self);
            e.connect_changed(move |_| if let Some(i) = w.upgrade() { i.validar(); });
        }
        let w = Rc::downgrade(self);
        self.computador.connect_changed(move |_| if let Some(i) = w.upgrade() { i.validar(); });
        let w = Rc::downgrade(self);
        self.forca.set_draw_func(move |_, cr, wd, h| {
            let Some(i) = w.upgrade() else { return };
            let n = discos::strength(&i.senha.text());
            let seg = (wd as f64 - 8.0) / 3.0;
            for k in 0..3u8 {
                let on = !i.senha.text().is_empty() && k < n.max(1);
                let (r, g, b) = rgb(if !on { "#2E2F3A" } else if n <= 1 { "#EFA2A6" } else if n == 2 { "#E5C98C" } else { "#8ED2C3" });
                cr.set_source_rgb(r, g, b);
                cr.rectangle(k as f64 * (seg + 4.0), 0.0, seg, h as f64);
                let _ = cr.fill();
            }
        });
        // a photo for the account (any picture; it becomes ~/.face)
        let w = Rc::downgrade(self);
        foto.connect_clicked(move |_| {
            let Some(i) = w.upgrade() else { return };
            let d = gtk::FileDialog::new();
            let f = gtk::FileFilter::new();
            f.add_mime_type("image/*");
            let fs = gio_list(&f);
            d.set_filters(Some(&fs));
            let w2 = Rc::downgrade(&i);
            d.open(Some(&i.janela), gtk::gio::Cancellable::NONE, move |r| {
                if let (Some(i), Ok(f)) = (w2.upgrade(), r) {
                    *i.avatar.borrow_mut() = f.path().map(|p| p.display().to_string());
                }
            });
        });
        lugar("nome", &self.nome);
        lugar("senha", &self.senha);
        lugar("senha2", &self.senha2);
    }

    /// The system as it is: the languages, the keyboards, the network, the disks.
    fn ler_sistema(self: &Rc<Self>) {
        // KalangoOS's languages first (their own names), then any other the image has
        let installed: Vec<String> = std::process::Command::new("locale").arg("-a").output().map(|o| String::from_utf8_lossy(&o.stdout).lines()
            .filter(|l| l.contains("utf8") || l.contains("UTF-8")).map(|l| l.replace(".utf8", ".UTF-8")).collect()).unwrap_or_default();
        let locs = kalango_i18n::offered_languages(&installed);
        let names: Vec<String> = locs.iter().map(|l| kalango_i18n::language_name(l)).collect();
        self.idioma.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        let lang = std::env::var("LANG").unwrap_or_default().replace(".utf8", ".UTF-8");
        if let Some(i) = locs.iter().position(|l| *l == lang).or_else(|| locs.iter().position(|l| l.starts_with("pt_BR"))) {
            self.idioma.set_selected(i as u32);
        }
        *self.idiomas.borrow_mut() = locs;
        // choosing another language on the first step shows the installer in it (the texts are read once per process)
        let w = Rc::downgrade(self);
        self.idioma.connect_selected_notify(move |d| {
            let Some(i) = w.upgrade() else { return };
            let Some(l) = i.idiomas.borrow().get(d.selected() as usize).cloned() else { return };
            if i.passo.get() == 0 && kalango_i18n::normalize(&l) != kalango_i18n::active_locale() {
                relaunch_in(&l);
            }
        });
        // keyboards: xkeyboard-config's (the layouts and their variants), the live session's first
        let teclados = catalogo_teclados();
        let names: Vec<String> = teclados.iter().map(|(_, _, n)| n.clone()).collect();
        self.teclado.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        let atual = std::env::var("KALANGO_TECLADO").unwrap_or_else(|_| "br".into());
        if let Some(i) = teclados.iter().position(|(l, v, _)| *l == atual && v.is_empty()) {
            self.teclado.set_selected(i as u32);
        }
        *self.teclados.borrow_mut() = teclados.into_iter().map(|(l, v, _)| (l, v)).collect();
        // the network: what NetworkManager has up
        let rede = std::process::Command::new("nmcli").args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find(|l| !l.ends_with(":loopback") && !l.contains(":bridge")).map(|l| l.split(':').next().unwrap_or("").to_string()));
        self.rede.set_text(&match rede {
            Some(n) if !n.is_empty() => trf("{name} · connected", &[("name", n)]),
            _ => tr("not connected · the installation works without it").to_string(),
        });
        *self.discos.borrow_mut() = discos::read().into_iter().filter(|d| !d.live).collect();
        if debug() {
            let d = self.discos.borrow();
            eprintln!("debug instalador: {} discos ({})", d.len(), d.iter().map(|d| format!("{} {}", d.title(), d.system.clone().unwrap_or_default())).collect::<Vec<_>>().join(", "));
        }
    }

    fn fill_disks(self: &Rc<Self>) {
        while let Some(c) = self.discos_box.first_child() {
            self.discos_box.remove(&c);
        }
        let discos = self.discos.borrow().clone();
        let mut primeiro: Option<gtk::ToggleButton> = None;
        for (k, d) in discos.iter().enumerate().take(3) {
            let b = gtk::ToggleButton::new();
            b.add_css_class("inst-disco");
            let c = caixa(Orientation::Horizontal, 12, "");
            let ib = caixa(Orientation::Horizontal, 0, "inst-escolha-icone");
            ib.set_size_request(32, 32);
            ib.set_homogeneous(true);
            ib.set_valign(Align::Center);
            ib.append(&icone("drive-harddisk-symbolic", 16));
            c.append(&ib);
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_valign(Align::Center);
            col.append(&rotulo(&d.title(), "inst-texto-forte"));
            let s = match &d.system {
                Some(sys) => trf("has {system}", &[("system", sys.clone())]),
                None if d.partitions.is_empty() => tr("empty").to_string(),
                None => tr("data · will not be touched").to_string(),
            };
            col.append(&rotulo(&s, "inst-sub"));
            c.append(&col);
            b.set_child(Some(&c));
            if let Some(p) = &primeiro {
                b.set_group(Some(p));
            } else {
                primeiro = Some(b.clone());
            }
            b.set_active(k == self.disco.get());
            let w = Rc::downgrade(self);
            b.connect_toggled(move |b| {
                if let Some(i) = w.upgrade().filter(|_| b.is_active()) {
                    i.disco.set(k);
                    i.mostrar_divisao();
                }
            });
            // the model and whether it can be taken out, on hover
            b.set_tooltip_text(Some(&if d.removable { format!("{} · {}", d.model, tr("removable")) } else { d.model.clone() }));
            lugar(&format!("disco {k}"), &b);
            self.discos_box.append(&b);
        }
        self.mostrar_divisao();
    }

    fn mostrar_divisao(self: &Rc<Self>) {
        let discos = self.discos.borrow();
        let Some(d) = discos.get(self.disco.get()) else { return };
        let sys = d.system.clone();
        // "beside" only where there is a system that can give up space (KalangoOS gets at least 30 GB)
        let cabe = sys.is_some() && d.size.saturating_sub((d.system_used as f64 * 1.1) as u64) >= 30 << 30;
        self.modo_ao_lado.set_label(Some(&trf("Install beside {system}", &[("system", sys.clone().unwrap_or_default())])));
        self.modo_ao_lado.parent().map(|p| p.set_visible(cabe));
        self.modo_apagar.set_label(Some(&trf("Erase the disk and install", &[])));
        if let Some(s) = self.modo_apagar.next_sibling().and_downcast::<gtk::Label>() {
            s.set_text(&trf("removes everything on the {disk}", &[("disk", d.title())]));
        }
        if !cabe && self.modo_ao_lado.is_active() || (!self.modo_ao_lado.is_active() && !self.modo_apagar.is_active() && !self.manual.is_active()) {
            if cabe { self.modo_ao_lado.set_active(true) } else { self.modo_apagar.set_active(true) }
        }
        let lado = self.modo_ao_lado.is_active() && cabe;
        self.divisao.set_visible(lado);
        self.divisao_txt.set_visible(lado);
        if lado {
            let max = 1.0 - (d.system_used as f64 * 1.1) / d.size as f64;
            self.fracao.set(self.fracao.get().min(max).max(0.1));
        }
        drop(discos);
        self.divisao.queue_draw();
        self.texto_divisao();
        let n_sys = self.discos.borrow().iter().filter(|d| d.system.is_some()).count();
        let n = self.discos.borrow().len();
        if self.passo.get() == 1 {
            self.sub.set_text(&match (n, n_sys) {
                (0, _) => tr("No disk where KalangoOS fits (it needs 30 GB).").to_string(),
                (1, 0) => tr("We found one disk.").to_string(),
                (n, 0) => trf("We found {n} disks.", &[("n", n.to_string())]),
                (1, _) => tr("We found one disk with a system already installed.").to_string(),
                (n, _) => trf("We found {n} disks and a system already installed.", &[("n", n.to_string())]),
            });
        }
    }

    fn texto_divisao(&self) {
        let discos = self.discos.borrow();
        let Some(d) = discos.get(self.disco.get()) else { return };
        self.divisao_txt.set_text(&trf("Drag the handle to split the space. {system} uses {used} today.", &[("system", d.system.clone().unwrap_or_default()), ("used", discos::human(d.system_used))]));
    }

    /// The account's fields: what is wrong says itself; "Install" only when all is right.
    fn validar(&self) -> bool {
        let p = self.senha.text().to_string();
        let n = discos::strength(&p);
        self.forca.queue_draw();
        self.forca_txt.set_text(&if p.is_empty() { String::new() } else {
            let q = [tr("weak"), tr("fair"), tr("good"), tr("strong")][n as usize];
            trf("{quality} · {n} characters", &[("quality", q.to_string()), ("n", p.chars().count().to_string())])
        });
        if n <= 1 { self.forca_txt.add_css_class("fraca") } else { self.forca_txt.remove_css_class("fraca") }
        let iguais = !p.is_empty() && p == self.senha2.text().as_str();
        self.confere.set_opacity(if iguais { 1.0 } else { 0.0 });
        let user_ok = discos::valid_user(&self.usuario.text());
        let host_ok = discos::valid_hostname(&self.computador.text());
        for (e, ok) in [(self.usuario.upcast_ref::<gtk::Widget>(), user_ok || self.usuario.text().is_empty()), (self.computador.upcast_ref(), host_ok || self.computador.text().is_empty())] {
            if ok { e.remove_css_class("erro") } else { e.add_css_class("erro") }
        }
        let ok = !self.nome.text().trim().is_empty() && user_ok && host_ok && iguais && n >= 1;
        if self.passo.get() == 2 {
            self.seguir.set_sensitive(ok);
        }
        ok
    }

    fn rodape(&self) {
        let p = self.passo.get();
        self.voltar.set_visible(p > 0 && p < 4);
        self.contagem.set_text(&trf("{n} of 5", &[("n", (p + 1).to_string())]));
        self.seguir.remove_css_class("perigo");
        let (label, visivel, sensivel) = match p {
            0 if !self.instalar.is_active() => (tr("Try it").to_string(), true, true),
            0 => (tr("Continue").to_string(), true, true),
            1 => (tr("Continue").to_string(), true, !self.discos.borrow().is_empty()),
            3 if self.confirmar.get() => {
                self.seguir.add_css_class("perigo");
                let discos = self.discos.borrow();
                let d = discos.get(self.disco.get()).map(|d| d.title()).unwrap_or_default();
                (if self.modo_apagar.is_active() { trf("Confirm: erase the {disk}", &[("disk", d)]) } else { trf("Confirm: install on the {disk}", &[("disk", d)]) }, true, true)
            }
            2 => (tr("Continue").to_string(), true, self.validar()),
            3 => (tr("Continue").to_string(), true, self.modelos.pode_continuar()),
            _ if self.terminou.get() => (tr("Restart now").to_string(), true, true),
            _ => (String::new(), false, false),
        };
        self.seguir.set_label(&label);
        self.seguir.set_visible(visivel);
        self.seguir.set_sensitive(sensivel);
    }

    /// Shows a step: the page slides in from the side it comes (a slow spring), the stepper marks what is done.
    fn ir(self: &Rc<Self>, p: usize) {
        let antes = self.passo.get();
        self.passo.set(p);
        self.confirmar.set(false);
        for (k, (_, a)) in self.paginas.iter().enumerate() {
            if k == p {
                a.set_visible(true);
                let de = if p >= antes { 40.0 } else { -40.0 };
                a.definir(Canal::X, if p == antes { 0.0 } else { de });
                a.definir(Canal::Opacidade, if p == antes { 1.0 } else { 0.0 });
                a.ir(Canal::X, 0.0, LENTA);
                a.ir(Canal::Opacidade, 1.0, SUAVE);
            } else {
                a.set_visible(false);
            }
        }
        for (k, (n, t)) in self.marcas.iter().enumerate() {
            for c in ["feito", "atual"] {
                n.remove_css_class(c);
                t.remove_css_class(c);
            }
            let cls = if k < p { "feito" } else if k == p { "atual" } else { "" };
            if !cls.is_empty() {
                n.add_css_class(cls);
                t.add_css_class(cls);
            }
            if let Some(l) = n.first_child().and_downcast::<gtk::Label>() {
                l.set_text(if k < p { "✓" } else { &["1", "2", "3", "4", "5"][k] });
            }
            if self.estreito {
                t.set_visible(k == p);
            }
        }
        let (t, s) = match p {
            0 => (tr("Welcome to KalangoOS"), tr("You can try it without installing anything, or install it now.")),
            1 => (tr("Where to install?"), ""),
            2 => (tr("Who will use it?"), tr("This will be the administrator's account. Other people can be added later.")),
            3 => (tr("Where do the AI models come from?"), if self.estreito { tr("They don't ship inside the installer. Choose the source and which to download.") } else { tr("They don't ship inside the installer. Choose the source and which to download; the downloads start after first use.") }),
            _ => (tr("Installing KalangoOS"), tr("You can keep using the computer. We'll tell you when it's done.")),
        };
        self.titulo.set_text(t);
        self.sub.set_text(s);
        self.sub.set_wrap(true);
        if p == 3 { self.sub.add_css_class("grande") } else { self.sub.remove_css_class("grande") }
        if p == 1 {
            self.fill_disks();
        }
        if p == 2 {
            self.nome.grab_focus();
        }
        if p == 3 {
            self.modelos.definir_capacidade(self.capacidade());
        }
        // the models' step is taller (the design's 800 px) when the screen has the room
        self.raiz.set_size_request(largura(self.estreito), altura(p == 3));
        self.rodape();
        if debug() {
            eprintln!("debug instalador: passo {}", p + 1);
        }
    }

    fn continuar(self: &Rc<Self>) {
        match self.passo.get() {
            0 if !self.instalar.is_active() => {
                if debug() {
                    eprintln!("debug instalador: experimentar");
                }
                self.janela.close();
            }
            3 if !self.confirmar.get() => {
                if !self.modelos.pode_continuar() {
                    return;
                }
                // a second click confirms what will happen to the disk
                self.confirmar.set(true);
                self.rodape();
            }
            3 => self.instalar_agora(),
            4 if self.terminou.get() => {
                if debug() {
                    eprintln!("debug instalador: reiniciar");
                } else {
                    // --force: the live session runs from the install media, and a normal reboot stopped forever on
                    // "Failed unmounting cdrom.mount" (seen in VirtualBox); the disk is already written and synced
                    let _ = std::process::Command::new("sudo").args(["-n", "systemctl", "reboot", "--force"]).spawn();
                }
            }
            p => self.ir(p + 1),
        }
    }

    /// Bytes KalangoOS will have: the part given to it beside another system, else the whole disk.
    fn capacidade(&self) -> u64 {
        let discos = self.discos.borrow();
        let Some(d) = discos.get(self.disco.get()) else { return 0 };
        if self.modo_ao_lado.is_active() && self.modo_ao_lado.parent().is_some_and(|p| p.get_visible()) { (d.size as f64 * self.fracao.get()) as u64 } else { d.size }
    }

    fn plano(&self) -> Plan {
        let discos = self.discos.borrow();
        let d = discos.get(self.disco.get()).cloned().unwrap_or_default();
        let (l, v) = self.teclados.borrow().get(self.teclado.selected() as usize).cloned().unwrap_or(("br".into(), String::new()));
        Plan {
            disk: d.path.clone(),
            disk_title: d.title(),
            // (the row's own flag: is_visible() is false here, the disk's page being hidden on the account's step)
            mode: if self.modo_ao_lado.is_active() && self.modo_ao_lado.parent().is_some_and(|p| p.get_visible()) { Mode::Beside } else { Mode::Erase },
            size: (d.size as f64 * self.fracao.get()) as u64,
            encrypt: self.cripto.is_active(),
            locale: self.idiomas.borrow().get(self.idioma.selected() as usize).cloned().unwrap_or_else(|| "pt_BR.UTF-8".into()),
            keyboard: if v.is_empty() { l } else { format!("{l}:{v}") },
            name: self.nome.text().trim().to_string(),
            user: self.usuario.text().to_string(),
            hostname: self.computador.text().to_string(),
            password: self.senha.text().to_string(),
            autologin: self.sem_senha.is_active(),
            fingerprint_later: self.digital.is_active(),
            avatar: self.avatar.borrow().clone(),
            models: self.modelos.escolha().0,
            models_token: String::new(),
        }
    }

    /// The engine, apart (through pkexec when it really installs); its lines move the window.
    fn instalar_agora(self: &Rc<Self>) {
        let plano = self.plano();
        // (KALANGO_INSTALADOR_DIR: where the session test keeps it, away from the person's runtime folder)
        let dir = std::env::var_os("KALANGO_INSTALADOR_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| glib::user_runtime_dir().join("kalango"));
        let _ = std::fs::create_dir_all(&dir);
        let caminho = dir.join("plano-instalacao.json");
        let _ = std::fs::write(&caminho, serde_json::to_string_pretty(&plano).unwrap_or_default());
        self.ir(4);
        let exe = std::env::current_exe().unwrap_or_else(|_| "kalango-instalador".into());
        let real = std::env::var("KALANGO_INSTALADOR_REAL").as_deref() == Ok("1");
        let mut cmd = if real {
            let mut c = std::process::Command::new("pkexec");
            c.args(["env", "KALANGO_INSTALADOR_REAL=1"]).arg(&exe);
            c
        } else {
            std::process::Command::new(&exe)
        };
        cmd.arg("--instalar").arg(&caminho).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
        let Ok(mut filho) = cmd.spawn() else {
            self.progresso_txt.set_text(tr("The installer could not start."));
            return;
        };
        if let Some(mut sin) = filho.stdin.take() {
            use std::io::Write;
            let _ = writeln!(sin, "{}", plano.password);
            // (the access token, if any, on a line of its own: never in the plan file nor on the command line)
            let _ = writeln!(sin, "{}", self.modelos.escolha().1.map(|t| t.expose().to_string()).unwrap_or_default());
        }
        let out = filho.stdout.take();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::BufRead;
            if let Some(o) = out {
                for l in std::io::BufReader::new(o).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            }
            let _ = filho.wait();
        });
        let w = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            let Some(i) = w.upgrade() else { return glib::ControlFlow::Break };
            loop {
                match rx.try_recv() {
                    Ok(l) => i.linha_do_motor(&l),
                    Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
        });
        self.descrever_etapas(&plano);
        self.dicas();
        self.animar_barra();
    }

    /// What each step will do, said before it starts (the engine replaces it as it goes).
    fn descrever_etapas(&self, p: &Plan) {
        let tamanho = discos::human(if p.mode == Mode::Erase { self.discos.borrow().get(self.disco.get()).map(|d| d.size).unwrap_or(p.size) } else { p.size });
        let placa = std::process::Command::new("lspci").output().ok().and_then(|o| {
            let t = String::from_utf8_lossy(&o.stdout).into_owned();
            t.lines().filter(|l| l.contains("VGA") || l.contains("3D controller")).find_map(|l| {
                // "NVIDIA Corporation GA102 [GeForce RTX 3090]" → "RTX"; else the maker
                let marca = l.split("[").nth(1).and_then(|m| m.split_whitespace().find(|w| ["RTX", "GTX", "Radeon", "Arc"].contains(w))).map(str::to_string);
                marca.or_else(|| ["NVIDIA", "AMD", "Intel"].iter().find(|m| l.contains(*m)).map(|m| m.to_string()))
            })
        });
        let n_modelos = p.models.escolhidos.len();
        let fonte = match p.models.fonte {
            kalango_modelos::config::SourceKind::Padrao => trf("public catalog · {n} models", &[("n", n_modelos.to_string())]),
            kalango_modelos::config::SourceKind::Servidor => trf("private server · {n} models", &[("n", n_modelos.to_string())]),
            kalango_modelos::config::SourceKind::Pasta => trf("folder · {n} models", &[("n", n_modelos.to_string())]),
            kalango_modelos::config::SourceKind::Depois => tr("later, from the App Center").to_string(),
        };
        let textos = [
            if p.encrypt { trf("{size} encrypted", &[("size", tamanho)]) } else { tamanho },
            String::new(),
            placa.map(|m| trf("{card} card found", &[("card", m)])).unwrap_or_else(|| tr("none needed").to_string()),
            fonte,
            tr("boot menu and account").to_string(),
        ];
        for ((_, _, _, v), t) in self.etapas.iter().zip(textos) {
            v.set_text(&t);
        }
    }

    fn linha_do_motor(self: &Rc<Self>, l: &str) {
        let Ok(v) = serde_json::from_str::<Value>(l) else { return };
        if debug() && v.get("comando").is_none() {
            eprintln!("debug instalador: motor {l}");
        }
        if let Some(c) = v["comando"].as_str() {
            if debug() {
                eprintln!("debug instalador: comando {c}");
            }
            return;
        }
        if let Some(e) = v["erro"].as_str() {
            self.titulo.set_text(tr("The installation stopped"));
            self.sub.set_text(e);
            self.progresso_txt.set_text(tr("Nothing else was changed. You can try again or ask for help."));
            return;
        }
        if v["fim"] == true {
            self.terminou.set(true);
            self.barra_valor.set((self.barra_valor.get().0, 1.0));
            self.progresso_pct.set_text("100%");
            self.titulo.set_text(tr("KalangoOS is installed"));
            self.sub.set_text(tr("Restart the computer and take the USB stick out when it goes dark."));
            self.progresso_txt.set_text(tr("done"));
            self.rodape();
            return;
        }
        let Some(etapa) = v["etapa"].as_str() else { return };
        let k = motor::ETAPAS.iter().position(|e| *e == etapa).unwrap_or(0);
        let estado = v["estado"].as_str().unwrap_or("");
        let prog = v["progresso"].as_f64().unwrap_or(0.0);
        let texto = v["texto"].as_str().unwrap_or("");
        // the whole: each step its share (the copy the most)
        let pesos = [0.12, 0.6, 0.08, 0.1, 0.1];
        let total: f64 = pesos[..k].iter().sum::<f64>() + pesos[k] * if estado == "feito" || estado == "pulado" { 1.0 } else { prog };
        self.barra_valor.set((self.barra_valor.get().0, total));
        self.progresso_pct.set_text(&format!("{:.0}%", total * 100.0));
        let falta = (1.0 - total) * 6.0;
        self.progresso_txt.set_text(&format!("{} · {}", [tr("preparing the disk"), tr("copying the system"), tr("graphics drivers"), tr("the AI models' source"), tr("finishing")][k],
            if falta >= 1.0 { trf("about {n} min left", &[("n", format!("{falta:.0}"))]) } else { tr("almost there").to_string() }));
        for (j, (r, marca, n, val)) in self.etapas.iter().enumerate() {
            if j < k || (j == k && (estado == "feito" || estado == "pulado")) {
                marca.set_visible_child_name("feito");
                r.remove_css_class("atual");
                n.remove_css_class("atual");
            } else if j == k {
                marca.set_visible_child_name("fazendo");
                r.add_css_class("atual");
                n.add_css_class("atual");
            }
            if j == k && !texto.is_empty() {
                val.set_text(texto);
            }
        }
    }

    /// The bar follows the engine with a slow spring (it never jumps).
    fn animar_barra(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.barra.set_draw_func(move |_, cr, wd, h| {
            let Some(i) = w.upgrade() else { return };
            let (v, _) = i.barra_valor.get();
            let (r, g, b) = rgb("#26272F");
            cr.set_source_rgb(r, g, b);
            cr.rectangle(0.0, 0.0, wd as f64, h as f64);
            let _ = cr.fill();
            let (r, g, b) = rgb("#A5A1E2");
            cr.set_source_rgb(r, g, b);
            cr.rectangle(0.0, 0.0, wd as f64 * v, h as f64);
            let _ = cr.fill();
        });
        let w = Rc::downgrade(self);
        let mola = RefCell::new(kalango_movimento::Mola::new(0.0));
        let ultimo = Rc::new(Cell::new(0i64));
        self.barra.add_tick_callback(move |b, clock| {
            let Some(i) = w.upgrade() else { return glib::ControlFlow::Break };
            let (_, alvo) = i.barra_valor.get();
            let t = clock.frame_time();
            let dt = if ultimo.get() == 0 { 0.016 } else { ((t - ultimo.get()) as f64 / 1e6).min(0.05) };
            ultimo.set(t);
            let mut m = mola.borrow_mut();
            m.ir(alvo, LENTA);
            m.passo(dt);
            i.barra_valor.set((m.x.clamp(0.0, 1.0), alvo));
            b.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    /// "Enquanto isso": a tip every 8 seconds, coming in from below.
    fn dicas(self: &Rc<Self>) {
        let n = Rc::new(Cell::new(0usize));
        let mostrar = {
            let w = Rc::downgrade(self);
            let n = n.clone();
            move || {
                let Some(i) = w.upgrade() else { return };
                let k = n.get() % DICAS.len();
                i.dica_n.set_text(&trf("MEANWHILE — {n} OF {total}", &[("n", (k + 1).to_string()), ("total", DICAS.len().to_string())]));
                i.dica_t.set_text(tr(DICAS[k].0));
                i.dica_s.set_text(tr(DICAS[k].1));
                i.dica.definir(Canal::Y, 10.0);
                i.dica.definir(Canal::Opacidade, 0.0);
                i.dica.ir(Canal::Y, 0.0, SUAVE);
                i.dica.ir(Canal::Opacidade, 1.0, SUAVE);
                n.set(k + 1);
            }
        };
        mostrar();
        let w = Rc::downgrade(self);
        glib::timeout_add_seconds_local(8, move || {
            if w.upgrade().is_none_or(|i| i.terminou.get()) {
                return glib::ControlFlow::Break;
            }
            mostrar();
            glib::ControlFlow::Continue
        });
    }
}

/// Is the screen too narrow for the full window (the design's narrow variant has no left side)?
/// KALANGO_INSTALADOR_ESTREITO=1 forces it (session tests and captures).
fn estreita() -> bool {
    if let Some(v) = std::env::var_os("KALANGO_INSTALADOR_ESTREITO") {
        return v == "1";
    }
    saida().is_some_and(|(w, _)| w < 1100)
}

/// The compositor output's size in logical pixels.
fn saida() -> Option<(i32, i32)> {
    let m = gdk::Display::default()?.monitors().item(0)?.downcast::<gdk::Monitor>().ok()?;
    let g = m.geometry();
    Some((g.width(), g.height()))
}

fn largura(estreito: bool) -> i32 {
    let max = saida().map(|(w, _)| w - 40).unwrap_or(1040);
    if estreito { 560.min(max) } else { 1040.min(max) }
}

/// The window's height: 660 px, or the design's 800 on the models' step, never more than the output allows.
fn altura(alta: bool) -> i32 {
    let max = saida().map(|(_, h)| h - 40).unwrap_or(800);
    (if alta { 800 } else { 660 }).min(max)
}

fn gio_list(f: &gtk::FileFilter) -> gtk::gio::ListStore {
    let l = gtk::gio::ListStore::new::<gtk::FileFilter>();
    l.append(f);
    l
}

/// Starts the installer again in another language (nothing is lost on the first step: the keyboard is already the
/// session's). `LANG` decides; `LANGUAGE` and `LC_ALL` would override it, so they go.
fn relaunch_in(locale: &str) {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return };
    if debug() {
        eprintln!("debug instalador: idioma {locale}");
    }
    let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env("LANG", locale).env_remove("LANGUAGE").env_remove("LC_ALL").env_remove("LC_MESSAGES").exec();
    eprintln!("kalango-instalador: {err}");
}

extern "C" {
    fn dgettext(domain: *const std::ffi::c_char, msgid: *const std::ffi::c_char) -> *const std::ffi::c_char;
}

/// A text of xkeyboard-config in the person's language (GTK set the locale).
fn xkb_tr(s: &str) -> String {
    let (Ok(d), Ok(m)) = (std::ffi::CString::new("xkeyboard-config"), std::ffi::CString::new(s)) else { return s.to_string() };
    unsafe {
        let r = dgettext(d.as_ptr(), m.as_ptr());
        if r.is_null() {
            return s.to_string();
        }
        std::ffi::CStr::from_ptr(r).to_string_lossy().into_owned()
    }
}

/// xkeyboard-config's layouts and their variants: (layout, variant, name), the name in the person's language.
fn catalogo_teclados() -> Vec<(String, String, String)> {
    let xml = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml").unwrap_or_default();
    let tag = |s: &str, t: &str| -> Option<String> {
        let a = s.find(&format!("<{t}>"))? + t.len() + 2;
        let b = s[a..].find(&format!("</{t}>"))? + a;
        Some(s[a..b].trim().replace("&amp;", "&"))
    };
    let mut out = Vec::new();
    let Some(lista) = xml.find("<layoutList>").map(|i| &xml[i..]) else { return vec![("br".into(), String::new(), "Português (Brasil)".into())] };
    for bloco in lista.split("<layout>").skip(1) {
        let bloco = bloco.split("</layout>").next().unwrap_or("");
        let cab = bloco.split("<variantList>").next().unwrap_or("");
        let (Some(n), Some(d)) = (tag(cab, "name"), tag(cab, "description")) else { continue };
        out.push((n.clone(), String::new(), xkb_tr(&d)));
        if let Some(vl) = bloco.split("<variantList>").nth(1) {
            for v in vl.split("<variant>").skip(1) {
                if let (Some(vn), Some(vd)) = (tag(v, "name"), tag(v, "description")) {
                    out.push((n.clone(), vn, xkb_tr(&vd)));
                }
            }
        }
    }
    out
}

/// Whether the desktop is light: KalangoOS's own theme (the live session's Settings), else GNOME's colour scheme.
/// (KALANGO_INSTALADOR_TEMA=claro|escuro decides in tests.)
fn tema_claro() -> bool {
    match std::env::var("KALANGO_INSTALADOR_TEMA").as_deref() {
        Ok("claro") => return true,
        Ok("escuro") => return false,
        _ => {}
    }
    let tema: serde_json::Value = std::fs::read(glib::user_config_dir().join("kalango/tema.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    match tema["estilo"].as_str() {
        Some("claro") => true,
        Some("escuro") => false,
        // automatic: light from 7h to 19h, as the shell
        Some("automatico") => glib::DateTime::now_local().map(|d| (7..19).contains(&d.hour())).unwrap_or(false),
        _ => std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("prefer-light")),
    }
}

/// Each dark token's colour → its light one (the shell's map; and the sheet's few colours without a token).
fn mapa_claro() -> Vec<(String, String)> {
    let tokens = |css: &'static str| -> Vec<(&'static str, String)> {
        css.lines().filter_map(|l| {
            let (nome, cor) = l.trim().strip_prefix("@define-color ")?.split_once(' ')?;
            Some((nome, cor.trim_end_matches(';').trim().to_uppercase()))
        }).collect()
    };
    let (e, c) = (tokens(include_str!("../assets/tokens-dark.css")), tokens(include_str!("../assets/tokens-light.css")));
    let mut mapa: Vec<(String, String)> = e.iter().filter_map(|(n, v)| c.iter().find(|(m, _)| m == n).map(|(_, l)| (v.clone(), l.clone()))).collect();
    for (d, l) in [("#3A3B47", "#DCDCE6"), ("#4A4B57", "#C9C9D6"), ("#22232E", "#F0F0F5"), ("#20212A", "#F5F5F9")] {
        mapa.push((d.to_string(), l.to_string()));
    }
    mapa
}

static CLARO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// A colour of the dark sheet as the theme shows it, as "#RRGGBB".
fn tema_hex(hex: &str) -> String {
    if *CLARO.get().unwrap_or(&false) {
        mapa_claro().into_iter().find(|(d, _)| d.eq_ignore_ascii_case(hex)).map(|(_, l)| l).unwrap_or_else(|| hex.to_string())
    } else {
        hex.to_string()
    }
}

/// A colour of the dark sheet as the theme shows it, for cairo (0 – 1 each).
fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = if *CLARO.get().unwrap_or(&false) {
        mapa_claro().into_iter().find(|(d, _)| d.eq_ignore_ascii_case(hex)).map(|(_, l)| l).unwrap_or_else(|| hex.to_string())
    } else {
        hex.to_string()
    };
    let c = |i: usize| f64::from(u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0)) / 255.0;
    (c(1), c(3), c(5))
}

/// The installer's sheet in the theme: in the light one, every dark token's colour becomes its light one (as the
/// shell does).
fn estilo(claro: bool) -> String {
    let _ = CLARO.set(claro);
    let escuro = include_str!("../assets/tokens-dark.css");
    let folha = include_str!("../assets/style.css");
    if !claro {
        return format!("{escuro}\n{folha}");
    }
    let mapa = mapa_claro();
    let mut out = String::with_capacity(folha.len());
    let mut resto = folha;
    while let Some(k) = resto.find('#') {
        out.push_str(&resto[..k]);
        let cand = resto.get(k..k + 7).unwrap_or("");
        let hex = cand.len() == 7 && cand[1..].chars().all(|ch| ch.is_ascii_hexdigit());
        match mapa.iter().find(|(d, _)| hex && d.eq_ignore_ascii_case(cand)) {
            Some((_, l)) => {
                out.push_str(l);
                resto = &resto[k + 7..];
            }
            None => {
                out.push('#');
                resto = &resto[k + 1..];
            }
        }
    }
    out.push_str(resto);
    format!("{}\n{out}", include_str!("../assets/tokens-light.css"))
}

fn main() -> glib::ExitCode {
    // the engine (`--instalar PLAN`): no window
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--instalar") {
        let code = motor::run(args.get(2).map(String::as_str).unwrap_or(""));
        return glib::ExitCode::from(code);
    }
    let app = gtk::Application::builder().application_id("os.kalango.Instalador").build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(&estilo(tema_claro()));
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
        }
    });
    app.connect_activate(|app| {
        let i = Instalador::new(app);
        i.janela.present();
        // the window keeps the installer alive
        let k = Rc::new(RefCell::new(Some(i)));
        let k2 = k.clone();
        app.connect_shutdown(move |_| {
            k2.borrow_mut().take();
        });
        std::mem::forget(k);
    });
    app.run_with_args::<&str>(&[])
}
