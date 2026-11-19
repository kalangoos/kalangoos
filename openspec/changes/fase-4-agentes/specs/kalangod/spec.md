# Spec Delta

## Purpose

Serviço do sistema que permite aos agentes nativos operar todo o sistema por MCP, com níveis de poder, aprovação e registro.

## ADDED Requirements

### Requirement: Somente local
O kalangod MUST aceitar conexões apenas pelo soquete local do sistema e MUST NOT abrir portas de rede.

#### Scenario: Varredura de portas
- **WHEN** a máquina é varrida pela rede
- **THEN** nenhuma porta do kalangod aparece

### Requirement: Identidade pelo kernel
O kalangod MUST identificar quem chama pelas credenciais do soquete e pelo cgroup, e MUST NOT confiar em nomes informados pelo chamador.

#### Scenario: Chamador falso
- **WHEN** um processo comum se apresenta como agente nativo
- **THEN** a chamada é recusada

### Requirement: Níveis de poder
Cada chamada MUST pertencer a um nível: observar, usuário, sistema ou total; o nível concedido MUST limitar as ferramentas disponíveis.

#### Scenario: Nível usuário
- **WHEN** um agente no nível usuário pede para instalar um pacote
- **THEN** a ferramenta não está disponível e o pedido vira solicitação ao usuário

### Requirement: Nível sistema
Ações do nível sistema MUST pedir aprovação ao usuário e criar um ponto de restauração antes de executar.

#### Scenario: Instalar pacote
- **WHEN** um agente pede para instalar um pacote
- **THEN** o usuário aprova e um ponto de restauração é criado antes

### Requirement: Nível total
O nível total MUST exigir aprovação presencial a cada sessão, MUST expirar em 15 minutos e MUST NOT ser concedido a rotinas sem o usuário presente.

#### Scenario: Expiração
- **WHEN** passaram 15 minutos da concessão
- **THEN** o agente volta ao nível anterior

#### Scenario: Rotina noturna
- **WHEN** uma rotina agendada pede nível total
- **THEN** o pedido é negado e registrado

### Requirement: Contrato das ferramentas
Cada ferramenta MUST declarar efeito (leitura, escreve, destrutiva), se é reversível e como desfazer.

#### Scenario: Ferramenta irreversível
- **WHEN** uma ferramenta não tem inverso
- **THEN** ela sempre pergunta antes, em qualquer nível

### Requirement: Regras do kernel para agentes
Nenhum processo descendente do cgroup dos agentes MUST abrir arquivos dentro de uma pasta protegida, em nenhum nível,
inclusive como root; a regra MUST ser imposta por eBPF LSM e cada bloqueio, execução e escrita de agente MUST virar
registro da linha do tempo.

#### Scenario: Agente root e /etc/shadow
- **WHEN** um agente no nível total executa `head /etc/shadow`
- **THEN** o kernel nega com EPERM e a linha do tempo registra o bloqueio

#### Scenario: Kernel sem eBPF LSM
- **WHEN** `bpf` não está na lista `lsm=`
- **THEN** o kalangod funciona com Landlock e as próprias verificações e `kernel.seguranca` informa a guarda desligada e o motivo

### Requirement: Tarefas limitadas
Todo programa executado para um agente MUST rodar num cgroup próprio com limites de memória, CPU, processos e tempo,
com o UID de quem pediu (exceto no nível total) e com escrita restrita às pastas permitidas.

#### Scenario: Estouro de memória
- **WHEN** uma tarefa passa do limite de memória
- **THEN** só ela é encerrada e a resposta informa `estourou_memoria`

#### Scenario: Limite maior que o permitido
- **WHEN** o agente pede mais memória que a configuração permite
- **THEN** vale o limite da configuração

### Requirement: Pontos de restauração
Toda ação do nível sistema MUST ser precedida de um ponto de restauração (snapshot btrfs, ou cópia do /etc, mais a
lista de pacotes); sem ponto, a ação MUST NOT acontecer. Restaurar MUST criar antes um ponto novo.

#### Scenario: Restaurar e desfazer
- **WHEN** o usuário restaura um ponto e depois desfaz a restauração
- **THEN** o /etc e os pacotes voltam ao ponto e, em seguida, ao estado anterior à restauração

### Requirement: Aprovação presencial
Pedidos dos níveis sistema e total MUST ser aprovados por um terminal do usuário ou pela tela de aprovação do sistema;
o kalangod MUST reconhecer a tela pelo caminho do executável informado pelo kernel, e só se o arquivo e a pasta dele
não puderem ser alterados por outros usuários.

#### Scenario: Agente imita a tela
- **WHEN** um processo que não é a tela de aprovação, sem terminal, tenta aprovar
- **THEN** a aprovação é recusada e o pedido continua esperando

#### Scenario: Tecla acidental
- **WHEN** a pessoa tecla Enter ou aperta um botão no instante em que o pedido aparece
- **THEN** nada é aprovado

### Requirement: Conectores de terceiros isolados
Cada conector MCP local de terceiros MUST rodar com o UID do usuário, em cgroup limitado, com Landlock restrito ao que o
manifesto declara, sem rede ou só nas portas declaradas, com seccomp e com ambiente limpo; ferramentas bloqueadas MUST
NOT chegar ao conector e cada chamada MUST ficar registrada fora do alcance dele.

#### Scenario: Conector tenta ler ~/.ssh
- **WHEN** um conector tenta ler a chave SSH do usuário
- **THEN** o kernel nega com "Permission denied"

### Requirement: Conectores remotos e segredos
Conectores remotos MUST usar HTTPS (exceto em loopback), autorização OAuth com PKCE S256 e indicador de recurso, e MUST
NOT seguir redirecionamentos para outra origem. Tokens e segredos MUST ficar no cofre (chaveiro do sistema ou arquivo
protegido), nunca em arquivos de configuração, e só chegar ao conector que os declara.

#### Scenario: Token revogado
- **WHEN** o servidor revoga a autorização
- **THEN** a chamada falha com a instrução de autorizar de novo, sem expor o token

