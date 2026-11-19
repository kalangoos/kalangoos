<p align="center">
  <img src=".github/assets/cabecalho.png" alt="KalangoOS: o Linux em que agentes de IA trabalham com você" width="100%">
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-GTK%204%20·%20niri-A5A1E2?style=flat-square&logo=rust&logoColor=white">
  <img alt="Base: Ubuntu LTS" src="https://img.shields.io/badge/base-Ubuntu%20LTS-E5C98C?style=flat-square">
  <img alt="Licença: GPL-3.0-or-later" src="https://img.shields.io/badge/licen%C3%A7a-GPL--3.0--or--later-8ED2C3?style=flat-square">
  <img alt="Estado: em desenvolvimento" src="https://img.shields.io/badge/estado-em%20desenvolvimento-EFA2A6?style=flat-square">
  <a href="https://github.com/kalangoos/kalangoos/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/kalangoos/kalangoos/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="#o-que-é">O que é</a> ·
  <a href="#onde-estamos">Onde estamos</a> ·
  <a href="#como-é-feito">Como é feito</a> ·
  <a href="#rodar-a-sessão-de-desenvolvimento">Rodar</a> ·
  <a href="ROADMAP.md">Roteiro</a> ·
  <a href="CONTRIBUTING.md">Contribuir</a> ·
  <a href="#english">English</a>
</p>

> [!IMPORTANT]
> **O KalangoOS está em desenvolvimento e ainda não tem imagem para instalar.** Hoje o desktop, os apps e os
> agentes rodam numa sessão de desenvolvimento (um compositor dentro de uma janela) e numa máquina virtual. Não
> use em um computador com dados que você não pode perder.

## O que é

Uma distribuição Linux para computadores pessoais em que agentes de IA trabalham junto com você: entendem pedidos
em linguagem natural, operam o sistema e os apps, e tudo o que fazem fica visível, pede aprovação quando é
arriscado e pode ser desfeito.

- **Agentes que fazem, não só conversam.** Eles mexem no sistema de verdade pelo `kalangod`, um serviço próprio
  que fala o protocolo MCP, com quatro níveis de poder (observar, usuário, sistema, total).
- **Nada irreversível sem você.** Ações arriscadas pedem aprovação numa tela desenhada pelo sistema, que o agente
  não consegue imitar; cada ação entra na Linha do tempo, com desfazer e ponto de restauração.
- **Local por padrão.** O modelo padrão (LFM3, da Liquid AI) roda no seu computador pelo llama.cpp. Nuvem só com
  a sua permissão.
- **Regras impostas pelo kernel.** eBPF LSM, Landlock, seccomp e cgroups, sem módulo de kernel próprio: as
  atualizações do kernel do Ubuntu continuam funcionando.
