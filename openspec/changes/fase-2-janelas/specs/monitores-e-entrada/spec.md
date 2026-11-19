# Spec Delta

## Purpose

Vários monitores, escala, gestos e modo toque.

## ADDED Requirements

### Requirement: Escala por monitor
Cada monitor MUST ter escala própria, inclusive fracionária, e as janelas MUST manter o tamanho físico ao mudar de monitor.

#### Scenario: Mover janela
- **WHEN** uma janela vai de um monitor 100% para um 125%
- **THEN** ela mantém o tamanho físico

### Requirement: Desconectar monitor
Ao desconectar um monitor, suas janelas MUST ir para os restantes e voltar ao reconectar.

#### Scenario: Notebook fechado
- **WHEN** o monitor externo é desconectado
- **THEN** as janelas aparecem no monitor do notebook

### Requirement: Gestos
Gestos de touchpad MUST seguir o dedo e completar pela velocidade.

#### Scenario: Trocar de espaço
- **WHEN** o usuário arrasta 3 dedos para o lado
- **THEN** o espaço desliza junto com os dedos

### Requirement: Modo toque
Em uso por toque, alvos MUST ter ao menos 48 px e a dock MUST virar barra de gesto.

#### Scenario: Tablet
- **WHEN** o teclado é destacado
- **THEN** a interface muda para o modo toque
