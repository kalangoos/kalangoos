//! Words of a request that bring a tool domain in, in the languages beyond Portuguese and English (es, fr, it, de,
//! ja, ru, zh). `motor::DOMAINS` keeps pt and en; a request in Japanese about files needs the file tools the same
//! way. Words in lowercase (requests are lowercased before matching).
//!
//! Matching is by word, not by substring (`mentions`): an entry matches where a word of the request STARTS with it,
//! so "suche" is in "Suche Wetter" and not in "versuche", and "porta" is not in "import". An entry ending in `$` is
//! the whole word ("порт$" is "порт", not "портрет"). Japanese katakana words start where a katakana run starts
//! ("ログ" is not in "カタログ"). Chinese and the other kana/kanji have no spaces: they match as they are, and
//! `EXCEPTIONS` lists the longer words that contain an entry without meaning it (服务员 is a waiter, not a service).

/// (domain, words) — the domains of `motor::DOMAINS`.
pub(crate) const DOMAIN_WORDS: &[(&str, &[&str])] = &[
    ("arquivos", &[
        "archivo", "carpeta", "papelera", "renombr", "mover", "fichier", "dossier", "corbeille", "renomm", "déplac",
        "cartella$", "cartelle$", "cestino", "rinomin", "sposta", "datei", "ordner", "papierkorb", "umbenenn", "verschieb",
        "ファイル", "フォルダ", "ゴミ箱", "名前を変更", "移動", "файл", "папк", "корзин", "переимен", "перемест",
        "文件", "文件夹", "回收站", "重命名", "移动"]),
    ("rede", &["la red", "de red", "réseau", "la rete", "della rete", "nella rete", "alla rete", "di rete", "netzwerk", "wlan", "ネットワーク", "接続", "сеть", "сети", "网络", "联网"]),
    ("firewall", &["cortafuegos", "pare-feu", "puerto", "bloquea", "bloquer", "blocca", "sperr", "ファイアウォール", "ポート", "брандмауэр", "порт$", "порты$", "порта$", "портов$", "порту$", "портом$", "портах$", "防火墙", "端口"]),
    ("discos", &["disco", "partici", "almacenamiento", "espacio", "disque", "stockage", "espace", "spazio", "archiviazione",
        "festplatte", "datenträger", "speicherplatz", "partition", "ディスク", "容量", "ストレージ", "диск", "раздел", "место$", "места$", "месте$", "месту$",
        "磁盘", "硬盘", "分区", "空间", "存储"]),
    ("usuarios", &["usuario", "cuenta", "utilisateur", "compte$", "comptes$", "utente", "account", "benutzer", "konto", "ユーザー", "アカウント",
        "пользовател", "учётн", "учетн", "用户", "账户", "帐户"]),
    ("atualizacoes", &["actualiz", "mise à jour", "mettre à jour", "aggiorna", "aktualisier", "アップデート", "更新", "обновл", "升级"]),
    ("pacotes", &["paquete", "programa", "paquet", "logiciel", "pacchetto", "installa", "paket", "installier", "programm",
        "インストール", "アンインストール", "パッケージ", "アプリ", "установ", "пакет", "программ", "удали программ", "安装", "卸载", "软件包", "软件"]),
    ("tarefas", &["ejecut", "comando", "exécut", "lance ", "commande", "esegui", "comando", "ausführ", "befehl", "starte ",
        "実行", "コマンド", "スクリプト", "запусти", "выполни", "команд", "скрипт", "运行", "执行", "命令", "脚本"]),
    ("pontos", &["punto de restauración", "copia de seguridad", "point de restauration", "sauvegarde", "punto di ripristino",
        "wiederherstellungspunkt", "sicherung", "復元ポイント", "バックアップ", "точк", "резервн", "还原点", "备份"]),
    ("servicos", &["servicio", "servizio", "dienst", "サービス", "служб", "сервис", "服务"]),
    ("processos", &["proceso", "memoria", "lento", "processus", "mémoire", "processo", "prozess", "arbeitsspeicher", "langsam",
        "プロセス", "メモリ", "遅い", "процесс", "памят", "тормоз", "медлен", "进程", "内存", "卡顿", "很慢"]),
    ("registros", &["registro", "journal", "erreur", "errore", "protokoll", "fehler", "ログ", "エラー", "журнал", "ошибк", "日志", "错误"]),
    ("kernel", &["seguridad", "sécurité", "sicurezza", "sicherheit", "セキュリティ", "безопасност", "安全"]),
    ("linha_do_tempo", &["deshacer", "cronología", "historial", "annuler", "chronologie", "historique", "annulla", "cronologia",
        "rückgängig", "zeitleiste", "verlauf", "元に戻", "タイムライン", "履歴", "отмен", "хронолог", "истори", "撤销", "时间线", "历史"]),
    ("sistema", &["système", "administrateur", "amministratore", "administrator", "システム", "管理者", "систем", "администратор", "系统", "管理员"]),
    ("web", &["página", "noticia", "buscar en internet", "busca en", "page web", "actualité", "recherche", "pagina", "notizi", "cerca",
        "webseite", "nachricht", "suche", "ウェブ", "ニュース", "検索", "サイト", "сайт", "новост", "найди в интернете", "поиск",
        "网页", "新闻", "搜索", "网站"]),
    ("vm", &["máquina virtual", "machine virtuelle", "macchina virtuale", "virtuelle maschine", "仮想マシン", "виртуальн", "虚拟机"]),
    ("tela", &["pantalla", "ventana", "haz clic", "escribe en", "botón", "écran", "fenêtre", "clique", "bouton", "schermo", "finestra",
        "clicca", "pulsante", "bildschirm", "fenster", "klick", "schaltfläche", "画面", "ウィンドウ", "クリック", "ボタン", "экран", "окн",
        "нажми", "кнопк", "屏幕", "窗口", "点击", "按钮"]),
    ("agentes", &["agente", "agent", "エージェント", "агент", "智能体", "代理"]),
    ("fluxos", &["flujo", "automatiza", "flux", "automatis", "flusso", "automatizz", "ablauf", "automatisier", "フロー", "自動化",
        "сценари", "автоматиз", "流程", "自动化"]),
    ("ferramentas", &["herramienta", "outil", "strumento", "werkzeug", "ツール", "инструмент", "工具"]),
    ("mcp", &["conector", "connecteur", "connettore", "konnektor", "コネクタ", "коннектор", "连接器"]),
    ("modelos", &["modelo", "modèle", "modello", "modell", "モデル", "модел", "模型"]),
];

