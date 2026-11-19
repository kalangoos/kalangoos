# Spec Delta

## Purpose

Indicadores na tela e menus de contexto com um molde único.

## ADDED Requirements

### Requirement: Indicadores na tela
Mudanças de volume, brilho, microfone, Caps Lock, layout e modo avião MUST mostrar um indicador acima da dock que some 1,2 s após a última mudança, sem tirar o foco.

#### Scenario: Volume
- **WHEN** o usuário aperta a tecla de volume
- **THEN** o indicador mostra o nível e some sozinho

### Requirement: Menus de contexto
Os menus do sistema MUST seguir um molde: itens de 32 px, atalho à direita, ações de agente agrupadas e ação destrutiva por último.

#### Scenario: Menu de arquivo
- **WHEN** o usuário abre o menu de um arquivo
- **THEN** "Mover para a lixeira" é o último item
