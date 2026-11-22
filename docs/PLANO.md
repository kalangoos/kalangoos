# KalangoOS · plano do que ainda falta

> **O plano detalhado, com checkbox de cada tarefa, está no OpenSpec:** `openspec/changes/fase-*/tasks.md`
> (e na página gerada `docs/especificacao/`). Este arquivo é o resumo.

Atualizado em 18 de novembro de 2026; o estado da versão 0.1.0 (22 de novembro de 2026) está em [LANCAMENTO.md](LANCAMENTO.md). Este documento lista o que falta para o KalangoOS deixar de ser um design e
virar um sistema instalável, na ordem em que faz sentido atacar.

Tamanho de cada item: **P** (dias), **M** (semanas), **G** (meses). São ordens de grandeza para uma pessoa
trabalhando com apoio de IA, não prazos.

---

## 1. Onde estamos

| Frente | O que existe | Onde |
|---|---|---|
| Design | 24 páginas, cerca de 200 telas, componentes com estados, variáveis de cor (escuro e claro), estilos de texto, molas | `design/KalangoOS-DesignSystem.figcraft` |
| Tokens em código | Cores (dois modos), movimento e medidas exportados | `design/tokens/` |
| Marca | Logo em PNG e SVG, 19 ícones de app em PNG e SVG | `assets/brand/`, `assets/icons/kalango/` |
| Prova de conceito | Barra superior e barra lateral do sistema em Rust + GTK4 sobre Wayland, com mola e troca de tema | `poc/` |
| Legado | `lunar-shell` (Rust, GTK3, X11): relógio, pasta de apps, central de controle, dock; tema GTK/GNOME "Lunar" | `shell/`, `theme/` |

O que a prova de conceito já respondeu: dá para desenhar superfícies do sistema fora do GNOME, com o estilo do
Design System e animação própria a 60 quadros por segundo, tudo em Rust. Detalhes em `poc/README.md`.

Desde então já existem: o niri com os patches do KalangoOS, o shell inteiro (barra superior, dock, barras laterais,
área de trabalho, Arquivos, Configurações, Central de agentes), o `kalangod` e o `kalango-agente` com o LFM3 local, a
tela de entrada (greetd) e o bloqueio, tudo testado na sessão (`sessao/testar.sh`) e numa VM com o kernel 7.3.4.

**Atualização (0.1.0, 22/11/2026):** a imagem instalável existe. A ISO (`imagem/construir.sh`) é montada e testada
numa VM pela esteira de lançamento: sessão ao vivo com o instalador, instalação num disco virtual vazio e o primeiro
boot do sistema instalado, com a tela de entrada. O que **ainda não existe**: validação em hardware real (a matriz
NVIDIA, AMD e Intel), atualizações com restauração e a ISO com o modelo local (licenças em revisão).

---

## 2. Decisões

O registro completo, com os motivos, está em `ARQUITETURA.md`.

| # | Decisão | Estado |
|---|---|---|
| D1 | Base do compositor | **Em teste:** partir de um compositor Wayland em Rust existente (niri ou cosmic-comp) |
| D2 | Toolkit do shell e dos apps | **Decidido:** GTK4 em Rust com CSS |
| D3 | Base da distribuição | **Decidido:** Ubuntu LTS com kernel HWE |
| D4 | Atualização e restauração | Aberto. Recomendação provisória: btrfs com snapshots |
| D5 | Apps de terceiros | **Decidido:** Flatpak |
| D6 | Motor de modelos locais | Decidido: llama.cpp com LFM3 (Liquid AI) em GGUF; LFM3-1.7B como padrão |
| D7 | Nome e marca | **Parcial:** domínio `kalangoos.com` comprado; falta verificar o registro de marca |
| D8 | Escopo da versão 0.1 | Proposto: fases 0 a 2 |
| D9 | Controle total do sistema pelos agentes | **Decidido:** serviço privilegiado `kalangod` que fala MCP, com regras impostas pelo kernel (eBPF, Landlock), sem módulo de kernel próprio |

---

## 3. Roteiro por fases

### Fase 0 · Fundações (P–M)

