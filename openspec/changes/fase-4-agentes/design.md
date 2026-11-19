# Design

## Context

Desenho completo em `docs/ARQUITETURA.md`, seção 3. O kernel do Ubuntu já oferece Landlock, eBPF com BTF, eBPF LSM
(falta ativar na linha de inicialização) e fanotify.

## Goals / Non-Goals

**Goals:** agentes conseguem administrar o sistema; nada destrutivo sem aprovação; tudo auditado e reversível.
**Non-Goals:** módulo de kernel próprio; agentes com root permanente.

## Decisions

- **kalangod em Rust, root, só por soquete local** (`/run/kalango/mcp.sock`), identificando o chamador por
  `SO_PEERCRED` e cgroup. Alternativa: módulo de kernel — rejeitada (quebra com atualizações, risco de travar).
- **eBPF com Aya (Rust)** para regras invioláveis e auditoria. Alternativa: só AppArmor — menos dinâmico.
- **Cada tarefa num cgroup e namespaces próprios**, com Landlock e seccomp.
- **Desfazer:** cada ferramenta declara efeito e inverso; mudanças de sistema criam ponto de restauração antes.
- **Aprovação desenhada pelo compositor**, fora do alcance do agente.
- **Modelos:** local por padrão; nuvem só com permissão e orçamento.

## Risks / Trade-offs

- [Instrução maliciosa em conteúdo lido] → conteúdo externo marcado como dado; ações de risco sempre perguntam.
- [Nível total abusado] → aprovação presencial por sessão, prazo de 15 min, ponto de restauração antes.
- [Desfazer impossível para algumas ações (ex.: enviar e-mail)] → essas ações são marcadas como irreversíveis e
  sempre perguntam.

## Open Questions

- Formato exato do registro da linha do tempo (decidir no primeiro protótipo, não muda as specs).
