//! The file chooser of every app (designs "Abrir arquivo" and "Salvar como"), asked through the portal
//! (portal.rs: org.freedesktop.impl.portal.FileChooser): one chooser for the whole system, with Files' places, list,
//! icons, dates and previews (and the agent's summary of a file, when Files made one).
//! - Open: the places at the left (Recentes, Pasta pessoal, Documentos, Downloads, Imagens, Projetos, the drives),
//!   the folder's list or grid, the chosen file's preview; the field searches this folder as it is typed, and Enter
//!   searches the person's files by what they say ("a fatura de outubro"); the app's filters at the bottom. Enter
//!   opens, Esc cancels.
//! - Save: compact — the name already selected to type over, where (the person's folders), the format (the app's
//!   filters), tags (Files' tags, set on the file once the app writes it) and, when similar files live elsewhere,
//!   "Quer salvar lá?". "Mais opções" opens the whole browser.

use super::*;

/// An app's filter: its name, and the globs or MIME types it takes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Filtro {
    pub nome: String,
    pub globs: Vec<String>,
    pub mimes: Vec<String>,
}

impl Filtro {
    fn aceita(&self, i: &Item) -> bool {
        if i.pasta || (self.globs.is_empty() && self.mimes.is_empty()) {
            return true;
        }
        let nome = i.nome.to_lowercase();
        self.globs.iter().any(|g| glob(&g.to_lowercase(), &nome))
            // (a type's parents count: Markdown is text, as the app's "Arquivos de texto" means)
            || self.mimes.iter().any(|m| m == "*" || gio::content_type_is_a(&i.tipo, m) || (m.ends_with("/*") && i.tipo.starts_with(m.trim_end_matches('*'))))
    }
}

/// "*.pdf", "*.tar.*", "notas*.md": * and ? only.
fn glob(p: &str, s: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (p.chars().collect(), s.chars().collect());
    fn m(p: &[char], s: &[char]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some('*'), _) => m(&p[1..], s) || (!s.is_empty() && m(p, &s[1..])),
            (Some('?'), Some(_)) => m(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &s[1..]),
            _ => false,
        }
    }
    m(&p, &s)
}

#[derive(Clone, Debug)]
pub(crate) enum Modo {
    Abrir { varios: bool, pasta: bool },
    Salvar { nome: String },
}

pub(crate) struct Pedido {
    pub modo: Modo,
    pub titulo: String,
    /// the app, as the person knows it ("Editor de Texto")
    pub app: String,
    pub rotulo: Option<String>,
    pub filtros: Vec<Filtro>,
    pub filtro: Option<usize>,
    pub pasta: Option<PathBuf>,
}

/// The answer: the chosen files (or the file to write), and the filter in use.
pub(crate) type Resposta = Option<(Vec<PathBuf>, Option<usize>)>;

struct Seletor {
    janela: gtk::Window,
    pedido: Pedido,
    pasta: RefCell<PathBuf>,
    itens: RefCell<Vec<Item>>,
    escolhido: RefCell<Option<PathBuf>>,
    lista: gtk::ListBox,
    grade: gtk::FlowBox,
    vistas: gtk::Stack,
    migalha: gtk::Box,
    lugares: gtk::Box,
    previa: gtk::Box,
    busca: gtk::Entry,
    filtro: gtk::DropDown,
    nome: gtk::Entry,
    aceitar: gtk::Button,
    em_grade: Cell<bool>,
    buscando: RefCell<Option<Vec<Item>>>,
    etapas: gtk::Stack,
    onde: gtk::DropDown,
    onde_pastas: RefCell<Vec<PathBuf>>,
    etiquetas: RefCell<Vec<String>>,
    sugestao: gtk::Box,
    feito: RefCell<Option<Box<dyn FnOnce(Resposta)>>>,
}

fn pastas_da_pessoa() -> Vec<(String, PathBuf, &'static str)> {
    let casa = glib::home_dir();
    let mut v = vec![(tr("Recent").to_string(), PathBuf::from("recent://"), "relogio"), (tr("Home folder").to_string(), casa.clone(), "casa")];
    // the person's folders (user-dirs), or the usual names when the profile has none
    for (d, g, nomes) in [(glib::UserDirectory::Documents, "doc", ["Documentos", "Documents"]), (glib::UserDirectory::Downloads, "download", ["Downloads", "Transferências"]),
        (glib::UserDirectory::Pictures, "imagem", ["Imagens", "Pictures"])] {
        let achada = glib::user_special_dir(d).filter(|p| p.is_dir() && *p != casa).or_else(|| nomes.iter().map(|n| casa.join(n)).find(|p| p.is_dir()));
        if let Some(p) = achada {
            let n = p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            v.push((n, p, g));
        }
    }
    for n in ["Projetos", "Projects", "repositorios"] {
        let p = casa.join(n);
        if p.is_dir() {
            v.push((tr("Projects").to_string(), p, "codigo"));
            break;
        }
    }
    v
}

