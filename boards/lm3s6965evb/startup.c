/* What an LM3S6965 does before `main`, its clock, and its debugger.
 *
 * A chip has no system under the program. When it is switched on, the
 * processor reads two words from the start of the flash: where the
 * stack is, and where the first instruction is. Everything else is done
 * here: the variables are given their first values, a clock is started,
 * the first serial port is set up, and `main` is called.
 * `zeno build --board` compiles this file with the program
 * (docs/board-build.md).
 *
 * Nothing here asks the C library for anything: there is no `printf`,
 * no `exit` and no stream. What the program prints, it prints through
 * board.h. */

#include <stddef.h>
#include <stdint.h>

/* The linker script (link.ld) says where these are. */
extern uint32_t _stack_top, _data_in_flash, _data_start, _data_end, _bss_start, _bss_end;
extern char _heap_start;

int main(int count, char **words);

/* The SysTick timer every Cortex-M has: it counts down from RELOAD at
 * the speed of the processor, and interrupts when it gets to zero. */
#define SYSTICK_CONTROL (*(volatile uint32_t *)0xE000E010u)
#define SYSTICK_RELOAD (*(volatile uint32_t *)0xE000E014u)
#define SYSTICK_CURRENT (*(volatile uint32_t *)0xE000E018u)

/* The processor's clock, in Hz, as QEMU sets it after a reset. (A real
 * LM3S6965 starts on an oscillator of its own that is 30% uncertain,
 * and a program for one would switch to the crystal here, and say its
 * speed on this line and on the same line of board.h.) */
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

/* Semihosting: a breakpoint with the number 0xAB is a call to the
 * debugger that holds the board, or to QEMU, which answers as one.
 * (With no debugger on its pins a real chip stops at the breakpoint
 * with a fault. A board that runs alone would send its messages out of
 * a serial port and wait in a loop: two functions to change.) */
static int debugger(int what, void *with) {
    register int r0 __asm__("r0") = what;
    register void *r1 __asm__("r1") = with;
    __asm__ volatile("bkpt 0xAB" : "+r"(r0) : "r"(r1) : "memory");
    return r0;
}

#define DEBUGGER_WRITE_CHARACTER 0x03
#define DEBUGGER_EXIT 0x20
#define THE_PROGRAM_ENDED 0x20026

/* One character for whoever watches the board: the message of an
 * error, and a warning. */
void board_says(char one) {
    debugger(DEBUGGER_WRITE_CHARACTER, &one);
}

/* The program is over. The debugger is told how it ended, and QEMU
 * ends with that number. */
__attribute__((noreturn)) void board_stops(int status) {
    int said[2] = {THE_PROGRAM_ENDED, status};
    debugger(DEBUGGER_EXIT, said);
    for (;;) {
    }
}

/* The two things the C library still asks for. A number with a
 * fraction is made into text by the C library's `snprintf`, and that
 * is the one piece of it a Zeno program on this board still uses. A
 * program that shows no such number never comes here, and the linker
 * leaves both out.
 *
 * The first is memory: `snprintf` takes a little while it works, from
 * the end of the variables up to the stack, less what the stack is
 * given. */
#define STACK_ROOM 8192

void *_sbrk(ptrdiff_t more) {
    static char *end = &_heap_start;
    if (end + more > (char *)&_stack_top - STACK_ROOM) return (void *)-1;
    char *was = end;
    end += more;
    return was;
}

/* The second is what to do if the C library finds a fault in itself.
 * Its own answer is to print the place with `fprintf` and stop, which
 * would bring every stream of the C library into the program: 20 KB of
 * flash for a message nobody is meant to see. Here it stops. */
__attribute__((noreturn)) void __assert_func(const char *file, int line, const char *function, const char *what) {
    (void)file;
    (void)line;
    (void)function;
    (void)what;
    board_stops(1);
}

/* The first serial port, UART0, is where the program prints. Its two
 * pins are PA0 and PA1. */
#define CLOCK_UARTS (*(volatile uint32_t *)0x400FE104u)
#define CLOCK_PORTS (*(volatile uint32_t *)0x400FE108u)
#define PORT_A_OTHER_USE (*(volatile uint32_t *)0x40004420u)
#define PORT_A_DIGITAL (*(volatile uint32_t *)0x4000451Cu)
#define UART0_WHOLE (*(volatile uint32_t *)0x4000C024u)
#define UART0_FRACTION (*(volatile uint32_t *)0x4000C028u)
#define UART0_LINE (*(volatile uint32_t *)0x4000C02Cu)
#define UART0_CONTROL (*(volatile uint32_t *)0x4000C030u)
#define CONSOLE_BAUD 115200u

static void reset(void) {
    /* The first values of the variables are in flash; the variables are in RAM. */
    uint32_t *from = &_data_in_flash;
    for (uint32_t *to = &_data_start; to < &_data_end; to++) *to = *from++;
    for (uint32_t *to = &_bss_start; to < &_bss_end; to++) *to = 0;

    SYSTICK_RELOAD = CLOCK_HZ / 1000u - 1u;
    SYSTICK_CURRENT = 0;
    SYSTICK_CONTROL = 7; /* on, with its interrupt, at the processor's speed */

    /* UART0 and port A get their clocks, the two pins are handed to
     * the UART, and it is set to 8 bits, no parity, one stop bit. The
     * speed is a divisor of the clock, in sixty-fourths. */
    CLOCK_UARTS |= 1u;
    CLOCK_PORTS |= 1u;
    PORT_A_OTHER_USE |= 3u;
    PORT_A_DIGITAL |= 3u;
    uint32_t divisor = (CLOCK_HZ * 4u + CONSOLE_BAUD / 2u) / CONSOLE_BAUD;
    UART0_CONTROL = 0;
    UART0_WHOLE = divisor / 64u;
    UART0_FRACTION = divisor % 64u;
    UART0_LINE = 0x60; /* 8 bits, one byte held at a time */
    UART0_CONTROL = 0x301; /* on, sending and receiving */

    /* Nobody types words after the name of a program on a board. */
    static char name[] = "program";
    static char *words[] = {name, NULL};
    board_stops(main(1, words));
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
