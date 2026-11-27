# O que vai dentro da ISO: modelos de IA, VirtualDesk e Fluxos

> **Decisão do mantenedor (2026-11-24): o modelo padrão é o LFM3 e ele NÃO vai dentro da ISO.** Ele só é baixado, no
> passo "Modelos de IA" do instalador ou em Ajustes/Central de Apps, depois que a pessoa aceita a licença (seção 5). A ISO
> publicada continua sem modelo nenhum (`KALANGO_MODELOS=/nenhum`); o mecanismo de embutir modelos (seção 4) é só uma
> opção local, de desenvolvimento, e não serve para publicar. As seções 1 a 3 continuam valendo como relatório de licenças e
> de custo; este documento não é aconselhamento jurídico: resume os textos como estavam em 2026-11-24.

*English summary.* The released ISO carries no AI model and neither VirtualDesk nor Fluxos. This document (1) reports
the real licence of every model in the store catalogue and says which could ship inside an ISO, (2) describes the
build, runtime and CI changes needed to carry VirtualDesk and `kalango-fluxosd` (all optional, off by default, the CI
change is a patch file in `docs/ISO-CI.patch`), and (3) describes the "with models" variant (`KALANGO_IMAGEM_TODOS_MODELOS`
or `KALANGO_IMAGEM_MODELOS`) and the licence notice it installs. The maintainer decided that LFM3 is only downloaded, after the person accepts its licence (section 5); the other decisions (licence of
the private repositories, deploy keys) are the maintainer's.

## 1. Relatório de licenças dos modelos

Fonte de cada texto: o arquivo `README.md` (cabeçalho YAML) e o `LICENSE` do repositório do Hugging Face de onde o
catálogo (`sessao/dist/share/kalango/loja/modelos.json`) baixa o arquivo, lidos em **2026-11-24**. Para o Llama 3.1 e o
Gemma 3, o repositório usado (do bartowski) só traz o cabeçalho `license:`; os textos completos foram lidos nas
páginas oficiais da Meta (`https://www.llama.com/llama3_1/license/` e `.../use-policy/`) e do Google
(`https://ai.google.dev/gemma/terms`, "Last modified: April 1, 2026", e `.../prohibited_use_policy`). Citações são
curtas; o texto vale, não esta tradução.

| Modelo (id no catálogo) | Repositório | SPDX / declarado | Na lista do AGENTS.md? | Tamanho |
|---|---|---|---|---|
| LFM3 1.7B (`lfm3-1.7b`) | LiquidAI/LFM3-1.7B-GGUF | `other`, `license_name: lfm1.0` (**custom**, LFM Open License v1.0) | **Não** | 1,0 GiB |
| LFM3 Audio 2B (`lfm3-audio-2b`) | LiquidAI/LFM3-Audio-2B-GGUF | idem | **Não** | 1,3 GiB (com mmproj) |
| LFM3 VL 2B (`lfm3-vl-2b`) | LiquidAI/LFM3-VL-2B-GGUF | idem | **Não** | 1,2 GiB (com mmproj) |
| LFM3 Omni 2B (`lfm3-omni-2b`) | LiquidAI/LFM3-Omni-2B-GGUF | idem | **Não** | 1,5 GiB (com mmproj) |
| d1 2B (`d1-2b`) | LiquidAI/d1-2B-GGUF | idem | **Não** | 1,2 GiB (com mmproj) |
| d1-Omni 2B (`d1-omni-2b`) | LiquidAI/d1-Omni-2B-GGUF | idem | **Não** | 2,0 GiB (com mmproj) |
| Qwen3 8B (`qwen3-8b`) | Qwen/Qwen3-8B-GGUF | `apache-2.0` | **Sim** | 4,7 GiB |
| Qwen3 32B (`qwen3-32b`) | Qwen/Qwen3-32B-GGUF | `apache-2.0` | **Sim** | 18,4 GiB |
| Llama 3.1 8B (`llama-3.1-8b`) | bartowski/Meta-Llama-3.1-8B-Instruct-GGUF | `llama3.1` (**custom**, Llama 3.1 Community License) | **Não** | 4,6 GiB |
| Gemma 3 27B (`gemma-3-27b`) | bartowski/google_gemma-3-27b-it-GGUF | `gemma` (**custom**, Gemma Terms of Use) | **Não** | 15,4 GiB |

Os cinco repositórios da Liquid AI têm o mesmo `LICENSE` (10 574 bytes, idênticos), então valem as mesmas respostas.

### 1.1 Família LFM3 e d1 (Liquid AI): LFM Open License v1.0

- **Pode redistribuir os pesos dentro de uma ISO?** Sim, com condições. A seção 2 dá licença "perpetual, worldwide,
  non-exclusive, no-charge, royalty-free, irrevocable" para "reproduce ... sublicense, and distribute the Work", mas
  "including the Commercial Use limitation set forth in Section 5".
- **Deveres de aviso.** Seção 4: dar a cada destinatário "a copy of this License"; marcar arquivos modificados (os
  GGUF são quantizações dos pesos originais: tratar como obra derivada e dizer isso); manter avisos de copyright; se
  houver arquivo `NOTICE`, reproduzi-lo (não há `NOTICE` nesses repositórios). Seção 7: sem uso das marcas da Liquid AI
  além de descrever a origem.
