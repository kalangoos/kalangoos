# Marca

## Purpose

Define a identidade do KalangoOS: nome, mascote, logo, ícones dos apps próprios e regras de uso.

## Requirements

### Requirement: Nome
O sistema MUST ser apresentado como KalangoOS em todas as telas, documentos e arquivos voltados ao usuário.

#### Scenario: Tela Sobre
- **WHEN** o usuário abre Configurações › Sistema › Sobre
- **THEN** o nome exibido é KalangoOS

### Requirement: Logo em versões
O logo MUST existir em versão principal, símbolo, ícone, horizontal, só nome, uma cor e tema claro, em PNG e SVG, usando só a paleta do Design System.

#### Scenario: Fundo claro
- **WHEN** o logo é usado sobre fundo claro
- **THEN** a versão do tema claro ou a de uma cor escura é a usada

### Requirement: Ícones dos apps próprios
Cada app próprio MUST ter um ícone no estilo do mascote: contorno carvão grosso, cores chapadas da paleta, sem fundo.

#### Scenario: App novo
- **WHEN** um app próprio é criado
- **THEN** ele ganha um ícone no mesmo estilo antes de ser publicado

### Requirement: Legibilidade pequena
O ícone do sistema MUST permanecer reconhecível a partir de 24 px.

#### Scenario: Barra de janela
- **WHEN** o ícone aparece a 24 px
- **THEN** o mascote continua identificável
