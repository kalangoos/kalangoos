# Evidências de VM (ISO 0.1.0, sessão ao vivo)

Duas VMs QEMU/KVM da ISO 0.1.0 publicada (sessão ao vivo, 4 GB cada), uma rede entre elas. Os binários novos
(`kalangod`, `kalango-remote`, `kalango-remote-core`) foram copiados para dentro das VMs; o resto é a imagem publicada.

- `aprovacao-grafica-recusada-0.1.0.jpg`: na 0.1.0 como publicada, o cartão de aprovação mostra "a aprovação precisa ser
  feita por você: num terminal ou na tela de aprovação do sistema (presencial)" ao clicar em "Permitir uma vez": o
  kalangod recusava a própria tela de aprovação, porque `aprovadores_graficos` apontava para um link que resolve para o
  script de `/usr/bin`, e o programa que o kernel reporta é `kalango-aprovacao.bin`.
- `aprovacao-grafica-com-a-correcao.jpg`: o cartão com o kalangod corrigido; o clique em "Permitir uma vez" fez a ação
  (`sistema.nome`) e o nome do computador mudou.
- `teclado-na-tela-com-a-correcao.jpg`: o teclado na tela do shell. Log do niri, antes: `kalango-shell.bin Keyboard →
  false`; depois: `Keyboard → true`.
- `remote-a-ve-b-com-a-correcao.jpg` e `remote-b-controlada-e-entrada-da-a.jpg`: o Kalango Remote com o app e o núcleo
  não depuráveis e o kalangod novo: uma VM vê a outra pelo túnel do Cloudflare, o teclado da primeira chega à segunda e
  a segunda mostra a faixa "está controlando este computador".

