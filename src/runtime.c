/* ---------------------------------------------------------------- Runtime
 *
 * The small library every compiled Zeno program carries with it. It is
 * pasted in front of the program, so the result is one C file with nothing
 * else to link except the C library.
 *
 * Everything here exists to make C behave the way Zeno promises: whole
 * numbers that report overflow, division that reports a zero, floats that
 * print the way `zeno run` prints them.
 */

#include <inttypes.h>
#include <math.h>
#ifndef ZENO_FIRMWARE
#include <setjmp.h>
#endif /* ZENO_FIRMWARE */
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The Zeno source file, for error messages. Set at the top of main. */
static const char *zeno_file = "";

/* ------------------------------------------------------------------- Memory
 *
 * Text built while the program runs, lists, objects and function values
 * all need memory, and a program that runs for a year must give it back.
 * Zeno does that with a garbage collector of the simplest kind, called
 * mark and sweep:
 *
 *   1. Every piece of memory handed out (a "block") is written in a table.
 *   2. Now and then the collector stops the program and MARKS every block
 *      the program can still reach. It starts from the "roots", which are
 *      the variables: the C stack and the top-level variables. Whatever a
 *      root points at is reachable. So is whatever THAT points at, and so
 *      on.
 *   3. Then it SWEEPS: every block left unmarked can never be used again,
 *      and is freed.
 *
 * The compiler does not tell the collector where the pointers are. The
 * collector looks at every word of the stack and of each block and asks
 * "is this the address of one of my blocks?". If it is, the block is kept.
 * This is called a conservative collector. A number that happens to look
 * like an address keeps a block alive for no reason, which wastes a little
 * memory and breaks nothing.
 */

/* What sits in front of every block. The first field does two jobs, at
 * different times. Between collections it is the block's `place` in the
 * table, so that zeno_free can take the block out without searching. During
 * a collection it is `next_waiting`. A collection sorts the table anyway,
 * and writes every place afresh as it finishes. */
typedef struct zeno_block {
    union {
        struct zeno_block *next_waiting;    /* see zeno_waiting below */
        size_t place;                       /* zeno_blocks[place] is this block */
    } link;
    size_t size : sizeof(size_t) * 8 - 2;   /* bytes handed to the program */
    size_t marked : 1;                      /* reached, during a collection */
    size_t plain : 1;                       /* holds text or one number: no pointers inside */
} zeno_block;

/* The table of every block there is. */
static zeno_block **zeno_blocks = NULL;
static size_t zeno_block_count = 0;
static size_t zeno_block_room = 0;

/* How much memory the program holds, and how much it may hold before the
 * collector runs next. After each run the limit is set to twice what
 * survived, so a program that needs a lot is not collected all the time. */
#ifndef ZENO_FIRST_COLLECTION
#define ZENO_FIRST_COLLECTION (1024 * 1024)
#endif
static size_t zeno_in_use = 0;
static size_t zeno_collect_at = ZENO_FIRST_COLLECTION;

/* Numbers for the report that `zeno build --memory` asks for. */
static size_t zeno_most_in_use = 0;
static size_t zeno_collections = 0;
static size_t zeno_blocks_made = 0;
static size_t zeno_bytes_made = 0;
static size_t zeno_blocks_freed_at_once = 0;

/* Where the stack starts. `main` sets it before anything else runs. */
static void *zeno_stack_base = NULL;

/* The roots that are not on the stack: the top-level variables. The
 * compiler writes this function, at the end of the file, since only it
 * knows what they are. */
static void zeno_roots(void);

static void zeno_out_of_memory(void) {
    fflush(stdout);
    fprintf(stderr, "error: out of memory\n");
    exit(1);
}

/* Blocks that are marked but whose insides have not been looked at yet,
 * chained through `next_waiting`. The chain ends at this marker instead
 * of NULL. Keeping the list inside the blocks means the collector needs
 * no memory of its own, at the one moment when there may be none. */
static zeno_block zeno_end_of_waiting;
static zeno_block *zeno_waiting = &zeno_end_of_waiting;

/* The lowest and highest addresses any block covers. Most words on the
 * stack are not addresses at all, and these two turn them away quickly. */
static uintptr_t zeno_lowest = 0;
static uintptr_t zeno_highest = 0;

/* `word` was found in a variable or inside a block. If it points at, or
 * into, a block, mark that block. */
static void zeno_mark(uintptr_t word) {
    if (word < zeno_lowest || word > zeno_highest) return;
    /* The table is sorted by address. Find the last block that starts at
     * or before `word`, by halving the range each time. */
    size_t low = 0, high = zeno_block_count;
    while (low < high) {
        size_t middle = low + (high - low) / 2;
        if ((uintptr_t)(zeno_blocks[middle] + 1) <= word) low = middle + 1;
        else high = middle;
    }
    if (low == 0) return;
    zeno_block *block = zeno_blocks[low - 1];
    /* `<=` and not `<`: C code is allowed to hold a pointer to just past
     * the end of an array, and a loop over a list may hold only that. */
    if (word > (uintptr_t)(block + 1) + block->size) return;
    if (block->marked) return;
    block->marked = 1;
    if (!block->plain) {
        block->link.next_waiting = zeno_waiting;
        zeno_waiting = block;
    }
}

/* Treat every word from `start` on as something that might be a pointer.
 * (The attribute is for AddressSanitizer, a tool that reports reads of
 * memory nobody declared. Reading the whole stack is exactly that, and
 * here it is on purpose.) */
#if defined(__GNUC__)
__attribute__((no_sanitize_address))
#endif
static void zeno_mark_words(const void *start, size_t size) {
    uintptr_t at = ((uintptr_t)start + sizeof(void *) - 1) / sizeof(void *) * sizeof(void *);
    uintptr_t end = (uintptr_t)start + size;
    for (; at + sizeof(void *) <= end; at += sizeof(void *)) {
        zeno_mark(*(const uintptr_t *)at);
    }
}

