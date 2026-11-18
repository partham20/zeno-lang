/* The C end of `import c` (docs/c.md).

   `zeno run` builds this once with the C compiler, and starts it. It
   connects back to Zeno over a socket of this machine, then does what
   each line it is sent asks for, and answers with one line. The lines
   are the ones bridge.py reads and writes:

       none, a bool, an int, a float, a str     as JSON writes them
       a float JSON cannot write                ["f", "nan"], "inf", "-inf"
       a whole number past 2^63                 ["u", "digits"]
       a list                                   ["l", [item, ...]]
       anything else, which stays here          ["p", number]

   What stays here is a library, a function of it, or a pointer. Each
   is kept in `kept` under its number until Zeno says that nobody there
   holds it any more.

   A library is not this program's to know. For each `c.library`, Zeno
   writes a small library of C of its own (the "stubs"): the functions
   that were declared, each wrapped in one that takes its arguments as
   an array of slots, and a table that says what each slot holds. It
   is built with the library it calls, and this program loads it. So
   this file knows no function of any library, and the C compiler,
   which knows the types, writes every call. */

#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#define _POSIX_C_SOURCE 200809L
#endif

#include <errno.h>
#include <math.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#include <windows.h>
typedef SOCKET zeno_socket;
#else
#include <arpa/inet.h>
#include <dlfcn.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>
typedef int zeno_socket;
#endif

/* ------------------------------------------------------ What the stubs give */

/* These four are written the same way in the stubs (bridge.rs). */
typedef union {
    int64_t i;
    uint64_t u;
    double d;
    void *p;
} zeno_slot;

/* What a slot holds. `code` is 'v' nothing, 'b' a bool, 'i' a signed
   whole number, 'u' an unsigned one, 'f' a float, 's' a pointer to
   char (text), 'p' any other pointer, 'o' something else. For a
   pointer, `pointee` and `pointee_size` say what it points to, and
   `konst` that the function promises not to change it. */
typedef struct {
    char code;
    unsigned char size;
    unsigned char konst;
    char pointee;
    unsigned char pointee_size;
} zeno_kind;

typedef struct {
    const char *name;
    const char *declared;
    int arity;
    const zeno_kind *kinds; /* kinds[0] is what it gives, then each parameter */
    const char *const *types;
    void (*call)(zeno_slot *, zeno_slot *);
} zeno_function;

typedef struct {
    int count;
    const zeno_function *functions;
} zeno_table;

/* ------------------------------------------------------------- What is kept */

enum { LIBRARY, FUNCTION, POINTER };

typedef struct {
    int alive;
    int what;
    const zeno_table *table; /* LIBRARY, FUNCTION */
    const zeno_function *function; /* FUNCTION */
    void *pointer; /* POINTER */
    char *label; /* LIBRARY: its name */
} zeno_thing;

static zeno_thing *kept;
static size_t kept_size;
static int64_t first_number;

static int64_t keep(zeno_thing thing) {
    zeno_thing *grown = realloc(kept, (kept_size + 1) * sizeof *kept);
    if (!grown) {
        fprintf(stderr, "zeno: the C helper ran out of memory\n");
        exit(1);
    }
    kept = grown;
    thing.alive = 1;
    kept[kept_size] = thing;
    kept_size += 1;
    return first_number + (int64_t)kept_size;
}

static zeno_thing *kept_as(int64_t number) {
    int64_t at = number - first_number - 1;
    if (at < 0 || (uint64_t)at >= kept_size || !kept[at].alive) return NULL;
    return &kept[at];
}

static void let_go(int64_t number) {
    zeno_thing *thing = kept_as(number);
    if (!thing) return;
    thing->alive = 0;
    free(thing->label);
    thing->label = NULL;
}

/* ------------------------------------------------------------ Reading JSON */

enum { J_NULL, J_FALSE, J_TRUE, J_INT, J_BIG, J_FLOAT, J_TEXT, J_LIST, J_OBJECT };

typedef struct json json;
struct json {
    int type;
    int64_t i; /* J_INT */
    uint64_t u; /* J_BIG: past INT64_MAX */
    double d; /* J_FLOAT */
    char *text; /* J_TEXT, with its length */
    size_t length;
    json *items; /* J_LIST, J_OBJECT */
    char **keys; /* J_OBJECT */
    size_t count;
};

typedef struct {
    const char *at;
    const char *end;
} reader;

static void blanks(reader *r) {
    while (r->at < r->end && (*r->at == ' ' || *r->at == '\t' || *r->at == '\r' || *r->at == '\n')) r->at++;
}

static int hex4(reader *r, unsigned *out) {
    unsigned value = 0;
    for (int k = 0; k < 4; k++) {
        if (r->at >= r->end) return 0;
        char c = *r->at++;
        value <<= 4;
        if (c >= '0' && c <= '9') value |= (unsigned)(c - '0');
        else if (c >= 'a' && c <= 'f') value |= (unsigned)(c - 'a' + 10);
        else if (c >= 'A' && c <= 'F') value |= (unsigned)(c - 'A' + 10);
        else return 0;
    }
    *out = value;
    return 1;
}

