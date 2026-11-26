# Gatilho de entrada pela internet

Um endereço público que inicia um fluxo, como o webhook do n8n. Alguém de fora (um serviço, um formulário, outro
computador) chama uma URL e um fluxo do Kalango Fluxos roda. O caminho até o computador é um túnel da Cloudflare
(`cloudflared`, o mesmo que o Kalango Remote já leva na imagem).

> **English summary.** A signed public URL starts a flow. A small receiver listens on a loopback port and answers only
> the paths of the enabled triggers (everything else is an empty 404). Each call carries `X-Kalango-Timestamp` and
> `X-Kalango-Signature` (HMAC-SHA256 over `timestamp.length.nonce.body`, ±5 min, replays refused, also across restarts). The body is
> external data: the flow runs at the lowest kalangod level unless the person pre-approved a higher one for that
> trigger, and nothing from the body can become an argument of an action that changes the system. The tunnel is run by
> the person's own service (`kalango-fluxosd`), opened only after an approval on kalangod's screen that lists what
> becomes reachable, through which provider and which triggers; there is a kill switch, an idle close for quick tunnels
> and a state file the shell bar can read. Traffic passes through Cloudflare. Other providers are not included.

## Como funciona

```
quem chama ──HTTPS──▶ Cloudflare ──▶ cloudflared (filho do kalango-fluxosd) ──▶ 127.0.0.1:PORTA (receptor)
                                                                                    │ confere tudo
                                                                                    ▼
                                                                    fluxo inicia no nível mais baixo
```

- **O receptor** (`fluxos-internet`, dentro do `kalango-fluxosd`) é um servidor HTTP/1.1 pequeno (hyper) preso a
  `127.0.0.1`. A porta é sorteada na primeira vez e guardada; o receptor só sobe quando há um gatilho ligado.
- **Cada gatilho** tem um caminho `/h/<48 dígitos hexadecimais>` (192 bits sorteados do kernel), uma chave HMAC própria
  (256 bits) no cofre do sistema (`gatilho.<id>.segredo`; o banco guarda só o nome), métodos, tamanho máximo do corpo,
  chamadas por minuto, tipos aceitos, tipo de resposta, lista de endereços (opcional), nível e limite da fila.
- **O túnel** é aberto por `kalango-fluxosd tunel abrir` (ou pela ferramenta `tunel.abrir` de um agente). Pede
  aprovação, sobe o `cloudflared` como filho da sessão da pessoa e mostra o endereço público.

### Onde mora o gerenciador de túneis, e por quê

No **serviço da pessoa (`kalango-fluxosd`)**, num módulo próprio (`fluxos-internet::tunnel`), e não no `kalangod`:

1. O `kalangod` é privilegiado; um processo que fala com a internet não deve ser filho dele. O `cloudflared` roda com o
   uid da pessoa, como o Kalango Remote já faz, e morre junto com a sessão (`PR_SET_PDEATHSIG`).
2. Quem recebe as chamadas (o receptor) e o túnel são a mesma peça: o serviço sabe a porta, quais gatilhos estão
   ligados (para mostrar na aprovação), e vê quando uma chamada foi aceita (para o fechamento por ociosidade).
3. O cofre da pessoa (onde está o token do túnel nomeado) é lido pelo serviço, no uid dela.
4. O `kalangod` fica com o que só ele faz bem: **perguntar à pessoa e escrever a Linha do tempo**. Antes de subir o
   túnel, o serviço pede `tunel.autorizar` ao `kalangod`, que mostra o texto exato e registra o pedido e a resposta.

### Como assinar um pedido

Cabeçalhos: `X-Kalango-Timestamp` (segundos desde 1970), `X-Kalango-Signature` (`sha256=` mais 64 dígitos
hexadecimais **minúsculos**; nenhuma outra grafia é aceita) e, opcionalmente, `X-Kalango-Nonce` (até 128 caracteres,
de uso único; quando existe, entra na assinatura). A assinatura é o HMAC-SHA256, com a chave do gatilho, de
`TIMESTAMP + "." + TAMANHO + "." + NONCE + "." + CORPO`, em que TAMANHO é o número de **bytes** do nonce (sem nonce: `0` e nada
entre os pontos, ou seja `TIMESTAMP + ".0.." + CORPO`). O tamanho faz a mensagem ter uma só leitura: um corpo que comece como
um nonce (`nonce.corpo`) não pode reaproveitar a assinatura de outro pedido. (Mudou na rodada 7: a forma antiga
`TIMESTAMP.CORPO` / `TIMESTAMP.NONCE.CORPO` não é mais aceita.)

