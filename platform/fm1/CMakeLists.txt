# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# The firmware image: fmzx_app.elf, fmzx_app.bin (the bytes from 0x02000120) and fmzx_app.dis.

set(FMZX_LCD_DIVIDER 4 CACHE STRING "LCD SPI clock divider at boot, 0-15: 60 MHz / (divider + 1)")
option(FMZX_LCD_RGB565 "RGB565 instead of RGB444 when no boot option is set (the fm1-emu preview package)" OFF)
set(FMZX_PRODUCT "FM-1_948" CACHE STRING "Product identity in the image and the update package: FM-1_ and three digits")
if(NOT FMZX_PRODUCT MATCHES "^FM-1_[0-9][0-9][0-9]$")
  message(FATAL_ERROR "FMZX_PRODUCT must be FM-1_ and three digits (got '${FMZX_PRODUCT}')")
endif()
if(FMZX_LCD_RGB565)
  set(_rgb565 1)
else()
  set(_rgb565 0)
endif()

file(SIZE "${PROJECT_SOURCE_DIR}/third_party/zx-rom/48.rom" _zx_rom_size)
if(NOT _zx_rom_size EQUAL 16384)
  message(FATAL_ERROR "third_party/zx-rom/48.rom must be exactly 16384 bytes (found ${_zx_rom_size})")
endif()
fmzx_embed("${PROJECT_SOURCE_DIR}/third_party/zx-rom/48.rom" zx_rom_data
           "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c")

set(_bsp "${CMAKE_CURRENT_SOURCE_DIR}/bsp")
set(_ours crt0.S main.c lcd.c input.c audio.c libc_min.c rt_mem.c)

# Code in RAMTEXT cannot reach XIP's memcpy and friends: the device copies of the core, of the
# sound library (core/sound: the decoder and the FM synth run every frame) and of app/pixels.c
# (pix_compose_zx runs next to the core) call the RAMTEXT copies (rt_mem.c) instead.
# The objects are renamed copies, not rebuilds: the code stays byte for byte.
find_program(FMZX_PYTHON python3 REQUIRED)
set(_ram "${CMAKE_CURRENT_BINARY_DIR}/ram")
set(_redefine "${CMAKE_CURRENT_SOURCE_DIR}/redefine_syms.py")
set(_rt_syms memcpy=fmzx_rt_memcpy memset=fmzx_rt_memset memmove=fmzx_rt_memmove memcmp=fmzx_rt_memcmp)
add_custom_command(OUTPUT "${_ram}/libzx_core.a"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" "$<TARGET_FILE:zx_core>" "${_ram}/libzx_core.a" ${_rt_syms}
  DEPENDS zx_core "${_redefine}" VERBATIM)
add_custom_command(OUTPUT "${_ram}/libzx_sound.a"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" "$<TARGET_FILE:zx_sound>" "${_ram}/libzx_sound.a" ${_rt_syms}
  DEPENDS zx_sound "${_redefine}" VERBATIM)
add_custom_command(OUTPUT "${_ram}/pixels.obj"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" --member pixels.c.obj "$<TARGET_FILE:fmzx_app>"
          "${_ram}/pixels.obj" ${_rt_syms}
  DEPENDS fmzx_app "${_redefine}" VERBATIM)
# app/pixmodes.c (the screen modes other than Crop) runs from XIP: its device copy calls the
# core's line accessors and pix_compose_zx, which are in RAMTEXT, through main.c's fmzx_xip_*
# (fmzx_ram) instead, and the XIP memset and memcpy.
add_custom_command(OUTPUT "${_ram}/pixmodes.obj"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" --member pixmodes.c.obj "$<TARGET_FILE:fmzx_app>"
          "${_ram}/pixmodes.obj" zx_screen_line=fmzx_xip_screen_line zx_border_line=fmzx_xip_border_line
          pix_compose_zx=fmzx_xip_compose_zx
  DEPENDS fmzx_app "${_redefine}" VERBATIM)
