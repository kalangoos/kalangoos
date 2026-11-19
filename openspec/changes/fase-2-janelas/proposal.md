# Proposal

## Why

Janelas, espaços e animações de janela são o que distingue o KalangoOS de um tema sobre outro sistema, e só podem
ser feitos dentro do compositor.

## What Changes

- Moldura e estados de janela; animações de janela com mola.
- Encaixe, alternador, visão geral e espaços por intenção.
- Vários monitores, escala, gestos, modo toque, captura e compartilhamento de tela.
- Compatibilidade com o driver NVIDIA proprietário como critério de aceite.

Fora do escopo: apps próprios (fase 3).

## Capabilities

### New Capabilities
- `janelas`: moldura, estados, animações, encaixe e alternância.
- `espacos`: espaços por intenção e visão geral.
- `monitores-e-entrada`: vários monitores, escala, gestos e modo toque.
- `captura-de-tela`: captura, gravação e compartilhamento.
- `compatibilidade-grafica`: GPUs e drivers suportados, incluindo NVIDIA proprietário.

### Modified Capabilities
Nenhuma.

## Impact

Crate `kalango-comp` sobre o compositor escolhido; portais xdg-desktop-portal; XWayland.
