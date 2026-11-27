# KalangoOS 0.1.0 — prévia técnica

**Português** · [English](#english)

Esta é a primeira prévia técnica do KalangoOS: uma distribuição Linux (base Ubuntu LTS) em que agentes de IA operam
o sistema com aprovação e desfazer. Ela é para desenvolvedores e testadores. **Foi testada só em máquina virtual**:
não instale num computador com dados que você não pode perder.

## O que há de novo

- **ISO instalável** (x86_64, UEFI). A esteira de lançamento monta a ISO e a testa numa VM a cada execução: a sessão
  ao vivo abre com o instalador, o motor do instalador grava num disco virtual vazio (a etapa gráfica "Modelos de
  IA" não é exercitada pelo teste) e o sistema instalado liga na sessão.
- **Instalador** com a etapa "Modelos de IA": escolha de onde vêm os modelos (catálogo público, servidor privado ou
  uma pasta). Os downloads retomam de onde pararam e têm a soma SHA-256 conferida.
- **Shell próprio** em Rust e GTK 4 sobre o niri com patches: barra, dock, barras laterais, área de trabalho,
  bloqueio, tela de entrada, janelas e espaços, e os apps do sistema (Arquivos, Configurações, Terminal, Editor,
  Calculadora, Central de agentes e outros).
- **`kalangod`**: o serviço que dá aos agentes acesso ao sistema, com aprovação numa tela que o agente não imita,
  desfazer, Linha do tempo e quatro níveis de poder (observar, usuário, sistema, total). Regras impostas pelo kernel
  (eBPF LSM, Landlock, seccomp e cgroups), lista branca de pastas para escrita, configuração do sistema fora do
  alcance dos agentes e defesa contra dado local saindo no endereço da web (nos casos cobertos, com limites documentados).
- **Agentes locais** com o LFM3 pelo llama.cpp, gestão de contexto da conversa (janela recente, resumo em segundo
  plano e busca na conversa) e agentes que operam a tela pela visão, sempre com aprovação.
- **Aprovação com o d1**: a tela de aprovação mostra a recomendação do modelo de decisão d1 ("parece seguro" ou
  "parece perigoso", a confiança e um motivo de uma lista fechada); quem decide continua sendo a pessoa. Em
  Permissões há a opção, **desligada por padrão**, "Aprovar sozinho o que é de baixo risco": vale só para três
  ações reversíveis na pasta da pessoa (mover para a lixeira, mover, criar um arquivo novo que não roda), só se o d1
  disser que é seguro com pelo menos a confiança mínima, nunca depois de conteúdo lido de fora na conversa, no máximo 5
  por pedido, e se desliga sozinha depois de 3 recusas. O `kalangod` confere tudo de novo, e instalar pacotes nunca
  é "seguro" (kalangoos/kalangoos#42).
- **Fluxos**: editor de fluxo completo no shell; o modelo d1 também responde nos blocos "Decidir".
- **Kalango Remote** (compartilhar a tela, controlar outro computador, SSH e Mosh; o Mosh ainda sem teste) e **Kalango Audio** (capturar,
  tratar, rotear e gravar som) na imagem. O Remote tem senha fixa e acesso sem supervisão com permissões limitadas
  (sem arquivos, terminal, câmera, túnel, reinício, bloqueio de entrada nem modo de privacidade; o aviso lista o que
  foi concedido), verificação em duas etapas (TOTP), gravação de sessões (Vídeos › Kalango Remote, 30 dias, com aviso
  à pessoa gravada), servidor próprio compatível com o RustDesk, bloqueio global depois de tentativas erradas e o
  painel do plano do agente (`kalango-remote agent-plan`) (kalangoos/kalango-remote#11 e #12).
- **Modo de privacidade do Remote ("tela preta aqui")**: enquanto alguém controla com um acesso que ninguém aprovou
  neste computador, a tela local fica coberta e quem controla vê a área de trabalho de verdade. Saídas de emergência:
  Esc Esc, troca de terminal virtual, botão de ligar e bloqueio (patch 0012 do niri, kalangoos/niri#4, e
  kalangoos/kalangoos#44).
- **Agente e Remote**: a ferramenta `remoto.conectar` deixa o agente propor uma conexão a um dos computadores da
  pessoa pelo painel do plano do Remote; só o clique de uma pessoa aprova (kalangoos/kalangoos#44).
- **Kalango VirtualDesk**: o `kalangod` tem as ferramentas `vm.*` e o shell tem a tela em Ajustes › Sandbox (as ferramentas `vm.*`
  fazem uma pergunta só, #40); o app em si não vai na ISO.
- **Fonte dos modelos de IA** também em Ajustes e na Central de Apps; o d1-Omni-2B entrou no catálogo da Central de
  Apps (#40).
- **Segurança**: dado local colado, repartido ou codificado no endereço da web é barrado nos casos cobertos (#36); os
  limites estão em [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md) (rot13, inverter, base32/58, XOR, ruído intercalado, segredo repartido
  entre parâmetros ou rótulos de DNS, base64 em minúsculas num rótulo de DNS, corpo POST e fragmento não são cobertos). Também: `url` 2.5.9 contra o
  RUSTSEC-2026-0592 (#37) e o ditado da Ajuda que para ao fechar a janela (#38).
- **Memória do shell**: janelas de apps fechadas liberam a superfície gráfica depois de 60 s.

A lista completa, com os números dos pull requests, está no [CHANGELOG.md](https://github.com/kalangoos/kalangoos/blob/main/CHANGELOG.md).

## Como instalar e verificar

Baixe os arquivos da [página de versões](https://github.com/kalangoos/kalangoos/releases): a ISO
`kalangoos-0.1.0-amd64.iso`, a soma `SHA256SUMS-iso.txt` e, se quiser, os binários
`kalangoos-0.1.0-linux-x86_64.tar.gz` com `SHA256SUMS.txt`.

1. **Confira a soma** na pasta dos downloads:

   ```sh
   sha256sum -c SHA256SUMS-iso.txt
   ```

2. **Confira a origem do build** (atestado de procedência do GitHub Actions; precisa da [CLI do GitHub](https://cli.github.com/)):

   ```sh
   gh attestation verify kalangoos-0.1.0-amd64.iso --repo kalangoos/kalangoos
   ```

3. **Teste numa VM.** No repositório (`git clone https://github.com/kalangoos/kalangoos`), com `qemu-system-x86` e
   `ovmf` instalados e sem root:

   ```sh
   KALANGO_ISO=/caminho/kalangoos-0.1.0-amd64.iso imagem/rodar-iso.sh novo
   ```

   O script cria um disco virtual de 40 GB vazio (`DISCO=`, `MEM=`, `CPUS=` e `RES=` ajustam), liga a ISO com
   firmware UEFI e abre a sessão ao vivo com o instalador. A conta ao vivo `kalango` não tem senha. Sem `/dev/kvm` a
   VM roda emulada e fica lenta. É o que o script faz por baixo (8 GB de RAM, 6 CPUs, disco em `target/iso`):

   ```sh
   qemu-system-x86_64 -enable-kvm -cpu host -machine q35 -m 8192 -smp 6 \
     -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
     -drive if=pflash,format=raw,file=uefi-vars.fd \
     -drive file=kalangoos-0.1.0-amd64.iso,media=cdrom,readonly=on,if=none,id=cd -device ide-cd,drive=cd,bootindex=1 \
     -drive file=disco.qcow2,format=qcow2,if=none,id=disco -device virtio-blk-pci,drive=disco,bootindex=0 \
     -device virtio-vga,xres=1920,yres=1080 -display sdl \
     -device virtio-keyboard-pci -device virtio-tablet-pci -nic user,model=virtio-net-pci
   ```

   (Sem a aceleração 3D de propósito: com `-device virtio-vga-gl … -display sdl,gl=on` o virgl do QEMU derrubou a VM inteira com
   o desenho do shell, no meio da instalação, deixando o disco pela metade. `imagem/rodar-iso.sh` já liga assim; `VM_GL=1` liga o 3D
   de novo, por sua conta e risco.)

   (antes, `qemu-img create -f qcow2 disco.qcow2 40G` e `cp /usr/share/OVMF/OVMF_VARS_4M.fd uefi-vars.fd`). Depois da
   instalação, `imagem/rodar-iso.sh` sem argumento liga o disco instalado primeiro.

4. **Modelos de IA.** A ISO não leva o modelo local. Na etapa "Modelos de IA" do instalador (ou depois, em Ajustes e
   na Central de Apps), escolha o catálogo público, um servidor privado ou uma pasta com os modelos.

## Ainda não incluído na 0.1.0

- A ISO não leva modelo local (as licenças estão em revisão): ele é baixado depois.
- O Kalango VirtualDesk e o Kalango Fluxos não vão na ISO.
- Validação em hardware real e a matriz NVIDIA, AMD e Intel completa.
- O Remote entre dois computadores físicos e numa sessão real do niri (sem ser aninhada).
- Ideias do [roteiro](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#integrações-planejadas-ideias-sem-data): fonte de áudio ao vivo para fluxos, o gatilho de entrada pela internet por túnel da
  Cloudflare ([ver](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#gatilho-de-entrada-pela-internet)) e o Remote dentro dos fluxos.
- Licença CC BY-SA 4.0 da documentação, ainda a confirmar ([LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md)).

## Limitações conhecidas

| Limitação | Situação |
|---|---|
| Hardware real | Testada só em VM (UEFI). A matriz NVIDIA, AMD e Intel não está completa |
| Modelo local | As licenças dos modelos estão em revisão: a ISO não leva modelo, ele é baixado depois |
| Acessibilidade | Sem certificação nem auditoria |
| Idioma | Interface e documentação em português do Brasil; o inglês vem na fase 6 |
| Kalango Remote | Não foi testado entre dois computadores físicos nem numa sessão real do niri (só aninhada); o Mosh não foi testado (não está instalado onde testamos); o caminho da tela de bloqueio e o quadro de transição da tela sob a cobertura do modo de privacidade só são cobertos pelo código, não por um teste |
| VirtualDesk e Fluxos | Fora da ISO; o serviço de fluxos está num repositório privado |
| Atualizações | Sem atualização com ponto de restauração nem tela de recuperação |
| Licença da documentação | A CC BY-SA 4.0 proposta para a documentação ainda está por confirmar ([LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md)) |
| Ideias do roteiro (fonte de áudio ao vivo para fluxos, gatilho de entrada pela internet por túnel da Cloudflare, Remote dentro dos fluxos) | Ainda não feitas ([ROADMAP.md](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#integrações-planejadas-ideias-sem-data)) |
| Segurança | Sem revisão externa; os limites da defesa de endereços estão em [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md) |

Mais contexto em [LANCAMENTO.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LANCAMENTO.md) e no [ROADMAP.md](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md).

## Segurança

Não abra issue pública sobre uma falha de segurança. Use **Security › Report a vulnerability** neste repositório; o
prazo de resposta e o que conta como falha estão em [SECURITY.md](https://github.com/kalangoos/kalangoos/blob/main/SECURITY.md).

## Licença

Código sob a GNU General Public License, versão 3 ou posterior ([LICENSE](https://github.com/kalangoos/kalangoos/blob/main/LICENSE)). O nome KalangoOS, o mascote e
os logos são a marca do projeto e ficam fora dessa licença ([LICENSE-brand.txt](https://github.com/kalangoos/kalangoos/blob/main/LICENSE-brand.txt)). Recursos de
terceiros: [ATTRIBUTION.md](https://github.com/kalangoos/kalangoos/blob/main/ATTRIBUTION.md) e [NOTICE](https://github.com/kalangoos/kalangoos/blob/main/NOTICE).

O Kalango Remote é derivado do RustDesk e é AGPL-3.0; o Kalango Audio é GPL-3.0-or-later; o Ubuntu, o kernel e o
llama.cpp seguem as licenças próprias (cada pacote do Ubuntu a sua; o kernel é GPL-2.0; o llama.cpp é MIT). Veja
[LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md) e [ATTRIBUTION.md](https://github.com/kalangoos/kalangoos/blob/main/ATTRIBUTION.md).

---

## English

**KalangoOS 0.1.0, technical preview.** The first preview of a Linux distribution (Ubuntu LTS base) where AI agents
operate the system with approval and undo. It is for developers and testers. **It has only been tested in virtual
machines**: do not install it on a computer with data you cannot lose.

### What is new

- **Installable ISO** (x86_64, UEFI). The release workflow builds the ISO and tests it in a VM on every run: the live
  session boots with the installer, the installer engine writes to an empty virtual disk (the graphical "AI models"
  step is not exercised by the test) and the installed system boots into its session.
- **Installer** with an "AI models" step: choose where models come from (public catalog, private server or a folder).
  Downloads resume and are checked against a SHA-256 sum.
- **Its own shell** in Rust and GTK 4 on a patched niri: bar, dock, side bars, desktop, lock screen, login screen,
  windows and spaces, and the system apps (Files, Settings, Terminal, Editor, Calculator, Agents center and more).
- **`kalangod`**: the service that gives agents access to the system, with approval on a screen the agent cannot
  imitate, undo, a timeline and four power levels (observe, user, system, full). Kernel-enforced rules (eBPF LSM,
  Landlock, seccomp, cgroups), a folder allow-list for writes, system configuration out of agents' reach and a
  defence against local data leaving in a web address (in the covered cases, with documented limits).
- **Local agents** with LFM3 through llama.cpp, conversation context management (recent window, background summary,
  search in the conversation) and agents that operate the screen by vision, always with approval.
- **Approval with the d1**: the approval screen shows the d1 decision model's recommendation ("looks safe" or "looks
  dangerous", its confidence and a reason from a closed list); the decision stays with the person. Permissions has an
  option, **off by default**, "Approve low-risk actions on my own": it covers only three reversible file actions in
  the person's folder (move to trash, move, create a new file that does not run), only if the d1 says safe with at
  least the minimum confidence, never after content read from outside in the conversation, at most 5 per request, and it turns
  itself off after 3 denials. `kalangod` re-checks everything, and installing packages is never "safe"
  (kalangoos/kalangoos#42).
- **Flows**: a full flow editor in the shell; the d1 model also answers in "Decidir" blocks.
- **Kalango Remote** (screen sharing, remote control, SSH and Mosh; Mosh not yet tested) and **Kalango Audio** (capture, process, route
  and record sound) on the image. Remote has a fixed password and unattended access with limited permissions (no
  files, terminal, camera, tunnel, restart, input blocking or privacy mode; the banner lists what was granted),
  two-step verification (TOTP), session recording (Videos › Kalango Remote, 30 days, with a notice to the recorded
  person), its own RustDesk-compatible server, a global lockout after wrong attempts and the agent plan panel
  (`kalango-remote agent-plan`) (kalangoos/kalango-remote#11 and #12).
- **Remote privacy mode ("black screen here")**: while someone controls with access nobody approved on this
  computer, the local screen is covered and the controller sees the real desktop. Emergency exits: Esc Esc, a
  virtual-terminal switch, the power button and the lock (niri patch 0012, kalangoos/niri#4, and
  kalangoos/kalangoos#44).
- **Agent and Remote**: the `remoto.conectar` tool lets the agent propose a connection to one of the person's
  computers through the Remote's plan panel; only a human click approves (kalangoos/kalangoos#44).
- **Kalango VirtualDesk**: `kalangod` has the `vm.*` tools and the shell has the Settings › Sandbox screen (the `vm.*` tools
  ask a single question, #40); the app itself is not on the ISO.
- **AI model source** also in Settings and in the app center; d1-Omni-2B joined the app center catalog (#40).
- **Security**: local data pasted, split or encoded into a web address is blocked in the covered cases (#36); the
  limits are in [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md) (rot13, reversal, base32/58, XOR, interleaved noise, a secret split across
  parameters or DNS labels, lowercase base64 in a DNS label, a POST body and the fragment are not covered). Also: `url` 2.5.9 against RUSTSEC-2026-0592
  (#37) and the Help dictation that stops when its window closes (#38).
- **Shell memory**: closed app windows release their graphics surface after 60 s.

The full list, with pull request numbers, is in [CHANGELOG.md](https://github.com/kalangoos/kalangoos/blob/main/CHANGELOG.md).

### Install and verify

Download from the [releases page](https://github.com/kalangoos/kalangoos/releases): the ISO
`kalangoos-0.1.0-amd64.iso`, the checksum file `SHA256SUMS-iso.txt` and, optionally, the binaries tarball
`kalangoos-0.1.0-linux-x86_64.tar.gz` with `SHA256SUMS.txt`.

```sh
sha256sum -c SHA256SUMS-iso.txt                                              # checksum
gh attestation verify kalangoos-0.1.0-amd64.iso --repo kalangoos/kalangoos   # build provenance (needs the GitHub CLI)
git clone https://github.com/kalangoos/kalangoos && cd kalangoos             # then, with qemu-system-x86 and ovmf:
KALANGO_ISO=/path/kalangoos-0.1.0-amd64.iso imagem/rodar-iso.sh novo         # empty 40 GB disk, UEFI, live session
```

The live account `kalango` has no password. Without `/dev/kvm` the VM runs emulated and slow. The plain `qemu` command
the script runs is shown in the Portuguese section above. The ISO ships no local model: pick a source in the
installer's "AI models" step.

### Not included in 0.1.0

- The ISO ships no local model (licences under review): it is downloaded afterwards.
- Kalango VirtualDesk and Kalango Flows are not on the ISO.
- Real-hardware validation and the complete NVIDIA, AMD and Intel matrix.
- Remote between two physical computers and in a real (non-nested) niri session.
- Ideas in the [roadmap](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#integrações-planejadas-ideias-sem-data): a live audio source for flows, an inbound internet trigger through a Cloudflare
  tunnel ([see](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#gatilho-de-entrada-pela-internet)) and Remote inside flows.
- The CC BY-SA 4.0 documentation licence is still to be confirmed ([LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md)).

### Known limitations

| Limitation | Status |
|---|---|
| Real hardware | Tested in VMs only (UEFI). The NVIDIA, AMD and Intel matrix is incomplete |
| Local model | Model licences are under review: the ISO ships no model, it is downloaded afterwards |
| Accessibility | Not certified or audited |
| Language | UI and docs in Brazilian Portuguese; English comes in phase 6 |
| Kalango Remote | Not tested between two physical computers nor in a real niri session (nested only); Mosh not tested (not installed where we tested); the lock-screen path and the screen-transition frame under the privacy cover are covered by code only, not by a test |
| VirtualDesk and Flows | Not on the ISO; the flows service is in a private repository |
| Updates | No update with a restore point and no recovery screen |
| Documentation licence | The CC BY-SA 4.0 proposed for the documentation is still to be confirmed ([LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md)) |
| Roadmap ideas (a live audio source for flows, an inbound internet trigger through a Cloudflare tunnel, Remote inside flows) | Not done yet ([ROADMAP.md](https://github.com/kalangoos/kalangoos/blob/main/ROADMAP.md#integrações-planejadas-ideias-sem-data)) |
| Security | No external review; the limits of the web-address defence are in [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md) |

### Security reporting and licence

Do not open a public issue for a vulnerability: use **Security › Report a vulnerability** in this repository
([SECURITY.md](https://github.com/kalangoos/kalangoos/blob/main/SECURITY.md)). The code is under the GNU GPL, version 3 or later ([LICENSE](https://github.com/kalangoos/kalangoos/blob/main/LICENSE)); the
KalangoOS name, mascot and logos are the project's brand and are not covered by it
([LICENSE-brand.txt](https://github.com/kalangoos/kalangoos/blob/main/LICENSE-brand.txt)).

Kalango Remote is derived from RustDesk and is AGPL-3.0; Kalango Audio is GPL-3.0-or-later; Ubuntu, the kernel and
llama.cpp keep their own licences (each Ubuntu package its own, the kernel GPL-2.0, llama.cpp MIT). See
[LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md) and [ATTRIBUTION.md](https://github.com/kalangoos/kalangoos/blob/main/ATTRIBUTION.md).
