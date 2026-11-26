# KalangoOS em vários idiomas — plano

Os mesmos nove idiomas do site (kalangoos.com): inglês (o texto do código), português do Brasil, espanhol, francês,
italiano, alemão, japonês, russo e chinês simplificado. Catálogos em `crates/kalango-i18n/locales/<locale>.json`
(`es`, `fr`, `it`, `de`, `ja`, `ru`, `zh_CN`), com as mesmas chaves do `pt_BR.json`. Termos em
[`GLOSSARIO.md`](GLOSSARIO.md); idiomas que pedem revisão de quem fala, em [`REVISAO.md`](REVISAO.md).

Tratamento da pessoa, igual ao do site: alemão *Sie*, francês *vous*, espanhol *tú*, italiano *tu*, russo *вы*,
japonês です/ます.

## F1 · Base

- [x] `kalango-i18n` carrega o catálogo de qualquer idioma da lista, normaliza o locale (`pt` → `pt_BR`, `es_*` →
      `es`, `zh_CN`/`zh_SG`/`zh_Hans` → `zh_CN`) e cai no inglês quando não há catálogo (`zh_TW`/`zh_HK`/`zh_Hant`:
      ainda não há chinês tradicional — a tela diz isso)
- [x] Nomes nativos dos idiomas num lugar só (`kalango_i18n::language_name`), usados pelo instalador e pelas
      Configurações
- [x] Testes na CI: cada catálogo tem exatamente as chaves do `pt_BR`, os mesmos `{marcadores}`, nenhum valor vazio,
      os mesmos espaços nas pontas e o mesmo formato e ordem de linhas do `pt_BR`; um teste de fumaça carrega cada
      idioma e confere textos conhecidos
- [x] `scripts/i18n-faltando.py`: lista as chaves que faltam em cada catálogo (e aplica traduções novas, na ordem do
      `pt_BR`); regra para quem contribui no `AGENTS.md` e no `CONTRIBUTING.md`

## F2 · Sistema

- [x] Catálogos completos para os sete idiomas novos (shell, Configurações, instalador, aprovação, entrada, Central de
      Apps, notificações, mensagens do `kalangod` e do motor que aparecem para a pessoa)
- [x] Configurações › Data e idioma: os nove idiomas com o nome nativo; aplicar avisa que o idioma novo entra no
      próximo início de sessão; idioma sem tradução avisa que a tela fica em inglês
- [x] Instalador: primeira tela lista os nove idiomas; trocar o idioma refaz a tela no idioma escolhido; o sistema
      instalado recebe o `LANG` escolhido
- [x] Imagem: gera os locales dos nove idiomas e guarda as traduções do sistema para eles
- [x] Agente: responde no idioma da pessoa (regra no prompt com o nome do idioma). O prompt e as descrições das
      ferramentas ficam em inglês para o modelo nos idiomas novos; o `pt_BR` continua como está (é o que os testes com
      o modelo real cobrem)
- [x] Agente: palavras dos pedidos que trazem as ferramentas certas também nos idiomas novos

## F3 · Apps

- [x] Kalango Remote (`kalangoos/app`), Kalango Audio, Kalango VirtualDesk: catálogos dos sete idiomas, mesmos testes
- [ ] Kalango Flow (`kalango-fluxos`): ver quais mensagens do serviço aparecem na tela e como traduzi-las
- [x] Entradas `.desktop` (Name/Comment/Keywords) e metainfo AppStream por idioma (sistema, Audio e VirtualDesk;
      o Kalango Remote não tem entrada própria no repositório)

## F4 · Fontes e tela

- [x] Glifos CJK e cirílicos na imagem com o menor pacote que cobre a interface. Cirílico: a Inter já cobre. CJK:
      `fonts-noto-cjk` só com `NotoSansCJK-Regular.ttc` (o negrito e as serifadas ficam fora pelo `path-exclude`):
      19,5 MB no disco, cerca de 13 MB comprimidos (zstd 15, o nível do `sqfstar`). Os locales compilados dos sete
      idiomas novos somam cerca de 2 MB comprimidos e as traduções do sistema, poucos MB. Estimativa para a ISO:
      +15 a +18 MB (medido arquivo a arquivo; a ISO inteira não foi construída). Alternativa menor (~1,6 MB,
      `fonts-wqy-microhei`, Apache-2.0) desenha o japonês com glifos chineses.
