# ファイルのブロック状態を確認
Get-Item .\build-inno-installer.ps1 -Stream Zone.Identifier -ErrorAction SilentlyContinue
#
# ブロックを解除
Unblock-File .\build-inno-installer.ps1
Unblock-File .\build-installer.ps1
Unblock-File .\build-portable.ps1
