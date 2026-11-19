# Tasks

## 1. Sessão

- [x] 1.1a Arquivo de sessão (`sessao/dist/kalango.desktop`, `kalango-sessao`), configuração do niri gerada (`sessao/gerar.sh`, validada) e unidades do usuário (verificadas com systemd-analyze)
- [ ] 1.1 Escolher KalangoOS na tela de entrada abre a sessão sem erros no journal (precisa instalar na máquina)
- [x] 1.2 Shell supervisionado: kill -9 três vezes sob systemd (Restart=always, 200 ms) — processo novo em ~227 ms, barra, dock e área de volta em 407–433 ms
- [ ] 1.3 Encerrar sessão fecha apps e agentes de forma ordenada; teste com app aberto não perde dados

## 0. Movimento

- [x] 0.1 Molas do Design System em código (`crates/kalango-movimento`): mesmos parâmetros, interrompíveis, só transformação e opacidade, cascata de 24 ms, reduzir animações; testes conferem ζ e os tempos do design
- [x] 0.2 Coreografia aplicada: barra de comando desce da barra, menu de energia nasce do ícone, aprovação nasce da pílula de agentes, dock (hover rápida, quique elástico), espaços (lenta), notificações (entra suave, sai rápida, pilha desliza)

## 2. Barra superior e ilha

- [x] 2.1 Prova: barra superior em layer-shell com relógio, indicador de agentes e ícones de estado (poc/sidebar)
- [ ] 2.2 Barra reserva espaço (zona exclusiva) em todos os monitores; verificado com janela maximizada
- [ ] 2.3 Ilha com estados relógio, agente, música, aprovação, foco e voz; troca com mola; teste visual contra o design
- [ ] 2.4 Ilha expandida com até 3 atividades; clique fora recolhe; teste manual

## 3. Dock

- [ ] 3.1 Dock flutuante com apps fixados e abertos, indicadores e menu de contexto; teste com 12 apps
- [ ] 3.2 Dock fixa (barra de tarefas) com menu Iniciar e prévia de janela; troca ao vivo sem duas docks
- [ ] 3.3 Ocultar automaticamente e reaparecer na borda; teste com janela maximizada

## 4. Barras laterais

- [x] 4.1 Prova: barra lateral direita com modos Avisos, Controles e Agente, mola suave/rápida e cascata
- [x] 4.2 Prova: fechada, a barra só recebe o ponteiro na alça; cliques passam para as janelas
- [x] 4.3 Prova: troca de tema escuro/claro ao vivo pelos tokens
- [x] 4.4 Prova: relógio de quadros desliga em repouso
- [ ] 4.5 Barra lateral esquerda Hoje com os widgets reais; abre pela borda, gesto e Super+W
- [ ] 4.6 Fixar barra: perde margem e raio, reserva espaço; janelas se reorganizam
- [ ] 4.7 Barras em monitores de escalas diferentes; teste com 3440×1440 + 1080×1920

## 5. Notificações e indicadores

- [x] 5.1 Serviço org.freedesktop.Notifications no shell: `notify-send` mostra banner (380 px) no canto superior direito, com ações, imagem, progresso, substituição, expiração e sinais ActionInvoked/NotificationClosed
- [ ] 5.2 Agrupamento acima de 3 banners e central no modo Avisos; teste com 10 notificações
- [ ] 5.3 Não perturbe e resumo do agente respeitados; teste com regras por app
- [ ] 5.4 Indicadores de volume, brilho, Caps Lock e layout; somem após 1,2 s
- [ ] 5.5 Menus de contexto da área de trabalho, dock e barra de título no molde único

## 6. Área de trabalho

- [ ] 6.1 Papel de parede por monitor, com transição de 320 ms ao trocar
- [ ] 6.2 Widgets nos tamanhos P, M e G, arrastáveis, com modo de edição e galeria

## 7. Bloqueio e entrada

- [ ] 7.1 Bloqueio por ext-session-lock: nenhuma janela visível antes de desbloquear; teste de segurança registrado
- [ ] 7.2 Tela de entrada (greeter) com relógio, contas, senha e troca de usuário
- [ ] 7.3 Desbloqueio por impressão digital quando disponível

## 8. Barra de comando

- [ ] 8.1 Super abre a barra de comando; busca apps, arquivos e configurações em menos de 100 ms para 10 mil itens
- [x] 8.2 Pedido em linguagem natural encaminhado ao motor de agentes: a barra de comando oferece "Pedir ao agente" (Enter quando nada mais casa, ou Tab), que abre a barra lateral no modo Agente e envia o pedido ao LFM3 local

## 9. Controles do sistema

- [ ] 9.1 Wi-Fi via NetworkManager: listar, conectar, senha; teste numa rede real
- [ ] 9.2 Bluetooth via BlueZ: parear, conectar, bateria
- [ ] 9.3 Som via PipeWire: volume por saída e por app, troca de dispositivo
- [ ] 9.4 Brilho, modo de energia (power-profiles-daemon) e bateria (UPower)
- [ ] 9.5 Mídia via MPRIS na barra, na ilha e na barra lateral

## 10. Atalhos

- [ ] 10.1 Atalhos globais configuráveis; folha de atalhos ao segurar Super
