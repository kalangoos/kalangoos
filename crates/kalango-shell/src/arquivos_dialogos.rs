//! Arquivos' menus (design "Arquivos · menus completos": the file's, the folder's and the trash's — what GNOME's Files
//! offers too, with the agent's actions) and its dialogs ("Propriedades do arquivo", "Abrir com", "Conflito de
//! arquivos"), which come over the window as it dims (a short fade and scale, the system's motion).

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// What to do with a file whose name is already in the folder it goes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decisao {
    Pular,
    Ambos,
    /// the one there goes to the trash (it can come back), the new one takes its name
    Substituir,
}

/// The archives that can be opened here (with unzip or tar).
fn e_compactado(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    [".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst"].iter().any(|e| n.ends_with(e))
}

/// An archive's name without its extensions ("fotos.tar.gz" → "fotos").
fn nome_sem_extensao(p: &Path) -> String {
    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let l = n.to_lowercase();
    for e in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".zip", ".tar"] {
        if l.ends_with(e) {
            return n[..n.len() - e.len()].to_string();
        }
    }
    n
}

/// Another terminal the person chose (KALANGO_TERMINAL); none: KalangoOS's own Terminal.
fn terminal() -> Option<String> {
    std::env::var("KALANGO_TERMINAL").ok().filter(|t| !t.is_empty())
}

fn bookmarks_path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

fn e_favorito(p: &Path) -> bool {
    let uri = gio::File::for_path(p).uri().to_string();
    std::fs::read_to_string(bookmarks_path()).unwrap_or_default().lines().any(|l| l.split(' ').next() == Some(uri.as_str()))
}

/// The name of whoever owns a file (from /etc/passwd).
fn dono(uid: u32) -> String {
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines()
        .find(|l| l.split(':').nth(2).and_then(|u| u.parse::<u32>().ok()) == Some(uid))
        .and_then(|l| l.split(':').next().map(str::to_string)).unwrap_or_else(|| uid.to_string())
}

/// The apps that open a kind of file: the default one first.
fn apps_para(tipo_mime: &str) -> (Vec<gio::AppInfo>, Option<gio::AppInfo>) {
    let ct = gio::content_type_from_mime_type(tipo_mime).unwrap_or_else(|| tipo_mime.into());
    let padrao = gio::AppInfo::default_for_type(&ct, false);
    let mut v: Vec<gio::AppInfo> = Vec::new();
    if let Some(d) = &padrao {
        v.push(d.clone());
    }
    for a in gio::AppInfo::recommended_for_type(&ct).into_iter().chain(gio::AppInfo::all_for_type(&ct)) {
        if a.should_show() && !v.iter().any(|x| x.id() == a.id()) {
            v.push(a);
        }
    }
    (v, padrao)
}

fn icone_do_app(a: &gio::AppInfo, px: i32) -> gtk::Image {
    let i = a.icon().map(|ic| gtk::Image::from_gicon(&ic)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable"));
    i.set_pixel_size(px);
    i
}

fn divisor_menu(c: &gtk::Box) {
    let div = caixa(Orientation::Horizontal, 0, "arq-menu-divisor");
    div.set_size_request(-1, 1);
    c.append(&div);
}

fn linha_info(k: &str, v: &str) -> (gtk::Box, gtk::Label) {
    let l = caixa(Orientation::Horizontal, 8, "arq-dialogo-linha");
    let a = rotulo(k, "arq-sub");
    a.set_hexpand(true);
    a.set_xalign(0.0);
    l.append(&a);
    let b = rotulo(v, "arq-dialogo-valor");
    b.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    b.set_max_width_chars(30);
    b.set_xalign(1.0);
    l.append(&b);
    (l, b)
}

/// A folder's content, counted off the interface thread: items and bytes (stops at 50 000 entries).
fn medir_pasta(p: &Path) -> (usize, u64) {
    let (mut n, mut t) = (0usize, 0u64);
    let mut pilha = vec![p.to_path_buf()];
    while let Some(d) = pilha.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            n += 1;
            if n > 50_000 {
                return (n, t);
            }
            if let Ok(m) = e.metadata() {
                if m.is_dir() {
                    pilha.push(e.path());
                } else {
                    t += m.len();
                }
            }
        }
    }
    (n, t)
}

/// Whether two files have the same content (size, then SHA-256 for files under 256 MB).
fn mesmo_conteudo(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
    if ma.len() != mb.len() || ma.is_dir() || mb.is_dir() || ma.len() > 256 << 20 {
        return false;
    }
    let h = |p: &Path| std::fs::read(p).ok().and_then(|d| glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &d)).map(|s| s.to_string());
    h(a).is_some() && h(a) == h(b)
}

impl Arquivos {
    // ---- dialogs

    /// A dialog over the window: the window dims, the card comes in; Esc, its own button or a click on the dimmed
    /// part close it. Returns the card and how to close it.
    pub(super) fn dialogo(self: &Rc<Self>, largura: i32) -> (gtk::Box, Rc<dyn Fn()>) {
        if let Some(f) = self.fechar_dialogo.borrow_mut().take() {
            f();
        }
        let escuro = caixa(Orientation::Vertical, 0, "arq-escurecer");
        escuro.set_hexpand(true);
        escuro.set_vexpand(true);
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.set_size_request(largura, -1);
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        self.sobre.add_overlay(&escuro);
        self.sobre.add_overlay(&card);
        let (sw, ew, cw, wk) = (self.sobre.downgrade(), escuro.downgrade(), card.downgrade(), Rc::downgrade(self));
        let fechar: Rc<dyn Fn()> = Rc::new(move || {
            if let (Some(s), Some(e), Some(c)) = (sw.upgrade(), ew.upgrade(), cw.upgrade()) {
                if c.parent().is_some() {
                    s.remove_overlay(&c);
                    s.remove_overlay(&e);
                }
            }
            if let Some(a) = wk.upgrade() {
                a.fechar_dialogo.borrow_mut().take();
                a.conteudo.grab_focus();
            }
        });
        let clique = gtk::GestureClick::new();
        let f = fechar.clone();
        clique.connect_released(move |_, _, _, _| f());
        escuro.add_controller(clique);
        *self.fechar_dialogo.borrow_mut() = Some(fechar.clone());
        (card, fechar)
    }

