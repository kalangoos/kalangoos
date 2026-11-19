# Design

## Context

A prova em `poc/sidebar` validou superfícies layer-shell em Rust, molas por quadro e tokens. Ver `docs/ARQUITETURA.md`.

## Goals / Non-Goals

**Goals:** um processo de shell leve, superfícies independentes, controle por linha de comando e D-Bus.
**Non-Goals:** desenhar janelas de apps; animações de janela (compositor).

## Decisions

- **Um processo, várias superfícies.** Barra, dock, barras laterais e banners no mesmo processo para dividir
  tokens, molas e estado. Alternativa: um processo por superfície — mais isolamento, mais memória.
- **Integração por serviços padrão:** NetworkManager, BlueZ, PipeWire/WirePlumber, UPower, MPRIS, logind, via D-Bus
  (zbus). Nada de chamar programas de linha de comando.
- **Notificações:** o shell implementa `org.freedesktop.Notifications`.
- **Bloqueio:** protocolo `ext-session-lock-v1`; a tela de entrada é um greeter próprio sobre greetd.
- **Comandos:** `kalango shell <ação>` e uma interface D-Bus `os.kalango.Shell`; atalhos e agentes usam a mesma porta.

## Risks / Trade-offs

- [Um processo cai e leva tudo] → supervisão pelo systemd do usuário com reinício em menos de 1 s, estado em memória mínimo.
- [Protocolos diferentes entre compositores] → usar só protocolos padrão (wlr-layer-shell, ext-*), nada proprietário.

## Migration Plan

O `lunar-shell` antigo deixa de ser instalado quando o shell novo cobrir relógio, pasta de apps, central e dock.
