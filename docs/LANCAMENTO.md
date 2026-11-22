# KalangoOS · versões e lançamento

## Numeração

`ANO.MÊS` para versões do sistema (ex.: 27.04), seguindo a base Ubuntu LTS. As versões 0.x são de desenvolvimento.

## Marcos

| Versão | Conteúdo (fases) | Público |
|---|---|---|
| **0.1 · prévia técnica** | Fases 0 a 2 e o núcleo das fases 4 e 5: ISO instalável testada numa VM, shell e janelas próprios (niri com patches), `kalangod` (aprovação, desfazer, Linha do tempo, níveis), agentes locais (LFM3), Kalango Remote e Audio, gestão de contexto, editor de fluxo. Limitações na tabela abaixo | Desenvolvedores e testadores |
| 0.2 · prévia | Completa a fase 3: tema, ícones, Configurações e apps essenciais | Entusiastas |
| 0.3 · beta | Completa a fase 4: agentes, kalangod, linha do tempo | Testadores com backup |
| 0.4 · beta | Completa a fase 5: instalador, atualizações, restauração, loja | Público em geral, com aviso |
| 1.0 | + Fase 6 cumprida: acessibilidade, inglês, metas de desempenho, revisão de segurança | Público em geral |

### Escopo da 0.1.0 (22 de novembro de 2026)

A 0.1.0 é uma **prévia técnica**, não um sistema para o dia a dia. O que a esteira de lançamento
(`.github/workflows/release.yml`) garante a cada execução: a ISO é montada, a sessão ao vivo liga numa VM com o
instalador, o motor do instalador grava num disco virtual vazio (a etapa gráfica "Modelos de IA" não é exercitada pelo teste) e o sistema instalado
liga na sessão. Uma tag anexa essa ISO testada, os binários, as somas SHA-256 e o atestado de origem ao rascunho da
versão. As notas estão em [NOTAS-0.1.0.md](NOTAS-0.1.0.md) e o histórico em [../CHANGELOG.md](../CHANGELOG.md).

### Limitações conhecidas da 0.1.0

| Limitação | Situação |
|---|---|
| Hardware real | Testada só em máquina virtual (UEFI). A matriz NVIDIA, AMD e Intel não está completa; não instale num computador com dados que você não pode perder |
| Modelo local fora da ISO | As licenças dos modelos estão em revisão, então a ISO da esteira não leva modelo. O instalador (etapa "Modelos de IA") e a Central de Apps o baixam do catálogo público, de um servidor privado ou de uma pasta |
| Acessibilidade | Sem certificação nem auditoria; leitor de tela e navegação completa por teclado ainda não foram validados |
| Idioma | Interface e documentação em português do Brasil; o inglês é da fase 6 |
| Kalango Remote | O modo privacidade, a verificação em duas etapas, a gravação de sessões e o compartilhamento por servidor próprio ainda não existem; o app ainda não foi testado entre dois computadores físicos nem numa sessão real do niri (só aninhada) |
| Kalango VirtualDesk e Kalango Fluxos | Repositórios próprios, fora da ISO da esteira; o `kalangod` e o shell já falam com eles, mas o serviço de fluxos é privado |
| Atualizações e restauração | Ainda sem atualização com ponto de restauração nem tela de recuperação (fase 5) |
| Marca e licenças da documentação e dos recursos gerados por IA | Registro da marca e revisão dos termos de uso do serviço de imagens pendentes ([LICENCAS.md](LICENCAS.md)) |
| Desempenho e segurança | Metas de desempenho e revisão de segurança externa são da fase 6; defesas e limites conhecidos em [SEGURANCA.md](SEGURANCA.md) |

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

Situação de cada critério na 0.1.0 (22 de novembro de 2026):

| Critério | Situação na 0.1.0 |
|---|---|
| `openspec validate --all --strict` sem erros e todas as tarefas da fase marcadas | **Não cumprido.** A validação passa (10 itens, 0 falhas, com `npx @fission-ai/openspec validate --all --strict`), mas há cerca de 62 tarefas sem marca nas fases 0 a 2 (`openspec/changes/fase-*/tasks.md`) |
| Matriz de hardware (NVIDIA, AMD, Intel) testada | **Dispensado** numa prévia 0.x; pendente |
| Metas de desempenho cumpridas na CI | **Não cumprido, dispensado** (fase 6) |
| Instalação, atualização e volta testadas automaticamente em VM | **Só a instalação** é testada (o motor do instalador, ao vivo e o primeiro boot); atualização e volta ainda não existem |
| Notas de versão em português e inglês em `kalangoos.com` | Estão no repositório ([NOTAS-0.1.0.md](NOTAS-0.1.0.md)); a publicação no site está pendente |

A 0.1.0 é publicada pela **exceção de prévia técnica**, com esses critérios dispensados até a 0.4 e a 1.0.
