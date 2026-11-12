# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Cross toolchain file for the JieLi pi32v2 CPU (clang 4.0.1, bare metal).

set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR pi32v2)

set(_jieli "$ENV{JIELI_TOOLCHAIN}")
set(_clang "${_jieli}/pi32v2/bin/clang")
if(NOT _jieli OR NOT EXISTS "${_clang}")
  message(FATAL_ERROR
    "JieLi pi32v2 clang not found (JIELI_TOOLCHAIN='${_jieli}'). "
    "Run this through tools/fm1-docker, which provides the pinned toolchain image.")
endif()

set(CMAKE_C_COMPILER "${_clang}")
set(CMAKE_ASM_COMPILER "${_clang}")
set(CMAKE_C_COMPILER_TARGET pi32v2)
set(CMAKE_ASM_COMPILER_TARGET pi32v2)
set(CMAKE_C_COMPILER_FORCED TRUE)
set(CMAKE_ASM_COMPILER_FORCED TRUE)

set(CMAKE_LINKER "${_jieli}/pi32v2/bin/ld")
set(CMAKE_AR "${_jieli}/pi32v2/bin/ar" CACHE FILEPATH "")
set(CMAKE_OBJCOPY "${_jieli}/common/bin/objcopy" CACHE FILEPATH "")
set(CMAKE_OBJDUMP "${_jieli}/common/bin/objdump" CACHE FILEPATH "")
set(FMZX_NM "${_jieli}/pi32v2/bin/nm" CACHE FILEPATH "")

include_directories(SYSTEM "${_jieli}/pi32v2/include")

set(_flags "-mcpu=r3 -mfprev1 -ffreestanding -fno-builtin")
set(CMAKE_C_FLAGS_INIT "${_flags}")
set(CMAKE_ASM_FLAGS_INIT "${_flags}")
set(CMAKE_C_FLAGS_RELEASE_INIT "-O2 -DNDEBUG")

set(CMAKE_C_LINK_EXECUTABLE "<CMAKE_LINKER> <LINK_FLAGS> <OBJECTS> -o <TARGET> <LINK_LIBRARIES>")
set(CMAKE_ASM_LINK_EXECUTABLE "<CMAKE_LINKER> <LINK_FLAGS> <OBJECTS> -o <TARGET> <LINK_LIBRARIES>")

set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
