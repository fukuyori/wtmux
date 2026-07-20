# ファイルのブロック状態を確認
Get-Item (Join-Path $PSScriptRoot "build-inno-installer.ps1") -Stream Zone.Identifier -ErrorAction SilentlyContinue
#
# ブロックを解除
Unblock-File (Join-Path $PSScriptRoot "build-inno-installer.ps1")
Unblock-File (Join-Path $PSScriptRoot "build-installer.ps1")
Unblock-File (Join-Path $PSScriptRoot "build-portable.ps1")
Unblock-File (Join-Path $PSScriptRoot "build-msix.ps1")
Unblock-File (Join-Path $PSScriptRoot "generate-icons.ps1")
