# Spec Delta

## Purpose

Deixa visível quem usa microfone, câmera, tela e dados, e permite parar tudo.

## ADDED Requirements

### Requirement: Indicadores
Enquanto microfone, câmera, compartilhamento de tela ou leitura por agente estiverem ativos, um indicador MUST ficar visível na barra e nenhum app MUST conseguir escondê-lo.

#### Scenario: Microfone
- **WHEN** um app usa o microfone
- **THEN** um ponto coral aparece na barra

### Requirement: Pausar tudo
Super+Shift+P MUST cortar sensores e pausar todos os agentes de uma vez.

#### Scenario: Pausa
- **WHEN** o usuário pressiona Super+Shift+P
- **THEN** câmera e microfone desligam e os agentes param

### Requirement: Saída de dados
O sistema MUST mostrar quanto dado saiu para a nuvem e para onde.

#### Scenario: Relatório do dia
- **WHEN** o usuário abre Privacidade
- **THEN** vê os KB enviados por serviço
