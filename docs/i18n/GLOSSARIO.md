# Glossário de tradução

Os termos do KalangoOS em cada idioma. Vale para o sistema (`crates/kalango-i18n/locales/`), para os apps (Kalango
Remote, Kalango Audio, Kalango VirtualDesk, Kalango Flow) e, como referência, para o site. Traduza a partir do inglês
(a chave do catálogo); o português serve só para tirar dúvidas de sentido.

## Regras gerais

- **Tratamento da pessoa** (igual ao do site): alemão *Sie*; francês *vous*; espanhol *tú*; italiano *tu*; russo
  *вы* (minúsculo no meio da frase); japonês estilo です/ます nas frases, substantivos ou formas curtas em botões e
  rótulos; chinês sem pronome de cortesia especial (您 só onde o inglês pede um tom formal; o padrão é 你).
- **"the person"** (quem usa o computador): a palavra comum do idioma — de *Sie* quando a frase fala com a pessoa,
  *die Person* / *der Benutzer* em terceira pessoa; es/it *la persona*; fr *la personne*; ja *ユーザー*; ru
  *пользователь*; zh *用户*.
- **Nomes que não se traduzem**: KalangoOS, Kalango Remote, Kalango Audio, Kalango VirtualDesk, Kalango Flow,
  Kalango Cast, kalangod, d1, Moli, LFM3, llama.cpp, niri, MCP, A2A, Wi-Fi, Bluetooth, USB, GPU, nomes de
  ferramentas (`arquivos.ler`, `idioma.sistema`), caminhos, comandos, variáveis de ambiente, teclas (Ctrl, Alt,
  Super, Shift, Enter) e marcadores técnicos como `<<<DADOS EXTERNOS`.
- **Partes genéricas** se traduzem quando o `pt_BR` também traduz: *App Center* → Central de Apps.
- **`{marcadores}`**, marcação (`<b>`, `<span …>`), `\n`, `%s`, e os espaços no começo e no fim do texto ficam
  exatamente como no inglês (o código junta pedaços).
- **Números** (`{n}`): o texto precisa servir para qualquer número, porque o código não escolhe forma de plural. Em
  russo, prefira a construção com dois-pontos ou genitivo invariável (*Запросов: {n}*, *Выбрано: {n}*); em alemão,
  francês, espanhol e italiano, use o plural quando o inglês usa o plural.
- **Pontuação**
  - fr: espaço inseparável (U+00A0) antes de `:` `;` `?` `!` e dentro de « » quando são pontuação de texto (não em
    horas `12:30`, endereços, código).
  - de: aspas „…“; es: «…» ou “…”; it: «…»; ru: «…»; ja: 「…」; zh: “…”.
  - ja/zh: pontuação de largura total (、。：？！（）) no texto corrido; sem espaços extras entre ideogramas e
    palavras latinas ou marcadores (`{n}件`, `{name}を開く`).
  - Reticências `…` (um caractere) como no inglês.
- **Maiúsculas**: rótulos em MAIÚSCULAS no inglês (títulos de cartão, `LANGUAGE AND REGION`) ficam em maiúsculas nas
  línguas com caixa; em ja/zh não há caixa.

## Termos

