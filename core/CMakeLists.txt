# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Keep the definitions and include dirs in step with DEFINES in host/crates/zx-sys/build.rs.
# ZX_TESTING is host-only and is not defined here.

# tapesig.c (the tape signal player) is for the host build only: the firmware's tape_ear stays NULL.
file(GLOB ZX_CORE_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/zx/*.c")
list(FILTER ZX_CORE_SOURCES EXCLUDE REGEX "/tapesig\\.c$")
set(ZX_VENDOR_SOURCES
  "${PROJECT_SOURCE_DIR}/third_party/Z80/sources/Z80.c"
  "${PROJECT_SOURCE_DIR}/third_party/Z80InsnClock/sources/Z80InsnClock.c")

add_library(zx_core STATIC ${ZX_CORE_SOURCES} ${ZX_VENDOR_SOURCES})

# Our sources are held to -Wall -Wextra -Werror; the vendored libraries are built as they are.
set_source_files_properties(${ZX_CORE_SOURCES} PROPERTIES COMPILE_OPTIONS "-Wall;-Wextra;-Werror")

target_include_directories(zx_core PUBLIC
  "${CMAKE_CURRENT_SOURCE_DIR}/z80"
  "${CMAKE_CURRENT_SOURCE_DIR}/zx"
  "${PROJECT_SOURCE_DIR}/third_party/Z80/API"
  "${PROJECT_SOURCE_DIR}/third_party/Z80InsnClock/API")

target_compile_definitions(zx_core PUBLIC
  Z80_STATIC
  Z80_WITH_LOCAL_HEADER
  Z80_WITH_Q
  Z80_WITH_ZILOG_NMOS_LD_A_IR_BUG
  Z80_WITH_PRECOMPUTED_DAA
  Z80_WITH_VOLATILE_CYCLES
  "Z80_EXTERNAL_HEADER=\"z_types.h\""
  Z80_INSN_CLOCK_EXTERNAL_HEADER
  Z80_INSN_CLOCK_STATIC
  Z80_INSN_CLOCK_WITH_LOCAL_HEADER
  Z80_INSN_CLOCK=2)

# FM music from the beeper (core/sound): portable fixed-point C beside the core, held to the same
# warnings. The firmware links it when it plays FM.
# engines_test.c (the test fixtures' engine table) is for the host build only.
file(GLOB ZX_SOUND_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/sound/*.c")
list(FILTER ZX_SOUND_SOURCES EXCLUDE REGEX "/engines_test\\.c$")
# learned.c (learned entries: the interpreter, the decoder and the generic reader) runs from flash,
# not SRAM: the firmware builds it itself (platform/fm1/CMakeLists.txt).
list(FILTER ZX_SOUND_SOURCES EXCLUDE REGEX "/learned\\.c$")
# sound_init.c (the start-up functions, once per game start) runs from flash too: its own library,
# zx_sound_init, whose archive name the firmware's linker script does not put in SRAM.
list(FILTER ZX_SOUND_SOURCES EXCLUDE REGEX "/sound_init\\.c$")
add_library(zx_sound STATIC ${ZX_SOUND_SOURCES})
target_compile_options(zx_sound PRIVATE -Wall -Wextra -Werror)
# Method B (engines.c) runs a few reads and one pass over the frame's edges: built for size, as
# all of core/sound sits in the firmware's 48 KiB of SRAM code.
set_source_files_properties("${CMAKE_CURRENT_SOURCE_DIR}/sound/engines.c" PROPERTIES COMPILE_OPTIONS "-Os")
target_include_directories(zx_sound PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/sound")
target_link_libraries(zx_sound PUBLIC zx_core)

add_library(zx_sound_init STATIC "${CMAKE_CURRENT_SOURCE_DIR}/sound/sound_init.c")
target_compile_options(zx_sound_init PRIVATE -Wall -Wextra -Werror)
target_link_libraries(zx_sound_init PUBLIC zx_sound)