/* Mark from the C stack: every local variable of every function that is
 * running now lies between this function's own variable and `main`'s. */
#if defined(__GNUC__)
__attribute__((noinline))
#endif
static void zeno_mark_stack(void) {
    void *here = NULL;
    uintptr_t low = (uintptr_t)&here, high = (uintptr_t)zeno_stack_base;
    if (low > high) { /* a machine whose stack grows upward */
        uintptr_t swap = low;
        low = high;
        high = swap;
    }
    zeno_mark_words((const void *)low, high - low + sizeof(void *));
}

static int zeno_by_address(const void *a, const void *b) {
    uintptr_t x = (uintptr_t)*(zeno_block *const *)a, y = (uintptr_t)*(zeno_block *const *)b;
    return x < y ? -1 : x > y;
}

static void zeno_collect(void) {
    if (zeno_block_count == 0 || zeno_stack_base == NULL) return;
    zeno_collections++;
    qsort(zeno_blocks, zeno_block_count, sizeof(zeno_block *), zeno_by_address);
    zeno_block *last = zeno_blocks[zeno_block_count - 1];
    zeno_lowest = (uintptr_t)(zeno_blocks[0] + 1);
    zeno_highest = (uintptr_t)(last + 1) + last->size;

    /* Mark. A pointer may be nowhere in memory at this moment, only in one
     * of the processor's registers. This copies the registers onto the
     * stack, where zeno_mark_stack will see them. (A builtin of GCC and
     * Clang, like the ones the checked arithmetic uses further down.) */
    __builtin_unwind_init();
    zeno_mark_stack();
    zeno_roots();
    /* Then follow the pointers inside each marked block, which marks more
     * blocks, until none are waiting. */
    while (zeno_waiting != &zeno_end_of_waiting) {
        zeno_block *block = zeno_waiting;
        zeno_waiting = block->link.next_waiting;
        zeno_mark_words(block + 1, block->size);
    }

    /* Sweep: free what was not reached, and close the gaps in the table. */
    size_t kept = 0;
    for (size_t i = 0; i < zeno_block_count; i++) {
        zeno_block *block = zeno_blocks[i];
        if (block->marked) {
            block->marked = 0;
            block->link.place = kept;
            zeno_blocks[kept++] = block;
        } else {
            zeno_in_use -= block->size;
            free(block);
        }
    }
    zeno_block_count = kept;
    zeno_collect_at = zeno_in_use * 2 > ZENO_FIRST_COLLECTION ? zeno_in_use * 2 : ZENO_FIRST_COLLECTION;
}

static char *zeno_new_block(size_t size, bool plain) {
#ifdef ZENO_COLLECT_ALWAYS
    zeno_collect(); /* for testing the collector: as hard on it as can be */
#else
    if (zeno_in_use + size > zeno_collect_at) zeno_collect();
#endif
    if (zeno_block_count == zeno_block_room) {
        size_t room = zeno_block_room == 0 ? 256 : zeno_block_room * 2;
        zeno_block **table = realloc(zeno_blocks, room * sizeof(zeno_block *));
        if (table == NULL) zeno_out_of_memory();
        zeno_blocks = table;
        zeno_block_room = room;
    }
    /* A block that can hold pointers starts as all zeros, so that what an
     * earlier owner left behind is not taken for a pointer. */
    size_t whole = sizeof(zeno_block) + size;
    zeno_block *block = plain ? malloc(whole) : calloc(1, whole);
    if (block == NULL) {
        /* The machine has no more, whatever the limit says. Collect, and
         * ask once more. */
        zeno_collect();
        block = plain ? malloc(whole) : calloc(1, whole);
        if (block == NULL) zeno_out_of_memory();
    }
    block->link.place = zeno_block_count;
    block->size = size;
    block->marked = 0;
    block->plain = plain;
    zeno_blocks[zeno_block_count++] = block;
    zeno_blocks_made++;
    zeno_bytes_made += size;
    zeno_in_use += size;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
    return (char *)(block + 1);
}

/* Memory for a list, an object or a function value: things with pointers
 * to other things inside. */
static char *zeno_alloc(size_t size) {
    return zeno_new_block(size, false);
}

/* Memory for text, or for one number. The collector need not look inside. */
static char *zeno_alloc_text(size_t size) {
    return zeno_new_block(size, true);
}

/* Give one block back at once, without waiting for the collector. This is
 * what the end of an owner's block does to an owned value (see
 * docs/ownership.md): the checker has made sure nothing else can still use
 * it. `memory` is what zeno_alloc returned, or NULL, which does nothing.
 *
 * The block leaves the table by letting the last entry take its place, so
 * this costs the same however many blocks there are. If a block is never
 * freed this way (an error jumped past the end of its owner's block, say),
 * nothing is lost: it is still in the table, and the collector frees it
 * when it finds nothing pointing at it, like any other block. */
static void zeno_free(void *memory) {
    if (memory == NULL) return;
    zeno_block *block = (zeno_block *)memory - 1;
    zeno_block *last = zeno_blocks[--zeno_block_count];
    last->link.place = block->link.place;
    zeno_blocks[block->link.place] = last;
    zeno_in_use -= block->size;
    zeno_blocks_freed_at_once++;
    free(block);
}

/* What a program built with `zeno build --memory` prints as it ends. */
#ifdef ZENO_MEMORY_REPORT
static void zeno_memory_report(void) {
    fflush(stdout);
    fprintf(stderr, "memory: %zu blocks made (%zu bytes), at most %zu bytes in use at once, %zu collections",
            zeno_blocks_made, zeno_bytes_made, zeno_most_in_use, zeno_collections);
    if (zeno_blocks_freed_at_once > 0) {
        fprintf(stderr, ", %zu blocks freed without the collector", zeno_blocks_freed_at_once);
    }
    fprintf(stderr, "\n");
}
#endif

