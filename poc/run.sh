#!/bin/sh
# KalangoOS · prova de conceito · barra superior + barra lateral direita em Rust/GTK4.
# Não instala nem altera nada no sistema.
#
#   ./run.sh            abre a prova. Numa sessão Wayland com layer-shell ela vira superfície do sistema de verdade;
#                       em X11 ou no GNOME ela abre como PRÉ-VISUALIZAÇÃO numa janela comum (mesmo código, mesma mola).
#   ./run.sh --niri     roda dentro do niri (compositor em Rust) numa janela, preso às bordas. Funciona com NVIDIA.
#   ./run.sh --nested   sobe um compositor Wayland (sway) dentro de uma janela e roda a prova nele, presa às bordas.
#                       Não funciona com o driver NVIDIA proprietário em X11 (limite do compositor aninhado).
#   ./capture.sh        roda num compositor sem janela e salva capturas em capturas/ (é o teste do layer-shell).
#
#   clique no relógio → Avisos · nos ícones de estado → Controles · em "2 agentes" → Agente
#   encoste o ponteiro na borda direita para abrir · Esc fecha · 1 / 2 / 3 trocam o modo · T troca o tema
. "$(dirname "$0")/env.sh"
export KALANGO_POC_WALLPAPER="$WALL"
if [ "$1" = "--niri" ]; then
  # compositor niri (Rust) numa janela: as barras ficam presas às bordas de verdade. Funciona com o driver NVIDIA.
  N="$HERE/.deps/src/niri/target/release/niri"
  [ -x "$N" ] || { echo "niri ainda não foi compilado em poc/.deps/src/niri (cargo build --release)"; exit 1; }
  CFG=$(mktemp --suffix=.kdl)
  cat > "$CFG" <<CFG
spawn-at-startup "swaybg" "-i" "$WALL" "-m" "fill"
spawn-at-startup "sh" "-c" "KALANGO_POC_APP_ID=os.kalango.SidebarPoc.Niri $APP"
hotkey-overlay { skip-at-startup; }
CFG
  exec "$N" --config "$CFG"
fi
if [ "$1" != "--nested" ]; then
  exec env LD_PRELOAD="$LIBS/libgtk4-layer-shell.so" "$BIN"
fi
CONF=$(mktemp --suffix=.kalango-poc.conf)
cat > "$CONF" <<CFG
output * bg $WALL fill
output * resolution ${KALANGO_POC_SIZE:-1600x900}
default_border none
focus_follows_mouse no
exec $APP
bindsym Mod1+c exec $BIN toggle
bindsym Mod1+t exec $BIN theme
bindsym Mod1+Shift+e exit
CFG
exec sway --unsupported-gpu -c "$CONF"