Segunda rodada (0.1.1, sem editar o `kalangod.toml`): o Remote **aberto pela grade de apps do dock**
(`dock-grade-de-apps.jpg`), com o `LD_PRELOAD=libgtk4-layer-shell.so.0` do shell no ambiente de quem o abre, ganhou a
permissão porque o script `/usr/bin/kalango-remote` agora tira `LD_PRELOAD` e `LD_AUDIT` (`remote-do-dock-toml-padrao-a-ve-b.jpg`
e `remote-do-dock-b-controlada.jpg`). Com a sessão aberta, um processo do root se anexou (`ptrace`) ao núcleo: em
menos de 1 s o kalangod tirou a permissão, **encerrou o núcleo** (SIGKILL) e registrou na Linha do tempo ("o núcleo … perdeu a
proteção (TracerPid)"); a outra VM mostra "Conexão perdida" (`nucleo-encerrado-ao-ser-depurado.jpg`).
Não verificado nas VMs: o áudio (sem placa de som: `docs/SEGURANCA.md`, "Áudio com o núcleo não depurável").

Terceira rodada (conferências pelo kernel, no kalangod novo, na sessão ao vivo da 0.1.0 com overlayfs):
- a sessão abre normalmente com o app e o núcleo novos: os mesmos namespaces, o `exe` é o arquivo confiado (dispositivo e inode),
  cada mapeamento executável confere pelo `map_files` (`codigo-na-lan-no-modo-padrao.jpg`: a outra VM vê esta "na mesma rede · 0 ms",
  achada pelo **código** com o ajuste padrão, o que antes dava "Ninguém nesta rede…");
- o app aberto dentro de um `unshare -m` com um núcleo falso montado por cima de `/usr/lib/kalango/remote/kalango-remote-core`
  (o que um `bwrap` faz) foi recusado: "…/kalango-remote roda em outro namespace de montagem ou de usuário";
- medido: o núcleo de verdade só tem `[vdso]` e `[vsyscall]` como código executável fora de arquivos; o app tem memória anônima
  executável (compilador de shaders do Mesa), por isso só o núcleo a recusa (limite documentado em `docs/SEGURANCA.md`).

Conferência final da 0.1.1 (binários finais: `kalangod` do SDRustOS `kit-hardware` 5fd5ae0, `cargo build --release -p kalangod`;
`kalango-remote` e `kalango-remote-core` do kalango-remote `validacao-duas-maquinas` 6ac9f28, com as correções do mouse e da
faixa, compilados em release). Duas VMs novas da ISO 0.1.0 publicada (SHA-256 conferido), 3 GB cada; trocados só os três
binários e o script `/usr/bin/kalango-remote` como a imagem 0.1.1 o gera (`imagem/construir.sh`). O `kalangod.toml` é o da
imagem, igual ao `crates/kalangod/dist/kalangod.toml` (sem edição). Testes de unidade dos dois repositórios passaram antes
(`cargo test --release -p kalangod`; `-p kalango-remote -p kalango-remote-core -p kalango-remote-proto`).

| # | Conferência | Resultado | Captura |
|---|---|---|---|
| 1 | aprovação gráfica de uma ação de agente (`sistema.nome`, "Permitir uma vez") | **passou**: a ação rodou e o nome mudou | `final-01-aprovacao-grafica.jpg` |
| 2 | teclado na tela do shell | **passou**: niri `kalango-shell.bin Keyboard → true` | `final-02-teclado-na-tela.jpg` |
| 3 | Remote aberto pela grade de apps do dock na B (o shell tem `LD_PRELOAD=libgtk4-layer-shell.so.0`; o app não), a A conecta pelo link do túnel | **passou**: pedido "via Cloudflare" na B; a A vê a B ("via Cloudflare"); o **movimento** do mouse da A move o ponteiro da B, o **clique** da A no ícone do dock abre as Configurações da B e o **teclado** da A escreve na busca delas | `final-03a…03f` |
| 4 | código do cartão no modo túnel (kalango-remote 0d05f78) | **passou**: sem o ajuste, o núcleo escuta só em `127.0.0.1` (sem `--lan`) e a A recebe "Ninguém nesta rede… Quem compartilha precisa ligar "Aceitar também conexões na mesma rede" ou escolher "Só rede local direta"…"; com o ajuste ligado, o núcleo escuta em `0.0.0.0` com a lista de IPs locais e a A acha a B "na mesma rede · 41 ms" (entrada de 192.168.77.1); pelo link do túnel (entrada de 127.0.0.1) também entra | `final-04a-codigo-sem-o-ajuste-nao-acha.jpg`, `final-04b-ajuste-aceitar-na-mesma-rede.jpg`, `final-04c-com-o-ajuste-na-mesma-rede.jpg` |
| 5a | root faz `PTRACE_SEIZE` no núcleo durante uma sessão | **passou**: núcleo encerrado em 0,61 s (1,01 s numa primeira rodada), permissão retirada da lista do niri, "Conexão perdida" na A | `final-05a-nucleo-encerrado-ao-ser-depurado.jpg` |
| 5b | app com `LD_PRELOAD=…libz.so.1` | **recusado**: a B avisa "Acesso recusado pelo sistema: …(LD_PRELOAD=…)", **sem** a faixa "está controlando"; a conexão é fechada e a A vê "A pessoa negou o acesso" | `final-05b-ld-preload-recusado-na-b.jpg`, `final-05b-ld-preload-a-desconectada.jpg` |
| 5c | app sob `ptrace` (TRACEME, mesmo usuário) | **recusado** ("… (TracerPid)"), igual a 5b | `final-05c-traceme-recusado-na-b.jpg` |
| 5d | app e núcleo num `unshare -m` com um núcleo falso montado por cima do de verdade | **recusado** ("…roda em outro namespace de montagem ou de usuário"), igual a 5b | `final-05d-namespace-recusado-na-b.jpg` |
| 6 | app e núcleo da 0.1.0 com o kalangod novo | **recusado** (incompatibilidade documentada: "…(dumpable)…"); a A fica em "Conectando pelo túnel" (o app da 0.1.0 ainda não fecha a conexão recusada) | `final-06-remote-0.1.0-recusado.jpg` |

Corrigido nesta rodada (kalango-remote b2ee182, 1bec4b0, 6ac9f28):
- o mouse de quem controla não chegava ao outro computador (investigação abaixo);
- com a permissão recusada pelo kalangod, a B mostrava a faixa "… está controlando este computador" e a A esperava sem imagem:
  agora a conexão é fechada e a B diz por quê;
- esse aviso, longo, passava das bordas da janela: agora quebra a linha;
- (revisão de segurança) o modo túnel não abre mais porta na rede local por padrão; o código na mesma rede pede o ajuste
  "Aceitar também conexões na mesma rede", e com ele só entram endereços locais (linha 4, rodada só para ela, kalango-remote
  0d05f78). Não verificado nas VMs: uma entrada de um endereço público sendo barrada (as duas VMs só têm endereços privados);
  o teste de unidade da lista de faixas cobre o que vai para o núcleo;
- (revisão seguinte, kalango-remote: ver CHANGELOG) a lista de endereços locais segue o jeito como o compartilhamento
  **começou**: desligar o ajuste no meio dele não a limpa mais enquanto o núcleo escuta na rede (teste de unidade
  `a_running_share_keeps_the_protection_it_started_with`); a busca pelo código (UDP) só responde a endereços locais, e o
  CGNAT/Tailscale (100.64.0.0/10) não conta como local. Também não verificado nas VMs: uma busca vinda de endereço público.
  Nas VMs (kalango-remote 155e8ad): compartilhando com o ajuste ligado (`serve --lan --port 0`), desligá-lo e mudar outro
  ajuste (o que manda um setup novo ao núcleo) deixou a lista de endereços locais no núcleo; parar e compartilhar de novo
  voltou a `serve --ask --setup-stdin` (só loopback) com a lista vazia. Legenda nova e campos desativados em
  `final-04d-ajustes-de-rede-do-remote.jpg`.
- Os campos "Aceitar só os aparelhos desta lista", "Endereços permitidos" e "Aceitar conexão por endereço IP" do Remote nunca
  foram ligados ao núcleo (já era assim na 0.1.0); agora aparecem desativados com "Ainda não funciona nesta versão", em vez de
  prometerem uma proteção que não existe.

Os binários da 0.1.0 e o kalangod novo **não** se entendem: o kalangod novo recusa o app/núcleo antigos. Os dois precisam
sair juntos na 0.1.1.

## O mouse de quem controla (investigação)

Mesmo método (duas VMs novas da ISO 0.1.0); o núcleo da B rodou com `WAYLAND_DEBUG=1` para ver o que ele pede ao niri
(`mouse-protocolo-do-nucleo.jpg`):

| Montagem | Teclado A→B | Mouse A→B |
|---|---|---|
| (a) kalangod c931c34 (só a correção da lista do Remote) + app e núcleo da 0.1.0 sem tocar | chega | **não**: o núcleo da B nunca recebe um evento de mouse (0 `motion_absolute`, 0 `button`; o ponteiro virtual só é criado quando a A digita) — `mouse-a-0.1.0-clica-na-vista.jpg`, `mouse-b-0.1.0-nada-acontece.jpg` |
| (b) kalangod a9a208f + app e núcleo endurecidos (352d499) | chega | **não**, exatamente como em (a) |
| (c) B como em (b); na A, o mesmo app com uma linha de teste (abaixo) | chega | **chega**: 58 `motion_absolute`, 2 `button`; o clique abre as Configurações da B — `mouse-b-com-o-app-corrigido-abre-configuracoes.jpg` |

Nas três o niri da B dá `Pointer → true` ao núcleo, que acha e liga o `zwlr_virtual_pointer_manager_v1`: a lista do
kalangod, o `dumpable` e as conferências do kernel não mexem no ponteiro. O defeito fica no app de **quem controla**:
a janela da sessão (`kalangoos/app/src/session.rs`, `Session::build_view`, linhas 659–663 no 352d499) põe no `GtkOverlay`, por cima da imagem, a área de
transferência de arquivos, um `gtk::Revealer` com `Align::Fill` e ainda sem filho. Fechado, ele continua do tamanho da
janela e é ele que o GTK escolhe para o ponteiro; os controladores de movimento e de clique da imagem (`Session::input`)
nunca disparam. O teclado chega porque a imagem recebe o foco com `grab_focus`.
A linha de teste foi `transfer.set_can_target(false);` logo depois de `transfer.set_valign(Align::Fill);`. A correção
(kalango-remote b2ee182) cria o revealer assim e só o deixa receber o ponteiro enquanto está aberto (`show_transfer`); a
conferência final acima (linha 3) mostra o movimento e o clique da A chegando à B.

