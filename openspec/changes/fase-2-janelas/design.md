# Design

## Context

Compositor base em Rust (niri ou cosmic-comp, decisão da fase 0). O niri 26.04 já rodou a prova com NVIDIA 580 em janela.

## Goals / Non-Goals

**Goals:** comportamento de janelas do design; 60 qps estáveis em 4K com NVIDIA.
**Non-Goals:** escrever um compositor do zero.

## Decisions

- **Alterações mínimas e isoladas** no compositor base, mantidas como camada própria e enviadas de volta ao projeto
  original quando fizer sentido.
- **Molas no compositor** com os mesmos parâmetros dos tokens.
- **NVIDIA:** exigir driver ≥ 560 com GBM e explicit sync; testar em sessão real a cada versão.
- **Captura** só pelos portais (o app nunca vê a tela inteira sem escolha do usuário).

## Risks / Trade-offs

- [Regressões NVIDIA após atualização do driver] → matriz de testes por versão de driver; fixar a versão na imagem.
- [Divergência do projeto original] → atualizar a base a cada lançamento dele.
