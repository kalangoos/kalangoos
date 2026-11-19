# Spec Delta

## Purpose

Aplica a aparência do KalangoOS aos apps, com ícones, cursor e sons próprios.

## ADDED Requirements

### Requirement: Apps no tema
Apps GTK e Qt MUST seguir as cores e o modo (claro/escuro) do sistema.

#### Scenario: Trocar o modo
- **WHEN** o usuário troca para o tema claro
- **THEN** apps GTK e Qt abertos mudam de cor

### Requirement: Ícones completos
Todo app, tipo de arquivo comum e dispositivo MUST ter ícone no estilo do sistema ou cair num ícone genérico do mesmo estilo.

#### Scenario: Tipo raro
- **WHEN** um arquivo de tipo raro é exibido
- **THEN** aparece o ícone genérico do estilo do sistema

### Requirement: Fontes com licença
Toda fonte distribuída MUST ter licença que permita redistribuição.

#### Scenario: Nova fonte
- **WHEN** uma fonte é adicionada à imagem
- **THEN** sua licença está registrada em docs/LICENCAS.md
