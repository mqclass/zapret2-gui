#!/usr/bin/env python3
"""Translate the pinned kinlay0 .bat strategies into Rust data.

Reads the 7 vendored .bat files in this directory (pin ce0cddc) and
regenerates src-tauri/src/engine/imported_strategies.rs. The
translation is mechanical and total: every token of the engine
command tail is either carried over verbatim, substituted
(%LISTS% -> {L}), or dropped by an explicit rule below (service/wf/
gamefilter/blobs.lua wrapping). Run from this directory:

    python3 translate.py
"""
import os
import re
import shlex
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(
    os.path.join(HERE, "..", "..", "..", "src-tauri", "src", "engine",
                 "imported_strategies.rs"))

PIN = "ce0cddc399f1ed1aed892df9c55c90e264877395"

# bat filename -> (id, human name, description, size filled at runtime)
STRATEGIES = [
    ("general (DOUBLE FAKE).bat", "kinlay0-double-fake",
     "zapret2 Двойной фейк",
     "Два поддельных пакета подряд с разной обвязкой, затем разрезание: "
     "для DPI, которые не пробивает одиночный фейк. Из сборника стратегий "
     "сообщества kinlay0."),
    ("general (DOUBLE FAKE ALT).bat", "kinlay0-double-fake-alt",
     "zapret2 Двойной фейк ALT",
     "Вариант двойного фейка со сдвигами seq/ack: тот же приём, другая "
     "маскировка пакетов. Из сборника kinlay0."),
    ("general (DOUBLE FAKE ALT2).bat", "kinlay0-double-fake-alt2",
     "zapret2 Двойной фейк ALT2",
     "Вариант двойного фейка: вместо фейка Teamspeak в общих секциях "
     "фейк Steam Chat. Из сборника kinlay0."),
    ("general (SINGLE FAKE).bat", "kinlay0-single-fake",
     "zapret2 Одиночный фейк",
     "Один поддельный пакет плюс разрезание в облегчённой конфигурации. "
     "Из сборника kinlay0."),
    ("general (SINGLE FAKE ALT).bat", "kinlay0-single-fake-alt",
     "zapret2 Одиночный фейк ALT",
     "Вариант одиночного фейка: фейки Сферума вместо 5ka и Teamspeak. "
     "Из сборника kinlay0."),
    ("general (HOSTFAKESPLIT TS).bat", "kinlay0-hostfakesplit-ts",
     "zapret2 Хост-фейк с TS",
     "Основная техника — разрезание TLS по границе имени сайта "
     "с подменой временной метки TCP: DPI видит чужой сайт и теряет "
     "настоящий. Из сборника kinlay0."),
    ("general (HOSTFAKESPLIT TS ALT).bat", "kinlay0-hostfakesplit-ts-alt",
     "zapret2 Хост-фейк с TS ALT",
     "Вариант хост-фейка с TS: в секциях YouTube и общих вместо него "
     "фейк плюс разрезание с большим перекрытием. Из сборника kinlay0."),
]

# Blob name -> payload file, subset of the collection's blobs.lua that
# the 7 strategies use. Files ship in the engine bin/ (upstream
# zapret2/Flowseal bin folder); registration is ours (--blob=name:@file).
BLOB_FILES = {
    "active_discord": "ACTIVE_DISCORD_UDP.bin",
    "quic_google": "quic_initial_www_google_com.bin",
    "stun": "stun.bin",
    "tls_5ka": "tls_clienthello_5ka_ru.bin",
    "tls_google": "tls_clienthello_www_google_com.bin",
    "tls_sferum": "tls_clienthello_www_sferum_ru.bin",
    "tls_steamchat": "tls_clienthello_steam_chat_com.bin",
    "tls_teamspeak": "tls_clienthello_teamspeak_com.bin",
}

GAME_SECTIONS = {"gamefiltertcp", "gamefilterudp"}
EXPECTED_INITS = {"zapret-lib.lua", "zapret-antidpi.lua", "zapret-auto.lua"}


def parse_bat(path):
    text = open(path, encoding="utf-8").read()
    lines = text.splitlines()
    start = next(i for i, l in enumerate(lines) if "winws2.exe" in l)
    tail_lines = []
    for line in lines[start:]:
        line = line.rstrip()
        if line.endswith("^"):
            line = line[:-1]
        tail_lines.append(line)
    tail = " ".join(tail_lines)
    # Cut everything up to and including the quoted exe path.
    tail = tail.split('winws2.exe"', 1)[1]
    tokens = shlex.split(tail, posix=True)

    header, segments, current = [], [], None
    for tok in tokens:
        if tok == "--new":
            segments.append(current)
            current = None
            continue
        if tok.startswith("--name="):
            if current is not None:
                raise SystemExit(f"{path}: --name inside a segment")
            current = [tok]
            continue
        (current if current is not None else header).append(tok)
    segments.append(current)
    segments = [s for s in segments if s]

    # Header: only wf/ctrack wrapping and lua-inits live there.
    inits = set()
    for tok in header:
        if tok.startswith("--lua-init=@"):
            inits.add(tok.split("%LUA%", 1)[-1])
        elif tok.startswith("--wf-") or tok.startswith("--ctrack-"):
            pass
        else:
            raise SystemExit(f"{path}: unexpected header token {tok!r}")
    inits.discard("blobs.lua")
    if inits != EXPECTED_INITS:
        raise SystemExit(f"{path}: lua inits {inits} != {EXPECTED_INITS}")

    profiles, blobs = [], []
    for seg in segments:
        name = seg[0][len("--name="):]
        slug = name.lower().replace(" ", "_")
        if slug in GAME_SECTIONS:
            continue
        args = []
        for tok in seg[1:]:
            if "%GameFilter" in tok:
                raise SystemExit(f"{path}: game filter leaked into {slug}")
            tok = tok.replace("%LISTS%", "{L}")
            for m in re.finditer(r"blob=([a-z0-9_]+)", tok):
                if m.group(1) not in blobs:
                    blobs.append(m.group(1))
            args.append(tok)
        profiles.append((slug, args))
    return profiles, blobs


