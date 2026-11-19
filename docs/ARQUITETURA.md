# KalangoOS · arquitetura

Atualizado em 15 de novembro de 2026. Registra as decisões tomadas e o desenho das partes do sistema.
O que ainda falta fazer está em `PLANO.md`.

---

## 1. Decisões

| # | Decisão | Estado | Escolha |
|---|---|---|---|
| D1 | Base do compositor | **Em teste** | Partir de um compositor Wayland em Rust existente. Candidatos: niri e cosmic-comp. Escolha final depois de rodar a prova de conceito nos dois |
| D2 | Toolkit do shell e dos apps | **Decidido** | GTK4 em Rust, estilizado com CSS a partir dos tokens. Validado em `poc/` |
| D3 | Base da distribuição e kernel | **Decidido** | Ubuntu LTS como base. Kernel: o estável mais recente do kernel.org (hoje 7.3.4), compilado pelo KalangoOS sobre a configuração do Ubuntu com `kernel/kalango.config`; o kernel HWE do Ubuntu fica como reserva |
| D4 | Atualização e restauração | Aberto | Depende do sistema de arquivos (ver seção 5) |
| D5 | Apps de terceiros | **Decidido** | Flatpak. Sem snaps por padrão |
| D6 | Motor de modelos locais | **Decidido (15 nov 2026)** | llama.cpp (`llama-server`, CUDA na NVIDIA) com os modelos LFM3 da Liquid AI em GGUF: LFM3-1.7B (texto, chamada de ferramentas, 256K de contexto, ≈ 1 GB em 4 bits) como padrão; LFM3-VL-2B, LFM3-Audio-2B e LFM3-Omni-2B para visão, voz e multimodal. Os agentes operam o sistema pelo kalangod (MCP). Motor: `crates/kalango-agente`; o `llama-server` ouve só num soquete Unix da pessoa (pasta 0700, sem porta TCP), com o contexto inteiro de 256K e sem teto de resposta; `--fit` põe na GPU o que couber. |
| D7 | Nome e marca | **Parcial** | Domínio `kalangoos.com` comprado. Falta verificar o registro de marca |
| D8 | Escopo da versão 0.1 | Proposto | Fases 0 a 2 do plano |
| D9 | Controle total do sistema pelos agentes | **Decidido (desenho abaixo)** | Serviço privilegiado que fala MCP, com regras impostas pelo kernel. Sem módulo de kernel próprio |

### Por que Ubuntu LTS e não Debian

- **Kernel novo sem trabalho nosso.** Esta máquina roda Ubuntu 24.04 com kernel 6.17, e o repositório já oferece o 7.0
  pela série HWE. No Debian estável o kernel fica mais velho; kernels novos vêm por *backports*.
- **Driver NVIDIA.** O Ubuntu entrega o driver proprietário já assinado para Secure Boot. É o ponto fraco conhecido do
  Wayland, então quanto menos atrito, melhor.
- **A máquina de desenvolvimento é igual ao alvo.** O que funciona aqui funciona na imagem.

O que pesa contra: o Ubuntu traz snap e marca da Canonical, que precisam ser removidos da imagem. Se isso virar
problema, a troca para Debian é possível porque nada abaixo depende de algo exclusivo do Ubuntu.

---

## Princípios

1. **Rust em tudo o que for nosso**, para o sistema ficar leve e rápido: shell, compositor, serviço de agentes,
   `kalangod` e apps próprios. Os programas eBPF são a exceção: são escritos em C com CO-RE (o eBPF em Rust ainda não
   tem relocação por BTF) e carregados pelo `kalangod` com a biblioteca Aya, em Rust. Ficam fora do nosso alcance o kernel, os
   drivers e as bibliotecas do sistema (Mesa, PipeWire, systemd). Rust não garante leveza sozinho: cada peça é medida
   (memória, CPU parada, tempo de início) antes de ser adotada.
2. **Compatibilidade com o driver NVIDIA proprietário é requisito**, não extra. Toda escolha de compositor e de
   toolkit passa por teste numa placa NVIDIA. Máquina de referência: RTX 3090, driver 580.
3. **Kernel sem código próprio.** Usamos o estável mais recente do kernel.org, sem patches, só com configuração
   (`kernel/kalango.config`): eBPF LSM e Landlock ativos por padrão, BTF, cgroups v2, fanotify, btrfs.
   Riscos: o kernel precisa ser assinado por nós para Secure Boot (chave MOK) e o driver NVIDIA precisa
   compilar contra ele a cada versão. Se algo falhar, a imagem volta ao kernel HWE do Ubuntu.

### O que os princípios mudam nas decisões

- **D2 (toolkit) volta a ficar aberto.** O GTK4 tem núcleo em C com ligação para Rust. A opção toda em Rust é
  iced/libcosmic. Falta uma segunda prova de conceito para comparar fidelidade ao design, consumo e maturidade.
- **D1 (compositor):** os dois candidatos são em Rust. O cosmic-comp tem a vantagem de vir de um fabricante que vende
  máquinas com NVIDIA.

