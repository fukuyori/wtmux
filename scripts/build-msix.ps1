# build-msix.ps1
# Build MSIX package for wtmux
#
# Prerequisites:
#   - Windows 10 SDK (for makeappx.exe and signtool.exe)
#   - Rust toolchain (for cargo build)
#
# Usage:
#   .\scripts\build-msix.ps1                    # Build unsigned MSIX
#   .\scripts\build-msix.ps1 -Sign              # Build and sign with self-signed cert
#   .\scripts\build-msix.ps1 -Sign -CertPath "path\to\cert.pfx" -CertPassword "password"
#

param(
    [switch]$Sign,
    [string]$CertPath = "",
    [string]$CertPassword = "",
    [switch]$CreateCert
)

$ErrorActionPreference = "Stop"

# Configuration
$AppName = "wtmux"
$Version = ""
$Revision = "0"
$Publisher = "CN=5EF2DE34-8C4A-42F6-9955-D951EB8F20B7"

# Paths
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$InstallerDir = Join-Path $ProjectRoot "installer"
$MsixDir = Join-Path $InstallerDir "msix"
$OutputDir = Join-Path $ProjectRoot "target\release"
$PackageDir = Join-Path $ProjectRoot "target\msix-package"
$AssetsDir = Join-Path $PackageDir "Assets"

# Get version from Cargo.toml
$cargoToml = Get-Content (Join-Path $ProjectRoot "Cargo.toml") -Raw
if ($cargoToml -match 'version\s*=\s*"([0-9.]+)"') {
    $Version = "$($matches[1]).$Revision"
} else {
    Write-Error "Could not determine version from Cargo.toml"
    exit 1
}

$MsixOutput = Join-Path $ProjectRoot "target\$AppName-$Version.msix"

# Find Windows SDK
$SdkPath = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\10.*" -Directory | 
    Sort-Object Name -Descending | 
    Select-Object -First 1

if (-not $SdkPath) {
    Write-Error "Windows 10 SDK not found. Please install it from https://developer.microsoft.com/windows/downloads/windows-sdk/"
    exit 1
}

$MakeAppx = Join-Path $SdkPath.FullName "x64\makeappx.exe"
$SignTool = Join-Path $SdkPath.FullName "x64\signtool.exe"

if (-not (Test-Path $MakeAppx)) {
    Write-Error "makeappx.exe not found at $MakeAppx"
    exit 1
}

Write-Host "Using Windows SDK: $($SdkPath.Name)" -ForegroundColor Cyan

# Step 1: Build release binary
Write-Host "`n[1/5] Building release binary..." -ForegroundColor Green
Push-Location $ProjectRoot
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Cargo build failed"
        exit 1
    }
} finally {
    Pop-Location
}

# Step 2: Create package directory
Write-Host "`n[2/5] Creating package directory..." -ForegroundColor Green
if (Test-Path $PackageDir) {
    Remove-Item -Recurse -Force $PackageDir
}
New-Item -ItemType Directory -Force -Path $PackageDir | Out-Null
New-Item -ItemType Directory -Force -Path $AssetsDir | Out-Null

# Step 3: Copy files
Write-Host "`n[3/5] Copying files..." -ForegroundColor Green

# Copy executable
Copy-Item (Join-Path $OutputDir "wtmux.exe") $PackageDir

# Copy manifest
$manifestPath = Join-Path $PackageDir "AppxManifest.xml"
$manifestXml = [xml](Get-Content (Join-Path $MsixDir "AppxManifest.xml") -Raw)
$manifestXml.Package.Identity.Version = $Version
$xmlSettings = New-Object System.Xml.XmlWriterSettings
$xmlSettings.Encoding = New-Object System.Text.UTF8Encoding($false)
$xmlSettings.Indent = $true
$xmlSettings.NewLineChars = "`r`n"
$xmlSettings.NewLineHandling = [System.Xml.NewLineHandling]::Replace
$xmlWriter = [System.Xml.XmlWriter]::Create($manifestPath, $xmlSettings)
try {
    $manifestXml.Save($xmlWriter)
} finally {
    $xmlWriter.Dispose()
}