```sh
# curl + openssl  (a chave vem do ambiente; nunca a escreva na linha de comando)
BODY='{"pedido": 7}'
TS=$(date +%s)
SIG=$(printf '%s.0..%s' "$TS" "$BODY" | openssl dgst -sha256 -hmac "$KALANGO_SEGREDO" -hex | sed 's/^.* //')
curl -sS -X POST "https://EXEMPLO.trycloudflare.com/h/SEU_CAMINHO" \
  -H 'Content-Type: application/json' -H "X-Kalango-Timestamp: $TS" -H "X-Kalango-Signature: sha256=$SIG" \
  --data-binary "$BODY"
```

```python
import hashlib, hmac, os, time, urllib.request
body = b'{"pedido": 7}'
ts = str(int(time.time()))
sig = hmac.new(os.environ["KALANGO_SEGREDO"].encode(), ts.encode() + b".0.." + body, hashlib.sha256).hexdigest()
req = urllib.request.Request("https://EXEMPLO.trycloudflare.com/h/SEU_CAMINHO", data=body, method="POST", headers={
    "Content-Type": "application/json", "X-Kalango-Timestamp": ts, "X-Kalango-Signature": "sha256=" + sig})
print(urllib.request.urlopen(req).read().decode())
```

```js
import { createHmac } from "node:crypto";
const body = JSON.stringify({ pedido: 7 });
const ts = String(Math.floor(Date.now() / 1000));
const sig = createHmac("sha256", process.env.KALANGO_SEGREDO).update(`${ts}.${body}`).digest("hex");
const r = await fetch("https://EXEMPLO.trycloudflare.com/h/SEU_CAMINHO", {
  method: "POST",
  headers: { "Content-Type": "application/json", "X-Kalango-Timestamp": ts, "X-Kalango-Signature": `sha256=${sig}` },
  body,
});
console.log(r.status, await r.text());
```

`kalango-fluxosd gatilho mostrar ID` imprime o endereço e um `curl` já assinado (vale 5 minutos e uma vez; a chave não
aparece). `gatilho mostrar ID --segredo` mostra a chave.

### Respostas

| Código | Quando |
|---|---|
| 202 `{"run_id": …}` | começou (o padrão: responde na hora) |
| 200 `{"status":"done","output":…}` | o gatilho espera o resultado e o fluxo terminou a tempo (senão 202; 422 se falhou) |
| 400 / 415 / 413 / 408 | JSON inválido / tipo não aceito ou corpo que não é texto / corpo grande demais / cliente lento |
| 401 | sem assinatura, assinatura errada, data fora da janela de ±5 min |
| 403 | endereço fora da lista |
| 404 sem corpo | qualquer caminho que não seja de um gatilho **ligado** |
| 405 / 409 / 429 | método não aceito / pedido repetido (ou fluxo desligado) / limite de chamadas ou fila cheia |

## Limites (padrões)

Corpo até 64 KiB (no máximo 1 MiB); 30 chamadas por minuto por gatilho; 20 execuções esperando por gatilho; 200 no
total; 64 conexões; 10 s para os cabeçalhos e 15 s para o corpo (cliente lento é cortado); cabeçalhos até 16 KiB;
sem CORS; sem Keep-Alive; 150 s no máximo por conexão. Além disso valem os limites de passos e de execuções de agentes
que cada fluxo já tem.

## Modelo de segurança: os 8 riscos aceitos

