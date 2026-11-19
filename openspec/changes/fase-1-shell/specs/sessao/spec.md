# Spec Delta

## Purpose

Garante que a sessão gráfica do KalangoOS inicie, encerre e se recupere de falhas sem perder o trabalho do usuário.

## ADDED Requirements

### Requirement: Iniciar a sessão
A sessão KalangoOS MUST iniciar o compositor, o shell e os serviços do usuário a partir da tela de entrada, sem intervenção.

#### Scenario: Login
- **WHEN** o usuário entra com a senha
- **THEN** a área de trabalho aparece com barra superior e dock em menos de 3 s num SSD

### Requirement: Recuperar o shell
Se o processo do shell terminar de forma inesperada, o sistema MUST reiniciá-lo em menos de 1 s sem fechar as janelas abertas.

#### Scenario: Falha do shell
- **WHEN** o processo do shell é encerrado à força
- **THEN** a barra e a dock voltam e as janelas continuam abertas

### Requirement: Encerrar com ordem
Ao encerrar a sessão, o sistema MUST pedir aos apps que salvem, pausar os agentes no passo atual e só então desligar.

#### Scenario: Documento não salvo
- **WHEN** o usuário encerra a sessão com um documento não salvo
- **THEN** o app pede para salvar antes de fechar