static void put_utf8(char *out, size_t *n, unsigned code) {
    if (code < 0x80) out[(*n)++] = (char)code;
    else if (code < 0x800) {
        out[(*n)++] = (char)(0xC0 | (code >> 6));
        out[(*n)++] = (char)(0x80 | (code & 0x3F));
    } else if (code < 0x10000) {
        out[(*n)++] = (char)(0xE0 | (code >> 12));
        out[(*n)++] = (char)(0x80 | ((code >> 6) & 0x3F));
        out[(*n)++] = (char)(0x80 | (code & 0x3F));
    } else {
        out[(*n)++] = (char)(0xF0 | (code >> 18));
        out[(*n)++] = (char)(0x80 | ((code >> 12) & 0x3F));
        out[(*n)++] = (char)(0x80 | ((code >> 6) & 0x3F));
        out[(*n)++] = (char)(0x80 | (code & 0x3F));
    }
}

static int read_text(reader *r, char **text, size_t *length) {
    if (r->at >= r->end || *r->at != '"') return 0;
    r->at++;
    /* Escapes only ever make the text shorter. */
    char *out = malloc((size_t)(r->end - r->at) + 1);
    size_t n = 0;
    if (!out) return 0;
    while (r->at < r->end && *r->at != '"') {
        char c = *r->at++;
        if (c != '\\') {
            out[n++] = c;
            continue;
        }
        if (r->at >= r->end) break;
        char e = *r->at++;
        switch (e) {
        case 'n': out[n++] = '\n'; break;
        case 't': out[n++] = '\t'; break;
        case 'r': out[n++] = '\r'; break;
        case 'b': out[n++] = '\b'; break;
        case 'f': out[n++] = '\f'; break;
        case 'u': {
            unsigned code;
            if (!hex4(r, &code)) {
                free(out);
                return 0;
            }
            if (code >= 0xD800 && code < 0xDC00 && r->end - r->at >= 6 && r->at[0] == '\\' && r->at[1] == 'u') {
                unsigned low;
                r->at += 2;
                if (hex4(r, &low) && low >= 0xDC00 && low < 0xE000) code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
            }
            put_utf8(out, &n, code);
            break;
        }
        default: out[n++] = e;
        }
    }
    if (r->at >= r->end) {
        free(out);
        return 0;
    }
    r->at++;
    out[n] = 0;
    *text = out;
    *length = n;
    return 1;
}

static int read_value(reader *r, json *out, int depth);

static int read_items(reader *r, json *out, int depth, int object) {
    size_t room = 4;
    out->items = malloc(room * sizeof *out->items);
    out->keys = object ? malloc(room * sizeof *out->keys) : NULL;
    out->count = 0;
    r->at++;
    blanks(r);
    if (r->at < r->end && *r->at == (object ? '}' : ']')) {
        r->at++;
        return 1;
    }
    for (;;) {
        if (out->count == room) {
            room *= 2;
            out->items = realloc(out->items, room * sizeof *out->items);
            if (object) out->keys = realloc(out->keys, room * sizeof *out->keys);
        }
        blanks(r);
        if (object) {
            size_t length;
            if (!read_text(r, &out->keys[out->count], &length)) return 0;
            blanks(r);
            if (r->at >= r->end || *r->at != ':') return 0;
            r->at++;
        }
        if (!read_value(r, &out->items[out->count], depth + 1)) return 0;
        out->count++;
        blanks(r);
        if (r->at < r->end && *r->at == ',') {
            r->at++;
            continue;
        }
        if (r->at < r->end && *r->at == (object ? '}' : ']')) {
            r->at++;
            return 1;
        }
        return 0;
    }
}

static int read_value(reader *r, json *out, int depth) {
    memset(out, 0, sizeof *out);
    if (depth > 1000) return 0;
    blanks(r);
    if (r->at >= r->end) return 0;
    char c = *r->at;
    if (c == '"') {
        out->type = J_TEXT;
        return read_text(r, &out->text, &out->length);
    }
    if (c == '[') {
        out->type = J_LIST;
        return read_items(r, out, depth, 0);
    }
    if (c == '{') {
        out->type = J_OBJECT;
        return read_items(r, out, depth, 1);
    }
    if (r->end - r->at >= 4 && !strncmp(r->at, "null", 4)) {
        r->at += 4;
        out->type = J_NULL;
        return 1;
    }
    if (r->end - r->at >= 4 && !strncmp(r->at, "true", 4)) {
        r->at += 4;
        out->type = J_TRUE;
        return 1;
    }
    if (r->end - r->at >= 5 && !strncmp(r->at, "false", 5)) {
        r->at += 5;
        out->type = J_FALSE;
        return 1;
    }
    const char *start = r->at;
    int floating = 0;
    while (r->at < r->end && strchr("0123456789+-.eE", *r->at)) {
        if (strchr(".eE", *r->at)) floating = 1;
        r->at++;
    }
    if (r->at == start) return 0;
    char number[64];
    size_t n = (size_t)(r->at - start);
    if (n >= sizeof number) n = sizeof number - 1;
    memcpy(number, start, n);
    number[n] = 0;
    if (floating) {
        out->type = J_FLOAT;
        out->d = strtod(number, NULL);
        return 1;
    }
    errno = 0;
    if (number[0] == '-') {
        out->type = J_INT;
        out->i = strtoll(number, NULL, 10);
    } else {
        uint64_t u = strtoull(number, NULL, 10);
        if (u > (uint64_t)INT64_MAX) {
            out->type = J_BIG;
            out->u = u;
        } else {
            out->type = J_INT;
            out->i = (int64_t)u;
        }
    }
    return errno == 0;
}

