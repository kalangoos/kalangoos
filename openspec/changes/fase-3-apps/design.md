# Design

## Context

Design completo nas páginas 15, 16 e 19 do arquivo de design. Toolkit decidido na fase 0.

## Goals / Non-Goals

**Goals:** nenhuma tarefa comum mostra tela de outro sistema; apps próprios leves.
**Non-Goals:** reescrever todos os apps de uma vez.

## Decisions

- **Reaproveitar primeiro, substituir depois.** Na 0.x, apps maduros existentes com o nosso tema (ex.: um terminal
  em Rust). Escrever próprios, nesta ordem: Configurações, Arquivos, Monitor, Calculadora.
- **Configurações como app próprio em Rust**, falando com os serviços do sistema por D-Bus.
- **Portal próprio** (`xdg-desktop-portal-kalango`) para seletor de arquivos, impressão e permissões.
- **Tema:** CSS gerado pelo `kalango-tokens` para GTK3/GTK4 e paleta equivalente para Qt.

## Risks / Trade-offs

- [Apps de terceiros fora do tema] → aceitar; só a moldura é nossa.
- [Volume de painéis] → priorizar os do dia a dia: Rede, Bluetooth, Tela, Som, Energia.
