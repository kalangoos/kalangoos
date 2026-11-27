#!/bin/bash
# Os testes do portão de confirmação do Kalango Mind que precisam de um host de verdade (sessão com /run/user/UID, systemd de
# usuário, cgroup delegado). Rode numa sessão de usuário (não como root) da VM ou do computador de desenvolvimento.
# Com KALANGOD_REQUIRE_HOST_TESTS=1, faltar o host é FALHA, não passagem silenciosa. O CI não tem esse host: estes testes só
# são provados aqui. Uso: crates/kalangod/vm/portao-mind.sh
set -eu
cd "$(dirname "$(realpath "$0")")/../../.."
KALANGOD_REQUIRE_HOST_TESTS=1 cargo test -p kalangod --locked -- --include-ignored \
  a_command_cannot_read_the_mind_confirmation_codes_outside_its_home \
  a_connector_cannot_read_the_mind_confirmation_codes \
  a_command_cannot_move_itself_to_another_cgroup \
  a_command_cannot_ask_the_user_manager_to_start_a_unit
