#!/bin/sh
# The test's stand-in for Kalango Remote's plan panel (`kalango-remote agent-plan -`): the plan JSON that came on stdin
# is kept in $KALANGO_PLANO_DIR (plano-PID.json) and the decision is what the test wrote in $KALANGO_PLANO_DIR/decisao
# (approved, adjust, silent = exit 0 without any decision line; anything else, or nothing, cancels). The contract is the
# real one: a JSON line with the decision and exit status 0 (approved), 1 (cancelled) or 3 (adjust). A human click on
# the real panel is exercised by sessao/testar-agente-remoto.sh.
[ "$1" = agent-plan ] && [ "$2" = "-" ] || { echo "usage: agent-plan -" >&2; exit 2; }
cat > "$KALANGO_PLANO_DIR/plano-$$.json"
case "$(cat "$KALANGO_PLANO_DIR/decisao" 2>/dev/null)" in
  approved) echo '{"decision":"approved"}'; exit 0 ;;
  adjust) echo '{"decision":"adjust"}'; exit 3 ;;
  silent) exit 0 ;;
  *) echo '{"decision":"cancelled"}'; exit 1 ;;
esac
