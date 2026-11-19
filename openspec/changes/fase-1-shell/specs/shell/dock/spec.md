# Spec Delta

## Purpose

Uma única dock para abrir e alternar apps, em estilo flutuante ou fixo.

## ADDED Requirements

### Requirement: Dock única
O sistema MUST exibir uma única dock por vez; trocar de estilo MUST transformar a dock existente em vez de criar outra.

#### Scenario: Trocar de estilo
- **WHEN** o usuário troca de flutuante para fixa
- **THEN** a mesma dock se transforma, sem que duas apareçam

### Requirement: Apps fixados e abertos
A dock MUST mostrar apps fixados e abertos, com indicador de janelas abertas e foco.

#### Scenario: App aberto
- **WHEN** um app não fixado é aberto
- **THEN** ele aparece na dock com indicador até ser fechado

### Requirement: Menu do item
Cada item MUST oferecer nova janela, janelas abertas, fixar/desafixar e fechar todas.

#### Scenario: Desafixar
- **WHEN** o usuário desafixa um app fechado
- **THEN** o item sai da dock
