# Spec Delta

## Purpose

Conectores MCP que dão ferramentas de outros apps e serviços aos agentes.

## ADDED Requirements

### Requirement: Local e remoto
O sistema MUST aceitar conectores locais (comando) e remotos (URL com OAuth).

#### Scenario: GitHub
- **WHEN** o usuário adiciona o conector remoto do GitHub
- **THEN** o login OAuth acontece no navegador e as ferramentas aparecem

### Requirement: Teste antes
Nenhum conector MUST ser adicionado sem um teste isolado que liste suas ferramentas.

#### Scenario: Teste falha
- **WHEN** o conector não inicia no teste
- **THEN** o botão adicionar fica desativado e o erro é mostrado

### Requirement: Níveis sugeridos
Cada ferramenta MUST receber um nível sugerido pelo tipo: leitura livre, escrita perguntar, destrutiva nunca.

#### Scenario: drop_table
- **WHEN** o conector oferece uma ferramenta destrutiva
- **THEN** ela entra como nunca

### Requirement: Segredos no cofre
Senhas e tokens dos conectores MUST ficar no cofre do sistema, nunca no arquivo de configuração.

#### Scenario: Arquivo de configuração
- **WHEN** o usuário abre mcp.json
- **THEN** no lugar da senha há só uma referência ao cofre
