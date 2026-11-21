#!/bin/sh
# A picture of a test display (the nested niri window on Xvfb): capturar-tela.sh DISPLAY OUT.png [WIDTHxHEIGHT]
ffmpeg -loglevel error -y -f x11grab -video_size "${3:-1280x800}" -i ":$1+0,0" -frames:v 1 "$2"
