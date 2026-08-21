# build-release-packages.ps1
# Build all Windows release packages in one go:
#   portable ZIP -> Inno Setup installer -> WiX MSI
#
# Packaging never builds: run `cargo build --release` first.
#
# -Sign: code-sign everything with the certificate named by CODESIGN_CERT,
#        with as few token PIN prompts as possible:
#          1. wtmux.exe        (signed once, in place — reused by all packages)
#          2. Inno uninstaller (signtool spawned by ISCC)
#          3. Inno setup exe   (signtool spawned by ISCC)
#          4. MSI
#        Each signtool process asks for the token PIN once; 4 prompts total.
#        To get down to one prompt per Windows session, enable
#        "Single Logon" (シングルログオン) in SafeNet Authentication Client.
#
# Usage:
#   .\scripts\build-release-packages.ps1 [-Sign] [-TimestampUrl <url>]

param(
    [string]$Version = "",
    [switch]$Sign,
    [string]$TimestampUrl = ""
)

$ErrorActionPreference = "Stop"

# Run from the repository root regardless of invocation directory
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "=== wtmux Release Package Builder ===" -ForegroundColor Cyan

# Packaging never builds: require an existing release build
$exePath = ".\target\release\wtmux.exe"
if (-not (Test-Path $exePath)) {
    Write-Host "Error: wtmux.exe not found at $exePath" -ForegroundColor Red
    Write-Host "Please build first: cargo build --release" -ForegroundColor Yellow
    exit 1
}

$childArgs = @{}
if ($Version) { $childArgs.Version = $Version }
if ($Sign) {
    $childArgs.Sign = $true
    if ($TimestampUrl) { $childArgs.TimestampUrl = $TimestampUrl }

    # Sign the exe up front (PIN prompt #1); the child scripts then see a valid
    # signature and skip their own exe-signing step
    . (Join-Path $PSScriptRoot "signing.ps1")
    $signTool = Assert-SignPrereqs
    if (-not $TimestampUrl) { $TimestampUrl = $script:DefaultTimestampUrl }
    Invoke-CodeSign -SignTool $signTool -Path $exePath -TimestampUrl $TimestampUrl -SkipIfSigned
}

$steps = @(
    @{ Name = "Portable ZIP";        Script = "build-portable.ps1" },
    @{ Name = "Inno Setup installer"; Script = "build-inno-installer.ps1" },
    @{ Name = "WiX MSI installer";    Script = "build-installer.ps1" }
)

foreach ($step in $steps) {
    Write-Host ""
    Write-Host ">>> $($step.Name)" -ForegroundColor Cyan
    # Child scripts only call `exit` on failure; clear any stale exit code first
    $global:LASTEXITCODE = 0
    & (Join-Path $PSScriptRoot $step.Script) @childArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: $($step.Name) build failed" -ForegroundColor Red
        exit 1
    }
}

Write-Host ""
Write-Host "=== All Packages Complete ===" -ForegroundColor Cyan
Get-ChildItem ".\installer\output" | ForEach-Object {
    Write-Host ("  {0}  ({1:N2} MB)" -f $_.Name, ($_.Length / 1MB)) -ForegroundColor Green
}
