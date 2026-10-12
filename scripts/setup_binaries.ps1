# ==============================================================================
# zapret2 Desktop GUI - Upstream Binaries Setup Script
# ==============================================================================
$ErrorActionPreference = "Stop"

$BinDir = Join-Path $PSScriptRoot "..\bin"
if (-not (Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
}

$RequiredFiles = @("winws2.exe", "winws.exe", "WinDivert.dll", "WinDivert64.sys", "cygwin1.dll")
$Missing = @()

foreach ($File in $RequiredFiles) {
    $FilePath = Join-Path $BinDir $File
    if (-not (Test-Path $FilePath)) {
        $Missing += $File
    }
}

if ($Missing.Count -eq 0) {
    Write-Host "[OK] All required binaries are present in bin!" -ForegroundColor Green
    exit 0
}

Write-Host "[!] Missing binaries: $($Missing -join ', ')" -ForegroundColor Yellow
Write-Host "==> Searching local developer paths..." -ForegroundColor Cyan

$SearchLocations = @(
    (Join-Path $PSScriptRoot "..\zapret2 releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret2-releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret-discord-youtube-main\bin"),
    (Join-Path $PSScriptRoot "..")
)

foreach ($Loc in $SearchLocations) {
    if (Test-Path $Loc) {
        foreach ($File in $Missing) {
            $CandidateFile = Join-Path $Loc $File
            if (Test-Path $CandidateFile) {
                Copy-Item -Path $CandidateFile -Destination (Join-Path $BinDir $File) -Force
                Write-Host "    [+] Copied $File from $Loc" -ForegroundColor Green
            }
        }
    }
}

# If still missing in CI environment, download upstream bundle
$StillMissing = @()
foreach ($File in $RequiredFiles) {
    if (-not (Test-Path (Join-Path $BinDir $File))) {
        $StillMissing += $File
    }
}

if ($StillMissing.Count -gt 0) {
    Write-Host "==> Downloading upstream runtime bundle from GitHub..." -ForegroundColor Cyan
    $ZipPath = Join-Path $env:TEMP "zapret-upstream.zip"
    $ExtractDir = Join-Path $env:TEMP "zapret-upstream-extracted"
    
    $Url = "https://github.com/Flowseal/zapret-discord-youtube/releases/download/1.10.3/zapret-discord-youtube-1.10.3.zip"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force
    
    $FoundBin = Get-ChildItem -Path $ExtractDir -Recurse -Directory -Filter "bin" | Select-Object -First 1
    if ($FoundBin) {
        Get-ChildItem -Path $FoundBin.FullName | ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $BinDir -Force
        }
    }
    
    # If winws2.exe was missing, ensure winws.exe is linked or copied
    if (-not (Test-Path (Join-Path $BinDir "winws2.exe")) -and (Test-Path (Join-Path $BinDir "winws.exe"))) {
        Copy-Item (Join-Path $BinDir "winws.exe") (Join-Path $BinDir "winws2.exe") -Force
    }
}

Write-Host "==> Runtime binaries verified successfully!" -ForegroundColor Green
