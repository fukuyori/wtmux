# signing.ps1
# Shared code-signing helpers, dot-sourced by the packaging scripts.
#
# Requirements when signing:
#   - CODESIGN_CERT environment variable = subject name of the certificate
#     in the current user's certificate store (signtool /n)
#   - signtool.exe (Windows SDK)

$script:DefaultTimestampUrl = "http://timestamp.sectigo.com"

function Find-SignTool {
    # Prefer the newest native-architecture Windows SDK signtool, then x64,
    # and finally fall back to PATH. Hardware-token KSPs may only be visible
    # to a native-architecture process (notably on Windows ARM64).
    $nativeArchitecture = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq 'Arm64') {
        'arm64'
    } else {
        'x64'
    }
    $candidates = @(
        Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\$nativeArchitecture\signtool.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName |
            Select-Object -Last 1 -ExpandProperty FullName
        Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\signtool.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName |
            Select-Object -Last 1 -ExpandProperty FullName
    )
    $inPath = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($inPath) { $candidates += $inPath.Source }
    return ($candidates | Where-Object { $_ } | Select-Object -First 1)
}

function Assert-SignPrereqs {
    if (-not $env:CODESIGN_CERT) {
        Write-Host "Error: CODESIGN_CERT environment variable is not set" -ForegroundColor Red
        Write-Host "Set it to the subject name of your code-signing certificate" -ForegroundColor Yellow
        exit 1
    }
    $signTool = Find-SignTool
    if (-not $signTool) {
        Write-Host "Error: signtool.exe not found (install the Windows SDK)" -ForegroundColor Red
        exit 1
    }
    return $signTool
}

function Invoke-CodeSign {
    param(
        [Parameter(Mandatory)][string]$SignTool,
        [Parameter(Mandatory)][string]$Path,
        [string]$TimestampUrl = $script:DefaultTimestampUrl,
        # Skip files that already carry a valid Authenticode signature
        # (e.g. an exe that was signed manually before packaging)
        [switch]$SkipIfSigned
    )
    if ($SkipIfSigned) {
        $sig = Get-AuthenticodeSignature $Path
        if ($sig.Status -eq "Valid") {
            Write-Host "Already signed, skipping: $Path" -ForegroundColor Gray
            return
        }
    }
    Write-Host "Signing: $Path" -ForegroundColor Green
    & $SignTool sign /n $env:CODESIGN_CERT /fd SHA256 /tr $TimestampUrl /td SHA256 $Path
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: signtool failed for $Path" -ForegroundColor Red
        exit 1
    }
}
