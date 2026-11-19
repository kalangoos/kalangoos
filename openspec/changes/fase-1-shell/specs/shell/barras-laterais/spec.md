# Spec Delta

## Purpose

Superfícies do sistema nas bordas da tela: Hoje à esquerda; Avisos, Controles e Agente à direita.

## ADDED Requirements

### Requirement: Barra esquerda Hoje
A barra esquerda MUST mostrar o resumo do dia e os widgets escolhidos pelo usuário.

#### Scenario: Abrir pela borda
- **WHEN** o ponteiro encosta na borda esquerda por 300 ms
- **THEN** a barra Hoje abre

### Requirement: Barra direita com três modos
A barra direita MUST ter os modos Avisos, Controles e Agente; trocar de modo MUST NOT fechar a barra.

#### Scenario: Trocar de modo
- **WHEN** a barra está em Controles e o usuário escolhe Agente
- **THEN** o conteúdo troca e a barra continua aberta

### Requirement: Sobre as janelas, sem cobrir a dock
Fechadas, as barras MUST deixar o ponteiro passar para as janelas, exceto numa alça de 12 px na borda; abertas, MUST NOT cobrir a dock.

#### Scenario: Clique perto da borda
- **WHEN** a barra está fechada e o usuário clica a 50 px da borda
- **THEN** o clique chega à janela de baixo

### Requirement: Fixar
Ao fixar uma barra, ela MUST reservar sua largura e as janelas MUST se reorganizar ao lado.

#### Scenario: Fixar em ultrawide
- **WHEN** o usuário fixa as duas barras numa tela de 3440 px
- **THEN** as janelas ocupam a área entre elas

### Requirement: Fechar
A barra MUST fechar com Esc, com o botão fechar ou ao clicar fora quando não estiver fixada.

#### Scenario: Esc
- **WHEN** a barra está aberta e o usuário pressiona Esc
- **THEN** a barra fecha com a mola rápida
