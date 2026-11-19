# Spec Delta

## Purpose

Controles rápidos ligados ao sistema real: rede, Bluetooth, som, brilho, energia e mídia.

## ADDED Requirements

### Requirement: Estado real
Cada controle MUST refletir o estado real do sistema e se atualizar quando ele mudar por outro caminho.

#### Scenario: Wi-Fi desligado por outro app
- **WHEN** a rede é desligada pela linha de comando
- **THEN** o controle de Wi-Fi mostra desligado em até 1 s

### Requirement: Wi-Fi
O usuário MUST poder ligar o Wi-Fi, escolher uma rede e informar a senha.

#### Scenario: Rede protegida
- **WHEN** o usuário escolhe uma rede WPA3
- **THEN** um diálogo pede a senha e conecta

### Requirement: Som por app
O usuário MUST poder ajustar o volume geral, de cada app e trocar a saída.

#### Scenario: Fone conectado
- **WHEN** um fone Bluetooth conecta
- **THEN** ele aparece como saída e pode ser escolhido

### Requirement: Energia
O usuário MUST poder ver a bateria e escolher entre Economia, Equilibrado e Desempenho.

#### Scenario: Desempenho
- **WHEN** o usuário escolhe Desempenho
- **THEN** o perfil de energia do sistema muda