static void forget(json *value) {
    free(value->text);
    for (size_t k = 0; k < value->count; k++) {
        forget(&value->items[k]);
        if (value->keys) free(value->keys[k]);
    }
    free(value->items);
    free(value->keys);
}

static json *member(json *object, const char *name) {
    if (!object || object->type != J_OBJECT) return NULL;
    for (size_t k = 0; k < object->count; k++)
        if (!strcmp(object->keys[k], name)) return &object->items[k];
    return NULL;
}

/* ------------------------------------------------------------ Writing JSON */

typedef struct {
    char *text;
    size_t length;
    size_t room;
} buffer;

static void add(buffer *b, const char *text, size_t n) {
    if (b->length + n + 1 > b->room) {
        while (b->length + n + 1 > b->room) b->room = b->room ? b->room * 2 : 256;
        b->text = realloc(b->text, b->room);
        if (!b->text) {
            fprintf(stderr, "zeno: the C helper ran out of memory\n");
            exit(1);
        }
    }
    memcpy(b->text + b->length, text, n);
    b->length += n;
    b->text[b->length] = 0;
}

static void say(buffer *b, const char *text) { add(b, text, strlen(text)); }

static void say_number(buffer *b, const char *format, ...) {
    char text[128];
    va_list rest;
    va_start(rest, format);
    vsnprintf(text, sizeof text, format, rest);
    va_end(rest);
    say(b, text);
}

static void say_text(buffer *b, const char *text, size_t n) {
    say(b, "\"");
    for (size_t k = 0; k < n; k++) {
        unsigned char c = (unsigned char)text[k];
        if (c == '"') say(b, "\\\"");
        else if (c == '\\') say(b, "\\\\");
        else if (c == '\n') say(b, "\\n");
        else if (c < 0x20 || c == 0x7f) say_number(b, "\\u%04x", c);
        else add(b, (const char *)&text[k], 1);
    }
    say(b, "\"");
}

/* Whether these bytes are text: UTF-8, with no character written in
   more bytes than it needs, and none of the halves of UTF-16. */
/* How many bytes the character of UTF-8 at `k` takes, or 0 if the bytes
   there are not one. */
static size_t character_at(const unsigned char *s, size_t n, size_t k) {
    unsigned char c = s[k];
    size_t more;
    unsigned code;
    if (c < 0x80) return 1;
    else if (c >= 0xC2 && c <= 0xDF) more = 1, code = c & 0x1F;
    else if (c >= 0xE0 && c <= 0xEF) more = 2, code = c & 0x0F;
    else if (c >= 0xF0 && c <= 0xF4) more = 3, code = c & 0x07;
    else return 0;
    if (k + more >= n) return 0;
    for (size_t m = 1; m <= more; m++) {
        if ((s[k + m] & 0xC0) != 0x80) return 0;
        code = (code << 6) | (s[k + m] & 0x3F);
    }
    if ((more == 2 && code < 0x800) || (more == 3 && (code < 0x10000 || code > 0x10FFFF)) || (code >= 0xD800 && code < 0xE000))
        return 0;
    return more + 1;
}

static int is_text(const unsigned char *s, size_t n) {
    for (size_t k = 0; k < n;) {
        size_t length = character_at(s, n, k);
        if (!length) return 0;
        k += length;
    }
    return 1;
}

static void say_float(buffer *b, double x) {
    if (isnan(x)) say(b, "[\"f\",\"nan\"]");
    else if (isinf(x)) say(b, x > 0 ? "[\"f\",\"inf\"]" : "[\"f\",\"-inf\"]");
    else {
        char text[64];
        snprintf(text, sizeof text, "%.17g", x);
        /* The shortest that reads back as the same float. */
        for (int digits = 1; digits <= 17; digits++) {
            char shorter[64];
            snprintf(shorter, sizeof shorter, "%.*g", digits, x);
            if (strtod(shorter, NULL) == x) {
                strcpy(text, shorter);
                break;
            }
        }
        /* A float always has a point or an exponent, so that it
           arrives as a float. A comma is a point in some languages. */
        for (char *c = text; *c; c++)
            if (*c == ',') *c = '.';
        if (!strpbrk(text, ".e")) strcat(text, ".0");
        say(b, text);
    }
}

