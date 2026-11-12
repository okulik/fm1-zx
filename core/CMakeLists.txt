# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Keep the definitions and include dirs in step with DEFINES in host/crates/zx-sys/build.rs.
# ZX_TESTING is host-only and is not defined here.

file(GLOB ZX_CORE_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/zx/*.c")
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
