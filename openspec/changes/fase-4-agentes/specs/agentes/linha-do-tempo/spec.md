# Spec Delta

## Purpose

Registro de tudo o que os agentes fizeram, com desfazer.

## ADDED Requirements

### Requirement: Registro completo
Cada ação de agente MUST ser registrada com horário, agente, o que mudou, por quê e permissões usadas.

#### Scenario: Mover arquivos
- **WHEN** um agente move 118 arquivos
- **THEN** a linha do tempo mostra a ação com antes e depois

### Requirement: Registro independente
O registro MUST ser alimentado pela auditoria do sistema e MUST NOT depender do relato do agente.

#### Scenario: Agente omite
- **WHEN** um agente não relata uma escrita
- **THEN** a escrita aparece na linha do tempo mesmo assim

### Requirement: Desfazer
Ações reversíveis MUST poder ser desfeitas individualmente ou todas desde um momento escolhido.

#### Scenario: Desfazer
- **WHEN** o usuário desfaz a organização de Downloads
- **THEN** os 118 arquivos voltam aos lugares originais