- **Restrição de uso que pesa numa distribuição aberta.** Seção 5: o uso comercial ("any use ... for direct or
  indirect commercial advantage or monetary compensation") só é licenciado enquanto o usuário ou a entidade dele não
  passar do "Threshold" de **US$ 10 milhões de receita anual**; acima disso, o uso comercial "is not licensed under
  this Agreement" (sem esse limite para organização sem fins lucrativos qualificada, em uso não comercial ou de
  pesquisa). Consequências: (1) não é uma licença livre no sentido da DFSG/OSI (limita campo de uso e quantidade de
  receita); (2) uma empresa acima do limite que instalar o KalangoOS com esses modelos dentro precisaria de licença da
  Liquid AI; (3) o KalangoOS, que é GPL-3.0-or-later, ficaria com um componente que não segue a mesma liberdade.
  Não há política de uso aceitável separada nesse texto. Seção 3: a licença de patentes termina se o usuário abrir
  processo de patente contra o trabalho.
- **SPDX.** Nenhum: `LicenseRef-LFM-Open-1.0` (custom). **Não está na lista aceita do AGENTS.md; pergunta obrigatória.**

### 1.2 Qwen3 (Alibaba): Apache-2.0

- **Pode redistribuir?** Sim, sem condição de receita ou de campo de uso (cabeçalho `license: apache-2.0`, `LICENSE` é
  o texto padrão da Apache License 2.0, 11 544 bytes).
- **Deveres.** Incluir uma cópia da licença, avisar modificações (os GGUF são conversões do Qwen/Qwen3-*; o
  repositório é o oficial da Qwen) e manter os avisos de copyright e de atribuição; reproduzir o `NOTICE` se houver.
- **Restrições de uso.** Nenhuma além da licença e da lei.
- **SPDX.** `Apache-2.0`. **Está na lista aceita.** É o único dos dez que pode entrar sem pergunta de licença.
  (A decisão de gastar 4,7 GiB, ou 18,4 GiB no 32B, na ISO continua sendo de tamanho, não de licença.)

### 1.3 Llama 3.1 8B (Meta): Llama 3.1 Community License

- **Pode redistribuir?** Sim, com condições (seção 1.b). Quem distribui "the Llama Materials (or any derivative works
  thereof), or a product or service ... that contains any of them" deve **(A)** dar uma cópia do acordo e **(B)**
  "prominently display 'Built with Llama' on a related website, user interface, blogpost, about page, or product
  documentation"; também deve manter, num arquivo "Notice", a frase "Llama 3.1 is licensed under the Llama 3.1
  Community License, Copyright (c) Meta Platforms, Inc. All Rights Reserved." Quem usa o material para treinar ou
  melhorar outro modelo e o distribui deve pôr "Llama" no começo do nome desse modelo.
- **Restrições de uso.** Cumprir a Política de Uso Aceitável da Meta (incorporada por referência: proíbe, por
  exemplo, uso para violar a lei, para violência, exploração de crianças etc.; o distribuidor deve repassá-la);
  "700 million monthly active users" ou mais na data de lançamento exige pedir licença à Meta, "in its sole
  discretion"; a Meta pode rescindir se houver violação, e então é preciso apagar o material.
- **SPDX.** Nenhum (`LicenseRef-Llama-3.1`). **Fora da lista; pergunta obrigatória.** O repositório oficial da Meta é
  fechado (HTTP 401 sem aceite), e o catálogo baixa da cópia de um terceiro (bartowski); redistribuir exigiria
  também conferir que a Meta aceita essa origem.

### 1.4 Gemma 3 27B (Google): Gemma Terms of Use

- **Pode redistribuir?** Sim, se cumprir todas as condições da seção 3.1: incluir as restrições de uso (seção 3.2)
  "as an enforceable provision in any agreement ... governing the use and/or distribution" e avisar os usuários
  seguintes; dar uma cópia dos Termos; marcar arquivos modificados; e acompanhar toda distribuição de um arquivo
  "Notice" com "Gemma is provided under and subject to the Gemma Terms of Use found at gemma.ai/resources/terms".
- **Restrições de uso.** Seção 3.2: não usar para os usos restritos da Gemma Prohibited Use Policy (incorporada por
  referência) nem contra a lei; o Google "reserves the right to restrict (remotely or otherwise) usage ... that
  Google reasonably believes are in violation". Um distribuidor teria de impor a política a todos que recebem a
  ISO, o que um sistema livre não consegue fazer sem um contrato de uso próprio. Seção 4.5: o Google pode rescindir
  em caso de violação, e então é preciso apagar todas as cópias e parar de distribuir.
- **SPDX.** Nenhum (`LicenseRef-Gemma`). **Fora da lista; pergunta obrigatória.**

### 1.5 Recomendação (para o mantenedor decidir)

