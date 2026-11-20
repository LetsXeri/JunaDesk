#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$PWD
work="$repo/media-build"
archives="$work/sources"
output="$repo/src-tauri/resources"
prefix="$work/prefix"
mkdir -p "$archives" "$output/licenses/ffmpeg" "$output/licenses/x264" "$prefix"

download() {
  local filename=$1 digest=$2 url=$3
  if [[ ! -f "$archives/$filename" ]]; then
    curl --fail --silent --show-error --location --retry 3 "$url" -o "$archives/$filename.part"
    mv "$archives/$filename.part" "$archives/$filename"
  fi
  if command -v sha256sum >/dev/null; then
    (cd "$archives"; printf '%s  %s\n' "$digest" "$filename" | sha256sum -c -)
  else
    (cd "$archives"; printf '%s  %s\n' "$digest" "$filename" | shasum -a 256 -c -)
  fi
}

ffmpeg=8.1.3
x264=baee400fa9ced6f5481a728138fed6e867b0ff7f
download "ffmpeg-$ffmpeg.tar.xz" 7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3 \
  "https://ffmpeg.org/releases/ffmpeg-$ffmpeg.tar.xz"
download "x264-$x264.tar.gz" 436a2be54d8bc0cb05dd33ecbbcb7df9c3b57362714fcdaa3a5991189a33319b \
  "https://codeload.github.com/mirror/x264/tar.gz/$x264"
build=$(mktemp -d "$work/build.XXXXXXXX")
trap 'rm -rf -- "$build"' EXIT
tar -xf "$archives/ffmpeg-$ffmpeg.tar.xz" -C "$build"
tar -xf "$archives/x264-$x264.tar.gz" -C "$build"

set --
suffix=
if [[ $(uname -s) == Darwin ]]; then
  export MACOSX_DEPLOYMENT_TARGET=14.0
  jobs=$(sysctl -n hw.ncpu 2>/dev/null || printf '4')
else
  [[ ${MSYSTEM:-} == UCRT64 ]] || { echo "Use the MSYS2 UCRT64 shell on Windows."; exit 1; }
  jobs=$(nproc)
  suffix=.exe
  set -- --extra-ldflags=-static
fi
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
(
  cd "$build/x264-$x264"
  ./configure --prefix="$prefix" --enable-static --disable-cli --disable-opencl
  make -j"${JUNADESK_JOBS:-$jobs}"
  make install
  cp COPYING "$output/licenses/x264/"
)
(
  cd "$build/ffmpeg-$ffmpeg"
  ./configure --prefix="$prefix" --enable-gpl --enable-libx264 --enable-static --disable-shared \
    --disable-autodetect --disable-network --disable-doc --disable-debug --disable-ffplay \
    --pkg-config-flags=--static "$@"
  make -j"${JUNADESK_JOBS:-$jobs}" ffmpeg"$suffix" ffprobe"$suffix"
  cp ffmpeg"$suffix" ffprobe"$suffix" "$output/"
  cp COPYING.GPLv2 COPYING.LGPLv2.1 LICENSE.md "$output/licenses/ffmpeg/"
  cp config.h "$archives/ffmpeg-build-config.h"
)
cp "$repo/scripts/build-ffmpeg.sh" "$archives/"
cp "$repo/scripts/THIRD-PARTY.txt" "$output/"
if [[ -z "$suffix" ]]; then
  codesign --force --sign - "$output/ffmpeg"
  codesign --force --sign - "$output/ffprobe"
fi
"$output/ffmpeg$suffix" -hide_banner -encoders | grep libx264