- [ ] Tomar D1, D2 e D8; registrar em `docs/ARQUITETURA.md`.
- [ ] Renomear o que ainda se chama SDRustOS ou Lunar: pasta do repositório, tema, `lunar-shell`, papéis de parede.
- [ ] Organizar o repositório como *workspace* Cargo: `shell/`, `compositor/`, `agentes/`, `apps/`, `tema/`.
- [ ] Exportador de tokens em Rust (hoje a exportação foi feita por script avulso): FigCraft → CSS do GTK + constantes Rust.
- [ ] Substituir os scripts Python (`scripts/*.py`, `install.sh`) por um binário `kalango` em Rust.
- [ ] Integração contínua: compilar, testes, `clippy`, `fmt`.
- [ ] Ambiente de desenvolvimento reproduzível, que rode a sessão dentro de uma janela ou máquina virtual.

**Pronto quando:** um comando compila tudo e abre a sessão de desenvolvimento.

### Fase 1 · Shell mínimo sobre Wayland (M–G)

Superfícies do sistema, sobre o compositor escolhido em D1.

- [ ] Sessão que inicia sozinha (arquivo de sessão, variáveis, serviços).
- [ ] Barra superior com a ilha (relógio, agente, música, aprovação, foco, voz).
- [ ] Dock única, nos dois estilos do design: flutuante e fixa (barra de tarefas).
- [ ] Barra lateral esquerda "Hoje" com widgets; barra lateral direita com Avisos, Controles e Agente.
- [ ] Notificações: serviço `org.freedesktop.Notifications`, banners, agrupamento, central.
- [ ] Indicadores na tela (volume, brilho, teclas) e menus de contexto.
- [ ] Papel de parede e widgets da área de trabalho.
- [ ] Bloqueio de tela (`ext-session-lock`) e tela de entrada.
- [ ] Barra de comando e busca universal (apps, arquivos, configurações).
- [ ] Ligações reais: rede (NetworkManager), Bluetooth (BlueZ), som (PipeWire), brilho, energia (UPower), mídia (MPRIS).
- [ ] Atalhos de teclado globais e a folha de atalhos.

**Pronto quando:** dá para usar o computador um dia inteiro só com este shell, abrindo apps existentes.

### Fase 2 · Janelas e espaços (G)

Trabalho dentro do compositor.

- [ ] Moldura e controles de janela no estilo do design; estados ativa, inativa, com folha modal.
- [ ] Animações de janela com mola (abrir, fechar, minimizar, maximizar).
- [ ] Encaixe: metades, grade, seletor de layouts, sugestão do agente.
- [ ] Alternar janelas (Alt+Tab) e visão geral com miniaturas ao vivo.
- [ ] Espaços por intenção (cada espaço lembra janelas, agentes e regras de foco).
- [ ] Vários monitores, escala fracionária, HiDPI, rotação.
- [ ] Gestos de touchpad que seguem os dedos; modo toque.
- [ ] Captura e gravação de tela, compartilhamento de tela (portais `xdg-desktop-portal`).
- [ ] Apps X11 antigos (XWayland).
- [ ] Driver NVIDIA proprietário: testar cedo; é o ponto fraco conhecido do Wayland.

**Pronto quando:** as telas das páginas 12 e 13 do design funcionam de verdade.

Decidido (18 de novembro de 2026): janelas lado a lado se encostam (niri `gaps 0`), sem faixa de papel de parede
entre elas nem nas bordas, e em coluna perdem os cantos arredondados (as flutuantes ficam com os 8 px). Minimizar
existe de verdade (patch 0006 do niri: a janela some sem fechar, a dock mostra o ponto apagado e a traz de volta;
clicar na dock no app em foco o minimiza; Mod+M). As telas se ajustam ao espaço: grades com quantas colunas couberem,
corpo que rola, painéis laterais que cedem em janela estreita, larguras máximas que encolhem (limite.rs).

### Fase 3 · Tema, ícones e apps base (G)

- [ ] Tema GTK4/libadwaita e GTK3 a partir dos tokens; tema Qt equivalente.
- [ ] Tema de ícones completo: símbolos (cerca de 80 desenhados), tipos de arquivo, dispositivos, ações, estados. Hoje só existem 19 ícones de app.
- [ ] Cursor, sons do sistema e fontes empacotadas (conferir licenças).
- [ ] Configurações: 20 painéis desenhados, cada um com funções reais por trás.
- [ ] Arquivos, Terminal, Editor de Texto, Imagens, Música, Vídeo, Documentos, Calculadora, Calendário, Monitor, Discos, Ajuda.
- [ ] Diálogos do sistema via portais: abrir e salvar arquivo, imprimir, permissões, autenticação (polkit).