/* ------------------------------------------------------------------- Errors
 *
 * An error value is what `ValueError("bad")` makes, what `raise` sends and
 * what `except ValueError as err` receives.
 */
typedef struct {
    const char *kind;    /* "ValueError", or the name of a class */
    const char *message; /* what `print(err)` shows */
    void *object;        /* the object itself, when an object was raised */
} zeno_error;

static zeno_error *zeno_error_new(const char *kind, const char *message, void *object) {
    zeno_error *error = (zeno_error *)zeno_alloc(sizeof(zeno_error));
    error->kind = kind;
    error->message = message;
    error->object = object;
    return error;
}

static bool zeno_error_same(zeno_error *a, zeno_error *b) {
    return strcmp(a->kind, b->kind) == 0 && strcmp(a->message, b->message) == 0;
}

/* The error on its way to an `except`, and where it was raised. */
static zeno_error *zeno_raised = NULL;
static int zeno_raised_line = 0;
static int zeno_raised_col = 0;

/* Nobody caught it: print what `zeno run` prints, and stop. */
__attribute__((noreturn)) static void zeno_uncaught(void) {
    fflush(stdout);
    fprintf(stderr, "error: uncaught %s: %s\n --> %s:%d:%d\n", zeno_raised->kind, zeno_raised->message, zeno_file,
            zeno_raised_line, zeno_raised_col);
    exit(1);
}

#ifndef ZENO_FIRMWARE

/* How an error travels in an ordinary build: it jumps.
 *
 * One of these exists for each `try` that is running: the place to come
 * back to if something inside it raises. They form a chain, innermost
 * first, so a `try` inside a `try` is found before the one around it.
 *
 * `setjmp` remembers a place in the program and `longjmp` goes back to it,
 * out of however many function calls lie in between. A `try` that raises
 * nothing costs one `setjmp`, and code outside any `try` costs nothing. */
typedef struct zeno_try {
    jmp_buf place;
    struct zeno_try *outer;
} zeno_try;

static zeno_try *zeno_trying = NULL;

/* `raise`. Inside a `try`, go to its `except` blocks. Outside every `try`,
 * nobody can catch it. */
__attribute__((noreturn)) static void zeno_raise(zeno_error *error, int line, int col) {
    zeno_raised = error;
    zeno_raised_line = line;
    zeno_raised_col = col;
    if (zeno_trying == NULL) zeno_uncaught();
    zeno_try *handler = zeno_trying;
    zeno_trying = handler->outer; /* that `try` is over, whatever happens next */
    longjmp(handler->place, 1);
}

/* No `except` of this `try` wanted the error: pass it outward unchanged. */
__attribute__((noreturn)) static void zeno_raise_again(void) {
    zeno_raise(zeno_raised, zeno_raised_line, zeno_raised_col);
}

/* Does `except Kind` catch the error on its way? */
static bool zeno_caught(const char *kind) {
    return strcmp(zeno_raised->kind, kind) == 0;
}

/* An error the runtime itself finds, such as a division by zero. */
__attribute__((noreturn)) static void zeno_fail(const char *kind, const char *message, int line, int col) {
    zeno_raise(zeno_error_new(kind, message, NULL), line, col);
}

#else /* ZENO_FIRMWARE */

/* How an error travels in a firmware build (`zeno build --firmware`): as
 * a value. Nothing jumps out of a function, which is what coding
 * standards for firmware ask for.
 *
 * A function that can raise returns `true` when all went well and `false`
 * when it did not, and gives its answer through a pointer. `raise` notes
 * the error here and returns `false`. Each caller tests what came back,
 * and either goes to its own `except` or returns `false` in its turn. */
static void zeno_note(zeno_error *error, int line, int col) {
    zeno_raised = error;
    zeno_raised_line = line;
    zeno_raised_col = col;
}

/* Does `except Kind` catch this error? */
static bool zeno_is(const zeno_error *error, const char *kind) {
    return strcmp(error->kind, kind) == 0;
}

/* An error the runtime itself finds, such as a division by zero or an
 * index past the end. In a firmware build these are faults: they stop the
 * program, and no `except` sees them. (Making them catchable would put a
 * test after nearly every line of every function. On a real board this is
 * where the fault handler would be called.) */
__attribute__((noreturn)) static void zeno_fail(const char *kind, const char *message, int line, int col) {
    zeno_note(zeno_error_new(kind, message, NULL), line, col);
    zeno_uncaught();
}

#endif /* ZENO_FIRMWARE */

/* A mistake in the program itself, which `try` does not catch (see
 * decisions.md 4.1). The checker finds nearly all of these before the
 * program is built, so there is very little left to report here. */
__attribute__((noreturn)) static void zeno_bug(const char *message, int line, int col) {
    fflush(stdout);
    fprintf(stderr, "error: %s\n --> %s:%d:%d\n", message, zeno_file, line, col);
    exit(1);
}

static void zeno_overflow(int line, int col) {
    zeno_fail("OverflowError", "the result is too big for an int", line, col);
}

static void zeno_by_zero(int line, int col) {
    zeno_fail("ZeroDivisionError", "division by zero", line, col);
}

/* ------------------------------------------------------------ Whole numbers
 *
 * In C, a signed overflow is undefined: the compiler may do anything. Zeno
 * says it is an OverflowError, so every operation is checked. The
 * __builtin_ functions are how GCC and Clang expose the processor's own
 * overflow flag.
 */