1. **Pode entrar sem pergunta de licença:** Qwen3 8B (Apache-2.0). Obriga a ISO "com modelos" a incluir o texto da
   Apache-2.0 e a atribuição à Alibaba/Qwen e a dizer que são conversões GGUF. Custo: +4,7 GiB (o squashfs passa de
   4 GiB; o `construir.sh` já usa `-iso-level 3` com modelos).
2. **Depende de decisão de licença (a mais útil na prática):** LFM3 1.7B, o modelo de texto que o agente usa por
   padrão (1 GiB). Entra com o texto da LFM Open License v1.0 e o aviso de que o uso comercial acima de US$ 10
   milhões de receita anual precisa de licença da Liquid AI. É a única mudança que torna o agente utilizável sem
   internet no primeiro uso. Os outros LFM3/d1 (voz, visão, omni, decisão) seguem a mesma licença; entrariam só se o
   mantenedor aceitar a licença para a família toda.
3. **Só por download (não incluir):** Llama 3.1 8B (exige "Built with Llama" visível, Política de Uso Aceitável
   repassada e origem de terceiro) e Gemma 3 27B (exige impor a Política de Uso a todo destinatário e permite ao
   Google restringir o uso). Também Qwen3 32B e Gemma 3 27B pelo tamanho (18,4 e 15,4 GiB não cabem numa ISO útil).
   Baixar pelo instalador ou pela Central de Apps, com o aceite da licença na tela, deixa a pessoa como parte do
   acordo com o fornecedor e não faz do KalangoOS um redistribuidor.
4. **Nenhuma dessas licenças é aceita hoje pelo AGENTS.md** exceto a Apache-2.0 do Qwen3. Para incluir qualquer
   modelo LFM, Llama ou Gemma a regra 8 manda parar e perguntar; este documento é o material para essa pergunta.

### 1.6 O que o instalador e a Central de Apps mostram (feito: seção 5)

- Na etapa "Modelos de IA" do instalador, para cada modelo que já está na ISO: o nome, "já está neste
  computador", a licença (nome e link) e o aceite quando a licença tem condição (LFM: limite de receita; Llama e Gemma:
  política de uso). Sem aceite, o modelo não é ativado para os agentes.
- Na Central de Apps (Modelos): a mesma linha de licença antes de baixar ou de usar, e um link para o texto completo
  instalado em `/usr/share/doc/kalango/modelos/LICENCAS.txt`.
- O arquivo `LICENCAS.txt` com o texto de aviso e os links de cada modelo que a ISO traz (ver a seção 4).
- Se um dia o Llama entrar: "Built with Llama" na tela do instalador e na Central, e o arquivo `Notice` exigido.
- Para qualquer ISO com modelos: no site, na página de download dessa ISO, os mesmos avisos.

## 2. VirtualDesk e Fluxos dentro da ISO

### 2.1 O que são, de verdade, e que licença têm

Os dois repositórios são **privados** (`kalangoos/kalango-virtualdesk`, `kalangoos/kalango-fluxos`), mas, ao
contrário do que se supunha, **os dois têm arquivo de licença** em `origin/main` (conferido em 2026-11-24):

| Programa | Licença (arquivo `LICENSE` e `Cargo.toml`) | Na lista do AGENTS.md? | Commit usado nos testes |
|---|---|---|---|
| Kalango VirtualDesk | GPL-3.0-or-later | Não está na lista de dependências, mas é a licença do próprio KalangoOS e do niri (aceita) | `9423e19` (`origin/main`) |
| Kalango Fluxos (`kalango-fluxosd`) | Apache-2.0 (com `NOTICE`) | Sim | `7d57280` (`origin/main`) |

**A consequência para uma ISO pública.** A ISO passaria a redistribuir os *binários* desses dois repositórios.
- **Fluxos (Apache-2.0):** não há obstáculo de licença para redistribuir o binário, desde que a ISO leve o texto da
  licença e o `NOTICE` (o `construir.sh` instala os dois em `/usr/share/doc/kalango/fluxos/`, junto de
  `DEPENDENCIAS.txt`, a lista de todas as crates que entram no binário com a licença de cada uma). O código-fonte pode
  continuar privado.
- **VirtualDesk (GPL-3.0-or-later):** quem recebe o binário tem direito ao **código-fonte correspondente** (GPLv3,
  seção 6): junto da ISO, ou por oferta escrita válida por 3 anos, ou por um servidor de rede equivalente. **Um
  repositório privado não cumpre isso.** Para publicar a ISO com o VirtualDesk, o mantenedor precisa de uma de duas
  ações: (a) tornar `kalango-virtualdesk` público (o caminho natural: o repositório já diz GPL-3.0-or-later; o
  `README` do KalangoOS já o trata como parte do sistema), ou (b) publicar o fonte desse commit junto da ISO (um
  `.tar.gz` no mesmo release, o que na prática torna o código público para quem baixar a ISO). Sem uma das duas, **não
  publique a ISO com o VirtualDesk**. Isto é uma decisão do mantenedor, não minha; nada foi publicado.
- O VirtualDesk também usa o QEMU, o OVMF e o swtpm do Ubuntu (GPL/BSD, com os avisos que o dpkg já instala em
  `/usr/share/doc/*/copyright`), e a ISO precisa continuar oferecendo o fonte desses pacotes (o Ubuntu o faz).
