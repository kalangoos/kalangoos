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
  alcance dos agentes e defesa contra dado local saindo no endereço da web.
- **Agentes locais** com o LFM3 pelo llama.cpp, gestão de contexto da conversa (janela recente, resumo em segundo
  plano e busca na conversa) e agentes que operam a tela pela visão, sempre com aprovação.
- **Fluxos**: editor de fluxo completo no shell; o modelo de decisão d1 responde nos blocos "Decidir".
- **Kalango Remote** (compartilhar a tela, controlar outro computador, SSH e Mosh) e **Kalango Audio** (capturar,
  tratar, rotear e gravar som) na imagem. O Remote ainda não foi testado entre dois computadores físicos nem numa
  sessão real do niri (só aninhada).
- **Kalango VirtualDesk**: o `kalangod` tem as ferramentas `vm.*` e o shell tem a tela em Ajustes › Sandbox; o app em
  si não vai na ISO.
- **Fonte dos modelos de IA** também em Ajustes e na Central de Apps.
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
     -device virtio-vga-gl,xres=1920,yres=1080 -display sdl,gl=on \
     -device virtio-keyboard-pci -device virtio-tablet-pci -nic user,model=virtio-net-pci
   ```

   (antes, `qemu-img create -f qcow2 disco.qcow2 40G` e `cp /usr/share/OVMF/OVMF_VARS_4M.fd uefi-vars.fd`). Depois da
   instalação, `imagem/rodar-iso.sh` sem argumento liga o disco instalado primeiro.

4. **Modelos de IA.** A ISO não leva o modelo local. Na etapa "Modelos de IA" do instalador (ou depois, em Ajustes e
   na Central de Apps), escolha o catálogo público, um servidor privado ou uma pasta com os modelos.

## Ainda não incluído na 0.1.0

- Modo privacidade do Kalango Remote.
- Aprovação automática pelo modelo d1: toda ação arriscada continua pedindo a aprovação da pessoa.

## Limitações conhecidas

| Limitação | Situação |
|---|---|
| Hardware real | Testada só em VM (UEFI). A matriz NVIDIA, AMD e Intel não está completa |
| Modelo local | As licenças dos modelos estão em revisão: a ISO não leva modelo, ele é baixado depois |
| Acessibilidade | Sem certificação nem auditoria |
| Idioma | Interface e documentação em português do Brasil; o inglês vem na fase 6 |
| Kalango Remote | Ainda não há modo privacidade, verificação em duas etapas, gravação de sessões nem compartilhamento por servidor próprio; não foi testado entre dois computadores físicos nem numa sessão real do niri (só aninhada) |
| VirtualDesk e Fluxos | Fora da ISO; o serviço de fluxos está num repositório privado |
| Atualizações | Sem atualização com ponto de restauração nem tela de recuperação |
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
  defence against local data leaving in a web address.
- **Local agents** with LFM3 through llama.cpp, conversation context management (recent window, background summary,
  search in the conversation) and agents that operate the screen by vision, always with approval.
- **Flows**: a full flow editor in the shell; the d1 decision model answers in "Decidir" blocks.
- **Kalango Remote** (screen sharing, remote control, SSH and Mosh) and **Kalango Audio** (capture, process, route
  and record sound) on the image. Remote has not been tested between two physical computers nor in a real niri
  session (nested only).
- **Kalango VirtualDesk**: `kalangod` has the `vm.*` tools and the shell has the Settings › Sandbox screen; the app
  itself is not on the ISO.
- **AI model source** also in Settings and in the app center.
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

- Kalango Remote privacy mode.
- Automatic approval by the d1 model: every risky action still asks for the person's approval.

### Known limitations

| Limitation | Status |
|---|---|
| Real hardware | Tested in VMs only (UEFI). The NVIDIA, AMD and Intel matrix is incomplete |
| Local model | Model licences are under review: the ISO ships no model, it is downloaded afterwards |
| Accessibility | Not certified or audited |
| Language | UI and docs in Brazilian Portuguese; English comes in phase 6 |
| Kalango Remote | No privacy mode, two-step verification, session recording or sharing through an own server yet; not tested between two physical computers nor in a real niri session (nested only) |
| VirtualDesk and Flows | Not on the ISO; the flows service is in a private repository |
| Updates | No update with a restore point and no recovery screen |
| Security | No external review; the limits of the web-address defence are in [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md) |

### Security reporting and licence

Do not open a public issue for a vulnerability: use **Security › Report a vulnerability** in this repository
([SECURITY.md](https://github.com/kalangoos/kalangoos/blob/main/SECURITY.md)). The code is under the GNU GPL, version 3 or later ([LICENSE](https://github.com/kalangoos/kalangoos/blob/main/LICENSE)); the
KalangoOS name, mascot and logos are the project's brand and are not covered by it
([LICENSE-brand.txt](https://github.com/kalangoos/kalangoos/blob/main/LICENSE-brand.txt)).

Kalango Remote is derived from RustDesk and is AGPL-3.0; Kalango Audio is GPL-3.0-or-later; Ubuntu, the kernel and
llama.cpp keep their own licences (each Ubuntu package its own, the kernel GPL-2.0, llama.cpp MIT). See
[LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md) and [ATTRIBUTION.md](https://github.com/kalangoos/kalangoos/blob/main/ATTRIBUTION.md).