| Risco (ROADMAP) | O que o código faz | Teste |
|---|---|---|
| 1. Qualquer um na internet pode tentar a URL | caminho de 192 bits; 404 sem corpo para tudo o que não é gatilho ligado; chave e HMAC por gatilho, comparação em tempo constante; liga/desliga individual (criado **desligado**; o receptor confere a cada pedido que a linha do gatilho existe e está ligada, e na dúvida responde 404; as portas genéricas `triggers.save`/`triggers.delete` não criam, não sobrescrevem nem apagam um webhook da internet; chaves de gatilhos que sumiram são varridas do cofre); lista de endereços opcional (o cabeçalho `CF-Connecting-IP` só vale com o nosso túnel aberto); "público, sem assinatura" só por escolha explícita, só no nível mais baixo, sem devolver o resultado | `unsigned_is_401…`, `wrong_signature…`, `a_disabled_trigger_and_an_unknown_path…`, `the_address_list…` (2), `no_cors…`, `an_unsigned_trigger_is_off_by_default…`, `sign::*` |
| 2. Repetir ou falsificar um pedido | janela de ±5 min; cache de assinaturas (e de nonces) limitado na memória e gravado no banco por gatilho, só de pedidos já verificados; uma só grafia da assinatura é aceita; o nonce entra na assinatura | `an_old_or_future_timestamp…`, `a_repeated_request_is_409…`, `a_nonce_is_used_once…`, `replays_are_still_refused_after_the_service_restarts`, `guard::*` |
| 3. Abuso e negação de serviço | corpo limitado (antes e durante a leitura), chamadas por minuto, fila por gatilho e global (429), conexões, cabeçalhos e tempos; os limites do fluxo | `a_body_over_the_limit…`, `too_many_calls…`, `a_full_queue…`, `a_slow_client…`, `huge_headers…` |
| 4. Instruções escondidas no corpo | o corpo vira entrada do fluxo **dentro do envelope de dado externo**; a execução tem um teto de nível (o mais baixo, salvo aprovação prévia e explícita, mostrada na configuração); ferramenta acima do teto nem é oferecida ao agente nem roda num bloco de ação, de máquina isolada (sandbox, e os blocos de dentro dela herdam o teto) ou de "falar"; "transcrever" e "descrever imagem" recusam um caminho que veio do pedido; nada que veio do corpo vira argumento de ferramenta que não é só de leitura, e a conferência vale para o texto já decodificado do JSON (barras, aspas, \\n, \\u) e para os outros valores; ferramentas HTTP sempre perguntam | `the_body_is_external_content…`, `external::*` (inclusive `json_escapes_do_not_hide_a_value…`), `external_gate.rs` (5), e os testes com o modelo real (`injection_real.rs`) |
| 5. Depender da Cloudflare | a aprovação diz que o tráfego passa por ela e que o túnel rápido não tem garantia; o webhook local continua; outros provedores entram depois | `the_tunnel_approval_says…` (kalangod) |
| 6. Vazamento do token do túnel nomeado | token só no cofre; lido na hora de abrir; vai ao filho **só em arquivos 0600 numa pasta 0700 que some ao fechar** (credenciais e `config.yml` com a rota única), nunca na linha de comando nem no ambiente; removido de qualquer linha de saída guardada; `kalango-fluxosd tunel token` lê da entrada padrão | `the_named_tunnel_runs_from_private_files…`, `an_error_never_carries_the_token_or_its_secret`, `the_secret_is_in_the_vault_only…` |
| 7. Cadeia de suprimentos do `cloudflared` | só roda o binário com a versão e o SHA-256 fixados (2026.10.7, os mesmos do Remote); nada é baixado em tempo de execução; se faltar, o erro diz como obter | `only_the_pinned_cloudflared_runs`, `a_missing_cloudflared_says_how…` |
| 8. Esquecer o túnel aberto | `tunel.json` (estado para a barra) sempre atualizado; interruptor geral; o túnel rápido fecha sozinho depois de 2 h sem chamadas aceitas (ajustável); o filho morre com a sessão | `a_quick_tunnel_closes_by_itself…`, `a_quick_tunnel_opens_shows…`, `a_cloudflared_that_dies…` |

O corpo de uma chamada fica no registro da execução e nos passos dela por 24 h (depois, só o hash e o tamanho); o **registro de chamadas**
(`internet_calls`) guarda só o estado, o tamanho, um trecho do hash do corpo e um hash do endereço de quem chamou.

## Privacidade

O tráfego passa pela Cloudflare: ela vê os endereços, os cabeçalhos e os corpos (o HTTPS termina lá). O túnel rápido
(`trycloudflare.com`) é para testes e não tem garantia de disponibilidade. Quem não quiser isso não abre túnel e fica
com o webhook local. O Cloudflare Access (verificação de identidade antes de chegar ao túnel) é configurado no painel da
Cloudflare e **não é verificado aqui**: o que o KalangoOS confere é a lista de endereços e a assinatura.

## Túnel nomeado (domínio próprio)