Decidido (pela pessoa, em 18 de novembro de 2026): nenhum app nem tela de outro sistema (GNOME, Yaru, Adwaita). Os
apps base são do KalangoOS, desenhados no FigCraft antes do código; os do GNOME que um deles substitui somem da grade,
da busca e da dock. Feitos até aqui: Arquivos, Configurações, Calculadora, Editor de Texto e Terminal. O Terminal usa a
VTE para GTK 4 por baixo (as células, o PTY, a seleção, a rolagem — o mesmo papel do GTK para as janelas); a janela,
as abas, os ganchos no shell e o agente são nossos, e o pacote `libvte-2.91-gtk4-0` entra nas dependências do sistema
(na máquina de desenvolvimento, `poc/setup-deps.sh` o traz para `poc/.deps`, sem sudo). Os seletores de arquivo
prontos do GTK também saíram: as janelas do shell chamam o seletor do sistema direto, e os outros apps o recebem pelo
portal.

**Pronto quando:** um usuário comum faz as tarefas do dia a dia sem ver uma tela "de outro sistema".

### Fase 4 · Agentes (G)

É o que diferencia o KalangoOS, e o que tem menos especificação técnica hoje.

- [ ] Serviço de agentes em Rust: ciclo de vida, fila de tarefas, pausa e retomada.
- [ ] `kalangod`, o "MCP do sistema": serviço root com ferramentas para arquivos, pacotes, serviços, rede, armazenamento, usuários, processos e kernel, em quatro níveis de poder (observar, usuário, sistema, total). Desenho em `ARQUITETURA.md`, seção 3.
- [ ] Programas eBPF para as regras invioláveis e para a trilha de auditoria; ativar `bpf` na lista de LSM da imagem.
- [ ] Modelo de permissões em três níveis (livre, perguntar, nunca), por agente e por ferramenta.
- [ ] Registro de ações com desfazer: o que mudou, por quê, com quais permissões. Define como cada tipo de ação é revertida (arquivos, pacotes, configurações).
- [ ] Isolamento: cada agente e cada conector MCP em caixa de areia (namespaces, Landlock, bubblewrap).
- [ ] Hospedeiro de MCP: conectores locais e remotos, OAuth, cofre de segredos, teste antes de adicionar.
- [ ] Modelos locais (D6) e uso opcional de nuvem com orçamento e aviso.
- [ ] Memória local, criptografada, legível e apagável.
- [ ] Rotinas (horário e gatilhos) e a tela de uso e custos.
- [ ] Defesa contra instruções escondidas em páginas e arquivos (conteúdo de fora nunca vira comando).
- [ ] App Agentes e o chat completo: blocos de texto, código, diff, terminal, Mermaid, LaTeX, tabela, gráfico, áudio, vídeo, arquivos, prévia de HTML e navegador do agente (motor web embutido).
- [ ] Modo voz (transcrição local), "descrever a tela", agente usando a tela.
- [ ] Privacidade visível: indicadores de microfone, câmera, tela e leitura por agente; "pausar tudo".

Decidido (18 de novembro de 2026), a web dos agentes: `web.buscar` pesquisa (o Bing responde sem scripts; o Google
pelo navegador; o DuckDuckGo sem scripts por último) e `web.ler` lê a página; um endereço de buscador vira pesquisa.
Páginas que só mostram o conteúdo com scripts abrem no [Moli](https://github.com/lexmount/moli) (motor de navegador em
Rust para agentes, Apache-2.0/MIT, versão fixa), preso pelo kalangod: toda requisição dele passa por um proxy que
recusa endereços locais e privados (o mesmo critério do `web.ler`, conferido no endereço conectado), Landlock (lê o
sistema, não a pasta pessoal; escreve só numa pasta própria; conecta só na porta do proxy), cgroup e nenhum ambiente
do kalangod. Na máquina de desenvolvimento, `poc/setup-deps.sh` o traz para `poc/.deps/moli`; no sistema instalado,
`/usr/lib/kalango/moli`. Sem ele, as páginas com scripts ficam só com o texto que vier sem eles. O controle de
contaminação das chamadas web deixa seguir links que páginas e resultados mostraram e recusa só dados lidos neste
computador (números, caminhos, identificadores) que a pessoa não escreveu, saindo no endereço ou na pesquisa.

