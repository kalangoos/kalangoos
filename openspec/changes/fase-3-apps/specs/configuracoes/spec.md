# Spec Delta

## Purpose

O app Configurações: todos os ajustes do sistema num só lugar, com busca e pedido em linguagem natural.

## ADDED Requirements

### Requirement: Painéis
Configurações MUST oferecer os painéis Início, Rede, Bluetooth, Tela, Som, Energia, Notificações, Entrada, Aparência, Agentes e IA, Privacidade, Acessibilidade e Sistema.

#### Scenario: Abrir Tela
- **WHEN** o usuário escolhe Tela no trilho
- **THEN** o painel mostra os monitores e suas opções

### Requirement: Mudança imediata
Ajustes MUST valer na hora, sem botão Aplicar, exceto resolução e escala, que MUST pedir confirmação com retorno automático em 15 s.

#### Scenario: Resolução ruim
- **WHEN** o usuário muda a resolução e não confirma
- **THEN** a resolução anterior volta após 15 s

### Requirement: Buscar ou pedir
O campo de busca MUST encontrar qualquer ajuste pelo nome e aceitar pedidos como "deixe a tela mais quente à noite".

#### Scenario: Busca
- **WHEN** o usuário digita "luz noturna"
- **THEN** o ajuste aparece e abre ao ser escolhido