/// The files most recently used, from the system's list (recently-used.xbel), newest first.
fn recentes() -> Vec<Item> {
    let Ok(t) = std::fs::read_to_string(glib::user_data_dir().join("recently-used.xbel")) else { return vec![] };
    let mut v: Vec<(String, PathBuf)> = Vec::new();
    for l in t.split("<bookmark ").skip(1) {
        let href = l.split("href=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("");
        let quando = l.split("modified=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("").to_string();
        if let Some(p) = gio::File::for_uri(href).path().filter(|p| p.is_file()) {
            v.push((quando, p));
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(40).map(|(_, p)| ler_um(&p)).collect()
}

impl Seletor {
    fn salvar(&self) -> bool {
        matches!(self.pedido.modo, Modo::Salvar { .. })
    }

    fn filtro_atual(&self) -> Option<Filtro> {
        let i = self.filtro.selected() as usize;
        self.pedido.filtros.get(i).cloned()
    }

    fn ir(self: &Rc<Self>, p: &Path) {
        *self.pasta.borrow_mut() = p.to_path_buf();
        *self.buscando.borrow_mut() = None;
        self.busca.set_text("");
        let mut itens = if p.as_os_str() == "recent://" { recentes() } else { ler_pasta(p, false) };
        if p.as_os_str() != "recent://" {
            itens.sort_by(|a, b| b.pasta.cmp(&a.pasta).then(a.nome.to_lowercase().cmp(&b.nome.to_lowercase())));
        }
        *self.itens.borrow_mut() = itens;
        if depurando() {
            eprintln!("debug seletor: {} ({} itens)", p.display(), self.itens.borrow().len());
        }
        self.render();
    }

    /// What shows: the folder (or the search's results) through the app's filter and the field's words.
    fn visiveis(&self) -> Vec<Item> {
        let f = self.filtro_atual();
        let q = self.busca.text().to_lowercase();
        let base = self.buscando.borrow().clone().unwrap_or_else(|| self.itens.borrow().clone());
        let so_pastas = matches!(self.pedido.modo, Modo::Abrir { pasta: true, .. });
        base.into_iter()
            .filter(|i| f.as_ref().is_none_or(|f| f.aceita(i)))
            .filter(|i| !so_pastas || i.pasta)
            .filter(|i| self.buscando.borrow().is_some() || q.is_empty() || i.nome.to_lowercase().contains(&q))
            .collect()
    }

    fn render(self: &Rc<Self>) {
        // the places: the current one lit
        let atual = self.pasta.borrow().clone();
        let mut c = self.lugares.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if w.widget_name().is_empty() {
                continue;
            }
            if Path::new(w.widget_name().as_str()) == atual {
                w.add_css_class("escolhido");
            } else {
                w.remove_css_class("escolhido");
            }
        }
        // the path: each folder a button (the last one lit)
        while let Some(w) = self.migalha.first_child() {
            self.migalha.remove(&w);
        }
        if self.buscando.borrow().is_some() {
            self.migalha.append(&rotulo(&trf("Results for “{q}”", &[("q", self.busca.text().to_string())]), "seletor-migalha-atual"));
        } else if atual.as_os_str() == "recent://" {
            self.migalha.append(&rotulo(tr("Recent"), "seletor-migalha-atual"));
        } else {
            let casa = glib::home_dir();
            let mut partes: Vec<PathBuf> = atual.ancestors().take_while(|p| p.starts_with(&casa) || *p == casa).map(Path::to_path_buf).collect();
            if partes.is_empty() {
                partes = atual.ancestors().map(Path::to_path_buf).collect();
            }
            partes.reverse();
            // a deep path: its start, "…", and the last folders (the bar never widens the window)
            if partes.len() > 4 {
                let fim = partes.split_off(partes.len() - 2);
                partes.truncate(1);
                self.migalha.append(&{
                    let b = gtk::Button::with_label(&if partes[0] == casa { tr("Home folder").to_string() } else { "/".to_string() });
                    b.add_css_class("seletor-migalha");
                    let (w, p) = (Rc::downgrade(self), partes[0].clone());
                    b.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.ir(&p) });
                    b
                });
                self.migalha.append(&ui_icon("chevron", "#8E8F99", 12));
                self.migalha.append(&rotulo("…", "seletor-meta"));
                partes = fim;
                self.migalha.append(&ui_icon("chevron", "#8E8F99", 12));
            }
            let n = partes.len();
            for (k, p) in partes.into_iter().enumerate() {
                if k > 0 {
                    self.migalha.append(&ui_icon("chevron", "#8E8F99", 12));
                }
                let nome = if p == casa { tr("Home folder").to_string() } else { p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()) };
                let b = gtk::Button::with_label(&nome);
                b.add_css_class("seletor-migalha");
                if k + 1 == n {
                    b.add_css_class("atual");
                }
                let w = Rc::downgrade(self);
                b.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.ir(&p) });
                self.migalha.append(&b);
            }
        }
        // the list and the grid
        while let Some(w) = self.lista.first_child() {
            self.lista.remove(&w);
        }
        while let Some(w) = self.grade.first_child() {
            self.grade.remove(&w);
        }
        let itens = self.visiveis();
        let escolhido = self.escolhido.borrow().clone();
        for i in &itens {
            let linha = self.linha(i);
            linha.set_widget_name(&i.caminho.to_string_lossy());
            self.lista.append(&linha);
            if escolhido.as_ref() == Some(&i.caminho) {
                if let Some(r) = self.lista.last_child().and_downcast::<gtk::ListBoxRow>() {
                    self.lista.select_row(Some(&r));
                }
            }
            self.grade.append(&self.icone(i));
        }
        if itens.is_empty() {
            let l = rotulo(if self.buscando.borrow().is_some() { tr("Nothing found with those words.") } else { tr("Nothing here for this app.") }, "seletor-vazio");
            l.set_margin_top(24);
            self.lista.append(&l);
        }
        self.vistas.set_visible_child_name(if self.em_grade.get() { "grade" } else { "lista" });
        self.previa_render();
        self.atualizar_aceitar();
        if depurando() {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                let Some(s) = w.upgrade() else { return };
                let mut r = s.lista.first_child();
                while let Some(x) = r {
                    if !x.widget_name().is_empty() {
                        let nome = Path::new(x.widget_name().as_str()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        crate::central::posicao_debug(&format!("seletor arquivo {nome}"), &x);
                    }
                    r = x.next_sibling();
                }
            });
        }
    }

    fn linha(self: &Rc<Self>, i: &Item) -> gtk::ListBoxRow {
        let r = caixa(Orientation::Horizontal, 10, "seletor-linha");
        let (g, _, cor) = if i.pasta { ("pasta", "", "#B9BAC4") } else { glifo_do_tipo(i) };
        r.append(&ui_icon(g, cor, 16));
        let n = rotulo(&i.nome, "seletor-nome");
        n.set_xalign(0.0);
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        r.append(&n);
        // a search's result says where it is
        if self.buscando.borrow().is_some() {
            let onde = i.caminho.parent().map(|p| p.strip_prefix(glib::home_dir()).map(|x| x.display().to_string()).unwrap_or_else(|_| p.display().to_string())).unwrap_or_default();
            let o = rotulo(&onde, "seletor-meta");
            o.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            o.set_max_width_chars(18);
            r.append(&o);
        }
        let m = rotulo(&quando_hora(i.modificado), "seletor-meta");
        m.set_xalign(0.0);
        m.set_size_request(120, -1);
        r.append(&m);
        let t = rotulo(&tamanho_ou_itens(i), "seletor-meta");
        t.set_xalign(1.0);
        t.set_size_request(70, -1);
        r.append(&t);
        // (a double click or Enter is the list's "row-activated": into a folder, or chosen)
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&r));
        row
    }

    fn icone(self: &Rc<Self>, i: &Item) -> gtk::Widget {
        let b = gtk::Button::new();
        b.add_css_class("seletor-icone");
        let v = caixa(Orientation::Vertical, 6, "");
        let (g, _, cor) = if i.pasta { ("pasta", "", "#B9BAC4") } else { glifo_do_tipo(i) };
        let ic = ui_icon(g, cor, 32);
        v.append(&ic);
        let n = rotulo(&i.nome, "seletor-nome");
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        n.set_max_width_chars(12);
        v.append(&n);
        b.set_child(Some(&v));
        let (w, it) = (Rc::downgrade(self), i.clone());
        b.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.escolher(&it) });
        let dois = gtk::GestureClick::new();
        let (w, it) = (Rc::downgrade(self), i.clone());
        dois.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                if let Some(s) = w.upgrade() {
                    s.ativar(&it);
                }
            }
        });
        b.add_controller(dois);
        b.upcast()
    }

    fn escolher(self: &Rc<Self>, i: &Item) {
        *self.escolhido.borrow_mut() = Some(i.caminho.clone());
        if self.salvar() && !i.pasta {
            self.nome.set_text(&i.nome);
        }
        if depurando() {
            eprintln!("debug seletor: escolhido {}", i.nome);
        }
        self.previa_render();
        self.atualizar_aceitar();
    }

    /// Double click or Enter on an item: a folder opens; a file is the answer.
    fn ativar(self: &Rc<Self>, i: &Item) {
        if i.pasta && !matches!(self.pedido.modo, Modo::Abrir { pasta: true, .. }) {
            *self.escolhido.borrow_mut() = None;
            self.ir(&i.caminho);
            return;
        }
        self.escolher(i);
        self.responder(true);
    }

    fn previa_render(self: &Rc<Self>) {
        while let Some(w) = self.previa.first_child() {
            self.previa.remove(&w);
        }
        let Some(p) = self.escolhido.borrow().clone() else {
            let l = rotulo(tr("Choose a file to see it here."), "seletor-meta");
            l.set_wrap(true);
            l.set_margin_top(24);
            self.previa.append(&l);
            return;
        };
        let i = ler_um(&p);
        let caixa_previa = gtk::Box::new(Orientation::Vertical, 0);
        caixa_previa.set_halign(Align::Center);
        caixa_previa.set_size_request(150, 196);
        let falsa = pagina_falsa(&i, 150, 196, false);
        caixa_previa.append(&falsa);
        self.previa.append(&caixa_previa);
        if !i.pasta && (e_imagem(&i.tipo) || i.tamanho < 256 << 20) {
            let (cw, fw) = (caixa_previa.downgrade(), falsa.downgrade());
            let (pp, tipo, mt) = (i.caminho.clone(), i.tipo.clone(), i.modificado);
            let alvo = if e_imagem(&i.tipo) { Alvo::Inteira(150, 196) } else { Alvo::Pagina(150, 196) };
            crate::lateral::em_segundo_plano(move || miniatura(&pp, &tipo, mt, alvo), move |r| {
                let (Some(c), Some(r)) = (cw.upgrade(), r) else { return };
                let t = textura(r);
                let img = gtk::Picture::for_paintable(&t);
                img.set_can_shrink(true);
                img.set_size_request(150, 196);
                img.add_css_class("seletor-pagina");
                if let Some(f) = fw.upgrade() {
                    c.remove(&f);
                }
                c.append(&img);
            });
        }
        let n = rotulo(&i.nome, "seletor-previa-nome");
        n.set_wrap(true);
        n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        n.set_justify(gtk::Justification::Center);
        self.previa.append(&n);
        let tipo = if i.pasta { tamanho_ou_itens(&i) } else { format!("{} · {}", descricao_tipo(&i), tamanho(i.tamanho)) };
        self.previa.append(&rotulo(&tipo, "seletor-meta"));
        // the agent's summary, when Files made one (by path and change: a file that changed is not described old)
        if let Ok(t) = std::fs::read_to_string(resumo_path(&i)) {
            let r = rotulo(&format!("{} {}", tr("Agent's summary:"), t.trim()), "seletor-resumo");
            r.set_wrap(true);
            r.set_xalign(0.0);
            r.set_max_width_chars(1);
            r.set_hexpand(true);
            self.previa.append(&r);
        }
    }

    fn atualizar_aceitar(&self) {
        let ok = match &self.pedido.modo {
            Modo::Salvar { .. } => !self.nome.text().trim().is_empty(),
            Modo::Abrir { pasta: true, .. } => true,
            Modo::Abrir { .. } => self.escolhido.borrow().as_ref().is_some_and(|p| p.is_file()),
        };
        self.aceitar.set_sensitive(ok);
    }

    /// The folder a save goes to: the compact one's "Onde", or the browser's folder.
    fn pasta_do_salvar(&self) -> PathBuf {
        if self.etapas.visible_child_name().as_deref() == Some("compacto") {
            if let Some(p) = self.onde_pastas.borrow().get(self.onde.selected() as usize) {
                return p.clone();
            }
        }
        let p = self.pasta.borrow().clone();
        if p.as_os_str() == "recent://" { glib::home_dir() } else { p }
    }

    fn responder(self: &Rc<Self>, sim: bool) {
        let Some(feito) = self.feito.borrow_mut().take() else { return };
        let filtro = (!self.pedido.filtros.is_empty()).then(|| self.filtro.selected() as usize);
        let r = if !sim {
            None
        } else {
            match &self.pedido.modo {
                Modo::Salvar { .. } => {
                    let alvo = self.pasta_do_salvar().join(self.nome.text().trim());
                    // the tags: set on the file once the app writes it (a minute at most)
                    let etiquetas = self.etiquetas.borrow().clone();
                    if !etiquetas.is_empty() {
                        etiquetar_quando_existir(alvo.clone(), etiquetas);
                    }
                    Some((vec![alvo], filtro))
                }
                Modo::Abrir { pasta: true, .. } => Some((vec![self.escolhido.borrow().clone().filter(|p| p.is_dir()).unwrap_or_else(|| self.pasta.borrow().clone())], filtro)),
                Modo::Abrir { .. } => {
                    let sel: Vec<PathBuf> = self.lista.selected_rows().iter().filter_map(|r| {
                        let n = r.widget_name();
                        (!n.is_empty()).then(|| PathBuf::from(n.as_str())).filter(|p| p.is_file())
                    }).collect();
                    let sel = if sel.is_empty() { self.escolhido.borrow().clone().filter(|p| p.is_file()).into_iter().collect() } else { sel };
                    if sel.is_empty() {
                        *self.feito.borrow_mut() = Some(feito);
                        return;
                    }
                    Some((sel, filtro))
                }
            }
        };
        if depurando() {
            match &r {
                Some((v, _)) => eprintln!("debug seletor: escolheu {}", v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")),
                None => eprintln!("debug seletor: cancelado"),
            }
        }
        self.janela.close();
        feito(r);
    }

    /// Enter in the field: a file's name here opens it; words search the person's files ("a fatura de outubro").
    fn buscar(self: &Rc<Self>) {
        let q = self.busca.text().trim().to_string();
        if q.is_empty() {
            return;
        }
        // a path typed: there it goes (a folder opens, a file is chosen)
        if q.starts_with('/') || q.starts_with("~/") || q == "~" {
            let p = if let Some(r) = q.strip_prefix('~') { glib::home_dir().join(r.trim_start_matches('/')) } else { PathBuf::from(&q) };
            if p.is_dir() {
                self.ir(&p);
            } else if p.is_file() {
                self.ativar(&ler_um(&p));
            } else if self.salvar() {
                if let (Some(d), Some(n)) = (p.parent().filter(|d| d.is_dir()), p.file_name()) {
                    self.ir(d);
                    self.nome.set_text(&n.to_string_lossy());
                }
            }
            return;
        }
        let aqui = self.visiveis();
        if self.buscando.borrow().is_none() && aqui.len() == 1 && !aqui[0].pasta {
            self.ativar(&aqui[0].clone());
            return;
        }
        let Some(s) = crate::shell_atual() else { return };
        let achados: Vec<Item> = s.comando().indice().search(&q, 60).into_iter().map(|e| ler_um(&e.path)).collect();
        if depurando() {
            eprintln!("debug seletor: busca «{q}» {} achados", achados.len());
        }
        *self.buscando.borrow_mut() = Some(achados);
        self.render();
    }
}

