# Movimento

## Purpose

Define como as superfícies do KalangoOS se movem: molas físicas em vez de durações fixas, coreografia e redução de movimento.

## Requirements

### Requirement: Molas no lugar de durações
Toda transição do sistema MUST ser uma mola amortecida com os parâmetros do Design System: rápida (520/38), suave (300/30), elástica (380/20) e lenta (180/26), massa 1.

#### Scenario: Abrir um painel
- **WHEN** um painel do sistema abre
- **THEN** ele usa a mola suave e chega a 96% do caminho em cerca de 210 ms

### Requirement: Interrupção
Uma animação em andamento MUST poder ser revertida a partir da posição e velocidade atuais, sem saltos.

#### Scenario: Fechar durante a abertura
- **WHEN** o usuário fecha um painel que ainda está abrindo
- **THEN** o painel inverte o movimento do ponto em que está

### Requirement: Cascata
Blocos que entram juntos MUST entrar em cascata, com 24 ms entre um e outro.

#### Scenario: Conteúdo de um painel
- **WHEN** um painel com seis blocos abre
- **THEN** cada bloco aparece 24 ms depois do anterior

### Requirement: Gestos seguem o dedo
Transições disparadas por gesto MUST acompanhar o dedo 1:1 e completar com a velocidade do gesto ao soltar.

#### Scenario: Visão geral por gesto
- **WHEN** o usuário arrasta três dedos para cima e solta no meio
- **THEN** a visão geral termina de abrir ou volta, conforme a velocidade

### Requirement: Reduzir animações
Com "reduzir animações" ativo, toda mola MUST ser substituída por um esmaecimento de até 90 ms, sem deslocamento nem escala.

#### Scenario: Preferência ativa
- **WHEN** a preferência está ativa e um painel abre
- **THEN** o painel aparece por esmaecimento, sem deslizar

### Requirement: Repouso sem custo
Uma superfície parada MUST NOT redesenhar a cada quadro.

#### Scenario: Barra aberta e parada
- **WHEN** a barra lateral está aberta e nada se move
- **THEN** o processo não consome CPU de animação
