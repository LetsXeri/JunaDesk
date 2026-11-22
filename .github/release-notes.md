## JunaDesk 0.1.5 Preview

Importiere animierte MP4-, WebM- und GIF-Dateien und weise sie deinen Bildschirmen zu. JunaDesk läuft über die Menüleiste oder den Infobereich weiter. Vollbild- und Akkupausen sowie Autostart lassen sich in den Einstellungen steuern.

In dieser Preview:

- Windows: Die Wiedergabe erfolgt über Windows Media Foundation. Beim Update werden das bisherige mpv und die zugehörigen Medien-DLLs entfernt. FFmpeg und ffprobe sind statisch gebaut und werden für den Import verwendet.

- Windows: JunaDesk erstellt das Wiedergabefenster mit einer eigenen Fensterklasse und übernimmt dabei den DPI-Modus von Explorer. Meldungen beim Einrichten der Desktop-Ebene enthalten den Win32-Fehlercode.

- Kompaktere Bibliothek mit Suche, Sortierung und Videovorschau. Monitorzuweisungen und Wiedergabestatus sind pro Bildschirm sichtbar.
- Ruhigere Startanimation beim sichtbaren Kaltstart. Unter macOS bleibt beim Video-Loop das letzte Bild sichtbar, bis das nächste bereitsteht.
- Update-Hinweise für spätere stabile Releases und optionale Systembenachrichtigungen unter **Einstellungen → Updates**.

### Installation

- **macOS 14 oder neuer mit Apple Silicon:** Lade die DMG, öffne sie und ziehe JunaDesk in den Programme-Ordner. Die App ist ad hoc signiert. Gatekeeper kann den ersten Start blockieren, da Apple die Preview nicht notarisiert hat.
- **Windows 11 ab 24H2 mit x64-Prozessor:** Lade die Setup-EXE und öffne sie. Wenn du von einer älteren Version aktualisierst, beende JunaDesk im Infobereich und wähle im Setup **Do not uninstall**.

Die Datei `SHA256SUMS.txt` enthält die Prüfsummen für beide Installer und beide Medienquellarchive. Die Archive enthalten den zu den Medienwerkzeugen passenden Quellcode und die Build-Rezepte. JunaDesk selbst steht unter MIT.
