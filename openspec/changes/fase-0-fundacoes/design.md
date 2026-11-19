# Design

## Context

Ver `docs/ARQUITETURA.md`. Hoje: design em FigCraft, tokens exportados por script avulso, `lunar-shell` legado em
GTK3/X11 e uma prova de conceito em GTK4/Wayland em `poc/`.

## Goals / Non-Goals

**Goals:** um comando compila e testa tudo; decisões registradas; nomes coerentes.
**Non-Goals:** recursos para o usuário; empacotamento final.

## Decisions

- **Workspace Cargo** com crates `kalango-shell`, `kalango-comp` (camada sobre o compositor escolhido),
  `kalango-agentes`, `kalangod`, `kalango-tokens`, `kalango` (CLI). Alternativa: repositórios separados — rejeitada
  na fase inicial pelo custo de coordenação.
- **Compositor:** comparar niri e cosmic-comp rodando a mesma prova. Critérios, em ordem: estabilidade com NVIDIA,
  suporte a layer-shell e protocolos de shell, facilidade de manter alterações próprias pequenas, consumo.
- **Toolkit:** segunda prova com iced/libcosmic contra a de GTK4. Critérios: fidelidade ao design, memória e CPU
  medidas, maturidade de acessibilidade.
- **Restauração:** btrfs com snapshots como hipótese principal; validar com o instalador e o apt.

## Risks / Trade-offs

- [Escolher o compositor errado] → manter as nossas alterações num crate separado e pequeno.
- [Medidas pouco confiáveis] → script de medida revisado e repetido 3 vezes antes de qualquer decisão.
