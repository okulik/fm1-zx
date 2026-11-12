/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The ST7789 240x240 panel on SPI1: bring-up, and the rows the app/lcdsched.h scheduler picks,
 * sent a band at a time by DMA from two band buffers in POOL (one composed while the other is
 * sent). The panel's display goes on (DISPON) once every row has been sent once. */
#ifndef FMZX_LCD_H
#define FMZX_LCD_H

#include <stdint.h>
#include "lcdsched.h"

/* Before interrupts are enabled (PA2 is both the backlight and the key matrix's /OE). fmt is
 * PIX_RGB444 or PIX_RGB565. Leaves the SPI clock at FMZX_LCD_DIVIDER. */
void lcd_init(int fmt);
/* SPI1 clock 60 MHz / (d + 1); waits for a band in flight first. */
void lcd_set_divider(unsigned d);
unsigned lcd_divider(void);
int lcd_format(void);
unsigned lcd_row_bytes(void);

/* Starts the next band when the DMA is idle and the scheduler has one, and composes the band
 * after it meanwhile; returns at once otherwise. Call it continuously from the main loop. */
void lcd_pump(void);
/* A band's DMA is in flight. */
int lcd_busy(void);

/* The scheduler (app/lcdsched.h), frame numbers as the caller counts them. */
void lcd_mark_row(unsigned row, uint32_t frame);
void lcd_mark_all(uint32_t frame);
void lcd_begin_frame(uint32_t frame, uint32_t budget_bytes);
unsigned lcd_pending(void);
const lcd_stats *lcd_get_stats(void);

/* Supplied by the application, called from lcd_pump: composes LCD rows row0 .. row0 + rows - 1
 * (rows <= LCD_BAND_ROWS) as panel bytes, lcd_row_bytes() a row, into lcd_band_out(). */
void lcd_compose_band(unsigned row0, unsigned rows);
uint8_t *lcd_band_out(void);

#endif
