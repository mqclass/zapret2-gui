#!/usr/bin/env python3
"""Translate the pinned kinlay0 .bat strategies into Rust data.

Reads the 24 vendored .bat files in this directory (pin ce0cddc) and
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
    # Second group: EXP5 (the clone technique, no circular orchestrator),
    # EXP (a distinct hostfakesplit+multisplit mix) and the continuations
    # of known families. CIRCULAR/CIRCULAR1 stay out: the circular
    # orchestrator is a second controller on top of our watchdog and
    # auto-tune, it comes last as a separate step.
    ("general (EXP5).bat", "kinlay0-exp5",
     "zapret2 Клон ClientHello",
     "Клон настоящего ClientHello в фейк, затем фейк и разрезание: "
     "DPI видит его же собственный пакет. Единственная стратегия "
     "с клоном без оркестратора circular. Из сборника kinlay0."),
    ("general (EXP).bat", "kinlay0-exp",
     "zapret2 EXP микс",
     "Микс: хост-фейк на YouTube, фейк Сферума плюс разрезание "
     "с перекрытием 480 на паттерне STUN2 в общих секциях. "
     "Из сборника kinlay0."),
    ("general (ALT14).bat", "kinlay0-alt14",
     "zapret2 ALT14",
     "Продолжение серии ALT: фейк плюс разрезание, фейк Сферума "
     "в секциях Discord Media и общих. Из сборника kinlay0."),
    ("general (ALT15).bat", "kinlay0-alt15",
     "zapret2 ALT15",
     "Продолжение серии ALT: тот же рецепт, что ALT14, но в секции "
     "Discord Media фейк Google вместо Сферума. Из сборника kinlay0."),
    ("general (ALT16).bat", "kinlay0-alt16",
     "zapret2 ALT16",
     "Продолжение серии ALT: в общих секциях первым идёт фейк STUN, "
     "меньше повторов на YouTube и IP-QUIC. Из сборника kinlay0."),
    ("general (SIMPLE FAKE ALT3).bat", "kinlay0-simple-fake-alt3",
     "zapret2 Простой фейк ALT3",
     "Простой фейк: STUN2 плюс фейк Сферума и разрезание. "
     "Из сборника kinlay0."),
    ("general (SIMPLE FAKE ALT4).bat", "kinlay0-simple-fake-alt4",
     "zapret2 Простой фейк ALT4",
     "Простой фейк: STUN2 плюс фейк Steam Chat и разрезание. "
     "Из сборника kinlay0."),
    ("general (FAKE TLS AUTO ALT4).bat", "kinlay0-fake-tls-auto-alt4",
     "zapret2 Авто TLS фейк ALT4",
     "Авто TLS фейк: разрезание только multisplit, без disorder "
     "и fakedsplit. Из сборника kinlay0."),
    ("general (FAKE TLS AUTO ALT5).bat", "kinlay0-fake-tls-auto-alt5",
     "zapret2 Авто TLS фейк ALT5",
     "Авто TLS фейк: разрезание только multidisorder, без multisplit "
     "и fakedsplit. Из сборника kinlay0."),
    ("general (FAKE TLS AUTO ALT6).bat", "kinlay0-fake-tls-auto-alt6",
     "zapret2 Авто TLS фейк ALT6",
     "Авто TLS фейк: разрезание только fakedsplit, без multisplit "
     "и multidisorder. Из сборника kinlay0."),
    ("general (LEGACY10).bat", "kinlay0-legacy10",
     "zapret2 LEGACY10",
     "Старая версия ALT-конфигов: фейк Teamspeak плюс разрезание. "
     "Резерв на случай, если новые не сработают. Из сборника kinlay0."),
    ("general (LEGACY11).bat", "kinlay0-legacy11",
     "zapret2 LEGACY11",
     "Старая версия ALT-конфигов: фейк Сферума плюс разрезание "
     "с перекрытием 661. Резерв на случай, если новые не сработают. "
     "Из сборника kinlay0."),
    ("general (EXP1).bat", "kinlay0-exp1",
     "zapret2 EXP1",
     "Эксперимент: disorder плюс fakedsplit, фейки Steam Chat "
     "с подменой SNI. Из сборника kinlay0."),
    ("general (EXP2).bat", "kinlay0-exp2",
     "zapret2 EXP2",
     "Эксперимент: disorder с экзотическими фейками, включая "
     "ClientHello retext.ai. Из сборника kinlay0."),
    ("general (EXP3).bat", "kinlay0-exp3",
     "zapret2 EXP3",
     "Эксперимент: disorder с фейками retext.ai, Сферума и 5ka. "
     "Из сборника kinlay0."),
    ("general (EXP4).bat", "kinlay0-exp4",
     "zapret2 EXP4",
     "Эксперимент: disorder с фейками retext.ai и Steam Chat. "
     "Из сборника kinlay0."),
    ("general (EXPERIMENTAL TS).bat", "kinlay0-experimental-ts",
     "zapret2 Эксперимент TS",
     "Вариация disorder с подменой временной метки TCP. "
     "Из сборника kinlay0."),
]

# Blob name -> payload file, subset of the collection's blobs.lua that
# the 24 strategies use. Files ship in the engine bin/ (upstream
# zapret2/Flowseal bin folder); registration is ours (--blob=name:@file).
# tls_retext is the one file taken from the collection's own bin/
# (tls_clienthello_retext_ai.bin, same class of artifact as the other
# captured ClientHello payloads); tls_mail/tls_onetrust map to files
# we already ship (sochi_park/max_ru) — only tls_mail is used outside
# the dropped gamefilter sections, tls_onetrust is gamefilter-only.
BLOB_FILES = {
    "active_discord": "ACTIVE_DISCORD_UDP.bin",
    "quic_google": "quic_initial_www_google_com.bin",
    "stun": "stun.bin",
    "stun2": "stun2.bin",
    "tls_5ka": "tls_clienthello_5ka_ru.bin",
    "tls_google": "tls_clienthello_www_google_com.bin",
    "tls_mail": "tls_clienthello_sochi_park.bin",
    "tls_retext": "tls_clienthello_retext_ai.bin",
    "tls_sferum": "tls_clienthello_www_sferum_ru.bin",
    "tls_steamchat": "tls_clienthello_steam_chat_com.bin",
    "tls_teamspeak": "tls_clienthello_teamspeak_com.bin",
}

# Blob names that are NOT file-backed: tls_client_hello_clone (stock
# zapret-antidpi.lua) writes the cloned ClientHello into the desync
# table under this name at runtime, with the chain's fallback blob if
# the clone fails. The desync chains reference it as blob=ch_clone,
# but nothing may be registered for it with --blob=name:@file.
RUNTIME_BLOBS = {"ch_clone"}

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
            # Blob references in a desync chain: blob= (the fake
            # payload), seqovl_pattern= (the overlap pattern of
            # multisplit/multidisorder) and fallback= (the clone's
            # fallback payload). All three resolve through blob() in
            # the stock Lua, so all three must be registered. 0x..
            # values are inline hex patterns, not blob names.
            for m in re.finditer(r"(?:blob|seqovl_pattern|fallback)=([a-z0-9_]+)", tok):
                if m.group(1).startswith("0x"):
                    continue
                # Runtime blobs (ch_clone) are filled in by the desync
                # function itself; they are not file-backed and must
                # not be collected for --blob registration.
                if m.group(1) in RUNTIME_BLOBS:
                    continue
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
    out.append("/// Blob names that are NOT file-backed: `tls_client_hello_clone`")
    out.append("/// (stock zapret-antidpi.lua) writes the cloned ClientHello into")
    out.append("/// the desync table under this name at runtime, with the chain's")
    out.append("/// fallback blob if the clone fails. Desync chains reference it")
    out.append("/// as `blob=ch_clone`, but nothing is registered for it.")
    out.append("/// Test-only: the shape test consults it when checking blob")
    out.append("/// registration completeness.")
    out.append("#[cfg(test)]")
    out.append("pub const RUNTIME_BLOBS: &[&str] = &[")
    for k in sorted(RUNTIME_BLOBS):
        out.append(f"    {rust_str(k)},")
    out.append("];")
    out.append("")
    out.append("/// Whether `name` is a runtime-filled blob (see RUNTIME_BLOBS).")
    out.append("#[cfg(test)]")
    out.append("pub fn is_runtime_blob(name: &str) -> bool {")
    out.append("    RUNTIME_BLOBS.contains(&name)")
    out.append("}")
    out.append("")
    out.append("")

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    sys.exit(main())
