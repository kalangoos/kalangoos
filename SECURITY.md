# Segurança

O KalangoOS dá a agentes de IA acesso ao sistema, então falhas de segurança são levadas a sério. O modelo de
ameaças e as camadas de defesa estão em [docs/SEGURANCA.md](docs/SEGURANCA.md).

## Como relatar uma falha

**Não abra issue, pull request nem discussão pública sobre uma falha.** Use um destes caminhos:

1. **Relato privado no GitHub** (preferido): aba **Security** deste repositório → **Report a vulnerability**.
   <!-- TODO (mantenedores): ativar "Private vulnerability reporting" em Settings → Code security. -->
2. **E-mail:** **TODO: definir o endereço de segurança (por exemplo, seguranca@ em kalangoos.com)** e, se quiser,
   uma chave PGP publicada em `kalangoos.com/seguranca` (página ainda não publicada).

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

O projeto ainda não tem versões publicadas. Até a primeira, só o ramo `main` recebe correções.

| Versão | Recebe correções |
|---|---|
| `main` | Sim |

---

**English.** Please do not report security issues in public. Use GitHub's private vulnerability reporting
(Security tab → Report a vulnerability) or the security e-mail above (to be defined). We acknowledge reports within
5 business days and coordinate disclosure with you.
