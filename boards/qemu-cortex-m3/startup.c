/* What an ARM Cortex-M3 does before `main`, and its clock.
 *
 * A chip has no system under the program. When it is switched on, the
 * processor reads two words from the start of the flash: where the
 * stack is, and where the first instruction is. Everything else is done
 * here: the variables are given their first values, a clock is started,
 * and `main` is called. `zeno build --board` compiles this file with
 * the program (docs/board-build.md). */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

/* The linker script (link.ld) says where these are. */
extern uint32_t _stack_top, _data_in_flash, _data_start, _data_end, _bss_start, _bss_end;

int main(int count, char **words);

/* The SysTick timer every Cortex-M has: it counts down from RELOAD at
 * the speed of the processor, and interrupts when it gets to zero. */
#define SYSTICK_CONTROL (*(volatile uint32_t *)0xE000E010u)
#define SYSTICK_RELOAD (*(volatile uint32_t *)0xE000E014u)
#define SYSTICK_CURRENT (*(volatile uint32_t *)0xE000E018u)

/* The processor's clock, in Hz, as QEMU sets it after a reset. (A real
 * LM3S6965 starts on an oscillator of its own that is 30% uncertain,
 * and a program for one would switch to the crystal here.) */
#define CLOCK_HZ 12500000u

/* Milliseconds since the reset: one more at each interrupt. */
static volatile uint32_t milliseconds;

static void every_millisecond(void) {
    milliseconds++;
}

/* The time since the reset, in milliseconds and millionths of one. */
void board_time(uint32_t *whole, uint32_t *millionths) {
    uint32_t before, left;
    /* Read again if an interrupt came between the two readings. */
    do {
        before = milliseconds;
        left = SYSTICK_CURRENT;
    } while (before != milliseconds);
    *whole = before;
    *millionths = (uint32_t)(((uint64_t)(CLOCK_HZ / 1000u - 1u - left) * 1000000u) / (CLOCK_HZ / 1000u));
}

/* Sleep until the next interrupt: at most a millisecond. */
void board_doze(void) {
    __asm__ volatile("wfi");
}

static void reset(void) {
    /* The first values of the variables are in flash; the variables are in RAM. */
    uint32_t *from = &_data_in_flash;
    for (uint32_t *to = &_data_start; to < &_data_end; to++) *to = *from++;
    for (uint32_t *to = &_bss_start; to < &_bss_end; to++) *to = 0;

    SYSTICK_RELOAD = CLOCK_HZ / 1000u - 1u;
    SYSTICK_CURRENT = 0;
    SYSTICK_CONTROL = 7; /* on, with its interrupt, at the processor's speed */

    /* Each character goes out when it is printed, so that what a
     * program prints and what it writes to a serial port stay in order. */
    setvbuf(stdout, NULL, _IONBF, 0);

    /* Nobody types words after the name of a program on a board. A
     * program that ends says so (system.c: `_exit`). */
    static char name[] = "program";
    static char *words[] = {name, NULL};
    exit(main(1, words));
}

/* A fault, or an interrupt nothing here asked for: stop where it can be found. */
static void stuck(void) {
    for (;;) {
    }
}

/* The table the processor reads: the stack, the reset, the faults, and
 * the interrupts of the processor itself. SysTick is the sixteenth. */
__attribute__((section(".vectors"), used)) static void (*const vectors[16])(void) = {
    (void (*)(void))&_stack_top,
    reset,
    stuck, /* NMI */
    stuck, /* hard fault */
    stuck, /* memory fault */
    stuck, /* bus fault */
    stuck, /* usage fault */
    0,
    0,
    0,
    0,
    stuck, /* SVCall */
    stuck, /* debug monitor */
    0,
    stuck, /* PendSV */
    every_millisecond,
};
