# Design

## Context

Base Ubuntu LTS. Restauração depende da decisão da fase 0 (btrfs ou imagem). Telas nas páginas 14 e 18 do design.

## Goals / Non-Goals

**Goals:** instalar em menos de 10 min; nenhuma atualização deixa a máquina sem iniciar.
**Non-Goals:** instalador para servidores.

## Decisions

- **Imagem gerada de forma reproduzível** a partir de uma receita no repositório (live-build ou mkosi).
- **Instalador:** adaptar o Calamares com as telas do design na 0.x; reescrever em Rust depois, se necessário.
- **Restauração:** btrfs com subvolumes e snapshots antes de cada atualização (hipótese principal).
- **Repositório próprio** apt para os pacotes KalangoOS, assinado, em kalangoos.com; Flatpak com Flathub.
- **Secure Boot:** shim e kernel assinados do Ubuntu; módulos NVIDIA assinados pelo mecanismo do Ubuntu.

## Risks / Trade-offs

- [Dual boot apaga dados] → nenhuma escrita no disco antes da confirmação final; teste automático em VM.
- [Atualização quebra o driver NVIDIA] → snapshot antes e volta automática se a sessão gráfica não subir.