**Pronto quando:** um agente organiza uma pasta de verdade, pede aprovação para apagar, e tudo pode ser desfeito
pela Linha do tempo.

### Fase 5 · Sistema (G)

- [x] Instalador próprio (`crates/kalango-instalador`, os 4 passos do design): discos pelo lsblk (o da sessão ao vivo
  nunca aparece; disco de dados ≠ sistema), ao lado de outro sistema (a partição dele encolhe **no mesmo setor de
  início**) ou o disco inteiro, LUKS2 com /boot separado, conta com força da senha, entrar sem senha (greetd). O motor
  (`--instalar PLANO`, por pkexec) recebe a senha pela entrada padrão e nunca a grava; sem root, só descreve os
  comandos. Falta: rodar de verdade num disco virtual (passo 2 abaixo).
- [x] Primeiro uso (43017, 43114, 43227): traz as pastas e o navegador do outro sistema do disco (kalangod
  `importar.*`, montado só para leitura, nada sobrescrito, com aprovação e progresso), tema/cor/dock/widgets na hora,
  onde os agentes pensam e quanto decidem sozinhos (as colunas de permissão).
- [ ] Animação de inicialização (Plymouth) e tela de entrada própria.
- [ ] Atualizações com ponto de restauração e a tela de recuperação.
- [ ] Central de Apps: catálogo de apps (Flatpak), agentes e modelos, com permissões antes de instalar.
- [ ] Usuários, convidado, criptografia de disco (LUKS com TPM), impressão digital.
- [x] Geração da imagem ISO e teste automático de instalação em máquina virtual (`imagem/`, esteira de lançamento; 0.1.0).

**Pronto quando:** a ISO instala em um computador vazio e em dual boot, e atualiza sem perder dados.

**O que a imagem precisa levar** para o instalador e o primeiro uso: `gdisk` (sgdisk), `cryptsetup`, `dosfstools`,
`e2fsprogs`, `ntfs-3g` (encolher e ler NTFS), `rsync`, `squashfs-tools`, `grub-efi-amd64`, `ubuntu-drivers-common`,
`policykit-1` (pkexec), `sqlite3` (contar os favoritos do Firefox), `orca` (leitor de tela), `wvkbd` (teclado virtual
no Wayland), `fprintd` e, se quiser o "Particionar manualmente", `gparted`. Os botões dessas três últimas só aparecem
quando o programa existe.

Ainda na imagem: uma regra do polkit para a conta da sessão ao vivo (`/etc/polkit-1/rules.d/49-kalango-ao-vivo.rules`,
como a do Ubuntu ao vivo), para o `pkexec` do instalador não precisar de agente de autenticação; no sistema instalado o
agente do polkit (design 52727) ainda falta.

Para o seletor de arquivos de todos os apps (designs 57410 e 52563), a imagem leva `xdg-desktop-portal` e
`xdg-desktop-portal-gtk`, o `kalango.portal` em `/usr/share/xdg-desktop-portal/portals/` e o
`kalangoos-portals.conf` em `/usr/share/xdg-desktop-portal/` (os dois estão em `sessao/dist`); a sessão exporta
`XDG_CURRENT_DESKTOP=KalangoOS`, `GDK_DEBUG=portals` e `GTK_USE_PORTAL=1` (o bloco `environment` do niri). O shell é
também o agente de senhas do NetworkManager e o agente de pareamento do BlueZ, e monta os pendrives (o `udisks2`
e o `gvfs` precisam estar lá). As telas de ligar (Inicialização 56783, Desbloquear disco 56796,
Atualizando 57287) viram um tema do Plymouth, testado na VM do passo 1 (sem root, o Plymouth não roda na sessão
aninhada); "Desligando" (57301) é do shell, no fluxo do menu de energia.

