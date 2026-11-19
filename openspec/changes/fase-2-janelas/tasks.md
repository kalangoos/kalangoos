# Tasks

## 1. Janelas

- [ ] 1.1 Moldura no estilo do design (controles à direita, estados ativa/inativa/modal); comparação visual com o design
- [ ] 1.2 Abrir, fechar, minimizar e maximizar com mola; gravação de 60 qps sem quadros perdidos
- [x] 1.3 Encaixe em metades, terços e grade; seletor de layouts com Super+Z: no niri (patch `sessao/niri/0001`), arrastar até a borda dá metades, até o canto dá quartos e até o topo maximiza, com prévia na cor do design e volta ao tamanho de antes ao arrastar para fora; Super+setas no estilo Windows/GNOME (metade → quarto → de volta); ação `snap-window` (IPC); seletor com Metades, Foco + lateral, Foco + pilha, Três colunas e Grade (área clicada ou layout inteiro para as janelas da área de trabalho). Validado em `sessao/testar.sh`
- [x] 1.3b Durante o arraste, o seletor desce da barra (evento `WindowDragChanged` do niri) e a janela pode ser solta numa área dele (alvos `SetSnapTargets`); as outras janelas da área de trabalho ocupam as áreas restantes do layout. Validado em `sessao/testar.sh`
- [ ] 1.4 Alt+Tab com miniaturas ao vivo
- [ ] 1.5 XWayland para apps antigos; teste com 3 apps X11

## 2. Espaços

- [ ] 2.1 Espaços com nome, cor e regras; trocar com Super+Tab e Super+1…4
- [ ] 2.2 Espaço lembra janelas e posições após reiniciar a sessão
- [ ] 2.3 Visão geral com janelas e espaços, por tecla e por gesto

## 3. Monitores e entrada

- [ ] 3.1 Dois monitores com escalas diferentes (100% e 125%); janela mantém o tamanho físico ao mudar de monitor
- [ ] 3.2 Desconectar monitor move janelas para o restante; reconectar devolve
- [ ] 3.3 Gestos de 3 e 4 dedos que seguem o dedo
- [ ] 3.4 Modo toque ativado ao destacar teclado ou girar a tela

## 4. Captura

- [ ] 4.1 Captura de tela, de janela e de região pelo portal; atalho Print
- [ ] 4.2 Gravação de tela e compartilhamento em navegador (Meet) pelo portal

## 5. Compatibilidade gráfica

- [x] 5.1 Compositor Rust (niri) rodando aninhado com NVIDIA 580 + layer-shell
- [ ] 5.2 Sessão real com NVIDIA RTX 3090 / driver 580: 60 qps em 3440×1440, sem artefatos, suspensão e retorno OK
- [ ] 5.3 Matriz de teste: NVIDIA (proprietário), AMD e Intel (Mesa); resultados em docs/REQUISITOS.md
- [ ] 5.4 Modo de segurança gráfico (desenho por software) acessível pela recuperação
