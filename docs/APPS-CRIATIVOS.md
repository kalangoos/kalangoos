# Apps criativos (projeto Craft)

Os 17 apps do projeto Craft (PhotoCraft, VectorCraft, FilmCraft, LightCraft, PdfCraft, EffectCraft, AniCraft,
DesignCraft, FigCraft, TextureCraft, MaterialCraft, SoundCraft, WordCraft, DeckCraft, CADCraft, GridCraft e Craft
Anything) entram no KalangoOS pela **Central de Apps › Apps criativos**. São apps livres, feitos em Rust, mantidos pelos
autores deles em [github.com/storytold](https://github.com/storytold) e publicados em
[getartcraft.com/apps](https://getartcraft.com/apps). Estão em alfa: a Central diz isso em cada cartão.

O que decidimos (25/11/2026): os 17 apps; **sem o launcher** do projeto (quem instala e atualiza é a Central); a fonte
são os lançamentos assinados do próprio projeto; atualização automática ligada; rede dos apps desligada; os agentes só
usam os apps pelo kalangod, com níveis por ferramenta.

O código: `crates/kalango-craft` (catálogo, verificação, instalação, níveis, cliente MCP), `crates/kalango-shell/src/
criativos.rs` (as telas e a atualização automática), `crates/kalangod/src/tools/craft.rs` (as ferramentas dos agentes)
e o cartão "craft" da tela de aprovação (`crates/kalango-aprovacao`). As telas estão no Design System, página
"36 · Central · Apps criativos".

## Fonte e verificação

O projeto publica cada app como imagem OCI de Flatpak no `ghcr.io/storytold/<app>`, um índice em
`https://getartcraft.com/index/static` e, para cada imagem, uma assinatura OpenPGP no formato "atomic container
signature" do containers/image em `https://getartcraft.com/oci-signatures/storytold/<app>@sha256=<hex>/signature-1`.

A chave do projeto é "ArtCraft Linux Release Signing Key", primária
**`2B76 741B 414A AD6C D33E  6312 13A0 05C0 B9D7 C3ED`** (ed25519, só certifica; a subchave
`568D C1F6 8F8F AECC 797F  3C7B 006D AF6B B0EF 334E` assina). Conferimos em dois lugares independentes: é o `GPGKey` de
`getartcraft.com/artcraft-oci.flatpakrepo` e é a impressão digital fixada no código do launcher
(`craft-launcher/crates/native/src/nightly_trust.rs`). A chave vai no nosso catálogo
(`crates/kalango-craft/catalogo/artcraft-release.gpg`), embutida no binário, e a impressão digital fica fixada no
catálogo (`apps-criativos.json › fonte.impressao_digital`).

Por que a Central verifica, e não o Flatpak: o Flatpak do Ubuntu 24.04 (1.14) não confere assinaturas de remotos OCI —
o `SignatureLookaside` do `.flatpakrepo` só existe a partir do Flatpak 1.17/1.18 — e, nos nossos testes, ele travou
baixando o índice desse remoto. Então a cadeia é nossa, e nada entra no sistema antes de ela fechar:

1. o índice diz o digest da imagem atual de cada app (o índice não é assinado: ele só aponta);
2. a assinatura dessa imagem é conferida pelo `gpgv` com a chave do catálogo, numa pasta temporária nova (0700); a
   chave primária que assinou tem de ser a fixada; o texto assinado tem de nomear exatamente esse digest e
   `ghcr.io/storytold/<app>`;
3. o manifesto baixado do registro tem de ter esse sha256; a configuração e cada camada, os sha256 do manifesto;
4. a configuração assinada dá o ref do Flatpak (tem de ser `app/<id-do-app>/<arquitetura>/stable`), o commit da
   construção, os metadados (`org.flatpak.metadata`) e a versão (do appdata dela). A versão que vale é a assinada: se o índice anunciar outra, a instalação é recusada;
5. os passos 1 a 4 e a importação rodam **no kalangod** (`kalango/craft` › `preparar`), não na sessão da pessoa: as
   camadas conferidas viram um commit num repositório do Flatpak onde só o kalangod escreve
   (`/var/lib/kalango-apps-criativos/repo`, legível por todos; `flatpak build-import-bundle --oci`), e o kalangod
   confere que os metadados do commit importado (as permissões que o app pede) são os da configuração assinada
   (se não forem, a referência volta ao que era e a importação é recusada), e guarda no estado dele
   (`/var/lib/kalango/craft/<uid>-verificados.json`) qual commit verificou. O id do commit importado **não** é o
   `org.flatpak.commit` da configuração: o `build-import-bundle` faz um commit novo das camadas (mesma data, outro id;
   medido com o AniCraft 0.2.1), então a conferência do id não existe; vale a das camadas pelo sha256 do manifesto
   assinado e a dos metadados; Um programa da
   pessoa não consegue pôr nada nesse repositório nem trocar essa lista;
6. a Central instala dele **exatamente esse commit** na instalação da própria pessoa (`flatpak --user`, remoto
   `kalango-apps-criativos`, `file://`, sem GPG do Flatpak de propósito: o repositório é só de leitura para ela e a
   instalação é presa ao commit; um remoto que alguém aponte para outro lugar volta ao certo). Depois, o commit
   instalado tem de ser o verificado; se não for, o app é removido e a tela mostra "Assinatura não confere". O kalangod
   só aceita "instalado" para o commit que ele preparou.

Qualquer elo que não bate é o estado "Assinatura não confere: instalação recusada. Nada foi alterado no sistema." da
tela de atualizações. O runtime (`org.freedesktop.Platform` 25.08 ou 26.08, conforme o app) vem do Flathub, verificado
pelo Flatpak com a chave do Flathub.

Uma imagem assinada **mais velha** que a instalada é recusada (sem voltar atrás escondido); voltar só pela escolha da
pessoa ("Desfazer", abaixo).

## Atualizações

- **Ligada por padrão**, geral (no topo da coleção) e por app (no detalhe). A Central procura em segundo plano a cada
  6 horas (a primeira, 3 minutos depois de a sessão abrir) e em "Procurar agora".
- Uma versão nova é verificada e baixada em segundo plano; se o app está aberto, ela espera ("v0.24.0 pronta · atualiza
  quando você fechar o app") e entra quando ele fecha, ou já com "Reiniciar app agora".
- **Ponto de restauração** (decidido: as duas versões anteriores + Desfazer): quando a Central avisa que instalou
  uma versão nova, o kalangod passa a de antes para a lista protegida dele e a mantém no repositório
  (`refs/heads/kalango-anterior/<uid>/<app>/<n>`, as duas últimas; uma pasta por pessoa e app, então guardar os pontos de
  um nunca apaga os de outro). Os commits **preparados** e ainda não instalados (até 3 por pessoa e app) também têm
  referência (`kalango-preparado/<uid>/<app>/<n>`) e ficam na lista do kalangod: um segundo "Procurar agora" durante uma
  atualização pendente não troca o commit que a Central vai instalar. "Desfazer (voltar ao ponto de restauração)" em Recentes,
  ou "Voltar para v…" em Ajustes › Apps, pede ao kalangod o commit anterior (só um que ele verificou), instala
  exatamente ele (`flatpak update --commit`), confere, e para a atualização automática daquele app até a pessoa
  religá-la (senão ele voltaria para a nova). A tela diz isso onde a atualização se liga: "Antes de cada atualização,
  a versão em uso fica guardada (as 2 últimas); Desfazer volta a ela".
  Os pontos de restauração do sistema (`pontos.criar`) guardam o `/etc` e os pacotes: não cobrem um app da instalação
  da pessoa, por isso o ponto do app é a versão anterior guardada pelo kalangod.
- **Bloquear uma versão**: o nosso catálogo não segura atualizações; ele só pode bloquear uma versão conhecida como
  ruim. Em `crates/kalango-craft/catalogo/apps-criativos.json`, no app, `"bloqueadas": [{"versao": "0.25.1", "motivo":
  "falha conhecida ao exportar"}]` (ou `"digest": "sha256:…"`). A versão comparada é a da configuração assinada. A tela
  mostra "Esta versão foi bloqueada pelo KalangoOS: ficou na anterior (v…). Motivo: …". O bloqueio chega com uma
  atualização do KalangoOS (o catálogo vai no binário; ninguém o troca por um arquivo da pasta da pessoa).
- Sem espaço (download duas vezes + instalado + 200 MB), a instalação nem começa: "Sem espaço: faltam … no disco do
  sistema".

## Permissões de cada app

Os apps pedem, no Flatpak deles, rede, `org.freedesktop.Flatpak` (o `flatpak-spawn --host` que o atualizador deles
usa para rodar programas fora da caixa) e pastas amplas (o FilmCraft pede a pasta pessoal inteira, `/media` e `/mnt`).
O kalangod troca isso por `flatpak override --user` (a Central pede, depois de instalar e quando a pessoa muda algo;
o kalangod apaga o override do app e o refaz do estado dele, e faz de novo antes de iniciar o servidor MCP do app, para
um override alargado por outro programa não valer). Sem o kalangod, a Central só aplica as restrições (sem rede, sem
pasta nenhuma) e mostra o erro:

- **rede desligada** (`--unshare=network`); a pessoa liga por app ("Permitir rede"). A Central atualiza o app mesmo
  com a rede dele desligada (quem baixa é ela);
- **sem o serviço do Flatpak** (`--no-talk-name=org.freedesktop.Flatpak`): quem atualiza é a Central;
- **sem acesso amplo ao computador**: `--nosocket=` `session-bus`, `system-bus`, `ssh-auth`, `gpg-agent`, `pcsc` e `cups`;
  `--nodevice=` `all`, `kvm` e `shm` (só `dri`, a GPU, fica); `--disallow=` `devel`, `multiarch`, `bluetooth` e `canbus`;
  `--no-talk-name=` para os chaveiros, o systemd, o login e o PolicyKit e para **cada nome** que o app pede em
  `[Session Bus Policy]` (e `--system-no-talk-name=` em `[System Bus Policy]`), menos `org.freedesktop.Notifications`.
  O Flatpak 1.14 não aceita `*` em `--no-talk-name`, por isso os nomes vêm dos metadados do app. A tela, o som e a
  GPU ficam (a Central abre a janela do app); o servidor sem janela tira também tela e som, e repete as negações na
  própria chamada (`flatpak run`). Conferido com o `flatpak` de verdade numa instalação de teste
  (`flatpak override --show`, teste `the_effective_override_denies_the_wide_access_a_creative_app_asks_for`);
- **só as pastas escolhidas**: fora `host`, `home`, `/media`, `/run/media`, `/mnt` e a pasta de versões do launcher;
  ficam as pastas do catálogo para cada app (Vídeos para os de vídeo, Imagens para os de imagem…), e a pessoa tira ou
  põe outras ("+ pasta", só dentro da pasta pessoal).

Remover o app tira o app (e, se a pessoa quiser, os dados dele em `~/.var/app`); os projetos nas pastas dela nunca são
apagados.

## Ajustes › Apps

Na Central de agentes, Ajustes lista os apps criativos instalados; cada um abre a página dele (design 346774): Arquivos
(as pastas que ele vê; "Alterar" leva ao detalhe na Central de Apps), Rede, Câmera e microfone (os portais pedem na
hora; "Alterar" abre Privacidade), Agentes podem usar, Atualização automática, Ponto de restauração ("Voltar para
v…"), "Abrir na Central" e Remover (só o app, ou com os dados em `~/.var/app`; os projetos nas pastas da pessoa nunca
são apagados). Os controles são os mesmos do detalhe: o mesmo estado (o do kalangod e o da Central).

## Agentes: só pelo kalangod

Cada app traz um servidor MCP (`<app>-cli mcp`). Os agentes não o iniciam: o **kalangod** é o único caminho, com as
ferramentas `craft.apps` (os apps instalados, se os agentes podem usar cada um e, com `ferramentas`, as ferramentas de
cada app com o nível que vale), `craft.chamar` (`app`, `ferramenta`, `argumentos`, `janela`) e `craft.encerrar`.

- O kalangod inicia o servidor como a pessoa que chamou (o uid dela), **dentro do Flatpak do app** (`flatpak run
  --command=<app>-cli`), sem rede, sem o serviço do Flatpak, sem pastas amplas e, sem janela, sem tela nem som. Os apps
  que aceitam (`--automation-read-root` / `--automation-write-root`) recebem as pastas escolhidas como raízes: os
  comandos do app só leem e escrevem nelas.
- A sessão vive entre as chamadas (o documento do servidor continua) e acaba depois de 10 minutos parada, com
  `craft.encerrar` ou quando a pessoa desliga os agentes daquele app. O interruptor age na hora: o kalangod tira a
  sessão da tabela e mata o servidor mesmo com uma chamada em andamento (que volta com erro), sem esperar o prazo de
  120 s; a tabela de sessões nunca fica trancada durante um início ou uma chamada.
- **Com janela** (`janela: true`), o kalangod abre a janela do app e o servidor no mesmo sandbox (`sh -c '<app>
  --control <porta> & exec <app>-cli mcp --bridge <porta>'`); a ficha de controle é gerada pelo kalangod a cada vez e
  vai só no ambiente desses dois processos (nunca num arquivo nem nos argumentos). Uma janela que a pessoa abriu
  sozinha não é alcançada (a rede de cada sandbox é só dela). Risco que fica: um processo da mesma pessoa que leia o
  ambiente desses processos em `/proc` veria a ficha; os comandos dos agentes rodam com Landlock e seccomp fora disso.
- Em Agentes › Ajustes, os Apps criativos aparecem como um conector embutido (as ferramentas `craft.*`).
- **"Agentes podem usar este app"** e **as pastas que o servidor do app vê** (no detalhe do app) ficam no estado do
  kalangod (`/var/lib/kalango/craft/<uid>.json`), mudados só pela interface da pessoa (`kalango/craft`,
  `is_person_ui`); nunca de um arquivo da casa, que os programas da pessoa — os agentes inclusive — podem escrever.
  Cada pasta é conferida ao entrar e de novo a cada sessão (`pasta_segura`), por uma lista do que pode: só as pastas
  de mídia e documentos do XDG (Vídeos, Imagens, Documentos, Música, Downloads) ou uma pasta que a pessoa escolheu;
  resolvida pelo kalangod (links seguidos), uma pasta que existe, dentro da casa (não a própria casa), sem nenhum
  componente com ponto nem de programas (`bin`, `scripts`…) e fora das pastas protegidas do kalangod.
  Até um `XDG_PICTURES_DIR="/"` escrito no `user-dirs.dirs` é recusado; uma pasta que não passa recusa a chamada
  (erro na linha do tempo), nunca é trocada por outra.

### Níveis por ferramenta

Fechado por padrão (`crates/kalango-craft/src/niveis.rs`; os nomes estão no catálogo, nunca no que o app diz de si):

| O que a chamada faz | Nível |
|---|---|
| ler, inspecionar, listar, prévia (`doc_inspect`, `command_list`, `render_preview`…) | livre |
| editar o documento do servidor sem janela com uma ferramenta de edição conhecida (`draw_shape`…) | livre |
| salvar, exportar, importar (`save_file`, `export`, `media_import`, `bounce_mix`…) | pede confirmação |
| qualquer caminho ou endereço nos argumentos (chave que termina em path/file/dir/url/output, ou valor que parece caminho) | pede confirmação, no mínimo |
| editar o documento aberto numa janela | pede confirmação |
| executores de comandos (`command_run`, `command_batch`, `batch`, `anicraft_run`): só os ids da lista conferida do app (`comandos_livres`, com a versão em que foi conferida) são livres; qualquer outro id, ou um id que fale de arquivo, rede ou script | pede aprovação |
| `run_command`, `execute*`, qualquer ferramenta com `script` | pede aprovação |
| controlar a janela (`ui_*`, cliques, teclas, diálogos, captura da janela) | pede aprovação, sem "sempre" |
| ferramenta que o catálogo não conhece (uma nova de uma versão nova) | pede aprovação, sem "sempre" |
| Craft Anything (roda programas para investigá-los) | toda ferramenta pede aprovação |

Os executores de comandos são lidos de forma estrita: `{id|cmd|command, params}` ou `{commands|steps: [...]}`, um id
por passo e nenhuma outra chave (nem `Command`, `name`, `action`, `op`); fora disso, aprovação. Caminhos e endereços são
procurados em todos os valores.

A tela de aprovação (design 346477) mostra o pedido como o design: "<agente> quer salvar ou exportar no <app>", o nível
("Nível: salvar e exportar → pede confirmação") e **O quê** (a ferramenta e o comando), **Para onde** (os caminhos,
"pasta já permitida" quando é uma delas), **Formato** (o pedido ou a extensão) e **Por quê** (o que o agente disse em
`por_que`, mostrado como dado), com a nota de que controlar a janela pediria aprovação separada. A confirmação aceita
"Sempre para este agente" para exatamente aquele pedido; a aprovação não. Toda chamada entra na
linha do tempo do kalangod (com o app, a ferramenta, os argumentos e o nível). As listas `comandos_livres` vêm
vazias para quase todos os apps (cada uma entra no catálogo depois de alguém conferir os comandos daquele app numa
versão); o AniCraft traz 149 ids conferidos na versão 0.2.1. A lista **só vale na versão em que foi conferida**: o
kalangod compara com a versão instalada que ele verificou, e numa versão diferente (a atualização automática trouxe a
0.3) ou desconhecida os mesmos ids voltam a pedir aprovação, e o cartão diz "roda um comando do app de uma lista que o
KalangoOS conferiu em outra versão do app".

Todo texto que veio do agente e aparece no cartão (ferramenta, comando, formato, caminhos, "por quê") passa por
`texto_de_agente`: uma linha, sem quebras, controles nem inversão de direção (U+202E), no máximo 80 a 200 caracteres.

A lista de comandos livres só vale se o commit **realmente instalado** na instalação da pessoa (`flatpak --user info
--show-commit`, como ela) é o do registro `atual` do kalangod; um registro velho ou um app trocado por fora faz os ids
pedirem aprovação, e a versão é conferida de novo com a sessão em mãos, antes de cada chamada. O interruptor "Agentes
podem usar" é conferido dentro de `com_sessao` (a geração é lida antes da conferência; de novo ao entrar a sessão na
tabela e logo antes da chamada), e o sinal que mata o servidor vai por um pidfd (ou, sem ele, sob um cadeado com o
recolhimento do filho), nunca a um número de processo que possa ter sido reaproveitado.

### Estado do kalangod e vários preparos ao mesmo tempo

A lista dos verificados (`<uid>-verificados.json`) e as escolhas da pessoa (`<uid>.json`) são lidas, mudadas e gravadas
de uma vez só sob um cadeado, e o arquivo é trocado por inteiro; um preparo de minutos nunca grava de volta o que leu
antes do download. Cada pessoa baixa numa pasta própria (`craft/baixando/<uid>/`), há um preparo por pessoa e app (o
segundo pedido recebe `em_andamento`), e as alterações do repositório protegido (importar, guardar referências) são uma
por vez.

## Arquivos e ícones

Cada app instala, pelo Flatpak, o próprio `.desktop` (com `MimeType=`), o XML do shared-mime-info e os ícones; a
Central não muda nada disso. Os tipos aparecem em "Abrir com" do Arquivos. Depois de instalar
(`criativos::associar`), a Central lê os tipos do `.desktop` do app: um tipo que um app do KalangoOS (`os.kalango.*`)
abre fica com o app do KalangoOS como padrão (escrito no `mimeapps.list`, para o app novo não tomar o lugar); um tipo
que só os apps criativos abrem (o `.anicraft`, o projeto do FilmCraft…) fica com o app novo, se ainda não houver
padrão.

Na coleção, até o app estar instalado, o cartão mostra as iniciais num marcador neutro (o design); instalado, o ícone
do próprio pacote.

## Kalango Cast → FilmCraft, EffectCraft e SoundCraft (nota de interface)

O Kalango Cast não fala com os apps criativos diretamente: ele pede ao kalangod, como qualquer agente, e só se o app
estiver instalado e com os agentes permitidos (o Cast descobre isso com `craft.apps`; sem o app, o botão não aparece).

1. O Cast termina uma gravação numa pasta que o app vê (por padrão `~/Vídeos/Kalango Cast/<gravação>/`, com as
   faixas ISO e os marcadores).
2. "Abrir no FilmCraft": `craft.chamar {app: "filmcraft", ferramenta: "media_import", argumentos: {"path": "…"}}`
   — importar pede confirmação, uma vez, com o caminho na tela; depois, `command_run` com os ids da lista conferida
   monta a sequência pelos marcadores, sem janela; salvar o projeto (`command_run` de salvar, ou a ferramenta de salvar)
   pede confirmação.
3. Efeitos no EffectCraft e mixagem no SoundCraft seguem o mesmo caminho (`effectcraft`: `open_project` / `render_*`;
   `soundcraft`: `import_audio` → `bounce_mix`), cada exportação com confirmação.
4. O que volta ao Cast são os caminhos exportados (dentro da raiz de escrita) e as prévias (`render_preview`, livres).

Nada disso dá ao Cast acesso além do que um agente tem: as mesmas pastas, os mesmos níveis e a linha do tempo.

## Licenças e marca

Os apps são `MIT OR Apache-2.0`. Dentro deles há a fonte Ubuntu (Ubuntu Font Licence 1.0, já aceita) e partes sob
`BSL-1.0` e `MIT-0`, aceitas pelo mantenedor em 25/11/2026 (estão na lista de `AGENTS.md` e no `deny.toml`). O texto de
cada licença vem no próprio app (`/app/share/doc`).

**Marca**: ArtCraft, Craft e os nomes e logos dos apps são dos autores. O KalangoOS **nunca reempacota, renomeia ou
modifica** esses apps: instala as imagens exatamente como o projeto as assina, com o nome e o ícone deles. O que o
KalangoOS muda são só permissões do Flatpak (override), que a pessoa vê e muda. Se um dia for preciso corrigir um app,
a correção vai para o projeto deles; uma versão modificada por nós teria de tirar o nome e a marca ArtCraft/Craft.

## Estudo (25/11/2026)

- Remoto: `oci+https://getartcraft.com`; índice com 19 repositórios (os 17 apps, o launcher e o ArtCraft); etiquetas
  `stable-x86_64` e `stable-aarch64`; ids `ai.storyteller.<app>`; runtime `org.freedesktop.Platform` 25.08, ou 26.08
  no PhotoCraft, AniCraft, FigCraft, TextureCraft, MaterialCraft, CADCraft e Craft Anything. O primeiro app de cada
  runtime traz o runtime (no teste, o 26.08 com as extensões de GL e codecs ocupou 2,2 GB).
- Tamanhos (download / instalado, x86_64, nesse dia): PhotoCraft 142/277 MB, VectorCraft 160/322, FilmCraft 112/214,
  LightCraft 123/252, PdfCraft 133/275, EffectCraft 90/221, AniCraft 20/46, DesignCraft 141/284, FigCraft 102/219,
  TextureCraft 43/85, MaterialCraft 22/55, SoundCraft 22/53, WordCraft 120/236, DeckCraft 105/214, CADCraft 75/160,
  GridCraft 55/116, Craft Anything 70/141 — uns 1,6 GB para baixar os 17, mais os runtimes.
- O servidor MCP: `<app>-cli mcp`, stdio (JSON-RPC 2.0, uma linha por mensagem); as opções para uma janela variam
  (`--bridge` no PhotoCraft, FilmCraft, EffectCraft, AniCraft, TextureCraft e MaterialCraft; `--connect` no
  VectorCraft, LightCraft, DesignCraft, SoundCraft, WordCraft, DeckCraft e CADCraft; `--attach` no FigCraft); a ficha
  de controle vem de `<APP>_CONTROL_TOKEN` ou de um `control.json` na pasta de configuração do app. Estão no catálogo.

## O que foi validado e o que não foi

Cobertos por teste automático (`cargo test`): o cartão de aprovação com texto forjado (`x\nPor quê: …`, U+202E e os
demais caracteres invisíveis e de direção, 10 KB), o limite de 8 caminhos no cartão (5000 caminhos dão menos de 8 KB);
preparos e instalações ao mesmo tempo sem perder estado (8 threads por vez, e 16 para as escolhas da pessoa); dois
preparos do mesmo app e a retirada do mais velho; o interruptor "Agentes podem usar" com uma chamada travada num app de
mentira, com o servidor ainda iniciando, e com uma chamada esperando atrás de uma sessão morta; o arquivo de escolhas
cortado (falha fechado: agentes desligados); a gravação atômica; o resultado efetivo das permissões com o `flatpak
override --show` real (instalação de teste, sem rede) e um override global que alarga (pasta isca) que impede o app de
iniciar; a lista de comandos livres presa à versão exata e ao commit realmente instalado; a recusa de metadados
importados que não são os assinados; o sinal por pidfd e pelo cadeado sem pidfd. Para três desses (o cadeado do
`definir`, a conferência `encerrada()` da sessão e a retirada dos preparados) retirei o código à mão e os testes
falharam; os demais não passaram por esse teste de mutação feito por mim.

Os testes que medem o `flatpak` de verdade **falham** se ele não estiver instalado; só com `KALANGO_TESTE_SEM_FLATPAK=1`
são pulados (com aviso). Os testes de permissões escrevem as listas esperadas no próprio teste, não usam as constantes do
código como gabarito.

Limites conhecidos do modelo de ameaça:

- **Override global.** `flatpak override --reset` de um app não apaga o override global da pessoa
  (`~/.local/share/flatpak/overrides/global`), que vale por baixo do dele. Os agentes não escrevem nele (está nas pastas
  protegidas do kalangod, junto com `~/.config/flatpak`), e o kalangod recusa iniciar o servidor se o global da pessoa
  alarga o que o app vê (pastas, `persistent`, sockets, dispositivos, funções, nomes de barramento, variáveis); um
  global que só nega vale. Ele nunca o apaga: é da pessoa e vale para os outros apps dela.
- **Janela entre a conferência e o `flatpak run`.** Além da conferência logo antes de iniciar, o override do app e cada
  `flatpak run` do servidor trazem `--nofilesystem=` para `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/gcloud`,
  `~/.kube`, `~/.docker`, `~/.local/share/keyrings`, perfis de navegadores, `~/.var/app` e os overrides do Flatpak: o
  que vem depois do global vence, então um global escrito nesse intervalo não abre essas pastas. Uma pasta fora dessa lista
  que um global escrito exatamente nesse intervalo abrisse não seria barrada; só os agentes é que não conseguem
  escrevê-lo. Uma instalação do Flatpak só do kalangod não serve: os apps estão na instalação da pessoa. O `flatpak`
  roda sempre com o ambiente limpo e `HOME`, `XDG_*`, `FLATPAK_USER_DIR` e `FLATPAK_CONFIG_DIR` fixos (também sem root),
  então a conferência e o `run` leem as mesmas pastas.
- **fsync.** A gravação dos arquivos de estado faz fsync do arquivo e da pasta; isso não tem teste automático (só uma
  queda de energia o distingue de um `write` comum). Está por revisão de código.
- **Quem é a mesma pessoa.** A versão instalada que vale para a lista de comandos livres vem de `flatpak info
  --show-commit`, que lê o arquivo `deploy` da instalação da pessoa; um programa com o mesmo uid pode forjá-lo, e então
  a conferência mente. O que protege é que esse mesmo programa já teria acesso total à conta da pessoa; a lista só
  poupa perguntas, não abre nada que o nível "confirmar" ou "aprovar" não cubra para caminhos e scripts. Não
  comparamos um hash dos arquivos instalados com o repositório (custo alto para o ganho).
- O interruptor e as permissões valem para os servidores que o kalangod inicia; um app aberto pela pessoa na Central
  roda com o override que o kalangod fez (sem sockets amplos), mas as sessões dos agentes são só as dele.

Validado de verdade, contra o servidor do projeto (teste ignorado `preparar_anicraft_sem_instalar`, ~20 MB, numa pasta
e numa instalação do Flatpak de teste): índice, assinatura, download das camadas, importação com
`flatpak build-import-bundle --oci` e a conferência dos metadados do commit importado. Achado: o commit importado
(`8692a55e…`) **não** é o `org.flatpak.commit` da configuração (`486c93c5…`); por isso a conferência é a dos metadados.

**Não validado:**

- a instalação ponta a ponta feita pelo kalangod a partir do servidor real: o AniCraft precisa do runtime
  `org.freedesktop.Platform//26.08` (283 MB de download, 724 MB instalado), acima do limite de 200 MB que combinamos
  para esse teste; o teste ignorado `instalar_anicraft_verificado` existe e roda numa instalação de teste com o runtime;
- voltar a uma versão mais velha com `flatpak update --commit` (o ponto de restauração) e o efeito de a referência
  `app/<id>/…/stable` apontar para o commit novo enquanto o instalado é o antigo;
- a referência `kalango-preparado/…` e a `kalango-anterior/<uid>/…` no repositório real: o `flatpak build-update-repo`
  as aceita nos testes com o `flatpak` de mentira, mas não rodamos com um repositório com um app instalado dele;
- o modo com janela (loopback entre a janela e o servidor, a ficha, o Wayland da pessoa);
- a atualização automática de um app em uso depois da mudança de versão (lista de comandos livres, preparados);
- que o app abra e funcione com as novas negações (`session-bus`, `--nodevice=all`…): o Flatpak as aceita, mas nenhum
  dos 17 apps foi aberto assim.
