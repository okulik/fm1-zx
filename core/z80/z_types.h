/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Minimal stand-in for the Zeta headers that the vendored Z80 and
 * Z80InsnClock sources would otherwise include. Only the types and macros
 * those two libraries use are defined. Little-endian targets only. */

#ifndef ZX_Z_TYPES_H
#define ZX_Z_TYPES_H

#include <stddef.h>
#include <stdint.h>

#if !defined(__BYTE_ORDER__) || __BYTE_ORDER__ != __ORDER_LITTLE_ENDIAN__
#	error "z_types.h supports little-endian targets only"
#endif

typedef uint8_t  zuint8;
typedef uint16_t zuint16;
typedef uint32_t zuint32;
typedef size_t   zusize;
typedef int      zsint;
typedef int8_t   zsint8;
typedef char     zchar;
typedef unsigned zuint;
typedef int      zbool;

typedef union {
	zuint16 uint16_value;
	struct {zuint8 at_0, at_1;} uint8_values;
} ZInt16;

typedef union {
	zuint32 uint32_value;
	zuint16 uint16_array[2];
	zuint8  uint8_array[4];
} ZInt32;

#define Z_NULL ((void *)0)
#define Z_EMPTY
#define Z_SAME(x) x
#define Z_API_IMPORT
#define Z_API_EXPORT
#define Z_EXTERN_C_BEGIN
#define Z_EXTERN_C_END
#define Z_MICROSOFT_STD_CALL
#define Z_ALWAYS_INLINE inline __attribute__((always_inline))
#define Z_CAST(type) (type)
#define Z_UNUSED(x) (void)(x);

#define Z_UINT16(x) UINT16_C(x)
#define Z_UINT32(x) UINT32_C(x)
#define Z_USIZE(x) ((zusize)(x))
#define Z_USIZE_MAXIMUM SIZE_MAX

#define Z_UINT16_BIG_ENDIAN(x) __builtin_bswap16(x)
#define Z_UINT32_BIG_ENDIAN(x) __builtin_bswap32(x)

#define Z_UINT8_ROTATE_LEFT(v, n)  ((zuint8)(((v) << (n)) | ((v) >> (8 - (n)))))
#define Z_UINT8_ROTATE_RIGHT(v, n) ((zuint8)(((v) >> (n)) | ((v) << (8 - (n)))))

#define Z_MEMBER_OFFSET(type, member) ((zuint8)offsetof(type, member))

#endif