    fn titulo_dialogo(card: &gtk::Box, titulo: &str, sub: &str, fechar: &Rc<dyn Fn()>) -> gtk::Label {
        let topo = caixa(Orientation::Horizontal, 8, "");
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        let t = rotulo(titulo, "arq-dialogo-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(1);
        t.set_hexpand(true);
        col.append(&t);
        let s = rotulo(sub, "arq-sub");
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_max_width_chars(1);
        s.set_visible(!sub.is_empty());
        col.append(&s);
        topo.append(&col);
        let x = botao_icone("fechar", "#8E8F99", 14, "arq-dialogo-fechar", tr("Close (Esc)"));
        x.set_valign(Align::Start);
        let f = fechar.clone();
        x.connect_clicked(move |_| f());
        topo.append(&x);
        card.append(&topo);
        s
    }

    /// Properties (design "Propriedades do arquivo"): General (the name, where, when, tags, the agent's summary),
    /// Permissions (who can read, change and run it) and Open with (the default app for its kind).
    pub(super) fn propriedades(self: &Rc<Self>, p: &Path) {
        let i = self.itens.borrow().iter().find(|i| i.caminho == p).cloned().unwrap_or_else(|| ler_um(p));
        let (card, fechar) = self.dialogo(460);
        // tabs
        let topo = caixa(Orientation::Horizontal, 20, "");
        let pilha = gtk::Stack::new();
        pilha.set_vhomogeneous(false);
        pilha.set_transition_type(gtk::StackTransitionType::Crossfade);
        pilha.set_transition_duration(160);
        let mut abas: Vec<(&str, gtk::Button)> = Vec::new();
        let ids: Vec<(&str, &str)> = if i.pasta { vec![("geral", tr("General")), ("permissoes", tr("Permissions"))] } else { vec![("geral", tr("General")), ("permissoes", tr("Permissions")), ("abrir", tr("Open with"))] };
        for (id, nome) in &ids {
            let b = gtk::Button::new();
            b.add_css_class("arq-dialogo-aba");
            let col = caixa(Orientation::Vertical, 6, "");
            col.append(&rotulo(nome, ""));
            let linha = caixa(Orientation::Horizontal, 0, "arq-dialogo-aba-linha");
            linha.set_size_request(24, 2);
            linha.set_halign(Align::Center);
            col.append(&linha);
            b.set_child(Some(&col));
            topo.append(&b);
            abas.push((id, b));
        }
        topo.append(&espaco());
        let x = botao_icone("fechar", "#8E8F99", 14, "arq-dialogo-fechar", tr("Close (Esc)"));
        let f = fechar.clone();
        x.connect_clicked(move |_| f());
        topo.append(&x);
        card.append(&topo);
        card.append(&pilha);
        let abas = Rc::new(abas);
        let marcar = {
            let (abas, pilha) = (abas.clone(), pilha.downgrade());
            move |id: &str| {
                for (i, b) in abas.iter() {
                    if *i == id { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
                }
                if let Some(p) = pilha.upgrade() {
                    p.set_visible_child_name(id);
                }
            }
        };
        let marcar = Rc::new(marcar);
        for (id, b) in abas.iter() {
            let (m, id) = (marcar.clone(), id.to_string());
            b.connect_clicked(move |_| m(&id));
        }
        // ---- General
        let g = caixa(Orientation::Vertical, 12, "");
        let cab = caixa(Orientation::Horizontal, 14, "");
        let pv: gtk::Widget = if i.pasta { arte_pasta(&i.caminho, i.emblema, 52).upcast() } else { self.previa(&i, 52, 68, (52, 68, 0), 40) };
        cab.append(&centro(52, 68, &pv));
        let col = caixa(Orientation::Vertical, 6, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let nome = gtk::Entry::new();
        nome.add_css_class("arq-dialogo-nome");
        nome.set_text(&i.nome);
        nome.set_hexpand(true);
        col.append(&nome);
        let desc = if i.pasta { tr("Folder").to_string() } else { format!("{} · {}", descricao_tipo(&i), tamanho(i.tamanho)) };
        let sub = rotulo(&desc, "arq-sub");
        sub.set_xalign(0.0);
        col.append(&sub);
        cab.append(&col);
        g.append(&cab);
        let info = caixa(Orientation::Vertical, 8, "");
        let meta = std::fs::metadata(p).ok();
        let quando = |t: Option<std::time::SystemTime>| t.map(|t| data(segundos(Ok(t)), true)).unwrap_or_default();
        info.append(&linha_info(tr("Where"), &com_til(p.parent().unwrap_or(Path::new("/")))).0);
        if i.pasta {
            let (l, v) = linha_info(tr("Content"), "…");
            info.append(&l);
            let (vw, pp) = (v.downgrade(), p.to_path_buf());
            crate::lateral::em_segundo_plano(move || medir_pasta(&pp), move |(n, t)| {
                if let Some(v) = vw.upgrade() {
                    v.set_text(&format!("{} · {}", if n == 1 { tr("1 item").to_string() } else { trf("{n} items", &[("n", n.to_string())]) }, tamanho(t)));
                }
            });
        }
        for (k, v) in [(tr("Created"), quando(meta.as_ref().and_then(|m| m.created().ok()))), (tr("Modified"), quando(meta.as_ref().and_then(|m| m.modified().ok()))),
            (tr("Last opened"), quando(meta.as_ref().and_then(|m| m.accessed().ok())))] {
            if !v.is_empty() {
                info.append(&linha_info(k, &v).0);
            }
        }
        g.append(&info);
        // tags: the file's, and "add" (any of the person's)
        let tags = caixa(Orientation::Horizontal, 6, "");
        for e in &i.etiquetas {
            tags.append(&self.chip_etiqueta(e, false));
        }
        let mais = gtk::MenuButton::new();
        mais.add_css_class("arq-dialogo-chip");
        let mc = caixa(Orientation::Horizontal, 4, "");
        mc.append(&ui_icon("mais", "#B9BAC4", 12));
        mc.append(&rotulo(tr("add"), ""));
        mais.set_child(Some(&mc));
        let tp = gtk::Popover::new();
        tp.add_css_class("arq-menu");
        tp.set_has_arrow(false);
        let tl = caixa(Orientation::Vertical, 2, "");
        tl.set_size_request(200, -1);
        for e in etiquetas_conhecidas() {
            let tem = i.etiquetas.contains(&e.nome);
            let b = Self::item_menu(&tl, if tem { "check" } else { "nenhum" }, "#A5A1E2", &e.nome, "", "");
            let (wk, pp, n, f) = (Rc::downgrade(self), p.to_path_buf(), e.nome.clone(), fechar.clone());
            b.connect_clicked(move |_| {
                if let Some(a) = wk.upgrade() {
                    a.etiquetar(std::slice::from_ref(&pp), &n);
                    f();
                    let (w2, p2) = (Rc::downgrade(&a), pp.clone());
                    glib::timeout_add_local_once(Duration::from_millis(350), move || if let Some(a) = w2.upgrade() { a.propriedades(&p2) });
                }
            });
        }
        tp.set_child(Some(&tl));
        mais.set_popover(Some(&tp));
        tags.append(&mais);
        g.append(&tags);
        if let Ok(t) = std::fs::read_to_string(resumo_path(&i)) {
            let r = caixa(Orientation::Horizontal, 10, "arq-dialogo-resumo");
            let e = estrela(14, "#A5A1E2");
            e.set_valign(Align::Start);
            r.append(&e);
            let l = rotulo(t.trim(), "arq-resumo-texto");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            r.append(&l);
            g.append(&r);
        }
        pilha.add_named(&g, Some("geral"));
        // a new name, on Enter (the same rules as F2)
        let (wk, pp) = (Rc::downgrade(self), p.to_path_buf());
        nome.connect_activate(move |e| {
            let novo = e.text().trim().to_string();
            let Some(a) = wk.upgrade() else { return };
            if novo.is_empty() || novo.contains('/') {
                e.add_css_class("erro");
                return;
            }
            let destino = pp.with_file_name(&novo);
            if destino == pp {
                return;
            }
            if destino.exists() || std::fs::rename(&pp, &destino).is_err() {
                e.add_css_class("erro");
                return;
            }
            e.remove_css_class("erro");
            *a.selecionados.borrow_mut() = BTreeSet::from([destino.clone()]);
            if depurando() {
                eprintln!("debug arquivos: renomeado {novo}");
            }
            a.recarregar();
            let fechar = a.fechar_dialogo.borrow().clone();
            if let Some(f) = fechar {
                f();
            }
        });
        // ---- Permissions
        let pm = caixa(Orientation::Vertical, 10, "");
        if let Some(m) = &meta {
            let meu = std::fs::metadata(glib::home_dir()).map(|h| h.uid() == m.uid()).unwrap_or(false);
            let quem = if meu { trf("{name} (you)", &[("name", dono(m.uid()))]) } else { dono(m.uid()) };
            pm.append(&linha_info(tr("Owner"), &quem).0);
            pm.append(&linha_info(tr("Group"), &dono_grupo(m.gid())).0);
            let modo = m.permissions().mode();
            let interruptor = |texto: &str, sub: &str, bit: u32| {
                let l = caixa(Orientation::Horizontal, 12, "arq-dialogo-opcao");
                let col = caixa(Orientation::Vertical, 2, "");
                col.set_hexpand(true);
                let t = rotulo(texto, "arq-nome");
                t.set_xalign(0.0);
                col.append(&t);
                if !sub.is_empty() {
                    let s = rotulo(sub, "arq-sub");
                    s.set_xalign(0.0);
                    s.set_wrap(true);
                    s.set_max_width_chars(1);
                    col.append(&s);
                }
                l.append(&col);
                let s = crate::configuracoes::interruptor();
                s.set_active(modo & bit != 0);
                s.set_sensitive(meu);
                s.set_valign(Align::Center);
                let pp = p.to_path_buf();
                s.connect_state_set(move |_, on| {
                    if let Ok(m) = std::fs::metadata(&pp) {
                        let mut perm = m.permissions();
                        let modo = perm.mode();
                        perm.set_mode(if on { modo | bit } else { modo & !bit });
                        if let Err(e) = std::fs::set_permissions(&pp, perm) {
                            eprintln!("kalango-shell: permissões {}: {e}", pp.display());
                        } else if depurando() {
                            eprintln!("debug arquivos: permissões {:o}", if on { modo | bit } else { modo & !bit } & 0o777);
                        }
                    }
                    glib::Propagation::Proceed
                });
                l.append(&s);
                l
            };
            pm.append(&interruptor(tr("You can change it"), "", 0o200));
            pm.append(&interruptor(tr("Other people can read it"), tr("everyone with an account on this computer"), 0o004));
            if !i.pasta {
                pm.append(&interruptor(tr("Allow running it as a program"), tr("only for programs and scripts you trust"), 0o100));
            }
            if !meu {
                let n = rotulo(tr("Only its owner can change these."), "arq-sub");
                n.set_xalign(0.0);
                pm.append(&n);
            }
        }
        pilha.add_named(&pm, Some("permissoes"));
        // ---- Open with: the default app for this kind of file
        if !i.pasta {
            let ab = caixa(Orientation::Vertical, 4, "");
            let (apps, padrao) = apps_para(&i.tipo);
            let s = rotulo(&trf("Opens files of the kind “{kind}”", &[("kind", descricao_tipo(&i))]), "arq-sub");
            s.set_xalign(0.0);
            s.set_margin_bottom(6);
            ab.append(&s);
            let ct = gio::content_type_from_mime_type(&i.tipo).unwrap_or_else(|| i.tipo.clone().into());
            let grupo: Option<gtk::CheckButton> = None;
            let mut primeiro: Option<gtk::CheckButton> = grupo;
            for a in apps.iter().take(8) {
                let r = gtk::CheckButton::new();
                r.add_css_class("arq-dialogo-app");
                let c = caixa(Orientation::Horizontal, 10, "");
                c.append(&icone_do_app(a, 24));
                let n = rotulo(&a.display_name(), "arq-nome");
                n.set_xalign(0.0);
                n.set_hexpand(true);
                c.append(&n);
                if padrao.as_ref().is_some_and(|d| d.id() == a.id()) {
                    c.append(&rotulo(tr("default"), "arq-dialogo-selo"));
                    r.set_active(true);
                }
                r.set_child(Some(&c));
                if let Some(p0) = &primeiro {
                    r.set_group(Some(p0));
                } else {
                    primeiro = Some(r.clone());
                }
                let (a2, ct2) = (a.clone(), ct.clone());
                r.connect_toggled(move |r| {
                    if r.is_active() {
                        if let Err(e) = a2.set_as_default_for_type(&ct2) {
                            eprintln!("kalango-shell: app padrão: {e}");
                        } else if depurando() {
                            eprintln!("debug arquivos: padrão para {ct2}: {}", a2.display_name());
                        }
                    }
                });
                ab.append(&r);
            }
            if apps.is_empty() {
                let n = rotulo(tr("No app installed opens this kind of file."), "arq-sub");
                n.set_xalign(0.0);
                ab.append(&n);
            }
            pilha.add_named(&ab, Some("abrir"));
        }
        marcar("geral");
        nome.grab_focus();
        let fim = i.nome.rfind('.').filter(|k| *k > 0 && !i.pasta).map(|k| i.nome[..k].chars().count() as i32).unwrap_or(-1);
        nome.select_region(0, fim);
        if depurando() {
            eprintln!("debug arquivos: propriedades {}", i.nome);
        }
    }

    /// "Abrir com" (design "Abrir com"): the apps for this kind (the default one recommended), the terminal for text,
    /// the agent; "always use this app" makes it the default for the kind.
    pub(super) fn abrir_com(self: &Rc<Self>, p: &Path) {
        let i = self.itens.borrow().iter().find(|i| i.caminho == p).cloned().unwrap_or_else(|| ler_um(p));
        let (card, fechar) = self.dialogo(440);
        let sub = Self::titulo_dialogo(&card, &trf("How to open “{name}”?", &[("name", i.nome.clone())]), &format!("{} · {}", descricao_tipo(&i), tamanho(i.tamanho)), &fechar);
        // a text's lines, counted in the background
        if i.tipo.starts_with("text/") && i.tamanho < 64 << 20 {
            let (sw, pp, base) = (sub.downgrade(), p.to_path_buf(), sub.text().to_string());
            crate::lateral::em_segundo_plano(move || std::fs::read(&pp).map(|d| d.iter().filter(|b| **b == b'\n').count()).unwrap_or(0), move |n| {
                if let (Some(s), true) = (sw.upgrade(), n > 0) {
                    s.set_text(&format!("{base} · {}", trf("{n} lines", &[("n", n.to_string())])));
                }
            });
        }
        let lista = caixa(Orientation::Vertical, 2, "");
        let (apps, padrao) = apps_para(&i.tipo);
        let escolha: Rc<RefCell<Option<gio::AppInfo>>> = Rc::new(RefCell::new(apps.first().cloned()));
        let mut primeiro: Option<gtk::ToggleButton> = None;
        let abrir_b = gtk::Button::with_label(tr("Open"));
        let linha = |icone: gtk::Widget, titulo: &str, sub: &str| -> gtk::ToggleButton {
            let r = gtk::ToggleButton::new();
            r.add_css_class("arq-dialogo-escolha");
            let c = caixa(Orientation::Horizontal, 12, "");
            c.append(&centro(24, 24, &icone));
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            let t = rotulo(titulo, "arq-nome");
            t.set_xalign(0.0);
            col.append(&t);
            if !sub.is_empty() {
                let s = rotulo(sub, "arq-sub");
                s.set_xalign(0.0);
                col.append(&s);
            }
            c.append(&col);
            r.set_child(Some(&c));
            r
        };
        for a in apps.iter().take(6) {
            let rec = padrao.as_ref().is_some_and(|d| d.id() == a.id());
            let r = linha(icone_do_app(a, 24).upcast(), &a.display_name(), if rec { tr("recommended") } else { "" });
            if let Some(p0) = &primeiro {
                r.set_group(Some(p0));
            } else {
                r.set_active(true);
                primeiro = Some(r.clone());
            }
            let (e, a2) = (escolha.clone(), a.clone());
            r.connect_toggled(move |r| if r.is_active() { *e.borrow_mut() = Some(a2.clone()) });
            // a double click opens at once
            let duplo = gtk::GestureClick::new();
            let ab = abrir_b.downgrade();
            duplo.connect_released(move |_, n, _, _| if n == 2 { if let Some(b) = ab.upgrade() { b.emit_clicked() } });
            r.add_controller(duplo);
            lista.append(&r);
        }
        // text in the terminal (with less)
        let term = i.tipo.starts_with("text/").then_some(());
        let term_b = term.as_ref().map(|_| {
            let r = linha(ui_icon("terminal", "#B9BAC4", 18).upcast(), tr("Terminal"), tr("opens with less"));
            if let Some(p0) = &primeiro { r.set_group(Some(p0)) } else { r.set_active(true); primeiro = Some(r.clone()) }
            lista.append(&r);
            r
        });
        // the agent: what it is, without opening an app
        let agente = linha(estrela(14, "#A5A1E2").upcast(), tr("Ask the agent about this file"), tr("summarizes it and answers, without opening an app"));
        if let Some(p0) = &primeiro { agente.set_group(Some(p0)) } else { agente.set_active(true) }
        lista.append(&agente);
        // more apps, from the store (when there is one)
        if glib::find_program_in_path("gnome-software").is_some() {
            let b = gtk::Button::new();
            b.add_css_class("arq-dialogo-escolha");
            let c = caixa(Orientation::Horizontal, 12, "");
            c.append(&centro(24, 24, &ui_icon("apps-n", "#B9BAC4", 16)));
            let t = rotulo(tr("Look for an app in the store"), "arq-nome");
            t.set_xalign(0.0);
            t.set_hexpand(true);
            c.append(&t);
            c.append(&ui_icon("seta-direita", "#7A7B88", 12));
            b.set_child(Some(&c));
            let (ext, f) = (extensao(p), fechar.clone());
            b.connect_clicked(move |_| {
                f();
                let _ = std::process::Command::new("gnome-software").arg(format!("--search={ext}")).spawn();
            });
            lista.append(&b);
        }
        card.append(&lista);
        let ext = extensao(p);
        let sempre = gtk::CheckButton::with_label(&if ext.is_empty() { tr("Always use this app for this kind of file").to_string() } else { trf("Always use this app for .{ext} files", &[("ext", ext.clone())]) });
        sempre.add_css_class("arq-dialogo-check");
        card.append(&sempre);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_halign(Align::End);
        let cancelar = gtk::Button::with_label(tr("Cancel"));
        cancelar.add_css_class("arq-acao");
        abrir_b.add_css_class("arq-acao-principal");
        bs.append(&cancelar);
        bs.append(&abrir_b);
        card.append(&bs);
        let f = fechar.clone();
        cancelar.connect_clicked(move |_| f());
        let (wk, pp, f, ag, tb) = (Rc::downgrade(self), p.to_path_buf(), fechar.clone(), agente.clone(), term_b.clone());
        let (sem, tipo) = (sempre.clone(), i.tipo.clone());
        abrir_b.connect_clicked(move |_| {
            let Some(a) = wk.upgrade() else { return };
            f();
            if ag.is_active() {
                a.perguntar(std::slice::from_ref(&pp));
                return;
            }
            if tb.as_ref().is_some_and(|t| t.is_active()) {
                if let Some(t) = terminal() {
                    let _ = std::process::Command::new(t).arg("--").arg("less").arg(&pp).spawn();
                } else if let Some(s) = a.shell.upgrade() {
                    // (in KalangoOS's Terminal: a tab in the file's folder, less once its prompt is up)
                    let quoted = format!("'{}'", pp.display().to_string().replace('\'', "'\\''"));
                    crate::terminal::run(&s, pp.parent().map(Path::to_path_buf), &format!("less {quoted}"));
                }
                return;
            }
            let Some(app) = escolha.borrow().clone() else { return };
            if sem.is_active() {
                let ct = gio::content_type_from_mime_type(&tipo).unwrap_or_else(|| tipo.clone().into());
                let _ = app.set_as_default_for_type(&ct);
            }
            let ctx = WidgetExt::display(&a.janela).app_launch_context();
            if let Err(e) = app.launch(&[gio::File::for_path(&pp)], Some(&ctx)) {
                eprintln!("kalango-shell: abrir com {}: {e}", app.display_name());
            } else {
                gtk::RecentManager::default().add_item(&gio::File::for_path(&pp).uri());
                if depurando() {
                    eprintln!("debug arquivos: aberto com {}", app.display_name());
                }
            }
        });
        abrir_b.grab_focus();
        if depurando() {
            eprintln!("debug arquivos: abrir com {} ({} apps)", i.nome, apps.len());
        }
    }

    /// A paste or a drop: files whose names are already there are asked about first (design "Conflito de
    /// arquivos"), one at a time ("do the same for the others" answers all); then it goes on.
    pub(super) fn com_conflitos(self: &Rc<Self>, ps: Vec<PathBuf>, destino: PathBuf, mover: bool) {
        let conflitos: Vec<PathBuf> = ps.iter().filter(|p| p.file_name().is_some_and(|n| destino.join(n).exists()) && p.parent() != Some(destino.as_path())).cloned().collect();
        if conflitos.is_empty() {
            self.colar_ja(ps, destino, mover, HashMap::new());
            return;
        }
        self.perguntar_conflito(ps, destino, mover, conflitos, 0, HashMap::new());
    }

    fn perguntar_conflito(self: &Rc<Self>, ps: Vec<PathBuf>, destino: PathBuf, mover: bool, conflitos: Vec<PathBuf>, k: usize, decisoes: HashMap<PathBuf, Decisao>) {
        let Some(novo) = conflitos.get(k).cloned() else {
            self.colar_ja(ps, destino, mover, decisoes);
            return;
        };
        let nome = novo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let antigo = destino.join(&nome);
        let (card, fechar) = self.dialogo(620);
        let topo = caixa(Orientation::Horizontal, 14, "");
        let alerta = centro(44, 44, &ui_icon("alerta", "#E5C98C", 20));
        alerta.add_css_class("arq-dialogo-alerta");
        topo.append(&alerta);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let t = rotulo(&trf("“{name}” is already in {folder}", &[("name", nome.clone()), ("folder", nome_da_pasta(&destino))]), "arq-dialogo-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(1);
        col.append(&t);
        let s = rotulo(tr("What to do with this file?"), "arq-sub");
        s.set_xalign(0.0);
        col.append(&s);
        topo.append(&col);
        card.append(&topo);
        // the two, side by side: what is there and what comes
        let (ia, ib) = (ler_um(&antigo), ler_um(&novo));
        let lado = caixa(Orientation::Horizontal, 12, "");
        lado.set_homogeneous(true);
        for (it, titulo, chega) in [(&ia, tr("What is already there"), false), (&ib, if mover { tr("What you are moving") } else { tr("What you are copying") }, true)] {
            let c = caixa(Orientation::Horizontal, 12, "arq-conflito-lado");
            if chega {
                c.add_css_class("chega");
            }
            let pv: gtk::Widget = if it.pasta { arte_pasta(&it.caminho, it.emblema, 44).upcast() } else { self.previa(it, 52, 68, (52, 68, 0), 40) };
            c.append(&centro(52, 68, &pv));
            let tc = caixa(Orientation::Vertical, 3, "");
            tc.set_valign(Align::Center);
            tc.set_hexpand(true);
            let a = rotulo(titulo, "arq-sub");
            a.set_xalign(0.0);
            tc.append(&a);
            let mut tam = tamanho_ou_itens(it);
            if it.tipo.contains("pdf") {
                if let Some(n) = contar_paginas(&it.caminho) {
                    tam = format!("{tam} · {}", if n == 1 { tr("1 page").to_string() } else { trf("{n} pages", &[("n", n.to_string())]) });
                }
            }
            let b = rotulo(&tam, "arq-conflito-tamanho");
            b.set_xalign(0.0);
            tc.append(&b);
            let d = rotulo(&quando_hora(it.modificado), "arq-sub");
            d.set_xalign(0.0);
            tc.append(&d);
            let outro = if chega { &ia } else { &ib };
            if it.modificado > outro.modificado {
                let n = rotulo(tr("newer"), "arq-dialogo-selo");
                n.set_halign(Align::Start);
                tc.append(&n);
            }
            c.append(&tc);
            lado.append(&c);
        }
        card.append(&lado);
        // how they differ (on this computer: content, size, pages)
        let nota = caixa(Orientation::Horizontal, 10, "arq-dialogo-resumo");
        nota.append(&estrela(14, "#A5A1E2"));
        let nt = rotulo(tr("Comparing…"), "arq-resumo-texto");
        nt.set_xalign(0.0);
        nt.set_wrap(true);
        nt.set_max_width_chars(1);
        nt.set_hexpand(true);
        nota.append(&nt);
        card.append(&nota);
        let (ntw, a2, n2, ta, tb, pastas) = (nt.downgrade(), antigo.clone(), novo.clone(), ia.tamanho, ib.tamanho, ia.pasta || ib.pasta);
        crate::lateral::em_segundo_plano(move || (mesmo_conteudo(&a2, &n2), contar_paginas(&a2), contar_paginas(&n2)), move |(igual, pa, pb)| {
            let Some(l) = ntw.upgrade() else { return };
            let texto = if pastas {
                tr("Both are folders: Replace swaps the whole folder (the old one goes to the trash); Keep both makes a copy with another name.").to_string()
            } else if igual {
                tr("They are the same: identical content. You can skip it.").to_string()
            } else if let (Some(a), Some(b)) = (pa.filter(|_| pa != pb), pb) {
                if b > a { trf("They are not the same: the new one has {n} more pages.", &[("n", (b - a).to_string())]) } else { trf("They are not the same: the new one has {n} fewer pages.", &[("n", (a - b).to_string())]) }
            } else if tb > ta {
                trf("They are not the same: the new one is {size} larger.", &[("size", tamanho(tb - ta))])
            } else if ta > tb {
                trf("They are not the same: the new one is {size} smaller.", &[("size", tamanho(ta - tb))])
            } else {
                tr("They are not the same: same size, different content.").to_string()
            };
            l.set_text(&texto);
        });
        let resto = conflitos.len() - k - 1;
        let todos = gtk::CheckButton::with_label(&trf("Do the same for the other {n} conflicts", &[("n", resto.to_string())]));
        todos.add_css_class("arq-dialogo-check");
        todos.set_visible(resto > 0);
        card.append(&todos);
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_halign(Align::End);
        let pular = gtk::Button::with_label(tr("Skip"));
        pular.add_css_class("arq-acao-texto");
        let ambos = gtk::Button::with_label(tr("Keep both"));
        ambos.add_css_class("arq-acao");
        let substituir = gtk::Button::with_label(tr("Replace"));
        substituir.add_css_class("arq-acao-principal");
        substituir.set_tooltip_text(Some(tr("The one already there goes to the trash (it can come back).")));
        bs.append(&pular);
        bs.append(&ambos);
        bs.append(&substituir);
        card.append(&bs);
        for (b, d, nome_b) in [(&pular, Decisao::Pular, "pular"), (&ambos, Decisao::Ambos, "ambos"), (&substituir, Decisao::Substituir, "substituir")] {
            crate::central::posicao_debug(&format!("conflito {nome_b}"), b.upcast_ref());
            let (wk, f, ps, destino, conflitos, decisoes, todos) = (Rc::downgrade(self), fechar.clone(), ps.clone(), destino.clone(), conflitos.clone(), decisoes.clone(), todos.clone());
            b.connect_clicked(move |_| {
                let Some(a) = wk.upgrade() else { return };
                f();
                let mut decisoes = decisoes.clone();
                let ate = if todos.is_active() { conflitos.len() } else { k + 1 };
                for c in &conflitos[k..ate] {
                    decisoes.insert(c.clone(), d);
                }
                if depurando() {
                    eprintln!("debug arquivos: conflito {nome_b} ({} de {})", ate - k, conflitos.len());
                }
                let (w2, ps, destino, conflitos) = (Rc::downgrade(&a), ps.clone(), destino.clone(), conflitos.clone());
                // the next one comes after this one has gone (a short pause, so it reads as a new question)
                glib::timeout_add_local_once(Duration::from_millis(120), move || {
                    if let Some(a) = w2.upgrade() {
                        a.perguntar_conflito(ps, destino, mover, conflitos, ate, decisoes);
                    }
                });
            });
        }
        substituir.grab_focus();
        if depurando() {
            eprintln!("debug arquivos: conflito {nome} ({} de {})", k + 1, conflitos.len());
        }
    }

    // ---- actions the menus offer

    pub(super) fn alternar_ocultos(self: &Rc<Self>) {
        let on = !self.ocultos.get();
        self.ocultos.set(on);
        salvar_preferencia("ocultos", Value::Bool(on));
        if depurando() {
            eprintln!("debug arquivos: ocultos {on}");
        }
        self.recarregar();
    }

    /// A folder in the side bar's favourites (GTK's bookmarks, as other file managers), or out of them.
    pub(super) fn alternar_favorito(self: &Rc<Self>, p: &Path) {
        let path = bookmarks_path();
        let uri = gio::File::for_path(p).uri().to_string();
        let atual = std::fs::read_to_string(&path).unwrap_or_default();
        let tem = e_favorito(p);
        let novo: String = if tem {
            atual.lines().filter(|l| l.split(' ').next() != Some(uri.as_str())).map(|l| format!("{l}\n")).collect()
        } else {
            format!("{atual}{}{uri}\n", if atual.is_empty() || atual.ends_with('\n') { "" } else { "\n" })
        };
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&path, novo) {
            eprintln!("kalango-shell: favoritos: {e}");
        }
        if depurando() {
            eprintln!("debug arquivos: favorito {} {}", p.display(), !tem);
        }
        self.lateral_render();
    }

    fn abrir_no_terminal(&self, p: &Path) {
        let Some(t) = terminal() else {
            if let Some(s) = self.shell.upgrade() {
                crate::terminal::open(&s, Some(p.to_path_buf()));
            }
            return;
        };
        let mut c = std::process::Command::new(&t);
        c.current_dir(p);
        if t.ends_with("gnome-terminal") || t.ends_with("ptyxis") {
            c.arg(format!("--working-directory={}", p.display()));
        }
        if let Err(e) = c.spawn() {
            eprintln!("kalango-shell: terminal: {e}");
        }
    }

    /// The chosen files in a .zip next to them (one: its name; several: "Arquivos.zip").
    fn compactar(self: &Rc<Self>, sel: &[PathBuf]) {
        let Some(pai) = sel.first().and_then(|p| p.parent()).map(Path::to_path_buf) else { return };
        let base = match sel {
            [um] => um.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            _ => tr("Archive").to_string(),
        };
        let zip = nome_livre(&pai, &format!("{base}.zip"));
        let nomes: Vec<String> = sel.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            std::process::Command::new("zip").arg("-rqy").arg(&zip).args(&nomes).current_dir(&pai).status().map(|s| s.success()).unwrap_or(false).then_some(zip)
        }, move |r| {
            let Some(a) = w.upgrade() else { return };
            match r {
                Some(z) => {
                    if depurando() {
                        eprintln!("debug arquivos: compactado {}", z.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    }
                    *a.selecionados.borrow_mut() = BTreeSet::from([z]);
                }
                None => eprintln!("kalango-shell: compactar: o zip falhou"),
            }
            a.recarregar();
        });
    }

    /// An archive's content in a folder of its name, next to it.
    fn extrair(self: &Rc<Self>, p: &Path) {
        let Some(pai) = p.parent().map(Path::to_path_buf) else { return };
        let destino = nome_livre(&pai, &nome_sem_extensao(p));
        let (pp, zip) = (p.to_path_buf(), extensao(p) == "zip");
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            if std::fs::create_dir_all(&destino).is_err() {
                return None;
            }
            let ok = if zip {
                std::process::Command::new("unzip").arg("-q").arg(&pp).arg("-d").arg(&destino).status()
            } else {
                std::process::Command::new("tar").arg("-xf").arg(&pp).arg("-C").arg(&destino).status()
            }.map(|s| s.success()).unwrap_or(false);
            if !ok {
                let _ = std::fs::remove_dir_all(&destino);
            }
            ok.then_some(destino)
        }, move |r| {
            let Some(a) = w.upgrade() else { return };
            match r {
                Some(d) => {
                    if depurando() {
                        eprintln!("debug arquivos: extraído em {}", d.display());
                    }
                    *a.selecionados.borrow_mut() = BTreeSet::from([d]);
                }
                None => eprintln!("kalango-shell: extrair: falhou"),
            }
            a.recarregar();
        });
    }

