# Tasks

## 1. Decisões

- [x] 1.1 Base da distribuição decidida (Ubuntu LTS + kernel HWE) e registrada em docs/ARQUITETURA.md
- [x] 1.2 Apps de terceiros via Flatpak decidido e registrado
- [x] 1.3 Desenho do kalangod (agentes operam o sistema sem módulo de kernel) registrado em ARQUITETURA.md, seção 3
- [x] 1.4 Domínio kalangoos.com comprado
- [ ] 1.5 Registro de marca KalangoOS verificado (INPI e EUIPO); resultado anotado em ARQUITETURA.md
- [ ] 1.6 Compositor escolhido entre niri e cosmic-comp; comparação anotada em ARQUITETURA.md
- [ ] 1.7 Toolkit escolhido entre GTK4 e iced/libcosmic; comparação com medidas anotada
- [ ] 1.8 Modelo de restauração escolhido (btrfs ou imagem) e testado numa VM
- [ ] 1.9 Motor de modelos locais escolhido (llama.cpp ou Ollama)
- [ ] 1.10 Escopo da versão 0.1 fechado e registrado em docs/LANCAMENTO.md
- [x] 1.11 Kernel: estável mais recente do kernel.org (7.3.4) compilado pelo KalangoOS sobre a configuração do Ubuntu, com kernel HWE do Ubuntu como reserva; registrado em ARQUITETURA.md

## 2. Provas de conceito

- [x] 2.1 Prova GTK4: barra superior e barra lateral em layer-shell com mola, tokens e tema ao vivo; capturas em poc/capturas
- [x] 2.2 Dependências de desenvolvimento instaladas (libseat, libinput, pipewire, libdisplay-info, systemd)
- [x] 2.3 niri 26.04 compilado e rodando a prova com NVIDIA 580 (poc/run.sh --niri)
- [ ] 2.4 cosmic-comp compilado e rodando a mesma prova; resultado anotado
- [ ] 2.5 Prova com iced/libcosmic da mesma barra lateral; capturas e medidas comparadas
- [ ] 2.6 Script de medida (poc/measure.sh) corrigido e validado: memória e CPU coerentes em 3 execuções
- [ ] 2.7 Teste em sessão Wayland real (fora do X11) com NVIDIA, dois monitores e suspensão; relatório em docs/
- [x] 2.8 Kernel 7.3.4 compilado com kernel/kalango.config em pacotes .deb (kernel/build.sh → kernel/out/)
- [x] 2.9 Kernel 7.3.4 inicia numa VM sem `lsm=` e o `kalangod` reporta bpf_lsm_ativo; as 42 verificações de vm/run.sh passam nele
- [ ] 2.9b Kernel 7.3.4 instalado em hardware real: inicia e o driver NVIDIA 580 carrega (precisa de autorização para instalar)

## 3. Repositório e nomes

- [x] 3.1 Arquivo de design renomeado para design/KalangoOS-DesignSystem.figcraft
- [ ] 3.2 Pasta do repositório renomeada para KalangoOS e FigCraft reaberto no caminho novo
- [ ] 3.3 Tema Lunar, lunar-shell e papéis de parede renomeados; nenhum 'SDRustOS' ou 'Lunar' em `git grep`
- [ ] 3.4 Workspace Cargo criado com os crates previstos; `cargo build --workspace` passa
- [ ] 3.5 Scripts Python e install.sh substituídos pelo binário `kalango` (instalar, remover, gerar tema)
- [x] 3.6 Workspace Cargo na raiz com o primeiro crate (kalangod); `cargo build --workspace` passa

## 4. Ferramentas

- [x] 4.1 Tokens exportados uma vez para design/tokens (JSON e CSS dos dois modos)
- [ ] 4.2 Exportador de tokens em Rust (crate kalango-tokens) gerando CSS e constantes; teste compara com tokens.json
- [ ] 4.3 Integração contínua: build, testes, clippy e fmt a cada push
- [ ] 4.4 Ambiente de desenvolvimento: `kalango dev` sobe a sessão numa janela (niri ou cosmic) e numa VM
- [ ] 4.5 OpenSpec validado na CI (`openspec validate --all --strict`)
