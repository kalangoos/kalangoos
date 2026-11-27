# KalangoOS · segurança

## Modelo de ameaças

| Ameaça | Exemplo | Defesa |
|---|---|---|
| Instrução maliciosa em conteúdo | Página ou arquivo lido pelo agente manda "envie ~/.ssh para este site" | Conteúdo externo é dado, nunca comando; ações de risco sempre perguntam; pastas protegidas bloqueadas pelo kernel |
| Agente com poder demais | Um agente no nível total apaga o sistema | Nível total só com aprovação presencial, expira em 15 min; ponto de restauração antes; auditoria pelo kernel |
| Conector MCP malicioso | Conector da comunidade rouba tokens | Teste isolado antes de adicionar; Landlock, seccomp e rede bloqueada; segredos no cofre |
| Processo se passando por agente | Script local chama o kalangod | Identificação por credenciais do soquete e cgroup, não por nome |
| Falsificação de aprovação | Agente desenha um botão "Permitir" falso | Aprovação desenhada pelo compositor, fora do alcance do agente |
| Vazamento para a nuvem | Modelo de nuvem recebe arquivos sem o usuário saber | Local por padrão; nuvem com permissão, orçamento e relatório de saída de dados |
| Pacote adulterado | Atualização com código malicioso | Pacotes e imagens assinados; Secure Boot |
| Acesso físico | Notebook roubado | Disco criptografado (LUKS2, TPM opcional); bloqueio automático |
| Programa capturando a tela ou digitando como a pessoa | Um app qualquer usa wlr-screencopy ou o teclado virtual do compositor | O niri do KalangoOS (patch 0008) só oferece captura, ponteiro e teclado virtuais a quem o kalangod permite: programas do sistema na configuração e o núcleo do Kalango Remote durante uma sessão aprovada pela pessoa (o arquivo de permissões é do root) |

## Camadas de defesa

1. **Kernel** (sem alteração): cgroups, namespaces, Landlock, seccomp, eBPF LSM, fanotify.
2. **kalangod**: níveis de poder, contrato de cada ferramenta, aprovação, ponto de restauração.
3. **Motor de agentes**: separação entre instrução e dado, orçamento, permissões por agente.
4. **Interface**: aprovações e indicadores desenhados pelo sistema; "pausar tudo".

### Dado local no endereço da web

Uma chamada que pode levar texto para a rede (ferramentas `web.*`, conectores, e comandos que trazem um endereço
`http(s)://…`) não pode levar para fora um dado lido neste computador (arquivo, sistema, tela, memória) que a pessoa
não escreveu. O segredo não precisa aparecer inteiro e sozinho para ser barrado. O motor monta candidatos a segredo a
partir do conteúdo local: cada token e suas partes, sequências de dígitos que atravessam separadores (cartão
`4111 1111 1111 1111`, CPF `123.456.789-09`, telefone, `58213-9` como `582139`), o token sem separadores, e-mails e
palavras longas (para as formas codificadas) e janelas de 24 caracteres de tokens muito longos (chaves, JWT, PEM).
Cada candidato é procurado no endereço normalizado: percent-encoding até não mudar mais (também `%uXXXX`),
entidades HTML (`&#53;`, `&#x35;`), caracteres parecidos (NFKC: dígitos e letras de largura total), caracteres de
largura zero removidos, sem diferenciar maiúsculas e acentos, colado a outros caracteres (`/conta-58213-9`,
`58213.evil.example`, `https://58213@host`, `x58213y`), em base64 (3 alinhamentos, alfabeto padrão e seguro para URL,
com e sem preenchimento) e em hex.

