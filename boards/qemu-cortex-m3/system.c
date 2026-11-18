/* What the C library asks of the system under it, on a board that has
 * none.
 *
 * The C library that comes with `arm-none-eabi-gcc` (newlib) does
 * `printf`, `malloc` and `exit` itself, and leaves the last step of
 * each to a few functions with fixed names: write these bytes, give me
 * more memory, stop. On a computer the operating system has them. Here
 * they are written for the board QEMU calls `lm3s6965evb`:
 *
 *   - what is printed goes out of the first serial port (UART0), and
 *     what is read comes in from it;
 *   - the message of an error goes to the debugger, by semihosting;
 *   - a program that ends tells the debugger so, which makes QEMU end.
 *
 * The runtime's simulated board (docs/board.md) sleeps and reads the
 * clock the POSIX way, so the two functions it calls for that are here
 * too, on the clock startup.c keeps. */

#include <errno.h>
#include <stdint.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>

/* startup.c */
void board_time(uint32_t *whole, uint32_t *millionths);
void board_doze(void);

/* UART0 of the LM3S6965: its data register, and the flag that says
 * nothing has come in. */
#define UART0_DATA (*(volatile uint32_t *)0x4000C000u)
#define UART0_FLAGS (*(volatile uint32_t *)0x4000C018u)
#define UART0_NOTHING_IN 0x10u

/* Semihosting: a breakpoint with the number 0xAB is a call to the
 * debugger that holds the board, or to QEMU, which answers as one. */
static int debugger(int what, void *with) {
    register int r0 __asm__("r0") = what;
    register void *r1 __asm__("r1") = with;
    __asm__ volatile("bkpt 0xAB" : "+r"(r0) : "r"(r1) : "memory");
    return r0;
}

#define DEBUGGER_WRITE_CHARACTER 0x03
#define DEBUGGER_EXIT 0x20
#define THE_PROGRAM_ENDED 0x20026

int _write(int file, const char *bytes, int count) {
    for (int i = 0; i < count; i++) {
        if (file == 2) {
            char one = bytes[i];
            debugger(DEBUGGER_WRITE_CHARACTER, &one);
        } else {
            UART0_DATA = (uint32_t)(unsigned char)bytes[i];
        }
    }
    return count;
}

/* One byte at a time, after waiting for it: a serial port has no end. */
int _read(int file, char *bytes, int count) {
    (void)file;
    if (count <= 0) return 0;
    while (UART0_FLAGS & UART0_NOTHING_IN) board_doze();
    bytes[0] = (char)UART0_DATA;
    return 1;
}

void _exit(int status) {
    int said[2] = {THE_PROGRAM_ENDED, status};
    debugger(DEBUGGER_EXIT, said);
    for (;;) {
    }
}

/* The memory `malloc` hands out: from the end of the variables up to
 * the stack, less what the stack is given. */
#define STACK_ROOM 8192
extern char _heap_start, _stack_top;

void *_sbrk(ptrdiff_t more) {
    static char *end = &_heap_start;
    if (end + more > &_stack_top - STACK_ROOM) {
        errno = ENOMEM;
        return (void *)-1;
    }
    char *was = end;
    end += more;
    return was;
}

/* There are no files on the board: the three streams are the serial port. */
int _open(const char *name, int flags, int mode) {
    (void)name;
    (void)flags;
    (void)mode;
    errno = ENOENT;
    return -1;
}

int _close(int file) {
    (void)file;
    return 0;
}

int _fstat(int file, struct stat *about) {
    (void)file;
    about->st_mode = S_IFCHR;
    return 0;
}

int _isatty(int file) {
    (void)file;
    return 1;
}

off_t _lseek(int file, off_t to, int from) {
    (void)file;
    (void)to;
    (void)from;
    return 0;
}

int _kill(int process, int signal) {
    (void)process;
    (void)signal;
    errno = EINVAL;
    return -1;
}

int _getpid(void) {
    return 1;
}

/* The clock and the sleep of the simulated board. */
int clock_gettime(clockid_t clock, struct timespec *now) {
    (void)clock;
    uint32_t whole, millionths;
    board_time(&whole, &millionths);
    now->tv_sec = (time_t)(whole / 1000u);
    now->tv_nsec = (long)(whole % 1000u) * 1000000L + (long)millionths;
    return 0;
}

int nanosleep(const struct timespec *wait, struct timespec *left) {
    (void)left;
    uint32_t start, start_millionths, now, now_millionths;
    board_time(&start, &start_millionths);
    uint64_t until = (uint64_t)start * 1000000u + start_millionths + (uint64_t)wait->tv_sec * 1000000000u + (uint64_t)wait->tv_nsec;
    for (;;) {
        board_time(&now, &now_millionths);
        if ((uint64_t)now * 1000000u + now_millionths >= until) return 0;
        board_doze();
    }
}
