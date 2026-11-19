# Spec Delta

## Purpose

Os apps que todo usuário precisa no primeiro dia.

## ADDED Requirements

### Requirement: Conjunto mínimo
A instalação padrão MUST incluir Arquivos, Terminal, Editor de Texto, Imagens, Música, Vídeo, Documentos, Calculadora, Calendário, Monitor do sistema, Discos, Ajuda, Navegador, Configurações, Agentes e Central de Apps.

#### Scenario: Primeiro uso
- **WHEN** o usuário abre a grade de apps após instalar
- **THEN** todos os apps do conjunto estão presentes

### Requirement: Ajuda sem internet
A Ajuda MUST responder sem conexão, usando o manual instalado e o modelo local.

#### Scenario: Sem rede
- **WHEN** o usuário pergunta algo na Ajuda sem internet
- **THEN** uma resposta aparece com link para o painel certo

### Requirement: Monitor explica o pico
O Monitor do sistema MUST mostrar CPU, memória, GPU, rede e processos, e explicar em linguagem simples o que está pesando.

#### Scenario: GPU alta
- **WHEN** um modelo local usa a GPU
- **THEN** o Monitor indica qual agente e tarefa estão usando