Para não barrar chamadas comuns: um número precisa de 4 dígitos e não ser ano (1900 a 2100), data ou versão
(`v1.2.3`); porta não é dado; uma palavra com número (`ubuntu24`, `sha256sum`, `python312`, `utf8mb4`) não é segredo
(sem separadores exige 8 caracteres com metade de dígitos, ou 12); um número maior que contém o trecho não conta
(`/item/14821` não leva o PIN `4821`); o que a pessoa escreveu é dela; um site que páginas ou buscas já mostraram pode
ser visitado; as formas codificadas precisam de pelo menos 5 caracteres de origem e 6 de saída.

Limites que a regra não cobre (inerentes a quem só vê o endereço): texto invertido, rot13, base32/base58, XOR,
caracteres de ruído intercalados no meio do segredo, segredo dividido entre parâmetros ou rótulos de domínio,
base64 em minúsculas num rótulo de DNS, e o que vai fora do endereço (corpo de POST, fragmento). O resto da defesa
(aprovação, nível de poder, rede bloqueada para ferramentas isoladas) continua valendo.
A regra está em `crates/kalango-agente/src/defesa.rs` (`glued_secret`) e é testada por funções puras e com o modelo
real (`tests/web.rs`).

## Controle remoto da tela: quem pode capturar e digitar

O niri (patch `sessao/niri/0008-kalangoos-controle-remoto.patch`) só mostra captura de tela, ponteiro virtual e teclado
virtual a quem o kalangod listou num arquivo que só o root escreve: por executável (`exe`, o teclado na tela do shell)
ou por processo (`pid` + hora de início, o núcleo do Kalango Remote depois que a pessoa aprovou o acesso).

O que o kalangod exige antes de listar um núcleo do Kalango Remote (`crates/kalangod/src/remote_control.rs`):
o app que pede é um dos listados em `controle_remoto_apps` e o alvo é o filho dele, do mesmo usuário, o `kalango-remote-core`
ao lado; nenhum dos dois leva carga de biblioteca ou de plugin no ambiente (`LD_PRELOAD`, `LD_AUDIT`, outro `LD_LIBRARY_PATH`
que não o do script da imagem, camadas Vulkan, plugins do PipeWire e do GStreamer, drivers GL, fornecedores EGL, módulos do GTK
e do GIO) nem tem depurador; os dois são **não depuráveis** (`prctl(PR_SET_DUMPABLE, 0)` logo no começo de cada um: os
arquivos de `/proc/<pid>/` passam a ser do root, o que o kalangod confere); e, como root, os arquivos não podem ser
trocados por outra conta, e, **como root, o que o processo é e o que ele carregou vem do kernel, não do texto de um caminho**:
- os dois estão nos **mesmos namespaces de montagem e de usuário que o kalangod** (`/proc/<pid>/ns/mnt` e `ns/user`): num
  `bwrap` ou `unshare -rm` o processo enxerga arquivos próprios montados por cima de `/usr/lib/kalango/remote/*`, e então o
  caminho do `exe`, o `maps` e tudo que se lê como texto parece o do sistema;
- o executável é **o mesmo arquivo** que o kalangod confia (dispositivo e inode de `/proc/<pid>/exe` contra o arquivo
  listado), e esse arquivo é do root sem escrita de outros;
- cada mapeamento executável é conferido pelo `/proc/<pid>/map_files/<faixa>` (o arquivo mapeado de verdade): é do root,
  sem escrita de grupo nem de outros, está em `/usr/lib/` ou `/lib/` (`controle_remoto_bibliotecas`) e é o mesmo arquivo
  (dispositivo e inode) que há nesse caminho; **qualquer mapeamento `(deleted)` é de fora**. No **núcleo**, também é de fora
  qualquer memória anônima executável (o núcleo de verdade só tem `[vdso]` e `[vsyscall]`; medido numa VM). O app pode ter
  memória anônima executável (compilador de shaders do Mesa, fechos do GObject): é um limite, abaixo.
Uma biblioteca carregada que apagou a própria linha `LD_PRELOAD` da memória some do `environ`, mas não do `maps`.

