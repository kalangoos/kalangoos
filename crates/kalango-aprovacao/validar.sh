#!/bin/bash
# KalangoOS · valida a tela de aprovação de ponta a ponta, sem tocar a tela real: compositor Wayland em memória
# (sway headless), kalangod em modo de simulação exigindo aprovação presencial, um "agente" pedindo ações e uma
# "pessoa" teclando pelo protocolo de teclado virtual (wtype). Capturas em crates/kalango-aprovacao/capturas/.
# Como em produção, o kalangod roda como root e a tela é instalada com dono root (pede sudo para essas duas coisas).
# Rode como o seu usuário: ./validar.sh
set -u
cd "$(dirname "$0")"
HERE=$PWD; ROOT=$(realpath ../..)
R="$ROOT/poc/.deps/root"; LIBS="$R/usr/lib/x86_64-linux-gnu"
[ -x "$R/usr/bin/sway" ] && [ -f "$LIBS/libgtk4-layer-shell.so" ] || "$ROOT/poc/setup-deps.sh"
[ -x "$R/usr/bin/wtype" ] || (cd "$ROOT/poc/.deps" && mkdir -p dl && cd dl && apt-get download wtype && dpkg -x wtype_*.deb "$R")
export LD_LIBRARY_PATH="$LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" PATH="$R/usr/bin:$PATH"
WALL="$ROOT/wallpapers/sdrustos-16x9-4k.png"
(cd "$ROOT" && PKG_CONFIG_PATH="$LIBS/pkgconfig" cargo build --release -q -p kalango-aprovacao -p kalangod) || exit 1
OUT="$HERE/capturas"; mkdir -p "$OUT"; rm -f "$OUT"/*.png
SUDO=sudo; [ -n "${SUDO_ASKPASS:-}" ] && SUDO="sudo -A"
T=$(mktemp -d); chmod 755 "$T"
$SUDO install -d -o root -m 755 "$T/libexec"
$SUDO install -o root -m 755 "$ROOT/target/release/kalango-aprovacao" "$T/libexec/"
cat > "$T/kalangod.toml" <<CFG
socket = "$T/mcp.sock"
state_dir = "$T/estado"
protected_paths = []
approval_requires_tty = true
aprovadores_graficos = ["$T/libexec/kalango-aprovacao"]
approval_timeout_s = 20
total_grant_s = 900
dry_run = true
restore_root = "$T/raiz"
[[callers]]
name = "agente-de-teste"
max_level = "total"
allowed_paths = ["$T"]
CFG
mkdir -p "$T/raiz/etc"
$SUDO "$ROOT/target/release/kalangod" servir --config "$T/kalangod.toml" 2>"$T/kalangod.log" &
for i in $(seq 50); do [ -S "$T/mcp.sock" ] && break; sleep 0.1; done
export KALANGOD_SOCKET="$T/mcp.sock"
AGENTE="python3 -I $HERE/agente_teste.py $T/mcp.sock"
INNER="$T/dentro.sh"
cat > "$INNER" <<SH
#!/bin/bash
export KALANGOD_SOCKET="$T/mcp.sock"
KALANGO_APROVACAO_APP_ID=os.kalango.AprovacaoValidar KALANGO_APROVACAO_DEBUG=1 GSK_RENDERER=cairo env LD_PRELOAD=$LIBS/libgtk4-layer-shell.so GDK_BACKEND=wayland $T/libexec/kalango-aprovacao 2>$T/tela.log &
sleep 1.5
# Envia uma tecla. O wtype cria um teclado virtual novo a cada chamada e, às vezes, o compositor tira e devolve o
# foco nesse instante (aparece "foco: false" no log da tela) e a tecla se perde. Um teclado real não faz isso.
# Só nesse caso, que fica registrado, a tecla é reenviada.
tecla() {
  local antes=\$(grep -c "foco: false" $T/tela.log)
  wtype -M shift -m shift "\$@"; sleep 0.35
  if [ \$(grep -c "foco: false" $T/tela.log) != \$antes ]; then echo "reenvio (foco piscou)" >> $T/tela.log; sleep 0.4; wtype -M shift -m shift "\$@"; fi
}
# processo não "dumpable": /proc/PID/environ passa a ser do root e o mesmo usuário não lê
P=\$(pgrep -f "$T/libexec/kalango-aprovacao" | head -1)
if [ -z "\$P" ]; then echo x > $T/dumpable; elif cat /proc/\$P/environ >/dev/null 2>&1; then echo 1 > $T/dumpable; else echo 0 > $T/dumpable; fi
# 1. sistema: Ctrl+Enter aprova
$AGENTE pacotes.instalar '{"nomes":["cowsay","fortune-mod"]}' > $T/r1 &
sleep 1.4; grim "$OUT/1-pedido-sistema.png"
tecla -M ctrl -k Return -m ctrl; wait %2 2>/dev/null; sleep 0.3
# 2. Enter sozinho não aprova; Esc nega
# (o wtype cria um teclado virtual novo a cada chamada e o primeiro evento se perde: um Shift na frente absorve isso)
$AGENTE servicos.acao '{"nome":"cron.service","acao":"parar"}' > $T/r2 &
sleep 1.8; tecla -k Return; sleep 0.6; grim "$OUT/2-enter-nao-aprova.png"
tecla -k Escape; wait %3 2>/dev/null; sleep 0.3
# 3. total: Ctrl+Enter logo que aparece é ignorado (trava de 0,7 s); depois aprova
$AGENTE sistema.executar '{"programa":"apt-get","argumentos":["autoremove","-y"]}' > $T/r3 &
# duas tentativas precoces dentro da janela de 0,7 s (a primeira pode chegar antes de a tela ter o foco)
sleep 0.3; wtype -M shift -m shift -M ctrl -k Return -m ctrl; sleep 0.15; wtype -M shift -m shift -M ctrl -k Return -m ctrl; echo "cedo \$(date +%s.%N)" >> $T/tela.log; sleep 1.6; grim "$OUT/3-pedido-total.png"
tecla -M ctrl -k Return -m ctrl; wait %4 2>/dev/null; sleep 0.8
# 4. sem pedidos, a tela some
grim "$OUT/4-sem-pedidos.png"
swaymsg exit
SH
chmod +x "$INNER"
CONF="$T/sway.conf"
printf 'output * bg %s fill\noutput * resolution 1920x1080\ndefault_border none\nexec %s\n' "$WALL" "$INNER" > "$CONF"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 timeout 90 sway --unsupported-gpu -c "$CONF" 2>"$T/sway.log"
$SUDO pkill -f "kalangod servir --config $T/kalangod.toml"
fail=0
ok() { if eval "$2"; then echo "  ok   $1"; else echo "  FALHOU $1"; fail=1; fi; }
ok "Ctrl+Enter aprova o pedido de sistema"   'grep -q "\"isError\": false" $T/r1'
ok "Enter sozinho não aprova e Esc nega"       'grep -q "negou" $T/r2 && grep -q "Key(65307)" $T/tela.log'
ok "trava de 0,7 s e depois aprova o total"    'grep -q "\"isError\": false" $T/r3 && [ $(grep -c "acabou de aparecer" $T/tela.log) -ge 1 ]'
ok "a decisão veio da tela (sem terminal)"     '! grep -q "decisão recusada" $T/tela.log'
ok "a tela não pode ser inspecionada (dumpable=0)" '[ "$(cat $T/dumpable)" = "0" ]'
ok "capturas geradas"                          '[ -s "$OUT/1-pedido-sistema.png" ] && [ -s "$OUT/3-pedido-total.png" ] && [ -s "$OUT/4-sem-pedidos.png" ]'
[ $fail = 0 ] || { echo "--- kalangod:"; tail -5 $T/kalangod.log; echo "--- tela:"; tail -5 $T/tela.log; for f in r1 r2 r3; do echo "--- $f:"; head -c 400 $T/$f; echo; done; }
[ $fail = 0 ] && $SUDO rm -rf "$T" || echo "estado mantido em $T"
exit $fail
