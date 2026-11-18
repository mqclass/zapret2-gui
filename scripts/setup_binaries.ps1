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

# 5. Hidden driver (Monkey): a full copy of bin/ with the renamed
#    WinDivert pair, so the engine can run with the driver masked as
#    "Monkey" for game anti-cheats. The engine spawns with CWD = its
#    bin dir, so every sibling file (.lua, .bin, split engine) must be
#    present here too - hence the whole-directory mirror, not just the
#    exes. The lists/ subdirectory is NOT mirrored: both modes share
#    the stock bin/lists/ (see resolve_binaries), so learned domains
#    and user edits stay in one place and the updater keeps backing
#    them up.
#
#    The renamed pair is BUILT from WinDivert sources, never downloaded
#    as a binary blob: WinDivert is LGPLv3, and shipping a modified
#    binary without sources would not comply. The build pins the
#    WinDivert v2.2.2 tag (same version as the stock driver), applies
#    our rename patch (scripts/windivert-monkey.patch - service
#    "Monkey", file Monkey64.sys, device name unchanged), and compiles
#    only the userspace DLL with MSBuild (no WDK needed; the toolset
#    is forced to v143 because the project still targets v140).
#    The .sys is the UNMODIFIED stock driver, copied and renamed - its
#    signature stays valid because Authenticode signs content, not the
#    file name. License notice: docs/THIRD_PARTY_NOTICES.md.
$WinDivertTag = "v2.2.2"
$WinDivertCommit = "1789526ecfb9ff5397c94f9f54c1a3dc2fb60440"
$MonkeyPatch = Join-Path $PSScriptRoot "windivert-monkey.patch"
$MonkeyDir = Join-Path $BinDir "monkey"
$MonkeyDll = Join-Path $MonkeyDir "WinDivert.dll"
$MonkeySys = Join-Path $MonkeyDir "Monkey64.sys"

New-Item -ItemType Directory -Path $MonkeyDir -Force | Out-Null
# Clean the mirror first (except the driver pair): files removed from
# bin/ upstream must not linger in bin/monkey/ forever.
Get-ChildItem -Path $MonkeyDir -Force | Where-Object {
    $_.Name -notin @("WinDivert.dll", "Monkey64.sys")
} | Remove-Item -Recurse -Force
# Mirror bin/ on every run: bin/ changes when upstream pins move, and a
# stale monkey/ engine would silently run the old version. The stock
# driver pair is excluded (the renamed pair goes in below); lists/ is
# excluded (shared from bin/); the target directory itself is excluded
# so the copy cannot recurse.
Get-ChildItem -Path $BinDir -Force | Where-Object {
    $_.Name -notin @("WinDivert.dll", "WinDivert64.sys", "lists", "monkey")
} | ForEach-Object {
    Copy-Item -Path $_.FullName -Destination $MonkeyDir -Recurse -Force
}

# The .sys is the unmodified stock driver, renamed: verify the copy
# matches the stock file byte for byte.
$StockSys = Join-Path $BinDir "WinDivert64.sys"
if (-not (Test-Path $StockSys)) {
    throw "Monkey setup: stock WinDivert64.sys is missing from bin/ (step 2 must run first)."
}
Copy-Item -Path $StockSys -Destination $MonkeySys -Force
$stockHash = (Get-FileHash -Path $StockSys -Algorithm SHA256).Hash
$monkeyHash = (Get-FileHash -Path $MonkeySys -Algorithm SHA256).Hash
if ($stockHash -ne $monkeyHash) {
    throw "SECURITY: Monkey64.sys does not match the stock WinDivert64.sys after copy."
}
Write-Host "    [+] Monkey64.sys is the unmodified stock driver (SHA256 matches)" -ForegroundColor Green

# Build the renamed DLL from WinDivert sources (LGPL compliance: the
# modified DLL is built from published sources + our in-repo patch).
if (-not (Test-Path $MonkeyPatch)) {
    throw "Monkey setup: rename patch not found at $MonkeyPatch."
}
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "Monkey setup: git is not on PATH - install Git to build the renamed WinDivert.dll."
}
$WDSrc = Join-Path $env:TEMP "windivert-2.2.2-monkey"
if (Test-Path $WDSrc) { Remove-Item -Path $WDSrc -Recurse -Force }
git clone --quiet --depth 1 --branch $WinDivertTag "https://github.com/basil00/WinDivert.git" $WDSrc
if ($LASTEXITCODE -ne 0) { throw "Monkey setup: failed to clone WinDivert $WinDivertTag." }
$clonedCommit = (git -C $WDSrc rev-parse HEAD).Trim()
if ($clonedCommit -ne $WinDivertCommit) {
    throw ("SECURITY: WinDivert tag {0} points at {1}, expected {2}. Aborting build." -f $WinDivertTag, $clonedCommit, $WinDivertCommit)
}
git -C $WDSrc apply $MonkeyPatch
if ($LASTEXITCODE -ne 0) { throw "Monkey setup: the rename patch does not apply to WinDivert $WinDivertTag." }

