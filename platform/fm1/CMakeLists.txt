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
set(_ours crt0.S main.c lcd.c input.c audio.c libc_min.c rt_mem.c)

# Code in RAMTEXT cannot reach XIP's memcpy and friends: the device copies of the core and of
# app/pixels.c (pix_compose_zx runs next to the core) call the RAMTEXT copies (rt_mem.c) instead.
# The objects are renamed copies, not rebuilds: the code stays byte for byte.
find_program(FMZX_PYTHON python3 REQUIRED)
set(_ram "${CMAKE_CURRENT_BINARY_DIR}/ram")
set(_redefine "${CMAKE_CURRENT_SOURCE_DIR}/redefine_syms.py")
set(_rt_syms memcpy=fmzx_rt_memcpy memset=fmzx_rt_memset memmove=fmzx_rt_memmove memcmp=fmzx_rt_memcmp)
add_custom_command(OUTPUT "${_ram}/libzx_core.a"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" "$<TARGET_FILE:zx_core>" "${_ram}/libzx_core.a" ${_rt_syms}
  DEPENDS zx_core "${_redefine}" VERBATIM)
add_custom_command(OUTPUT "${_ram}/pixels.obj"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" --member pixels.c.obj "$<TARGET_FILE:fmzx_app>"
          "${_ram}/pixels.obj" ${_rt_syms}
  DEPENDS fmzx_app "${_redefine}" VERBATIM)
add_custom_target(fmzx_ram_objects DEPENDS "${_ram}/libzx_core.a" "${_ram}/pixels.obj")
set_source_files_properties("${_ram}/pixels.obj" PROPERTIES EXTERNAL_OBJECT TRUE GENERATED TRUE)

add_executable(fmzx_app_image ${_ours} "${_bsp}/chip/fm1_vec.S" "${_bsp}/chip/fm1_isr.S"
  "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c" "${_ram}/pixels.obj")
add_dependencies(fmzx_app_image fmzx_ram_objects)
set_target_properties(fmzx_app_image PROPERTIES OUTPUT_NAME fmzx_app SUFFIX .elf
  LINK_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/app.ld;${_ram}/libzx_core.a")
target_include_directories(fmzx_app_image PRIVATE "${_bsp}/chip" "${_bsp}/boards/fm1"
  $<TARGET_PROPERTY:fmzx_app,INTERFACE_INCLUDE_DIRECTORIES>
  $<TARGET_PROPERTY:zx_core,INTERFACE_INCLUDE_DIRECTORIES>)
target_compile_definitions(fmzx_app_image PRIVATE FMZX_LCD_DIVIDER=${FMZX_LCD_DIVIDER}
  $<TARGET_PROPERTY:zx_core,INTERFACE_COMPILE_DEFINITIONS>)
# felucca-bsp's headers are libraries of static helpers: the ones a unit does not use are fine.
set_source_files_properties(${_ours} PROPERTIES
  COMPILE_OPTIONS "-Wall;-Wextra;-Werror;-Wno-unused-function")
# The renamed core instead of zx_core itself, then the rest of app/ (pixels.c comes from the
# renamed object above, so its archive member is never pulled in).
target_link_libraries(fmzx_app_image PRIVATE "${_ram}/libzx_core.a" "$<TARGET_FILE:fmzx_app>")
add_dependencies(fmzx_app_image fmzx_app zx_core)
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
  COMMAND ${FMZX_PYTHON} "${CMAKE_CURRENT_SOURCE_DIR}/check_reach.py" ${_out}/fmzx_app.dis ${_out}/fmzx_app.map
  VERBATIM)
