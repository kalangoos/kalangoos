# kalangod · o MCP do sistema KalangoOS

Serviço em Rust que deixa os agentes nativos operarem o sistema inteiro pelo protocolo MCP, com níveis de poder,
aprovação, ponto de restauração, linha do tempo com desfazer e isolamento pelo kernel. Especificação:
`openspec/changes/fase-4-agentes/specs/kalangod/spec.md`. Desenho: `docs/ARQUITETURA.md`, seção 3.

## Como funciona

- Ouve só num soquete local (`/run/kalango/mcp.sock`), JSON-RPC 2.0 com uma mensagem por linha (MCP 2025-06-18).
- Quem chama é identificado pelo kernel (`SO_PEERCRED`, `/proc/<pid>/cgroup`), nunca pelo que o chamador diz.
- Cada ferramenta declara nível, efeito e se é reversível; `tools/list` só mostra o que o chamador pode usar.
- **Observar** roda direto. **Usuário** roda com o UID de quem chamou: arquivos num processo auxiliar preso pelo
  Landlock e por seccomp; programas (`tarefas.executar`) numa tarefa limitada. **Sistema** pede aprovação e cria um
  ponto de restauração antes; sem ponto, nada é feito. **Total** pede aprovação presencial e vale por 15 minutos.
- Toda chamada entra na linha do tempo (`/var/lib/kalango/linha-do-tempo.jsonl`, pasta 0700) com o inverso, quando
  existe; `linha_do_tempo.desfazer` executa o inverso.

### Guarda no kernel (eBPF LSM)

`ebpf/guarda.bpf.c` (C com CO-RE, compilado pelo `build.rs` e carregado pelo crate `aya`) prende dois ganchos LSM:

- `file_open`: um processo descendente de `/kalango.slice/kalango-agentes.slice` não abre nada dentro de uma pasta
  protegida (`~/.ssh`, `~/.gnupg`, chaveiros, `/etc/shadow`…), nem como root, nem no nível total. A busca sobe pelos
  diretórios pai e compara (dispositivo do superbloco, inode). Essas chaves são descobertas abrindo cada pasta com uma
  sonda ligada, porque em btrfs e overlay o `st_dev` do `stat()` não é o do superbloco.
- `bprm_check_security` e escritas em `file_open`: auditoria. Cada programa executado e cada arquivo aberto para
  escrita por um agente vira um registro da linha do tempo, independente do que o agente relata.

Precisa de `bpf` na lista `lsm=` do kernel. O kernel do KalangoOS (`kernel/kalango.config`) já vem assim. Sem ele, o
kalangod continua com Landlock e as próprias verificações, e `kernel.seguranca` diz que a guarda está desligada e por quê.

### Tarefas limitadas

Cada programa que o kalangod executa para um agente (`tarefas.executar`, `sistema.executar`) roda num cgroup próprio
dentro de `kalango-agentes.slice`, com limites de memória (sem swap), CPU, processos e tempo. O agente pode pedir
limites menores, nunca maiores que os de `[limites]` na configuração. O trampolim `kalangod __tarefa` entra no
cgroup, larga o root, liga `no_new_privs` e aplica o Landlock antes do `exec`: a escrita fica restrita às pastas
permitidas e a `/tmp`. Com systemd, a tarefa é um escopo (`systemd-run --scope`). Sem systemd, como na VM de teste,
o kalangod cria o cgroup direto. Sem root, usa o systemd do usuário. A resposta traz o código, a saída, o pico de
memória e se a tarefa passou do limite de memória ou de tempo.

### Pontos de restauração

Antes de toda ação do nível sistema, um ponto guarda o `/etc` e a lista exata de pacotes (nome:arquitetura=versão):

- raiz em btrfs: um snapshot somente leitura (via snapper, quando instalado);
- em ext4 e outros: uma cópia compactada do `/etc`, com donos, permissões e atributos estendidos.