    fn usar_como_papel(&self, p: &Path) {
        let mut t = crate::tema::ler();
        let s = p.display().to_string();
        t.papeis.retain(|x| *x != s);
        t.papeis.insert(0, s.clone());
        t.papeis.truncate(5);
        t.papel = Some(s);
        crate::tema::salvar(&t);
        crate::tema::aplicar(&t);
        if depurando() {
            eprintln!("debug arquivos: papel de parede {}", p.display());
        }
    }

    fn esvaziar_lixeira(self: &Rc<Self>) {
        let lx = lixeira().join("files");
        let todos: Vec<PathBuf> = std::fs::read_dir(&lx).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        let n = todos.len();
        self.apagar_de_vez(&todos);
        if depurando() {
            eprintln!("debug arquivos: lixeira esvaziada ({n})");
        }
    }

    fn restaurar_tudo(self: &Rc<Self>) {
        let lx = lixeira().join("files");
        let mut n = 0;
        for p in std::fs::read_dir(&lx).map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default() {
            if restaurar(&p).is_ok() {
                n += 1;
            }
        }
        if depurando() {
            eprintln!("debug arquivos: restaurados {n}");
        }
        self.recarregar();
    }

    fn novo_de_modelo(self: &Rc<Self>, modelo: &Path) {
        let nome = modelo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let p = nome_livre(&self.pasta.borrow(), &nome);
        if copiar(modelo, &p).is_ok() {
            *self.selecionados.borrow_mut() = BTreeSet::from([p]);
            self.recarregar();
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(500), move || if let Some(a) = w.upgrade() { a.renomear() });
        }
    }

    // ---- the menus

    /// A submenu: its own page in the menu, with a back row on top.
    fn submenu(pilha: &gtk::Stack, nome: &str, titulo: &str) -> gtk::Box {
        let cc = caixa(Orientation::Vertical, 2, "");
        cc.set_size_request(280, -1);
        let voltar = Self::item_menu(&cc, "seta-esquerda", "#8E8F99", titulo, "", "arq-menu-voltar");
        let pl = pilha.downgrade();
        voltar.connect_clicked(move |_| if let Some(p) = pl.upgrade() { p.set_visible_child_name("menu") });
        pilha.add_named(&cc, Some(nome));
        cc
    }

    fn item_sub(c: &gtk::Box, glifo: &str, t: &str, pilha: &gtk::Stack, nome: &'static str) -> gtk::Button {
        let b = Self::item_menu(c, glifo, "#E2E2E6", t, "", "");
        if let Some(l) = b.child().and_downcast::<gtk::Box>() {
            let ch = ui_icon("seta-direita", "#7A7B88", 12);
            ch.set_margin_end(2);
            l.append(&ch);
        }
        let pl = pilha.downgrade();
        b.connect_clicked(move |_| if let Some(p) = pl.upgrade() { p.set_visible_child_name(nome) });
        b
    }

    /// The menu of a file (design "Arquivos · menus completos", ARQUIVO): open, open with, copy, cut, rename, share,
    /// wallpaper, tags, compress or extract; the agent's actions; properties and the trash.
    pub(super) fn menu(self: &Rc<Self>, w: &gtk::Widget, x: f64, y: f64) {
        let pop = Self::popover_em(w, x, y);
        let pilha = gtk::Stack::new();
        pilha.set_vhomogeneous(false);
        pilha.set_hhomogeneous(false);
        pilha.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        pilha.set_transition_duration(180);
        let c = caixa(Orientation::Vertical, 2, "");
        c.set_size_request(280, -1);
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        let um = (sel.len() == 1).then(|| sel[0].clone());
        let fechar = {
            let pw = pop.downgrade();
            move || if let Some(p) = pw.upgrade() { p.popdown() }
        };
        let item = Self::item_menu;
        let acao = |b: &gtk::Button, f: Rc<dyn Fn(&Rc<Arquivos>)>| {
            let (wk, fc) = (Rc::downgrade(self), fechar.clone());
            b.connect_clicked(move |_| {
                fc();
                if let Some(a) = wk.upgrade() {
                    f(&a);
                }
            });
        };
        if na_lixeira(&self.pasta.borrow()) {
            let restaurar_b = item(&c, "desfazer", "#E2E2E6", tr("Restore"), "", "");
            let apagar_b = item(&c, "lixeira", "#EFA2A6", tr("Delete forever"), "Del", "perigo");
            let s = sel.clone();
            acao(&restaurar_b, Rc::new(move |a| {
                for p in &s {
                    if let Err(e) = restaurar(p) {
                        eprintln!("kalango-shell: restaurar {}: {e}", p.display());
                    }
                }
                a.recarregar();
            }));
            let s = sel.clone();
            acao(&apagar_b, Rc::new(move |a| a.apagar_de_vez(&s)));
            pilha.add_named(&c, Some("menu"));
            pop.set_child(Some(&pilha));
            pop.popup();
            return;
        }
        let pasta = um.as_ref().is_some_and(|p| p.is_dir());
        let item_um = um.as_ref().map(|p| self.itens.borrow().iter().find(|i| i.caminho == *p).cloned().unwrap_or_else(|| ler_um(p)));
        let imagem = item_um.as_ref().is_some_and(|i| e_imagem(&i.tipo));
        let abrir = item(&c, "pasta", "#E2E2E6", tr("Open"), "↵", "");
        let abrir_com_b = if um.is_some() && !pasta { Some(Self::item_sub(&c, "apps-n", tr("Open with"), &pilha, "abrir-com")) } else { None };
        let nova_aba = if pasta { Some(item(&c, "pasta", "#E2E2E6", tr("Open in a new tab"), "Ctrl+↵", "")) } else { None };
        divisor_menu(&c);
        let copiar_b = item(&c, "copiar", "#E2E2E6", tr("Copy"), "Ctrl+C", "");
        let recortar_b = item(&c, "tesoura", "#E2E2E6", tr("Cut"), "Ctrl+X", "");
        let renomear_b = item(&c, "lapis", "#E2E2E6", tr("Rename"), "F2", "");
        let compartilhar = Self::item_sub(&c, "compartilhar", tr("Share"), &pilha, "compartilhar");
        let papel_b = if imagem { Some(item(&c, "imagem", "#E2E2E6", tr("Set as wallpaper"), "", "")) } else { None };
        let etiquetas_b = Self::item_sub(&c, "estrela-n", tr("Tags"), &pilha, "etiquetas");
        let arquivo_compactado = um.as_ref().filter(|p| e_compactado(p)).cloned();
        let extrair_b = arquivo_compactado.as_ref().map(|_| item(&c, "pasta", "#E2E2E6", tr("Extract here"), "", ""));
        let compactar_b = item(&c, "doc", "#E2E2E6", tr("Compress (.zip)"), "", "");
        divisor_menu(&c);
        let cab = caixa(Orientation::Horizontal, 5, "arq-menu-secao");
        cab.append(&estrela(13, "#A5A1E2"));
        cab.append(&rotulo(tr("Agent"), "arq-menu-secao-texto"));
        c.append(&cab);
        let resumir = item(&c, "spark", "#A5A1E2", tr("Summarize"), "", "");
        let perguntar = item(&c, "spark", "#A5A1E2", tr("Ask about the file"), tr("Super+Space"), "agente");
        let renomear_ia = item(&c, "spark", "#A5A1E2", tr("Rename with AI"), "", "");
        let organizar = item(&c, "spark", "#A5A1E2", tr("Organise this folder"), "", "");
        divisor_menu(&c);
        let propriedades_b = item(&c, "info", "#E2E2E6", tr("Properties"), "Alt+↵", "");
        let lixo = item(&c, "lixeira", "#EFA2A6", tr("Move to the trash"), "Del", "perigo");
        for (b, n) in [(&abrir, "abrir"), (&compartilhar, "compartilhar"), (&lixo, "lixeira"), (&perguntar, "perguntar"), (&resumir, "resumir"), (&organizar, "organizar"),
            (&renomear_ia, "renomear"), (&propriedades_b, "propriedades"), (&etiquetas_b, "etiquetas"), (&compactar_b, "compactar")] {
            crate::central::posicao_debug(&format!("menu {n}"), b.upcast_ref());
        }
        if let Some(b) = &abrir_com_b {
            crate::central::posicao_debug("menu abrir com", b.upcast_ref());
        }
        if let Some(b) = &extrair_b {
            crate::central::posicao_debug("menu extrair", b.upcast_ref());
        }
        pilha.add_named(&c, Some("menu"));
        // Open with: the apps for its kind, then "another app…" (the dialog)
        if let (Some(p), Some(i), false) = (um.clone(), item_um.clone(), pasta) {
            let cc = Self::submenu(&pilha, "abrir-com", tr("Open with"));
            let (apps, padrao) = apps_para(&i.tipo);
            for a in apps.iter().take(5) {
                let b = gtk::Button::new();
                b.add_css_class("arq-menu-item");
                let l = caixa(Orientation::Horizontal, 10, "");
                let ic = icone_do_app(a, 16);
                ic.set_margin_start(12);
                l.append(&ic);
                let t = rotulo(&a.display_name(), "");
                t.set_xalign(0.0);
                t.set_hexpand(true);
                l.append(&t);
                if padrao.as_ref().is_some_and(|d| d.id() == a.id()) {
                    l.append(&rotulo(tr("default"), "arq-menu-atalho"));
                }
                b.set_child(Some(&l));
                cc.append(&b);
                let (a2, pp, fc, wk) = (a.clone(), p.clone(), fechar.clone(), Rc::downgrade(self));
                b.connect_clicked(move |_| {
                    fc();
                    let Some(a) = wk.upgrade() else { return };
                    let ctx = WidgetExt::display(&a.janela).app_launch_context();
                    if let Err(e) = a2.launch(&[gio::File::for_path(&pp)], Some(&ctx)) {
                        eprintln!("kalango-shell: abrir com: {e}");
                    }
                });
            }
            let outro = item(&cc, "mais", "#E2E2E6", tr("Another app…"), "", "");
            crate::central::posicao_debug("menu outro app", outro.upcast_ref());
            let pp = p.clone();
            acao(&outro, Rc::new(move |a| a.abrir_com(&pp)));
        }
        // Share: outside this window
        let cc = Self::submenu(&pilha, "compartilhar", tr("Share"));
        let copiar_fora = item(&cc, "copiar", "#E2E2E6", tr("Copy (to paste in another app)"), "Ctrl+C", "");
        let caminho_b = item(&cc, "codigo", "#E2E2E6", tr("Copy the path"), "", "");
        let email_b = item(&cc, "enviar", "#E2E2E6", tr("Send by email"), "", "");
        email_b.set_visible(glib::find_program_in_path("xdg-email").is_some());
        let (s, f) = (sel.clone(), fechar.clone());
        copiar_fora.connect_clicked(move |b| {
            f();
            let files: Vec<gio::File> = s.iter().map(gio::File::for_path).collect();
            b.clipboard().set_content(Some(&gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))).ok();
        });
        let (s, f) = (sel.clone(), fechar.clone());
        caminho_b.connect_clicked(move |b| {
            f();
            b.clipboard().set_text(&s.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        });
        let (s, f) = (sel.clone(), fechar.clone());
        email_b.connect_clicked(move |_| {
            f();
            let mut c = std::process::Command::new("xdg-email");
            for p in &s {
                c.arg("--attach").arg(p);
            }
            let _ = c.spawn();
        });
        // Tags: each of the person's, marked when all the chosen files have it
        let ce = Self::submenu(&pilha, "etiquetas", tr("Tags"));
        for e in etiquetas_conhecidas() {
            let todas = !sel.is_empty() && sel.iter().all(|p| etiquetas_de(p).contains(&e.nome));
            let b = item(&ce, if todas { "check" } else { "nenhum" }, "#A5A1E2", &e.nome, "", "");
            crate::central::posicao_debug(&format!("menu etiqueta {}", e.nome), b.upcast_ref());
            let (s, n) = (sel.clone(), e.nome.clone());
            acao(&b, Rc::new(move |a| a.etiquetar(&s, &n)));
        }
        pop.set_child(Some(&pilha));
        // the actions
        let u = um.clone();
        acao(&abrir, Rc::new(move |a| if let Some(p) = &u { a.abrir(p) }));
        if let Some(b) = &nova_aba {
            let u = um.clone();
            acao(b, Rc::new(move |a| if let Some(p) = &u { a.nova_aba(p) }));
        }
        acao(&copiar_b, Rc::new(|a| a.copiar_selecao(false)));
        acao(&recortar_b, Rc::new(|a| a.copiar_selecao(true)));
        acao(&renomear_b, Rc::new(|a| a.renomear()));
        if let Some(b) = &papel_b {
            let u = um.clone();
            acao(b, Rc::new(move |a| if let Some(p) = &u { a.usar_como_papel(p) }));
        }
        if let (Some(b), Some(p)) = (&extrair_b, arquivo_compactado) {
            acao(b, Rc::new(move |a| a.extrair(&p)));
        }
        let s = sel.clone();
        acao(&compactar_b, Rc::new(move |a| a.compactar(&s)));
        let s = sel.clone();
        acao(&lixo, Rc::new(move |a| a.lixeira(&s)));
        let s = sel.clone();
        acao(&perguntar, Rc::new(move |a| a.perguntar(&s)));
        let u = um.clone();
        acao(&resumir, Rc::new(move |a| {
            if let Some(p) = &u {
                let it = a.itens.borrow().iter().find(|i| i.caminho == *p).cloned().unwrap_or_else(|| ler_um(p));
                let o = a.dups.borrow().get(p).cloned();
                a.resumir(&it, o.as_deref());
            }
        }));
        acao(&organizar, Rc::new(|a| a.organizar()));
        let u = um.clone();
        acao(&renomear_ia, Rc::new(move |a| {
            if let Some(p) = &u {
                a.pedir_ao_agente(&trf("Read the file {path} and rename it, in the same folder, to a short, clear name that says what it is (keep the extension).", &[("path", p.display().to_string())]), 1);
            }
        }));
        let u = um.clone();
        acao(&propriedades_b, Rc::new(move |a| if let Some(p) = &u { a.propriedades(p) }));
        abrir.set_sensitive(um.is_some());
        renomear_b.set_sensitive(um.is_some());
        renomear_ia.set_sensitive(um.as_ref().is_some_and(|p| p.is_file()));
        resumir.set_sensitive(um.as_ref().is_some_and(|p| p.is_file()));
        propriedades_b.set_sensitive(um.is_some());
        pop.popup();
    }

    /// The menu of the folder itself (a right click where there is no file; design "Arquivos · menus completos",
    /// PASTA): new folder or document, paste, select all, hidden files, the terminal, favourites, the agent, its
    /// properties. In the trash: restore everything, empty it.
    pub(super) fn menu_da_pasta(self: &Rc<Self>, w: &gtk::Widget, x: f64, y: f64) {
        let pop = Self::popover_em(w, x, y);
        let pilha = gtk::Stack::new();
        pilha.set_vhomogeneous(false);
        pilha.set_hhomogeneous(false);
        pilha.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        pilha.set_transition_duration(180);
        let c = caixa(Orientation::Vertical, 2, "");
        c.set_size_request(280, -1);
        let item = Self::item_menu;
        let fechar = {
            let pw = pop.downgrade();
            move || if let Some(p) = pw.upgrade() { p.popdown() }
        };
        let acao = |b: &gtk::Button, f: Rc<dyn Fn(&Rc<Arquivos>)>| {
            let (wk, fc) = (Rc::downgrade(self), fechar.clone());
            b.connect_clicked(move |_| {
                fc();
                if let Some(a) = wk.upgrade() {
                    f(&a);
                }
            });
        };
        let pasta = self.pasta.borrow().clone();
        if na_lixeira(&pasta) {
            let n = self.itens.borrow().len();
            let restaurar_b = item(&c, "desfazer", "#E2E2E6", tr("Restore everything"), "", "");
            let esvaziar = item(&c, "lixeira", "#EFA2A6", tr("Empty the trash"), "", "perigo");
            restaurar_b.set_sensitive(n > 0);
            esvaziar.set_sensitive(n > 0);
            crate::central::posicao_debug("menu esvaziar", esvaziar.upcast_ref());
            acao(&restaurar_b, Rc::new(|a| a.restaurar_tudo()));
            // emptying asks once more, in place: the row says what it will do
            let confirmado = Rc::new(Cell::new(false));
            let (wk, fc, cf) = (Rc::downgrade(self), fechar.clone(), confirmado.clone());
            esvaziar.connect_clicked(move |b| {
                if !cf.replace(true) {
                    if let Some(l) = b.child().and_downcast::<gtk::Box>().and_then(|l| l.first_child()).and_then(|i| i.next_sibling()).and_downcast::<gtk::Label>() {
                        l.set_text(&trf("Delete {n} items forever?", &[("n", n.to_string())]));
                    }
                    b.add_css_class("confirmar");
                    return;
                }
                fc();
                if let Some(a) = wk.upgrade() {
                    a.esvaziar_lixeira();
                }
            });
            pilha.add_named(&c, Some("menu"));
            pop.set_child(Some(&pilha));
            pop.popup();
            return;
        }
        let nova = item(&c, "pasta", "#E2E2E6", tr("New folder"), "Ctrl+Shift+N", "");
        let _novo_doc = Self::item_sub(&c, "doc", tr("New document"), &pilha, "novo");
        let colar = item(&c, "colar", "#E2E2E6", tr("Paste"), "Ctrl+V", "");
        divisor_menu(&c);
        let tudo = item(&c, "check", "#E2E2E6", tr("Select all"), "Ctrl+A", "");
        let ocultos = item(&c, if self.ocultos.get() { "check" } else { "olho" }, "#E2E2E6", if self.ocultos.get() { tr("Hide hidden files") } else { tr("Show hidden files") }, "Ctrl+H", "");
        let term = item(&c, "terminal", "#E2E2E6", tr("Open in the terminal"), "", "");
        let fav = e_favorito(&pasta);
        let favorito = item(&c, "estrela-n", "#E2E2E6", if fav { tr("Remove from favourites") } else { tr("Add to favourites") }, "Ctrl+D", "");
        divisor_menu(&c);
        let cab = caixa(Orientation::Horizontal, 5, "arq-menu-secao");
        cab.append(&estrela(13, "#A5A1E2"));
        cab.append(&rotulo(tr("Agent"), "arq-menu-secao-texto"));
        c.append(&cab);
        let organizar = item(&c, "spark", "#A5A1E2", tr("Organise this folder"), "", "");
        divisor_menu(&c);
        let props = item(&c, "info", "#E2E2E6", tr("Folder properties"), "Alt+↵", "");
        for (b, n) in [(&organizar, "pasta organizar"), (&ocultos, "ocultos"), (&favorito, "favorito"), (&props, "pasta propriedades"), (&nova, "nova pasta")] {
            crate::central::posicao_debug(&format!("menu {n}"), b.upcast_ref());
        }
        pilha.add_named(&c, Some("menu"));
        // New document: a text one, and the person's templates (~/Modelos)
        let cn = Self::submenu(&pilha, "novo", tr("New document"));
        let texto = item(&cn, "doc", "#E2E2E6", tr("Text document"), "", "");
        acao(&texto, Rc::new(|a| a.novo_documento()));
        if let Some(t) = glib::user_special_dir(glib::UserDirectory::Templates).filter(|t| t.is_dir() && *t != glib::home_dir()) {
            for e in std::fs::read_dir(&t).into_iter().flatten().flatten().take(12) {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let nome = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let b = item(&cn, "doc", "#E2E2E6", &nome, "", "");
                acao(&b, Rc::new(move |a| a.novo_de_modelo(&p)));
            }
        }
        pop.set_child(Some(&pilha));
        acao(&nova, Rc::new(|a| a.nova_pasta()));
        acao(&colar, Rc::new(|a| a.colar()));
        acao(&tudo, Rc::new(|a| {
            *a.selecionados.borrow_mut() = a.ordem_na_tela().into_iter().collect();
            a.selecao_mudou();
        }));
        acao(&ocultos, Rc::new(|a| a.alternar_ocultos()));
        let p = pasta.clone();
        acao(&term, Rc::new(move |a| a.abrir_no_terminal(&p)));
        let p = pasta.clone();
        acao(&favorito, Rc::new(move |a| a.alternar_favorito(&p)));
        acao(&organizar, Rc::new(|a| a.organizar()));
        let p = pasta;
        acao(&props, Rc::new(move |a| a.propriedades(&p)));
        pop.popup();
    }
}

