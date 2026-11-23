# ==============================================================================
# zapret2 Desktop GUI - portable layout assembly
# ==============================================================================
# Copies the release binaries and runtime data into portable\ so the app
# resolves everything strictly relative to its own exe (production rule,
# see resolve_binaries in platform/windows/process.rs):
#
#   portable\zapret2.exe        GUI (renamed from zapret2-supervisor.exe)
#   portable\zapret2-svc.exe    headless service, must sit next to the GUI
#   portable\bin\               engine + driver + payloads + working lists
#   portable\lists\             curated domain lists (fallback resolution)
#   portable\strategies\        Lua strategy presets
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

New-Item -ItemType Directory -Path $Out -Force | Out-Null
Copy-Item $Gui (Join-Path $Out "zapret2.exe") -Force
Copy-Item $Svc (Join-Path $Out "zapret2-svc.exe") -Force

foreach ($Dir in @("bin", "lists", "strategies")) {
    $Src = Join-Path $Root $Dir
    $Dst = Join-Path $Out $Dir
    if (Test-Path $Dst) { Remove-Item $Dst -Recurse -Force }
    Copy-Item $Src $Dst -Recurse -Force
}

Write-Host "==> Portable layout ready: $Out" -ForegroundColor Green
Write-Host "    Launch: Запустить.bat (repo root) or portable\zapret2.exe"