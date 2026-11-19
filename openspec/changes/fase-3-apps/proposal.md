# Proposal

## Why

Um usuário comum precisa fazer as tarefas do dia a dia sem cair em telas de outro sistema. Hoje os apps existem só
no design.

## What Changes

- Tema do sistema para os toolkits, gerado dos tokens; tema de ícones completo, cursor e sons.
- Configurações com os 20 painéis desenhados ligados ao sistema real.
- Diálogos do sistema pelos portais: arquivos, impressão, permissões, senha de administrador.
- Apps essenciais: Arquivos, Terminal, Editor, Imagens, Música, Vídeo, Documentos, Calculadora, Calendário,
  Monitor, Discos e Ajuda (próprios ou existentes com o nosso tema).

Fora do escopo: o motor de agentes (fase 4); a loja (fase 5).

## Capabilities

### New Capabilities
- `tema-e-icones`: tema dos toolkits, ícones, cursor, sons e fontes.
- `configuracoes`: o app Configurações e seus painéis.
- `dialogos-do-sistema`: diálogos e portais do sistema.
- `apps-base`: os apps essenciais e o que cada um precisa fazer.

### Modified Capabilities
Nenhuma.

## Impact

Novos crates de apps; pacotes de tema; xdg-desktop-portal próprio para os diálogos.
