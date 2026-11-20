#!/usr/bin/env bash
set -euo pipefail
[[ ${MSYSTEM:-} == UCRT64 ]] || { echo "Use the MSYS2 UCRT64 shell."; exit 1; }
cd "$(dirname "$0")/.."
output="$PWD/src-tauri/resources"
sources="$PWD/media-build/sources"
mkdir -p "$output/licenses" "$sources"

mpv=mingw-w64-ucrt-x86_64-mpv
mpv_release=0.42.0
version="$mpv_release-2"
mirror=https://repo.msys2.org/mingw
archive="$mpv-$version-any.pkg.tar.zst"
if [[ $(pacman -Q "$mpv" 2>/dev/null || :) != "$mpv $version" ]]; then
  pacman -U --noconfirm "$mirror/ucrt64/$archive"
fi
test "$(pacman -Q "$mpv")" = "$mpv $version"

declare -A seen=()
declare -A packages=()
queue=(mpv.exe)
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
  if (("${#files[@]}" == 0)); then
    pacman -Sw --noconfirm "$package"
    files=(/var/cache/pacman/pkg/"$package-$filename_version"-*.pkg.tar.zst)
  fi
  if (("${#files[@]}" == 0)) && [[ $package == "$mpv" ]]; then
    curl --fail --silent --show-error --location "$mirror/ucrt64/$archive" -o "$sources/$archive"
    files=("$sources/$archive")
  fi
  test "${#files[@]}" -eq 1
  metadata=$(bsdtar -xOf "${files[0]}" .PKGINFO)
  base=$(sed -n 's/^pkgbase = //p' <<< "$metadata")
  test -n "$base"
  source="$base-$filename_version.src.tar.zst"
  if [[ ! -f "$sources/$source" ]]; then
    curl --fail --silent --show-error --location --retry 3 "$mirror/sources/$source" -o "$sources/$source"
    curl --fail --silent --show-error --location --retry 3 "$mirror/sources/$source.sig" -o "$sources/$source.sig"
    pacman-key --verify "$sources/$source.sig" "$sources/$source"
  fi
  printf '%s %s %s\n' "$package" "$version" "$source" >> "$sources/msys2-packages.txt"
  notices="$output/licenses/$package"
  mkdir -p "$notices"
  grep -E '^(pkgname|pkgver|url|license) = ' <<< "$metadata" > "$notices/PACKAGE.txt"
  if [[ $package == "$mpv" ]]; then
    bsdtar -xOf "$sources/$source" "$base/mpv-$mpv_release.tar.gz" |
      bsdtar -xf - -C "$notices" --strip-components=1 \
        "mpv-$mpv_release/Copyright" "mpv-$mpv_release/LICENSE.GPL" "mpv-$mpv_release/LICENSE.LGPL"
  fi
  while IFS= read -r license; do
    [[ ! -f $license ]] || cp "$license" "$output/licenses/$package/"
  done < <(pacman -Qlq "$package" | grep '/share/licenses/' || :)
done
sort -u "$sources/msys2-packages.txt" -o "$sources/msys2-packages.txt"
cp scripts/bundle-windows-player.sh "$sources/"
"$output/mpv.exe" --version