# core/sound/learned.c (the learned entries: their decoder and the reader method B calls through
# an entry's read pointer) is not in zx_sound: it runs from XIP, its device copy calling the
# core's zx_peek and zx_get_regs, which are in RAMTEXT, through main.c's fmzx_xip_* (fmzx_ram).
add_library(fmzx_learned STATIC "${PROJECT_SOURCE_DIR}/core/sound/learned.c")
target_compile_options(fmzx_learned PRIVATE -Wall -Wextra -Werror)
target_link_libraries(fmzx_learned PRIVATE zx_sound)
add_custom_command(OUTPUT "${_ram}/learned.obj"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" --member learned.c.obj "$<TARGET_FILE:fmzx_learned>"
          "${_ram}/learned.obj" zx_peek=fmzx_xip_peek zx_get_regs=fmzx_xip_get_regs
  DEPENDS fmzx_learned "${_redefine}" VERBATIM)
# core/sound/engines_builtin.c (method B's built-in table, which sound_init.c and main.c read, and
# the readers method B calls through an entry's read pointer) runs from XIP the same way, its
# device copy calling zx_peek and zx_get_regs through fmzx_xip_*.
add_library(fmzx_engines_builtin STATIC "${PROJECT_SOURCE_DIR}/core/sound/engines_builtin.c")
target_compile_options(fmzx_engines_builtin PRIVATE -Wall -Wextra -Werror)
target_link_libraries(fmzx_engines_builtin PRIVATE zx_sound)
add_custom_command(OUTPUT "${_ram}/engines_builtin.obj"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ram}"
  COMMAND ${FMZX_PYTHON} "${_redefine}" --ar "${CMAKE_AR}" --member engines_builtin.c.obj
          "$<TARGET_FILE:fmzx_engines_builtin>" "${_ram}/engines_builtin.obj" zx_peek=fmzx_xip_peek
          zx_get_regs=fmzx_xip_get_regs
  DEPENDS fmzx_engines_builtin "${_redefine}" VERBATIM)
add_custom_target(fmzx_ram_objects DEPENDS "${_ram}/libzx_core.a" "${_ram}/libzx_sound.a" "${_ram}/pixels.obj"
  "${_ram}/pixmodes.obj" "${_ram}/learned.obj" "${_ram}/engines_builtin.obj")
set_source_files_properties("${_ram}/pixels.obj" "${_ram}/pixmodes.obj" "${_ram}/learned.obj"
  "${_ram}/engines_builtin.obj" PROPERTIES EXTERNAL_OBJECT TRUE GENERATED TRUE)

add_executable(fmzx_app_image ${_ours} "${_bsp}/chip/fm1_vec.S" "${_bsp}/chip/fm1_isr.S"
  "${CMAKE_CURRENT_BINARY_DIR}/rom_data.c" "${_ram}/pixels.obj" "${_ram}/pixmodes.obj" "${_ram}/learned.obj"
  "${_ram}/engines_builtin.obj")