# Generate and copy assets
$iconScript = Join-Path $PSScriptRoot "generate-icons.ps1"
if (Test-Path $iconScript) {
    Write-Host "  Generating icon assets..." -ForegroundColor Green
    try {
        & $iconScript
    } catch {
        Write-Error "Icon generation failed"
        exit 1
    }
}

$SourceAssets = Join-Path $MsixDir "Assets"
if (-not (Test-Path $SourceAssets)) {
    Write-Error "MSIX icon assets not found at $SourceAssets"
    exit 1
}
Copy-Item "$SourceAssets\*" $AssetsDir -Recurse -Force

# Step 4: Create MSIX package
Write-Host "`n[4/5] Creating MSIX package..." -ForegroundColor Green
if (Test-Path $MsixOutput) {
    Remove-Item $MsixOutput
}

& $MakeAppx pack /d $PackageDir /p $MsixOutput /nv
if ($LASTEXITCODE -ne 0) {
    Write-Error "makeappx failed"
    exit 1
}

Write-Host "  Created: $MsixOutput" -ForegroundColor Cyan

# Step 5: Sign package (optional)
if ($Sign) {
    Write-Host "`n[5/5] Signing package..." -ForegroundColor Green
    
    if ($CreateCert -or (-not $CertPath)) {
        # Create self-signed certificate
        $CertPath = Join-Path $ProjectRoot "target\wtmux-dev.pfx"
        $CertPassword = "wtmux-dev"
        
        Write-Host "  Creating self-signed certificate..." -ForegroundColor Yellow
        
        # Check if cert already exists in store
        $existingCert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $Publisher }
        
        if (-not $existingCert) {
            $cert = New-SelfSignedCertificate `
                -Type Custom `
                -Subject $Publisher `
                -KeyUsage DigitalSignature `
                -FriendlyName "wtmux Development Certificate" `
                -CertStoreLocation "Cert:\CurrentUser\My" `
                -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
            
            $pwd = ConvertTo-SecureString -String $CertPassword -Force -AsPlainText
            Export-PfxCertificate -Cert $cert -FilePath $CertPath -Password $pwd | Out-Null
            
            Write-Host "  Certificate created and exported to: $CertPath" -ForegroundColor Cyan
            Write-Host "  Password: $CertPassword" -ForegroundColor Cyan
            Write-Host ""
            Write-Host "  NOTE: To install the MSIX, you need to trust this certificate." -ForegroundColor Yellow
            Write-Host "  Run this command as Administrator:" -ForegroundColor Yellow
            Write-Host "    Import-Certificate -FilePath `"$CertPath`" -CertStoreLocation Cert:\LocalMachine\TrustedPeople" -ForegroundColor White
        } else {
            $cert = $existingCert
            Write-Host "  Using existing certificate from store" -ForegroundColor Cyan
        }
        
        # Sign with certificate from store
        & $SignTool sign /fd SHA256 /a /s My /n $Publisher $MsixOutput
    } else {
        # Sign with provided certificate
        if (-not $CertPassword) {
            $securePassword = Read-Host "Enter certificate password" -AsSecureString
            $CertPassword = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
                [Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePassword))
        }
        
        & $SignTool sign /fd SHA256 /f $CertPath /p $CertPassword $MsixOutput
    }
    
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Signing failed. The package was created but is unsigned."
    } else {
        Write-Host "  Package signed successfully" -ForegroundColor Cyan
    }
} else {
    Write-Host "`n[5/5] Skipping signing (use -Sign to sign the package)" -ForegroundColor Yellow
}

# Done
Write-Host "`n========================================" -ForegroundColor Green
Write-Host "MSIX package created successfully!" -ForegroundColor Green
Write-Host "Output: $MsixOutput" -ForegroundColor Cyan
Write-Host ""
Write-Host "To install (requires signed package or developer mode):" -ForegroundColor White
Write-Host "  Add-AppxPackage -Path `"$MsixOutput`"" -ForegroundColor Gray
Write-Host ""
if (-not $Sign) {
    Write-Host "NOTE: To install unsigned packages, enable Developer Mode in Windows Settings" -ForegroundColor Yellow
    Write-Host "      Settings > Update & Security > For developers > Developer Mode" -ForegroundColor Yellow
}
