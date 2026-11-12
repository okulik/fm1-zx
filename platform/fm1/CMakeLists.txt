# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# The firmware image: fmzx_app.elf, fmzx_app.bin (the bytes from 0x02000120) and fmzx_app.dis.

set(FMZX_LCD_DIVIDER 4 CACHE STRING "LCD SPI clock divider at boot, 0-15: 60 MHz / (divider + 1)")

file(SIZE "${PROJECT_SOURCE_DIR}/third_party/zx-rom/48.rom" _zx_rom_size)
if(NOT _zx_rom_size EQUAL 16384)
  message(FATAL_ERROR "third_party/zx-rom/48.rom must be exactly 16384 bytes (found ${_zx_rom_size})")
endif()
fmzx_embed("${PROJECT_SOURCE_DIR}/third_party/zx-rom/48.rom" zx_rom_data
           "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c")

set(_bsp "${CMAKE_CURRENT_SOURCE_DIR}/bsp")
set(_ours crt0.S main.c lcd.c input.c libc_min.c)

add_executable(fmzx_app_image ${_ours} "${_bsp}/chip/fm1_vec.S" "${_bsp}/chip/fm1_isr.S" "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c")
set_target_properties(fmzx_app_image PROPERTIES OUTPUT_NAME fmzx_app SUFFIX .elf
  LINK_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/app.ld")
target_include_directories(fmzx_app_image PRIVATE "${_bsp}/chip" "${_bsp}/boards/fm1")
target_compile_definitions(fmzx_app_image PRIVATE FMZX_LCD_DIVIDER=${FMZX_LCD_DIVIDER})
# felucca-bsp's headers are libraries of static helpers: the ones a unit does not use are fine.
set_source_files_properties(${_ours} PROPERTIES
  COMPILE_OPTIONS "-Wall;-Wextra;-Werror;-Wno-unused-function")
target_link_libraries(fmzx_app_image PRIVATE fmzx_app zx_core)
target_link_options(fmzx_app_image PRIVATE
  -T "${CMAKE_CURRENT_SOURCE_DIR}/app.ld" --orphan-handling=error -e _start --gc-sections --emit-relocs
  "-Map=${CMAKE_CURRENT_BINARY_DIR}/fmzx_app.map")

set(_elf "$<TARGET_FILE:fmzx_app_image>")
set(_out "${CMAKE_CURRENT_BINARY_DIR}")
add_custom_command(TARGET fmzx_app_image POST_BUILD
  COMMAND ${CMAKE_COMMAND} -DELF=${_elf} -DOBJCOPY=${CMAKE_OBJCOPY} -DOUT=${_out}/fmzx_app.bin
          -P "${CMAKE_CURRENT_SOURCE_DIR}/image.cmake"
  COMMAND ${CMAKE_COMMAND} -DOUT=${_out}/fmzx_app.dis -DOBJDUMP=${CMAKE_OBJDUMP} -DELF=${_elf}
          -P "${PROJECT_SOURCE_DIR}/bench/fm1/dis.cmake"
  VERBATIM)
