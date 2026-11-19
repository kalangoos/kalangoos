#!/bin/sh
# "Navegador" de teste: abre o endereço de autorização e segue o redirecionamento até a porta local do kalangod,
# como se a pessoa tivesse clicado em "autorizar".
exec curl -s -L -o /dev/null "$1"
