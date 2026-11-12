# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Script mode: write the disassembly of -DELF to -DOUT using -DOBJDUMP.
execute_process(COMMAND "${OBJDUMP}" -d "${ELF}" OUTPUT_FILE "${OUT}" RESULT_VARIABLE _rc)
if(NOT _rc EQUAL 0)
  message(FATAL_ERROR "objdump failed (${_rc})")
endif()