O script `/usr/bin/kalango-remote` tira `LD_PRELOAD` e `LD_AUDIT` do ambiente antes do `exec` (o shell que abre o app pelo dock os
tem), e o shell também não os passa aos apps que abre.

**Depois** da permissão, a cada segundo o kalangod olha de novo (`recheck`; pid e hora de início continuam conferidos). Se o
núcleo passar a poder ser depurado ou ganhar um depurador, a permissão cai, o arquivo do niri é reescrito e **o núcleo é
encerrado** (SIGKILL, só se pid e hora de início ainda forem os da permissão) e a Linha do tempo registra. Encerrar é
necessário: o niri só confere o arquivo quando um cliente liga os globais, então tirar a linha não pararia uma sessão que já
está rodando. Um `kalangod` que não roda como root não consegue ler o `environ` nem o `maps` de um processo protegido: diz
isso no log ("não conferido") e segue, em vez de dizer "limpo" (só desenvolvimento).

O `ptrace_scope` 1 do Yama (só ancestrais anexam) ajuda, mas não é o que sustenta isto. Antes da lista não há um clique da
pessoa **no kalangod**: a aprovação é a do app ("Pedido de acesso remoto", ou a senha fixa que a pessoa ligou antes), e o
kalangod confia no app só depois das conferências acima.

**Áudio com o núcleo não depurável: não verificado.** O núcleo deixa de ser legível pelo `/proc` de quem não é root
(`exe` e `environ` negados); o PipeWire/PulseAudio consulta o processo do cliente por pid e pode perder o nome do
programa nas propriedades do fluxo. As VMs de teste não têm placa de som, então o áudio da sessão remota com o núcleo
protegido ainda precisa de um teste em hardware (a captura de som do Remote vem desligada por padrão).

**Limites que sobram, ainda abertos:**
- **O app (não o núcleo) pode ter memória anônima executável** legítima: uma biblioteca carregada no app que se copie para
  memória anônima, descarregue o próprio arquivo e apague o `LD_PRELOAD` passa pelo `maps`. O script do app tira `LD_PRELOAD` do
  ambiente e o `exec` do app é conferido pelo inode, mas o que roda dentro do app depois do `exec` só é visto pelo que ele mapeou
  de arquivo.
- **A conferência é do momento da permissão e de cada segundo depois**, não contínua: entre dois olhares um processo do mesmo
  usuário que não possa depurar o núcleo (não depurável) não consegue mexer nele, mas o app, que é depurável até o `prctl`,
  tem a janela abaixo.
- **Janela entre o `exec` e o `prctl`:** entre o `exec` do app (ou do núcleo) e o `prctl` na primeira linha do `main`, o processo ainda
é depurável. Um processo do mesmo usuário que o lance com `PTRACE_TRACEME`, mexa nele nessa janela (injeta código, muda a
memória) e solte antes do `prctl` deixa `TracerPid` em 0 e, se o que ele injetou não vem de um arquivo de fora das pastas do
sistema, passa nas conferências. O `maps` pega o que veio de arquivo; código injetado em memória anônima não. Fechar de vez
pede que o kernel já entregue o processo sem depuração (um binário setgid, ou `PR_SET_DUMPABLE` herdado de um lançador do
próprio `kalangod`, ou um `RUNPATH` fixo no lugar do `LD_LIBRARY_PATH`): endurecimento futuro, não feito na 0.1.1.

Limites conhecidos, ainda sem correção (o niri precisa ser recompilado e a imagem refeita para testar):

