# Segurança

O KalangoOS dá a agentes de IA acesso ao sistema, então falhas de segurança são levadas a sério. O modelo de
ameaças e as camadas de defesa estão em [docs/SEGURANCA.md](docs/SEGURANCA.md).

## Como relatar uma falha

**Não abra issue, pull request nem discussão pública sobre uma falha.** Use o caminho abaixo:

1. **Relato privado no GitHub** (preferido): aba **Security** deste repositório → **Report a vulnerability**.
2. Sem acesso ao GitHub? Por enquanto não há outro canal privado; um endereço de e-mail de segurança será publicado
   aqui quando existir.

Inclua, se puder: a versão ou o commit, o componente (`kalangod`, `kalango-agente`, shell, instalador, patches do
niri…), os passos para reproduzir e o impacto que você enxerga. Não teste em computadores ou contas que não são seus.

## O que acontece depois

- Resposta em até **5 dias úteis** confirmando o recebimento.
- Avaliação, correção e uma data combinada para a divulgação. Correções de segurança saem fora do ciclo normal de
  versões.
- Crédito no aviso de segurança, se você quiser.

## O que conta como falha

Interessam em especial:

- Qualquer forma de um agente, um conector MCP ou um conteúdo lido (página, arquivo, resposta de ferramenta) fazer
  o `kalangod` executar algo acima do nível de poder concedido, sem a aprovação exigida, ou fora do registro na
  Linha do tempo.
- Instruções escondidas em conteúdo que viram comando (injeção de prompt) e burlas do controle de contaminação das
  chamadas web.
- Escapes do isolamento (Landlock, seccomp, cgroups, namespaces, o proxy do navegador dos agentes).
- Falsificação da tela de aprovação, do bloqueio de tela ou da tela de entrada.
- Vazamento de dados locais para a rede ou para modelos na nuvem sem permissão.

## Versões cobertas

Só a versão mais recente (0.1.x, prévia técnica) e o ramo `main` recebem correções.
