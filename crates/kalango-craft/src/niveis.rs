//! O que cada ferramenta MCP de um app criativo pede quando um agente a usa pelo kalangod:
//! - livre: ler, inspecionar, listar, ver uma prévia (e, sem janela, as edições conhecidas do documento do próprio
//!   servidor, que só sai dele salvando ou exportando);
//! - confirmar: salvar, exportar, importar, qualquer caminho de arquivo ou endereço nos argumentos (e editar o
//!   documento aberto numa janela);
//! - aprovar: controlar a janela (`ui_*`, cliques, teclas), rodar scripts, executar comandos que o catálogo não
//!   conhece — e toda ferramenta que o catálogo não conhece.
//!
//! Fechado por padrão: os nomes vêm do catálogo (`ferramentas`), nunca do que o app diz de si (as anotações do
//! `tools/list` não baixam o nível). Um executor de comandos (`command_run`, `batch`…) é livre só para os ids de
//! comando da lista do app (`comandos_livres`, conferida numa versão); qualquer outro id — ou um pedido sem id que se
//! reconheça — pede aprovação, e um lote vale o pior dos seus itens.

use crate::catalogo::{App, Catalogo};
use kalango_i18n::n_;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Nivel {
    Livre = 0,
    Confirmar = 1,
    Aprovar = 2,
}

impl Nivel {
    pub fn nome(self) -> &'static str {
        match self {
            Nivel::Livre => "livre",
            Nivel::Confirmar => "confirmar",
            Nivel::Aprovar => "aprovar",
        }
    }
}

/// O porquê do nível (para a tela de aprovação e a linha do tempo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motivo {
    Leitura,
    Edicao,
    EdicaoNaJanela,
    Arquivo,
    Janela,
    Script,
    ComandoDesconhecido,
    /// o id está na lista de livres, mas o app instalado não é a versão em que a lista foi conferida
    ListaDesatualizada,
    Desconhecida,
    SoAprovando,
}

impl Motivo {
    /// Em inglês, para o tr() de quem mostra.
    pub fn texto(self) -> &'static str {
        match self {
            Motivo::Leitura => n_("reads, inspects or previews"),
            Motivo::Edicao => n_("edits the server's own document (nothing leaves it without saving or exporting)"),
            Motivo::EdicaoNaJanela => n_("changes the document open in the app's window"),
            Motivo::Arquivo => n_("saves, exports, imports, or reaches a file or an address"),
            Motivo::Janela => n_("drives the app's window (clicks, keys, dialogs)"),
            Motivo::Script => n_("runs a script, a macro or arbitrary commands inside the app"),
            Motivo::ComandoDesconhecido => n_("runs an app command KalangoOS has not checked"),
            Motivo::ListaDesatualizada => n_("runs an app command from a list KalangoOS checked on another version of the app: it asks again"),
            Motivo::Desconhecida => n_("a tool KalangoOS does not know yet"),
            Motivo::SoAprovando => n_("this app runs programs to investigate them: every tool asks"),
        }
    }
}

/// Partes de um id de comando que rodam código (procuradas no id inteiro, em minúsculas).
const SCRIPT: &[&str] = &["script", "macro", "plugin", "exec", "shell", "eval", "python", "lua", "javascript", "command_line", "terminal"];
/// Partes de um id de comando que mexem em arquivos ou na rede.
const ARQUIVO: &[&str] = &[
    "save",
    "export",
    "import",
    "open",
    "publish",
    "print",
    "bounce",
    "write",
    "place",
    "link",
    "render_to",
    "load",
    "revert",
    "plot",
    "file",
    "download",
    "upload",
    "url",
    "http",
    "share",
    "send",
];

/// Uma chave de argumento que aponta um arquivo, uma pasta ou um endereço.
fn chave_de_arquivo(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    let k = k.trim_end_matches('s');
    ["path", "file", "dir", "folder", "url", "uri", "output", "out", "dest", "destination", "filename", "target"]
        .iter()
        .any(|s| k == *s || k.ends_with(s))
}