/// Words of a request about the agent's memory (remember, forget, preferences).
pub(crate) const MEMORY_WORDS: &[&str] = &[
    "recuerda", "olvida", "prefiero", "llámame", "mi nombre", "souviens", "rappelle-toi", "oublie", "je préfère", "appelle-moi",
    "mon nom", "ricorda", "dimentica", "preferisco", "chiamami", "il mio nome", "merk dir", "erinnere", "vergiss", "ich bevorzuge",
    "nenn mich", "mein name", "覚えて", "忘れて", "好み", "私の名前", "запомни", "забудь", "предпочитаю", "зови меня", "меня зовут",
    "记住", "忘记", "忘掉", "我喜欢", "叫我", "我的名字",
];

/// Words of a request that deletes or removes something.
pub(crate) const DESTRUCTIVE_WORDS: &[&str] = &[
    "borra", "elimina", "limpia", "supprime", "efface", "nettoie", "cancella", "rimuovi", "pulisci", "lösch", "entferne$", "entfernen$", "entfernst$",
    "aufräum", "削除", "消して", "片付け", "удали", "очисти", "сотри", "删除", "清理", "清除",
];

/// The words of `domain` in these languages.
pub(crate) fn domain_words(domain: &str) -> &'static [&'static str] {
    DOMAIN_WORDS.iter().find(|(d, _)| *d == domain).map(|(_, w)| *w).unwrap_or(&[])
}

