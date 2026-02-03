# build-msix.ps1
# Build MSIX package for wtmux
#
# Prerequisites:
#   - Windows 10 SDK (for makeappx.exe and signtool.exe)
#   - Rust toolchain (for cargo build)
#
# Usage:
#   .\build-msix.ps1                    # Build unsigned MSIX
#   .\build-msix.ps1 -Sign              # Build and sign with self-signed cert
#   .\build-msix.ps1 -Sign -CertPath "path\to\cert.pfx" -CertPassword "password"
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
$Version = "1.1.0.0"
$Publisher = "CN=wtmux"

# Paths
$ProjectRoot = $PSScriptRoot
$InstallerDir = Join-Path $ProjectRoot "installer"
$MsixDir = Join-Path $InstallerDir "msix"
$OutputDir = Join-Path $ProjectRoot "target\release"
$PackageDir = Join-Path $ProjectRoot "target\msix-package"
$AssetsDir = Join-Path $PackageDir "Assets"
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
Copy-Item (Join-Path $MsixDir "AppxManifest.xml") $PackageDir

# Copy or generate assets
$SourceAssets = Join-Path $MsixDir "Assets"
if (Test-Path $SourceAssets) {
    Copy-Item "$SourceAssets\*" $AssetsDir -Recurse
} else {
    Write-Host "  Generating placeholder icons..." -ForegroundColor Yellow
    
    # Try to use System.Drawing, fall back to creating simple PNG files
    try {
        Add-Type -AssemblyName System.Drawing -ErrorAction Stop
        
        $sizes = @{
            "StoreLogo.png" = @(50, 50)
            "Square44x44Logo.png" = @(44, 44)
            "Square150x150Logo.png" = @(150, 150)
            "Wide310x150Logo.png" = @(310, 150)
        }
        
        foreach ($file in $sizes.Keys) {
            $dims = $sizes[$file]
            $width = $dims[0]
            $height = $dims[1]
            
            $bitmap = New-Object System.Drawing.Bitmap($width, $height)
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            
            # Dark background (Tokyo Night theme color)
            $bgColor = [System.Drawing.Color]::FromArgb(255, 26, 27, 38)
            $graphics.Clear($bgColor)
            
            # Draw "W" text
            $fontSize = [float][Math]::Max(8, [Math]::Min($width, $height) * 0.4)
            $font = New-Object System.Drawing.Font("Consolas", $fontSize, [System.Drawing.FontStyle]::Bold)
            $fgColor = [System.Drawing.Color]::FromArgb(255, 122, 162, 247)
            $brush = New-Object System.Drawing.SolidBrush($fgColor)
            
            $format = New-Object System.Drawing.StringFormat
            $format.Alignment = [System.Drawing.StringAlignment]::Center
            $format.LineAlignment = [System.Drawing.StringAlignment]::Center
            
            $rect = New-Object System.Drawing.RectangleF(0, 0, $width, $height)
            $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
            $graphics.DrawString("W", $font, $brush, $rect, $format)
            
            $outputPath = Join-Path $AssetsDir $file
            $bitmap.Save($outputPath, [System.Drawing.Imaging.ImageFormat]::Png)
            
            $font.Dispose()
            $brush.Dispose()
            $format.Dispose()
            $graphics.Dispose()
            $bitmap.Dispose()
            
            Write-Host "    Created $file ($width x $height)"
        }
    } catch {
        Write-Host "  System.Drawing not available, creating minimal PNG files..." -ForegroundColor Yellow
        
        # Create minimal valid PNG files (1x1 dark pixel, will be scaled by Windows)
        # PNG header + IHDR + IDAT + IEND for a 1x1 dark blue pixel
        $pngData = @(
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,  # PNG signature
            0x00, 0x00, 0x00, 0x0D,                          # IHDR length
            0x49, 0x48, 0x44, 0x52,                          # "IHDR"
            0x00, 0x00, 0x00, 0x01,                          # Width: 1
            0x00, 0x00, 0x00, 0x01,                          # Height: 1
            0x08, 0x02,                                      # Bit depth: 8, Color type: RGB
            0x00, 0x00, 0x00,                                # Compression, Filter, Interlace
            0x90, 0x77, 0x53, 0xDE,                          # IHDR CRC
            0x00, 0x00, 0x00, 0x0C,                          # IDAT length
            0x49, 0x44, 0x41, 0x54,                          # "IDAT"
            0x08, 0xD7, 0x63, 0x18, 0x19, 0x1B, 0x00, 0x00,  # Compressed data (dark pixel)
            0x00, 0x07, 0x00, 0x01,
            0x7D, 0x7F, 0xA6, 0x5D,                          # IDAT CRC (approximate)
            0x00, 0x00, 0x00, 0x00,                          # IEND length
            0x49, 0x45, 0x4E, 0x44,                          # "IEND"
            0xAE, 0x42, 0x60, 0x82                           # IEND CRC
        )
        
        $files = @("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png", "Wide310x150Logo.png")
        foreach ($file in $files) {
            $outputPath = Join-Path $AssetsDir $file
            [System.IO.File]::WriteAllBytes($outputPath, [byte[]]$pngData)
            Write-Host "    Created $file (placeholder)"
        }
        
        Write-Host ""
        Write-Host "  NOTE: Placeholder icons created. For production, replace with proper icons:" -ForegroundColor Yellow
        Write-Host "        $AssetsDir" -ForegroundColor Gray
    }
}

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
