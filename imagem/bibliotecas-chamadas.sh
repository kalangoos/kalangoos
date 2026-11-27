#!/bin/bash
# Bibliotecas que um programa carrega: as do cabeçalho (NEEDED) e as que ele abre sozinho, por nome (o niri abre a
# libwayland-server assim). Usado por imagem/construir.sh (a conferência de que toda biblioteca está na imagem) e por
# imagem/testar-bibliotecas.sh.
#
# As bibliotecas que o próprio programa traz EMBUTIDAS e grava ao iniciar (o `libkalango_netguard.so` do VirtualDesk,
# que o `build.rs` dele compila e põe dentro do binário) aparecem em `strings` mas não são do sistema: não entram.
bibliotecas_chamadas() {
  readelf -d "$1" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'
  strings "$1" | grep -E '^lib[a-z0-9_+-]+\.so(\.[0-9]+)*$' | grep -v -E '^libkalango_[a-z0-9_]+\.so$' || true
}