/// Longer words that contain an entry without meaning it. An occurrence of an entry inside one of these words (at
/// any offset) does not count.
const EXCEPTIONS: &[&str] = &[
    // de: Dienstag, ausführlich
    "dienstag", "ausführlich",
    // es / it: disco is not discoteca, discorso, discordia; borra is not borrador, borrasca; elimina is not eliminatoria
    "discoteca", "discorso", "discordia", "borrador", "borrasca", "eliminatoria",
    // fr: journal is not journaliste
    "journaliste", "journalisme",
    // ru: диск is not дискуссия, дискотека, дисконт; удали is not удалился; модел is not моделирование
    "дискусс", "дискотек", "дисконт", "удалился", "удалились", "удалилась", "удалилось", "моделирован", "модельер",
    // ja: ログ is not ログイン, ポート is not ポートレート, メモリ is not メモリアル
    "ログイン", "ポートレート", "ポートフォリオ", "メモリアル",
    // zh: a waiter, an agent (a seller), a seat belt, a history class, a space station
    "服务员", "代理商", "代理人", "安全带", "历史课", "历史书", "空间站",
];

fn is_katakana(c: char) -> bool {
    ('\u{30A0}'..='\u{30FF}').contains(&c)
}

/// Han and hiragana have no spaces between words: an entry in them matches wherever it is.
fn is_unspaced(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c) || ('\u{3040}'..='\u{309F}').contains(&c)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || ('\u{300}'..='\u{36F}').contains(&c)
}

/// Whether `entry` (see the module docs) is in `text`, already lowercase.
pub(crate) fn has_word(text: &str, entry: &str) -> bool {
    let (stem, whole) = match entry.strip_suffix('$') {
        Some(s) => (s, true),
        None => (entry, false),
    };
    let Some(first) = stem.chars().next() else { return false };
    'occurrence: for (i, _) in text.match_indices(stem) {
        for e in EXCEPTIONS.iter().filter(|e| e.contains(stem)) {
            if e.match_indices(stem).any(|(j, _)| i >= j && text.get(i - j..).is_some_and(|t| t.starts_with(e))) {
                continue 'occurrence;
            }
        }
        let before = text[..i].chars().next_back();
        let starts = match before {
            None => true,
            Some(_) if is_unspaced(first) => true,
            Some(p) if is_katakana(first) => !is_katakana(p),
            Some(p) => !is_word_char(p),
        };
        let ends = !whole || text[i + stem.len()..].chars().next().is_none_or(|n| !is_word_char(n));
        if starts && ends {
            return true;
        }
    }
    false
}

/// Whether any of `words` is in `text` (already lowercase), by word.
pub(crate) fn any_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| has_word(text, w))
}