1. **Só túneis gerenciados localmente.** Crie o túnel com o próprio `cloudflared` (`cloudflared tunnel login`,
   `cloudflared tunnel create NOME`, `cloudflared tunnel route dns NOME hooks.example.test`) e guarde o token dele; **não**
   crie o túnel pelo painel da Cloudflare, onde as rotas ficam no painel e a Cloudflare pode empurrar uma configuração
   que substitui a local ("Updated to new configuration"). O KalangoOS serve só o receptor deste computador e responde
   404 a todo o resto; se o `cloudflared` disser que recebeu uma configuração do provedor, o túnel é fechado na hora e a
   barra diz por quê. (Não foi possível testar contra um túnel de verdade da Cloudflare: só com um `cloudflared` de teste.)
2. `printf '%s' "$TOKEN" | kalango-fluxosd tunel token` guarda o token no cofre (entrada padrão, nunca argumento).
3. `kalango-fluxosd tunel abrir --dominio hooks.example.test`.

## Comandos

```
kalango-fluxosd gatilho criar --fluxo ID [--nivel usuario] [--ips 203.0.113.0/24] [--corpo-max 65536] [--por-minuto 30] [--espera 20]
kalango-fluxosd gatilho ativar|desativar|apagar|mostrar [--segredo]|girar-segredo ID
kalango-fluxosd gatilho listar | chamadas [ID]
kalango-fluxosd tunel abrir [--dominio HOST] [--ocioso SEGUNDOS] | fechar | fechar-todos | estado | token
```

Ferramentas do `kalangod` (para agentes): `gatilho_internet.listar|mostrar|chamadas|criar|ativar|desativar|girar_segredo|apagar`,
`tunel.estado|abrir|fechar|fechar_todos|autorizar`. Criar, ligar, girar a chave e abrir o túnel **sempre** pedem a
aprovação da pessoa, com o texto exato, sem a opção "sempre permitir"; nenhuma devolve a chave a um agente.

O estado que a barra pode ler está em `$XDG_RUNTIME_DIR/kalango/tunel.json` (`open`, `mode`, `url`, `opened_at`,
`last_activity`, `idle_close_s`, `healthy`, `connections`, `closed_reason`), reescrito a cada mudança.

## Revisão de segurança, rodada 7: o que mudou

- **Um cartão só para ligar um gatilho.** Quando um programa da pessoa liga um gatilho pelo soquete do serviço, o serviço
  pergunta (`gatilho_internet.autorizar_ampliacao`, "enable") e o kalangod mostra agora o mesmo cartão de
  `gatilho_internet.ativar`, montado pela mesma função a partir do que o serviço diz do gatilho naquele momento: assinatura
  (NENHUMA = público, aviso coral), nível e endereço. Antes, esse caminho mostrava um cartão genérico que falava sempre de
  chave, mesmo de um gatilho público.
- **Os fatos do cartão como pré-condição.** Depois do "sim", o kalangod lê o gatilho de novo e só responde que sim se ele
  ainda é o que o cartão mostrou (nível, assinatura, fluxo, lista de endereços, endereço); se mudou, nada é ligado. Os fatos
  voltam na resposta (`esperado`).

## Revisão de segurança, rodada 6: o que mudou

- **Arquivos de transcribe e describe-image.** Em execuções vindas de fora, o arquivo é escrito no próprio fluxo: um nome de
  variável (`x`) ou o campo vazio (a saída anterior) são recusados, além de qualquer `{…}`. O mesmo vale para os arquivos que
  o sandbox de um agente move (`{entrada}` e qualquer outro campo).
- **O que não é coberto (documentado).** Um campo curto do pedido (uma ou duas letras, um número) usado numa chamada de
  ferramenta que o **próprio agente** monta não é pego pela comparação: quem protege é o teto de nível.
- **Segredo na saída do cloudflared.** Uma linha longa é remontada antes de mascarar o token e o segredo: um segredo cortado na
  borda de um bloco de 64 KiB não aparece em dois pedaços.

## Revisão de segurança, rodada 5: o que mudou

- **O soquete do serviço.** Depois de conectar, o kalangod confere pelo kernel (SO_PEERCRED) que quem está do outro lado é o
  usuário da pessoa (o caminho pode ter sido trocado entre olhar e conectar); a resposta lida tem no máximo 1 MiB. O serviço
  só deixa o root chamar métodos `internet.*`.
- **Configuração empurrada.** Qualquer "Updated to new configuration" depois da primeira (a local) fecha o túnel, e a versão é a
  do **fim** da linha (uma configuração empurrada pode conter `?version=0`). O leitor mata o `cloudflared` na hora, sem esperar o
  monitor.