/* ------------------------------------------------------------- An error */

/* What went wrong, as Zeno raises it: its kind and what it says. */
static char failure_kind[32];
static char failure_text[1024];

static int fail(const char *kind, const char *format, ...) {
    va_list rest;
    va_start(rest, format);
    snprintf(failure_kind, sizeof failure_kind, "%s", kind);
    vsnprintf(failure_text, sizeof failure_text, format, rest);
    va_end(rest);
    return 0;
}

/* ------------------------------------------------------- Loading a library */

static void *open_library(const char *path) {
#ifdef _WIN32
    return (void *)LoadLibraryA(path);
#else
    return dlopen(path, RTLD_NOW | RTLD_LOCAL);
#endif
}

static void *find_symbol(void *library, const char *name) {
#ifdef _WIN32
    return (void *)GetProcAddress((HMODULE)library, name);
#else
    return dlsym(library, name);
#endif
}

static const char *why_not_opened(void) {
#ifdef _WIN32
    static char said[64];
    snprintf(said, sizeof said, "Windows error %lu", (unsigned long)GetLastError());
    return said;
#else
    const char *said = dlerror();
    return said ? said : "it could not be opened";
#endif
}

/* `c.library`: the stubs Zeno built, loaded. */
static int load(json *ask, buffer *answer) {
    json *path = member(ask, "name");
    json *label = member(ask, "label");
    if (!path || path->type != J_TEXT) return fail("Error", "Zeno asked to load a library and named none");
    void *library = open_library(path->text);
    if (!library) {
        const char *why = why_not_opened();
        /* A function that was declared and that the library does not
           have shows here, when the stubs that call it are loaded. */
        const char *missing = strstr(why, "undefined symbol: ");
        if (missing) {
            return fail("Error", "the library `%s` has no `%s`, which was declared",
                        label && label->type == J_TEXT ? label->text : "?", missing + strlen("undefined symbol: "));
        }
        return fail("FileError", "C could not load the library: %s", why);
    }
    const zeno_table *(*table)(void) = (const zeno_table *(*)(void))find_symbol(library, "zeno_library");
    if (!table) return fail("Error", "what Zeno built for the library has no table of its functions");
    zeno_thing thing = {0};
    thing.what = LIBRARY;
    thing.table = table();
    if (label && label->type == J_TEXT) {
        thing.label = malloc(label->length + 1);
        if (thing.label) memcpy(thing.label, label->text, label->length + 1);
    }
    say_number(answer, "[\"p\",%lld]", (long long)keep(thing));
    return 1;
}

/* --------------------------------------------------------------- A call */

static const char *nth(int n) {
    static char text[16];
    const char *ending = (n % 100 >= 11 && n % 100 <= 13) ? "th" : n % 10 == 1 ? "st" : n % 10 == 2 ? "nd" : n % 10 == 3 ? "rd" : "th";
    snprintf(text, sizeof text, "%d%s", n, ending);
    return text;
}

static const char *described(json *value) {
    switch (value->type) {
    case J_NULL: return "none";
    case J_TRUE:
    case J_FALSE: return "a bool";
    case J_INT:
    case J_BIG: return "an int";
    case J_FLOAT: return "a float";
    case J_TEXT: return "a str";
    case J_LIST:
        if (value->count == 2 && value->items[0].type == J_TEXT) {
            const char *tag = value->items[0].text;
            if (!strcmp(tag, "f")) return "a float";
            if (!strcmp(tag, "u")) return "an int";
            if (!strcmp(tag, "l")) return "a list";
            if (!strcmp(tag, "d")) return "a dict";
            if (!strcmp(tag, "p")) {
                zeno_thing *thing = kept_as(value->items[1].i);
                if (thing && thing->what == LIBRARY) return "a library of C";
                if (thing && thing->what == FUNCTION) return "a function of C";
                return "a pointer of C";
            }
        }
        return "a list";
    }
    return "a value";
}

/* What a parameter is called in a message: "the 2nd parameter of
   `crc32` (`const unsigned char *`)". */
static char where_text[512];
static const char *where(const zeno_function *f, int at) {
    snprintf(where_text, sizeof where_text, "the %s parameter of `%s` (`%s`)", nth(at), f->name, f->types[at]);
    return where_text;
}

/* A number Zeno sent: whole or not, and whether it is past INT64_MAX. */
static int number_of(json *value, int *whole, int64_t *i, uint64_t *u, double *d, int *big) {
    *big = 0;
    if (value->type == J_INT) {
        *whole = 1, *i = value->i, *d = (double)value->i;
        return 1;
    }
    if (value->type == J_BIG) {
        *whole = 1, *big = 1, *u = value->u, *d = (double)value->u;
        return 1;
    }
    if (value->type == J_FLOAT) {
        *whole = 0, *d = value->d;
        return 1;
    }
    if (value->type == J_LIST && value->count == 2 && value->items[0].type == J_TEXT && value->items[1].type == J_TEXT) {
        const char *tag = value->items[0].text, *inside = value->items[1].text;
        if (!strcmp(tag, "f")) {
            *whole = 0;
            *d = !strcmp(inside, "nan") ? NAN : !strcmp(inside, "inf") ? INFINITY : !strcmp(inside, "-inf") ? -INFINITY : strtod(inside, NULL);
            return 1;
        }
        if (!strcmp(tag, "u")) {
            *whole = 1, *big = 1, *u = strtoull(inside, NULL, 10), *d = (double)*u;
            return 1;
        }
    }
    return 0;
}

