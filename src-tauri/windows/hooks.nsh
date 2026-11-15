; Хуки установщика NSIS (Tauri, `bundle.windows.nsis.installerHooks`).

; Tauri по умолчанию ставит программу в `%LOCALAPPDATA%\<productName>`, а у нас там профиль
; WebView2 и данные переноса со старого названия. Поэтому ставим в
; `%LOCALAPPDATA%\Programs\LeeLauncher`. Срабатывает и тогда, когда старую папку выбрали вручную
; или её запомнила прошлая установка: отличить эти случаи нельзя.
; Из старой папки убираем только файлы программы, профиль WebView2 не трогаем.
!macro NSIS_HOOK_PREINSTALL
	${If} $INSTDIR == "$LOCALAPPDATA\${PRODUCTNAME}"
		Delete "$INSTDIR\${MAINBINARYNAME}.exe"
		Delete "$INSTDIR\uninstall.exe"
		StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
		SetOutPath $INSTDIR
	${EndIf}
!macroend
