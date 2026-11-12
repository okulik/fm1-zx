# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

cmake_minimum_required(VERSION 3.20)
project(fmzx C ASM)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

include(cmake/embed.cmake)

if(CMAKE_CROSSCOMPILING)
  add_compile_options(-ffunction-sections -fdata-sections)
endif()

add_subdirectory(core)
add_subdirectory(app)

if(CMAKE_CROSSCOMPILING)
  add_subdirectory(bench/fm1)
  add_subdirectory(platform/fm1)
endif()
