# Atribuições

Todo recurso que não é código (imagens, ícones, fontes, dados) aparece aqui com a origem e a licença. A verificação
"Regras da casa" da CI reprova um arquivo de imagem, fonte ou PDF que não case com algum caminho desta página.

> O código do KalangoOS é GPL-3.0-or-later ([LICENSE](LICENSE)). Onde a tabela diz "a definir", o recurso segue a
> licença do código até uma decisão própria; a marca segue [LICENSE-brand.txt](LICENSE-brand.txt).

## Recursos gerados por IA

A marca e as ilustrações do KalangoOS foram geradas com o serviço Higgsfield (modelo GPT Image) a partir de
descrições escritas pelo projeto, a partir de 15 de novembro de 2026, e depois ajustadas e vetorizadas
(`vtracer`, MIT). A proteção por direito autoral de imagens geradas por IA varia de país para país e os termos do
serviço precisam ser revisados para uso comercial antes da versão 1.0. O nome KalangoOS, o mascote e os logos são
a marca do projeto e **não** entram na licença do código.

| Caminho | Origem | Licença | Observação |
|---|---|---|---|
| `assets/brand/**` (logo principal, símbolo, ícone, horizontal, nome, versões de uma cor e do tema claro, conceitos, `svg/`, `1024/`, `icones/`) | Gerado por IA (Higgsfield, GPT Image) e vetorizado | Marca do projeto (`LICENSE-brand.txt`) | Mascote "calango" |
| `assets/icons/kalango/**` (19 ícones dos apps do sistema, PNG e SVG) | Gerado por IA (Higgsfield, GPT Image) no estilo do logo, vetorizado | Gerado por IA; licença a definir | |
| `crates/kalango-shell/assets/*.png`, `crates/kalango-instalador/assets/*.png` (logo, logo do tema claro, símbolo) | Cópias reduzidas da marca acima | Marca do projeto | |
| `wallpapers/sdrustos-16x9-4k.png`, `wallpapers/sdrustos-16x9-4k-claro.png` | Papel de parede do KalangoOS, gerado por IA (Higgsfield, GPT Image) | Gerado por IA; licença a definir | |
| `crates/kalango-shell/assets/loja-destaque.png` | Ilustração da Central de Apps, gerada por IA (Higgsfield, GPT Image) | Gerado por IA; licença a definir | |
| `.github/assets/**` (cabeçalho do README e prévia social; o avatar, o banner e as prévias dos outros repositórios ficam no repositório `.github` da organização) | Composição feita no FigCraft (página "GitHub · artes" do Design System) com a marca acima, formas planas e textos | Marca do projeto | Nenhuma imagem nova gerada por IA |

## Recursos próprios

| Caminho | Origem | Licença |
|---|---|---|
| `crates/kalango-shell/assets/ui/*.svg` | Ícones de interface desenhados no Design System (FigCraft) e exportados | A definir |
| `crates/kalango-instalador/assets/icones/*.svg` | Ícones de linha da etapa "Modelos de IA" do instalador (rede, servidor, pen drive, relógio), desenhados por agente de IA no estilo do Design System e escritos como SVG à mão | A definir | |
| `crates/kalango-aprovacao/capturas/*.png` | Capturas da tela de aprovação em sessão de teste | A definir |
| `docs/pagina/img/**` | Capturas e exportações do Design System e da sessão de teste | A definir |
| `crates/kalangod/tests/fixtures/*.pdf` | Arquivo de teste gerado pelo projeto | A definir |
| `assets/sounds/kalango/**` | Sons do sistema compostos pelo projeto no LMMS só com sintetizadores (ZynAddSubFX, TripleOscillator, OpulenZ, BitInvader, Organic), sem samples de terceiros | CC0-1.0 |
| `site/public/**` (capturas da sessão de teste, ícones dos apps, marca reduzida, papéis de parede, imagem de compartilhamento) | Capturas e cópias reduzidas dos recursos acima, feitas para o site. A marca, os ícones dos apps e os papéis de parede são gerados por IA (Higgsfield, GPT Image); as capturas são da sessão do próprio KalangoOS, com dados de teste | Marca do projeto, nos logos (`LICENSE-brand.txt`); ícones e papéis de parede: a definir, como os originais; capturas: como o código (GPL-3.0-or-later) |

## Dados de terceiros

