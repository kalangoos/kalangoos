# Spec Delta

## Purpose

Avisos de apps, agentes e sistema, com controle de interrupção.

## ADDED Requirements

### Requirement: Padrão freedesktop
O sistema MUST aceitar notificações pelo padrão org.freedesktop.Notifications, incluindo ações.

#### Scenario: App comum
- **WHEN** um app envia uma notificação com ação "Responder"
- **THEN** um banner aparece com o botão Responder

### Requirement: Agrupamento
Com mais de 3 banners do mesmo app, eles MUST ser agrupados.

#### Scenario: Muitas mensagens
- **WHEN** um app envia 5 notificações seguidas
- **THEN** um grupo com contador aparece

### Requirement: Não perturbe
Com Não perturbe ativo, banners MUST NOT aparecer, exceto alarmes, alertas críticos e aprovações de risco alto.

#### Scenario: Mensagem comum
- **WHEN** chega uma mensagem com Não perturbe ativo
- **THEN** ela vai direto para a central

### Requirement: Resumo do agente
Quando o usuário ativar o resumo, avisos não urgentes MUST ser reunidos e entregues nos horários escolhidos.

#### Scenario: Horário do resumo
- **WHEN** chega o horário do resumo
- **THEN** um único aviso reúne os pendentes