O mesmo backend de portal do shell responde ao `Access` (o pedido de permissão de câmera, microfone e localização,
design 52877) e ao `ScreenCast` (o seletor do sistema para compartilhar a tela, 53005): o `kalangoos-portals.conf`
os manda para o `kalango`; o que é compartilhado é gravado pelo `org.gnome.Mutter.ScreenCast` do próprio niri (que
precisa do recurso `xdp-gnome-screencast` e de um dispositivo GBM: só no TTY, não na sessão aninhada). A ferramenta de
captura (Print, 40189) grava vídeo pelo mesmo caminho com o GStreamer: a imagem leva `gstreamer1.0-pipewire`,
`gstreamer1.0-plugins-good` (vp8, webm, videocrop, pulse) e `gstreamer1.0-plugins-base` (opus, videoconvert); o
"Extrair texto" usa o modelo de visão local (LFM3-VL). O niri do KalangoOS leva os patches de `sessao/niri/`
(0004: a posição das janelas lado a lado na IPC, que a captura usa para escolher a janela pelo ponteiro). A câmera
bloqueada em "Privacidade agora" (41799) é uma regra do WirePlumber 0.4 em `~/.config/wireplumber/main.lua.d/`.

O backend também responde ao `Settings` (`org.freedesktop.appearance`): escuro ou claro como o shell está agora e
a cor de destaque, com `SettingChanged` a cada troca. É por ele que os apps GTK 4 e libadwaita seguem o tema do
KalangoOS ao vivo (o `color-scheme` do GNOME continua sendo gravado fora da sessão de desenvolvimento, para os apps
GTK 3); o `kalangoos-portals.conf` o põe antes do `gtk` (`Settings=kalango;gtk`). A cor de destaque precisa do
libadwaita 1.6 ou mais novo. O patch 0005 do niri torna opcional o encaixe ao arrastar para as bordas (`SetEdgeSnap`).

### Fase 6 · Qualidade e lançamento (contínuo)

- [ ] Acessibilidade: leitor de tela (AT-SPI), alto contraste, texto grande, navegação só por teclado, "reduzir animações".
- [ ] Idiomas: hoje todo o texto está em português do Brasil; preparar tradução e formatos regionais.
- [ ] Metas de desempenho medidas: tempo de inicialização, memória em repouso, quadros por segundo.
- [ ] Segurança: modelo de ameaças, revisão do isolamento dos agentes, atualizações assinadas.
- [ ] Testes de interface automatizados e de regressão visual contra o design.
- [ ] Documentação, site, licenças do código e dos recursos gerados por IA.

---

## 4. O que ainda falta no design

| Lacuna | Situação | Tamanho |
|---|---|---|
| Outras resoluções | Só a área de trabalho e a dock fixa têm versões além de 1920×1080 | M |
| Tema claro | Cerca de 40 telas de 200 têm cópia clara; trechos coloridos de código foram recoloridos à mão | M |
| Alto contraste | Citado em Acessibilidade, não desenhado | P |
| Animações | Descritas em anotações; só alguns fluxos viraram protótipo, e não foram assistidos rodando | M |
| Estados | Vazio, carregando e erro existem como padrões (página 09), não tela por tela | M |
| Ícones | 19 apps prontos; faltam tipos de arquivo, dispositivos e o tema de símbolos como arquivos | M |
| Ícones de terceiros | Chrome, Discord e outros seguem no estilo antigo | P |
| Cursor, sons, mais papéis de parede | Não existem | P |
| Abas não desenhadas | Rede (Ethernet, VPN, Proxy, Firewall); Aparência (Cores, Papel de parede, Fontes); Agentes › Ajustes (Geral, Notificações, Avançado); Tela (Luz noturna, Cor); Loja (página de app, atualizações) | M |
| SVG | Os logos e ícones em SVG foram traçados automaticamente; têm mais pontos que um desenho limpo | P |
| Originalidade | Bloqueio, Arquivos e dock fixa seguem estruturas convencionais | — |

---

## 5. Riscos

