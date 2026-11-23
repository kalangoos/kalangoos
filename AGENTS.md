# KalangoOS — instruções para agentes de código

Este arquivo é para agentes de código (Claude Code, Codex e outros) e para quem os usa. As regras valem também
para pessoas. O `CLAUDE.md` só aponta para cá.

## Regras da casa

1. **Uma pessoa responde pelo que o agente faz.** Quem roda o agente responde pelo que ele abre, comenta e envia.
2. **Confira antes de reivindicar.** Antes de escrever código, leia a issue e os comentários, procure pull requests
   com `#N` e as palavras-chave, e veja os commits recentes no `main`. Comente na issue que vai trabalhar nela.
3. **Verifique, não adivinhe.** Abra a definição antes de usar uma função. Confirme a causa no código, mesmo quando
   alguém já apontou o lugar.
4. **Correções estreitas e testadas.** Corrija o que a issue descreve, onde está o defeito, com um teste que falha
   sem a correção.
5. **Revise o diff antes do pull request.** Os nomes existem como usados, o teste falha sem a correção, nada fora do
   assunto mudou e a descrição é verdadeira.
6. **Diga só o que rodou.** Diga o que você testou e o que não conseguiu rodar. Nunca diga que está pronto algo que
   ainda tem pendências conhecidas: implemente e valide antes, ou diga claramente o que está bloqueado e por quê.
7. **O `main` muda só por pull request.** Sem push direto.
8. **Pare e pergunte** sobre: licenças fora da lista abaixo, publicação, versões, um remoto novo, segredos, gastar
   dinheiro, git destrutivo, instalar algo no sistema (sudo, kernel, pacotes) e qualquer coisa difícil de desfazer.
   No resto, continue trabalhando.
9. **Sem segredos e sem dados pessoais** no repositório, nos logs, nas capturas e nos testes: nomes, redes Wi-Fi,
   e-mails e arquivos são sempre inventados.
10. **A CI é compartilhada.** Não mude workflows num pull request de funcionalidade ou de correção; abra outro.
11. **Proteções dizem por quê.** Uma verificação que parece removível explica o que ela evita. Leia antes de mexer.
12. **Instruções que qualquer um consegue ler.** Nada de notas sobre uma máquina específica nem links para arquivos
    que o leitor não consegue abrir.

### Licenças aceitas sem perguntar

Dependências e recursos com identificadores SPDX desta lista: `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`,
`ISC`, `Zlib`, `0BSD`, `Unlicense`, `Unicode-3.0`, `MPL-2.0`, `CDLA-Permissive-2.0`, `OFL-1.1`, `Ubuntu-font-1.0`,
`CC0-1.0`, `CC-BY-4.0`. Bibliotecas do sistema ligadas dinamicamente (GTK, VTE, Poppler, GStreamer) e o niri
(GPL-3.0-or-later) já estão aceitos. Qualquer outra: pergunte. O `deny.toml` confere as crates na CI.

## O que é o projeto

Uma distribuição Linux (base Ubuntu LTS) em que agentes de IA operam o sistema com aprovação e desfazer. Peças:

