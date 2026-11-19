# Proposal

## Why

Para ser uma distribuição de mercado, o KalangoOS precisa instalar, atualizar e se recuperar sozinho, com uma loja e
contas seguras.

## What Changes

- Imagem ISO com sessão de experimentar e instalador em 4 passos, dual boot e criptografia.
- Primeiro uso com importação de dados.
- Atualizações com ponto de restauração, canais e recuperação.
- Central de Apps para apps (Flatpak), agentes e modelos, com permissões antes de instalar.
- Contas, convidado, criptografia de disco com TPM, impressão digital e inicialização com a marca.

Fora do escopo: suporte comercial e versão para servidores.

## Capabilities

### New Capabilities
- `instalacao`: imagem, sessão de teste, instalador e primeiro uso.
- `atualizacao-e-restauracao`: atualizações, pontos de restauração e recuperação.
- `loja`: Central de Apps para apps, agentes e modelos.
- `contas-e-seguranca`: contas, criptografia, inicialização segura e desbloqueio.

### Modified Capabilities
Nenhuma.

## Impact

Ferramentas de imagem, instalador, repositórios de pacotes próprios, assinatura, infraestrutura de distribuição em
kalangoos.com.
