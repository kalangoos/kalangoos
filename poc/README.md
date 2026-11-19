# KalangoOS · prova de conceito

Barra superior e barra lateral direita **do sistema** (não são janelas de app), escritas em Rust com GTK4 e presas
às bordas da tela pelo protocolo *layer-shell* do Wayland. Serve para responder a uma pergunta: *dá para ter a
mesma qualidade do design, com animação própria, fora do GNOME?*

## Como rodar

```sh
./run.sh            # abre a prova
./capture.sh        # roda num compositor sem janela e salva capturas em capturas/
./run.sh --niri     # dentro do niri (compositor em Rust) numa janela, presa às bordas; funciona com NVIDIA
./run.sh --nested   # compositor Wayland (sway) dentro de uma janela, com a prova presa às bordas
```

- Numa sessão Wayland com layer-shell, `./run.sh` cria as superfícies de verdade.
- Em X11 ou no GNOME (que não tem layer-shell), `./run.sh` abre uma **pré-visualização** numa janela comum:
  mesmo código, mesma mola, mesmos tokens.
- `--nested` não funciona com o driver NVIDIA proprietário em X11 (limite do compositor aninhado). Nesse caso o
  teste do layer-shell é o `./capture.sh`.

Na primeira vez os scripts chamam `setup-deps.sh`, que baixa o sway e o grim e compila o `gtk4-layer-shell`
(o Ubuntu 24.04 não tem pacote) dentro de `poc/.deps/`. **Nada é instalado no sistema**; apague `poc/.deps/`
para desfazer.

Uso: clique no relógio → Avisos; nos ícones de estado → Controles; em "2 agentes" → Agente. Encoste o ponteiro
na borda direita para abrir. `Esc` fecha, `1` `2` `3` trocam o modo, `T` troca o tema. Uma segunda chamada do
binário fala com a instância aberta: `kalango-sidebar-poc open avisos | close | toggle | theme | quit`.

## O que ela prova

| Pergunta | Resultado |
|---|---|
| Superfície presa à borda, por cima das janelas, reservando espaço | Sim: duas superfícies layer-shell (barra superior com zona exclusiva; barra lateral em *overlay*) |
| Estilo fiel ao Design System | Sim: `assets/style.css` só usa os tokens exportados do FigCraft (`design/tokens/`), sem sombras, raios 8/6/4 |
| Animação própria, não do CSS | Sim: mola integrada a cada quadro (`Spring` em `src/main.rs`) com os números do DS: suave 300/30, rápida 520/38 |
| Fluidez | 60 quadros por segundo mesmo com desenho por software; a mola chega a 96 % em ~207 ms e assenta em ~600 ms (`capturas/mola.txt`) |
| Cascata | Os blocos entram um a um, com 24 ms de intervalo |
| Tema claro/escuro ao vivo | Sim: troca o arquivo de tokens sem recriar a interface |
| Cliques passam quando fechada | Sim: fechada, a superfície só aceita o ponteiro na alça da borda |
| Tudo em Rust | Sim: 733 linhas de Rust e 67 de CSS; binário de 0,6 MB |

## O que ela **não** prova

- Não há compositor nosso: janelas, espaços, encaixe e animações de janela não foram testados.
- Os controles não mexem no sistema (Wi-Fi, som e brilho são só interface).
- Os ícones são os simbólicos do Adwaita, não os do Design System; o calendário usa o widget padrão do GTK.
- Não foi medido em GPU nem em 120 Hz; não foi testado em tela HiDPI nem com dois monitores.
- O modo Agente é uma maquete: não há motor de agentes por trás.
