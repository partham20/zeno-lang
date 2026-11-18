/* The LM3S6965 evaluation board under a Zeno program.
 *
 * `zeno build --board boards/lm3s6965evb` puts this file between the
 * program and the chip (docs/board.md has the list of functions a
 * board supplies, and what each must do). Everything here is a
 * register of the chip, written from its data sheet. QEMU emulates the
 * board, and that is the only place this file has run: it has never
 * run on the chip itself.
 *
 * The pins. The chip has seven ports of eight pins, A to G, and a
 * Zeno program numbers them in that order:
 *
 *      0 to  7   PA0 to PA7        32 to 39   PE0 to PE7
 *      8 to 15   PB0 to PB7        40 to 47   PF0 to PF7
 *     16 to 23   PC0 to PC7        48 to 55   PG0 to PG7
 *     24 to 31   PD0 to PD7
 *
 * To a Zeno program the LED is always pin 13. On this board the LED is
 * on PF0, so 13 is PF0 here, as 40 is. (PB5, which the table would
 * give that number, is not reached from Zeno on this board.) The
 * button marked "select" is PF1: `input_pin(41, pull_up)`.
 *
 * The serial ports. `serial(0, ...)` is UART0, where `print` goes too,
 * `serial(1, ...)` is UART1 (pins PD2 and PD3) and `serial(2, ...)` is
 * UART2 (PG0 and PG1).
 *
 * There is no analog pin, PWM pin or interrupt here yet: a program
 * that asks for one is not built for this board. */

/* This board prints, warns, reads and stops itself (decision 52). */
#define ZENO_BOARD_PRINTS

/* startup.c */
void board_time(uint32_t *whole, uint32_t *millionths);
void board_doze(void);
void board_says(char one);
__attribute__((noreturn)) void board_stops(int status);

/* The speed of the processor, as in startup.c. */
#define BOARD_CLOCK_HZ 12500000u

#define BOARD_REGISTER(address) (*(volatile uint32_t *)(address))

/* A port or a UART does nothing until it is given a clock. */
#define BOARD_CLOCK_UARTS BOARD_REGISTER(0x400FE104u)
#define BOARD_CLOCK_PORTS BOARD_REGISTER(0x400FE108u)

/* ------------------------------------------------------------------ Pins */

#define ZENO_PINS 56

static const uint32_t board_ports[7] = {0x40004000u, 0x40005000u, 0x40006000u, 0x40007000u, 0x40024000u, 0x40025000u, 0x40026000u};

/* The registers of a port, from where the port begins. The level of a
 * pin is read and written at an address that names the pin: the bits
 * of the address say which pins of the port are meant, so that one pin
 * is changed and the other seven are left alone, in one step. */
#define BOARD_PIN_LEVEL(port, bit) BOARD_REGISTER((port) + ((uint32_t)(bit) << 2))
#define BOARD_PIN_OUT(port) BOARD_REGISTER((port) + 0x400u)
#define BOARD_PIN_OTHER_USE(port) BOARD_REGISTER((port) + 0x420u)
#define BOARD_PIN_PULL_UP(port) BOARD_REGISTER((port) + 0x510u)
#define BOARD_PIN_PULL_DOWN(port) BOARD_REGISTER((port) + 0x514u)
#define BOARD_PIN_DIGITAL(port) BOARD_REGISTER((port) + 0x51Cu)

/* Where a pin is: its port, with the port's clock on, and its bit. */
static uint32_t board_pin(int pin, uint32_t *bit) {
    if (pin == ZENO_LED) pin = 40;
    BOARD_CLOCK_PORTS |= 1u << (pin / 8);
    *bit = 1u << (pin % 8);
    return board_ports[pin / 8];
}

static void zeno_board_write(int pin, bool level) {
    uint32_t bit, port = board_pin(pin, &bit);
    /* The pin is made an output, given its level, and only then joined
     * to the world, so that it never drives a level nobody asked for. */
    BOARD_PIN_OUT(port) |= bit;
    BOARD_PIN_LEVEL(port, bit) = level ? bit : 0u;
    BOARD_PIN_DIGITAL(port) |= bit;
}

static bool zeno_board_read(int pin) {
    uint32_t bit, port = board_pin(pin, &bit);
    return BOARD_PIN_LEVEL(port, bit) != 0u;
}

/* An input: nothing pulls it (0), or the chip's own pull-up (1) or
 * pull-down (2) does. */
static void zeno_board_input(int pin, int pull) {
    uint32_t bit, port = board_pin(pin, &bit);
    BOARD_PIN_OUT(port) &= ~bit;
    if (pull == 1) BOARD_PIN_PULL_UP(port) |= bit; else BOARD_PIN_PULL_UP(port) &= ~bit;
    if (pull == 2) BOARD_PIN_PULL_DOWN(port) |= bit; else BOARD_PIN_PULL_DOWN(port) &= ~bit;
    BOARD_PIN_DIGITAL(port) |= bit;
}

