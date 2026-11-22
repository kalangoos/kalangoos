# Roteiro do KalangoOS

Resumo público do plano. O detalhe, com cada tarefa marcada, está no OpenSpec (`openspec/changes/fase-*/tasks.md`)
e em [docs/PLANO.md](docs/PLANO.md). Datas não são prometidas: o projeto anda em passos pequenos, e um passo só
termina quando liga do zero numa máquina virtual, tem teste automático e as capturas batem com o design.

**Estado em 22 de novembro de 2026:** a versão **0.1.0** é uma prévia técnica com ISO instalável, testada numa VM
pela esteira de lançamento (veja [CHANGELOG.md](CHANGELOG.md) e [docs/NOTAS-0.1.0.md](docs/NOTAS-0.1.0.md)). Os passos
1 a 4 abaixo estão feitos na imagem; faltam hardware real, o resto das telas e o AG-UI.

## Próximos passos, em ordem

| # | Passo | Estado |
|---|---|---|
| 0 | Fechar o lote atual: suíte da sessão inteira e versão local | Contínuo |
| 1 | **Imagem de disco de verdade**: a ISO (`imagem/construir.sh`) com o kernel do KalangoOS, o niri e o shell, a mesma que o instalador grava | Feito na 0.1.0 (ISO testada numa VM) |
| 2 | **Instalador** gravando de verdade num disco virtual, com a etapa "Modelos de IA" | Motor testado na VM da esteira; as telas (inclusive "Modelos de IA") não são exercitadas pelo teste; falta hardware real |
| 3 | **Login** no sistema instalado (greetd + `kalango-entrada`) e o primeiro uso depois do primeiro login | Feito na 0.1.0 (o sistema instalado liga na sessão) |
| 4 | **Área de trabalho mínima**: barra superior, dock, Arquivos e Configurações, com um kit único de componentes GTK | Feito e já além do mínimo; as telas restantes seguem em [docs/telas.md](docs/telas.md) |
| 5 | **AG-UI 1.0** como protocolo de eventos entre o agente e a interface: texto chegando enquanto é escrito, aprovações do `kalangod` como interrupções, progresso, raciocínio e subagentes | Planejado |
| 6 | **Hardware real**: matriz NVIDIA, AMD e Intel, a ISO com o modelo local quando as licenças forem revistas | Próximo |
| 7 | **Kalango Audio nos fluxos e nos agentes**: ferramentas `audio.*`, blocos "Áudio · capturar" e gatilho ao vivo por transcrição (sem passar por um arquivo) | Ideia; detalhes abaixo |
| 8 | **Suporte remoto com o Kalango Remote**: máquinas salvas, consultar (só leitura) e agir (com aprovação) por fluxos e agentes | Ideia; detalhes abaixo |
| 9 | **Gatilho de entrada pela internet** (como o webhook do n8n): uma URL pública gerada, ou um domínio próprio apontado para um túnel da Cloudflare, que dispara um fluxo | Ideia; detalhes abaixo |

## Fases

| Fase | O que entrega | Estado |
|---|---|---|
| 0 · Fundações | Workspace Cargo, exportador de tokens, CI, ambiente de desenvolvimento reproduzível | Feita: workspace, sessão de desenvolvimento e CI (testes, dependências, regras e a esteira de lançamento) |
| 1 · Shell sobre Wayland | Sessão própria, barra superior com a ilha, dock, barras laterais, notificações, bloqueio, busca, atalhos | Grande parte feita, na sessão e na ISO |
| 2 · Janelas e espaços | Encaixe, Alt+Tab, visão geral, espaços por intenção, vários monitores, captura de tela | Grande parte feita (patches do niri em `sessao/niri/`) |
| 3 · Tema, ícones e apps base | Arquivos, Configurações, Terminal, Editor, Calculadora e os outros apps do sistema; diálogos por portais | Em andamento; vários apps na 0.1.0 |
| 4 · Agentes | `kalangod`, permissões, linha do tempo com desfazer, isolamento, conectores MCP, modelos locais, chat completo | Em andamento; o núcleo (aprovação, desfazer, níveis, agentes locais, contexto) está na 0.1.0 |
| 5 · Sistema | Instalador, primeiro uso, atualizações com restauração, Central de Apps, imagem ISO | Instalador, primeiro uso e ISO feitos; faltam atualizações com restauração e hardware real |
| 6 · Qualidade e lançamento | Acessibilidade, inglês, metas de desempenho, revisão de segurança, site, licenças | Planejado |

## Versões

| Versão | Conteúdo | Para quem |
|---|---|---|
| **0.1 · prévia técnica** (0.1.0, 22/11/2026) | Fases 0 a 2 + núcleo das fases 4 e 5: ISO instalável testada em VM, shell e janelas próprios, `kalangod` e agentes locais, Remote, Audio | Desenvolvedores e testadores |
| 0.2 · prévia | Completa a fase 3 | Entusiastas |
| 0.3 · beta | Completa a fase 4 | Testadores com backup |
| 0.4 · beta | Completa a fase 5 | Público em geral, com aviso |
| 1.0 | + fase 6 | Público em geral |

## Integrações planejadas (ideias, sem data)

