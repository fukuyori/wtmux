
# build-installer.ps1
# Build wtmux MSI installer using WiX Toolset
#
# Prerequisites:
#   1. Install WiX Toolset from https://wixtoolset.org/releases/
#   2. Build wtmux in release mode first: cargo build --release
#
# -Sign: code-sign the bundled wtmux.exe (if not already signed) and the MSI,
#        using the certificate named by CODESIGN_CERT.
#        (MSI has no separate uninstaller executable — uninstall runs through
#        the signed MSI itself.)

param(
    [string]$Version = "",
    [string]$OutputDir = ".\installer\output",
    [switch]$Sign,
    [string]$TimestampUrl = ""
)

$ErrorActionPreference = "Stop"

# Run from the repository root regardless of invocation directory
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "=== wtmux Installer Build Script ===" -ForegroundColor Cyan

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

# Find WiX tools - check for v7/v6/v5 (wix.exe), then v3.x (candle.exe/light.exe)
$wixPaths = @(
    "${env:ProgramFiles}\WiX Toolset v7.0\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v7.0\bin",
    "${env:ProgramFiles}\WiX Toolset v6.0\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v6.0\bin",
    "${env:ProgramFiles}\WiX Toolset v5.0\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v5.0\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v3.14\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v3.11\bin",
    "${env:ProgramFiles(x86)}\WiX Toolset v3.10\bin",
    "${env:WIX}bin"
)

$wixBin = $null
$wixVersion = 0

# Check for wix.exe (v4+/v5+/v6+/v7+)
foreach ($path in $wixPaths) {
    if ($path -and (Test-Path "$path\wix.exe")) {
        $wixBin = $path
        if ($path -match 'WiX Toolset v7\.0') {
            $wixVersion = 7
        } elseif ($path -match 'WiX Toolset v6\.0') {
            $wixVersion = 6
        } elseif ($path -match 'WiX Toolset v5\.0') {
            $wixVersion = 5
        } else {
            $wixVersion = 4
        }
        break
    }
}

# Check for candle.exe (v3.x)
if (-not $wixBin) {
    foreach ($path in $wixPaths) {
        if ($path -and (Test-Path "$path\candle.exe")) {
            $wixBin = $path
            $wixVersion = 3
            break
        }
    }
}

# Also check PATH
if (-not $wixBin) {
    $wixExe = Get-Command "wix.exe" -ErrorAction SilentlyContinue
    if ($wixExe) {
        $wixBin = Split-Path $wixExe.Source
        if ($wixBin -match 'WiX Toolset v7\.0') {
            $wixVersion = 7
        } elseif ($wixBin -match 'WiX Toolset v6\.0') {
            $wixVersion = 6
        } elseif ($wixBin -match 'WiX Toolset v5\.0') {
            $wixVersion = 5
        } else {
            $wixVersion = 4
        }
    } else {
        $candle = Get-Command "candle.exe" -ErrorAction SilentlyContinue
        if ($candle) {
            $wixBin = Split-Path $candle.Source
            $wixVersion = 3
        }
    }
}

if (-not $wixBin) {
    Write-Host "Error: WiX Toolset not found" -ForegroundColor Red
    Write-Host "Searched locations:" -ForegroundColor Yellow
    foreach ($path in $wixPaths) {
        if ($path) {
            Write-Host "  - $path" -ForegroundColor Gray
        }
    }
    Write-Host ""
    Write-Host "Please install WiX Toolset from https://wixtoolset.org/releases/" -ForegroundColor Yellow
    exit 1
}

Write-Host "Using WiX Toolset v$wixVersion : $wixBin" -ForegroundColor Gray

# Packaging never builds: require an existing release build
$exePath = ".\target\release\wtmux.exe"
if (-not (Test-Path $exePath)) {
    Write-Host "Error: wtmux.exe not found at $exePath" -ForegroundColor Red
    Write-Host "Please build first: cargo build --release" -ForegroundColor Yellow
    exit 1
}

# Generate icon assets
$iconScript = Join-Path $PSScriptRoot "generate-icons.ps1"
if (Test-Path $iconScript) {
    Write-Host "Generating icon assets..." -ForegroundColor Green
    try {
        & $iconScript
    } catch {
        Write-Host "Error: Icon generation failed" -ForegroundColor Red
        exit 1
    }
}

# Create output directory
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir | Out-Null
}

# Create staging directory
$stagingDir = ".\installer\staging"
if (Test-Path $stagingDir) {
    Remove-Item -Recurse -Force $stagingDir
}
New-Item -ItemType Directory -Path $stagingDir | Out-Null

# Copy files to staging
Write-Host "Copying files to staging..." -ForegroundColor Green
# Sign the executable in place before staging, so every package format reuses
# one signature (= one token PIN prompt); skipped when already validly signed
if ($Sign) {
    Invoke-CodeSign -SignTool $signTool -Path $exePath -TimestampUrl $TimestampUrl -SkipIfSigned
}

Copy-Item $exePath "$stagingDir\wtmux.exe"
Copy-Item ".\installer\license.rtf" "$stagingDir\license.rtf"
Copy-Item ".\assets\generated\wtmux.ico" "$stagingDir\wtmux.ico"

$msiPath = "$OutputDir\wtmux-$Version-x64.msi"

