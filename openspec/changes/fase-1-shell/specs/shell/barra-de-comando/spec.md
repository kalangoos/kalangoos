# Spec Delta

## Purpose

Ponto único para buscar e pedir: apps, arquivos, configurações e agentes.

## ADDED Requirements

### Requirement: Abrir
A tecla Super MUST abrir a barra de comando sobre qualquer janela.

#### Scenario: Super
- **WHEN** o usuário toca Super
- **THEN** a barra abre com o cursor no campo

### Requirement: Busca rápida
Resultados de apps, arquivos e configurações MUST aparecer em até 100 ms enquanto o usuário digita.

#### Scenario: Busca por app
- **WHEN** o usuário digita "term"
- **THEN** Terminal aparece como primeiro resultado

### Requirement: Pedido ao agente
Um texto que não seja busca MUST poder ser enviado ao agente como pedido.

#### Scenario: Pedido
- **WHEN** o usuário escreve "organiza meus downloads" e confirma
- **THEN** o pedido vai para o agente padrão
