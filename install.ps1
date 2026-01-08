# install.ps1
# Simple installer for wtmux (no WiX required)
#
# Usage:
#   .\install.ps1           # Install to default location
#   .\install.ps1 -Uninstall   # Uninstall

param(
    [switch]$Uninstall,
    [string]$InstallDir = "$env:LOCALAPPDATA\wtmux"
)

$ErrorActionPreference = "Stop"

function Add-ToPath {
    param([string]$Dir)
    
    $currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($currentPath -notlike "*$Dir*") {
        $newPath = "$currentPath;$Dir"
        [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
        Write-Host "Added to PATH: $Dir" -ForegroundColor Green
        return $true
    }
    return $false
}

function Remove-FromPath {
    param([string]$Dir)
    
    $currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    $paths = $currentPath -split ";" | Where-Object { $_ -ne $Dir -and $_ -ne "" }
    $newPath = $paths -join ";"
    [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
    Write-Host "Removed from PATH: $Dir" -ForegroundColor Green
}

# Uninstall
if ($Uninstall) {
    Write-Host "=== Uninstalling wtmux ===" -ForegroundColor Cyan
    
    $binDir = "$InstallDir\bin"
    
    if (Test-Path $InstallDir) {
        Remove-Item -Recurse -Force $InstallDir
        Write-Host "Removed: $InstallDir" -ForegroundColor Green
    }
    
    Remove-FromPath $binDir
    
    Write-Host ""
    Write-Host "wtmux has been uninstalled." -ForegroundColor Cyan
    Write-Host "Please restart your terminal for PATH changes to take effect." -ForegroundColor Yellow
    exit 0
}

# Install
Write-Host "=== Installing wtmux ===" -ForegroundColor Cyan
Write-Host "Install directory: $InstallDir" -ForegroundColor Gray

# Check for wtmux.exe
$exePath = ".\target\release\wtmux.exe"
if (-not (Test-Path $exePath)) {
    # Try current directory
    $exePath = ".\wtmux.exe"
    if (-not (Test-Path $exePath)) {
        Write-Host "Error: wtmux.exe not found" -ForegroundColor Red
        Write-Host "Please build first: cargo build --release" -ForegroundColor Yellow
        Write-Host "Or place wtmux.exe in the current directory" -ForegroundColor Yellow
        exit 1
    }
}

# Create directories
$binDir = "$InstallDir\bin"
if (-not (Test-Path $binDir)) {
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
}

# Copy executable
Write-Host "Copying wtmux.exe..." -ForegroundColor Green
Copy-Item $exePath "$binDir\wtmux.exe" -Force

# Copy config example if exists
$configExample = ".\config.example.toml"
if (Test-Path $configExample) {
    $configDir = "$env:USERPROFILE\.wtmux"
    if (-not (Test-Path $configDir)) {
        New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    }
    if (-not (Test-Path "$configDir\config.toml")) {
        Copy-Item $configExample "$configDir\config.toml"
        Write-Host "Created config: $configDir\config.toml" -ForegroundColor Green
    }
}

# Add to PATH
$pathChanged = Add-ToPath $binDir

Write-Host ""
Write-Host "=== Installation Complete ===" -ForegroundColor Cyan
Write-Host "Location: $binDir\wtmux.exe" -ForegroundColor Green

if ($pathChanged) {
    Write-Host ""
    Write-Host "PATH has been updated. Please restart your terminal." -ForegroundColor Yellow
    Write-Host "Then run 'wtmux' to start." -ForegroundColor Yellow
} else {
    Write-Host ""
    Write-Host "Run 'wtmux' to start." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "To uninstall: .\install.ps1 -Uninstall" -ForegroundColor Gray
