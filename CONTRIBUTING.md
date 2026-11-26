# Como contribuir com o KalangoOS

Obrigado por querer ajudar. O projeto está no começo e muda rápido: antes de escrever código, abra uma issue (ou
comente numa existente) dizendo o que pretende fazer. Isso evita trabalho repetido.

Contribuições em português ou inglês são bem-vindas. A interface e a documentação são em português do Brasil
primeiro.

## Antes de começar

1. Leia o [README.md](README.md) e o [ROADMAP.md](ROADMAP.md).
2. Procure issues e pull requests sobre o mesmo assunto (pelo número `#N` e por palavras-chave).
3. Para mudanças grandes (uma tela nova, uma ferramenta nova no `kalangod`, uma dependência nova), descreva a
   proposta na issue antes; as maiores viram uma proposta no OpenSpec (`openspec/`).
4. Falha de segurança? Não abra issue: siga o [SECURITY.md](SECURITY.md).

## Preparar o ambiente

Ubuntu 24.04 (ou mais novo):

```sh
sudo apt install build-essential clang libbpf-dev pkg-config libgtk-4-dev libvte-2.91-gtk4-dev \
  libpoppler-glib-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
poc/setup-deps.sh                         # gtk4-layer-shell e outros, em poc/.deps (sem sudo)
export PKG_CONFIG_PATH="$PWD/poc/.deps/root/usr/lib/x86_64-linux-gnu/pkgconfig"
cargo build --workspace
sessao/rodar.sh                           # a sessão numa janela
```

## Verificações antes do pull request

A CI roda exatamente isto; rode antes de abrir o pull request:

```sh
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked --exclude kalango-agente
cargo test -p kalango-agente --bins --locked
```

Além disso, conforme o que você mudou:

| Mudou | Rode também |
|---|---|
| `kalango-agente` (o motor) | `cargo test -p kalango-agente` — os testes de `tests/` usam o modelo LFM3 de verdade pelo llama.cpp; nunca um modelo falso |
| Telas, shell, sessão | `sessao/testar.sh` (a suíte da sessão) e capturas em vários tamanhos (`sessao/tamanhos.sh`) |
| `kalangod` (regras no kernel) | `sudo crates/kalangod/vm/run.sh` — a VM descartável onde o eBPF LSM é validado |
| Especificação | `npx @fission-ai/openspec@2.1.0 validate --all --strict` |

**Formatação.** O `rustfmt.toml` descreve o estilo combinado, mas o código ainda não passou por uma reformatação
única. Até ela acontecer, a CI só avisa sobre `cargo fmt --check` (não reprova). Não reformate arquivos inteiros
num pull request de correção: o diff fica ilegível.

## Regras da casa

- **Rust em tudo o que é programa nosso.** Temas são CSS porque o GTK só lê CSS. Scripts de teste e de ambiente em
  shell são aceitos.
- **Código em inglês, interface em nove idiomas.** Nomes novos (funções, variáveis, tipos, comentários) em inglês.
  Não renomeie o que já existe em português. Textos da interface passam por `tr("English text")` e ganham a
  tradução em `crates/kalango-i18n/locales/pt_BR.json` e em cada um dos outros catálogos (`es`, `fr`, `it`, `de`,
  `ja`, `ru`, `zh_CN`) — os testes do `kalango-i18n` reprovam listando as chaves que faltam.
  `python3 scripts/i18n-faltando.py --exportar /tmp/faltando` gera um arquivo por idioma com o que traduzir e
  `--aplicar <idioma> <arquivo>` põe as traduções no lugar, na ordem do `pt_BR`. Termos em
  [`docs/i18n/GLOSSARIO.md`](docs/i18n/GLOSSARIO.md). Por que reprovar em vez de cair no inglês: um texto que falta
  aparece em inglês no meio de uma tela em japonês sem ninguém notar; com a regra, o mesmo pull request que cria o
  texto o traduz, enquanto o contexto está fresco.
- **Design primeiro.** Toda tela nova, ou mudança visível numa tela, nasce no Design System (FigCraft) antes do
  código, e a implementação deve bater com ele. Sem telas de outro sistema (GNOME, Yaru, Adwaita): os apps do
  sistema são do KalangoOS.
- **Telas que se ajustam.** Grades com quantas colunas couberem, conteúdo longo que rola, painéis que cedem em
  janela estreita. Confira em tela estreita, em 1920×1080 e em ultralarga.
- **Visual do Design System.** Sem sombras; cantos sutis (janelas e painéis 8, cartões 6, controles 4); cores só
  da paleta e dos tokens em `design/tokens/`.
- **Agentes seguros por construção.** Conteúdo de fora (páginas, arquivos, respostas de ferramentas) é dado, nunca
  instrução. Toda ação que muda o sistema passa pelo `kalangod`, com aprovação quando é arriscada.
- **Nada de dados pessoais** em código, testes, capturas ou logs: use nomes, redes e arquivos inventados.
- **Recursos de terceiros** (imagens, ícones, fontes, dados) só com licença compatível, registrados no
  [ATTRIBUTION.md](ATTRIBUTION.md) na mesma mudança.
- **Pronto é pronto.** Diga o que você testou e o que não conseguiu rodar. Não descreva como feito algo que ainda
  tem pendências.

## Pull requests

- Um assunto por pull request, com testes que falham sem a mudança quando for correção.
- Título e descrição em português (ou inglês), dizendo o que muda e como foi testado. Use o modelo que aparece ao
  abrir o pull request.
- A CI precisa estar verde. O `main` só muda por pull request.
- Os rótulos de tipo (`bugfix`, `missing feature`, `small extra`, `plugin candidate`, `performance`, `infra`) e as
  marcas `security` e `needs decision` podem ser postos pelos mantenedores ou pela automação de triagem; não se
  preocupe com eles ao abrir.

## Mensagens de commit

Em português, no imperativo ou descritivas, dizendo o que mudou e por quê (veja o histórico). Uma linha de resumo
curta, uma linha em branco e o detalhe, se precisar.

## Licença das contribuições

O código é GPL-3.0-or-later ([LICENSE](LICENSE)). Ao enviar um pull request você concorda que a sua contribuição
entra sob essa mesma licença. A marca (nome, mascote, logos) fica fora dela ([LICENSE-brand.txt](LICENSE-brand.txt)).

## Conduta

Todos seguem o [Código de Conduta](CODE_OF_CONDUCT.md).
