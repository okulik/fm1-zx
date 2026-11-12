# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# fmzx_embed(<input> <symbol> <out.c>)
# Generates <out.c> defining `const uint8_t <symbol>[]` and `const uint32_t <symbol>_size`
# from the bytes of <input>; regenerated when <input> changes.

set(_FMZX_EMBED_SCRIPT "${CMAKE_CURRENT_LIST_DIR}/embed_run.cmake")

function(fmzx_embed input symbol out)
  get_filename_component(input "${input}" ABSOLUTE)
  add_custom_command(
    OUTPUT "${out}"
    COMMAND ${CMAKE_COMMAND} -DINPUT=${input} -DSYMBOL=${symbol} -DOUTPUT=${out}
            -P "${_FMZX_EMBED_SCRIPT}"
    DEPENDS "${input}" "${_FMZX_EMBED_SCRIPT}"
    COMMENT "Embedding ${input} as ${symbol}"
    VERBATIM)
  set_source_files_properties("${out}" PROPERTIES GENERATED TRUE)
endfunction()
