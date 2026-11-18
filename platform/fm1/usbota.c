/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* USB through felucca-bsp (MPL-2.0, platform/fm1/VENDORED.md): its USB-MIDI device (chip/usb.c)
 * and its update entry (chip/ota.c), included into this one unit as felucca-bsp's own firmwares
 * include them, and its flash driver (fm1_flash.h), which may write felucca-bsp's OTA area only.
 * The identity (FELUCCA_ID) and the USB product string (FELUCCA_USB_PRODUCT) come from CMake.
 * Not built with felucca-bsp's console (FELUCCA_CDC 0) or USB audio. */

#include <stdint.h>

#include "fm1_cc.h"
#include "fm1_time.h"
#include "fm1_sys.h"

#include "usbota.h"

#define FELUCCA_OTA 1
#define FELUCCA_CDC 0
/* the identity reply's bytes 24..26: the ID read at boot */
#define OTA_FLASH_ID() flash_id
/* the only flash this firmware writes while it runs: the OTA area (ota.c's OTA_AREA, OTA_AREA_LEN) */
#define FM1_FLASH_WRITE_REGIONS(X) X(0xE0000u, 0xE5000u)
/* the rings are shared with the TIMER5 interrupt on the one core: a compiler barrier */
#define RING_PUBLISH() __asm__ volatile("" ::: "memory")
/* the driver's RAM functions in a section of their own (app.ld: .ram_flash, in the RAM region, not
 * RAMTEXT), and only those this unit calls (no `used`: the compiler drops the others) */
#define RAMFN __attribute__((section(".ram_flash"), noinline))

static uint32_t flash_id;

#include "fm1_flash.h"
#include "usb.c"
#include "ota.c"

_Static_assert(OTA_AREA == 0xE0000u && OTA_AREA_LEN == 0x5000u, "FM1_FLASH_WRITE_REGIONS is ota.c's OTA area");

#define JEDEC_PUYA 0x856014u     /* the FM-1's usual part */
#define JEDEC_UC25HQ80 0xB36014u /* the other part FM-1s ship with */

static uint32_t ms_last; /* fm1_ticks() at the last millisecond counted (usbota_poll_isr) */

/* A flash part the update may write. */
static int flash_ok(void)
{
    return flash_id == JEDEC_PUYA || flash_id == JEDEC_UC25HQ80;
}

/* ---- ota.c's hooks ---- */

static uint32_t ota_now_ms(void)
{
    return fm1_ms;
}

static void ota_idle(void)
{
    fm1_wdt_feed();
}

static int ota_in_area(uint32_t off, uint32_t n)
{
    return FL_IN(off, n, OTA_AREA, OTA_AREA + OTA_AREA_LEN);
}

static int ota_erase(uint32_t off)
{
    uint32_t took;
    if (!ota_in_area(off, 0x1000u) || (off & 0xFFFu))
        return -8;
    return fl_erase4k(off, &took);
}

static int ota_prog(uint32_t off, const void *p, uint32_t n)
{
    if (!ota_in_area(off, n))
        return -8;
    return fl_write(off, p, n);
}

static int ota_fread(uint32_t off, void *p, uint32_t n)
{
    return fl_read(off, p, n);
}

/* ota_session's progress: not shown (the session is not run) */
static void ota_show(uint32_t step, int32_t code)
{
    (void)step;
    (void)code;
}

static void ota_commit(const uint8_t *parm)
{
    usb_detach();
    fm1_delay_ms(30);
    fm1_enter_update(parm);
}

/* ---- usbota.h ---- */

void usbota_init(void)
{
    flash_id = fl_jedec();
    if (flash_ok())
        ota_boot_cleanup();
    usb_start();
    ms_last = fm1_ticks();
}

void usbota_poll_isr(void)
{
    uint32_t t = fm1_ticks();
    while ((uint32_t)(t - ms_last) >= 1000u * FM1_TICKS_PER_US) { /* usb.c's clock, ota.c's too */
        ms_last += 1000u * FM1_TICKS_PER_US;
        fm1_ms++;
    }
    usb_poll();
}

int usbota_retry_due(void)
{
    return !usb.up && !usb.detached && fm1_ms - usb_retry_ms >= 1000u;
}

void usbota_retry(void)
{
    usb_retry(fm1_ms);
}

int usbota_service(void)
{
    /* nothing here plays MIDI: what came in is dropped, so notes never hold a SysEx back */
    mi_r = mi_w;
    midi_in_overflow = 0;
    ota_service();
    if (usb.uboot_req) {
        usb.uboot_req = 0;
        usb_detach(); /* the host sees the device leave before the chip resets */
        fm1_delay_ms(30);
        return USBOTA_UBOOT;
    }
    if (usb.ota_req) {
        usb.ota_req = 0;
        if (flash_ok())
            return USBOTA_UPDATE;
    }
    return USBOTA_NONE;
}

uint32_t usbota_flash_id(void)
{
    return flash_id;
}
