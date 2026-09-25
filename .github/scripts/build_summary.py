#!/usr/bin/env python3
"""Write a Markdown build report for one firmware job to $GITHUB_STEP_SUMMARY.

Usage: build_summary.py TITLE ELF TIMINGS_HTML CACHE_HIT

Sections: toolchain, Rust cache result, compile times per crate (from
`cargo build --timings`), and the firmware's memory use per ELF section.
Standard library only. Output is plain tables with text labels (no colour- or
icon-only meaning) so it reads well in screen readers and high-contrast themes.
"""

import json
import os
import re
import struct
import subprocess
import sys

SLOWEST = 10


def toolchain():
    try:
        return subprocess.run(["rustc", "-V"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def timings(path):
    """(total time text, compiled units, fresh unit count) from a cargo timing report."""
    html = open(path, encoding="utf-8").read()
    units = json.loads(re.search(r"const UNIT_DATA = (\[.*?\]);\n", html, re.S).group(1))
    total = re.search(r"<td>Total time:</td><td>([^<]*)</td>", html)
    compiled = [u for u in units if u["duration"] > 0]
    compiled.sort(key=lambda u: -u["duration"])
    return (total.group(1) if total else "unknown"), compiled, len(units) - len(compiled)


def elf_sections(path):
    """Allocated sections of a 32-bit little-endian ELF: (name, address, size, in_image)."""
    data = open(path, "rb").read()
    if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
        raise ValueError("expected a 32-bit little-endian ELF")
    shoff, = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x2E)
    headers = [struct.unpack_from("<10I", data, shoff + i * shentsize) for i in range(shnum)]
    strtab = headers[shstrndx][4]  # sh_offset of the section name table
    sections = []
    for name_off, sh_type, flags, addr, _off, size, *_ in headers:
        SHF_ALLOC, SHT_NOBITS = 0x2, 8
        if flags & SHF_ALLOC and size:
            name = data[strtab + name_off : data.index(b"\0", strtab + name_off)].decode()
            sections.append((name, addr, size, sh_type != SHT_NOBITS))
    return sorted(sections, key=lambda s: s[1])


def main():
    title, elf, timing_html, cache_hit = sys.argv[1:5]
    out = [f"## {title}", ""]

    out += ["| Item | Value |", "|---|---|", f"| Toolchain | `{toolchain()}` |"]
    cache = "hit (exact key match)" if cache_hit == "true" else "miss or partial (restored from an older key, or nothing)"
    out += [f"| Rust cache | {cache} |", ""]

    total, compiled, fresh = timings(timing_html)
    out += [
        "### Compile time",
        "",
        f"Total build time {total}. {len(compiled)} crates compiled, {fresh} reused from cache "
        "(the HTML timing chart is attached to this run as an artifact).",
        "",
        f"Slowest {min(SLOWEST, len(compiled))} crates:",
        "",
        "| Crate | Version | Seconds |",
        "|---|---|--:|",
    ]
    out += [f"| {u['name']} | {u['version']} | {u['duration']:.2f} |" for u in compiled[:SLOWEST]]
    out.append("")

    sections = elf_sections(elf)
    image = sum(s[2] for s in sections if s[3])
    ram_only = sum(s[2] for s in sections if not s[3])
    out += [
        "### Memory use",
        "",
        f"Stored in the flash image: {image:,} bytes. Reserved but not stored (zero-filled, e.g. `.bss`, stack): {ram_only:,} bytes.",
        "",
        "| Section | Address | Bytes | Kind |",
        "|---|---|--:|---|",
    ]
    out += [
        f"| `{name}` | `0x{addr:08x}` | {size:,} | {'stored' if in_image else 'reserved'} |"
        for name, addr, size, in_image in sections
    ]
    out.append("")

    text = "\n".join(out) + "\n"
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