`pontos.restaurar` devolve o `/etc` ao estado do ponto, apagando também o que foi criado depois, e reinstala ou
remove pacotes até a lista bater. Antes de restaurar, ele cria um ponto novo, então a restauração também se desfaz.
Com `so_plano`, só mostra o que mudaria. Ficam os `pontos_max` pontos mais recentes.

### Tela de aprovação

`crates/kalango-aprovacao` é a tela que a pessoa vê quando um agente pede uma ação de nível sistema ou total.
É uma superfície layer-shell na camada mais alta, com o teclado só para ela. Mostra quem pede, a ferramenta, o que ela
faz, o efeito, se pode ser desfeita e os argumentos exatos.

- Recebe os pedidos por espera longa (`kalango/aguardar`).
- Só ela e um terminal aprovam sem `kalangod aprovar`. O kalangod confere o caminho do executável que o kernel
  informa, e esse caminho precisa estar em `aprovadores_graficos`. Um agente não consegue se passar pela tela, nem
  trocá-la: o arquivo e a pasta dela precisam ser do root, sem escrita para outros.
- A tela não pode ser inspecionada por outro processo (`PR_SET_DUMPABLE=0`).
- Os botões só respondem 0,7 s depois de o pedido aparecer.
- Enter sozinho não aprova: Ctrl+Enter aprova e Esc nega.

### Conectores MCP de terceiros

`kalangod conector testar|rodar MANIFESTO` sobe um servidor MCP local de terceiros (node, python, binário) isolado,
como o usuário, num cgroup de `kalango-agentes.slice`:

- **Landlock**: lê o sistema, a pasta de instalação e o que o manifesto declarar; escreve só na própria pasta de
  dados (que vira o `HOME` e o `TMPDIR` dele) e no que declarar. Um manifesto que pede `~`, `/home`, `/` ou uma pasta
  protegida é recusado, porque o Landlock não abre exceção dentro de uma permissão.
- **Rede**: nenhuma (seccomp nega sockets IPv4/IPv6/pacote e Landlock nega TCP), só as portas declaradas
  (Landlock ABI 4), ou livre.
- **seccomp**: sem ptrace, montagem, módulos, eBPF, namespaces, chaves do kernel, io_uring e userfaultfd.
- **Ambiente limpo**: tokens e variáveis de quem chamou não passam; só `PATH`, `LANG` e o que o manifesto declarar.
- **Limites** de memória, CPU e processos. Se o conector passar do limite, só ele morre, e a ponte termina com o
  código dele.

Na ponte (`rodar`), as ferramentas bloqueadas pelo manifesto somem de `tools/list` e são recusadas sem chegar ao
conector. Cada chamada, recusa e resultado vai para `~/.local/state/kalango/conectores/NOME.jsonl`, fora do alcance
do conector. O `testar` faz o aperto de mão MCP, lista as ferramentas e sugere a permissão de cada uma pelo tipo:
só leitura é livre; destrutiva, ou sem declaração, pergunta sempre.

```toml
# exemplo de manifesto
nome = "github"
comando = "/usr/bin/node"
argumentos = ["dist/index.js"]
pasta = "~/.local/lib/kalango/conectores/github"
rede = true
portas = [443]
ambiente = { GITHUB_TOKEN = "cofre:github-token" }   # o valor vem do cofre, só na hora de subir o conector
ferramentas_bloqueadas = ["delete_repository"]
[limites]
memoria_mb = 512
```

### Conectores remotos (MCP por HTTP + OAuth)

Um manifesto com `url` em vez de `comando` descreve um servidor MCP remoto. Os comandos:

- `kalangod conector entrar MANIFESTO` autoriza o conector. Ele descobre o servidor de autorização pelos metadados
  (RFC 9728 e RFC 8414) e registra o KalangoOS como cliente quando o servidor permite (RFC 7591). Depois abre o
  navegador e recebe o código numa porta local de 127.0.0.1, com PKCE S256, `state` e `resource` (RFC 8707). Os tokens
  vão para o cofre.
