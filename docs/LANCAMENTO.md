# KalangoOS · versões e lançamento

## Numeração

`ANO.MÊS` para versões do sistema (ex.: 27.04), seguindo a base Ubuntu LTS. As versões 0.x são de desenvolvimento.

## Marcos

| Versão | Conteúdo (fases) | Público |
|---|---|---|
| 0.1 · prévia técnica | Fases 0, 1 e 2: shell e janelas próprios sobre Ubuntu, sem agentes reais | Desenvolvedores e testadores |
| 0.2 · prévia | + Fase 3: tema, ícones, Configurações e apps essenciais | Entusiastas |
| 0.3 · beta | + Fase 4: agentes, kalangod, linha do tempo | Testadores com backup |
| 0.4 · beta | + Fase 5: instalador, atualizações, restauração, loja | Público em geral, com aviso |
| 1.0 | + Fase 6 cumprida: acessibilidade, inglês, metas de desempenho, revisão de segurança | Público em geral |

O escopo exato da 0.1 é decidido na fase 0.

## Canais

| Canal | Para quem | Atualizações |
|---|---|---|
| Estável | Todos | Mensal, mais correções de segurança quando preciso |
| Beta | Testadores | Semanal |
| Diário | Desenvolvimento | A cada build da CI |

## Suporte

Cada versão estável recebe atualizações até 3 meses depois da seguinte. Versões baseadas numa LTS nova recebem
suporte estendido de 2 anos.

## Critérios para publicar uma versão

- `openspec validate --all --strict` sem erros e todas as tarefas da fase marcadas.
- Matriz de hardware (NVIDIA, AMD, Intel) testada.
- Metas de desempenho cumpridas na CI.
- Instalação, atualização e volta testadas automaticamente em VM.
- Notas de versão em português e inglês em `kalangoos.com`.
