#!/bin/sh
# Gera a configuração do niri para a imagem do KalangoOS (caminhos instalados), a partir de niri.kdl.modelo.
#   sessao/gerar.sh > /usr/share/kalango/niri.kdl
cd "$(dirname "$0")"
sed -e '/layout "@TECLADO@"/d' \
    -e 's#@PAPEL_DE_PAREDE@#/usr/share/kalango/papeis-de-parede/padrao.png#' \
    -e 's#@SHELL@#/usr/libexec/kalango/kalango-shell#g' \
    -e 's#@TERMINAL@#x-terminal-emulator#' \
    -e 's#@CURSOR_TEMA@#Adwaita#' -e 's#@CURSOR_TAMANHO@#24#' -e 's#@CONFIG@#~/.config#g' \
    -e 's#@INICIO@#// o shell e a tela de aprovação sobem como serviços do usuário (kalango-shell.service, kalango-aprovacao.service)#' \
    niri.kdl.modelo