- **O niri identifica o cliente por pid.** `allowed()` lê `/proc/<pid>/exe` e a hora de início quando o cliente lista os
  globais, não quando ele conectou. Para a regra `pid` (núcleo do Remote) a hora de início evita um pid reaproveitado.
  Para a regra `exe` (teclado na tela) **não há** nada além do caminho do programa: qualquer processo do mesmo usuário que
  rode `/usr/libexec/kalango/kalango-shell.bin` com uma biblioteca sua (`LD_PRELOAD`), ou que conecte e depois faça `exec`
  desse programa com o descritor do soquete herdado (a conexão sobrevive ao `exec`), é visto como o shell e ganha o
  teclado virtual. Isto vem da leitura do código do patch (`remote_control::allowed`, `permits`) e **não foi reproduzido
  de ponta a ponta**: um cliente Wayland que fizesse isso (um programa carregado no `kalango-shell.bin` por `LD_PRELOAD`,
  ou um `exec` do shell com o soquete herdado) é o teste que falta, e o nível de log `niri::remote_control=debug` mostra a
  decisão (`... Keyboard → true`) para quem for escrevê-lo. Para o `pid` (núcleo do Remote), o teste de reuso de pid cabe
  no patch: um cliente que conecta, sai, e outro processo que herda o pid não pode combinar com a hora de início.
- **Correção planejada:** ligar a permissão ao objeto do cliente Wayland na conexão, com `SO_PEERPIDFD` (kernel 6.5+):
  conferir a identidade UMA vez pelo pidfd, guardar o resultado nos dados do cliente e nunca mais derivar de um pid; ou o
  kalangod entregar ao app uma conexão própria (`socketpair`/`wp_security_context_v1` com um id de app) e o niri confiar
  na conexão, não no pid. A regra `exe` ganharia a mesma exigência de não depurável e sem biblioteca estranha que o
  kalangod faz para o núcleo do Remote.

## Portão de confirmação do Kalango Mind: o que a caixa de areia dos agentes nega

O Kalango Mind (`kalango-knowd`) não confia no que o chamador diz para confirmar uma lembrança (nem um campo do pedido, nem
`isatty`, nem o cgroup, que um processo do mesmo usuário troca sem root). Lembrar cria um item pendente e um código de uso
único em `$XDG_RUNTIME_DIR/kalango/mind-confirm/<revisão>.token`; só quem lê esse arquivo confirma, e quem deve lê-lo é a
sessão da pessoa. O que impede um agente de confirmar o próprio pedido é esta caixa de areia:

| O que se nega aos agentes | Onde está no kalangod | O que o impede |
|---|---|---|
| `%t/kalango/mind-confirm/` (ler, escrever, listar, mover, apagar, abrir por fd) e o banco `~/.local/share/kalango/mind` | `policy::KALANGO_PROTECTED` (e `dist/kalangod.toml`, que substitui a lista padrão); `%t` é `/run/user/UID` (`policy::expand`) | ferramentas de arquivo (`sandbox::check_path`: recusa pelo lugar, também por `..`, elo ou arquivo que ainda não existe); comandos e conectores (Landlock: `/run` deixa de ser concedido inteiro, `admin::system_read` concede o resto e a pasta fica de fora, mesmo criada depois); guarda eBPF por inode (`guard.rs` procura a pasta de novo a cada segundo, porque ela nasce depois do `kalangod`) |
| escrita em `/sys/fs/cgroup` (criar pasta em `app.slice` e escrever o pid em `cgroup.procs`) | já negado: o Landlock de todo comando, navegador e conector só concede escrita em `/tmp`, `/dev/null` e nas pastas de trabalho; `/sys` é só leitura | Landlock (negação de escrita) |
| D-Bus do gerenciador de usuário do systemd (`StartTransientUnit`, `systemctl --user`) e `systemd-run` | já negado: nenhum soquete local fora dos que o manifesto de um conector nomeia (Landlock ABI 9, `ResolveUnix` e escopo de soquetes abstratos; em kernel mais antigo, seccomp sem `AF_UNIX`) | Landlock + seccomp |
| um caminho para `confirmar` | o `kalangod` não tem ferramenta do Mind (as `conhecimento.*` são a P2 e só terão `remember`/`search`/`show` pelo escopo do chamador) | catálogo de ferramentas (teste) |