### Teste do niri (15 nov 2026)

O niri 26.04 compilou nesta máquina e rodou a prova de conceito numa janela, com as duas superfícies presas às bordas
e o driver NVIDIA 580. Cerca de 50 quadros por segundo nesse modo em janela. O que **não** foi testado: sessão real
fora do X11 (é onde aparecem os problemas de NVIDIA), dois monitores, suspensão e o cosmic-comp.

---

## 2. Camadas

```
┌───────────────────────────────────────────────────────────────────────┐
│ Apps (GTK4 + tema Kalango)   Apps de terceiros (Flatpak)              │
├───────────────────────────────────────────────────────────────────────┤
│ Shell: barra superior e ilha · dock · barras laterais · notificações  │   Rust + GTK4, superfícies layer-shell
├───────────────────────────────────────────────────────────────────────┤
│ Compositor Wayland (base em Rust: niri ou cosmic-comp)                │   janelas, espaços, animações, captura
├──────────────────────────────┬────────────────────────────────────────┤
│ Agentes (sessão do usuário)  │ kalangod (serviço do sistema, root)    │   os dois em Rust
│ motor, memória, chat, MCP    │ "MCP do sistema" + trilha de auditoria │
├──────────────────────────────┴────────────────────────────────────────┤
│ Kernel Linux do Ubuntu, sem alterações: Landlock, seccomp, cgroups,   │
│ namespaces, eBPF (LSM e auditoria), fanotify                          │
└───────────────────────────────────────────────────────────────────────┘
```

---

## 3. O "MCP do sistema" (D9)

**Objetivo:** os agentes nativos da plataforma conseguem operar o sistema inteiro quando necessário, com as regras
impostas pelo kernel e tudo registrado.

### 3.1 Por que não um módulo de kernel

MCP é um protocolo de mensagens (JSON-RPC). Colocá-lo dentro do kernel traria três problemas:

- **Perde-se o motivo de usar Ubuntu.** Um módulo fora da árvore precisa ser recompilado e reassinado a cada kernel
  novo. Quando a compilação falha, a máquina fica sem o recurso ou sem iniciar.
- **Um erro derruba o sistema.** Interpretar JSON e conversar com modelos de IA dentro do kernel transforma qualquer
  falha em travamento ou brecha de segurança com poder total.
- **Não é preciso.** Tudo o que um administrador faz no Linux já é feito do espaço de usuário, pelas interfaces do
  kernel. Um serviço com privilégio de root alcança o sistema inteiro.

O que faz sentido rodar **dentro** do kernel é a parte de *impor* e *registrar*, e para isso existe o eBPF: programas
pequenos, verificados pelo próprio kernel antes de carregar, que continuam funcionando quando o kernel é atualizado.

### 3.2 Peças

| Peça | Onde roda | Papel |
|---|---|---|
| `kalangod` | Serviço do sistema (root), em Rust | Servidor MCP num soquete local (`/run/kalango/mcp.sock`). Executa as ações privilegiadas. Nunca escuta na rede |
| Motor de agentes | Sessão do usuário, em Rust | Conversa com os modelos, decide, pede ações ao `kalangod`. Não tem privilégio próprio |
| Programas eBPF | Dentro do kernel, carregados pelo `kalangod` | Impõem as regras que nem o root dos agentes pode quebrar e geram a trilha de auditoria |
| Aprovação | Superfície do compositor | A pergunta ao usuário é desenhada pelo sistema, não pelo agente, para não poder ser falsificada |

### 3.3 Ferramentas expostas

Agrupadas por domínio. Cada ferramenta declara o efeito (leitura, escreve, destrutiva), se é reversível e como desfazer.

| Domínio | Exemplos |
|---|---|
| Arquivos | ler, listar, mover, apagar (vai para a lixeira), permissões |
| Pacotes e apps | instalar, remover, atualizar (apt e Flatpak) |
| Serviços | estado, iniciar, parar, habilitar (systemd) |
| Rede | interfaces, Wi-Fi, VPN, DNS, firewall |
| Armazenamento | discos, partições, montar, uso, saúde |
| Usuários e sessão | contas, grupos, bloqueio, energia |
| Processos | listar, prioridade, encerrar |
| Kernel | parâmetros (sysctl), módulos, registro do kernel |
| Registros | journal, falhas, auditoria |
| Sistema | atualização, ponto de restauração, recuperação |
| Comando livre | executar qualquer comando como root (só no nível 3) |

### 3.4 Níveis de poder

| Nível | O que permite | Como é concedido |
|---|---|---|
| 0 · Observar | Ler estado do sistema, sem conteúdo de arquivos pessoais | Livre para agentes nativos |
| 1 · Usuário | Agir como o usuário, dentro das pastas permitidas | Pela tabela de permissões (livre, perguntar, nunca) |
| 2 · Sistema | Mudanças reversíveis: instalar pacote, mexer em serviço, rede, configurações | Pergunta ao usuário; ponto de restauração antes |
| 3 · Total | Qualquer ação como root, inclusive comando livre | Aprovação presencial a cada sessão, com prazo curto; ponto de restauração antes; tudo gravado |