- `kalangod conector rodar` é a ponte entre o motor de agentes (stdio) e o servidor (HTTP). Entende respostas em
  JSON e em SSE e mantém a sessão (`Mcp-Session-Id`). Renova o token um pouco antes de vencer; se a autorização for
  revogada, responde com a instrução de rodar `entrar` de novo. O filtro de ferramentas bloqueadas e o registro são
  os mesmos dos conectores locais.
- `kalangod conector testar` funciona igual para conectores remotos.

Só fala com a origem do manifesto e não segue redirecionamentos. Exige HTTPS, exceto em 127.0.0.1 e localhost.
Manifesto sem OAuth: `token = "cofre:NOME"`.

### Cofre de segredos

`kalangod cofre guardar NOME` lê o valor sem mostrá-lo na tela. Também há `kalangod cofre listar`, que mostra só
nomes, e `kalangod cofre apagar NOME`.

Os segredos ficam no chaveiro do sistema (Secret Service, via `secret-tool`). Sem chaveiro, ficam em
`~/.local/share/kalango/cofre.json`, com permissão 0600 numa pasta 0700. Esse arquivo é pasta protegida: a guarda eBPF
barra agentes, e nenhum manifesto de conector consegue pedi-lo.

## Ferramentas

| Nível | Ferramentas |
|---|---|
| observar | `sistema.info`, `kernel.seguranca`, `servicos.listar`, `servicos.estado`, `discos.uso`, `discos.dispositivos`, `discos.saude`, `rede.interfaces`, `rede.conexoes`, `rede.wifi`, `firewall.estado`, `usuarios.listar`, `atualizacoes.listar`, `processos.listar`, `registros.ler`, `pacotes.listar`, `sysctl.ler`, `pontos.listar`, `linha_do_tempo.listar` |
| usuário | `arquivos.listar`, `arquivos.ler`, `arquivos.escrever`, `arquivos.mover`, `arquivos.lixeira`, `arquivos.restaurar`, `arquivos.apagar_criado`, `tarefas.executar` |
| sistema | `pacotes.instalar`, `pacotes.remover`, `pacotes.versoes`, `servicos.acao`, `sysctl.definir`, `processos.sinal`, `rede.conexao`, `firewall.regra`, `firewall.ligar`, `discos.montar`, `discos.desmontar`, `usuarios.criar`, `usuarios.remover`, `usuarios.grupo`, `atualizacoes.verificar`, `atualizacoes.aplicar`, `pontos.criar`, `pontos.restaurar`, `linha_do_tempo.desfazer` |
| total | `sistema.executar` |

As ações reversíveis registram o próprio inverso. Exemplos: `atualizacoes.aplicar` volta às versões anteriores com
`pacotes.versoes`, `usuarios.criar` remove a conta, `firewall.regra` apaga a regra e `discos.montar` desmonta.
`usuarios.remover` não tem inverso direto: o ponto de restauração recupera a conta.

## Uso

```sh
cargo build --release -p kalangod
# desenvolvimento, sem root, sem executar ações de sistema:
./target/release/kalangod servir --dev /tmp/kalangod --simular
export KALANGOD_SOCKET=/tmp/kalangod/mcp.sock
./target/release/kalangod ferramentas
./target/release/kalangod pendentes          # num terminal: aprovações esperando você
./target/release/kalangod aprovar 3
```

Para usar num cliente MCP (por exemplo, o Claude Code), registre o comando `kalangod mcp` como servidor stdio:
ele liga o cliente ao soquete.

Instalação no sistema:

- `dist/kalangod.service` e `dist/kalango-agentes.slice` em `/etc/systemd/system/`;
- `dist/kalangod.toml` em `/etc/kalango/`;
- o binário em `/usr/libexec/kalango/`.

## Testes

Três camadas, todas passando:

