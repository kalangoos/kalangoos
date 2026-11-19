# Spec Delta

## Purpose

Papel de parede e widgets na área de trabalho.

## ADDED Requirements

### Requirement: Papel de parede por monitor
O sistema MUST permitir um papel de parede por monitor e ajustar a imagem sem distorcer.

#### Scenario: Monitor vertical
- **WHEN** um monitor de 1080×1920 está conectado
- **THEN** o papel de parede o cobre sem esticar

### Requirement: Widgets
Widgets MUST poder ser adicionados, movidos e redimensionados (P, M, G) no modo de edição, e os mesmos widgets MUST poder aparecer na barra Hoje.

#### Scenario: Mover widget
- **WHEN** no modo de edição o usuário arrasta um widget
- **THEN** ele encaixa na grade de 184 px
