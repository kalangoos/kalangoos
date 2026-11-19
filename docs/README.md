# KalangoOS · documentação

| Documento | Para que serve |
|---|---|
| [VISAO.md](VISAO.md) | O que é o produto, para quem, promessas e diferenciais |
| [ARQUITETURA.md](ARQUITETURA.md) | Decisões técnicas e o desenho do sistema, incluindo o kalangod |
| [REQUISITOS.md](REQUISITOS.md) | Hardware suportado, placas de vídeo e metas de desempenho |
| [SEGURANCA.md](SEGURANCA.md) | Modelo de ameaças, camadas de defesa e processo |
| [LANCAMENTO.md](LANCAMENTO.md) | Versões, marcos, canais e critérios de publicação |
| [LICENCAS.md](LICENCAS.md) | Licenças do código, da marca e de recursos de terceiros |
| [PLANO.md](PLANO.md) | Resumo do plano; o detalhe em checkbox está no OpenSpec |
| [../openspec](../openspec) | Especificação completa: 7 fases (propostas, desenhos, tarefas) e 40 capacidades |
| [especificacao/](especificacao/index.html) | Página gerada do OpenSpec, com progresso e requisitos |
| [pagina/](pagina/index.html) | Diário do projeto: etapas, casos de uso e evolução |

## Como trabalhar com a especificação

```sh
npx @fission-ai/openspec list                 # fases em andamento e progresso
npx @fission-ai/openspec show fase-1-shell    # proposta, tarefas e requisitos de uma fase
npx @fission-ai/openspec validate --all --strict
```

Ao concluir uma tarefa, marque `[x]` no `tasks.md` da fase só depois de verificar o que a própria tarefa diz.
Quando todas as tarefas de uma fase estiverem marcadas, `openspec archive <fase>` move as especificações dela para
`openspec/specs/`, que passa a descrever o sistema em vigor.