O que fica aberto, de propósito e dito:

- **Um processo do mesmo usuário fora de toda caixa de areia continua sendo a própria pessoa para o sistema.** O portão só
  vale para o que o `kalangod` põe na caixa de areia (as tarefas, os conectores e os navegadores dos agentes). Um agente que
  consiga rodar fora dela (por um erro nosso, ou por um programa que a própria pessoa lançou para ele) lê o código.
- **O `/etc/kalango/kalangod.toml` substitui a lista padrão** (`Config::load` não a mescla) e o pacote não o atualiza numa
  máquina que já o tem. Por isso o `kalangod` acrescenta sempre `~/.local/share/kalango/mind` e `%t/kalango/mind-confirm` à lista
  carregada (e avisa no log de partida quando o arquivo não as tinha); as demais linhas novas de uma versão continuam
  dependendo de o arquivo as ter. Para ter as duas no arquivo: acrescentar `"~/.local/share/kalango/mind"` e
  `"%t/kalango/mind-confirm"` a `protected_paths`.
- `kalango/protegidas` devolve `%t` já expandido para quem chama (o painel de privacidade mostra o lugar real).
- **`%t` é `/run/user/UID` fixo** na política (`policy::runtime_dir`), nas ferramentas de arquivo, no Landlock de comandos e na
  guarda eBPF. Uma sessão com `XDG_RUNTIME_DIR` diferente disso fica fora de cobertura (só o conector soma o `XDG_RUNTIME_DIR` do
  próprio processo). O logind faz `/run/user/UID` em toda sessão do KalangoOS.
- A sondagem da guarda eBPF a cada 1 s (e a pasta nascendo depois do `kalangod`) **só se prova com root e o LSM `bpf`**: nenhum
  teste de `cargo test` roda assim. Provado à mão no host (27/11/2026, `kalango-teste-guarda-host`): fora do cgroup dos agentes o
  arquivo protegido é lido; dentro de `kalango-agentes.slice` o kernel nega (`Operação não permitida`).
- As pastas protegidas fora de `%t` (como `~/.local/share/kalango/mind`) são procuradas de novo a cada 15 s; só as de `%t/...` a
  cada 1 s. A pasta dos Fluxos fica fora da guarda do kernel e segue protegida pela política e pelo Landlock (limite conhecido:
  Fluxos acessado como outro usuário é decisão do mantenedor; ver `docs/ISO-CONTEUDO.md`).
- A janela entre a pasta ser criada e a guarda eBPF conhecê-la (até um segundo). O Landlock e as ferramentas de arquivo não
  têm essa janela (valem pelo lugar). Fecha de vez se o serviço criar a pasta ao iniciar a sessão.
- Os nomes na pasta (ids das revisões, não códigos) ainda podem ser listados por um comando quando a guarda eBPF está
  desligada: a regra de listar de uma pasta vale para as de baixo no Landlock. O conteúdo não.
- `/etc` segue concedido inteiro para leitura (não dá para recortá-lo sem perder seus elos); o que há nele de protegido é do
  root.
- A guarda eBPF e o Landlock real em kernel de produção (e o `%t` com o `XDG_RUNTIME_DIR` de verdade) precisam da prova na
  VM: veja a lista na descrição do pull request. (`KALANGOD_RUNTIME_BASE` só existe para a suíte de testes mover o `%t`
  para dentro da casa de teste; o serviço de produção usa `/run/user`.)