- Para o Fluxos, a pergunta de licença não existe. A pergunta que resta é se o código **privado** pode ser
  distribuído em binário: pela Apache-2.0, sim, e a decisão de manter o fonte fechado é só do mantenedor.

### 2.2 Como entram na imagem (`imagem/construir.sh`)

Entram como o Remote e o Audio: compilados ao lado e copiados; mas **opcionais**, para a ISO que a CI publica hoje
não mudar enquanto a CI não os entregar.

- Pastas: `KALANGO_VIRTUALDESK_DIR` (padrão `../kalango-virtualdesk`) e `KALANGO_FLUXOS_DIR` (padrão `../kalango-fluxos`),
  com o binário em `target/release/` e `dist/`, `assets/` do repositório.
- `KALANGO_IMAGEM_VIRTUALDESK` e `KALANGO_IMAGEM_FLUXOS`: `auto` (padrão: entra se o binário estiver compilado; sem
  ele, diz "sem ... nesta ISO" e segue), `1` (exige; falha se faltar) ou `0` (nunca). `KALANGO_IMAGEM_SEM_APPS=1` tira
  os dois, como tira o Remote e o Audio.
- Instalados em: `/usr/bin/kalango-virtualdesk`, `/usr/share/applications/os.kalango.VirtualDesk.desktop` (o `Exec`
  vira `kalango-virtualdesk`), os ícones de 64, 128, 256 e 512 px, `/usr/lib/kalango/kalango-fluxosd` (o caminho em
  que o kalangod o procura, `crates/kalangod/src/tools/plataforma.rs`) e, em `/usr/share/doc/kalango/virtualdesk|fluxos/`,
  a licença e o inventário de dependências. A checagem de bibliotecas (`readelf`) da imagem passa a cobrir os dois.
- **Dependências de execução do VirtualDesk** (pacotes do Ubuntu, só quando ele entra; ficam em `imagem/pacotes-virtualdesk.txt` e vão para a raiz de pacotes, a etapa 1 do `construir.sh`, que a CI monta em paralelo: ao partir de uma raiz pronta, o `construir.sh` confere que o QEMU está nela): `qemu-system-x86`, `qemu-utils`,
  `ovmf`, `swtpm` (o QEMU roda as máquinas, o `qemu-img` cria os discos, o OVMF é o UEFI, o swtpm é o TPM do Windows
  11). A GTK 4 e o resto de que ele precisa já estão em `pacotes.txt`. **Custo**: ver a medida na seção 2.5.
- **KVM**: o instalador cria a conta só no grupo `sudo`, mas o `70-uaccess.rules` do próprio systemd já marca o `/dev/kvm`
  com `uaccess`: a pessoa da sessão ativa o abre sem entrar no grupo `kvm`, e o VirtualDesk não precisa de regra nossa
  (uma regra do udev que eu tinha posto era redundante e foi tirada). Consequência de segurança, registrada em
  `docs/SEGURANCA.md`: um agente que roda como o usuário da sessão também abre o `/dev/kvm`. Sem KVM (VM sem
  virtualização aninhada) as máquinas rodam emuladas, bem mais lentas: o próprio app avisa.
- **Procedência e licenças** (`inventario()` do `construir.sh`): para cada app que entra, `/usr/share/doc/kalango/<app>/`
  recebe a licença, o `NOTICE`, `DEPENDENCIAS.txt` (as crates do alvo `x86_64-unknown-linux-gnu`, com a licença de cada
  uma, de `cargo metadata --locked --offline --filter-platform …`) e `COMMIT` (o `git rev-parse HEAD` de onde o binário
  veio, impresso também na construção). Se a árvore tem mudanças não commitadas, ou o binário é mais antigo que o
  commit, ou não há git, o `construir.sh` avisa. Com `KALANGO_IMAGEM_PUBLICAVEL=1` (posto só pelo workflow que
  publica) cada um desses avisos vira **erro**, inclusive a lista de dependências que não saiu.
- **GPL do VirtualDesk**: com `KALANGO_IMAGEM_PUBLICAVEL=1` o VirtualDesk só entra se `KALANGO_VIRTUALDESK_FONTE` aponta
  o `.tar.gz` do fonte **do mesmo commit** (o nome carrega o commit); a ISO ganha `/usr/share/doc/kalango/virtualdesk/FONTE.txt`
  dizendo onde o fonte está. Em ISO local sem o arquivo, avisa que não é para publicar.
- **Fluxos, só quando há uso.** O serviço vem como **unidade de usuário do systemd com ativação por soquete**
  (`/usr/lib/systemd/user/kalango-fluxos.socket` e `.service`, do repositório do Fluxos; o soquete é ligado em
  `sockets.target.wants`): o systemd só abre o soquete `/run/user/<uid>/kalango/fluxos.sock`; o serviço sobe na primeira
  conexão (do shell ou do `kalangod`, que roda `kalango-fluxosd chamar`) e, sem automação e sem cliente por 10
  minutos, sai sozinho. Parado, ele custa zero de memória e de CPU. `Slice=kalango-agentes.slice` e
  `MemoryHigh=256M` vêm da unidade do repositório.

