# ==============================================================================
# zapret2 Desktop GUI - Upstream Binaries Setup Script
# ==============================================================================
$ErrorActionPreference = "Stop"

$BinDir = Join-Path $PSScriptRoot "..\bin"
$ListsDir = Join-Path $PSScriptRoot "..\lists"
$BinListsDir = Join-Path $BinDir "lists"

if (-not (Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
}
if (-not (Test-Path $ListsDir)) {
    New-Item -ItemType Directory -Path $ListsDir -Force | Out-Null
}
if (-not (Test-Path $BinListsDir)) {
    New-Item -ItemType Directory -Path $BinListsDir -Force | Out-Null
}

$RequiredFiles = @("winws2.exe", "winws.exe", "WinDivert.dll", "WinDivert64.sys", "cygwin1.dll")

# 1. Search local development paths first
$SearchLocations = @(
    (Join-Path $PSScriptRoot "..\zapret2 releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret2-releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\zapret-discord-youtube-main\bin"),
    (Join-Path $PSScriptRoot "..")
)

foreach ($Loc in $SearchLocations) {
    if (Test-Path $Loc) {
        foreach ($File in $RequiredFiles) {
            $TargetFile = Join-Path $BinDir $File
            if (-not (Test-Path $TargetFile)) {
                $CandidateFile = Join-Path $Loc $File
                if (Test-Path $CandidateFile) {
                    Copy-Item -Path $CandidateFile -Destination $TargetFile -Force
                    Write-Host "    [+] Copied $File from $Loc" -ForegroundColor Green
                }
            }
        }
    }
}

# 2. Check if winws2.exe, cygwin1.dll, or WinDivert are missing -> Download official bol-van/zapret2 release
$NeedsZapret2 = (-not (Test-Path (Join-Path $BinDir "winws2.exe"))) -or 
                (-not (Test-Path (Join-Path $BinDir "WinDivert.dll"))) -or 
                (-not (Test-Path (Join-Path $BinDir "WinDivert64.sys"))) -or 
                (-not (Test-Path (Join-Path $BinDir "cygwin1.dll")))

if ($NeedsZapret2) {
    Write-Host "==> Downloading official bol-van/zapret2 release from GitHub..." -ForegroundColor Cyan
    $Z2Zip = Join-Path $env:TEMP "zapret2-v1.0.5.2.zip"
    $Z2Dir = Join-Path $env:TEMP "zapret2-v1.0.5.2-extracted"
    $Z2Url = "https://github.com/bol-van/zapret2/releases/download/v1.0.5.2/zapret2-v1.0.5.2.zip"
    
    Invoke-WebRequest -Uri $Z2Url -OutFile $Z2Zip
    Expand-Archive -Path $Z2Zip -DestinationPath $Z2Dir -Force

    $Win64Bin = Get-ChildItem -Path $Z2Dir -Recurse -Directory | Where-Object { $_.FullName -like "*windows-x86_64" } | Select-Object -First 1
    if ($Win64Bin) {
        Get-ChildItem -Path $Win64Bin.FullName | ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $BinDir -Force
            Write-Host "    [+] Extracted $($_.Name) from zapret2 upstream" -ForegroundColor Green
        }
    }
}

# 3. Check if winws.exe, binary packet dumps (.bin), or lists are missing -> Download Flowseal release
$NeedsFlowseal = (-not (Test-Path (Join-Path $BinDir "winws.exe"))) -or 
                 (-not (Test-Path (Join-Path $BinDir "quic_initial_www_google_com.bin"))) -or 
                 (-not (Test-Path (Join-Path $BinDir "ACTIVE_DISCORD_UDP.bin")))

if ($NeedsFlowseal) {
    Write-Host "==> Downloading upstream Flowseal preset bundle from GitHub..." -ForegroundColor Cyan
    $FsZip = Join-Path $env:TEMP "zapret-flowseal.zip"
    $FsDir = Join-Path $env:TEMP "zapret-flowseal-extracted"
    $FsUrl = "https://github.com/Flowseal/zapret-discord-youtube/releases/download/1.10.3/zapret-discord-youtube-1.10.3.zip"
    
    Invoke-WebRequest -Uri $FsUrl -OutFile $FsZip
    Expand-Archive -Path $FsZip -DestinationPath $FsDir -Force
    
    $FoundBin = Get-ChildItem -Path $FsDir -Recurse -Directory -Filter "bin" | Select-Object -First 1
    if ($FoundBin) {
        Get-ChildItem -Path $FoundBin.FullName | ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $BinDir -Force
        }
    }

    $FoundLists = Get-ChildItem -Path $FsDir -Recurse -Directory -Filter "lists" | Select-Object -First 1
    if ($FoundLists) {
        Get-ChildItem -Path $FoundLists.FullName | ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $ListsDir -Force
            Copy-Item -Path $_.FullName -Destination $BinListsDir -Force
        }
    }
}

# Ensure root copies exist for direct access
if (Test-Path (Join-Path $BinDir "WinDivert.dll")) { Copy-Item (Join-Path $BinDir "WinDivert.dll") (Join-Path $PSScriptRoot "..\WinDivert.dll") -Force }
if (Test-Path (Join-Path $BinDir "WinDivert64.sys")) { Copy-Item (Join-Path $BinDir "WinDivert64.sys") (Join-Path $PSScriptRoot "..\WinDivert64.sys") -Force }
if (Test-Path (Join-Path $BinDir "cygwin1.dll")) { Copy-Item (Join-Path $BinDir "cygwin1.dll") (Join-Path $PSScriptRoot "..\cygwin1.dll") -Force }
if (Test-Path (Join-Path $BinDir "winws2.exe")) { Copy-Item (Join-Path $BinDir "winws2.exe") (Join-Path $PSScriptRoot "..\winws2.exe") -Force }

# Verify files and distinct binaries
$w1 = Join-Path $BinDir "winws.exe"
$w2 = Join-Path $BinDir "winws2.exe"
if ((Test-Path $w1) -and (Test-Path $w2)) {
    $s1 = (Get-Item $w1).Length
    $s2 = (Get-Item $w2).Length
    Write-Host "==> Verified winws.exe ($s1 bytes) and winws2.exe ($s2 bytes)" -ForegroundColor Green
    if ($s1 -eq $s2) {
        Write-Warning "winws.exe and winws2.exe have identical sizes! winws2 should be distinct bol-van/zapret2 binary."
    }
}

Write-Host "==> Upstream binaries setup and validation completed successfully!" -ForegroundColor Green