/// Files' tags on a file the app is about to write: once it exists (checked for a minute).
fn etiquetar_quando_existir(p: PathBuf, etiquetas: Vec<String>) {
    let mut n = 0;
    glib::timeout_add_local(Duration::from_millis(500), move || {
        n += 1;
        if p.exists() {
            let _ = gravar_etiquetas(&p, &etiquetas);
            if depurando() {
                eprintln!("debug seletor: etiquetas {} em {}", etiquetas.join(", "), p.display());
            }
            return glib::ControlFlow::Break;
        }
        if n > 120 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// Where files like this one usually go: the folder holding most of the person's files with the same kind and
/// similar names, when it is not the one already chosen ("Os outros relatórios deste projeto estão em …").
fn pasta_sugerida(nome: &str, atual: &Path) -> Option<(PathBuf, usize)> {
    let s = crate::shell_atual()?;
    let ext = Path::new(nome).extension().map(|e| e.to_string_lossy().to_lowercase());
    let palavras: String = Path::new(nome).file_stem()?.to_string_lossy().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit())).collect::<Vec<_>>().join(" ");
    if palavras.is_empty() {
        return None;
    }
    let mut conta: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for e in s.comando().indice().search(&palavras, 80) {
        if ext.as_ref().is_some_and(|x| e.path.extension().map(|y| y.to_string_lossy().to_lowercase()) != Some(x.clone())) {
            continue;
        }
        if let Some(p) = e.path.parent() {
            *conta.entry(p.to_path_buf()).or_default() += 1;
        }
    }
    conta.into_iter().filter(|(p, n)| *n >= 2 && p != atual).max_by_key(|(_, n)| *n)
}

/// Where it opens: the app's folder, else Documents (the usual name when the profile has no user-dirs), else home.
/// (The session test opens it in its own folder: its HOME is the real one.)
fn pasta_inicial(pedido: &Pedido) -> PathBuf {
    std::env::var_os("KALANGO_SELETOR_INICIO").map(PathBuf::from).filter(|p| p.is_dir())
        .or_else(|| pedido.pasta.clone().filter(|p| p.is_dir()))
        .or_else(|| pastas_da_pessoa().into_iter().find(|(_, _, g)| *g == "doc").map(|(_, p, _)| p))
        .unwrap_or_else(glib::home_dir)
}

/// Opens the chooser for an app's request; its window (closing it is cancelling).
pub(crate) fn escolher(pedido: Pedido, feito: impl FnOnce(Resposta) + 'static) -> Option<gtk::Window> {
    let Some(app) = gio::Application::default().and_downcast::<gtk::Application>() else {
        feito(None);
        return None;
    };
    let salvar = matches!(pedido.modo, Modo::Salvar { .. });
    let janela = gtk::Window::builder().application(&app).title(&pedido.titulo).build();
    janela.add_css_class("seletor-janela");
    // its own window group: a modal gtk::FileDialog of the shell's own windows comes back here through the portal,
    // and GTK's portal client holds a grab on the default group while it waits (filechoosernativeportal.c) — in
    // that group the chooser got no click, no key and no close (agent editor › "Usar um SVG…", "Documentos")
    gtk::WindowGroup::new().add_window(&janela);
    // its own top (title, search, close), as the design's
    janela.set_decorated(false);
    let raiz = caixa(Orientation::Vertical, 0, "seletor");
    // ---- the whole browser
    let completo = caixa(Orientation::Vertical, 0, "");
    let topo = caixa(Orientation::Horizontal, 12, "seletor-topo");
    let tt = caixa(Orientation::Vertical, 1, "");
    tt.set_valign(Align::Center);
    let titulo = rotulo(&pedido.titulo, "seletor-titulo");
    titulo.set_xalign(0.0);
    tt.append(&titulo);
    if !pedido.app.is_empty() {
        let a = rotulo(&if salvar { trf("from {app}", &[("app", pedido.app.clone())]) } else { trf("for {app}", &[("app", pedido.app.clone())]) }, "seletor-meta");
        a.set_xalign(0.0);
        tt.append(&a);
    }
    topo.append(&tt);
    let espaco = gtk::Box::new(Orientation::Horizontal, 0);
    espaco.set_hexpand(true);
    topo.append(&espaco);
    let campo = caixa(Orientation::Horizontal, 8, "seletor-busca");
    campo.set_size_request(380, -1);
    campo.set_valign(Align::Center);
    campo.append(&ui_icon("spark", "#A5A1E2", 14));
    let busca = gtk::Entry::new();
    busca.set_hexpand(true);
    busca.set_placeholder_text(Some(tr("Search or ask: “the October invoice”")));
    campo.append(&busca);
    let atalho = rotulo("Ctrl K", "seletor-atalho");
    campo.append(&atalho);
    topo.append(&campo);
    let fechar = gtk::Button::new();
    fechar.add_css_class("seletor-fechar");
    fechar.set_child(Some(&ui_icon("fechar", "#8E8F99", 14)));
    fechar.set_valign(Align::Center);
    topo.append(&fechar);
    completo.append(&topo);
    completo.append(&caixa(Orientation::Horizontal, 0, "seletor-div"));
    let corpo = caixa(Orientation::Horizontal, 0, "");
    corpo.set_vexpand(true);
    let lugares = caixa(Orientation::Vertical, 2, "seletor-locais");
    lugares.set_size_request(200, -1);
    corpo.append(&lugares);
    let centro = caixa(Orientation::Vertical, 0, "");
    centro.set_hexpand(true);
    let barra = caixa(Orientation::Horizontal, 6, "seletor-caminho");
    let migalha = caixa(Orientation::Horizontal, 6, "");
    // (in a scroller that asks no width: a long name never widens the window)
    let rolar_migalha = gtk::ScrolledWindow::new();
    rolar_migalha.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    rolar_migalha.set_propagate_natural_width(false);
    rolar_migalha.set_hexpand(true);
    rolar_migalha.set_child(Some(&migalha));
    barra.append(&rolar_migalha);
    let b_lista = gtk::ToggleButton::new();
    b_lista.add_css_class("seletor-vista");
    b_lista.set_child(Some(&ui_icon("lista", "#B9BAC4", 14)));
    b_lista.set_active(true);
    let b_grade = gtk::ToggleButton::new();
    b_grade.add_css_class("seletor-vista");
    b_grade.set_child(Some(&ui_icon("grade", "#B9BAC4", 14)));
    b_grade.set_group(Some(&b_lista));
    barra.append(&b_lista);
    barra.append(&b_grade);
    centro.append(&barra);
    let colunas = caixa(Orientation::Horizontal, 8, "seletor-colunas");
    let c1 = rotulo(tr("Name"), "seletor-coluna");
    c1.set_xalign(0.0);
    c1.set_hexpand(true);
    colunas.append(&c1);
    let c2 = rotulo(tr("Modified"), "seletor-coluna");
    c2.set_xalign(0.0);
    c2.set_size_request(120, -1);
    colunas.append(&c2);
    let c3 = rotulo(tr("Size"), "seletor-coluna");
    c3.set_xalign(1.0);
    c3.set_size_request(70, -1);
    colunas.append(&c3);
    centro.append(&colunas);
    let lista = gtk::ListBox::new();
    lista.add_css_class("seletor-lista");
    // a click chooses; a double click (or Enter) opens
    lista.set_activate_on_single_click(false);
    lista.set_selection_mode(match pedido.modo {
        Modo::Abrir { varios: true, .. } => gtk::SelectionMode::Multiple,
        _ => gtk::SelectionMode::Single,
    });
    let grade = gtk::FlowBox::new();
    grade.add_css_class("seletor-grade");
    grade.set_selection_mode(gtk::SelectionMode::None);
    grade.set_valign(Align::Start);
    grade.set_max_children_per_line(6);
    let vistas = gtk::Stack::new();
    let rolar = gtk::ScrolledWindow::new();
    rolar.set_child(Some(&lista));
    rolar.set_vexpand(true);
    rolar.set_hscrollbar_policy(gtk::PolicyType::Never);
    vistas.add_named(&rolar, Some("lista"));
    let rolar2 = gtk::ScrolledWindow::new();
    rolar2.set_child(Some(&grade));
    rolar2.set_hscrollbar_policy(gtk::PolicyType::Never);
    vistas.add_named(&rolar2, Some("grade"));
    centro.append(&vistas);
    corpo.append(&centro);
    let previa = caixa(Orientation::Vertical, 10, "seletor-previa");
    previa.set_size_request(250, -1);
    corpo.append(&previa);
    completo.append(&corpo);
    completo.append(&caixa(Orientation::Horizontal, 0, "seletor-div"));
    let rodape = caixa(Orientation::Horizontal, 8, "seletor-rodape");
    let nomes: Vec<String> = pedido.filtros.iter().map(|f| f.nome.clone()).collect();
    let filtro = gtk::DropDown::from_strings(&nomes.iter().map(String::as_str).collect::<Vec<_>>());
    filtro.add_css_class("seletor-filtro");
    filtro.set_size_request(260, -1);
    filtro.set_visible(!nomes.is_empty());
    if let Some(i) = pedido.filtro {
        filtro.set_selected(i as u32);
    }
    rodape.append(&filtro);
    let nome = gtk::Entry::new();
    nome.add_css_class("seletor-nome-campo");
    nome.set_hexpand(true);
    nome.set_visible(salvar);
    rodape.append(&nome);
    let espaco2 = gtk::Box::new(Orientation::Horizontal, 0);
    espaco2.set_hexpand(!salvar);
    rodape.append(&espaco2);
    let cancelar = gtk::Button::with_label(tr("Cancel"));
    cancelar.add_css_class("wifi-botao");
    let rotulo_ok = pedido.rotulo.clone().unwrap_or_else(|| if salvar { tr("Save").to_string() } else { tr("Open").to_string() }).replace('_', "");
    let aceitar = gtk::Button::with_label(&rotulo_ok);
    aceitar.add_css_class("wifi-botao");
    aceitar.add_css_class("principal");
    rodape.append(&cancelar);
    rodape.append(&aceitar);
    completo.append(&rodape);
    // ---- the compact save
    let compacto = caixa(Orientation::Vertical, 12, "seletor-compacto");
    let ch = caixa(Orientation::Horizontal, 12, "");
    let tile = caixa(Orientation::Horizontal, 0, "seletor-salvar-icone");
    tile.set_size_request(36, 36);
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let ic = ui_icon("baixar", "#E2E2E6", 16);
    ic.set_hexpand(true);
    ic.set_halign(Align::Center);
    ic.set_valign(Align::Center);
    tile.append(&ic);
    ch.append(&tile);
    let ct = caixa(Orientation::Vertical, 1, "");
    ct.set_valign(Align::Center);
    let t2 = rotulo(&pedido.titulo, "seletor-titulo");
    t2.set_xalign(0.0);
    ct.append(&t2);
    if !pedido.app.is_empty() {
        let a = rotulo(&trf("from {app}", &[("app", pedido.app.clone())]), "seletor-meta");
        a.set_xalign(0.0);
        ct.append(&a);
    }
    ch.append(&ct);
    compacto.append(&ch);
    let rot = |t: &str| {
        let l = rotulo(t, "seletor-rotulo");
        l.set_xalign(0.0);
        l
    };
    compacto.append(&rot(tr("Name")));
    let campo_nome = caixa(Orientation::Horizontal, 8, "wifi-campo");
    campo_nome.append(&ui_icon("doc", "#8E8F99", 14));
    let nome_c = gtk::Text::new();
    nome_c.set_hexpand(true);
    campo_nome.append(&nome_c);
    compacto.append(&campo_nome);
    let duas = caixa(Orientation::Horizontal, 12, "");
    let col_onde = caixa(Orientation::Vertical, 6, "");
    col_onde.set_hexpand(true);
    col_onde.append(&rot(tr("Where")));
    let onde_pastas: Vec<PathBuf> = pastas_da_pessoa().into_iter().filter(|(_, p, _)| p.as_os_str() != "recent://").map(|(_, p, _)| p).collect();
    let onde = gtk::DropDown::from_strings(&[]);
    onde.add_css_class("seletor-filtro");
    col_onde.append(&onde);
    duas.append(&col_onde);
    let col_fmt = caixa(Orientation::Vertical, 6, "");
    col_fmt.set_size_request(200, -1);
    col_fmt.append(&rot(tr("Format")));
    let formato = gtk::DropDown::from_strings(&nomes.iter().map(String::as_str).collect::<Vec<_>>());
    formato.add_css_class("seletor-filtro");
    if let Some(i) = pedido.filtro {
        formato.set_selected(i as u32);
    }
    col_fmt.append(&formato);
    col_fmt.set_visible(!nomes.is_empty());
    duas.append(&col_fmt);
    compacto.append(&duas);
    let tags = caixa(Orientation::Horizontal, 6, "");
    tags.append(&rot(tr("Tags")));
    let tags_lista = caixa(Orientation::Horizontal, 6, "");
    tags.append(&tags_lista);
    compacto.append(&tags);
    let sugestao = caixa(Orientation::Horizontal, 10, "seletor-sugestao");
    sugestao.set_visible(false);
    compacto.append(&sugestao);
    let pe = caixa(Orientation::Horizontal, 8, "");
    let mais = gtk::Button::new();
    mais.add_css_class("seletor-mais");
    let mb = caixa(Orientation::Horizontal, 6, "");
    mb.append(&ui_icon("chevron-baixo", "#8E8F99", 12));
    mb.append(&rotulo(tr("More options"), ""));
    mais.set_child(Some(&mb));
    pe.append(&mais);
    let espaco3 = gtk::Box::new(Orientation::Horizontal, 0);
    espaco3.set_hexpand(true);
    pe.append(&espaco3);
    let cancelar_c = gtk::Button::with_label(tr("Cancel"));
    cancelar_c.add_css_class("wifi-botao");
    let salvar_c = gtk::Button::with_label(&rotulo_ok);
    salvar_c.add_css_class("wifi-botao");
    salvar_c.add_css_class("principal");
    pe.append(&cancelar_c);
    pe.append(&salvar_c);
    compacto.append(&pe);
    let etapas = gtk::Stack::new();
    etapas.set_transition_type(gtk::StackTransitionType::Crossfade);
    etapas.set_transition_duration(180);
    etapas.set_hhomogeneous(false);
    etapas.set_vhomogeneous(false);
    etapas.set_interpolate_size(true);
    etapas.add_named(&completo, Some("completo"));
    etapas.add_named(&compacto, Some("compacto"));
    raiz.append(&etapas);
    janela.set_child(Some(&raiz));
    if salvar {
        etapas.set_visible_child_name("compacto");
        compacto.set_size_request(620, -1);
        janela.set_resizable(false);
    } else {
        completo.set_size_request(1040, 562);
    }
    let s = Rc::new(Seletor {
        janela, pasta: RefCell::new(glib::home_dir()), itens: RefCell::default(), escolhido: RefCell::default(), lista, grade, vistas, migalha,
        lugares, previa, busca, filtro, nome, aceitar, em_grade: Cell::new(false), buscando: RefCell::default(), etapas, onde, onde_pastas: RefCell::new(onde_pastas),
        etiquetas: RefCell::default(), sugestao, feito: RefCell::new(Some(Box::new(feito))), pedido,
    });
    // the places
    s.lugares.append(&rotulo_secao(tr("PLACES")));
    for (n, p, g) in pastas_da_pessoa() {
        let b = lugar_botao(&n, g);
        b.set_widget_name(&p.to_string_lossy());
        let w = Rc::downgrade(&s);
        b.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.ir(&p) });
        s.lugares.append(&b);
    }
    let montados: Vec<gio::Mount> = gio::VolumeMonitor::get().mounts();
    if !montados.is_empty() {
        s.lugares.append(&rotulo_secao(tr("DEVICES")));
        for m in montados {
            let Some(p) = m.root().path() else { continue };
            let b = lugar_botao(&m.name(), "pasta");
            b.set_widget_name(&p.to_string_lossy());
            let w = Rc::downgrade(&s);
            b.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.ir(&p) });
            s.lugares.append(&b);
        }
    }
    // the compact save's places, name, format, tags and suggestion
    if let Modo::Salvar { nome } = s.pedido.modo.clone() {
        let lista: Vec<String> = s.onde_pastas.borrow().iter().map(|p| {
            p.strip_prefix(glib::home_dir()).map(|x| if x.as_os_str().is_empty() { tr("Home folder").to_string() } else { x.display().to_string().replace('/', " › ") }).unwrap_or_else(|_| p.display().to_string())
        }).collect();
        let modelo = gtk::StringList::new(&lista.iter().map(String::as_str).collect::<Vec<_>>());
        s.onde.set_model(Some(&modelo));
        let inicial = pasta_inicial(&s.pedido);
        if !s.onde_pastas.borrow().contains(&inicial) {
            s.onde_pastas.borrow_mut().push(inicial.clone());
            modelo.append(&inicial.strip_prefix(glib::home_dir()).map(|x| x.display().to_string().replace('/', " › ")).unwrap_or_else(|_| inicial.display().to_string()));
        }
        let k = s.onde_pastas.borrow().iter().position(|p| *p == inicial).unwrap_or(0);
        s.onde.set_selected(k as u32);
        nome_c.set_text(&nome);
        s.nome.set_text(&nome);
        // the name selected up to its extension, to type over
        let fim = Path::new(&nome).file_stem().map(|x| x.to_string_lossy().chars().count()).unwrap_or(0) as i32;
        nome_c.grab_focus();
        nome_c.select_region(0, fim);
        let (n1, n2) = (nome_c.clone(), s.nome.clone());
        nome_c.connect_changed(move |t| if n2.text() != t.text() { n2.set_text(&t.text()) });
        s.nome.connect_changed(move |e| if n1.text() != e.text() { n1.set_text(&e.text()) });
        formato.bind_property("selected", &s.filtro, "selected").bidirectional().sync_create().build();
        // tags: Files' known ones, each a chip that toggles
        for e in etiquetas_conhecidas().into_iter().take(5) {
            let b = gtk::ToggleButton::new();
            b.add_css_class("seletor-etiqueta");
            let bc = caixa(Orientation::Horizontal, 6, "");
            let ponto = caixa(Orientation::Horizontal, 0, "arq-ponto");
            ponto.add_css_class(&format!("tom-{}", tom_de(&e.cor)));
            ponto.set_size_request(6, 6);
            ponto.set_valign(Align::Center);
            ponto.set_halign(Align::Center);
            bc.append(&ponto);
            bc.append(&rotulo(&e.nome, ""));
            b.set_child(Some(&bc));
            crate::central::posicao_debug(&format!("seletor etiqueta {}", e.nome), b.upcast_ref());
            let (w, n) = (Rc::downgrade(&s), e.nome.clone());
            b.connect_toggled(move |b| {
                let Some(s) = w.upgrade() else { return };
                let mut v = s.etiquetas.borrow_mut();
                v.retain(|x| *x != n);
                if b.is_active() {
                    v.push(n.clone());
                }
            });
            tags_lista.append(&b);
        }
        let nova = gtk::MenuButton::new();
        nova.add_css_class("seletor-etiqueta");
        let nb = caixa(Orientation::Horizontal, 4, "");
        nb.append(&ui_icon("mais", "#B9BAC4", 12));
        nb.append(&rotulo(tr("add"), ""));
        nova.set_child(Some(&nb));
        let pop = gtk::Popover::new();
        pop.add_css_class("cfg-escolhas");
        let campo_tag = gtk::Entry::new();
        campo_tag.set_placeholder_text(Some(tr("New tag")));
        pop.set_child(Some(&campo_tag));
        nova.set_popover(Some(&pop));
        let (w, lista_w, popw) = (Rc::downgrade(&s), tags_lista.downgrade(), pop.downgrade());
        campo_tag.connect_activate(move |e| {
            let (Some(s), Some(lista)) = (w.upgrade(), lista_w.upgrade()) else { return };
            let n = e.text().trim().to_string();
            if n.is_empty() || s.etiquetas.borrow().contains(&n) {
                return;
            }
            let b = gtk::ToggleButton::new();
            b.add_css_class("seletor-etiqueta");
            b.set_child(Some(&rotulo(&n, "")));
            b.set_active(true);
            s.etiquetas.borrow_mut().push(n.clone());
            let w2 = Rc::downgrade(&s);
            b.connect_toggled(move |b| {
                let Some(s) = w2.upgrade() else { return };
                let mut v = s.etiquetas.borrow_mut();
                v.retain(|x| *x != n);
                if b.is_active() {
                    v.push(n.clone());
                }
            });
            lista.append(&b);
            e.set_text("");
            if let Some(p) = popw.upgrade() {
                p.popdown();
            }
        });
        tags.append(&nova);
        // where similar files live, when that is somewhere else
        let w = Rc::downgrade(&s);
        let atual = inicial.clone();
        crate::lateral::em_segundo_plano(move || pasta_sugerida(&nome, &atual), move |r| {
            let (Some(s), Some((p, n))) = (w.upgrade(), r) else { return };
            let curto = p.strip_prefix(glib::home_dir()).map(|x| x.display().to_string()).unwrap_or_else(|_| p.display().to_string());
            s.sugestao.append(&ui_icon("spark", "#A5A1E2", 14));
            let t = rotulo(&trf("{n} similar files are in {folder}. Save there?", &[("n", n.to_string()), ("folder", curto)]), "seletor-sugestao-texto");
            t.set_wrap(true);
            t.set_xalign(0.0);
            t.set_hexpand(true);
            s.sugestao.append(&t);
            let usar = gtk::Button::with_label(tr("Use this folder"));
            usar.add_css_class("seletor-usar");
            usar.set_valign(Align::Center);
            let w = Rc::downgrade(&s);
            usar.connect_clicked(move |b| {
                let Some(s) = w.upgrade() else { return };
                let k = s.onde_pastas.borrow().iter().position(|x| *x == p);
                let k = match k {
                    Some(k) => k,
                    None => {
                        s.onde_pastas.borrow_mut().push(p.clone());
                        if let Some(m) = s.onde.model().and_downcast::<gtk::StringList>() {
                            m.append(&p.strip_prefix(glib::home_dir()).map(|x| x.display().to_string().replace('/', " › ")).unwrap_or_default());
                        }
                        s.onde_pastas.borrow().len() - 1
                    }
                };
                s.onde.set_selected(k as u32);
                b.set_sensitive(false);
                if depurando() {
                    eprintln!("debug seletor: pasta sugerida {}", p.display());
                }
            });
            s.sugestao.append(&usar);
            s.sugestao.set_visible(true);
            if depurando() {
                eprintln!("debug seletor: sugestão {n} parecidos");
            }
        });
        let w = Rc::downgrade(&s);
        mais.connect_clicked(move |_| {
            let Some(s) = w.upgrade() else { return };
            let p = s.pasta_do_salvar();
            s.janela.set_resizable(true);
            s.etapas.set_visible_child_name("completo");
            if let Some(c) = s.etapas.child_by_name("completo") {
                c.set_size_request(1040, 562);
            }
            s.ir(&p);
            s.nome.grab_focus();
        });
        let w = Rc::downgrade(&s);
        salvar_c.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.responder(true) });
        let w = Rc::downgrade(&s);
        nome_c.connect_activate(move |_| if let Some(s) = w.upgrade() { s.responder(true) });
        let w = Rc::downgrade(&s);
        cancelar_c.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.responder(false) });
    }
    // the browser's controls
    let w = Rc::downgrade(&s);
    s.lista.connect_row_selected(move |_, r| {
        let (Some(s), Some(r)) = (w.upgrade(), r) else { return };
        let n = r.widget_name();
        if !n.is_empty() {
            let it = ler_um(Path::new(n.as_str()));
            s.escolher(&it);
        }
    });
    let w = Rc::downgrade(&s);
    s.lista.connect_row_activated(move |_, r| {
        let Some(s) = w.upgrade() else { return };
        let n = r.widget_name();
        if !n.is_empty() {
            let it = ler_um(Path::new(n.as_str()));
            s.ativar(&it);
        }
    });
    let w = Rc::downgrade(&s);
    b_grade.connect_toggled(move |b| if let Some(s) = w.upgrade() { s.em_grade.set(b.is_active()); s.render() });
    let w = Rc::downgrade(&s);
    s.busca.connect_changed(move |_| {
        // typing again filters the folder (a finished search's results go)
        if let Some(s) = w.upgrade() {
            s.buscando.borrow_mut().take();
            s.render();
        }
    });
    let w = Rc::downgrade(&s);
    s.busca.connect_activate(move |_| if let Some(s) = w.upgrade() { s.buscar() });
    let w = Rc::downgrade(&s);
    s.filtro.connect_selected_notify(move |_| if let Some(s) = w.upgrade() { s.render() });
    let w = Rc::downgrade(&s);
    s.nome.connect_changed(move |_| if let Some(s) = w.upgrade() { s.atualizar_aceitar() });
    let w = Rc::downgrade(&s);
    s.nome.connect_activate(move |_| if let Some(s) = w.upgrade() { s.responder(true) });
    let w = Rc::downgrade(&s);
    s.aceitar.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.responder(true) });
    let w = Rc::downgrade(&s);
    cancelar.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.responder(false) });
    let w = Rc::downgrade(&s);
    fechar.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.responder(false) });
    // closing the window is cancelling
    let w = Rc::downgrade(&s);
    s.janela.connect_close_request(move |_| {
        if let Some(s) = w.upgrade() {
            if let Some(f) = s.feito.borrow_mut().take() {
                if depurando() {
                    eprintln!("debug seletor: cancelado");
                }
                f(None);
            }
        }
        glib::Propagation::Proceed
    });
    // keys: Esc cancels, Ctrl+K searches, Ctrl+L types a path
    let teclas = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&s);
    teclas.connect_key_pressed(move |_, k, _, m| {
        let Some(s) = w.upgrade() else { return glib::Propagation::Proceed };
        match k {
            gdk::Key::Escape => {
                s.responder(false);
                glib::Propagation::Stop
            }
            gdk::Key::k | gdk::Key::f if m.contains(gdk::ModifierType::CONTROL_MASK) => {
                s.busca.grab_focus();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    s.janela.add_controller(teclas);
    // its app id: a window of the system, floating in the middle (niri's rule)
    let app_id = |j: &gtk::Window| {
        if let Some(sf) = j.surface().filter(|x| x.type_().name().contains("Wayland")) {
            let id = std::ffi::CString::new("os.kalango.Seletor").unwrap();
            unsafe {
                gdk_wayland_toplevel_set_application_id(sf.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr());
            }
        }
    };
    s.janela.connect_realize(move |j| app_id(j));
    s.janela.connect_map(move |j| app_id(j));
    let inicio = pasta_inicial(&s.pedido);
    s.ir(&inicio);
    if depurando() {
        eprintln!("debug seletor: aberto ({}) para {}", if salvar { "salvar" } else { "abrir" }, s.pedido.app);
        for (n, wd) in [("seletor aceitar", s.aceitar.upcast_ref::<gtk::Widget>()), ("seletor busca", s.busca.upcast_ref()), ("seletor salvar", salvar_c.upcast_ref()), ("seletor mais", mais.upcast_ref())] {
            crate::central::posicao_debug(n, wd);
        }
    }
    s.janela.present();
    if !salvar {
        s.lista.grab_focus();
    }
    // (kept alive while open)
    SELETORES.with(|v| v.borrow_mut().push(s.clone()));
    let w = Rc::downgrade(&s);
    s.janela.connect_destroy(move |_| {
        if let Some(s) = w.upgrade() {
            SELETORES.with(|v| v.borrow_mut().retain(|x| !Rc::ptr_eq(x, &s)));
        }
    });
    Some(s.janela.clone())
}

impl Filtro {
    /// A filter from its name and its globs ("*.json") or MIME types ("image/*").
    pub(crate) fn de(nome: &str, padroes: &[&str]) -> Filtro {
        let (mimes, globs) = padroes.iter().map(|p| p.to_string()).partition(|p| p.contains('/'));
        Filtro { nome: nome.to_string(), globs, mimes }
    }
}

/// The system's chooser for the shell's own windows (the Editor, Settings, the agents' centre…): the same one every
/// other app gets through the portal, without the round trip — never GTK's own chooser.
/// `rotulo`: the button's word ("Resumir"), if not "Abrir".
pub(crate) fn abrir_arquivo(app: &str, titulo: &str, rotulo: Option<&str>, filtros: Vec<Filtro>, pasta: Option<PathBuf>, feito: impl FnOnce(PathBuf) + 'static) {
    pedir(Modo::Abrir { varios: false, pasta: false }, app, titulo, rotulo, filtros, pasta, feito);
}

/// The same chooser, for one file or more (each chosen one, in order).
pub(crate) fn abrir_varios(app: &str, titulo: &str, filtros: Vec<Filtro>, pasta: Option<PathBuf>, feito: impl FnOnce(Vec<PathBuf>) + 'static) {
    let filtro = (!filtros.is_empty()).then_some(0);
    let pedido = Pedido { modo: Modo::Abrir { varios: true, pasta: false }, titulo: titulo.to_string(), app: app.to_string(), rotulo: None, filtros, filtro, pasta };
    escolher(pedido, move |r| {
        if let Some((ps, _)) = r.filter(|(ps, _)| !ps.is_empty()) {
            feito(ps);
        }
    });
}

/// The same chooser, for a folder.
pub(crate) fn escolher_pasta(app: &str, titulo: &str, pasta: Option<PathBuf>, feito: impl FnOnce(PathBuf) + 'static) {
    pedir(Modo::Abrir { varios: false, pasta: true }, app, titulo, None, Vec::new(), pasta, feito);
}

/// The same chooser, for where to write a file (`nome` ready to type over).
pub(crate) fn salvar_arquivo(app: &str, titulo: &str, nome: &str, pasta: Option<PathBuf>, feito: impl FnOnce(PathBuf) + 'static) {
    pedir(Modo::Salvar { nome: nome.to_string() }, app, titulo, None, Vec::new(), pasta, feito);
}

fn pedir(modo: Modo, app: &str, titulo: &str, rotulo: Option<&str>, filtros: Vec<Filtro>, pasta: Option<PathBuf>, feito: impl FnOnce(PathBuf) + 'static) {
    let filtro = (!filtros.is_empty()).then_some(0);
    let pedido = Pedido { modo, titulo: titulo.to_string(), app: app.to_string(), rotulo: rotulo.map(str::to_string), filtros, filtro, pasta };
    escolher(pedido, move |r| {
        if let Some(p) = r.and_then(|(ps, _)| ps.into_iter().next()) {
            feito(p);
        }
    });
}

thread_local! {
    static SELETORES: RefCell<Vec<Rc<Seletor>>> = const { RefCell::new(Vec::new()) };
}

fn rotulo_secao(t: &str) -> gtk::Label {
    let l = rotulo(t, "seletor-secao");
    l.set_xalign(0.0);
    l
}

fn lugar_botao(n: &str, glifo: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("seletor-lugar");
    let c = caixa(Orientation::Horizontal, 12, "");
    c.append(&ui_icon(glifo, "#B9BAC4", 16));
    let l = rotulo(n, "");
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    c.append(&l);
    b.set_child(Some(&c));
    b
}

#[cfg(test)]
mod tests_seletor {
    use super::glob;

    #[test]
    fn globs_as_apps_send_them() {
        assert!(glob("*.pdf", "fatura.pdf"));
        assert!(!glob("*.pdf", "fatura.pdf.txt"));
        assert!(glob("*.tar.*", "a.tar.gz"));
        assert!(glob("notas?.md", "notas1.md"));
        assert!(glob("*", "qualquer"));
    }
}

/// The files used last (the system's recently-used list), newest first: (path, its type, when, in Unix time).
pub(crate) fn recentes_com_tipo(n: usize) -> Vec<(PathBuf, String, i64)> {
    let Ok(t) = std::fs::read_to_string(glib::user_data_dir().join("recently-used.xbel")) else { return vec![] };
    let mut v: Vec<(i64, PathBuf, String)> = Vec::new();
    for l in t.split("<bookmark ").skip(1) {
        let campo = |k: &str| l.split(&format!("{k}=\"")).nth(1).and_then(|x| x.split('"').next()).unwrap_or("").to_string();
        let mime = l.split("<mime:mime-type type=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("").to_string();
        let quando = glib::DateTime::from_iso8601(&campo("modified"), None).map(|d| d.to_unix()).unwrap_or(0);
        if let Some(p) = gio::File::for_uri(&campo("href")).path().filter(|p| p.is_file()) {
            if !v.iter().any(|x| x.1 == p) {
                v.push((quando, p, mime));
            }
        }
    }
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
    v.into_iter().take(n).map(|(q, p, m)| (p, m, q)).collect()
}
