# Spec Delta

## Purpose

Captura, gravação e compartilhamento de tela sob controle do usuário.

## ADDED Requirements

### Requirement: Escolha do usuário
Nenhum app MUST capturar a tela sem o usuário escolher o que compartilhar num diálogo do sistema.

#### Scenario: Videochamada
- **WHEN** um navegador pede para compartilhar a tela
- **THEN** o sistema mostra o seletor de tela, janela ou região

### Requirement: Indicador
Durante captura ou compartilhamento, um indicador MUST ficar visível na barra com opção de parar.

#### Scenario: Compartilhando
- **WHEN** uma janela está sendo compartilhada
- **THEN** um indicador azul aparece na barra

### Requirement: Captura rápida
Print MUST abrir a ferramenta de captura com tela, janela, região e gravação.

#### Scenario: Print
- **WHEN** o usuário pressiona Print
- **THEN** a barra de captura aparece
