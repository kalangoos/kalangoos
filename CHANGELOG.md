# Changelog

Todas as mudanças relevantes do KalangoOS ficam aqui. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/). Os números `#N` são pull requests do repositório
[kalangoos/kalangoos](https://github.com/kalangoos/kalangoos/pulls?q=is%3Apr+is%3Amerged). As versões 0.x são
prévias de desenvolvimento ([docs/LANCAMENTO.md](docs/LANCAMENTO.md)).

## [0.1.2] — 2026-11-26

Prévia técnica, como a 0.1.0 e a 0.1.1. Traz a interface em nove idiomas, os Apps criativos instaláveis a pedido, o
gatilho de entrada pela internet e a criação de fluxos pelo chat por etapas para o modelo pequeno. O Kalango Remote e o
Kalango Audio fixados na esteira de lançamento sobem para os commits com os mesmos idiomas; o que o `kalangod` exige do
Remote não mudou desde a 0.1.1 (o commit novo do Remote só traz catálogos de idioma).

### Mudanças de padrão

- **Fluxos pelo chat: `fluxos.criar` (o JSON inteiro) deixou de ser oferecido ao modelo por padrão** (#61). Um fluxo novo
  nasce só pelas ferramentas de rascunho (`fluxos.planejar`, `fluxos.adicionar_bloco`, `fluxos.ligar`, `fluxos.ajustar`,
  `fluxos.concluir`), que leem os passos com o modelo de decisão d1 e sempre terminam na aprovação da pessoa.
  `KALANGOD_FLUXOS_AUTORIA=classico` (só o JSON) ou `=etapas` (as duas formas) restauram o comportamento anterior;
  `fluxos.editar` continua disponível. Detalhes e medições: [docs/FLUXOS-NO-CHAT.md](docs/FLUXOS-NO-CHAT.md).
- **Pedidos para criar um fluxo começam por `fluxos.planejar`** (a primeira chamada do modelo é forçada) e o motor lembra
  de chamar `fluxos.concluir` quando o modelo tenta responder com um rascunho aberto (#61). Nada é forçado para ações que a
  pessoa pôs em "nunca" ou desligou, e a criação continua pedindo aprovação.

### Adicionado

- **Idiomas** (#64): a interface passa a existir em nove idiomas, os do site: English, Português (Brasil), Español,
  Français, Italiano, Deutsch, 日本語, Русский e 简体中文. O idioma vem do sistema (`LANGUAGE`, `LC_*`, `LANG`), e o
  instalador e as Configurações › Idioma o escolhem. A imagem leva a fonte Noto Sans CJK (peso regular).
- **Aviso sobre a qualidade:** os sete catálogos novos (es, fr, it, de, ja, ru, zh_CN) foram **gerados por IA e
  nenhuma pessoa nativa os revisou**. Os testes conferem a forma (chaves, marcadores, atalhos, caracteres ocultos,
  palavras de consentimento), não o sentido nem o tom. As Configurações › Idioma dizem isso na tela. Como ajudar:
  [docs/i18n/REVISAO.md](docs/i18n/REVISAO.md).
- O texto que o modelo de IA lê (instruções, recusas, limites) fica em inglês nos idiomas novos, e as palavras de
  outros idiomas que trazem ferramentas para o pedido casam por palavra, não por trecho.
- **Gatilho de entrada pela internet** (#60): um endereço público e assinado (HMAC-SHA256) que inicia um fluxo, por um
  túnel da Cloudflare aberto só depois de uma aprovação. Ferramentas no `kalangod`, a tela de túneis em Ajustes e o
  gatilho no editor de fluxos do shell. O receptor e o túnel moram no serviço de fluxos da pessoa (`kalango-fluxosd`, do
  Kalango Fluxos). Detalhes e modelo de ameaça: [docs/GATILHO-INTERNET.md](docs/GATILHO-INTERNET.md).
- **Apps criativos** (#62): os 17 apps do projeto Craft podem ser instalados pela Central de Apps, a pedido, e usados
  pelos agentes pelo servidor MCP de cada um, com níveis (confirmar ou aprovar) e cartão de aprovação. O download confere a
  assinatura do projeto; a instalação é por Flatpak, com permissões (override) que negam as pastas sensíveis. Os apps
  **não vão na ISO**. Um override global amplo do Flatpak feito pela própria pessoa (por exemplo no Flatseal) bloqueia o
  uso dos apps pelos agentes até ser removido. Detalhes: [docs/APPS-CRIATIVOS.md](docs/APPS-CRIATIVOS.md).
- `agentes.criar` e `agentes.editar` aceitam `output_fields` (os campos da saída estruturada de um agente), para que um
  passo seguinte do fluxo leia cada campo (#61).

### Segurança

- **Apps criativos: uma pasta concedida não pode mais ser trocada por um link** (#65). Uma pasta escolhida para um app
  virava um override permanente do Flatpak; quem escrevesse num ancestral podia trocá-la por um link para `~/.ssh`, e,
  quando a pasta deixava de passar, o override antigo continuava valendo. Agora a escolha recusa caminhos com link,
  pastas dentro das pastas onde os agentes escrevem, pastas de outro app criativo e ancestrais que outros usuários podem
  mudar (permissões ou ACL), guarda a identidade (dispositivo, inode) de cada pasta até a casa e confere tudo de novo a
  cada vez que refaz o override: ao entrar na sessão, antes de o shell abrir um app criativo por qualquer caminho, antes
  do servidor MCP e quando quem escreve em volta muda. Uma pasta que mudou sai do override e vai para a Linha do tempo.
- **Gatilho pela internet: o cartão para ligar um gatilho diz o que ele é** (#65). O pedido do serviço mostrava um cartão
  genérico ("quem tem a chave…") mesmo para um gatilho público; agora os dois caminhos mostram assinatura (NENHUMA =
  público), nível e endereço, e depois do sim o `kalangod` lê o gatilho de novo e só confirma se ele ainda é o do cartão.

### Esteira de lançamento e repositório

- O Kalango Remote fixado sobe para `e1795114` (interface nos mesmos nove idiomas, kalangoos/kalango-remote#14) e o
  Kalango Audio para `3e68e343` (kalangoos/kalango-audio#9, só idiomas). O isolamento de voz do Audio
  (kalangoos/kalango-audio#10) fica para a próxima versão.
- A ação `preparar` da CI instala o Flatpak, para que o teste do override real dos Apps criativos rode (#63).
- Saíram dois restos de mescla esquecidos no repositório (`server.rs.orig` e `pt_BR.json.orig`).
- O Kalango VirtualDesk e o Kalango Fluxos continuam fora da ISO: a leitura dos repositórios privados pela esteira (o
  GitHub App só de leitura) ainda não está ligada e fica fora da 0.1.2.

### Problemas conhecidos da 0.1.2

- Traduções de IA sem revisão por pessoas nativas; sem conferência visual das telas em cada idioma, nem de datas,
  números e glifos CJK na ISO instalada; o modelo local não foi testado com conversas nos idiomas novos.
- Criar fluxos pelo chat com o modelo pequeno acerta cerca de **2 em 3** pedidos novos (67 % no conjunto C; 64 % e 62 %
  nos conjuntos D e E, mais recentes), abaixo do alvo de 90 %; por isso o `fluxos.criar` ficou desligado por padrão.
- O serviço de fluxos (`kalango-fluxosd`) não está na ISO, então o gatilho pela internet e os fluxos não rodam na imagem
  publicada. O gatilho nunca foi exercitado com um túnel real da Cloudflare, e o serviço ainda ignora o `esperado` da
  resposta de `autorizar_ampliacao` (a recusa acontece no `kalangod`).
- Apps criativos: a instalação ponta a ponta pelo `kalangod` (o runtime de 283 MB), o modo em janela, a atualização
  automática e abrir os 17 apps com as novas negações não foram validados; o detalhe da Central ainda não marca uma pasta
  recusada (o aviso chega por notificação e pela Linha do tempo).
- O teste da ISO na esteira não exercita a tela de aprovação gráfica nem a tela do Remote; elas foram corrigidas na 0.1.1,
  e as listas do `kalangod` que as liberam não mudaram desde então.
- Continuam valendo os problemas conhecidos da 0.1.1 e as limitações da 0.1.0.

## [0.1.1] — 2026-11-25

Correção de segurança e de uso da 0.1.0, achada ao testar a ISO publicada em máquinas virtuais (duas VMs, uma controlando
a outra). **O `kalangod` desta versão exige o Kalango Remote desta versão: os dois saem juntos** (o núcleo da 0.1.0 é
recusado por ser depurável) e o pin do Remote na esteira de lançamento sobe junto.

### Corrigido

- **A tela de aprovação gráfica era recusada** na imagem instalada ("a aprovação precisa ser feita por você: num terminal…"),
  assim como o shell como interface da pessoa e o teclado na tela (`Keyboard → false` no niri): `/usr/bin/kalango-shell` e
  `kalango-aprovacao` são scripts que executam os `.bin`, e as listas do `kalangod` (`interface_da_pessoa`,
  `aprovadores_graficos`, `controle_remoto_sempre`) apontavam para os scripts. Agora nomeiam o programa que o kernel reporta,
  e o `LD_PRELOAD=libgtk4-layer-shell.so.0` dos scripts é permitido exatamente.
- **O computador controlado pelo Kalango Remote nunca mostrava a tela** na imagem instalada: o app roda como
  `/usr/lib/kalango/remote/kalango-remote`, que não estava em `controle_remoto_apps`.
- O script `/usr/bin/kalango-remote` tira `LD_PRELOAD` e `LD_AUDIT` do ambiente (o shell que abre o app pelo dock os tem) e o
  shell não passa o `LD_PRELOAD` dele aos apps que abre.
- **O código do cartão do Remote na mesma rede, no modo túnel do Cloudflare, agora é uma escolha da pessoa**: o ajuste
  "Aceitar também conexões na mesma rede" (desligado por padrão) faz o app escutar também na rede local (`--lan`, porta
  qualquer) enquanto compartilha, e o túnel continua; a senha e a aprovação seguem valendo. Desligado, o modo túnel escuta só
  neste computador, como na 0.1.0 (uma porta aberta numa rede pública não é o que compartilhar pelo túnel deve fazer), e quem
  digita o código ouve o que precisa ser ligado. Quando o app escuta na rede (esse ajuste ou "Só rede local direta"), só entram
  endereços locais (loopback, faixas privadas e link-local; a lista de IPs do RustDesk, conferida antes da senha), e só eles
  recebem a resposta da busca pelo código. Endereços do Tailscale e outros 100.64.0.0/10 (CGNAT) **não** contam como rede
  local. O ajuste vale a partir do próximo compartilhamento: desligá-lo no meio de um compartilhamento não tira a proteção
  do que já escuta na rede. Os campos "Aceitar só os aparelhos desta lista", "Endereços permitidos" e "Aceitar conexão por
  endereço IP" ainda não funcionam e agora aparecem desativados, com "Ainda não funciona nesta versão". O antigo
  "Preferir conexão direta na mesma rede", que não fazia nada no modo túnel, saiu. E a consulta do código vai ao endereço de
  difusão de cada rede, não só ao da rota padrão (com duas placas ela não achava).
- `is_engine` (marcas de confiança do d1) comparava só o nome do arquivo; agora é o caminho completo do motor instalado.
- **Kalango Remote: o mouse de quem controla não chegava ao outro computador** (só o teclado): a área de transferência de
  arquivos, fechada, cobria a imagem da sessão e recebia os cliques e movimentos. Já acontecia na 0.1.0.
- Kalango Remote: com o acesso recusado pelo sistema (kalangod), o computador compartilhado não mostra mais "… está
  controlando este computador"; a conexão é fechada e o aviso diz o motivo.

### Segurança

- O Remote só recebe captura, entrada e cobertura de privacidade se o app e o núcleo não puderem ser depurados nem lidos
  pelos outros processos da pessoa (`prctl(PR_SET_DUMPABLE, 0)` logo no começo de cada um, conferido pelo `kalangod`), sem
  depurador, sem carga de biblioteca ou plugin estranha no ambiente (`LD_PRELOAD`, `LD_AUDIT`, `LD_LIBRARY_PATH` que não o do
  script, camadas Vulkan, plugins do PipeWire e do GStreamer, módulos do GTK e do GIO…) e, como root, sem código executável
  mapeado de fora das pastas do sistema (`/proc/<pid>/maps`).
- A permissão **cai em até 1 s** se o núcleo deixar de ser seguro e, como o niri só olha o arquivo quando um cliente conecta,
  o `kalangod` **encerra o núcleo** (SIGKILL, depois de conferir pid e hora de início) e registra na Linha do tempo.
- Como root, o `kalangod` confia no que o **kernel** diz do processo, não no texto do caminho: mesmos namespaces de montagem e
  de usuário que ele (um `bwrap` ou `unshare` monta arquivos falsos sobre `/usr/lib/kalango/remote/*`), mesmo arquivo (dispositivo e
  inode de `/proc/<pid>/exe`), e cada mapeamento executável pelo `map_files`: do root, sem escrita de outros, de `/usr/lib/` ou
  `/lib/`, nada `(deleted)`, e nenhuma memória anônima executável no núcleo. A conferência se repete a cada segundo.
- Limites conhecidos, documentados em `docs/SEGURANCA.md`: a janela entre o `exec` e o `prctl` do app e do núcleo (um
  processo que se anexa nela e solta antes do `prctl` não é visto), e o niri que identifica o cliente por pid (correção
  planejada com pidfd).

### Problemas conhecidos da 0.1.1

- Áudio da sessão remota com o núcleo não depurável: não verificado (as VMs de teste não têm placa de som).

### Kit de validação em hardware

- `imagem/validar-hardware.sh` (relatório sem dados pessoais, com testes em `imagem/testar-validar-hardware.sh`),
  `docs/VALIDACAO-HARDWARE.md` e o formulário de issue "Teste em hardware real".
- A imagem mascara `NetworkManager-wait-online.service` (nada nela espera "rede no ar"; falhava depois de ~60 s sem cabo).

## [0.1.0] — 2026-11-22

Primeira prévia técnica: uma ISO instalável, testada numa máquina virtual, com o shell, o `kalangod` e os agentes
locais. Notas completas: [docs/NOTAS-0.1.0.md](docs/NOTAS-0.1.0.md).

### Imagem, instalador e lançamento

- ISO instalável com imagem própria (pacotes do Ubuntu 26.04 LTS, kernel do KalangoOS 7.3.4, niri do fork e llama.cpp
  em commits fixos), sessão ao vivo com o instalador e teste numa VM: ao vivo, instalação num disco vazio e primeiro
  boot do sistema instalado (#3, #5, #10).
- Esteira de lançamento: compila o kernel (com cache), monta a ISO, instala numa VM e, numa tag, anexa ao rascunho da
  versão a ISO testada, as somas SHA-256 e o atestado de origem do build (#6). Ajustes da esteira: o kernel compila
  com caminhos absolutos e com `build-essential` (#7, #8) e a esteira compila também o `kalango-tela` (#30).
- A esteira monta também o Kalango Remote e o Kalango Audio, de repositórios próprios em commits fixos, e os leva na
  imagem (#9, #11, #12, #13, #15).
- Instalador: etapa "Modelos de IA" e a fonte dos modelos (catálogo público, servidor privado ou pasta) também em
  Ajustes e na Central de Apps; downloads com retomada e soma SHA-256 conferida; o token do servidor fica preso à
  origem (#33).
- A ISO da esteira sai sem o modelo local, com as licenças dos modelos em revisão (#6).
- Pastas do sistema 755 e conferidas no teste da ISO; `/usr/local` sem escrita do grupo `staff` (#29).
- ISO de teste local opcional com todos os modelos (`KALANGO_IMAGEM_TODOS_MODELOS=1`) e correção do teste de
  instalação (#39).

### Shell e janelas

- Shell completo em Rust e GTK 4 sobre o niri com patches: barra, dock, barras laterais, área de trabalho com os
  arquivos da pasta, bloqueio e tela de entrada (#5, #10).
- Janelas de apps fechadas liberam a superfície gráfica após 60 s e a memória deixa de crescer (de cerca de 8 GB para
  cerca de 0,7 GB em 20 ciclos de abrir e fechar, em renderização por software) (#35).
- Terminal suspenso (F12), SSH e Mosh (ainda sem teste) no Terminal com painel SFTP, foto da conta e sons do sistema (#5, #10, #18, #19).
- Código igual em todo o shell; LaTeX desenhado em páginas, isolado e sem rede (#26).
- Chat com blocos aninhados, diagramas de sequência com zoom, LaTeX como documento e PDF, HTML no Canvas e cores do
  código (#24); Central de agentes responsiva e Canvas largo (#25).
- Telas migradas do Design System, 22 delas em #10; o inventário está em [docs/telas.md](docs/telas.md).
- Ajuda: fechar a janela para a escuta e descarta a gravação (#38).
- Pendências da 0.1.0: testes do motor, o d1-Omni-2B no catálogo da Central de Apps e uma pergunta só nas ferramentas
  `vm.*` (#40).

### `kalangod`, segurança e níveis de poder

- Aprovação em tela desenhada pelo sistema, desfazer e Linha do tempo, com quatro níveis de poder (observar, usuário,
  sistema, total).
- Agentes operam a tela pela visão (ferramentas `tela.*`), com aprovação por janela e recusas de segurança (#20).
- A configuração do KalangoOS fica fora do alcance dos agentes (#22).
- Arquivos que rodam ao entrar e pastas do `PATH` só mudam sob aprovação; comandos de agente presos ao próprio grupo
  de processos (#27).
- Lista branca: comandos e ferramentas de arquivo dos agentes só escrevem nas pastas liberadas (#28).
- Defesa contra vazamento: dado local colado, repartido ou codificado no endereço da web é barrado nos casos
  cobertos; os limites estão em [docs/SEGURANCA.md](docs/SEGURANCA.md) (rot13, inverter, base32/58, XOR, ruído
  intercalado, segredo repartido entre parâmetros ou rótulos de DNS, base64 em minúsculas num rótulo de DNS, corpo POST e fragmento não são cobertos) (#36).
- `url` 2.5.9, que corrige RUSTSEC-2026-0592 (#37).
- Máquinas virtuais do Kalango VirtualDesk como ferramentas `vm.*` (#16), GPUs para máquinas virtuais por vfio-pci
  com a GPU da tela protegida pelo grupo IOMMU (#17) e máquinas com rede conferida, cópias de sandbox e rede sem
  agentes (#31).

### Agentes locais, Fluxos e contexto

- `kalango-agente` com o LFM3 local pelo llama.cpp; o contexto do modelo é escolhido pela RAM e os comandos de
  estado do sistema têm limite de tempo (#14).
- Plataforma de agentes e fluxos: ferramentas no `kalangod`, modelos no motor e as telas da Central (#21); editor de
  fluxo completo com passos animados, tipos de agente pela tela e Ajustes (#23).
- Editor de fluxo: ligações nunca trocam de porta, portas com nome e roteador que contorna os cartões (#32).
- Decisões do modelo d1 nos blocos "Decidir" dos Fluxos (o código está em `crates/kalango-agente` e no editor de
  fluxo do shell).
- Aprovação com o d1: a tela de aprovação mostra a recomendação ("parece seguro" ou "parece perigoso", confiança e um
  motivo de uma lista fechada; a decisão é da pessoa). Opção "Aprovar sozinho o que é de baixo risco" em Permissões,
  desligada por padrão: só três ações reversíveis na pasta da pessoa (lixeira, mover, criar arquivo novo que não
  roda), só com o d1 seguro com pelo menos a confiança mínima, nunca depois de conteúdo lido de fora, no máximo 5 por pedido,
  desliga sozinha depois de 3 recusas; o `kalangod` confere de novo e instalar pacotes nunca é "seguro" (#42).
- Ferramenta `remoto.conectar`: o agente propõe uma conexão a um computador da pessoa pelo painel do plano do Remote;
  só o clique de uma pessoa aprova (#44).
- Gestão de contexto da conversa: janela recente, resumo em segundo plano e busca na conversa (#34).

### Aplicativos

- Kalango Remote (acesso remoto, SSH e Mosh; o Mosh ainda sem teste) e Kalango Audio na imagem (#9, #10).
- Kalango Remote: senha fixa e acesso sem supervisão com permissões limitadas (sem arquivos, terminal, câmera, túnel,
  reinício, bloqueio de entrada nem modo de privacidade; o aviso lista o que foi concedido), verificação em duas etapas
  (TOTP), gravação de sessões (Vídeos › Kalango Remote, 30 dias, com aviso à pessoa gravada), servidor próprio
  compatível com o RustDesk, bloqueio global depois de tentativas erradas e painel do plano do agente
  (`kalango-remote agent-plan`) (kalangoos/kalango-remote#11 e #12).
- Modo de privacidade do Remote ("tela preta aqui"): com um acesso que ninguém aprovou neste computador, a tela local
  fica coberta e quem controla vê a área de trabalho de verdade; saídas de emergência Esc Esc, troca de terminal
  virtual, botão de ligar e bloqueio. Patch 0012 do niri (kalangoos/niri#4) e a ISO leva o Remote com ele (#44, #46).
- Kalango VirtualDesk integrado ao `kalangod` e aos Ajustes › Sandbox (#16, #25, #31); fora da ISO.

### Conhecido / pendente

- Hardware real: testada só em máquina virtual; a matriz NVIDIA, AMD e Intel não está completa.
- A ISO da esteira não leva modelo local (licenças em revisão); o modelo vem pela etapa "Modelos de IA" ou pela
  Central de Apps.
- Sem certificação de acessibilidade; interface só em português do Brasil.
- Kalango Remote: não testado entre dois computadores físicos nem numa sessão real (não aninhada) do niri; Mosh não
  testado; o caminho da tela de bloqueio e o quadro de transição sob a cobertura do modo de privacidade só são
  cobertos pelo código, não por um teste.
- Ideias do [roteiro](ROADMAP.md#integrações-planejadas-ideias-sem-data) ainda não feitas: fonte de áudio ao vivo para
  fluxos, gatilho de entrada pela internet por túnel da Cloudflare e o Remote dentro dos fluxos.
- Licença CC BY-SA 4.0 da documentação ainda a confirmar ([docs/LICENCAS.md](docs/LICENCAS.md)).
- Kalango VirtualDesk e Kalango Fluxos não vão na ISO (o serviço de fluxos está num repositório privado).
- Atualizações com ponto de restauração e tela de recuperação (fase 5) pendentes.
- Limites da defesa de endereços da web (rot13, inverter, base32/58, XOR, ruído intercalado, segredo repartido entre
  parâmetros ou rótulos de DNS, base64 em minúsculas num rótulo de DNS, corpo POST e fragmento): [docs/SEGURANCA.md](docs/SEGURANCA.md).
- Registro da marca e revisão dos termos do serviço que gerou o logo e os ícones.

---

## English summary

**Unreleased.** The UI exists in nine languages (English, Brazilian Portuguese, Spanish, French, Italian, German, Japanese, Russian, Simplified Chinese). The seven catalogs beyond Portuguese are AI-generated and have not been reviewed by native speakers; Settings › Language says so, and [docs/i18n/REVISAO.md](docs/i18n/REVISAO.md) says where a review helps most. Text for the AI model stays in English in the new languages.

**0.1.0 (2026-11-22), the first technical preview.** An installable ISO (x86_64, UEFI) that the release workflow
builds and tests in a VM (live session, installer, installed system boots), the shell on a patched niri, `kalangod`
(approval screen, undo, timeline, four power levels), local agents (LFM3 through llama.cpp), flow editor, context
management, Kalango Remote and Kalango Audio on the image, a model source chosen in the installer (public catalog,
private server or folder), and a fix for window memory growth. Known gaps: not validated on real hardware, no local
model in the ISO (licences under review), no accessibility certification, Portuguese-only UI, and Kalango Remote
untested between two physical computers nor in a real (non-nested) niri session; Mosh untested; the lock-screen path and the transition frame under the privacy cover are covered by code only. Also in 0.1.0: the approval screen shows the d1 model's
recommendation (an opt-in "approve low-risk actions on my own", off by default, limited to three reversible file
actions, #42); Kalango Remote with a fixed password and unattended access with limited permissions, two-step
verification, session recording, its own server and a privacy mode ("black screen here", niri patch 0012;
kalangoos/kalango-remote#11 and #12, kalangoos/niri#4); the `remoto.conectar` agent tool (#44); and a defence against
local data leaving in a web address in the covered cases (#36, limits in docs/SEGURANCA.md). PR numbers refer to
[kalangoos/kalangoos](https://github.com/kalangoos/kalangoos) unless a repository is named.

[0.1.0]: https://github.com/kalangoos/kalangoos/releases
