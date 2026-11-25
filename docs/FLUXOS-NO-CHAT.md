# Fluxos pelo chat com o modelo pequeno

O modelo local do KalangoOS (LFM3 de 1,7 bilhão de parâmetros) cria agentes e fluxos pelo chat, mas escrever um grafo
inteiro de blocos é mais do que ele faz de forma confiável. Este documento diz o que foi feito para ajudá-lo, o que
foi medido, o que ele ainda não faz e para que serve o editor visual.

## O que o modelo pequeno faz e o que não faz

Faz bem (3 de 3 nas medições): um agente assistente; uma sequência de agentes; uma condição simples (se tiver tal
texto, avise); uma espera seguida de um aviso; perguntar e responder.

Faz com falhas: fluxos com mais de um desvio, aprovação da pessoa no meio e `decide` (o modelo de decisão d1; o modelo
escreve uma busca de texto em vez de pedi-lo). Os erros típicos são esquecer o bloco de condição, ligar blocos que não existem,
escrever só um bloco ou repetir a chamada que falhou.

Não faz: tarefas que precisam de um bloco que não existe (enviar e-mail, escutar o navegador ao vivo). Quando o pedido
não cabe nos 19 blocos, o certo é dizer isso; o modelo pequeno muitas vezes responde como se tivesse feito.

Isso é o limite do tamanho do modelo, não um defeito do contrato: um modelo maior (o catálogo traz o Qwen3-8B como
opcional) acerta mais. Nada abaixo substitui isso, só reduz o que o modelo pequeno precisa acertar.

## O editor visual é para o que é grande ou delicado

O chat serve para o esboço e para os fluxos curtos. Um fluxo com vários desvios, ações do sistema, máquinas virtuais ou
dados precisos deve ser montado ou revisado no editor visual (Agentes › Fluxos): lá a pessoa vê o grafo inteiro, e a
aprovação do chat sempre mostra cada bloco e cada conexão antes de qualquer coisa existir.

## Como o modelo cria um fluxo agora

Dois caminhos, os dois terminando na mesma aprovação, que mostra tudo o que vai existir:

1. **JSON inteiro** (`fluxos.criar`): como antes, mas o argumento `fluxo` leva um esquema em que `type` é um dos 19 tipos.
   O llama.cpp transforma o esquema em gramática: um tipo que não existe não consegue ser escrito. Quando o JSON falha,
   o servidor acrescenta ao erro **sugestões** (falta um fim, um bloco sem ligação, o pedido diz "se" e não há
   condição): só texto para o modelo, nada é aplicado em silêncio.
2. **Em etapas** (`fluxos.planejar`, `fluxos.ajustar`, `fluxos.ligar`, `fluxos.adicionar_bloco`, `fluxos.concluir`): o
   modelo dá passos curtos em texto ("transcrever o áudio", "se falar do gta 6, resumir", "senão, fim"), o servidor
   mapeia cada passo para um bloco por regras fixas de palavras e desenha as conexões; o modelo corrige o que falta e
   conclui. Um julgamento ("se for uma reclamação") vira o bloco `decide` (o modelo de decisão d1); uma busca
   ("se contiver urgente") vira `condition`; pedir permissão vira `approval`; "chamar o fluxo X" vira `subflow`.

As receitas (formatos prontos com campos a preencher) foram feitas e medidas, não ajudaram e foram removidas.

## Segurança

- Criar sempre pergunta; a prévia mostra todos os blocos e conexões, igual à de `fluxos.criar`.
- A aprovação fica presa à conversa (usuário e processo) e aos blocos exatos que a pessoa viu: rascunho mudado depois,
  outro processo ou chamada sem prévia são recusados, e `concluir` nunca aceita "sempre" (a tela de aprovação não
  oferece o botão e o servidor recusa a resposta).
- O planejador nunca escolhe `system_action`, `sandbox`, `speak` nem `describe_image`: esses só entram se o modelo os
  adicionar de propósito com `fluxos.adicionar_bloco`, e a pessoa os vê na aprovação. O bloco `subflow` (chama outro
  fluxo) só nasce de um passo que **nomeia** o fluxo ("chamar o fluxo limpeza"): o `flow_id` é então a palavra da
  pessoa, mostrada na aprovação; um passo sem nome deixa o campo vazio. O texto de um passo é dado, nunca muda o tipo
  do bloco; um agente, arquivo ou fluxo que o passo não nomeou fica vazio e o servidor recusa concluir, em vez de
  inventar.
