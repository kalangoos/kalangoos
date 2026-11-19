# Spec Delta

## Purpose

O que o sistema lembra do usuário para ajudar melhor.

## ADDED Requirements

### Requirement: Local e criptografada
A memória MUST ficar só no computador, criptografada.

#### Scenario: Disco copiado
- **WHEN** o arquivo da memória é copiado para outra máquina
- **THEN** ele não pode ser lido

### Requirement: Legível e apagável
O usuário MUST poder ver, corrigir e apagar cada lembrança e a memória inteira.

#### Scenario: Esquecer
- **WHEN** o usuário apaga uma lembrança
- **THEN** ela some e não é mais usada

### Requirement: Modo anônimo
No modo anônimo nada da sessão MUST ser lembrado.

#### Scenario: Conversa anônima
- **WHEN** o usuário conversa no modo anônimo
- **THEN** nenhuma lembrança nova é criada