| Caminho | O quê |
|---|---|
| `crates/kalango-shell` | O desktop inteiro em Rust + GTK 4 sobre layer-shell: barra, dock, barras laterais, área de trabalho, bloqueio, apps do sistema (Arquivos, Configurações, Terminal, Editor…), Central de agentes |
| `crates/kalangod` | Serviço privilegiado que fala MCP; níveis de poder; regras no kernel (eBPF LSM em `ebpf/`, Landlock, seccomp, cgroups); pontos de restauração |
| `crates/kalango-agente` | Motor dos agentes: llama.cpp com LFM3 local, defesa contra instruções escondidas, permissões, memória |
| `crates/kalango-aprovacao`, `kalango-entrada`, `kalango-instalador` | Tela de aprovação, tela de entrada (greetd), instalador |
| `crates/kalango-movimento`, `kalango-i18n` | Molas das animações; traduções |
| `sessao/` | Sessão de desenvolvimento (niri aninhado), patches do niri, arquivos da imagem (`dist/`), a suíte `testar.sh` |
| `design/tokens/` | Tokens exportados do Design System (fonte e contribuições: [kalangoos/design](https://github.com/kalangoos/design)) |
| `openspec/` | Especificação por fases |
| `docs/` | Visão, arquitetura, segurança, requisitos, plano |

## Não negociáveis

- **Design primeiro.** Toda tela, ou mudança visível, existe no Design System (FigCraft) antes do código — fonte e PRs em
  [kalangoos/design](https://github.com/kalangoos/design) — e a implementação bate com ele: compare capturas lado a lado,
  ampliadas, inclusive o alinhamento de cada ícone.
- **Nenhuma tela de outro sistema.** Nada de apps ou telas do GNOME, Yaru ou Adwaita: os apps do sistema são do
  KalangoOS. A folha de estilo para apps de terceiros é só um recurso de reserva.
- **Telas que se ajustam ao espaço**: colunas que cabem, conteúdo que rola, painéis que cedem. Tamanhos de janela
  vêm do tamanho da saída do compositor, não de números fixos.
- **Visual**: sem sombras; cantos sutis (8 janelas e painéis, 6 cartões, 4 controles; círculos só em chaves,
  pontos e avatares); cores só dos tokens.
- **Rust** em tudo o que é programa nosso. **Identificadores novos em inglês**; não renomeie os que existem em
  português. **Textos da interface** passam por `tr("English")` com a tradução no catálogo `pt_BR.json`.
- **Testes do motor de agentes usam o modelo real** (LFM3 pelo llama.cpp). Nunca um modelo falso ou roteirizado;
  o que é determinístico vira teste de unidade de funções puras.
- **Testes nunca tocam a pasta pessoal real**: use as pastas do teste (`crate::pastas`, `KALANGO_AREA_TESTE`,
  `XDG_*` temporários) e confira depois que nada vazou.

## Construir e testar

```sh
poc/setup-deps.sh                                   # dependências que o Ubuntu não tem, em poc/.deps (sem sudo)
export PKG_CONFIG_PATH="$PWD/poc/.deps/root/usr/lib/x86_64-linux-gnu/pkgconfig"
cargo clippy --workspace --all-targets --locked     # a CI reprova erros do clippy
cargo test --workspace --locked --exclude kalango-agente
cargo test -p kalango-agente --bins --locked        # os testes de tests/ precisam do modelo local
sessao/rodar.sh                                     # a sessão numa janela
sessao/testar.sh                                    # a suíte da sessão (demorada)
# testes de GTK (precisam de KALANGO_TESTE_GTK=1 e de uma tela): o GTK só vale na thread que o iniciou (gtk4 rt.rs),
# então cada um roda sozinho, um filtro por execução:
KALANGO_TESTE_GTK=1 xvfb-run -a cargo test -p kalango-shell --bin kalango-shell -- --test-threads=1 release_tests
KALANGO_TESTE_GTK=1 xvfb-run -a cargo test -p kalango-shell --bin kalango-shell -- --test-threads=1 closing_the_window_stops_the_listening
```

Formatação: o `rustfmt.toml` vale para código novo, mas não reformate arquivos inteiros num pull request até a
reformatação única do projeto (a CI só avisa sobre `cargo fmt --check`).

## Armadilhas conhecidas

Coisas que custaram horas. Leia antes de depurar uma falha de tela ou de teste da sessão.

- **Sessão aninhada:** a tecla Mod do niri aninhado é o Alt (atalhos com Alt não chegam ao app). O aviso "captura
  feita" do niri cobre o topo da tela por alguns segundos: espere os avisos sumirem antes de clicar. O `xdotool type`
  perde acentos: digite ASCII quando o teste lê o texto de volta.
- **Nunca edite `sessao/testar.sh` enquanto ele roda** (o bash lê o arquivo enquanto executa) e não compile durante
  uma rodada: `rodar.sh` e `isolada.sh` rodam `cargo build` e o supervisor reinicia o shell quando o binário muda.
- **`pkill -f`** com um padrão que aparece no seu próprio comando mata o seu shell: filtre por argumentos exatos.
- **GTK:** a folha de estilo do usuário (`~/.config/gtk-4.0/gtk.css`) tem prioridade USER — as nossas carregam em
  `USER + 1`. `button { all: unset }` zera o padding: todo botão de texto define o seu. `min-height` não conta o
  padding. Uma superfície layer totalmente transparente não recebe ponteiro no niri: use `alpha(#000, 0.01)`.
  `gtk::Fixed` dá aos filhos o tamanho mínimo; rótulos em painéis de largura fixa precisam de `ellipsize` e
  `max_width_chars`.
- **Tema claro:** o shell monta o claro trocando cada cor do escuro pela clara do mesmo token, **pelo valor**: dois
  tokens com o mesmo valor no escuro viram a mesma cor no claro (a borda da bolinha, igual à bolinha no escuro, ficava
  branca no claro). Um token novo precisa de um valor próprio no escuro.
- **D-Bus:** uma tupla com um `glib::Variant` dentro vira `v` (caixa); monte respostas com
  `Variant::tuple_from_iter` e teste a assinatura.
- **niri:** recusa ações de IPC com a sessão bloqueada; só dá a posição de janelas lado a lado com o patch 0004;
  remove um espaço de trabalho vazio sem foco (os índices mudam).
- **`kalangod`** resolve `~` pelo passwd, não por `$HOME`: testes que capturam arquivos precisam liberar a pasta do
  teste explicitamente.
- **Modelo:** as ferramentas aparecem para o modelo com `__` no lugar do ponto (`agente__plano`).

## Pull requests

Use o modelo de pull request. Diga o que testou (comandos e resultado) e o que não conseguiu rodar. Mensagens de
commit em português. Quem abre o pull request responde por ele, tenha sido escrito por uma pessoa ou por um agente.