/* ------------------------------------------------------------- The clock */

/* Milliseconds since the reset. */
static double zeno_board_now(void) {
    uint32_t whole, millionths;
    board_time(&whole, &millionths);
    return (double)whole + (double)millionths / 1e6;
}

/* The processor sleeps between the ticks of the clock. */
static void zeno_board_sleep(double seconds) {
    double until = zeno_board_now() + seconds * 1000.0;
    while (zeno_board_now() < until) board_doze();
}

/* ------------------------------------------------------ The serial ports */

#define ZENO_SERIALS 3

static const uint32_t board_uarts[ZENO_SERIALS] = {0x4000C000u, 0x4000D000u, 0x4000E000u};

#define BOARD_UART_DATA(uart) BOARD_REGISTER((uart) + 0x000u)
#define BOARD_UART_FLAGS(uart) BOARD_REGISTER((uart) + 0x018u)
#define BOARD_UART_WHOLE(uart) BOARD_REGISTER((uart) + 0x024u)
#define BOARD_UART_FRACTION(uart) BOARD_REGISTER((uart) + 0x028u)
#define BOARD_UART_LINE(uart) BOARD_REGISTER((uart) + 0x02Cu)
#define BOARD_UART_CONTROL(uart) BOARD_REGISTER((uart) + 0x030u)
#define BOARD_UART_NOTHING_IN 0x10u
#define BOARD_UART_NO_ROOM 0x20u

static void board_uart_send(uint32_t uart, const char *bytes, size_t count) {
    for (size_t i = 0; i < count; i++) {
        while (BOARD_UART_FLAGS(uart) & BOARD_UART_NO_ROOM) {
        }
        BOARD_UART_DATA(uart) = (uint32_t)(unsigned char)bytes[i];
    }
}

/* The next byte, after waiting for it: a serial port has no end. */
static int board_uart_next(uint32_t uart) {
    while (BOARD_UART_FLAGS(uart) & BOARD_UART_NOTHING_IN) board_doze();
    return (int)(BOARD_UART_DATA(uart) & 0xFFu);
}

/* The port gets its clock and its two pins, and its speed: a divisor
 * of the processor's clock, in sixty-fourths. 8 bits, no parity, one
 * stop bit, and one byte held at a time. (The chip can queue sixteen.
 * A board that is sent bytes faster than its program reads them would
 * turn the queue on, and take them in an interrupt.) */
static void zeno_board_serial_open(int port, double baud) {
    /* The pins of each UART, receive then send: PA0 PA1, PD2 PD3, PG0 PG1. */
    static const int pins[ZENO_SERIALS] = {0, 26, 48};
    uint32_t bit, where = board_pin(pins[port], &bit);
    BOARD_PIN_OTHER_USE(where) |= bit * 3u;
    BOARD_PIN_DIGITAL(where) |= bit * 3u;
    BOARD_CLOCK_UARTS |= 1u << port;
    uint32_t uart = board_uarts[port];
    uint32_t divisor = (uint32_t)((double)BOARD_CLOCK_HZ * 4.0 / baud + 0.5);
    BOARD_UART_CONTROL(uart) = 0;
    BOARD_UART_WHOLE(uart) = divisor / 64u;
    BOARD_UART_FRACTION(uart) = divisor % 64u;
    BOARD_UART_LINE(uart) = 0x60;
    BOARD_UART_CONTROL(uart) = 0x301;
}

static void zeno_board_serial_write(int port, const uint8_t *bytes, size_t count) {
    board_uart_send(board_uarts[port], (const char *)bytes, count);
}

static bool zeno_board_serial_ready(int port) {
    return (BOARD_UART_FLAGS(board_uarts[port]) & BOARD_UART_NOTHING_IN) == 0u;
}

static int zeno_board_serial_read(int port) {
    return board_uart_next(board_uarts[port]);
}

/* ------------------------------------------- Printing, reading, stopping */

/* What the program prints goes out of UART0 (startup.c set it up). */
static void zeno_board_print(const char *bytes, size_t count) {
    board_uart_send(board_uarts[0], bytes, count);
}

/* A warning, and the message of the error that stops the program, go
 * to the debugger. */
static void zeno_board_warn(const char *bytes, size_t count) {
    for (size_t i = 0; i < count; i++) board_says(bytes[i]);
}

/* What is typed comes in on UART0. */
static int zeno_board_typed(void) {
    return board_uart_next(board_uarts[0]);
}

static void zeno_board_stop(int status) {
    board_stops(status);
}
