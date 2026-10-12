# zapret2 Desktop Supervisor & GUI

<p align="center">
  <img src="assets/logo.png" width="96" height="96" alt="zapret2 Logo" />
</p>

<p align="center">
  <strong>High-performance, fault-tolerant Windows desktop GUI and low-level background supervisor for zapret2.</strong>
</p>

<p align="center">
  <strong><a href="README.md">🇷🇺 Русский</a></strong> | <strong><a href="README_EN.md">🇬🇧 English</a></strong>
</p>

<p align="center">
  <a href="https://github.com/mqclass/zapret2-gui/releases"><img src="https://img.shields.io/github/v/release/mqclass/zapret2-gui?color=brightgreen&label=Latest%20Release" alt="Latest Release" /></a>
  <a href="https://github.com/mqclass/zapret2-gui/releases"><img src="https://img.shields.io/github/downloads/mqclass/zapret2-gui/total?color=blue&label=Downloads" alt="Downloads" /></a>
  <a href="https://github.com/mqclass/zapret2-gui/stargazers"><img src="https://img.shields.io/github/stars/mqclass/zapret2-gui?style=flat&color=yellow" alt="Stars" /></a>
  <img src="https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20x64-blue.svg" alt="Platform" />
  <img src="https://img.shields.io/badge/Backend-Rust%20%2B%20Tauri%20v2-orange.svg" alt="Backend" />
  <img src="https://img.shields.io/badge/Frontend-React%20%2B%20TypeScript%20%2B%20Tailwind-61dafb.svg" alt="Frontend" />
  <img src="https://img.shields.io/badge/License-MIT-purple.svg" alt="License" />
</p>

---

