# Aparência

## Purpose

Define a linguagem visual do KalangoOS: temas claro e escuro, tokens de cor, tipografia, formas e as regras que toda superfície do sistema segue.

## Requirements

### Requirement: Temas claro e escuro
O sistema MUST oferecer um tema escuro e um tema claro, e toda cor de interface MUST vir de um token com valor definido para os dois modos.

#### Scenario: Trocar de tema
- **WHEN** o usuário troca o tema em Configurações
- **THEN** todas as superfícies do sistema mudam de cor sem reiniciar a sessão

#### Scenario: Cor sem token
- **WHEN** uma superfície usa uma cor que não está entre os tokens
- **THEN** a revisão visual do design a reporta como defeito

### Requirement: Tokens exportados para código
Os tokens do Design System MUST ser exportados para os formatos usados pelo código (CSS do toolkit e constantes), a partir de uma única fonte.

#### Scenario: Mudança de cor no design
- **WHEN** um token muda de valor no Design System e a exportação é refeita
- **THEN** o código passa a usar o valor novo sem edição manual

### Requirement: Superfícies sem sombra
As superfícies do sistema MUST NOT usar sombras; a profundidade MUST vir de camadas de cor e bordas finas.

#### Scenario: Painel flutuante
- **WHEN** um painel flutuante é exibido sobre a área de trabalho
- **THEN** ele se destaca por cor de fundo e borda de 1 px, sem sombra

### Requirement: Cantos sutis
Os raios de canto MUST seguir a escala 8 (janelas e painéis), 6 (cartões e controles grandes) e 4 (controles); formas circulares MUST ficar restritas a interruptores, pontos e avatares.

#### Scenario: Novo componente
- **WHEN** um componente novo é desenhado
- **THEN** seus cantos usam um valor da escala

### Requirement: Tipografia
O texto de interface MUST usar a família de interface do tema ativo (Inter no escuro, Roboto no claro) e títulos de destaque MUST usar a família de exibição do tema.

#### Scenario: Tema claro
- **WHEN** o tema claro está ativo
- **THEN** textos usam Roboto e títulos de destaque usam Ubuntu Bold

### Requirement: Tema claro com a paleta definida
O tema claro MUST usar fundo #F0F0F5, blocos #FFFFFF, títulos #322153, texto #6C6C80 e verde #34CB79 como destaque.

#### Scenario: Bloco no tema claro
- **WHEN** um cartão é exibido no tema claro
- **THEN** seu fundo é #FFFFFF sobre o fundo #F0F0F5