| Risco | Efeito | Como reduzir |
|---|---|---|
| Escopo | 200 telas e um compositor são trabalho de equipe | Versão 0.1 pequena (fases 0 a 2); reaproveitar apps existentes |
| NVIDIA no Wayland | Falhas gráficas ou de desempenho; nesta máquina o compositor aninhado nem abre em X11 | Testar na fase 1 com o driver real; ter outra placa para comparação |
| Depender de um compositor de terceiros | Mudanças deles quebram o nosso | Contribuir de volta; manter as nossas alterações pequenas e isoladas |
| Agentes com poder demais | Perda de dados ou vazamento | Isolamento e desfazer antes de qualquer recurso novo; nada destrutivo sem aprovação |
| Nível total (root) nas mãos de um agente | Um texto malicioso lido pelo agente vira comando de administrador | Aprovação presencial a cada sessão, prazo curto, ponto de restauração antes, regras do kernel que nem o root dos agentes quebra |
| Modelos locais lentos em máquinas fracas | A promessa "local primeiro" não se sustenta | Definir requisito mínimo; modo híbrido claro |
| Marca | Nome já registrado por outro | Verificar antes de publicar (D7) |
| Recursos gerados por IA | Dúvida de licença dos logos e ícones | Registrar a origem; revisar os termos do serviço usado |

---

## 6. Próximos passos, em ordem (combinado em 17 de novembro de 2026)

> **Estado em 22/11/2026:** os passos 0 a 4 abaixo foram cumpridos na ISO da 0.1.0 (testada numa VM); o 5 (AG-UI) segue
> planejado. O texto original dos passos fica como registro.

Passos pequenos, um de cada vez. Um passo só termina quando **liga do zero numa VM**, tem **teste automático** e as
**capturas batem com o FigCraft**. Os dois temas seguem o mesmo padrão; o claro usa as cores e as fontes da referência
(`01 · Fundamentos` › "Cores e fontes · tema claro").

0. **Fechar o lote atual:** rodar a suíte da sessão inteira e fazer o commit local.
1. **Base de teste rápida e fiel:** uma imagem de disco de verdade (qcow2, ext4) feita por script com o kernel
   7.3.4-kalango, o niri e o shell (a mesma que o instalador vai gravar), no lugar da VM que lê o sistema do host por
   9p. As telas continuam sendo feitas na sessão aninhada (`sessao/rodar.sh`); a VM fica para ligar → instalar → entrar.
2. **Instalador** (telas e motor feitos, testados na sessão com o motor simulado — 18/11; falta gravar de verdade num
   disco virtual e a Inicialização): Inicialização 56783 (marca e cinco pontos), Boas-vindas 56835
   (idioma, teclado, rede), Disco 56930 (disco inteiro, criptografia opcional), Conta 57032, Instalando 57164. As telas
   rodam sem root; um ajudante separado particiona, criptografa, copia o sistema, instala o carregador de boot e cria a
   conta, com um modo de simulação para os testes. Teste: instalar num disco virtual vazio e ligar a VM por ele.
3. **Login:** a tela de entrada (2705, greetd + `kalango-entrada`) ligada no sistema instalado (a VM de hoje entra
   direto), e o Primeiro uso (43017, 43114, 43227) depois do primeiro login.
4. **Área de trabalho mínima:** barra superior, dock, **Arquivos** (o redesign já está no FigCraft: lista, ícones,
   colunas, galeria, compacto e claro) e Configurações, com um kit único de componentes GTK. O resto (loja, widgets
   extras) fica escondido na imagem até a base estar sólida.
5. **AG-UI** (https://github.com/ag-ui-protocol/ag-ui, versão 1.0) como o protocolo de eventos entre o agente e a
   interface, com os tipos gerados do JSON Schema oficial, pelo mesmo canal local de hoje:
   - a resposta chegando enquanto é escrita (`TEXT_MESSAGE_START/CONTENT/END`; hoje ela aparece inteira no fim);
   - as aprovações do `kalangod` como o *interrupt* do protocolo (aprovar, editar, recusar e continuar);
   - progresso e estado (`ACTIVITY_*`, `STATE_SNAPSHOT/DELTA`) para a ilha, a faixa e as barras;
   - raciocínio (`REASONING_*`), subagentes e as respostas que viram interface (design 46004);
   - por último, uma porta para agentes de fora que já falam AG-UI aparecerem no chat do KalangoOS.

Pendências conhecidas que acompanham esses passos: a barra "Hoje" na lateral inteira (como a direita), a página
Teclado e atalhos (48349, com `shortcuts.rs`), Mouse e touchpad (48709) e as outras páginas de Configurações.
