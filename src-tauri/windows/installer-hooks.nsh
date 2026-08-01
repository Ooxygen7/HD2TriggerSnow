; Stop the legacy Electron build before this installer copies the Rust build.
; The shared Tauri macro gives the user a chance to cancel, then closes only
; the exact legacy executable name. It never targets generic electron.exe.
!macro NSIS_HOOK_PREINSTALL
  !insertmacro CheckIfAppIsRunning "HD2 Macro Terminal.exe" "旧版 HD2 Macro Terminal"
  !insertmacro CheckIfAppIsRunning "HD2-Trigger.exe" "旧版 HD2 Macro Terminal"
!macroend
