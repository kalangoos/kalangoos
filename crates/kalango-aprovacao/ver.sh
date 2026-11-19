#!/bin/bash
# KalangoOS · mostra a tela de aprovação ao vivo, numa janela do niri (compositor Wayland em Rust) no seu monitor.
# Um agente de teste pede ações uma a uma; você aprova (botão ou Ctrl+Enter) ou nega (botão ou Esc) e o resultado
# aparece no terminal. Tudo em simulação: nada é instalado nem alterado. Feche a janela do niri para sair.
# (Aqui o kalangod roda sem root, então a checagem do "aprovador confiável" fica desligada; ela é validada em validar.sh.)
set -u
cd "$(dirname "$0")"
HERE=$PWD; ROOT=$(realpath ../..)
R="$ROOT/poc/.deps/root"; LIBS="$R/usr/lib/x86_64-linux-gnu"
export LD_LIBRARY_PATH="$LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" PATH="$R/usr/bin:$PATH"
N="$ROOT/poc/.deps/src/niri/target/release/niri"
WALL="$ROOT/wallpapers/sdrustos-16x9-4k.png"
(cd "$ROOT" && PKG_CONFIG_PATH="$LIBS/pkgconfig" cargo build --release -q -p kalango-aprovacao -p kalangod) || exit 1
T=$(mktemp -d); mkdir -p "$T/raiz/etc"
cat > "$T/kalangod.toml" <<CFG
socket = "$T/mcp.sock"
state_dir = "$T/estado"
protected_paths = []
approval_requires_tty = false
approval_timeout_s = 90
total_grant_s = 900
dry_run = true
restore_root = "$T/raiz"
[[callers]]
name = "assistente"
max_level = "total"
allowed_paths = ["$T"]
CFG
"$ROOT/target/release/kalangod" servir --config "$T/kalangod.toml" 2>"$T/kalangod.log" &
KP=$!
for i in $(seq 50); do [ -S "$T/mcp.sock" ] && break; sleep 0.1; done
CFGN="$T/niri.kdl"
cat > "$CFGN" <<KDL
spawn-at-startup "swaybg" "-i" "$WALL" "-m" "fill"
spawn-at-startup "sh" "-c" "KALANGOD_SOCKET=$T/mcp.sock LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland $ROOT/target/release/kalango-aprovacao"
hotkey-overlay { skip-at-startup; }
KDL
"$N" --config "$CFGN" >"$T/niri.log" 2>&1 &
NP=$!
sleep 3
python3 -I "$HERE/agente_demo.py" "$T/mcp.sock" &
AP=$!
wait $NP
kill $AP $KP 2>/dev/null
rm -rf "$T"
