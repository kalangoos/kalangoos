# Spec Delta

## Purpose

Metas de consumo e fluidez que cada versão precisa cumprir.

## ADDED Requirements

### Requirement: Fluidez
Animações do shell e das janelas MUST manter 60 qps no hardware de referência, inclusive com NVIDIA.

#### Scenario: Abrir a barra
- **WHEN** a barra lateral abre num monitor 4K
- **THEN** nenhum quadro é perdido

### Requirement: Repouso
Com a sessão parada, o shell MUST NOT consumir CPU de forma contínua.

#### Scenario: Parado
- **WHEN** nada se move por 1 min
- **THEN** o uso de CPU do shell fica em 0%

### Requirement: Metas publicadas
Tempo de inicialização, memória em repouso e tempo de abertura dos apps próprios MUST ser medidos a cada versão contra as metas de docs/REQUISITOS.md.

#### Scenario: Meta estourada
- **WHEN** uma versão passa da meta de memória
- **THEN** a CI falha o build
