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

Numeração e canais: [docs/LANCAMENTO.md](docs/LANCAMENTO.md).

## Decisões em aberto

- **Licenças** do código, da documentação e da marca ([docs/LICENCAS.md](docs/LICENCAS.md)).
- **Registro da marca** KalangoOS.
- **Atualização e restauração** (btrfs com snapshots é a recomendação provisória).

---

**English summary.** Next steps: real hardware (the NVIDIA, AMD and Intel matrix), the
rest of the screens, then AG-UI 1.0 between the agent and the interface. Version 0.1.0 is a technical preview: it
has an installable ISO that the release workflow builds and tests in a VM (live session, installer, installed system
boots), the shell on a patched niri, `kalangod` and local agents. Real-hardware validation is still pending; see
[CHANGELOG.md](CHANGELOG.md).
