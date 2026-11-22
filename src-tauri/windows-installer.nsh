!macro NSIS_HOOK_POSTINSTALL
  ; Remove the legacy mpv payload after an in-place upgrade.
  ${If} ${FileExists} "$INSTDIR\tools\mpv.exe"
    Delete /REBOOTOK "$INSTDIR\tools\*.dll"
    Delete /REBOOTOK "$INSTDIR\tools\mpv.exe"
  ${EndIf}
!macroend
