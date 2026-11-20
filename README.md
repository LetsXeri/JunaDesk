# JunaDesk

JunaDesk bringt eigene animierte Wallpaper auf den Desktop von macOS und Windows. Importiere MP4, WebM oder GIF, lege pro Bildschirm einen Hintergrund fest und steuere die Wiedergabe über die Menüleiste oder den Windows-Infobereich. Bibliothek und Konfiguration bleiben auf deinem Gerät.

## Download und Installation

[Zu den GitHub Releases](https://github.com/LetsXeri/JunaDesk/releases)

| System | Datei | Installation |
| --- | --- | --- |
| macOS 14+ mit Apple Silicon | `JunaDesk-v0.1.1-macOS-arm64.dmg` | DMG öffnen, JunaDesk in **Programme** ziehen und von dort starten. |
| Windows 11 ab 24H2 mit x64 | `JunaDesk-v0.1.1-Windows-x64.exe` | Setup öffnen und dem Installationsdialog folgen. |

Die macOS-Preview ist ad hoc signiert und ohne Apple-Notarisierung. Gatekeeper kann den ersten Start blockieren. Für einen Download, dem du vertraust, erklärt Apple die Freigabe über **Systemeinstellungen → Datenschutz & Sicherheit**. Unter Windows kann SmartScreen beim unsignierten Installer warnen. Smart App Control kann die Ausführung abhängig von der Systemkonfiguration blockieren. [Apple Gatekeeper](https://support.apple.com/de-de/102445), [Microsoft SmartScreen und Smart App Control](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)

Die Release-Datei `SHA256SUMS.txt` enthält die SHA-256-Prüfsummen. Zum Abgleich kannst du auf dem Mac `shasum -a 256 DATEI` oder in PowerShell `Get-FileHash DATEI -Algorithm SHA256` ausführen.

## Verwendung

1. Öffne die Bibliothek und ziehe deine Dateien ins Fenster oder klicke auf **Hinzufügen**. JunaDesk kopiert die Quellen in seine Bibliothek und erstellt Vorschaubilder.
2. Wähle **Aktivieren** auf einer Kachel und wähle den Bildschirm. Unter **Monitore** kannst du Zuweisungen ändern oder den Systemhintergrund wieder anzeigen.
3. Schließe das Fenster, um JunaDesk im Hintergrund laufen zu lassen. Das Menü bietet **Öffnen**, **Pause/Fortsetzen** und **Beenden**. Vollbildpausen sind standardmäßig aktiv. Autostart, Akkupause und FPS-Limit findest du in den Einstellungen.

JunaDesk speichert Bibliothek und Einstellungen unter `~/Library/Application Support/io.github.letsxeri.junadesk` auf macOS bzw. `%LOCALAPPDATA%\io.github.letsxeri.junadesk` auf Windows. Zum vollständigen Entfernen zuerst den Autostart ausschalten, JunaDesk beenden, die App entfernen und bei Bedarf den lokalen Datenordner löschen.

## Entwicklung

Voraussetzungen sind Node.js 24, aktuelles Rust stable und die plattformspezifischen Tauri-Voraussetzungen. [Tauri-Voraussetzungen](https://v2.tauri.app/start/prerequisites/)

```sh
npm ci
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

**macOS:** Mit Xcode, `pkg-config` und `npm run native:mac` den AppKit-Helfer erstellen. Im Entwicklungsmodus sucht JunaDesk nach FFmpeg und ffprobe auf dem PATH, wenn keine lokalen Binärdateien bereitliegen. `bash scripts/build-ffmpeg.sh` erstellt die mitgelieferte Version.

**Windows:** In einer MSYS2-UCRT64-Shell mit `curl`, `libarchive` und `mingw-w64-ucrt-x86_64-binutils` das Skript `scripts/bundle-windows-player.sh` ausführen. Es bündelt die festgelegten MSYS2-Versionen von FFmpeg, ffprobe und mpv mit den passenden Quellen. JunaDesk selbst wird mit dem MSVC-Rust-Target gebaut.

Nach dem Vorbereiten der Werkzeuge startet `npm run tauri dev` die App. `npm run tauri build -- -- --locked` baut das Installationspaket des aktuellen Systems. Die Medienprogramme und Hinweise liegen in `src-tauri/resources`. Die vom Build erzeugten Medienquellen liegen in `media-build/sources`.

## Updates

Unter **Einstellungen → Updates** findest du die installierte Version und die manuelle Prüfung. JunaDesk fragt standardmäßig einmal täglich die öffentliche GitHub-Schnittstelle nach einem stabilen Release ab. Du kannst die automatische Prüfung abschalten und Systembenachrichtigungen separat einschalten. Der Update-Hinweis führt zum manuellen Download auf GitHub.

## Releases veröffentlichen

Die Versionsnummer in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` und `src-tauri/tauri.conf.json` aktualisieren und einen passenden `v*`-Tag pushen. GitHub Actions erstellt einen Release-Entwurf mit beiden Installern, Medienquellen und Prüfsummen. Für einen vorhandenen Tag lässt sich **Prepare preview** bei Bedarf manuell starten. Die CI prüft den Build. Die Desktop-Integration wird anschließend mit den Installern aus dem Entwurf auf den Zielsystemen abgenommen.

- Systemhintergrund wiederherstellen können. Wallpaper hinter anklickbaren Icons und normalen Fenstern anzeigen. Verwaltung schließen, im Tray erneut öffnen und über **Beenden** vollständig stoppen.
- MP4, WebM und GIF zusammen mit einer fehlerhaften Datei importieren. Vorschau, Umbenennen und Löschen eines zugewiesenen Wallpapers prüfen.
- Zwei Bildschirme mit gemischter Skalierung separat zuweisen. Ab- und Anstecken, Sleep/Wake, macOS-Spaces und Explorer-Neustart prüfen.
- Vollbild und Akkubetrieb prüfen. Manuelle Pause muss beim Ende eines automatischen Pausengrunds bestehen bleiben. Schleifenübergänge, FPS-Limit und CPU-/GPU-Last aktiv und pausiert vergleichen.
- Autostart bei der nächsten Anmeldung und Wiederherstellung der Zuweisungen prüfen. Beide Paketdateien und Medienquellarchive anhand der Prüfsummen kontrollieren. Nach der Abnahme den Entwurf als Preview oder als stabile Veröffentlichung freigeben. Für einen stabilen Release **Set as a pre-release** deaktivieren und ihn als **Latest** markieren. Nur stabile Releases mit passenden Installern lösen einen Update-Hinweis aus.

## Lizenz

Der JunaDesk-Code und die eigenen Grafiken stehen unter der [MIT-Lizenz](LICENSE). Die gebündelten Medienprogramme haben eigene Lizenzen. Lizenztexte liegen bei den Installationspaketen. Passende Quellen, Build-Konfiguration und MSYS2-Paketrezepte gibt es als zusätzliche Downloads im jeweiligen Release.