> [!CAUTION]
> ### ⚠️ SECURITY ADVISORY: BEWARE OF FAKES & SCAMS
>
> - **I do NOT maintain any third-party pages, Telegram groups/channels, or YouTube channels.**
> - This application is distributed **EXCLUSIVELY on the official [GitHub repository](https://github.com/mqclass/zapret2-gui)**.
> - I have **no** YouTube channels, and I do **not** upload zapret2-gui to forums, file sharing portals, or third-party websites.
> - **Do NOT download zapret2-gui from other resources** — they may contain modified files, trojans, or malware.
> - Anything distributed outside this official GitHub repository under my name is an **UNOFFICIAL FAKE**.

---

## ⚡ Overview

**zapret2 Desktop GUI** is a modern graphical interface and low-level system supervisor for `zapret2` (`winws2.exe` / `winws.exe`) by *bol-van*, architected for **maximum stability, instantaneous UI response (0ms latency), and minimal footprint (<20 MB RAM)**.

The application enables 1-click circumvention of ISP throttling and blocking for **YouTube (including 4K 60FPS)**, **Discord (voice channels and video streams)**, and restricted web resources without ever needing to touch terminal commands, edit complex configuration strings, or run unverified `.bat` scripts.

---

## ✨ Core Features

- 🛡️ **Kernel Process Tree Isolation via Windows Job Objects:**
  The child engine process `winws2.exe` is bound to a native `JobObject` configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Even if the UI process crashes or is forcefully terminated via Task Manager, the Windows kernel terminates the worker immediately, preventing driver leaks, network locks, or orphaned processes.
- 📌 **System Tray Integration (Minimize on Close):**
  Closing the main window with `[X]` hides the window straight into the Windows notification area (System Tray near the clock) without stopping the circumvention engine. Left-click or double-click instantly restores the window; right-click opens the context menu to toggle bypass or exit completely.
- 🌐 **Built-in Domain & Hostlist Manager:**
  A full-featured visual editor to manage target domain lists (`list-general.txt`, `list-general-user.txt`, `list-exclude.txt`), live search entries, add new websites with automatic formatting and duplicate prevention, remove domains, or open them in your default browser.
- ⚡ **1-Click DNS Cache Flush:**
  Instantly flushes the Windows DNS client cache (`ipconfig /flushdns`) directly from the UI, ensuring routing updates and newly added domains take effect immediately.
- 🎯 **Flagship Preset «zapret2 РФ Ultimate»:**
  Custom-tailored configuration engineered against modern Russian TSPU DPI filters:
  - **YouTube 4K 60FPS:** HTTP/3 QUIC Initial desync (UDP port 443) + TCP TLS Fake with repeats and sequence overlapping (`seqovl=681`).
  - **Discord Voice & STUN:** Direct evasion for voice gateways on ports `50000-50100` and `19294-19344` using binary payload `ACTIVE_DISCORD_UDP.bin`.
  - **General Web Bypass:** Universal `tls_max_ru` masking with fine-tuned offsets.
- 🌟 **Dual Engine Support (Flowseal v1 & zapret2 Lua v2):**
  Seamlessly switch between proven Flowseal presets and modern modular Lua configuration scripts.
- 🔍 **Autonomous Diagnostics Probes (Tokio Probes):**
  An independent background service continuously measures Round Trip Time (latency in ms) and TLS 1.3 handshake availability to YouTube / Google Video CDN and official Discord gateways.
- ⚠️ **1-Click Conflict Detection & Resolution:**
  Automatically discovers competing processes and services (GoodbyeDPI, GoodCheck, other zapret instances, locked WinDivert drivers) with a 1-click resolution mechanism.
- 🚀 **Silent Autostart on Windows Logon (No UAC Prompts):**
  Integrated with Windows Task Scheduler (`schtasks`) with highest run level (`RL HIGHEST`). zapret2 boots automatically on logon in the background (`--minimized`) with administrative privileges without intrusive UAC modals.
- 🛠️ **Custom Config Builder & Quick Templates:**
  Built-in configuration editor featuring quick template buttons (YouTube 4K, Discord Voice, Web, Combo). The smart command-line parser can ingest raw launch arguments from existing `.bat` scripts with automatic variable expansion (`%BIN%`, `%LISTS%`).
- 📖 **Comprehensive Configuration Guide:**
  Includes a complete Russian/English technical reference: [CONFIG_GUIDE.md](CONFIG_GUIDE.md).
- 🌍 **Multi-Language Support:**
  Native localizations in **Russian**, **English**, and **Ukrainian**.
- 📊 **Streaming Real-Time Log Viewer:**
  Asynchronous stdout/stderr capture throttled at 50ms intervals using an in-memory Ring Buffer to protect WebView memory.

---

## 🚀 Quick Start

### For End Users:
1. Download the latest **`zapret2-windows-x86_64.zip`** release from the [Releases](../../releases) tab.
2. Extract the archive into any preferred folder.
3. Run **`zapret2.exe`** as Administrator.
4. Select the **«zapret2 РФ Ultimate»** or **«Flowseal General»** preset and click **«Start Bypass»**.

---

## 🛠️ Building from Source

### Prerequisites:
- **Node.js** v18+ and **npm**
- **Rust** stable toolchain (`x86_64-pc-windows-msvc` or `llvm-mingw`)
- **Tauri CLI v2**

### Build Steps:

```powershell
# 1. Clone the repository
git clone https://github.com/mqclass/zapret2-gui.git
cd zapret2-gui

# 2. Install frontend dependencies
npm install

# 3. Setup core binaries (if missing)
powershell -ExecutionPolicy Bypass -File scripts/setup_binaries.ps1

# 4. Build Vite frontend assets
npm run build

# 5. Compile Rust release binary
cd src-tauri
cargo build --release

# 6. Production executable is generated at:
# src-tauri/target/release/zapret2-supervisor.exe
```

---

## 🔒 Security & Windows Internals

1. **UAC Elevation:**
   Packet filtering via `WinDivert` installs a kernel callout driver into the Windows Filtering Platform (WFP), which fundamentally requires local Administrator privileges. The application embeds an `app.manifest` declaring `requireAdministrator`. If launched without elevation, it relaunches cleanly with the original working directory preserved.
2. **Safe Driver Teardown:**
   Upon exit, the supervisor gracefully halts the engine and unregisters the `WinDivert` service, safely returning the Windows network stack to its normal state.

---

## 🤝 Credits & Acknowledgements

We express our deepest gratitude to the creators of the original utilities and research that made this project possible:

- 🛠️ [**bol-van/zapret2**](https://github.com/bol-van/zapret2) — The core zapret / zapret2 engine (`winws2.exe`, `winws.exe`), WinDivert packet inspection, and low-level TCP/UDP desynchronization mechanics.
- ⚡ [**Flowseal/zapret-discord-youtube**](https://github.com/Flowseal/zapret-discord-youtube) — Curated evasion strategies, hostlists, and desync offsets tailored for YouTube and Discord.
- 🎨 [**medvedeff-true/Zapret-Gui**](https://github.com/medvedeff-true/Zapret-Gui) — Frontend UI inspiration, user experience ideas, and site manager concepts.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).

---

## 🏷️ Topics & Search Keywords

`zapret` · `zapret2` · `winws` · `winws2` · `WinDivert` · `dpi` · `dpi-bypass` · `youtube-unblock` · `discord-fix` · `bypass-censorship` · `anti-censorship` · `goodbyedpi` · `russia-dpi` · `tauri` · `rust` · `react` · `typescript` · `windows-11` · `windows-10` · `gui`

