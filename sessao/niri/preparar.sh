#!/bin/bash
# KalangoOS · obtém o niri na versão-base e aplica os patches do KalangoOS, depois compila.
#
# Patches (sessao/niri/*.patch):
#   0001 · encaixe de janelas (arrastar até a borda/canto: metades, quartos, maximizar; ação snap-window
#          para Super+setas e IPC; prévia na cor do design) e teclas presas na sessão aninhada (quando a
#          janela perde o foco, as teclas pressionadas são soltas: sem Alt preso).
#   0002 · miniaturas de janelas (ação ThumbnailWindow: desenha a janela em tamanho reduzido num PNG, sem
#          área de transferência nem notificação; usada pelo alternador Alt+Tab).
#   0003 · Super sozinho: um toque roda KALANGO_MOD_TAP (a busca) e segurar meio segundo roda KALANGO_MOD_HOLD
#          (o modo voz), ao soltar KALANGO_MOD_HOLD_END; outra tecla, um clique ou a janela perder o foco cancelam
#          (KALANGO_MOD_HOLD_CANCEL). Os comandos vêm do ambiente da sessão.
#   0004 · posição das janelas lado a lado na IPC (tile_pos_in_workspace_view, pela posição-alvo da vista: sem
#          eventos a cada quadro de animação), como as flutuantes já tinham; a ferramenta de captura escolhe a
#          janela pelo ponteiro com ela.
#   0005 · encaixe ao arrastar para as bordas opcional (ação SetEdgeSnap { on }): Configurações › Aparência ›
#          Dock e multitarefa › "Encaixar ao arrastar para as bordas".
#   0006 · minimizar: o botão de minimizar dos apps (xdg set_minimized), a ação minimize-window (Mod+M) e o pedido
#          das barras de tarefas (wlr) escondem a janela sem fechá-la — ela fica no espaço flutuante, sem ser
#          desenhada, sem cliques e sem foco, e sai com a animação de fechar; a IPC diz is_minimized; ativá-la (a dock,
#          Alt+Tab, o próprio app) a traz de volta com a animação de abrir, e para a coluna se veio de uma.
set -eu
cd "$(dirname "$0")/../.."
BASE=8ed0da44d974c32c6877d2f4630c314da0717ecb
D=poc/.deps/src/niri
if [ ! -d "$D/.git" ]; then
  git clone https://github.com/YaLTeR/niri "$D"
fi
cd "$D"
git checkout -q "$BASE"
git checkout -q -- .
for p in ../../../../sessao/niri/*.patch; do
  git apply "$p"
  echo "aplicado: $(basename "$p")"
done
cargo build --release
