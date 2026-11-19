# KalangoOS · prova de conceito · ambiente comum (carregado por run.sh e capture.sh)
HERE=$(cd "$(dirname "$0")" && pwd)
R="$HERE/.deps/root"
LIBS="$R/usr/lib/x86_64-linux-gnu"
BIN="$HERE/sidebar/target/release/kalango-sidebar-poc"
[ -x "$R/usr/bin/sway" ] && [ -f "$LIBS/libgtk4-layer-shell.so" ] || "$HERE/setup-deps.sh"
[ -x "$BIN" ] || (cd "$HERE/sidebar" && PKG_CONFIG_PATH="$LIBS/pkgconfig" cargo build --release)
export LD_LIBRARY_PATH="$LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$R/usr/bin:$PATH"
# a biblioteca de layer-shell precisa ser carregada antes da libwayland-client
APP="env LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland $BIN"
WALL="$HERE/../wallpapers/sdrustos-16x9-4k.png"
