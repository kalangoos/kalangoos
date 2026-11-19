# Roteiro do KalangoOS

Resumo público do plano. O detalhe, com cada tarefa marcada, está no OpenSpec (`openspec/changes/fase-*/tasks.md`)
e em [docs/PLANO.md](docs/PLANO.md). Datas não são prometidas: o projeto anda em passos pequenos, e um passo só
termina quando liga do zero numa máquina virtual, tem teste automático e as capturas batem com o design.

## Próximos passos, em ordem

| # | Passo | Estado |
|---|---|---|
| 0 | Fechar o lote atual: suíte da sessão inteira e versão local | Contínuo |
| 1 | **Imagem de disco de verdade** (qcow2, ext4) feita por script, com o kernel do KalangoOS, o niri e o shell — a mesma que o instalador vai gravar | Próximo |
| 2 | **Instalador** gravando de verdade num disco virtual (as telas e o motor já existem e passam na sessão com o motor simulado) | Em andamento |
| 3 | **Login** no sistema instalado (greetd + `kalango-entrada`) e o primeiro uso depois do primeiro login | Telas prontas |
| 4 | **Área de trabalho mínima**: barra superior, dock, Arquivos e Configurações, com um kit único de componentes GTK | Em andamento |
| 5 | **AG-UI 1.0** como protocolo de eventos entre o agente e a interface: texto chegando enquanto é escrito, aprovações do `kalangod` como interrupções, progresso, raciocínio e subagentes | Planejado |

## Fases

| Fase | O que entrega | Estado |
|---|---|---|
| 0 · Fundações | Workspace Cargo, exportador de tokens, CI, ambiente de desenvolvimento reproduzível | Parcial: workspace e sessão de desenvolvimento prontos; CI chegando com a publicação |
| 1 · Shell sobre Wayland | Sessão própria, barra superior com a ilha, dock, barras laterais, notificações, bloqueio, busca, atalhos | Grande parte feita na sessão de desenvolvimento |
| 2 · Janelas e espaços | Encaixe, Alt+Tab, visão geral, espaços por intenção, vários monitores, captura de tela | Grande parte feita (patches do niri em `sessao/niri/`) |
| 3 · Tema, ícones e apps base | Arquivos, Configurações, Terminal, Editor, Calculadora e os outros apps do sistema; diálogos por portais | Em andamento |
| 4 · Agentes | `kalangod`, permissões, linha do tempo com desfazer, isolamento, conectores MCP, modelos locais, chat completo | Em andamento |
| 5 · Sistema | Instalador, primeiro uso, atualizações com restauração, Central de Apps, imagem ISO | Instalador e primeiro uso feitos; imagem pendente |
| 6 · Qualidade e lançamento | Acessibilidade, inglês, metas de desempenho, revisão de segurança, site, licenças | Planejado |

## Versões

| Versão | Conteúdo | Para quem |
|---|---|---|
| 0.1 · prévia técnica | Fases 0 a 2: shell e janelas próprios sobre o Ubuntu | Desenvolvedores e testadores |
| 0.2 · prévia | + fase 3 | Entusiastas |
| 0.3 · beta | + fase 4 | Testadores com backup |
| 0.4 · beta | + fase 5 | Público em geral, com aviso |
| 1.0 | + fase 6 | Público em geral |

Numeração e canais: [docs/LANCAMENTO.md](docs/LANCAMENTO.md).

## Decisões em aberto

- **Licenças** do código, da documentação e da marca ([docs/LICENCAS.md](docs/LICENCAS.md)).
- **Registro da marca** KalangoOS.
- **Atualização e restauração** (btrfs com snapshots é a recomendação provisória).

---

**English summary.** Next steps: a real disk image built by script, the installer writing to a virtual disk, login on
the installed system, a minimal desktop, then AG-UI 1.0 between the agent and the interface. Version 0.1 (a
technical preview) covers phases 0 to 2. There is no installable image yet.
