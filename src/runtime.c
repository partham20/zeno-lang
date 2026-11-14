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
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The Zeno source file, for error messages. Set at the top of main. */
static const char *zeno_file = "";

/* Stop the program with an error nobody caught. This prints what
 * `zeno run` prints, and exits with the same status. */
static void zeno_fail(const char *kind, const char *message, int line, int col) {
    fflush(stdout);
    fprintf(stderr, "error: uncaught %s: %s\n --> %s:%d:%d\n", kind, message, zeno_file, line, col);
    exit(1);
}

static void zeno_overflow(int line, int col) {
    zeno_fail("OverflowError", "the result is too big for an int", line, col);
}

static void zeno_by_zero(int line, int col) {
    zeno_fail("ZeroDivisionError", "division by zero", line, col);
}

/* Memory for strings built while the program runs. Nothing is freed yet:
 * giving memory back is the job of the memory management stage. */
static char *zeno_alloc(size_t size) {
    char *memory = malloc(size);
    if (memory == NULL) {
        fflush(stdout);
        fprintf(stderr, "error: out of memory\n");
        exit(1);
    }
    return memory;
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
    char *copy = zeno_alloc(length + 1);
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

    char *joined = zeno_alloc(length + 1);
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
    char *text = zeno_alloc((size_t)length + 1);
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
    char *out = zeno_alloc(length + (size_t)padding + 1);
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

/* ------------------------------------------------------------- The program */