/// Whether a request (already lowercase) brings `domain` in, in these languages.
pub(crate) fn mentions(domain: &str, text: &str) -> bool {
    any_word(text, domain_words(domain))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(request: &str, domain: &str) -> bool {
        mentions(domain, &request.to_lowercase())
    }
    fn destructive(request: &str) -> bool {
        any_word(&request.to_lowercase(), DESTRUCTIVE_WORDS)
    }
    fn memory(request: &str) -> bool {
        any_word(&request.to_lowercase(), MEMORY_WORDS)
    }

    #[test]
    fn every_domain_exists_and_words_are_lowercase() {
        for (d, ws) in DOMAIN_WORDS {
            assert!(crate::motor::domain_names().contains(d), "{d} is not a domain of motor::DOMAINS");
            for w in ws.iter().chain(MEMORY_WORDS).chain(DESTRUCTIVE_WORDS) {
                assert_eq!(w.to_lowercase(), *w, "{w}");
            }
        }
        for e in EXCEPTIONS {
            assert_eq!(e.to_lowercase(), *e, "{e}");
        }
    }

    /// An exception that contains no entry of any table does nothing: it is a typo or a leftover.
    #[test]
    fn every_exception_is_about_an_entry() {
        let stems: Vec<&str> = DOMAIN_WORDS.iter().flat_map(|(_, w)| w.iter()).chain(MEMORY_WORDS).chain(DESTRUCTIVE_WORDS)
            .map(|w| w.trim_end_matches('$')).collect();
        for e in EXCEPTIONS {
            assert!(stems.iter().any(|s| e.contains(s)), "no entry is inside the exception '{e}'");
        }
    }

    #[test]
    fn requests_in_other_languages_find_their_domain() {
        assert!(found("ダウンロードフォルダのファイルを整理して", "arquivos"));
        assert!(found("Lösche die alten Dateien im Ordner Downloads", "arquivos"));
        assert!(found("Удали временные файлы", "arquivos"));
        assert!(found("把下载文件夹里的文件移动到桌面", "arquivos"));
        assert!(found("¿Por qué está lento el ordenador? Mira la memoria", "processos"));
        assert!(found("Installe le paquet htop", "pacotes"));
        assert!(found("Annulla l'ultima azione", "linha_do_tempo"));
        assert!(destructive("Удали временные файлы"));
        // positives the word rules must keep: inflections, punctuation, a word in the middle of a sentence
        assert!(found("Suche im Internet nach dem Wetter", "web"));
        assert!(found("Starte den Dienst nginx neu", "servicos"));
        assert!(found("Welche Dienste laufen?", "servicos"));
        assert!(found("Bitte ausführen: ls", "tarefas"));
        assert!(found("Sperre den Port 22", "firewall"));
        assert!(found("Открой порт 22", "firewall"));
        assert!(found("Открой порты 80 и 443", "firewall"));
        assert!(found("Сколько места на диске?", "discos"));
        assert!(found("Покажи диски", "discos"));
        assert!(found("Mira el espacio en disco", "discos"));
        assert!(found("Quanto spazio c'è sul disco?", "discos"));
        assert!(found("Mostra la cartella Download", "arquivos"));
        assert!(found("Mostrami lo stato della rete", "rede"));
        assert!(found("Cosa c'è nella rete?", "rede"));
        assert!(found("Qu'est-ce qui utilise mon compte ?", "usuarios"));
        assert!(found("Crée un compte utilisateur", "usuarios"));
        assert!(found("ログを見せて", "registros"));
        assert!(found("ポートを開けて", "firewall"));
        assert!(found("ファイルを開いて", "arquivos"));
        assert!(found("打开防火墙的端口", "firewall"));
        assert!(found("请检查服务状态", "servicos"));
        for r in ["Borra los archivos temporales", "Lösche die Datei", "Entferne die alten Dateien", "Entfernen Sie den Ordner",
            "Supprime les fichiers", "Efface le cache", "Cancella la cartella", "Pulisci il disco", "Rimuovi il pacchetto",
            "削除してください", "古いファイルを消して", "删除这些文件", "Очисти диск", "Elimina el archivo"] {
            assert!(destructive(r), "{r}");
        }
        for r in ["Recuerda que me llamo Ana", "Merk dir meinen Namen", "Souviens-toi de mon nom", "Ricorda il mio nome", "覚えて", "Запомни это", "记住这个"] {
            assert!(memory(r), "{r}");
        }
    }

    /// Substring matching was the bug: plain `contains` fires on a word that only contains an entry.
    #[test]
    fn plain_substring_search_was_too_loose() {
        assert!("ich versuche es".contains("suche"));
        assert!("wir treffen uns am dienstag".contains("dienst"));
        assert!("экспорт".contains("порт"));
        assert!("ein borrador".contains("borra"));
    }

    #[test]
    fn german_words_that_only_contain_an_entry_do_not_count() {
        assert!(!found("Ich versuche es morgen noch einmal", "web"));
        assert!(!found("Am Dienstag habe ich Zeit", "servicos"));
        assert!(!found("Am Dienstag Abend", "servicos"));
        assert!(!found("Eine ausführliche Erklärung bitte", "tarefas"));
        assert!(!destructive("Das Haus ist 5 km entfernt"));
        assert!(!destructive("Die Stadt ist weit entfernt"));
        assert!(!found("Die Absperrung war offen", "firewall"));
        assert!(!found("Das Rollenmodell der Firma", "modelos"));
        assert!(!found("Das Fernsehprogramm von heute", "pacotes"));
        assert!(!found("Er versucht es seit Tagen", "web"));
    }

    #[test]
    fn russian_words_that_only_contain_an_entry_do_not_count() {
        for (r, d) in [("Это импорт и экспорт товаров", "firewall"), ("Мне нравится спорт", "firewall"), ("Нарисуй портрет", "firewall"),
            ("Пойдём в портал", "firewall"), ("Он купил молоко вместо хлеба", "discos"), ("Это была дискуссия", "discos"),
            ("Ходим на дискотеку", "discos"), ("Салат из редиски", "discos"), ("Льняное волокно", "tela")] {
            assert!(!found(r, d), "{r}");
        }
        assert!(!destructive("Он удалился из комнаты"));
        assert!(!destructive("Они удалились"));
    }

    #[test]
    fn spanish_words_that_only_contain_an_entry_do_not_count() {
        assert!(!destructive("Guarda el borrador del correo"));
        assert!(!destructive("Viene una borrasca"));
        assert!(!destructive("La eliminatoria es mañana"));
        assert!(!found("Es un chico con mucho talento", "processos"));
        assert!(!found("Voy al aeropuerto", "firewall"));
        assert!(!found("Vamos a la discoteca", "discos"));
        assert!(!found("Esa película va a conmover a todos", "arquivos"));
        assert!(!found("Es muy violento", "processos"));
    }

    #[test]
    fn french_words_that_only_contain_an_entry_do_not_count() {
        assert!(!found("Je fais le décompte des points", "usuarios"));
        assert!(!found("Je regarde la balance", "tarefas"));
        assert!(!found("C'est une journaliste", "registros"));
        assert!(!found("Ils dégagent la route", "agentes"));
        assert!(!found("Le reflux gastrique", "fluxos"));
        assert!(!found("Il comptera les jours", "usuarios"));
    }

    #[test]
    fn italian_words_that_only_contain_an_entry_do_not_count() {
        assert!(!found("Vado in discoteca", "discos"));
        assert!(!found("Un discorso importante", "discos"));
        assert!(!found("Il cartello stradale", "arquivos"));
        assert!(!found("Il cartellone del cinema", "arquivos"));
        assert!(!found("Era disposta ad aiutare", "arquivos"));
        assert!(!found("Il terrore del film", "registros"));
        assert!(!destructive("La cancelleria è chiusa"));
    }

    #[test]
    fn japanese_katakana_inside_a_longer_word_do_not_count() {
        assert!(!found("カタログを見せて", "registros"));
        assert!(!found("ダイアログが出た", "registros"));
        assert!(!found("プロファイルを変えて", "arquivos"));
        assert!(!found("サポートに連絡して", "firewall"));
        assert!(!found("レポートを書いて", "firewall"));
        assert!(!found("パスポートを探して", "firewall"));
        assert!(!found("ポートレートを撮って", "firewall"));
        assert!(!found("メモリアルホールへ行く", "processos"));
        assert!(!found("ログインして", "registros"));
    }

    #[test]
    fn chinese_words_that_contain_an_entry_without_meaning_it_do_not_count() {
        assert!(!found("服务员来了", "servicos"));
        assert!(!found("他是代理商", "agentes"));
        assert!(!found("请系好安全带", "kernel"));
        assert!(!found("历史课在三点", "linha_do_tempo"));
        assert!(!found("参观空间站", "discos"));
        assert!(found("检查系统安全", "kernel"));
    }

    #[test]
    fn whole_word_entries_end_at_the_word() {
        assert!(has_word("открой порт", "порт$"));
        assert!(has_word("порт.", "порт$"));
        assert!(!has_word("портрет", "порт$"));
        assert!(has_word("compte", "compte$"));
        assert!(!has_word("comptera", "compte$"));
        assert!(has_word("ein dienst", "dienst"));
        assert!(!has_word("eindienst", "dienst"));
    }
}
