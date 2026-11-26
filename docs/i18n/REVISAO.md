# Revisão por quem fala o idioma

**Os sete catálogos novos (`es`, `fr`, `it`, `de`, `ja`, `ru`, `zh_CN`) foram gerados por IA, a partir do inglês e do
[glossário](GLOSSARIO.md), e nenhuma pessoa nativa os revisou.** Só o português do Brasil foi escrito e conferido por
quem fala o idioma. Os testes do `kalango-i18n` conferem a forma (chaves, `{marcadores}`, quebras de linha, espaços
nas pontas, ordem, atalhos, variáveis e caminhos, caracteres ocultos ou bidirecionais, as palavras de consentimento
numa tabela fixa, textos deixados em inglês). Não conferem o sentido nem o tom: pode haver erros. Quem usa um desses
idiomas vê esse aviso em Configurações › Idioma. Esta lista diz onde uma revisão de quem fala o idioma rende mais.

## Prioridade

| Idioma | Prioridade | Por quê |
|---|---|---|
| 日本語 (`ja`) | alta | Registro (です/ます nas frases, formas curtas em botões) e ordem das palavras em textos montados por pedaços (`{x}` no meio da frase); glifos e quebras de linha na tela |
| 简体中文 (`zh_CN`) | alta | Termos de IA ainda variam na região (智能体 × 代理 para *agent*; o site usa 特工, que quer dizer "agente secreto"); pontuação de largura total |
| Русский (`ru`) | alta | O código não escolhe forma de plural: textos com `{n}` foram escritos para servir a qualquer número (*Файлов: {n}*), o que pode soar rígido; gênero em frases curtas sem substantivo |
| Deutsch (`de`) | média | Palavras compostas longas em botões e cartões estreitos; *Sie* em todo lugar |
| Français (`fr`) | média | Espaço inseparável antes de `: ; ? !`; *vous*; gênero em rótulos soltos ("Arrêtée") |
| Italiano (`it`) | média | Rótulos soltos sem o substantivo a que se referem (gênero e número) |
| Español (`es`) | baixa | *Tú* e "ordenador" (Espanha); pessoas da América Latina podem preferir "computadora" |

## O que olhar primeiro

- Textos que são pedaços de frase montados pelo código (`"at"`, `"is empty"`, `"last"`, `"next"`, `"Through"`,
  `"RUN OF"`): a ordem das palavras do inglês nem sempre serve em `ja`, `de` e `ru`.
- Rótulos de uma palavra sem contexto (`"Held"`, `"Marked"`, `"Bar"`, `"Agenda"`, `"island"`, `"Captures"`).
- Instruções para o modelo de IA (no `kalango-agente` e no `kalangod`): estão traduzidas para aparecer na tela, mas
  o modelo recebe o texto em inglês nos idiomas novos (`kalango_i18n::tr_model`, `for_model`).
- Datas no formato `strftime` traduzidas por idioma (`%-d. %B`, `%-m月%-d日`).

## Como corrigir

Mude o valor no catálogo do idioma (a chave é o inglês) e rode `cargo test -p kalango-i18n`. Termo que muda em
vários lugares: mude primeiro o glossário.
