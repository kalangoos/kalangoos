# Sons do sistema do KalangoOS

Tema de sons freedesktop (`index.theme` + `stereo/*.oga`), compostos no LMMS por `tools/sons_sistema.py` do
projeto de vídeos (remotion). Mesma identidade do jingle: a caixinha de música do Organizador, vibrafone,
xilofone, piano elétrico e o motivo Kalango (Lá–Ré–Fá♯–Lá). Feitos só com sintetizadores do LMMS, sem samples.

Para instalar: copie esta pasta para `/usr/share/sounds/kalango` (ou `~/.local/share/sounds/kalango`) e use
`gsettings set org.gnome.desktop.sound theme-name kalango`, ou toque um som com `canberra-gtk-play -i <nome>`.

| Nome | Quando toca |
|---|---|
| `system-bootup` | Ligar o computador: o motivo Kalango inteiro, com pad |
| `desktop-login` | Entrar na sessão: o motivo Kalango, curto |
| `desktop-logout` | Sair da sessão: o motivo descendo |
| `system-shutdown` | Desligar: o motivo descendo, com pad |
| `message-new-instant` | Mensagem nova |
| `message` | Notificação comum |
| `dialog-information` | Informação |
| `dialog-question` | Pergunta do sistema |
| `dialog-warning` | Aviso |
| `dialog-error` | Erro |
| `complete` | Tarefa concluída |
| `bell` | Campainha (terminal) |
| `trash-empty` | Esvaziar a lixeira |
| `device-added` | Dispositivo conectado |
| `device-removed` | Dispositivo removido |
| `power-plug` | Carregador conectado |
| `power-unplug` | Carregador desconectado |
| `battery-low` | Bateria fraca |
| `screen-capture` | Captura de tela |
| `audio-volume-change` | Mudar o volume |
| `kalango-agent-request` | O agente pergunta algo (a pergunta do Organizador no vídeo) |
| `kalango-agent-approval` | O agente espera aprovação (ação arriscada) |
| `kalango-agent-approved` | Ação aprovada |
| `kalango-agent-denied` | Ação negada |
| `kalango-agent-done` | O agente terminou a tarefa |
| `kalango-agent-failed` | O agente falhou ou foi bloqueado |
| `kalango-undo` | Desfazer na Linha do tempo (rebobinar) |
| `kalango-voice-start` | Modo voz: segurar Super |
| `kalango-voice-stop` | Modo voz: soltar Super |
| `kalango-screen-lock` | Bloquear a tela |
| `kalango-screen-unlock` | Desbloquear a tela |
| `kalango-auth-failed` | Senha errada |
| `kalango-screen-record-start` | Começar a gravar a tela |
| `kalango-screen-record-stop` | Parar a gravação de tela |
| `alarm-clock-elapsed` | Alarme: fim do foco e lembretes |
| `kalango-agent-start` | O agente começa a trabalhar |
| `network-connectivity-established` | Rede conectada |
| `network-connectivity-lost` | Rede caiu |
| `kalango-remote-start` | Sessão remota começou |
| `kalango-remote-stop` | Sessão remota terminou |
| `kalango-update-ready` | Atualização pronta |

## Licença

CC0-1.0 (domínio público): compostos pelo projeto só com sintetizadores do LMMS, sem samples de terceiros. Veja o
`ATTRIBUTION.md` na raiz do repositório.
