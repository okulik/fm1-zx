# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Script mode: -DBIN -DSIZE -DSHA256. Fails unless BIN is SIZE bytes with that SHA-256: the
# loader built from the vendored felucca-bsp sources must be the pinned one, byte for byte.
file(SIZE "${BIN}" _size)
file(SHA256 "${BIN}" _sha)
if(NOT _size EQUAL SIZE OR NOT _sha STREQUAL SHA256)
  file(REMOVE "${BIN}")
  message(FATAL_ERROR "${BIN}: ${_size} bytes, SHA-256 ${_sha}; expected ${SIZE} bytes, ${SHA256} "
                      "(platform/fm1/VENDORED.md). Do not change the vendored sources.")
endif()
