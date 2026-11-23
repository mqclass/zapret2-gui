# Third-party notices

## WinDivert (hidden driver / Monkey)

The hidden-driver (Monkey) mode ships a modified WinDivert DLL.
WinDivert is Copyright (C) Basil S. Hollis and contributors, dual-licensed
under the GNU Lesser General Public License v3 or the GNU General Public
License v2, at your choice.

- Upstream sources: https://github.com/basil00/WinDivert - tag v2.2.2,
  commit 1789526ecfb9ff5397c94f9f54c1a3dc2fb60440 (the same version as
  the stock driver shipped in this project).
- Our modification: scripts/windivert-monkey.patch - renames the driver
  service to "Monkey" and the driver file to Monkey64.sys; the device
  object name (\\.\WinDivert) is unchanged, because the stock driver
  creates it. Modified 2026-11-17 by the zapret2-gui project. The
  modified DLL is built from the upstream sources plus this patch by
  scripts/setup_binaries.ps1 - no modified binary is downloaded from
  anywhere.
- The Monkey64.sys file is the UNMODIFIED stock WinDivert 2.2.2 driver,
  copied from the official zapret2 release and renamed; its signature
  stays valid because Authenticode signs content, not the file name.

The full LGPLv3 text ships with the WinDivert sources:
https://github.com/basil00/WinDivert/blob/master/LICENSE