if ($wixVersion -ge 4) {
    # WiX v4/v5/v6 - use wix.exe
    Write-Host "Building MSI with WiX v$wixVersion..." -ForegroundColor Green
    
    # Create WiX v4+ compatible wxs file
    $wxsV4Path = "$stagingDir\wtmux-v4.wxs"
    $wxsV4Content = @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs"
     xmlns:ui="http://wixtoolset.org/schemas/v4/wxs/ui">
    <Package Name="wtmux"
             Version="$Version"
             Manufacturer="wtmux"
             UpgradeCode="A1B2C3D4-E5F6-7890-ABCD-EF1234567890"
             Scope="perMachine">
        
        <MajorUpgrade DowngradeErrorMessage="A newer version of wtmux is already installed." />
        <MediaTemplate EmbedCab="yes" />
        <Icon Id="AppIcon" SourceFile="wtmux.ico" />
        <Property Id="ARPPRODUCTICON" Value="AppIcon" />
        
        <StandardDirectory Id="ProgramFiles64Folder">
            <Directory Id="INSTALLFOLDER" Name="wtmux">
                <Directory Id="BinFolder" Name="bin">
                    <Component Id="WtmuxExe" Guid="B2C3D4E5-F6A7-8901-BCDE-F12345678901">
                        <File Id="WtmuxExeFile" Source="wtmux.exe" />
                    </Component>
                </Directory>
            </Directory>
        </StandardDirectory>
        <StandardDirectory Id="ProgramMenuFolder">
            <Directory Id="ProgramMenuDir" Name="wtmux" />
        </StandardDirectory>
        
        <Component Id="PathEnvComponent" Directory="INSTALLFOLDER" Guid="C3D4E5F6-A7B8-9012-CDEF-123456789012">
            <Environment Id="PathEnv" 
                         Name="PATH" 
                         Value="[BinFolder]" 
                         Permanent="no" 
                         Part="last" 
                         Action="set" 
                         System="yes" />
        </Component>

        <Component Id="StartMenuShortcutComponent" Directory="ProgramMenuDir" Guid="D4E5F6A7-B8C9-0123-DEF0-234567890123">
            <Shortcut Id="ApplicationStartMenuShortcut"
                      Name="wtmux"
                      Description="Launch wtmux"
                      Target="[#WtmuxExeFile]"
                      WorkingDirectory="BinFolder" />
            <RemoveFolder Id="ProgramMenuDirRemove" On="uninstall" />
            <RegistryValue Root="HKLM"
                           Key="Software\wtmux"
                           Name="StartMenuShortcut"
                           Type="integer"
                           Value="1"
                           KeyPath="yes" />
        </Component>
        
        <Feature Id="ProductFeature" Title="wtmux">
            <ComponentRef Id="WtmuxExe" />
            <ComponentRef Id="PathEnvComponent" />
            <ComponentRef Id="StartMenuShortcutComponent" />
        </Feature>
        
        <ui:WixUI Id="WixUI_Minimal" />
    </Package>
</Wix>
"@
    $wxsV4Content | Out-File -FilePath $wxsV4Path -Encoding UTF8
    
    Push-Location $stagingDir
    try {
        if ($wixVersion -ge 7) {
            Write-Host "Accepting WiX v7 OSMF EULA for this environment..." -ForegroundColor DarkYellow
            & "$wixBin\wix.exe" eula accept wix7
            if ($LASTEXITCODE -ne 0) {
                Write-Host "Error: wix eula accept failed" -ForegroundColor Red
                exit 1
            }
        }
        $wixArgs = @(
            "build",
            "-arch", "x64",
            "-o", "..\output\wtmux-$Version-x64.msi",
            "wtmux-v4.wxs",
            "-ext", "WixToolset.UI.wixext"
        )
        & "$wixBin\wix.exe" @wixArgs
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: wix build failed" -ForegroundColor Red
            exit 1
        }
    } finally {
        Pop-Location
    }
} else {
    # WiX v3.x - use candle.exe and light.exe
    $candle = "$wixBin\candle.exe"
    $light = "$wixBin\light.exe"
    
    # Update version in wxs file
    $wxsContent = Get-Content ".\installer\wtmux.wxs" -Raw
    $wxsContent = $wxsContent -replace 'Version="[0-9.]+"', "Version=`"$Version`""
    $wxsContent | Set-Content ".\installer\wtmux.wxs"

    # Compile WiX source
    Write-Host "Compiling WiX source..." -ForegroundColor Green
    $wixobjPath = "$stagingDir\wtmux.wixobj"
    & $candle -nologo -arch x64 -dSourceDir="$stagingDir" -out $wixobjPath ".\installer\wtmux.wxs"
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: candle.exe failed" -ForegroundColor Red
        exit 1
    }

    # Link to create MSI
    Write-Host "Linking MSI..." -ForegroundColor Green
    & $light -nologo -ext WixUIExtension -out $msiPath $wixobjPath
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: light.exe failed" -ForegroundColor Red
        exit 1
    }
}

# Sign the MSI itself
if ($Sign) {
    Invoke-CodeSign -SignTool $signTool -Path $msiPath -TimestampUrl $TimestampUrl
}

# Cleanup
Remove-Item -Recurse -Force $stagingDir

Write-Host ""
Write-Host "=== Build Complete ===" -ForegroundColor Cyan
Write-Host "Installer created: $msiPath" -ForegroundColor Green
Write-Host ""
Write-Host "To install: msiexec /i `"$msiPath`"" -ForegroundColor Yellow
Write-Host "To uninstall: msiexec /x `"$msiPath`"" -ForegroundColor Yellow