**O que o CI prova e o que não prova.** O CI roda `cargo test -p kalangod` num runner sem sessão de usuário. Sem depender do host
(provado no CI): `the_landlock_assembly_leaves_out_the_mind_confirmation_codes` (a montagem única `admin::agent_landlock` corta
`/run` em volta da pasta, pelo lugar) e `a_command_and_a_connector_build_their_rules_only_through_the_assembly` (um teste de
fonte: `task_run` e `conector::spawn` não montam `Landlock` nem usam a lista do sistema por conta própria; reverter qualquer um
dos dois, ou a montagem, derruba um dos dois testes), mais as ferramentas de arquivo (`sandbox.rs`), a lista padrão e o toml
(`policy.rs`) e a expansão da guarda (`guard.rs`). **Só provado no host/VM** (pulam no CI, e com `KALANGOD_REQUIRE_HOST_TESTS=1`
a falta do host é falha): a leitura real de um comando e de um conector com `%t` sob `/run/user/UID`, o escape de cgroup e o
D-Bus do gerenciador de usuário; o comando exato é `crates/kalangod/vm/portao-mind.sh`. **Só provado na VM, como root**:
a guarda eBPF (o LSM `bpf`), a sondagem de 1 s da pasta recém-criada e o Mind real criando o token.

Testes (`crates/kalangod`): `agent_file_tools_cannot_reach_the_mind_confirmation_codes`,
`an_agent_command_cannot_read_or_forge_the_mind_confirmation_codes`, `a_command_cannot_move_itself_to_another_cgroup`,
`a_command_cannot_ask_the_user_manager_to_start_a_unit`, `kalangod_exposes_no_path_for_an_agent_to_confirm_the_mind`
(`tests/mcp.rs`), e as de unidade `a_command_cannot_read_the_mind_confirmation_codes_outside_its_home` (`mcp.rs`, com `%t` de verdade fora da casa),
`a_connector_cannot_read_the_mind_confirmation_codes` (`tests/conectores.rs`, só onde há systemd de usuário), `the_connector_protects_the_codes_in_its_own_runtime_folder` (`conector.rs`, o `XDG_RUNTIME_DIR` do conector), `the_landlock_assembly_leaves_out_the_mind_confirmation_codes`, `a_commands_rules_come_from_the_protected_list_of_the_configuration` e `a_command_and_a_connector_build_their_rules_only_through_the_assembly` (`tools/system_read_tests.rs`), `the_connector_rules_leave_out_the_mind_codes_and_the_shown_isolation_says_so` (`conector.rs`: as regras e o JSON `isolamento` mostrado à pessoa, com a pasta do Mind cortada de `/run`), e os testes da própria `KALANGOD_REQUIRE_HOST_TESTS` (`the_require_host_tests_variable_*`, em `tests/mcp.rs` e `tests/conectores.rs`), `the_mind_confirmation_codes_are_out_of_reach` (`sandbox.rs`),
`the_mind_confirmation_folder_is_protected_by_default_and_in_the_installed_config` (`policy.rs`) e
`the_guard_expands_the_runtime_folder_for_every_user` (`guard.rs`) e `an_old_configuration_file_still_protects_the_mind_folders` (`policy.rs`).

## Processo

- Divulgação responsável em `kalangoos.com/seguranca` (a publicar), com resposta em até 5 dias úteis.
- Correções de segurança publicadas fora do ciclo normal de versões.
- Revisão externa do kalangod e do isolamento antes da versão 1.0.
- Testes automáticos com páginas e arquivos maliciosos a cada versão (fase 4).

## O `/dev/kvm` e os agentes

O `70-uaccess.rules` do systemd dá o `/dev/kvm` a quem está na sessão ativa (sem grupo `kvm`), e o Kalango VirtualDesk
conta com isso. Um agente que roda como o usuário da sessão (o `kalango-agente`, o serviço dos Fluxos, as tarefas do
`kalangod`) abre o mesmo dispositivo: ele pode rodar uma máquina virtual, e dentro dela fazer o que quiser, fora das
regras de arquivo e de rede dos agentes. O que limita isso é a camada de aprovação (as ferramentas de máquinas pedem
aprovação e seguem a política de rede do VirtualDesk), não o kernel. Quem não quer isso pode tirar o `uaccess` do
`/dev/kvm` com uma regra do udev própria.