- **Soquete do túnel.** Só o `cloudflared` que esta abertura iniciou (o pid que o kernel informa) pode conectar; outro programa
  da pessoa que ache o soquete é fechado, e o cabeçalho dele nunca é lido.
- **Valores no caminho.** Em execuções vindas de fora, qualquer `{variável}` colada a outro texto num caminho ou comando é
  recusada (uma variável pode carregar o pedido: `set_variables x = {evento…}`, depois `/pasta/{x}`).
- **Depois da resposta da pessoa.** Ampliar ou ligar um gatilho confere de novo, depois de esperar, que nenhum túnel abriu e que o
  gatilho continua o mesmo. Ligar um gatilho pelo soquete cru também pergunta. Os fatos do cartão (nível, assinatura, fluxo, lista
  de endereços) viajam como pré-condições e o serviço recusa um gatilho que já não seja aquilo.
- **Aprovações guardadas** valem para o mesmo processo e a mesma regra que pediram.

## Revisão de segurança, rodada 4: o que mudou

- **Quem fala em nome de um agente.** O kalangod chama o serviço **pelo soquete da pessoa, como root**, e o serviço só aceita o
  campo `creator` vindo do uid 0; nada é executado como root a partir de uma pasta que a pessoa possa escrever. Em uma
  sessão de desenvolvimento (tudo no mesmo usuário) ninguém é confiável e a pessoa é perguntada.
- **Ligar, editar e abrir.** Ligar um gatilho (e ampliar quem pode chamá-lo: tornar público, esvaziar ou alargar a lista de
  endereços) é recusado enquanto o túnel está aberto **ou abrindo** (o cartão na tela); ampliar pede a aprovação da pessoa
  (`gatilho_internet.autorizar_ampliacao`). Depois da resposta, a lista de gatilhos é lida de novo: se mudou, nada abre.
- **Endereço de quem chamou.** O `CF-Connecting-IP` só vale numa conexão que chegou pelo soquete próprio do túnel
  (`unix:` na pasta privada da abertura, que o `cloudflared` recebe como origem); pela porta local nenhum cabeçalho é aceito.
- **Valores curtos.** Um campo do pedido (`{evento…}`, `{entrada_anterior}`) colado a outro texto num caminho ou comando é recusado
  em execuções vindas de fora, mesmo que o campo seja curto ou um número. (Não há lista de exceções por gatilho: um valor que
  é exatamente um campo continua sendo comparado inteiro.)
- **Cartões.** O cartão do túnel diz o nível mais alto dos gatilhos listados e quem pediu (nunca "você"); ligar um gatilho e
  trocar a chave também têm cartão. Uma aprovação de subida de nível só é lembrada depois do "sim". Um pedido já decidido não
  aceita uma segunda decisão. A área de transferência é limpa depois de 30 s (e o token colado, na hora).

## Revisão de segurança, rodada 2: o que mudou

- **Túnel nomeado, do jeito local.** O token vira um arquivo de credenciais (0600, numa pasta 0700 da pasta de execução)
  e um `config.yml` que fixa a ÚNICA rota (o receptor deste computador, `http://127.0.0.1:PORTA`) e responde 404 a todo o
  resto: as rotas do painel do provedor não decidem o que fica exposto. O token não vai nem para os argumentos nem para o
  ambiente do `cloudflared`; a pasta é apagada ao fechar. (Não foi possível testar com um túnel nomeado de verdade.)
- **O programa é uma cópia feita depois da aprovação**, conferida contra a soma fixada a partir dos mesmos bytes que
  foram copiados e executada de uma pasta só da pessoa: trocar o arquivo enquanto a pergunta está aberta não adianta.
- **Uma abertura de cada vez:** a vaga é tomada antes de perguntar; duas aberturas ao mesmo tempo viram uma aprovação e um
  processo. Ligar um gatilho com o túnel aberto é recusado (feche, ligue, abra de novo: a aprovação lista os gatilhos).
- **Nível acima do mais baixo** pede a pessoa também quando ninguém pediu (o soquete do serviço: o `kalangod` mostra o
  cartão). O teto de um agente é o nível mais alto das ferramentas que a pessoa deixa ele usar, não o da regra (que é
  "total" para todos).
