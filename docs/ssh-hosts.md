# Servidores SSH no Terminal (`hosts.json` do Kalango Remote)

O esquema do arquivo é o do Kalango Remote, dono dos servidores: `docs/HOSTS.md` no repositório
`kalango-remote`. Este documento diz como o Terminal do KalangoOS o lê e o usa.

```sh
kalango-shell terminal-ssh app-prod-01              # abre (ou traz para a frente) a aba do servidor
kalango-shell terminal-ssh app-prod-01 --arquivos   # a mesma aba, com o painel de arquivos (SFTP) aberto
```

O arquivo é `$XDG_CONFIG_HOME/kalango/remote/hosts.json` (normalmente `~/.config/kalango/remote/hosts.json`); a
variável `KALANGO_SSH_HOSTS` aponta para outro (testes). O Terminal **só lê**, e ignora os campos que não conhece.

## O comando da aba

1. Com o `kalango-remote` no PATH, o Terminal pede o comando a ele: `kalango-remote ssh-command <id>` responde um
   array JSON (`ssh …` ou `mosh --ssh=… …`) que roda como está, sem shell, na VTE da aba. O Terminal só acrescenta o
   soquete de controle (`ControlMaster`/`ControlPath`/`ControlPersist`), para o SFTP, a segunda metade da aba
   dividida e a latência usarem a mesma conexão.
   - Saída 2 (id que ele não conhece): o Terminal monta o comando sozinho (abaixo).
   - Saída 1: a causa aparece na aba ("Kalango Remote could not prepare …"), com Reconectar.
   - As chaves do cofre que ele escreve em `$XDG_RUNTIME_DIR/kalango-remote/keys/` (o `-i` do comando) são apagadas
     pelo Terminal quando a conexão termina, e pedidas de novo a cada reconexão. Nenhum arquivo fora dessa pasta é
     apagado.
   - O comando dele confere a chave do servidor com rigor (`StrictHostKeyChecking=yes` e o `known_hosts` dele): uma
     chave desconhecida vira o aviso "Kalango Remote does not know the key…" com o botão que abre
     `kalango-remote --page ssh`; uma chave mudada vira o diálogo coral "A impressão digital MUDOU", cujo
     "Abrir servidores conhecidos" abre a mesma página.
2. Sem o Kalango Remote, o Terminal monta `ssh`/`mosh` com o que o arquivo diz: `-p`, `-i` (só quando a identidade é
   um caminho de arquivo; uma identidade do cofre só o Kalango Remote entrega, e uma do agente usa o agente), `-J`
   com os saltos (ids de outros servidores, com os saltos deles antes), `ServerAliveInterval`, `-A`, os
   encaminhamentos, `StrictHostKeyChecking=ask`. A pergunta do ssh sobre uma chave desconhecida aparece no diálogo
   "Primeira conexão" e a resposta vai para o ssh.

## O que o Terminal lê do arquivo

| Onde | Campo | Uso no Terminal |
|---|---|---|
| `hosts[]` | `id`, `name` | O id de `terminal-ssh`; o nome é o título da aba (sem `id`, vale o `name`, e vice-versa) |
| | `address`, `port` | Destino (porta 22 por padrão; número ou texto) |
| | `type` | `"ssh"` ou `"mosh"`: o selo "mosh"; sem `mosh` aqui ou `mosh-server` lá, a aba volta para SSH com um aviso |
| | `group` | Id (ou nome) do grupo |
| | `user`, `identity`, `jump`, `keepalive`, `forward_agent` | `null` herda do grupo e dos grupos acima |
| | `tags` | `produção` (`producao`, `production`, `prod`): faixa coral e selo "produção" |
| | `confirm` | Comandos que apagam ou reiniciam pedem confirmação antes do Enter (como em produção) |
| `groups[]` | `id`, `name`, `parent`, `defaults` | Herança; os mesmos campos escritos direto no grupo também valem |
| `identities[]` | `id`, `kind` | `agent` e `vault-key`: sem `-i` no comando montado aqui |
| `forwards[]` | `kind`, `listen`, `target`, `host`, `with_host`, `loopback_only` | Os que têm `with_host` vão no comando da aba do `host` |

Além do esquema do Kalango Remote, o leitor aceita (para testes e servidores escritos à mão): o arquivo ser só a
lista de servidores, a lista em `"servers"`, `identity_file` (caminho da chave), `auth` (`"agent"`), `jump` como
lista ou texto com vírgulas e `usuario@endereco:porta`, `production`, `forwards` dentro do servidor, `options`
(`-o Chave=Valor`), `config_file` (`-F`) e `mosh_server` (`mosh --server=`).

## Segurança

- O Terminal nunca guarda nem pede senhas: o `ssh` pergunta no próprio terminal ou usa o agente; as chaves do cofre
  passam só pelo arquivo temporário do Kalango Remote, apagado ao fim da conexão.
- O comando do Kalango Remote só é aceito se começar por `ssh` ou `mosh`. Do arquivo, endereços, usuários e saltos que
  começam com `-`, têm espaços, aspas ou caracteres de shell, e nomes de opção que não são palavras simples são
  recusados (um `hosts.json` não vira outro comando).
- O agente nunca aceita uma chave de servidor: não há ação para isso.
- O soquete de controle fica em `$XDG_RUNTIME_DIR/kalango/ssh/` e fecha com a última aba do servidor (ou 10 minutos
  depois).
