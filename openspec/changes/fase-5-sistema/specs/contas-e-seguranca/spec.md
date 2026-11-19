# Spec Delta

## Purpose

Contas de usuário e proteção do computador.

## ADDED Requirements

### Requirement: Convidado
A conta de convidado MUST apagar tudo ao sair e MUST NOT ter agentes.

#### Scenario: Sair do convidado
- **WHEN** o convidado encerra a sessão
- **THEN** nenhum arquivo dele permanece

### Requirement: Secure Boot
O sistema MUST iniciar com Secure Boot ativo, inclusive com o driver NVIDIA.

#### Scenario: Secure Boot ligado
- **WHEN** o computador inicia com Secure Boot
- **THEN** a sessão abre acelerada

### Requirement: Pacotes assinados
Pacotes do KalangoOS MUST ser assinados e verificados antes de instalar.

#### Scenario: Pacote adulterado
- **WHEN** um pacote tem a assinatura inválida
- **THEN** a instalação é recusada
