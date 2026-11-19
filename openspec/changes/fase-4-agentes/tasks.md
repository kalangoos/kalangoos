# Tasks

## 1. kalangod

- [x] 1.1 Desenho do serviço, ferramentas por domínio e níveis de poder em docs/ARQUITETURA.md
- [x] 1.2 Serviço root ouvindo só em /run/kalango/mcp.sock; teste confirma que não abre porta de rede
- [x] 1.3 Identificação do chamador por SO_PEERCRED e cgroup; teste com chamador falso é recusado (teste `chamador_sem_regra_e_recusado`; na VM, tarefa no cgroup dos agentes é reconhecida como `agentes-nativos`)
- [x] 1.4 Ferramentas de leitura (nível 0): sistema, serviços, discos, rede, processos, journal
- [x] 1.5 Ferramentas do nível 2 com ponto de restauração antes: pacotes, serviços, rede, configurações (sem ponto, a ação não acontece; verificado na VM)
- [x] 1.6 Nível 3 (comando livre como root) com aprovação presencial e prazo de 15 min; teste de expiração (`concessao_total_expira`)
- [ ] 1.7 Cada ferramenta declara efeito, reversibilidade e inverso; teste de contrato para todas
- [x] 1.8 Protocolo MCP 2025-06-18 (initialize, tools/list, tools/call) por soquete e ponte stdio `kalangod mcp`; testes de protocolo passam
- [x] 1.9 Ferramentas do nível 2 já feitas: pacotes (apt), serviços (systemd), sysctl, sinais; com aprovação e ponto de restauração; teste em simulação passa
- [x] 1.10 Concessão do nível total por aprovação de outro processo; segunda chamada dentro do prazo não pede de novo (teste passa)
- [x] 1.11 Processo não aprova o próprio pedido; negação registrada (teste passa)
- [x] 1.12 Rodando como root: ação de arquivo executa com o UID de quem chamou (verificado: arquivo criado pertence ao usuário)
- [x] 1.13 Unidade systemd e configuração padrão em crates/kalangod/dist; o serviço só ouve no soquete Unix (a rede não é cortada na unidade porque apt, nmcli e ufw, executados por ele, precisam dela)
- [x] 1.14 Ferramentas de rede (NetworkManager, ufw), discos (lsblk, SMART, udisks), usuários e atualização do sistema, com inversos; validadas como root na VM (`vm/run.sh`) e no host (`vm/host.sh`)

## 2. Regras do kernel e isolamento

- [x] 2.1 Verificado: Landlock ativo, eBPF com BTF e eBPF LSM compilado no kernel do Ubuntu
- [ ] 2.2 Imagem ativa `bpf` na lista lsm=; verificado em /sys/kernel/security/lsm
- [x] 2.3 Programa eBPF LSM (C com CO-RE, carregado por Aya) para pastas protegidas: nem o nível 3 lê; na VM com `lsm=…,bpf`, `ls ~/.ssh` e `head /etc/shadow` de agentes (inclusive root) dão EPERM
- [x] 2.4 Auditoria por eBPF alimenta a linha do tempo independentemente do agente: bloqueios, execuções e escritas (verificado na VM)
- [x] 2.5 Cada tarefa em cgroup com limite de CPU, memória, processos e tempo; testes de estouro de memória, tempo e processos (sem root, na VM e no host com systemd)
- [x] 2.6 Landlock por tarefa e Landlock + seccomp no auxiliar de arquivos; teste de escrita fora da pasta permitida e de exec/rede bloqueados
- [x] 2.6b Landlock (arquivos e portas TCP) + seccomp + cgroup + ambiente limpo por conector MCP de terceiros; teste com conector hostil (`tests/conectores.rs`)
- [x] 2.7 Landlock no processo auxiliar das ações de arquivo; pastas protegidas recusadas inclusive por link simbólico e '..' (testes passam)

## 3. Motor e permissões

