# kalango-aprovacao · tela de aprovação do KalangoOS

Mostra cada pedido de agente que precisa da sua aprovação (níveis sistema e total do `kalangod`), numa superfície do
sistema que nenhum agente desenha. Detalhes de segurança: `crates/kalangod/README.md`, seção "Tela de aprovação".

- `./ver.sh`: abre ao vivo numa janela do niri, com um agente de demonstração pedindo ações (tudo simulado).
- `./validar.sh`: validação de ponta a ponta num compositor em memória, com capturas em `capturas/`. Pede `sudo`,
  porque o kalangod roda como root e a tela é instalada como root, igual em produção.
- `KALANGO_APROVACAO_DEMO=1`: mostra um pedido de exemplo sem `kalangod`, útil para design.

Compilar: `PKG_CONFIG_PATH=poc/.deps/root/usr/lib/x86_64-linux-gnu/pkgconfig cargo build --release -p kalango-aprovacao`
(o `gtk4-layer-shell` vem de `poc/setup-deps.sh`). Em produção: `/usr/libexec/kalango/kalango-aprovacao`, de dono root,
iniciado com a sessão.
