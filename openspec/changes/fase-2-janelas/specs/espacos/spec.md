# Spec Delta

## Purpose

Espaços por intenção que lembram janelas, agentes e regras de foco.

## ADDED Requirements

### Requirement: Espaço com intenção
Cada espaço MUST ter nome, cor, janelas, agentes ativos e regras de interrupção próprias.

#### Scenario: Espaço Desenvolver
- **WHEN** o usuário entra no espaço Desenvolver
- **THEN** só as notificações permitidas nele aparecem

### Requirement: Persistência
Espaços MUST lembrar janelas e posições entre sessões.

#### Scenario: Reiniciar
- **WHEN** o computador é reiniciado
- **THEN** cada espaço reabre os apps nas posições anteriores

### Requirement: Visão geral
A visão geral MUST mostrar as janelas do espaço atual e a fileira de espaços, e permitir mover janelas entre eles.

#### Scenario: Mover janela
- **WHEN** o usuário arrasta uma janela para outro espaço
- **THEN** ela passa a pertencer ao outro espaço