Hoje o Kalango Audio e o Kalango Remote são vistos pela plataforma de agentes só pela ponta de fora: o bloco
"Transcrever áudio" lê um arquivo, e o agente consegue pedir uma conexão ao Remote (com o painel de aprovação), mas não
faz nada depois que ela abre. A ideia é integrá-los de forma nativa, na ordem do risco:

1. **Kalango Audio.** Ferramentas no `kalangod` (`audio.sessoes`, `audio.ligar`, `audio.desligar`, `audio.gravar`,
   `audio.transcrever`), blocos de fluxo como "Áudio · capturar" com a fonte escolhida na lista do Audio (navegador,
   microfone, som do sistema) e um gatilho ao vivo: o Audio transcreve em janelas curtas e dispara o fluxo quando o texto
   bate com uma condição. Capturar som é sensível: o `kalangod` pergunta, o indicador do Audio fica sempre visível, a
   transcrição é local e o que ela traz é tratado como conteúdo externo.
2. **Kalango Remote, só consultar.** `remoto.maquinas` (as máquinas salvas, sem nenhum segredo) e `remoto.consultar`
   (comandos de leitura, como `df`, `systemctl status` e `journalctl`), com um perfil por máquina e uma lista de comandos
   permitidos. A saída da máquina remota passa pela defesa de conteúdo externo.
3. **Kalango Remote, agir.** `remoto.executar`, sempre com aprovação, nas duas pontas quando é uma sessão de tela, e
   com tudo na Linha do tempo das duas máquinas (com desfazer onde der). O agente continua sem ler o cofre de chaves e
   sem aceitar a chave de um servidor sozinho. Um fluxo "Suporte remoto" fecha o ciclo: gatilho, decisão do d1, conexão,
   consultas, resumo e proposta de correção, aprovação, aplicação, verificação e relatório.

Cada passo passa pelas mesmas verificações dos demais: testes com o modelo real, revisão de segurança às cegas e uma
captura contra o desenho no Design System antes de entrar.

### Gatilho de entrada pela internet

Hoje o gatilho "Webhook local" só recebe chamadas deste computador. A ideia é um gatilho de entrada pela internet, como o
webhook do n8n: algo de fora (um serviço, um formulário, outro computador) chama uma URL e um fluxo roda. O
`cloudflared` que o Kalango Remote já usa para compartilhar a tela dá o caminho:

- **URL gerada, sem conta.** Um túnel rápido da Cloudflare dá um endereço aleatório e temporário (`*.trycloudflare.com`),
  bom para testar e para uso eventual. O endereço muda a cada vez que o túnel abre.
- **Domínio próprio.** Para um endereço fixo, a pessoa cria um túnel nomeado na Cloudflare e aponta um domínio dela para
  ele. O token do túnel vai para o cofre do sistema; o KalangoOS roda o `cloudflared` como serviço da pessoa e liga o
  túnel ao receptor local de gatilhos. Um túnel serve vários gatilhos, cada um com o seu caminho.
- **Um receptor mínimo, só dele.** Um servidor HTTP pequeno, preso a uma porta local, que só aceita os caminhos dos gatilhos
  ligados, com limite de tamanho do corpo e de pedidos por minuto. Responde na hora com um número de execução; o fluxo roda
  em seguida (e, se o gatilho pedir, devolve o resultado).
- **Segurança, porque isso abre uma porta para o mundo.** Abrir um túnel é uma decisão da pessoa e pede aprovação, com um
  indicador sempre visível enquanto ele estiver aberto e um botão de fechar. Cada gatilho tem um segredo próprio e
  assinatura do pedido (HMAC, com data para barrar repetição), caminho impossível de adivinhar e liga/desliga individual;
  como opção, a lista de quem pode chamar (Cloudflare Access). O corpo que chega é conteúdo externo: passa pela defesa de
  contaminação e nunca vira argumento de uma ferramenta que age no sistema sem a aprovação de sempre. Tudo vai para a Linha
  do tempo, e um interruptor geral derruba todos os túneis.
- **Privacidade, dita com clareza.** O tráfego passa pela Cloudflare. Quem não quiser isso fica no webhook local; outros
  túneis (Tailscale Funnel, por exemplo) entram depois como adaptadores.

Passa pelas mesmas verificações dos demais: testes com um servidor de verdade e o modelo real, revisão de segurança às cegas
e captura contra o desenho antes de entrar.

Numeração e canais: [docs/LANCAMENTO.md](docs/LANCAMENTO.md).

## Decisões em aberto

- **Licenças** do código, da documentação e da marca ([docs/LICENCAS.md](docs/LICENCAS.md)).
- **Registro da marca** KalangoOS.
- **Atualização e restauração** (btrfs com snapshots é a recomendação provisória).

---

**English summary.** Next steps: real hardware (the NVIDIA, AMD and Intel matrix), the
rest of the screens, then AG-UI 1.0 between the agent and the interface. Planned ideas, with no date: Kalango Audio
and Kalango Remote as native parts of flows and agents (a live audio source and trigger; a remote-support flow that first
only reads from the machines you can reach, then acts with approval), and an internet-facing trigger like n8n's
webhook, through a Cloudflare tunnel (a generated URL or your own domain). Version 0.1.0 is a technical preview: it
has an installable ISO that the release workflow builds and tests in a VM (live session, installer, installed system
boots), the shell on a patched niri, `kalangod` and local agents. Real-hardware validation is still pending; see
[CHANGELOG.md](CHANGELOG.md).