/// Um texto que parece um caminho ou um endereço.
fn parece_caminho(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    s.starts_with('/')
        || s.starts_with('~')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.contains("://")
        || s.starts_with("file:")
        || (b.len() > 2 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.contains("\\\\")
        || s.contains("/../")
}

/// O nível de um id de comando pedido a um executor.
fn nivel_do_id(app: &App, id: &str, janela: bool, instalada: Option<&str>) -> (Nivel, Motivo) {
    let low = id.to_ascii_lowercase();
    if SCRIPT.iter().any(|p| low.contains(p)) {
        return (Nivel::Aprovar, Motivo::Script);
    }
    if app.comandos_livres.ids.iter().any(|x| x == id) && !ARQUIVO.iter().any(|p| low.contains(p)) {
        // (a lista foi conferida numa versão do app: noutra, ou sem saber qual está instalada, os ids voltam a pedir)
        if instalada != Some(app.comandos_livres.versao.as_str()) || app.comandos_livres.versao.is_empty() {
            return (Nivel::Aprovar, Motivo::ListaDesatualizada);
        }
        return if janela { (Nivel::Confirmar, Motivo::EdicaoNaJanela) } else { (Nivel::Livre, Motivo::Edicao) };
    }
    (Nivel::Aprovar, Motivo::ComandoDesconhecido)
}

fn pior(a: Option<(Nivel, Motivo)>, b: (Nivel, Motivo)) -> Option<(Nivel, Motivo)> {
    match a {
        Some(x) if x.0 >= b.0 => Some(x),
        _ => Some(b),
    }
}

/// O que os argumentos pedem por si: caminhos e endereços (chaves ou valores) e ids de script.
fn argumentos_pedem(args: &Value) -> Option<(Nivel, Motivo)> {
    fn andar(v: &Value, chave: &str, acc: &mut Option<(Nivel, Motivo)>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    andar(x, k, acc);
                }
            }
            Value::Array(a) => {
                for x in a {
                    andar(x, chave, acc);
                }
            }
            Value::String(s) => {
                let low = s.to_ascii_lowercase();
                if matches!(chave.to_ascii_lowercase().as_str(), "id" | "cmd" | "command" | "commands" | "action" | "name")
                    && SCRIPT.iter().any(|p| low.contains(p))
                {
                    *acc = pior(*acc, (Nivel::Aprovar, Motivo::Script));
                }
                if (chave_de_arquivo(chave) && !s.trim().is_empty()) || parece_caminho(s) {
                    *acc = pior(*acc, (Nivel::Confirmar, Motivo::Arquivo));
                }
            }
            _ => {}
        }
    }
    let mut acc = None;
    andar(args, "", &mut acc);
    acc
}

/// As chaves de um passo de executor: um id (com um destes nomes) e os parâmetros.
const CHAVES_DE_ID: &[&str] = &["id", "cmd", "command"];
const CHAVES_DE_LISTA: &[&str] = &["commands", "steps"];

/// Os ids de comando de um pedido a um executor, lidos de forma estrita: `{id|cmd|command, params}` ou
/// `{commands|steps: [{id|cmd|command, params}…]}`, exatamente uma chave de id por passo e nenhuma outra chave
/// (nem com outra caixa: `Command`, `name`, `action`, `op` não contam como id). Qualquer coisa fora disso: None
/// (o pedido pede aprovação).
fn ids_de_comando(args: &Value) -> Option<Vec<String>> {
    fn passo(o: &serde_json::Map<String, Value>) -> Option<String> {
        let mut id = None;
        for (k, v) in o {
            if CHAVES_DE_ID.contains(&k.as_str()) {
                if id.is_some() {
                    return None;
                }
                id = Some(v.as_str()?.to_string());
            } else if k != "params" || !(v.is_object() || v.is_null()) {
                return None;
            }
        }
        id
    }
    let o = args.as_object()?;
    if let Some((k, v)) = o.iter().find(|(k, _)| CHAVES_DE_LISTA.contains(&k.as_str())) {
        if o.len() != 1 {
            return None;
        }
        let _ = k;
        let lista = v.as_array()?;
        if lista.is_empty() {
            return None;
        }
        return lista.iter().map(|x| x.as_object().and_then(passo)).collect();
    }
    passo(o).map(|i| vec![i])
}