- **Um desktop próprio.** Shell em Rust e GTK 4 sobre o compositor [niri](https://github.com/YaLTeR/niri), apps
  do próprio sistema e um único Design System, nos temas escuro e claro.

## Onde estamos

| Frente | Situação |
|---|---|
| Shell (barra, dock, barras laterais, área de trabalho, bloqueio) | Funciona na sessão de desenvolvimento, com testes automáticos |
| Apps do sistema (Arquivos, Configurações, Terminal, Editor, Calculadora, Central de agentes…) | Vários prontos; outros em andamento |
| `kalangod` e agentes (`kalango-agente` com LFM3 local) | Funcionam; validados numa VM com o kernel do KalangoOS |
| Tela de entrada, instalador e primeiro uso | Telas e motor prontos; o instalador ainda não grava num disco de verdade |
| Imagem instalável (disco ou ISO) | **Ainda não existe** — é o próximo passo ([ROADMAP.md](ROADMAP.md)) |

## Como é feito

```text
crates/
  kalango-shell/        o desktop inteiro: barra, dock, barras laterais, apps do sistema, Central de agentes
  kalangod/             o "MCP do sistema": serviço privilegiado com regras no kernel (eBPF, Landlock, cgroups)
  kalango-agente/       o motor dos agentes (llama.cpp + LFM3 local), fala com o kalangod
  kalango-aprovacao/    a tela de aprovação, desenhada fora do alcance dos agentes
  kalango-entrada/      a tela de entrada (greetd)
  kalango-instalador/   o instalador
  kalango-movimento/    as molas das animações
  kalango-i18n/         as traduções (pt-BR hoje)
sessao/                 a sessão: niri, patches do niri, arquivos da imagem, suíte de testes da sessão
design/tokens/          cores, movimento e medidas exportados do Design System
openspec/               a especificação, fase por fase
docs/                   visão, arquitetura, segurança, requisitos, plano
```

As decisões e os motivos estão em [docs/ARQUITETURA.md](docs/ARQUITETURA.md); o modelo de ameaças em
[docs/SEGURANCA.md](docs/SEGURANCA.md).

## Rodar a sessão de desenvolvimento

Testado no Ubuntu 24.04. Nada é instalado no sistema: as dependências que o Ubuntu não tem vão para `poc/.deps`.

```sh
sudo apt install build-essential clang libbpf-dev pkg-config libgtk-4-dev libvte-2.91-gtk4-dev \
  libpoppler-glib-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev   # bibliotecas de desenvolvimento
poc/setup-deps.sh        # gtk4-layer-shell, Inter e o resto, em poc/.deps (sem sudo)
sessao/rodar.sh          # a sessão inteira numa janela (niri aninhado), kalangod em simulação
```

O niri com os patches do KalangoOS é montado por `sessao/niri/preparar.sh`. Os agentes precisam do llama.cpp e do
modelo LFM3 (veja [docs/ARQUITETURA.md](docs/ARQUITETURA.md), decisão D6). Detalhes em
[sessao/README.md](sessao/README.md).

Testes:

```sh
cargo test --workspace --exclude kalango-agente   # o que a CI roda
cargo test -p kalango-agente                      # inclui os testes com o modelo local de verdade
sessao/testar.sh                                  # a suíte da sessão (niri aninhado, ~25 min)
```

## Contribuir

Leia o [CONTRIBUTING.md](CONTRIBUTING.md). Agentes de código (Claude Code, Codex e outros) seguem o
[AGENTS.md](AGENTS.md). Falhas de segurança: [SECURITY.md](SECURITY.md), nunca em issue pública.

## Licença

O código do KalangoOS é software livre sob a **GNU General Public License, versão 3 ou posterior**
([LICENSE](LICENSE), `GPL-3.0-or-later`). O nome KalangoOS, o mascote e os logos são a marca do projeto e ficam fora
dessa licença ([LICENSE-brand.txt](LICENSE-brand.txt)). Recursos de terceiros: [ATTRIBUTION.md](ATTRIBUTION.md) e
[NOTICE](NOTICE).

---

## English

**KalangoOS is a Linux distribution where AI agents work alongside you:** they understand plain-language requests
and operate the system and its apps, and everything they do is visible, asks for approval when it is risky, and
can be undone.

> [!IMPORTANT]
> KalangoOS is in development. **There is no installable image yet**: the desktop, apps and agents run in a
> development session (a compositor inside a window) and in a virtual machine.

- **Agents that act.** They operate the system through `kalangod`, our privileged service that speaks MCP, with
  four power levels and kernel-enforced rules (eBPF LSM, Landlock, seccomp, cgroups; no out-of-tree module).
- **Nothing irreversible without you.** Risky actions ask for approval on a screen drawn by the system; every action
  goes to an undoable timeline.
- **Local first.** The default model (Liquid AI's LFM3) runs on your machine through llama.cpp.
- **Its own desktop.** A Rust and GTK 4 shell on the [niri](https://github.com/YaLTeR/niri) compositor, with the
  system's own apps and one design system.

The docs and the UI are in Brazilian Portuguese first; English comes next ([ROADMAP.md](ROADMAP.md), phase 6).
Build and run steps are above; contributions are welcome in Portuguese or English ([CONTRIBUTING.md](CONTRIBUTING.md)).
The code is licensed under the GNU GPL, version 3 or later ([LICENSE](LICENSE)); the KalangoOS name, mascot and
logos are the project's brand and are not covered by it ([LICENSE-brand.txt](LICENSE-brand.txt)).