/* A value of Zeno into a slot of this kind: a number, a bool, or a
   pointer that Zeno holds. `what` names the place for a message. */
static int to_slot(json *value, char code, int size, zeno_slot *slot, const char *what) {
    int whole, big;
    int64_t i = 0;
    uint64_t u = 0;
    double d = 0;
    if (code == 'b') {
        if (value->type != J_TRUE && value->type != J_FALSE) return fail("Error", "%s takes a bool, and this is %s", what, described(value));
        slot->u = value->type == J_TRUE;
        return 1;
    }
    if (code == 'f') {
        if (!number_of(value, &whole, &i, &u, &d, &big)) return fail("Error", "%s takes a number, and this is %s", what, described(value));
        slot->d = d;
        return 1;
    }
    if (code == 'i' || code == 'u') {
        if (!number_of(value, &whole, &i, &u, &d, &big) || !whole) return fail("Error", "%s takes a whole number, and this is %s", what, described(value));
        int bits = size * 8;
        if (code == 'i') {
            int64_t low = bits >= 64 ? INT64_MIN : -((int64_t)1 << (bits - 1));
            int64_t high = bits >= 64 ? INT64_MAX : ((int64_t)1 << (bits - 1)) - 1;
            if (big || i < low || i > high)
                return fail("OverflowError", "%s holds %lld to %lld, and this is %s%llu", what, (long long)low, (long long)high,
                            big ? "" : (i < 0 ? "-" : ""), big ? (unsigned long long)u : (unsigned long long)(i < 0 ? -(uint64_t)i : (uint64_t)i));
            slot->i = i;
        } else {
            uint64_t high = bits >= 64 ? UINT64_MAX : ((uint64_t)1 << bits) - 1;
            if ((!big && i < 0) || (big ? u : (uint64_t)i) > high)
                return fail("OverflowError", "%s holds 0 to %llu, and this is %s%llu", what, (unsigned long long)high, !big && i < 0 ? "-" : "",
                            big ? (unsigned long long)u : (unsigned long long)(i < 0 ? -(uint64_t)i : (uint64_t)i));
            slot->u = big ? u : (uint64_t)i;
        }
        return 1;
    }
    if (code == 'p' || code == 's') {
        if (value->type == J_NULL) {
            slot->p = NULL;
            return 1;
        }
        if (value->type == J_LIST && value->count == 2 && value->items[0].type == J_TEXT && !strcmp(value->items[0].text, "p")) {
            zeno_thing *thing = kept_as(value->items[1].i);
            if (thing && thing->what == POINTER) {
                slot->p = thing->pointer;
                return 1;
            }
        }
        return fail("Error", "%s takes a pointer: a pointer that C gave, or none, and this is %s", what, described(value));
    }
    return fail("Error", "%s is of a kind Zeno cannot give C yet", what);
}

/* Memory that a list or a str was copied into for one call. */
typedef struct {
    void *memory;
    int argument; /* which parameter, from 0 */
    size_t count; /* how many items */
    char pointee;
    int size;
    int back; /* copied back into the list after the call */
} lent;

static void store(void *at, char code, int size, zeno_slot slot) {
    switch (code) {
    case 'f':
        if (size == 4) {
            float x = (float)slot.d;
            memcpy(at, &x, 4);
        } else memcpy(at, &slot.d, 8);
        break;
    case 'p':
    case 's': memcpy(at, &slot.p, sizeof slot.p); break;
    default: {
        /* Little or big end first, the low bytes of the number are the
           ones a smaller type keeps. */
        uint8_t u8 = (uint8_t)slot.u;
        uint16_t u16 = (uint16_t)slot.u;
        uint32_t u32 = (uint32_t)slot.u;
        if (size == 1) memcpy(at, &u8, 1);
        else if (size == 2) memcpy(at, &u16, 2);
        else if (size == 4) memcpy(at, &u32, 4);
        else memcpy(at, &slot.u, 8);
    }
    }
}

