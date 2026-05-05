; soundEQ NSIS installer hooks
;
; Tauri v2 calls these four macros at fixed points in the installer/uninstaller.
; Only NSIS_HOOK_POSTUNINSTALL does real work here; the other three are required
; placeholders — tauri-bundler will error if any of the four is missing.

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

; Runs after the uninstaller has already removed the app binary, registry keys,
; and Start Menu shortcut. At this point the program is gone but user data
; (%APPDATA%\com.soundeq.app — EQ profiles + config.json) is still on disk.
;
; We prompt rather than auto-delete for two reasons:
;   1. A user reinstalling later may want their profiles preserved.
;   2. Silent force-deletion of user data without consent is poor practice.
;
; The IfFileExists check guards against showing the dialog on machines where the
; app was never launched (no data directory was ever created).
!macro NSIS_HOOK_POSTUNINSTALL
  IfFileExists "$APPDATA\com.soundeq.app\" 0 soundeq_skip_cleanup
  MessageBox MB_YESNO|MB_ICONQUESTION \
    "Remove soundEQ profiles and settings?$\n$\n\
     Your EQ profiles and configuration are stored in:$\n\
     $APPDATA\com.soundeq.app$\n$\n\
     Yes  =  delete everything (clean uninstall)$\n\
     No   =  keep them (useful if you plan to reinstall)" \
    IDNO soundeq_skip_cleanup
  RMDir /r "$APPDATA\com.soundeq.app"
  soundeq_skip_cleanup:
!macroend
