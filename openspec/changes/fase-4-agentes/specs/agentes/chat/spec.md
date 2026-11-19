# Spec Delta

## Purpose

A conversa com os agentes e os conteúdos que ela consegue exibir.

## ADDED Requirements

### Requirement: Conteúdo rico
O chat MUST exibir texto formatado, código com realce, diff, saída de terminal, tabelas, gráficos, diagramas Mermaid, fórmulas LaTeX, áudio, vídeo, imagens e arquivos.

#### Scenario: Diagrama
- **WHEN** a resposta contém um bloco Mermaid
- **THEN** o diagrama aparece desenhado, com opção de ver o código

### Requirement: Prévia de HTML isolada
Páginas HTML geradas MUST rodar isoladas, sem acesso a arquivos nem à rede local.

#### Scenario: Página tenta ler arquivo
- **WHEN** um HTML gerado tenta abrir file:///
- **THEN** o acesso é bloqueado

### Requirement: Navegador do agente
O agente MUST navegar numa sessão separada dos logins do usuário, visível, e o usuário MUST poder assumir o controle a qualquer momento.

#### Scenario: Assumir
- **WHEN** o usuário clica em Assumir o controle
- **THEN** o ponteiro do agente para e o do usuário age na página

### Requirement: Mudanças com aceite
Edições de arquivo propostas pelo agente MUST ser mostradas como diff e só aplicadas após aceite.

#### Scenario: Rejeitar
- **WHEN** o usuário rejeita um diff
- **THEN** o arquivo não muda
