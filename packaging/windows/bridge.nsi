; Installer for Windows (NSIS). Built by the release workflow:
;   makensis /DVERSION=0.3.0 /DEXE=path\to\bridge-app.exe /DOUT=Bridge-setup.exe /DROOT=path\to\repo packaging\windows\bridge.nsi
; It installs for the current user only, so it needs no administrator rights.

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef EXE
  !error "pass /DEXE=path\to\bridge-app.exe"
!endif
!ifndef OUT
  !define OUT "Bridge-setup.exe"
!endif
!ifndef ROOT
  !define ROOT "..\.."
!endif

!define APP "Bridge"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Bridge"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${APP} ${VERSION}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Bridge"
InstallDirRegKey HKCU "Software\Bridge" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Bridge ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Bridge"
VIAddVersionKey "FileDescription" "Bridge installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT licence"

!define MUI_ICON "${ROOT}\packaging\icons\bridge.ico"
!define MUI_UNICON "${ROOT}\packaging\icons\bridge.ico"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_LICENSE "${ROOT}\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\bridge-app.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Bridge"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Bridge" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; a running Bridge would keep the file locked
  nsExec::Exec 'taskkill /F /IM bridge-app.exe'
  Sleep 300
  File "${EXE}"
  File "${ROOT}\packaging\icons\bridge.ico"
  File "${ROOT}\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\Bridge.lnk" "$INSTDIR\bridge-app.exe" "" "$INSTDIR\bridge.ico"

  WriteRegStr HKCU "Software\Bridge" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Bridge"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Bridge"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\bridge.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section /o "Start Bridge when I sign in to Windows" SecAutostart
  WriteRegStr HKCU "${RUN_KEY}" "Bridge" '"$INSTDIR\bridge-app.exe" --hidden'
SectionEnd

LangString DESC_Main ${LANG_ENGLISH} "The Bridge program."
LangString DESC_Autostart ${LANG_ENGLISH} "Starts Bridge in the tray, with its window hidden, each time you sign in."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} $(DESC_Autostart)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; Your paired devices, messages and received files are not removed by the uninstaller.
Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM bridge-app.exe'
  Sleep 300
  Delete "$INSTDIR\bridge-app.exe"
  Delete "$INSTDIR\bridge.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Bridge.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "Bridge"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Bridge"
SectionEnd
