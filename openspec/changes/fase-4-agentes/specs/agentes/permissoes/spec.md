# Spec Delta

## Purpose

Quanto cada agente pode fazer sem perguntar.

## ADDED Requirements

### Requirement: Três níveis
Para cada agente e ferramenta, a permissão MUST ser livre, perguntar ou nunca.

#### Scenario: Nunca
- **WHEN** a ferramenta "apagar" está em nunca
- **THEN** o agente não consegue usá-la nem pedir

### Requirement: Vale na hora
Mudar uma permissão MUST valer imediatamente, inclusive para tarefas em andamento.

#### Scenario: Revogar no meio
- **WHEN** o usuário revoga escrita durante uma tarefa
- **THEN** a próxima escrita é bloqueada

### Requirement: Aprovação do sistema
Pedidos de aprovação MUST ser desenhados pelo sistema e mostrar agente, ação, comandos e escopo.

#### Scenario: Apagar duplicados
- **WHEN** um agente pede para apagar 3 arquivos
- **THEN** o diálogo lista os 3 caminhos

### Requirement: Pastas protegidas
Pastas marcadas como protegidas MUST ser inacessíveis a qualquer agente, em qualquer nível.

#### Scenario: ~/.ssh
- **WHEN** um agente no nível total tenta ler ~/.ssh
- **THEN** o kernel bloqueia e a tentativa é registrada
