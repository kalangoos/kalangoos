#!/bin/bash
# KalangoOS · how long an app's window takes to show again after it was closed, in an isolated nested session: from the
# command reaching the shell to the window's first frame, opened again while still hidden (what the shell always did)
# and opened again after it let go of its surface (unrealized: GTK realizes it anew). Median of three for each.
#   ./sessao/medir-reabrir.sh [DISPLAY] [APPS…]     (default :88; every app)
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
DISP=${1:-88}
shift || true
OUT=$ROOT/target/medir-reabrir
rm -rf "$OUT"
mkdir -p "$OUT"
KALANGO_MEDIR_JANELAS=1 KALANGO_SOLTAR_JANELA_MS=1500 KALANGO_ISOLADA=$OUT/iso KALANGO_TESTE_DISPLAY=$DISP ./sessao/isolada.sh 1920x1080 > "$OUT/isolada.log" 2>&1 &
ISO=$!
trap 'kill "$ISO" 2>/dev/null; wait "$ISO" 2>/dev/null' EXIT
for _ in $(seq 150); do [ -s "$OUT/iso/env" ] && break; sleep 2; done
[ -s "$OUT/iso/env" ] || { echo "a sessão isolada não subiu (veja $OUT/isolada.log)"; exit 2; }
. "$OUT/iso/env"
for _ in $(seq 60); do grep -aq "pronto (" "$T/shell.log" && break; sleep 1; done
sleep 8
shell_cmd() { "$N" msg action spawn -- "$T/kalango-shell" "$@" >/dev/null; }
janela_de() { "$N" msg -j windows | python3 -c 'import json,sys; print(" ".join(str(w["id"]) for w in json.load(sys.stdin) if w.get("app_id") == sys.argv[1]))' "$1"; }
fechar() { for _ in 1 2 3; do for i in $(janela_de "$1"); do "$N" msg action close-window --id "$i" >/dev/null; done; sleep 0.8; [ -z "$(janela_de "$1")" ] && return 0; done; }
esperar_janela() { for _ in $(seq 150); do [ -n "$(janela_de "$1")" ] && return 0; sleep 0.1; done; }
# ms from the last "comando APP" to the next "APPID pintada", and how the window came back
medir() { python3 - "$T/shell.log" "$1" "$2" <<'PY'
import re, sys
log, cmd, wid = sys.argv[1:]
lines = open(log, errors='replace').read().splitlines()
start = None
for i, l in enumerate(lines):
    if re.match(r'debug medir: comando ' + re.escape(cmd) + r' t=\d+$', l):
        start = i
if start is None:
    print('- -'); sys.exit()
t0 = int(lines[start].rsplit('t=', 1)[1])
for l in lines[start:]:
    m = re.match(r'debug medir: ' + re.escape(wid) + r' pintada t=(\d+) (.*)', l)
    if m:
        print((int(m.group(1)) - t0) // 1000, m.group(2).replace(' ', '_')); break
else:
    print('- -')
PY
}
declare -A ID=( [calculadora]=Calculadora [ajuda]=Ajuda [discos]=Discos [loja]=Apps [calendario]=Calendario [musica]=Musica
  [video]=Video [imagens]=Imagens [arquivos]=Arquivos [central]=Agentes [configuracoes]=Configuracoes [documentos]=Documentos
  [editor]=Editor [terminal]=Terminal [monitor]=Monitor )
APPS=("${@:-calculadora ajuda discos loja calendario musica video imagens arquivos central configuracoes documentos editor terminal monitor}")
read -r -a APPS <<< "${APPS[*]}"
mediana() { tr ' ' '\n' | grep -v '^-$' | sort -n | awk '{a[NR]=$1} END {print (NR ? a[int((NR+1)/2)] : "-")}'; }
printf '  %-15s %20s %22s\n' app "escondida (ms)" "desrealizada (ms)"
for app in "${APPS[@]}"; do
  id=os.kalango.${ID[$app]}
  # (each round starts with the window open and realized)
  shell_cmd "$app"; esperar_janela "$id"; sleep 3
  E=""; D=""; MODO=""
  for _ in 1 2 3; do
    shell_cmd estado prazo 4294967295; sleep 0.3
    fechar "$id"; sleep 1.5
    shell_cmd "$app"; esperar_janela "$id"; sleep 1.5
    read -r ms how < <(medir "$app" "$id"); E="$E $ms"
    shell_cmd estado prazo 1500; sleep 0.3
    fechar "$id"; sleep 4
    shell_cmd "$app"; esperar_janela "$id"; sleep 1.5
    read -r ms how < <(medir "$app" "$id"); D="$D $ms"; MODO="$MODO $how"
  done
  fechar "$id"; sleep 1
  printf '  %-15s %20s %22s   (%s)\n' "$app" "$(echo $E | mediana)" "$(echo $D | mediana)" "$(echo $MODO | tr ' ' '\n' | sort | uniq -c | tr '\n' ' ')"
done
