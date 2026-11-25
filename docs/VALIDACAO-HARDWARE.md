# KalangoOS · validação em hardware de verdade

A versão 0.1.0 foi testada só em máquinas virtuais. Este guia é para quem quer ajudar a fechar essa lacuna num
computador real, **sem instalar nada**: tudo roda na sessão ao vivo, que não toca o disco.

> **English summary.** KalangoOS 0.1.0 has only been tested in VMs. To help: download the ISO, verify its SHA-256,
> write it to a USB stick, boot the live session, run `imagem/validar-hardware.sh` (no root needed, nothing is written
> to disks, personal data is replaced by placeholders), try the manual checks below, and open an issue with the
> "Teste em hardware real" template pasting `resumo.md`. The table at the end is what the maintainer fills in.

## 1. Baixar e conferir a imagem

> **A imagem 0.1.0 tem defeitos conhecidos** que aparecem em hardware de verdade também (achados em VMs, veja o
> `CHANGELOG.md` da 0.1.1): a tela de aprovação gráfica é recusada, o teclado na tela não funciona e o Kalango Remote
> abre a sessão mas o computador controlado não mostra a tela. Nada disso é do seu computador. Quando a **0.1.1** sair,
> teste com ela (o comando abaixo muda só o nome da versão); até lá, marque esses itens como "falha conhecida" e teste
> o resto.

```sh
gh release download v0.1.0 -R kalangoos/kalangoos -p 'kalangoos-0.1.0-amd64.iso' -p 'SHA256SUMS-iso.txt'
sha256sum -c SHA256SUMS-iso.txt        # tem de dizer "OK"
gh attestation verify kalangoos-0.1.0-amd64.iso -R kalangoos/kalangoos    # a procedência: foi gerada pelo repositório
```

(Sem o `gh`: baixe os dois arquivos na página da versão e confira o `sha256sum` à mão; a atestação só se confere com o `gh`.)

## 2. Fazer o pendrive (a imagem tem 738 MB: qualquer pendrive de 2 GB ou mais; **apaga o pendrive**)

Antes de gravar, descubra qual é o pendrive e **confira três vezes**: gravar no dispositivo errado apaga um disco seu.

- **Linux, `dd`:** `lsblk` mostra os dispositivos; desmonte o que estiver montado (`sudo umount /dev/sdX*`) e grave:
  `sudo dd if=kalangoos-0.1.0-amd64.iso of=/dev/sdX bs=4M status=progress conv=fsync`
- **macOS, `dd`:** `diskutil list` acha o disco (`diskN`); `diskutil unmountDisk /dev/diskN`; grave no dispositivo cru:
  `sudo dd if=kalangoos-0.1.0-amd64.iso of=/dev/rdiskN bs=4m` (o `dd` do macOS não tem `status=progress`; aperte Ctrl+T
  para ver o andamento); `diskutil eject /dev/diskN` no fim.
- **Ventoy:** copie o `.iso` para a partição do Ventoy. Funciona na maioria dos computadores; se a sessão não subir,
  grave com `dd` ou Rufus antes de relatar um defeito.
- **Rufus (Windows):** escolha o `.iso`, esquema de partição **GPT**, destino **UEFI**, e **modo imagem DD** quando perguntar.
- O KalangoOS pede **UEFI**. Se o Secure Boot impedir o boot, diga isso no relatório (não precisa desligá-lo para testar,
  só se não ligar).

## 3. Ligar e rodar o script

1. Ligue pelo pendrive (tecla de boot: F12, F10, Esc ou F2, conforme o fabricante) e escolha a sessão ao vivo.
2. Pegue o script **de uma versão fixa, não de `main`**, e confira o hash antes de rodar (ele roda na sua máquina):
   ```sh
   curl -LO https://raw.githubusercontent.com/kalangoos/kalangoos/TAG/imagem/validar-hardware.sh
   echo "SHA256_DO_SCRIPT  validar-hardware.sh" | sha256sum -c -
   ```
   `TAG` e `SHA256_DO_SCRIPT` vêm na descrição da versão em que o script foi publicado (quem mantém os preenche ali; a
   imagem 0.1.0 não traz o script). Quem tem o repositório pode ver o hash com `sha256sum imagem/validar-hardware.sh`.
   Depois rode, num Terminal:
   ```sh
   bash validar-hardware.sh
   ```
   Ele pergunta antes de **suspender** (padrão: não). `--suspender` faz sem perguntar; `--sem-suspender` nunca faz.
3. Ele grava uma pasta `kalango-validacao-DATA-HASH` com `resumo.md` e um arquivo por área. O que ele faz: lista PCI/USB,
   placa de vídeo e driver em uso, avisos do kernel e firmware que faltou, se o niri, o shell e o kalangod subiram,
   áudio, rede e Bluetooth, telas e modos, **uma simulação do instalador que não grava em disco nenhum**, o tempo
   até o shell subir e a memória em repouso.
4. O que precisa de root (mensagens do kernel) usa `sudo -n`, que a sessão ao vivo permite; sem isso o relatório diz "sem acesso".
5. **Dados pessoais:** MACs, IPs (v4 e v6), números de série, UUIDs, nomes de contas e do computador, nomes de aparelhos
   e redes Wi-Fi são trocados por marcadores antes de gravar, e no fim o script procura o que sobrou. Mesmo assim,
   **leia a pasta antes de enviar**: o filtro pode errar.
6. A sessão ao vivo some ao desligar: copie a pasta para um pendrive de dados.

## 4. O que testar à mão

Marque no resumo o que funcionou:

- **Entrar:** a sessão ao vivo abre sem pedir senha; abra o Terminal e o Arquivos.
- **Wi-Fi:** conecte a uma rede pela barra lateral. (O script só diz se há adaptador.)
- **Som:** toque um áudio e ajuste o volume; teste fone e alto-falante.
- **Monitor externo:** plugue por HDMI/DisplayPort/USB-C; a tela nova aparece, muda de modo, e some ao desplugar.
- **Remote entre dois computadores reais** (na 0.1.0 a tela do computador controlado não aparece: veja o aviso acima; use a 0.1.1): abra o Kalango Remote nos dois (na mesma rede e, se der, em redes diferentes),
  compartilhe num e conecte do outro pelo código; aprove o pedido; veja a tela, mova o mouse, digite, use o chat.
  Conte também o tempo de conexão e se o vídeo engasga.
- **Suspender:** pelo menu de energia (ou o script com `--suspender`); o computador volta com a sessão de pé?
- **Mosh/SSH** (opcional): conecte a outro computador com o Terminal. A imagem 0.1.0 traz o cliente (`ssh`, `mosh`) mas
  não o servidor (`openssh-server`): para ser o computador acessado, instale-o antes (`sudo apt install openssh-server`).

## 5. Enviar

Abra uma issue com o formulário **Teste em hardware real**
(<https://github.com/kalangoos/kalangoos/issues/new?template=hardware.yml>) e cole o `resumo.md`.

## 6. Matriz (a pessoa mantenedora preenche)

| Vídeo | Modelo testado | Sessão (niri) | Telas | Som | Wi-Fi | Suspender | Remote | Relatório |
|---|---|---|---|---|---|---|---|---|
| NVIDIA | — | não testado | não testado | não testado | não testado | não testado | não testado | — |
| AMD | — | não testado | não testado | não testado | não testado | não testado | não testado | — |
| Intel | — | não testado | não testado | não testado | não testado | não testado | não testado | — |
| Híbrido (Intel + NVIDIA) | — | não testado | não testado | não testado | não testado | não testado | não testado | — |

Máquinas virtuais não entram na matriz: o script avisa quando roda numa.
