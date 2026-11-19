# Spec Delta

## Purpose

Limites que o kernel impõe a agentes e conectores.

## ADDED Requirements

### Requirement: Recursos limitados
Cada tarefa MUST rodar com limite de CPU, memória e processos.

#### Scenario: Estouro de memória
- **WHEN** uma tarefa passa do limite de memória
- **THEN** só ela é encerrada e o usuário é avisado

### Requirement: Acesso a arquivos
Cada tarefa e conector MUST acessar apenas as pastas permitidas.

#### Scenario: Fora da pasta
- **WHEN** um conector tenta ler fora das pastas visíveis
- **THEN** o acesso é negado

### Requirement: Rede
Conectores locais MUST NOT acessar a rede externa sem permissão explícita.

#### Scenario: Conector de banco
- **WHEN** o conector Postgres tenta acessar a internet
- **THEN** a conexão é bloqueada
