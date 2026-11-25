# KalangoOS 0.1.1 — correção do Remote e da aprovação gráfica

**Português** · [English](#english)

Esta versão corrige o que encontramos ao testar a ISO da 0.1.0 em máquinas virtuais (duas VMs, uma controlando a
outra) e endurece o Kalango Remote. Continua sendo uma prévia técnica para desenvolvedores e testadores, **testada só
em máquina virtual**: não instale num computador com dados que você não pode perder.

**O `kalangod` e o Kalango Remote desta versão saem juntos e precisam ser usados juntos.** O `kalangod` novo recusa o
app e o núcleo do Remote da 0.1.0 (eles podem ser depurados). Quem mistura as versões vê o acesso recusado.

## O que foi corrigido

- **A tela de aprovação gráfica funciona na imagem instalada.** Na 0.1.0 ela era recusada ("a aprovação precisa ser
  feita por você: num terminal…"), assim como o teclado na tela. As listas do `kalangod` apontavam para scripts em vez
  dos programas que o kernel reporta.
- **O Kalango Remote mostra a tela do computador controlado na imagem instalada.** O app instalado não estava na lista
  do `kalangod` e o computador compartilhado nunca mostrava a tela.
- **O mouse de quem controla chega ao outro computador** (na 0.1.0 só o teclado chegava).
- **O código do cartão acha o computador na mesma rede** também no modo padrão e com duas placas de rede. No modo
  túnel do Cloudflare isso agora é uma escolha: o ajuste **"Aceitar também conexões na mesma rede"** (desligado por
  padrão) faz o app escutar também na rede local enquanto compartilha. Sem ele, quem digita o código ouve o que precisa
  ser ligado. Só entram endereços locais (loopback, faixas privadas e link-local); Tailscale e outros 100.64.0.0/10 não
  contam como rede local. O ajuste vale a partir do próximo compartilhamento.
- **Faixa "está controlando" só quando há controle:** se o sistema recusa a sessão, o computador compartilhado não
  mostra a faixa, fecha a conexão e o aviso diz o motivo (o aviso longo quebra a linha dentro da janela).
- Três ajustes do Remote que ainda não funcionam ("Aceitar só os aparelhos desta lista", "Endereços permitidos" e
  "Aceitar conexão por endereço IP") aparecem **desativados**, com "Ainda não funciona nesta versão".
- O script do Remote tira `LD_PRELOAD` e `LD_AUDIT` do ambiente e o shell não passa o `LD_PRELOAD` dele aos apps que
  abre; `is_engine` (marcas de confiança do d1) compara o caminho completo do motor.

## Endurecimento do Remote

O Remote só recebe captura, entrada e cobertura de privacidade se o app e o núcleo não puderem ser depurados nem lidos
por outros processos da pessoa: `prctl(PR_SET_DUMPABLE, 0)` logo no começo de cada um, sem depurador, sem biblioteca ou
plugin estranho no ambiente, e sem código executável mapeado de fora das pastas do sistema. O `kalangod` confia no que
o kernel diz do processo (mesmos namespaces, mesmo arquivo, cada mapeamento), repete a conferência a cada segundo e, se
o núcleo deixar de ser seguro, retira a permissão em até 1 s e o encerra, registrando na Linha do tempo. Limites
conhecidos em [docs/SEGURANCA.md](https://github.com/kalangoos/kalangoos/blob/main/docs/SEGURANCA.md).

Também entra o **kit de validação em hardware** (`imagem/validar-hardware.sh`,
[docs/VALIDACAO-HARDWARE.md](https://github.com/kalangoos/kalangoos/blob/main/docs/VALIDACAO-HARDWARE.md) e o
formulário "Teste em hardware real"), e as evidências das VMs em
[docs/evidencias-vm](https://github.com/kalangoos/kalangoos/tree/main/docs/evidencias-vm).

A lista completa está no [CHANGELOG.md](https://github.com/kalangoos/kalangoos/blob/main/CHANGELOG.md).

## Como instalar e verificar

Baixe da [página de versões](https://github.com/kalangoos/kalangoos/releases) a ISO `kalangoos-0.1.1-amd64.iso`, a soma
`SHA256SUMS-iso.txt` e, se quiser, `kalangoos-0.1.1-linux-x86_64.tar.gz` com `SHA256SUMS.txt`.

1. **Confira a soma:** `sha256sum -c SHA256SUMS-iso.txt`
   (ISO: `7edab7cf27dc8b657d50c18f839e5c9d90c6d62fbe7a5ff963c81335182ee520`; tarball: `10c9a18d17117b0cbc7a096c39c0e1ee3de6112cf328bba0063de4789e6ca0f9`).
2. **Confira a origem do build** (precisa da CLI do GitHub):
   `gh attestation verify kalangoos-0.1.1-amd64.iso --repo kalangoos/kalangoos`
3. **Teste numa VM:** `KALANGO_ISO=/caminho/kalangoos-0.1.1-amd64.iso imagem/rodar-iso.sh novo`
   (veja os detalhes nas [notas da 0.1.0](NOTAS-0.1.0.md)).
4. **Modelos de IA:** a ISO não leva modelo local; escolha a fonte na etapa "Modelos de IA" do instalador ou depois em
   Ajustes e na Central de Apps.

## Problemas conhecidos

- **O código do cartão na mesma rede, no modo túnel, só funciona com o ajuste "Aceitar também conexões na mesma
  rede" ligado** por quem compartilha (desligado por padrão). Sem ele, quem digita o código ouve o que precisa ser ligado.
- Os ajustes "Aceitar só os aparelhos desta lista", "Endereços permitidos" e "Aceitar conexão por endereço IP" ainda não
  funcionam: aparecem desativados, com "Ainda não funciona nesta versão".
- Possível e não reproduzido: iniciar o compartilhamento duas vezes muito rápido pode deixar a mensagem `Exited` do núcleo
  antigo derrubar o núcleo novo (`on_core(Msg::Exited)` pega `self.core` sem conferir qual núcleo saiu). Fica para uma
  correção seguinte; não mudou na 0.1.1.
- Quem controla vê "A pessoa negou o acesso" mesmo quando quem recusou foi o sistema do computador controlado (a
  mensagem certa aparece só no computador controlado).
- Com o app e o núcleo da 0.1.0 e o `kalangod` novo, o acesso é recusado e quem controla fica em "Conectando pelo
  túnel" (o app da 0.1.0 não fecha a conexão recusada). Por isso o `kalangod` e o Remote vão juntos.
- Áudio da sessão remota com o núcleo não depurável: não verificado (as VMs de teste não têm placa de som).
- Janela entre o `exec` e o `prctl` do app e do núcleo, e identificação do cliente por pid no niri: limites
  documentados, correção planejada.
- Continua valendo o que as [notas da 0.1.0](NOTAS-0.1.0.md)
  dizem: testada só em VM, sem modelo local na ISO, VirtualDesk e Fluxos fora da ISO, Remote ainda não testado entre dois
  computadores físicos, Mosh sem teste.

## Segurança e licença

Falhas de segurança: **Security › Report a vulnerability** neste repositório
([SECURITY.md](https://github.com/kalangoos/kalangoos/blob/main/SECURITY.md)). Licenças como na 0.1.0: código GPL-3.0 ou
posterior; Remote derivado do RustDesk (AGPL-3.0); detalhes em
[LICENCAS.md](https://github.com/kalangoos/kalangoos/blob/main/docs/LICENCAS.md).

---

## English

**KalangoOS 0.1.1** fixes what we found testing the 0.1.0 ISO in virtual machines and hardens Kalango Remote. Still a
technical preview, tested in VMs only.

**This `kalangod` and this Kalango Remote ship together and must be used together**: the new `kalangod` refuses the
0.1.0 Remote app and core.

Fixed: graphical approval and the on-screen keyboard on the installed image; the Remote screen on the installed image;
the controller's mouse now reaches the other computer; the LAN code works in the default mode and, in tunnel mode, with
the new opt-in "Aceitar também conexões na mesma rede" (off by default); no "is controlling" banner when the system
refuses the session; three Remote settings that do nothing yet are disabled ("Ainda não funciona nesta versão").

Hardening: the Remote app and core are not debuggable, `kalangod` verifies them through kernel facts every second and
revokes access within 1 s. A hardware validation kit is included.

Known issues: the LAN code in tunnel mode needs the opt-in; three Remote settings are disabled; a very fast double start of
sharing might let the old core's exit tear down the new one (unreproduced, follow-up); the controlling side sees "the person
denied access" even when the system refused; audio with the hardened core is unverified; the other 0.1.0 limitations apply.

Verify: `sha256sum -c SHA256SUMS-iso.txt`, `gh attestation verify kalangoos-0.1.1-amd64.iso --repo kalangoos/kalangoos`.