static zeno_slot load_slot(const void *at, char code, int size) {
    zeno_slot slot;
    slot.u = 0;
    switch (code) {
    case 'f':
        if (size == 4) {
            float x;
            memcpy(&x, at, 4);
            slot.d = x;
        } else memcpy(&slot.d, at, 8);
        break;
    case 'p':
    case 's': memcpy(&slot.p, at, sizeof slot.p); break;
    case 'i': {
        if (size == 1) slot.i = *(const int8_t *)at;
        else if (size == 2) {
            int16_t x;
            memcpy(&x, at, 2);
            slot.i = x;
        } else if (size == 4) {
            int32_t x;
            memcpy(&x, at, 4);
            slot.i = x;
        } else memcpy(&slot.i, at, 8);
        break;
    }
    default: {
        if (size == 1) slot.u = *(const uint8_t *)at;
        else if (size == 2) {
            uint16_t x;
            memcpy(&x, at, 2);
            slot.u = x;
        } else if (size == 4) {
            uint32_t x;
            memcpy(&x, at, 4);
            slot.u = x;
        } else memcpy(&slot.u, at, 8);
    }
    }
    return slot;
}

/* A value of C, as Zeno receives it. */
static void say_slot(buffer *b, char code, int size, zeno_slot slot) {
    (void)size;
    switch (code) {
    case 'v': say(b, "null"); break;
    case 'b': say(b, slot.u ? "true" : "false"); break;
    case 'f': say_float(b, slot.d); break;
    case 'i': say_number(b, "%lld", (long long)slot.i); break;
    case 'U': /* a u64 of Zeno, whatever its size */
        say_number(b, "[\"u\",\"%llu\"]", (unsigned long long)slot.u);
        break;
    case 'u':
        if (slot.u > (uint64_t)INT64_MAX) say_number(b, "[\"u\",\"%llu\"]", (unsigned long long)slot.u);
        else say_number(b, "%llu", (unsigned long long)slot.u);
        break;
    case 's':
        if (!slot.p) say(b, "null");
        else {
            const char *text = slot.p;
            size_t n = strlen(text);
            if (is_text((const unsigned char *)text, n)) say_text(b, text, n);
            else {
                /* Text of C is bytes. A byte that is not part of a
                   character of UTF-8 arrives as U+FFFD, the character
                   that stands for one that could not be read. */
                char *fixed = malloc(n * 3 + 1);
                size_t out = 0;
                if (!fixed) {
                    say(b, "null");
                    break;
                }
                for (size_t k = 0; k < n;) {
                    size_t length = character_at((const unsigned char *)text, n, k);
                    if (length) {
                        memcpy(fixed + out, text + k, length);
                        out += length;
                        k += length;
                    } else {
                        memcpy(fixed + out, "\xEF\xBF\xBD", 3);
                        out += 3;
                        k++;
                    }
                }
                say_text(b, fixed, out);
                free(fixed);
            }
        }
        break;
    case 'p':
        if (!slot.p) say(b, "null");
        else {
            zeno_thing thing = {0};
            thing.what = POINTER;
            thing.pointer = slot.p;
            say_number(b, "[\"p\",%lld]", (long long)keep(thing));
        }
        break;
    default: say(b, "null");
    }
}

/* A list or a str given where a pointer is wanted: the memory it
   points to, made for this call. */
static int lend(json *value, const zeno_kind *kind, int argument, const char *what, lent *out, zeno_slot *slot) {
    char pointee = kind->pointee;
    int size = kind->pointee_size;
    /* `void *` and a pointer to something Zeno does not know the size
       of take bytes. */
    if (pointee == 'o' || size == 0) pointee = 'u', size = 1;
    memset(out, 0, sizeof *out);
    out->argument = argument;
    out->pointee = pointee;
    out->size = size;
    if (value->type == J_TEXT) {
        if (size != 1) return fail("Error", "%s points to numbers of %d bytes, and text is bytes", what, size);
        if (strlen(value->text) != value->length) return fail("ValueError", "%s takes text with no zero byte in it, as C ends text with one", what);
        out->memory = malloc(value->length + 1);
        if (!out->memory) return fail("Error", "there is no memory for %s", what);
        memcpy(out->memory, value->text, value->length + 1);
        slot->p = out->memory;
        return 1;
    }
    json *items = NULL;
    if (value->type == J_LIST && value->count == 2 && value->items[0].type == J_TEXT && !strcmp(value->items[0].text, "l") && value->items[1].type == J_LIST)
        items = &value->items[1];
    if (!items) return 0;
    out->count = items->count;
    out->back = !kind->konst;
    /* One more item, zero, so that text a function writes ends. */
    out->memory = calloc(items->count + 1, (size_t)size);
    if (!out->memory) return fail("Error", "there is no memory for %s", what);
    for (size_t k = 0; k < items->count; k++) {
        zeno_slot item;
        char said[600];
        snprintf(said, sizeof said, "item %zu of the list given for %s", k, what);
        if (!to_slot(&items->items[k], pointee == 's' ? 'p' : pointee, size, &item, said)) return 0;
        store((char *)out->memory + k * (size_t)size, pointee, size, item);
    }
    slot->p = out->memory;
    return 1;
}

