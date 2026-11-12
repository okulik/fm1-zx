# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Bare-metal Z80 benchmark image, run later in the fm1-emu emulator.

fmzx_embed("${PROJECT_SOURCE_DIR}/third_party/zx-rom/48.rom" zx_rom_data
           "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c")

add_executable(zx_bench crt0.S libc_min.c zx_bench_main.c "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c")
set_target_properties(zx_bench PROPERTIES SUFFIX .elf)
target_link_libraries(zx_bench PRIVATE zx_core)
target_link_options(zx_bench PRIVATE
  -T "${CMAKE_CURRENT_SOURCE_DIR}/bench.ld" --orphan-handling=error -e _start --gc-sections)
set_target_properties(zx_bench PROPERTIES LINK_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/bench.ld")

# JieLi's objcopy takes one -j per run: extract .text and .data separately and concatenate.
set(_elf "$<TARGET_FILE:zx_bench>")
set(_out "${CMAKE_CURRENT_BINARY_DIR}")
add_custom_command(TARGET zx_bench POST_BUILD
  COMMAND ${CMAKE_OBJCOPY} -O binary -j .text "${_elf}" "${_out}/text.bin"
  COMMAND ${CMAKE_OBJCOPY} -O binary -j .data "${_elf}" "${_out}/data.bin"
  COMMAND ${CMAKE_COMMAND} -DOUT=${_out}/zx_bench.bin -DA=${_out}/text.bin -DB=${_out}/data.bin
          -P "${CMAKE_CURRENT_SOURCE_DIR}/concat.cmake"
  COMMAND ${CMAKE_COMMAND} -DOUT=${_out}/zx_bench.dis -DOBJDUMP=${CMAKE_OBJDUMP} -DELF=${_elf}
          -P "${CMAKE_CURRENT_SOURCE_DIR}/dis.cmake"
  VERBATIM)
