# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Writes a symbol's address from a 32-bit little-endian ELF into keymaps.json (tools/web-demo):
#   python3 web/elf-symbol.py ELF SYMBOL SIZE KEYMAPS.json
# adds "<SYMBOL>": <address> to the JSON object; fails (exit 1) when the symbol is missing or its
# size is not SIZE (the firmware's layout changed: web/fmzx.js reads it as FwStats).
import json
import struct
import sys

elf_path, name, size, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
data = open(elf_path, "rb").read()
if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
    sys.exit(f"{elf_path}: not a 32-bit little-endian ELF")
shoff, = struct.unpack_from("<I", data, 0x20)
shentsize, shnum = struct.unpack_from("<HH", data, 0x2E)
sections = [struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize) for i in range(shnum)]
found = None
for sh in sections:
    if sh[1] != 2:  # SHT_SYMTAB
        continue
    strtab = sections[sh[6]]
    for off in range(sh[4], sh[4] + sh[5], 16):
        st_name, st_value, st_size = struct.unpack_from("<III", data, off)
        end = data.index(b"\0", strtab[4] + st_name)
        if data[strtab[4] + st_name:end].decode() == name:
            found = (st_value, st_size)
if not found:
    sys.exit(f"{elf_path}: no symbol {name}")
if found[1] != size:
    sys.exit(f"{elf_path}: {name} is {found[1]} bytes, not {size}")
with open(out, encoding="utf-8") as fh:
    doc = json.load(fh)
doc[name] = found[0]
with open(out, "w", encoding="utf-8") as fh:
    json.dump(doc, fh, separators=(",", ":"))
    fh.write("\n")
