#!/bin/bash
# KalangoOS · prepara as peças da imagem que não são crates nossas, em poc/.deps, do mesmo jeito em qualquer máquina e
# na CI (sem sudo):
#   - o niri com os patches do KalangoOS (o fork kalangoos/niri, num commit fixo);
#   - o llama.cpp da imagem (num commit fixo): o modelo local na GPU pelo Vulkan (NVIDIA, AMD, Intel) ou na CPU (as
#     variantes para qualquer x86-64), backends escolhidos na hora;
#   - o resto pelo poc/setup-deps.sh (Moli, glslc e os cabeçalhos SPIR-V do Vulkan, gtk4-layer-shell…).
#
#   imagem/dependencias.sh            tudo o que falta
#   imagem/dependencias.sh niri       só o niri  ·  llama: só o llama.cpp
#
# Pacotes do sistema que a compilação usa (a CI instala; aqui, se faltar algum, o erro diz qual): cmake, clang,
# libvulkan-dev e as bibliotecas de desenvolvimento do niri (libudev, libgbm, libxkbcommon, libinput, libseat,
# libpipewire, libpango, libdisplay-info, libdbus, libsystemd).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
DEPS=$ROOT/poc/.deps
mkdir -p "$DEPS/src"
O_QUE=${1:-tudo}

# as versões: mudar aqui muda a imagem (e a chave do cache da CI)
NIRI_REPO=https://github.com/kalangoos/niri.git
NIRI_COMMIT=57a304bbd8589600d3da188e3c4c066e4eed109f
LLAMA_REPO=https://github.com/ggml-org/llama.cpp.git
LLAMA_COMMIT=b1b770d90d94cad9e9ea5b993bdbc383a522fb28
# (a interface web do llama-server, que o agente não usa mas a compilação exige: a versão publicada para esse commit)
LLAMA_UI=b12428

pegar() {  # pegar <repositório> <commit> <pasta>
  local repo=$1 commit=$2 dir=$3
  if [ ! -d "$dir/.git" ]; then
    git init -q "$dir"
    git -C "$dir" remote add origin "$repo"
  fi
  if [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" != "$(git -C "$dir" rev-parse --verify -q "$commit^{commit}" 2>/dev/null || echo x)" ]; then
    git -C "$dir" fetch -q --depth 1 origin "$commit" 2>/dev/null || git -C "$dir" fetch -q origin
    git -C "$dir" checkout -q --detach "$commit"
  fi
}

if [ "$O_QUE" = tudo ]; then
  poc/setup-deps.sh
fi

if [ "$O_QUE" = tudo ] || [ "$O_QUE" = niri ]; then
  N=$DEPS/src/niri
  # (uma pasta do niri que não é o fork — o upstream com os patches aplicados à mão, no desenvolvimento — fica como está)
  if [ -d "$N/.git" ] && ! git -C "$N" remote get-url origin | grep -q kalangoos/niri; then
    echo ">> niri: $N não é o fork kalangoos/niri; usando o que está lá"
  else
    echo ">> niri ($NIRI_COMMIT)"
    pegar "$NIRI_REPO" "$NIRI_COMMIT" "$N"
  fi
  (cd "$N" && cargo build --release --locked)
fi

if [ "$O_QUE" = tudo ] || [ "$O_QUE" = llama ]; then
  L=$DEPS/src/llama.cpp
  echo ">> llama.cpp ($LLAMA_COMMIT)"
  [ -d "$L/.git" ] || pegar "$LLAMA_REPO" "$LLAMA_COMMIT" "$L"
  R=$DEPS/root
  # os cabeçalhos SPIR-V vêm do pacote spirv-headers em poc/.deps (o setup-deps.sh): uma pasta só com eles, para não
  # misturar o resto de poc/.deps/root nos cabeçalhos do sistema
  I=$DEPS/spirv-include
  mkdir -p "$I" && ln -sfn "$R/usr/include/spirv" "$I/spirv"
  GLSLC=$(command -v glslc || echo "$R/usr/bin/glslc")
  export PATH="$R/usr/bin:$PATH" LD_LIBRARY_PATH="$R/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export HF_UI_VERSION=$LLAMA_UI
  cmake -S "$L" -B "$L/build-imagem" -DCMAKE_BUILD_TYPE=Release \
    -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_NATIVE=OFF -DGGML_CUDA=OFF -DGGML_VULKAN=ON \
    -DVulkan_GLSLC_EXECUTABLE="$GLSLC" -DCMAKE_PREFIX_PATH="$R/usr" -DCMAKE_CXX_FLAGS="-I$I" \
    -DLLAMA_CURL=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_SERVER=ON \
    -DLLAMA_BUILD_MTMD=ON >/dev/null
  cmake --build "$L/build-imagem" --target llama-server ggml-vulkan -j"$(nproc)"
  ls "$L/build-imagem/bin/llama-server" "$L/build-imagem/bin/libggml-vulkan.so" >/dev/null
fi
echo ">> pronto"
