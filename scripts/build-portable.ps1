
# build-portable.ps1
# Create portable ZIP package for wtmux
#
# Packaging never builds: run `cargo build --release` first.
#
# Usage:
#   .\scripts\build-portable.ps1 [-Sign]
#
# -Sign: code-sign the bundled wtmux.exe (if not already signed),
#        using the certificate named by CODESIGN_CERT.

param(
    [string]$Version = "",
    [switch]$Sign,
    [string]$TimestampUrl = ""
)

$ErrorActionPreference = "Stop"

# Run from the repository root regardless of invocation directory
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "=== wtmux Portable Package Builder ===" -ForegroundColor Cyan

# Signing prerequisites
$signTool = $null
if ($Sign) {
    . (Join-Path $PSScriptRoot "signing.ps1")
    $signTool = Assert-SignPrereqs
    if (-not $TimestampUrl) { $TimestampUrl = $script:DefaultTimestampUrl }
    Write-Host "Code signing enabled: /n `"$env:CODESIGN_CERT`"" -ForegroundColor Gray
}

# Get version from Cargo.toml if not specified
if (-not $Version) {
    $cargoToml = Get-Content ".\Cargo.toml" -Raw
    if ($cargoToml -match 'version\s*=\s*"([0-9.]+)"') {
        $Version = $matches[1]
        Write-Host "Version from Cargo.toml: $Version" -ForegroundColor Gray
    } else {
        Write-Host "Error: Could not determine version from Cargo.toml" -ForegroundColor Red
        exit 1
    }
}

# Packaging never builds: require an existing release build
$exePath = ".\target\release\wtmux.exe"
if (-not (Test-Path $exePath)) {
    Write-Host "Error: wtmux.exe not found at $exePath" -ForegroundColor Red
    Write-Host "Please build first: cargo build --release" -ForegroundColor Yellow
    exit 1
}

# Create output directory
$outputDir = ".\installer\output"
if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir | Out-Null
}

# Create staging directory
$stagingDir = ".\installer\portable"
$packageDir = "$stagingDir\wtmux-$Version"

if (Test-Path $stagingDir) {
    Remove-Item -Recurse -Force $stagingDir
}
New-Item -ItemType Directory -Path $packageDir | Out-Null

# Copy files
Write-Host "Copying files..." -ForegroundColor Green

# Main executable — signed in place first so every package format reuses
# one signature (= one token PIN prompt); skipped when already validly signed
if ($Sign) {
    Invoke-CodeSign -SignTool $signTool -Path $exePath -TimestampUrl $TimestampUrl -SkipIfSigned
}
Copy-Item $exePath "$packageDir\wtmux.exe"

# Documentation
Copy-Item ".\README.md" "$packageDir\"
Copy-Item ".\README.ja.md" "$packageDir\"
Copy-Item ".\LICENSE" "$packageDir\"
Copy-Item ".\CHANGELOG.md" "$packageDir\"

# Config example
Copy-Item ".\config.example.toml" "$packageDir\"

# Create portable marker file (tells wtmux to use local config)
@"
# wtmux Portable Edition
# 
# This file indicates that wtmux is running in portable mode.
# Configuration will be stored in this directory instead of ~/.wtmux/
#
# To use: Place config.toml in this directory alongside wtmux.exe
"@ | Out-File -FilePath "$packageDir\PORTABLE" -Encoding UTF8

# Create a starter batch file
@"
@echo off
REM wtmux Portable Launcher
REM This sets the config directory to the current folder

set WTMUX_CONFIG_DIR=%~dp0
"%~dp0wtmux.exe" %*
"@ | Out-File -FilePath "$packageDir\wtmux-portable.bat" -Encoding ASCII

# Create ZIP
$zipPath = "$outputDir\wtmux-$Version-windows-x64.zip"
if (Test-Path $zipPath) {
    Remove-Item $zipPath
}

Write-Host "Creating ZIP archive..." -ForegroundColor Green

# Use Compress-Archive (PowerShell 5+)
Compress-Archive -Path "$packageDir\*" -DestinationPath $zipPath -CompressionLevel Optimal

# Cleanup
Remove-Item -Recurse -Force $stagingDir

# Get file size
$fileSize = (Get-Item $zipPath).Length
$fileSizeMB = [math]::Round($fileSize / 1MB, 2)

Write-Host ""
Write-Host "=== Build Complete ===" -ForegroundColor Cyan
Write-Host "Portable package: $zipPath" -ForegroundColor Green
Write-Host "Size: $fileSizeMB MB" -ForegroundColor Gray
Write-Host ""
Write-Host "Contents:" -ForegroundColor Yellow
Write-Host "  - wtmux.exe           (main executable)" -ForegroundColor Gray
Write-Host "  - wtmux-portable.bat  (launcher for portable mode)" -ForegroundColor Gray
Write-Host "  - config.example.toml (configuration template)" -ForegroundColor Gray
Write-Host "  - README.md           (English documentation)" -ForegroundColor Gray
Write-Host "  - README.ja.md        (Japanese documentation)" -ForegroundColor Gray
Write-Host "  - LICENSE             (MIT license)" -ForegroundColor Gray
Write-Host "  - CHANGELOG.md        (version history)" -ForegroundColor Gray
Write-Host "  - PORTABLE            (portable mode marker)" -ForegroundColor Gray
Write-Host ""
Write-Host "Usage:" -ForegroundColor Yellow
Write-Host "  1. Extract ZIP to any folder" -ForegroundColor Gray
Write-Host "  2. Run wtmux.exe or wtmux-portable.bat" -ForegroundColor Gray
Write-Host "  3. (Optional) Copy config.example.toml to config.toml and customize" -ForegroundColor Gray
