## O que muda

<!-- Uma ou duas frases. Se fecha uma issue: "Fecha #123". -->

## Por quê

## Como testei

<!-- Comandos e resultado. Diga também o que você não conseguiu rodar. -->

- [ ] `cargo clippy --workspace --all-targets --locked`
- [ ] `cargo test --workspace --locked --exclude kalango-agente`
- [ ] `cargo test -p kalango-agente` (se mexeu no motor: usa o modelo local de verdade)
- [ ] `sessao/testar.sh` (se mexeu em telas, no shell ou na sessão)

## Telas

<!-- Se a interface mudou: capturas antes/depois e o quadro do design com que foram comparadas, em mais de um
tamanho de tela e nos dois temas. Apague esta seção se não se aplica. -->

## Conferido

- [ ] Sem dados pessoais nem segredos (código, testes, capturas, logs)
- [ ] Textos novos da interface com `tr("…")` e tradução no `pt_BR.json`
- [ ] Imagem, ícone, fonte ou dado de terceiros novo registrado no `ATTRIBUTION.md`
- [ ] Workflows da CI não mudaram (ou este pull request é só sobre eles)
