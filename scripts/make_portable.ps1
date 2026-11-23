# ==============================================================================
# zapret2 Desktop GUI - portable layout assembly
# ==============================================================================
# Copies the release binaries and runtime data into portable\ so the app
# resolves everything strictly relative to its own exe (production rule,
# see resolve_binaries in platform/windows/process.rs):
#
#   portable\zapret2.exe        GUI (renamed from zapret2-supervisor.exe)
#   portable\zapret2-svc.exe    headless service, must sit next to the GUI
#   portable\bin\               engine + driver + payloads (lists/ NOT wiped)
#   portable\bin\lists\         WORKING lists - user edits, learned domains
#   portable\lists\             curated domain lists (fallback resolution)
#   portable\strategies\        Lua strategy presets
#
# DATA SAFETY RULE (do not regress to wipe-and-recopy):
#   portable\bin\lists and portable\lists hold the user's configured
#   addresses (list-*-user.txt) and the engine-learned auto-hostlist.
#   They are NEVER deleted and NEVER overwritten - only files that are
#   MISSING there are added from the repo. Product files (exe/dll/payload/
#   lua) are refreshed in place.
#
# Run after: scripts/setup_binaries.ps1, npm run build, cargo build --release
# Launch with the repo-root "Запустить.bat" (or portable\zapret2.exe directly).
# ==============================================================================
$ErrorActionPreference = "Stop"

$Root = Join-Path $PSScriptRoot ".."
$Release = Join-Path $Root "src-tauri\target\release"
$Out = Join-Path $Root "portable"

$Gui = Join-Path $Release "zapret2-supervisor.exe"
$Svc = Join-Path $Release "zapret2-svc.exe"
foreach ($Exe in @($Gui, $Svc)) {
    if (-not (Test-Path $Exe)) {
        throw "Missing $Exe - run 'cargo build --release' in src-tauri first."
    }
}
if (-not (Test-Path (Join-Path $Root "bin\winws2.exe"))) {
    throw "bin\winws2.exe missing - run scripts/setup_binaries.ps1 first."
}

# Refreshes product files in place: overwrites/adds files and subdirectories
# but never deletes anything. The `lists` subdirectory is skipped - runtime
# user data is handled by Copy-AddOnly below.
function Copy-Product($Src, $Dst) {
    New-Item -ItemType Directory -Path $Dst -Force | Out-Null
    Get-ChildItem -Path $Src -Force | ForEach-Object {
        if ($_.PSIsContainer) {
            if ($_.Name -ieq "lists") { return }
            Copy-Item $_.FullName (Join-Path $Dst $_.Name) -Recurse -Force
        } else {
            Copy-Item $_.FullName (Join-Path $Dst $_.Name) -Force
        }
    }
}

# Add-only data copy: an existing destination file is never touched, so
# user edits and the engine-learned auto-hostlist survive rebuilds.
# Missing files (new curated lists shipped with an update) are added.
function Copy-AddOnly($Src, $Dst) {
    # Canonicalize: $Src may carry a `..\` segment (built from $PSScriptRoot)
    # while Get-ChildItem returns fully resolved paths - Substring against a
    # non-canonical prefix would chop real characters off the file names.
    $Src = (Resolve-Path $Src).Path
    New-Item -ItemType Directory -Path $Dst -Force | Out-Null
    Get-ChildItem -Path $Src -Recurse -File -Force | ForEach-Object {
        $Rel = $_.FullName.Substring($Src.Length).TrimStart("\", "/")
        $Target = Join-Path $Dst $Rel
        if (-not (Test-Path $Target)) {
            $TargetDir = Split-Path $Target -Parent
            New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null
            Copy-Item $_.FullName $Target
        }
    }
}

New-Item -ItemType Directory -Path $Out -Force | Out-Null
Copy-Item $Gui (Join-Path $Out "zapret2.exe") -Force
Copy-Item $Svc (Join-Path $Out "zapret2-svc.exe") -Force

Copy-Product (Join-Path $Root "bin") (Join-Path $Out "bin")
Copy-AddOnly (Join-Path $Root "bin\lists") (Join-Path $Out "bin\lists")
Copy-AddOnly (Join-Path $Root "lists") (Join-Path $Out "lists")
Copy-Product (Join-Path $Root "strategies") (Join-Path $Out "strategies")

Write-Host "==> Portable layout ready: $Out" -ForegroundColor Green
Write-Host "    Launch: Запустить.bat (repo root) or portable\zapret2.exe"
