# Changelog

Todas as mudanças relevantes do KalangoOS ficam aqui. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/). Os números `#N` são pull requests do repositório
[kalangoos/kalangoos](https://github.com/kalangoos/kalangoos/pulls?q=is%3Apr+is%3Amerged). As versões 0.x são
prévias de desenvolvimento ([docs/LANCAMENTO.md](docs/LANCAMENTO.md)).

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
