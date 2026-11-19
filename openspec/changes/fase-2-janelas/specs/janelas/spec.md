# Spec Delta

## Purpose

Como as janelas aparecem, se movem e se organizam.

## ADDED Requirements

### Requirement: Moldura do sistema
Janelas sem decoração própria MUST receber a moldura do KalangoOS com fechar, maximizar e minimizar à direita.

#### Scenario: App sem decoração
- **WHEN** um app pede decoração do servidor
- **THEN** ele aparece com a moldura do sistema

### Requirement: Estado ativo
A janela ativa MUST ser distinguível das inativas por contorno e contraste, sem sombra.

#### Scenario: Trocar de janela
- **WHEN** o usuário clica numa janela inativa
- **THEN** ela passa a mostrar o contorno de ativa

### Requirement: Animações de janela
Abrir, fechar, minimizar e maximizar MUST usar as molas do sistema e manter 60 qps no hardware suportado.

#### Scenario: Abrir app
- **WHEN** o usuário abre um app
- **THEN** a janela entra com a mola suave sem quadros perdidos

### Requirement: Encaixe
Arrastar uma janela até a borda MUST mostrar a zona de encaixe e, ao soltar, encaixá-la; Super+setas MUST fazer o mesmo pelo teclado.

#### Scenario: Metade esquerda
- **WHEN** o usuário arrasta uma janela até a borda esquerda e solta
- **THEN** ela ocupa a metade esquerda da área útil

### Requirement: Alternar janelas
Alt+Tab MUST mostrar as janelas do espaço atual com miniaturas ao vivo, em ordem de uso.

#### Scenario: Alt+Tab
- **WHEN** o usuário pressiona Alt+Tab uma vez
- **THEN** o foco vai para a janela usada antes

### Requirement: Apps X11
Apps que só funcionam em X11 MUST abrir e funcionar dentro da sessão.

#### Scenario: App antigo
- **WHEN** o usuário abre um app só X11
- **THEN** ele abre normalmente
