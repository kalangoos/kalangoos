#!/bin/sh
# KalangoOS · prova de conceito
# Baixa SEM sudo o que falta para rodar a prova (compositor sway, captura grim, gtk4-layer-shell) e extrai em poc/.deps/root.
# Nada é instalado no sistema: apague poc/.deps para desfazer.
# Alternativa "de verdade":  sudo apt install sway grim libgtk4-layer-shell-dev
set -e
cd "$(dirname "$0")"
mkdir -p .deps/debs .deps/root
# (KALANGO_DEPS: only these packages, e.g. one added later; the Terminal's emulator is VTE for GTK 4)
# (glslc and spirv-headers: the Vulkan shaders of the llama.cpp in the image — local models on any GPU)
todo="${KALANGO_DEPS:-sway grim libgtk4-layer-shell-dev fonts-inter wlsunset libvte-2.91-gtk4-dev glslc spirv-headers}"; seen=""; need=""
while [ -n "$todo" ]; do
  set -- $todo; p=$1; shift; todo="$*"
  case " $seen " in *" $p "*) continue ;; esac
  seen="$seen $p"
  dpkg -s "$p" >/dev/null 2>&1 && continue
  LC_ALL=C apt-cache show "$p" >/dev/null 2>&1 || continue          # pacote virtual
  need="$need $p"
  for d in $(LC_ALL=C apt-cache depends --no-recommends --no-suggests --no-conflicts --no-breaks --no-replaces --no-enhances "$p" | awk '/Depends:/{print $2}' | grep -v '^<' || true); do
    todo="$todo $d"
  done
done
echo "baixando:$need"
(cd .deps/debs && apt-get download $need)
for f in .deps/debs/*.deb; do dpkg -x "$f" .deps/root; done
ROOT="$(pwd)/.deps/root"
# a -dev package whose library is already installed on the system (not downloaded): its link points there
for so in "$ROOT"/usr/lib/x86_64-linux-gnu/*.so; do
  [ -L "$so" ] && [ ! -e "$so" ] || continue
  alvo=$(readlink "$so")
  [ -e "/usr/lib/x86_64-linux-gnu/$alvo" ] && ln -sf "/usr/lib/x86_64-linux-gnu/$alvo" "$ROOT/usr/lib/x86_64-linux-gnu/$alvo"
done
# os arquivos .pc apontam para /usr: aponta para a pasta local
for pc in "$ROOT"/usr/lib/x86_64-linux-gnu/pkgconfig/*.pc; do [ -f "$pc" ] && sed -i "s#^prefix=/usr#prefix=$ROOT/usr#" "$pc"; done
# fontconfig da sessão de teste: as fontes do sistema mais as baixadas aqui (Inter, do Design System)
printf '<?xml version="1.0"?>\n<!DOCTYPE fontconfig SYSTEM "fonts.dtd">\n<fontconfig><include ignore_missing="yes">/etc/fonts/fonts.conf</include><dir>%s/usr/share/fonts</dir></fontconfig>\n' "$ROOT" > "$ROOT/fonts.conf"
echo "pronto: $ROOT"

# gtk4-layer-shell não tem pacote no Ubuntu 24.04: compila do código-fonte para a pasta local
if [ ! -f "$ROOT/usr/lib/x86_64-linux-gnu/pkgconfig/gtk4-layer-shell-0.pc" ]; then
  [ -x .deps/venv/bin/meson ] || { python3 -m venv .deps/venv && .deps/venv/bin/pip install -q meson; }
  TAG=$(git ls-remote --tags --refs https://github.com/wmww/gtk4-layer-shell 'v1.*' | awk -F/ '{print $3}' | sort -V | tail -1)
  rm -rf .deps/src && git clone -q --depth 1 --branch "$TAG" https://github.com/wmww/gtk4-layer-shell .deps/src
  .deps/venv/bin/meson setup .deps/src/build .deps/src --prefix="$ROOT/usr" --libdir=lib/x86_64-linux-gnu \
      -Dintrospection=false -Dvapi=false -Dtests=false -Ddocs=false -Dexamples=false >/dev/null
  ninja -C .deps/src/build install >/dev/null
  echo "gtk4-layer-shell $TAG compilado"
fi

# Moli (github.com/lexmount/moli): the browser engine for pages that show their content only with scripts (the agents'
# web reading and search, held by kalangod: filtering proxy, Landlock, cgroup). A fixed release, unpacked here; kalangod
# finds it next to itself (target/release/moli, a link), in the other profiles' folders or poc/.deps/moli (a session built
# with the "sessao" profile), or in /usr/lib/kalango/moli on the installed system.
MOLI_VERSAO=v1.1.26
if [ ! -x .deps/moli/moli ] || [ "$(cat .deps/moli/VERSION 2>/dev/null)" != "${MOLI_VERSAO#v}" ]; then
  rm -rf .deps/moli .deps/moli-dl && mkdir -p .deps/moli-dl
  curl -sSL --max-time 600 -o .deps/moli-dl/moli.tar.gz "https://github.com/lexmount/moli/releases/download/$MOLI_VERSAO/moli-x86_64-unknown-linux-gnu.tar.gz"
  tar xzf .deps/moli-dl/moli.tar.gz -C .deps/moli-dl
  mv ".deps/moli-dl/moli-$MOLI_VERSAO-x86_64-unknown-linux-gnu" .deps/moli && rm -rf .deps/moli-dl
  echo "moli $MOLI_VERSAO baixado"
fi
mkdir -p ../target/release && ln -sfn ../../poc/.deps/moli/moli ../target/release/moli