/// O nível de `ferramenta` deste app com estes argumentos; `janela`: o servidor está ligado a uma janela aberta;
/// `instalada`: a versão verificada do app instalado (a lista de comandos livres só vale nela).
pub fn classificar(cat: &Catalogo, app: &App, ferramenta: &str, args: &Value, janela: bool, instalada: Option<&str>) -> (Nivel, Motivo) {
    if app.somente_aprovar {
        return (Nivel::Aprovar, Motivo::SoAprovando);
    }
    let f = &cat.ferramentas;
    let em = |l: &Vec<String>| l.iter().any(|n| n == ferramenta);
    let low = ferramenta.to_ascii_lowercase();
    if low.contains("script") || low.starts_with("execute") || low == "run_command" || low.contains("eval") || low.contains("shell") {
        return (Nivel::Aprovar, Motivo::Script);
    }
    if low.starts_with("ui_") || em(&f.aprovar) {
        let m = if ["command_line", "call_app", "enable_toolset"].contains(&ferramenta) { Motivo::Script } else { Motivo::Janela };
        return (Nivel::Aprovar, m);
    }
    let dos_argumentos = argumentos_pedem(args);
    if em(&f.executores) {
        let Some(ids) = ids_de_comando(args) else {
            return (Nivel::Aprovar, Motivo::ComandoDesconhecido);
        };
        let mut n: Option<(Nivel, Motivo)> = None;
        for id in &ids {
            n = pior(n, nivel_do_id(app, id, janela, instalada));
        }
        if let Some(x) = dos_argumentos {
            n = pior(n, x);
        }
        return n.unwrap_or((Nivel::Aprovar, Motivo::ComandoDesconhecido));
    }
    if em(&f.confirmar) {
        return match dos_argumentos {
            Some(x) if x.0 == Nivel::Aprovar => x,
            _ => (Nivel::Confirmar, Motivo::Arquivo),
        };
    }
    if em(&f.edicao) {
        let base = if janela { (Nivel::Confirmar, Motivo::EdicaoNaJanela) } else { (Nivel::Livre, Motivo::Edicao) };
        return match dos_argumentos {
            Some(x) if x.0 >= base.0 => x,
            _ => base,
        };
    }
    if em(&f.livre) {
        // a captura de uma janela aberta mostra o que está na tela da pessoa
        if janela && ferramenta == "screenshot" {
            return (Nivel::Aprovar, Motivo::Janela);
        }
        return dos_argumentos.unwrap_or((Nivel::Livre, Motivo::Leitura));
    }
    (Nivel::Aprovar, Motivo::Desconhecida)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cat() -> &'static Catalogo {
        crate::catalogo::catalogo()
    }

    fn nivel(app: &str, f: &str, args: Value, janela: bool) -> Nivel {
        let c = cat();
        let a = c.app(app).unwrap();
        classificar(c, a, f, &args, janela, Some(&a.comandos_livres.versao)).0
    }

    /// An app with a checked list of edit-only commands.
    fn com_lista(app: &str, ids: &[&str]) -> App {
        let mut a = cat().app(app).unwrap().clone();
        a.comandos_livres.versao = "teste".into();
        a.comandos_livres.ids = ids.iter().map(|s| s.to_string()).collect();
        a
    }

    #[test]
    fn reading_inspecting_listing_and_previewing_are_free() {
        for (app, f) in [
            ("filmcraft", "doc_inspect"),
            ("filmcraft", "command_list"),
            ("filmcraft", "render_preview"),
            ("anicraft", "anicraft_document"),
            ("anicraft", "anicraft_render"),
            ("gridcraft", "read_range"),
            ("effectcraft", "get_layer"),
        ] {
            assert_eq!(nivel(app, f, json!({}), false), Nivel::Livre, "{app} {f}");
        }
    }

    #[test]
    fn saving_exporting_importing_and_paths_ask_to_confirm() {
        for (app, f) in [
            ("filmcraft", "media_import"),
            ("vectorcraft", "save_file"),
            ("vectorcraft", "export"),
            ("soundcraft", "bounce_mix"),
            ("designcraft", "export_png"),
            ("deckcraft", "insert_picture"),
        ] {
            assert_eq!(nivel(app, f, json!({"path": "/tmp/x"}), false), Nivel::Confirmar, "{app} {f}");
        }
        // a path or an address anywhere in a free tool's arguments
        assert_eq!(nivel("filmcraft", "render_preview", json!({"output": "/home/x/a.png"}), false), Nivel::Confirmar);
        assert_eq!(nivel("filmcraft", "render_preview", json!({"frame": 2, "x": "~/a.png"}), false), Nivel::Confirmar);
        assert_eq!(nivel("filmcraft", "doc_inspect", json!({"source_url": "https://exemplo.invalid/a"}), false), Nivel::Confirmar);
        assert_eq!(nivel("gridcraft", "open_workbook", json!({"workbook_path": "planilha.xlsx"}), false), Nivel::Confirmar);
    }

    #[test]
    fn executors_are_free_only_for_checked_edit_commands() {
        let c = cat();
        let a = com_lista("filmcraft", &["clip.trim", "layer.newText"]);
        let n = |args: Value, j: bool| classificar(c, &a, "command_run", &args, j, Some("teste")).0;
        assert_eq!(n(json!({"id": "clip.trim", "params": {"seconds": 2}}), false), Nivel::Livre);
        assert_eq!(n(json!({"id": "clip.trim"}), true), Nivel::Confirmar, "on an open window an edit is confirmed");
        // a command not in the list: approval, even one that sounds harmless
        assert_eq!(n(json!({"id": "clip.split"}), false), Nivel::Aprovar);
        // exporting through the executor: not in the list → approval (fail closed)
        assert_eq!(n(json!({"id": "file.export", "params": {}}), false), Nivel::Aprovar);
        // a listed id with a path in its params: at least confirm
        assert_eq!(n(json!({"id": "clip.trim", "params": {"path": "/x"}}), false), Nivel::Confirmar);
        // a batch is worth its worst item
        assert_eq!(
            classificar(c, &a, "command_batch", &json!({"commands": [{"id": "layer.newText"}, {"id": "render.mp4"}]}), false, Some("teste")).0,
            Nivel::Aprovar
        );
        assert_eq!(
            classificar(c, &a, "command_batch", &json!({"commands": [{"id": "layer.newText"}, {"id": "clip.trim"}]}), false, Some("teste")).0,
            Nivel::Livre
        );
        // an executor call with no recognisable command id: approval
        assert_eq!(n(json!({"op": "whatever"}), false), Nivel::Aprovar);
        // with the catalog's empty list (no version checked yet), every command asks
        assert_eq!(nivel("filmcraft", "command_run", json!({"id": "clip.trim"}), false), Nivel::Aprovar);
    }

    #[test]
    fn executor_arguments_are_read_strictly() {
        let c = cat();
        let a = com_lista("filmcraft", &["clip.trim"]);
        let n = |args: Value| classificar(c, &a, "command_run", &args, false, Some("teste")).0;
        assert_eq!(n(json!({"id": "clip.trim", "params": {"seconds": 2}})), Nivel::Livre);
        // another case, another name for the id, a second id, an unknown key: approval
        for bad in [
            json!({"Command": "file.export"}),
            json!({"name": "file.export"}),
            json!({"action": "file.export"}),
            json!({"op": "file.export"}),
            json!({"id": "clip.trim", "cmd": "file.export"}),
            json!({"id": "clip.trim", "extra": "file.export"}),
            json!({"id": "clip.trim", "params": "file.export"}),
            json!({"commands": [{"id": "clip.trim"}], "id": "clip.trim"}),
            json!({"commands": [{"id": "clip.trim", "Command": "file.export"}]}),
            json!({"commands": []}),
            json!("clip.trim"),
        ] {
            assert_eq!(n(bad.clone()), Nivel::Aprovar, "{bad}");
        }
        // a path anywhere in the params of a listed command: at least confirm
        assert_eq!(n(json!({"id": "clip.trim", "params": {"deep": [{"x": "/etc/passwd"}]}})), Nivel::Confirmar);
    }

    #[test]
    fn an_allowlisted_id_that_names_a_script_or_a_file_still_asks() {
        let c = cat();
        let a = com_lista("anicraft", &["macro.run", "file.save_as"]);
        assert_eq!(classificar(c, &a, "anicraft_run", &json!({"cmd": "macro.run"}), false, Some("teste")).0, Nivel::Aprovar);
        assert_eq!(classificar(c, &a, "anicraft_run", &json!({"cmd": "file.save_as"}), false, Some("teste")).0, Nivel::Aprovar);
    }

    #[test]
    fn scripts_execute_and_run_command_always_need_approval() {
        for (app, f) in [
            ("effectcraft", "run_script"),
            ("anicraft", "anicraft_run_script"),
            ("gridcraft", "execute_command"),
            ("wordcraft", "execute"),
            ("vectorcraft", "run_command"),
            ("cadcraft", "script"),
        ] {
            assert_eq!(nivel(app, f, json!({"id": "view.zoom"}), false), Nivel::Aprovar, "{app} {f}");
        }
    }

    #[test]
    fn driving_the_window_needs_approval() {
        for (app, f) in [
            ("filmcraft", "ui_click"),
            ("filmcraft", "ui_screenshot"),
            ("effectcraft", "ui_type"),
            ("designcraft", "pointer"),
            ("soundcraft", "menu_invoke"),
            ("deckcraft", "call_app"),
        ] {
            assert_eq!(nivel(app, f, json!({}), false), Nivel::Aprovar, "{app} {f}");
        }
        // a picture of an open window shows the person's screen
        assert_eq!(nivel("vectorcraft", "screenshot", json!({}), true), Nivel::Aprovar);
        assert_eq!(nivel("vectorcraft", "screenshot", json!({}), false), Nivel::Livre);
    }

    #[test]
    fn edit_tools_are_free_headless_and_confirmed_on_a_window() {
        assert_eq!(nivel("vectorcraft", "draw_shape", json!({"kind": "rect"}), false), Nivel::Livre);
        assert_eq!(nivel("vectorcraft", "draw_shape", json!({"kind": "rect"}), true), Nivel::Confirmar);
    }

    #[test]
    fn unknown_tools_need_approval() {
        let c = cat();
        let (n, m) = classificar(c, c.app("filmcraft").unwrap(), "brand_new_tool", &json!({}), false, None);
        assert_eq!((n, m), (Nivel::Aprovar, Motivo::Desconhecida));
        assert_eq!(nivel("filmcraft", "list_everything_and_delete", json!({}), false), Nivel::Aprovar);
    }

    #[test]
    fn craft_anything_always_asks() {
        assert_eq!(nivel("craft-anything", "list_toolsets", json!({}), false), Nivel::Aprovar);
    }

    #[test]
    fn the_free_command_list_holds_only_for_the_version_it_was_checked_on() {
        let c = cat();
        let a = com_lista("anicraft", &["clip.trim"]);
        let n = |instalada: Option<&str>| classificar(c, &a, "anicraft_run", &json!({"cmd": "clip.trim"}), false, instalada);
        assert_eq!(n(Some("teste")).0, Nivel::Livre);
        // auto-update brought a newer version, or the installed one is not known: the same id asks, and says why
        assert_eq!(n(Some("0.3.0")), (Nivel::Aprovar, Motivo::ListaDesatualizada));
        assert_eq!(n(None), (Nivel::Aprovar, Motivo::ListaDesatualizada));
        // a list with no version never holds
        let mut sem = a.clone();
        sem.comandos_livres.versao.clear();
        assert_eq!(classificar(c, &sem, "anicraft_run", &json!({"cmd": "clip.trim"}), false, Some("")).0, Nivel::Aprovar);
        // an id that is not on the list is the plain unknown command, as before
        assert_eq!(
            classificar(c, &a, "anicraft_run", &json!({"cmd": "other.thing"}), false, Some("0.3.0")),
            (Nivel::Aprovar, Motivo::ComandoDesconhecido)
        );
    }

    #[test]
    fn only_exactly_the_checked_version_keeps_the_free_list() {
        let c = cat();
        let a = com_lista("anicraft", &["clip.trim"]);
        for outra in ["teste-beta", "vteste", " teste", "teste ", "", "tes", "teste.1", "TESTE", "0.2.1"] {
            assert_eq!(classificar(c, &a, "anicraft_run", &json!({"cmd": "clip.trim"}), false, Some(outra)).0, Nivel::Aprovar, "{outra:?}");
        }
        assert_eq!(classificar(c, &a, "anicraft_run", &json!({"cmd": "clip.trim"}), false, Some("teste")).0, Nivel::Livre);
    }
}