- [x] Auditoria de transbordo em japonês, alemão e russo numa sessão aninhada (`sessao/tamanhos.sh` a 1366×768,
      que mede se algum elemento passa da largura; **não houve passada visual com capturas de tela: não há imagens
      guardadas e ninguém que fale esses idiomas olhou as telas**:
      Configurações › Agentes e IA, Privacidade e segurança e Sistema; Central › Conversas; Central de Apps). Três
      transbordos corrigidos: título da página e tipo das linhas da tabela de permissões (ja), títulos das linhas
      do painel de Sistema (de), linha de abas mais larga que a janela (ru, agora rola de lado). Depois: 0
      problemas nos três idiomas. Em pt_BR, Central › Permissões e Uso já passavam da largura antes desta
      mudança (fora do escopo).
- [x] Datas e números pelo locale (exemplo de moeda nas Configurações)

## F5 · Documentos e site

- [x] Relatório: termos do site × glossário, sem mudar o site — [`SITE-TERMOS.md`](SITE-TERMOS.md)
- [ ] Proposta (não feita): README em inglês como principal e um `README.<idioma>.md` curto por idioma (o que é, como
      instalar, link para o site no idioma), gerado do mesmo texto que o site já tem traduzido; o README completo
      continua num idioma só para não envelhecer em nove

## F6 · Próxima leva (só proposta)

- **Idiomas**: coreano (`ko`), híndi (`hi`), indonésio (`id`), turco (`tr`), polonês (`pl`), holandês (`nl`),
  ucraniano (`uk`), vietnamita (`vi`), tailandês (`th`), português de Portugal (`pt_PT`), chinês tradicional
  (`zh_TW`, que hoje cai no inglês), árabe (`ar`) e hebraico (`he`).
  - Polonês, russo, ucraniano e árabe pedem **formas de plural**: uma função `trn(singular, plural, n)` com as
    regras do CLDR, em vez de frases que servem a qualquer número.
  - Árabe e hebraico pedem **layout da direita para a esquerda**: `gtk::Widget::set_default_direction` pelo locale,
    ícones de seta espelhados, e conferir cada tela (barras laterais nas bordas, dock, Central) — trabalho de design
    antes do código.
  - Tailandês não separa palavras por espaço: quebra de linha pelo Pango (já faz) e testar rótulos com `ellipsize`.
  - Fontes: Noto Sans Thai, Devanagari, Arabic e Hebrew (cada uma pequena, um peso); coreano já vem no Noto Sans CJK.
- **Weblate** (plano gratuito para projetos livres em hosted.weblate.org): um componente por catálogo — sistema
  (`crates/kalango-i18n/locales/*.json`), Kalango Remote, Kalango Audio e Kalango VirtualDesk (um componente por
  tela, `locales/*/<tela>.json`). Formato "JSON file" (chave = texto em inglês, valor = tradução), idioma de origem
  inglês com o `pt_BR` como mais um idioma; `{marcadores}` com a verificação "placeholders" configurada para
  `\{[a-z_]+\}`; glossário do Weblate importado de `GLOSSARIO.md`. O Weblate abre pull requests; os testes do
  `kalango-i18n` continuam sendo a porta (chaves, marcadores, ordem: o Weblate preserva a ordem do arquivo-base
  quando o "template" é o `pt_BR.json`... no modelo monolíngue o template seria um `en.json` gerado das chaves).
  Ponto em aberto: o Weblate escreve o JSON com o recuo que se configurar (usar 1 espaço, `ensure_ascii` falso).

## Design

O seletor de idioma já existe nas telas do Design System (Instalador › 1 Boas-vindas e Configurações › Data e
idioma): só mudam os itens da lista (os nove idiomas com o nome nativo). Mudanças visíveis que pedem quadro:

- Configurações › Data e idioma: a linha "Outros idiomas" ganha, quando o idioma escolhido não tem tradução
  (chinês tradicional, coreano…), uma segunda linha de aviso ("… ainda não tem tradução: as telas aparecem em
  inglês").
- (Opcional) Configurações › Data e idioma: um botão "Encerrar a sessão agora" ao lado do aviso "Um idioma novo
  entra no próximo início de sessão" — não implementado; hoje fica só o aviso.
- Instalador › 1 Boas-vindas: trocar o idioma refaz a janela no idioma escolhido (mesma tela, sem quadro novo).
