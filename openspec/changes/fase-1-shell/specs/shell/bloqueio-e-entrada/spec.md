# Spec Delta

## Purpose

Bloqueio de tela e tela de entrada.

## ADDED Requirements

### Requirement: Bloqueio seguro
Com a tela bloqueada, nenhum conteúdo de janela MUST ser visível e nenhuma entrada MUST chegar aos apps.

#### Scenario: Bloquear
- **WHEN** o usuário pressiona Super+L
- **THEN** a tela de bloqueio cobre todos os monitores imediatamente

### Requirement: Entrada única
Uma única tela MUST servir para entrar, desbloquear e trocar de usuário.

#### Scenario: Trocar de usuário
- **WHEN** outro usuário é escolhido
- **THEN** a sessão anterior continua aberta e o campo de senha aparece

### Requirement: Agentes visíveis no bloqueio
A tela de bloqueio MUST indicar se há agentes trabalhando ou aguardando aprovação, sem mostrar conteúdo.

#### Scenario: Tarefa em andamento
- **WHEN** um agente está trabalhando com a tela bloqueada
- **THEN** o bloqueio mostra "1 agente trabalhando"
