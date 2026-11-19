# Spec Delta

## Purpose

Tradução da interface e formatos regionais.

## ADDED Requirements

### Requirement: Idiomas da 1.0
A versão 1.0 MUST estar completa em português do Brasil e inglês.

#### Scenario: Inglês
- **WHEN** o idioma é inglês
- **THEN** nenhum texto do sistema aparece em português

### Requirement: Formatos regionais
Datas, números e moedas MUST seguir a região escolhida.

#### Scenario: Brasil
- **WHEN** a região é Brasil
- **THEN** valores aparecem como R$ 1.234,56