static int64_t zeno_add(int64_t a, int64_t b, int line, int col) {
    int64_t result;
    if (__builtin_add_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
}

static int64_t zeno_sub(int64_t a, int64_t b, int line, int col) {
    int64_t result;
    if (__builtin_sub_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
}

static int64_t zeno_mul(int64_t a, int64_t b, int line, int col) {
    int64_t result;
    if (__builtin_mul_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
}

static int64_t zeno_neg(int64_t a, int line, int col) {
    return zeno_sub(0, a, line, col);
}

static int64_t zeno_abs(int64_t a, int line, int col) {
    return a < 0 ? zeno_neg(a, line, col) : a;
}

/* `//` rounds down. C rounds toward zero, so step down once more when the
 * signs differ and the division was not exact. */
static int64_t zeno_floordiv(int64_t a, int64_t b, int line, int col) {
    if (b == 0) zeno_by_zero(line, col);
    if (a == INT64_MIN && b == -1) zeno_overflow(line, col);
    int64_t q = a / b;
    if (a % b != 0 && (a < 0) != (b < 0)) q -= 1;
    return q;
}

/* `%` takes the sign of the right side, to pair with `//`. */
static int64_t zeno_mod(int64_t a, int64_t b, int line, int col) {
    if (b == 0) zeno_by_zero(line, col);
    if (b == -1) return 0; /* also avoids INT64_MIN % -1, which C leaves undefined */
    int64_t r = a % b;
    if (r != 0 && (r < 0) != (b < 0)) r += b;
    return r;
}

static int64_t zeno_pow(int64_t base, int64_t power, int line, int col) {
    if (power < 0) {
        zeno_fail("ValueError",
                  "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`",
                  line, col);
    }
    /* These never overflow, however big the power, so answer them directly
     * instead of looping. */
    if (base == 1 || power == 0) return 1;
    if (base == 0) return 0;
    if (base == -1) return power % 2 == 0 ? 1 : -1;
    int64_t result = 1;
    for (int64_t i = 0; i < power; i++) result = zeno_mul(result, base, line, col);
    return result;
}

/* How many numbers `range(start, stop, step)` holds. */
static int64_t zeno_range_count(int64_t start, int64_t stop, int64_t step, int line, int col) {
    if (step == 0) zeno_fail("ValueError", "the step of a range cannot be 0", line, col);
    if (step > 0 && start < stop) {
        return (int64_t)(((uint64_t)stop - (uint64_t)start - 1) / (uint64_t)step) + 1;
    }
    if (step < 0 && start > stop) {
        return (int64_t)(((uint64_t)start - (uint64_t)stop - 1) / (0 - (uint64_t)step)) + 1;
    }
    return 0;
}

/* ------------------------------------------------------------------- Floats */

static double zeno_div(double a, double b, int line, int col) {
    if (b == 0.0) zeno_by_zero(line, col);
    return a / b;
}

static double zeno_floordiv_float(double a, double b, int line, int col) {
    return floor(zeno_div(a, b, line, col));
}

static double zeno_mod_float(double a, double b, int line, int col) {
    if (b == 0.0) zeno_by_zero(line, col);
    return a - b * floor(a / b);
}

/* A float to a whole number, dropping the fraction. Too big a float gives
 * the biggest int, as it does in `zeno run`. */
static int64_t zeno_whole(double x) {
    if (x != x) return 0;
    if (x >= 9223372036854775807.0) return INT64_MAX;
    if (x <= -9223372036854775808.0) return INT64_MIN;
    return (int64_t)x;
}

/* `min` and `max` keep the earlier value when two are equal. */
static int64_t zeno_min(int64_t a, int64_t b) { return b < a ? b : a; }
static int64_t zeno_max(int64_t a, int64_t b) { return b > a ? b : a; }
static double zeno_min_float(double a, double b) { return b < a ? b : a; }
static double zeno_max_float(double a, double b) { return b > a ? b : a; }

/* ------------------------------------------------------------------ Strings
 *
 * A Zeno str is a C string: bytes of UTF-8 ending in a zero byte.
 */

static const char *zeno_copy(const char *text, size_t length) {
    char *copy = zeno_alloc_text(length + 1);
    memcpy(copy, text, length);
    copy[length] = '\0';
    return copy;
}

/* Join `count` strings into a new one. This is what an f-string becomes. */
static const char *zeno_join(int count, ...) {
    va_list parts;
    size_t length = 0;
    va_start(parts, count);
    for (int i = 0; i < count; i++) length += strlen(va_arg(parts, const char *));
    va_end(parts);

    char *joined = zeno_alloc_text(length + 1);
    char *end = joined;
    va_start(parts, count);
    for (int i = 0; i < count; i++) {
        const char *part = va_arg(parts, const char *);
        size_t n = strlen(part);
        memcpy(end, part, n);
        end += n;
    }
    va_end(parts);
    *end = '\0';
    return joined;
}

/* `len` counts characters, not bytes. In UTF-8 every character has exactly
 * one byte that does not look like 10xxxxxx, so count those. */
static int64_t zeno_len(const char *text) {
    int64_t count = 0;
    for (; *text != '\0'; text++) {
        if (((unsigned char)*text & 0xC0) != 0x80) count++;
    }
    return count;
}

static const char *zeno_str_int(int64_t n) {
    char buffer[32];
    int length = snprintf(buffer, sizeof buffer, "%" PRId64, n);
    return zeno_copy(buffer, (size_t)length);
}

static const char *zeno_str_bool(bool b) {
    return b ? "true" : "false";
}

/* A float as text: the fewest digits that read back as the same float, and
 * always with a `.0` or an exponent so it cannot be mistaken for an int.
 * 0.1 + 0.2 prints as 0.30000000000000004, and 3.0 as 3.0. */
static const char *zeno_str_float(double x) {
    if (x != x) return "NaN";
    if (x == INFINITY) return "inf";
    if (x == -INFINITY) return "-inf";

    /* Find the shortest run of digits that still names this exact float. */
    char digits[40];
    int precision = 1;
    for (; precision < 17; precision++) {
        snprintf(digits, sizeof digits, "%.*e", precision - 1, x);
        if (strtod(digits, NULL) == x) break;
    }
    snprintf(digits, sizeof digits, "%.*e", precision - 1, x);

    /* `digits` now looks like -d.ddddde+XX. Pull it apart. */
    char mantissa[24];
    int count = 0;
    const char *p = digits;
    bool negative = *p == '-';
    if (negative) p++;
    for (; *p != 'e'; p++) {
        if (*p != '.') mantissa[count++] = *p;
    }
    int exponent = atoi(p + 1);
    while (count > 1 && mantissa[count - 1] == '0') count--; /* 1.50 is 1.5 */

    char out[64];
    char *o = out;
    if (negative) *o++ = '-';
    double size = fabs(x);
    if (size != 0.0 && (size < 1e-4 || size >= 1e16)) {
        /* Very small or very big: 1.5e-7 */
        *o++ = mantissa[0];
        if (count > 1) {
            *o++ = '.';
            for (int i = 1; i < count; i++) *o++ = mantissa[i];
        }
        o += sprintf(o, "e%d", exponent);
    } else if (exponent < 0) {
        /* 0.00123 */
        *o++ = '0';
        *o++ = '.';
        for (int i = 0; i < -exponent - 1; i++) *o++ = '0';
        for (int i = 0; i < count; i++) *o++ = mantissa[i];
    } else {
        /* 123.45, or 1200.0 */
        for (int i = 0; i <= exponent; i++) *o++ = i < count ? mantissa[i] : '0';
        *o++ = '.';
        if (count <= exponent + 1) *o++ = '0';
        for (int i = exponent + 1; i < count; i++) *o++ = mantissa[i];
    }
    return zeno_copy(out, (size_t)(o - out));
}

/* `500 ms`. A whole quantity prints without a decimal point. */
static const char *zeno_str_quantity(double x, const char *unit) {
    if (x == floor(x) && fabs(x) < 1e15) {
        return zeno_join(3, zeno_str_int((int64_t)x), " ", unit);
    }
    return zeno_join(3, zeno_str_float(x), " ", unit);
}

/* ---------------------------------------------------- f-string formats
 *
 * `{x:.2f}`, `{n:5}`, `{reg:08b}`. The format is read when the program is
 * compiled; these functions do what it asked for.
 */

/* `{x:.2f}`: a fixed number of decimals. */
static const char *zeno_fixed(double x, int decimals) {
    int length = snprintf(NULL, 0, "%.*f", decimals, x);
    char *text = zeno_alloc_text((size_t)length + 1);
    snprintf(text, (size_t)length + 1, "%.*f", decimals, x);
    return text;
}

/* `{n:x}`, `{n:X}`, `{n:b}`: hexadecimal or binary. */
static const char *zeno_radix(int64_t n, char kind) {
    char buffer[72];
    char *end = buffer + sizeof buffer;
    char *p = end;
    uint64_t size = n < 0 ? 0 - (uint64_t)n : (uint64_t)n;
    unsigned base = kind == 'b' ? 2 : 16;
    const char *symbols = kind == 'X' ? "0123456789ABCDEF" : "0123456789abcdef";
    do {
        *--p = symbols[size % base];
        size /= base;
    } while (size != 0);
    if (n < 0) *--p = '-';
    return zeno_copy(p, (size_t)(end - p));
}

/* Pad `body` out to `width` characters. `align` is '<', '>' or '^'.
 * With `zeros`, the padding is zeros between the sign and the digits. */
static const char *zeno_pad(const char *body, int width, char align, bool zeros) {
    int padding = width - (int)zeno_len(body);
    if (padding <= 0) return body;

    size_t length = strlen(body);
    char *out = zeno_alloc_text(length + (size_t)padding + 1);
    char *o = out;
    int left = align == '>' ? padding : align == '^' ? padding / 2 : 0;
    if (zeros) {
        if (*body == '-') {
            *o++ = *body++;
            length--;
        }
        left = padding;
    }
    for (int i = 0; i < left; i++) *o++ = zeros ? '0' : ' ';
    memcpy(o, body, length);
    o += length;
    for (int i = left; i < padding; i++) *o++ = ' ';
    *o = '\0';
    return out;
}

/* ----------------------------------------------------------------- Built-ins */

/* `print(a, b, c)`: the values, already turned into text, with a space
 * between them and a line break after. */
static void zeno_print(int count, ...) {
    va_list values;
    va_start(values, count);
    for (int i = 0; i < count; i++) {
        if (i > 0) fputc(' ', stdout);
        fputs(va_arg(values, const char *), stdout);
    }
    va_end(values);
    fputc('\n', stdout);
}

static double zeno_sqrt(double x, int line, int col) {
    if (x < 0.0) {
        zeno_fail("ValueError", zeno_join(2, "cannot take the square root of ", zeno_str_float(x)), line, col);
    }
    return sqrt(x);
}

/* `int(x)` on a float drops the fraction. Infinity has no int to become. */
static int64_t zeno_int(double x, int line, int col) {
    if (x != x || x == INFINITY || x == -INFINITY) {
        zeno_fail("ValueError", zeno_join(3, "cannot turn ", zeno_str_float(x), " into an int"), line, col);
    }
    return zeno_whole(x);
}

/* A str the way it is shown inside a list or an object: in quotes. */
static const char *zeno_repr_str(const char *text) {
    size_t length = 2;
    for (const char *p = text; *p != '\0'; p++) {
        length += (*p == '"' || *p == '\\' || *p == '\n' || *p == '\t' || *p == '\r') ? 2 : 1;
    }
    char *out = zeno_alloc_text(length + 1);
    char *o = out;
    *o++ = '"';
    for (const char *p = text; *p != '\0'; p++) {
        switch (*p) {
        case '"': *o++ = '\\'; *o++ = '"'; break;
        case '\\': *o++ = '\\'; *o++ = '\\'; break;
        case '\n': *o++ = '\\'; *o++ = 'n'; break;
        case '\t': *o++ = '\\'; *o++ = 't'; break;
        case '\r': *o++ = '\\'; *o++ = 'r'; break;
        default: *o++ = *p;
        }
    }
    *o++ = '"';
    *o = '\0';
    return out;
}

/* -------------------------------------------------------------------- Lists
 *
 * One list type serves every kind of item. Each item sits in a slot big
 * enough for any Zeno value: a number, a bool, or a pointer to a str, an
 * object, an enum value or another list. The compiler knows what kind of
 * item each list holds, and reads the matching member of the slot.
 *
 * As with strings, the memory is taken and not given back yet.
 */

typedef union {
    int64_t i;
    double f;
    bool b;
    const char *s;
    void *p;
} zeno_slot;

typedef struct {
    int64_t len;
    int64_t room; /* how many items fit before `items` must grow */
    zeno_slot *items;
} zeno_list;

static zeno_list *zeno_list_new(int64_t room) {
    zeno_list *list = (zeno_list *)zeno_alloc(sizeof(zeno_list));
    list->len = 0;
    list->room = room;
    list->items = room > 0 ? (zeno_slot *)zeno_alloc((size_t)room * sizeof(zeno_slot)) : NULL;
    return list;
}

static void zeno_list_append(zeno_list *list, zeno_slot item) {
    if (list->len == list->room) {
        /* Full: move to a place twice the size. Only the list itself ever
         * points at its `items`, so the old place can go back at once. */
        int64_t room = list->room == 0 ? 4 : list->room * 2;
        zeno_slot *items = (zeno_slot *)zeno_alloc((size_t)room * sizeof(zeno_slot));
        if (list->len > 0) memcpy(items, list->items, (size_t)list->len * sizeof(zeno_slot));
        zeno_free(list->items);
        list->items = items;
        list->room = room;
    }
    list->items[list->len++] = item;
}

/* The end of an owned list (`owned list[int]`): its owner's block is over,
 * or its owner was given a new one. What the items point at is shared, and
 * stays the collector's business. NULL means the owner had already given
 * the list away, and there is nothing to do. */
static void zeno_list_drop(zeno_list *list) {
    if (list == NULL) return;
    zeno_free(list->items);
    zeno_free(list);
}

/* `[a, b, c]`: a list holding `count` items. */
static zeno_list *zeno_list_of(int count, ...) {
    zeno_list *list = zeno_list_new(count);
    va_list items;
    va_start(items, count);
    for (int i = 0; i < count; i++) zeno_list_append(list, va_arg(items, zeno_slot));
    va_end(items);
    return list;
}

static zeno_list *zeno_list_copy(zeno_list *list) {
    zeno_list *copy = zeno_list_new(list->len);
    if (list->len > 0) memcpy(copy->items, list->items, (size_t)list->len * sizeof(zeno_slot));
    copy->len = list->len;
    return copy;
}

static zeno_list *zeno_list_concat(zeno_list *a, zeno_list *b) {
    zeno_list *joined = zeno_list_new(a->len + b->len);
    for (int64_t i = 0; i < a->len; i++) zeno_list_append(joined, a->items[i]);
    for (int64_t i = 0; i < b->len; i++) zeno_list_append(joined, b->items[i]);
    return joined;
}

/* Every use of `list[i]` goes through here, so a position that is not in
 * the list is an IndexError and never a read of someone else's memory. */
static zeno_slot *zeno_list_at(zeno_list *list, int64_t i, int line, int col) {
    if (i < 0 || i >= list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "index ", zeno_str_int(i), " is out of range for a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
    return &list->items[i];
}

static zeno_slot zeno_list_pop(zeno_list *list, int line, int col) {
    if (list->len == 0) zeno_fail("IndexError", "cannot pop from an empty list", line, col);
    zeno_slot item = list->items[--list->len];
    /* Wipe the place it was in, or the collector would go on thinking the
     * list holds it. */
    memset(&list->items[list->len], 0, sizeof(zeno_slot));
    return item;
}

/* The three below need to know how to show or compare one item, which
 * depends on its kind. The compiler passes a small function that does. */

static const char *zeno_list_str(zeno_list *list, const char *(*show)(zeno_slot)) {
    const char *text = "[";
    for (int64_t i = 0; i < list->len; i++) {
        text = zeno_join(3, text, i > 0 ? ", " : "", show(list->items[i]));
    }
    return zeno_join(2, text, "]");
}

static bool zeno_list_eq(zeno_list *a, zeno_list *b, bool (*same)(zeno_slot, zeno_slot)) {
    if (a->len != b->len) return false;
    for (int64_t i = 0; i < a->len; i++) {
        if (!same(a->items[i], b->items[i])) return false;
    }
    return true;
}

static bool zeno_list_contains(zeno_list *list, zeno_slot item, bool (*same)(zeno_slot, zeno_slot)) {
    for (int64_t i = 0; i < list->len; i++) {
        if (same(list->items[i], item)) return true;
    }
    return false;
}

/* A str is a row of characters, and in UTF-8 a character can take from one
 * to four bytes. Every byte after the first looks like 10xxxxxx. */
static size_t zeno_char_size(const char *p) {
    size_t size = 1;
    while (((unsigned char)p[size] & 0xC0) == 0x80) size++;
    return size;
}

/* `text[i]`: the character at position i, as a str of its own. */
static const char *zeno_char_at(const char *text, int64_t i, int line, int col) {
    int64_t position = 0;
    for (const char *p = text; *p != '\0' && i >= 0; p += zeno_char_size(p), position++) {
        if (position == i) return zeno_copy(p, zeno_char_size(p));
    }
    zeno_fail("IndexError",
              zeno_join(4, "index ", zeno_str_int(i), " is out of range for a str of length ",
                        zeno_str_int(zeno_len(text))),
              line, col);
}

/* `for c in text`: the characters of `text`, each as a str. */
static zeno_list *zeno_chars(const char *text) {
    zeno_list *chars = zeno_list_new(0);
    for (const char *p = text; *p != '\0'; p += zeno_char_size(p)) {
        zeno_list_append(chars, (zeno_slot){.s = zeno_copy(p, zeno_char_size(p))});
    }
    return chars;
}

/* `sum`, `min` and `max` of a list of numbers. */

static int64_t zeno_list_sum_int(zeno_list *list, int line, int col) {
    int64_t total = 0;
    for (int64_t i = 0; i < list->len; i++) total = zeno_add(total, list->items[i].i, line, col);
    return total;
}

static double zeno_list_sum_float(zeno_list *list) {
    double total = 0.0;
    for (int64_t i = 0; i < list->len; i++) total += list->items[i].f;
    return total;
}

static void zeno_list_not_empty(zeno_list *list, const char *name, int line, int col) {
    if (list->len == 0) zeno_fail("ValueError", zeno_join(3, "`", name, "` of nothing"), line, col);
}

static int64_t zeno_list_best_int(zeno_list *list, bool smallest, const char *name, int line, int col) {
    zeno_list_not_empty(list, name, line, col);
    int64_t best = list->items[0].i;
    for (int64_t i = 1; i < list->len; i++) {
        int64_t item = list->items[i].i;
        if (smallest ? item < best : item > best) best = item;
    }
    return best;
}

static double zeno_list_best_float(zeno_list *list, bool smallest, const char *name, int line, int col) {
    zeno_list_not_empty(list, name, line, col);
    double best = list->items[0].f;
    for (int64_t i = 1; i < list->len; i++) {
        double item = list->items[i].f;
        if (smallest ? item < best : item > best) best = item;
    }
    return best;
}

/* ------------------------------------------------------- Text into numbers */

/* The blank characters that `strip`, `split`, `int` and `float` skip. */
static bool zeno_is_blank(char c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f';
}

/* `text` without the blanks at either end. */
static const char *zeno_strip(const char *text) {
    while (zeno_is_blank(*text)) text++;
    size_t length = strlen(text);
    while (length > 0 && zeno_is_blank(text[length - 1])) length--;
    return zeno_copy(text, length);
}

/* Read a whole number out of text. Blanks around the number and `_`
 * inside it are allowed, as in `1_000`. Gives `false` for anything else. */
static bool zeno_read_int(const char *text, int64_t *result) {
    const char *stripped = zeno_strip(text);
    /* Take every `_` out first, then read what is left. */
    char *digits = zeno_alloc_text(strlen(stripped) + 1);
    char *end = digits;
    for (const char *p = stripped; *p != '\0'; p++) {
        if (*p != '_') *end++ = *p;
    }
    *end = '\0';

    const char *p = digits;
    bool negative = *p == '-';
    if (*p == '-' || *p == '+') p++;
    bool good = *p != '\0';
    int64_t n = 0;
    for (; *p != '\0' && good; p++) {
        int digit = *p - '0';
        /* Build the number as a negative when it is one, because the
         * smallest int has no positive twin to pass through. */
        good = *p >= '0' && *p <= '9' && !__builtin_mul_overflow(n, 10, &n)
               && !(negative ? __builtin_sub_overflow(n, digit, &n) : __builtin_add_overflow(n, digit, &n));
    }
    *result = n;
    return good;
}

static bool zeno_read_float(const char *text, double *result) {
    const char *p = zeno_strip(text);
    char *end = NULL;
    /* C's own reader also takes hexadecimal and `nan(...)`; Zeno does not. */
    bool good = *p != '\0' && strpbrk(p, "xX(") == NULL;
    *result = good ? strtod(p, &end) : 0.0;
    return good && end != p && *end == '\0';
}

/* The ValueError for text that is not a number. `what` is "an int" or
 * "a float". */
static zeno_error *zeno_not_a_number(const char *text, const char *what) {
    return zeno_error_new("ValueError", zeno_join(4, "cannot turn ", zeno_repr_str(text), " into ", what), NULL);
}

/* `int("42")` and `float("2.5")`. Text from outside may not be a number
 * at all, so these two can raise, and how they do it depends on the kind
 * of build. */
#ifndef ZENO_FIRMWARE

static int64_t zeno_parse_int(const char *text, int line, int col) {
    int64_t n;
    if (!zeno_read_int(text, &n)) zeno_raise(zeno_not_a_number(text, "an int"), line, col);
    return n;
}

static double zeno_parse_float(const char *text, int line, int col) {
    double x;
    if (!zeno_read_float(text, &x)) zeno_raise(zeno_not_a_number(text, "a float"), line, col);
    return x;
}

#else /* ZENO_FIRMWARE */

static bool zeno_parse_int(const char *text, int line, int col, int64_t *result) {
    if (zeno_read_int(text, result)) return true;
    zeno_note(zeno_not_a_number(text, "an int"), line, col);
    return false;
}

static bool zeno_parse_float(const char *text, int line, int col, double *result) {
    if (zeno_read_float(text, result)) return true;
    zeno_note(zeno_not_a_number(text, "a float"), line, col);
    return false;
}

#endif /* ZENO_FIRMWARE */

/* ------------------------------------------------------- Methods of a str
 *
 * `upper` and `lower` change the letters a to z only. See decisions.md.
 */

static const char *zeno_upper(const char *text) {
    size_t length = strlen(text);
    char *out = zeno_alloc_text(length + 1);
    for (size_t i = 0; i <= length; i++) out[i] = (text[i] >= 'a' && text[i] <= 'z') ? text[i] - 32 : text[i];
    return out;
}

static const char *zeno_lower(const char *text) {
    size_t length = strlen(text);
    char *out = zeno_alloc_text(length + 1);
    for (size_t i = 0; i <= length; i++) out[i] = (text[i] >= 'A' && text[i] <= 'Z') ? text[i] + 32 : text[i];
    return out;
}

static bool zeno_starts_with(const char *text, const char *start) {
    return strncmp(text, start, strlen(start)) == 0;
}

static bool zeno_ends_with(const char *text, const char *end) {
    size_t length = strlen(text);
    size_t tail = strlen(end);
    return tail <= length && strcmp(text + length - tail, end) == 0;
}

/* `text.split()`: the words of `text`, with the blanks between them gone. */
static zeno_list *zeno_split_blanks(const char *text) {
    zeno_list *parts = zeno_list_new(0);
    const char *p = text;
    while (*p != '\0') {
        while (zeno_is_blank(*p)) p++;
        const char *start = p;
        while (*p != '\0' && !zeno_is_blank(*p)) p++;
        if (p > start) zeno_list_append(parts, (zeno_slot){.s = zeno_copy(start, (size_t)(p - start))});
    }
    return parts;
}

/* `text.split(",")`: the pieces of `text` between each `separator`. */
static zeno_list *zeno_split(const char *text, const char *separator, int line, int col) {
    size_t step = strlen(separator);
    if (step == 0) zeno_fail("ValueError", "cannot split on an empty str", line, col);
    zeno_list *parts = zeno_list_new(0);
    const char *start = text;
    for (;;) {
        const char *found = strstr(start, separator);
        size_t length = found != NULL ? (size_t)(found - start) : strlen(start);
        zeno_list_append(parts, (zeno_slot){.s = zeno_copy(start, length)});
        if (found == NULL) break;
        start = found + step;
    }
    return parts;
}

/* `text.replace(old, new)`: every `old` in `text` becomes `new`. */
static const char *zeno_replace(const char *text, const char *old, const char *new_text) {
    size_t step = strlen(old);
    const char *out = "";
    const char *start = text;
    if (step == 0) {
        /* An empty `old` is found between every two characters. */
        for (const char *p = text; *p != '\0'; p++) {
            if (((unsigned char)*p & 0xC0) != 0x80) out = zeno_join(2, out, new_text);
            out = zeno_join(2, out, zeno_copy(p, 1));
        }
        return zeno_join(2, out, new_text);
    }
    for (;;) {
        const char *found = strstr(start, old);
        if (found == NULL) return zeno_join(2, out, start);
        out = zeno_join(3, out, zeno_copy(start, (size_t)(found - start)), new_text);
        start = found + step;
    }
}

/* --------------------------------------------------- Values that may be none
 *
 * `str or none` and the other kinds that are already pointers need no extra
 * room: the pointer is NULL when there is nothing. A number or a bool has no
 * spare value to mean "nothing", so it travels with a flag beside it.
 */

typedef struct {
    bool present;
    int64_t value;
} zeno_opt_int;

typedef struct {
    bool present;
    double value;
} zeno_opt_float;

typedef struct {
    bool present;
    bool value;
} zeno_opt_bool;

/* A list gives every item one slot, which is too small for a number and its
 * flag together. So in a list, a number that may be `none` is kept as a
 * pointer: NULL for `none`, or the address of a copy of the number. */

static void *zeno_box_int(zeno_opt_int value) {
    if (!value.present) return NULL;
    int64_t *box = (int64_t *)zeno_alloc_text(sizeof(int64_t));
    *box = value.value;
    return box;
}

static zeno_opt_int zeno_unbox_int(const void *box) {
    return box == NULL ? (zeno_opt_int){false, 0} : (zeno_opt_int){true, *(const int64_t *)box};
}

static void *zeno_box_float(zeno_opt_float value) {
    if (!value.present) return NULL;
    double *box = (double *)zeno_alloc_text(sizeof(double));
    *box = value.value;
    return box;
}

static zeno_opt_float zeno_unbox_float(const void *box) {
    return box == NULL ? (zeno_opt_float){false, 0.0} : (zeno_opt_float){true, *(const double *)box};
}

static void *zeno_box_bool(zeno_opt_bool value) {
    if (!value.present) return NULL;
    bool *box = (bool *)zeno_alloc_text(sizeof(bool));
    *box = value.value;
    return box;
}

static zeno_opt_bool zeno_unbox_bool(const void *box) {
    return box == NULL ? (zeno_opt_bool){false, false} : (zeno_opt_bool){true, *(const bool *)box};
}

/* A variable that other code can change may have become `none` again since
 * it was checked. These look once more before the value is used. */
__attribute__((noreturn)) static void zeno_gone(int line, int col) {
    zeno_bug("this was checked with `is not none`, but it has been set to `none` since", line, col);
}

static void *zeno_still(const void *pointer, int line, int col) {
    if (pointer == NULL) zeno_gone(line, col);
    return (void *)pointer;
}

static int64_t zeno_still_int(zeno_opt_int value, int line, int col) {
    if (!value.present) zeno_gone(line, col);
    return value.value;
}

static double zeno_still_float(zeno_opt_float value, int line, int col) {
    if (!value.present) zeno_gone(line, col);
    return value.value;
}

static bool zeno_still_bool(zeno_opt_bool value, int line, int col) {
    if (!value.present) zeno_gone(line, col);
    return value.value;
}

/* ----------------------------------------------------- Functions as values
 *
 * A function held in a variable is a pointer to one of these. `code` is the
 * C function to run. It is stored as the plainest kind of function pointer
 * and turned back into its real type at each call, where the compiler
 * knows what that type is.
 *
 * A function written inside another one can use the variables around it.
 * Those are stored straight after this header, in a struct the compiler
 * writes for that function, and the function receives a pointer to its own
 * struct as a hidden first argument.
 */

typedef void (*zeno_code)(void);

typedef struct zeno_closure {
    zeno_code code;
    const char *name; /* for `print`; empty when the function has no name */
} zeno_closure;

static const char *zeno_show_function(zeno_closure *function) {
    if (function->name[0] == '\0') return "<function>";
    return zeno_join(3, "<function ", function->name, ">");
}

/* ------------------------------------------------------------- The program */
