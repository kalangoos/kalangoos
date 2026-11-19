# Spec Delta

## Purpose

Atalhos de teclado globais.

## ADDED Requirements

### Requirement: Atalhos padrão
O sistema MUST oferecer os atalhos padrão do Design System (Super, Super+Espaço, Super+L, Super+V, Alt+Tab, entre outros).

#### Scenario: Bloquear
- **WHEN** o usuário pressiona Super+L
- **THEN** a tela bloqueia

### Requirement: Personalizar
O usuário MUST poder mudar qualquer atalho e ser avisado de conflitos.

#### Scenario: Conflito
- **WHEN** o usuário atribui uma combinação já em uso
- **THEN** o sistema mostra o conflito e oferece substituir

### Requirement: Folha de atalhos
Segurar Super por 600 ms MUST mostrar os atalhos atuais do usuário.

#### Scenario: Segurar Super
- **WHEN** o usuário segura Super
- **THEN** a folha aparece e some ao soltar