add_dependencies(fmzx_app_image fmzx_ram_objects)
set_target_properties(fmzx_app_image PROPERTIES OUTPUT_NAME fmzx_app SUFFIX .elf
  LINK_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/app.ld;${_ram}/libzx_core.a;${_ram}/libzx_sound.a")
target_include_directories(fmzx_app_image PRIVATE "${_bsp}/chip" "${_bsp}/boards/fm1"
  $<TARGET_PROPERTY:fmzx_app,INTERFACE_INCLUDE_DIRECTORIES>
  $<TARGET_PROPERTY:zx_core,INTERFACE_INCLUDE_DIRECTORIES>
  $<TARGET_PROPERTY:zx_sound,INTERFACE_INCLUDE_DIRECTORIES>)
target_compile_definitions(fmzx_app_image PRIVATE FMZX_LCD_DIVIDER=${FMZX_LCD_DIVIDER}
  FMZX_LCD_RGB565=${_rgb565} "FMZX_PRODUCT=\"${FMZX_PRODUCT}\""
  $<TARGET_PROPERTY:zx_core,INTERFACE_COMPILE_DEFINITIONS>)
# felucca-bsp's headers are libraries of static helpers: the ones a unit does not use are fine.
set_source_files_properties(${_ours} PROPERTIES
  COMPILE_OPTIONS "-Wall;-Wextra;-Werror;-Wno-unused-function")
# The sound library's and the core's start-up functions (core/sound/sound_init.c,
# core/zx/beeper_init.c: XIP, they call nothing in RAMTEXT; main.c calls them directly), the
# effects follower's reader (core/sound/fx_read.c: XIP, it calls nothing in RAMTEXT; the mixer
# calls it through the pointer sound_init.c sets), the renamed sound library and core instead of
# zx_sound and zx_core themselves, then the rest of app/ (pixels.c and pixmodes.c come from the
# renamed objects above, so their archive members are never pulled in).
target_link_libraries(fmzx_app_image PRIVATE "$<TARGET_FILE:zx_sound_init>" "$<TARGET_FILE:zx_sound_fx>"
  "$<TARGET_FILE:zx_core_init>" "${_ram}/libzx_sound.a" "${_ram}/libzx_core.a" "$<TARGET_FILE:fmzx_app>")
add_dependencies(fmzx_app_image fmzx_app zx_core zx_core_init zx_sound zx_sound_init zx_sound_fx)
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

# Felucca's update loader (third_party/felucca-loader, GPL-3.0-only): a separate program that the
# update package carries as ota.bin, never linked into the image. Built with exactly the commands
# of Felucca 1.6.1's tools/build.py build_loader(), from the vendored tree as working directory,
# and none of our flags; the result must be Felucca's loader.bin byte for byte.
set(_ldr_src "${PROJECT_SOURCE_DIR}/third_party/felucca-loader")
set(_ldr "${PROJECT_BINARY_DIR}/third_party/felucca-loader")
set(_ldr_sources
  firmware/loader/crt0_ldr.S firmware/loader/ldr_core.c firmware/loader/loader.c firmware/loader/loader.ld
  firmware/src/usb.c firmware/src/ota.c firmware/src/libc.c
  firmware/hal/fm1_cc.h firmware/hal/fm1_flash.h firmware/hal/fm1_sys.h firmware/hal/fm1_time.h
  firmware/hal/fm1_usb.h firmware/hal/fm1_xip.h)
list(TRANSFORM _ldr_sources PREPEND "${_ldr_src}/" OUTPUT_VARIABLE _ldr_deps)
set(FMZX_LOADER_SIZE 8216)
set(FMZX_LOADER_SHA256 2cde42b729d787a3e591ed4eaaffbbf33cebee7d58421d33bf6b6d27cc03f1aa)
add_custom_command(OUTPUT "${_ldr}/loader.bin"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${_ldr}"
  COMMAND "${CMAKE_C_COMPILER}" -target pi32v2 -c firmware/loader/crt0_ldr.S -o "${_ldr}/crt0_ldr.o"
  COMMAND "${CMAKE_C_COMPILER}" -target pi32v2 -Os -ffunction-sections -fno-builtin -Wall -Wno-unused-function
          -Ifirmware/hal -Ifirmware/src -c firmware/loader/loader.c -o "${_ldr}/loader.o"
  COMMAND "${CMAKE_LINKER}" --gc-sections -e _start -T firmware/loader/loader.ld
          "${_ldr}/crt0_ldr.o" "${_ldr}/loader.o" -o "${_ldr}/loader.elf"
  COMMAND "${CMAKE_OBJCOPY}" -O binary -j .text "${_ldr}/loader.elf" "${_ldr}/loader.bin"
  COMMAND ${CMAKE_COMMAND} -DBIN=${_ldr}/loader.bin -DSIZE=${FMZX_LOADER_SIZE} -DSHA256=${FMZX_LOADER_SHA256}
          -P "${PROJECT_SOURCE_DIR}/cmake/check_loader.cmake"
  WORKING_DIRECTORY "${_ldr_src}"
  DEPENDS ${_ldr_deps} "${PROJECT_SOURCE_DIR}/cmake/check_loader.cmake"
  VERBATIM)
add_custom_target(felucca_loader ALL DEPENDS "${_ldr}/loader.bin")