O nível 3 é o "operar todo o sistema se necessário". Recomendação: ele nunca fica como "livre" permanente. Um agente
com root permanente e um modelo que pode ser enganado por um texto malicioso é o pior cenário do sistema.

### 3.5 O que o kernel impõe

- **Cada tarefa de agente roda isolada:** cgroup próprio em `kalango-agentes.slice` (limites de CPU, memória sem swap,
  processos e tempo), Landlock (escrita só nas pastas permitidas) e `no_new_privs`. O auxiliar das ações de arquivo tem
  também um filtro seccomp: sem exec, rede, montagem, ptrace, bpf e módulos. *Implementado e testado.*
- **Regras invioláveis via eBPF LSM:** por exemplo "pasta protegida: nenhum agente lê" vale até para o nível 3. Só o
  usuário, numa tela do sistema, suspende a regra. *Implementado*: o gancho `file_open` compara (dispositivo do
  superbloco, inode) dos diretórios pai com as pastas protegidas, para todo processo que descende do cgroup dos agentes.
  Verificado numa VM com `lsm=…,bpf`.
- **Auditoria que não depende do agente:** a Linha do tempo é alimentada por eventos do kernel, não pelo que o agente
  diz ter feito. *Implementado*: bloqueios, execuções (`bprm_check_security`) e aberturas para escrita chegam por um
  ringbuf eBPF. O fanotify fica para a auditoria de leituras, se ela for necessária.
- **Identidade pelo kernel:** o `kalangod` reconhece quem chama pelas credenciais do soquete (`SO_PEERCRED`) e pelo
  cgroup, não por um nome informado pelo chamador.

### 3.6 O que esta máquina já oferece

Verificado no kernel 6.17 do Ubuntu 24.04:

| Recurso | Estado |
|---|---|
| Landlock | Compilado e ativo |
| eBPF, com informações de tipo (BTF) | Disponível |
| eBPF LSM | Compilado, mas **não ativo**: falta incluir `bpf` na lista `lsm=` da linha de inicialização do kernel |
| fanotify com permissão | Disponível |
| Sistema de arquivos raiz | ext4, que não tem snapshots (o ponto de restauração usa a cópia do /etc; ver seção 5) |

Ou seja: nenhuma alteração no kernel é necessária. Basta um parâmetro de inicialização, que a nossa imagem define.

### 3.7 Perguntas em aberto

- O nível 3 pode ser dado sem o usuário presente (rotinas agendadas)? Recomendação: não.
- Agentes de terceiros (da loja) podem pedir nível 2? Recomendação: só com revisão; nível 3 nunca.
- Acesso remoto ao `kalangod` (administrar outra máquina)? Fora da versão 0.1.

---

## 4. Shell e compositor

- O shell são processos em Rust + GTK4 que criam superfícies layer-shell: barra superior, dock, barras laterais,
  notificações, indicadores. A prova de conceito (`poc/`) é o primeiro deles.
- O compositor cuida de janelas, espaços, encaixe, animações de janela e captura. As animações usam as mesmas molas
  dos tokens (`design/tokens/tokens.json`).
- Um comando por superfície (`kalango barra abrir controles`) permite que atalhos, o compositor e os agentes usem a
  mesma porta de entrada. A prova de conceito já funciona assim.

## 5. Atualização e desfazer (D4)

O design promete "ponto de restauração" e "voltar em 1 clique", e os níveis 2 e 3 dependem disso. O `kalangod` já
cria um ponto antes de toda ação do nível sistema; sem o ponto, a ação não acontece. Cada ponto guarda:

- **o /etc**: com raiz em **btrfs**, um snapshot somente leitura da raiz (via snapper, quando instalado); em ext4, uma
  cópia compactada do `/etc` com donos, permissões e atributos;
- **a lista exata de pacotes** (`nome:arquitetura=versão`).

Restaurar devolve o `/etc` ao estado do ponto, inclusive apagando o que foi criado depois, e reinstala ou remove
pacotes até a lista bater. Como a restauração cria um ponto antes, ela também se desfaz. Tudo isso foi validado numa VM:
pacote removido e reinstalado pela rede, conta criada por fora desfeita, snapshot btrfs restaurado.

Decisão: o instalador do KalangoOS usa **btrfs** por padrão. O snapshot cobre a raiz inteira, não só o `/etc`, e
permite voltar pelo menu de inicialização quando a sessão não sobe (fase 5, 2.2). Em ext4 o ponto continua
funcionando, limitado ao `/etc` e aos pacotes. Um sistema por imagem com troca atômica fica para depois da 1.0.

## 6. O que este documento ainda não cobre

Motor de agentes por dentro (fila, memória, orçamento), formato do registro da Linha do tempo, hospedeiro de
conectores MCP de terceiros, instalador e geração da imagem. Cada um ganha uma seção quando for especificado.
