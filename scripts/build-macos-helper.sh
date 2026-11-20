#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p src-tauri/resources
swiftc -O -target arm64-apple-macos14.0 native/MacWallpaper.swift -o src-tauri/resources/junadesk-wallpaper
codesign --force --sign - src-tauri/resources/junadesk-wallpaper