### 2.3 O que foi testado de verdade

Uma ISO local com **tudo junto** (VirtualDesk e `kalango-fluxosd` do `origin/main` acima, mais os modelos de texto, voz e
visão da seção 4) foi montada com `imagem/construir.sh` e testada com `MEM=4096 imagem/testar-iso.sh instalar` numa VM
UEFI de 4 GB (uma de cada vez): sessão ao vivo, instalação real no disco virtual, primeiro boot do sistema
instalado. **32 verificações passaram, 0 falharam** (com o código e a unidade de antes das mudanças da seção 3 e da seção 5: a VM final não rodou, ver abaixo) (a primeira tentativa, antes de eu mudar o teste, parou em "a VM
não desligou" depois de a instalação terminar bem: a máquina estava com carga 20; a repetição passou). As verificações
novas de `imagem/testar-iso.py` só valem quando o app está na ISO:

- VirtualDesk: nenhuma biblioteca faltando (`ldd`), atalho e ícone, `qemu-system-x86_64`, OVMF, `swtpm` e `qemu-img` no
  sistema; `kalango-virtualdesk ferramenta vm.listar` numa biblioteca vazia responde `"maquinas":[]`; o **`kalangod`
  chama `vm.listar`** e a resposta chega; a **janela abre** na sessão niri da conta instalada e continua de pé 12 s
  depois.
- Fluxos: o soquete `kalango-fluxos.socket` está ativo e o serviço ainda **inativo** antes de qualquer chamada; o
  **`kalangod` chama `agentes.listar`** e o serviço responde (o systemd o sobe pelo soquete); `modelos.listar` pelo
  mesmo caminho lista os modelos locais da ISO como instalados.
- Sem VirtualDesk/Fluxos na ISO (a ISO de CI de hoje) essas verificações só escrevem "não está nesta ISO": a lógica de
  "opcional" (`auto`, `1`, `0`, `KALANGO_IMAGEM_SEM_APPS`) foi exercitada à parte, com o `quer()` isolado.

### 2.4 O que a CI precisa (patch, não aplicado)

`docs/ISO-CI.patch` muda `.github/workflows/release.yml` (a regra 10 do AGENTS.md proíbe mudar workflow num PR de
funcionalidade: ele vai num PR só dele, com `git apply docs/ISO-CI.patch`). O que ele faz:

1. um passo `actions/create-github-app-token` (fixado pelo SHA do commit) que gera um token **somente leitura e de vida curta**
   do GitHub App "KalangoOS Release Reader" (variável `KALANGO_RELEASE_APP_ID`, segredo `KALANGO_RELEASE_APP_KEY`; dono
   `kalangoos`, repositórios `kalango-virtualdesk` e `kalango-fluxos`) e dois `actions/checkout` desses repositórios, por
   https com o token, em commits fixos; sem a variável do App os passos são pulados e a ISO sai como sempre. (As chaves de
   deploy foram descartadas: a organização as desativa.) O token é mascarado pela própria ação; nada o imprime;
2. compila o VirtualDesk (`libgtk-4-dev`, `cargo build --release --locked --workspace`) e o `kalango-fluxosd`
   (`cargo build --release --locked -p kalango-fluxosd`, com o toolchain que o repositório fixa);
3. gera o **código-fonte da GPL** do VirtualDesk no mesmo `if:` (`git archive` do commit fixado em
   `kalango-virtualdesk-<commit>-src.tar.gz`), manda-o ao `construir.sh` (`KALANGO_VIRTUALDESK_FONTE`), coloca-o em
   `dist-iso/` com a ISO, nas somas (`SHA256SUMS-iso.txt`) e, portanto, no **mesmo rascunho de versão**; as notas da versão
   dizem onde ele está. Só a chave do VirtualDesk sem esse passo não publicaria a ISO: o `construir.sh` recusa;
4. passa `KALANGO_VIRTUALDESK_DIR` e `KALANGO_FLUXOS_DIR` ao `construir.sh`, `KALANGO_IMAGEM_PUBLICAVEL=1` (erro em vez
   de aviso para tudo o que falta de atribuição, e **nenhum modelo de IA na ISO**), e o `testar-iso.sh instalar` já roda
   as verificações novas (só valem quando o app está na ISO).
   (A variante `-com-modelos` que a primeira versão deste patch trazia foi tirada: o LFM3 só é baixado.)

**O que o mantenedor precisa fazer (nada disso eu faço):**

- **Ler os repositórios privados**: já existe o GitHub App "KalangoOS Release Reader" (id 5929164, Contents e Metadata só de
  leitura, instalado só nesses dois repositórios), com a variável e o segredo acima em `kalangoos/kalangoos`. O patch usa
  esse App; não há chave de deploy a criar.
- Alternativa que dispensa o App: tornar os repositórios públicos (e então tirar o passo do token e o `token:` dos checkouts).
  Para o VirtualDesk, é também o jeito de cumprir a GPL (seção 2.1).
- Fixar os commits do patch na versão que quiser publicar (o patch usa `origin/main` de 2026-11-24). A unidade do Fluxos
  com sandbox está na branch `unidade-sandbox` do repositório do Fluxos (`fa091ac`, uma mudança só, sobre `7d57280`): depois
  de integrá-la, o commit do patch deve passar a ser o dela.
- Decidir se a ISO passa a levar os dois (um PR do patch, depois de tornar o VirtualDesk público ou de publicar o fonte).

### 2.5 O que isso custa (medido)

Medido comparando a lista de pacotes e o `Installed-Size` da ISO 0.1.0 (532 pacotes) com a desta construção (568):
**+36 pacotes, +133 MiB instalados** (`qemu-system-x86` 66 MiB, `qemu-system-data` 19, `ovmf-*` 21, `qemu-utils` 12,
`ipxe-qemu` 5, o resto bibliotecas pequenas), nenhum removido; mais os binários: `kalango-virtualdesk` 5,1 MiB e
`kalango-fluxosd` 16,4 MiB. Em resumo, uns **+155 MiB descomprimidos** na raiz; a ISO 0.1.0 tem 0,7 GiB. Licenças que
isso traz: QEMU (GPL-2.0 e outras, do Ubuntu), OVMF (BSD-2-Clause-Patent), swtpm (BSD-3-Clause), iPXE (GPL-2.0), com o
`copyright` de cada pacote em `/usr/share/doc/<pacote>/copyright` e o fonte pelo arquivo do Ubuntu. O serviço dos Fluxos
parado não gasta nada; com o VirtualDesk fechado, nenhum processo roda (as máquinas são processos próprios do QEMU).

## 3. O que não está feito, e o cgroup do Fluxos

- Nada foi publicado, nenhuma chave ou segredo foi criado, nenhum repositório mudou de visibilidade, e o workflow não foi
  alterado (só o patch).
- **O cgroup do serviço dos Fluxos (era um bloqueio; corrigido).** O `kalango-fluxosd` sobe sob o gerenciador de usuário:
  `user.slice/user-UID.slice/user@UID.service/kalango.slice/kalango-agentes.slice/kalango-fluxos.service` (medido na VM).
  O que o `kalangod` já reconhecia por `contains("/kalango-agentes.slice")` (a aprovação, o `allowlist`, as ferramentas de
  máquinas) já o pegava. Duas coisas não pegavam, e eram as que importam: a **regra do kernel** (a guarda eBPF só conhecia
  `/kalango.slice/kalango-agentes.slice`, no nível 2) e o `rule_for` (`starts_with` do prefixo, então o serviço caía na
  regra "usuario-local"). Agora: `policy::cgroup_matches` aceita também o caminho sob `user@UID.service`
  (teste de unidade, com casos que não devem casar: sessão da pessoa, `user@` de outro UID, nome parecido); a guarda tem
  um mapa de cgroups de agentes (`CG_AGENTES`, uma tabela hash de id do cgroup para 1, **até 4096 cgroups: o do sistema e um por
  gerenciador de usuário**, sem limite de usuários além disso) com o do sistema e o de cada gerenciador de usuário, que
  o `kalangod` registra assim que o slice aparece (varredura de 1 s; a janela sem proteção é de até 1 s depois de o slice
  nascer), e o programa eBPF olha os ancestrais do processo nos níveis 1 a 8 (um passo por nível, sem laço) e procura cada um no mapa. Isto também fecha a mesma lacuna do caminho `systemd-run --user` do
  próprio `kalangod` (`limits.rs`). **Verificado numa VM (2026-11-26, ISO local publicável, kernel 7.3.4-kalango)**, e o teste achou dois defeitos
  que os testes de unidade não viam: (1) o **verificador do kernel recusava o programa** ("infinite loop detected": o laço
  de 64 passos tinha o contador na pilha, por causa do ponteiro da chave do mapa) e a guarda inteira ficava desligada, sem
  erro no resto do teste; a busca virou oito passos sem laço, um por nível de ancestral (`NIVEL(1)` a `NIVEL(8)` em `guarda.bpf.c`;
  `the_ebpf_program_has_no_loop_and_no_per_user_limit`), e o teste da ISO passou a **falhar** se o LSM bpf está ativo e a
  guarda não carregou; (2) o cgroup dos agentes de uma pessoa nascia junto do primeiro processo do agente, e esse processo
  rodava sem a guarda por até 1 s (a varredura do `kalangod` é de 1 s): uma unidade de usuário vazia e ativa desde o
  início da sessão (`kalango-agentes.slice`, `crates/kalangod/dist/kalango-agentes-usuario.slice`) já o cria antes de
  qualquer agente. Resultado na VM: fora do slice dos agentes `~/.ssh` abre; dentro dele, pelo gerenciador de usuário, o
  kernel nega (`Operation not permitted`). A janela que resta: o primeiro segundo depois de a sessão começar, antes de
  qualquer agente existir.
  **A guarda e o banco dos Fluxos** (achado da segunda VM, 2026-11-27): com a guarda de fato ativa e o cgroup dos agentes
  existindo desde o início da sessão, o `kalango-fluxosd` (que roda na fatia dos agentes) **não abria o próprio banco**
  (`~/.local/share/kalango/fluxos` é pasta protegida dos agentes): o serviço falhava ao subir. Antes isso passava
  despercebido porque a guarda ainda estava desligada, e depois porque a primeira abertura caía na janela de 1 s. A correção:
  o serviço roda na fatia `kalango-agentes-plataforma.slice`, **filha** da dos agentes (para a política do `kalangod`, que
  reconhece agentes por `/kalango-agentes.slice`, nada muda) mas **isenta da guarda do kernel** (mapa `CG_ISENTOS`; o programa
  olha os ancestrais nos níveis 1 a 8, e quem tem uma fatia isenta entre eles não é tratado como agente). Quem escolhe a fatia
  é um drop-in que a imagem instala (`kalango-fluxos.service.d/10-plataforma.conf`); a unidade vazia e ativa desde o início da
  sessão é `kalango-agentes-plataforma.slice`. **Decisão de segurança a confirmar**: um processo que um agente pusesse nessa fatia
  (`systemd-run --user --slice=kalango-agentes-plataforma.slice`) também seria isento, como já escapa da guarda quem se põe numa
  fatia qualquer; o que segura o agente é o Landlock e a política do `kalangod`, a guarda do kernel é a camada a mais. Provado
  na VM: o serviço sobe nessa fatia, responde `agentes.listar` e, no mesmo boot, um processo na fatia dos agentes continua negado.
  **Limite e falha fechada**: a primeira versão guardava 64 usuários numa tabela e os excedentes ficavam sem proteção, em silêncio.
  Agora o limite é o do mapa (4096 cgroups de agentes) e, se um dia passar disso, o `kalangod` conta quantos não couberam
  (`plano_agentes`), `kernel.seguranca` mostra `guarda_ebpf_completa: false` e a linha do tempo recebe o alerta "a guarda cobre só
  N de M cgroups de agentes" (testes `a_hundred_users_all_get_the_guard_and_an_overflow_is_counted` e
  `the_plan_adds_the_new_ones_removes_the_gone_ones_and_ignores_zero_and_repeats`). **Ainda não provado numa VM**: o programa
  novo (hash + oito níveis) compila, mas o verificador do kernel só o vê numa VM; é o que o `agents_guard` confere.
