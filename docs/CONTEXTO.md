# Contexto dos agentes

Como o KalangoOS decide o que vai para o modelo local (LFM3 1.7B, 32K a 256K de contexto) em cada pedido de uma
conversa. O código está em `crates/kalango-agente/src/context.rs`; a plataforma de agentes (kalango-fluxos) aplica os
ajustes de memória do editor de agentes com as mesmas regras de segurança.

## O que Andrej Karpathy recomenda

Só o que está publicado e foi lido; as citações estão no original.

1. **Engenharia de contexto, não só de prompt.** No post em que defende o termo, ele descreve o trabalho como *"the
   delicate art and science of filling the context window with just the right information for the next step"*, e
   lista o que entra: descrição da tarefa, exemplos, RAG, dados relacionados, ferramentas, estado e histórico, e
   *"compacting"*. E avisa: *"Too little or of the wrong form and the LLM doesn't have the right context for optimal
   performance. Too much or too irrelevant and the LLM costs might go up and performance might come down."*
   — [x.com/karpathy/status/1937902205765607626](https://x.com/karpathy/status/1937902205765607626) (25/06/2025;
   o mesmo texto citado em [simonwillison.net](https://simonwillison.net/2025/Jun/27/context-engineering/)).
2. **A janela de contexto é a memória RAM.** Na palestra *Intro to Large Language Models*: *"this context window is
   your finite precious resource of your working memory"*, com o modelo trazendo para a janela e tirando dela o que a
   tarefa precisa (*"page relevant information in and out of its context window"*) —
   [youtube.com/watch?v=zjkBMFhNj_g](https://www.youtube.com/watch?v=zjkBMFhNj_g) (legendas automáticas, ~44 min). No
   "LLM OS" a RAM é a janela e o disco é o sistema de arquivos com busca —
   [x.com/karpathy/status/1723140519554105733](https://x.com/karpathy/status/1723140519554105733),
   [x.com/karpathy/status/1707437820045062561](https://x.com/karpathy/status/1707437820045062561).
3. **Amnésia: a memória de trabalho é programada por quem monta o contexto.** Na palestra *Software Is Changing
   (Again)* ([youtube.com/watch?v=LCEmiRjPEtQ](https://www.youtube.com/watch?v=LCEmiRjPEtQ)), ele compara os modelos a
   alguém com amnésia anterógrada: *"context windows are really kind of like working memory, and you have to sort of
   program the working memory quite directly"*; e, das ferramentas de autonomia parcial, *"the LLMs basically do a ton
   of the context management"*. Ele também diz para trabalhar *"in small incremental chunks"* e manter a IA *"on the
   leash"*. (Lido numa [transcrição automática](https://cdn.billmill.org/static/yt-transcribe/karpathy/httpswwwyoutubecomwatchvLCEmiRjPEtQ.html);
   não há transcrição oficial publicada, e as automáticas divergem em algumas palavras, por isso citamos só trechos
   em que elas concordam.)
4. **Anotações em vez de lembrar tudo.** No post sobre "system prompt learning": *"It feels more like taking notes for
   yourself"* e *"LLMs are quite literally like the guy in Memento, except we haven't given them their scratchpad
   yet"* — [x.com/karpathy/status/1921368644069765486](https://x.com/karpathy/status/1921368644069765486).
5. **No código dele, o contexto tem teto e o resultado de ferramenta é delimitado.** No nanoGPT, a geração corta o que
   passa do bloco (`idx[:, -self.config.block_size:]`, `model.py`); no nanochat, `render_conversation` trunca a conversa
   em `max_tokens` e o resultado da calculadora entra entre `<|output_start|>` e `<|output_end|>`, como algo que o modelo
   lê mas não escreve (`tokenizer.py`, `engine.py`) — [github.com/karpathy/nanoGPT](https://github.com/karpathy/nanoGPT),
   [github.com/karpathy/nanochat](https://github.com/karpathy/nanochat).

Não encontramos nada dele com o termo "context rot" nem um método detalhado de compactação: só a palavra
*"compacting"* do item 1. O que segue é a nossa aplicação dessas ideias a um modelo pequeno.

## O que fazemos

| Princípio | No KalangoOS |
|---|---|
| Só a informação certa para o próximo passo | O pedido leva as instruções, as memórias que a pessoa deixou, as ferramentas do assunto do pedido, o resumo das trocas antigas (quando há) e as trocas desde o resumo. |
| Janela é RAM, finita e preciosa | Dois tetos: um orçamento de qualidade (**12.288 tokens** para o 1.7B) e um limite duro (60 % do contexto do modelo). Perto deles, quem conta é o próprio modelo (`/tokenize`), não uma estimativa. |
| Mexer na janela só quando precisa | Enquanto cabe, a janela é toda a conversa desde o último resumo, e só cresce no fim: o cache de prompt do llama-server continua valendo. Passou do orçamento: primeiro os maiores resultados de trocas anteriores viram um resumo de poucas linhas (o agente reabre com `conversa.abrir`); depois saem as trocas mais antigas, **nunca as últimas 6**; a troca atual pode passar do orçamento até o limite duro em vez de perder o que está usando. Só depois do limite duro algo da troca atual encolhe. |
| A conversa inteira fica no disco | `mensagens.json` guarda tudo, sempre (gravado inteiro ou nada: arquivo temporário e troca de nome). O que muda é o que **vai** ao modelo. |
| Compactar | Quando o maior pedido passou de 75 % do orçamento — ou quando as trocas desde o resumo passam de 40 (ajustável; em 0, nunca) —, as mais antigas viram um **resumo contínuo**: uma chamada só, limitada (1.000 tokens, 60 s), com as trocas mais novas primeiro. Ela roda **em segundo plano depois da resposta**, enquanto a pessoa lê; se chega outro pedido, a chamada é **cancelada** (a conexão com o servidor do modelo fecha e o único lugar dele fica livre) e o pedido é respondido na hora; o resumo é tentado de novo depois da próxima resposta. Nenhuma resposta espera por um resumo na conversa do shell (um processo do agente por conversa, com a entrada em fila). Um pedido único na linha de comando e o terminal interativo ainda resumem no mesmo processo, depois da resposta, e quem espera o fim do processo espera o resumo. Nenhuma resposta espera por um resumo: sem resumo, o que não cabe sai da janela e continua pesquisável. |
| Anotações, não memória total | O resumo é uma lista de fatos, saída presa à forma `{"fatos": [...]}` pela gramática do llama.cpp (sem texto solto). Ao refazer a lista, as linhas antigas com números ou nomes que sumiram da nova voltam; quando está longa demais, saem primeiro as linhas sem fatos. |
| Buscar em vez de encher | `conversa.buscar` procura nas mensagens que saíram da janela e nos resultados guardados. As ferramentas seguem o **estado** da conversa, não as palavras do pedido (`conversa.buscar` quando parte da conversa está fora da janela; `conversa.abrir` quando a janela tem um resultado que pode virar resumo), para essas duas ferramentas não mudarem o começo do prompt de um pedido para o outro (o cache do servidor vale). Outras ferramentas ainda entram por palavras do pedido (memória, calculadora, plano, apagar) e, nesses pedidos, o começo do prompt muda e o servidor o avalia de novo; o relógio e o aviso sobre as mensagens anteriores ficam no fim do prompt. Só o aviso sobre as mensagens anteriores depende do pedido ("antes", "aquela", "de novo"…, palavras inteiras), e vai no fim do prompt, na mensagem atual. |
| Resultado de ferramenta é lido, não escrito | O resultado vai inteiro na troca em que é usado e nas seguintes enquanto cabe. Um resultado pequeno (menos de 2.000 caracteres) nunca é encurtado. |
| Dado externo nunca vira instrução | O resumidor recebe, linha a linha em JSON, o que a pessoa escreveu e o que o agente respondeu (uma citação dentro da resposta do agente continua sendo do agente) — nunca as páginas. O resumo passa pelos padrões da defesa (linha que parece ordem sai; linha que diz que a pessoa "autorizou" algo sai), perde marcas de envelope e tokens de controle, e vai ao modelo dentro do envelope de dados. Na verificação de origem (taint) ele conta como conteúdo lido neste computador, nunca como da web: um endereço que o agente montou com um dado local e que o resumo carregou não sai. |
| Passos pequenos e à vista | O cartão Uso do chat mostra o que foi enviado de fato em cada pedido, de quanto, quantas trocas foram inteiras e quantas estão no resumo; o marcador "Conversa compactada" deixa ver o que foi resumido. |

## Ajustes

Em Agentes › Ajustes › Avançado › Memória da conversa (gravados em `agente-ajustes.json`, na pasta de configuração do
KalangoOS):

- **Lembrar as últimas trocas inteiras** (`trocas_mantidas`, 1 a 30, padrão 6) — as trocas que nunca saem da janela;
- **Resumir conversas longas a cada** (`resumir_a_cada`, 0 a 60, padrão 40) — passando desse número de trocas desde o
  último resumo, as mais antigas são resumidas mesmo cabendo; em 0, nada é resumido e as antigas só saem da janela (quando não cabem);
- `contexto_max_tokens` (só no arquivo) — o orçamento de qualidade (padrão 12.288; nunca passa do limite duro).

Na plataforma (kalango-fluxos), o editor de agentes tem "Lembrar as 20 últimas trocas" (`keep_exchanges`) e "Resumir
conversas longas a cada 30 trocas" (`compact_every`): com o resumo ligado, nada sai da janela sem estar no resumo; com
`compact_every = 0`, o modelo recebe as últimas `keep_exchanges` trocas; com a conversa desligada, só o pedido atual.

## Medidas

Conversa de 40 trocas (um arquivo de 14 KB lido na segunda), LFM3 1.7B no llama.cpp, mesmo servidor e mesmas
perguntas para as duas versões (`measure_a_long_conversation`, em `tests/contexto.rs`). "Enviado" é o `prompt_tokens`
do pedido; "avaliado" é o que o servidor de fato processou (`/metrics`): o resto veio do cache de prompt.

| | antes (main) | agora |
|---|---|---|
| Maior prompt enviado | 7.607 tokens | 6.955 tokens |
| Prompt avaliado, soma das 40 trocas | 40.806 | 47.794 (cinco trocas reavaliaram o prompt inteiro quando o cache do servidor falhou, duas a mais que antes) |
| Tempo, soma das 40 trocas | 51 s | 77 s (medianas 1,0 s e 0,95 s; a diferença são duas trocas isoladas de 23 s e 10 s) |
| Resposta final (armário 4817, dito na troca 1) | certa | certa |

Não há ganho de tokens enviados nessa conversa: ela cabe no orçamento, e por isso **nada é resumido nem cortado** (antes,
a cada 12 trocas o histórico era reescrito, o que também invalidava o cache). O que muda é o que ela não perde. O
número "-55 %" de uma versão anterior deste documento media uma conversa que o orçamento nem exigia resumir, e foi
retirado.

Com orçamento de 4.096 tokens (para forçar resumos), 30 trocas resumidas custaram em média 3,4 s de processo contra
1,7 s das trocas sem resumo; a resposta já foi entregue antes do resumo começar, então a pessoa não espera por ele
(o processo é que termina depois).
