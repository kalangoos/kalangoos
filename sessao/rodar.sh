#!/bin/bash
# KalangoOS · abre a sessão inteira numa janela do seu desktop atual (niri aninhado), para ver e testar.
# Nada no sistema muda: o kalangod roda sem root em modo de simulação, e bloquear/suspender/reiniciar/desligar
# só são registrados (KALANGO_SHELL_DEV=1). Feche a janela, ou use o menu de energia > Sair, para terminar.
#
# Dentro da janela, a tecla Mod é o Alt (o niri aninhado deixa o Super para o seu desktop):
#   Alt+Espaço barra de comando · Alt+A todos os apps · Alt+Shift+E energia · Alt+T terminal · Alt+Q fecha janela
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
R="$ROOT/poc/.deps/root"; LIBS="$R/usr/lib/x86_64-linux-gnu"
[ -f "$R/fonts.conf" ] && export FONTCONFIG_FILE="$R/fonts.conf"   # Inter, a fonte do Design System
export LD_LIBRARY_PATH="$LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" PATH="$R/usr/bin:$PATH"
N="${KALANGO_NIRI:-$ROOT/poc/.deps/src/niri/target/release/niri}"
[ -x "$N" ] || { echo "falta o niri em poc/.deps/src/niri (cargo build --release lá)"; exit 1; }
PKG_CONFIG_PATH="$LIBS/pkgconfig" cargo build --release -q -p kalango-shell -p kalango-aprovacao -p kalangod -p kalango-agente -p kalango-tela || exit 1
T=$(mktemp -d /tmp/kalango-sessao-XXXX)
mkdir -p "$T/raiz/etc"
# KALANGO_AREA_TESTE: the only folder the agents may touch (tests and captures: never the person's real home, which
# kalangod resolves from passwd)
cat > "$T/kalangod.toml" <<CFG
socket = "$T/mcp.sock"
state_dir = "$T/estado"
protected_paths = ["~/.ssh", "~/.local/share/kalango/memoria.json", "~/.local/share/kalango/memoria.bin"]
approval_requires_tty = false
approval_timeout_s = 120
total_grant_s = 900
dry_run = true
restore_root = "$T/raiz"
# who may capture the screen and inject input in this niri (patch 0008): the shell types (on-screen keyboard);
# Kalango Remote (the repository next to this one, or KALANGO_REMOTE_APP) asks for its core after an approval
controle_remoto = "$T/controle-remoto"
controle_remoto_sempre = ["$ROOT/target/release/kalango-shell teclado"]
controle_remoto_apps = ["${KALANGO_REMOTE_APP:-$ROOT/../kalango-remote/target/release/kalango-remote}", "$ROOT/../kalango-remote/target/debug/kalango-remote", "/usr/bin/kalango-remote"]
# an agent operating the screen (tela.*): the helper kalangod starts for a session, as the person
tela_auxiliar = "$ROOT/target/release/kalango-tela"
[[callers]]
name = "voce"
max_level = "total"
allowed_paths = ["${KALANGO_AREA_TESTE:-~}"]
CFG
"$ROOT/target/release/kalangod" servir --config "$T/kalangod.toml" 2>"$T/kalangod.log" &
KP=$!
# supervisão como na sessão real (Restart=always, 200 ms): se o processo cair, volta sozinho.
# Recarga ao vivo: quando o binário é recompilado, o processo é reiniciado com a versão nova.
supervisor() { # nome binário
cat > "$T/$1.sh" <<SH
#!/bin/bash
# a saída de cada processo fica num arquivo da sessão (o niri descarta a saída do que inicia)
exec >> $T/$1.log 2>&1
export KALANGO_SHELL_DEBUG=${KALANGO_SHELL_DEBUG:-} KALANGO_TESTE_COPIA_LENTA=${KALANGO_TESTE_COPIA_LENTA:-} KALANGO_PACKAGEKIT_BUS=${KALANGO_PACKAGEKIT_BUS:-} KALANGO_NMCLI=${KALANGO_NMCLI:-} KALANGO_NMCLI_ESTADO=${KALANGO_NMCLI_ESTADO:-} KALANGO_BLUETOOTHCTL=${KALANGO_BLUETOOTHCTL:-} KALANGO_BLUETOOTHCTL_ESTADO=${KALANGO_BLUETOOTHCTL_ESTADO:-} KALANGO_PW_DUMP=${KALANGO_PW_DUMP:-} KALANGO_WPCTL=${KALANGO_WPCTL:-} KALANGO_PW_CLI=${KALANGO_PW_CLI:-} KALANGO_PW_RECORD=${KALANGO_PW_RECORD:-} KALANGO_PW_ESTADO=${KALANGO_PW_ESTADO:-} KALANGO_PW_FILTRO=${KALANGO_PW_FILTRO:-} KALANGO_SONS=${KALANGO_SONS:-} KALANGO_POWERPROFILESCTL=${KALANGO_POWERPROFILESCTL:-} KALANGO_UPOWER=${KALANGO_UPOWER:-} KALANGO_ENERGIA_ESTADO=${KALANGO_ENERGIA_ESTADO:-} KALANGO_POWER_SUPPLY=${KALANGO_POWER_SUPPLY:-} KALANGO_ACPI_LID=${KALANGO_ACPI_LID:-} KALANGO_MINUTO_MS=${KALANGO_MINUTO_MS:-} KALANGO_CLIMA_URL=${KALANGO_CLIMA_URL:-} KALANGO_PAM_DIR=${KALANGO_PAM_DIR:-} KALANGO_PAPEL_DE_PAREDE=$ROOT/wallpapers/sdrustos-16x9-4k.png KALANGO_PAPEL_DE_PAREDE_CLARO=$ROOT/wallpapers/sdrustos-16x9-4k-claro.png KALANGO_PAPEL_DE_PAREDE_ALTO=$ROOT/wallpapers/lunar-tall.png KALANGO_PAPEL_DE_PAREDE_LARGO=$ROOT/wallpapers/lunar-wide.png KALANGOD_SOCKET=$T/mcp.sock KALANGO_SHELL_DEV=1 LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland
# the system's own apps (sessao/dist/share: the agents center) and the shell's command (kalango-shell central)
export XDG_DATA_DIRS=$ROOT/sessao/dist/share:\${XDG_DATA_DIRS:-/usr/local/share:/usr/share} PATH=$T:\$PATH
B=$2
while :; do
  \$B & P=\$!; M=\$(stat -c %Y \$B)
  while kill -0 \$P 2>/dev/null; do
    sleep 1
    N=\$(stat -c %Y \$B 2>/dev/null || echo \$M)
    if [ "\$N" != "\$M" ]; then sleep 0.5; echo "$1: binário novo, recarregando" >&2; kill \$P; fi
  done
  wait \$P; echo "$1 saiu (código \$?); reiniciando" >&2; sleep 0.2
done
SH
chmod +x "$T/$1.sh"
}
supervisor shell "$ROOT/target/release/kalango-shell"
supervisor aprovacao "$ROOT/target/release/kalango-aprovacao"
# o comando do atalho fala com o shell que já está rodando (mesma instância GApplication)
cat > "$T/kalango-shell" <<SH
#!/bin/sh
exec env KALANGO_SHELL_DEV=1 LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland $ROOT/target/release/kalango-shell "\$@"
SH
chmod +x "$T/kalango-shell"
# the person's own mouse or keyboard while an agent controls the screen (niri patch 0011): the session ends
cat > "$T/tela-pessoa" <<SH
#!/bin/sh
exec env KALANGOD_SOCKET=$T/mcp.sock $ROOT/target/release/kalangod tela pessoa
SH
chmod +x "$T/tela-pessoa"
# Super alone (niri patch 0003; in this nested session the Mod key is Alt): a tap opens the search, held it is voice mode
export KALANGO_MOD_TAP="$T/kalango-shell super tocar" KALANGO_MOD_HOLD="$T/kalango-shell super segurar" KALANGO_MOD_HOLD_END="$T/kalango-shell super soltar" KALANGO_MOD_HOLD_CANCEL="$T/kalango-shell super cancelar"
# cursor: o mesmo tema e tamanho do desktop de quem está testando
CURSOR_TEMA=$(gsettings get org.gnome.desktop.interface cursor-theme 2>/dev/null | tr -d "'"); CURSOR_TEMA=${CURSOR_TEMA:-Adwaita}
CURSOR_TAMANHO=$(gsettings get org.gnome.desktop.interface cursor-size 2>/dev/null); CURSOR_TAMANHO=${CURSOR_TAMANHO:-24}
TERMINAL=$(command -v gnome-terminal || command -v kitty || command -v alacritty || command -v foot || echo xterm)
sed -e "s#@TECLADO@#${KALANGO_TECLADO:-br}#" -e "s#@PAPEL_DE_PAREDE@#$ROOT/wallpapers/sdrustos-16x9-4k.png#" -e "s#@SHELL@#$T/kalango-shell#g" -e "s#@TERMINAL@#$TERMINAL#" -e "s#@CURSOR_TEMA@#$CURSOR_TEMA#" -e "s#@CURSOR_TAMANHO@#$CURSOR_TAMANHO#" -e "s#@CONFIG@#${XDG_CONFIG_HOME:-$HOME/.config}#g" \
    -e "s#@INICIO@#spawn-at-startup \"$T/shell.sh\"\nspawn-at-startup \"$T/aprovacao.sh\"#" sessao/niri.kdl.modelo > "$T/niri.kdl"
