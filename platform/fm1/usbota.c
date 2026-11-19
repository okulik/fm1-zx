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
/* OTA_FLASH_ID stays ota.c's default (0): the identity field is the text and zeros, which the FM-1
 * Firmware Hub's page needs (it reads everything before the trailing zeros as the identity) */
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
    /* nothing here plays MIDI: what the computer sends to the port is dropped, as in usbota_service,
     * so it never fills the ring and makes EP1 refuse the host's replies */
    mi_r = mi_w;
    midi_in_overflow = 0;
    fm1_wdt_feed();
}

static int ota_in_area(uint32_t off, uint32_t n)
{
    return FL_IN(off, n, OTA_AREA, OTA_AREA + OTA_AREA_LEN);
}

static int ota_erase(uint32_t off)
{
    uint32_t took;
    int rc;
    if (!ota_in_area(off, 0x1000u) || (off & 0xFFFu))
        return -8;
    rc = fl_erase4k(off, &took);
    fm1_wdt_feed(); /* five erases in a row: each may take seconds on a slow part */
    return rc;
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

/* ota_session's steps 1-5, as usbota_screen shows them */
static const char *const OTA_STEPS[] = {"", "PACKAGE", "CHECK HEAD", "LOADER", "CONFIRM", "RESTART"};
#define OTA_FAIL_MS 1500u /* a failure stays on the panel this long */

static uint32_t shown_step; /* the step last painted: usbota_run_update's PACKAGE is not painted twice */

static void ota_show(uint32_t step, int32_t code)
{
    char line[16] = "FAILED -";
    uint32_t i = 8, v, t;
    if (step < sizeof OTA_STEPS / sizeof OTA_STEPS[0]) {
        if (step != shown_step)
            usbota_screen(OTA_STEPS[step]);
        shown_step = step;
        return;
    }
    v = code < 0 ? (uint32_t)-code : (uint32_t)code;
    if (v >= 10u)
        line[i++] = (char)('0' + v / 10u % 10u);
    line[i++] = (char)('0' + v % 10u);
    line[i] = 0;
    usbota_screen(line);
    for (t = fm1_ms; fm1_ms - t < OTA_FAIL_MS;)
        fm1_wdt_feed();
}

static void ota_commit(const uint8_t *parm)
{
    usb_detach(); /* the host sees the device leave before the chip resets */
    fm1_delay_ms(30);
    fmzx_enter_update(parm);
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

/* the flash.bin felucca-bsp's update loader takes (loader/ldr_core.c, ldr_head: an entry of type 0,
 * from LDR_APP_HI to LDR_FL_MAX long): up to the app area's end at least, the 1 MiB flash at most */
#define FLASH_BIN_MIN 0x93000u
#define FLASH_BIN_MAX 0x100000u
#define OTA_ERR_LAYOUT (-15) /* a flash.bin it refuses (ota.c's codes end at -14) */

/* The package's loader felucca-bsp's: ota.c's test in ota_stage (its marker in the first LZ4
 * literals, a size it can stage), on the loader's first 512 bytes. */
static int ota_loader_own(const uint8_t *b)
{
    uint32_t i;
    for (i = 0x48; i + 16u <= 0x48u + 64u; i++)
        if (ota_memeq(b + i, (const uint8_t *)"FELUCCA-LOADER-1", 16) && ota_rd32(b + 0x20 + 4) <= 0x14000u)
            return 1;
    return 0;
}

void usbota_run_update(void)
{
    static uint8_t b[0x400];
    uint32_t fl_off, fl_len, ota_off, ota_len;
    int rc;
    /* the package's entry list and its loader's head read first, as ota_session reads them again:
     * with felucca-bsp's loader, ota_session does not check flash.bin, and a flash.bin that loader
     * refuses after the restart is refused here, before ota_session erases the OTA area. Another
     * writer's package goes on to ota_session's own checks */
    shown_step = 0;
    ota_deadline = 0; /* no time limit on these reads (a failed session leaves its limit set) */
    ota_show(1, 0);
    rc = ota_ufw(b, &fl_off, &fl_len, &ota_off, &ota_len);
    if (!rc && ota_off) {
        if (ota_read(ota_off, b, 512))
            rc = -6;
        else if (ota_loader_own(b) && (!fl_off || fl_len < FLASH_BIN_MIN || fl_len > FLASH_BIN_MAX))
            rc = OTA_ERR_LAYOUT;
    }
    if (rc) {
        ota_show(9, rc);
        return;
    }
    ota_session();
}
