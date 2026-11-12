#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

"""Renames symbols in ELF32 little-endian relocatable objects, as `objcopy --redefine-sym
old=new` does (the JieLi objcopy has no such option, and GNU objcopy does not know pi32v2).

    redefine_syms.py --ar AR IN.a OUT.a old=new...               every member of an archive
    redefine_syms.py --ar AR --member M.obj IN.a OUT.obj old=new...   one member, as an object
    redefine_syms.py IN.obj OUT.obj old=new...

Each symbol table entry named `old` (defined or not) gets the name `new`: the new names are
appended to the symbol string table, which moves to the end of the file. Nothing else changes,
so the code, the relocations and every other section stay byte for byte.
"""

import os
import shutil
import struct
import subprocess
import sys
import tempfile

SHT_SYMTAB = 2


def rename(data, mapping):
    """Returns `data` (an ELF32 LE object) with the symbols in `mapping` renamed, and the count."""
    if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
        raise SystemExit("not an ELF32 little-endian object")
    (shoff,) = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum = struct.unpack_from("<HH", data, 0x2E)
    out = bytearray(data)
    count = 0
    for i in range(shnum):
        sh = shoff + i * shentsize
        typ, off, size, link = (struct.unpack_from("<I", data, sh + o)[0] for o in (4, 16, 20, 24))
        if typ != SHT_SYMTAB:
            continue
        str_sh = shoff + link * shentsize
        str_off, str_size = struct.unpack_from("<II", data, str_sh + 16)
        strtab = bytes(data[str_off : str_off + str_size])
        extra = bytearray()
        for k in range(size // 16):
            (name,) = struct.unpack_from("<I", data, off + 16 * k)
            sym = strtab[name : strtab.index(b"\0", name)].decode()
            if sym in mapping:
                struct.pack_into("<I", out, off + 16 * k, str_size + len(extra))
                extra += mapping[sym].encode() + b"\0"
                count += 1
        if extra:
            out += b"\0" * (-len(out) % 4)
            struct.pack_into("<II", out, str_sh + 16, len(out), str_size + len(extra))
            out += strtab + extra
    return bytes(out), count


def main(argv):
    ar, member = None, None
    while argv and argv[0].startswith("--"):
        opt, val, argv = argv[0], argv[1], argv[2:]
        if opt == "--ar":
            ar = val
        elif opt == "--member":
            member = val
        else:
            raise SystemExit(f"unknown option {opt}")
    if len(argv) < 3:
        raise SystemExit(__doc__)
    src, dst = argv[0], argv[1]
    mapping = dict(a.split("=", 1) for a in argv[2:])

    with open(src, "rb") as f:
        head = f.read(8)
    if head != b"!<arch>\n":
        with open(src, "rb") as f:
            data, _ = rename(f.read(), mapping)
        with open(dst, "wb") as f:
            f.write(data)
        return
    if not ar:
        raise SystemExit("an archive needs --ar")
    src = os.path.abspath(src)
    work = tempfile.mkdtemp(dir=os.path.dirname(os.path.abspath(dst)))
    try:
        names = subprocess.run([ar, "t", src], check=True, capture_output=True, text=True).stdout.split()
        if member is not None:
            if member not in names:
                raise SystemExit(f"{src} has no member {member}")
            names = [member]
        subprocess.run([ar, "x", src] + names, check=True, cwd=work)
        for n in names:
            p = os.path.join(work, n)
            with open(p, "rb") as f:
                data, _ = rename(f.read(), mapping)
            with open(p, "wb") as f:
                f.write(data)
        if member is not None:
            shutil.copyfile(os.path.join(work, member), dst)
        else:
            tmp = os.path.join(work, "out.a")
            subprocess.run([ar, "rcs", tmp] + names, check=True, cwd=work)
            os.replace(tmp, dst)
    finally:
        shutil.rmtree(work)


if __name__ == "__main__":
    main(sys.argv[1:])