"$N" validate --config "$T/niri.kdl" || exit 1
echo "sessão de teste em $T (registros: $T/niri.log, $T/shell.log, $T/aprovacao.log, $T/kalangod.log)"
# barramento D-Bus próprio, como numa sessão real: notificações, instâncias únicas e serviços da sessão de
# teste não se misturam com os do seu desktop
# XMODIFIERS=@im=none: com o IBus via XIM, as letras chegariam ao niri como texto do método de entrada,
# que ele descarta (os atalhos passam, a digitação não).          
KALANGO_PESSOA_ATIVA="$T/tela-pessoa" KALANGO_ESC_DUPLO="${KALANGO_REMOTE_APP:-$ROOT/../kalango-remote/target/release/kalango-remote} end-sessions" KALANGO_NIRI_CONTROLE="$T/controle-remoto" KALANGO_NIRI_CONTROLE_DONO="$(id -u)" XMODIFIERS=@im=none XCURSOR_THEME="$CURSOR_TEMA" XCURSOR_SIZE="$CURSOR_TAMANHO" /usr/bin/dbus-run-session -- "$N" --config "$T/niri.kdl" > "$T/niri.log" 2>&1
kill $KP 2>/dev/null
pkill -f "$T/shell.sh"; pkill -f "$T/aprovacao.sh"