- **Unidade do Fluxos com sandbox** (repositório do Fluxos, branch `unidade-sandbox`, `fa091ac`): `PrivateTmp`,
  `ProtectSystem=strict` com `ReadWritePaths` só do que o serviço guarda (`~/.local/share/kalango`, `~/.config/kalango`,
  `~/.local/state/kalango` e a pasta de execução), `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`, `LockPersonality`,
  `SystemCallFilter=@system-service`. Verificado: um serviço transitório com essas mesmas propriedades sobe, guarda um
  fluxo, roda o fluxo (`done`, saída `ola`) e lista a execução; e a unidade com o sandbox subiu pela ativação do soquete na VM de 2026-11-26 (o serviço ficou no `kalango-agentes.slice` do gerenciador de usuário). Essa unidade ainda não está no `main` do repositório do Fluxos (só na branch `unidade-sandbox`): o commit que o patch fixa precisa conter a unidade antes de a ISO ser publicada.

## 4. Modelos dentro da ISO: só uma opção local de desenvolvimento

**Não serve para publicar.** A ISO publicada fica sem modelo (decisão acima); o `construir.sh` com
`KALANGO_IMAGEM_PUBLICAVEL=1` **recusa qualquer modelo** (`KALANGO_IMAGEM_TODOS_MODELOS`, `KALANGO_IMAGEM_MODELOS`, ou um
arquivo na pasta de modelos da imagem), e o workflow põe essa variável. O que existe, para quem monta uma ISO de teste:

- **Escolher o que entra** (`imagem/construir.sh`): `KALANGO_IMAGEM_TODOS_MODELOS=1` ou
  `KALANGO_IMAGEM_MODELOS="arquivo1.gguf arquivo2.gguf"` (exatamente esses, da pasta `KALANGO_MODELOS`; um arquivo que falta
  é erro). O `-iso-level 3` vem do tamanho do squashfs (acima de 4 GB).
- **Conferência e aviso** (`imagem/licencas-modelos.py` + `imagem/licencas-modelos.json`; os textos das licenças ficam em
  `sessao/dist/share/doc/kalango/modelos/`): cada arquivo da pasta (não só `.gguf`) é conferido contra o catálogo (nome,
  tamanho e SHA-256); a ISO leva `/usr/share/doc/kalango/modelos/LICENCAS.txt`. O JSON marca `"somente_download": true`
  para LFM3/d1, Llama e Gemma e `"aprovado": false`: com um deles dentro, a construção avisa "só para uso local".
- **Sistema instalado**: a fila de downloads do shell conta o que está em `/usr/share/kalango/modelos` como instalado
  (`Queue::set_system_dirs`) e o passo "Modelos de IA" do instalador mostra "já está no sistema" (`Chip::InSystem`).
- Um arquivo de uma versão do GitHub tem no máximo 2 GiB: uma ISO com modelos nem caberia no rascunho da versão.

## 5. A licença do modelo, antes de baixar (feito)