/// A group's name (from /etc/group).
fn dono_grupo(gid: u32) -> String {
    std::fs::read_to_string("/etc/group").unwrap_or_default().lines()
        .find(|l| l.split(':').nth(2).and_then(|u| u.parse::<u32>().ok()) == Some(gid))
        .and_then(|l| l.split(':').next().map(str::to_string)).unwrap_or_else(|| gid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_by_their_names() {
        assert!(e_compactado(Path::new("/x/fotos.tar.gz")) && e_compactado(Path::new("/x/a.ZIP")));
        assert!(!e_compactado(Path::new("/x/notas.md")));
        assert_eq!(nome_sem_extensao(Path::new("/x/fotos.tar.gz")), "fotos");
        assert_eq!(nome_sem_extensao(Path::new("/x/Relatório.zip")), "Relatório");
    }

    #[test]
    fn compress_and_extract_with_the_systems_tools() {
        let d = std::env::temp_dir().join(format!("kalango-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("pasta")).unwrap();
        std::fs::write(d.join("pasta/a.txt"), "oi").unwrap();
        let ok = std::process::Command::new("zip").arg("-rqy").arg("pasta.zip").arg("pasta").current_dir(&d).status().unwrap().success();
        assert!(ok);
        std::fs::create_dir_all(d.join("saida")).unwrap();
        assert!(std::process::Command::new("unzip").arg("-q").arg(d.join("pasta.zip")).arg("-d").arg(d.join("saida")).status().unwrap().success());
        assert_eq!(std::fs::read_to_string(d.join("saida/pasta/a.txt")).unwrap(), "oi");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn same_content_is_found() {
        let d = std::env::temp_dir().join(format!("kalango-igual-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), "igual").unwrap();
        std::fs::write(d.join("b"), "igual").unwrap();
        std::fs::write(d.join("c"), "outro").unwrap();
        assert!(mesmo_conteudo(&d.join("a"), &d.join("b")));
        assert!(!mesmo_conteudo(&d.join("a"), &d.join("c")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
