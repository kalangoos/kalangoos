#!/bin/bash
# KalangoOS · regression test of the real session input path, without touching your screen.
#
# Starts the KalangoOS session (nested niri, the same as rodar.sh) inside a virtual X display (Xvfb) and drives it
# with xdotool, the same route the keyboard and mouse take on a desktop:
#   1. typing into the command bar reaches the text field;
#   2. a GTK app opens with its own title bar and the X button closes it;
#   3. the session cursor has a theme set (niri draws the cursor);
#   4. the agent conversation in the side bar: Enter sends (the real local model starts working) and the stop
#      button ends the engine;
#   5. a modifier held while the session window loses focus is released (no stuck Alt);
#   6. window snapping: drag to the left edge → left half; Super+Up → top-left quarter; Super+Down back;
#   7. the layout picker (Super+Z) snaps the window into the clicked region;
#   8. dragging a window near the top bar drops the picker; releasing on a region snaps it there and the
#      other window takes the remaining region of that layout;
#   9. dictation in the agent composer: the microphone records and the local voice model (LFM3-Audio)
#      writes what was said.
# Needs: Xvfb, xdotool. The apps the tests open are KalangoOS's own (the Calculator and the Text Editor), never another
# system's.
set -u
# a crash leaves its core in the repository's root (core_pattern "core"): gdb reads the backtrace from it
ulimit -c unlimited
cd "$(dirname "$0")/.."
ROOT=$PWD
DISP=:${KALANGO_TESTE_DISPLAY:-77}
OUT=$(mktemp -d /tmp/kalango-teste-XXXX)
N="${KALANGO_NIRI:-$ROOT/poc/.deps/src/niri/target/release/niri}"
fail=0
ok() { if eval "$2"; then echo "  ok   $1"; else echo "  FALHOU $1"; fail=1; fi; }
# another run still on this display (one that stopped half-way): two sessions on one screen take each other's keys
if [ -e "/tmp/.X${DISP#:}-lock" ] && kill -0 "$(tr -d ' ' < "/tmp/.X${DISP#:}-lock")" 2>/dev/null; then
  echo "a tela $DISP já está em uso (outra rodada da suíte?): nada foi testado"; rmdir "$OUT" 2>/dev/null; exit 2
fi
# whatever way the run ends (the last check, or a script error half-way), its session goes: niri, the display, the
# stand-ins, and everything on the session's private bus (portals, gvfs, goa) — left behind, they pile up, hold the
# display and the stand-ins' ports, and use up the user's inotify instances (folders stop being watched)
terminou=0
limpar() {
  [ -n "${CLIMA:-}" ] && kill "$CLIMA" 2>/dev/null
  local bus=""
  if [ -n "${T:-}" ]; then
    bus=$(tr '\0' '\n' < /proc/$(pgrep -f "^[^ ]*niri --config $T/niri.kdl" | head -1)/environ 2>/dev/null | sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p' | sed 's/,.*//')
    # (niri itself, not the dbus-run-session around it: that one then ends its bus daemon, as it should)
    pkill -f "^[^ ]*/niri --config $T/niri.kdl"
    sleep 1
  fi
  [ -n "${XP:-}" ] && kill "$XP" 2>/dev/null
  if [ -n "$bus" ]; then
    for d in /proc/[0-9]*; do
      { tr '\0' '\n' < "$d/environ"; } 2>/dev/null | grep -q "^DBUS_SESSION_BUS_ADDRESS=$bus" && kill "${d#/proc/}" 2>/dev/null
    done
  fi
  if [ "$terminou" = 1 ] && [ "$fail" = 0 ]; then rm -rf "$OUT" "${T:-}"; else
    [ "$terminou" = 1 ] || echo "  FALHOU a rodada parou no meio (veja o erro acima)"
    echo "registros em $OUT e ${T:-}"; fi
}
trap limpar EXIT

Xvfb "$DISP" -screen 0 1600x900x24 >/dev/null 2>&1 &
XP=$!
sleep 1
# the apps opened by the test use throwaway profile folders: they never read or change the person's
# documents, recent files or settings (the local model is pointed to explicitly)
MODELO="${XDG_DATA_HOME:-$HOME/.local/share}/kalango/modelos/LFM3-1.7B-Q4_K_M.gguf"
# a spoken sentence for the microphone test, made by the local voice model itself (LFM3-Audio)
MODELOS="${XDG_DATA_HOME:-$HOME/.local/share}/kalango/modelos"
TTS="$ROOT/poc/.deps/src/llama.cpp/build/bin/llama-tts"
FALA="$OUT/fala.wav"
[ -x "$TTS" ] && "$TTS" -m "$MODELOS/LFM3-Audio-2B-Q4_K_M.gguf" -mm "$MODELOS/mmproj-LFM3-Audio-2B-Q8_0.gguf" \
    -p "Quanto espaço livre eu tenho no disco principal?" --tts-lang pt --seed 7 -o "$FALA" >/dev/null 2>&1
X="$OUT/perfil"
mkdir -p "$X/data" "$X/config" "$X/state" "$X/cache"
# the "Hoje" bar's sources: a local calendar with three events today, a city for the weather, and a stand-in for
# Open-Meteo on this computer (the test never reaches the internet)
mkdir -p "$X/data/evolution/calendar/system" "$X/config/kalango"
D=$(date +%Y%m%d); H=$(date -d '+25 min' +%H%M)
printf 'BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Reunião de produto\r\nDTSTART:%sT%s00\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Academia\r\nDTSTART:%sT183000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Revisar PR\r\nDTSTART:%sT210000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n' "$D" "$H" "$D" "$D" > "$X/data/evolution/calendar/system/calendar.ics"
printf '{"cidade": "São Paulo", "regiao": "São Paulo, Brasil", "lat": -23.55, "lon": -46.63}' > "$X/config/kalango/clima.json"
# the lock screen checks passwords through PAM: a configuration of its own with a test password (never the person's)
# Settings › Power: a laptop's battery with a charge limit and a lid (folders of the test's own, as sysfs and ACPI), the
# idle times off until the test sets them (a "minute" is 4 s here), and the last day's draw (a sample every 5 minutes)
mkdir -p "$OUT/power_supply/BAT0" "$OUT/lid/LID0" "$X/state/kalango"
echo 100 > "$OUT/power_supply/BAT0/charge_control_end_threshold"
# (and Do not disturb only by hand: by itself it would hold every banner back when the test runs at night)
printf '{"screen_off_min": 0, "suspend_min": 0, "notifications": {"dnd_auto": false}}' > "$X/config/kalango/configuracoes.json"
python3 - "$X/state/kalango/energia.jsonl" <<'PY'
import json, math, sys, time
agora = int(time.time()); linhas = []
for k in range(288, 0, -1):
    t = agora - k * 300
    h = time.localtime(t).tm_hour + time.localtime(t).tm_min / 60
    w = 42 + 30 * max(0.0, math.sin((h - 7) / 16 * math.pi)) + (370 if abs(h - 21.17) < 0.05 else 0)
    wh = w * 300 / 3600
    linhas.append(json.dumps({"t": t, "w": round(w, 1), "wh": round(wh, 4), "apps": {"Modelo local": round(wh * 0.42, 4), "Chrome": round(wh * 0.14, 4), "Terminal": round(wh * 0.07, 4)}}))
open(sys.argv[1], "w").write("\n".join(linhas) + "\n")
PY
mkdir -p "$X/pam"
printf '#!/bin/sh\nread -r senha\n[ "$senha" = "kalango-teste" ]\n' > "$X/pam/confere.sh"; chmod +x "$X/pam/confere.sh"
printf 'auth required pam_exec.so expose_authtok quiet %s\naccount required pam_permit.so\n' "$X/pam/confere.sh" > "$X/pam/kalango-bloqueio"
/usr/bin/python3 "$ROOT/sessao/ferramentas/clima-teste.py" 18766 "$OUT/clima.log" &
CLIMA=$!
mkdir -p "$X/area"
# the App Center's catalog, shelf and models (§104): the test's own, nothing from the network
/usr/bin/python3 "$ROOT/sessao/ferramentas/loja-teste.py" "$OUT/loja" >/dev/null
# another system "on the disk" for the first use: an Ubuntu with this person's folders and a Firefox profile
OUTRO="$OUT/outro-sistema"; EU=$(id -un)
mkdir -p "$OUTRO/etc" "$OUTRO/home/$EU/Documentos" "$OUTRO/home/$EU/Imagens" "$OUTRO/home/$EU/.mozilla/firefox"
printf 'NAME="Ubuntu"\nVERSION="24.04"\n' > "$OUTRO/etc/os-release"; echo "carta" > "$OUTRO/home/$EU/Documentos/carta.txt"
printf '[Profile0]\nPath=abc.default\n' > "$OUTRO/home/$EU/.mozilla/firefox/profiles.ini"
env KALANGO_POLKIT_BUS=session KALANGO_POLKIT_HELPER="$ROOT/sessao/ferramentas/polkit-helper-teste" KALANGO_POLKIT_LOG="$OUT/polkit/polkit.log" KALANGO_SELETOR_INICIO="$OUT/seletor" XDG_CURRENT_DESKTOP=KalangoOS XDG_DESKTOP_PORTAL_DIR="$ROOT/sessao/dist/share/xdg-desktop-portal/portals" XDG_CONFIG_DIRS="$ROOT/sessao/dist/etc/xdg:/etc/xdg" GDK_DEBUG=portals GTK_USE_PORTAL=1 KALANGO_MIDIA_TESTE="$OUT/midia" KALANGO_MIDIA_DESTINO="$X/area/Imagens" KALANGO_BLUEZ_BUS=session KALANGO_NM_BUS=session KALANGO_DISCO_TESTE="$OUT/disco-livre" KALANGO_FLATPAK="$ROOT/sessao/ferramentas/flatpak-teste" KALANGO_FLATPAK_ESTADO="$OUT/flatpak.json" KALANGO_FLATPAK_LOG="$OUT/flatpak.log" KALANGO_LOJA_APPSTREAM="$OUT/loja/appstream" KALANGO_LOJA_DADOS="$OUT/loja/dados" KALANGOD_IMPORTAR_RAIZ="$OUTRO" KALANGO_PACTL="$ROOT/sessao/ferramentas/pactl-teste" KALANGO_PACTL_LOG="$OUT/pactl.log" KALANGO_PACTL_ESTADO="$OUT/pactl.estado" DISPLAY="$DISP" WAYLAND_DISPLAY= XMODIFIERS=@im=none KALANGO_SHELL_DEBUG=1 KALANGO_TESTE_COPIA_LENTA=8000000 KALANGO_MEMORIA_CHAVEIRO=nao KALANGO_COFRE=arquivo KALANGO_AREA_TESTE="$X/area" KALANGO_PACKAGEKIT_BUS=session KALANGO_NMCLI="$ROOT/sessao/ferramentas/nmcli-teste.py" KALANGO_NMCLI_ESTADO="$OUT/nmcli.json" KALANGO_BLUETOOTHCTL="$ROOT/sessao/ferramentas/bluetoothctl-teste.py" KALANGO_BLUETOOTHCTL_ESTADO="$OUT/bluetoothctl.json" KALANGO_GST="$ROOT/sessao/ferramentas/gst-teste" KALANGO_GST_LOG="$OUT/gst.log" KALANGO_CAPTURA_DESTINO="$OUT/capturas" KALANGO_PW_DUMP="$ROOT/sessao/ferramentas/pw-dump-teste" KALANGO_WPCTL="$ROOT/sessao/ferramentas/wpctl-teste" KALANGO_PW_CLI="$ROOT/sessao/ferramentas/pw-cli-teste" KALANGO_PW_RECORD="$ROOT/sessao/ferramentas/pw-record-teste" KALANGO_PW_ESTADO="$OUT/pipewire.json" KALANGO_PW_FILTRO="$ROOT/sessao/ferramentas/pipewire-teste" KALANGO_SONS=nao KALANGO_POWERPROFILESCTL="$ROOT/sessao/ferramentas/powerprofilesctl-teste" KALANGO_UPOWER="$ROOT/sessao/ferramentas/upower-teste" KALANGO_ENERGIA_ESTADO="$OUT/energia.json" KALANGO_POWER_SUPPLY="$OUT/power_supply" KALANGO_ACPI_LID="$OUT/lid" KALANGO_MINUTO_MS=4000 KALANGO_CLIMA_URL=http://127.0.0.1:18766 KALANGOD_BUSCA_URL=http://127.0.0.1:18766/search KALANGO_MUSICA="$X/area/Música" KALANGO_DISCOS_TESTE="$OUT/discos.json" KALANGO_DISCOS_LOG="$OUT/discos.log" KALANGO_DISCOS_TESTE_STATS="$OUT/discos.stats" KALANGO_LPSTAT="$ROOT/sessao/ferramentas/lpstat-teste" KALANGO_LP="$ROOT/sessao/ferramentas/lp-teste" KALANGO_LP_LOG="$OUT/impressao.log" KALANGO_PAM_DIR="$X/pam" \
    XDG_DATA_HOME="$X/data" XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_CACHE_HOME="$X/cache" \
    KALANGO_MODELO="$MODELO" KALANGO_MODELOS="$MODELOS" KALANGO_GRAVADOR="$ROOT/sessao/ferramentas/gravador-teste.sh" KALANGO_TESTE_FALA="$FALA" \
    __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 \
    ./sessao/rodar.sh > "$OUT/rodar.log" 2>&1 &
for _ in $(seq 90); do grep -q "sessão de teste em" "$OUT/rodar.log" && break; sleep 1; done
T=$(grep -o "/tmp/kalango-sessao-[A-Za-z0-9]*" "$OUT/rodar.log" | head -1)
for _ in $(seq 90); do grep -q pronto "$T/shell.log" 2>/dev/null && break; sleep 1; done
SOCK=$(grep -ao "niri.wayland-[0-9]*\.[0-9]*\.sock" "$T/niri.log" | head -1)
export NIRI_SOCKET="$XDG_RUNTIME_DIR/$SOCK" DISPLAY="$DISP"
W=$(xdotool search --name "^niri$" | head -1)
xdotool windowfocus --sync "$W"
xdotool mousemove --window "$W" 640 400
sleep 0.5

# capture of the desktop with its widgets (the day summary needs the model: give it time)
if [ -n "${KALANGO_TESTE_CAPTURA:-}" ]; then
  sleep 25
  "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-area.png" >/dev/null
  sleep 0.5
fi

# 1. typing in the command bar (Alt is Mod in a nested niri)
xdotool key --window "$W" alt+space
sleep 1.5
xdotool type --window "$W" --delay 80 term
sleep 1.5
ok "digitar na barra de comando chega ao campo" 'grep -aq "debug busca: \"term\"" "$T/shell.log"'
# every text button with a background or a border has room on its sides (the sheet unsets every button's padding)
ok "nenhum botão de texto sem espaço nas laterais" 'python3 "$ROOT/sessao/ferramentas/auditar-botoes.py"'
# small fixed boxes (an icon's tile, a dot) aligned in their rows: unaligned, a row stretches them into squeezed pills
ok "nenhum ícone ou marcador esticado pela linha (caixas pequenas alinhadas)" 'python3 "$ROOT/sessao/ferramentas/auditar-caixas.py"'
xdotool key --window "$W" Escape
sleep 1

# the apps the tests open: KalangoOS's own (never another system's), windows of the running shell
abrir_calc() { "$N" msg action spawn -- "$T/kalango-shell" calculadora; }
abrir_ed() { "$N" msg action spawn -- "$T/kalango-shell" editor "$@"; }
# where an app's window is on the screen, and where a widget is in its window (the shell logs it)
janela_app() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]==sys.argv[1]][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))' "$1"; }
lugar() { grep -a "debug central: $1 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
# a banner (niri's "Screenshot captured" after each capture) lies over the top of the window for 6 s: wait it out
sem_avisos() { for _ in $(seq 40); do [ "$(grep -a "debug aviso: [0-9]* na tela" "$T/shell.log" | tail -1 | grep -o "[0-9]* na tela")" = "0 na tela" ] && return 0
  grep -aq "debug aviso: [0-9]* na tela" "$T/shell.log" || return 0; sleep 0.25; done; }
# where the Editor's bar is free to drag the window by (it logs it at every size)
barra_ed() { grep -a "debug editor: barra livre em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
id_da_janela() { "$N" msg -j windows | python3 -c 'import json,sys; print(([w["id"] for w in json.load(sys.stdin) if w["app_id"]==sys.argv[1]] or [""])[0])' "$1"; }

# 2. an app's window has its title bar and the X closes it (the Calculator: its own bar, as drawn)
X0=$(grep -ac "debug central: janela fechar em" "$T/shell.log")
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && [ "$(grep -ac "debug central: janela fechar em" "$T/shell.log")" -gt "$X0" ] && break; sleep 0.5; done
sleep 1
read -r OW _ < <(xdotool getwindowgeometry "$W" | awk '/Geometry/ {split($2, a, "x"); print a[1], a[2]}')
read -r JX JY < <(janela_app os.kalango.Calculadora); read -r XX XY < <(lugar "janela fechar")
xdotool mousemove --window "$W" $((JX + ${XX:-0})) $((JY + ${XY:-0})) click 1
sleep 2
ok "o X da janela fecha o app" '! "$N" msg -j windows | grep -q "\"os.kalango.Calculadora\""'

# 3. cursor theme set for the session
ok "o cursor da sessão tem tema" 'grep -q "xcursor-theme \"[A-Za-z]" "$T/niri.kdl"'

# agents of this test session only (not one the person may be using in another session)
agente_desta_sessao() {
  for p in $(pgrep -f "[k]alango-agente --json"); do
    tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && return 0
  done
  return 1
}

# 4. agent conversation in the side bar (real local model): type, send with Enter, the steps start, the stop
#    button ends the engine process
xdotool key --window "$W" alt+g
sleep 2
read -r OW OH < <(xdotool getwindowgeometry "$W" | awk '/Geometry/ {split($2, a, "x"); print a[1], a[2]}')
# the bar ends above the dock's reserved area: composer and stop button sit 119 px above the bottom
xdotool mousemove --window "$W" $((OW - 200)) $((OH - 150)) click 1
sleep 0.5
xdotool type --window "$W" --delay 40 "Liste os processos e o uso dos discos e faça um relatório"
xdotool key --window "$W" Return
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.2; done
ok "Enter envia a mensagem e o agente começa" 'agente_desta_sessao'
# the power menu shows the agent at work (and can wait for it before shutting down)
xdotool key --window "$W" alt+shift+e
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-energia.png" >/dev/null && sleep 0.5
ok "o menu de energia mostra o agente em execução" 'grep -aq "debug energia: agente em execução" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 1
xdotool mousemove --window "$W" $((OW - 49)) $((OH - 119)) click 1
sleep 1
for _ in $(seq 25); do agente_desta_sessao || break; sleep 0.2; done
ok "o botão parar encerra o agente" 'grep -aq "debug conversa: botão" "$T/shell.log" && ! agente_desta_sessao'

# 4c. "wait for it to finish": shutting down waits while the agent works and happens when it ends
xdotool mousemove --window "$W" $((OW - 200)) $((OH - 150)) click 1
sleep 0.5
xdotool type --window "$W" --delay 40 "Liste os processos e o uso dos discos e faça um relatório"
xdotool key --window "$W" Return
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.2; done
xdotool key --window "$W" alt+shift+e
sleep 1.5
# turn "wait" on (click the switch), then Enter: Shut down is the default
read -r SWX SWY < <(grep -a "debug energia: interruptor" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "${SWX:-0}" "${SWY:-0}" click 1
sleep 0.6
xdotool key --window "$W" Return
sleep 2.5
ok "com \"esperar\" ligado, desligar espera o agente" 'grep -aq "debug energia: esperando" "$T/shell.log" && ! grep -aq "teste\] .desligar." "$T/shell.log"'
for p in $(pgrep -f "[k]alango-agente --json"); do
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && kill "$p"
done
for _ in $(seq 40); do grep -aq "teste\] .desligar." "$T/shell.log" && break; sleep 0.25; done
ok "quando o agente termina, o desligar acontece" 'grep -aq "teste\] .desligar." "$T/shell.log"'

# 5. a modifier held while the session window loses focus is released (it used to stay stuck: typing
#    turned into shortcuts and clicks into Alt+drag)
xdotool windowfocus --sync "$W"
xdotool keydown --window "$W" alt
sleep 0.3
# the host grabs the keyboard (like GNOME's Alt+Tab switcher) and Alt is released during the grab:
# the release never reaches the session window
GRAB="$OUT/agarrar-teclado"
gcc -O2 -o "$GRAB" sessao/ferramentas/agarrar-teclado.c -lX11
"$GRAB" 1200 >/dev/null &
GP=$!
sleep 0.4
xdotool keyup alt
wait $GP
xdotool windowfocus --sync "$W"
sleep 0.5
xdotool key --window "$W" alt+space
sleep 1.5
xdotool type --window "$W" --delay 80 xyz
sleep 1
ok "Alt preso ao perder o foco é solto (digitar volta a funcionar)" 'grep -aq "debug busca: \"xyz\"" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.8

# 6. window snapping: dragging a window by its title bar to the left edge snaps it to the left half;
#    Super+Up from there makes it the top-left quarter
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2.5
read -r OW OH < <(xdotool getwindowgeometry "$W" | awk '/Geometry/ {split($2, a, "x"); print a[1], a[2]}')
geo() { "$N" msg -j windows | python3 -c '
import json, sys
w = json.load(sys.stdin)[0]; l = w["layout"]; p = l.get("tile_pos_in_workspace_view") or [0, 0]
print(int(w["is_floating"]), l["window_size"][0], l["window_size"][1], int(p[0]), int(p[1]))'; }
WA=$("$N" msg -j windows | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["layout"]["tile_pos_in_workspace_view"])')
# the Editor's bar (it opens floating, as drawn): the free stretch between its tabs and "Pedir ao agente", with no
# notice over it
sem_avisos
read -r EX0 EY0 < <(janela_app os.kalango.Editor); read -r BX BY < <(barra_ed)
xdotool mousemove --window "$W" $((EX0 + ${BX:-300})) $((EY0 + ${BY:-22})) mousedown 1
for x in 200 160 120 80 40 10 2 0; do xdotool mousemove --window "$W" $x 300; sleep 0.08; done
sleep 0.6
xdotool mouseup 1
sleep 1.5
read -r FL GW GH GX GY < <(geo)
echo "  (encaixe à esquerda: flutuante=$FL tamanho=${GW}x${GH} posição=${GX},${GY}; saída ${OW}x${OH})"
# (no gaps: niri.kdl's "gaps 0" — the two halves make the whole width)
ok "arrastar até a borda esquerda encaixa na metade" '[ "$FL" = 1 ] && [ "$GX" -le 14 ] && [ $((GW * 2)) -ge $((OW - 60)) ] && [ $((GW * 2)) -le $((OW + 2)) ] && [ "$GH" -ge $((OH / 2)) ]'
xdotool key --window "$W" alt+Up
sleep 1.5
read -r FL QW QH QX QY < <(geo)
echo "  (Super+↑: tamanho=${QW}x${QH} posição=${QX},${QY})"
# (the width as the half's, within niri's rounding: half of 1245 px is 622 by a key, 623 by a drag)
ok "Super+↑ a partir da metade dá o quarto de cima" '[ $((QW - GW)) -le 2 ] && [ $((GW - QW)) -le 2 ] && [ $((QH * 2)) -le $((GH + 20)) ] && [ "$QY" -le "$GY" ]'
xdotool key --window "$W" alt+Down
sleep 1
xdotool key --window "$W" alt+Down
sleep 1.5
read -r FL RW RH RX RY < <(geo)
ok "Super+↓ duas vezes volta ao tamanho de antes do encaixe" '[ "$RW" != "$GW" ] || [ "$RH" != "$GH" ]'

# 7. layout picker (Super+Z): clicking the left region of "Halves" snaps the focused window there
xdotool key --window "$W" alt+z
sleep 1.5
read -r PX PY < <(grep -a "debug encaixe área" "$T/shell.log" | tail -1 | awk '{print $4, $5}')
xdotool mousemove --window "$W" $PX $PY click 1
sleep 1.5
read -r FL SW SH SX SY < <(geo)
echo "  (seletor → metade esquerda: tamanho=${SW}x${SH} posição=${SX},${SY})"
ok "o seletor de layouts (Super+Z) encaixa a janela na área clicada" '[ "$FL" = 1 ] && [ "$SX" -le 14 ] && [ $((SW * 2)) -ge $((OW - 60)) ] && [ $((SW * 2)) -le $((OW + 2)) ]'

# 8. picker during a drag: dragging the editor near the top bar drops the picker; releasing on the left
#    region of "Halves" puts the editor there and the other window (calculator) in the right half
xdotool key --window "$W" Escape
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 2.5
geo_app() { "$N" msg -j windows | python3 -c '
import json, sys
for w in json.load(sys.stdin):
    if sys.argv[1] in (w.get("app_id") or "").lower():
        l = w["layout"]; p = l.get("tile_pos_in_workspace_view") or [0, 0]
        print(int(w["is_floating"]), l["window_size"][0], l["window_size"][1], int(p[0]), int(p[1])); break' "$1"; }
read -r _ EW EH EX EY < <(geo_app kalango.editor)
"$N" msg action focus-window --id "$("$N" msg -j windows | python3 -c 'import json,sys; print([w["id"] for w in json.load(sys.stdin) if "kalango.editor" in (w.get("app_id") or "").lower()][0])')"
sleep 0.5
sem_avisos
read -r BX BY < <(barra_ed)
xdotool mousemove --window "$W" $((EX + ${BX:-260})) $((EY + ${BY:-20})) mousedown 1
for p in "$((EX + 250)) 200" "$((OW / 2)) 120" "$((OW / 2)) 60" "$((OW / 2)) 40" "$((OW / 2)) 30"; do xdotool mousemove --window "$W" $p; sleep 0.12; done
sleep 0.6
for p in "$((OW / 2 - 100)) 60" "$((PX + 5)) $((PY - 5))" "$PX $PY"; do xdotool mousemove --window "$W" $p; sleep 0.15; done
sleep 0.6
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "$KALANGO_TESTE_CAPTURA" >/dev/null && sleep 0.5
xdotool mouseup 1
sleep 2
read -r EF EW EH EX EY < <(geo_app kalango.editor)
read -r CF CW CH CX CY < <(geo_app kalango.calculadora)
echo "  (arraste até o seletor: editor ${EW}x${EH} em ${EX},${EY}; calculadora ${CW}x${CH} em ${CX},${CY})"
ok "arrastar até o seletor encaixa a janela e ajusta a outra no mesmo layout" '[ "$EX" -le 14 ] && [ $((EW * 2)) -ge $((OW - 60)) ] && [ "$CX" -ge $((OW / 2)) ] && [ "$CX" -le $((OW / 2 + 14)) ]'

# 8b. the agent's suggestion in the picker (design "Sugestão do agente"): the drop above was kept as an arrangement;
#     used once more (written here, an hour ago) it is the one used most with these apps, and the picker offers it
#     under the layouts; with the windows swapped, "Aplicar" puts the editor back on the left, the calculator right
python3 - "$X/state/kalango/arranjos.json" <<'PYEOF'
import json, sys, time
p = sys.argv[1]
try: v = json.load(open(p))
except Exception: v = []
if v: v.append(dict(v[-1], t=int(time.time()) - 3600))
json.dump(v, open(p, "w"))
PYEOF
id_app() { "$N" msg -j windows | python3 -c 'import json,sys; print([w["id"] for w in json.load(sys.stdin) if sys.argv[1] in (w.get("app_id") or "").lower()][0])' "$1"; }
"$N" msg action snap-window --id "$(id_app kalango.calculadora)" left-half
"$N" msg action snap-window --id "$(id_app kalango.editor)" right-half
sleep 1.5
A0=$(grep -ac "debug encaixe aplicar" "$T/shell.log")
xdotool key --window "$W" alt+z
for _ in $(seq 20); do [ "$(grep -ac "debug encaixe aplicar" "$T/shell.log")" -gt "$A0" ] && break; sleep 0.3; done
grep -a "debug layouts: sugestão “" "$T/shell.log" | tail -1 | sed 's/^debug layouts: /  (/; s/$/)/'
read -r AX AY < <(grep -a "debug encaixe aplicar" "$T/shell.log" | tail -1 | awk '{print $4, $5}')
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-sugestao-layout.png" >/dev/null && sleep 0.5
xdotool mousemove --window "$W" "${AX:-0}" "${AY:-0}" click 1
sleep 2
read -r EF EW EH EX EY < <(geo_app kalango.editor)
read -r CF CW CH CX CY < <(geo_app kalango.calculadora)
echo "  (sugestão aplicada: editor ${EW}x${EH} em ${EX},${EY}; calculadora ${CW}x${CH} em ${CX},${CY})"
ok "o seletor sugere o arranjo mais usado com estes apps e \"Aplicar\" o refaz" '[ "$(grep -ac "debug encaixe aplicar" "$T/shell.log")" -gt "$A0" ] && grep -aq "debug layouts: sugestão aplicada" "$T/shell.log" && [ "$EX" -le 14 ] && [ "$CX" -ge $((OW / 2)) ] && [ "$CX" -le $((OW / 2 + 14)) ]'

# 9. dictation: the microphone records, the second click stops it, the local voice model writes the
#    sentence in the composer (the side bar from item 4 is still open in Agent mode)
xdotool key --window "$W" alt+g
sleep 1.5
xdotool key --window "$W" alt+g
sleep 1.5
xdotool mousemove --window "$W" $((OW - 85)) $((OH - 119)) click 1
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-voz.png" >/dev/null
xdotool mousemove --window "$W" $((OW - 85)) $((OH - 119)) click 1
for _ in $(seq 60); do grep -aq "debug voz: \(Ok\|Err\)" "$T/shell.log" && break; sleep 0.5; done
grep -a "debug voz" "$T/shell.log" | tail -2 | sed 's/^/  (/; s/$/)/'
ok "o microfone grava e o modelo de voz local escreve a fala no compositor" 'grep -a "debug voz" "$T/shell.log" | tail -1 | grep -i "livre" | grep -qi "disco principal"'

# 10. opening the agent permissions (settings button) changes nothing by itself
# 4b. the power menu shows the agent at work while it runs
# 4c. with "wait for it to finish" on, shutting down waits for the agent and happens when it ends
# 11. the command bar takes the focused window as context ("Janela: <app>") and searches files by name
# 12. the power menu offers an update prepared by PackageKit and Enter shuts down with it
# 13. a notification banner, under the island, with "Mark as read"
# 14. the system approval screen: "Always" approves, and the same request does not ask again
# 15. Controls: pause agents, night light, and the pin (windows give the bar room)
# 16. Notices: the local model sums up the unread notices in three points
# 17. "Hoje": local calendar, weather (only the city's coordinates go out), focus ring, editing the widgets
# 18. desktop: "Agora" (ordered by the local model) and "Pergunte ao KalangoOS"
# 19. Alt+Tab: the switcher with window pictures; letting go of Alt goes to the window; a quick tap switches directly
# 20. app grid: search + Enter opens the app, and the launch is recorded for the suggestions
# 21. lock screen: locks through the compositor; a wrong password is refused, the right one unlocks
# 22. overview (Mod+O; Alt in the nested session): pictures of the windows, drag to a new workspace, rename, go to a window, the bar above
# 23. widgets: edit mode, drag to the grid, add from the gallery, remove, Esc saves the layout
# 24. login screen (greetd greeter): choose a person, a wrong password is refused, the right one starts the session
# 25. agents center: its own app (os.kalango.Agentes) with the tasks that ended
# 26. agents center › permissions: a click on a cell changes the column; paused, the agent refuses new work
# 27. agents center › memory: "lembre que…" in its field is remembered and shown; the trash forgets it
# 28. agents center › routines: one made in the form runs by itself at its time, with the real agent
# 29. agents center › timeline: the agent's file move is shown as reversible and its "Desfazer" puts the file back
# 30. agents center › usage: the session's real counts; the suggested daily limit, applied, is enforced by the agent
# 31. agents center › settings › connectors: an MCP connector added shows up connected and the agent uses it
# 32. a connector that fails is diagnosed (and its fix goes through kalangod); one is added from the catalog, tested first
# 33. conversations: started in the composer with the real agent, kept, listed and opened again from its events
# 34. failures: one card each with a real fix — "raise only today" and "allow this folder", and the request goes on
# 35. approval notice: the agent asks while its conversation is off screen; the island's "Permitir" answers it
# 36. Files: copies, previews, trash and Ctrl+Z, tags and folders by dropping, the agent organising the folder
# 37. a long copy: the island's card pauses, resumes and cancels; the notice opens the folder it went to
# 38. Settings: compact here with the system's data; updates tonight; the full layout; the night light's schedule
# 39. Settings › Appearance: light, accent, dock at the side, widgets on cards, live and kept — and back
# 40. Settings › Network (a stand-in NetworkManager): the cards, metered, agents kept off (kalangod refuses), password, forget
# 41. Settings › Bluetooth (a stand-in BlueZ): the person's devices, a scan finds one nearby, pair, disconnect, forget
# 42. Settings › Display: the session's monitor from niri; a new scale goes back by itself unless kept; kept, it is saved
# 43. Settings › Sound (stand-in PipeWire): devices as cards, the meter, a new default, the volume, kept behaviour
# 44. Settings › Power (stand-in power-profiles-daemon and battery): the mode, the day's draw, the idle times dark the
#     screen and suspend, agents keep the computer awake, charge limit / lid / graphics card through kalangod
# 45. Settings › Notifications: counts and apps, an app off, "only in the digest" and the agent's digest (local model),
#     an important person breaks through, Do not disturb, the banners' corner
# 46. Clipboard history (Super+V): copies kept with their app, a password manager's secret never; ↵ pastes the
#     chosen one into the window in use; "Colar como… Texto limpo"; Ctrl+P pins
botao() { grep -a "debug compositor botão $1:" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
read -r AJX AJY < <(botao 2)
xdotool mousemove --window "$W" "$AJX" $((AJY + 40)) click 1
sleep 2.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-permissoes.png" >/dev/null
xdotool key --window "$W" Escape
sleep 0.5
ok "abrir as permissões não muda nenhuma permissão" 'grep -aq "debug permissões abertas" "$T/shell.log" && [ ! -e "$X/config/kalango/agente-permissoes.json" ]'

# 12. power menu with an update prepared (PackageKit stand-in on the session's bus): it offers to install it and
#     Enter shuts down with the update
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/packagekit-teste.py" 2
sleep 1.5
xdotool key --window "$W" alt+shift+e
sleep 2
ok "o menu de energia oferece a atualização preparada e Enter desliga com ela" 'grep -aq "debug energia: atualização preparada (2)" "$T/shell.log" && { xdotool key --window "$W" Return; sleep 2; grep -aq "teste\] .desligar. (com a atualização preparada)" "$T/shell.log"; }'
pkill -f "[p]ackagekit-teste.py 2"

# 13. notification banner: an app's notification shows centered under the island with "Mark as read", which
#     closes it and keeps it in the list as read
"$N" msg action spawn -- notify-send -a Teste -A responder=Responder "Revisão pronta" "Terminei a revisão: 3 sugestões."
sleep 1.8
read -r LX LY < <(grep -a "debug aviso: marcar como lida em .* :: Revisão pronta" "$T/shell.log" | tail -1 | awk '{print $7, $8}')
# the pointer enters the banner first (a click in the same instant can arrive before the enter)
xdotool mousemove --window "$W" "${LX:-0}" "${LY:-0}"
sleep 0.4
xdotool click 1
sleep 1
ok "o aviso tem \"Marcar como lida\", que o fecha e guarda como lido" 'grep -aq "debug aviso: marcada como lida: Revisão pronta" "$T/shell.log"'
# notify-send -A ends when its notice closes; if not, only this session's one is stopped
for p in $(pgrep -x notify-send); do
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "NIRI_SOCKET=$NIRI_SOCKET" && kill "$p"
done

# 14. system approval screen: a system change from an agent asks; "Always" approves it, the same request does not ask
#     again (it is simulated: kalangod runs in dry-run in the test session)
CHAMAR="$ROOT/sessao/ferramentas/mcp-chamar.py"
python3 "$CHAMAR" "$T/mcp.sock" pacotes.instalar '{"nomes": ["cowsay"]}' > "$OUT/sempre1.json" 2>&1 &
CP=$!
for _ in $(seq 60); do grep -aq "debug aprovação: sempre em" "$T/aprovacao.log" && break; sleep 0.25; done
sleep 0.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-aprovacao.png" >/dev/null && sleep 0.3
read -r SX SY < <(grep -a "debug aprovação: sempre em" "$T/aprovacao.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "${SX:-0}" "${SY:-0}"
sleep 0.4
xdotool click 1
for _ in $(seq 40); do kill -0 $CP 2>/dev/null || break; sleep 0.25; done
ok "a tela de aprovação aparece e \"Sempre\" aprova a mudança" 'grep -q "\"isError\": false" "$OUT/sempre1.json"'
ANTES=$(grep -ac "debug aprovação: sempre em" "$T/aprovacao.log")
timeout 10 python3 "$CHAMAR" "$T/mcp.sock" pacotes.instalar '{"nomes": ["cowsay"]}' > "$OUT/sempre2.json" 2>&1
sleep 1
ok "o mesmo pedido de novo não pergunta" 'grep -q "\"isError\": false" "$OUT/sempre2.json" && [ "$(grep -ac "debug aprovação: sempre em" "$T/aprovacao.log")" = "$ANTES" ]'
kill $CP 2>/dev/null

# 15. Controls: "Pause agents" freezes the agent at work and resumes it; "Night light" starts and stops wlsunset
#     (the compositor's gamma; the nested session has no real gamma, the process is what the shell controls);
#     the pin makes the windows give the bar room
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2
# (the Editor opens floating, as drawn; put in the row of windows and maximized, it is the one that makes room)
"$N" msg action toggle-window-floating --id "$(id_da_janela os.kalango.Editor)"
sleep 1
"$N" msg action focus-window --id "$(id_da_janela os.kalango.Editor)"
"$N" msg action maximize-column
sleep 1.5
largura() { "$N" msg -j windows | python3 -c 'import json,sys; print(max(w["layout"]["window_size"][0] for w in json.load(sys.stdin)))'; }
L0=$(largura)
"$N" msg action spawn -- "$T/kalango-shell" lateral agente abrir
sleep 2
xdotool mousemove --window "$W" $((OW - 200)) $((OH - 150)) click 1
sleep 0.5
xdotool type --window "$W" --delay 40 "Liste os processos e o uso dos discos e faça um relatório"
xdotool key --window "$W" Return
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.2; done
"$N" msg action spawn -- "$T/kalango-shell" lateral controles
sleep 2.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-controles.png" >/dev/null && sleep 0.3
# moves the pointer with a real motion (a pixel off first): after a popover closes niri keeps the pointer's old surface
# until the next motion, and a move to where the X pointer already is sends none (the click then lands on the old one)
apontar() { xdotool mousemove --window "$W" $(( $1 + 2 )) $(( $2 + 2 )); sleep 0.08; xdotool mousemove --window "$W" "$1" "$2"; }
pos() { grep -a "debug controles: $1 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
# closes the windows of the apps the tests open (the Calculator, the Text Editor), by id, and waits for them to go: a
# window left over (floating, focus following the mouse) would cover the next tests' windows and take their clicks.
# The Editor with a test's text asks "Salvar as alterações?" inside its window: "Não salvar"
fechar_apps() { local ids; for _ in 1 2 3 4 5; do
  ids=$("$N" msg -j windows | python3 -c 'import json,sys; print(" ".join(str(w["id"]) for w in json.load(sys.stdin) if (w.get("app_id") or "") in ("os.kalango.Calculadora", "os.kalango.Editor", "os.kalango.Terminal", "os.kalango.Documentos", "os.kalango.Imagens", "os.kalango.Musica", "os.kalango.Video", "os.kalango.Calendario", "os.kalango.Discos", "os.kalango.Ajuda", "os.kalango.Imprimir")))')
  [ -z "$ids" ] && return 0; for i in $ids; do "$N" msg action close-window --id "$i"; done; sleep 1
  if "$N" msg -j windows | grep -q '"os.kalango.Editor"'; then
    read -r JX JY < <(janela_app os.kalango.Editor); read -r CX CY < <(lugar "editor descartar")
    apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1; sleep 1
  fi
  # the Terminal with a command at work asks "Fechar a aba?": "Fechar a aba"
  if "$N" msg -j windows | grep -q '"os.kalango.Terminal"'; then
    read -r JX JY < <(janela_app os.kalango.Terminal); read -r CX CY < <(lugar "terminal fechar aba")
    apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1; sleep 1
  fi
done; }
clicar() { apontar "${1:-0}" "${2:-0}"; sleep 0.3; xdotool click 1; sleep 1.2; }
estado_agente() {
  for p in $(pgrep -f "[k]alango-agente --json"); do
    tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && { awk '{print $3}' /proc/$p/stat; return; }
  done
  echo nenhum
}
read -r PX PY < <(pos "Pausar agentes")
echo "  (agente antes de pausar: $(estado_agente); bloco em $PX,$PY)"
clicar "$PX" "$PY"
echo "  (agente depois: $(estado_agente))"
ok "\"Pausar agentes\" congela o agente em execução" '[ "$(estado_agente)" = T ]'
clicar "$PX" "$PY"
ok "e retomar o descongela" '[ "$(estado_agente)" != T ] && [ "$(estado_agente)" != nenhum ]'
noturna() {
  for p in $(pgrep -x wlsunset); do
    tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "NIRI_SOCKET=$NIRI_SOCKET" && return 0
  done
  return 1
}
read -r NX NY < <(pos "Luz noturna")
clicar "$NX" "$NY"
# on real screens wlsunset keeps running; the nested session has no gamma control, and the tile must say so
ok "\"Luz noturna\" liga a luz quente, ou diz que esta tela não tem controle de gama" 'noturna || grep -aq "debug controles: luz noturna indisponível" "$T/shell.log"'
noturna && clicar "$NX" "$NY"
ok "e nada fica rodando depois de desligar" '! noturna'
read -r FX FY < <(pos fixar)
clicar "$FX" "$FY"
sleep 1
L1=$(largura)
echo "  (janela maximizada: ${L0} px; com a barra fixada: ${L1} px)"
ok "fixar a barra faz as janelas abrirem espaço para ela" '[ "$L1" -le $((L0 - 300)) ]'
xdotool key --window "$W" Escape
sleep 1.5
L2=$(largura)
ok "fechar a barra a solta e as janelas voltam ao tamanho" '[ "$L2" -ge $((L0 - 10)) ]'
for p in $(pgrep -f "[k]alango-agente --json"); do
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && kill -CONT "$p" && kill "$p"
done
fechar_apps

# 16. Notices: with several unread notices, the local model sums them up in three points at the top of the list
for t in "Fatura de energia|Sua fatura de R\$ 182,40 vence em 20/11." "Organização concluída|118 arquivos movidos para 4 pastas." "Marina|Você pode mandar a pauta da reunião das 15:00?" "Roo|viu o novo tema?"; do
  "$N" msg action spawn -- notify-send -a Teste "${t%%|*}" "${t#*|}"
  sleep 0.4
done
sleep 1
"$N" msg action spawn -- "$T/kalango-shell" lateral avisos abrir
for _ in $(seq 120); do grep -aq "debug avisos: resumo" "$T/shell.log" && break; sleep 0.5; done
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-avisos.png" >/dev/null && sleep 0.3
ok "os avisos não lidos são resumidos em 3 pontos pelo modelo local" 'grep -aq "debug avisos: resumo do modelo local (3 pontos)" "$T/shell.log"'
# the notification centre takes the whole right side (flush with the edge, under the top bar): its quick buttons at
# the bottom work from there — Do not disturb on and off again
rapido_avisos() { read -r QX QY < <(grep -a "debug central: avisos rapido $1 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'); xdotool mousemove --window "$W" $((OW - 400 + QX)) $((32 + QY)); sleep 0.3; xdotool click 1; sleep 1; }
sleep 0.8
for _try in 1 2 3; do
  rapido_avisos "Não perturbe"
  for _ in $(seq 10); do grep -aq "debug avisos: rápido Não perturbe → true" "$T/shell.log" && break; sleep 0.3; done
  grep -aq "debug avisos: rápido Não perturbe → true" "$T/shell.log" && break
done
ok "a central de avisos ocupa a lateral inteira e o botão rápido liga o Não perturbe" 'grep -aq "debug avisos: rápido Não perturbe → true" "$T/shell.log"'
for _try in 1 2 3; do
  rapido_avisos "Não perturbe"
  for _ in $(seq 10); do grep -aq "debug avisos: rápido Não perturbe → false" "$T/shell.log" && break; sleep 0.3; done
  grep -aq "debug avisos: rápido Não perturbe → false" "$T/shell.log" && break
done
"$N" msg action spawn -- "$T/kalango-shell" lateral avisos
sleep 1

# 17. "Hoje" bar: today's events from the local calendar; the weather (only the city's coordinates go out); the focus
#     ring starts with a click; "Edit widgets" + × hides a widget and the choice is kept
"$N" msg action spawn -- "$T/kalango-shell" hoje
for _ in $(seq 40); do grep -aq "debug hoje: clima lido" "$T/shell.log" && break; sleep 0.25; done
sleep 1.2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-hoje.png" >/dev/null && sleep 0.3
ok "o Hoje mostra os 3 eventos de hoje da agenda local" 'grep -aq "debug hoje: agenda com 3 eventos" "$T/shell.log"'
ok "o clima é lido só com as coordenadas da cidade escolhida" 'grep -aq "debug hoje: clima lido" "$T/shell.log" && grep -q "latitude=-23.550&longitude=-46.630" "$OUT/clima.log" && ! grep -qv "^/v1/forecast" "$OUT/clima.log"'
hpos() { grep -a "debug hoje: $1 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
read -r HX HY < <(hpos foco)
clicar "$HX" "$HY"
ok "clicar no anel do foco começa o cronômetro" 'grep -aq "debug hoje: foco rodando=true" "$T/shell.log"'
# (a double click resets it: a focus session left running would hold the notices of the sections below)
xdotool mousemove --window "$W" "$HX" "$HY" click --repeat 2 --delay 90 1
for _ in $(seq 20); do grep -aq "debug foco: encerrado" "$T/shell.log" && break; sleep 0.3; done
ok "e um clique duplo zera o cronômetro (a sessão de foco termina)" 'grep -aq "debug foco: começou" "$T/shell.log" && grep -aq "debug foco: encerrado" "$T/shell.log"'
read -r EX EY < <(hpos editar)
clicar "$EX" "$EY"
sleep 1
read -r RX RY < <(hpos "remover agenda")
clicar "$RX" "$RY"
ok "\"Editar widgets\" tira um widget e a escolha fica guardada" 'grep -aq "debug hoje: widget agenda removido" "$T/shell.log" && grep -q agenda "$X/config/kalango/hoje.json"'
"$N" msg action spawn -- "$T/kalango-shell" hoje
sleep 1

# 18. desktop: "Agora" lists what needs the person now (an event in 25 min), ordered by the local model; what is
#     typed in "Pergunte ao KalangoOS" goes to the agent in the side bar
for _ in $(seq 120); do grep -aq "debug agora: .* sugestões ordenadas pelo modelo local" "$T/shell.log" && break; sleep 0.5; done
ok "\"Agora\" mostra a reunião que vem aí, ordenada pelo modelo local" 'grep -aq "debug agora: [1-9][0-9]* sugestões ordenadas pelo modelo local" "$T/shell.log"'
# the desktop widgets sit under the windows: an empty workspace shows them
"$N" msg action focus-workspace-down
sleep 1
read -r QX QY < <(grep -a "debug area: pergunta em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$QX" "$QY"
xdotool type --window "$W" --delay 40 "Quanto espaço livre eu tenho?"
xdotool key --window "$W" Return
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.2; done
ok "o que se digita em \"Pergunte ao KalangoOS\" vai para o agente" 'agente_desta_sessao'
"$N" msg action focus-workspace-up
for p in $(pgrep -f "[k]alango-agente --json"); do
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && kill "$p"
done
sleep 1

# 18b. the Desktop folder's files on the desktop: a folder made there by another app (here, mkdir) shows as an icon at
#      once (gio FileMonitor), a rename keeps it on the desktop with its new name, and deleting it takes the icon away
MESA="$X/area/Área de trabalho"
mkdir -p "$MESA"
L=$(wc -l < "$T/shell.log")
mkdir "$MESA/Pasta do teste"
for _ in $(seq 40); do tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: ícone Pasta do teste em" && break; sleep 0.25; done
ok "uma pasta criada na pasta Área de trabalho aparece na área de trabalho" 'tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: ícone Pasta do teste em"'
N0=$(tail -n +"$L" "$T/shell.log" | grep -ao "debug desktop: [0-9]* ícones" | tail -1 | awk '{print $3}')
L=$(wc -l < "$T/shell.log")
mv "$MESA/Pasta do teste" "$MESA/Pasta renomeada"
for _ in $(seq 40); do tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: ícone Pasta renomeada em" && break; sleep 0.25; done
ok "renomeada por outro app, a pasta continua na área de trabalho com o nome novo" 'tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: ícone Pasta renomeada em" && ! tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: ícone Pasta do teste em"'
L=$(wc -l < "$T/shell.log")
rmdir "$MESA/Pasta renomeada"
for _ in $(seq 40); do tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: $((N0 - 1)) ícones" && break; sleep 0.25; done
ok "apagada, a pasta sai da área de trabalho" 'tail -n +"$L" "$T/shell.log" | grep -aq "debug desktop: $((N0 - 1)) ícones"'

# 19. Alt+Tab: holding Alt shows the windows (with pictures niri makes for us) and letting go goes to the previous
#     one; a quick Alt+Tab switches straight away without showing the panel (two windows of its own: the earlier
#     sections' "Desligar" closes every window)
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 1
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2
foco() { "$N" msg -j focused-window | python3 -c 'import json,sys; w=json.load(sys.stdin); print(w["app_id"] if w else "")'; }
F0=$(foco)
xdotool keydown --window "$W" alt
xdotool key --window "$W" Tab
sleep 1.2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-alternador.png" >/dev/null && sleep 0.3
ok "Alt+Tab segurado mostra as janelas com a miniatura de cada uma" 'grep -aq "debug alternador: aberto com" "$T/shell.log" && grep -aq "debug alternador: miniatura da janela" "$T/shell.log"'
xdotool keyup --window "$W" alt
sleep 1
F1=$(foco)
echo "  (foco antes: $F0; depois de soltar o Alt: $F1)"
ok "soltar o Alt vai para a janela anterior" '[ -n "$F1" ] && [ "$F1" != "$F0" ]'
ABERTOS=$(grep -ac "debug alternador: aberto com" "$T/shell.log")
xdotool keydown --window "$W" alt key --window "$W" Tab keyup --window "$W" alt
sleep 1.2
F2=$(foco)
ok "Alt+Tab rápido troca direto, sem mostrar o painel" '[ "$F2" = "$F0" ] && [ "$(grep -ac "debug alternador: aberto com" "$T/shell.log")" = "$ABERTOS" ]'

# 20. app grid: opens from the dock button / Mod+A; typing finds the app and Enter opens it; the launch is recorded
#     for the suggestions ("você costuma abrir às…")
# (the apps of §19 closed: the Calculator is one window, and the one the grid opens must be new)
fechar_apps
JAN0=$("$N" msg -j windows | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')
"$N" msg action spawn -- "$T/kalango-shell" apps
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-grade.png" >/dev/null && sleep 0.3
xdotool type --window "$W" --delay 60 calcul
xdotool key --window "$W" Return
for _ in $(seq 40); do [ "$("$N" msg -j windows | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')" -gt "$JAN0" ] && break; sleep 0.5; done
ok "na grade, digitar e Enter abre o app (e o uso fica registrado)" '[ "$("$N" msg -j windows | python3 -c "import json,sys; print(len(json.load(sys.stdin)))")" -gt "$JAN0" ] && grep -q "os.kalango.Calculadora" "$X/state/kalango/uso-apps.json"'
"$N" msg action close-window
sleep 1
# the grid in three zones (design "Grade de aplicativos"): a request in the field is answered by the agent's zone —
# the local model says what it means, and this computer's apps and the AppStream catalog answer it
"$N" msg action spawn -- "$T/kalango-shell" apps
sleep 1.5
xdotool type --window "$W" --delay 50 "algo para editar fotos"
for _ in $(seq 120); do grep -aq "debug grade: o agente achou" "$T/shell.log" && break; sleep 0.5; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-grade-agente.png" >/dev/null
echo "  ($(grep -a "debug grade: tela\|debug grade: o agente achou" "$T/shell.log" | tail -2 | tr '\n' ' '))"
ok "na grade, um pedido (\"algo para editar fotos\") é respondido pelo agente com os apps daqui ou do catálogo" \
  'grep -aE "debug grade: o agente achou ([1-9][0-9]* aqui|[0-9]+ aqui, [1-9])" "$T/shell.log" >/dev/null'
xdotool key --window "$W" Escape
sleep 1

# 21. lock screen (the compositor's lock protocol): a key shows the password field; a wrong password is refused
#     and the screen stays locked; the right one unlocks
# (niri refuses screenshots while locked, as it should: these captures photograph the virtual X display instead)
foto_x() { [ -n "${KALANGO_TESTE_CAPTURA:-}" ] && python3 -c "from PIL import ImageGrab; ImageGrab.grab(xdisplay='$DISP').save('${KALANGO_TESTE_CAPTURA%.png}-$1.png')"; true; }
"$N" msg action spawn -- "$T/kalango-shell" bloquear
for _ in $(seq 20); do grep -aq "debug bloqueio: tela bloqueada" "$T/shell.log" && break; sleep 0.5; done
ok "bloquear trava a tela pelo protocolo do compositor" 'grep -aq "debug bloqueio: tela bloqueada" "$T/shell.log"'
sleep 1
foto_x bloqueio
xdotool key --window "$W" space
sleep 1
xdotool type --window "$W" --delay 60 errada
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug bloqueio: senha incorreta" "$T/shell.log" && break; sleep 0.5; done
ok "senha errada é recusada e a tela continua bloqueada" 'grep -aq "debug bloqueio: senha incorreta" "$T/shell.log" && ! grep -aq "debug bloqueio: desbloqueado" "$T/shell.log"'
foto_x bloqueio-senha
# if the shell stops while locked, the compositor keeps the screen locked and the restarted shell locks again
PRONTOS=$(grep -ac "pronto (" "$T/shell.log")
for p in $(pgrep -x kalango-shell); do
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -qx "KALANGOD_SOCKET=$T/mcp.sock" && kill -9 "$p"
done
for _ in $(seq 40); do [ "$(grep -ac "pronto (" "$T/shell.log")" -gt "$PRONTOS" ] && [ "$(grep -ac "debug bloqueio: tela bloqueada" "$T/shell.log")" -ge 2 ] && break; sleep 0.5; done
ok "se o shell cai com a tela bloqueada, ele volta e bloqueia de novo" 'grep -aq "debug bloqueio: retomando o bloqueio" "$T/shell.log" && [ "$(grep -ac "debug bloqueio: tela bloqueada" "$T/shell.log")" -ge 2 ]'
sleep 1
xdotool key --window "$W" space
sleep 1
xdotool type --window "$W" --delay 60 kalango-teste
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug bloqueio: desbloqueado" "$T/shell.log" && break; sleep 0.5; done
sleep 1
abrir_calc
for _ in $(seq 20); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.3; done
sleep 1
ok "a senha certa desbloqueia (as janelas voltam a receber o foco)" 'grep -aq "debug bloqueio: desbloqueado" "$T/shell.log" && "$N" msg focused-window | grep -q "Window ID"'
"$N" msg action close-window
sleep 1

# 22. overview (Mod+O; Alt in the nested session): the workspace's windows as pictures; dragging one onto "+" moves it to a new workspace; a
#     double click renames a workspace; clicking a picture goes to that window and closes it; the top bar stays usable
abrir_calc
sleep 2
abrir_ed
sleep 3
NJ=$("$N" msg -j windows | python3 -c 'import json,sys; ws=[w for w in json.load(sys.stdin)]; print(len(ws))')
xdotool key --window "$W" alt+o
for _ in $(seq 20); do grep -aq "debug visão geral: aberta" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
ULT=$(grep -a "debug visão geral: [0-9]* janelas:" "$T/shell.log" | tail -1)
ok "Mod+O abre a visão geral com as janelas do espaço" '[ -n "$ULT" ] && echo "$ULT" | grep -q "janelas: [0-9]"'
# drag the first picture onto "+"
read -r JID JX JY < <(echo "$ULT" | sed 's/.*janelas: //' | awk '{split($1, a, "@"); split(a[2], p, ","); split($2, s, "x"); print a[1], int(p[1] + s[1] / 2), int(p[2] + s[2] / 2)}')
read -r NX NY < <(grep -a "debug visão geral: espaço #[0-9]* novo em" "$T/shell.log" | tail -1 | awk '{print int($(NF-1)), int($NF)}')
ESP0=$("$N" msg -j windows | python3 -c "import json,sys; print([w['workspace_id'] for w in json.load(sys.stdin) if w['id'] == $JID][0])")
xdotool mousemove --window "$W" "$JX" "$JY" mousedown 1
for i in 1 2 3 4 5 6 7 8; do xdotool mousemove --window "$W" $((JX + (NX - JX) * i / 8)) $((JY + (NY - JY) * i / 8)); sleep 0.08; done
sleep 0.3
xdotool mouseup 1
sleep 1.5
ok "arrastar uma janela até + a leva a um espaço novo" '[ "$("$N" msg -j windows | python3 -c "import json,sys; print([w[\"workspace_id\"] for w in json.load(sys.stdin) if w[\"id\"] == $JID][0])")" != "$ESP0" ]'
# rename the first workspace (double click)
read -r EX EY < <(grep -a "debug visão geral: espaço #0 existente em" "$T/shell.log" | tail -1 | awk '{print int($(NF-1)), int($NF)}')
xdotool mousemove --window "$W" "$EX" "$EY"
sleep 0.4
xdotool click --repeat 2 --delay 80 1
sleep 1
xdotool type --window "$W" --delay 60 Estudar
xdotool key --window "$W" Return
sleep 1
ok "clique duplo renomeia o espaço" '"$N" msg -j workspaces | grep -q "\"name\":\"Estudar\""'
# click a picture: that window gets the focus and the overview closes
ULT=$(grep -a "debug visão geral: [0-9]* janelas:" "$T/shell.log" | tail -1)
read -r JID JX JY < <(echo "$ULT" | sed 's/.*janelas: //' | awk '{split($1, a, "@"); split(a[2], p, ","); split($2, s, "x"); print a[1], int(p[1] + s[1] / 2), int(p[2] + s[2] / 2)}')
xdotool mousemove --window "$W" "$JX" "$JY" click 1
sleep 1.5
ok "clicar numa janela vai até ela e fecha a visão geral" '"$N" msg -j focused-window | grep -q "\"id\":$JID," && grep -a "debug visão geral: fechada" "$T/shell.log" | tail -1 | grep -q fechada'
# the top bar works above the overview: a click there reaches it and leaves the overview
# (the picture is taken here: niri's "screenshot captured" banner would sit over the workspaces in the steps above)
xdotool key --window "$W" alt+o
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-visao.png" >/dev/null && sleep 0.3
LAT0=$(grep -ac "debug lateral: aberta" "$T/shell.log")
FECH0=$(grep -ac "debug visão geral: fechada" "$T/shell.log")
xdotool mousemove --window "$W" $((OW / 2)) 16 click 1
sleep 1.5
ok "a barra superior funciona sobre a visão geral (e a fecha)" '[ "$(grep -ac "debug visão geral: fechada" "$T/shell.log")" -gt "$FECH0" ] && [ "$(grep -ac "debug lateral: aberta" "$T/shell.log")" -gt "$LAT0" ]'
xdotool key --window "$W" Escape
sleep 0.6
xdotool mousemove --window "$W" $((OW / 2)) 16 click 1
sleep 1
# the test apps' windows (the editor and the calculators), by id: a floating one may not be the focused one
fechar_apps
"$N" msg action focus-workspace 1
"$N" msg action unset-workspace-name
sleep 1

# 23. widgets (design "Editar widgets"): editing mode; dragging snaps a widget to the 184 px grid; a gallery card adds
#     one in a free cell; "−" removes one; Esc saves the layout (~/.config/kalango/widgets.json)
"$N" msg action focus-workspace 3
sleep 0.8
"$N" msg action spawn -- "$T/kalango-shell" widgets
for _ in $(seq 20); do grep -aq "debug widgets: editando" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
ok "o modo de edição dos widgets abre" 'grep -aq "debug widgets: editando" "$T/shell.log"'
read -r RX RY < <(grep -a "debug widgets: relogio P meio em" "$T/shell.log" | tail -1 | awk '{print $7, $8}')
xdotool mousemove --window "$W" "$RX" "$RY" mousedown 1
for i in 1 2 3 4 5 6 7 8; do xdotool mousemove --window "$W" $((RX + 368 * i / 8)) "$RY"; sleep 0.06; done
sleep 0.3
xdotool mouseup 1
sleep 1
ok "arrastar encaixa o widget na grade de 184 px" 'grep -aq "debug widgets: relogio em 416 64 (esquerda)" "$T/shell.log"'
# (at 1280 px the gallery covers the right column: the test uses what stays in view)
read -r CX CY < <(grep -a "debug widgets: cartão tokens em" "$T/shell.log" | tail -1 | awk '{print $6, $7}')
xdotool mousemove --window "$W" "$CX" "$CY" click 1
sleep 1
ok "um cartão da galeria adiciona o widget num lugar livre" 'grep -aq "debug widgets: tokens adicionado em" "$T/shell.log"'
read -r DX DY < <(grep -a "debug widgets: resumo G meio em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "$DX" "$DY" click 1
sleep 1
# a widget that was moved is still the same widget: its "−" removes it (it used to point at the old one)
read -r MX MY < <(grep -a "debug widgets: relogio P meio em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "$MX" "$MY" click 1
sleep 1
xdotool key --window "$W" Escape
sleep 1.5
ok "\"−\" remove (também um widget que foi movido) e Esc salva a disposição" 'grep -aq "debug widgets: resumo removido" "$T/shell.log" && grep -aq "debug widgets: relogio removido" "$T/shell.log" && python3 -c "import json,sys; v=json.load(open(\"$X/config/kalango/widgets.json\")); t=[w[\"tipo\"] for w in v]; sys.exit(0 if \"tokens\" in t and \"resumo\" not in t and \"relogio\" not in t else 1)"'
"$N" msg action focus-workspace 1
sleep 0.5

# 25. agents center: opens as the Agents app (its own app id, the dock and Alt+Tab know it), with the tasks that ended
#     in this session (the agent requests of the checks above)
"$N" msg action spawn -- "$T/kalango-shell" central
for _ in $(seq 20); do grep -aq "debug central: aberta" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
ok "a Central de agentes abre como o app Agentes, com as tarefas que terminaram" '"$N" msg -j windows | grep -q "\"app_id\":\"os.kalango.Agentes\"" && grep -a "debug central: [0-9]* tarefas" "$T/shell.log" | tail -1 | grep -qv "debug central: 0 tarefas"'
"$N" msg action close-window
sleep 0.8

# 26. agents center › permissions: a click on a cell changes that column for the agent (the agent reads the file before
#     every action, so it holds at once); with the agent paused, it refuses new work saying why
"$N" msg action spawn -- "$T/kalango-shell" central permissoes
for _ in $(seq 40); do grep -aq "debug central: pausar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
ultima_perm() { grep -a "debug central: permissões (" "$T/shell.log" | tail -1; }
read -r CX CY < <(lugar "célula apagar")
xdotool mousemove --window "$W" $((JX + CX)) $((JY + CY)) click 1
for _ in $(seq 30); do ultima_perm | grep -q "apagar=nunca" && break; sleep 0.3; done
ok "Permissões: um clique na célula muda a coluna do agente" 'ultima_perm | grep -q "apagar=nunca" && grep -q "\"apagar\": \"nunca\"" "$X/config/kalango/agente-permissoes.json"'
read -r PX PY < <(lugar pausar)
xdotool mousemove --window "$W" $((JX + PX)) $((JY + PY)) click 1
for _ in $(seq 30); do ultima_perm | grep -q "pausado true" && break; sleep 0.3; done
RECUSA=$(echo '"Quanto espaço livre tem no disco?"' | env LANGUAGE=pt_BR XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_DATA_HOME="$X/data" \
    KALANGOD_SOCKET="$T/mcp.sock" KALANGO_MODELO="$MODELO" KALANGO_MODELOS="$MODELOS" timeout 120 "$ROOT/target/release/kalango-agente" --json 2>/dev/null | grep '"tipo":"erro"')
ok "Permissões: pausado, o agente recusa trabalho novo dizendo por quê" 'ultima_perm | grep -q "pausado true" && echo "$RECUSA" | grep -q "pausado"'
# resumes (the same button)
xdotool mousemove --window "$W" $((JX + PX)) $((JY + PY)) click 1
for _ in $(seq 30); do ultima_perm | grep -q "pausado false" && break; sleep 0.3; done
"$N" msg action close-window
sleep 0.8

# 27. agents center › memory: a request typed in its field ("lembre que…") makes the agent remember it (what the person
#     said), the list shows it at once, and the trash button forgets it
"$N" msg action spawn -- "$T/kalango-shell" central memoria
for _ in $(seq 40); do grep -aq "debug central: busca em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
ultima_mem() { grep -a "debug central: memória (" "$T/shell.log" | tail -1; }
read -r BX BY < <(lugar busca)
xdotool mousemove --window "$W" $((JX + BX)) $((JY + BY)) click 1
sleep 0.4
xdotool type --window "$W" --delay 40 "lembre que eu prefiro respostas curtas"
xdotool key --window "$W" Return
for _ in $(seq 150); do ultima_mem | grep -q "(1 lembranças" && break; sleep 0.5; done
ok "Memória: o pedido no campo vira uma lembrança (você disse) e aparece na lista" 'ultima_mem | grep -q "(1 lembranças" && python3 -c "import json,sys; l=json.load(open(sys.argv[1]))[\"lembrancas\"]; sys.exit(not (len(l)==1 and l[0][\"origem\"]==\"voce\" and \"curt\" in l[0][\"texto\"].lower()))" "$X/data/kalango/memoria.json"'
# the agent bar opened for the request: closed, so nothing covers the list
"$N" msg action spawn -- "$T/kalango-shell" lateral agente
sleep 1.2
read -r AX AY < <(grep -a "debug central: apagar lembrança" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" $((JX + AX)) $((JY + AY)) click 1
for _ in $(seq 30); do ultima_mem | grep -q "(0 lembranças" && break; sleep 0.3; done
ok "Memória: a lixeira esquece a lembrança" 'ultima_mem | grep -q "(0 lembranças" && ! grep -qi "curt" "$X/data/kalango/memoria.json"'
"$N" msg action close-window
sleep 0.8

# 28. agents center › routines: a routine made in the form ("Nova rotina": name, what the agent does, every day at a
#     time a minute or two ahead) runs by itself at that time, with the real agent, and its result is recorded
"$N" msg action spawn -- "$T/kalango-shell" central rotinas
for _ in $(seq 40); do grep -aq "debug central: nova rotina em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r NX NY < <(lugar "nova rotina")
# (the agent's notice of the memory request — the bar was closed before it ended — drops at the top centre)
sem_avisos
xdotool mousemove --window "$W" $((JX + NX)) $((JY + NY)) click 1
for _ in $(seq 20); do grep -aq "debug rotinas: formulário aberto" "$T/shell.log" && break; sleep 0.2; done
sleep 0.6
# (the name field is clicked: the typing does not depend on where the focus went)
read -r RX RY < <(lugar "rotina nome")
[ -n "$RX" ] && xdotool mousemove --window "$W" $((JX + RX)) $((JY + RY)) click 1
sleep 0.3
HORA=$(date -d "+$(( 10#$(date +%S) > 30 ? 2 : 1 )) min" +%H:%M)
# (xdotool under the session's ABNT2 layout cannot type ":" or "ç": the time goes as 13h37, which the field accepts)
xdotool type --window "$W" --delay 30 "Espaco livre"
# (each field clicked: a Tab that lands on the frequency's drop-down turns the description's spaces into presses)
read -r RX RY < <(lugar "rotina pedido")
[ -n "$RX" ] && xdotool mousemove --window "$W" $((JX + RX)) $((JY + RY)) click 1 && sleep 0.3
xdotool type --window "$W" --delay 30 "Quanto espaco livre tem no disco"
read -r RX RY < <(lugar "rotina hora")
[ -n "$RX" ] && xdotool mousemove --window "$W" $((JX + RX)) $((JY + RY)) click 1 && sleep 0.3
xdotool key --window "$W" ctrl+a
xdotool type --window "$W" --delay 30 "${HORA/:/h}"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug rotinas: criada Espaco livre" "$T/shell.log" && break; sleep 0.3; done
ok "Rotinas: a rotina criada no formulário fica com o horário pedido" 'python3 -c "import json,sys; r=[x for x in json.load(open(sys.argv[1]))[\"rotinas\"] if x[\"nome\"]==\"Espaco livre\"][0][\"gatilho\"]; sys.exit(not (r[\"tipo\"]==\"horario\" and \"%02d:%02d\" % (r[\"hora\"], r[\"minuto\"])==sys.argv[2]))" "$X/config/kalango/rotinas.json" "$HORA"'
for _ in $(seq 240); do grep -aq "debug rotinas: Espaco livre terminou" "$T/shell.log" && break; sleep 1; done
ok "Rotinas: no horário ela roda sozinha com o agente e o resultado fica registrado" 'grep -a "debug rotinas: Espaco livre terminou" "$T/shell.log" | tail -1 | grep -q "(ok" && grep -q "\"rotina\":\"Espaco livre\"" "$X/state/kalango/tarefas.jsonl"'
"$N" msg action close-window
sleep 0.8

# 29. agents center › timeline: the agent moves a file (in the test area; writing set to "free" for it), the timeline
#     shows the move as reversible, and its "Desfazer" (approved like any system change) puts the file back
AG() { env LANGUAGE=pt_BR XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_DATA_HOME="$X/data" KALANGOD_SOCKET="$T/mcp.sock" \
    KALANGO_MODELO="$MODELO" KALANGO_MODELOS="$MODELOS" KALANGO_MEMORIA_CHAVEIRO=nao "$ROOT/target/release/kalango-agente" "$@"; }
AG coluna escrever livre
mkdir -p "$X/area/pasta"; echo "relatório" > "$X/area/relatorio.txt"
"$N" msg action spawn -- "$T/kalango-shell" pedir Mova o arquivo "$X/area/relatorio.txt" para "$X/area/pasta/relatorio.txt"
for _ in $(seq 120); do [ -e "$X/area/pasta/relatorio.txt" ] && break; sleep 1; done
"$N" msg action spawn -- "$T/kalango-shell" central linha
for _ in $(seq 40); do grep -aq "debug central: desfazer registro" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r DX DY < <(grep -a "debug central: desfazer registro" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" $((JX + DX)) $((JY + DY)) click 1
# the undo waits for approval: approved from here (another process, as on the approval screen)
for _ in $(seq 40); do PID_AP=$(KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" pendentes 2>/dev/null | grep "linha_do_tempo.desfazer" | head -1 | grep -o "^#[0-9]*" | tr -d '#'); [ -n "$PID_AP" ] && break; sleep 0.3; done
[ -n "$PID_AP" ] && KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" aprovar "$PID_AP" >/dev/null 2>&1
for _ in $(seq 40); do grep -a "debug central: linha do tempo (" "$T/shell.log" | tail -1 | grep -q "1 desfeitas" && break; sleep 0.5; done
ok "Linha do tempo: o Desfazer da ação do agente devolve o arquivo (com aprovação)" '[ -e "$X/area/relatorio.txt" ] && [ ! -e "$X/area/pasta/relatorio.txt" ] && grep -a "debug central: linha do tempo (" "$T/shell.log" | tail -1 | grep -q "1 desfeitas"'
"$N" msg action close-window
sleep 0.8

# 30. agents center › usage: the real counts of this session (tokens, tasks done), and the suggested daily limit,
#     applied with a click, becomes the limit the agent enforces
"$N" msg action spawn -- "$T/kalango-shell" central uso
for _ in $(seq 40); do grep -aq "debug central: aplicar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r UX UY < <(lugar aplicar)
xdotool mousemove --window "$W" $((JX + UX)) $((JY + UY)) click 1
for _ in $(seq 30); do grep -q '"tokens_dia"' "$X/config/kalango/agente-permissoes.json" 2>/dev/null && break; sleep 0.3; done
ok "Uso: os números reais da sessão, e o limite sugerido vira o limite que o agente aplica" 'grep -a "debug central: uso (" "$T/shell.log" | head -1 | grep -qv "(0 tokens" && grep -a "debug central: uso (" "$T/shell.log" | head -1 | grep -qv " 0 tarefas" && grep -q "\"tokens_dia\"" "$X/config/kalango/agente-permissoes.json"'
"$N" msg action close-window
sleep 0.8

# 31. agents center › settings › connectors: a third-party MCP connector (the world clock) added to the configuration
#     shows up connected after a real handshake (kalangod runs it isolated), and the agent uses its tool when a
#     request names it — the call is in kalangod's record of the connector
mkdir -p "$X/config/kalango/conectores"
printf 'nome = "relogio"\ncomando = "/usr/bin/python3"\nargumentos = ["-I", "%s/sessao/ferramentas/relogio-mcp.py"]\npasta = "%s/sessao/ferramentas"\n' "$ROOT" "$ROOT" > "$X/config/kalango/conectores/relogio.toml"
"$N" msg action spawn -- "$T/kalango-shell" central ajustes
for _ in $(seq 60); do grep -a "debug central: ajustes (" "$T/shell.log" | tail -1 | grep -q "relogio=ok" && break; sleep 0.5; done
ok "Ajustes: o conector MCP adicionado aparece conectado (aperto de mão de verdade, isolado)" 'grep -a "debug central: ajustes (" "$T/shell.log" | tail -1 | grep -q "relogio=ok:1"'
"$N" msg action close-window
sleep 0.5
AG coluna ler livre
# (check 30 set a daily token limit on purpose: off again, so this request is not refused for it)
AG limite tokens 0
"$N" msg action spawn -- "$T/kalango-shell" pedir Use o relogio para me dizer que horas são em Tóquio agora
for _ in $(seq 120); do grep -qs "hora_em" "$X/state/kalango/conectores/relogio.jsonl" && break; sleep 1; done
ok "Ajustes: o agente usa a ferramenta do conector quando o pedido o nomeia" 'grep -qs "hora_em" "$X/state/kalango/conectores/relogio.jsonl"'
"$N" msg action spawn -- "$T/kalango-shell" lateral agente
sleep 0.8

# 32. connectors that fail and adding one: a connector whose command is missing opens in diagnosis ("Node.js is
#     missing") and its fix installs Node.js through kalangod (approved like any system change); "Adicionar conector"
#     picks the world clock from the catalog, tests it isolated and adds it
printf 'nome = "quebrado"\ncomando = "/usr/local/bin/npx"\nargumentos = ["-y", "@modelcontextprotocol/server-postgres"]\n' > "$X/config/kalango/conectores/quebrado.toml"
rm -f "$X/config/kalango/conectores/relogio.toml"
"$N" msg action spawn -- "$T/kalango-shell" central ajustes
for _ in $(seq 60); do grep -aq "debug central: diagnóstico quebrado" "$T/shell.log" && break; sleep 0.5; done
sleep 1
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
ok "Conector com falha: o diagnóstico diz em palavras simples que falta o Node.js" 'grep -a "debug central: diagnóstico quebrado" "$T/shell.log" | tail -1 | grep -q "Node.js"'
read -r CX CY < <(lugar correcao)
xdotool mousemove --window "$W" $((JX + CX)) $((JY + CY)) click 1
for _ in $(seq 40); do PID_AP=$(KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" pendentes 2>/dev/null | grep "pacotes.instalar" | head -1 | grep -o "^#[0-9]*" | tr -d '#'); [ -n "$PID_AP" ] && break; sleep 0.3; done
[ -n "$PID_AP" ] && KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" aprovar "$PID_AP" >/dev/null 2>&1
for _ in $(seq 30); do grep -q '"tool":"pacotes.instalar".*nodejs' "$T/estado/linha-do-tempo.jsonl" 2>/dev/null && break; sleep 0.5; done
ok "Conector com falha: “Instalar Node.js” instala pelo kalangod, com aprovação" 'grep -q "\"tool\":\"pacotes.instalar\".*nodejs" "$T/estado/linha-do-tempo.jsonl"'
# the approval opened the agent bar: the Central again (in front, floating bars closed)
sleep 1.5
"$N" msg action spawn -- "$T/kalango-shell" central ajustes
sleep 2
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r AX AY < <(lugar "botao adicionar conector")
xdotool mousemove --window "$W" $((JX + AX)) $((JY + AY)) click 1
for _ in $(seq 20); do grep -aq "debug central: catalogo primeiro em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
read -r KX KY < <(lugar "catalogo primeiro")
xdotool mousemove --window "$W" $((JX + KX)) $((JY + KY)) click 1
for _ in $(seq 60); do grep -aq "debug central: novo conector testado" "$T/shell.log" && break; sleep 0.5; done
sleep 0.6
read -r BX BY < <(lugar "adicionar conector")
xdotool mousemove --window "$W" $((JX + BX)) $((JY + BY)) click 1
for _ in $(seq 30); do grep -aq "debug central: conector adicionado relogio" "$T/shell.log" && break; sleep 0.3; done
ok "Adicionar conector: do catálogo, testado isolado antes, e adicionado" 'grep -aq "debug central: conector adicionado relogio" "$T/shell.log" && grep -q "sob_demanda" "$X/config/kalango/conectores/relogio.toml"'
"$N" msg action close-window
sleep 0.8

# 33. agents center › conversations: a request typed in the new conversation's composer starts a conversation with the
#     real agent, kept on disk and listed; "Nova conversa" clears the view and the list opens it again (its events
#     rebuild what was said and done)
"$N" msg action spawn -- "$T/kalango-shell" central conversas
for _ in $(seq 40); do grep -aq "debug central: nova conversa texto em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r TX TY < <(lugar "nova conversa texto")
xdotool mousemove --window "$W" $((JX + TX)) $((JY + TY)) click 1
sleep 0.4
xdotool type --window "$W" --delay 30 "Quanto espaco livre tem no disco"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug central: conversa começou" "$T/shell.log" && break; sleep 0.3; done
CID=$(grep -a "debug central: conversa começou" "$T/shell.log" | tail -1 | awk '{print $NF}')
for _ in $(seq 120); do grep -qs '"tipo":"fim"' "$X/data/kalango/conversas/$CID/eventos.jsonl" && break; sleep 1; done
ok "Conversas: a conversa começa no compositor, com o agente, e fica guardada" '[ -n "$CID" ] && grep -qs "\"tipo\":\"resposta\"" "$X/data/kalango/conversas/$CID/eventos.jsonl" && grep -qs "Quanto espaco" "$X/data/kalango/conversas/$CID/meta.json" && [ -s "$X/data/kalango/conversas/$CID/mensagens.json" ]'
# (the suite's window is 1200 px: the medium band, where the side bar is the rail — its button opens it)
read -r RX RY < <(lugar "central trilho lista")
[ -n "$RX" ] && apontar $((JX + RX)) $((JY + RY)) && sleep 0.3 && xdotool click 1
sleep 1
read -r NX NY < <(lugar "nova conversa botao")
xdotool mousemove --window "$W" $((JX + NX)) $((JY + NY)) click 1
sleep 1.2
read -r LX LY < <(lugar "conversa $CID")
xdotool mousemove --window "$W" $((JX + LX)) $((JY + LY)) click 1
for _ in $(seq 20); do grep -aq "debug central: conversa aberta $CID" "$T/shell.log" && break; sleep 0.3; done
ok "Conversas: a lista abre a conversa guardada, refeita pelos eventos" 'grep -a "debug central: conversa aberta $CID" "$T/shell.log" | tail -1 | grep -qv "(0 eventos)"'
# 93. The fluid chat (design "Chat · fluido"): a medium window (1000–1400 px) has the rail and the context folded into
#     its tabs; a tab opens the context on it. Narrower than 1000 px it is one column: the side bar and the context
#     open as sheets over it (Esc closes them)
janela_central() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
sleep 1
echo "  ($(grep -a "debug central: conversas em" "$T/shell.log" | tail -1))"
read -r FX FY < <(lugar "conversas faixa 1")
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-media.png" >/dev/null
ok "Chat fluido: na janela média o contexto vira a faixa de abas (a lista, como a pessoa a deixou)" \
  'grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "em Media (.*contexto false" && [ -n "$FX" ]'
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
[ -n "$FX" ] && apontar $((JX + FX)) $((JY + FY)) && sleep 0.3 && xdotool click 1
sleep 1
ok "Chat fluido: a aba Arquivos da faixa abre o contexto nela" \
  'grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "em Media (.*contexto true"'
"$N" msg action toggle-window-floating
sleep 0.5
"$N" msg action set-window-width 760
sleep 2
echo "  ($(grep -a "debug central: conversas em" "$T/shell.log" | tail -1))"
ok "Chat fluido: abaixo de 1000 px é uma coluna (sem trilho, sem lista, sem contexto ao lado)" \
  'grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "em Estreita (.*contexto false · lista false"'
sleep 1
read -r JX JY < <(janela_central)
read -r AX AY < <(lugar "conversas abrir lista")
[ -n "$AX" ] && apontar $((JX + AX)) $((JY + AY)) && sleep 0.3 && xdotool click 1
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-estreita.png" >/dev/null
ok "Chat fluido: na janela estreita a lista abre como folha por cima" \
  'grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "em Estreita (.*lista true"'
xdotool key --window "$W" Escape
sleep 0.8
ok "Chat fluido: Esc fecha a folha" 'grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "em Estreita (.*lista false"'
"$N" msg action close-window
sleep 0.8

# 34. the agent's failures, one card each with a real fix (in the agents center's conversation): past today's token limit
#     the card "Orçamento atingido" raises it only for today and the request goes on; outside the agent's folders the
#     card "Ação bloqueada" allows that folder and the request goes on
janela_central() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
conversa_atual() { grep -a "debug central: conversa começou" "$T/shell.log" | tail -1 | awk '{print $NF}'; }
AG limite tokens 1000
F0=$(grep -ac "debug conversa: falha" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" conversar Quanto espaco livre tem no disco
for _ in $(seq 60); do grep -aq "debug central: aumentar so hoje final em" "$T/shell.log" && break; sleep 0.5; done
sleep 0.8
CB=$(conversa_atual)
read -r JX JY < <(janela_central)
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r BX BY < <(lugar "aumentar so hoje final")
xdotool mousemove --window "$W" $((JX + BX)) $((JY + BY)) click 1
for _ in $(seq 120); do grep -qs '"tipo":"resposta"' "$X/data/kalango/conversas/$CB/eventos.jsonl" && break; sleep 1; done
ok "Falhas: “Orçamento atingido” aumenta só hoje e o pedido continua" 'grep -qs "\"codigo\":\"orcamento\"" "$X/data/kalango/conversas/$CB/eventos.jsonl" && grep -qs "\"tipo\":\"resposta\"" "$X/data/kalango/conversas/$CB/eventos.jsonl" && grep -q "extra_hoje" "$X/config/kalango/agente-permissoes.json"'
AG limite tokens 0
mkdir -p "$X/area/Arquivo" "$X/area/fora"; echo "um" > "$X/area/fora/um.txt"
AG pasta "$X/area/Arquivo" adicionar
AG pasta "~" desativada
"$N" msg action spawn -- "$T/kalango-shell" conversar Liste os arquivos da pasta "$X/area/fora"
# (the button's place once the turn ended: the answer after the card moves it)
for _ in $(seq 120); do grep -aq "debug central: liberar final em" "$T/shell.log" && break; sleep 0.5; done
sleep 0.8
CL=$(conversa_atual)
read -r JX JY < <(janela_central)
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r LBX LBY < <(lugar "liberar final")
xdotool mousemove --window "$W" $((JX + LBX)) $((JY + LBY)) click 1
for _ in $(seq 120); do python3 -c "import json,sys; ev=[json.loads(l) for l in open(sys.argv[1])]; sys.exit(not any(e.get('tipo')=='resultado' and e.get('nome')=='arquivos.listar' and e.get('ok') for e in ev))" "$X/data/kalango/conversas/$CL/eventos.jsonl" 2>/dev/null && break; sleep 1; done
ok "Falhas: “Ação bloqueada” libera a pasta e o pedido continua" 'grep -qs "\"tipo\":\"bloqueio\"" "$X/data/kalango/conversas/$CL/eventos.jsonl" && grep -q "area/fora" "$X/config/kalango/agente-permissoes.json" && python3 -c "import json,sys; ev=[json.loads(l) for l in open(sys.argv[1])]; sys.exit(not any(e.get(\"tipo\")==\"resultado\" and e.get(\"nome\")==\"arquivos.listar\" and e.get(\"ok\") for e in ev))" "$X/data/kalango/conversas/$CL/eventos.jsonl"'
AG pasta "~" ativa
"$N" msg action close-window
sleep 0.8

# 35. approval notice: the agent asks to move a file while its conversation is not on screen (the side bar closed):
#     the island becomes "Agente pede aprovação" with a card under it, and the island's "Permitir" answers it (once)
AG coluna escrever perguntar
# (check 34 left the agent's folders as a list: the test area goes in it, so this is a request, not a block)
AG pasta "$X/area" adicionar
echo "nota" > "$X/area/nota-aviso.txt"
# (in the agents center's conversation; its window closed right after: the conversation goes on off screen)
"$N" msg action spawn -- "$T/kalango-shell" conversar Mova o arquivo "$X/area/nota-aviso.txt" para "$X/area/pasta/nota-aviso.txt"
for _ in $(seq 20); do "$N" msg -j focused-window | grep -q "os.kalango.Agentes" && break; sleep 0.3; done
"$N" msg action close-window
for _ in $(seq 120); do grep -aq "debug aviso pedido: mostrado" "$T/shell.log" && grep -aq "debug central: pedido permitir em" "$T/shell.log" && break; sleep 0.5; done
sleep 0.6
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-aviso-pedido.png" >/dev/null
read -r PPX PPY < <(lugar "pedido permitir")
xdotool mousemove --window "$W" "$PPX" "$PPY" click 1
for _ in $(seq 60); do [ -e "$X/area/pasta/nota-aviso.txt" ] && break; sleep 0.5; done
ok "Notificação de aprovação: com a conversa fora da tela, a ilha pede e o Permitir dela responde" 'grep -aq "debug aviso pedido: permitido" "$T/shell.log" && [ -e "$X/area/pasta/nota-aviso.txt" ] && [ ! -e "$X/area/nota-aviso.txt" ]'
AG coluna escrever livre

# 94. The conversation's blocks (design "Chat · blocos de mensagem"): code in an answer is a block with its language,
#     numbered lines and the theme's colours; a program the agent runs shows as a terminal block (command, output,
#     exit code and time); a file it wants to write shows as a diff to accept before it is written
R0=$(grep -ac "debug codigo: bloco Rust (" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" conversar Mostre um exemplo curto de codigo em Rust que imprime ola
for _ in $(seq 120); do [ "$(grep -ac "debug codigo: bloco Rust (" "$T/shell.log")" -gt "$R0" ] && break; sleep 1; done
echo "  ($(grep -a "debug codigo: bloco " "$T/shell.log" | tail -2 | tr '\n' ' '))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1.5 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-codigo.png" >/dev/null
ok "Chat: o código de uma resposta vira o bloco com a linguagem e as linhas numeradas (o modelo local)" '[ "$(grep -ac "debug codigo: bloco Rust ([1-9]" "$T/shell.log")" -gt "$R0" ]'
"$N" msg action close-window
sleep 0.8
AG coluna terminal livre
mkdir -p "$X/area/codigo"
# (a request only a program answers: for a folder's list the agent rightly uses its own file tool)
"$N" msg action spawn -- "$T/kalango-shell" conversar Use o terminal para rodar o programa echo com o argumento ola-do-terminal e me diga o que saiu
for _ in $(seq 120); do grep -aq "debug codigo: terminal «echo" "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug codigo: terminal «" "$T/shell.log" | tail -1))"
ok "Chat: um programa que o agente roda aparece como o bloco do terminal, com a saída e o exit" 'grep -a "debug codigo: terminal «echo ola-do-terminal»" "$T/shell.log" | tail -1 | grep -q "(exit 0, 1 linhas)"'
"$N" msg action close-window
sleep 0.8
AG coluna terminal perguntar
AG coluna escrever perguntar
# (in a git repository of the test's own: the conversation becomes code mode)
REPO="$X/area/projeto"
mkdir -p "$REPO"
printf 'linha um\nlinha dois\nlinha tres\n' > "$REPO/notas.txt"
git -C "$REPO" init -q -b main
git -C "$REPO" config user.name Teste
git -C "$REPO" config user.email teste@exemplo.invalid
git -C "$REPO" add notas.txt && git -C "$REPO" commit -q -m inicio
"$N" msg action spawn -- "$T/kalango-shell" conversar No arquivo "$REPO/notas.txt", troque a linha "linha dois" por "linha 2" e mantenha as outras linhas
for _ in $(seq 150); do grep -aq "debug codigo: diff de " "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug codigo: diff de " "$T/shell.log" | tail -1))"
ok "Chat: escrever um arquivo mostra o diff antes (o que sai e o que entra)" 'grep -a "debug codigo: diff de " "$T/shell.log" | tail -1 | grep -q "notas.txt (+[1-9][0-9]* −[1-9]"'
for _ in $(seq 20); do grep -aq "debug central: diff aceitar em" "$T/shell.log" && break; sleep 0.3; done
sleep 1.8
read -r JX JY < <(janela_central)
read -r DAX DAY < <(lugar "diff aceitar")
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-diff.png" >/dev/null
sem_avisos 2>/dev/null || sleep 4
[ -n "$DAX" ] && apontar $((JX + DAX)) $((JY + DAY)) && sleep 0.3 && xdotool click 1
# (what is written is what the diff showed — the small model may get the edit wrong: that is what the diff is for)
CD=$(conversa_atual)
# (the one accepted, not the newest: the small model sometimes asks for a second edit right after, still pending)
gravou_o_diff() { python3 -c 'import json,sys; ev=[json.loads(l) for l in open(sys.argv[1])]; p=[e for e in ev if e.get("tipo")=="pedido" and e.get("nome")=="arquivos.escrever"]; t=open(sys.argv[2]).read(); sys.exit(not any(t == x["argumentos"]["conteudo"] for x in p))' "$X/data/kalango/conversas/$CD/eventos.jsonl" "$REPO/notas.txt" 2>/dev/null; }
for _ in $(seq 60); do gravou_o_diff && break; sleep 0.5; done
ok "Chat: Aceitar grava a mudança que o diff mostrou" 'gravou_o_diff && grep -q "\"nome\":\"arquivos.escrever\",\"ok\":true" "$X/data/kalango/conversas/$CD/eventos.jsonl"'
# code mode: the tabs change (Changes in front) and Changes has the file and the local model's message
for _ in $(seq 60); do grep -aq "debug conversas: abas Arquivos · Alterações · Terminal" "$T/shell.log" && break; sleep 0.5; done
for _ in $(seq 120); do agente_desta_sessao || break; sleep 1; done
sleep 1
read -r JX JY < <(janela_central)
read -r FX FY < <(lugar "conversas faixa 3")
sem_avisos 2>/dev/null || sleep 4
[ -n "$FX" ] && apontar $((JX + FX)) $((JY + FY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 90); do grep -aq "debug alteracoes: mensagem sugerida" "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug conversas: abas" "$T/shell.log" | tail -1); $(grep -a "debug alteracoes: mensagem sugerida" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-alteracoes.png" >/dev/null
ok "Chat · código: o arquivo mudado num repositório leva ao modo código, e Alterações traz o ramo e a mensagem do modelo local" \
  'grep -aq "debug conversas: abas Arquivos · Alterações · Terminal" "$T/shell.log" && grep -a "debug alteracoes: main em $REPO (1 arquivos" "$T/shell.log" >/dev/null && grep -aq "debug alteracoes: mensagem sugerida «..*»" "$T/shell.log"'
H0=$(git -C "$REPO" rev-parse HEAD)
sleep 1
read -r CX CY < <(lugar "alteracoes criar commit")
[ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug alteracoes: commit [0-9a-f]* «" "$T/shell.log" && break; sleep 0.3; done
H1=$(git -C "$REPO" rev-parse HEAD)
ok "Chat · código: “Criar commit” faz o commit com a mensagem sugerida (nada é enviado)" '[ "$H1" != "$H0" ] && [ -n "$(git -C "$REPO" log -1 --format=%s)" ] && [ -z "$(git -C "$REPO" remote)" ]'
sleep 0.8
[ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug alteracoes: commit [0-9a-f]* desfeito" "$T/shell.log" && break; sleep 0.3; done
ok "Chat · código: “Desfazer” tira o commit e a mudança fica" '[ "$(git -C "$REPO" rev-parse HEAD)" = "$H0" ] && ! git -C "$REPO" diff --quiet HEAD -- notas.txt'
# "Abrir PR": with no remote it says so, and nothing leaves
read -r PRX PRY < <(lugar "alteracoes abrir pr")
[ -n "$PRX" ] && apontar $((JX + PRX)) $((JY + PRY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do grep -aq "debug alteracoes: abrir PR" "$T/shell.log" && break; sleep 0.3; done
ok "Chat · código: “Abrir PR” sem remoto diz o que falta e nada sai" 'grep -a "debug alteracoes: abrir PR" "$T/shell.log" | tail -1 | grep -q "origin" && [ -z "$(git -C "$REPO" remote)" ]'
"$N" msg action close-window
sleep 0.8
AG coluna escrever livre

# 97. The chat's data (design "Chat · dados (tabela e gráfico)"): a file goes with the request as a chip (and is listed,
#     with its preview, in the context's Files tab); a table in the answer is one block with the table and its chart;
#     "CSV" keeps it in Downloads
printf 'categoria,outubro,novembro\nMercado,1240,1180\nEnergia,189,214\nAssinaturas,96,142\n' > "$X/area/codigo/gastos.csv"
"$N" msg action spawn -- "$T/kalango-shell" conversar "--anexo=$X/area/codigo/gastos.csv" Monte uma tabela em markdown com as colunas Categoria, Outubro e Novembro com os valores desse arquivo
for _ in $(seq 150); do grep -aq "debug dados: tabela " "$T/shell.log" && break; sleep 1; done
CQ=$(conversa_atual)
echo "  ($(grep -a "debug dados: tabela " "$T/shell.log" | tail -1); $(grep -a "debug anexar: .*gastos.csv" "$T/shell.log" | tail -1 | cut -c1-80))"
ok "Chat · dados: o arquivo vai com o pedido (a ficha, guardada na conversa)" 'grep -qs "gastos.csv" "$X/data/kalango/conversas/$CQ/eventos.jsonl" && python3 -c "import json,sys; ev=[json.loads(l) for l in open(sys.argv[1])]; sys.exit(not any(e.get(\"tipo\")==\"pessoa\" and e.get(\"anexos\") for e in ev))" "$X/data/kalango/conversas/$CQ/eventos.jsonl"'
ok "Chat · dados: a tabela da resposta vira o bloco com as colunas de números" 'grep -a "debug dados: tabela " "$T/shell.log" | tail -1 | grep -Eq "tabela [1-9][0-9]*×[2-9] \([1-9]"'
sleep 1.5
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r GX GY < <(lugar "tabela grafico")
[ -n "$GX" ] && apontar $((JX + GX)) $((JY + GY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do grep -aq "debug dados: vista gráfico" "$T/shell.log" && break; sleep 0.3; done
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-dados.png" >/dev/null
ok "Chat · dados: o mesmo bloco mostra o gráfico" 'grep -aq "debug dados: vista gráfico" "$T/shell.log"'
read -r CX CY < <(lugar "tabela csv")
[ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do grep -aq "debug dados: csv salvo" "$T/shell.log" && break; sleep 0.3; done
CSV=$(grep -a "debug dados: csv salvo" "$T/shell.log" | tail -1 | sed 's/.*csv salvo //')
ok "Chat · dados: “CSV” guarda a tabela em Downloads" '[ -s "$CSV" ] && case "$CSV" in "$X"/area/Downloads/*) true;; *) false;; esac'
"$N" msg action close-window
sleep 0.8

# 98. Diagrams, formulas and media in the conversation (designs "Chat · diagramas e fórmulas" and "Chat · mídia"): a
#     flowchart the agent writes in mermaid is drawn; a formula in LaTeX is typeset; a picture and a sound added to the
#     message show as themselves (the sound with its play button)
"$N" msg action spawn -- "$T/kalango-shell" conversar Desenhe em mermaid um fluxograma LR com tres passos ligados: Pedido, Analisar e Responder
for _ in $(seq 150); do grep -aq "debug diagrama: fluxograma" "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug diagrama: fluxograma" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1.5 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-diagrama.png" >/dev/null
ok "Chat: um fluxograma em mermaid vira o diagrama desenhado" 'grep -a "debug diagrama: fluxograma" "$T/shell.log" | tail -1 | grep -Eq "fluxograma [2-9] nós, [1-9]"'
"$N" msg action close-window
sleep 0.8
"$N" msg action spawn -- "$T/kalango-shell" conversar Escreva a formula de Bhaskara em LaTeX entre cifroes duplos
for _ in $(seq 150); do grep -aq "debug formulas: " "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug formulas: " "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1.5 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-formula.png" >/dev/null
ok "Chat: a fórmula em LaTeX aparece composta, numerada" 'grep -aq "debug formulas: [1-9] equações" "$T/shell.log"'
"$N" msg action close-window
sleep 0.8
python3 - "$X/area/codigo" <<'PYEOF'
import math, struct, sys, wave, zlib
d = sys.argv[1]
w, h = 64, 48
raw = b"".join(b"\x00" + b"".join(bytes([x * 4 % 256, y * 5 % 256, 160]) for x in range(w)) for y in range(h))
def chunk(t, data):
    return struct.pack(">I", len(data)) + t + data + struct.pack(">I", zlib.crc32(t + data) & 0xffffffff)
open(d + "/foto.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
f = wave.open(d + "/som.wav", "wb"); f.setnchannels(1); f.setsampwidth(2); f.setframerate(16000)
f.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * 440 * i / 16000))) for i in range(32000))); f.close()
PYEOF
"$N" msg action spawn -- "$T/kalango-shell" conversar "--anexo=$X/area/codigo/foto.png" "--anexo=$X/area/codigo/som.wav" Recebeu a foto e o audio? Responda numa frase
for _ in $(seq 40); do grep -aq "debug midia: áudio .*som.wav" "$T/shell.log" && break; sleep 0.5; done
ok "Chat · mídia: a foto e o áudio da mensagem aparecem como eles mesmos" 'grep -aq "debug midia: 1 imagens" "$T/shell.log" && grep -aq "debug midia: áudio .*som.wav" "$T/shell.log"'
# (the play button's place once the answer came and the conversation scrolled: logged again when the turn ends)
for _ in $(seq 150); do grep -aq "debug central: midia tocar final em" "$T/shell.log" && break; sleep 1; done
sleep 0.8
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
# (the side bar may show the same conversation and log its own "final" places: the central window's are the last)
sleep 3
read -r PX PY < <(lugar "midia tocar final")
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do grep -aq "debug midia: áudio tocando" "$T/shell.log" && break; sleep 0.3; done
ok "Chat · mídia: o áudio toca ali mesmo (mudo no teste)" 'grep -aq "debug midia: áudio tocando" "$T/shell.log"'
"$N" msg action close-window
sleep 0.8

# 100. A voice message (design "Chat · mídia"): with nothing written, the microphone records a message (the test
#      recorder, no microphone); the local voice model writes its words and the recording goes with them — the
#      person's bubble shows it as audio, kept with the conversation; the agent gets the words, not the file
"$N" msg action spawn -- "$T/kalango-shell" conversar Responda so com a palavra pronto
for _ in $(seq 150); do grep -aq "debug central: conversa microfone em" "$T/shell.log" && break; sleep 0.5; done
# (the answer first: the agent starts, then ends)
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.5; done
for _ in $(seq 180); do agente_desta_sessao || break; sleep 1; done
sleep 1.5
sem_avisos 2>/dev/null || sleep 4
V0=$(grep -ac "debug voz: Ok" "$T/shell.log")
read -r JX JY < <(janela_central)
read -r MX MY < <(lugar "conversa microfone")
[ -n "$MX" ] && apontar $((JX + MX)) $((JY + MY)) && sleep 0.3 && xdotool click 1
sleep 2
[ -n "$MX" ] && apontar $((JX + MX)) $((JY + MY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 60); do [ "$(grep -ac "debug voz: Ok" "$T/shell.log")" -gt "$V0" ] && break; sleep 0.5; done
grep -a "debug voz: Ok\|debug anexar: .*mensagem-de-voz" "$T/shell.log" | tail -2 | cut -c1-160 | sed 's/^/  (/; s/$/)/'
ok "Chat: com o compositor vazio, o microfone grava uma mensagem de voz (as palavras e a gravação)" \
  '[ "$(grep -ac "debug voz: Ok" "$T/shell.log")" -gt "$V0" ] && grep -aq "debug anexar: .*mensagem-de-voz-[0-9]*\.wav" "$T/shell.log"'
xdotool key --window "$W" Return
for _ in $(seq 40); do grep -aq "debug midia: áudio .*mensagem-de-voz" "$T/shell.log" && break; sleep 0.5; done
VOZ_EV=$(grep -l "mensagem-de-voz" "$X"/data/kalango/conversas/*/eventos.jsonl 2>/dev/null | head -1)
ok "Chat: a mensagem de voz aparece como áudio na bolha e fica guardada com a conversa" \
  'grep -aq "debug midia: áudio .*mensagem-de-voz" "$T/shell.log" && [ -n "$VOZ_EV" ] && ls "$(dirname "$VOZ_EV")"/mensagem-de-voz-*.wav >/dev/null 2>&1'
for _ in $(seq 60); do agente_desta_sessao && break; sleep 0.5; done
for _ in $(seq 180); do agente_desta_sessao || break; sleep 1; done
"$N" msg action close-window
sleep 0.8

# 101. Creating an agent by description (design "Criar agente (por descrição)"): the sentence goes to the local model,
#      which returns its name, when it runs, its steps and how much it asks; the page shows the permissions those steps
#      need and a simulation counted in the test Downloads — nothing changes —; "Criar agente" keeps it as a routine
#      that runs without the permission columns it does not need
DL="$X/area/Downloads"
mkdir -p "$DL"
for f in relatorio.pdf foto.png planilha.csv; do echo x > "$DL/$f"; done
head -c 200000 /dev/zero > "$DL/editor.deb"; head -c 300000 /dev/zero > "$DL/jogo.AppImage"
touch -d "40 days ago" "$DL/editor.deb" "$DL/jogo.AppImage"
ANTES_DL=$(ls -A "$DL" | sort | tr '\n' ' ')
NA0=$(grep -ac "debug novo agente: «" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" central novo-agente
for _ in $(seq 30); do "$N" msg -j windows | grep -q '"os.kalango.Agentes"' && break; sleep 0.3; done
sleep 1.5
"$N" msg action focus-window --id "$(id_app kalango.agentes)" 2>/dev/null
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r GX GY < <(lugar "novo agente gerar")
# (the sentence's field is left of the send button, on its line above)
[ -n "$GX" ] && apontar $((JX + GX - 300)) $((JY + GY - 40)) && sleep 0.3 && xdotool click 1
sleep 0.5
xdotool type --window "$W" --delay 30 "Toda sexta, organize meus downloads, apague instaladores com mais de 30 dias e me mande um resumo do que fez."
xdotool key --window "$W" Return
for _ in $(seq 150); do [ "$(grep -ac "debug novo agente: «" "$T/shell.log")" -gt "$NA0" ] && break; sleep 1; done
for _ in $(seq 30); do grep -aq "debug novo agente: simulação" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug novo agente: «" "$T/shell.log" | tail -1))"
echo "  ($(grep -a "debug novo agente: simulação" "$T/shell.log" | tail -1 | cut -c1-200))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-novo-agente.png" >/dev/null
ok "Criar agente: a frase vira nome, quando (toda sexta) e os passos, pelo modelo local" \
  'grep -a "debug novo agente: «" "$T/shell.log" | tail -1 | grep -Eq "«.+» · .*sext.* · [2-9] passos"'
ok "Criar agente: a simulação conta os arquivos de Downloads e não muda nada" \
  'grep -a "debug novo agente: simulação" "$T/shell.log" | tail -1 | grep -Eq "[1-9][0-9]* arquivos" && [ "$(ls -A "$DL" | sort | tr "\n" " ")" = "$ANTES_DL" ]'
sleep 1
read -r CX CY < <(lugar "novo agente criar")
[ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug novo agente: criado" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug novo agente: criado" "$T/shell.log" | tail -1))"
agente_criado() { python3 -c 'import json,sys; r=[x for x in json.load(open(sys.argv[1]))["rotinas"] if x.get("agente")]; a=r[-1]["agente"] if r else {}; sys.exit(not (r and r[-1]["ativa"] and "rede" in a["desligadas"] and "terminal" in a["desligadas"] and a["autonomia"] in ("risco","sempre") and len(a["passos"]) >= 2 and r[-1]["gatilho"]["tipo"] == "horario" and 5 in r[-1]["gatilho"]["dias"]))' "$X/config/kalango/rotinas.json" 2>/dev/null; }
ok "Criar agente: fica como rotina de sexta, sem rede nem terminal, e pergunta antes de apagar" 'agente_criado'
"$N" msg action close-window
sleep 0.8

# 102. Installing an agent (design "Instalar agente (permissões antes de instalar)"): a manifest (*.kalango-agente)
#      opens its page with the permissions it asks for; the network switched off leaves its scope; "Experimentar em
#      simulação" runs it for real and nothing changes (the file it would delete is still there); "Instalar com estas
#      permissões" keeps a routine with exactly the folders, programs and columns left on
PJ="$X/area/Projetos"
mkdir -p "$PJ"
echo "rascunho velho" > "$PJ/rascunho.txt"
cat > "$X/area/limpeza.kalango-agente" <<MANIFESTO
nome = "Limpeza de projetos"
id = "teste.limpeza"
editor = "Teste"
descricao = "Apaga rascunhos velhos da pasta de projetos."
instrucoes = "Liste a pasta $PJ e apague o arquivo $PJ/rascunho.txt. Responda numa frase."
autonomia = "autonomo"
[[permissoes]]
tipo = "ler"
pastas = ["$PJ"]
[[permissoes]]
tipo = "apagar"
pastas = ["$PJ"]
[[permissoes]]
tipo = "rede"
dominios = ["github.com"]
MANIFESTO
IA0=$(grep -ac "debug instalar agente: «" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" instalar-agente "$X/area/limpeza.kalango-agente"
for _ in $(seq 30); do [ "$(grep -ac "debug instalar agente: «" "$T/shell.log")" -gt "$IA0" ] && break; sleep 0.3; done
sleep 1.5
sem_avisos 2>/dev/null || sleep 4
echo "  ($(grep -a "debug instalar agente: «" "$T/shell.log" | tail -1))"
ok "Instalar agente: o manifesto abre a página com as permissões que ele pede" 'grep -a "debug instalar agente: «" "$T/shell.log" | tail -1 | grep -q "«Limpeza de projetos» de Teste (3 permissões pedidas)"'
read -r JX JY < <(janela_central)
read -r RX RY < <(lugar "instalar agente permissao 2")
[ -n "$RX" ] && apontar $((JX + RX)) $((JY + RY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do grep -aq "debug instalar agente: permissão 2 false" "$T/shell.log" && break; sleep 0.3; done
ok "Instalar agente: desligar a rede antes de instalar a tira do resumo de confiança" 'grep -a "debug instalar agente: confiança" "$T/shell.log" | tail -1 | grep -q "sites  · sem .*rede"'
read -r SX SY < <(lugar "instalar agente simular")
[ -n "$SX" ] && apontar $((JX + SX)) $((JY + SY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 180); do grep -aq "debug instalar agente: simulação com" "$T/shell.log" && break; sleep 1; done
echo "  ($(grep -a "debug instalar agente: simulação com" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-instalar-agente.png" >/dev/null
ok "Instalar agente: a simulação roda o agente de verdade e não muda nada" 'grep -aq "debug instalar agente: simulação com [1-9]" "$T/shell.log" && [ -s "$PJ/rascunho.txt" ]'
sleep 1
read -r IX IY < <(lugar "instalar agente instalar fim")
[ -z "$IX" ] && read -r IX IY < <(lugar "instalar agente instalar")
[ -n "$IX" ] && apontar $((JX + IX)) $((JY + IY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug instalar agente: instalado" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug instalar agente: instalado" "$T/shell.log" | tail -1))"
instalado() { python3 -c 'import json,sys; r=[x for x in json.load(open(sys.argv[1]))["rotinas"] if (x.get("agente") or {}).get("origem")=="teste.limpeza"]; a=r[-1]["agente"] if r else {}; sys.exit(not (r and a["pastas"]==[sys.argv[2]] and a["dominios"]==[] and "rede" in a["desligadas"] and "apagar" not in a["desligadas"] and a["autonomia"]=="autonomo" and r[-1]["gatilho"]["tipo"]=="manual"))' "$X/config/kalango/rotinas.json" "$PJ" 2>/dev/null; }
ok "Instalar agente: fica como rotina com o escopo escolhido (a pasta, sem rede)" 'instalado'
"$N" msg action close-window
sleep 0.8

# 103. The agent's calculator and clock: a calculation is worked out by the calculator tool (exact), and "today" is
#      the system's date (in the instructions of every request)
conta_e_data() { python3 -c 'import json,sys,glob,os
fs=sorted(glob.glob(sys.argv[1]+"/*/eventos.jsonl"), key=os.path.getmtime)
for f in reversed(fs):
    ev=[json.loads(l) for l in open(f)]
    if any(e.get("tipo")=="pessoa" and sys.argv[2] in (e.get("texto") or "") for e in ev):
        usou=any(e.get("tipo")=="ferramenta" and e.get("nome")=="calculadora.calcular" for e in ev)
        resp=" ".join(e.get("texto") or "" for e in ev if e.get("tipo")=="resposta")
        fim=any(e.get("tipo")=="fim" for e in ev)
        resp=resp.replace("\u202f"," ").replace("\u00a0"," ").replace("\u2009"," ").replace("\u2011","-")
        print(int(usou), int(fim), resp.replace("\n"," ")[:300]); break' "$X/data/kalango/conversas" "$1"; }
"$N" msg action spawn -- "$T/kalango-shell" conversar "Quanto e 15% de 230, mais 1234,56 vezes 3?"
for _ in $(seq 150); do read -r USOU FIM RESP < <(conta_e_data "15% de 230"); [ "${FIM:-0}" = 1 ] && break; sleep 1; done
echo "  (calculadora: $USOU; $RESP)"
ok "Agente: uma conta passa pela calculadora e dá o valor exato" '[ "$USOU" = 1 ] && echo "$RESP" | grep -Eq "3[. ]?738,18"'
"$N" msg action close-window
sleep 0.8
HOJE_D=$(date +%-d); HOJE_M=$(LC_TIME=pt_BR.UTF-8 date +%B 2>/dev/null); HOJE_N=$(date +%m)
"$N" msg action spawn -- "$T/kalango-shell" conversar "Que dia e hoje? Responda com a data."
for _ in $(seq 150); do read -r _U FIM RESP < <(conta_e_data "Que dia e hoje"); [ "${FIM:-0}" = 1 ] && break; sleep 1; done
echo "  (hoje: $HOJE_D $HOJE_M; $RESP)"
ok "Agente: “hoje” é a data do sistema" 'echo "$RESP" | grep -Eqi "(^|[^0-9])0?$HOJE_D( de $HOJE_M|/$HOJE_N|/[0-9]?$((10#$HOJE_N)))" || echo "$RESP" | grep -q "$(date +%F)"'
"$N" msg action close-window
sleep 0.8

# 99. An answer that becomes an interface (design "Barra de comando · 3 resposta que vira interface"): asked what fills
#     the disk, the command bar answers with the parts that take the room, measured here; a project's build cache
#     (a Cargo target with its CACHEDIR.TAG) can go — asked twice — and nothing else of the project goes with it
mkdir -p "$X/area/projetos/app/.git" "$X/area/projetos/app/target/release" "$X/area/projetos/app/src"
printf 'Signature: 8a477f597d28d172789f06886806bc55\n' > "$X/area/projetos/app/target/CACHEDIR.TAG"
head -c 3000000 /dev/zero > "$X/area/projetos/app/target/release/app"
echo 'fn main() {}' > "$X/area/projetos/app/src/main.rs"
xdotool key --window "$W" alt+space
sleep 1.5
xdotool type --window "$W" --delay 50 "o que esta ocupando meu disco"
for _ in $(seq 60); do grep -aq "debug comando: interface do disco" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug comando: interface do disco" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-comando-interface.png" >/dev/null
ok "Barra de comando: “o que está ocupando meu disco” vira a interface com o que ocupa, medido aqui" \
  'grep -a "debug comando: interface do disco" "$T/shell.log" | tail -1 | grep -Eq "livres · builds [1-9]"'
sleep 1.5
read -r LX LY < <(lugar "cmd ui liberar")
[ -n "$LX" ] && apontar "$LX" "$LY" && sleep 0.3 && xdotool click 1 && sleep 0.8 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug comando: builds apagados" "$T/shell.log" && break; sleep 0.3; done
ok "Barra de comando: “Liberar” (confirmado) apaga só o cache de build do projeto" \
  '[ ! -e "$X/area/projetos/app/target" ] && [ -s "$X/area/projetos/app/src/main.rs" ] && [ -d "$X/area/projetos/app/.git" ]'
xdotool key --window "$W" Escape
sleep 0.8

fechar_apps
# 36. Files (designs "Arquivos" and its menu; the agent at work in it: "Agente usando a tela"), on a folder of the
#     test area: copies found by their content, the page's preview from the system's thumbnailer (in the shared
#     thumbnail cache), the trash and Ctrl+Z, a tag by dropping a file on it, the agent organising the folder (the
#     banner, the window outlined, the island's progress) and stopped by the banner's button and by Esc
A="$X/area/baixados"
mkdir -p "$A/PDFs"
python3 - "$A" <<'PYEOF'
import os, shutil, struct, sys, time, zlib
d = sys.argv[1]
def pdf(p, linhas):
    c = 'BT /F1 22 Tf 60 770 Td 28 TL ' + ' '.join('(%s) Tj T*' % l for l in linhas) + ' ET'
    objs = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
            '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
            '<< /Length %d >>\nstream\n%s\nendstream' % (len(c), c), '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>']
    out, offs = b'%PDF-1.4\n', []
    for i, o in enumerate(objs, 1):
        offs.append(len(out)); out += ('%d 0 obj\n%s\nendobj\n' % (i, o)).encode()
    x = len(out)
    out += ('xref\n0 %d\n0000000000 65535 f \n' % (len(objs) + 1)).encode() + b''.join(('%010d 00000 n \n' % o).encode() for o in offs)
    out += ('trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objs) + 1, x)).encode()
    open(p, 'wb').write(out)
def png(p, cor):
    w, h = 64, 48
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b) & 0xffffffff)
    raw = b''.join(b'\0' + bytes(cor) * w for _ in range(h))
    open(p, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))
agora = time.time()
pdf(d + '/fatura.pdf', ['Fatura de outubro', 'Total: R$ 189,90']); os.utime(d + '/fatura.pdf', (agora - 30 * 86400,) * 2)
shutil.copy(d + '/fatura.pdf', d + '/fatura-copia.pdf')
png(d + '/foto.png', (40, 60, 120)); os.utime(d + '/foto.png', (agora - 20 * 86400,) * 2)
shutil.copy(d + '/foto.png', d + '/foto(1).png')
png(d + '/noite.png', (10, 12, 30))
open(d + '/notas.md', 'w').write('# Notas\n- cabo HDMI\n')
pdf(d + '/PDFs/extrato.pdf', ['Extrato'])
PYEOF
janela_arq() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Arquivos"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
clique_arq() { sem_avisos; read -r JX JY < <(janela_arq); read -r CX CY < <(lugar "$1"); apontar $((JX + CX)) $((JY + CY)); sleep 0.2; xdotool click "${2:-1}"; }
"$N" msg action spawn -- "$T/kalango-shell" arquivos "$A"
for _ in $(seq 60); do grep -aq "debug arquivos: $A (7 itens" "$T/shell.log" && grep -aq "debug arquivos: miniatura fatura" "$T/shell.log" && break; sleep 0.5; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-arquivos.png" >/dev/null
MINI=$(python3 -c 'import hashlib,sys,urllib.parse; print(hashlib.md5(("file://"+urllib.parse.quote(sys.argv[1])).encode()).hexdigest())' "$A/fatura-copia.pdf")
ok "Arquivos: abre a pasta, acha as cópias pelo conteúdo e a prévia da página vem do thumbnailer do sistema (no cache compartilhado)" \
  'grep -aq "debug arquivos: $A (7 itens, 7 visíveis, 2 duplicados" "$T/shell.log" && [ -s "$X/cache/thumbnails/large/$MINI.png" ] && grep -aq "Thumb::MTime" "$X/cache/thumbnails/large/$MINI.png"'
clique_arq "arquivo fatura-copia.pdf"
for _ in $(seq 20); do grep -aq "debug arquivos: 1 selecionados" "$T/shell.log" && break; sleep 0.3; done
# on this 1280×800 screen the window is short: the compact layout (design "Arquivos · 1366×768"), its details in
# the drawer of the "i" button
clique_arq "arquivos info"
for _ in $(seq 20); do grep -aq "debug arquivos: gaveta fatura-copia.pdf" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-arquivos-compacto.png" >/dev/null
ok "Arquivos: numa tela pequena, o layout compacto; o \"i\" abre a gaveta com o arquivo escolhido" 'grep -aq "debug arquivos: compacto" "$T/shell.log" && grep -aq "debug arquivos: gaveta fatura-copia.pdf" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.5
clique_arq "arquivo fatura-copia.pdf"
sleep 0.5
xdotool key --window "$W" Delete
for _ in $(seq 20); do grep -aq "debug arquivos: lixeira 1" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
NA_LIXEIRA=$([ -e "$X/data/Trash/files/fatura-copia.pdf" ] && grep -q "^Path=.*fatura-copia.pdf" "$X/data/Trash/info/fatura-copia.pdf.trashinfo" && [ ! -e "$A/fatura-copia.pdf" ] && echo sim)
xdotool key --window "$W" ctrl+z
for _ in $(seq 20); do grep -aq "debug arquivos: restaurados 1" "$T/shell.log" && break; sleep 0.3; done
ok "Arquivos: Delete manda para a lixeira (com o registro de onde veio) e Ctrl+Z traz de volta" '[ "$NA_LIXEIRA" = sim ] && [ -e "$A/fatura-copia.pdf" ] && [ ! -e "$X/data/Trash/files/fatura-copia.pdf" ]'
# on a bigger screen, a window of the design's size is the full layout again (the tags are in its side bar); a
# file dropped on a tag gets it (freedesktop's user.xdg.tags). (This screen leaves at most 698 px between the top
# bar and the dock: the session grows to 1600×900 for this, and back.)
read -r SW0 SH0 < <(xdotool getwindowgeometry "$W" | awk '/Geometry/ {split($2, a, "x"); print a[1], a[2]}')
xdotool windowsize "$W" 1600 900
sleep 2
"$N" msg action set-window-width 1320
"$N" msg action set-window-height 800
for _ in $(seq 30); do grep -aq "debug arquivos: completo" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
read -r JX JY < <(janela_arq); read -r NX NY < <(lugar "arquivo notas.md"); read -r EX EY < <(lugar "etiqueta Trabalho")
xdotool mousemove --window "$W" $((JX + NX)) $((JY + NY)) mousedown 1
for i in 1 2 3 4 5 6 7 8 9 10; do xdotool mousemove --window "$W" $((JX + NX + (EX - NX) * i / 10)) $((JY + NY + (EY - NY) * i / 10)); sleep 0.08; done
sleep 0.4; xdotool mouseup 1
for _ in $(seq 20); do grep -aq "debug arquivos: etiqueta Trabalho posta em 1" "$T/shell.log" && break; sleep 0.3; done
ok "Arquivos: um arquivo solto numa tag ganha a tag (no próprio arquivo)" 'python3 -c "import os,sys; sys.exit(os.getxattr(sys.argv[1], \"user.xdg.tags\") != b\"Trabalho\")" "$A/notas.md"'
# a file dropped on a folder moves into it (the same disk; Ctrl would copy)
sleep 1
read -r JX JY < <(janela_arq); read -r FX FY < <(lugar "arquivo foto(1).png"); read -r PX PY < <(lugar "arquivo PDFs")
xdotool mousemove --window "$W" $((JX + FX)) $((JY + FY)) mousedown 1
for i in 1 2 3 4 5 6 7 8 9 10; do xdotool mousemove --window "$W" $((JX + FX + (PX - FX) * i / 10)) $((JY + FY + (PY - FY) * i / 10)); sleep 0.08; done
sleep 0.4; xdotool mouseup 1
for _ in $(seq 20); do grep -aq "debug arquivos: movidos 1" "$T/shell.log" && break; sleep 0.3; done
ok "Arquivos: um arquivo solto numa pasta vai para ela" '[ -e "$A/PDFs/foto(1).png" ] && [ ! -e "$A/foto(1).png" ]'
# 48. Files' menus and dialogs (design "Arquivos · menus completos", "Propriedades do arquivo", "Conflito de
#     arquivos"): Alt+Enter shows a file's properties; a paste over a file of the same name asks first, and "Replace"
#     sends the old one to the trash; the window's layout holds (nothing grown past its design's width)
sleep 1
clique_arq "arquivo notas.md"
sleep 0.4
xdotool key --window "$W" alt+Return
for _ in $(seq 20); do grep -aq "debug arquivos: propriedades notas.md" "$T/shell.log" && break; sleep 0.3; done
ok "Arquivos: Alt+Enter mostra as propriedades do arquivo" 'grep -aq "debug arquivos: propriedades notas.md" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.6
L0=$(grep -ac "debug layout: .* janelas, " "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" auditar-layout
for _ in $(seq 20); do [ "$(grep -ac "debug layout: .* janelas, " "$T/shell.log")" -gt "$L0" ] && break; sleep 0.3; done
ok "Arquivos: nenhuma parte de largura fixa cresceu além do design (auditoria de layout)" 'grep -a "debug layout: .* janelas, " "$T/shell.log" | tail -1 | grep -q " 0 problemas"'
clique_arq "arquivo notas.md"
sleep 0.4
xdotool key --window "$W" ctrl+c
sleep 0.4
clique_arq "arquivo PDFs"
xdotool click --repeat 2 --delay 80 1
for _ in $(seq 20); do grep -aq "debug arquivos: $A/PDFs (" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
xdotool key --window "$W" ctrl+v
for _ in $(seq 20); do [ -e "$A/PDFs/notas.md" ] && break; sleep 0.3; done
sleep 0.8
xdotool key --window "$W" ctrl+v
for _ in $(seq 20); do grep -aq "debug arquivos: conflito notas.md (1 de 1)" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
clique_arq "conflito substituir"
for _ in $(seq 20); do grep -aq "debug arquivos: conflito substituir" "$T/shell.log" && break; sleep 0.3; done
sleep 1
ok "Arquivos: colar sobre um nome igual pergunta antes; Substituir manda o antigo para a lixeira" \
  'grep -aq "debug arquivos: conflito notas.md (1 de 1)" "$T/shell.log" && [ "$(ls "$A/PDFs" | grep -c "^notas")" = 1 ] && ls "$X/data/Trash/files" | grep -q "^notas"'
clique_arq "arquivo notas.md"
sleep 0.4
xdotool key --window "$W" BackSpace
for _ in $(seq 20); do grep -a "debug arquivos: /" "$T/shell.log" | tail -1 | grep -q "debug arquivos: $A (" && break; sleep 0.3; done
ok "Arquivos: Backspace volta para a pasta de cima" 'grep -a "debug arquivos: /" "$T/shell.log" | tail -1 | grep -q "debug arquivos: $A ("'
xdotool windowsize "$W" "$SW0" "$SH0"
sleep 2
"$N" msg action set-window-width 1232
"$N" msg action set-window-height 684
for _ in $(seq 30); do [ "$(grep -ac "debug arquivos: compacto" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
# the agent organises the folder: the banner and the outline while it works, the files in subfolders at the end
sleep 1
# (the folder's menu: a right click on an empty part of it, low in the window above the status line)
read -r FX FY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Arquivos"][0]["layout"]; x=l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]; y=l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]; w,h=l["window_size"]; print(int(x+w*0.5), int(y+h-80))')
sem_avisos
apontar "$FX" "$FY"; sleep 0.2; xdotool click 3
sleep 1.2
clique_arq "menu pasta organizar"
for _ in $(seq 30); do grep -aq "debug usando: Arquivos (0/" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-arquivos-agente.png" >/dev/null
for _ in $(seq 480); do grep -aq "debug usando: terminou" "$T/shell.log" && break; sleep 0.5; done
SOLTOS=$(find "$A" -maxdepth 1 -type f | wc -l)
EM_PASTAS=$(find "$A" -mindepth 2 -type f | wc -l)
ok "Arquivos: \"Organizar esta pasta\" — o agente (LFM3) põe os arquivos em subpastas, com a faixa e o contorno enquanto trabalha" \
  'grep -aq "debug arquivos: agente usando (1/" "$T/shell.log" && grep -aq "debug usando: terminou" "$T/shell.log" && [ "$SOLTOS" -le 3 ] && [ "$EM_PASTAS" -ge 5 ]'
# stopped by the banner's button, and by Esc in the window
for como in botao esc; do
  N0=$(grep -ac "debug usando: parado pela pessoa" "$T/shell.log")
  U0=$(grep -ac "debug usando: Arquivos (0/" "$T/shell.log")
  for f in a b c d; do printf 'nota %s\n' "$f" > "$A/solta-$f.txt"; done
  sleep 1.5
  # (an empty part of the folder: low in the window, above the status line)
  read -r FX FY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Arquivos"][0]["layout"]; x=l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]; y=l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]; w,h=l["window_size"]; print(int(x+w*0.5), int(y+h-80))')
  xdotool mousemove --window "$W" "$FX" "$FY" click 3
  sleep 1.2
  clique_arq "menu pasta organizar"
  for _ in $(seq 30); do [ "$(grep -ac "debug usando: Arquivos (0/" "$T/shell.log")" -gt "$U0" ] && break; sleep 0.3; done
  sleep 1
  if [ "$como" = botao ]; then
    read -r PX PY < <(grep -a "debug usando: parar na tela" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
    xdotool mousemove --window "$W" "$PX" "$PY" click 1
  else
    xdotool key --window "$W" Escape
  fi
  for _ in $(seq 20); do [ "$(grep -ac "debug usando: parado pela pessoa" "$T/shell.log")" -gt "$N0" ] && break; sleep 0.3; done
  sleep 1
  eval "PARADO_$como=\$(grep -ac \"debug usando: parado pela pessoa\" \"$T/shell.log\")"
  eval "N0_$como=$N0"
  sleep 4
done
ok "Arquivos: o agente para pelo botão da faixa e pelo Esc da janela" '[ "$PARADO_botao" -gt "$N0_botao" ] && [ "$PARADO_esc" -gt "$N0_esc" ]'

# 37. a long copy (design "Operação de arquivos (cópia em andamento)"): it goes to the island, whose card has the
#     speed and the time left, Pause and Cancel (nothing half written stays); when it ends, a notice opens the folder
#     it went to. (The session's disk is made slow for this: KALANGO_TESTE_COPIA_LENTA, 8 MB/s — about 15 s here.)
head -c 120M /dev/zero > "$A/grande.bin"
mkdir -p "$A/destino"
sleep 1.5
clique_arq "arquivo grande.bin"
sleep 0.4
xdotool key --window "$W" ctrl+c
read -r JX JY < <(janela_arq); read -r DX DY < <(lugar "arquivo destino")
xdotool mousemove --window "$W" $((JX + DX)) $((JY + DY)); sleep 0.2; xdotool click --repeat 2 --delay 90 1
for _ in $(seq 30); do grep -aq "debug arquivos: $A/destino (" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
xdotool key --window "$W" ctrl+v
for _ in $(seq 40); do grep -aq "debug central: ilha operacao em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
read -r IX IY < <(lugar "ilha operacao")
xdotool mousemove --window "$W" "$IX" "$IY" click 1
for _ in $(seq 30); do grep -aq "debug operacao: operacao pausar na tela" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-copia.png" >/dev/null
na_tela() { grep -a "debug operacao: operacao $1 na tela" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
read -r BX BY < <(na_tela pausar); xdotool mousemove --window "$W" "$BX" "$BY" click 1
for _ in $(seq 20); do grep -aq "debug operacao: pausada" "$T/shell.log" && break; sleep 0.2; done
sleep 0.6; T1=$(stat -c %s "$A/destino/grande.bin" 2>/dev/null); sleep 1.5; T2=$(stat -c %s "$A/destino/grande.bin" 2>/dev/null)
xdotool mousemove --window "$W" "$BX" "$BY" click 1
for _ in $(seq 20); do grep -aq "debug operacao: retomada" "$T/shell.log" && break; sleep 0.2; done
sleep 1.2; T3=$(stat -c %s "$A/destino/grande.bin" 2>/dev/null)
read -r CX CY < <(na_tela cancelar); xdotool mousemove --window "$W" "$CX" "$CY" click 1
for _ in $(seq 30); do grep -aq "debug operacao: terminou (.*cancelada)" "$T/shell.log" && break; sleep 0.3; done
ok "Cópia longa: vai para a ilha, o cartão pausa (nada mais é escrito), retoma e cancela (sem arquivo pela metade)" \
  '[ -n "$T1" ] && [ "$T1" = "$T2" ] && [ "$T3" -gt "$T2" ] && grep -aq "debug operacao: terminou (0 erros, cancelada)" "$T/shell.log" && [ ! -e "$A/destino/grande.bin" ]'
# again, to the end: the notice's "Mostrar na pasta" opens the folder it went to (from another folder). (Files gets
# the keyboard back first: a click on an empty part of it)
foco_arq() { read -r FX FY < <("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Arquivos"][0]["layout"]; x=l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]; y=l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]; w,h=l["window_size"]; print(int(x+w*0.5), int(y+h-80))'); xdotool mousemove --window "$W" "$FX" "$FY" click 1; sleep 0.4; }
foco_arq
xdotool key --window "$W" ctrl+v
sleep 1
xdotool key --window "$W" BackSpace
for _ in $(seq 90); do grep -aq "debug operacao: terminou (0 erros, completa)" "$T/shell.log" && break; sleep 0.5; done
for _ in $(seq 20); do grep -aq "debug aviso: primeira ação em .* :: Cópia concluída" "$T/shell.log" && break; sleep 0.3; done
N_DEST=$(grep -ac "debug arquivos: $A/destino (" "$T/shell.log")
read -r MX MY < <(grep -a "debug aviso: primeira ação em .* :: Cópia concluída" "$T/shell.log" | tail -1 | awk '{print $6, $7}')
xdotool mousemove --window "$W" "$MX" "$MY" click 1
for _ in $(seq 20); do [ "$(grep -ac "debug arquivos: $A/destino (" "$T/shell.log")" -gt "$N_DEST" ] && break; sleep 0.3; done
ok "Cópia longa: termina inteira e o aviso \"Mostrar na pasta\" abre o Arquivos onde ela foi parar" \
  '[ "$(stat -c %s "$A/destino/grande.bin")" = "$(stat -c %s "$A/grande.bin")" ] && [ "$(grep -ac "debug arquivos: $A/destino (" "$T/shell.log")" -gt "$N_DEST" ]'
rm -f "$A/grande.bin" "$A/destino/grande.bin"
"$N" msg action close-window
fechar_apps
# 38. Settings (designs "Configurações · Início" and "· 1366×768"): on this screen the compact layout, with what
#     the system reports (the host's name, the power profile, the disk — read-only: the test never changes the real
#     Wi-Fi, sound or profile); the updates card schedules the prepared update for tonight; on a bigger screen the
#     full layout, whose suggestion (the person uses the computer at night) turns the night light's schedule on and
#     its switch off again; and at night, with the screen locked, the computer restarts into the update
python3 - "$X/state/kalango/uso-apps.json" <<'PYEOF'
import json, sys, time
p = sys.argv[1]
try: v = json.load(open(p))
except Exception: v = []
agora = time.time()
for d in range(1, 6):
    t = agora - d * 86400; lt = time.localtime(t)
    v.append(["os.kalango.Arquivos", int(t - (lt.tm_hour * 3600 + lt.tm_min * 60 + lt.tm_sec) + 21 * 3600)])
json.dump(v, open(p, "w"))
PYEOF
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/packagekit-teste.py" 2
sleep 1.5
"$N" msg action spawn -- "$T/kalango-shell" configuracoes
for _ in $(seq 40); do grep -aq "debug configuracoes: lido" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
CFG="$X/config/kalango/configuracoes.json"
# (a control's place after the window changed: wait until the session logs it somewhere new, or ~9 s)
nova_posicao() { local v0; v0=$(lugar "$1"); for _ in $(seq 30); do [ "$(lugar "$1")" != "$v0" ] && break; sleep 0.3; done; sleep 0.5; }
janela_cfg() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
clique_cfg() { sem_avisos; read -r JX JY < <(janela_cfg); read -r CX CY < <(lugar "$1"); apontar $((JX + CX + ${2:-0})) $((JY + CY)); sleep 0.3; xdotool click 1; sleep 1; }
# a drop-down: clicked, and only once its list has the keyboard do the keys go (else they reach the control above it)
lista_cfg() { local n0; n0=$(grep -ac "debug configuracoes: foco em GtkListItemWidget" "$T/shell.log"); clique_cfg "$1"
  for _ in $(seq 20); do [ "$(grep -ac "debug configuracoes: foco em GtkListItemWidget" "$T/shell.log")" -gt "$n0" ] && break; sleep 0.2; done; sleep 0.3; }
# a choice in a drop-down, tried again (up to 3 times) when its list closed before the keys: the pointer wakes the
# screen first (the test's idle times are seconds); $2 the keys, $3 the line that says it was chosen
escolher_cfg() { for _ in 1 2 3; do xdotool mousemove --window "$W" 300 300; sleep 0.4; lista_cfg "$1"; xdotool key --window "$W" $2
  for _ in $(seq 10); do grep -aq "$3" "$T/shell.log" && return 0; sleep 0.3; done; xdotool key --window "$W" Escape; sleep 0.5; done; return 1; }
LIDO=$(grep -a "debug configuracoes: lido" "$T/shell.log" | tail -1)
echo "  ($LIDO)"
PERFIL=$(KALANGO_ENERGIA_ESTADO="$OUT/energia.json" "$ROOT/sessao/ferramentas/powerprofilesctl-teste" get 2>/dev/null || true)
DISCO=$(df -B1 --output=size / | tail -1 | tr -d ' ')
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-configuracoes.png" >/dev/null
ok "Configurações: abre como app (os.kalango.Configuracoes), compacta nesta tela, com o nome, o perfil de energia e o disco do sistema" \
  '"$N" msg -j windows | grep -q "\"os.kalango.Configuracoes\"" && grep -aq "debug configuracoes: compacto" "$T/shell.log" && echo "$LIDO" | grep -q "host=$(cat /proc/sys/kernel/hostname) perfil=$PERFIL disco=[0-9]*/$DISCO "'
clique_cfg "cfg resumo 5"
for _ in $(seq 20); do grep -aq "debug configuracoes: instalar à noite" "$T/shell.log" && break; sleep 0.3; done
ok "Configurações: o cartão de atualizações agenda a atualização preparada para esta noite" \
  'grep -aq "debug configuracoes: instalar à noite" "$T/shell.log" && python3 -c "import json,sys; sys.exit(not json.load(open(sys.argv[1])).get(\"instalar_a_partir\"))" "$CFG"'
# the full layout on a bigger screen (and back)
read -r SW0 SH0 < <(xdotool getwindowgeometry "$W" | awk '/Geometry/ {split($2, a, "x"); print a[1], a[2]}')
xdotool windowsize "$W" 1600 900
sleep 2
"$N" msg action set-window-width 1520
"$N" msg action set-window-height 710
for _ in $(seq 30); do grep -aq "debug configuracoes: completo" "$T/shell.log" && break; sleep 0.3; done
sleep 4
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-configuracoes-completa.png" >/dev/null
clique_cfg "cfg sugestao aplicar"
for _ in $(seq 20); do grep -aq "debug configuracoes: luz noturna automática true" "$T/shell.log" && break; sleep 0.3; done
noturna_auto() { python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1])).get(\"luz_noturna_auto\") is not $1)" "$CFG"; }
# (the suggestion goes away: the cards move up)
nova_posicao "cfg luz noturna"
ok "Configurações: numa tela maior o layout completo; a sugestão (usa o computador à noite) liga a Luz noturna das 20h às 6h" \
  'grep -aq "debug configuracoes: completo" "$T/shell.log" && noturna_auto True'
sleep 3.5
clique_cfg "cfg luz noturna"
for _ in $(seq 20); do grep -aq "debug configuracoes: luz noturna automática false" "$T/shell.log" && break; sleep 0.3; done
ok "e a chave da Luz noturna desliga o agendamento (e nenhum wlsunset fica rodando)" 'noturna_auto False && ! noturna'
xdotool windowsize "$W" "$SW0" "$SH0"
sleep 2
# at night, locked, no agent working: the computer restarts into the update (the test session only records it)
python3 -c "import json,sys,time; p=sys.argv[1]; v=json.load(open(p)); v['instalar_a_partir']=int(time.time())-60; json.dump(v,open(p,'w'))" "$CFG"
"$N" msg action spawn -- "$T/kalango-shell" bloquear
for _ in $(seq 30); do grep -aq "teste\] .reiniciar. (com a atualização preparada)" "$T/shell.log" && break; sleep 0.5; done
ok "Configurações: à noite, com a tela bloqueada, o computador reinicia para instalar a atualização agendada" \
  'grep -aq "debug configuracoes: instalando a atualização à noite (2 pacotes)" "$T/shell.log" && grep -aq "teste\] .reiniciar. (com a atualização preparada)" "$T/shell.log"'
sleep 1
xdotool key --window "$W" space
sleep 1
xdotool type --window "$W" --delay 60 kalango-teste
xdotool key --window "$W" Return
for _ in $(seq 20); do [ "$(grep -ac "debug bloqueio: desbloqueado" "$T/shell.log")" -ge 2 ] && break; sleep 0.5; done
pkill -f "[p]ackagekit-teste.py 2"
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
# 39. Settings › Appearance (design "Configurações · Aparência"): "Claro" turns the whole shell light at once (the
#     style sheet and the wallpaper's light version), a swatch becomes the shell's accent, the dock goes to the side,
#     the widgets leave the wallpaper for cards — each kept in its settings — and all of it back
"$N" msg action spawn -- "$T/kalango-shell" configuracoes aparencia
for _ in $(seq 30); do grep -aq "debug configuracoes: página aparencia" "$T/shell.log" && break; sleep 0.3; done
sleep 2
TEMA="$X/config/kalango/tema.json"
tema_tem() { python3 -c "import json,sys; v=json.load(open(sys.argv[1])); sys.exit(not all(str(v.get(k))==x for k,x in (a.split('=') for a in sys.argv[2:])))" "$TEMA" "$@"; }
clique_cfg "cfg aparencia claro"
for _ in $(seq 20); do grep -aq "debug tema: claro accent" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
"$N" msg action screenshot-screen --path "$OUT/aparencia-claro.png" >/dev/null; sleep 1
read -r JX JY < <(janela_cfg)
CLARA=$(python3 -c "
from PIL import Image; import sys
im=Image.open(sys.argv[1]).convert('L'); x,y=int(sys.argv[2]),int(sys.argv[3])
print(sum(im.getpixel((x+300+i*7, y+600)) for i in range(40))//40)" "$OUT/aparencia-claro.png" "$JX" "$JY" 2>/dev/null || echo 0)
echo "  (brilho da janela no tema claro: $CLARA)"
ok "Aparência: \"Claro\" deixa o shell inteiro claro na hora (folha de estilo e o papel de parede claro)" \
  'tema_tem estilo=claro && [ "${CLARA:-0}" -gt 200 ] && grep -aq "debug papel: swaybg .*-claro.png" "$T/shell.log"'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && cp "$OUT/aparencia-claro.png" "${KALANGO_TESTE_CAPTURA%.png}-aparencia-clara.png"
# the apps follow it too: KalangoOS's portal tells GTK 4 / libadwaita apps (org.freedesktop.appearance) light now
aparencia() { "$N" msg action spawn -- sh -c "gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop --method org.freedesktop.portal.Settings.ReadOne org.freedesktop.appearance color-scheme > $OUT/aparencia-$1.txt 2>&1"; sleep 1.5; cat "$OUT/aparencia-$1.txt" 2>/dev/null; }
echo "  (portal no tema claro: $(aparencia claro))"
ok "Aparência: os apps GTK ganham as cores do KalangoOS (kalangoos.css importada no gtk.css do perfil) no tema claro" \
  'head -1 "$X/config/gtk-4.0/gtk.css" | grep -q "kalangoos.css" && grep -q "window_bg_color" "$X/config/gtk-4.0/kalangoos.css" && grep -aq "debug tema dos apps: claro" "$T/shell.log"' 
ok "Aparência: os apps GTK seguem o tema claro pelo portal do KalangoOS (color-scheme 2)" 'grep -q "uint32 2" "$OUT/aparencia-claro.txt" && grep -aq "debug portal: aparência color-scheme = 2" "$T/shell.log"'
clique_cfg "cfg aparencia teal"
for _ in $(seq 20); do grep -aq "debug tema: claro teal" "$T/shell.log" && break; sleep 0.3; done
clique_cfg "cfg aparencia transparentes"
for _ in $(seq 20); do grep -aq "widgets em cartões" "$T/shell.log" && break; sleep 0.3; done
# the dock last: at the side it takes room at the left, and the window may change size (its controls move)
clique_cfg "cfg aparencia lateral"
for _ in $(seq 20); do grep -aq "debug dock: lateral" "$T/shell.log" && break; sleep 0.3; done
nova_posicao "cfg aparencia flutuante"
ok "Aparência: a cor de destaque, a dock na lateral e os widgets em cartões valem na hora e ficam guardados" \
  'tema_tem destaque=teal widgets_transparentes=False && grep -q "\"lateral\"" "$X/config/kalango/dock-estilo.json" && grep -aq "debug dock: lateral 40 px" "$T/shell.log"'
clique_cfg "cfg aparencia flutuante"
for _ in $(seq 20); do grep -aq "debug dock: flutuante" "$T/shell.log" && break; sleep 0.3; done
nova_posicao "cfg aparencia escuro"
for n in escuro lavanda transparentes; do clique_cfg "cfg aparencia $n"; sleep 1.5; done
for _ in $(seq 20); do grep -aq "debug tema: escuro accent cantos 6 widgets transparentes" "$T/shell.log" && break; sleep 0.3; done
ok "e tudo volta: escuro, lavanda, dock flutuante, widgets no papel de parede" \
  'tema_tem estilo=escuro destaque=accent widgets_transparentes=True && grep -q "\"flutuante\"" "$X/config/kalango/dock-estilo.json" && grep -a "debug papel: swaybg" "$T/shell.log" | tail -1 | grep -q "sdrustos-16x9-4k.png"'
echo "  (portal de volta no escuro: $(aparencia escuro))"
ok "Aparência: e de volta ao escuro, os apps também (color-scheme 1)" 'grep -q "uint32 1" "$OUT/aparencia-escuro.txt"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
# 40. Settings › Network (design "Configurações · Rede"), against a stand-in NetworkManager (the computer's own is never
#     touched): the networks as cards (connected, saved, open, protected) and the connected one's details; "Rede
#     limitada" changes the connection; without the agents, kalangod refuses the agent's web reading on that network;
#     a protected network wants its password (a wrong one is refused); "Esquecer rede" forgets it
NMLOG="$OUT/nmcli.json.log"
"$N" msg action spawn -- "$T/kalango-shell" configuracoes rede
for _ in $(seq 40); do grep -aq "debug rede: 4 redes, em uso Lunar-Casa_5G" "$T/shell.log" && break; sleep 0.3; done
sleep 2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-rede.png" >/dev/null
ok "Rede: as redes por perto como cartões (conectada, salva, aberta, protegida) e a conectada no painel" 'grep -aq "debug rede: 4 redes, em uso Lunar-Casa_5G" "$T/shell.log"'
clique_cfg "cfg rede limitada"
for _ in $(seq 20); do grep -q "connection modify Lunar-Casa_5G connection.metered yes" "$NMLOG" && break; sleep 0.3; done
clique_cfg "cfg rede agentes"
for _ in $(seq 20); do grep -aq "debug rede: agentes proibidos em Lunar-Casa_5G: ok" "$T/shell.log" && break; sleep 0.3; done
WEB=$(python3 "$CHAMAR" "$T/mcp.sock" web.ler '{"url": "https://exemplo.invalid/"}' 2>&1)
echo "  (web.ler nesta rede: $(echo "$WEB" | tr '\n' ' ' | cut -c1-160))"
ok "Rede: \"Rede limitada\" muda a conexão; sem os agentes, o kalangod recusa a leitura de páginas do agente nesta rede" \
  'grep -q "connection modify Lunar-Casa_5G connection.metered yes" "$NMLOG" && echo "$WEB" | grep -q "Lunar-Casa_5G"'
clique_cfg "cfg rede agentes"
for _ in $(seq 20); do grep -aq "debug rede: agentes permitidos em Lunar-Casa_5G: ok" "$T/shell.log" && break; sleep 0.3; done
# a protected network: its password in the system's dialog (design "Senha do Wi-Fi") — "Conectar" waits for a
# password the network can take, a wrong one brings the dialog back in coral, the right one connects (and is saved)
clique_cfg "cfg rede cartao Vizinho 2.4"
sleep 0.8
clique_cfg "cfg rede conectar"
for _ in $(seq 20); do grep -aq "debug wifi: pedindo Vizinho 2.4 (WPA2)" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
xdotool type --window "$W" --delay 40 "curta"; xdotool key --window "$W" Return
sleep 0.6
CURTA=$(grep -ac "debug wifi: respondido Vizinho 2.4" "$T/shell.log")
xdotool key --window "$W" ctrl+a; xdotool type --window "$W" --delay 30 "senha-errada"; xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug wifi: pedindo Vizinho 2.4 (WPA2, senha recusada)" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x wifi-senha-recusada
xdotool type --window "$W" --delay 30 "senha-certa"; xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug rede: conectada Vizinho 2.4" "$T/shell.log" && break; sleep 0.3; done
ok "Rede: uma rede protegida pede a senha no diálogo do sistema — curta não vai, a errada volta em coral, a certa conecta" \
  '[ "$CURTA" = 0 ] && grep -aq "debug rede: não conectou Vizinho 2.4" "$T/shell.log" && grep -aq "debug wifi: pedindo Vizinho 2.4 (WPA2, senha recusada)" "$T/shell.log" && grep -aq "debug rede: conectada Vizinho 2.4" "$T/shell.log" && grep -q "wifi connect Vizinho 2.4 password senha-certa" "$NMLOG"'
sleep 3.5
clique_cfg "cfg rede esquecer"
for _ in $(seq 20); do grep -aq "debug rede: esquecida Vizinho 2.4" "$T/shell.log" && break; sleep 0.3; done
ok "Rede: \"Esquecer rede\" apaga a rede salva" 'grep -q "connection delete Vizinho 2.4" "$NMLOG" && ! python3 -c "import json,sys; sys.exit(0 if any(c[\"nome\"]==\"Vizinho 2.4\" for c in json.load(open(sys.argv[1]))[\"conexoes\"]) else 1)" "$OUT/nmcli.json"'
# the Ethernet tab, as the Wi-Fi one (design "Configurações · Rede · Ethernet"): this computer's port as a card, a
# container's interface and Docker's bridge behind "Mostrar interfaces virtuais"; the keyboard's arrows move the focus
V0=$(grep -ac "debug central: cfg rede virtuais em" "$T/shell.log")
clique_cfg "cfg aba ethernet"
for _ in $(seq 20); do grep -aq "debug rede: aba ethernet" "$T/shell.log" && [ "$(grep -ac "debug central: cfg rede virtuais em" "$T/shell.log")" -gt "$V0" ] && break; sleep 0.3; done
ok "Rede › Ethernet: as portas do computador como cartões; as interfaces de contêineres ficam atrás de um link" 'grep -aq "debug rede: aba ethernet" "$T/shell.log" && grep -aq "debug rede: cabos 1 físicos, 2 virtuais" "$T/shell.log"'
sleep 0.5
clique_cfg "cfg rede virtuais"
for _ in $(seq 20); do grep -aq "debug rede: cabos 1 físicos, 2 virtuais (à vista)" "$T/shell.log" && break; sleep 0.3; done
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-rede-ethernet.png" >/dev/null
ok "Rede › Ethernet: \"Mostrar interfaces virtuais\" mostra as dos contêineres" 'grep -aq "debug rede: cabos 1 físicos, 2 virtuais (à vista)" "$T/shell.log"'
clique_cfg "cfg aba ethernet"
sleep 0.5
FOCO0=$(grep -ac "debug configuracoes: foco em" "$T/shell.log")
for k in Right Right Down; do xdotool key --window "$W" "$k"; sleep 0.4; done
grep -a "debug configuracoes: foco em" "$T/shell.log" | tail -4 | sed 's/^/  /'
ok "o teclado navega nas Configurações: as setas levam o foco de um controle a outro" '[ "$(grep -ac "debug configuracoes: foco em" "$T/shell.log")" -ge $((FOCO0 + 2)) ]'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
# 41. Settings › Bluetooth (design "Configurações · Bluetooth"), against a stand-in BlueZ (the computer's own is never
#     touched): the person's devices (connected with their battery, a phone paired but away) and those a scan finds
#     nearby with "Parear"; pairing one makes it the person's (paired, trusted, connected); the chosen one disconnects
#     and is forgotten
BTLOG="$OUT/bluetoothctl.json.log"
BZ="$OUT/bluez"; mkdir -p "$BZ"
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/bluez-teste.py" "$BZ"
for _ in $(seq 40); do grep -q "agente /os/kalango/BluezAgent KeyboardDisplay" "$BZ/bluez.log" 2>/dev/null && break; sleep 0.3; done
"$N" msg action spawn -- "$T/kalango-shell" configuracoes bluetooth
for _ in $(seq 40); do grep -aq "debug bluetooth: 3 meus" "$T/shell.log" && break; sleep 0.3; done
ok "Bluetooth: os aparelhos da pessoa (os conectados com a bateria, o celular pareado e longe)" 'grep -aq "debug bluetooth: 3 meus, 0 por perto, 2 conectados" "$T/shell.log"'
for _ in $(seq 60); do grep -aq "debug central: cfg bt parear TV da sala em" "$T/shell.log" && break; sleep 0.5; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-bluetooth.png" >/dev/null
clique_cfg "cfg bt parear TV da sala"
for _ in $(seq 30); do grep -aq "debug bluetooth: pareado TV da sala" "$T/shell.log" && break; sleep 0.3; done
ok "Bluetooth: a busca acha a TV por perto e \"Parear\" a pareia, confia nela e conecta" \
  'grep -q "Pair 34:FC:99:13:52:32" "$BZ/bluez.log" && grep -q "^trust 34:FC:99:13:52:32" "$BTLOG" && grep -q "^connect 34:FC:99:13:52:32" "$BTLOG"'
sleep 3.5
clique_cfg "cfg bt conectar"
for _ in $(seq 20); do grep -q "^disconnect 34:FC:99:13:52:32" "$BTLOG" && break; sleep 0.3; done
sleep 3.5
clique_cfg "cfg bt esquecer"
for _ in $(seq 20); do grep -q "^remove 34:FC:99:13:52:32" "$BTLOG" && break; sleep 0.3; done
ok "Bluetooth: o aparelho escolhido desconecta e é esquecido" 'grep -q "^disconnect 34:FC:99:13:52:32" "$BTLOG" && grep -q "^remove 34:FC:99:13:52:32" "$BTLOG"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
# 42. Settings › Display (design "Configurações · Tela"): the session's monitor as niri has it; a new scale applies at
#     once and, unanswered, goes back by itself after 15 s ("Manter estas configurações?"); kept, it is saved (and
#     applied again when the session starts)
saida_escala() { "$N" msg -j outputs | python3 -c 'import json,sys; print(list(json.load(sys.stdin).values())[0]["logical"]["scale"])'; }
# (a click while the screen is at another scale: niri's places are logical, the X display's are pixels)
clique_escala() { read -r JX JY < <(janela_cfg); read -r CX CY < <(lugar "$1"); xdotool mousemove --window "$W" $(( (JX + CX) * $2 / 100 )) $(( (JY + CY) * $2 / 100 )); sleep 0.3; xdotool click 1; sleep 1; }
"$N" msg action spawn -- "$T/kalango-shell" configuracoes tela
for _ in $(seq 40); do grep -aq "debug tela: winit" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-tela.png" >/dev/null
ok "Tela: o monitor da sessão como o niri o tem" 'grep -aq "debug tela: winit [0-9]*x[0-9]*+0+0 escala 1 " "$T/shell.log"'
clique_cfg "cfg tela escala 125"
for _ in $(seq 20); do [ "$(saida_escala)" = 1.25 ] && break; sleep 0.3; done
E1=$(saida_escala)
for _ in $(seq 60); do grep -aq "debug tela: voltou sozinho" "$T/shell.log" && break; sleep 0.5; done
sleep 1.5
E2=$(saida_escala)
echo "  (escala aplicada: $E1; sem resposta, voltou para $E2)"
ok "Tela: a escala nova vale na hora e, sem resposta, volta sozinha depois de 15 s" '[ "$E1" = 1.25 ] && [ "$E2" = 1.0 ]'
sleep 3.5
clique_cfg "cfg tela escala 125"
for _ in $(seq 20); do [ "$(saida_escala)" = 1.25 ] && break; sleep 0.3; done
sleep 2
clique_escala "cfg tela manter" 125
for _ in $(seq 20); do grep -aq "debug tela: mantido" "$T/shell.log" && break; sleep 0.3; done
ok "Tela: \"Manter\" guarda a escala (aplicada de novo quando a sessão começa)" \
  'grep -aq "debug tela: mantido" "$T/shell.log" && python3 -c "import json,sys; v=json.load(open(sys.argv[1])); sys.exit(not any(abs(s.get(\"escala\",0)-1.25)<0.01 for s in v[\"saidas\"].values()))" "$X/config/kalango/telas.json"'
# back to 100 % (through niri: at 125 % the window is wider than this small screen), for what comes after
"$N" msg output "$("$N" msg -j outputs | python3 -c 'import json,sys; print(list(json.load(sys.stdin))[0])')" scale 1
sleep 1.5
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
# 43. Settings › Sound (design "Configurações · Som"), against stand-in PipeWire commands (the computer's own sound and
#     microphone are never opened): the outputs and inputs as cards, the one in use outlined, the microphone's level
#     read while the page shows; "Áudio espacial" for the headphones runs PipeWire's filter (a process of its own) and
#     makes it the output, and off it ends; a click makes the speakers the default; the volume slider sets it; "Baixar a música
#     durante chamadas" is kept; closed, the page stops reading the microphone
PWLOG="$OUT/pipewire.json.log"
"$N" msg action spawn -- "$T/kalango-shell" configuracoes som
for _ in $(seq 40); do grep -aq "debug som: 3 saídas, 2 entradas, 3 apps" "$T/shell.log" && break; sleep 0.3; done
sleep 2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-som.png" >/dev/null
ok "Som: as saídas e as entradas como cartões (a em uso marcada), e o nível do microfone lido enquanto a página aparece" \
  'grep -aq "debug som: 3 saídas, 2 entradas, 3 apps; saída bluez_output.C8_24_78" "$T/shell.log" && grep -aq "debug som: medidor na entrada 70" "$T/shell.log" && grep -q "pw-record --target 70" "$PWLOG"'
clique_cfg "cfg som espacial"
for _ in $(seq 30); do grep -aq "debug som: áudio espacial ligado" "$T/shell.log" && break; sleep 0.3; done
ok "Som: \"Áudio espacial\" do fone roda um filtro do PipeWire (processo próprio) e faz dele a saída" \
  'grep -aq "debug som: áudio espacial ligado (bluez_output.C8_24_78)" "$T/shell.log" && grep -q "^pipewire -c .*audio-espacial.conf" "$PWLOG" && grep -q "wpctl set-default 95" "$PWLOG" && grep -q "target.object = \"bluez_output.C8_24_78\"" "$X/state/kalango/audio-espacial.conf"'
sleep 2.5
clique_cfg "cfg som espacial"
for _ in $(seq 30); do grep -aq "debug som: áudio espacial desligado" "$T/shell.log" && break; sleep 0.3; done
sleep 1
ok "e desligado, o fone volta a ser a saída e o filtro termina" \
  'grep -q "wpctl set-default 60" "$PWLOG" && ! pgrep -f "pipewire-teste -c $X/state" >/dev/null && python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1]))[\"saida\"]!=\"bluez_output.C8_24_78\")" "$OUT/pipewire.json"'
sleep 1.5
clique_cfg "cfg som cartao Alto-falantes"
for _ in $(seq 20); do grep -q "wpctl set-default 61" "$PWLOG" && break; sleep 0.3; done
sleep 2.5
clique_cfg "cfg som volume" 120
for _ in $(seq 20); do grep -q "wpctl set-volume @DEFAULT_AUDIO_SINK@" "$PWLOG" && break; sleep 0.3; done
ok "Som: um clique faz do alto-falante a saída padrão, e o controle muda o volume dela" \
  'grep -q "wpctl set-default 61" "$PWLOG" && grep -q "wpctl set-volume @DEFAULT_AUDIO_SINK@ " "$PWLOG" && python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1]))[\"saida\"]!=\"alsa_output.pci-analog-stereo\")" "$OUT/pipewire.json"'
sleep 1.5
# (tried again when the click is lost — a loaded machine drops a press now and then; the setting is the check)
for _ in 1 2 3; do
  clique_cfg "cfg som baixar"
  for _ in $(seq 20); do grep -aq "debug som: baixar_musica_em_chamadas false" "$T/shell.log" && break; sleep 0.3; done
  grep -aq "debug som: baixar_musica_em_chamadas false" "$T/shell.log" && break
done
grep -aq "debug som: baixar_musica_em_chamadas false" "$T/shell.log" || { echo "  (baixar: clicado em $(lugar "cfg som baixar") + janela $(janela_cfg))"; tail -8 "$T/shell.log" | grep -av amostra | sed 's/^/    /'; }
ok "Som: \"Baixar a música durante chamadas\" fica guardado" 'python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1])).get(\"baixar_musica_em_chamadas\") is not False)" "$X/config/kalango/configuracoes.json"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 4
ok "e, fechada, a página para de ler o microfone" '! pgrep -f "pw-record-teste --target 70" >/dev/null'

# 44. Settings › Power (design "Configurações · Energia"), against a stand-in power-profiles-daemon and a laptop's
#     battery (the computer's own power mode is never changed): the mode, the last day's draw and the apps that drew
#     most, the battery; a click chooses Economy; the idle times chosen dark the screen and then suspend (in the test
#     session suspending is only logged); the agent at work in check 15 kept the computer awake and chose Performance,
#     giving the mode back after; charge limit, lid and graphics card go through kalangod (approved; it simulates)
ENLOG="$OUT/energia.json.log"
"$N" msg action spawn -- "$T/kalango-shell" configuracoes energia
for _ in $(seq 40); do grep -aq "debug energia: modo balanced, [0-9]* amostras, bateria 82%" "$T/shell.log" && break; sleep 0.3; done
sleep 2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-energia.png" >/dev/null
ok "Energia: o modo, o consumo do dia (amostras guardadas) e a bateria" \
  'grep -a "debug energia: modo balanced, [0-9]* amostras, bateria 82%" "$T/shell.log" | tail -1 | awk "{exit !(\$5 >= 280)}"'
ok "Energia: a cada minuto o shell mede o consumo e guarda" 'grep -aq "debug energia: amostra [0-9.]* W" "$T/shell.log" && [ "$(wc -l < "$X/state/kalango/energia.jsonl")" -gt 288 ]'
nova_posicao "cfg energia modo power-saver"
clique_cfg "cfg energia modo power-saver"
for _ in $(seq 20); do grep -q "powerprofilesctl set power-saver" "$ENLOG" && break; sleep 0.3; done
ok "Energia: um clique escolhe Economia (power-profiles-daemon)" 'grep -q "powerprofilesctl set power-saver" "$ENLOG" && grep -q "\"power-saver\"" "$OUT/energia.json"'
ok "Energia: o agente que trabalhou manteve o computador acordado e escolheu Desempenho, e o modo voltou depois" \
  'grep -aq "debug energia: agente trabalhando, computador acordado" "$T/shell.log" && grep -aq "debug energia: agente parou, suspensão liberada" "$T/shell.log" && grep -q "powerprofilesctl set performance" "$ENLOG" && grep -aq "debug energia: o modo da pessoa voltou (balanced)" "$T/shell.log"'
# the idle times: "after 1 min" (4 s here) darks the screen; "after 2 min" suspends
lista_cfg "cfg energia tela"
xdotool key --window "$W" Down Return
for _ in $(seq 20); do grep -aq "debug energia: screen_off_min 1" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 30); do grep -aq "debug energia: ocioso, telas apagadas" "$T/shell.log" && break; sleep 0.3; done
ok "Energia: \"Apagar a tela\" após o tempo escolhido apaga as telas quando ninguém usa o computador" 'grep -aq "debug energia: ocioso, telas apagadas" "$T/shell.log"'
xdotool mousemove --window "$W" 300 300
sleep 1
lista_cfg "cfg energia suspender"
xdotool key --window "$W" Down Down Return
for _ in $(seq 20); do grep -aq "debug energia: suspend_min 2" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 40); do grep -aq "debug energia: ocioso, suspendendo" "$T/shell.log" && break; sleep 0.3; done
ok "Energia: \"Suspender\" após o tempo escolhido suspende (na sessão de teste, só registrado)" \
  'grep -aq "debug energia: ocioso, suspendendo" "$T/shell.log" && grep -aq "teste\] .suspender. não foi executado de verdade" "$T/shell.log"'
xdotool mousemove --window "$W" 320 320
sleep 1
# (back to "never": a GTK drop-down's list moves with the arrows)
escolher_cfg "cfg energia suspender" "Up Up Up Up Up Up Return" "debug energia: suspend_min 0"
sleep 1
escolher_cfg "cfg energia tela" "Up Up Up Up Up Up Return" "debug energia: screen_off_min 0"
ok "Energia: os tempos ficam guardados" 'python3 -c "import json,sys; v=json.load(open(sys.argv[1])); sys.exit(not (v[\"screen_off_min\"]==0 and v[\"suspend_min\"]==0))" "$X/config/kalango/configuracoes.json"'
# charge limit, lid and graphics card: system changes through kalangod (approved from here, as on the approval screen)
aprovar_pendente() { local id f0; f0=$(grep -ac "debug aprovação: fechada" "$T/aprovacao.log" 2>/dev/null); for _ in $(seq 40); do id=$(KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" pendentes 2>/dev/null | grep "$1" | head -1 | grep -o "^#[0-9]*" | tr -d '#'); [ -n "$id" ] && break; sleep 0.3; done; [ -n "$id" ] && KALANGOD_SOCKET="$T/mcp.sock" "$ROOT/target/release/kalangod" aprovar "$id" >/dev/null 2>&1
  for _ in $(seq 20); do [ "$(grep -ac "debug aprovação: fechada" "$T/aprovacao.log" 2>/dev/null)" -gt "${f0:-0}" ] && break; sleep 0.2; done; sleep 0.3; }
clique_cfg "cfg energia limite"
aprovar_pendente energia.limite_carga
for _ in $(seq 30); do grep -aq "debug energia: limite de carga 80 (pedido aprovado)" "$T/shell.log" && break; sleep 0.3; done
ok "Energia: \"Limitar a carga em 80%\" pede ao sistema (kalangod, aprovado)" 'grep -aq "debug energia: limite de carga 80 (pedido aprovado)" "$T/shell.log"'
lista_cfg "cfg energia tampa"
sleep 0.8
xdotool key --window "$W" Up Up Up Down
sleep 0.5
xdotool key --window "$W" Return
aprovar_pendente energia.tampa
for _ in $(seq 30); do grep -aq "debug energia: tampa desligar (pedido aprovado)" "$T/shell.log" && break; sleep 0.3; done
ok "Energia: \"Ao fechar a tampa\" pede ao sistema (kalangod, aprovado)" 'grep -aq "debug energia: tampa desligar (pedido aprovado)" "$T/shell.log"'
if command -v nvidia-smi >/dev/null && command -v prime-select >/dev/null; then
  clique_cfg "cfg energia gpu dedicado"
  aprovar_pendente grafico.modo
  for _ in $(seq 30); do grep -aq "debug energia: placa de vídeo dedicado" "$T/shell.log" && break; sleep 0.3; done
  ok "Energia: a placa de vídeo dedicada pede ao sistema (kalangod, aprovado; vale ao reiniciar)" 'grep -aq "debug energia: placa de vídeo dedicado (pedido aprovado)" "$T/shell.log"'
fi
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1

# 45. Settings › Notifications (design "Configurações · Notificações"): the day's counts and the apps that sent
#     notifications (the agents first); an app turned off shows nothing; an app "only in the digest" is held and the
#     agent sums it up (the local model); someone important breaks through the digest; Do not disturb keeps banners
#     away (the notice goes only to the list); the banners move to the corner chosen
notificar() { "$N" msg action spawn -- notify-send -a "$1" "$2" "$3"; sleep 1.2; }
for a in Discord Correio Música; do notificar "$a" "Olá de $a" "uma notificação de teste"; done
"$N" msg action spawn -- "$T/kalango-shell" configuracoes notificacoes
for _ in $(seq 40); do grep -aq "debug notif: [0-9]* hoje" "$T/shell.log" && break; sleep 0.3; done
sleep 2
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-notificacoes.png" >/dev/null
ultima_notif() { grep -a "debug notif: [0-9]* hoje" "$T/shell.log" | tail -1; }
ok "Notificações: as contagens do dia e os apps que mandaram notificações (os agentes primeiro)" \
  'ultima_notif | grep -q "apps Agentes," && ultima_notif | grep -q "Discord" && ultima_notif | grep -q "Correio" && ultima_notif | grep -q "Música"'
clique_cfg "cfg notif app Música"
for _ in $(seq 20); do grep -aq "debug notif: Música desligado" "$T/shell.log" && break; sleep 0.3; done
notificar Música "Tocando agora" "Faixa 2"
ok "Notificações: um app desligado não mostra nada" 'grep -aq "debug aviso: descartada (Música): Tocando agora" "$T/shell.log"'
clique_cfg "cfg notif app texto Correio"
sleep 1
clique_cfg "cfg notif modo DigestOnly"
for _ in $(seq 20); do grep -aq "debug notif: Correio DigestOnly" "$T/shell.log" && break; sleep 0.3; done
xdotool key --window "$W" Escape
sleep 0.5
notificar Correio "Promoção da semana" "Até 30% de desconto em livros"
ok "Notificações: \"Só no resumo\" guarda a notificação para o resumo do agente" 'grep -aq "debug aviso: resumo (Correio): Promoção da semana" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" avisos resumir
for _ in $(seq 120); do grep -aq "debug aviso: resumo do agente (1 avisos)" "$T/shell.log" && break; sleep 0.5; done
sleep 1
ok "e o agente resume (o modelo local) num só aviso" 'grep -aq "debug aviso: resumo do agente (1 avisos)" "$T/shell.log" && grep -aq "debug aviso: marcar como lida em .* :: Resumo do agente: 1 notificação" "$T/shell.log"'
clique_cfg "cfg notif pessoas"
sleep 1
xdotool type --window "$W" --delay 40 "Marina"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug notif: pessoa importante Marina" "$T/shell.log" && break; sleep 0.3; done
xdotool key --window "$W" Escape
sleep 0.5
notificar Correio "Marina: almoço amanhã?" "Responde quando puder"
ok "Notificações: alguém importante fura o resumo (aparece o banner)" \
  'grep -aq "debug notif: pessoa importante Marina" "$T/shell.log" && ! grep -aq "debug aviso: resumo (Correio): Marina" "$T/shell.log" && grep -aq "debug aviso: marcar como lida em .* :: Marina: almoço amanhã?" "$T/shell.log"'
clique_cfg "cfg notif nao perturbe"
for _ in $(seq 20); do grep -aq "debug notif: não perturbe true" "$T/shell.log" && break; sleep 0.3; done
notificar Discord "Nova mensagem" "no canal geral"
ok "Notificações: com Não perturbe a notificação vai só para a lista" 'grep -aq "debug aviso: lista (Discord): Nova mensagem" "$T/shell.log"'
# (off again — tried a second time when the click did not reach the switch: what follows needs it off)
for _ in 1 2 3; do
  clique_cfg "cfg notif nao perturbe"
  for _ in $(seq 10); do grep -aq "debug notif: não perturbe false" "$T/shell.log" && break; sleep 0.3; done
  grep -aq "debug notif: não perturbe false" "$T/shell.log" && break
done
clique_cfg "cfg notif posicao"
xdotool key --window "$W" Down Return
for _ in $(seq 20); do grep -aq "debug aviso: banners em TopRight" "$T/shell.log" && break; sleep 0.3; done
ok "Notificações: os banners vão para o canto escolhido (e fica guardado)" \
  'grep -aq "debug aviso: banners em TopRight" "$T/shell.log" && python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1]))[\"notifications\"][\"position\"]!=\"top_right\")" "$X/config/kalango/configuracoes.json"'
clique_cfg "cfg notif posicao"
sleep 0.8
xdotool key --window "$W" Up Up Up Up
sleep 0.5
xdotool key --window "$W" Return
for _ in $(seq 20); do [ "$(grep -a "debug aviso: banners em" "$T/shell.log" | tail -1)" = "debug aviso: banners em Island" ] && break; sleep 0.3; done
# 49. Settings › Input (designs "Configurações · Teclado", "Teclado e atalhos", "Mouse e touchpad"): a shortcut takes
#     new keys (written as niri binds: the old keys do nothing, niri reloads them) and goes back with ⌫; Caps Lock as
#     Ctrl becomes niri's xkb option; a shortcut made by request (the local model) is shown and kept
"$N" msg action spawn -- "$T/kalango-shell" configuracoes entrada
sleep 2.5
clique_cfg "cfg aba atalhos"
clique_cfg "cfg entrada busca"
xdotool type --window "$W" --delay 60 fechar
sleep 1
clique_cfg "cfg atalho fechar"
xdotool key --window "$W" alt+shift+k
for _ in $(seq 20); do grep -aq "debug entrada: atalho fechar → Alt+Shift+K" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
ok "Entrada: um atalho ganha novas teclas (bind do niri; as antigas não fazem mais nada)" \
  'grep -q "Alt+Shift+K repeat=false { close-window; }" "$X/config/kalango/atalhos.kdl" && grep -q "Mod+Q { spawn \"true\"; }" "$X/config/kalango/atalhos.kdl" && ! grep -aq "failed to parse" "$T/niri.log"'
clique_cfg "cfg atalho fechar"
xdotool key --window "$W" BackSpace
for _ in $(seq 20); do grep -aq "debug entrada: atalho fechar → padrão" "$T/shell.log" && break; sleep 0.3; done
ok "Entrada: ⌫ devolve o atalho às teclas padrão" '! grep -q "close-window" "$X/config/kalango/atalhos.kdl"'
clique_cfg "cfg aba teclado"
escolher_cfg "cfg entrada caps" "Down Return" "caps:ctrl_modifier"
sleep 1
ok "Entrada: Caps Lock como Ctrl vira a opção do xkb no niri" 'grep -q "caps:ctrl_modifier" "$X/config/kalango/entrada.kdl" && "$N" validate --config "$T/niri.kdl" >/dev/null 2>&1'
clique_cfg "cfg entrada pedido"
xdotool type --window "$W" --delay 30 "Super+Shift+D abre a pasta Downloads"
clique_cfg "cfg entrada criar"
for _ in $(seq 240); do grep -aq "debug entrada: atalho proposto\|debug entrada: atalho por pedido falhou" "$T/shell.log" && break; sleep 0.5; done
sleep 0.8
clique_cfg "cfg entrada confirmar"
for _ in $(seq 20); do grep -aq "debug entrada: atalho criado" "$T/shell.log" && break; sleep 0.3; done
ok "Entrada: um atalho por pedido (o modelo local monta; a pessoa vê e cria)" \
  'grep -aq "debug entrada: atalho criado Mod+Shift+D" "$T/shell.log" && grep -q "Mod+Shift+D repeat=false" "$X/config/kalango/atalhos.kdl" && grep -q "xdg-open" "$X/config/kalango/atalhos-proprios.json"'
# 50. Settings › System (designs "Configurações · Sobre", "Armazenamento", "Data e idioma"): the hardware and software
#     as this computer has them; what fills the disk; the time zone and the network time as timedatectl says
"$N" msg action spawn -- "$T/kalango-shell" configuracoes sistema
for _ in $(seq 40); do grep -aq "debug sistema: sobre (" "$T/shell.log" && break; sleep 0.5; done
ok "Sistema › Sobre: o processador, a memória e o kernel deste computador" 'grep -a "debug sistema: sobre (" "$T/shell.log" | tail -1 | grep -q "threads.*GB, Linux "'
clique_cfg "cfg aba armazenamento"
for _ in $(seq 120); do grep -aq "debug sistema: armazenamento " "$T/shell.log" && break; sleep 0.5; done
ok "Sistema › Armazenamento: o disco do sistema medido (usado, livre, caches)" 'grep -aq "debug sistema: armazenamento .* usados, .* livres" "$T/shell.log"'
clique_cfg "cfg aba data"
for _ in $(seq 40); do grep -aq "debug sistema: data (ntp" "$T/shell.log" && break; sleep 0.5; done
ok "Sistema › Data e idioma: o fuso e a hora pela rede como o sistema diz" 'grep -aq "debug sistema: data (ntp .*, fuso .*, [1-9][0-9]* fusos" "$T/shell.log"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1

# 51. the installer (designs "Instalador · 1 Boas-vindas", "2 Disco", "3 Conta", "4 Instalando"), simulated (its engine
#     only says the commands it would run): the disks as lsblk says (here a stand-in: an NVMe with Ubuntu and a data
#     SSD), "beside Ubuntu" by default, the account (the user from the name, the password's strength and match), a
#     second click to confirm, the engine's steps to the end; the plan on disk never has the password
cat > "$OUT/lsblk.json" <<'J'
{"blockdevices": [
 {"name":"nvme0n1","path":"/dev/nvme0n1","size":1000204886016,"type":"disk","model":"Samsung SSD 980","tran":"nvme","rota":false,"rm":false,"mountpoints":[null],
  "children":[{"name":"nvme0n1p1","path":"/dev/nvme0n1p1","size":536870912,"start":2048,"type":"part","fstype":"vfat","partlabel":"EFI System Partition","mountpoints":[null],"fsused":"6291456"},
              {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","size":999667941376,"start":1050624,"type":"part","fstype":"ext4","label":"ubuntu","mountpoints":[null],"fsused":"312000000000"}]},
 {"name":"sda","path":"/dev/sda","size":2000398934016,"type":"disk","model":"Crucial MX500","tran":"sata","rota":false,"rm":false,"mountpoints":[null],
  "children":[{"name":"sda1","path":"/dev/sda1","size":2000397885440,"start":2048,"type":"part","fstype":"ext4","label":"dados","mountpoints":[null],"fsused":"900000000000"}]}
]}
J
INST="$OUT/instalador.log"
lugar_inst() { grep -a "debug instalador: $1 em" "$INST" | tail -1 | awk '{print $(NF-1), $NF}'; }
clique_inst() { sem_avisos; read -r JX JY < <(janela_app os.kalango.Instalador); read -r CX CY < <(lugar_inst "$1"); apontar $((JX + CX)) $((JY + CY)); sleep 0.2; xdotool click 1; sleep 0.8; }
"$N" msg action spawn -- env KALANGO_INSTALADOR_DEBUG=1 KALANGO_INSTALADOR_LSBLK="$OUT/lsblk.json" KALANGO_INSTALADOR_RAPIDO=1 KALANGO_INSTALADOR_DIR="$OUT/instalador" \
  sh -c "exec '$ROOT/target/release/kalango-instalador' 2>'$INST'"
for _ in $(seq 40); do grep -q "debug instalador: continuar em" "$INST" 2>/dev/null && "$N" msg -j windows | grep -q '"os.kalango.Instalador"' && break; sleep 0.5; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-instalador.png" >/dev/null
ok "Instalador: abre nas boas-vindas com os discos lidos (um NVMe com Ubuntu; o SSD só de dados não é um sistema)" \
  'grep -q "debug instalador: passo 1" "$INST" && grep -q "debug instalador: 2 discos (NVMe 1 TB Ubuntu, SSD 2 TB )" "$INST"'
clique_inst continuar
for _ in $(seq 20); do grep -q "debug instalador: passo 2" "$INST" && break; sleep 0.3; done
clique_inst continuar
for _ in $(seq 20); do grep -q "debug instalador: passo 3" "$INST" && break; sleep 0.3; done
sleep 0.6
xdotool type --window "$W" --delay 40 "Ana Lima"
clique_inst senha
xdotool type --window "$W" --delay 40 "Lua-cheia-2026"
clique_inst senha2
xdotool type --window "$W" --delay 40 "Lua-cheia-2026"
sleep 0.5
clique_inst continuar
clique_inst continuar
for _ in $(seq 60); do grep -q 'debug instalador: motor {"fim":true}' "$INST" && break; sleep 0.5; done
ok "Instalador: ao lado do Ubuntu, o motor encolhe a partição dele no mesmo lugar e faz a do KalangoOS (simulado, até o fim)" \
  'grep -q "debug instalador: passo 4" "$INST" && grep -q "comando resize2fs /dev/nvme0n1p2" "$INST" && grep -q "comando sgdisk -d2 -n2:1050624:+" "$INST" && grep -q "comando sgdisk -n0:0:0 -t0:8300 -c0:KalangoOS /dev/nvme0n1" "$INST" && grep -q "debug instalador: motor {\"fim\":true}" "$INST"'
ok "Instalador: a conta vem do nome (ana), e a senha nunca fica no plano em disco" \
  'grep -q "comando chroot /target useradd .* ana" "$INST" && grep -q "\"user\": \"ana\"" "$OUT/instalador/plano-instalacao.json" && ! grep -q "Lua-cheia" "$OUT/instalador/plano-instalacao.json"'
clique_inst continuar
for _ in $(seq 20); do grep -q "debug instalador: reiniciar" "$INST" && break; sleep 0.3; done
ok "Instalador: no fim, \"Reiniciar agora\"" 'grep -q "debug instalador: reiniciar" "$INST"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Instalador"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 52. first use (designs "Primeiro uso · 1 Olá", "2 Aparência", "3 Agentes"): the other system on the disk (kalangod
#     reads it: a stand-in Ubuntu with this person's folders and Firefox) and its copy with approval; the theme, the
#     accent and the dock change at once and are kept; "Começar" sets the agent's permission columns as chosen
printf '{"feito": false, "impressao_digital": false}\n' > "$X/config/kalango/primeiro-uso.json"
clique_pu() { sem_avisos; read -r JX JY < <(janela_app os.kalango.BoasVindas); read -r CX CY < <(lugar "pu $1"); apontar $((JX + CX)) $((JY + CY)); sleep 0.2; xdotool click 1; sleep 0.8; }
"$N" msg action spawn -- "$T/kalango-shell" primeiro-uso
for _ in $(seq 40); do grep -aq "debug primeiro uso: achado\|debug primeiro uso: nenhum" "$T/shell.log" && "$N" msg -j windows | grep -q '"os.kalango.BoasVindas"' && break; sleep 0.5; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-primeiro-uso.png" >/dev/null
ok "Primeiro uso: acha o outro sistema do disco (as pastas e o navegador da pessoa lá, lidos pelo kalangod)" 'grep -aq "debug primeiro uso: achado Ubuntu (2 pastas, navegador Firefox)" "$T/shell.log"'
clique_pu revisar
clique_pu continuar
aprovar_pendente importar.copiar
for _ in $(seq 120); do grep -aq "debug primeiro uso: importação terminou" "$T/shell.log" && grep -aq "Suas coisas chegaram" "$T/shell.log" && break; sleep 0.5; done
ok "Primeiro uso: Continuar traz as pastas e o Firefox, com aprovação (lá só leitura; aqui nada é sobrescrito)" \
  'grep -aq "debug primeiro uso: importar começou (3 pastas)" "$T/shell.log" && grep -aq "debug primeiro uso: importação terminou" "$T/shell.log" && grep -aq "Suas coisas chegaram" "$T/shell.log"'
clique_pu "tema claro"
clique_pu "destaque teal"
clique_pu "dock fixa"
for _ in $(seq 20); do grep -aq "debug primeiro uso: dock fixa" "$T/shell.log" && break; sleep 0.3; done
ok "Primeiro uso: o tema, a cor de destaque e a dock mudam na hora e ficam guardados" \
  'python3 -c "import json,sys; t=json.load(open(sys.argv[1])); sys.exit(not (t[\"estilo\"]==\"claro\" and t[\"destaque\"]==\"teal\"))" "$X/config/kalango/tema.json" && grep -q "fixa" "$X/config/kalango/dock-estilo.json"'
clique_pu "tema escuro"
clique_pu "destaque accent"
clique_pu "dock flutuante"
clique_pu continuar
clique_pu "autonomia sempre"
clique_pu continuar
for _ in $(seq 40); do grep -aq "debug primeiro uso: agentes" "$T/shell.log" && break; sleep 0.5; done
ok "Primeiro uso: Começar põe as colunas de permissão do agente como escolhido (perguntar sempre) e o primeiro uso fica feito" \
  'grep -aq "debug primeiro uso: agentes hibrido, autonomia sempre (0 erros)" "$T/shell.log" && python3 -c "import json,sys; c=json.load(open(sys.argv[1]))[\"categorias\"]; sys.exit(not all(c.get(k)==\"perguntar\" for k in [\"ler\",\"escrever\",\"apagar\",\"rede\",\"terminal\"]))" "$X/config/kalango/agente-permissoes.json" && grep -q "\"feito\": true" "$X/config/kalango/primeiro-uso.json"'
# 53. Settings › Accessibility (design "Configurações · Acessibilidade"): high contrast and the text's size in the
#     theme (and back), reduced motion as niri's own "animations off" (valid configuration), visual alerts flash on a
#     notification, mono audio through pipewire-pulse (a stand-in pactl), live captions of what plays (the local audio
#     model on the test's recorded speech), control by voice as the Super key's setting, and "describe the screen" with
#     the local vision model — its picture neither announced nor kept in the clipboard history
"$N" msg action spawn -- "$T/kalango-shell" configuracoes acessibilidade
for _ in $(seq 40); do grep -aq "debug acessibilidade: lido" "$T/shell.log" && break; sleep 0.5; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-acessibilidade.png" >/dev/null
clique_cfg "cfg acess contraste"
for _ in $(seq 20); do grep -aq "debug acessibilidade: contraste true" "$T/shell.log" && break; sleep 0.3; done
clique_cfg "cfg acess texto 125"
for _ in $(seq 20); do grep -aq "debug acessibilidade: texto 125%" "$T/shell.log" && break; sleep 0.3; done
ok "Acessibilidade: alto contraste e o tamanho do texto mudam o tema na hora e ficam guardados" \
  'python3 -c "import json,sys; t=json.load(open(sys.argv[1])); sys.exit(not (t[\"contraste\"] is True and abs(t[\"texto\"]-1.25)<0.01))" "$X/config/kalango/tema.json"'
clique_cfg "cfg acess contraste"
clique_cfg "cfg acess texto 100"
clique_cfg "cfg acess reduzir"
for _ in $(seq 20); do grep -aq "debug acessibilidade: reduzir animações true" "$T/shell.log" && break; sleep 0.3; done
ok "Acessibilidade: reduzir animações desliga as do niri (configuração válida) e as do shell" \
  'grep -q "off" "$X/config/kalango/acessibilidade.kdl" && "$N" validate --config "$T/niri.kdl" >/dev/null 2>&1'
clique_cfg "cfg acess reduzir"
clique_cfg "cfg acess alertas"
for _ in $(seq 20); do grep -aq "debug acessibilidade: alertas visuais true" "$T/shell.log" && break; sleep 0.3; done
notificar Alerta "Lembrete" "teste do alerta visual"
for _ in $(seq 20); do grep -aq "debug acessibilidade: alerta visual" "$T/shell.log" && break; sleep 0.3; done
ok "Acessibilidade: com alertas visuais, a tela pisca quando uma notificação tocaria o som" 'grep -aq "debug acessibilidade: alerta visual" "$T/shell.log"'
clique_cfg "cfg acess alertas"
clique_cfg "cfg acess mono"
for _ in $(seq 20); do grep -aq "debug acessibilidade: áudio mono true" "$T/shell.log" && break; sleep 0.3; done
ok "Acessibilidade: áudio mono põe uma saída mono do PipeWire na frente da saída padrão" \
  'grep -q "load-module module-remap-sink sink_name=kalango_mono master=alsa_output.alto-falantes channels=1" "$OUT/pactl.log" && grep -q "set-default-sink kalango_mono" "$OUT/pactl.log"'
clique_cfg "cfg acess mono"
for _ in $(seq 20); do grep -q "unload-module 536870913" "$OUT/pactl.log" && break; sleep 0.3; done
ok "e desligado, a saída de antes volta a ser a padrão" 'grep -q "set-default-sink alsa_output.alto-falantes" "$OUT/pactl.log" && grep -q "unload-module 536870913" "$OUT/pactl.log"'
clique_cfg "cfg acess voz"
for _ in $(seq 20); do grep -aq "debug acessibilidade: controle por voz false" "$T/shell.log" && break; sleep 0.3; done
ok "Acessibilidade: controlar por voz é o mesmo ajuste da tecla Super (segurar)" 'grep -q "\"super_segurar\": \"nada\"" "$X/config/kalango/configuracoes.json"'
clique_cfg "cfg acess voz"
clique_cfg "cfg acess legendas"
for _ in $(seq 240); do grep -aq "debug legendas: «" "$T/shell.log" && break; sleep 0.5; done
ok "Acessibilidade: legendas ao vivo escrevem o que toca no computador (o modelo de áudio local)" 'grep -a "debug legendas: «" "$T/shell.log" | head -1 | grep -qi "disco"'
clique_cfg "cfg acess legendas"
IMGS0=$(grep -ac "debug transferencia: copiado imagem" "$T/shell.log")
clique_cfg "cfg acess descrever"
for _ in $(seq 360); do grep -aq "debug acessibilidade: tela descrita\|debug acessibilidade: descrever a tela falhou" "$T/shell.log" && break; sleep 0.5; done
sleep 1
ok "Acessibilidade: descrever a tela — o modelo de visão local diz o que há na tela e a voz lê (sem aviso de captura nem cópia no histórico)" \
  'grep -aq "debug acessibilidade: tela descrita" "$T/shell.log" && grep -aq "debug acessibilidade: falaria «" "$T/shell.log" && [ "$(grep -ac "debug transferencia: copiado imagem" "$T/shell.log")" = "$IMGS0" ]'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 54. Settings › Privacy and security (design "Configurações · Privacidade e segurança"): who can use what — an
#     isolated app (a stand-in Flatpak, "Fotos") and the agent in one table; a click on a cell changes the app's
#     sandbox or the portals' store, or the agent's column; the agents' history kept for how long; erasing the AI's data
"$N" msg action spawn -- "$T/kalango-shell" configuracoes privacidade
for _ in $(seq 40); do grep -aq "debug privacidade: 1 apps isolados e o agente" "$T/shell.log" && break; sleep 0.5; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-privacidade.png" >/dev/null
ok "Privacidade: o app isolado (Flatpak) e o agente na mesma tabela, com a segurança lida do sistema" \
  'grep -aq "debug privacidade: 1 apps isolados e o agente" "$T/shell.log" && grep -aq "debug privacidade: segurança (disco" "$T/shell.log"'
clique_cfg "cfg priv Fotos rede"
for _ in $(seq 20); do grep -q "override --user --unshare=network org.exemplo.Fotos" "$OUT/flatpak.log" && break; sleep 0.3; done
clique_cfg "cfg priv Fotos camera"
for _ in $(seq 20); do grep -q "permission-set devices camera org.exemplo.Fotos no" "$OUT/flatpak.log" && break; sleep 0.3; done
ok "Privacidade: um clique tira a rede do app (o isolamento do Flatpak) e nega a câmera (o portal)" \
  'grep -q "override --user --unshare=network org.exemplo.Fotos" "$OUT/flatpak.log" && grep -q "permission-set devices camera org.exemplo.Fotos no" "$OUT/flatpak.log"'
REDE0=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("categorias",{}).get("rede",""))' "$X/config/kalango/agente-permissoes.json" 2>/dev/null)
clique_cfg "cfg priv Agente rede"
for _ in $(seq 20); do grep -aq "debug privacidade: Agente · rede (mudou)" "$T/shell.log" && break; sleep 0.3; done
REDE1=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("categorias",{}).get("rede",""))' "$X/config/kalango/agente-permissoes.json" 2>/dev/null)
echo "  (rede do agente: ${REDE0:-padrão} → $REDE1)"
ok "Privacidade: e a célula do agente muda a coluna dele (a mesma regra da Central de agentes)" '[ -n "$REDE1" ] && [ "$REDE1" != "$REDE0" ]'
lista_cfg "cfg priv historico"
sleep 0.8
xdotool key --window "$W" Up Up Up Up Down Down
sleep 0.5
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug privacidade: histórico de 90 dias" "$T/shell.log" && break; sleep 0.3; done
ok "Privacidade: o histórico dos agentes fica pelo tempo escolhido" 'grep -q "\"historico_agentes_dias\": 90" "$X/config/kalango/configuracoes.json"'
clique_cfg "cfg priv apagar"
sleep 0.5
clique_cfg "cfg priv apagar"
for _ in $(seq 30); do grep -aq "debug privacidade: dados de IA apagados" "$T/shell.log" && break; sleep 0.3; done
ok "Privacidade: \"Apagar todos os dados de IA\" (com um segundo clique) apaga as conversas e o que o agente lembra" \
  'grep -aq "debug privacidade: dados de IA apagados (ok)" "$T/shell.log" && [ ! -d "$X/data/kalango/conversas" ]'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 55. Emoji and symbols (design "Emoji e símbolos (Super+.)"): over the window in use (the text editor), the
#     search by the Portuguese names, ↵ puts it where the text cursor is; with nothing found, the local model turns
#     the description into words that are searched
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2.5
"$N" msg action spawn -- "$T/kalango-shell" emoji
for _ in $(seq 20); do grep -aq "debug emoji: aberto sobre os.kalango.Editor" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
xdotool type --window "$W" --delay 80 "sorri"
for _ in $(seq 20); do grep -aq "debug emoji: «sorri» [1-9]" "$T/shell.log" && break; sleep 0.3; done
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug emoji: colado" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
COLADO=$(grep -a "debug emoji: colado " "$T/shell.log" | tail -1 | sed 's/.*colado //')
C0=$(grep -ac "debug transferencia: conteúdo «$COLADO»" "$T/shell.log")
xdotool key --window "$W" ctrl+a ctrl+c
for _ in $(seq 20); do [ "$(grep -ac "debug transferencia: conteúdo «$COLADO»" "$T/shell.log")" -gt "$C0" ] && break; sleep 0.3; done
echo "  (colado: $COLADO)"
ok "Emoji: \"sorri\" acha um rosto sorrindo pelo nome em português e ↵ o põe no editor" \
  '[ -n "$COLADO" ] && grep -a "$COLADO" "$ROOT/crates/kalango-shell/assets/emoji-pt.tsv" | head -1 | grep -qi "sorri" && [ "$(grep -ac "debug transferencia: conteúdo «$COLADO»" "$T/shell.log")" -gt "$C0" ]'
"$N" msg action spawn -- "$T/kalango-shell" emoji
sleep 1.2
xdotool type --window "$W" --delay 60 "comemorar vitoria"
sleep 0.8
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug emoji: o modelo deu" "$T/shell.log" && break; sleep 0.5; done
ok "Emoji: sem nada com as palavras, o modelo local dá as palavras da descrição e elas são buscadas" 'grep -a "debug emoji: o modelo deu" "$T/shell.log" | tail -1 | grep -q "→ [1-9]"'
xdotool key --window "$W" Escape
sleep 0.5
# (an empty document closes without asking to save)
xdotool key --window "$W" ctrl+a Delete
sleep 0.5
fechar_apps
# 56. On-screen indicators (design "Indicadores na tela"): the volume key changes the default output (the stand-in
#     PipeWire) and shows its card above the dock; the microphone key mutes the input and says so
vol() { KALANGO_PW_ESTADO="$OUT/pipewire.json" "$ROOT/sessao/ferramentas/wpctl-teste" get-volume "$1" 2>/dev/null; }
V0=$(vol @DEFAULT_AUDIO_SINK@ | awk '{print $2}')
"$N" msg action spawn -- "$T/kalango-shell" osd volume +5
for _ in $(seq 20); do grep -aq "debug osd: Volume [0-9]*%" "$T/shell.log" && break; sleep 0.3; done
V1=$(vol @DEFAULT_AUDIO_SINK@ | awk '{print $2}')
echo "  (volume: $V0 → $V1)"
ok "Indicadores: a tecla de volume muda a saída padrão e mostra o cartão acima da dock" \
  'grep -aq "debug osd: Volume [0-9]*%" "$T/shell.log" && python3 -c "import sys; sys.exit(not float(sys.argv[2]) > float(sys.argv[1]))" "$V0" "$V1"'
"$N" msg action spawn -- "$T/kalango-shell" osd microfone
for _ in $(seq 20); do grep -aq "debug osd: Microfone desligado" "$T/shell.log" && break; sleep 0.3; done
ok "Indicadores: a tecla do microfone o desliga e diz" 'grep -aq "debug osd: Microfone desligado" "$T/shell.log" && vol @DEFAULT_AUDIO_SOURCE@ | grep -q MUTED'
"$N" msg action spawn -- "$T/kalango-shell" osd microfone
sleep 1
# 57. Critical alerts (design "Alertas críticos (bateria e disco)"): on battery at 5% a notice that stays, and its
#     "Economia máxima" chooses the power-saver mode; the disk almost full (the test's "free bytes") and "Liberar" frees
#     the caches and the trash, nothing of the person's
python3 -c 'import json,sys; p=sys.argv[1]
try: e=json.load(open(p))
except Exception: e={"perfil": "balanced"}
e["bateria"]={"pct": 5, "minutos": 6}; json.dump(e, open(p, "w"))' "$OUT/energia.json"
for _ in $(seq 40); do grep -aq "debug alerta: Bateria em 5%" "$T/shell.log" && break; sleep 0.5; done
sleep 1
read -r AX AY < <(grep -a "debug aviso: primeira ação em .* :: Bateria em 5%" "$T/shell.log" | tail -1 | sed 's/.*primeira ação em \([0-9]*\) \([0-9]*\) ::.*/\1 \2/')
foto_x alerta-bateria
[ -n "$AX" ] && clicar "$AX" "$AY"
for _ in $(seq 20); do grep -q "powerprofilesctl set power-saver" "$ENLOG" && grep -aq "debug alerta: bateria → economia" "$T/shell.log" && break; sleep 0.3; done
ok "Alertas: bateria em 5% fora da tomada — o aviso fica até ser respondido e \"Economia máxima\" escolhe Economia" \
  'grep -aq "debug alerta: Bateria em 5% — cerca de 6 minutos" "$T/shell.log" && grep -aq "debug alerta: bateria → economia" "$T/shell.log"'
python3 -c 'import json,sys; p=sys.argv[1]; e=json.load(open(p)); e.pop("bateria", None); json.dump(e, open(p, "w"))' "$OUT/energia.json"
mkdir -p "$X/cache/teste-alerta" && head -c 2000000 /dev/zero > "$X/cache/teste-alerta/lixo.bin"
echo 3000000000 > "$OUT/disco-livre"
for _ in $(seq 40); do grep -aq "debug alerta: Disco quase cheio" "$T/shell.log" && break; sleep 0.5; done
sleep 1
read -r DX DY < <(grep -a "debug aviso: primeira ação em .* :: Disco quase cheio" "$T/shell.log" | tail -1 | sed 's/.*primeira ação em \([0-9]*\) \([0-9]*\) ::.*/\1 \2/')
[ -n "$DX" ] && clicar "$DX" "$DY"
for _ in $(seq 30); do grep -aq "debug alerta: liberado" "$T/shell.log" && break; sleep 0.3; done
rm -f "$OUT/disco-livre"
ok "Alertas: disco quase cheio — \"Liberar\" apaga os caches e a lixeira (nada da pessoa)" \
  'grep -aq "debug alerta: Disco quase cheio: restam 3 GB" "$T/shell.log" && grep -aq "debug alerta: liberado" "$T/shell.log" && [ ! -e "$X/cache/teste-alerta" ]'
# 58. The Wi-Fi password from NetworkManager (the shell is its secret agent, as GNOME Shell is): a network joined
#     anywhere else — or a saved one whose password changed — asks in the same dialog, and NetworkManager gets the
#     password in its own shape; Esc tells it the person canceled
NMA="$OUT/nm-agente"; mkdir -p "$NMA"
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/nm-agente-teste.py" "$NMA"
for _ in $(seq 40); do grep -q "registrado os.kalango.shell" "$NMA/nm.log" 2>/dev/null && break; sleep 0.3; done
printf '{"ssid": "Biblioteca", "key_mgmt": "sae", "flags": 1}' > "$NMA/pedir.json"
for _ in $(seq 30); do grep -aq "debug wifi: pedindo Biblioteca (WPA3)" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x wifi-senha
xdotool type --window "$W" --delay 30 "lua-cheia-42"; xdotool key --window "$W" Return
for _ in $(seq 30); do [ -s "$NMA/resposta.json" ] && break; sleep 0.3; done
echo "  (o NetworkManager de teste: $(tr '\n' ' ' < "$NMA/nm.log"); recebeu $(cat "$NMA/resposta.json" 2>/dev/null))"
ok "Wi-Fi: o NetworkManager pede a senha ao shell (o agente de senhas) e a recebe do mesmo diálogo" \
  'grep -q "registrado os.kalango.shell" "$NMA/nm.log" && python3 -c "import json,sys; r=json.load(open(sys.argv[1])); sys.exit(0 if r[\"segredos\"][\"802-11-wireless-security\"][\"psk\"]==\"lua-cheia-42\" else 1)" "$NMA/resposta.json"'
rm -f "$NMA/resposta.json"
printf '{"ssid": "Biblioteca", "key_mgmt": "sae", "flags": 3}' > "$NMA/pedir.json"
for _ in $(seq 30); do grep -aq "debug wifi: pedindo Biblioteca (WPA3, senha recusada)" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
xdotool key --window "$W" Escape
for _ in $(seq 30); do [ -s "$NMA/resposta.json" ] && break; sleep 0.3; done
ok "Wi-Fi: a senha que mudou pede de novo (em coral); Esc cancela e o NetworkManager ouve \"UserCanceled\"" \
  'grep -aq "debug wifi: cancelado Biblioteca" "$T/shell.log" && grep -q "UserCanceled" "$NMA/resposta.json"'
# 59. Bluetooth pairing through the shell's agent (designs "Parear Bluetooth" and "· confirmar código"): a keyboard
#     shows the code to type, its boxes lighting as the keys arrive, and the ✓ closes it; a phone's code is compared
#     and "Parear" pairs it; "Cancelar" stops a keyboard's pairing (BlueZ hears CancelPairing)
echo "  (o BlueZ de teste: $(head -3 "$BZ/bluez.log" | tr '\n' ' '))"
printf '{"mac": "AA:BB:CC:00:11:22"}' > "$BZ/parear.json"
for _ in $(seq 40); do grep -aq "debug bluetooth: Teclado K3 digitou 2 de 6" "$T/shell.log" && break; sleep 0.2; done
foto_x bt-teclado
for _ in $(seq 40); do grep -aq "debug bluetooth: pareado pelo diálogo Teclado K3" "$T/shell.log" && break; sleep 0.3; done
ok "Bluetooth: um teclado mostra o código para digitar, as caixas acendem com cada tecla e o ✓ fecha o diálogo" \
  'grep -aq "debug bluetooth: pedido de Teclado K3 (Type" "$T/shell.log" && grep -aq "debug bluetooth: Teclado K3 digitou 6 de 6" "$T/shell.log" && grep -aq "debug bluetooth: pareado pelo diálogo Teclado K3" "$T/shell.log"'
sleep 1.5
printf '{"mac": "AA:BB:CC:33:44:55"}' > "$BZ/parear.json"
for _ in $(seq 30); do grep -aq "debug bluetooth: pedido de Pixel 8 (Compare" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x bt-celular
read -r BX BY < <(grep -a "debug central: bt dialogo sim em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$BX" "$BY"
for _ in $(seq 30); do grep -q "pareado AA:BB:CC:33:44:55" "$BZ/bluez.log" && break; sleep 0.3; done
ok "Bluetooth: um celular compara o código e \"Parear\" o pareia" \
  'grep -q "confirmação AA:BB:CC:33:44:55: sim" "$BZ/bluez.log" && grep -q "pareado AA:BB:CC:33:44:55" "$BZ/bluez.log" && grep -aq "debug bluetooth: pareado pelo diálogo Pixel 8" "$T/shell.log"'
sleep 1.5
N0=$(grep -ac "debug central: bt dialogo nao em" "$T/shell.log")
printf '{"mac": "AA:BB:CC:00:11:22"}' > "$BZ/parear.json"
for _ in $(seq 30); do [ "$(grep -ac "debug central: bt dialogo nao em" "$T/shell.log")" -gt "$N0" ] && break; sleep 0.3; done
sleep 0.6
read -r BX BY < <(grep -a "debug central: bt dialogo nao em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$BX" "$BY"
for _ in $(seq 30); do grep -q "cancelado AA:BB:CC:00:11:22" "$BZ/bluez.log" && break; sleep 0.3; done
ok "Bluetooth: \"Cancelar\" para o pareamento do teclado (o BlueZ ouve CancelPairing)" \
  'grep -q "CancelPairing AA:BB:CC:00:11:22" "$BZ/bluez.log" && grep -q "cancelado AA:BB:CC:00:11:22" "$BZ/bluez.log"'
# 95. A device that connects (design "Indicadores na tela", DISPOSITIVO): the indicator above the dock with its kind's
#     glyph, its name and its battery (BlueZ's Battery1); one that goes says so
sem_avisos 2>/dev/null || sleep 4
printf '{"mac": "C8:24:78:67:3D:CF", "conectado": true, "bateria": 80}' > "$BZ/conectar.json"
for _ in $(seq 20); do grep -aq "debug osd: Fone Lunar conectado" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug osd: Fone Lunar" "$T/shell.log" | tail -1))"
ok "Indicadores: um fone que conecta aparece com o nome e a bateria" 'grep -a "debug osd: Fone Lunar conectado" "$T/shell.log" | tail -1 | grep -q "(80%)"'
sleep 1.6
printf '{"mac": "C8:24:78:67:3D:CF", "conectado": false}' > "$BZ/conectar.json"
for _ in $(seq 20); do grep -aq "debug osd: Fone Lunar desconectado" "$T/shell.log" && break; sleep 0.3; done
ok "Indicadores: e quando ele sai, o indicador diz" 'grep -aq "debug osd: Fone Lunar desconectado" "$T/shell.log"'
# 96. Settings › Agents and AI (design "Configurações · Agentes e IA"; it used to send to the Agents app): the
#     providers with their state now (KalangoOS's own model server, Ollama and LM Studio where they answer), the model
#     for each kind of task; "Memória" leads to the Agents app's page
"$N" msg action spawn -- "$T/kalango-shell" configuracoes agentes
for _ in $(seq 40); do grep -aq "debug agentes-ia: local " "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug agentes-ia: local " "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 0.8 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-agentes-ia.png" >/dev/null
ok "Agentes e IA: a página das Configurações mostra os provedores com o estado de agora" 'grep -a "debug agentes-ia: local " "$T/shell.log" | tail -1 | grep -Eq "local (carregado|parado) · ollama"'
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_app os.kalango.Configuracoes); read -r MX MY < <(lugar "cfg agentes memoria")
[ -n "$MX" ] && apontar $((JX + MX)) $((JY + MY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 30); do "$N" msg -j focused-window | grep -q '"os.kalango.Agentes"' && break; sleep 0.3; done
ok "Agentes e IA: “Memória” leva à página do app Agentes" '"$N" msg -j focused-window | grep -q "\"os.kalango.Agentes\""'
"$N" msg action close-window
sleep 0.6
"$N" msg action close-window --id "$(id_app kalango.configuracoes)" 2>/dev/null
sleep 0.6
# 60. A drive connected (design "Dispositivo conectado (pendrive)"; the session test's drives are folders): the card
#     under the status area with the photos Pictures does not have yet; a drive in use does not eject (and says so);
#     "Importar" copies only the new photos into Imagens/<year>; another drive ejects and says it can be removed
MID="$OUT/midia"; IMG="$X/area/Imagens/$(date +%Y)"; NOVO="$OUT/midia-novo"
mkdir -p "$IMG" "$NOVO/Kingston/DCIM/100" "$NOVO/Cartao SD" "$MID"
for i in 1 2 3; do head -c 2000 /dev/urandom > "$NOVO/Kingston/DCIM/100/IMG_000$i.JPG"; done
cp "$NOVO/Kingston/DCIM/100/IMG_0001.JPG" "$IMG/"
echo nota > "$NOVO/Kingston/leia.txt"; touch "$NOVO/Kingston/em-uso"
mv "$NOVO/Kingston" "$MID/"
for _ in $(seq 30); do grep -aq "debug midia: 2 fotos novas em Kingston" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 20); do grep -aq "debug midia: importar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
foto_x pendrive
ok "Pendrive: o cartão sob a área de status, com as 2 fotos que Imagens ainda não tem" \
  'grep -aq "debug midia: conectado Kingston" "$T/shell.log" && grep -aq "debug midia: 2 fotos novas em Kingston" "$T/shell.log" && grep -aq "debug midia: ações abrir, importar, organizar" "$T/shell.log"'
read -r EX EY < <(grep -a "debug midia: ejetar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$EX" "$EY"
for _ in $(seq 20); do grep -aq "debug midia: não ejetou Kingston" "$T/shell.log" && break; sleep 0.3; done
read -r IX IY < <(grep -a "debug midia: importar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$IX" "$IY"
for _ in $(seq 60); do [ -e "$IMG/IMG_0003.JPG" ] && [ -e "$IMG/IMG_0002.JPG" ] && break; sleep 0.3; done
sleep 1
echo "  (Imagens/$(date +%Y): $(ls "$IMG" | tr '\n' ' '))"
ok "Pendrive: em uso não ejeta (e diz); \"Importar\" traz só as fotos novas para Imagens/<ano>" \
  'grep -aq "debug midia: não ejetou Kingston" "$T/shell.log" && [ -e "$IMG/IMG_0002.JPG" ] && [ -e "$IMG/IMG_0003.JPG" ] && [ ! -e "$IMG/leia.txt" ] && [ "$(ls "$IMG" | wc -l)" = 3 ]'
sleep 2
N0=$(grep -ac "debug midia: ejetar em" "$T/shell.log")
mv "$NOVO/Cartao SD" "$MID/"
for _ in $(seq 30); do [ "$(grep -ac "debug midia: ejetar em" "$T/shell.log")" -gt "$N0" ] && break; sleep 0.3; done
sleep 0.5
read -r EX EY < <(grep -a "debug midia: ejetar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$EX" "$EY"
for _ in $(seq 20); do grep -aq "debug midia: pode remover Cartao SD" "$T/shell.log" && break; sleep 0.3; done
ok "Pendrive: \"Ejetar\" diz que pode remover só depois de ejetado" 'grep -aq "debug midia: pode remover Cartao SD" "$T/shell.log" && [ -e "$MID/Cartao SD.ejetado" ]'
# 61. The system's file chooser (designs "Abrir arquivo" and "Salvar como"): the Text Editor's Ctrl+O opens it (the
#     shell's own apps call it directly); a double click opens the file in the editor (its text filter takes Markdown
#     too). Save as: the compact form, the name from the text's first words ready to type over, a tag set on the file
#     once written. Another system's app (a browser) asks through xdg-desktop-portal and gets the same chooser
SEL="$OUT/seletor"; mkdir -p "$SEL"
printf '# Notas\n- cabo HDMI\n' > "$SEL/notas.md"; printf 'Fatura de outubro\n' > "$SEL/fatura-outubro.txt"
fechar_apps
abrir_ed
for _ in $(seq 40); do janela_app os.kalango.Editor >/dev/null 2>&1 && break; sleep 0.3; done
sleep 2
xdotool key --window "$W" ctrl+o
for _ in $(seq 40); do grep -aq "debug central: seletor arquivo notas.md em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x seletor-abrir
read -r JX JY < <(janela_app os.kalango.Seletor); read -r CX CY < <(lugar "seletor arquivo notas.md")
apontar $((JX + CX)) $((JY + CY)); sleep 0.3; xdotool click --repeat 2 --delay 90 1
for _ in $(seq 30); do "$N" msg -j windows | grep -q '"title": *"notas.md' && break; sleep 0.3; done
ok "Seletor: o Ctrl+O do Editor de Texto abre o seletor do sistema; o duplo clique abre o arquivo no editor" \
  'grep -aq "debug seletor: aberto (abrir) para Editor de Texto" "$T/shell.log" && grep -aq "debug seletor: escolheu $SEL/notas.md" "$T/shell.log" && "$N" msg -j windows | grep -q "\"title\": *\"notas.md"'
xdotool key --window "$W" ctrl+n
sleep 1
xdotool type --window "$W" --delay 20 "relatorio de novembro"
xdotool key --window "$W" ctrl+shift+s
for _ in $(seq 40); do grep -aq "debug seletor: aberto (salvar)" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 20); do grep -aq "debug central: seletor etiqueta Trabalho em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x seletor-salvar
read -r JX JY < <(janela_app os.kalango.Seletor); read -r CX CY < <(lugar "seletor etiqueta Trabalho")
apontar $((JX + CX)) $((JY + CY)); sleep 0.3; xdotool click 1
sleep 0.5
read -r CX CY < <(lugar "seletor salvar")
apontar $((JX + CX)) $((JY + CY)); sleep 0.3; xdotool click 1
SALVO="$SEL/relatorio de novembro.md"
for _ in $(seq 30); do [ -s "$SALVO" ] && break; sleep 0.3; done
sleep 1
echo "  (salvo: $(ls "$SEL" | tr '\n' ' '); etiquetas: $(python3 -c 'import os,sys; print(os.getxattr(sys.argv[1], "user.xdg.tags").decode())' "$SALVO" 2>/dev/null))"
ok "Seletor: \"Salvar como\" compacto grava onde o seletor abriu, com o nome das primeiras palavras e a tag escolhida no próprio arquivo" \
  '[ -s "$SALVO" ] && grep -q "relatorio de novembro" "$SALVO" && python3 -c "import os,sys; sys.exit(0 if \"Trabalho\" in os.getxattr(sys.argv[1], \"user.xdg.tags\").decode() else 1)" "$SALVO"'
fechar_apps
# another system's app (a browser) asks through xdg-desktop-portal (a stand-in plays the portal): the same chooser,
# with the app's "*.txt" filter; the chosen file goes back to the app
rm -f "$OUT/seletor-portal"
F0=$(grep -ac "debug central: seletor arquivo fatura-outubro.txt em" "$T/shell.log")
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/seletor-teste.py" abrir "$OUT/seletor-portal" com.google.Chrome
for _ in $(seq 40); do [ "$(grep -ac "debug central: seletor arquivo fatura-outubro.txt em" "$T/shell.log")" -gt "$F0" ] && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <(janela_app os.kalango.Seletor); read -r CX CY < <(lugar "seletor arquivo fatura-outubro.txt")
apontar $((JX + CX)) $((JY + CY)); sleep 0.3; xdotool click --repeat 2 --delay 90 1
for _ in $(seq 30); do [ -s "$OUT/seletor-portal" ] && break; sleep 0.3; done
echo "  (resposta ao app: $(tr '\n' ' ' < "$OUT/seletor-portal" 2>/dev/null))"
ok "Seletor: um app de outro sistema pede pelo portal e recebe o arquivo escolhido no mesmo seletor" \
  'grep -aq "debug portal: OpenFile de «" "$T/shell.log" && grep -q "^resposta 0" "$OUT/seletor-portal" && grep -q "fatura-outubro.txt" "$OUT/seletor-portal"'
# 62. The admin password (design "Autenticação (senha de administrador)"): the shell is polkit's agent — the dialog
#     says who asks and what changes; a wrong password shakes in coral and asks again (a new run of polkit's helper);
#     the right one authorizes
PK="$OUT/polkit"; mkdir -p "$PK"
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/polkit-teste.py" "$PK"
for _ in $(seq 40); do grep -q "registrado" "$PK/polkit.log" 2>/dev/null && break; sleep 0.3; done
printf '{"acao": "org.kalango.instalar", "mensagem": "É necessária autenticação para instalar o Termius", "comando": "/usr/bin/apt-get install termius"}' > "$PK/pedir.json"
for _ in $(seq 30); do grep -aq "debug polkit: pedindo a senha" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x autenticacao
xdotool type --window "$W" --delay 30 "errada123"; xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug polkit: senha recusada" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
xdotool type --window "$W" --delay 30 "kalango-teste"; xdotool key --window "$W" Return
for _ in $(seq 30); do [ -s "$PK/resposta.json" ] && break; sleep 0.3; done
echo "  (o polkitd de teste: $(tr '\n' ' ' < "$PK/polkit.log"))"
ok "Autenticação: o shell é o agente do polkit — diz quem pede e o que muda; a senha errada volta em coral e a certa autoriza" \
  'grep -q "registrado unix-" "$PK/polkit.log" && grep -aq "debug polkit: senha recusada" "$T/shell.log" && grep -q "\"ok\": true" "$PK/resposta.json" && grep -q "helper: errada" "$PK/polkit.log" && grep -q "helper: certa" "$PK/polkit.log"'
# 63. Privacy as visible state (design "Privacidade visível (quem está vendo o quê)"): who is capturing comes from
#     PipeWire's graph (a stand-in: Discord in a call on the microphone, OBS receiving the screen cast) as chips in the
#     bar; the panel's switch mutes the microphone; "Pausar tudo" mutes it and cuts the screen stream, and a camera
#     that opens while paused is cut as it comes; Super+Shift+P again gives back only what the pause muted; the
#     camera's switch blocks it in the system (WirePlumber's camera monitors off, in the test profile)
PWT="$ROOT/sessao/ferramentas/pw-dump-teste"
capturas() { KALANGO_PW_ESTADO="$OUT/pipewire.json" "$PWT" --capturas "$1"; }
estado_cap() { python3 -c "import json; print(' '.join('%s:%s:%s' % (c['app'], c['tipo'], 'mudo' if c.get('mudo') else 'ativo') for c in json.load(open('$OUT/pipewire.json')).get('capturas', [])))"; }
capturas '[{"id": 120, "tipo": "microfone", "app": "Discord", "chamada": true}, {"id": 122, "tipo": "tela", "app": "OBS"}]'
for _ in $(seq 30); do grep -aq "debug privacidade agora: .*Discord; .*OBS" "$T/shell.log" && break; sleep 0.3; done
"$N" msg action spawn -- "$T/kalango-shell" privacidade
for _ in $(seq 20); do grep -aq "debug privacidade: privacidade pausar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
foto_x privacidade
ok "Privacidade: quem captura vira indicador na barra (Discord no microfone, OBS recebendo a tela) e o painel abre" \
  'grep -aq "debug privacidade agora: .*Discord; .*OBS" "$T/shell.log" && grep -aq "debug privacidade agora: painel aberto (2 acessos)" "$T/shell.log"'
read -r MX MY < <(grep -a "debug privacidade: privacidade chave microfone em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$MX" "$MY"
for _ in $(seq 20); do [ "$(estado_cap)" = "Discord:microfone:mudo OBS:tela:ativo" ] && break; sleep 0.3; done
C1=$(estado_cap)
ok "Privacidade: a chave do microfone o silencia na hora (no PipeWire)" '[ "$C1" = "Discord:microfone:mudo OBS:tela:ativo" ]'
clicar "$MX" "$MY"
for _ in $(seq 20); do [ "$(estado_cap)" = "Discord:microfone:ativo OBS:tela:ativo" ] && break; sleep 0.3; done
sleep 2.5
read -r PX PY < <(grep -a "debug privacidade: privacidade pausar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$PX" "$PY"
for _ in $(seq 20); do [ "$(estado_cap)" = "Discord:microfone:mudo" ] && break; sleep 0.3; done
C2=$(estado_cap)
capturas "$(python3 -c "import json; e=json.load(open('$OUT/pipewire.json')); print(json.dumps(e['capturas'] + [{'id': 121, 'tipo': 'camera', 'app': 'Zoom'}]))")"
for _ in $(seq 30); do grep -aq "debug privacidade agora: .*Zoom cortado" "$T/shell.log" && break; sleep 0.3; done
# (the line is logged as the cut starts: the stream is gone a moment later)
for _ in $(seq 20); do [ "$(estado_cap)" = "Discord:microfone:mudo" ] && break; sleep 0.2; done
C3=$(estado_cap)
echo "  (pausado: $C2; a câmera aberta na pausa: $C3)"
ok "Privacidade: \"Pausar tudo\" silencia o microfone, corta a tela, e a câmera aberta durante a pausa é cortada ao chegar" \
  '[ "$C2" = "Discord:microfone:mudo" ] && [ "$C3" = "Discord:microfone:mudo" ] && grep -aq "debug privacidade agora: tudo pausado" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" privacidade pausar
for _ in $(seq 20); do [ "$(estado_cap)" = "Discord:microfone:ativo" ] && break; sleep 0.3; done
C4=$(estado_cap)
ok "Privacidade: Super+Shift+P de novo retoma (o microfone que a pausa silenciou volta)" '[ "$C4" = "Discord:microfone:ativo" ] && grep -aq "debug privacidade agora: retomado" "$T/shell.log"'
sleep 2.5
read -r KX KY < <(grep -a "debug privacidade: privacidade chave bloquear camera em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$KX" "$KY"
REGRA="$X/config/wireplumber/main.lua.d/51-kalango-camera.lua"
for _ in $(seq 20); do [ -e "$REGRA" ] && break; sleep 0.3; done
ok "Privacidade: a chave da câmera a bloqueia no sistema (os monitores de câmera do WirePlumber desligados)" \
  'grep -q "v4l2_monitor.enabled = false" "$REGRA" && grep -q "libcamera_monitor.enabled = false" "$REGRA"'
rm -f "$REGRA"
capturas '[]'
"$N" msg action spawn -- "$T/kalango-shell" privacidade
sleep 1
# 64. An app asks for the microphone (design "Permissão de app (microfone)"): xdg-desktop-portal's Access goes to the
#     shell (a stand-in plays the frontend): the card says who asks for what; "Só desta vez" answers yes, and once the
#     app has no window the permission the frontend stored is taken back; Esc is "Não permitir"
AC="$ROOT/sessao/ferramentas/acesso-teste.py"
rm -f "$OUT/acesso1" "$OUT/acesso2" "$OUT/acesso-lista"
"$N" msg action spawn -- "$AC" pedir "$OUT/acesso1" com.discordapp.Discord audio-input-microphone-symbolic
for _ in $(seq 30); do grep -aq "debug permissao: pedindo microfone para" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 20); do grep -aq "debug central: permissao uma vez em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
foto_x permissao
ok "Permissão: o pedido do portal vira o cartão do sistema (quem pede, o quê)" 'grep -aq "debug portal: AccessDialog de" "$T/shell.log" && grep -aq "debug permissao: pedindo microfone para «" "$T/shell.log"'
read -r UX UY < <(grep -a "debug central: permissao uma vez em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$UX" "$UY"
for _ in $(seq 20); do [ -s "$OUT/acesso1" ] && break; sleep 0.3; done
"$N" msg action spawn -- "$AC" guardar com.discordapp.Discord
for _ in $(seq 50); do grep -aq "debug permissao: microfone de «com.discordapp.Discord» devolvida" "$T/shell.log" && break; sleep 0.3; done
"$N" msg action spawn -- "$AC" consultar "$OUT/acesso-lista"
for _ in $(seq 20); do [ -e "$OUT/acesso-lista" ] && break; sleep 0.3; done
echo "  (resposta: $(cat "$OUT/acesso1" 2>/dev/null); guardado depois: $(tr '\n' ' ' < "$OUT/acesso-lista" 2>/dev/null))"
ok "Permissão: \"Só desta vez\" responde sim e devolve a permissão guardada quando o app não tem mais janela" \
  'grep -q "resposta 0" "$OUT/acesso1" && grep -aq "debug permissao: respondido só desta vez" "$T/shell.log" && grep -aq "debug permissao: microfone de «com.discordapp.Discord» devolvida$" "$T/shell.log" && ! grep -q "com.discordapp.Discord" "$OUT/acesso-lista"'
"$N" msg action spawn -- "$AC" pedir "$OUT/acesso2" com.discordapp.Discord camera-web-symbolic
for _ in $(seq 30); do grep -aq "debug permissao: pedindo câmera para" "$T/shell.log" && break; sleep 0.3; done
sleep 1
xdotool key --window "$W" Escape
for _ in $(seq 20); do [ -s "$OUT/acesso2" ] && break; sleep 0.3; done
ok "Permissão: Esc é \"Não permitir\" (o portal recebe a recusa)" 'grep -q "resposta 1" "$OUT/acesso2" && grep -aq "debug permissao: respondido não permitir" "$T/shell.log"'
# 66. Focus mode (design "Modo foco"): the timer starts a session about the window in focus — the island becomes the
#     timer and the desktop behind the windows dims; a notice that arrives is held (counted at the bottom left); the
#     panel opens from the island; "Encerrar" ends it with the summary of what waited
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 1.5
F0=$(grep -ac "debug foco: começou" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" foco
for _ in $(seq 20); do [ "$(grep -ac "debug foco: começou" "$T/shell.log")" -gt "$F0" ] && break; sleep 0.3; done
"$N" msg action spawn -- notify-send -a Teste "Mensagem durante o foco" "Esta deve esperar o fim da sessão."
for _ in $(seq 30); do grep -aq "debug foco: 1 guardadas" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
foto_x foco
ok "Foco: a sessão começa sobre a janela em foco, a área de trabalho escurece e o aviso que chega fica guardado" \
  'grep -aq "debug foco: começou «" "$T/shell.log" && grep -aq "debug foco: a área de trabalho escurece" "$T/shell.log" && grep -aq "debug foco: 1 guardadas" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" foco painel
for _ in $(seq 20); do grep -aq "debug foco: foco encerrar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
foto_x foco-painel
read -r EX EY < <(grep -a "debug foco: foco encerrar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$EX" "$EY"
for _ in $(seq 30); do grep -aq "debug foco: resumo (" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug foco: resumo (" "$T/shell.log" | tail -1))"
ok "Foco: \"Encerrar\" termina a sessão e o resumo diz o que esperou" \
  '[ "$(grep -ac "debug foco: encerrado" "$T/shell.log")" -ge 2 ] && grep -a "debug foco: resumo (" "$T/shell.log" | tail -1 | grep -q "1 aviso esperou"'
fechar_apps
# 67. Sharing the screen (design "Compartilhar tela"): xdg-desktop-portal's ScreenCast goes to the shell (a stand-in
#     plays the portal, another niri's org.gnome.Mutter.ScreenCast — the nested niri cannot cast): the system's
#     chooser lists the windows with their pictures; the chosen one is recorded (RecordWindow) and its PipeWire node
#     goes back to the app; the app closing its session stops the recording
CT="$ROOT/sessao/ferramentas/compartilhar-teste.py"
rm -f "$OUT/mutter.log" "$OUT/compartilhar"
"$N" msg action spawn -- "$CT" mutter "$OUT/mutter.log"
for _ in $(seq 30); do grep -q registrado "$OUT/mutter.log" 2>/dev/null && break; sleep 0.3; done
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 1.5
"$N" msg action spawn -- "$CT" pedir "$OUT/compartilhar" com.google.Chrome
for _ in $(seq 30); do grep -aq "debug central: compartilhar fonte 0 em" "$T/shell.log" && break; sleep 0.3; done
sleep 1.2
foto_x compartilhar
ok "Compartilhar: o pedido do app vira o seletor do sistema, com as janelas abertas" \
  'grep -aq "debug compartilhar: pedido de «[^»]*Chrome»" "$T/shell.log" && grep -aq "debug compartilhar: [1-9][0-9]* janelas" "$T/shell.log"'
read -r JX JY < <(grep -a "debug central: compartilhar fonte 0 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$JX" "$JY"
read -r BX BY < <(grep -a "debug central: compartilhar botao em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$BX" "$BY"
for _ in $(seq 30); do [ -s "$OUT/compartilhar" ] && break; sleep 0.3; done
echo "  (portal: $(tr '\n' ' ' < "$OUT/compartilhar" 2>/dev/null); niri: $(tr '\n' ' ' < "$OUT/mutter.log"))"
ok "Compartilhar: a janela escolhida é gravada pelo niri e o nó do PipeWire volta ao app" \
  'grep -q "resposta 0" "$OUT/compartilhar" && grep -q "^stream 77 2 " "$OUT/compartilhar" && grep -q "^RecordWindow [0-9]" "$OUT/mutter.log" && grep -q "^Start" "$OUT/mutter.log"'
"$N" msg action spawn -- "$CT" fechar
for _ in $(seq 20); do grep -q "^Stop" "$OUT/mutter.log" && break; sleep 0.3; done
ok "Compartilhar: o app fechando a sessão para a gravação" 'grep -q "^Stop" "$OUT/mutter.log" && grep -aq "debug compartilhar: parou" "$T/shell.log"'
fechar_apps
# 68. The screenshot tool (design "Captura de tela"; Print): the screen freezes under the veil with the bar; an area
#     is drawn by dragging; "Capturar" saves it (Imagens/Capturas de tela) and copies it; a video of the area: niri
#     records (the ScreenCast stand-in), GStreamer (a stand-in) writes it cropped, the island shows it and stops it
"$N" msg action spawn -- "$CT" mutter "$OUT/mutter.log"
sleep 1
"$N" msg action spawn -- "$T/kalango-shell" captura
for _ in $(seq 30); do grep -aq "debug captura: aberta" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 20); do grep -aq "debug central: captura capturar em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.6
# (from outside the area it opens with, so it draws a new one)
xdotool mousemove --window "$W" 60 100; sleep 0.2
xdotool mousedown 1; sleep 0.1; xdotool mousemove --window "$W" 200 220; sleep 0.1; xdotool mousemove --window "$W" 360 350; sleep 0.2; xdotool mouseup 1
sleep 0.5
foto_x captura
ok "Captura: Print congela a tela com a barra; arrastar desenha a área" 'grep -aq "debug captura: aberta" "$T/shell.log" && grep -aq "debug captura: área 300×250 em 60 100" "$T/shell.log"'
read -r CX CY < <(grep -a "debug central: captura capturar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$CX" "$CY"
for _ in $(seq 20); do grep -aq "debug captura: foto " "$T/shell.log" && break; sleep 0.3; done
FOTO=$(ls "$OUT/capturas/Imagens/Capturas de tela/"*.png 2>/dev/null | head -1)
echo "  (foto: ${FOTO##*/} $(python3 -c "from PIL import Image; import sys; print(Image.open(sys.argv[1]).size)" "$FOTO" 2>/dev/null))"
ok "Captura: \"Capturar\" salva a área em Imagens/Capturas de tela" '[ -s "$FOTO" ] && grep -aq "debug captura: foto .*(300x250)" "$T/shell.log"'
sem_avisos 2>/dev/null || sleep 3
"$N" msg action spawn -- "$T/kalango-shell" captura
for _ in $(seq 30); do [ "$(grep -ac "debug captura: aberta" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
sleep 1
read -r VX VY < <(grep -a "debug central: captura video em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$VX" "$VY"
read -r CX CY < <(grep -a "debug central: captura capturar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$CX" "$CY"
for _ in $(seq 30); do grep -aq "debug captura: gravando" "$T/shell.log" && break; sleep 0.3; done
sleep 2
read -r IX IY < <(grep -a "debug central: ilha gravando em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$IX" "$IY"
for _ in $(seq 30); do grep -aq "debug captura: vídeo salvo" "$T/shell.log" && break; sleep 0.3; done
VIDEO=$(ls "$OUT/capturas/Vídeos/Gravações de tela/"*.webm 2>/dev/null | head -1)
echo "  (gst: $(head -1 "$OUT/gst.log" 2>/dev/null | cut -c1-140))"
ok "Captura: o vídeo da área é gravado pelo niri, recortado, e um clique na ilha o salva em Vídeos" \
  'grep -aq "debug captura: gravando (nó 77, área" "$T/shell.log" && [ -s "$VIDEO" ] && grep -q "pipewiresrc path=77" "$OUT/gst.log" && grep -q "videocrop left=60 top=100" "$OUT/gst.log" && grep -q "^fim" "$OUT/gst.log"'
# "Extrair texto": the local vision model (LFM3-VL) reads the chosen window (a new editor with a line typed, in
# front, no banner over it)
fechar_apps
# (a file not there yet: the Editor opens it empty, and saving would create it)
abrir_ed "$OUT/lista-de-teste.txt"
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2.5
"$N" msg action focus-window --id "$(id_da_janela os.kalango.Editor)"
sleep 0.6
xdotool type --window "$W" --delay 30 "Lista KalangoOS 2026"
sleep 0.5
sem_avisos 2>/dev/null || sleep 4
"$N" msg action spawn -- "$T/kalango-shell" captura
for _ in $(seq 30); do [ "$(grep -ac "debug captura: aberta" "$T/shell.log")" -ge 3 ] && break; sleep 0.3; done
sleep 1
read -r JX JY < <(grep -a "debug central: captura janela em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$JX" "$JY"
read -r EX EY < <(grep -a "debug central: captura extrair em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$EX" "$EY"
for _ in $(seq 240); do grep -aq "debug captura: texto extraído\|debug captura: ler o texto falhou" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug captura: texto extraído\|debug captura: ler o texto falhou" "$T/shell.log" | tail -1 | cut -c1-160))"
ok "Captura: \"Extrair texto\" — o modelo de visão local lê a janela e o texto é copiado" 'grep -a "debug captura: texto extraído" "$T/shell.log" | tail -1 | grep -iq "Kalango.*2026"'
fechar_apps
# 69. The shortcuts sheet (design "Folha de atalhos"; Super+/, bound in niri's config): every
#     shortcut as it is now, in its groups; typing filters it; Esc closes
A0=$(grep -ac "debug atalhos: folha aberta" "$T/shell.log")
# (what Super+/ runs: the key itself depends on the layout Xvfb and niri read it with)
"$N" msg action spawn -- "$T/kalango-shell" atalhos
for _ in $(seq 20); do [ "$(grep -ac "debug atalhos: folha aberta" "$T/shell.log")" -gt "$A0" ] && break; sleep 0.3; done
sleep 0.8
foto_x atalhos
ok "Atalhos: Super+/ abre a folha com os atalhos de agora em seus grupos" \
  'grep -q "Mod+Slash .*\"atalhos\"" "$T/niri.kdl" && [ "$(grep -ac "debug atalhos: folha aberta" "$T/shell.log")" -gt "$A0" ] && grep -a "debug atalhos: [0-9]* atalhos em [0-9]* grupos" "$T/shell.log" | tail -1 | awk "{exit !(\$3 >= 15)}"'
xdotool type --window "$W" --delay 60 "captura"
for _ in $(seq 20); do grep -aq "debug atalhos: filtro «captura» mostra" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug atalhos: filtro «captura» mostra" "$T/shell.log" | tail -1))"
ok "Atalhos: digitar filtra ao vivo" 'grep -a "debug atalhos: filtro «captura» mostra" "$T/shell.log" | tail -1 | grep -q "mostra 1$"'
xdotool key --window "$W" Escape
sleep 0.8
# 70. The expanded island (design "Ilha expandida (atividades ao vivo)"): with something happening (a player playing:
#     the MPRIS stand-in) a click grows the island into the card of live activities; its buttons act (the player gets
#     PlayPause); Esc folds it; with nothing happening a click opens the notices, as before
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/mpris-teste.py" Alright "Jonathan Ogden" "$OUT/mpris.log"
sleep 2.5
I0=$(grep -ac "debug ilha: expandida" "$T/shell.log")
xdotool mousemove --window "$W" $((OW / 2)) 16 click 1
for _ in $(seq 20); do [ "$(grep -ac "debug ilha: expandida" "$T/shell.log")" -gt "$I0" ] && break; sleep 0.3; done
sleep 0.8
foto_x ilha
ok "Ilha: com algo acontecendo (a música), o clique a expande no cartão das atividades" '[ "$(grep -ac "debug ilha: expandida" "$T/shell.log")" -gt "$I0" ]'
read -r MX MY < <(grep -a "debug central: ilha cartao PlayPause em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$MX" "$MY"
for _ in $(seq 20); do grep -q "PlayPause" "$OUT/mpris.log" 2>/dev/null && break; sleep 0.3; done
R0=$(grep -ac "debug ilha: recolhida" "$T/shell.log")
xdotool key --window "$W" Escape
for _ in $(seq 20); do [ "$(grep -ac "debug ilha: recolhida" "$T/shell.log")" -gt "$R0" ] && break; sleep 0.3; done
ok "Ilha: os botões do cartão agem (o player recebe PlayPause) e Esc a recolhe" 'grep -q "PlayPause" "$OUT/mpris.log" && [ "$(grep -ac "debug ilha: recolhida" "$T/shell.log")" -gt "$R0" ]'
pkill -f "[m]pris-teste.py Alright"
sleep 2.5
L0=$(grep -ac "debug lateral: aberta (avisos)" "$T/shell.log")
xdotool mousemove --window "$W" $((OW / 2 + 2)) 16 click 1
for _ in $(seq 20); do [ "$(grep -ac "debug lateral: aberta (avisos)" "$T/shell.log")" -gt "$L0" ] && break; sleep 0.3; done
ok "Ilha: sem nada acontecendo, o clique abre os avisos" '[ "$(grep -ac "debug lateral: aberta (avisos)" "$T/shell.log")" -gt "$L0" ]'
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
sleep 1
# 71. Settings › Appearance › Dock and multitasking (design "Configurações · Dock e multitarefa"): the pointer resting
#     on the right edge opens the notices' bar; "Larga" makes the bars 480 px; the top-left corner opens the
#     overview; Super+D goes to the desktop (the empty space) and back; niri gets the focus rule from janelas.kdl
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
sleep 0.8
B0=$(grep -ac "debug bordas: direita" "$T/shell.log")
xdotool mousemove --window "$W" $((OW - 30)) $((OH / 2)); sleep 0.2; xdotool mousemove --window "$W" $((OW - 1)) $((OH / 2))
for _ in $(seq 20); do [ "$(grep -ac "debug bordas: direita" "$T/shell.log")" -gt "$B0" ] && break; sleep 0.2; done
sleep 0.6
ok "Dock e multitarefa: o ponteiro parado na borda direita abre a barra dos avisos" '[ "$(grep -ac "debug bordas: direita" "$T/shell.log")" -gt "$B0" ] && grep -a "debug lateral: aberta" "$T/shell.log" | tail -1 | grep -q avisos'
xdotool mousemove --window "$W" $((OW / 2)) $((OH / 2))
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
sleep 0.8
"$N" msg action spawn -- "$T/kalango-shell" configuracoes aparencia
for _ in $(seq 30); do grep -aq "debug configuracoes: página aparencia" "$T/shell.log" && break; sleep 0.3; done
sleep 2
clique_cfg "cfg aba dock"
sleep 1.2
foto_x dock-multitarefa
read -r JX JY < <(janela_cfg)
apontar $((JX + 400)) $((JY + 400)); sleep 0.2
xdotool click --repeat 8 --delay 40 5
nova_posicao "cfg aparencia larga"
clique_cfg "cfg aparencia larga"
for _ in $(seq 20); do grep -aq "debug aparencia: largura das barras larga (480 px)" "$T/shell.log" && break; sleep 0.3; done
ok "Dock e multitarefa: \"Larga\" deixa as barras com 480 px" 'grep -aq "debug aparencia: largura das barras larga (480 px)" "$T/shell.log" && grep -q "\"barras_largura\": \"larga\"" "$X/config/kalango/configuracoes.json"'
clique_cfg "cfg aparencia encaixe"
for _ in $(seq 20); do grep -aq "debug janelas: encaixe ao arrastar false" "$T/shell.log" && break; sleep 0.3; done
clique_cfg "cfg aparencia encaixe"
for _ in $(seq 20); do [ "$(grep -ac "debug janelas: encaixe ao arrastar true" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
ok "Dock e multitarefa: a chave do encaixe ao arrastar chega ao niri (desliga e liga)" \
  'grep -aq "debug janelas: encaixe ao arrastar false$" "$T/shell.log" && [ "$(grep -ac "debug janelas: encaixe ao arrastar true$" "$T/shell.log")" -ge 2 ]'
clique_cfg "cfg aparencia sugere layouts"
for _ in $(seq 20); do grep -aq "debug layouts: sugestões false" "$T/shell.log" && break; sleep 0.3; done
clique_cfg "cfg aparencia sugere layouts"
for _ in $(seq 20); do grep -aq "debug layouts: sugestões true" "$T/shell.log" && break; sleep 0.3; done
ok "Dock e multitarefa: \"O agente sugere layouts\" desliga e liga, e fica guardado" \
  'grep -aq "debug layouts: sugestões false" "$T/shell.log" && grep -aq "debug layouts: sugestões true" "$T/shell.log" && grep -q "\"sugerir_layouts\": true" "$X/config/kalango/configuracoes.json"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
V0=$(grep -ac "debug bordas: visao" "$T/shell.log")
xdotool mousemove --window "$W" 40 40; sleep 0.2; xdotool mousemove --window "$W" 0 0
for _ in $(seq 20); do [ "$(grep -ac "debug bordas: visao" "$T/shell.log")" -gt "$V0" ] && break; sleep 0.2; done
sleep 0.8
ok "Dock e multitarefa: o canto de cima à esquerda abre a visão geral" '[ "$(grep -ac "debug bordas: visao" "$T/shell.log")" -gt "$V0" ] && grep -a "debug visão geral: aberta" "$T/shell.log" | tail -1 | grep -q aberta'
xdotool mousemove --window "$W" $((OW / 2)) $((OH / 2))
xdotool key --window "$W" Escape
sleep 1
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 1.5
"$N" msg action spawn -- "$T/kalango-shell" area-de-trabalho
for _ in $(seq 20); do grep -aq "debug bordas: área de trabalho (espaço" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
VAZIO=$("$N" msg -j focused-window | python3 -c 'import json,sys; print("vazio" if json.load(sys.stdin) is None else "janela")')
"$N" msg action spawn -- "$T/kalango-shell" area-de-trabalho
for _ in $(seq 20); do grep -aq "debug bordas: de volta ao espaço" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
echo "  (na área de trabalho: $VAZIO)"
ok "Dock e multitarefa: Super+D mostra a área de trabalho (o espaço vazio) e volta" '[ "$VAZIO" = vazio ] && grep -aq "debug bordas: de volta ao espaço" "$T/shell.log" && "$N" msg -j focused-window | grep -q os.kalango.Calculadora'
ok "Dock e multitarefa: o foco que segue o ponteiro vem do janelas.kdl, que o niri inclui" 'grep -q "focus-follows-mouse" "$X/config/kalango/janelas.kdl" && grep -q "janelas.kdl" "$T/niri.kdl"'
fechar_apps
# the dock's recent and suggested apps: the calculator, opened from the grid (§20) and closed now, comes after the
# pinned and open ones
for _ in $(seq 20); do grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "+os.kalango.Calculadora" && break; sleep 0.3; done
grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | sed 's/^debug dock: /  (/; s/$/)/'
ok "Dock e multitarefa: a calculadora aberta pela grade, já fechada, fica na dock entre os recentes" 'grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "+os.kalango.Calculadora"'
# "Ocultar automaticamente": on, the dock leaves the screen; the pointer on the bottom edge brings it back and, away
# from it, the dock goes again; off, it is back and stays. "Mostrar apps recentes e sugeridos" off leaves the pinned
# and open apps only
P0=$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" configuracoes aparencia
for _ in $(seq 30); do [ "$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log")" -gt "$P0" ] && break; sleep 0.3; done
sleep 2
clique_cfg "cfg aba dock"
sleep 1.2
O0=$(grep -ac "debug dock: oculta$" "$T/shell.log")
clique_cfg "cfg aparencia dock ocultar"
for _ in $(seq 20); do [ "$(grep -ac "debug dock: oculta$" "$T/shell.log")" -gt "$O0" ] && break; sleep 0.3; done
sleep 1
foto_x dock-oculta
ok "Dock e multitarefa: \"Ocultar automaticamente\" tira a dock da tela e fica guardado" \
  '[ "$(grep -ac "debug dock: oculta$" "$T/shell.log")" -gt "$O0" ] && python3 -c "import json,sys; sys.exit(not json.load(open(sys.argv[1]))[\"ocultar\"])" "$X/config/kalango/dock-estilo.json" && "$N" msg layers | grep -q kalango-borda-dock'
M0=$(grep -ac "debug dock: à mostra" "$T/shell.log")
xdotool mousemove --window "$W" $((OW / 2)) $((OH - 40)); sleep 0.2; xdotool mousemove --window "$W" $((OW / 2)) $((OH - 1))
for _ in $(seq 20); do [ "$(grep -ac "debug dock: à mostra" "$T/shell.log")" -gt "$M0" ] && break; sleep 0.2; done
sleep 0.4
foto_x dock-de-volta
ok "Dock e multitarefa: o ponteiro na borda de baixo traz a dock oculta" '[ "$(grep -ac "debug dock: à mostra" "$T/shell.log")" -gt "$M0" ]'
O1=$(grep -ac "debug dock: oculta$" "$T/shell.log")
xdotool mousemove --window "$W" $((OW / 2)) $((OH / 2))
for _ in $(seq 25); do [ "$(grep -ac "debug dock: oculta$" "$T/shell.log")" -gt "$O1" ] && break; sleep 0.2; done
ok "Dock e multitarefa: e com o ponteiro longe dela, a dock sai de novo" '[ "$(grep -ac "debug dock: oculta$" "$T/shell.log")" -gt "$O1" ]'
clique_cfg "cfg aparencia dock ocultar"
for _ in $(seq 20); do python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1]))[\"ocultar\"])" "$X/config/kalango/dock-estilo.json" && break; sleep 0.3; done
sleep 1
ok "Dock e multitarefa: desligado, a dock volta e fica (sem a borda que a traz)" \
  'python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1]))[\"ocultar\"])" "$X/config/kalango/dock-estilo.json" && ! "$N" msg layers | grep -q kalango-borda-dock && grep -a "debug dock: flutuante" "$T/shell.log" | tail -1 | grep -qv oculta'
clique_cfg "cfg aparencia dock recentes"
for _ in $(seq 20); do grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "\[\]" && break; sleep 0.3; done
ok "Dock e multitarefa: sem \"apps recentes e sugeridos\", só os fixados e os abertos" 'grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "\[\]"'
clique_cfg "cfg aparencia dock recentes"
for _ in $(seq 20); do grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "os.kalango.Calculadora" && break; sleep 0.3; done
ok "Dock e multitarefa: e ligado de novo, os recentes voltam" 'grep -a "debug dock: recentes e sugeridos" "$T/shell.log" | tail -1 | grep -q "os.kalango.Calculadora" && python3 -c "import json,sys; sys.exit(not json.load(open(sys.argv[1]))[\"recentes\"])" "$X/config/kalango/dock-estilo.json"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 72. Spaces by intention (design "Espaços por intenção", Super+Tab): the screen shows each space with its apps; a
#     click on a card goes there; in a space whose rule holds the notifications a notice waits in the list; the local
#     model builds a new space from a sentence (named, with its apps open in it); a space entered at this hour on
#     three days is offered as a routine, and "Ativar rotina" keeps it; Settings' "Espaços por intenção" turns off
fechar_apps
"$N" msg action focus-workspace 1
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
"$N" msg action focus-workspace 2
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 1
"$N" msg action set-workspace-name Estudar
"$N" msg action focus-workspace 1
sleep 1
A0=$(grep -ac "debug espaços: aberto" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" espacos
for _ in $(seq 30); do [ "$(grep -ac "debug espaços: aberto" "$T/shell.log")" -gt "$A0" ] && break; sleep 0.3; done
sleep 1.2
foto_x espacos
grep -a "debug espaços: aberto" "$T/shell.log" | tail -1 | sed 's/^debug espaços: /  (/; s/$/)/'
ok "Espaços: Super+Tab abre a tela com os espaços e seus apps" 'grep -a "debug espaços: aberto" "$T/shell.log" | tail -1 | grep -q "([2-9] espaços)" && grep -aq "debug espaços: cartão [0-9] Estudar" "$T/shell.log"'
read -r EX EY < <(grep -a "debug espaços: cartão [0-9] Estudar" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "${EX:-0}" "${EY:-0}" click 1
for _ in $(seq 20); do "$N" msg -j focused-window | grep -q os.kalango.Editor && break; sleep 0.3; done
espaco_focado() { "$N" msg -j workspaces | python3 -c 'import json,sys; print([w["name"] or "" for w in json.load(sys.stdin) if w["is_focused"]][0])'; }
ok "Espaços: um clique no cartão vai para o espaço" 'grep -aq "debug espaços: para o espaço" "$T/shell.log" && [ "$(espaco_focado)" = Estudar ]'
python3 - "$X/config/kalango/espacos.json" <<'PYEOF'
import json, sys
p = sys.argv[1]
try: v = json.load(open(p))
except Exception: v = {}
v.setdefault("espacos", {})["Estudar"] = {"cor": 1, "interromper": "guardar", "agentes_silencio": False}
json.dump(v, open(p, "w"))
PYEOF
notificar Teste "Lembrete do grupo" "Reunião às 15h"
for _ in $(seq 20); do grep -aq "debug aviso: guardada pela regra do espaço: Lembrete do grupo" "$T/shell.log" && break; sleep 0.3; done
ok "Espaços: no espaço que guarda as notificações, o aviso vai só para a lista" 'grep -aq "debug aviso: guardada pela regra do espaço: Lembrete do grupo" "$T/shell.log"'
"$N" msg action focus-workspace 1
sleep 0.8
notificar Teste "Lembrete solto" "Fora do espaço Estudar"
for _ in $(seq 20); do grep -aq "debug aviso: marcar como lida em .* :: Lembrete solto" "$T/shell.log" && break; sleep 0.3; done
ok "Espaços: e fora dele o aviso aparece" '! grep -aq "debug aviso: guardada pela regra do espaço: Lembrete solto" "$T/shell.log" && grep -aq "debug aviso: marcar como lida em .* :: Lembrete solto" "$T/shell.log"'
sem_avisos
# "Montar espaço": the local model (LFM3) names the space and picks its apps; the shell opens them in it
A1=$(grep -ac "debug espaços: aberto" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" espacos
for _ in $(seq 30); do [ "$(grep -ac "debug espaços: aberto" "$T/shell.log")" -gt "$A1" ] && break; sleep 0.3; done
sleep 1.2
read -r FX FY < <(grep -a "debug espaços: campo em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "${FX:-0}" "${FY:-0}" click 1
sleep 0.4
xdotool type --window "$W" --delay 40 "escrever um texto e fazer contas"
xdotool key --window "$W" Return
for _ in $(seq 160); do grep -aq "debug espaços: espaço montado" "$T/shell.log" && break; sleep 0.5; done
grep -a "debug espaços: espaço montado" "$T/shell.log" | tail -1 | sed 's/^debug espaços: /  (/; s/$/)/'
sleep 2.5
foto_x espaco-montado
# (the model says the kinds of app — a text editor and a calculator here — and the shell finds KalangoOS's own)
ok "Espaços: \"Montar espaço\" — o modelo local dá nome ao espaço e abre nele o Editor e a Calculadora do KalangoOS" \
  'grep -a "debug espaços: espaço montado" "$T/shell.log" | tail -1 | grep "os.kalango.Editor" | grep -q "os.kalango.Calculadora" && [ -n "$(espaco_focado)" ] && [ "$(espaco_focado)" != Estudar ]'
# (the apps the space opened go — this system's or another's: one left open would sit over the next sections' windows)
WS=$("$N" msg -j workspaces | python3 -c 'import json,sys; print([w["id"] for w in json.load(sys.stdin) if w["is_focused"]][0])')
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["workspace_id"] == int(sys.argv[1])]' "$WS" | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1.5
fechar_apps
# a habit: "Estudar" entered at this hour on three other days → offered as a routine; "Ativar rotina" keeps it
python3 - "$X/state/kalango/espacos-uso.json" <<'PYEOF'
import json, sys, time
p = sys.argv[1]
try: v = json.load(open(p))
except Exception: v = []
agora = int(time.time())
v = [["Estudar", agora - d * 86400] for d in (3, 2, 1)] + v
json.dump(v, open(p, "w"))
PYEOF
A2=$(grep -ac "debug espaços: aberto" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" espacos
for _ in $(seq 30); do [ "$(grep -ac "debug espaços: aberto" "$T/shell.log")" -gt "$A2" ] && break; sleep 0.3; done
sleep 1.2
foto_x espacos-habito
grep -a "debug espaços: hábito" "$T/shell.log" | tail -1 | sed 's/^debug espaços: /  (/; s/$/)/'
read -r RX RY < <(grep -a "debug espaços: ativar em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "${RX:-0}" "${RY:-0}" click 1
for _ in $(seq 20); do grep -aq "debug espaços: rotina ativada (Estudar" "$T/shell.log" && break; sleep 0.3; done
ok "Espaços: o hábito de ir a um espaço na mesma hora vira rotina com \"Ativar rotina\"" \
  'grep -aq "debug espaços: rotina ativada (Estudar" "$T/shell.log" && grep -q "\"espaco\": \"Estudar\"" "$X/config/kalango/espacos.json" && grep -q "\"espacos_por_rotina\": true" "$X/config/kalango/configuracoes.json"'
xdotool key --window "$W" Escape
sleep 0.8
# Settings › Appearance › Dock and multitasking › ESPAÇOS: off, the spaces' rules stop
P1=$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" configuracoes aparencia
for _ in $(seq 30); do [ "$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log")" -gt "$P1" ] && break; sleep 0.3; done
sleep 2
clique_cfg "cfg aba dock"
sleep 1.2
foto_x espacos-ajustes
clique_cfg "cfg aparencia espacos intencao"
for _ in $(seq 20); do grep -aq "debug espaços: por intenção false" "$T/shell.log" && break; sleep 0.3; done
clique_cfg "cfg aparencia espacos intencao"
for _ in $(seq 20); do grep -aq "debug espaços: por intenção true" "$T/shell.log" && break; sleep 0.3; done
ok "Espaços: \"Espaços por intenção\" desliga e liga em Configurações" 'grep -aq "debug espaços: por intenção false" "$T/shell.log" && grep -aq "debug espaços: por intenção true" "$T/shell.log"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
fechar_apps
"$N" msg -j workspaces | python3 -c 'import json,sys; [print(w["name"]) for w in json.load(sys.stdin) if w["name"]]' | while read -r nome; do "$N" msg action unset-workspace-name "$nome"; done
rm -f "$X/config/kalango/espacos.json"
"$N" msg action focus-workspace 1
sleep 1
# 73. The fixed dock is the taskbar (designs "Dock fixa · barra de tarefas", "· Menu Iniciar aberto", "· prévia de
#     janela", "· apps à esquerda"): the top bar steps aside; "Esquerda" puts the apps on the left; Iniciar opens the
#     start menu with the pinned apps and Esc closes it; the pointer resting on an open app shows its window's
#     preview; back to the floating dock, the top bar returns
fechar_apps
abrir_aparencia_dock() { local p0; p0=$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log"); "$N" msg action spawn -- "$T/kalango-shell" configuracoes aparencia
  for _ in $(seq 30); do [ "$(grep -ac "debug configuracoes: página aparencia" "$T/shell.log")" -gt "$p0" ] && break; sleep 0.3; done; sleep 2; clique_cfg "cfg aba dock"; sleep 1.2; }
fechar_cfg() { "$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done; sleep 1; }
dock_tem() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); sys.exit(not all(str(d.get(k))==v for k,v in (a.split('=') for a in sys.argv[2:])))" "$X/config/kalango/dock-estilo.json" "$@"; }
abrir_aparencia_dock
L0=$(grep -ac "debug barra de tarefas: iniciar em" "$T/shell.log")
clique_cfg "cfg aparencia dock fixa"
for _ in $(seq 20); do [ "$(grep -ac "debug barra de tarefas: iniciar em" "$T/shell.log")" -gt "$L0" ] && break; sleep 0.3; done
sleep 1
ok "Barra de tarefas: a dock fixa vira a barra de tarefas e a barra de cima sai" \
  '[ "$(grep -ac "debug barra de tarefas: iniciar em" "$T/shell.log")" -gt "$L0" ] && dock_tem estilo=fixa && ! "$N" msg layers | grep -q kalango-barra-superior'
clique_cfg "cfg aparencia alinhamento esquerda"
for _ in $(seq 20); do dock_tem alinhamento=esquerda && break; sleep 0.3; done
sleep 1
foto_x barra-de-tarefas
ok "Barra de tarefas: \"Esquerda\" leva os apps para a esquerda (e fica guardado)" 'dock_tem alinhamento=esquerda'
fechar_cfg
read -r IX ID < <(grep -a "debug barra de tarefas: iniciar em" "$T/shell.log" | tail -1 | awk '{print $7, $9}')
apontar "${IX:-0}" $((OH - ${ID:-0})); sleep 0.3; xdotool click 1
for _ in $(seq 20); do grep -aq "debug menu iniciar: aberto" "$T/shell.log" && break; sleep 0.3; done
sleep 1
foto_x menu-iniciar
ok "Barra de tarefas: Iniciar abre o menu com os apps fixados" 'grep -a "debug menu iniciar: aberto" "$T/shell.log" | tail -1 | grep -q "([1-9][0-9]* fixados)"'
xdotool key --window "$W" Escape
for _ in $(seq 20); do grep -aq "debug menu iniciar: fechado" "$T/shell.log" && break; sleep 0.3; done
ok "Barra de tarefas: e Esc fecha o menu" 'grep -aq "debug menu iniciar: fechado" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" arquivos
for _ in $(seq 30); do "$N" msg -j windows | grep -q os.kalango.Arquivos && break; sleep 0.3; done
sleep 1.5
read -r AX AD < <(grep -a "debug barra de tarefas: primeiro app em" "$T/shell.log" | tail -1 | awk '{print $8, $10}')
apontar $(( ${AX:-0} + 4 )) $((OH - ${AD:-0} - 4)); sleep 0.2; apontar "${AX:-0}" $((OH - ${AD:-0}))
for _ in $(seq 20); do grep -aq "debug prévia: os.kalango.Arquivos" "$T/shell.log" && break; sleep 0.3; done
sleep 1
foto_x previa-de-janela
ok "Barra de tarefas: o ponteiro parado no Arquivos aberto mostra a prévia da janela" 'grep -aq "debug prévia: os.kalango.Arquivos (1 janelas)" "$T/shell.log"'
apontar $((OW / 2)) $((OH / 2))
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Arquivos"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
fechar_apps
# with the taskbar (the whole bottom, no top bar), the side bars are sides of the screen: flush, from the top to the
# taskbar — not floating cards over it
L0=$(grep -ac "debug lateral: .* inteira, do topo até a barra de tarefas" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" lateral avisos abrir
sleep 1.2
"$N" msg action spawn -- "$T/kalango-shell" hoje
for _ in $(seq 20); do [ "$(grep -ac "debug lateral: .* inteira, do topo até a barra de tarefas" "$T/shell.log")" -ge $((L0 + 2)) ] && break; sleep 0.3; done
sleep 1
foto_x barra-de-tarefas-laterais
ok "Barra de tarefas: as barras laterais ficam inteiras, do topo até a barra, encostadas nas bordas" \
  'grep -a "debug lateral: .* inteira, do topo até a barra de tarefas" "$T/shell.log" | tail -2 | grep -q direita && grep -a "debug lateral: .* inteira, do topo até a barra de tarefas" "$T/shell.log" | tail -2 | grep -q esquerda'
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
"$N" msg action spawn -- "$T/kalango-shell" hoje
sleep 1
abrir_aparencia_dock
clique_cfg "cfg aparencia alinhamento centro"
for _ in $(seq 20); do dock_tem alinhamento=centro && break; sleep 0.3; done
clique_cfg "cfg aparencia dock flutuante"
for _ in $(seq 20); do dock_tem estilo=flutuante && break; sleep 0.3; done
sleep 1.5
ok "Barra de tarefas: de volta à dock flutuante, a barra de cima volta" 'dock_tem estilo=flutuante alinhamento=centro && "$N" msg layers | grep -q kalango-barra-superior'
fechar_cfg
# 74. KalangoOS's own Calculator (designs "Calculadora", "· Científica", "· Conversões"; not GNOME's): the keyboard
#     makes the calculation; "15% de 1.240 em dólares" in words; the scientific page's sin; the converter from words
#     ("100 km em milhas") and ⇅
fechar_apps
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"] in ("os.kalango.Arquivos", "os.kalango.Configuracoes")]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
C0=$(grep -ac "debug calculadora: aberta" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" calculadora
for _ in $(seq 30); do "$N" msg -j windows | grep -q os.kalango.Calculadora && break; sleep 0.3; done
sleep 1.5
janela_calc() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Calculadora"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
lugar_calc() { grep -a "debug calculadora: $1 em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}'; }
clique_calc() { sem_avisos; read -r JX JY < <(janela_calc); read -r CX CY < <(lugar_calc "$1"); apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1; sleep 0.8; }
foto_x calculadora
ok "Calculadora: a do KalangoOS abre (os.kalango.Calculadora, desenho \"Calculadora\")" '[ "$(grep -ac "debug calculadora: aberta" "$T/shell.log")" -gt "$C0" ] && "$N" msg -j windows | grep -q os.kalango.Calculadora'
read -r JX JY < <(janela_calc); apontar $((JX + 230)) $((JY + 140)); sleep 0.3
xdotool type --window "$W" --delay 70 "1240*15%"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug calculadora: 1.240 × 15% = 186" "$T/shell.log" && break; sleep 0.3; done
ok "Calculadora: pelo teclado, 1.240 × 15% = 186" 'grep -aq "debug calculadora: 1.240 × 15% = 186" "$T/shell.log"'
clique_calc "campo"
xdotool type --window "$W" --delay 50 "15% de 1.240 em dolares"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug calculadora: «15% de 1.240 em dolares» = 186" "$T/shell.log" && break; sleep 0.3; done
ok "Calculadora: em palavras, \"15% de 1.240 em dólares\" dá 186 (em dólares)" 'grep -aq "debug calculadora: «15% de 1.240 em dolares» = 186" "$T/shell.log"'
clique_calc "aba Científica"
for _ in $(seq 20); do grep -aq "debug calculadora: página cientifica" "$T/shell.log" && break; sleep 0.3; done
sleep 1
foto_x calculadora-cientifica
clique_calc "tecla C"
clique_calc "tecla sin"
xdotool type --window "$W" --delay 70 "30"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug calculadora: sin(30 = 0,5" "$T/shell.log" && break; sleep 0.3; done
ok "Calculadora: na Científica, sin(30) = 0,5 (em graus)" 'grep -aq "debug calculadora: sin(30 = 0,5" "$T/shell.log"'
clique_calc "campo"
xdotool type --window "$W" --delay 50 "100 km em milhas"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug calculadora: 100 km = 62,1371 mi" "$T/shell.log" && break; sleep 0.3; done
sleep 1
foto_x calculadora-conversoes
ok "Calculadora: \"100 km em milhas\" abre as Conversões com 62,1371 milhas" 'grep -aq "debug calculadora: página conversoes" "$T/shell.log" && grep -aq "debug calculadora: 100 km = 62,1371 mi" "$T/shell.log"'
clique_calc "trocar"
for _ in $(seq 20); do grep -aq "debug calculadora: 62,1371 mi = 100 km" "$T/shell.log" && break; sleep 0.3; done
ok "Calculadora: ⇅ troca os lados (62,1371 milhas = 100 km)" 'grep -aq "debug calculadora: 62,1371 mi = 100 km" "$T/shell.log"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Calculadora"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 75. KalangoOS's own Text Editor (design "Editor de texto"; not GNOME's): a markdown file opens in a tab with its
#     preview; what is typed marks the tab and Ctrl+S writes the file; the local model suggests what the agent could
#     do with the text; "Pedir ao agente" (Ctrl+K) sends the request with the text to the agent's bar
fechar_apps
printf '# KalangoOS\n\n- [x] tema e ícones\n- [ ] tela de bloqueio nova\n\n## Ideias\nIlha do agente na barra superior.\n' > "$A/notas-editor.md"
E0=$(grep -ac "debug editor: abriu" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" editor "$A/notas-editor.md"
for _ in $(seq 30); do "$N" msg -j windows | grep -q os.kalango.Editor && break; sleep 0.3; done
sleep 1.5
foto_x editor
ok "Editor: o do KalangoOS abre o arquivo numa aba (os.kalango.Editor, desenho \"Editor de texto\")" \
  '"$N" msg -j windows | grep -q os.kalango.Editor && grep -a "debug editor: abriu" "$T/shell.log" | tail -1 | grep -q "notas-editor.md (7 linhas)"'
janela_ed() { "$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Editor"][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))'; }
read -r EX EY < <(janela_ed)
apontar $((EX + 200)) $((EY + 300)); sleep 0.2; xdotool click 1; sleep 0.3
xdotool key --window "$W" ctrl+End
xdotool type --window "$W" --delay 40 " Mais uma linha."
xdotool key --window "$W" ctrl+s
for _ in $(seq 20); do grep -q "Mais uma linha." "$A/notas-editor.md" && break; sleep 0.3; done
ok "Editor: o que é digitado vai para o arquivo com Ctrl+S" 'grep -q "Mais uma linha." "$A/notas-editor.md" && grep -aq "debug editor: salvou .*notas-editor.md" "$T/shell.log"'
for _ in $(seq 240); do grep -a "debug editor: [0-9]* sugestões" "$T/shell.log" | tail -1 | grep -q "debug editor: [1-9]" && break; sleep 0.5; done
foto_x editor-sugestoes
ok "Editor: o modelo local sugere o que fazer com o texto" 'grep -a "debug editor: [0-9]* sugestões" "$T/shell.log" | tail -1 | grep -q "debug editor: [1-9]"'
xdotool key --window "$W" ctrl+k
sleep 0.3
xdotool type --window "$W" --delay 40 "resuma este texto"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug editor: ao agente «resuma este texto»" "$T/shell.log" && break; sleep 0.3; done
ok "Editor: \"Pedir ao agente\" manda o pedido com o texto para a barra do agente" 'grep -aq "debug editor: ao agente «resuma este texto»" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" agente parar
sleep 1
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Editor"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 76. KalangoOS's own Terminal (designs "Terminal" and "Terminal · fechar com processo rodando"; not GNOME's): a tab in
#     the folder asked for, with the person's shell wrapped (two hooks say each command's start, status and folder);
#     a failed command gets the local model's hint under the prompt and "Explicar" sends it to the agent's bar; "Pedir
#     um comando" (Ctrl+Shift+K) puts the model's command at the prompt and the person's Enter runs it; the status
#     line; closing a tab while a command runs asks first, and "Fechar a aba" ends it
fechar_apps
TD="$OUT/terminal"; mkdir -p "$TD"; printf 'um\ndois\ntrês\n' > "$TD/lista.txt"
"$N" msg action spawn -- "$T/kalango-shell" terminal "$TD"
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Terminal"' && grep -aq "debug terminal: aba [0-9]* em $TD" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 30); do grep -aq "debug terminal: «" "$T/shell.log" || grep -a "debug terminal: status " "$T/shell.log" | grep -q "×" && break; sleep 0.3; done
sleep 2
foto_x terminal
ok "Terminal: o do KalangoOS abre uma aba na pasta pedida, com o shell da pessoa (os.kalango.Terminal, desenho \"Terminal\")" \
  '"$N" msg -j windows | grep -q "\"os.kalango.Terminal\"" && grep -aq "debug terminal: aba [0-9]* em $TD" "$T/shell.log"'
read -r JX JY < <(janela_app os.kalango.Terminal)
sem_avisos
apontar $((JX + 300)) $((JY + 300)); sleep 0.2; xdotool click 1; sleep 0.4
xdotool type --window "$W" --delay 40 "ls nao-existe"
xdotool key --window "$W" Return
for _ in $(seq 40); do grep -aq "debug terminal: «ls nao-existe» terminou com" "$T/shell.log" && break; sleep 0.3; done
grep -a "debug terminal: «ls nao-existe» terminou com" "$T/shell.log" | tail -1 | sed 's/^debug terminal: /  (/; s/$/)/'
ok "Terminal: os ganchos no shell dizem quando o comando começa e como termina (status e pasta)" \
  'grep -aq "debug terminal: «ls nao-existe» começou" "$T/shell.log" && grep -aq "debug terminal: «ls nao-existe» terminou com [1-9][0-9]* em $TD" "$T/shell.log"'
for _ in $(seq 240); do grep -aq "debug terminal: dica «" "$T/shell.log" && break; sleep 0.5; done
grep -a "debug terminal: dica «" "$T/shell.log" | tail -1 | sed 's/^debug terminal: /  (/; s/$/)/'
sleep 1.5
foto_x terminal-dica
ok "Terminal: o comando que falha ganha a dica do modelo local embaixo do prompt" 'grep -aq "debug terminal: dica «" "$T/shell.log"'
read -r CX CY < <(lugar "terminal explicar")
apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1
for _ in $(seq 20); do grep -aq "debug terminal: ao agente (explicar)" "$T/shell.log" && break; sleep 0.3; done
ok "Terminal: \"Explicar\" manda o erro para a barra do agente" 'grep -aq "debug terminal: ao agente (explicar)" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" agente parar
sleep 1
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
sleep 1
"$N" msg action focus-window --id "$(id_da_janela os.kalango.Terminal)"
sleep 0.6
xdotool key --window "$W" ctrl+shift+k
sleep 0.4
xdotool type --window "$W" --delay 40 "contar as linhas do arquivo lista.txt"
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug terminal: pedido «contar as linhas" "$T/shell.log" && break; sleep 0.5; done
grep -a "debug terminal: pedido «" "$T/shell.log" | tail -1 | sed 's/^debug terminal: /  (/; s/$/)/'
sleep 1
foto_x terminal-pedido
xdotool key --window "$W" Return
for _ in $(seq 40); do grep -aq "debug terminal: «[^»]*lista.txt[^»]*» terminou com 0" "$T/shell.log" && break; sleep 0.3; done
ok "Terminal: \"Pedir um comando\" põe o comando do modelo local no prompt, e o Enter da pessoa o roda" \
  'grep -aq "debug terminal: pedido «contar as linhas do arquivo lista.txt» → «[^»]*lista.txt" "$T/shell.log" && grep -aq "debug terminal: «[^»]*lista.txt[^»]*» terminou com 0" "$T/shell.log"'
grep -a "debug terminal: status " "$T/shell.log" | tail -1 | sed 's/^debug terminal: /  (/; s/$/)/'
ok "Terminal: a linha de status diz o shell, a codificação e o tamanho em células" 'grep -a "debug terminal: status " "$T/shell.log" | tail -1 | grep -qE "status [a-z]+ UTF-8 [0-9]+×[0-9]+"'
# a command at work: closing its tab asks first, and "Fechar a aba" ends it
xdotool type --window "$W" --delay 40 "sleep 300"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug terminal: «sleep 300» começou" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
xdotool key --window "$W" ctrl+shift+w
for _ in $(seq 20); do grep -aq "debug terminal: pergunta fechar «" "$T/shell.log" && break; sleep 0.3; done
sleep 1
foto_x terminal-fechar
read -r CX CY < <(lugar "terminal fechar aba")
apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1
for _ in $(seq 20); do grep -aq "debug terminal: fechado" "$T/shell.log" && break; sleep 0.3; done
sleep 1
ok "Terminal: fechar a aba com um comando rodando pergunta antes, e \"Fechar a aba\" o encerra" \
  'grep -a "debug terminal: pergunta fechar «" "$T/shell.log" | tail -1 | grep -q "sleep 300 · rodando há" && grep -aq "debug terminal: fechado" "$T/shell.log" && ! ps -eo args= | grep -qx "sleep 300"'
fechar_apps
# 77. KalangoOS's own System Monitor (designs "Monitor do sistema" and its pages "Processos", "Agentes", "Discos" and
#     "Rede"; not GNOME's): the processor, the memory and the processes read once a second; every page opens and
#     shows what this computer has (nothing breaks on the way)
fechar_apps
P0=$(grep -ac "panicked" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" monitor
for _ in $(seq 40); do grep -aq "debug monitor: cpu" "$T/shell.log" && break; sleep 0.5; done
sleep 4
foto_x monitor
grep -a "debug monitor: cpu" "$T/shell.log" | tail -1 | sed 's/^debug monitor: /  (/; s/$/)/'
ok "Monitor: o do KalangoOS abre e lê o processador, a memória e os processos agrupados (os.kalango.Monitor, desenho \"Monitor do sistema\")" \
  '"$N" msg -j windows | grep -q "\"os.kalango.Monitor\"" && grep -a "debug monitor: cpu" "$T/shell.log" | tail -1 | grep -qE "cpu [0-9]+% memória [0-9].* [1-9][0-9]* grupos"'
read -r JX JY < <(janela_app os.kalango.Monitor)
for pg in processos agentes discos rede visao; do
  sem_avisos
  read -r CX CY < <(lugar "monitor página $pg")
  apontar $((JX + ${CX:-0})) $((JY + ${CY:-0})); sleep 0.3; xdotool click 1
  for _ in $(seq 20); do grep -a "debug monitor: página" "$T/shell.log" | tail -1 | grep -q "página $pg" && break; sleep 0.3; done
  sleep 3
  foto_x "monitor-$pg"
done
ok "Monitor: Processos, Agentes, Discos e Rede abrem pela barra lateral e se atualizam sem erro" \
  'for pg in processos agentes discos rede; do grep -aq "debug monitor: página $pg" "$T/shell.log" || exit 1; done; [ "$(grep -ac "panicked" "$T/shell.log")" = "$P0" ]'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Monitor"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 1
# 78. The composer's "Ver a tela" (design "Chat · conversa"): the agent looks at the screen — the shell takes the picture
#     (no notice, the clipboard kept) and the local vision model (LFM3-VL) reads it
"$N" msg action spawn -- "$T/kalango-shell" lateral agente abrir
sleep 2
sem_avisos
apontar $((OW - 200)) $((OH - 150)); sleep 0.3; xdotool click 1; sleep 0.5
xdotool type --window "$W" --delay 40 "O que esta aparecendo na minha tela agora?"
xdotool key --window "$W" Return
for _ in $(seq 360); do grep -hsa '"nome":"tela.ver"' "$X"/data/kalango/conversas/*/eventos.jsonl 2>/dev/null | grep -q '"tipo":"resultado"' && break; sleep 0.5; done
TV=$(grep -lsa '"nome":"tela.ver"' "$X"/data/kalango/conversas/*/eventos.jsonl 2>/dev/null | head -1)
echo "  (tela.ver: $(grep -a '"tipo":"resultado"' "$TV" 2>/dev/null | grep -a tela.ver | tail -1 | cut -c1-160))"
ok "Ver a tela: o agente olha a tela (o shell tira a foto, o modelo de visão local a lê) e responde sobre ela" \
  'grep -aq "debug agente: tela capturada para o agente" "$T/shell.log" && [ -n "$TV" ] && grep -a "\"tipo\":\"resultado\"" "$TV" | grep -a tela.ver | grep -q "\"ok\":true"'
"$N" msg action spawn -- "$T/kalango-shell" agente parar
sleep 1
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
sleep 1
# 79. The agents' web: a search (kalangod's web.buscar, here against the stand-in's results page in Bing's markup:
#     the real address comes out of Bing's counter); a search engine's address given to web.ler becomes a search; a
#     page that shows its text only with scripts opens in the browser (Moli, held by kalangod: its proxy refused the
#     script's try at another name for this machine); and the real local model answers a question of the day with it
BUSCA=$(python3 "$CHAMAR" "$T/mcp.sock" web.buscar '{"consulta": "que horas é o jogo do Kalango FC hoje"}' 2>&1)
ok "Web: a pesquisa traz título, o endereço real (fora do contador do buscador) e o resumo de cada resultado" \
  'echo "$BUSCA" | grep -q "https://exemplo.com.br/jogo-do-kalango" && echo "$BUSCA" | grep -q "às 21h30, no Estádio da Serra" && grep -q "^/search?q=que" "$OUT/clima.log"'
GOOGLE=$(python3 "$CHAMAR" "$T/mcp.sock" web.ler '{"url": "https://www.google.com/search?q=agenda+do+kalango+fc"}' 2>&1)
ok "Web: o endereço de um buscador vira pesquisa (a página dele só mostra resultados com scripts)" \
  'echo "$GOOGLE" | grep -q "exemplo.com.br/agenda" && grep -q "^/search?q=agenda" "$OUT/clima.log"'
if [ -x "$ROOT/target/release/moli" ]; then
  JS=$(python3 "$CHAMAR" "$T/mcp.sock" web.ler '{"url": "http://127.0.0.1:18766/js"}' 2>&1)
  echo "  (página com scripts: $(echo "$JS" | tr '\n' ' ' | grep -o 'Conteúdo montado[^"]*' | head -1))"
  ok "Web: a página que só tem texto com scripts abre no navegador preso, e o proxy dele recusa outro nome desta máquina" \
    'echo "$JS" | grep -q "Conteúdo montado por script: o Kalango FC joga às 21h30" && echo "$JS" | grep -q "navegador.*true" && grep -q "^/js" "$OUT/clima.log" && ! grep -q "^/vazou" "$OUT/clima.log"'
else
  ok "Web: o navegador (Moli) está em target/release/moli (poc/setup-deps.sh o traz)" false
fi
# (an earlier section left the agents' network column at "never": free for this question, back after)
AG coluna rede livre
RESP=$(echo '"que horas é o jogo do Kalango FC hoje?"' | env LANGUAGE=pt_BR XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_DATA_HOME="$X/data" \
    KALANGOD_SOCKET="$T/mcp.sock" KALANGO_MODELO="$MODELO" KALANGO_MODELOS="$MODELOS" KALANGO_MEMORIA_CHAVEIRO=nao timeout 240 "$ROOT/target/release/kalango-agente" --json 2>/dev/null)
echo "  (resposta: $(echo "$RESP" | grep -a '"tipo":"resposta"' | cut -c1-200))"
ok "Web: o agente pesquisa a pergunta do dia e responde com o que achou (o modelo local de verdade)" \
  'echo "$RESP" | grep -a "\"tipo\":\"ferramenta\"" | grep -q "web.buscar" && echo "$RESP" | grep -a "\"tipo\":\"resposta\"" | grep -Eq "21h30|21:30"'
# 79b. "O resultado do jogo de ontem": the summaries do not have the score, the team page does — the agent opens it (or
#      the engine opens the best result after two searches) and answers with the score
RESP=$(echo '"resultado do jogo de ontem do Kalango FC"' | env LANGUAGE=pt_BR XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_DATA_HOME="$X/data" \
    KALANGOD_SOCKET="$T/mcp.sock" KALANGO_MODELO="$MODELO" KALANGO_MODELOS="$MODELOS" KALANGO_MEMORIA_CHAVEIRO=nao timeout 240 "$ROOT/target/release/kalango-agente" --json 2>/dev/null)
echo "  (buscas: $(echo "$RESP" | grep -a '"tipo":"ferramenta"' | grep -c web.buscar); resposta: $(echo "$RESP" | grep -a '"tipo":"resposta"' | cut -c1-200))"
ok "Web: o resultado de ontem — o agente abre a página do time (o resumo não tem o placar) e responde com ele" \
  'grep -q "^/time-kalango" "$OUT/clima.log" && echo "$RESP" | grep -a "\"tipo\":\"resposta\"" | sed "s/\xe2\x80\xaf/ /g; s/\xc2\xa0/ /g" | grep -Eq "3 ?(a|x|×|-|–|—) ?1"'
# 79c. The agent's browser (design "Chat · navegador do agente"): the same question in a conversation — its side bar
#      shows the Browser tab with the page the agent read (Moli, held by kalangod), the part read outlined; "Assumir o
#      controle" passes the page to the person: a click follows the page's link, "Voltar" returns, control goes back
NV0=$(wc -l < "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" conversar "resultado do jogo de ontem do Kalango FC"
for _ in $(seq 300); do tail -n +"$NV0" "$T/shell.log" | grep -aq "debug navegador: destaque" && break; sleep 1; done
echo "  ($(tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: abrir →" | tail -1 | cut -c1-140); $(tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: destaque" | tail -1 | cut -c1-120))"
ok "Navegador do agente: a conversa abre a aba Navegador com a página que o agente leu" \
  'tail -n +"$NV0" "$T/shell.log" | grep -a "debug conversas: abas" | tail -1 | grep -q "Navegador" && tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: abrir →" | grep -q "/time-kalango"'
ok "Navegador do agente: o trecho que o agente usou fica contornado" \
  'tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: destaque" | tail -1 | grep -q "Some("'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-navegador.png" >/dev/null
read -r JX JY < <(janela_app os.kalango.Agentes)
# the answer's sources: a chip opens its page in the browser
AB0=$(tail -n +"$NV0" "$T/shell.log" | grep -ac "debug navegador: abrir →")
read -r FX FY < <(lugar "fonte 1 final")
[ -n "$FX" ] && apontar $((JX + FX)) $((JY + FY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 40); do [ "$(tail -n +"$NV0" "$T/shell.log" | grep -ac "debug navegador: abrir →")" -gt "$AB0" ] && break; sleep 0.5; done
ok "Navegador do agente: as Fontes da resposta abrem a página no navegador" \
  'tail -n +"$NV0" "$T/shell.log" | grep -a "debug conversa: fontes" | tail -1 | grep -q "/time-kalango" && [ "$(tail -n +"$NV0" "$T/shell.log" | grep -ac "debug navegador: abrir →")" -gt "$AB0" ]'
read -r AX AY < <(lugar "navegador assumir")
[ -n "$AX" ] && apontar $((JX + AX)) $((JY + AY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do tail -n +"$NV0" "$T/shell.log" | grep -aq "debug navegador: no controle da pessoa" && break; sleep 0.3; done
read -r PX PY < <(lugar "navegador pagina")
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 40); do tail -n +"$NV0" "$T/shell.log" | grep -aq "debug navegador: clicar →" && break; sleep 0.5; done
ok "Navegador do agente: no controle, um clique na página segue o link dela (sem recarregar a sessão)" \
  'tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: clicar →" | tail -1 | grep -q "/tabela-kalango" && grep -q "^/tabela-kalango" "$OUT/clima.log"'
# (the button turns on when the page it left is known: waited for, then its place read again)
for _ in $(seq 30); do tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: voltar ativo" | tail -1 | grep -q true && break; sleep 0.3; done
sleep 0.6
read -r VX VY < <(lugar "navegador voltar")
echo "  (voltar em $VX $VY; $(tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: voltar ativo" | tail -1))"
[ -n "$VX" ] && apontar $((JX + VX)) $((JY + VY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 40); do tail -n +"$NV0" "$T/shell.log" | grep -aq "debug navegador: voltar →" && break; sleep 0.5; done
read -r AX AY < <(lugar "navegador assumir")
[ -n "$AX" ] && apontar $((JX + AX)) $((JY + AY)) && sleep 0.3 && xdotool click 1
sleep 0.8
ok "Navegador do agente: Voltar retorna à página do agente e o controle volta para ele" \
  'tail -n +"$NV0" "$T/shell.log" | grep -a "debug navegador: voltar →" | tail -1 | grep -q "/time-kalango" && tail -n +"$NV0" "$T/shell.log" | grep -aq "debug navegador: controle devolvido"'
"$N" msg action close-window
sleep 0.8
# 79d. A page behind a login (403, to the browser too): the conversation shows the card; "Entrar com a minha conta"
#      opens it in the agent's browser with the person in control (their login would stay in that isolated session)
LG0=$(wc -l < "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" conversar "Leia a página http://127.0.0.1:18766/privado-kalango e me diga o que ela mostra."
for _ in $(seq 240); do tail -n +"$LG0" "$T/shell.log" | grep -aq "debug conversa: .* pede login" && break; sleep 1; done
echo "  ($(tail -n +"$LG0" "$T/shell.log" | grep -a "debug conversa: .* pede login" | tail -1))"
ok "Navegador do agente: uma página que pede login vira o cartão na conversa" 'tail -n +"$LG0" "$T/shell.log" | grep -aq "debug conversa: 127.0.0.1:18766 pede login"'
sleep 1.5
read -r JX JY < <(janela_app os.kalango.Agentes)
read -r EX EY < <(lugar "login entrar final")
[ -n "$EX" ] && apontar $((JX + EX)) $((JY + EY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 40); do tail -n +"$LG0" "$T/shell.log" | grep -a "debug navegador: abrir →" | grep -q "/privado-kalango" && break; sleep 0.5; done
ok "Navegador do agente: “Entrar com a minha conta” abre a página no navegador, com a pessoa no controle" \
  'tail -n +"$LG0" "$T/shell.log" | grep -aq "debug navegador: no controle da pessoa" && tail -n +"$LG0" "$T/shell.log" | grep -a "debug navegador: abrir →" | grep -q "/privado-kalango"'
"$N" msg action close-window
sleep 0.8
AG coluna rede nunca
# 80. The agents center in a column of half the screen (640 px here): the window is the column's size (its parts give
#     way — the context, then the side bar into the rail, the chips into icons — instead of holding it wider)
"$N" msg action spawn -- "$T/kalango-shell" central conversas
for _ in $(seq 40); do "$N" msg -j focused-window | grep -q '"os.kalango.Agentes"' && break; sleep 0.3; done
sleep 1
"$N" msg action toggle-window-floating
sleep 0.5
"$N" msg action set-column-width 50%
sleep 2.5
read -r AGW AGT < <("$N" msg -j windows | python3 -c 'import json,sys; [print(int(w["layout"]["window_size"][0]), int(w["layout"]["tile_size"][0])) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Agentes"]')
echo "  (central em meia coluna: janela $AGW px, coluna $AGT px; $(grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | cut -c1-90))"
ok "Agentes: em meia tela a central tem o tamanho da coluna (o contexto e a lista cedem, nada passa da borda)" \
  '[ -n "$AGW" ] && [ "$AGW" = "$AGT" ] && [ "$AGW" -le $((OW / 2 + 2)) ] && grep -a "debug central: conversas em" "$T/shell.log" | tail -1 | grep -q "lista false"'
"$N" msg action close-window
sleep 0.8
# 81. Minimize (KalangoOS's niri: GTK's minimize asks the compositor, which keeps the window open out of sight and
#     says so — is_minimized): the calculator's own "–" button; focusing it (the dock, Alt+Tab) brings it back;
#     Mod+M minimizes the focused window; a window in a column goes back to its column when restored
minim() { "$N" msg -j windows | python3 -c 'import json,sys; w=[w for w in json.load(sys.stdin) if w["app_id"]==sys.argv[1]]; print(w[0].get("is_minimized", "?") if w else "fechada", w[0]["is_floating"] if w else "")' "$1"; }
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 2.5
sem_avisos
read -r JX JY < <(janela_app os.kalango.Calculadora)
read -r MX MY < <(lugar "janela minimizar")
xdotool mousemove --window "$W" $((JX + MX)) $((JY + MY)) click 1
sleep 1.2
read -r M1 _ < <(minim os.kalango.Calculadora)
echo "  (calculadora depois do –: minimizada=$M1)"
ok "Minimizar: o botão – da janela a esconde, e o niri a guarda aberta (minimizada)" '[ "$M1" = True ] && ! "$N" msg -j focused-window | grep -q "os.kalango.Calculadora"'
"$N" msg action focus-window --id "$(id_app kalango.calculadora)"
sleep 1.2
read -r M2 _ < <(minim os.kalango.Calculadora)
ok "Minimizar: focar a janela (a dock, Alt+Tab) a traz de volta" '[ "$M2" = False ] && "$N" msg -j focused-window | grep -q "os.kalango.Calculadora"'
# in a column: Mod+M (Alt+M in the nested session), then back to the column
"$N" msg action toggle-window-floating
sleep 0.8
xdotool key --window "$W" alt+m
sleep 1.2
read -r M3 F3 < <(minim os.kalango.Calculadora)
"$N" msg action focus-window --id "$(id_app kalango.calculadora)"
sleep 1.2
read -r M4 F4 < <(minim os.kalango.Calculadora)
echo "  (em coluna: Mod+M → minimizada=$M3; de volta → minimizada=$M4 flutuante=$F4)"
ok "Minimizar: Mod+M minimiza a janela em foco, e a que estava numa coluna volta para a coluna" '[ "$M3" = True ] && [ "$M4" = False ] && [ "$F4" = False ]'
"$N" msg action close-window
sleep 0.8

# 82. Agents › Settings › General, Notifications and Advanced (designs "Ajustes · Geral", "· Notificações" and
#     "· Avançado"): the tabs open; "Curtas" is kept for the engine; "Só na lista" puts the notice of a task the agent
#     finished out of sight in the list without a banner; keeping conversations 7 days removes an older one
AJ="$X/config/kalango/agente-ajustes.json"
mkdir -p "$X/data/kalango/conversas/velha-de-teste"
echo '{"titulo": "conversa velha"}' > "$X/data/kalango/conversas/velha-de-teste/meta.json"
touch -d "20 days ago" "$X/data/kalango/conversas/velha-de-teste/meta.json" "$X/data/kalango/conversas/velha-de-teste"
"$N" msg action spawn -- "$T/kalango-shell" central ajustes
for _ in $(seq 40); do "$N" msg -j focused-window | grep -q '"os.kalango.Agentes"' && break; sleep 0.3; done
sleep 1.5
clique_aj() { read -r JX JY < <(janela_app os.kalango.Agentes); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && xdotool mousemove --window "$W" $((JX + CX)) $((JY + CY)) click 1; sleep 0.8; }
clique_aj "ajustes aba geral"
clique_aj "ajustes curtas"
for _ in $(seq 20); do grep -qs '"respostas": "curtas"' "$AJ" && break; sleep 0.3; done
ok "Ajustes · Geral: a aba abre e \"Curtas\" fica guardado para o motor" 'grep -aq "debug ajustes: aba geral" "$T/shell.log" && grep -qs "\"respostas\": \"curtas\"" "$AJ"'
clique_aj "ajustes aba notificacoes"
clique_aj "ajustes lista"
for _ in $(seq 20); do grep -qs '"aviso_tarefa": "lista"' "$AJ" && break; sleep 0.3; done
clique_aj "ajustes aba avancado"
clique_aj "ajustes 7"
for _ in $(seq 20); do [ ! -d "$X/data/kalango/conversas/velha-de-teste" ] && break; sleep 0.3; done
ok "Ajustes · Avançado: guardar as conversas por 7 dias apaga a mais antiga" '[ ! -d "$X/data/kalango/conversas/velha-de-teste" ] && grep -qs "\"guardar_dias\": 7" "$AJ"'
"$N" msg action close-window
sleep 0.8
# a request in the side bar, closed once the agent is at work (reading free here — the column asks at this point,
# and a question would hold the request): its notice goes to the list only
AG coluna ler livre
B0=$(grep -ac "debug compositor botão 7: botao-enviar.parar" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" pedir Quanto espaço livre tem no disco e quais processos usam mais memória?
for _ in $(seq 40); do [ "$(grep -ac "debug compositor botão 7: botao-enviar.parar" "$T/shell.log")" -gt "$B0" ] && break; sleep 0.25; done
sleep 0.6
"$N" msg action spawn -- "$T/kalango-shell" lateral fechar
for _ in $(seq 120); do grep -aq "debug aviso: tarefa do agente (lista)" "$T/shell.log" && break; sleep 0.5; done
ok "Ajustes · Notificações: com \"Só na lista\", a tarefa terminada fora da vista vai para a lista, sem banner" \
  'grep -aq "debug aviso: tarefa do agente (lista): O agente terminou" "$T/shell.log"'
AG coluna ler perguntar
# back to the defaults for what follows
rm -f "$AJ"

# 83. KalangoOS's own image viewer (design "Visualizador de imagens"; not GNOME's): a picture opens with its folder's
#     pictures in the strip and its details (made by AI, from its metadata); → goes to the next; "Remover o fundo"
#     clears the background from the edges (before and after under a curtain) and "Salvar cópia" keeps a PNG with it
#     transparent; turning saves the picture turned; the trash takes one; the local vision model describes it; and
#     the session's apps open pictures with it
IMG="$X/area/Imagens/teste"
mkdir -p "$IMG"
python3 - "$IMG" <<'PY'
import sys
from PIL import Image, ImageDraw
from PIL.PngImagePlugin import PngInfo
d = sys.argv[1]
nave = Image.new("RGB", (400, 300), (16, 17, 20))
g = ImageDraw.Draw(nave)
g.polygon([(60, 150), (200, 60), (340, 150), (200, 240)], fill=(200, 30, 40))
g.rectangle([170, 130, 230, 170], fill=(230, 230, 235))
meta = PngInfo()
meta.add_text("parameters", "a red spaceship, Steps: 30")
nave.save(f"{d}/nave.png", pnginfo=meta)
Image.new("RGB", (320, 200), (40, 90, 160)).save(f"{d}/ceu.png")
Image.new("RGB", (240, 240), (220, 200, 90)).save(f"{d}/sol.jpg")
PY
"$N" msg action spawn -- "$T/kalango-shell" imagens "$IMG/nave.png"
for _ in $(seq 40); do grep -aq "debug imagens: aberta «nave.png»" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-imagens.png" >/dev/null
ok "Imagens: o do KalangoOS abre a imagem com as da pasta (os.kalango.Imagens, desenho \"Visualizador de imagens\")" \
  'grep -a "debug imagens: aberta «nave.png»" "$T/shell.log" | tail -1 | grep -q "(400×300, 2 de 3 na pasta)" && "$N" msg -j windows | grep -q "\"os.kalango.Imagens\""'
"$N" msg action focus-window --id "$(id_app kalango.imagens)"
sleep 0.4
xdotool key --window "$W" Right
for _ in $(seq 20); do grep -aq "debug imagens: aberta «sol.jpg»" "$T/shell.log" && break; sleep 0.3; done
xdotool key --window "$W" Left
for _ in $(seq 20); do [ "$(grep -ac "debug imagens: aberta «nave.png»" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
ok "Imagens: → e ← passam pelas imagens da pasta" 'grep -aq "debug imagens: aberta «sol.jpg»" "$T/shell.log" && [ "$(grep -ac "debug imagens: aberta «nave.png»" "$T/shell.log")" -ge 2 ]'
sleep 1
clique_img() { read -r JX JY < <(janela_app os.kalango.Imagens); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.8; }
clique_img "imagens acao fundo"
for _ in $(seq 40); do grep -aq "debug imagens: sem fundo pronto" "$T/shell.log" && break; sleep 0.3; done
clique_img "imagens salvar"
for _ in $(seq 20); do [ -f "$IMG/nave (sem fundo).png" ] && break; sleep 0.3; done
FUNDO=$(python3 -c 'import sys; from PIL import Image; i=Image.open(sys.argv[1]).convert("RGBA"); print(i.getpixel((5,5))[3], i.getpixel((200,150))[3])' "$IMG/nave (sem fundo).png" 2>/dev/null)
echo "  (sem fundo: alfa no canto e no meio = $FUNDO; $(grep -a "debug imagens: sem fundo pronto" "$T/shell.log" | tail -1))"
ok "Imagens: \"Remover o fundo\" tira o fundo pelas bordas e \"Salvar cópia\" guarda um PNG transparente" '[ "$FUNDO" = "0 255" ]'
# the copy is in view now: turned to the right, saved turned
clique_img "imagens ferramenta girar"
for _ in $(seq 20); do grep -aq "debug imagens: girada e salva «nave (sem fundo).png»" "$T/shell.log" && break; sleep 0.3; done
GIRO=$(python3 -c 'import sys; from PIL import Image; print("%dx%d" % Image.open(sys.argv[1]).size)' "$IMG/nave (sem fundo).png" 2>/dev/null)
ok "Imagens: girar salva a imagem girada" '[ "$GIRO" = "300x400" ]'
clique_img "imagens acao descrever"
for _ in $(seq 240); do grep -aq "debug imagens: descrição (" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug imagens: descrição (" "$T/shell.log" | tail -1))"
ok "Imagens: \"Descrever a imagem\" pelo modelo de visão local" 'grep -a "debug imagens: descrição (" "$T/shell.log" | tail -1 | grep -Eq "\(([2-9][0-9]|[0-9]{3,}) caracteres\)"'
"$N" msg action focus-window --id "$(id_app kalango.imagens)"
sleep 0.5
xdotool key --window "$W" Delete
for _ in $(seq 20); do [ ! -f "$IMG/nave (sem fundo).png" ] && break; sleep 0.3; done
ok "Imagens: Delete manda a imagem para a lixeira" '[ ! -f "$IMG/nave (sem fundo).png" ] && grep -aq "debug imagens: para a lixeira «nave (sem fundo).png»" "$T/shell.log"'
# (the session's PATH: GLib leaves out an app whose program it cannot find)
PADRAO=$(env PATH="$T:$PATH" XDG_CURRENT_DESKTOP=KalangoOS XDG_CONFIG_DIRS="$ROOT/sessao/dist/etc/xdg:/etc/xdg" XDG_DATA_DIRS="$ROOT/sessao/dist/share:/usr/share" XDG_CONFIG_HOME="$X/config" XDG_DATA_HOME="$X/data" gio mime image/png 2>/dev/null | head -1)
echo "  (image/png: $PADRAO)"
ok "Imagens: na sessão do KalangoOS as imagens abrem nele" 'echo "$PADRAO" | grep -q "os.kalango.Imagens.desktop"'
"$N" msg action close-window --id "$(id_app kalango.imagens)"
sleep 0.8

# 84. KalangoOS's own Music app (design "Música"; not GNOME's): the Music folder's songs with their tags and covers
#     (read in Rust), a song played — muted in a test profile — with its real waveform, the next one, a favourite, a
#     click on the waveform goes there, MPRIS tells the island it is playing, and the local model makes a mix that plays
MUS="$X/area/Música"
mkdir -p "$MUS"
python3 -c 'import sys; from PIL import Image; Image.new("RGB", (200, 200), (120, 70, 190)).save(sys.argv[1])' "$OUT/capa.png"
faixa() { ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=$2:duration=$3" -i "$OUT/capa.png" -map 0:a -map 1:v -c:a flac -c:v png -disposition:v attached_pic \
  -af "afade=t=in:d=$(($3 / 2))" -metadata title="$4" -metadata artist="$5" -metadata album="$6" -metadata genre="$7" "$MUS/$1.flac"; }
# (long enough not to end by themselves while the test goes on)
faixa noite-calma 330 40 "Noite calma" "Kalango Trio" "Madrugada" "Ambient"
faixa rua-acesa 520 36 "Rua acesa" "Os Lagartos" "Cidade" "Rock"
faixa cafe 440 38 "Café da manhã" "Kalango Trio" "Manhãs" "Jazz"
"$N" msg action spawn -- "$T/kalango-shell" musica
for _ in $(seq 60); do grep -aq "debug musica: biblioteca (3 faixas" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-musica.png" >/dev/null
CAPAS=$(python3 -c 'import json,sys; v=json.load(open(sys.argv[1])); print(sum(1 for t in v if t.get("cover")), sorted(t["title"] for t in v))' "$X/cache/kalango/musica/biblioteca.json" 2>/dev/null)
echo "  (biblioteca: $CAPAS)"
ok "Música: o do KalangoOS lê a pasta Música com as tags e as capas (os.kalango.Musica, desenho \"Música\")" \
  'grep -aq "debug musica: biblioteca (3 faixas" "$T/shell.log" && echo "$CAPAS" | grep -q "^3 .*Café da manhã.*Noite calma.*Rua acesa" && "$N" msg -j windows | grep -q "\"os.kalango.Musica\""'
clique_mus() { read -r JX JY < <(janela_app os.kalango.Musica); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX + ${2:-0})) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 1; }
clique_mus "musica tocar"
for _ in $(seq 30); do grep -aq "debug musica: onda (96 barras)" "$T/shell.log" && break; sleep 0.3; done
ok "Música: tocar a faixa (muda num perfil de teste) com a onda de verdade dela" \
  'grep -a "debug musica: tocando «" "$T/shell.log" | tail -1 | grep -q "mudo (teste)" && grep -aq "debug musica: onda (96 barras)" "$T/shell.log"'
"$N" msg action spawn -- sh -c "gdbus call --session --dest org.mpris.MediaPlayer2.kalango_musica --object-path /org/mpris/MediaPlayer2 --method org.freedesktop.DBus.Properties.Get org.mpris.MediaPlayer2.Player PlaybackStatus > '$OUT/mpris-musica.txt' 2>&1"
for _ in $(seq 20); do [ -s "$OUT/mpris-musica.txt" ] && break; sleep 0.3; done
echo "  (MPRIS: $(cat "$OUT/mpris-musica.txt" 2>/dev/null))"
ok "Música: o MPRIS diz que está tocando (a ilha e o widget mostram a mesma faixa)" 'grep -q "Playing" "$OUT/mpris-musica.txt"'
T0=$(grep -ac "debug musica: tocando «" "$T/shell.log")
clique_mus "musica proxima"
for _ in $(seq 20); do [ "$(grep -ac "debug musica: tocando «" "$T/shell.log")" -gt "$T0" ] && break; sleep 0.3; done
clique_mus "musica favorita"
for _ in $(seq 20); do grep -qs "flac" "$X/data/kalango/musica/favoritas.json" && break; sleep 0.3; done
ok "Música: a próxima faixa toca, e a estrela a guarda nas favoritas" \
  'grep -a "debug musica: tocando «" "$T/shell.log" | tail -1 | grep -q "(2 de 3)" && grep -qs "flac" "$X/data/kalango/musica/favoritas.json"'
# a click near the end of the waveform goes there
read -r WX WY < <(lugar "musica onda")
clique_mus "musica onda" 100
for _ in $(seq 20); do grep -aEq "debug musica: posição [1-3][0-9]s" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug musica: posição" "$T/shell.log" | tail -1))"
ok "Música: um clique na onda leva a faixa até ali" 'grep -aEq "debug musica: posição [1-3][0-9]s" "$T/shell.log"'
for _ in $(seq 240); do grep -aq "debug musica: mix «" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug musica: mix «" "$T/shell.log" | tail -1))"
T0=$(grep -ac "debug musica: tocando «" "$T/shell.log")
sem_avisos 2>/dev/null || sleep 4
clique_mus "musica mix tocar"
for _ in $(seq 20); do [ "$(grep -ac "debug musica: tocando «" "$T/shell.log")" -gt "$T0" ] && break; sleep 0.3; done
ok "Música: o modelo local monta um mix da biblioteca, e \"Tocar\" o toca e guarda nas listas" \
  'grep -aq "debug musica: mix «.*» ([1-3] faixas)" "$T/shell.log" && [ "$(grep -ac "debug musica: tocando «" "$T/shell.log")" -gt "$T0" ] && grep -qs "flac" "$X/data/kalango/musica/listas.json"'
# typing in the search is typing (its spaces do not play or pause), and Enter plays the first song found
"$N" msg action focus-window --id "$(id_app kalango.musica)"
sleep 0.4
P0=$(grep -ac "debug musica: tocando\|debug musica: pausado" "$T/shell.log")
xdotool key --window "$W" ctrl+k
sleep 0.4
xdotool type --window "$W" --delay 40 "rua acesa"
sleep 0.6
P1=$(grep -ac "debug musica: tocando\|debug musica: pausado" "$T/shell.log")
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -a "debug musica: tocando «" "$T/shell.log" | tail -1 | grep -q "Rua acesa" && break; sleep 0.3; done
ok "Música: digitar na busca não toca nem pausa, e Enter toca a música achada" \
  '[ "$P1" = "$P0" ] && grep -aq "debug musica: página musicas" "$T/shell.log" && grep -a "debug musica: tocando «" "$T/shell.log" | tail -1 | grep -q "Rua acesa"'
"$N" msg action close-window --id "$(id_app kalango.musica)"
sleep 0.8

# 85. KalangoOS's own Video app (design "Reprodutor de vídeo"; not GNOME's): a video of three scenes (red, a test
#     pattern, blue) with the test's spoken sentence plays muted; its controls fade 2 s after the pointer stops; the
#     agent finds the chapters where the scenes change and names them with the vision model; a click on the timeline
#     goes there; the live captions hear the sentence with the audio model; the speed changes; the summary and a
#     request go to the local model; ← the folder's other video; and the session opens videos with it
VID="$X/area/Vídeos/teste"
mkdir -p "$VID"
AUD="$FALA"
[ -s "$AUD" ] || { ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=440:duration=3" "$OUT/tom.wav"; AUD="$OUT/tom.wav"; }
ffmpeg -loglevel error -y -f lavfi -i "color=c=0xC83228:s=640x360:d=8:r=25" -f lavfi -i "testsrc2=s=640x360:d=8:r=25" \
  -f lavfi -i "color=c=0x2850C8:s=640x360:d=8:r=25" -i "$AUD" \
  -filter_complex "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v];[3:a]aresample=48000,apad=whole_dur=24[a]" \
  -map "[v]" -map "[a]" -c:v libvpx -g 25 -b:v 500k -c:a libvorbis -t 24 "$VID/viagem.webm"
ffmpeg -loglevel error -y -f lavfi -i "color=c=0x2E8B57:s=320x180:d=4:r=25" -c:v libvpx -g 25 "$VID/a-praia.webm"
"$N" msg action spawn -- "$T/kalango-shell" video "$VID/viagem.webm"
for _ in $(seq 40); do grep -aq "debug video: aberto «viagem.webm»" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug video: aberto «viagem.webm»" "$T/shell.log" | tail -1))"
ok "Vídeo: o do KalangoOS toca o vídeo, mudo num perfil de teste (os.kalango.Video, desenho \"Reprodutor de vídeo\")" \
  'grep -a "debug video: aberto «viagem.webm»" "$T/shell.log" | tail -1 | grep -q "(640×360, 24 s) · mudo (teste)" && "$N" msg -j windows | grep -q "\"os.kalango.Video\""'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-video.png" >/dev/null
for _ in $(seq 20); do grep -aq "debug video: controles somem" "$T/shell.log" && break; sleep 0.3; done
ok "Vídeo: os controles somem 2 s depois que o ponteiro para" 'grep -aq "debug video: controles somem" "$T/shell.log"'
"$N" msg action focus-window --id "$(id_app kalango.video)"
sleep 0.3
xdotool key --window "$W" space
for _ in $(seq 20); do grep -aq "debug video: pausado em" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug video: pausado em" "$T/shell.log" | tail -1))"
ok "Vídeo: o vídeo anda, e o espaço pausa" 'grep -a "debug video: pausado em" "$T/shell.log" | tail -1 | grep -Eq "em ([2-9]|1[0-9])s"'
# the controls come back with the pointer, and a click on the timeline goes there (its middle: about 12 s)
clique_vid() { read -r JX JY < <(janela_app os.kalango.Video); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX + ${2:-0} - 20)) $((JY + CY - 30)) && sleep 0.2 && apontar $((JX + CX + ${2:-0})) $((JY + CY)) && sleep 0.4 && xdotool click 1; sleep 0.8; }
clique_vid "video linha do tempo"
for _ in $(seq 20); do grep -aEq "debug video: posição 1[0-4]s" "$T/shell.log" && break; sleep 0.3; done
ok "Vídeo: um clique na linha do tempo leva o vídeo até ali" 'grep -aEq "debug video: posição 1[0-4]s" "$T/shell.log"'
for _ in $(seq 400); do grep -aq "debug video: capítulos (" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug video: capítulos (" "$T/shell.log" | tail -1 | cut -c1-200))"
ok "Vídeo: o agente acha os capítulos onde a cena muda e o modelo de visão os nomeia" \
  'grep -a "debug video: capítulos (" "$T/shell.log" | tail -1 | grep -q "(3): 00:00 «.*»; 00:08 «.*»; 00:16 «"'
# live captions from the start: the audio model hears the test's sentence
xdotool key --window "$W" Home
clique_vid "video legendas"
xdotool key --window "$W" space
for _ in $(seq 240); do grep -aq "debug video: legenda 00:00–00:06 «" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug video: legenda 00:00" "$T/shell.log" | tail -1))"
ok "Vídeo: as legendas ao vivo ouvem a fala com o modelo de áudio local" \
  'grep -aq "debug video: legendas true" "$T/shell.log" && grep -a "debug video: legenda 00:00–00:06 «" "$T/shell.log" | tail -1 | grep -iEq "espa|disco|livre"'
clique_vid "video velocidade"
sleep 0.5
clique_vid "video velocidade 2"
for _ in $(seq 20); do grep -aq "debug video: velocidade 2" "$T/shell.log" && break; sleep 0.3; done
ok "Vídeo: a velocidade muda (2×)" 'grep -aq "debug video: velocidade 2" "$T/shell.log"'
clique_vid "video resumir"
for _ in $(seq 400); do grep -aq "debug video: resumo (" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug video: resumo (\|debug video: ouvido" "$T/shell.log" | tail -2 | tr '\n' ' '))"
ok "Vídeo: \"Resumir o vídeo\" ouve a fala e o modelo local resume" 'grep -a "debug video: resumo (" "$T/shell.log" | tail -1 | grep -Eq "\(([2-9][0-9]|[0-9]{3,}) caracteres\)"'
# a request: typing in it is typing (its spaces do not pause), and the agent jumps or answers
P0=$(grep -ac "debug video: pausado\|debug video: tocando" "$T/shell.log")
xdotool key --window "$W" ctrl+k
sleep 0.4
xdotool type --window "$W" --delay 40 "pule para a parte azul"
P1=$(grep -ac "debug video: pausado\|debug video: tocando" "$T/shell.log")
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug video: pedido →" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug video: pedido →" "$T/shell.log" | tail -1))"
ok "Vídeo: um pedido (\"pule para a parte azul\") vai ao modelo local, que pula ou responde" '[ "$P1" = "$P0" ] && grep -aq "debug video: pedido →" "$T/shell.log"'
clique_vid "video anterior"
for _ in $(seq 30); do grep -aq "debug video: aberto «a-praia.webm»" "$T/shell.log" && break; sleep 0.3; done
ok "Vídeo: o botão anterior abre o outro vídeo da pasta" 'grep -a "debug video: aberto «a-praia.webm»" "$T/shell.log" | tail -1 | grep -q "(320×180, 4 s)"'
PADRAO=$(env PATH="$T:$PATH" XDG_CURRENT_DESKTOP=KalangoOS XDG_CONFIG_DIRS="$ROOT/sessao/dist/etc/xdg:/etc/xdg" XDG_DATA_DIRS="$ROOT/sessao/dist/share:/usr/share" XDG_CONFIG_HOME="$X/config" XDG_DATA_HOME="$X/data" gio mime video/webm 2>/dev/null | head -1)
echo "  (video/webm: $PADRAO)"
ok "Vídeo: na sessão do KalangoOS os vídeos abrem nele" 'echo "$PADRAO" | grep -q "os.kalango.Video.desktop"'
"$N" msg action close-window --id "$(id_app kalango.video)"
sleep 0.8

# 107. Chat · mídia (design 77183): the video goes with a message — its card has the chapters the Video app found
#      (kept), "Ir para" opens the video at the chapter chosen and "Cortar este trecho" saves that piece in Vídeos; a
#      sound with speech is transcribed by the local audio model piece by piece, each line with its time, and a click
#      on a line plays from it (the line lights up)
L107=$(wc -l < "$T/shell.log")
# (a question the model answers without tools: one that asks it to read the file leaves the turn waiting for the
# person's permission, the list scrolls to that card and the buttons are no longer where they were)
"$N" msg action spawn -- "$T/kalango-shell" conversar "--anexo=$VID/viagem.webm" "--anexo=$AUD" Nao use ferramentas. Responda so com a palavra recebido
for _ in $(seq 300); do tail -n +"$L107" "$T/shell.log" | grep -aq "debug central: midia ir final em" && break; sleep 1; done
echo "  ($(tail -n +"$L107" "$T/shell.log" | grep -a "debug midia: capítulos no cartão" | tail -1 | cut -c1-160))"
ok "Chat · mídia: o vídeo da mensagem traz os capítulos que o app Vídeo achou" 'tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: capítulos no cartão (3): 00:00"'
sleep 0.8
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r PX PY < <(tail -n +"$L107" "$T/shell.log" | grep -a "debug central: midia ir \(final \)\?em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 40); do tail -n +"$L107" "$T/shell.log" | grep -aq "debug video: posição 8s" && break; sleep 0.3; done
ok "Chat · mídia: \"Ir para 00:08\" abre o vídeo no capítulo escolhido" 'tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: ir para 00:08" && tail -n +"$L107" "$T/shell.log" | grep -aq "debug video: posição 8s"'
"$N" msg action close-window --id "$(id_app kalango.video)" >/dev/null 2>&1
sleep 0.8
"$N" msg action focus-window --id "$(id_app os.kalango.agentes)" >/dev/null 2>&1
sleep 0.5
read -r JX JY < <(janela_central)
read -r PX PY < <(tail -n +"$L107" "$T/shell.log" | grep -a "debug central: midia cortar \(final \)\?em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 120); do tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: trecho cortado" && break; sleep 0.5; done
CORTE=$(tail -n +"$L107" "$T/shell.log" | grep -a "debug midia: trecho cortado" | tail -1 | sed 's/.*cortado \(.*\) ([0-9]* bytes)/\1/')
echo "  (trecho: $CORTE)"
ok "Chat · mídia: \"Cortar este trecho\" salva o pedaço do capítulo como um vídeo novo em Vídeos" '[ -s "$CORTE" ] && case "$CORTE" in "$X/area/Vídeos/"*) true;; *) false;; esac'
read -r PX PY < <(tail -n +"$L107" "$T/shell.log" | grep -a "debug central: midia transcrever \(final \)\?em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 180); do tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: transcrito (" && break; sleep 1; done
echo "  ($(tail -n +"$L107" "$T/shell.log" | grep -a "debug midia: transcrito (" | tail -1 | cut -c1-200))"
ok "Chat · mídia: o áudio é transcrito no modelo local, cada linha com a hora em que começa" 'tail -n +"$L107" "$T/shell.log" | grep -a "debug midia: transcrito (" | tail -1 | grep -q "linhas: 0:00 «"'
sleep 0.8
read -r PX PY < <(tail -n +"$L107" "$T/shell.log" | grep -a "debug central: midia linha 1 em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: linha 1 acesa" && break; sleep 0.3; done
ok "Chat · mídia: um clique na linha toca dali e a linha ouvida acende" 'tail -n +"$L107" "$T/shell.log" | grep -aq "debug midia: linha 1 acesa"'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-chat-midia-capitulos.png" >/dev/null && sleep 0.5
"$N" msg action close-window --id "$(id_app os.kalango.agentes)" >/dev/null 2>&1
sleep 0.8

# 108. The canvas with live HTML (design "Chat · canvas com HTML ao vivo", 75452): a page the agent writes (accepted in
#      its diff) opens on the canvas, drawn by kalangod's Moli; a click on the button outlines it and opens the field
#      next to it; "Baixar .zip" puts the page in Downloads
mkdir -p "$X/area/Documentos"
L108=$(wc -l < "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" conversar "Crie o arquivo $X/area/Documentos/landing.html com uma pagina HTML simples: um titulo h1, um paragrafo e um link com class cta escrito Baixar. Use CSS dentro do arquivo."
for _ in $(seq 240); do tail -n +"$L108" "$T/shell.log" | grep -aq "debug central: diff aceitar em" && break; sleep 1; done
sleep 1.5
sem_avisos 2>/dev/null || sleep 4
read -r JX JY < <(janela_central)
read -r PX PY < <(tail -n +"$L108" "$T/shell.log" | grep -a "debug central: diff aceitar em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 60); do tail -n +"$L108" "$T/shell.log" | grep -aq "debug canvas: página desenhada" && break; sleep 1; done
ok "Canvas: a página que o agente escreveu abre no canvas, desenhada pelo navegador do kalangod" \
  'tail -n +"$L108" "$T/shell.log" | grep -aq "debug conversas: canvas com .*landing.html" && tail -n +"$L108" "$T/shell.log" | grep -aq "debug canvas: página desenhada (1100 px, versão 1)"'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && sleep 1 && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-canvas-html.png" >/dev/null && sleep 0.5
read -r JX JY < <(janela_central)
read -r PX PY < <(tail -n +"$L108" "$T/shell.log" | grep -a "debug central: canvas zip em" | tail -1 | awk '{print $(NF-1), $NF}')
[ -n "$PX" ] && apontar $((JX + PX)) $((JY + PY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 10); do tail -n +"$L108" "$T/shell.log" | grep -aq "debug canvas: zip" && break; sleep 0.5; done
ok "Canvas: \"Baixar .zip\" põe a página em Downloads" '[ -s "$X/area/Downloads/landing.zip" ]'
"$N" msg action close-window --id "$(id_app os.kalango.agentes)" >/dev/null 2>&1
sleep 0.8

# 86. KalangoOS's own document reader (design "Leitor de documentos"; not GNOME's): a three-page PDF (real text) opens
#     in a tab with its thumbnails; the local model sums it up with the places worth finding, and a click on one goes
#     to its page and lights the words in gold; Ctrl+F finds a word in every page; a question is answered with its
#     page; the zoom; the panel goes away; and the session opens PDFs with it
DOCS="$X/area/Documentos/teste"
mkdir -p "$DOCS"
python3 "$ROOT/sessao/ferramentas/pdf-teste.py" "$DOCS/relatorio.pdf"
"$N" msg action spawn -- "$T/kalango-shell" documentos "$DOCS/relatorio.pdf"
for _ in $(seq 40); do grep -aq "debug documentos: aberto «relatorio.pdf»" "$T/shell.log" && break; sleep 0.3; done
ok "Documentos: o do KalangoOS abre o PDF numa aba com as páginas (os.kalango.Documentos, desenho \"Leitor de documentos\")" \
  'grep -a "debug documentos: aberto «relatorio.pdf»" "$T/shell.log" | tail -1 | grep -q "(3 páginas)" && "$N" msg -j windows | grep -q "\"os.kalango.Documentos\""'
for _ in $(seq 400); do grep -aq "debug documentos: resumo" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug documentos: resumo" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-documentos.png" >/dev/null
ok "Documentos: o modelo local resume o documento e aponta onde está cada coisa" \
  'grep -a "debug documentos: resumo" "$T/shell.log" | tail -1 | grep -Eq "\([1-4] pontos, [1-4] trechos\)"'
sem_avisos 2>/dev/null || sleep 4
clique_doc() { read -r JX JY < <(janela_app os.kalango.Documentos); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.8; }
"$N" msg layers > "$OUT/camadas-documentos.txt" 2>&1
clique_doc "documentos trecho 1"
for _ in $(seq 20); do grep -aq "debug documentos: citação p\." "$T/shell.log" && break; sleep 0.3; done
# (a click that did not take: the screen as it is, for the record, and once more with the place read again)
if ! grep -aq "debug documentos: citação p\." "$T/shell.log"; then
  [ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-documentos-clique.png" >/dev/null
  echo "  (o clique no trecho não pegou: janela $(janela_app os.kalango.Documentos), trecho $(lugar "documentos trecho 1"); camadas em $OUT/camadas-documentos.txt)"
  sleep 1; sem_avisos 2>/dev/null || sleep 4
  clique_doc "documentos trecho 1"
  for _ in $(seq 20); do grep -aq "debug documentos: citação p\." "$T/shell.log" && break; sleep 0.3; done
fi
echo "  ($(grep -a "debug documentos: citação p\." "$T/shell.log" | tail -1))"
ok "Documentos: um trecho do resumo leva à página e realça as palavras em dourado" 'grep -a "debug documentos: citação p\." "$T/shell.log" | tail -1 | grep -Eq "\([1-9][0-9]* marcas\)"'
"$N" msg action focus-window --id "$(id_app kalango.documentos)"
xdotool key --window "$W" ctrl+f
sleep 0.4
# (typed without its accent, as people often do: "pendências" is found all the same)
xdotool type --window "$W" --delay 40 "pendencias"
for _ in $(seq 20); do grep -aq "debug documentos: busca «pendencias»" "$T/shell.log" && break; sleep 0.3; done
ok "Documentos: Ctrl+F acha a palavra em todas as páginas, com ou sem acento" 'grep -a "debug documentos: busca «pendencias»" "$T/shell.log" | tail -1 | grep -Eq "\(([3-9]|[1-9][0-9]) lugares\)"'
xdotool key --window "$W" Escape
clique_doc "documentos perguntar"
xdotool type --window "$W" --delay 40 "Quando sera a proxima revisao?"
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug documentos: resposta p\." "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug documentos: resposta p\." "$T/shell.log" | tail -1))"
# (the citation's place is logged once it is laid out, a little after the answer)
for _ in $(seq 20); do grep -aq "debug central: documentos citacao em" "$T/shell.log" && break; sleep 0.3; done
sleep 0.3
clique_doc "documentos citacao"
for _ in $(seq 20); do [ "$(grep -ac "debug documentos: citação p\. 3" "$T/shell.log")" -ge 1 ] && break; sleep 0.3; done
ok "Documentos: uma pergunta é respondida pelo modelo local com a página de onde vem, e a citação acha o trecho" \
  'grep -a "debug documentos: resposta p\." "$T/shell.log" | tail -1 | grep -q "resposta p\. 3 " && grep -a "debug documentos: citação p\. 3" "$T/shell.log" | tail -1 | grep -Eq "\([1-9][0-9]* marcas\)"'
xdotool key --window "$W" ctrl+plus
for _ in $(seq 20); do grep -aq "debug documentos: zoom 110%" "$T/shell.log" && break; sleep 0.3; done
clique_doc "documentos painel"
for _ in $(seq 20); do grep -aq "debug documentos: painel false" "$T/shell.log" && break; sleep 0.3; done
ok "Documentos: o zoom aumenta e o painel do agente pode ser recolhido" 'grep -aq "debug documentos: zoom 110%" "$T/shell.log" && grep -aq "debug documentos: painel false" "$T/shell.log"'
PADRAO=$(env PATH="$T:$PATH" XDG_CURRENT_DESKTOP=KalangoOS XDG_CONFIG_DIRS="$ROOT/sessao/dist/etc/xdg:/etc/xdg" XDG_DATA_DIRS="$ROOT/sessao/dist/share:/usr/share" XDG_CONFIG_HOME="$X/config" XDG_DATA_HOME="$X/data" gio mime application/pdf 2>/dev/null | head -1)
echo "  (application/pdf: $PADRAO)"
ok "Documentos: na sessão do KalangoOS os PDFs abrem nele" 'echo "$PADRAO" | grep -q "os.kalango.Documentos.desktop"'
"$N" msg action close-window --id "$(id_app kalango.documentos)"
sleep 0.8

# 87. KalangoOS's own Disks app (design "Discos"; not GNOME's): the test's four drives (no real disk is read or touched:
#     the actions are only written down); the HDD wearing out holds the backups and the agent offers to copy them to
#     the SSD — they are copied; a partition chosen on the bar; checking and formatting go through a confirmation that
#     says exactly what happens; the system's partition in use is never formatted; the read rate is live; a request
#     ("verifique o pendrive") becomes the action
python3 "$ROOT/sessao/ferramentas/discos-teste.py" "$OUT/discos.json" "$X/discos"
printf 'nvme0n1 1000 1000\nsda 0 0\nsdb 0 0\nsdc 0 0\n' > "$OUT/discos.stats"
"$N" msg action spawn -- "$T/kalango-shell" discos
for _ in $(seq 40); do grep -aq "debug discos: 4 unidades" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug discos: 4 unidades" "$T/shell.log" | tail -1))"
ok "Discos: o do KalangoOS mostra as unidades com a saúde de cada uma (os.kalango.Discos, desenho \"Discos\")" \
  'grep -a "debug discos: 4 unidades" "$T/shell.log" | tail -1 | grep -q "NVMe 1 TB (Healthy); SSD 2 TB (Healthy); HDD 500 GB (Attention); Kingston 64 GB (Removable)" && "$N" msg -j windows | grep -q "\"os.kalango.Discos\""'
# the read rate: the counts grow 2 GB a second for three seconds
for k in 1 2 3; do printf 'nvme0n1 %d %d\nsda 0 0\nsdb 0 0\nsdc 0 0\n' $((1000 + k * 3906250)) $((1000 + k * 1953125)) > "$OUT/discos.stats"; sleep 1; done
for _ in $(seq 10); do grep -aq "debug discos: velocidade nvme0n1" "$T/shell.log" && break; sleep 0.3; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-discos.png" >/dev/null
ok "Discos: a velocidade de leitura e gravação é medida ao vivo" 'grep -a "debug discos: velocidade nvme0n1" "$T/shell.log" | tail -1 | grep -Eq "leitura [1-4]\.?[0-9]{3} MB/s, gravação [0-9.]+ MB/s"'
ok "Discos: o agente vê o HDD gasto com os backups e oferece copiá-los para o SSD" 'grep -aq "debug discos: aviso: HDD 500 GB gasto, backups → SSD 2 TB" "$T/shell.log"'
sem_avisos 2>/dev/null || sleep 4
clique_dsc() { read -r JX JY < <(janela_app os.kalango.Discos); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX + ${2:-0})) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.8; }
clique_dsc "discos copiar backups"
for _ in $(seq 30); do [ -f "$X/discos/Dados/Backups (do HDD 500 GB)/fotos-2025.tar" ] && break; sleep 0.5; done
ok "Discos: \"Copiar backups\" copia os backups do HDD para o SSD" '[ -f "$X/discos/Dados/Backups (do HDD 500 GB)/fotos-2025.tar" ] && [ -f "$X/discos/Dados/Backups (do HDD 500 GB)/documentos-2026.tar" ]'
# the system's partition in use: formatting it is refused; Ubuntu's (left on the bar) is erased only once confirmed
sem_avisos 2>/dev/null || sleep 4
clique_dsc "discos formatar"
F0=$(grep -c '"Format"' "$OUT/discos.log" 2>/dev/null || true)
sem_avisos 2>/dev/null || sleep 4
clique_dsc "discos particoes" -200
for _ in $(seq 10); do grep -aq "debug discos: partição «Ubuntu · ext4 · não montada»" "$T/shell.log" && break; sleep 0.3; done
clique_dsc "discos formatar"
for _ in $(seq 10); do grep -aq "debug discos: confirmar «Formatar…»" "$T/shell.log" && break; sleep 0.3; done
clique_dsc "discos confirmar"
for _ in $(seq 20); do grep -qs '"Format"' "$OUT/discos.log" && break; sleep 0.3; done
echo "  ($(tail -1 "$OUT/discos.log" 2>/dev/null))"
ok "Discos: formatar a partição escolhida passa por uma confirmação, e a do sistema em uso nunca" \
  '[ "${F0:-0}" = "0" ] && grep -a "debug discos: partição «Ubuntu" "$T/shell.log" >/dev/null && grep -q "\"Format\":{\"part\":\"/org/freedesktop/UDisks2/block_devices/nvme0n1p2\",\"fs\":\"ext4\"" "$OUT/discos.log"'
clique_dsc "discos verificar"
clique_dsc "discos confirmar"
for _ in $(seq 20); do grep -qs '"Check"' "$OUT/discos.log" && break; sleep 0.3; done
ok "Discos: verificar confirma e pede a verificação da partição" 'grep -q "\"Check\":{\"drive\":\"/org/freedesktop/UDisks2/drives/Samsung_990\",\"part\":\"/org/freedesktop/UDisks2/block_devices/nvme0n1p2\"}" "$OUT/discos.log"'
"$N" msg action focus-window --id "$(id_app kalango.discos)"
sleep 0.3
xdotool key --window "$W" ctrl+k
sleep 0.4
xdotool type --window "$W" --delay 40 "verifique o pendrive"
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug discos: pedido →" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug discos: pedido →" "$T/shell.log" | tail -1))"
ok "Discos: um pedido (\"verifique o pendrive\") vira a ação no pendrive, mostrada para confirmar" \
  'grep -a "debug discos: pedido →" "$T/shell.log" | tail -1 | grep -q "verificar (unidade Some(3)" && grep -a "debug discos: confirmar «" "$T/shell.log" | tail -1 | grep -q "Verificar"'
"$N" msg action close-window --id "$(id_app kalango.discos)"
sleep 0.8

# 104. The App Center (design "Central de Apps (descobrir)", 42430; os.kalango.Apps): what it shows is read from the
#      test's own catalog (loja-teste.py: a Flatpak remote's AppStream with icons, the shelf, a models catalog whose
#      test model is a local file) and the system's agent manifests; installs go to the flatpak stand-in. The agents'
#      cards say what each reads, changes, runs and reaches, from the permissions; a chip filters; "ver todos" opens the
#      section's page; an app installs (Flatpak, its progress in the button) and then opens; the search reaches the
#      whole catalog; a model downloads into the models folder; an agent installs through "Instalar agente" (from the
#      catalog: verified) and its card then opens its routine; Atualizações updates the Flatpak app with a new version;
#      "Como funcionam as permissões" opens the Help on that topic
clique_loja() { "$N" msg action focus-window --id "$(id_app kalango.apps)" >/dev/null 2>&1; sleep 0.3; read -r JX JY < <(janela_app os.kalango.Apps); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.8; }
L0=$(grep -ac "debug loja: " "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" loja
for _ in $(seq 40); do grep -a "debug loja: " "$T/shell.log" | tail -n +$((L0 + 1)) | grep -q "debug loja: descobrir · 3 agentes, 4 apps, 3 modelos" && break; sleep 0.3; done
for _ in $(seq 20); do grep -aq "debug loja: catálogo com 5 apps" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug loja: catálogo com\|debug loja: descobrir ·" "$T/shell.log" | tail -2 | tr '\n' ' '))"
ok "Central de Apps: abre com os agentes do catálogo do sistema, os apps essenciais (catálogo do Flatpak) e os modelos (os.kalango.Apps)" \
  'grep -aq "debug loja: catálogo com 5 apps" "$T/shell.log" && grep -a "debug loja: descobrir ·" "$T/shell.log" | tail -1 | grep -q "3 agentes, 4 apps, 3 modelos" && "$N" msg -j windows | grep -q "\"os.kalango.Apps\""'
ok "Central de Apps: o modelo instalado diz que cabe; o grande demais diz por quê" \
  'grep -aq "debug loja: modelo lfm3-1.7b instalado (cabe" "$T/shell.log" && grep -a "debug loja: modelo teste-grande a baixar" "$T/shell.log" | tail -1 | grep -Eq "maior que a GPU|não cabe"'
sem_avisos 2>/dev/null || sleep 4
clique_loja "loja categoria jogos"
for _ in $(seq 10); do grep -a "debug loja: descobrir ·" "$T/shell.log" | tail -1 | grep -q "0 agentes, 1 apps, 0 modelos" && break; sleep 0.3; done
ok "Central de Apps: a categoria Jogos deixa só o jogo" 'grep -a "debug loja: descobrir ·" "$T/shell.log" | tail -1 | grep -q "0 agentes, 1 apps, 0 modelos"'
clique_loja "loja categoria tudo"
clique_loja "loja ver todos apps"
for _ in $(seq 10); do grep -aq "debug loja: apps · 0 agentes, 4 apps" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: \"ver todos\" abre a página dos apps" 'grep -a "debug loja: página" "$T/shell.log" | tail -1 | grep -q "página apps" && grep -aq "debug loja: apps · 0 agentes, 4 apps" "$T/shell.log"'
sem_avisos 2>/dev/null || sleep 4
clique_loja "loja instalar org.exemplo.Notas"
for _ in $(seq 30); do grep -aq "debug loja: app:org.exemplo.Notas pronto" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 10); do grep -aq "debug central: loja abrir org.exemplo.Notas em" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: Instalar instala pelo Flatpak (na instalação da pessoa) e o cartão passa a Abrir" \
  'grep -q "flatpak --user install --noninteractive -y flathub org.exemplo.Notas" "$OUT/flatpak.log" && grep -aq "debug central: loja abrir org.exemplo.Notas em" "$T/shell.log"'
clique_loja "loja abrir org.exemplo.Notas"
for _ in $(seq 10); do grep -q "flatpak run org.exemplo.Notas" "$OUT/flatpak.log" && break; sleep 0.3; done
ok "Central de Apps: Abrir roda o app instalado" 'grep -q "flatpak run org.exemplo.Notas" "$OUT/flatpak.log"'
clique_loja "loja busca"
xdotool type --window "$W" --delay 40 "mapas"
for _ in $(seq 20); do grep -aq "debug loja: apps · 0 agentes, 1 apps, 0 modelos para «mapas»" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 10); do grep -aq "debug central: loja instalar org.exemplo.Mapas em" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: a busca alcança o catálogo inteiro (um app fora dos essenciais)" 'grep -aq "debug loja: apps · 0 agentes, 1 apps, 0 modelos para «mapas»" "$T/shell.log" && grep -aq "debug central: loja instalar org.exemplo.Mapas em" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.5
clique_loja "loja página modelos"
clique_loja "loja baixar teste-mini"
for _ in $(seq 30); do grep -aq "debug loja: modelo:teste-mini pronto" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 10); do grep -aq "debug central: loja instalado teste-mini em" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: Baixar traz o modelo para a pasta dos modelos (inteiro, sem o .part) e ele fica Instalado" \
  '[ "$(stat -c %s "$X/data/kalango/modelos/teste-mini.gguf" 2>/dev/null)" = "$(stat -c %s "$OUT/loja/mini.gguf")" ] && [ ! -e "$X/data/kalango/modelos/teste-mini.gguf.part" ] && grep -aq "debug central: loja instalado teste-mini em" "$T/shell.log"'
rm -f "$X/data/kalango/modelos/teste-mini.gguf"
clique_loja "loja página agentes"
for _ in $(seq 10); do grep -aq "debug loja: agente comunidade.tutor:" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: cada agente diz o que lê, muda, roda e alcança — das permissões — e quanto decide sozinho" \
  'grep -aq "debug loja: agente labs.devsync: lê ~/Projetos | git | github.com · pergunta antes do que é arriscado" "$T/shell.log" && grep -aq "debug loja: agente labs.organizador: lê e muda ~/Downloads | sem rede · pergunta antes de cada mudança" "$T/shell.log" && grep -aq "debug loja: agente comunidade.tutor: vê a tela quando chamado | muda ~/Documentos/Estudos | sem rede" "$T/shell.log"'
sem_avisos 2>/dev/null || sleep 4
clique_loja "loja instalar labs.organizador"
for _ in $(seq 20); do grep -aq "debug instalar agente: «Organizador» de KalangoOS Labs (2 permissões pedidas, do catálogo)" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: Instalar um agente abre \"Instalar agente\" com as permissões dele (do catálogo: verificado)" \
  'grep -aq "debug instalar agente: «Organizador» de KalangoOS Labs (2 permissões pedidas, do catálogo)" "$T/shell.log" && grep -aq "debug loja: agente labs.organizador → instalar" "$T/shell.log"'
for _ in $(seq 20); do "$N" msg -j windows | grep -q "\"os.kalango.Agentes\"" && break; sleep 0.3; done
sleep 0.8
read -r JX JY < <(janela_app os.kalango.Agentes); read -r CX CY < <(lugar "instalar agente instalar")
[ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug instalar agente: instalado «Organizador»" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
"$N" msg action spawn -- "$T/kalango-shell" loja --pagina=agentes
for _ in $(seq 20); do grep -aq "debug central: loja abrir labs.organizador em" "$T/shell.log" && break; sleep 0.3; done
clique_loja "loja abrir labs.organizador"
for _ in $(seq 10); do grep -aq "debug loja: agente labs.organizador → rotinas" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: instalado, o agente vira rotina e o cartão Abrir leva a ela" \
  'grep -aq "debug instalar agente: instalado «Organizador» (pastas ~/Downloads" "$T/shell.log" && grep -aq "debug loja: agente labs.organizador → rotinas" "$T/shell.log"'
clique_loja "loja página atualizacoes"
for _ in $(seq 20); do grep -aq "debug loja: atualizações na tela: org.exemplo.Fotos" "$T/shell.log" && break; sleep 0.3; done
clique_loja "loja atualizar org.exemplo.Fotos"
for _ in $(seq 30); do grep -aq "debug loja: atualizar:org.exemplo.Fotos pronto" "$T/shell.log" && break; sleep 0.3; done
sleep 0.5
ok "Central de Apps: Atualizações lista o app do Flatpak com versão nova e Atualizar o atualiza" \
  'grep -q "flatpak update --noninteractive -y org.exemplo.Fotos" "$OUT/flatpak.log" && grep -a "debug loja: atualizações na tela:" "$T/shell.log" | tail -1 | grep -q "tela: $"'
clique_loja "loja página descobrir"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-loja.png" >/dev/null
sem_avisos 2>/dev/null || sleep 4
clique_loja "loja permissoes"
for _ in $(seq 20); do grep -aq "debug ajuda: tópico «O que os agentes podem ver e fazer»" "$T/shell.log" && break; sleep 0.3; done
ok "Central de Apps: \"Como funcionam as permissões\" abre a Ajuda no tópico das permissões dos agentes" 'grep -aq "debug ajuda: tópico «O que os agentes podem ver e fazer»" "$T/shell.log"'
for a in kalango.ajuda kalango.agentes kalango.apps; do "$N" msg action close-window --id "$(id_app $a)" >/dev/null 2>&1; sleep 0.4; done
# (the stand-in back as it was: later checks list its apps)
/usr/bin/python3 -c 'import json,sys; p=sys.argv[1]; e=json.load(open(p)); e["instaladas"]=[]; e["atualizacoes"]=["org.exemplo.Fotos"]; json.dump(e, open(p,"w"))' "$OUT/flatpak.json" 2>/dev/null
sleep 0.8
# 105. "App não está respondendo" (design 54053): an app whose interface waits for a child of ~200 MB reads none of
#      the compositor's events; 5 s later its windows go to 60 % and the dialog says what holds it (its process, the
#      diagnosis, the system's memory) with the least destructive way out first: ending only that child gives the app
#      back, the dialog closes by itself and the window is whole again
: > "$OUT/travado.log"
L105=$(wc -l < "$T/shell.log")
"$N" msg action spawn -- "$ROOT/sessao/ferramentas/travado-teste.py" "$OUT/travado.log"
for _ in $(seq 20); do grep -q "travado: esperando o filho" "$OUT/travado.log" && break; sleep 0.5; done
# events for it to leave unread: its window focused and the pointer moving over it (wherever niri put it), until the
# shell sees it stuck
for _ in $(seq 20); do [ -n "$(id_da_janela org.exemplo.Planilha)" ] && break; sleep 0.5; done
"$N" msg action focus-window --id "$(id_da_janela org.exemplo.Planilha)" >/dev/null 2>&1
read -r PJX PJY < <(janela_app org.exemplo.Planilha 2>/dev/null)
for i in $(seq 30); do
  apontar $((${PJX:-300} + 120 + i % 5 * 20)) $((${PJY:-300} + 120 + i % 3 * 20)); sleep 0.5
  tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: janelas de org.exemplo.Planilha a 60%" && break
done
# (the diagnosis comes from the local model: it may be starting)
for _ in $(seq 240); do tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: diagnóstico «" && break; sleep 0.5; done
sleep 0.5
ok "Não está respondendo: o app que não lê os eventos é visto, fica a 60% e o diálogo aponta o processo que o prende" \
  'tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: janelas de org.exemplo.Planilha a 60%" && tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: diálogo «Planilha» · estado S · 2 processos · destaque planilha-calculo" && tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: diagnóstico «.*planilha-calculo"'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-travado.png" >/dev/null && sleep 0.5
sleep 0.5
sem_avisos 2>/dev/null || sleep 4
read -r PX PY < <(tail -n +"$L105" "$T/shell.log" | grep -a "debug central: travado primeiro em" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$PX" "$PY"
for i in $(seq 20); do apontar 300 $((420 + i % 3 * 20)); grep -q "travado: respondendo de novo" "$OUT/travado.log" && tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: voltou a responder" && break; sleep 0.5; done
ok "Não está respondendo: encerrar só o processo pesado devolve o app, e o diálogo fecha sozinho" \
  'tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: encerrar só" && grep -q "travado: respondendo de novo" "$OUT/travado.log" && tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: diálogo fechado" && tail -n +"$L105" "$T/shell.log" | grep -aq "debug travado: voltou a responder"'
"$N" msg action close-window --id "$(id_app org.exemplo.planilha)" >/dev/null 2>&1
sleep 1
# (whatever happened above, the test's app and its child do not stay)
for p in $(sed -n 's/^travado: janela aberta (pid \([0-9]*\))$/\1/p; s/^travado: esperando o filho \([0-9]*\)$/\1/p' "$OUT/travado.log"); do kill "$p" 2>/dev/null; done
sleep 0.5
# 106. "Janelas lado a lado" (design 39375): three windows snapped as "Foco + pilha" get a divider where they meet;
#      dragging it resizes the editor and the stack together, and a double click makes the sides equal
fechar_apps
abrir_ed
abrir_calc
"$N" msg action spawn -- "$T/kalango-shell" terminal
for _ in $(seq 40); do [ "$("$N" msg -j windows | grep -cE '"os.kalango.(Editor|Calculadora|Terminal)"')" -ge 3 ] && break; sleep 0.5; done
sleep 1.5
L106=$(wc -l < "$T/shell.log")
"$N" msg action snap-window --id "$(id_da_janela os.kalango.Editor)" "0,0,600,1000"
"$N" msg action snap-window --id "$(id_da_janela os.kalango.Terminal)" "600,0,400,500"
"$N" msg action snap-window --id "$(id_da_janela os.kalango.Calculadora)" "600,500,400,500"
for _ in $(seq 20); do tail -n +"$L106" "$T/shell.log" | grep -aq "debug central: divisória 0 em" && break; sleep 0.3; done
ok "Lado a lado: as janelas encaixadas ganham a divisória onde se encontram" \
  'tail -n +"$L106" "$T/shell.log" | grep -a "debug divisória: 2 — vertical em" | tail -1 | grep -q "horizontal em"'
# (a banner from a screenshot above covers the screen's top for a while: wait for it to go)
sem_avisos 2>/dev/null || sleep 4
read -r DX DY < <(tail -n +"$L106" "$T/shell.log" | grep -a "debug central: divisória 0 em" | tail -1 | awk '{print $(NF-1), $NF}')
apontar "$DX" "$DY"; sleep 0.3; xdotool mousedown 1
for d in 20 60 120 200 250; do xdotool mousemove --window "$W" $((DX - d)) "$DY"; sleep 0.1; done
sleep 0.3; xdotool mouseup 1; sleep 1.5
LARG=$("$N" msg -j windows | python3 -c 'import json,sys; print([int(w["layout"]["tile_size"][0]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Editor"][0])')
echo "  (editor com $LARG px depois de arrastar a divisória de $DX para $((DX - 250)))"
ok "Lado a lado: arrastar a divisória redimensiona o editor e a pilha juntos" \
  'tail -n +"$L106" "$T/shell.log" | grep -aq "debug divisória: arrastada de $DX para $((DX - 250)) (3 janelas)" && [ "$LARG" = "$((DX - 250))" ]'
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-lado-a-lado.png" >/dev/null && sleep 0.5
apontar $((DX - 250)) "$DY"; sleep 0.3; xdotool click --repeat 2 --delay 80 1; sleep 1.5
ok "Lado a lado: o duplo clique na divisória iguala os lados" 'tail -n +"$L106" "$T/shell.log" | grep -aq "debug divisória: igualada em"'
fechar_apps
# 88. KalangoOS's own Calendar (design "Calendário"; not GNOME's): today's events from its own calendar file; the
#     agent notices the meeting with no agenda and drafts it (saved and opened); a drag on the grid makes an event
#     (named where it was drawn) and another drag moves it; a request ("marque dentista quinta as 14h") becomes an
#     event; a calendar hidden with its eye; the month view
CAL="$X/data/kalango/calendario"
mkdir -p "$CAL"
HOJE=$(date +%Y%m%d)
printf 'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Pessoal\r\nBEGIN:VEVENT\r\nUID:teste-reuniao\r\nSUMMARY:Reunião de produto\r\nDTSTART:%sT150000\r\nDTEND:%sT160000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:teste-academia\r\nSUMMARY:Academia\r\nDTSTART:%sT180000\r\nDTEND:%sT190000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n' "$HOJE" "$HOJE" "$HOJE" "$HOJE" > "$CAL/pessoal.ics"
"$N" msg action spawn -- "$T/kalango-shell" calendario
for _ in $(seq 40); do grep -aq "debug calendario: [0-9]* eventos de" "$T/shell.log" && break; sleep 0.3; done
sleep 1
echo "  ($(grep -a "debug calendario: [0-9]* eventos de" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-calendario.png" >/dev/null
ok "Calendário: o do KalangoOS mostra a semana com os eventos da agenda (os.kalango.Calendario, desenho \"Calendário\")" \
  'grep -a "debug calendario: [0-9]* eventos de" "$T/shell.log" | tail -1 | grep -Eq "^debug calendario: ([2-9]|[1-9][0-9]+) eventos" && "$N" msg -j windows | grep -q "\"os.kalango.Calendario\""'
ok "Calendário: o agente nota a reunião de hoje sem pauta" 'grep -aq "debug calendario: o agente notou «Reunião de produto» sem pauta" "$T/shell.log"'
sem_avisos 2>/dev/null || sleep 4
clique_cal() { read -r JX JY < <(janela_app os.kalango.Calendario); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX + ${2:-0})) $((JY + CY + ${3:-0})) && sleep 0.3 && xdotool click 1; sleep 0.8; }
clique_cal "calendario rascunhar"
for _ in $(seq 240); do grep -aq "debug calendario: pauta salva «" "$T/shell.log" && break; sleep 0.5; done
PAUTA=$(grep -a "debug calendario: pauta salva «" "$T/shell.log" | tail -1 | sed 's/.*«\(.*\)».*/\1/')
echo "  (pauta: $PAUTA)"
ok "Calendário: \"Rascunhar pauta\" — o modelo local escreve a pauta, salva e aberta no Editor" '[ -s "$PAUTA" ] && grep -q "^# Pauta: Reunião de produto" "$PAUTA"'
fechar_editor() { "$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Editor"]' | while read -r id; do "$N" msg action close-window --id "$id"; done; }
sleep 1
fechar_editor
"$N" msg action focus-window --id "$(id_app kalango.calendario)"
# (the Editor just closed: niri settles the windows and the pointer first — run 45's drag reached nothing and its
# title became view shortcuts)
sleep 1.2
sem_avisos 2>/dev/null || sleep 4
# a drag on the grid (an hour down from where it starts) makes an event; its name typed there
arrastar_grade() {
  read -r JX JY < <(janela_app os.kalango.Calendario); read -r GX GY < <(lugar "calendario grade")
  apontar $((JX + GX)) $((JY + GY)); sleep 0.3
  xdotool mousedown 1; sleep 0.2
  for k in 1 2 3 4; do xdotool mousemove_relative 0 14; sleep 0.08; done
  sleep 0.2; xdotool mouseup 1
  for _ in $(seq 20); do grep -aq "debug calendario: novo bloco" "$T/shell.log" && return 0; sleep 0.3; done
  return 1
}
if ! arrastar_grade; then
  [ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-calendario-arrastar.png" >/dev/null
  echo "  (o arrasto não fez bloco: de novo; janela $JX $JY, grade $GX $GY)"
  sleep 1; "$N" msg action focus-window --id "$(id_app kalango.calendario)"; sleep 0.8
  arrastar_grade
fi
sleep 0.5
xdotool type --window "$W" --delay 40 "Teste do calendario"
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug calendario: evento criado «Teste do calendario»" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug calendario: evento criado «Teste do calendario»" "$T/shell.log" | tail -1))"
ok "Calendário: arrastar na grade cria o evento, com o nome digitado ali" \
  'grep -a "debug calendario: evento criado «Teste do calendario»" "$T/shell.log" | grep -q "[0-9]–[0-9].* em pessoal" && grep -q "SUMMARY:Teste do calendario" "$CAL/pessoal.ics"'
# and a drag on it moves it two hours later
sleep 0.5
apontar $((JX + GX)) $((JY + GY + 20)); sleep 0.3
xdotool mousedown 1; sleep 0.2
for k in 1 2 3 4 5 6 7 8; do xdotool mousemove_relative 0 14; sleep 0.08; done
sleep 0.2; xdotool mouseup 1
for _ in $(seq 20); do grep -aq "debug calendario: movido «Teste do calendario»" "$T/shell.log" && break; sleep 0.3; done
ok "Calendário: arrastar o evento o move" 'grep -aq "debug calendario: movido «Teste do calendario»" "$T/shell.log"'
xdotool key --window "$W" ctrl+k
sleep 0.4
xdotool type --window "$W" --delay 40 "marque dentista quinta as 14h"
xdotool key --window "$W" Return
for _ in $(seq 240); do grep -aq "debug calendario: pedido →" "$T/shell.log" && break; sleep 0.5; done
for _ in $(seq 10); do grep -aq "debug calendario: evento criado «.*» .* (pedido)" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug calendario: pedido →\|(pedido)" "$T/shell.log" | tail -2 | tr '\n' ' '))"
ok "Calendário: um pedido (\"marque dentista quinta as 14h\") vira o evento na quinta às 14h" \
  'grep -a "debug calendario: evento criado «.*» .* (pedido)" "$T/shell.log" | tail -1 | grep -iq "dentista.* 14:00–15:00 em pessoal" && [ "$(date -d "$(grep -a "debug calendario: evento criado «.*» .* (pedido)" "$T/shell.log" | tail -1 | grep -o "[0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}")" +%u)" = 4 ]'
clique_cal "calendario agenda pessoal"
for _ in $(seq 10); do grep -aq "debug calendario: agenda pessoal escondida" "$T/shell.log" && break; sleep 0.3; done
clique_cal "calendario agenda pessoal"
clique_cal "calendario trilho mes"
for _ in $(seq 10); do grep -aq "debug calendario: vista Month" "$T/shell.log" && break; sleep 0.3; done
ok "Calendário: o olho esconde uma agenda, e a vista do mês" 'grep -aq "debug calendario: agenda pessoal escondida" "$T/shell.log" && grep -aq "debug calendario: vista Month" "$T/shell.log"'
"$N" msg action close-window --id "$(id_app kalango.calendario)"
sleep 0.8

# 89. KalangoOS's own Help (design "Ajuda"): the installed manual; a question answered by the local model from it with
#     its Settings page; "Mostre para mim" dims the screen and rings the place to click in Settings (a click there goes
#     on); a common question shows its topic; a problem report saved on the computer (nothing sent)
"$N" msg action spawn -- "$T/kalango-shell" ajuda "como eu conecto um segundo monitor?"
for _ in $(seq 240); do grep -aq "debug ajuda: resposta (" "$T/shell.log" && break; sleep 0.5; done
echo "  ($(grep -a "debug ajuda: manual com\|debug ajuda: resposta (" "$T/shell.log" | tail -2 | tr '\n' ' ' | cut -c1-220))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-ajuda.png" >/dev/null
ok "Ajuda: a do KalangoOS responde pelo modelo local a partir do manual instalado, com a página de Configurações (os.kalango.Ajuda)" \
  'grep -aEq "debug ajuda: manual com 1[0-9] tópicos" "$T/shell.log" && grep -a "debug ajuda: resposta (" "$T/shell.log" | tail -1 | grep -q "página Some(\"tela\")" && "$N" msg -j windows | grep -q "\"os.kalango.Ajuda\""'
sem_avisos 2>/dev/null || sleep 4
clique_aju() { read -r JX JY < <(janela_app os.kalango.Ajuda); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.8; }
clique_aju "ajuda mostre"
for _ in $(seq 30); do grep -aq "debug ajuda: mostre para mim «tela» em" "$T/shell.log" && break; sleep 0.3; done
sleep 1
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-ajuda-mostre.png" >/dev/null
read -r AX AY AW AH < <(grep -a "debug ajuda: mostre para mim «tela» em" "$T/shell.log" | tail -1 | sed 's/.* em \([0-9-]*\) \([0-9-]*\) (\([0-9]*\)×\([0-9]*\)).*/\1 \2 \3 \4/')
[ -n "$AX" ] && apontar $((AX + AW / 2)) $((AY + AH / 2)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug ajuda: alvo clicado" "$T/shell.log" && break; sleep 0.3; done
ok "Ajuda: \"Mostre para mim\" abre Configurações e marca na tela onde clicar (Tela); clicar ali segue" \
  '[ -n "$AX" ] && grep -aq "debug ajuda: alvo clicado" "$T/shell.log"'
"$N" msg -j windows | python3 -c 'import json,sys; [print(w["id"]) for w in json.load(sys.stdin) if w["app_id"]=="os.kalango.Configuracoes"]' | while read -r id; do "$N" msg action close-window --id "$id"; done
sleep 0.8
"$N" msg action focus-window --id "$(id_app kalango.ajuda)"
sleep 0.4
clique_aju "ajuda topico 02-wifi"
for _ in $(seq 10); do grep -aq "debug ajuda: tópico «Conectar ao Wi-Fi»" "$T/shell.log" && break; sleep 0.3; done
ok "Ajuda: uma pergunta comum mostra o tópico do manual" 'grep -aq "debug ajuda: tópico «Conectar ao Wi-Fi»" "$T/shell.log"'
clique_aju "ajuda relatar"
sleep 0.8
xdotool type --window "$W" --delay 30 "a tela piscou ao abrir o calendario"
sleep 0.3
read -r CX CY < <(lugar "ajuda salvar relato")
DJ=$("$N" msg -j windows | python3 -c 'import json,sys; l=[w for w in json.load(sys.stdin) if w.get("is_focused")][0]["layout"]; print(int(l["tile_pos_in_workspace_view"][0]+l["window_offset_in_tile"][0]), int(l["tile_pos_in_workspace_view"][1]+l["window_offset_in_tile"][1]))')
read -r DX DY <<< "$DJ"
[ -n "$CX" ] && apontar $((DX + CX)) $((DY + CY)) && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug ajuda: relato salvo «" "$T/shell.log" && break; sleep 0.3; done
RELATO=$(grep -a "debug ajuda: relato salvo «" "$T/shell.log" | tail -1 | sed 's/.*«\(.*\)».*/\1/')
echo "  (relato: $RELATO)"
ok "Ajuda: \"Relatar um problema\" salva o relato no computador, com os dados do sistema (nada é enviado)" \
  '[ -s "$RELATO" ] && grep -q "a tela piscou ao abrir o calendario" "$RELATO" && grep -q "Sobre este computador" "$RELATO"'
"$N" msg action close-window --id "$(id_app kalango.ajuda)"
sleep 0.8

# 90. KalangoOS's print dialog (design "Imprimir"): Ctrl+P in an app goes through the Print portal to the system's own
#     dialog, with the document's real pages; two copies of pages 2–3 go to CUPS (the test's lp: nothing is printed);
#     "Salvar como PDF" keeps it in Documents
"$N" msg action spawn -- "$T/kalango-shell" documentos "$X/area/Documentos/teste/relatorio.pdf"
for _ in $(seq 40); do [ "$(grep -ac "debug documentos: aberto «relatorio.pdf»" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
sleep 1.5
"$N" msg action focus-window --id "$(id_app kalango.documentos)"
sleep 0.4
xdotool key --window "$W" ctrl+p
for _ in $(seq 40); do grep -aq "debug imprimir: aberto «" "$T/shell.log" && break; sleep 0.3; done
sleep 1.2
echo "  ($(grep -a "debug imprimir: aberto «" "$T/shell.log" | tail -1))"
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-imprimir.png" >/dev/null
ok "Imprimir: Ctrl+P passa pelo portal e abre o diálogo do KalangoOS com as páginas do documento e as impressoras (os.kalango.Imprimir)" \
  'grep -a "debug imprimir: aberto «" "$T/shell.log" | tail -1 | grep -q "(3 páginas, 2 impressoras)" && "$N" msg -j windows | grep -q "\"os.kalango.Imprimir\""'
sem_avisos 2>/dev/null || sleep 4
clique_imp() { read -r JX JY < <(janela_app os.kalango.Imprimir); read -r CX CY < <(lugar "$1"); [ -n "$CX" ] && apontar $((JX + CX)) $((JY + CY)) && sleep 0.3 && xdotool click 1; sleep 0.6; }
clique_imp "imprimir copias mais"
clique_imp "imprimir paginas 2"
sleep 0.4
xdotool type --window "$W" --delay 40 "2-3"
sleep 0.4
clique_imp "imprimir imprimir"
for _ in $(seq 20); do [ -s "$OUT/impressao.log" ] && break; sleep 0.3; done
echo "  ($(tail -1 "$OUT/impressao.log" 2>/dev/null | cut -c1-200))"
PAGINAS_PDF=$(pdfinfo "$OUT/impressao.pdf" 2>/dev/null | awk '/^Pages:/ {print $2}')
ok "Imprimir: duas cópias das páginas 2–3 vão para o CUPS (lp), com o PDF do documento" \
  'grep -q "^lp -d HP_LaserJet_M110 -n 2 .*-P 2-3 " "$OUT/impressao.log" && [ "$PAGINAS_PDF" = 3 ]'
"$N" msg action focus-window --id "$(id_app kalango.documentos)"
sleep 0.4
xdotool key --window "$W" ctrl+p
for _ in $(seq 40); do [ "$(grep -ac "debug imprimir: aberto «" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
sleep 1.2
clique_imp "imprimir salvar pdf"
for _ in $(seq 20); do grep -aq "debug imprimir: salvo como PDF «" "$T/shell.log" && break; sleep 0.3; done
SALVO=$(grep -a "debug imprimir: salvo como PDF «" "$T/shell.log" | tail -1 | sed 's/.*«\(.*\)».*/\1/')
ok "Imprimir: \"Salvar como PDF\" guarda o documento em Documentos" '[ -s "$SALVO" ] && pdfinfo "$SALVO" 2>/dev/null | grep -q "^Pages: *3"'
"$N" msg action close-window --id "$(id_app kalango.documentos)"
sleep 0.8

# 91. KalangoOS's own on-screen keyboard (design "Teclado virtual"; not another system's): shown, it types into the
#     focused app through the compositor's virtual keyboard; a word is completed from its suggestions; a held key
#     offers its accents; the compositor tells it when a text field has the focus (input-method-v2)
# (the keyboard first: bound to the compositor, it hears of the Editor's field when the Editor takes the focus)
"$N" msg action spawn -- "$T/kalango-shell" teclado
for _ in $(seq 30); do grep -aq "debug teclado: à vista (" "$T/shell.log" && break; sleep 0.3; done
"$N" msg action spawn -- "$T/kalango-shell" editor
for _ in $(seq 30); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.3; done
sleep 1.5
"$N" msg action focus-window --id "$(id_app kalango.editor)"
sleep 0.5
read -r KX KY < <(grep -a "debug teclado: à vista (" "$T/shell.log" | tail -1 | sed 's/.* em \(-\?[0-9]*\) \(-\?[0-9]*\)$/\1 \2/')
# (a key whose press was not taken — the first one, while the keyboard settles — is pressed again)
tecla() { local n0; n0=$(grep -ac "debug teclado: \(tecla\|sugestão\)" "$T/shell.log"); read -r CX CY < <(lugar "teclado $1"); [ -n "$CX" ] || return
  apontar $((KX + CX)) $((KY + CY)) && sleep 0.15 && xdotool click 1; sleep 0.35
  [ "$(grep -ac "debug teclado: \(tecla\|sugestão\)" "$T/shell.log")" -gt "$n0" ] || { apontar $((KX + CX)) $((KY + CY)); sleep 0.2; xdotool click 1; sleep 0.35; }; }
# the editor's text gets the focus (a click in it): the compositor tells the keyboard a field is focused
read -r EJX EJY < <(janela_app os.kalango.Editor)
xdotool mousemove --window "$W" $((EJX + 160)) $((EJY + 120)) click 1
sleep 0.8
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-teclado.png" >/dev/null
for k in r e u; do tecla "$k"; done
tecla "sugestao 1"
# held "a": its accents
read -r CX CY < <(lugar "teclado a")
apontar $((KX + CX)) $((KY + CY)); sleep 0.2; xdotool mousedown 1; sleep 0.9; xdotool mouseup 1; sleep 0.6
# (the accents' bubble sits centred over the key — five 48 px keys: "á" is 104 px left of the "a" and 88 px above;
# a popover's own logged place is off inside a layer surface)
apontar $((KX + CX - 104)) $((KY + CY - 88)) && sleep 0.2 && xdotool click 1
sleep 0.6
echo "  ($(grep -a "debug teclado:" "$T/shell.log" | grep -v "à vista" | tail -6 | tr '\n' ' ' | cut -c1-260))"
"$N" msg action focus-window --id "$(id_app kalango.editor)"
sleep 0.3
xdotool key --window "$W" ctrl+a; sleep 0.2; xdotool key --window "$W" ctrl+c
for _ in $(seq 20); do grep -aq "debug transferencia: conteúdo «reunião á" "$T/shell.log" && break; sleep 0.3; done
echo "  ($(grep -a "debug transferencia: conteúdo «" "$T/shell.log" | tail -1 | cut -c1-120))"
ok "Teclado na tela: o do KalangoOS digita no app em foco, completa a palavra e dá os acentos" \
  'grep -aq "debug teclado: sugestão «reunião»" "$T/shell.log" && grep -aq "debug transferencia: conteúdo «reunião á" "$T/shell.log"'
ok "Teclado na tela: o compositor avisa quando um campo de texto tem o foco (input-method-v2)" 'grep -aq "debug teclado: campo de texto em foco" "$T/shell.log"'
tecla "guardar"
for _ in $(seq 10); do grep -aq "debug teclado: guardado" "$T/shell.log" && break; sleep 0.3; done
ok "Teclado na tela: ▾ o guarda" 'grep -aq "debug teclado: guardado" "$T/shell.log"'
fechar_apps

# 92. The Shelf (design "Prateleira"): a new download shows the pile beside the dock; its panel lists it (and the
#     captures); a click opens it with its app (a PDF: the Documents app)
DL="$X/area/Downloads"
mkdir -p "$DL"
# (the pile's place after this download: an older line from earlier captures says where it was then)
N0=$(grep -ac "debug prateleira: novo «relatorio-final.pdf»" "$T/shell.log")
python3 "$ROOT/sessao/ferramentas/pdf-teste.py" "$DL/relatorio-final.pdf"
for _ in $(seq 40); do [ "$(grep -ac "debug prateleira: novo «relatorio-final.pdf»" "$T/shell.log")" -gt "$N0" ] && break; sleep 0.5; done
sleep 1
read -r PX PY < <(grep -a "debug prateleira: pilha em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
echo "  ($(grep -a "debug prateleira: à vista" "$T/shell.log" | tail -1); pilha em $PX $PY)"
ok "Prateleira: um download novo faz a pilha aparecer ao lado da dock" '[ -n "$PX" ]'
sem_avisos 2>/dev/null || sleep 4
[ -n "$PX" ] && apontar "$PX" "$PY" && sleep 0.3 && xdotool click 1
for _ in $(seq 20); do grep -aq "debug prateleira: aberta" "$T/shell.log" && break; sleep 0.3; done
sleep 0.8
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-prateleira.png" >/dev/null
ok "Prateleira: o painel abre com os recentes, o download primeiro" 'grep -a "debug prateleira: aberta" "$T/shell.log" | tail -1 | grep -Eq "\([1-9][0-9]* itens: relatorio-final.pdf"'
# Esc closes it (the dock takes the keyboard while it is open; left open, it kept the next panel's keys — run 51)
F0=$(grep -ac "debug prateleira: fechada" "$T/shell.log")
xdotool key --window "$W" Escape
for _ in $(seq 10); do [ "$(grep -ac "debug prateleira: fechada" "$T/shell.log")" -gt "$F0" ] && break; sleep 0.3; done
ok "Prateleira: Esc fecha o painel" '[ "$(grep -ac "debug prateleira: fechada" "$T/shell.log")" -gt "$F0" ]'
sleep 0.5

# 46. Clipboard history (design "Área de transferência (Super+V)"): every copy in the session comes in with the app
#     in use (through the compositor's data control); what a password manager marks as secret never does; the panel
#     opens over the window, ↵ pastes the chosen item into it (the clipboard takes it and the virtual keyboard presses
#     Ctrl+V: the editor's text copied back is the same); "Colar como… Texto limpo" pastes it without the marks;
#     Ctrl+P pins. (No screenshot in between: niri puts each one on the clipboard.)
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && break; sleep 0.5; done
sleep 2.5
copiar() { "$N" msg action spawn -- "$T/kalango-shell" transferencia copiar "$@"; sleep 1.2; }
copiar "Olá do teste de transferência"
copiar --segredo "senha-secreta-123"
copiar "#A5A1E2"
HIST="$X/state/kalango/area-de-transferencia/historico.json"
ok "Área de transferência: as cópias entram com o app em uso; o segredo de um gerenciador de senhas não" \
  'grep -aq "debug transferencia: copiado texto (Text) de Editor de Texto" "$T/shell.log" && grep -aq "debug transferencia: copiado texto (Color)" "$T/shell.log" && grep -aq "debug transferencia: um segredo ficou fora do histórico" "$T/shell.log" && ! grep -q "senha-secreta" "$HIST"'
TEC0=$(grep -ac "debug transferencia: com o teclado" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" transferencia
for _ in $(seq 20); do grep -aq "debug transferencia: aberta" "$T/shell.log" && break; sleep 0.3; done
for _ in $(seq 30); do [ "$(grep -ac "debug transferencia: com o teclado" "$T/shell.log")" -gt "$TEC0" ] && break; sleep 0.2; done
sleep 0.5
# Down until the second item is the chosen one (a key can arrive before the keyboard does, on a loaded machine)
for _ in 1 2 3; do xdotool key --window "$W" Down; sleep 0.5; grep -a "debug transferencia: escolhido" "$T/shell.log" | tail -1 | grep -q "escolhido 1$" && break; done
xdotool key --window "$W" Return
for _ in $(seq 20); do grep -aq "debug transferencia: colado Text" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
xdotool key --window "$W" ctrl+a ctrl+c
for _ in $(seq 20); do [ "$(grep -ac "debug transferencia: conteúdo «Olá do teste de transferência»" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
ok "Área de transferência: ↵ cola o item escolhido na janela em uso (o texto do editor volta igual)" \
  'grep -aq "debug transferencia: colado Text" "$T/shell.log" && [ "$(grep -ac "debug transferencia: conteúdo «Olá do teste de transferência»" "$T/shell.log")" -ge 2 ]'
copiar "**Negrito** e espaços"
"$N" msg action spawn -- "$T/kalango-shell" transferencia
for _ in $(seq 20); do [ "$(grep -ac "debug transferencia: aberta" "$T/shell.log")" -ge 2 ] && break; sleep 0.3; done
sleep 1
read -r FX FY < <(grep -a "debug central: transferencia forma limpo em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
apontar "${FX:-0}" "${FY:-0}"
sleep 0.3
xdotool click 1
for _ in $(seq 20); do grep -aq "debug transferencia: colar como limpo pronto" "$T/shell.log" && break; sleep 0.3; done
sleep 1.5
xdotool key --window "$W" ctrl+a ctrl+c
for _ in $(seq 20); do grep -aq "debug transferencia: conteúdo «Negrito e espaços»" "$T/shell.log" && break; sleep 0.3; done
ok "Área de transferência: \"Colar como… Texto limpo\" cola sem as marcas" 'grep -aq "debug transferencia: conteúdo «Negrito e espaços»" "$T/shell.log"'
"$N" msg action spawn -- "$T/kalango-shell" transferencia
sleep 1.5
xdotool key --window "$W" ctrl+p
for _ in $(seq 20); do grep -aq "debug transferencia: fixado" "$T/shell.log" && break; sleep 0.3; done
xdotool key --window "$W" Escape
ok "Área de transferência: Ctrl+P fixa o item (fica guardado)" 'grep -aq "debug transferencia: fixado" "$T/shell.log" && grep -q "\"pinned\": true" "$HIST"'
# (the pasted text is not saved: the Editor asks, and "Não salvar")
fechar_apps

# 47. Super alone (niri patch 0003; the nested session's Mod is Alt) and voice mode (design "Modo voz"): a tap opens the
#     search; held, the island listens (the test recorder "says" $KALANGO_TESTE_FALA) and on release the local model
#     transcribes it; another key joining the hold makes it a shortcut and the recording goes away
B0=$(grep -ac "debug busca: aberta" "$T/shell.log")
xdotool key --window "$W" alt
for _ in $(seq 20); do [ "$(grep -ac "debug busca: aberta" "$T/shell.log")" -gt "$B0" ] && break; sleep 0.3; done
ok "Super sozinho (um toque) abre a busca" '[ "$(grep -ac "debug busca: aberta" "$T/shell.log")" -gt "$B0" ]'
xdotool key --window "$W" Escape
sleep 0.8
xdotool keydown --window "$W" alt; sleep 1.5; xdotool keyup --window "$W" alt
for _ in $(seq 120); do grep -aq "debug voz: transcrito «" "$T/shell.log" && break; sleep 0.5; done
ok "segurar o Super é o modo voz: ouve e o modelo local transcreve" 'grep -aq "debug voz: ouvindo" "$T/shell.log" && grep -aq "debug voz: transcrito «" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.8
xdotool keydown --window "$W" alt; sleep 0.9; xdotool key --window "$W" F9; xdotool keyup --window "$W" alt
for _ in $(seq 20); do grep -aq "debug voz: cancelado (era um atalho)" "$T/shell.log" && break; sleep 0.3; done
ok "outra tecla junto do Super era um atalho: a gravação é descartada" 'grep -aq "debug voz: cancelado (era um atalho)" "$T/shell.log"'
sleep 0.5

# 24. login screen (the greetd greeter, against a stand-in greetd): another person is chosen by a click; a wrong
#     password is refused (greetd cancels the session); the right one starts the KalangoOS session and the greeter ends
LIBS="$ROOT/poc/.deps/root/usr/lib/x86_64-linux-gnu"
mkdir -p "$OUT/sessoes"
printf '[Desktop Entry]\nName=KalangoOS\nExec=/usr/libexec/kalango/kalango-sessao\nType=Application\n' > "$OUT/sessoes/kalango.desktop"
/usr/bin/python3 "$ROOT/sessao/ferramentas/greetd-teste.py" "$OUT/greetd.sock" "$OUT/greetd.log" &
GP=$!
sleep 0.5
"$N" msg action spawn -- sh -c "env GREETD_SOCK=$OUT/greetd.sock KALANGO_ENTRADA_TESTE=1 KALANGO_ENTRADA_SESSOES=$OUT/sessoes KALANGO_ENTRADA_PESSOAS='ana:Ana::,bruno:Bruno::,convidado:Convidado' XDG_STATE_HOME=$OUT/entrada KALANGO_PAPEL_DE_PAREDE=$ROOT/wallpapers/sdrustos-16x9-4k.png LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland $ROOT/target/release/kalango-entrada 2> $OUT/entrada.log"
for _ in $(seq 30); do grep -aq "debug entrada: bruno em" "$OUT/entrada.log" 2>/dev/null && break; sleep 0.3; done
read -r BX BY < <(grep -a "debug entrada: bruno em" "$OUT/entrada.log" | tail -1 | awk '{print $(NF-1), $NF}')
xdotool mousemove --window "$W" "$BX" "$BY"
sleep 0.3
xdotool click 1
sleep 1
ok "na tela de entrada, clicar numa pessoa a escolhe" 'grep -aq "debug entrada: escolhida bruno" "$OUT/entrada.log"'
xdotool type --window "$W" --delay 50 errada
xdotool key --window "$W" Return
sleep 1.5
ok "senha errada é recusada (o greetd cancela a sessão)" 'grep -aq "debug entrada: senha incorreta" "$OUT/entrada.log" && grep -q "cancel_session" "$OUT/greetd.log"'
xdotool type --window "$W" --delay 50 kalango-teste
xdotool key --window "$W" Return
for _ in $(seq 30); do pgrep -f "[t]arget/release/kalango-entrada" >/dev/null || break; sleep 0.2; done
ok "a senha certa inicia a sessão KalangoOS e a tela de entrada termina" 'grep -q "\"start_session\"" "$OUT/greetd.log" && grep -q "kalango-sessao" "$OUT/greetd.log" && grep -q "\"bruno\"" "$OUT/greetd.log" && ! pgrep -f "[t]arget/release/kalango-entrada" >/dev/null'
kill $GP 2>/dev/null

# 11. command bar: with an app focused, the bar opens with that window as context; typing searches
abrir_calc
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 2
"$N" msg focused-window > "$OUT/foco-comando.txt" 2>&1; "$N" msg windows >> "$OUT/foco-comando.txt" 2>&1
xdotool key --window "$W" alt+space
sleep 1.5
[ -n "${KALANGO_TESTE_CAPTURA:-}" ] && "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-comando.png" >/dev/null && sleep 0.5
ok "a barra de comando leva a janela ativa como contexto" 'grep -aq "debug contexto: janela [^ ]" "$T/shell.log"'
xdotool key --window "$W" Escape
sleep 0.6
"$N" msg action close-window
sleep 1

# captures (with KALANGO_TESTE_CAPTURA): the composer's context menu and the attach file dialog
if [ -n "${KALANGO_TESTE_CAPTURA:-}" ]; then
  xdotool key --window "$W" alt+g
  sleep 1.5
  xdotool mousemove --window "$W" $((OW - 200)) $((OH - 158)) click 3
  sleep 1.2
  "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-menu.png" >/dev/null
  xdotool key --window "$W" Escape
  sleep 0.6
  # (a click right after a menu may only close it)
  read -r ANX ANY < <(botao 0)
  xdotool mousemove --window "$W" "$ANX" $((ANY + 40)) click 1
  sleep 0.8
  grep -aq "debug anexar" "$T/shell.log" || xdotool mousemove --window "$W" "$ANX" $((ANY + 40)) click 1
  sleep 2.5
  "$N" msg action screenshot-screen --path "${KALANGO_TESTE_CAPTURA%.png}-arquivo.png" >/dev/null
  xdotool key --window "$W" Escape
  sleep 0.6
fi

# 65. Shutting down (design "Desligando"): last, as it closes the session's windows. Every app is asked to close: the
#     calculator goes, the editor with unsaved text asks to save and the screen says it waits for it — "Ver" cancels
#     and brings it forward; again, "Desligar mesmo assim" goes on (the test session only records it)
# (nothing an earlier section left open waits instead of the editor: run 46 waited for a Documents window)
fechar_apps
abrir_calc
abrir_ed
for _ in $(seq 40); do "$N" msg -j windows | grep -q '"os.kalango.Editor"' && "$N" msg -j windows | grep -q '"os.kalango.Calculadora"' && break; sleep 0.5; done
sleep 2.5
"$N" msg action focus-window --id "$(id_da_janela os.kalango.Editor)"
sleep 0.8
xdotool type --window "$W" --delay 30 "rascunho que ninguém salvou"
sleep 0.5
D0=$(grep -ac "debug desligando: esperando «" "$T/shell.log"); P0=$(grep -ac "teste\] .desligar. não foi executado" "$T/shell.log")
echo "  (janelas antes de desligar: $("$N" msg -j windows | python3 -c 'import json,sys; print(", ".join("%s %s" % (w["id"], w["app_id"]) for w in json.load(sys.stdin)))'))"
"$N" msg action spawn -- "$T/kalango-shell" energia desligar
for _ in $(seq 50); do [ "$(grep -ac "debug desligando: esperando «" "$T/shell.log")" -gt "$D0" ] && break; sleep 0.3; done
sleep 0.6
foto_x desligando
ok "Desligando: cada app é fechado (a calculadora vai) e o que pede para salvar aparece esperando você" \
  'grep -aq "debug desligando: [0-9]* janelas para fechar" "$T/shell.log" && grep -aiq "debug desligando: calculadora fechou" "$T/shell.log" && [ "$(grep -ac "debug desligando: esperando «" "$T/shell.log")" -gt "$D0" ] && [ "$(grep -ac "teste\] .desligar. não foi executado" "$T/shell.log")" = "$P0" ]'
read -r VX VY < <(grep -a "debug central: desligando ver em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$VX" "$VY"
for _ in $(seq 20); do grep -aq "debug desligando: cancelado" "$T/shell.log" && break; sleep 0.3; done
ok "Desligando: \"Ver\" cancela e traz o app que espera" 'grep -aq "debug desligando: cancelado" "$T/shell.log" && "$N" msg -j focused-window | grep -q "os.kalango.Editor"'
sleep 1.5
D1=$(grep -ac "debug desligando: esperando «" "$T/shell.log")
"$N" msg action spawn -- "$T/kalango-shell" energia desligar
for _ in $(seq 50); do [ "$(grep -ac "debug desligando: esperando «" "$T/shell.log")" -gt "$D1" ] && break; sleep 0.3; done
sleep 0.6
read -r FX FY < <(grep -a "debug central: desligando mesmo assim em" "$T/shell.log" | tail -1 | awk '{print $(NF-1), $NF}')
clicar "$FX" "$FY"
for _ in $(seq 20); do [ "$(grep -ac "teste\] .desligar. não foi executado" "$T/shell.log")" -gt "$P0" ] && break; sleep 0.3; done
ok "Desligando: \"Desligar mesmo assim\" segue sem esperar" 'grep -aq "debug desligando: desligando mesmo assim" "$T/shell.log" && [ "$(grep -ac "teste\] .desligar. não foi executado" "$T/shell.log")" -gt "$P0" ]'
fechar_apps

terminou=1
exit $fail
