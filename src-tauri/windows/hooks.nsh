; Branchdeck NSIS installer hooks (bundle.windows.nsis.installerHooks).
; Adds "Open Branchdeck here" to Explorer's right-click menu: on a folder,
; and on the empty space inside a folder. SHCTX follows the install mode
; (per-user -> HKCU, per-machine -> HKLM).

!define BD_BG_KEY "Software\Classes\Directory\Background\shell\Branchdeck"
!define BD_DIR_KEY "Software\Classes\Directory\shell\Branchdeck"

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "${BD_BG_KEY}" "" "Open Branchdeck here"
  WriteRegStr SHCTX "${BD_BG_KEY}" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${BD_BG_KEY}\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%V$\""

  WriteRegStr SHCTX "${BD_DIR_KEY}" "" "Open Branchdeck here"
  WriteRegStr SHCTX "${BD_DIR_KEY}" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${BD_DIR_KEY}\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey SHCTX "${BD_BG_KEY}"
  DeleteRegKey SHCTX "${BD_DIR_KEY}"
!macroend
