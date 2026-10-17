# Vendored: tg-ws-proxy-rs

## What is this

A vendored copy of **tg-ws-proxy-rs v2.5.4** — a Telegram MTProto WebSocket
bridge proxy written in Rust.

- Upstream repository: https://github.com/valnesfjord/tg-ws-proxy-rs
- Upstream tag/commit: `v2.5.4`
- License: **MIT** (see `LICENSE` in this directory;
  copyright (c) 2026 valnesfjord)
- The upstream project is itself a Rust port of
  [Flowseal/tg-ws-proxy](https://github.com/Flowseal/tg-ws-proxy).

zapret2-gui embeds this crate **as a library** (in-process Tokio task) to run
the Telegram bridge — no separate `TgWsProxy_windows.exe` sidecar is
downloaded or shipped. The integration layer lives in
`src-tauri/src/engine/tgproxy.rs` (lifecycle, DPAPI secret storage, logging
with secret redaction, `TG_*` env cleanup before config parsing).

## What was changed vs upstream

The library sources under `src/` are **byte-identical** to upstream `v2.5.4`.
Only the packaging was trimmed for embedding:

- `Cargo.toml`: removed the `[workspace]` table (we do not vendor
  `crates/android-jni`), the `[[bin]]` target and `src/main.rs` (the standalone
  CLI is not part of this product — the app itself is the frontend), and the
  `[package.metadata.android]` section.
- Not vendored: `tests/`, `examples/`, `docs/`, `android/`, `openwrt/`,
  `metadata/`, `Dockerfile`, `Cross.toml`, `install.sh`, `rust-toolchain.toml`,
  upstream `Cargo.lock`.

## Syncing with upstream

To pull a newer upstream release:

1. `git clone --branch <tag> https://github.com/valnesfjord/tg-ws-proxy-rs`
2. Replace `src/` with the new tag's `src/` (minus `main.rs`), update
   `LICENSE`/`README.md` if they changed, bump `version` in `Cargo.toml`.
3. Re-apply the `Cargo.toml` trims listed above.
4. Run the stub-crate check (see `~/MEMORY.md` lesson) and the Windows CI.

Note: `vendor/` is in the repo `.gitignore` (it normally holds `cargo vendor`
output), so this directory is tracked via explicit `git add -f`. Use
`git add -f src-tauri/vendor/tg-ws-proxy` when updating it.

## License note

MIT permits use, copy, modification, merge and distribution provided the
copyright notice and permission notice are included — both are preserved
here (`LICENSE`) and the crate is credited in the app's Settings UI.
