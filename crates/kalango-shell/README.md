# kalango-shell · o shell da sessão KalangoOS

Um processo, várias superfícies layer-shell sobre o niri (GTK4 em Rust):

- **Barra superior** (32 px, reserva espaço): espaços de trabalho pelo IPC do niri, com a pílula do ativo deslizando
  pela mola lenta; ilha com data e hora; pílula de agentes com os pedidos do kalangod (pulso elástico quando chega um);
  estado e menu de energia.
- **Dock** flutuante a 8 px da borda: apps fixados (os instalados, com os ícones do KalangoOS quando há equivalente) e
  abertos. Traço lilás = em foco, ponto = aberto. Hover sobe o ícone (mola rápida); abrir um app quica (elástica).
  Clicar foca a janela usada por último.
- **Área de trabalho**: relógio do design e os pedidos de agentes, entrando em cascata.
- **Barra de comando** (Super+Espaço; Super+A abre todos os apps): busca de apps e ações. Cascata de 24 ms, seleção
  deslizando (mola rápida).
- **Menu de energia** (ícone ou Super+Shift+E): bloquear, suspender, reiniciar e desligar pelo logind; sair pelo niri.
  Nasce do ícone tocado; a ação escolhida afunda o bloco e dissolve a tela.
- **Notificações**: o shell é o servidor `org.freedesktop.Notifications` da sessão. Banners de 380 px com ações,
  imagem, progresso, substituição e expiração; o mouse em cima segura o tempo.

Todo movimento usa `crates/kalango-movimento` (molas da página "11 · Motion" do Design System, só transformação e
opacidade, interrompível).

Comandos: `kalango-shell comando | apps | energia | sair` (falam com o shell em execução).
`KALANGO_SHELL_DEV=1`: sessão de teste, em que bloquear, suspender, reiniciar e desligar só são registrados e o
shell nunca toma o serviço de notificações de outro programa.

Ver ao vivo: `sessao/rodar.sh` (abre a sessão inteira numa janela, com recarga automática ao recompilar).
