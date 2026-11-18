/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* USB: a class-compliant USB-MIDI device whenever the cable is in, which answers the FM-1 Firmware
 * Hub's handshake and takes the soft UBOOT key (felucca-bsp's chip/usb.c and chip/ota.c, usbota.c). */
#ifndef FMZX_USBOTA_H
#define FMZX_USBOTA_H

#include <stdint.h>

/* What usbota_service saw. */
#define USBOTA_NONE 0
#define USBOTA_UPDATE 1 /* the upgrade message, on a flash part the update may write */
#define USBOTA_UBOOT 2  /* the soft UBOOT key: the device has left the bus */

/* Once at boot, before TIMER5 starts: the flash's JEDEC ID read, a staging area an earlier update
 * left behind erased (on a part the update may write), the device attached. */
void usbota_init(void);

/* The TIMER5 interrupt, every fifth tick (2 kHz): the SIE polled. */
void usbota_poll_isr(void);

/* The main loop, between frames: a cable plugged in after boot attached. While no host is seen,
 * once a second (usbota_retry_due), the device is reset and attached again, which waits about 27 ms
 * (25 ms, and the SIE's timeouts without a cable). */
int usbota_retry_due(void);
void usbota_retry(void);

/* The main loop, between frames: the MIDI in from the computer dropped, a handshake answered.
 * Returns USBOTA_NONE, USBOTA_UPDATE or USBOTA_UBOOT. */
int usbota_service(void);

/* The flash's JEDEC ID read at boot (0x856014: the Puya part). */
uint32_t usbota_flash_id(void);

#endif