| Caminho | Origem | Licença |
|---|---|---|
| `crates/kalango-shell/assets/emoji-pt.tsv` | Nomes e palavras-chave dos emojis em português, montados a partir do [Unicode CLDR](https://github.com/unicode-org/cldr-json) (anotações) e do `emoji-test.txt` do Unicode | Unicode License v3 (`Unicode-3.0`); texto em `NOTICE` |

## Legado (origem a confirmar)

Restos do primeiro protótipo ("Lunar", para GNOME). A origem destas imagens precisa ser confirmada antes de serem
publicadas; o plano de publicação deixa estes caminhos de fora até lá.

| Caminho | Situação |
|---|---|
| `assets/icons/*.png` (fora de `kalango/`), `assets/thumbs/**`, `icons/Lunar/**` | Arte de um projeto anterior do autor; licença a confirmar |
| `wallpapers/lunar-*.png` | Papéis de parede do protótipo; usados pela sessão para telas altas e largas; licença a confirmar |

## Software de terceiros usado pelo KalangoOS (fora deste repositório)

Nada disto é copiado para o repositório; os scripts baixam ou compilam cada um, e a imagem do sistema vai levar
as licenças junto.

| Projeto | Para quê | Licença |
|---|---|---|
| [niri](https://github.com/YaLTeR/niri) | O compositor Wayland; o KalangoOS aplica os patches de `sessao/niri/` | GPL-3.0-or-later (os patches também) |
| [Smithay](https://github.com/Smithay/smithay) | Base do niri | MIT |
| [gtk4-layer-shell](https://github.com/wmww/gtk4-layer-shell) | Superfícies do shell e bloqueio de tela | MIT |
| GTK 4, VTE (GTK 4), Poppler (GLib), GStreamer | Interface, terminal, PDF, mídia (bibliotecas do sistema, ligadas dinamicamente) | LGPL-2.1-or-later; LGPL-3.0-or-later; GPL-2.0-or-later; LGPL-2.1-or-later |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) | Executa os modelos locais | MIT |
| [LFM3](https://huggingface.co/LiquidAI) (Liquid AI), em GGUF | Modelos locais padrão (texto, visão, áudio), baixados na primeira vez | LFM Open License v1.0 (licença própria, não OSI: conferir os termos antes de embutir os modelos na imagem) |
| Kalango Remote (repositório próprio, na ISO) | Acesso remoto; derivado do [RustDesk](https://github.com/rustdesk/rustdesk) | AGPL-3.0 |
| Kalango Audio (repositório próprio, na ISO) | Captura, tratamento e gravação de som | GPL-3.0-or-later |
| [Moli](https://github.com/lexmount/moli) | Navegador sem tela dos agentes, isolado pelo `kalangod` | Apache-2.0 |
| Programa eBPF `crates/kalangod/ebpf/guarda.bpf.c` | Regras no kernel | Declara `GPL` ao kernel (exigência dos ganchos LSM) |
| Ubuntu LTS e o kernel Linux | Base do sistema | As de cada pacote; kernel GPL-2.0 |

## Fontes

Nenhum arquivo de fonte está neste repositório. A sessão e a imagem usam os pacotes do sistema. O site
(`site/`) serve as fontes ele mesmo, sem pedir nada a terceiros: o `npm ci` baixa os pacotes `@fontsource/inter`,
`@fontsource/jetbrains-mono`, `@fontsource/jost`, `@fontsource/roboto` e `@fontsource/ubuntu` (subconjunto latino,
WOFF2), que o build copia para o `dist/`; as versões ficam no `site/package-lock.json`.

| Família | Uso | Licença |
|---|---|---|
| Inter | Texto da interface; também no site | OFL-1.1 |
| URW Gothic (`fonts-urw-base35`) | Títulos | AGPL-3.0 com exceção para fontes |
| JetBrains Mono | Código e números; também no site | OFL-1.1 |
| Jost (`@fontsource/jost`) | Títulos do site no tema escuro, no lugar do URW Gothic (sem licença para a web) | OFL-1.1 |
| Roboto | Tema claro (referência do Design System); também no site (`@fontsource/roboto`) | OFL-1.1 |
| Ubuntu | Títulos do tema claro; também no site (`@fontsource/ubuntu`) | Ubuntu Font Licence 1.0 (`Ubuntu-font-1.0`) |

## Site: área de membros

A área de membros do site (`site/src/members/`, `site/worker/`) não traz arquivos de imagem nem de fonte novos. Traz:

| Item | Uso | Licença |
|---|---|---|
| Marca do GitHub (Octicons `mark-github`), desenhada como caminho SVG em `site/src/lib/icons.site.ts` | Ícone do botão "Entrar com o GitHub" | MIT (GitHub Inc.) |
| `marked` (`site/package.json`) | Só como analisador do Markdown dos documentos públicos; o HTML não é inserido na página | MIT |
| `wrangler` (só em `site/worker/package.json`, desenvolvimento do Worker de login) | Publicar o Worker, feito pelos mantenedores | MIT ou Apache-2.0 |
| Dependências de `wrangler` (`site/worker/package-lock.json`: `workerd`, `esbuild`, `miniflare`, `sharp` opcional com `libvips` LGPL-3.0-or-later, pacotes 0BSD e BSD) | Só a ferramenta de publicação, instalada com `npm ci` em `site/worker/`; nada disso entra no site nem no código do Worker publicado | As de cada pacote (MIT, Apache-2.0, BSD, ISC, 0BSD, CC0-1.0; LGPL-3.0-or-later nos binários `libvips` opcionais do `sharp`) |

## Dependências em Rust

As crates mantêm as próprias licenças, todas permissivas ou de copyleft fraco: MIT, Apache-2.0, BSD-3-Clause,
ISC, Zlib, Unicode-3.0, MPL-2.0 (Symphonia, decodificação de áudio) e CDLA-Permissive-2.0 (`webpki-roots`). O
`deny.toml` mantém essa lista e a CI confere a cada mudança no `Cargo.lock`.
