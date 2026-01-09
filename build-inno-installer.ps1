# build-inno-installer.ps1
# Build wtmux installer using Inno Setup
#
# Prerequisites:
#   1. Install Inno Setup from https://jrsoftware.org/isinfo.php
#   2. Build wtmux in release mode: cargo build --release

param(
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

Write-Host "=== wtmux Inno Setup Installer Build ===" -ForegroundColor Cyan

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

# Find Inno Setup compiler
$isccPaths = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 5\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 5\ISCC.exe"
)

$iscc = $null
foreach ($path in $isccPaths) {
    if (Test-Path $path) {
        $iscc = $path
        break
    }
}

if (-not $iscc) {
    $iscc = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
    if ($iscc) {
        $iscc = $iscc.Source
    }
}

if (-not $iscc) {
    Write-Host "Error: Inno Setup not found" -ForegroundColor Red
    Write-Host "Please install Inno Setup from https://jrsoftware.org/isinfo.php" -ForegroundColor Yellow
    exit 1
}

Write-Host "Using Inno Setup: $iscc" -ForegroundColor Gray

# Check for release build
$exePath = ".\target\release\wtmux.exe"
if (-not (Test-Path $exePath)) {
    Write-Host "Release build not found. Building..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: Build failed" -ForegroundColor Red
        exit 1
    }
}

# Create output directory
$outputDir = ".\installer\output"
if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir | Out-Null
}

# Update version in iss file
$issPath = ".\installer\wtmux.iss"
$issContent = Get-Content $issPath -Raw
$issContent = $issContent -replace '#define MyAppVersion "[0-9.]+"', "#define MyAppVersion `"$Version`""
$issContent | Set-Content $issPath

# Build installer (run from installer directory)
Write-Host "Building installer..." -ForegroundColor Green
Push-Location .\installer
try {
    & $iscc /Q "wtmux.iss"
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: Inno Setup compilation failed" -ForegroundColor Red
        exit 1
    }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "=== Build Complete ===" -ForegroundColor Cyan
Write-Host "Installer: $outputDir\wtmux-$Version-setup.exe" -ForegroundColor Green
