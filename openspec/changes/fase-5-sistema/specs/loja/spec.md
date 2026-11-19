# Spec Delta

## Purpose

Central de Apps para apps, agentes e modelos.

## ADDED Requirements

### Requirement: Três tipos
A loja MUST oferecer apps (Flatpak), agentes e modelos locais.

#### Scenario: Categoria Modelos
- **WHEN** o usuário abre Modelos
- **THEN** vê modelos com tamanho e uso de GPU

### Requirement: Permissões antes
Agentes MUST mostrar suas permissões antes de instalar e o usuário MUST poder reduzi-las.

#### Scenario: Reduzir
- **WHEN** o usuário desliga 'rede' antes de instalar um agente
- **THEN** o agente é instalado sem acesso à rede

### Requirement: Cabe na máquina
Modelos MUST indicar se cabem na memória da GPU disponível.

#### Scenario: Modelo grande
- **WHEN** um modelo de 24 GB numa GPU de 24 GB já ocupada
- **THEN** a loja avisa que não cabe junto