# Locate MSBuild via vswhere (ships with every VS 2017+ install):
# msbuild is not on PATH on GitHub runners or plain dev machines.
$VsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $VsWhere)) {
    throw "Monkey setup: vswhere.exe not found - Visual Studio 2022 or later is required to build the renamed WinDivert.dll."
}
$MsBuild = & $VsWhere -latest -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
if (-not $MsBuild -or -not (Test-Path $MsBuild)) {
    throw "Monkey setup: MSBuild not found via vswhere - Visual Studio 2022 or later with MSBuild is required."
}
$BuildOut = Join-Path $WDSrc "install\MSVC\amd64\"
# /p:UseDebugLibraries=false: the WinDivert project links the Debug CRT
# even in its Release configuration (UseDebugLibraries=true in the
# vcxproj). With the v143 toolset the Debug CRT's memmove resolved its
# internal memcpy call back to memmove itself - infinite recursion and
# STATUS_STACK_OVERFLOW on engine start (issue #7; 1.5.2 shipped this
# broken DLL). A shipped DLL must link the Release CRT. The patch also
# adds a local memmove so the DLL never depends on the CRT's.
& $MsBuild (Join-Path $WDSrc "dll\windivert.vcxproj") /p:Configuration=Release /p:platform=x64 /p:PlatformToolset=v143 /p:UseDebugLibraries=false /p:OutDir=$BuildOut /nologo /v:m
if ($LASTEXITCODE -ne 0) { throw "Monkey setup: MSBuild failed for the renamed WinDivert.dll." }
$BuiltDll = Join-Path $BuildOut "WinDivert.dll"
if (-not (Test-Path $BuiltDll)) { throw "Monkey setup: the renamed WinDivert.dll was not produced by the build." }
Copy-Item -Path $BuiltDll -Destination $MonkeyDll -Force

# Sanity check: the built DLL must carry the renamed service name
# (UTF-16LE "Monkey") - if the patch silently failed to change the
# binary, the build must fail here, not ship a stock DLL as Monkey.
$dllText = [System.Text.Encoding]::Unicode.GetString([System.IO.File]::ReadAllBytes($MonkeyDll))
if ($dllText -notmatch 'Monkey') {
    throw "Monkey setup: the built DLL does not contain the renamed service name - the patch did not take effect."
}
# Tripwire (issue #7): a Debug-CRT build carries a Just-My-Code section
# (.msvcjmc) - the 1.5.2 DLL had one, and its memmove recursed into
# itself. A clean Release build has no such section; if it appears,
# the Debug CRT leaked back in and the build must fail here.
$dllAscii = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($MonkeyDll))
if ($dllAscii.Contains('.msvcjmc')) {
    throw "Monkey setup: the built DLL contains a .msvcjmc section - it was built against the Debug CRT (issue #7)."
}
# Smoke check (issue #7): the mirrored engines must at least start with
# the renamed DLL. This proves the DLL loads and the process runs; it
# does NOT prove WinDivertOpen works (a live start on a real machine is
# the gate for that). Any NTSTATUS crash code (negative exit code)
# fails the build.
foreach ($exe in @((Join-Path $MonkeyDir "winws2.exe"), (Join-Path $MonkeyDir "winws.exe"))) {
    if (Test-Path $exe) {
        $p = Start-Process -FilePath $exe -ArgumentList '--help' -WorkingDirectory $MonkeyDir -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $env:TEMP 'monkey-help.out') -RedirectStandardError (Join-Path $env:TEMP 'monkey-help.err')
        if (-not $p.WaitForExit(30000)) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            throw ("Monkey setup: {0} --help did not exit within 30s - the renamed DLL may be broken (issue #7)." -f $exe)
        }
        if ($p.ExitCode -lt 0) {
            throw ("Monkey setup: {0} --help crashed with exit code {1} - the renamed DLL may be broken (issue #7)." -f $exe, $p.ExitCode)
        }
    } else {
        Write-Warning "Monkey setup: $exe not found in bin/monkey - smoke check skipped for it."
    }
}
Write-Host "    [+] Renamed WinDivert.dll built from WinDivert v2.2.2 sources + in-repo patch" -ForegroundColor Green
# The renamed DLL is unsigned (a local build): Test-Authenticode warns,
# which is expected. The .sys is the signed stock driver.
Test-Authenticode -Path $MonkeySys
Test-Authenticode -Path $MonkeyDll

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
