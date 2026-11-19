# Proposal

## Why

O KalangoOS tem design, marca e uma prova de conceito, mas não tem as bases para começar o produto: decisões
técnicas fechadas, repositório organizado, ferramentas de build e um ambiente de desenvolvimento reproduzível.

## What Changes

- Fechar as decisões de arquitetura que travam o resto (compositor, toolkit, escopo da 0.1, restauração).
- Renomear o legado (SDRustOS, Lunar) para KalangoOS.
- Organizar o código como workspace Cargo e trocar os scripts Python por um único binário `kalango` em Rust.
- Exportador de tokens em Rust e integração contínua.
- Ambiente de desenvolvimento que sobe a sessão numa janela ou máquina virtual, inclusive com NVIDIA.

Fora do escopo: qualquer funcionalidade para o usuário final.

## Capabilities

### New Capabilities
Nenhuma: mudança de infraestrutura e ferramentas (`skip_specs: true`).

### Modified Capabilities
Nenhuma.

## Impact

Todo o repositório: estrutura de pastas, build, CI, nomes de pacotes e binários.
