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
        Label  = "bol-van/zapret2 v1.0.7"
        Url    = "https://github.com/bol-van/zapret2/releases/download/v1.0.7/zapret2-v1.0.7.zip"
        Sha256 = "35e14e4e1f345b870c3b87269fa45353efd597241a25304d38375c46b9fd2967"
    }
    "flowseal" = @{
        Label  = "Flowseal/zapret-discord-youtube 1.10.4"
        Url    = "https://github.com/Flowseal/zapret-discord-youtube/releases/download/1.10.4/zapret-discord-youtube-1.10.4.zip"
        Sha256 = "291af673bd0238506f9548e0e94ca0abf244000f4c9a06960544fb01c5f97b34"
    }
    "singbox" = @{
        Label  = "shtorm-7/sing-box-extended v1.14.3-extended-2.7.3"
        Url    = "https://github.com/shtorm-7/sing-box-extended/releases/download/v1.14.3-extended-2.7.3/sing-box-1.14.3-extended-2.7.3-windows-amd64.zip"
        Sha256 = "ab7a7bddd51524854b853da1e72a8b94626d56aee0acfca46239146df6c099c7"
    }
    "wintun" = @{
        Label  = "Wintun 0.14.1"
        Url    = "https://www.wintun.net/builds/wintun-0.14.1.zip"
        Sha256 = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51"
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
    $Z2Zip = Join-Path $env:TEMP "zapret2-v1.0.7.zip"
    $Z2Dir = Join-Path $env:TEMP "zapret2-v1.0.7-extracted"

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
        # The repository's lists are curated (extra domains, fixes) and are the
        # source of truth: only fill in files that are missing locally, never
        # overwrite tracked lists with the upstream Flowseal snapshot.
        Get-ChildItem -Path $FoundLists.FullName | ForEach-Object {
            foreach ($DestDir in @($ListsDir, $BinListsDir)) {
                $Target = Join-Path $DestDir $_.Name
                if (-not (Test-Path $Target)) {
                    Copy-Item -Path $_.FullName -Destination $DestDir
                }
            }
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

# 4. Optional: winws2-split.exe from our own fork (mqclass/zapret2).
#    This is the split-enforcing winws2 build the supervisor launches when
#    per-app split is enabled on a winws2 strategy. It is OPTIONAL: the
#    supervisor falls back to plain winws2.exe while the split binary is
#    absent, so this step must never fail the build before the fork ships.
#    HOW TO PIN after the first fork build:
#      1. Publish the fork build as a release asset (or a pinned commit
#         artifact) and note its download URL below.
#      2. Compute its hash:  Get-FileHash winws2-split.exe -Algorithm SHA256
#      3. Fill $SplitPinUrl + $SplitPinSha256 in ONE commit and mention the
#         fork commit in the commit message.
#    Until then both values stay empty and this block is skipped with a
#    warning, keeping CI green.
$SplitPinUrl = "https://github.com/mqclass/zapret2/releases/download/winws2-split-v1.0.7/winws2-split.exe"
$SplitPinSha256 = "24ebd97067cdedaf92dd3f956d227ee13cdd8e8f875ab9481885e39453ad45ff"
$SplitTarget = Join-Path $BinDir "winws2-split.exe"
if (-not (Test-Path $SplitTarget)) {
    if ([string]::IsNullOrWhiteSpace($SplitPinUrl) -or [string]::IsNullOrWhiteSpace($SplitPinSha256)) {
        Write-Warning "winws2-split.exe: fork pin not filled yet (first mqclass/zapret2 build pending) - skipping, the build continues without the split engine."
    } else {
        Write-Host "==> Downloading winws2-split.exe from the mqclass/zapret2 fork..." -ForegroundColor Cyan
        Invoke-WebRequest -Uri $SplitPinUrl -OutFile $SplitTarget
        Test-PinnedHash -Path $SplitTarget -ExpectedSha256 $SplitPinSha256 -Label "mqclass/zapret2 winws2-split.exe"
        Test-Authenticode -Path $SplitTarget
    }
} elseif (-not [string]::IsNullOrWhiteSpace($SplitPinUrl) -and -not [string]::IsNullOrWhiteSpace($SplitPinSha256)) {
    # Pin filled and a split binary is already installed: verify the file
    # on disk too, not just fresh downloads, so a stale or swapped binary
    # fails the build with the same SECURITY error as a bad download.
    # With an empty pin this branch is skipped, as before.
    Test-PinnedHash -Path $SplitTarget -ExpectedSha256 $SplitPinSha256 -Label "mqclass/zapret2 winws2-split.exe"
}

# 5. sing-box.exe from the shtorm-7 fork (WARP/MASQUE sidecar, stage 1 of
#    the WARP mode). The binary is only SHIPPED here, never launched: the
#    supervisor does not spawn it until stage 3. A missing sing-box FAILS
#    the build (design section 11): a release without the promised WARP
#    engine must not ship silently.
#    The fork publishes no checksums or signatures for its releases
#    (upstream "unsigned"), so the pinned hashes below are OUR OWN pins,
#    computed from the release asset and cross-checked at audit time
#    (see docs/THIRD-PARTY.md). Update both hashes in ONE commit when the
#    fork pin moves, and mention the fork version in the commit message.
$SingBoxExeSha256 = "c8395a7db22ce321a10132aead5c9a2201adc4ae6847c47ccc5fff5200c1ac07"
$SingBoxTarget = Join-Path $BinDir "sing-box.exe"
if (-not (Test-Path $SingBoxTarget)) {
    $Pin = $UpstreamPins["singbox"]
    Write-Host ("==> Downloading {0} from GitHub..." -f $Pin.Label) -ForegroundColor Cyan
    $SbZip = Join-Path $env:TEMP "sing-box-1.14.3-extended-2.7.3-windows-amd64.zip"
    $SbDir = Join-Path $env:TEMP "sing-box-1.14.3-extended-2.7.3-extracted"

    Invoke-WebRequest -Uri $Pin.Url -OutFile $SbZip
    Test-PinnedHash -Path $SbZip -ExpectedSha256 $Pin.Sha256 -Label $Pin.Label
    # Clear any previous extraction: Expand-Archive -Force does not remove
    # files from an older pin, and the name search below would then see two
    # exe files and could pick the stale one.
    if (Test-Path $SbDir) { Remove-Item $SbDir -Recurse -Force }
    Expand-Archive -Path $SbZip -DestinationPath $SbDir -Force

    # The zip wraps the exe in a versioned top directory; find it by name
    # rather than hardcoding the directory, so a pin bump only touches the
    # version strings above and $UpstreamPins.
    $SbExe = Get-ChildItem -Path $SbDir -Recurse -Filter "sing-box.exe" | Select-Object -First 1
    if (-not $SbExe) {
        throw "sing-box.exe not found inside the downloaded archive. Aborting build."
    }
    Copy-Item -Path $SbExe.FullName -Destination $SingBoxTarget -Force
    Write-Host "    [+] Extracted sing-box.exe (108 MB) from the shtorm-7 fork release" -ForegroundColor Green

    # Second check: pin the exact exe, not just the archive (same
    # double-check pattern as nfqws2 in setup_binaries.sh).
    Test-PinnedHash -Path $SingBoxTarget -ExpectedSha256 $SingBoxExeSha256 -Label "sing-box.exe (shtorm-7/sing-box-extended v1.14.3-extended-2.7.3)"
    # The fork does not sign its builds; a missing signature is a warning,
    # a tampered one fails the build.
    Test-Authenticode -Path $SingBoxTarget
} else {
    # Already installed: verify the file on disk against the pin, not just
    # fresh downloads (same as the split-pin behaviour above).
    Test-PinnedHash -Path $SingBoxTarget -ExpectedSha256 $SingBoxExeSha256 -Label "sing-box.exe (shtorm-7/sing-box-extended v1.14.3-extended-2.7.3)"
}

# 6. wintun.dll 0.14.1 (WireGuard LLC) - the signed TUN driver library the
#    sing-box TUN inbound loads at runtime (stage 3). Only the amd64 DLL is
#    installed; the other architectures in the archive are not needed.
#    The archive hash below is the SHA2-256 published on wintun.net.
$WintunDllSha256 = "e5da8447dc2c320edc0fc52fa01885c103de8c118481f683643cacc3220dafce"
$WintunTarget = Join-Path $BinDir "wintun.dll"
if (-not (Test-Path $WintunTarget)) {
    $Pin = $UpstreamPins["wintun"]
    Write-Host ("==> Downloading {0}..." -f $Pin.Label) -ForegroundColor Cyan
    $WtZip = Join-Path $env:TEMP "wintun-0.14.1.zip"
    $WtDir = Join-Path $env:TEMP "wintun-0.14.1-extracted"

    Invoke-WebRequest -Uri $Pin.Url -OutFile $WtZip
    Test-PinnedHash -Path $WtZip -ExpectedSha256 $Pin.Sha256 -Label $Pin.Label
    Expand-Archive -Path $WtZip -DestinationPath $WtDir -Force

    $WtDll = Join-Path $WtDir "wintun/bin/amd64/wintun.dll"
    if (-not (Test-Path $WtDll)) {
        throw "wintun/bin/amd64/wintun.dll not found inside the downloaded archive. Aborting build."
    }
    Copy-Item -Path $WtDll -Destination $WintunTarget -Force
    Write-Host "    [+] Extracted wintun.dll (amd64) from the Wintun 0.14.1 release" -ForegroundColor Green

    Test-PinnedHash -Path $WintunTarget -ExpectedSha256 $WintunDllSha256 -Label "wintun.dll (Wintun 0.14.1 amd64)"
    # The DLL is Microsoft-signed by WireGuard LLC; a valid signature is expected.
    Test-Authenticode -Path $WintunTarget
} else {
    Test-PinnedHash -Path $WintunTarget -ExpectedSha256 $WintunDllSha256 -Label "wintun.dll (Wintun 0.14.1 amd64)"
}

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

# Verify the WARP sidecar binaries are in place (design section 11: a build
# without them must fail, not ship silently).
$sb = Join-Path $BinDir "sing-box.exe"
$wt = Join-Path $BinDir "wintun.dll"
if (-not (Test-Path $sb)) { throw "sing-box.exe is missing from bin/ after setup. Aborting build." }
if (-not (Test-Path $wt)) { throw "wintun.dll is missing from bin/ after setup. Aborting build." }
Write-Host ("==> Verified sing-box.exe ({0} bytes) and wintun.dll ({1} bytes)" -f (Get-Item $sb).Length, (Get-Item $wt).Length) -ForegroundColor Green

Write-Host "==> Upstream binaries setup and validation completed successfully!" -ForegroundColor Green
