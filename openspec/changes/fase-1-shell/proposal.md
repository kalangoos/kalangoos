# Proposal

## Why

O shell é o que o usuário vê o tempo todo. Sem ele não há KalangoOS: hoje só existe uma prova de conceito da barra
superior e da barra lateral direita.

## What Changes

- Sessão gráfica própria que inicia sozinha sobre o compositor escolhido.
- Barra superior com a ilha de atividades; dock única nos estilos flutuante e fixa.
- Barras laterais do sistema: esquerda (Hoje) e direita (Avisos, Controles, Agente).
- Notificações, indicadores na tela, menus de contexto, papel de parede e widgets.
- Bloqueio de tela e tela de entrada; barra de comando e busca universal.
- Ligações reais com rede, Bluetooth, som, brilho, energia e mídia; atalhos globais.

Fora do escopo: janelas e espaços (fase 2), apps (fase 3), agentes de verdade (fase 4; aqui só a superfície).

## Capabilities

### New Capabilities
- `sessao`: início, encerramento e recuperação da sessão gráfica.
- `shell/barra-superior`: barra superior e ilha de atividades.
- `shell/dock`: dock única, flutuante ou fixa.
- `shell/barras-laterais`: barras laterais do sistema nas bordas da tela.
- `shell/notificacoes`: banners, central e regras de interrupção.
- `shell/indicadores`: indicadores na tela e menus de contexto.
- `shell/area-de-trabalho`: papel de parede e widgets.
- `shell/bloqueio-e-entrada`: bloqueio de tela e tela de entrada.
- `shell/barra-de-comando`: busca universal e pedidos ao agente.
- `controles-do-sistema`: rede, Bluetooth, som, brilho, energia e mídia ligados ao sistema real.
- `atalhos`: atalhos globais e folha de atalhos.

### Modified Capabilities
Nenhuma.

## Impact

Novos crates `kalango-shell` (superfícies) e serviços de integração; depende do compositor (fase 0) e do toolkit.
