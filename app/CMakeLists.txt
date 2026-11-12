# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Hardware-free firmware logic, built for the device and (by host/crates/zx-sys/build.rs) for host tests.

file(GLOB FMZX_APP_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/*.c")

add_library(fmzx_app STATIC ${FMZX_APP_SOURCES})
target_compile_options(fmzx_app PRIVATE -Wall -Wextra -Werror)
target_include_directories(fmzx_app PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}")
target_link_libraries(fmzx_app PUBLIC zx_core)