static int call(zeno_thing *thing, json *ask, buffer *answer) {
    const zeno_function *f = thing->function;
    json *args = member(ask, "args");
    json *named = member(ask, "named");
    if (named && named->count > 0) return fail("Error", "a function of C takes what it is given in order, and `%s` has no named parameters", f->name);
    int given = args ? (int)args->count : 0;
    if (given != f->arity)
        return fail("Error", "`%s` takes %d argument%s, but %d %s given", f->name, f->arity, f->arity == 1 ? "" : "s", given, given == 1 ? "was" : "were");
    zeno_slot slots[64];
    lent lents[64];
    int lent_count = 0;
    int ok = 1;
    if (f->arity > 64) return fail("Error", "`%s` takes more than 64 arguments", f->name);
    for (int k = 0; k < f->arity && ok; k++) {
        const zeno_kind *kind = &f->kinds[k + 1];
        json *value = &args->items[k];
        slots[k].u = 0;
        if ((kind->code == 'p' || kind->code == 's') && (value->type == J_TEXT || value->type == J_LIST)) {
            int is_pointer = value->type == J_LIST && value->count == 2 && value->items[0].type == J_TEXT && !strcmp(value->items[0].text, "p");
            if (!is_pointer) {
                ok = lend(value, kind, k, where(f, k + 1), &lents[lent_count], &slots[k]);
                if (ok) lent_count++;
                else if (!failure_kind[0])
                    ok = fail("Error", "%s takes a pointer, a list or a str, and this is %s", where(f, k + 1), described(value));
                continue;
            }
        }
        ok = to_slot(value, kind->code, kind->size, &slots[k], where(f, k + 1));
    }
    if (ok) {
        zeno_slot result;
        result.u = 0;
        f->call(slots, &result);
        fflush(NULL);
        say(answer, "{\"back\":[");
        int first = 1;
        for (int k = 0; k < lent_count; k++) {
            lent *l = &lents[k];
            if (!l->back) continue;
            say_number(answer, first ? "[%d,[\"l\",[" : ",[%d,[\"l\",[", l->argument);
            first = 0;
            for (size_t m = 0; m < l->count; m++) {
                if (m) say(answer, ",");
                say_slot(answer, l->pointee == 's' ? 'p' : l->pointee, l->size, load_slot((char *)l->memory + m * (size_t)l->size, l->pointee, l->size));
            }
            say(answer, "]]]");
        }
        say(answer, "],\"ok\":");
        say_slot(answer, f->kinds[0].pointee == 'U' ? 'U' : f->kinds[0].code, f->kinds[0].size, result);
        say(answer, "}");
    }
    for (int k = 0; k < lent_count; k++) free(lents[k].memory);
    return ok ? 2 : 0;
}

/* ------------------------------------------------------- What Zeno asks */

static void shown(zeno_thing *thing, buffer *b) {
    char text[1100];
    if (thing->what == LIBRARY) snprintf(text, sizeof text, "<C library %s>", thing->label ? thing->label : "");
    else if (thing->what == FUNCTION) snprintf(text, sizeof text, "%s", thing->function->declared);
    else snprintf(text, sizeof text, "<C pointer %p>", thing->pointer);
    say_text(b, text, strlen(text));
}

static const char *kind_of(zeno_thing *thing) {
    return thing->what == LIBRARY ? "a library of C" : thing->what == FUNCTION ? "a function of C" : "a pointer of C";
}

/* Returns 1 when `answer` holds the value, 2 when it holds the whole
   line, and 0 when it failed. */
static int act(json *ask, buffer *answer) {
    json *op = member(ask, "op");
    if (!op || op->type != J_TEXT) return fail("Error", "Zeno sent a line with nothing to do");
    if (!strcmp(op->text, "import")) return load(ask, answer);
    json *on = member(ask, "on");
    zeno_thing *thing = NULL;
    if (on && on->type == J_LIST && on->count == 2) thing = kept_as(on->items[1].i);
    if (!thing) return fail("Error", "this value was kept by a C helper that has stopped since");
    json *name = member(ask, "name");
    if (!strcmp(op->text, "get")) {
        if (thing->what != LIBRARY) return fail("Error", "%s has no `%s`", kind_of(thing), name && name->type == J_TEXT ? name->text : "?");
        for (int k = 0; k < thing->table->count; k++) {
            if (name && name->type == J_TEXT && !strcmp(thing->table->functions[k].name, name->text)) {
                const zeno_function *f = &thing->table->functions[k];
                if (f->arity < 0) {
                    /* A value the declarations name, such as a constant of
                       a header: read now, as C would read it. */
                    zeno_slot result = {0};
                    f->call(NULL, &result);
                    say_slot(answer, f->kinds[0].pointee == 'U' ? 'U' : f->kinds[0].code, f->kinds[0].size, result);
                    return 1;
                }
                zeno_thing function = {0};
                function.what = FUNCTION;
                function.table = thing->table;
                function.function = &thing->table->functions[k];
                say_number(answer, "[\"p\",%lld]", (long long)keep(function));
                return 1;
            }
        }
        buffer names = {0};
        for (int k = 0; k < thing->table->count; k++) {
            say(&names, k ? ", " : "");
            say(&names, thing->table->functions[k].name);
        }
        fail("Error", "the library `%s` has no `%s` among what is declared for it. Declared: %s", thing->label ? thing->label : "",
             name && name->type == J_TEXT ? name->text : "?", names.text ? names.text : "none");
        free(names.text);
        return 0;
    }
    if (!strcmp(op->text, "call")) {
        if (thing->what != FUNCTION) return fail("Error", "%s cannot be called", kind_of(thing));
        return call(thing, ask, answer);
    }
    if (!strcmp(op->text, "str") || !strcmp(op->text, "repr")) {
        shown(thing, answer);
        return 1;
    }
    if (!strcmp(op->text, "equal")) {
        json *to = member(ask, "to");
        zeno_thing *other = NULL;
        if (to && to->type == J_LIST && to->count == 2 && to->items[0].type == J_TEXT && !strcmp(to->items[0].text, "p")) other = kept_as(to->items[1].i);
        int same = other && other->what == thing->what &&
                   (thing->what == POINTER ? other->pointer == thing->pointer : thing->what == FUNCTION ? other->function == thing->function : other->table == thing->table);
        say(answer, same ? "true" : "false");
        return 1;
    }
    if (!strcmp(op->text, "set")) return fail("Error", "%s has no fields to set", kind_of(thing));
    if (!strcmp(op->text, "copy")) return fail("Error", "%s holds nothing that can be copied. A list given for a pointer is the memory it points to", kind_of(thing));
    if (!strcmp(op->text, "items")) return fail("Error", "%s cannot be gone through with `for`", kind_of(thing));
    return fail("Error", "%s has no items and no length", kind_of(thing));
}

