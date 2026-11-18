#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

"""Checks the linked firmware (its objdump -d listing and its link map) for what app.ld promises:

* no direct call or jump between SRAM code (.ram_text in RAMTEXT, and .ram_flash, the flash
  driver, in RAM) and XIP (.entry, .text): they are out of each other's branch reach, or only
  partly in it, which the linker does not report;
* no address in XIP taken by SRAM code (the audio interrupt reads no flash, the core reads its
  tables from SRAM, and the flash driver runs with the flash switched off);
* the core, the sound library, app/pixels.c, app/pixmodes.c, core/sound/learned.c and
  core/sound/engines_builtin.c linked from their renamed device copies, not the originals.

    check_reach.py fmzx_app.dis fmzx_app.map

Prints the RAMTEXT and .ram_flash sizes; exits 1 with the offending lines otherwise.
"""

import re
import sys

RAMTEXT = (0x01C00000, 0x01C0C000)
XIP = (0x02000000, 0x02000120 + 0x8DFBC)  # up to the app slot's end (larger numbers are plain constants)
INSN = re.compile(r"^\s*([0-9a-f]+):\s")
TARGET = re.compile(r"<([^<>]*?) : ([0-9a-f]+) >")
BRANCH = re.compile(r"\b(call|goto)\s+-?\d+\s+<")


def inside(a, r):
    return r[0] <= a < r[1]


def ram_flash_range(map_path):
    """.ram_flash's (start, end) from the map; an empty range when the image has none."""
    with open(map_path) as f:
        for line in f:
            m = re.match(r"^\.ram_flash\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)", line)
            if m:
                a = int(m.group(1), 16)
                return (a, a + int(m.group(2), 16))
    return (0, 0)


def main(dis_path, map_path):
    bad = []
    ram_flash = ram_flash_range(map_path)
    sram_code = {".ram_text": RAMTEXT, ".ram_flash": ram_flash}

    def in_sram_code(a):
        return any(inside(a, r) for r in sram_code.values())

    section, func = None, None
    with open(dis_path) as f:
        for line in f:
            m = re.match(r"Disassembly of section (\S+):", line)
            if m:
                section = m.group(1)
                continue
            m = re.match(r"^(\S+):\s*$", line)
            if m:
                func = m.group(1)
                continue
            if not INSN.match(line):
                continue
            t = TARGET.search(line)
            if not t:
                continue
            dest = int(t.group(2), 16)
            branch = BRANCH.search(line) is not None
            where = f"{section} {func}: {line.strip()}"
            if section in sram_code:
                if branch and not in_sram_code(dest):
                    bad.append(f"SRAM code branches out of SRAM code: {where}")
                elif not branch and inside(dest, XIP):
                    bad.append(f"SRAM code takes an XIP address: {where}")
            elif section in (".entry", ".text") and branch and in_sram_code(dest):
                bad.append(f"XIP branches into SRAM code: {where}")

    used = None
    with open(map_path) as f:
        for line in f:
            if re.search(r"[/\\]core[/\\]libzx_core\.a\(", line):
                bad.append(f"the original core is linked (not the renamed copy): {line.strip()}")
            if re.search(r"[/\\]core[/\\]libzx_sound\.a\(", line):
                bad.append(f"the original sound library is linked (not the renamed copy): {line.strip()}")
            if re.search(r"libfmzx_app\.a\(pixels\.c\.obj\)", line):
                bad.append(f"the original pixels.c is linked (not the renamed copy): {line.strip()}")
            if re.search(r"libfmzx_app\.a\(pixmodes\.c\.obj\)", line):
                bad.append(f"the original pixmodes.c is linked (not the renamed copy): {line.strip()}")
            if re.search(r"libfmzx_learned\.a\(learned\.c\.obj\)", line):
                bad.append(f"the original learned.c is linked (not the renamed copy): {line.strip()}")
            if re.search(r"libfmzx_engines_builtin\.a\(engines_builtin\.c\.obj\)", line):
                bad.append(f"the original engines_builtin.c is linked (not the renamed copy): {line.strip()}")
            m = re.match(r"^\.ram_text\s+0x[0-9a-f]+\s+0x([0-9a-f]+)", line)
            if m:
                used = int(m.group(1), 16)

    if used is None:
        bad.append("no .ram_text in the map")
    if bad:
        print("\n".join(bad[:50]), file=sys.stderr)
        if len(bad) > 50:
            print(f"... and {len(bad) - 50} more", file=sys.stderr)
        sys.exit(1)
    size = RAMTEXT[1] - RAMTEXT[0]
    print(f"RAMTEXT: {used} of {size} bytes used ({size - used} free); .ram_flash: {ram_flash[1] - ram_flash[0]} bytes")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
