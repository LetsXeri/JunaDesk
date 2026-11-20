#!/usr/bin/env bash
set -euo pipefail
[[ ${MSYSTEM:-} == UCRT64 ]] || { echo "Use the MSYS2 UCRT64 shell."; exit 1; }
cd "$(dirname "$0")/.."
output="$PWD/src-tauri/resources"
sources="$PWD/media-build/sources"
mkdir -p "$output/licenses" "$sources"

mpv_release=0.42.0
ffmpeg_release=9.1
mpv=mingw-w64-ucrt-x86_64-mpv
ffmpeg=mingw-w64-ucrt-x86_64-ffmpeg
mirror=https://repo.msys2.org/mingw
declare -A pinned=(["$mpv"]="$mpv_release-2" ["$ffmpeg"]="$ffmpeg_release-1")
install=()
for package in "$mpv" "$ffmpeg"; do
  version=${pinned[$package]}
  if [[ $(pacman -Q "$package" 2>/dev/null || :) != "$package $version" ]]; then
    install+=("$mirror/ucrt64/$package-$version-any.pkg.tar.zst")
  fi
done
if (("${#install[@]}")); then
  pacman -U --noconfirm "${install[@]}"
fi
for package in "$mpv" "$ffmpeg"; do
  test "$(pacman -Q "$package")" = "$package ${pinned[$package]}"
done

declare -A seen=()
declare -A packages=()
queue=(mpv.exe ffmpeg.exe ffprobe.exe)
while (("${#queue[@]}")); do
  name=${queue[0]}
  queue=("${queue[@]:1}")
  [[ -z ${seen[$name]+x} ]] || continue
  seen[$name]=1
  path="/ucrt64/bin/$name"
  [[ -f $path ]] || continue # Windows system DLL
  cp "$path" "$output/"
  package=$(pacman -Qoq "$path")
  packages[$package]=1
  while IFS= read -r dll; do
    queue+=("$dll")
  done < <(objdump -p "$path" | awk '/DLL Name:/ { print tolower($3) }')
done

for package in "${!packages[@]}"; do
  read -r _ version <<< "$(pacman -Q "$package")"
  filename_version=${version#*:}
  shopt -s nullglob
  files=(/var/cache/pacman/pkg/"$package-$filename_version"-*.pkg.tar.zst)
  if (("${#files[@]}" == 0)) && [[ -z ${pinned[$package]+x} ]]; then
    pacman -Sw --noconfirm "$package"
    files=(/var/cache/pacman/pkg/"$package-$filename_version"-*.pkg.tar.zst)
  fi
  if (("${#files[@]}" == 0)) && [[ -n ${pinned[$package]+x} ]]; then
    archive="$package-$filename_version-any.pkg.tar.zst"
    curl --fail --silent --show-error --location --max-time 180 "$mirror/ucrt64/$archive" -o "$output/$archive"
    files=("$output/$archive")
  fi
  test "${#files[@]}" -eq 1
  metadata=$(bsdtar -xOf "${files[0]}" .PKGINFO)
  if [[ ${files[0]} == "$output/"* ]]; then
    rm -- "${files[0]}"
  fi
  base=$(sed -n 's/^pkgbase = //p' <<< "$metadata")
  test -n "$base"
  source="$base-$filename_version.src.tar.zst"
  if [[ ! -f "$sources/$source" ]]; then
    curl --fail --silent --show-error --location --retry 3 --max-time 180 "$mirror/sources/$source" -o "$sources/$source"
    curl --fail --silent --show-error --location --retry 3 --max-time 180 "$mirror/sources/$source.sig" -o "$sources/$source.sig"
    # The MSYS2 public distribution keyring is sufficient to verify source archives.
    gpgv --keyring /usr/share/pacman/keyrings/msys2.gpg "$sources/$source.sig" "$sources/$source"
  fi
  printf '%s %s %s\n' "$package" "$version" "$source" >> "$sources/msys2-packages.txt"
  notices="$output/licenses/$package"
  mkdir -p "$notices"
  grep -E '^(pkgname|pkgver|url|license) = ' <<< "$metadata" > "$notices/PACKAGE.txt"
  if [[ $package == "$mpv" ]]; then
    bsdtar -xOf "$sources/$source" "$base/mpv-$mpv_release.tar.gz" |
      bsdtar -xf - -C "$notices" --strip-components=1 \
        "mpv-$mpv_release/Copyright" "mpv-$mpv_release/LICENSE.GPL" "mpv-$mpv_release/LICENSE.LGPL"
  elif [[ $package == "$ffmpeg" ]]; then
    bsdtar -xOf "$sources/$source" "$base/ffmpeg-$ffmpeg_release.tar.xz" |
      bsdtar -xf - -C "$notices" --strip-components=1 \
        "ffmpeg-$ffmpeg_release/COPYING.GPLv2" "ffmpeg-$ffmpeg_release/COPYING.GPLv3" \
        "ffmpeg-$ffmpeg_release/COPYING.LGPLv2.1" "ffmpeg-$ffmpeg_release/LICENSE.md"
  fi
  while IFS= read -r license; do
    [[ ! -f $license ]] || cp "$license" "$output/licenses/$package/"
  done < <(pacman -Qlq "$package" | grep '/share/licenses/' || :)
done
sort -u "$sources/msys2-packages.txt" -o "$sources/msys2-packages.txt"
cp scripts/bundle-windows-player.sh "$sources/"
cp scripts/THIRD-PARTY.txt "$output/"
"$output/ffmpeg.exe" -hide_banner -encoders | grep libx264
"$output/ffprobe.exe" -version
"$output/mpv.exe" --version