/* --------------------------------------------------------------- The line */

static int send_all(zeno_socket s, const char *text, size_t n) {
    while (n > 0) {
        int sent = (int)send(s, text, (int)n, 0);
        if (sent <= 0) return 0;
        text += sent;
        n -= (size_t)sent;
    }
    return 1;
}

int main(void) {
    const char *said = getenv("ZENO_BRIDGE");
    char port[16] = "", token[128] = "";
    long long first = 0;
    if (!said || sscanf(said, "%15s %127s %lld", port, token, &first) != 3) {
        fprintf(stderr, "zeno: the C helper is started by `zeno run`, not by hand\n");
        return 1;
    }
    first_number = first;
#ifdef _WIN32
    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) return 1;
    _putenv("ZENO_BRIDGE=");
#else
    unsetenv("ZENO_BRIDGE");
#endif
    zeno_socket s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in to;
    memset(&to, 0, sizeof to);
    to.sin_family = AF_INET;
    to.sin_port = htons((unsigned short)atoi(port));
    to.sin_addr.s_addr = htonl(0x7f000001);
    if (connect(s, (struct sockaddr *)&to, sizeof to) != 0) return 1;
    int yes = 1;
    setsockopt(s, IPPROTO_TCP, TCP_NODELAY, (const char *)&yes, sizeof yes);
    buffer hello = {0};
    say(&hello, "{\"hello\":");
    say_text(&hello, token, strlen(token));
    say(&hello, "}\n");
    if (!send_all(s, hello.text, hello.length)) return 1;
    free(hello.text);

    buffer line = {0};
    char chunk[65536];
    for (;;) {
        char *end = line.text ? memchr(line.text, '\n', line.length) : NULL;
        if (!end) {
            int got = (int)recv(s, chunk, sizeof chunk, 0);
            if (got <= 0) break;
            add(&line, chunk, (size_t)got);
            continue;
        }
        size_t length = (size_t)(end - line.text);
        json ask;
        reader r = {line.text, line.text + length};
        buffer answer = {0};
        buffer value = {0};
        failure_kind[0] = 0;
        int done;
        if (!read_value(&r, &ask, 0)) {
            forget(&ask);
            memset(&ask, 0, sizeof ask);
            done = fail("Error", "Zeno sent a line the C helper cannot read");
        } else {
            json *free_these = member(&ask, "free");
            if (free_these && free_these->type == J_LIST)
                for (size_t k = 0; k < free_these->count; k++) let_go(free_these->items[k].i);
            done = act(&ask, &value);
        }
        if (done == 2) {
            add(&answer, value.text, value.length);
        } else if (done == 1) {
            say(&answer, "{\"ok\":");
            add(&answer, value.text, value.length);
            say(&answer, "}");
        } else {
            say(&answer, "{\"err\":[");
            say_text(&answer, failure_kind[0] ? failure_kind : "Error", strlen(failure_kind[0] ? failure_kind : "Error"));
            say(&answer, ",");
            say_text(&answer, failure_text, strlen(failure_text));
            say(&answer, "]}");
        }
        say(&answer, "\n");
        forget(&ask);
        fflush(NULL);
        if (!send_all(s, answer.text, answer.length)) break;
        free(answer.text);
        free(value.text);
        memmove(line.text, end + 1, line.length - length - 1);
        line.length -= length + 1;
    }
    return 0;
}
