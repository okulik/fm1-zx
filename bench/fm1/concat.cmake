# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Script mode: concatenate -DA and -DB into -DOUT.
execute_process(COMMAND ${CMAKE_COMMAND} -E cat "${A}" "${B}" OUTPUT_FILE "${OUT}" RESULT_VARIABLE _rc)
if(NOT _rc EQUAL 0)
  message(FATAL_ERROR "concatenation failed (${_rc})")
endif()