- Rascunhos: por conversa (usuário e processo), até 24 passos por plano, 40 blocos, 64 KB de JSON, 8 rascunhos por
  usuário, 32 ao todo e uma hora de vida; uma linha do socket tem no máximo 8 MiB.
- Um passo que precisa enviar e-mail ou mensagem não vira bloco: o planejador diz que não está disponível. (Existem:
  notificações neste computador, ações do sistema, `web.ler` por uma `system_action`, e rotinas da pessoa para horários.)
- O planejador conhece palavras em português e em inglês; em outro idioma do sistema ele recusa e manda montar o fluxo
  bloco a bloco (o modelo escolhe cada tipo). O idioma da transcrição vem do idioma do sistema, nunca é suposto.

## Medições

Pedidos em português, 3 execuções cada, com o modelo real (LFM3 1,7B) e o dublê do serviço da plataforma (teste
`flow_benchmark`, `KALANGO_CRIAR_BENCH=1`, em `crates/kalango-agente/tests/criar.rs`). Passa quando o fluxo salvo tem os
blocos pedidos, as ligações e os desvios desenhados (o pedido do GTA 6 exige ainda um `end`; o pedido impossível passa
quando a resposta diz que não há bloco para aquilo). O modelo não é determinístico: 3 execuções por pedido dão uma ideia,
não uma taxa, e as diferenças de uns poucos pontos estão dentro do ruído.

| pedido | antes | só esquema com enum | receitas | etapas (v1) | etapas final |
|---|---|---|---|---|---|
| sequência de agentes | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| condição com aviso | 2/3 | 3/3 | 2/3 | 3/3 | 3/3 |
| laço (`for_each`) | 0/3 | 2/3 | 2/3 | 2/3 | 3/3 |
| espera e aviso | 2/3 | 2/3 | 3/3 | 3/3 | 3/3 |
| aprovação da pessoa | 0/3 | 2/3 | 1/3 | 1/3 | 2/3 |
| agente e aviso | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| `decide` (d1) | 0/3 | 0/3 | 0/3 | 0/3 | 1/3 |
| transcrever, condição, agente | 1/3 | 3/3 | 2/3 | 3/3 | 2/3 |
| chamar outro fluxo | 0/3 | 0/3 | 0/3 | 3/3 | 3/3 |
| perguntar e responder | 2/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| GTA 6 (condição, agente e fim) | 1/3 | 2/3 | 2/3 | 2/3 | **3/3** |
| pedido impossível (e-mail) | 1/3 | 1/3 | 1/3 | 0/3 | 3/3 |
| **total** | **15/36 (41 %)** | **24/36 (66 %)** | **22/36 (61 %)** | **26/36 (72 %)** | **32/36 (88 %)** |

"Etapas (v1)" são as ferramentas de rascunho com o planejador da primeira rodada de ajustes; entre ele e "etapas final"
houve duas rodadas de ajuste das palavras do planejador (aprovação, `decide`, `subflow`, "termine"), exemplos na
descrição de `fluxos.planejar`, o aviso de que não há bloco para e-mail, sites ou horários, e o `for_each` sem lista
percorrendo a saída anterior. Uma rodada intermediária das etapas deu 29/36 (80 %). O "antes" e o "só esquema" usam
`KALANGOD_FLUXOS_AUTORIA=classico` com o código de antes dos ajustes.

O que se aprendeu:

- **O esquema com `enum` de tipos foi o maior ganho isolado** (41 % para 66 %): o modelo não escreve mais um tipo que não
  existe e erra menos o resto. Um esquema com uma variante por tipo (cada uma com as chaves de `data` exigidas) foi
  medido e **piorou muito** (0 de 13): a gramática empurrava o modelo para o tipo que pede menos (`end`, `join`).
