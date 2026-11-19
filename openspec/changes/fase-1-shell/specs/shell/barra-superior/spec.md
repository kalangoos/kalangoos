# Spec Delta

## Purpose

Barra no topo da tela com espaços, relógio, ilha de atividades e estado do sistema.

## ADDED Requirements

### Requirement: Reserva de espaço
A barra superior MUST ocupar 32 px no topo de cada monitor e as janelas maximizadas MUST NOT cobri-la.

#### Scenario: Janela maximizada
- **WHEN** uma janela é maximizada
- **THEN** ela ocupa a área abaixo da barra

### Requirement: Ilha de atividades
A ilha MUST mostrar a atividade mais importante do momento (aprovação pendente, agente trabalhando, mídia, cronômetro ou relógio) e expandir num cartão ao clique.

#### Scenario: Aprovação pendente
- **WHEN** um agente aguarda aprovação e há música tocando
- **THEN** a ilha mostra a aprovação

#### Scenario: Expandir
- **WHEN** o usuário clica na ilha
- **THEN** um cartão mostra até 3 atividades com seus controles

### Requirement: Entradas para as barras laterais
O relógio MUST abrir a barra direita em Avisos, os ícones de estado em Controles e o indicador de agentes em Agente.

#### Scenario: Clique no relógio
- **WHEN** o usuário clica no relógio
- **THEN** a barra lateral direita abre no modo Avisos
