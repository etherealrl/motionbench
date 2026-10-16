; NSIS fits the pictures on the installer's pages to them by picking pixels, which leaves the
; icon's edges jagged on a scaled display. these load each picture whole and let Windows average
; it down onto a bitmap the size the page gives it, so it stays smooth at any scaling. Tauri
; includes this file after MUI2 and before the pages, so the two loaders MUI uses get replaced

Var SmoothControl
Var SmoothPath
Var SmoothBitmap

!macro MOTION_BENCH_SMOOTH_IMAGE PREFIX
  Function ${PREFIX}SmoothImage
    Push $0
    Push $1
    Push $2
    Push $3
    Push $4
    Push $5
    Push $6
    Push $7
    Push $8
    Push $9
    ; the size the page gives the picture, in the display's pixels
    System::Call "*(i, i, i, i) p .r0"
    System::Call "user32::GetClientRect(p $SmoothControl, p r0)"
    System::Call "*$0(i, i, i .r1, i .r2)"
    System::Free $0
    ; the picture whole, and its own size. 0x2010 loads it from the file as a DIB section
    StrCpy $9 $SmoothPath
    System::Call "user32::LoadImageW(p 0, w r9, i 0, i 0, i 0, i 0x2010) p .r3"
    System::Call "*(&i32) p .r0"
    System::Call "gdi32::GetObjectW(p r3, i 32, p r0)"
    System::Call "*$0(i, i .r4, i .r5)"
    System::Free $0
    ; averaged down, mode 4 being HALFTONE, which needs the brush origin set after it
    System::Call "user32::GetDC(p 0) p .r0"
    System::Call "gdi32::CreateCompatibleDC(p r0) p .r6"
    System::Call "gdi32::CreateCompatibleDC(p r0) p .r7"
    System::Call "gdi32::CreateCompatibleBitmap(p r0, i r1, i r2) p .r8"
    System::Call "user32::ReleaseDC(p 0, p r0)"
    System::Call "gdi32::SelectObject(p r6, p r3) p .r0"
    System::Call "gdi32::SelectObject(p r7, p r8) p .r9"
    System::Call "gdi32::SetStretchBltMode(p r7, i 4)"
    System::Call "gdi32::SetBrushOrgEx(p r7, i 0, i 0, p 0)"
    System::Call "gdi32::StretchBlt(p r7, i 0, i 0, i r1, i r2, p r6, i 0, i 0, i r4, i r5, i 0xCC0020)"
    System::Call "gdi32::SelectObject(p r6, p r0)"
    System::Call "gdi32::SelectObject(p r7, p r9)"
    System::Call "gdi32::DeleteDC(p r6)"
    System::Call "gdi32::DeleteDC(p r7)"
    System::Call "gdi32::DeleteObject(p r3)"
    ; STM_SETIMAGE with IMAGE_BITMAP
    SendMessage $SmoothControl 0x0172 0 $8
    StrCpy $SmoothBitmap $8
    Pop $9
    Pop $8
    Pop $7
    Pop $6
    Pop $5
    Pop $4
    Pop $3
    Pop $2
    Pop $1
    Pop $0
  FunctionEnd
!macroend
!insertmacro MOTION_BENCH_SMOOTH_IMAGE ""
!insertmacro MOTION_BENCH_SMOOTH_IMAGE "un."

; the picture beside the first and last pages. MUI frees the bitmap it gets back when the page goes
!macroundef MUI_INTERNAL_FULLWINDOW_LOADWIZARDIMAGE
!macro MUI_INTERNAL_FULLWINDOW_LOADWIZARDIMAGE _un _hwndImg _ImgPath _RetImgHandle
  StrCpy $SmoothControl ${_hwndImg}
  StrCpy $SmoothPath "${_ImgPath}"
  !if "${_un}" == ""
    Call SmoothImage
  !else
    Call un.SmoothImage
  !endif
  StrCpy ${_RetImgHandle} $SmoothBitmap
!macroend

; the picture in the corner of the other pages, set once when the window opens
!macroundef MUI_HEADERIMAGE_INITHELPER_LOADIMAGE
!macro MUI_HEADERIMAGE_INITHELPER_LOADIMAGE UN RTL IMGRESID PATH
  GetDlgItem $mui.Header.Image $HWNDPARENT ${IMGRESID}
  StrCpy $SmoothControl $mui.Header.Image
  StrCpy $SmoothPath "${PATH}"
  !if "${UN}" == ""
    Call SmoothImage
  !else
    Call un.SmoothImage
  !endif
!macroend
