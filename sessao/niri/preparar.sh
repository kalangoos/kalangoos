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
#   0007 · a lupa (ação SetZoom { factor, mode }): um efeito de framebuffer copia a parte da tela em volta do ponteiro,
#          ampliada, para a lente (620 × 380, contorno de 2 px no destaque), a tela inteira ou a metade longe do
#          ponteiro; o ponteiro fica na frente, no tamanho normal; não aparece no bloqueio nem nas capturas de tela.
#   0008 · controle remoto: com KALANGO_NIRI_CONTROLE (a sessão do KalangoOS define), wlr-screencopy, o ponteiro virtual
#          e o teclado virtual só aparecem para os processos que esse arquivo permite (por executável, ou por pid + início
#          do processo). Quem escreve o arquivo é o kalangod (root): o teclado na tela do shell sempre, e o núcleo do
#          Kalango Remote enquanto durar uma sessão aprovada pela pessoa. Sem a variável, como o niri original.
#   0009 · Esc Esc: com uma sessão de controle remoto acontecendo (um processo no arquivo de 0008), Esc duas vezes em
#          meio segundo no teclado deste computador roda KALANGO_ESC_DUPLO (kalango-remote end-sessions), seja qual for
#          a janela com o foco; o que o outro lado digita vem pelo teclado virtual e não passa por aí.
#   0010 · sem aceleração na GPU principal (VirtualBox ou VMware sem 3D, um servidor com VGA simples), aceita o
#          renderizador por software (llvmpipe) nela, em vez de ficar sem tela; as outras GPUs continuam recusando.
#   0011 · agente na tela: com um agente controlando a tela (uma linha "pid" do arquivo de 0008 com a palavra "agente", que
#          o kalangod escreve para o kalango-tela), o teclado, o mouse ou o toque da própria pessoa rodam KALANGO_PESSOA_ATIVA
#          (kalangod tela pessoa: a sessão do agente termina) — só que aconteceu, nunca qual tecla; no máximo uma vez por
#          segundo; e com a sessão bloqueada o ponteiro virtual não faz nada.
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
# (os arquivos novos dos patches também saem, para reaplicar do zero; o target/ é ignorado e fica)
git clean -fdq
for p in ../../../../sessao/niri/*.patch; do
  git apply "$p"
  echo "aplicado: $(basename "$p")"
done
cargo build --release
