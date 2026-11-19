# Spec Delta

## Purpose

Execução das tarefas dos agentes, com modelos locais e de nuvem.

## ADDED Requirements

### Requirement: Tarefas com pausa
Cada tarefa MUST poder ser pausada, retomada e cancelada, e MUST continuar do ponto em que estava após reiniciar o computador.

#### Scenario: Desligar no meio
- **WHEN** o computador desliga durante uma tarefa
- **THEN** a tarefa retoma do passo em que parou

### Requirement: Local por padrão
O motor MUST usar o modelo local por padrão e MUST usar a nuvem só com permissão.

#### Scenario: Sem permissão de nuvem
- **WHEN** a nuvem não foi permitida
- **THEN** nenhum dado sai do computador

### Requirement: Orçamento
O uso de nuvem MUST respeitar o orçamento definido e avisar ao chegar a 80%.

#### Scenario: Limite
- **WHEN** o orçamento do mês acaba
- **THEN** o motor passa ao modelo local e avisa

### Requirement: Conteúdo externo é dado
Texto vindo de páginas, arquivos ou conectores MUST ser tratado como dado e MUST NOT ser executado como instrução.

#### Scenario: Instrução escondida
- **WHEN** uma página lida pede ao agente para enviar arquivos
- **THEN** o agente ignora, para a tarefa e avisa
