# ==============================================================================
# zapret2 Desktop GUI - Upstream Binaries Setup Script
# ==============================================================================
# This script ensures that winws2.exe, WinDivert.dll, and WinDivert64.sys
# are properly present in the ./bin directory for local development and runtime.
# ==============================================================================

$ErrorActionPreference = "Stop"

$BinDir = Join-Path $PSScriptRoot "..\bin"
if (-not (Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
}

Write-Host "==> Checking runtime binaries in $BinDir..." -ForegroundColor Cyan

$RequiredFiles = @("winws2.exe", "WinDivert.dll", "WinDivert64.sys")
$Missing = @()

foreach ($File in $RequiredFiles) {
    $FilePath = Join-Path $BinDir $File
    if (-not (Test-Path $FilePath)) {
        $Missing += $File
    }
}

if ($Missing.Count -eq 0) {
    Write-Host "[OK] All required binaries are present!" -ForegroundColor Green
    exit 0
}

Write-Host "[!] Missing binaries: $($Missing -join ', ')" -ForegroundColor Yellow
Write-Host "==> Looking for local releases or upstream archives..." -ForegroundColor Cyan

$SearchLocations = @(
    (Join-Path $PSScriptRoot "..\zapret2 releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret2-releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret-discord-youtube-main\bin")
)

foreach ($Loc in $SearchLocations) {
    if (Test-Path $Loc) {
        Write-Host "--> Found candidate directory: $Loc" -ForegroundColor Gray
        foreach ($File in $Missing) {
            $CandidateFile = Join-Path $Loc $File
            if (Test-Path $CandidateFile) {
                Copy-Item -Path $CandidateFile -Destination (Join-Path $BinDir $File) -Force
                Write-Host "    [+] Copied $File from $Loc" -ForegroundColor Green
            }
        }
    }
}

Write-Host "==> Setup completed successfully!" -ForegroundColor Green
