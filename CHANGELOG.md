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
- Terminal suspenso (F12), SSH e Mosh no Terminal com painel SFTP, foto da conta e sons do sistema (#5, #10, #18, #19).
- Código igual em todo o shell; LaTeX desenhado em páginas, isolado e sem rede (#26).
- Chat com blocos aninhados, diagramas de sequência com zoom, LaTeX como documento e PDF, HTML no Canvas e cores do
  código (#24); Central de agentes responsiva e Canvas largo (#25).
- Telas migradas do Design System, 22 delas em #10; o inventário está em [docs/telas.md](docs/telas.md).
- Ajuda: fechar a janela para a escuta e descarta a gravação (#38).

### `kalangod`, segurança e níveis de poder

- Aprovação em tela desenhada pelo sistema, desfazer e Linha do tempo, com quatro níveis de poder (observar, usuário,
  sistema, total).
- Agentes operam a tela pela visão (ferramentas `tela.*`), com aprovação por janela e recusas de segurança (#20).
- A configuração do KalangoOS fica fora do alcance dos agentes (#22).
- Arquivos que rodam ao entrar e pastas do `PATH` só mudam sob aprovação; comandos de agente presos ao próprio grupo
  de processos (#27).
- Lista branca: comandos e ferramentas de arquivo dos agentes só escrevem nas pastas liberadas (#28).
- Defesa contra vazamento: dado local colado, repartido ou codificado no endereço da web não sai; os limites
  conhecidos estão em [docs/SEGURANCA.md](docs/SEGURANCA.md) (#36).
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
- Gestão de contexto da conversa: janela recente, resumo em segundo plano e busca na conversa (#34).

### Aplicativos

- Kalango Remote (acesso remoto, SSH e Mosh) e Kalango Audio na imagem (#9, #10).
- Kalango VirtualDesk integrado ao `kalangod` e aos Ajustes › Sandbox (#16, #25, #31); fora da ISO.

### Conhecido / pendente

- Hardware real: testada só em máquina virtual; a matriz NVIDIA, AMD e Intel não está completa.
- A ISO da esteira não leva modelo local (licenças em revisão); o modelo vem pela etapa "Modelos de IA" ou pela
  Central de Apps.
- Sem certificação de acessibilidade; interface só em português do Brasil.
- Kalango Remote: modo privacidade, verificação em duas etapas, gravação de sessões e servidor próprio de
  compartilhamento ainda não existem.
- Kalango VirtualDesk e Kalango Fluxos não vão na ISO (o serviço de fluxos está num repositório privado).
- Atualizações com ponto de restauração e tela de recuperação (fase 5) pendentes.
- Limites conhecidos da defesa de endereços da web (inverter, rot13, base32/58, XOR, segredo repartido entre
  parâmetros ou rótulos de DNS, corpos POST): [docs/SEGURANCA.md](docs/SEGURANCA.md).
- Registro da marca e revisão dos termos do serviço que gerou o logo e os ícones.

---

## English summary

**0.1.0 (2026-11-22), the first technical preview.** An installable ISO (x86_64, UEFI) that the release workflow
builds and tests in a VM (live session, installer, installed system boots), the shell on a patched niri, `kalangod`
(approval screen, undo, timeline, four power levels), local agents (LFM3 through llama.cpp), flow editor, context
management, Kalango Remote and Kalango Audio on the image, a model source chosen in the installer (public catalog,
private server or folder), and a fix for window memory growth. Known gaps: not validated on real hardware, no local
model in the ISO (licences under review), no accessibility certification, Portuguese-only UI, and some Kalango Remote
features pending. PR numbers refer to [kalangoos/kalangoos](https://github.com/kalangoos/kalangoos).

[0.1.0]: https://github.com/kalangoos/kalangoos/releases