Os modelos cuja licença não está na lista do AGENTS.md (LFM Open License v1.0, para LFM3, LFM3 Audio, LFM3 VL, Omni e d1;
também Llama e Gemma, se entrarem no catálogo) exigem **aceite explícito** antes do download; os de Apache-2.0 (Qwen3) só
mostram o aviso.

- **O que a pessoa vê** (instalador, passo "Modelos de IA"; shell, ao baixar de Ajustes ou da Central): "Licença do
  modelo", um resumo em português simples (uso, cópia e compartilhamento permitidos; uso comercial licenciado só enquanto a
  receita anual for menor que US$ 10 milhões; "o texto completo prevalece sobre este resumo"), "Ler a licença completa" (o
  texto inteiro), uma caixa "Li e aceito a licença" **obrigatória**, "Baixar" desligado até marcá-la e "Pular: baixar
  depois". Textos em `kalango-modelos::licenca::textos`, no catálogo pt-BR.
- **O texto completo**: a LFM Open License v1.0 permite reproduzir e distribuir o texto (seção 4 da própria licença: quem
  distribui dá uma cópia dela), e o repositório já a trazia como `imagem/licencas-modelos/`; agora ela vai em toda ISO, em
  `/usr/share/doc/kalango/modelos/LFM-Open-1.0.txt` (versão "1.0"), junto do texto da Apache-2.0 do Qwen3. Llama e Gemma
  não têm texto no repositório: a tela mostra o endereço oficial.
- **O registro**: `~/.config/kalango/modelos-licencas.json`, uma entrada por modelo, licença, **versão da licença** e data.
  O instalador grava o mesmo arquivo para a conta nova (o plano leva `licencas`; o motor recusa instalar se um modelo
  escolhido não tem aceite válido, e descarta registros de versão ou licença que não são os vigentes). Em Ajustes, cada
  modelo aceito mostra "Licença aceita em AAAA-MM-DD · ver".
- **Sem aceite não baixa, nem por outro caminho**: a **fila de downloads** (`kalango-modelos::queue`, o único lugar por onde
  todo download passa) recusa o modelo (`Failure::LicenseNotAccepted`, "licença não aceita") quando falta o aceite ou ele é
  de uma versão antiga da licença; um modelo que já está no disco, ou que o sistema traz, não é bloqueado.
- **Testes que falham sem isso**: `a_model_whose_licence_is_not_accepted_is_refused_by_the_queue_and_goes_on_after_the_acceptance`
  e `a_new_licence_version_asks_again_and_a_model_already_on_disk_is_not_blocked` (fila),
  `an_acceptance_is_per_model_licence_and_version` (registro), `a_licence_that_asks_for_acceptance_blocks_continuing_until_accepted`
  (instalador), `the_engine_refuses_models_whose_licence_was_not_accepted_and_records_the_accepted_ones` (motor) e
  `a_licence_not_accepted_offers_the_download_which_asks_for_it` (shell).
- **O desenho** (kalangoos/design, `a32d1e0`, página "35 · Licença do modelo (LFM3)"). Seguido: o cartão de licença com borda
  âmbar (título "Licença do LFM3 · LFM Open License v1.0", "Resumo em linguagem simples", quatro linhas com ✓ ⚠ ⓘ, "Ler a licença
  completa" e "texto original (inglês) · v1.0"), a caixa "Li e aceito a licença do LFM3 / Obrigatório para baixar os modelos LFM3 ·
  fica registrado só neste computador", "Pular: baixar depois", a **folha com o texto completo** (o texto real da licença, com
  a fonte, "Copiar texto" e "Fechar"), o diálogo "Baixar LFM3 1.7B" de Ajustes/Central (modelo, cartão, caixa, nota "Sem modelo
  os agentes continuam desligados…", Cancelar e "Baixar 1,2 GB" desligado até aceitar), e "Licença aceita em … · ver" com
  **"Revogar o aceite"** ("Remover modelos LFM3 e revogar": remove os modelos da licença e apaga o registro). **Passo do instalador**: abre no cartão do LFM3 1.7B
  ("recomendado", 1,2 GB, "roda nesta máquina · LFM Open License v1.0"), a licença, "Li e aceito" e "Pular: baixar depois"; as quatro
  fontes e a lista dos cinco modelos continuam, mas dentro de "Outras fontes e modelos" (recolhido). Diálogos de Ajustes seguem a largura
  da janela (variantes estreitas) e o de um modelo instalado mostra o estado ("Instalado e verificado"), o aceite e "Revogar o aceite".
  **Não feito**: as telas de download dentro do instalador (progresso por arquivo com pausar/retomar, erros "sem rede / sem espaço / soma
  não confere") e a de Qwen3 só com aviso: o instalador não baixa nada (grava a escolha e o sistema instalado baixa depois), então
  não há o que mostrar ali; o Qwen3 não está entre os cinco modelos do instalador. O que foi feito nesta rodada **não foi compilado nem
  visto numa captura** (a máquina estava parada): `rustfmt` confirma só a sintaxe; o build, os testes e as capturas lado a lado ficam
  para depois do reinício. O token novo `cor/legenda` não foi necessário.