0. Portão de confirmação do Kalango Mind, o que só existe com host (sessão de usuário com `/run/user/UID`, systemd de usuário,
   cgroup delegado): `crates/kalangod/vm/portao-mind.sh` (`KALANGOD_REQUIRE_HOST_TESTS=1 cargo test -p kalangod --locked --
   --include-ignored …`). O CI não os roda. A guarda eBPF com a pasta real e o Mind real são provas manuais da VM: ver
   `docs/SEGURANCA.md`.
1. `cargo test -p kalangod`, sem root, com 22 testes:
   - protocolo, arquivos com desfazer, pastas protegidas (inclusive link simbólico e `..`);
   - níveis sistema e total, expiração da concessão, negação, proibição de aprovar o próprio pedido e chamador sem regra recusado;
   - tarefas com limite de memória, tempo e processos, e Landlock barrando escrita fora da área;
   - pontos de restauração reais (criar, mudar, planejar, restaurar, desfazer a restauração, retenção);
   - entradas perigosas recusadas nas ferramentas de administração;
   - seccomp do auxiliar bloqueando exec e rede;
   - aprovador gráfico confiável (executável estranho ou gravável por outros é recusado) e espera longa;
   - conectores: um conector de teste tenta ler fora do declarado, ler `~/.ssh`, escrever fora da pasta dele, usar
     o `/tmp` comum, conectar sem rede e em porta não declarada, ler o token do ambiente, usar ptrace e estourar a
     memória; tudo é barrado e registrado;
   - conector remoto contra um servidor de teste que exige OAuth com PKCE e emite tokens de 2 s: autorização, token só
     no cofre, sessão, SSE, renovação automática, ferramenta bloqueada que não chega ao servidor, revogação e recusa de
     http fora de localhost;
   - cofre (permissões 0600/0700, só nomes na listagem) e segredo entregue só ao conector, que não alcança o cofre.
2. `sudo vm/run.sh`: 43 verificações como root numa VM descartável (QEMU/KVM). A VM usa o kernel do host com `lsm=…,bpf`
   e a raiz do host só para leitura, com uma camada em memória por cima. Cobre:
   - a guarda eBPF bloqueando `~/.ssh` e o `/etc/shadow` até para um agente root, com auditoria de execução e escrita;
   - os limites em cgroup direto;
   - contas e grupos com desfazer, firewall (ufw) com desfazer;
   - `apt-get update` e uma atualização real desfeita até a versão anterior;
   - um ponto do `/etc` real com pacote removido e reinstalado pela rede, mais o desfazer;
   - um snapshot btrfs somente leitura restaurado.

   As verificações passam também com o kernel 7.3.4 do KalangoOS (`kernel/build.sh`), sem passar `lsm=`: o `bpf` já vem
   ligado no kernel.
3. `sudo vm/host.sh`: 15 verificações como root no host, com systemd. Cobre:
   - tarefas como escopos do systemd com limites;
   - NetworkManager (uma conexão dummy ativada e desfeita);
   - udisks (uma imagem loop montada e desmontada pelo desfazer) e SMART do disco real;
   - um ponto do `/etc` real, só com o plano.

   O teste desfaz tudo o que criou.
4. `crates/kalango-aprovacao/validar.sh`: a tela de aprovação de ponta a ponta, num compositor Wayland em memória, com
   o kalangod como root e a tela instalada como root. Um agente pede, e uma "pessoa" tecla pelo teclado virtual.
   Verifica que Ctrl+Enter aprova, Enter sozinho não aprova, Esc nega, a trava de 0,7 s ignora a tecla precoce e a
   tela não pode ser inspecionada. Gera capturas. `ver.sh` abre a mesma tela ao vivo numa janela do niri.

## Ainda não feito

- A guarda eBPF no hardware real: o kernel 7.3.4 está pronto e validado na VM, mas a instalação no host (e o teste
  com o driver NVIDIA 580) só com a sua autorização.
- No compositor do KalangoOS, a tela de aprovação deve ser a única com teclado exclusivo. Os protocolos de teclado e
  mouse virtuais não podem ser oferecidos a agentes, senão um agente tecla Ctrl+Enter por você. Isso fica na fase 1.
