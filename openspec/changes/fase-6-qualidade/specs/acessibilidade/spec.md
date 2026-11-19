# Spec Delta

## Purpose

Permitir o uso completo do KalangoOS por pessoas com deficiência visual, auditiva e motora.

## ADDED Requirements

### Requirement: Leitor de tela
Todo elemento do shell e dos apps próprios MUST ser anunciado pelo leitor de tela com nome, papel e estado.

#### Scenario: Interruptor
- **WHEN** o foco chega ao interruptor de Wi-Fi
- **THEN** o leitor anuncia "Wi-Fi, interruptor, ligado"

### Requirement: Teclado
Toda função MUST ser alcançável só pelo teclado, com foco visível.

#### Scenario: Barra lateral
- **WHEN** o usuário navega com Tab na barra lateral
- **THEN** o foco percorre todos os controles com contorno visível

### Requirement: Alto contraste e texto grande
O sistema MUST oferecer alto contraste e texto até 200% sem cortar conteúdo.

#### Scenario: Texto 200%
- **WHEN** o texto é ajustado para 200%
- **THEN** nenhum rótulo do shell aparece cortado

### Requirement: Legendas
O sistema MUST gerar legendas ao vivo para qualquer áudio, no modelo local.

#### Scenario: Vídeo sem legenda
- **WHEN** um vídeo sem legenda toca
- **THEN** legendas aparecem em tempo real