def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    out = []
    out.append("//! Imported community strategies (kinlay0 collection),")
    out.append("//! vendored as data. GENERATED by")
    out.append("//! strategies/imported/kinlay0/translate.py from the .bat")
    out.append(f"//! files pinned at kinlay0 commit {PIN};")
    out.append("//! do not edit by hand — edit the translator or the pin.")
    out.append("//!")
    out.append("//! The argument vectors below are the effective winws2")
    out.append("//! arguments of the collection's .bat files with the")
    out.append("//! collection's own wrapping removed (service.bat calls,")
    out.append("//! --wf-* root filters, --ctrack-disable, the gamefilter")
    out.append("//! sections and blobs.lua) and with list paths left as")
    out.append("//! `{L}<file>` placeholders: the supervisor substitutes")
    out.append("//! its own resolved list paths and registers the blobs")
    out.append("//! itself (--blob=name:@file). License: MIT, copyright")
    out.append("//! bol-van, Flowseal and kinlay0 — see the LICENSE file")
    out.append("//! next to the vendored .bat files.")
    out.append("")
    out.append("/// One imported strategy: metadata plus its translated")
    out.append("/// argument template, split into named profiles in the")
    out.append("/// .bat's own order.")
    out.append("#[derive(Debug, Clone, Copy)]")
    out.append("pub struct ImportedStrategy {")
    out.append("    pub id: &'static str,")
    out.append("    pub name: &'static str,")
    out.append("    /// Source .bat filename in strategies/imported/kinlay0/.")
    out.append("    pub filename: &'static str,")
    out.append("    pub description: &'static str,")
    out.append("    pub size_bytes: u64,")
    out.append("    /// Blob names this strategy's desync chains reference,")
    out.append("    /// in first-use order; files via `blob_file`.")
    out.append("    pub blobs: &'static [&'static str],")
    out.append("    /// (profile slug, argument template) pairs. `{L}` in an")
    out.append("    /// argument stands for the supervisor-resolved path of")
    out.append("    /// the list file named right after it.")
    out.append("    pub profiles: &'static [(&'static str, &'static [&'static str])],")
    out.append("}")
    out.append("")
    out.append("static STRATEGIES: &[ImportedStrategy] = &[")

    for bat, sid, name, desc in STRATEGIES:
        path = os.path.join(HERE, bat)
        profiles, blobs = parse_bat(path)
        unknown = [b for b in blobs if b not in BLOB_FILES]
        if unknown:
            raise SystemExit(f"{bat}: blobs without file mapping: {unknown}")
        size = os.path.getsize(path)
        out.append("    ImportedStrategy {")
        out.append(f"        id: {rust_str(sid)},")
        out.append(f"        name: {rust_str(name)},")
        out.append(f"        filename: {rust_str(bat)},")
        out.append(f"        description: {rust_str(desc)},")
        out.append(f"        size_bytes: {size},")
        out.append("        blobs: &[")
        for b in blobs:
            out.append(f"            {rust_str(b)},")
        out.append("        ],")
        out.append("        profiles: &[")
        for slug, args in profiles:
            out.append(f"            ({rust_str(slug)}, &[")
            for a in args:
                out.append(f"                {rust_str(a)},")
            out.append("            ]),")
        out.append("        ],")
        out.append("    },")

    out.append("];")
    out.append("")
    out.append("/// All imported strategies, in registration order.")
    out.append("pub fn all() -> &'static [ImportedStrategy] {")
    out.append("    STRATEGIES")
    out.append("}")
    out.append("")
    out.append("/// Looks an imported strategy up by its id.")
    out.append("pub fn find(id: &str) -> Option<&'static ImportedStrategy> {")
    out.append("    STRATEGIES.iter().find(|s| s.id == id)")
    out.append("}")
    out.append("")
    out.append("/// Whether `id` belongs to an imported strategy (the")
    out.append("/// supervisor reports the split-tunnel mode as not applied")
    out.append("/// for these, like for the flagship: their argument lists")
    out.append("/// are fixed data, the Lua tunnel modes do not reshape them).")
    out.append("pub fn is_imported(id: &str) -> bool {")
    out.append("    find(id).is_some()")
    out.append("}")
    out.append("")
    out.append("/// Payload file for a blob name, mirroring the subset of")
    out.append("/// the collection's blobs.lua that these strategies use.")
    out.append("/// The files ship in the engine bin/ (upstream zapret2 /")
    out.append("/// Flowseal bin folder); registration is ours.")
    out.append("pub fn blob_file(name: &str) -> Option<&'static str> {")
    out.append("    Some(match name {")
    for k in sorted(BLOB_FILES):
        out.append(f"        {rust_str(k)} => {rust_str(BLOB_FILES[k])},")
    out.append("        _ => return None,")
    out.append("    })")
    out.append("}")
    out.append("")

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    sys.exit(main())
