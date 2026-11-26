# KalangoOS 0.1.2 — nove idiomas, Apps criativos e gatilho pela internet

**Português** · [English](#english)

Continua sendo uma prévia técnica para desenvolvedores e testadores, **testada só em máquina virtual**: não instale num
computador com dados que você não pode perder.

O Kalango Remote desta versão é o da 0.1.1 com a interface nos mesmos nove idiomas; o que o `kalangod` exige do Remote
não mudou, e os dois continuam saindo juntos na ISO.

## O que há de novo

- **A interface em nove idiomas:** English, Português (Brasil), Español, Français, Italiano, Deutsch, 日本語, Русский e
  简体中文, no sistema, no Kalango Remote e no Kalango Audio. O idioma vem do sistema, e o instalador e as
  Configurações › Idioma o escolhem. **Os sete catálogos além do português e do inglês foram traduzidos por IA e não
  foram revisados por pessoas nativas** (veja abaixo).
- **Apps criativos a pedido:** os 17 apps do projeto Craft podem ser instalados pela Central de Apps e usados pelos
  agentes, com níveis e cartão de aprovação. O download confere a assinatura do projeto e as permissões do Flatpak negam
  as pastas sensíveis. Eles não vêm na ISO. Detalhes em
  [docs/APPS-CRIATIVOS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/APPS-CRIATIVOS.md).
- **Gatilho de entrada pela internet:** um endereço público e assinado que inicia um fluxo, por um túnel da Cloudflare
  aberto só depois de uma aprovação (tela de túneis em Ajustes e gatilho no editor de fluxos). Depende do serviço do
  Kalango Fluxos, que não está na ISO. Detalhes em
  [docs/GATILHO-INTERNET.md](https://github.com/kalangoos/kalangoos/blob/main/docs/GATILHO-INTERNET.md).
- **Criar fluxos pelo chat com o modelo pequeno, por etapas:** o fluxo nasce por ferramentas de rascunho
  (`fluxos.planejar` … `fluxos.concluir`) e termina sempre na aprovação da pessoa. **Mudança de padrão:** o
  `fluxos.criar` (o JSON inteiro) deixou de ser oferecido ao modelo; `KALANGOD_FLUXOS_AUTORIA=classico` o traz de volta.

## Segurança

- **Apps criativos:** uma pasta concedida a um app não pode mais ser trocada por um link (por exemplo para `~/.ssh`)
  por quem escreve em volta dela. A escolha recusa caminhos arriscados, e as pastas são conferidas de novo ao entrar na
  sessão, antes de qualquer abertura do app pelo shell e quando algo muda em volta; uma pasta que mudou sai das
  permissões e aparece na Linha do tempo.
- **Gatilho pela internet:** o cartão para ligar um gatilho diz se ele é público (sem assinatura), o nível e o endereço,
  e o `kalangod` só confirma se o gatilho ainda é o do cartão depois do sim.

A lista completa está no [CHANGELOG.md](https://github.com/kalangoos/kalangoos/blob/main/CHANGELOG.md).

## Como instalar e verificar

Baixe da [página de versões](https://github.com/kalangoos/kalangoos/releases) a ISO `kalangoos-0.1.2-amd64.iso`, a soma
`SHA256SUMS-iso.txt` e, se quiser, `kalangoos-0.1.2-linux-x86_64.tar.gz` com `SHA256SUMS.txt`.

1. **Confira a soma:** `sha256sum -c SHA256SUMS-iso.txt` (as somas também estão no texto da versão no GitHub).
2. **Confira a origem do build** (precisa da CLI do GitHub):
   `gh attestation verify kalangoos-0.1.2-amd64.iso --repo kalangoos/kalangoos`
3. **Teste numa VM:** `KALANGO_ISO=/caminho/kalangoos-0.1.2-amd64.iso imagem/rodar-iso.sh novo`
   (veja os detalhes nas [notas da 0.1.0](NOTAS-0.1.0.md)).
4. **Modelos de IA:** a ISO não leva modelo local; escolha a fonte na etapa "Modelos de IA" do instalador ou depois em
   Ajustes e na Central de Apps.

## Problemas conhecidos

- **Traduções de IA, sem revisão por pessoas nativas.** Podem ter erros de sentido, de tom, de termos técnicos e de
  formalidade; as Configurações › Idioma avisam na tela. Não houve conferência visual das telas em cada idioma, nem de
  datas, números e glifos CJK na ISO instalada, e o modelo local não foi testado com conversas nesses idiomas (o texto
  que o modelo lê continua em inglês). Revisões são bem-vindas:
  [docs/i18n/REVISAO.md](https://github.com/kalangoos/kalangoos/blob/main/docs/i18n/REVISAO.md).
- **Criar fluxos pelo chat com o modelo pequeno acerta cerca de 2 em 3 pedidos novos** (67 % no conjunto C; 64 % e 62 %
  nos conjuntos D e E, mais recentes), abaixo do alvo de 90 %. Por isso o `fluxos.criar` está desligado por padrão; a
  aprovação da pessoa continua obrigatória. Medições em
  [docs/FLUXOS-NO-CHAT.md](https://github.com/kalangoos/kalangoos/blob/main/docs/FLUXOS-NO-CHAT.md).
- **O Kalango VirtualDesk e o Kalango Fluxos não estão na ISO.** A leitura dos repositórios privados pela esteira de
  lançamento ainda não está ligada e fica fora desta versão. Sem o serviço de fluxos, o gatilho pela internet e os fluxos
  não rodam na imagem publicada.
- O gatilho pela internet não foi exercitado com um túnel real da Cloudflare, e o serviço de fluxos ainda ignora a
  confirmação do cartão que o `kalangod` devolve (a recusa acontece no `kalangod`).
- Apps criativos: a instalação ponta a ponta (o runtime de 283 MB), o modo em janela, a atualização automática e abrir
  os 17 apps com as novas negações não foram validados. Um override global amplo do Flatpak feito pela própria pessoa
  bloqueia o uso dos apps pelos agentes até ser removido. O detalhe da Central ainda não marca uma pasta recusada.
- O isolamento de voz do Kalango Audio ainda não entra: a ISO leva o Audio da 0.1.1 com os idiomas novos.
- O teste da ISO na esteira (sessão ao vivo, instalação e primeiro boot) não exercita a tela de aprovação gráfica nem a
  tela do Remote. Elas foram corrigidas e conferidas à mão na 0.1.1, e as listas do `kalangod` que as liberam não
  mudaram desde então.
- Continua valendo o que as [notas da 0.1.1](NOTAS-0.1.1.md) e as [da 0.1.0](NOTAS-0.1.0.md) dizem: testada só em VM,
  sem modelo local na ISO, Remote ainda não testado entre dois computadores físicos, Mosh sem teste, três ajustes do
  Remote desativados.

## Segurança e licença

Falhas de segurança: **Security › Report a vulnerability** neste repositório
([SECURITY.md](https://github.com/kalangoos/kalangoos/blob/main/SECURITY.md)). Licenças como na 0.1.1: código GPL-3.0 ou
posterior; Remote derivado do RustDesk (AGPL-3.0); a fonte Noto Sans CJK (OFL-1.1) e os apps do projeto Craft, baixados
a pedido, estão em [LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md).

---

## English

**KalangoOS 0.1.2** is still a technical preview, tested in VMs only.

New: the UI in nine languages (English, Brazilian Portuguese, Spanish, French, Italian, German, Japanese, Russian and
Simplified Chinese) in the system, Kalango Remote and Kalango Audio; Creative Apps (the 17 Craft apps) installable on
request from the App Center and usable by agents with approval; a signed public internet trigger that starts a flow
through an approved Cloudflare tunnel; step-by-step flow drafting in chat for the small model. **Default change:**
`fluxos.criar` (the whole JSON) is no longer offered to the model; `KALANGOD_FLUXOS_AUTORIA=classico` restores it.

Security: a folder granted to a Creative App can no longer be swapped for a link, and is re-checked at session start and
before every launch; the card for turning a trigger on shows whether it is public, its level and address.

Known issues: **the seven new catalogs are AI translations not reviewed by native speakers**; small-model flow authoring
succeeds on about 2 in 3 fresh requests (67 % on set C, 64 % and 62 % on the newer sets D and E; target 90 %), so it is
off by default; **Kalango VirtualDesk and Kalango Fluxos are not in the ISO** (the release workflow does not read the
private repositories yet), so the internet trigger and flows do not run on the published image; the trigger was never
run through a real Cloudflare tunnel; Creative Apps end-to-end install, windowed mode and auto-update are unvalidated;
the release VM test does not exercise the graphical approval or the Remote screen (fixed and checked by hand in 0.1.1,
their kalangod allowlists unchanged since); the 0.1.1 and 0.1.0 limitations still apply.

Verify: `sha256sum -c SHA256SUMS-iso.txt`, `gh attestation verify kalangoos-0.1.2-amd64.iso --repo kalangoos/kalangoos`.
