# KalangoOS · requisitos de hardware e metas de desempenho

## Hardware

| | Mínimo | Recomendado | Para modelos locais grandes |
|---|---|---|---|
| Processador | 64 bits, 4 núcleos (x86-64-v2) | 8 núcleos | 8 núcleos ou mais |
| Memória | 8 GB | 16 GB | 32 GB |
| Armazenamento | 64 GB SSD | 256 GB NVMe | 512 GB NVMe |
| Vídeo | Intel, AMD ou NVIDIA com Vulkan | GPU dedicada | NVIDIA ou AMD com 16 GB ou mais de memória |
| Firmware | UEFI | UEFI com Secure Boot e TPM 2.0 | — |

Abaixo do recomendado, o sistema usa um modelo local pequeno e oferece o modo híbrido (nuvem com permissão).

## Placas de vídeo

| Fabricante | Driver | Estado |
|---|---|---|
| NVIDIA | Proprietário, série 560 ou mais nova (GBM e explicit sync) | Requisito. Prova de conceito rodou no niri com RTX 3090 / driver 580 em janela; sessão real a testar |
| AMD | Mesa (radeonsi, RADV) | A testar |
| Intel | Mesa (iris, ANV) | A testar |

A matriz completa de testes é preenchida a cada versão (tarefa da fase 2).

## Metas de desempenho

Medidas em hardware de referência, a cada versão, na CI (fase 6).

| Medida | Meta |
|---|---|
| Ligar até a área de trabalho (SSD NVMe, após login) | ≤ 3 s |
| Memória da sessão em repouso, sem apps abertos | ≤ 900 MB |
| CPU do shell parado por 1 min | 0% contínuo |
| Animações do shell e das janelas | 60 qps sem quadros perdidos, inclusive em 4K com NVIDIA |
| Abrir um app próprio | ≤ 300 ms |
| Busca da barra de comando (10 mil itens) | ≤ 100 ms |

## O que já foi medido

| Medida | Resultado | Onde |
|---|---|---|
| Mola suave da barra lateral | 96% do caminho em 207 ms, assenta em ~600 ms | `poc/capturas/mola.txt` |
| Quadros por segundo da prova (desenho por software) | 60 | idem |
| Prova dentro do niri, em janela, com NVIDIA | ~50 qps | `poc/capturas/mola-niri.txt` |
| Memória e CPU da prova | Não confiável ainda | `poc/measure.sh` (a corrigir) |