- **Conteúdo.** Um valor montado com um campo do corpo (um modelo que cola o campo num caminho), um `..` num caminho e
  uma máquina (sandbox) com seus arquivos e comandos passam pelas mesmas verificações; um agente remoto não roda numa
  execução vinda de fora. Valores que o próprio agente reescreve (base64, outras palavras) estão além da checagem: quem
  protege nesse caso é o teto de nível.
- **Registro.** A taxa do gatilho só é gasta por chamadas autenticadas (uma enxurrada de chamadas ruins não tranca a
  pessoa); antes da assinatura, cada endereço tem o seu orçamento. O cache de repetição tem um teto por gatilho.
- **Conteúdo guardado por 24 h** (configurável, `internet_body_retention_s`): depois o corpo, no registro da execução e
  nos passos dela, vira só o hash e o tamanho. Com `respond: wait` quem chama recebe o resultado do fluxo mesmo no nível
  mais baixo (a configuração e o cartão dizem isso).
- **Chaves.** A chave aparece uma vez, ao criar ou trocar; `mostrar` nunca a mostra. Apagar um fluxo apaga as chaves dos
  gatilhos dele na hora, e o túnel só aceita a entrada do cofre feita para o token (`tunel.<nome>.token`).

## As telas (design "37 · Gatilho pela internet")

- **Um túnel leva vários gatilhos.** O chip da barra diz "túnel aberto · 2 gatilhos" (um túnel, dois gatilhos); o menu
  rápido mostra o endereço uma vez e, abaixo, cada gatilho com a última chamada. "Fechar todos os túneis" fecha esse
  túnel; Ctrl+Shift+F12 fecha sem perguntar.
- **Nome do gatilho.** Opcional; vazio quer dizer o nome do fluxo. É o nome que os cartões, a barra e o registro mostram.
- **Estados.** "Abrindo túnel…" (a barra mostra os segundos; o estado `opening` está em `tunel.json`); "Falhou ao abrir"
  (cloudflared ausente, verificação do programa falhou, não permitido: cada um diz o que fazer); "Limite de chamadas"
  (as recusas 429 do último minuto e um gráfico por minuto); "Gatilho desligado" (desde quando e quantas chamadas
  recusou, 404).
- **Registro de chamadas.** Inclui os endereços errados (404), só como hash de quem chamou e nunca o caminho, no máximo
  20 linhas por minuto para uma varredura não encher o registro. Numa janela estreita (menos de 900 px) o detalhe de uma
  chamada abre como folha lateral de altura inteira.
- **Aprovação "Só no mais baixo".** Quando um agente pede um gatilho com nível acima do mais baixo, o cartão oferece
  aprovar o MESMO pedido com o nível reescrito para o mais baixo: o `kalango-aprovacao` devolve a resposta alternativa
  (um "sempre" que não guarda nada), o `kalangod` roda o pedido reescrito uma vez e a Linha do tempo registra as duas
  coisas (o pedido original aprovado com o nível baixado, e a execução).
- **País ao lado da origem: não existe.** Mostrar o país exigiria uma base de GeoIP, que não vem no sistema e não é
  baixada; a origem aparece só como hash.

## O que não está incluído

- Outros provedores de túnel (Tailscale Funnel, ngrok…): entram depois como adaptadores.
- Verificação do token do Cloudflare Access.
- A interface (veja abaixo): só existe o estado em arquivo, os comandos e as ferramentas.
- Resposta com o resultado para gatilhos públicos sem assinatura (nunca será devolvido).

## Telas que faltam no Design System (FigCraft)

Nada disto foi codificado, porque as telas ainda não existem no desenho:

1. **Configuração do gatilho de entrada pela internet** (no construtor de fluxos): endereço e botão de copiar, chave
   (mostrada uma vez, girar), métodos, limite de corpo e de chamadas, tipos, resposta (na hora ou esperar), lista de
   endereços, **nível** (com o aviso quando for acima do mais baixo), "público, sem assinatura" com aviso forte, ligar e
   desligar.
2. **Estado do túnel na barra** (indicador sempre visível enquanto aberto, rápido/nomeado, endereço, desde quando).
3. **Interruptor "Fechar todos os túneis"** (na barra/painel do indicador e em Configurações).
4. **Cartão de aprovação** de abrir o túnel e de criar/ligar um gatilho (o texto já existe; falta o cartão com a lista
   de gatilhos, o provedor e o aviso de privacidade).
5. **Registro de chamadas** (estado, hora, tamanho, execução aberta), e a tela de ligar o túnel com o token do domínio
   próprio (quando foi criado, apagar).