- [ ] 3.1 Fila de tarefas com pausa, retomada e retomada após reinício
- [x] 3.2 Permissões livre/perguntar/nunca por ferramenta (`kalango-agente/src/permissoes.rs`), lidas a cada pedido (mudança vale na hora); padrão: ler livre, mudar arquivos pergunta na conversa, sistema/total pela tela de aprovação; cartão "Permissão necessária" do design (Negar / Permitir uma vez / Sempre) e lista no botão de ajustes do compositor; testes com o LFM3 real (negar, uma vez, nunca, sempre)
- [x] 3.3 Aprovação por superfície do sistema (layer-shell, camada overlay, teclado exclusivo) com resumo, comandos e escopo; teste de que o agente não a imita (executável confiável conferido pelo kernel) e validação de ponta a ponta (`crates/kalango-aprovacao/validar.sh`)
- [ ] 3.3b No compositor do KalangoOS: teclado/mouse virtuais negados a agentes (senão um agente tecla a aprovação)
- [x] 3.4a Modelo local como padrão: motor `kalango-agente` com LFM3-1.7B (llama.cpp CUDA, soquete Unix, 256K de contexto); age só pelas ferramentas do kalangod, oferecidas por domínio do pedido; testes de integração com o modelo real (`tests/motor.rs`)
- [ ] 3.4 Nuvem com orçamento e aviso a 80% (o local já é o padrão)
- [x] 3.5a Defesa contra instruções em conteúdo externo (`kalango-agente/src/defesa.rs`): envelope de dados sem tokens de controle, ordem escondida para a tarefa sem o modelo ver, valor vindo só do conteúdo não aciona ação de nível sistema/total; testes de unidade e com o LFM3 real (arquivo com ordem escondida; arquivo recomendando pacote: o modelo tenta instalar e é barrado, 5 de 5)
- [x] 3.5 Leitura de páginas (`web.ler` no kalangod: sem endereços locais/privados, limites, comentários à parte) e suíte com páginas maliciosas com o LFM3 real (`kalango-agente/tests/web.rs`): página normal lida; ordem em comentário HTML e texto invisível param a tarefa; página que manda abrir outro endereço é barrada; globo desligado não lê páginas

## 4. Linha do tempo e memória

- [ ] 4.1 Registro de cada ação: o que mudou, por quê, permissões, inverso
- [ ] 4.2 Desfazer uma ação e desfazer tudo desde um momento; teste com 118 arquivos movidos
- [ ] 4.3 Memória local criptografada, listável, editável e apagável; modo anônimo
- [ ] 4.4 Rotinas por horário e por gatilho (USB, rede, e-mail)
- [x] 4.5 Linha do tempo do kalangod com inverso por ação e desfazer de uma ação; teste de mover e desfazer passa

## 5. Conectores MCP

- [x] 5.1 Hospedeiro de conectores locais (comando): `kalangod conector rodar`, com filtro de ferramentas bloqueadas e registro
- [x] 5.1b Conectores remotos: MCP por HTTP (JSON e SSE, sessão) com OAuth 2.1 (metadados, registro dinâmico, PKCE S256, resource, renovação); teste contra servidor OAuth local (`tests/remoto.rs`)
- [x] 5.2 Teste isolado antes de adicionar e permissão sugerida pelo tipo da ferramenta (`kalangod conector testar`)
- [x] 5.3 Cofre de segredos fora dos arquivos de configuração: chaveiro do sistema (Secret Service) ou arquivo 0600 protegido pela guarda eBPF; manifestos usam `cofre:NOME` (validado no chaveiro real e na VM)
- [ ] 5.4 Diagnóstico de falha em linguagem simples com o registro bruto

## 6. App Agentes e chat

- [x] 6.1 Design completo do chat (34 blocos e 9 telas) e do app Agentes
- [x] 6.2a Chat compacto na barra lateral (modo Agente): mensagem, passos com estado e tempo, resposta em markdown (texto, títulos, listas, código, tabelas), cartões de tarefa parada e erro, compositor com anexar, globo (web), chip do modelo, enviar/parar, /nova e /parar; validado com o LFM3 real na interface e em `sessao/testar.sh`
- [ ] 6.2 Chat completo (app Agentes): diff, terminal e arquivos
- [ ] 6.3 Mermaid, LaTeX, gráficos, áudio, vídeo e imagens
- [ ] 6.4 Prévia de HTML isolada e navegador do agente com 'assumir o controle'
- [x] 6.5a Ditado no compositor com transcrição local: o microfone grava (PipeWire, arquivo só até transcrever, numa pasta 0700), o LFM3-Audio-2B (llama.cpp, `kalango-agente transcrever`) escreve o texto na caixa; validado em `sessao/testar.sh` com fala gerada pelo próprio LFM3-Audio
- [ ] 6.5 Modo voz completo: conversa falada (resposta em voz)
- [ ] 6.6 Privacidade visível: indicadores de microfone, câmera, tela e leitura; 'pausar tudo' (Super+Shift+P)
