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

# ==============================================================================
# PINNED UPSTREAM HASHES (supply-chain hardening, see SECURITY.md H-1)
# ------------------------------------------------------------------------------
# These SHA256 hashes pin the exact upstream release archives. The script
# aborts if a downloaded archive does not match its pinned hash, so a
# compromised upstream release asset cannot silently slip a trojaned
# driver/exe into our build.
#
# HOW TO UPDATE when upstream publishes a new version:
#   1. Download the new archive manually from the official release page.
#   2. Compute its hash:  Get-FileHash <file>.zip -Algorithm SHA256
#   3. Sanity-check it independently (second download, publisher's published
#      checksum if any, VirusTotal, community reports).
#   4. Update Url + Sha256 (+ file name where used) below in ONE commit and
#      mention the upstream version in the commit message.
# ==============================================================================
$UpstreamPins = @{
    "zapret2" = @{
        Label  = "bol-van/zapret2 v1.0.5.2"
        Url    = "https://github.com/bol-van/zapret2/releases/download/v1.0.5.2/zapret2-v1.0.5.2.zip"
        Sha256 = "f585590bea6da82ac2c74926f6ac17e6204ee8b53e455c2ba2edae0c58c5d4ac"
    }
    "flowseal" = @{
        Label  = "Flowseal/zapret-discord-youtube 1.10.3"
        Url    = "https://github.com/Flowseal/zapret-discord-youtube/releases/download/1.10.3/zapret-discord-youtube-1.10.3.zip"
        Sha256 = "244314ae1c24538a0d751601da8e0c925c843371eec4456eb15f14c4fd6b7058"
    }
}

function Test-PinnedHash {
    param([string]$Path, [string]$ExpectedSha256, [string]$Label)
    $actual = (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        throw ("SECURITY: SHA256 mismatch for {0}!`n  expected: {1}`n  actual:   {2}`n" +
               "The upstream archive may have been tampered with. Aborting build.") -f $Label, $ExpectedSha256, $actual
    }
    Write-Host "    [+] SHA256 verified: $Label" -ForegroundColor Green
}

function Test-Authenticode {
    param([string]$Path)
    $fileName = [System.IO.Path]::GetFileName($Path)
    $sig = Get-AuthenticodeSignature -FilePath $Path
    switch ($sig.Status) {
        "Valid" {
            Write-Host "    [+] Authenticode valid: $fileName ($($sig.SignerCertificate.Subject))" -ForegroundColor Green
        }
        "NotSigned" {
            # Upstream does not sign every binary; a missing signature is a warning,
            # not a build break - the pinned SHA256 above is the hard guarantee.
            Write-Warning "Authenticode: $fileName is not signed by upstream."
        }
        default {
            throw ("SECURITY: Authenticode check failed for {0} (status: {1}). Aborting build." -f $fileName, $sig.Status)
        }
    }
}

# 1. Search local development paths first
$SearchLocations = @(
    (Join-Path $PSScriptRoot "..\vendor\zapret2 releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\vendor\zapret2-releases\binaries\windows-x86_64"),
    (Join-Path $PSScriptRoot "..\vendor\zapret-discord-youtube-main\bin"),
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
    $Pin = $UpstreamPins["zapret2"]
    Write-Host ("==> Downloading {0} from GitHub..." -f $Pin.Label) -ForegroundColor Cyan
    $Z2Zip = Join-Path $env:TEMP "zapret2-v1.0.5.2.zip"
    $Z2Dir = Join-Path $env:TEMP "zapret2-v1.0.5.2-extracted"

    Invoke-WebRequest -Uri $Pin.Url -OutFile $Z2Zip
    Test-PinnedHash -Path $Z2Zip -ExpectedSha256 $Pin.Sha256 -Label $Pin.Label
    Expand-Archive -Path $Z2Zip -DestinationPath $Z2Dir -Force

    $Win64Bin = Get-ChildItem -Path $Z2Dir -Recurse -Directory | Where-Object { $_.FullName -like "*windows-x86_64" } | Select-Object -First 1
    if ($Win64Bin) {
        Get-ChildItem -Path $Win64Bin.FullName | ForEach-Object {
            Copy-Item -Path $_.FullName -Destination $BinDir -Force
            Write-Host "    [+] Extracted $($_.Name) from zapret2 upstream" -ForegroundColor Green
        }
    }

    # Authenticode sanity check on the security-critical binaries (warn if unsigned, fail if tampered)
    foreach ($Bin in @("winws2.exe", "WinDivert.dll", "WinDivert64.sys")) {
        $P = Join-Path $BinDir $Bin
        if (Test-Path $P) { Test-Authenticode -Path $P }
    }
}

# 3. Check if winws.exe, binary packet dumps (.bin), or lists are missing -> Download Flowseal release
$NeedsFlowseal = (-not (Test-Path (Join-Path $BinDir "winws.exe"))) -or 
                 (-not (Test-Path (Join-Path $BinDir "quic_initial_www_google_com.bin"))) -or 
                 (-not (Test-Path (Join-Path $BinDir "ACTIVE_DISCORD_UDP.bin")))

if ($NeedsFlowseal) {
    $Pin = $UpstreamPins["flowseal"]
    Write-Host ("==> Downloading {0} from GitHub..." -f $Pin.Label) -ForegroundColor Cyan
    $FsZip = Join-Path $env:TEMP "zapret-flowseal.zip"
    $FsDir = Join-Path $env:TEMP "zapret-flowseal-extracted"

    Invoke-WebRequest -Uri $Pin.Url -OutFile $FsZip
    Test-PinnedHash -Path $FsZip -ExpectedSha256 $Pin.Sha256 -Label $Pin.Label
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

    # Authenticode sanity check on the Flowseal engine binary
    $W1 = Join-Path $BinDir "winws.exe"
    if (Test-Path $W1) { Test-Authenticode -Path $W1 }
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
    Write-Host ("==> Verified winws.exe ({0} bytes) and winws2.exe ({1} bytes)" -f $s1, $s2) -ForegroundColor Green
    if ($s1 -eq $s2) {
        Write-Warning "winws.exe and winws2.exe have identical sizes! winws2 should be distinct bol-van/zapret2 binary."
    }
}

Write-Host "==> Upstream binaries setup and validation completed successfully!" -ForegroundColor Green
