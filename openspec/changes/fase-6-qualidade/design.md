# Design

## Context

Requisitos de desempenho em `docs/REQUISITOS.md`; segurança em `docs/SEGURANCA.md`.

## Goals / Non-Goals

**Goals:** metas mensuráveis e verificadas na CI.
**Non-Goals:** otimização sem medida.

## Decisions

- **AT-SPI** para leitor de tela em todos os componentes próprios.
- **Fluent ou gettext** para traduções (decidir com o toolkit).
- **Banco de testes de desempenho** em hardware de referência (NVIDIA RTX, AMD e Intel integrada).

## Risks / Trade-offs

- [Desempenho piora sem perceber] → limite na CI que falha o build.
