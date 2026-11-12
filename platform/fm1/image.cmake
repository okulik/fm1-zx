# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Script mode: -DELF -DOBJCOPY -DOUT. Writes the image: the load bytes of .entry, .ram_text,
# .text and .data, which app.ld places back to back from 0x02000120. JieLi's objcopy needs a -j
# and takes one per run, so each section is extracted on its own and the pieces concatenated.
set(_parts "")
foreach(_sec .entry .ram_text .text .data)
  set(_part "${OUT}${_sec}")
  execute_process(COMMAND "${OBJCOPY}" -O binary -j ${_sec} "${ELF}" "${_part}" RESULT_VARIABLE _rc)
  if(NOT _rc EQUAL 0)
    message(FATAL_ERROR "objcopy -j ${_sec} failed (${_rc})")
  endif()
  list(APPEND _parts "${_part}")
endforeach()
execute_process(COMMAND ${CMAKE_COMMAND} -E cat ${_parts} OUTPUT_FILE "${OUT}" RESULT_VARIABLE _rc)
if(NOT _rc EQUAL 0)
  message(FATAL_ERROR "concatenation failed (${_rc})")
endif()
file(REMOVE ${_parts})
