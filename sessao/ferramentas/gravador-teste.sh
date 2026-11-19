#!/bin/bash
# KalangoOS · gravador de teste (a sessão de teste não tem microfone): "grava" no caminho recebido a fala
# de $KALANGO_TESTE_FALA e espera ser parado (SIGINT, como o pw-record).
cp "$KALANGO_TESTE_FALA" "$1"
trap 'exit 0' INT TERM
while :; do sleep 0.1; done