| Inglês | Deutsch | Español | Français | Italiano | 日本語 | Русский | 简体中文 |
|---|---|---|---|---|---|---|---|
| agent | Agent | agente | agent | agente | エージェント | агент | 智能体 |
| Agents (app/central) | Agenten | Agentes | Agents | Agenti | エージェント | Агенты | 智能体 |
| flow (automation) | Flow | flujo | flux | flusso | フロー | сценарий | 流程 |
| trigger | Auslöser | disparador | déclencheur | trigger | トリガー | триггер | 触发器 |
| approval | Genehmigung | aprobación | approbation | approvazione | 承認 | одобрение | 批准 |
| approve / refuse | Genehmigen / Ablehnen | Aprobar / Rechazar | Approuver / Refuser | Approva / Rifiuta | 承認 / 拒否 | Одобрить / Отклонить | 批准 / 拒绝 |
| deny | Verweigern | Denegar | Refuser | Nega | 拒否 | Запретить | 拒绝 |
| allow | Erlauben | Permitir | Autoriser | Consenti | 許可 | Разрешить | 允许 |
| permission | Berechtigung | permiso | autorisation | permesso | 権限 | разрешение | 权限 |
| Timeline | Zeitleiste | Cronología | Chronologie | Cronologia | タイムライン | Хронология | 时间线 |
| undo | Rückgängig machen | Deshacer | Annuler | Annulla | 元に戻す | Отменить | 撤销 |
| level (power level) | Stufe | nivel | niveau | livello | レベル | уровень | 级别 |
| restore point | Wiederherstellungspunkt | punto de restauración | point de restauration | punto di ripristino | 復元ポイント | точка восстановления | 还原点 |
| vault (secrets) | Tresor | bóveda | coffre-fort | cassaforte | 保管庫 | хранилище | 保险库 |
| secret | Geheimnis | secreto | secret | segreto | シークレット | секрет | 密钥 |
| tunnel | Tunnel | túnel | tunnel | tunnel | トンネル | туннель | 隧道 |
| connector | Konnektor | conector | connecteur | connettore | コネクタ | коннектор | 连接器 |
| tool | Werkzeug | herramienta | outil | strumento | ツール | инструмент | 工具 |
| model | Modell | modelo | modèle | modello | モデル | модель | 模型 |
| memory (agent) | Gedächtnis | memoria | mémoire | memoria | メモリー | память | 记忆 |
| memory (RAM) | Arbeitsspeicher | memoria | mémoire | memoria | メモリ | память | 内存 |
| request (to the agent) | Anfrage | solicitud | demande | richiesta | リクエスト | запрос | 请求 |
| task | Aufgabe | tarea | tâche | attività | タスク | задача | 任务 |
| routine (scheduled) | Routine | rutina | routine | routine | ルーティン | регулярная задача | 例行任务 |
| Settings | Einstellungen | Configuración | Paramètres | Impostazioni | 設定 | Настройки | 设置 |
| App Center | App-Center | Centro de apps | Centre d'applications | Centro app | アプリセンター | Центр приложений | 应用中心 |
| Files | Dateien | Archivos | Fichiers | File | ファイル | Файлы | 文件 |
| folder | Ordner | carpeta | dossier | cartella | フォルダー | папка | 文件夹 |
| Trash | Papierkorb | Papelera | Corbeille | Cestino | ゴミ箱 | Корзина | 回收站 |
| Spaces (workspaces) | Arbeitsbereiche | Espacios | Espaces | Spazi | スペース | Пространства | 空间 |
| tokens (of a model) | Tokens | Tokens | Jetons | Token | トークン | Токены | 词元 (令牌 is only a credential: "Access token" → 访问令牌) |
| dock | Dock | dock | dock | dock | ドック | док | 程序坞 |
| sidebar | Seitenleiste | barra lateral | barre latérale | barra laterale | サイドバー | боковая панель | 侧边栏 |
| notification | Benachrichtigung | notificación | notification | notifica | 通知 | уведомление | 通知 |
| lock screen | Sperrbildschirm | pantalla de bloqueo | écran de verrouillage | schermata di blocco | ロック画面 | экран блокировки | 锁屏 |
| sign in / sign out | Anmelden / Abmelden | Iniciar sesión / Cerrar sesión | Se connecter / Se déconnecter | Accedi / Esci | サインイン / サインアウト | Войти / Выйти | 登录 / 注销 |
| session | Sitzung | sesión | session | sessione | セッション | сеанс | 会话 |
| update | Update / Aktualisierung | actualización | mise à jour | aggiornamento | アップデート | обновление | 更新 |
| install | Installieren | Instalar | Installer | Installa | インストール | Установить | 安装 |
| installer | Installationsprogramm | instalador | programme d'installation | programma di installazione | インストーラー | установщик | 安装程序 |
| try it (live) | Ausprobieren | Probar | Essayer | Prova | 試す | Попробовать | 试用 |
| package | Paket | paquete | paquet | pacchetto | パッケージ | пакет | 软件包 |
| service | Dienst | servicio | service | servizio | サービス | служба | 服务 |
| process | Prozess | proceso | processus | processo | プロセス | процесс | 进程 |
| disk | Datenträger | disco | disque | disco | ディスク | диск | 磁盘 |
| network | Netzwerk | red | réseau | rete | ネットワーク | сеть | 网络 |
| firewall | Firewall | cortafuegos | pare-feu | firewall | ファイアウォール | брандмауэр | 防火墙 |
| virtual machine | virtuelle Maschine | máquina virtual | machine virtuelle | macchina virtuale | 仮想マシン | виртуальная машина | 虚拟机 |
| snapshot (VM) | Snapshot | instantánea | instantané | snapshot | スナップショット | снимок | 快照 |
| remote support | Fernsupport | soporte remoto | assistance à distance | assistenza remota | リモートサポート | удалённая поддержка | 远程协助 |
| screen sharing | Bildschirmfreigabe | compartir pantalla | partage d'écran | condivisione schermo | 画面共有 | демонстрация экрана | 屏幕共享 |
| recording | Aufnahme | grabación | enregistrement | registrazione | 録音 / 録画 | запись | 录制 |
| prompt (to a model) | Prompt | prompt | prompt | prompt | プロンプト | промпт | 提示词 |
| hidden instructions (prompt injection) | versteckte Anweisungen | instrucciones ocultas | instructions cachées | istruzioni nascoste | 隠された指示 | скрытые инструкции | 隐藏指令 |
| reversible / irreversible | umkehrbar / nicht umkehrbar | reversible / irreversible | réversible / irréversible | reversibile / irreversibile | 元に戻せる / 元に戻せない | обратимо / необратимо | 可撤销 / 不可撤销 |
| read only | nur Lesen | solo lectura | lecture seule | sola lettura | 読み取りのみ | только чтение | 只读 |
| writes / destructive | schreibt / zerstörerisch | escribe / destructiva | écrit / destructrice | scrive / distruttiva | 書き込み / 破壊的 | запись / разрушительное | 写入 / 破坏性 |
| administrator | Administrator | administrador | administrateur | amministratore | 管理者 | администратор | 管理员 |
| password | Passwort | contraseña | mot de passe | password | パスワード | пароль | 密码 |