- **As etapas valem a pena, mas só depois de o planejador entender as palavras do modelo pequeno** ("pergunte se quer
  que eu rode" é aprovação, "se for uma reclamação" é um julgamento para o d1, "termine" é fim). A primeira versão
  (com as receitas também no ar) ficou abaixo do esquema sozinho: 20 de 36 (55 %).
- **As receitas não ajudaram** (61 %, abaixo do esquema sozinho): o modelo coloca os campos no lugar errado e não as
  escolhe para pedidos que não são um formato pronto. Foram removidas do código.
- **O pedido impossível** só passou a ser dito quando a descrição de `fluxos.criar` e o erro do planejador dizem que não
  há bloco para e-mail, sites ou horário.
- **Falhas que sobraram:** `decide` (o modelo escreve uma busca de texto em vez de pedir o d1 quando o pedido o cita),
  uma execução de aprovação sem nenhuma chamada e transcrever com três chamadas recusadas pela plataforma.
- **Não implementado: um modelo maior para criar fluxos.** O motor carrega um único modelo de texto
  (`DEFAULT_MODEL` em `modelo.rs`); usar o Qwen3-8B opcional só para esta tarefa exigiria escolher o modelo por
  ferramenta, e o modelo não está instalado aqui para testar.

**Pedidos novos (held-out).** As 12 linhas acima foram usadas para ajustar as palavras do planejador, as descrições e os
exemplos, então o resultado nelas é otimista. Para medir de verdade foram escritos 11 pedidos depois, sem ajustar nada a
eles (`KALANGO_CRIAR_BENCH_SET=heldout`; o pedido impossível agora exige também que **nenhum fluxo seja salvo**). Com o
código dos commits desta rodada, no dublê, 3 execuções cada:

| conjunto | passes | intervalo de 95 % (Wilson) |
|---|---|---|
| os 12 pedidos usados no ajuste | 33/36 (92 %) | 78 %–97 % |
| os 11 pedidos novos | 23/33 (70 %) | 53 %–83 % |
| os 23 juntos | 56/69 (81 %) | 70 %–89 % |

(As 3 execuções de um pedido não são independentes; o intervalo só dá uma ideia de quanto 3 execuções dizem.) Os pedidos
novos ficam **abaixo de 80 %**: o alvo vale para o conjunto todo, não para os pedidos novos sozinhos. Nos novos, os
pedidos que mais falharam foram "repetir o agente classificador para cada linha de um arquivo" (0/3: o planejador lia
"classificador" como pedido do modelo de decisão; corrigido depois da medição, sem nova medição) e "saber se o texto é
spam" (0/3: o modelo não pediu o d1). Esses dois pedidos deixaram de ser "novos" ao serem usados para o ajuste.

**Contra o serviço real** (`kalango-fluxosd`, banco temporário, mesmo teste com `KALANGO_CRIAR_REAL`): 31 de 36 (86 %),
GTA 6 3/3; falharam `decide` (0/3), aprovação (2/3) e chamar outro fluxo (2/3). A primeira rodada no serviço real
mostrou algo que o dublê aceitava: o motor recusa um bloco `end` com `data`, até `{}` ("expected unit variant
Block::End"); o servidor agora tira os dados do bloco de fim e o dublê passou a recusar igual.

Alvo do trabalho: pelo menos 2 de 3 no pedido do GTA 6 (obtido: 3/3 nas duas rodadas finais) e 80 % no conjunto
(obtido nos 12 pedidos do ajuste: 88 % no dublê, intervalo de 95 % de 75 % a 96 %, e 86 % no serviço real, de 71 % a 94 %).
Os pedidos novos deram 70 % no dublê (ver acima), e **a rodada nos pedidos novos contra o serviço real ainda não foi feita**
(a máquina estava sobrecarregada; fazer depois do reinício).

## Rodada do d1 e das respostas tipadas (2026-11-25)

Objetivo desta rodada: pelo menos 90 % de acerto em pedidos em que o modelo **não** foi ajustado. **Não foi atingido**: o
número desta primeira parte (superado pelos conjuntos D e E abaixo), num conjunto novo congelado antes do resultado, é **74/111 (67 %), intervalo de 95 % de Wilson 57 %–75 %**
(serviço real `kalango-fluxosd`, 3 execuções por pedido, o planejador só conhece português e inglês).

### O que mudou

- **O d1 lê o tipo de cada passo** (`kalango-agente/src/passos.rs`). Antes de `fluxos.planejar` ir ao kalangod, o motor
  pergunta ao d1 (uma pergunta `choice` por passo, só probabilidades, sem texto) qual bloco o passo pede e acrescenta
  `tipos`. É só uma dica: o planejador a aceita se o passo tem o que o bloco precisa (um fluxo nomeado para `subflow`, uma
  palavra para `condition`), nunca troca o que as palavras do passo dizem (uma busca por palavra continua busca), nunca cria
  `system_action`, `sandbox`, `speak` ou `describe_image` e nunca encerra um fluxo. Sem o d1 instalado, ou sem certeza
  (menos de 55 %), valem só as regras de palavras. A opção `unsupported` (só com 80 % ou mais) faz o planejador recusar o
  passo como o faz com e-mail.
- **O pedido da pessoa vale mais que a paráfrase do modelo**: o motor passa o texto que a pessoa escreveu (`pedido`); se ele
  nomeia o d1 ou o modelo de decisão e nenhum passo virou decisão, a primeira busca por palavra vira o bloco `decide`, com a
  pergunta tirada da frase da pessoa. O texto da pessoa só decide entre `condition` e `decide`; não muda nível nem permissão.
- **Planejador**: "decisão"/"modelo de decisão" pedem o d1; "se o resultado do d1 for verdadeiro: …" e "se a resposta for
  sim" são os caminhos da decisão (antes viravam outra decisão); "conteúdo do e-mail" lido não é e-mail enviado; transcrever
  exige áudio; "pedir ao usuário" é `ask`; "desejar que eu rode" é aprovação; "enviar uma mensagem" é `message`; passos
  amontoados ("Aplicar condição: se …, avisar; senão, terminar"; duas frases num passo) são desfeitos; um passo que nenhum
  bloco faz é deixado de fora com uma nota (antes o plano inteiro era recusado e o modelo desistia); redes sociais,
  calendário e bancos de dados valem como "não existe bloco".
- **Pergunta do d1 pré-preenchida**: o bloco `decide` nasce com a pergunta tirada do passo ("avaliar se o texto é spam" vira
  "o texto é spam?"), com as saídas `true` e `false` ligadas.
- **Só pelas etapas por padrão** (`KALANGOD_FLUXOS_AUTORIA=so-etapas`): `fluxos.criar` (o JSON inteiro) não é oferecido ao
  modelo, para que ele não pule o planejador escrevendo o JSON. `etapas` (as duas) e `classico` (só o JSON) continuam
  disponíveis por essa variável. É uma mudança de comportamento do produto: quem edita fluxos já criados continua usando
  `fluxos.editar`.
- **Lembrete do motor**: se o modelo planejou um rascunho e tenta responder sem chamar `fluxos.concluir`, o motor lembra,
  até duas vezes. A criação continua pedindo a aprovação da pessoa.
- **Respostas tipadas dos agentes** (`output_fields` em `agentes.criar`/`agentes.editar`): um assistente pode declarar os
  campos da resposta (`{name, kind, required, description}`, tipos `text`, `number`, `integer`, `boolean`, `list`, `object`,
  até 30). O motor de fluxos já valida a resposta e pergunta de novo ao modelo quando ela não cabe; um bloco seguinte lê o
  campo (`content.urgente`). Aqui só se acrescentou o que o servidor deixava passar: a validação do pedido, a prévia da
  aprovação ("responde com os campos: …") e o texto de `agentes.criar`. Não há tipo `enum` nos campos (mudaria o repositório
  do motor de fluxos); níveis e permissões não dependem desses campos.

### O d1 contra o modelo de texto, em classificação

60 textos curtos escritos para isso (spam, reclamação, urgência; metade sim, metade não), a mesma pergunta para os três
jeitos, LFM3 1,7B sem raciocínio contra d1-2B:

| jeito | acertos | intervalo de 95 % |
|---|---|---|
| d1 (`noul`) | 60/60 (100 %) | 94 %–100 % |
| modelo de texto, resposta livre e uma condição "começa com sim" | 54/60 (90 %) | 80 %–95 % |
| modelo de texto, JSON com `enum` {sim, nao} | 42/60 (70 %) | 58 %–80 % |

Para classificar e rotear o d1 é melhor e mais barato; a saída tipada com `enum` não ajudou o modelo de texto. Por isso o
planejador manda julgamentos para `decide` e deixa o modelo de texto para escrever e resumir. (Poucos textos, escritos por
quem fez o planejador: é um indício, não uma medição de uso.)

### Medições desta rodada

Todas contra o serviço real, 3 execuções por pedido. Cada conjunto foi escrito e entrou no git antes do resultado dele
(`KALANGO_CRIAR_BENCH_SET=a|b|c`).

| conjunto | código | passes | intervalo de 95 % |
|---|---|---|---|
| A (36 pedidos novos; vários não dizem "fluxo" e o modelo os executa em vez de criar um fluxo) | de antes desta rodada | 43/108 (40 %) | 31 %–49 % |
| os 23 pedidos já usados no ajuste, só etapas, d1 e regras novas | no meio da rodada | 57/69 (83 %) | 72 %–90 % |
| B (36 pedidos novos) | com o d1, as regras e `so-etapas` | 81/108 (75 %) | 66 %–82 % |
| C (37 pedidos novos, escritos depois do B) | final, com o lembrete do motor | **74/111 (67 %)** | **57 %–75 %** |

O B foi usado para decidir a mudança seguinte (o lembrete do motor), por isso o número desta parte é o do C. Antes e depois no
mesmo conjunto só existe para o A (antes) e para os 23 pedidos de ajuste, que já não são novos. As 3 execuções de um pedido
não são independentes; os intervalos são otimistas.

### Por que ficou abaixo de 90 %

Histograma do C (37 falhas): 19 são "nenhuma chamada de fluxo recusada e nenhum fluxo salvo" (o modelo executa o pedido,
lista arquivos ou responde sem chegar a `fluxos.planejar`/`fluxos.concluir`: "automação: antes de rodar o agente de
limpeza, pedir a minha permissão" → `agentes.listar` e nada mais), 10 são fluxos não salvos depois de chamadas que o
servidor recusou, 5 são o pedido impossível sem a resposta "não há bloco" (ligar para o telefone, postar no Discord) e 3
são blocos que faltam no fluxo salvo. Ou seja: o que sobrou
não é mais o mapeamento de palavras para blocos, é o modelo pequeno **não decidir criar o fluxo**.

Próxima alavanca (não implementada): o motor perguntar ao d1 se o pedido é "criar uma automação" ou "fazer agora" e, se for
criar, chamar o planejador direto ou obrigar a primeira chamada a ser `fluxos.planejar`; e `unsupported` para o d1 também
ler o pedido inteiro (telefone, mensagens) antes de o modelo responder. Um modelo de texto maior (Qwen3-8B, opcional) para
esta tarefa continua sendo uma opção documentada, não um requisito.

### Testes

Testes de unidade do planejador (31) e do `passos.rs` (3), do `output_fields` e das ferramentas; checagem de mutação: 10
mutantes das regras novas (palavra "decisão", resultado da decisão, e-mail lido, transcrever sem áudio, fim vindo do d1,
pedido do d1 ignorado, `unsupported` ignorado, frases não separadas, passo sem bloco, d1 sobre busca) morrem nos testes.
Clippy sem avisos novos em `kalangod`, `kalango-agente` e `kalango-i18n`.

## Portão de intenção, chamada forçada e o conjunto D (2026-11-25, segunda parte)

Resultado desta parte (o mais recente é o do conjunto E, mais abaixo), em 37 pedidos novos congelados antes de qualquer execução (conjunto D, vários sem a palavra
"fluxo"), 3 execuções cada, serviço real: **71/111 (64 %), intervalo de 95 % de Wilson 55 %–72 %. O alvo de 90 % não foi
atingido.** O conjunto C de novo, com o código final, numa só passada: 27/37 (73 %, 57 %–85 %), contra 74/111 (67 %) antes do
portão; a diferença não é distinguível do acaso.

### O que foi feito

- **Portão de intenção** (`kalango-agente/src/intencao.rs`): regras de palavras dizem se a mensagem pede para construir um
  fluxo (fluxo, automação, workflow, rotina… e "rode o fluxo X" sozinho não é construir); sem palavra de fluxo, o d1 desempata
  (uma pergunta `choice`: fluxo, agora, agente, conversa; vale a partir de 5 % de "fluxo", limiar medido em 16 mensagens
  escritas para isso, em que as que não eram fluxo ficaram em 2 % ou menos). Para pedidos de fluxo a primeira chamada do
  modelo é forçada em `fluxos.planejar` (`tool_choice: "required"` com só essa ferramenta, sem raciocínio e com teto de 2048
  tokens) e, se o modelo tenta responder com um rascunho aberto, a chamada seguinte é forçada em `fluxos.concluir`. Antes do
  teto e do "sem raciocínio" uma geração forçada rodava por minutos.
- **Permissões**: só se força o que é oferecido; uma ferramenta que a pessoa pôs em "nunca" ou desligou não é oferecida pelo
  portão, não é forçada e, por defesa em profundidade, qualquer chamada a ela é recusada na execução. A aprovação de
  `fluxos.concluir` continua sendo da pessoa.
- **Pedido impossível**: o planejador marca a recusa (`[no_block]`); o motor então não oferece mais ferramentas e, se a
  resposta do modelo não diz que não há bloco, acrescenta a frase "Não há bloco que faça o que você pediu…".
- **Recusas do servidor** (dos 10 fluxos não salvos do C): as duas causas mais frequentes eram (1) um "senão, fim" sem ramo
  aberto, que deixava um bloco de fim solto, e (2) `fluxos.adicionar_bloco` sem `depois_de` num rascunho já terminado, que
  deixava um bloco solto; as duas agora não geram bloco solto (a segunda devolve uma mensagem que diz onde o bloco entra). O
  pedido do d1 também vale para uma pergunta que o modelo fez à pessoa ("perguntar se o ticket é um bug").
- **Mudança de padrão confirmada**: `fluxos.criar` não é mais oferecido por padrão (veja o CHANGELOG, "Mudanças de padrão");
  `KALANGOD_FLUXOS_AUTORIA=classico` o restaura.

### Histograma do D (40 falhas em 111)

12 sem nenhuma chamada de fluxo (o portão não disparou, por exemplo "antes de rodar o agente atualizador, quero ser
consultado…", ou o modelo não chamou `fluxos.planejar` mesmo assim), 4 não salvos depois de recusas, 21 salvos com um bloco
que faltava, 3 impossíveis sem a frase. Os 21 blocos que faltavam têm duas causas que o planejador ainda não trata e que
ficam para a próxima rodada (corrigi-las agora invalidaria o D): um passo que junta o julgamento e a ação ("analisar se o
comentário é ofensivo, se sim mostrar aviso") perde a ação, e "pedir aprovação do usuário antes de rodar o agente X" vira só
a aprovação. Nas 3 execuções impossíveis que falharam (Teams e MySQL) o modelo falou do fluxo sem dizer que não havia bloco: ou o planejador não chegou a recusar o passo, ou o motor não pôde acrescentar a frase.

### Contaminação e cuidados

Antes da execução final do D, 5 pedidos dele rodaram em verificações de funcionamento (o portão disparando, uma geração
forçada que demorava) e uma execução parcial foi descartada por um travamento (um servidor de modelo parado, e depois a
geração forçada sem teto); o limiar do d1 veio das 16 mensagens próprias, não do D; nenhuma regra do planejador foi mudada
depois de ver resultados do D. As execuções do C e do D dependem da máquina: um servidor de modelo esquecido de uma execução
anterior deixou uma rodada quatro vezes mais lenta; todas as medições citadas foram feitas com um só servidor de cada modelo.

**O conjunto E não é independente do D.** As correções medidas no E (julgamento+ação e aprovação+ação em um passo, a forma do
fluxo no portão, os serviços sem bloco ditos pelo motor) saíram do histograma de falhas do D, e vários pedidos do E imitam
padrões do D (d1 + notificação, aprovação antes de um agente, transcrever e passar ao agente); portanto o 62 % do E não é uma
estimativa em pedidos nunca vistos, e o ganho que ele mostraria é, se algo, otimista. A taxa de falsos positivos do portão de
intenção (mensagens comuns tratadas como pedido de fluxo) **não foi medida** em uso; só existe o conjunto de 29 pedidos comuns do
teste de unidade `one_off_requests_are_not_flow_requests`, que cobre as regras e não o d1.

### Modelo maior para criar fluxos

Um modelo maior (Qwen3-8B, Apache-2.0, opcional) não está baixado nesta máquina (só LFM3 e o d1), então **não foi medido nem
implementado**: o motor carrega um único modelo de texto, e usar outro só para as chamadas de `fluxos.*` pediria dois
servidores de texto e a escolha do modelo por ferramenta, o que não é pequeno. O que ele acrescentaria, pelo histograma: os
12 pedidos em que o modelo não chega ao planejador e os 21 fluxos incompletos são falhas de decomposição do pedido, que é
onde um modelo maior ganha; as palavras de bloco que faltam não mudam. É a próxima alavanca depois das duas correções do
planejador acima.

## Correções do histograma do D e o conjunto E (2026-11-25, terceira parte)

Resultado: **E, 37 pedidos novos congelados antes de qualquer execução, 69/111 (62 %), intervalo de 95 % de Wilson 53 %–71 %**,
no serviço real, com o código `cc75f10`. Junto do D (71/111, 64 %, 55 %–72 %): as correções abaixo **não** subiram o
número, e o alvo de 90 % continua longe.

### O que mudou antes de medir o E

> Parte **substituída** pelas seções "Correções da revisão cega" e "Segunda verificação" no fim: o portão por forma e vocabulário, "nenhuma ferramenta é oferecida" e "o `editar` aceita um id que não existe" descrevem o código medido no E (`cc75f10`), não o atual.

- Um passo que junta o julgamento e a ação ("analisar se o comentário é ofensivo, se sim mostrar aviso"; "julgar … e, sendo,
  mostrar um aviso") agora mantém a ação como o ramo "sim" da decisão; "pedir aprovação antes de rodar o agente X" vira a
  aprovação e o bloco do agente; "julgar" pede o d1.
- O portão de intenção reconhece também a forma de um fluxo sem a palavra "fluxo": dois ou mais tipos de bloco na frase
  (agente, aviso, aprovação, decisão, transcrição, pergunta, espera, mensagem), ou um tipo com um conector (depois, se,
  para cada, antes de, sempre que…), a não ser que a frase diga "agora"; o d1 só desempata o que sobra.
- Serviços que nenhum bloco alcança (Teams, banco de dados, ligação para o telefone, além dos de antes) são nomeados pelo
  motor na mensagem da pessoa: nesse caso nenhuma ferramenta é oferecida e a resposta diz que não há bloco, mesmo que o
  modelo nunca chame o planejador.
- Teste de ponta a ponta com o modelo real: com `fluxos.planejar` e `fluxos.concluir` em "nunca", o portão não os oferece
  nem os força; e a recusa de uma chamada a uma ferramenta em "nunca" ou desligada é uma função só (`refuse_off_tool`),
  testada. Descoberta do teste: o modelo, sem essas duas, tenta `fluxos.editar` com um id inventado, que cria um fluxo novo;
  isso continua pedindo a aprovação da pessoa (o `editar` aceita um id que não existe).
- 16 mutantes das regras novas (a divisão julgamento+ação, aprovação+ação, "se sim X", "julgar", o limiar do d1, a forma do
  fluxo, o "agora", os serviços sem bloco, a recusa de "nunca", a oferta de ferramentas em "nunca") morrem nos testes; dois
  sobreviveram na primeira rodada (telefone e "um tipo com conector") e ganharam testes.

### Histograma do E (42 falhas)

12 sem nenhuma chamada de fluxo (pedidos como "antes de o agente apagador rodar, tenho que dar o ok", 3 de 3, e
"para cada trecho de código … o d1 decide …"), 6 não salvos depois de recusas, 20 salvos com um bloco faltando (7 de agente,
5 de condição), 4 impossíveis sem a frase ("manda uma mensagem pelo Signal", que não está na lista de serviços).

Achado depois da medição, corrigido sem nova medição (por isso o E acima vale para `cc75f10`): a opção `unsupported` do d1
recusava passos como "enviar o texto ao agente secretário" (3 de 3 no pedido de transcrever e enviar ao agente); agora o d1 não
pode recusar um passo que nomeia um agente ou um fluxo. O mesmo defeito aparecia no D (transcrever e passar ao agente).

### Para o mantenedor decidir

O que sobra não é mais mapeamento de palavras: são pedidos em que o LFM3 de 1,7 bilhão não decompõe a frase em blocos (agente
que falta, condição que não aparece, blocos que o modelo planeja e não conclui). Um modelo de texto maior só para criar
fluxos (Qwen3-8B, Apache-2.0, opcional, não baixado nesta máquina) é a alavanca que resta; não foi implementada nem medida.

## Summary (English)

The 1.7B local model cannot reliably write a whole flow graph as one JSON. KalangoOS helps it in three ways, measured
with a benchmark of 12 Portuguese requests times 3 runs against the real model (41 % before, 88 % after): a JSON
schema that makes unknown block types unwritable, a staged draft (plain-text steps mapped to blocks by fixed rules)
and deterministic suggestions. Recipes with slots were tried and removed: they did not help. Every path ends in the same approval that shows every block and connection; drafts are per
conversation and bound to what the person saw. The small model still fails on multi-branch flows and on requests that
need a block that does not exist; the visual editor is the place for large or delicate flows, and a bigger text model
(Qwen3-8B, optional) does better.

### Update 2026-11-25 (English)

The d1 now reads the kind of each draft step (an advisory hint the planner may refuse), the person's own request can turn a
word search into the decision block when they name the d1, agents can declare typed answer fields (`output_fields`), and
flows are authored only through the draft tools by default. The latest measurements against the real service, 3 runs per
request: set D (37 fresh requests) 71/111 (64 %, Wilson 95 % interval 55 %–72 %) and set E 69/111 (62 %, 53 %–71 %); E is
not independent of D (its fixes came from D's failure histogram and its requests mimic D's patterns), and 74/111 (67 %) on
set C was an earlier step. The 90 % target was not reached. The remaining failures are the 1.7B model not decomposing the
request into blocks or not calling the planner; the intent gate's false-positive rate was measured only by a 29-request unit
probe over its keyword rules, not in use. A larger authoring model is the lever left. In 60 hand-written classification cases the d1 scored 60/60, free text from the 1.7B model
54/60 and enum-constrained JSON 42/60.

### Correções da revisão cega (6e36ffb → esta), não medidas de novo

Sem novo benchmark: os números do D e do E acima são de antes destas correções e **não foram refeitos**. Mudou: `fluxos.editar`
sem id, com id vazio ou com um id que nenhum fluxo tem agora é recusado (antes criava um fluxo novo sem como desfazer); a regra do
portão "um tipo de bloco com um conector" foi estreitada para estrutura (esperar, aprovação, decisão ou transcrição com outro
bloco; laço; gatilho ou recorrência no começo da frase), porque "Resuma e depois me avise" ou "Pergunte ao agente tradutor se
ele sabe francês" não são fluxos, e o d1 desempata o resto; os serviços sem bloco só entram na resposta quando a frase cita um
fluxo e nenhum fluxo foi criado, sem tirar ferramentas; o código `[no_block]` fica no começo da recusa do planejador e o texto
do passo (que o modelo controla) não pode imitá-lo; o d1 não substitui o que as palavras do passo dizem (esperar, aprovação,
laço…).

### Segunda verificação (a seguir ao de01740), também sem nova medição

- A opção `unsupported` do d1 recusa um passo **somente quando nenhuma regra de palavras o mapeia** (e ele não nomeia um agente nem
  um fluxo): "esperar 5 segundos", uma aprovação, um laço ou uma transcrição nunca são recusados por ele. (A frase acima, "o d1 não
  substitui o que as palavras do passo dizem", vale para os tipos `decide`, `agent`, `notify`… e agora também para `unsupported`.)
- O portão de intenção sem palavra de fluxo ficou só com a recorrência ("toda vez que", "todo dia", "toda segunda", "a cada",
  "sempre que", menos "sempre que possível") junto de um bloco; tudo o mais que parece uma automação ("Aguarde 5 segundos e me
  avise", "Se o agente achar erros, me avise") é deixado ao d1, e uma frase cuja primeira oração termina em "?" é pergunta
  ("Todo mundo precisa aprovar isso? me avise"). As regras só mandam direto ao planejador com a palavra de fluxo ou uma
  recorrência: o d1 (5 % de "fluxo") responde pelo resto, e a quanto isso custa em fluxos reconhecidos não foi medido.
- Quando o d1 (ou as regras) tomam a frase por pedido de fluxo e ela cita um serviço sem bloco ("Mande um SMS toda hora"), a
  resposta diz que não há bloco se nenhum fluxo foi criado, tenha ou não a palavra "fluxo".
- `fluxos.editar` recusa também quando o serviço falha ao ler o fluxo (teste com falha transitória).
