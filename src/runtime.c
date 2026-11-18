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
#ifdef ZENO_MINIMAL
/* About `ZENO_MINIMAL`, which marks parts of this file.
 *
 * `zeno build --minimal` writes the least C that runs the program
 * (docs/minimal.md, decisions chapter 30). Three things are done to this
 * file for it, by `runtime_of` in src/emit_c.rs and by src/minimal.rs:
 *
 *   1. What is between `#ifndef ZENO_MINIMAL` and its `#endif` is left
 *      out, or changed for what follows `#else`. Those parts are the
 *      checks a correct program never fails: an index out of range, an
 *      overflow, a division by zero, a key a dict does not have. With
 *      them gone such a mistake does whatever the C does, as in C.
 *   2. Nothing is told where in the Zeno source it was called from: the
 *      `int line, int col` at the end of a function's parameters, and the
 *      `line, col` at the end of a call, are taken out.
 *   3. Every function, type and variable the program does not use is
 *      left out, and so are the comments.
 *
 * And one thing is added. `print` there is C's `printf`, which shows a
 * whole number by itself but not a float the way Zeno writes one. So
 * `zeno_str_float` is in two pieces there: `zeno_float_text` writes the
 * float into sixty-four bytes it is handed, with no memory asked for,
 * and `zeno_quantity_text` does the same for the number of a quantity.
 * `zeno_radix` is in two pieces in the same way, for `{n:x}` and
 * `{reg:08b}`: `zeno_radix_text` writes into bytes it is handed.
 *
 * Every other build has these marks taken out and what is between them
 * kept, so its C is the same as before there was such a thing. A mark
 * has to be written exactly as the others are, on a line of its own:
 * the `#else` and the `#endif` name ZENO_MINIMAL in a comment after
 * them, as those of ZENO_FIRMWARE do. */
#endif /* ZENO_MINIMAL */

/* Microsoft's C library marks `fopen`, `strcpy` and their like as unsafe
 * and offers its own in their place, which no other system has. The
 * runtime checks every size itself, so the warnings are asked not to be
 * given. It has to be said before the first header is read. */
#ifdef _WIN32
#define _CRT_SECURE_NO_WARNINGS
#define _CRT_NONSTDC_NO_WARNINGS
#endif

#include <errno.h>
#include <inttypes.h>
#include <math.h>
#ifndef ZENO_FIRMWARE
#include <setjmp.h>
/* A `try` jumps with `_setjmp` and `_longjmp`, which leave the signal mask
 * alone, outside Windows. The LLVM build calls `_setjmp` (IR cannot call a
 * macro), and on macOS plain `longjmp` sets the mask from the jmp_buf,
 * where `_setjmp` stored none: after a caught error the program could be
 * left with the write-fault signal held back, and hang on the next write
 * to a guarded page (decision 22.14). One pair everywhere avoids that, and
 * saves a system call per `try` on macOS.
 *
 * On Windows `setjmp` is a macro that also hands over the frame of the
 * function, and `longjmp` then unwinds the stack to that frame as for an
 * exception of C++, which a Zeno program has no use for. Built by gcc 15
 * with `-Os`, that unwinding stopped a program with a caught error
 * (decision 32.1). MinGW says the frame itself, so there it is said to be
 * none, which asks for a plain jump as on every other system, and as the
 * LLVM build always asked. */
#ifdef _WIN32
#if defined(__MINGW32__) && defined(__x86_64__)
#define zeno_setjmp(place) _setjmp(place, NULL)
#else
#define zeno_setjmp(place) setjmp(place)
#endif
#define zeno_longjmp(place) longjmp(place, 1)
#else
#define zeno_setjmp(place) _setjmp(place)
#define zeno_longjmp(place) _longjmp(place, 1)
#endif
#endif /* ZENO_FIRMWARE */
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* `a * b + c` is two steps: multiply and round, then add and round. Some
 * processors (ARM64, and x86 where the compiler is told it may) have one
 * instruction that does both and rounds once, which is faster and now and
 * then gives another last digit: 74.3 where two steps give
 * 74.30000000000001. A C compiler is allowed to pick it without being
 * asked. `zeno run` always takes two steps, so the compiled program must
 * too, and these lines say so to the two families of C compiler. */
#if defined(__clang__)
#pragma STDC FP_CONTRACT OFF
#elif defined(__GNUC__)
#pragma GCC optimize("fp-contract=off")
#endif

/* A 32-bit x86 processor has two sets of float instructions. The old set
 * works to more digits than a float holds and rounds at the end, which
 * gives another last digit in the same way. gcc picks the old set there
 * unless it is told otherwise. The new set (SSE2, in every x86 processor
 * since 2001) rounds after each step, as every other processor does. */
#if defined(__i386__) && defined(__GNUC__) && !defined(__clang__)
#pragma GCC target("sse2,fpmath=sse")
#endif

/* On Windows a C program that prints "\n" is given "\r\n" in its place,
 * unless it says it wants its bytes left alone. `zeno run` prints "\n" on
 * every machine, so a compiled program says so, before `main` starts. A
 * Windows terminal shows either the same way. */
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
__attribute__((constructor)) static void zeno_plain_line_ends(void) {
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
}
#endif

/* The Zeno source file, for error messages. Set at the top of main. */
static const char *zeno_file = "";

/* The other files of a program that is written in several, set at the top
 * of main as well. A line number says which file it is in: the lines of
 * the first file count from 1, those of the second from 1,000,001, those
 * of the third from 2,000,001, and so on (see `Span` in src/token.rs). */
static const char *const *zeno_modules = NULL;

/* The file a line is in, and the line as that file numbers it. */
static const char *zeno_file_of(int line) {
    return line < 1000000 ? zeno_file : zeno_modules[line / 1000000 - 1];
}

static int zeno_line_of(int line) {
    return line % 1000000;
}

/* The name a kind of error is printed by. A class of an imported file is
 * `shapes.BadShape` inside the program, and prints as `BadShape`. */
static const char *zeno_kind_shown(const char *kind) {
    const char *dot = strrchr(kind, '.');
    return dot == NULL ? kind : dot + 1;
}

/* ------------------------------------------------------------------- Memory
 *
 * Text built while the program runs, lists, objects and function values
 * all need memory, and a program that runs for a year must give it back.
 * Zeno does that with a garbage collector of the simplest kind, called
 * mark and sweep:
 *
 *   1. Every piece of memory handed out (a "block") is kept in a page that
 *      knows which of its blocks are in use.
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
 * (How it answers that quickly is under "Where blocks live", below.)
 * This is called a conservative collector. A number that happens to look
 * like an address keeps a block alive for no reason, which wastes a little
 * memory and breaks nothing.
 */

#ifndef ZENO_NO_COLLECTOR

#ifdef _WIN32
#include <malloc.h> /* _aligned_malloc */
#endif

static void zeno_out_of_memory(void) {
    fflush(stdout);
    fprintf(stderr, "error: out of memory\n");
    exit(1);
}

/* Where blocks live: pages.
 *
 * Memory is asked of the system 64 KB at a time, at an address that is a
 * multiple of 64 KB: a page. A page holds blocks of one size only, and
 * says at its start which size, which of its blocks are in use, and which
 * the collector has reached. A block bigger than 2 KB gets a run of pages
 * of its own, a "big block".
 *
 * Three things follow, and they are why this is fast:
 *
 *   - Handing out a block is taking the first spare one of a page that
 *     has the right size: a few instructions, with no table to grow.
 *   - Asking "is this word the address of one of my blocks?" is one look
 *     in a hash table of pages (`zeno_units`), by the word's top bits,
 *     then a division. No sorting, no searching.
 *   - A block has no header of its own. Its page keeps one bit to say it
 *     is in use and one to say it was reached, so small blocks cost
 *     nothing extra.
 *
 * This is the way most collectors that look at memory without help from
 * the compiler are built (the Boehm collector, used by many languages,
 * is one). docs/memory.md walks through it. */
#define ZENO_PAGE_BITS 16
#define ZENO_PAGE ((size_t)1 << ZENO_PAGE_BITS)
#define ZENO_CLASSES 16
#define ZENO_BIGGEST_SMALL 2048
static const uint32_t zeno_class_size[ZENO_CLASSES] = {16, 32, 48, 64, 80, 96, 128, 160, 192, 256, 384, 512, 768, 1024, 1536, 2048};

typedef struct zeno_page {
    struct zeno_page *prev, *next; /* the list of every page */
    struct zeno_page *next_room;   /* the list of pages of this size and kind with a spare block */
    char *data;                    /* the first block */
    void *spare;                   /* blocks given back, chained through their first word */
    size_t big;                    /* a big block: the bytes in it. 0 for a page of small blocks */
    size_t units;                  /* how many 64 KB units it covers */
    uint32_t size;                 /* the size of each block */
    uint32_t count;                /* how many blocks fit */
    uint32_t cut;                  /* how many have ever been handed out; the rest were never touched */
    uint32_t used;                 /* how many are in use now */
    uint8_t kind;                  /* its size class, + ZENO_CLASSES times its sort */
    bool plain;                    /* its blocks hold text or numbers, never a pointer */
    bool rows;                     /* its blocks are the items of lists (see zeno_mark) */
    bool guarded;                  /* written to by nobody since the last collection (see "Generations") */
    bool listed;                   /* it is on its list of pages with room */
    uint64_t in_use[64];           /* one bit for each block */
    uint64_t reached[64];
} zeno_page;

/* Room for the header, and blocks start lined up for any C type. */
#define ZENO_PAGE_HEAD ((sizeof(zeno_page) + 15) / 16 * 16)

static zeno_page *zeno_pages = NULL;                  /* every page */
/* The three sorts of block: with pointers, plain (text and numbers), and
 * the rows of items of lists. Each sort has pages of its own. */
#define ZENO_POINTERS 0
#define ZENO_PLAIN 1
#define ZENO_ROWS 2
static zeno_page *zeno_room[3 * ZENO_CLASSES];         /* pages with a spare block, by kind */
static uint8_t zeno_class_of[ZENO_BIGGEST_SMALL / 16 + 1]; /* size / 16, rounded up, to its class */

/* How much memory the program holds (the sizes of the blocks in use),
 * and how much it may hold before the collector runs next. After each
 * run the limit is set to twice what survived, so a program that needs a
 * lot is not collected all the time. */
#ifndef ZENO_FIRST_COLLECTION
#define ZENO_FIRST_COLLECTION (4 * 1024 * 1024)
#endif
static size_t zeno_in_use = 0;
static size_t zeno_collect_at = ZENO_FIRST_COLLECTION;

/* Numbers for the report that `zeno build --memory` asks for. */
static size_t zeno_most_in_use = 0;
static size_t zeno_collections = 0;
static size_t zeno_blocks_made = 0;
static size_t zeno_bytes_made = 0;
static size_t zeno_blocks_freed_at_once = 0;
/* Blocks made by `allocate` and given back by `free`, inside `unsafe`.
 * They are not the collector's: it never looks at them or frees them. */
static size_t zeno_by_hand_made = 0;
static size_t zeno_by_hand_freed = 0;

/* Where the stack starts. `main` sets it before anything else runs. */
static void *zeno_stack_base = NULL;

/* The roots that are not on the stack: the top-level variables. The
 * compiler writes this function, at the end of the file, since only it
 * knows what they are. */
static void zeno_roots(void);

/* The lowest and highest addresses any page covers. Most words on the
 * stack are not addresses at all, and these two turn them away quickly. */
static uintptr_t zeno_lowest = UINTPTR_MAX;
static uintptr_t zeno_highest = 0;

/* ---- The table of pages: which page covers each 64 KB unit of memory.
 * Open addressing: a unit's place is worked out from its number, and if
 * that place is taken, the next free one is used. */
static uintptr_t *zeno_unit_keys = NULL; /* unit number, or 0 for an empty place */
static zeno_page **zeno_unit_pages = NULL;
static size_t zeno_unit_room = 0, zeno_unit_count = 0;
static int zeno_unit_bits = 0;

static size_t zeno_unit_place(uintptr_t key) {
    return (size_t)(((uint64_t)key * UINT64_C(11400714819323198485)) >> (64 - zeno_unit_bits));
}

static zeno_page *zeno_unit_find(uintptr_t key) {
    if (zeno_unit_room == 0) return NULL;
    for (size_t at = zeno_unit_place(key);; at = (at + 1) & (zeno_unit_room - 1)) {
        if (zeno_unit_keys[at] == key) return zeno_unit_pages[at];
        if (zeno_unit_keys[at] == 0) return NULL;
    }
}

static void zeno_unit_put(uintptr_t key, zeno_page *page);

/* Keep the table at most half full, so that a look rarely goes on past
 * the place it starts at. */
static void zeno_unit_grow(void) {
    uintptr_t *keys = zeno_unit_keys;
    zeno_page **pages = zeno_unit_pages;
    size_t room = zeno_unit_room;
    zeno_unit_bits = room == 0 ? 10 : zeno_unit_bits + 1;
    zeno_unit_room = (size_t)1 << zeno_unit_bits;
    zeno_unit_keys = calloc(zeno_unit_room, sizeof(uintptr_t));
    zeno_unit_pages = calloc(zeno_unit_room, sizeof(zeno_page *));
    if (zeno_unit_keys == NULL || zeno_unit_pages == NULL) zeno_out_of_memory();
    zeno_unit_count = 0;
    for (size_t i = 0; i < room; i++) {
        if (keys[i] != 0) zeno_unit_put(keys[i], pages[i]);
    }
    free(keys);
    free(pages);
}

static void zeno_unit_put(uintptr_t key, zeno_page *page) {
    if (2 * (zeno_unit_count + 1) > zeno_unit_room) zeno_unit_grow();
    size_t at = zeno_unit_place(key);
    while (zeno_unit_keys[at] != 0) at = (at + 1) & (zeno_unit_room - 1);
    zeno_unit_keys[at] = key;
    zeno_unit_pages[at] = page;
    zeno_unit_count++;
}

/* Take a unit out. The entries after it that were pushed along by it are
 * moved back, so that no look stops early at the hole. */
static void zeno_unit_take(uintptr_t key) {
    size_t at = zeno_unit_place(key);
    while (zeno_unit_keys[at] != key) at = (at + 1) & (zeno_unit_room - 1);
    zeno_unit_keys[at] = 0;
    zeno_unit_count--;
    for (size_t next = (at + 1) & (zeno_unit_room - 1); zeno_unit_keys[next] != 0; next = (next + 1) & (zeno_unit_room - 1)) {
        uintptr_t moved = zeno_unit_keys[next];
        size_t home = zeno_unit_place(moved);
        /* Can the entry at `next` sit at `at`? Only if its home is not
         * between `at` (exclusive) and `next` (inclusive), going round. */
        bool between = at <= next ? (home > at && home <= next) : (home > at || home <= next);
        if (!between) {
            zeno_unit_keys[at] = moved;
            zeno_unit_pages[at] = zeno_unit_pages[next];
            zeno_unit_keys[next] = 0;
            at = next;
        }
    }
}

/* ---- Pages from the system, and back. */
static zeno_page *zeno_page_new(size_t units) {
    size_t bytes = units * ZENO_PAGE;
    void *memory;
#ifdef _WIN32
    memory = _aligned_malloc(bytes, ZENO_PAGE);
#else
    if (posix_memalign(&memory, ZENO_PAGE, bytes) != 0) memory = NULL;
#endif
    if (memory == NULL) return NULL;
    zeno_page *page = memory;
    memset(page, 0, sizeof(zeno_page));
    page->units = units;
    page->data = (char *)page + ZENO_PAGE_HEAD;
    page->next = zeno_pages;
    if (zeno_pages != NULL) zeno_pages->prev = page;
    zeno_pages = page;
    uintptr_t first = (uintptr_t)page >> ZENO_PAGE_BITS;
    for (size_t u = 0; u < units; u++) zeno_unit_put(first + u, page);
    if ((uintptr_t)page < zeno_lowest) zeno_lowest = (uintptr_t)page;
    if ((uintptr_t)page + bytes > zeno_highest) zeno_highest = (uintptr_t)page + bytes;
    return page;
}

static void zeno_unguard(zeno_page *page);

static void zeno_page_free(zeno_page *page) {
    zeno_unguard(page);
    uintptr_t first = (uintptr_t)page >> ZENO_PAGE_BITS;
    for (size_t u = 0; u < page->units; u++) zeno_unit_take(first + u);
    if (page->prev != NULL) page->prev->next = page->next;
    else zeno_pages = page->next;
    if (page->next != NULL) page->next->prev = page->prev;
#ifdef _WIN32
    _aligned_free(page);
#else
    free(page);
#endif
}

/* ---- Marking. A stack of blocks that were reached but whose insides
 * have not been looked at yet. */
typedef struct {
    const char *start;
    size_t size;
} zeno_to_look;
static zeno_to_look *zeno_marking = NULL;
static size_t zeno_marking_count = 0, zeno_marking_room = 0;

static void zeno_reach(zeno_page *page, size_t i) {
    uint64_t bit = (uint64_t)1 << (i & 63);
    if (!(page->in_use[i >> 6] & bit) || (page->reached[i >> 6] & bit)) return;
    page->reached[i >> 6] |= bit;
    if (page->plain) return;
    if (zeno_marking_count == zeno_marking_room) {
        zeno_marking_room = zeno_marking_room == 0 ? 1024 : zeno_marking_room * 2;
        zeno_marking = realloc(zeno_marking, zeno_marking_room * sizeof(zeno_to_look));
        if (zeno_marking == NULL) zeno_out_of_memory();
    }
    size_t size = page->big != 0 ? page->big : page->size;
    zeno_marking[zeno_marking_count++] = (zeno_to_look){page->data + i * page->size, size};
}

/* `word` was found in a variable or inside a block. If it points at, or
 * into, a block, mark that block.
 *
 * A row of the items of a list is the one block that compiled code may
 * be left holding only the end of: a loop over a list can keep a pointer
 * just past the last item and nothing else. So in a page of rows, a word
 * that points just past the end of a block marks that block too. (In any
 * other page it would be wrong to: the same word is the start of the
 * next block, and every block reached that way would keep the one before
 * it alive, and that one the one before it.) A row's page, like every
 * page, leaves a few bytes free at its end, so that address is always in
 * the same page. */
static void zeno_mark(uintptr_t word) {
    if (word < zeno_lowest || word >= zeno_highest) return;
    zeno_page *page = zeno_unit_find(word >> ZENO_PAGE_BITS);
    if (page == NULL || word < (uintptr_t)page->data) return;
    size_t offset = (size_t)(word - (uintptr_t)page->data);
    if (page->big != 0) {
        if (offset <= page->big) zeno_reach(page, 0);
        return;
    }
    size_t i = offset / page->size;
    if (page->rows && offset % page->size == 0 && i > 0) zeno_reach(page, i - 1);
    if (i < page->cut) zeno_reach(page, i);
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

/* Put a page on the list of pages with room for its size, if it has room
 * and is not on it already. */
static void zeno_page_has_room(zeno_page *page) {
    if (page->listed || page->big != 0) return;
    if (page->spare == NULL && page->cut == page->count) return;
    page->listed = true;
    page->next_room = zeno_room[page->kind];
    zeno_room[page->kind] = page;
}

/* ---- Generations.
 *
 * Most blocks are garbage soon after they are made, and a block that has
 * lived through one collection usually lives through many. So most
 * collections are small: they look only at the blocks made since the
 * last one (the young), and take every block that lived through a
 * collection (the old) to be still in use. Once the old have grown to
 * twice what lived through the last full collection, the next one is
 * full again, and looks at everything.
 *
 * A block is old when its `reached` bit is still set from the collection
 * it lived through: a small collection leaves those bits as they are, and
 * stops at an old block instead of looking inside it again.
 *
 * The danger is an old block that has been given a pointer to a young one
 * since: nobody would look inside it, and the young block would be freed
 * while still in use. So after each collection, every full page of blocks
 * with pointers in them is made read-only, with the system's `mprotect`.
 * The first write to it stops the program for a moment; the handler below
 * makes the page writable again and notes that it is no longer guarded.
 * A small collection looks inside the old blocks of every page that is not
 * guarded (pages with room, which new blocks go into, and pages written
 * to), so nothing is missed: the processor itself keeps watch, and no line
 * of compiled code has to remember to say it wrote a pointer.
 *
 * Pages of text and numbers are never guarded: they hold no pointers, and
 * a file read straight into one by the system must not find it read-only.
 * On a chip (`--firmware`), or where `mprotect` is not to be had, every
 * collection is full, as it was before generations. */
#ifndef ZENO_FIRMWARE
#if !defined(ZENO_BOARD) && !defined(ZENO_ONE_GENERATION) && (defined(__linux__) || defined(__APPLE__))
#define ZENO_GENERATIONS 1
#include <signal.h>
#include <sys/mman.h>
#endif
#endif /* ZENO_FIRMWARE */

static size_t zeno_major_base = 0; /* what lived through the last full collection */
static size_t zeno_after_last = 0; /* what lived through the last collection */
static size_t zeno_small_collections = 0;

#ifdef ZENO_GENERATIONS
static bool zeno_watching = false;
static struct sigaction zeno_old_segv, zeno_old_bus;

static void zeno_unguard(zeno_page *page) {
    if (!page->guarded) return;
    mprotect(page, page->units * ZENO_PAGE, PROT_READ | PROT_WRITE);
    page->guarded = false;
}

static void zeno_guard(zeno_page *page) {
    page->guarded = true;
    if (mprotect(page, page->units * ZENO_PAGE, PROT_READ) != 0) page->guarded = false;
}

/* A write to a read-only page. If the page is one of the collector's,
 * open it and carry on with the write. If not, it is a real mistake: put
 * back what the system had, and the write fails again, as it would have. */
static void zeno_fault(int sig, siginfo_t *info, void *context) {
    (void)context;
    uintptr_t at = (uintptr_t)info->si_addr;
    zeno_page *page = at >= zeno_lowest && at < zeno_highest ? zeno_unit_find(at >> ZENO_PAGE_BITS) : NULL;
    if (page != NULL && page->guarded) {
        zeno_unguard(page);
        return;
    }
    sigaction(sig, sig == SIGSEGV ? &zeno_old_segv : &zeno_old_bus, NULL);
}

static void zeno_watch(void) {
    struct sigaction act;
    memset(&act, 0, sizeof act);
    act.sa_sigaction = zeno_fault;
    act.sa_flags = SA_SIGINFO;
    sigemptyset(&act.sa_mask);
    sigaction(SIGSEGV, &act, &zeno_old_segv);
    sigaction(SIGBUS, &act, &zeno_old_bus);
    zeno_watching = true;
}
#else
static void zeno_unguard(zeno_page *page) {
    (void)page;
}
#endif

/* Look inside every old block of a page: a small collection takes what
 * they point at to be in use. */
static void zeno_mark_old(zeno_page *page) {
    if (page->big != 0) {
        if (page->reached[0] & 1) zeno_mark_words(page->data, page->big);
        return;
    }
    for (uint32_t w = 0; w * 64 < page->cut; w++) {
        uint64_t old = page->in_use[w] & page->reached[w];
        while (old != 0) {
            int b = __builtin_ctzll(old);
            old &= old - 1;
            zeno_mark_words(page->data + ((size_t)w * 64 + (size_t)b) * page->size, page->size);
        }
    }
}

static void zeno_collect(void) {
    if (zeno_pages == NULL || zeno_stack_base == NULL) return;
    zeno_collections++;
    bool full = true;
#ifdef ZENO_GENERATIONS
    if (!zeno_watching) zeno_watch();
#ifdef ZENO_COLLECT_ALWAYS
    full = zeno_collections % 64 == 1; /* testing: mostly small ones, which are the harder */
#else
    full = zeno_major_base == 0 || zeno_after_last > 2 * zeno_major_base;
#endif
#endif
    if (full) {
        /* Everything is looked at afresh: no block is old. */
        for (zeno_page *page = zeno_pages; page != NULL; page = page->next) {
            zeno_unguard(page);
            memset(page->reached, 0, sizeof page->reached);
        }
    } else {
        zeno_small_collections++;
    }

    /* Mark. A pointer may be nowhere in memory at this moment, only in one
     * of the processor's registers. This copies the registers onto the
     * stack, where zeno_mark_stack will see them. (A builtin of GCC and
     * Clang, like the ones the checked arithmetic uses further down.) */
    __builtin_unwind_init();
    zeno_mark_stack();
    zeno_roots();
    if (!full) {
        for (zeno_page *page = zeno_pages; page != NULL; page = page->next) {
            if (!page->guarded && !page->plain) zeno_mark_old(page);
        }
    }
    /* Then follow the pointers inside each marked block, which marks more
     * blocks, until none are waiting. */
    while (zeno_marking_count > 0) {
        zeno_to_look next = zeno_marking[--zeno_marking_count];
        zeno_mark_words(next.start, next.size);
    }

    /* Sweep: every block in use that was not reached goes on its page's
     * list of spare blocks. A page left with nothing in use goes back to
     * the system. Then the lists of pages with room are made afresh. The
     * `reached` bits stay: they are what makes a block old. A guarded page
     * has only old blocks, and nothing to sweep. */
    for (int k = 0; k < 3 * ZENO_CLASSES; k++) zeno_room[k] = NULL;
    zeno_page *page = zeno_pages;
    while (page != NULL) {
        zeno_page *next = page->next;
        if (page->guarded) {
            page = next;
            continue;
        }
        page->listed = false;
        if (page->big != 0) {
            if (!(page->reached[0] & 1)) {
                zeno_in_use -= page->big;
                zeno_page_free(page);
            }
            page = next;
            continue;
        }
        for (uint32_t w = 0; w * 64 < page->cut; w++) {
            uint64_t dead = page->in_use[w] & ~page->reached[w];
            if (dead == 0) continue;
            page->in_use[w] &= page->reached[w];
            while (dead != 0) {
                int b = __builtin_ctzll(dead);
                dead &= dead - 1;
                void **block = (void **)(page->data + ((size_t)w * 64 + (size_t)b) * page->size);
                *block = page->spare;
                page->spare = block;
                page->used--;
                zeno_in_use -= page->size;
            }
        }
        if (page->used == 0) zeno_page_free(page);
        else zeno_page_has_room(page);
        page = next;
    }
#ifdef ZENO_GENERATIONS
    /* Watch the full pages of pointers for writes until the next one. */
    for (page = zeno_pages; page != NULL; page = page->next) {
        bool full_page = page->big != 0 || (page->spare == NULL && page->cut == page->count);
        if (!page->guarded && !page->plain && full_page) zeno_guard(page);
    }
#endif
    if (full) zeno_major_base = zeno_in_use;
    zeno_after_last = zeno_in_use;
    zeno_collect_at = zeno_in_use * 2 > ZENO_FIRST_COLLECTION ? zeno_in_use * 2 : ZENO_FIRST_COLLECTION;
}

static void zeno_counted(size_t size) {
    zeno_blocks_made++;
    zeno_bytes_made += size;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
}

/* A block bigger than any size class: a run of pages of its own. There
 * is always at least one byte to spare at its end (see zeno_mark). */
static char *zeno_new_big(size_t size, int sort) {
    bool plain = sort == ZENO_PLAIN;
    size_t units = (ZENO_PAGE_HEAD + size + 1 + ZENO_PAGE - 1) / ZENO_PAGE;
    zeno_page *page = zeno_page_new(units);
    if (page == NULL) {
        zeno_collect();
        page = zeno_page_new(units);
        if (page == NULL) zeno_out_of_memory();
    }
    page->big = size;
    page->size = 0;
    page->count = page->cut = page->used = 1;
    page->plain = plain;
    page->in_use[0] = 1;
    zeno_in_use += size;
    zeno_counted(size);
    if (!plain) memset(page->data, 0, size);
    return page->data;
}

static char *zeno_new_block(size_t size, int sort) {
    bool plain = sort == ZENO_PLAIN;
#ifdef ZENO_COLLECT_ALWAYS
    zeno_collect(); /* for testing the collector: as hard on it as can be */
#else
    if (zeno_in_use + size > zeno_collect_at) zeno_collect();
#endif
    if (size == 0) size = 1;
    if (size > ZENO_BIGGEST_SMALL) return zeno_new_big(size, sort);
    static bool classes_known = false;
    if (!classes_known) {
        for (size_t s = 0, k = 0; s <= ZENO_BIGGEST_SMALL / 16; s++) {
            while (zeno_class_size[k] < s * 16) k++;
            zeno_class_of[s] = (uint8_t)k;
        }
        classes_known = true;
    }
    int cls = zeno_class_of[(size + 15) / 16];
    int kind = cls + sort * ZENO_CLASSES;
    zeno_page *page = zeno_room[kind];
    while (page != NULL && page->spare == NULL && page->cut == page->count) {
        page->listed = false; /* full: off the list until something in it is freed */
        page = zeno_room[kind] = page->next_room;
    }
    if (page == NULL) {
        page = zeno_page_new(1);
        if (page == NULL) {
            zeno_collect();
            page = zeno_page_new(1);
            if (page == NULL) zeno_out_of_memory();
        }
        page->size = zeno_class_size[cls];
        /* One byte less than fits, so that the end of the last block is
         * still inside the page. */
        page->count = (uint32_t)((ZENO_PAGE - ZENO_PAGE_HEAD - 1) / page->size);
        page->kind = (uint8_t)kind;
        page->plain = plain;
        page->rows = sort == ZENO_ROWS;
        zeno_page_has_room(page);
    }
    char *block;
    size_t i;
    if (page->spare != NULL) {
        block = page->spare;
        page->spare = *(void **)block;
        i = (size_t)(block - page->data) / page->size;
    } else {
        i = page->cut++;
        block = page->data + i * page->size;
    }
    page->in_use[i >> 6] |= (uint64_t)1 << (i & 63);
    page->reached[i >> 6] &= ~((uint64_t)1 << (i & 63)); /* young */
    page->used++;
    zeno_in_use += page->size;
    zeno_counted(size);
    /* A block that can hold pointers starts as all zeros, so that what an
     * earlier owner left behind is not taken for a pointer. */
    if (!plain) memset(block, 0, page->size);
    return block;
}

/* Memory for a list, an object or a function value: things with pointers
 * to other things inside. */
static char *zeno_alloc(size_t size) {
    return zeno_new_block(size, ZENO_POINTERS);
}

/* Memory for the items of a list. */
static char *zeno_alloc_rows(size_t size) {
    return zeno_new_block(size, ZENO_ROWS);
}

/* Memory for text, or for one number. The collector need not look inside. */
static char *zeno_alloc_text(size_t size) {
    return zeno_new_block(size, ZENO_PLAIN);
}

/* Give one block back at once, without waiting for the collector. This is
 * what the end of an owner's block does to an owned value (see
 * docs/ownership.md): the checker has made sure nothing else can still use
 * it. `memory` is what zeno_alloc returned, or NULL, which does nothing.
 *
 * Its page is found from its address alone: the block starts in the first
 * 64 KB of its page, so cutting the address down to a multiple of 64 KB
 * gives the page. If a block is never freed this way (an error jumped
 * past the end of its owner's block, say), nothing is lost: the collector
 * frees it when it finds nothing pointing at it, like any other block. */
static void zeno_free(void *memory) {
    if (memory == NULL) return;
    zeno_page *page = (zeno_page *)((uintptr_t)memory & ~(uintptr_t)(ZENO_PAGE - 1));
    zeno_blocks_freed_at_once++;
    if (page->big != 0) {
        zeno_in_use -= page->big;
        zeno_page_free(page);
        return;
    }
    size_t i = (size_t)((char *)memory - page->data) / page->size;
    page->in_use[i >> 6] &= ~((uint64_t)1 << (i & 63));
    page->reached[i >> 6] &= ~((uint64_t)1 << (i & 63));
    *(void **)memory = page->spare;
    page->spare = memory;
    page->used--;
    zeno_in_use -= page->size;
    zeno_page_has_room(page);
}

/* What a program built with `zeno build --memory` prints as it ends. */
#ifdef ZENO_MEMORY_REPORT
static void zeno_memory_report(void) {
    fflush(stdout);
    /* "1 block", "2 blocks": the report is read by people. */
    fprintf(stderr, "memory: %zu block%s made (%zu byte%s), at most %zu byte%s in use at once, %zu collection%s",
            zeno_blocks_made, zeno_blocks_made == 1 ? "" : "s", zeno_bytes_made, zeno_bytes_made == 1 ? "" : "s",
            zeno_most_in_use, zeno_most_in_use == 1 ? "" : "s", zeno_collections, zeno_collections == 1 ? "" : "s");
    if (zeno_blocks_freed_at_once > 0) {
        fprintf(stderr, ", %zu block%s freed without the collector", zeno_blocks_freed_at_once,
                zeno_blocks_freed_at_once == 1 ? "" : "s");
    }
    fprintf(stderr, "\n");
    if (zeno_by_hand_made > 0) {
        fprintf(stderr, "by hand: %zu block%s from `allocate`, %zu never freed\n", zeno_by_hand_made,
                zeno_by_hand_made == 1 ? "" : "s", zeno_by_hand_made - zeno_by_hand_freed);
    }
}
#endif

#else /* ZENO_NO_COLLECTOR */

/* A build with no collector: `zeno build --no-collector`, and
 * docs/no-collector.md.
 *
 * Nothing here looks for memory that is no longer used, because the
 * compiler has made sure there is none to find. Memory comes from two
 * places, and each gives it back at a line that can be pointed to:
 *
 *   1. A list or an object has one owner. The line that ends its owner's
 *      block hands it back. Both come from the pool below: one array of a
 *      fixed size. The C library's heap is not used at all.
 *   2. Text built while the program runs lasts until the statement that
 *      built it is over. It is written into one array of a fixed size, the
 *      text room, and giving it back is setting one number.
 */

/* Numbers for the report that `zeno build --memory` asks for. */
#ifdef ZENO_MEMORY_REPORT
static size_t zeno_in_use = 0;
static size_t zeno_most_in_use = 0;
static size_t zeno_blocks_made = 0;
static size_t zeno_bytes_made = 0;
static size_t zeno_blocks_held = 0;
static size_t zeno_most_text = 0;
#endif

/* The pool: where lists, objects and errors live.
 *
 * It is one array, ZENO_POOL bytes long, that is part of the program
 * from the start. So how much memory the program can ever use is known
 * when it is built, and no line of it can fail to get memory from the
 * system, because it never asks.
 *
 * Blocks come in fixed sizes, each twice the one before: 16 bytes, 32,
 * 64 and so on. A request gets the smallest size it fits in.
 *
 *     taking    Is there a spare block of that size? Take it. If not,
 *               cut a new one from the part of the array never used yet
 *               (everything above `zeno_pool_top`).
 *     giving    The block goes on the list of spare blocks of its size.
 *
 * Both are a few instructions, however full the pool is. A block is never
 * cut in two and two blocks are never joined, so the pool cannot break up
 * into pieces too small to use: a size the program has given back can
 * always be taken again. What that costs is that memory cut for one size
 * is never used for another. docs/pool.md goes through all of it.
 *
 * In front of each block are eight bytes that say which size it is and
 * how much of it was asked for. */
#ifndef ZENO_POOL
#ifdef ZENO_HEAP
#define ZENO_POOL (1024 * 1024) /* the first piece; each next one is twice as big */
#else /* ZENO_HEAP */
#define ZENO_POOL 32768
#endif /* ZENO_HEAP */
#endif

/* How many sizes there are. The largest holds 16 << 23 bytes, which is
 * more than a pool is likely to be. */
#ifndef ZENO_POOL_STEPS
#ifdef ZENO_HEAP
#define ZENO_POOL_STEPS 40
#else /* ZENO_HEAP */
#define ZENO_POOL_STEPS 24
#endif /* ZENO_HEAP */
#endif

typedef struct {
    uint32_t step;  /* the block holds 16 << step bytes */
    uint32_t asked; /* how many of them are in use; 0 while the block is spare */
} zeno_head;

/* A spare block. Its first bytes say where the next spare one of its size
 * is. (The smallest block is 16 bytes so that this always fits.) */
typedef struct zeno_spare {
    zeno_head head;
    struct zeno_spare *next;
} zeno_spare;

#ifdef ZENO_HEAP
/* A program built for a computer rather than a chip (`memory owned` in
 * `zeno.package`, or `zeno build --owned`) cannot know in advance how much
 * it will need. Its pool grows: when the piece it is cutting from runs
 * out, it asks the system for another, twice as big as the last, so a
 * program that needs a lot asks only a few dozen times in all. Blocks,
 * sizes and spare lists work exactly as in the fixed pool. */
#define ZENO_PIECES 48
static struct { char *bytes; size_t size; } zeno_pieces[ZENO_PIECES];
static int zeno_piece_count = 0;
static size_t zeno_pool_top = 0;      /* how much of the newest piece is cut */
static size_t zeno_pool_taken = 0;    /* bytes asked of the system, in all */
static zeno_spare *zeno_pool_spare[ZENO_POOL_STEPS];

static void zeno_pool_full(void) {
    fflush(stdout);
    fprintf(stderr, "error: out of memory\n");
    exit(1);
}

/* Room for `whole` more bytes, from the newest piece or a new one. What
 * was left at the end of the old piece is not used again: at most half
 * of a piece, and usually far less. */
static char *zeno_pool_cut(size_t whole) {
    if (zeno_piece_count == 0 || whole > zeno_pieces[zeno_piece_count - 1].size - zeno_pool_top) {
        if (zeno_piece_count == ZENO_PIECES) zeno_pool_full();
        size_t size = zeno_piece_count == 0 ? (size_t)ZENO_POOL : zeno_pieces[zeno_piece_count - 1].size * 2;
        while (size < whole) size *= 2;
        char *bytes = malloc(size);
        if (bytes == NULL) zeno_pool_full();
        zeno_pieces[zeno_piece_count].bytes = bytes;
        zeno_pieces[zeno_piece_count].size = size;
        zeno_piece_count++;
        zeno_pool_top = 0;
        zeno_pool_taken += size;
    }
    char *at = zeno_pieces[zeno_piece_count - 1].bytes + zeno_pool_top;
    zeno_pool_top += whole;
    return at;
}

/* Memory for a list, an object, an error or kept text, as it was left.
 * zeno_alloc, below, clears it. */
static char *zeno_alloc_raw(size_t size) {
    if (size == 0) size = 1;
    uint32_t step = 0;
    size_t room = 16;
    while (room < size) {
        room *= 2;
        step++;
        if (step == ZENO_POOL_STEPS) zeno_pool_full();
    }
    zeno_spare *block = zeno_pool_spare[step];
    if (block != NULL) {
        zeno_pool_spare[step] = block->next;
    } else {
        block = (zeno_spare *)zeno_pool_cut(sizeof(zeno_head) + room);
        block->head.step = step;
    }
    block->head.asked = (uint32_t)size;
    char *memory = (char *)block + sizeof(zeno_head);
#ifdef ZENO_MEMORY_REPORT
    zeno_blocks_made++;
    zeno_blocks_held++;
    zeno_bytes_made += size;
    zeno_in_use += size;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
#endif
    return memory;
}
#else /* ZENO_HEAP */
/* The array. The second member is never used: it is there so that the C
 * compiler lines the array up for an int or a float. */
static union {
    char bytes[ZENO_POOL];
    uint64_t lined_up;
} zeno_pool;
static size_t zeno_pool_top = 0;
static zeno_spare *zeno_pool_spare[ZENO_POOL_STEPS];

static void zeno_pool_full(void) {
    fflush(stdout);
    fprintf(stderr, "error: the memory pool is full (ZENO_POOL is %d bytes)\n", (int)ZENO_POOL);
    exit(1);
}

/* Memory for a list, an object, an error or kept text, as it was left.
 * zeno_alloc, below, clears it. */
static char *zeno_alloc_raw(size_t size) {
    if (size == 0) size = 1;
    uint32_t step = 0;
    size_t room = 16;
    while (room < size) {
        room *= 2;
        step++;
        if (step == ZENO_POOL_STEPS) zeno_pool_full();
    }
    zeno_spare *block = zeno_pool_spare[step];
    if (block != NULL) {
        zeno_pool_spare[step] = block->next;
    } else {
        size_t whole = sizeof(zeno_head) + room;
        if (whole > (size_t)ZENO_POOL - zeno_pool_top) zeno_pool_full();
        block = (zeno_spare *)(zeno_pool.bytes + zeno_pool_top);
        zeno_pool_top += whole;
        block->head.step = step;
    }
    block->head.asked = (uint32_t)size;
    char *memory = (char *)block + sizeof(zeno_head);
#ifdef ZENO_MEMORY_REPORT
    zeno_blocks_made++;
    zeno_blocks_held++;
    zeno_bytes_made += size;
    zeno_in_use += size;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
#endif
    return memory;
}

#endif /* ZENO_HEAP */

/* Memory for a list, an object or an error. It starts as all zeros. */
static char *zeno_alloc(size_t size) {
    char *memory = zeno_alloc_raw(size);
    memset(memory, 0, (size_t)16 << ((zeno_head *)memory - 1)->step); /* the whole block */
    return memory;
}

#ifdef ZENO_HEAP
/* The rows of a big list come from the system, not the pool, so that
 * `realloc` can grow them where they lie, or move them without copying,
 * as C's own growing arrays do (decision 23.3). In front of the rows is
 * their size, then a head like a pool block's, whose step says where
 * they came from. */
#define ZENO_FROM_SYSTEM UINT32_MAX
#define ZENO_BIG_ROWS ((size_t)1 << 16)

static void zeno_rows_free(zeno_spare *block) {
    size_t *start = (size_t *)block - 1;
#ifdef ZENO_MEMORY_REPORT
    zeno_in_use -= start[0];
    zeno_blocks_held--;
#endif
    block->head.asked = 0;
    free(start);
}
#endif /* ZENO_HEAP */

/* Give a block back. `memory` is what zeno_alloc returned, or NULL, which
 * does nothing. The compiler writes one of these for each block, at the
 * line where its owner ends. If a block ever came here twice, the list of
 * spare blocks would hand it to two owners, so that is looked for. */
static void zeno_free(void *memory) {
    if (memory == NULL) return;
    zeno_spare *block = (zeno_spare *)((char *)memory - sizeof(zeno_head));
#ifndef ZENO_MINIMAL
    if (block->head.asked == 0) {
        fflush(stdout);
        fprintf(stderr, "error: a block of memory was given back twice\n");
        exit(1);
    }
#endif /* ZENO_MINIMAL */
#ifdef ZENO_HEAP
    if (block->head.step == ZENO_FROM_SYSTEM) {
        zeno_rows_free(block);
        return;
    }
#endif /* ZENO_HEAP */
#ifdef ZENO_MEMORY_REPORT
    zeno_in_use -= block->head.asked;
    zeno_blocks_held--;
#endif
    block->head.asked = 0;
    block->next = zeno_pool_spare[block->head.step];
    zeno_pool_spare[block->head.step] = block;
}

/* The text room. Text is written at `zeno_text_top`, which then moves up.
 * The compiled program notes where the top was before a statement that
 * builds text, and puts it back there when the statement is over:
 *
 *     size_t m1 = zeno_text_top;
 *     zeno_print(1, zeno_join(2, "count: ", zeno_str_int(z_count)));
 *     zeno_text_top = m1;
 *
 * A function called in the middle does the same with its own statements,
 * above this one's text, so the room is used like a stack. Its size is the
 * most text that may be in the making at one moment. */
#ifdef ZENO_HEAP
/* With a growing pool the text room grows too. It is a chain of rooms,
 * each twice as big as the one before, and `zeno_text_top` counts bytes
 * as if they stood end to end: room 0 holds 0 up to its size, room 1 the
 * next bytes, and so on. The compiled program only ever saves the top and
 * puts it back, so it cannot tell one room from many. A text never spans
 * two rooms: one that does not fit at the end of a room starts the next. */
#ifndef ZENO_TEXT_ROOM
#define ZENO_TEXT_ROOM (64 * 1024) /* the first room */
#endif
#define ZENO_ROOMS 40
static struct { char *bytes; size_t start, size; } zeno_rooms[ZENO_ROOMS];
static int zeno_room_count = 0;
static size_t zeno_text_top = 0;

/* The room `top` is in, or the one it is the end of. */
static int zeno_room_of(size_t top) {
    for (int k = 0; k < zeno_room_count; k++) {
        if (top < zeno_rooms[k].start + zeno_rooms[k].size) return k;
    }
    return zeno_room_count - 1;
}

static char *zeno_alloc_text(size_t size) {
    int k = zeno_room_count == 0 ? -1 : zeno_room_of(zeno_text_top);
    while (k < 0 || zeno_text_top + size > zeno_rooms[k].start + zeno_rooms[k].size) {
        k++;
        if (k == zeno_room_count) {
            if (k == ZENO_ROOMS) zeno_pool_full();
            size_t room = k == 0 ? (size_t)ZENO_TEXT_ROOM : zeno_rooms[k - 1].size * 2;
            while (room < size) room *= 2;
            zeno_rooms[k].bytes = malloc(room);
            if (zeno_rooms[k].bytes == NULL) zeno_pool_full();
            zeno_rooms[k].start = k == 0 ? 0 : zeno_rooms[k - 1].start + zeno_rooms[k - 1].size;
            zeno_rooms[k].size = room;
            zeno_room_count++;
        }
        zeno_text_top = zeno_rooms[k].start;
    }
    char *text = zeno_rooms[k].bytes + (zeno_text_top - zeno_rooms[k].start);
    zeno_text_top += size;
#ifdef ZENO_MEMORY_REPORT
    if (zeno_text_top > zeno_most_text) zeno_most_text = zeno_text_top;
#endif
    return text;
}

/* Where in the chain `text` is, counted as `zeno_text_top` counts. */
static size_t zeno_text_place(const char *text) {
    for (int k = 0; k < zeno_room_count; k++) {
        if ((uintptr_t)text - (uintptr_t)zeno_rooms[k].bytes < (uintptr_t)zeno_rooms[k].size) {
            return zeno_rooms[k].start + (size_t)(text - zeno_rooms[k].bytes);
        }
    }
    return 0;
}
#else /* ZENO_HEAP */
#ifndef ZENO_TEXT_ROOM
#define ZENO_TEXT_ROOM 1024
#endif
static char zeno_text_room[ZENO_TEXT_ROOM];
static size_t zeno_text_top = 0;

static char *zeno_alloc_text(size_t size) {
    if (size > (size_t)ZENO_TEXT_ROOM - zeno_text_top) {
        fflush(stdout);
        fprintf(stderr, "error: a statement built more text than there is room for (ZENO_TEXT_ROOM is %d bytes)\n",
                (int)ZENO_TEXT_ROOM);
        exit(1);
    }
    char *text = zeno_text_room + zeno_text_top;
    zeno_text_top += size;
#ifdef ZENO_MEMORY_REPORT
    if (zeno_text_top > zeno_most_text) zeno_most_text = zeno_text_top;
#endif
    return text;
}

/* Where in the room `text` is, counted as `zeno_text_top` counts. */
static size_t zeno_text_place(const char *text) {
    return (size_t)(text - zeno_text_room);
}
#endif /* ZENO_HEAP */

/* Text that is kept.
 *
 * Text can be in three places, and its address says which:
 *
 *     written in the program    in neither array. It is there for ever.
 *     the text room             built by the statement now running, or a
 *                               function's answer. Gone when that
 *                               statement is over.
 *     the pool                  kept: a variable, a field or a list holds
 *                               it, and frees it when it is done with it.
 *
 * So a str is never given away and never shared. Whatever keeps text has
 * its own copy, and text is never changed once made, so nobody can tell
 * a copy from the original. docs/text.md goes through it. */
#ifdef ZENO_HEAP
static bool zeno_in_pool(const char *text) {
    for (int k = zeno_piece_count - 1; k >= 0; k--) {
        if ((uintptr_t)text - (uintptr_t)zeno_pieces[k].bytes < (uintptr_t)zeno_pieces[k].size) return true;
    }
    return false;
}

static bool zeno_in_room(const char *text) {
    for (int k = zeno_room_count - 1; k >= 0; k--) {
        if ((uintptr_t)text - (uintptr_t)zeno_rooms[k].bytes < (uintptr_t)zeno_rooms[k].size) return true;
    }
    return false;
}
#else /* ZENO_HEAP */
static bool zeno_in_pool(const char *text) {
    return (uintptr_t)text - (uintptr_t)zeno_pool.bytes < (uintptr_t)ZENO_POOL;
}

static bool zeno_in_room(const char *text) {
    return (uintptr_t)text - (uintptr_t)zeno_text_room < (uintptr_t)ZENO_TEXT_ROOM;
}
#endif /* ZENO_HEAP */

/* `text` is about to be kept in a variable, a field or a list. Text
 * written in the program can be kept as it is. Anything else will not
 * last, or is somebody else's, so the keeper gets a copy of its own. */
static const char *zeno_text_keep(const char *text) {
    if (text == NULL || !(zeno_in_pool(text) || zeno_in_room(text))) return text;
    size_t size = strlen(text) + 1;
    char *kept = zeno_alloc_raw(size); /* every byte is written next */
    memcpy(kept, text, size);
    return kept;
}

/* Whatever kept `text` is done with it. Only a copy made by zeno_text_keep
 * is in the pool; for text written in the program there is nothing to do. */
static void zeno_text_drop(const char *text) {
    if (text != NULL && zeno_in_pool(text)) zeno_free((void *)text);
}

/* `text` was read from a place that a function may change: a variable at
 * the top of the file, a field, an item of a list. If it is a kept copy,
 * the statement gets a copy of its own in the text room. Then a function
 * called later in the statement can give the place new text, which frees
 * the old, and the statement is still holding good text. */
static const char *zeno_text_lend(const char *text) {
    if (text == NULL || !zeno_in_pool(text)) return text;
    size_t size = strlen(text) + 1;
    char *copy = zeno_alloc_text(size);
    memcpy(copy, text, size);
    return copy;
}

/* The same, for text that is leaving the place that kept it for good
 * (`names.pop()`): the statement gets it, and the kept copy is freed. */
static const char *zeno_text_take(const char *text) {
    const char *lent = zeno_text_lend(text);
    zeno_text_drop(text);
    return lent;
}

/* `return text`. `mark` is where the top of the text room was when the
 * `return` began. A function's answer travels in the text room: it is
 * moved down to `mark`, and the top is left just above it, so that the
 * statement that called the function finds it there and gives it back
 * with the rest of its text. Text that will outlast the call anyway
 * (written in the program, or the caller's own, lower in the room) is
 * given back as it is. */
static const char *zeno_text_return(size_t mark, const char *text) {
    bool above = text != NULL && zeno_in_room(text) && zeno_text_place(text) >= mark;
    if (text == NULL || !(above || zeno_in_pool(text))) {
        zeno_text_top = mark;
        return text;
    }
    size_t size = strlen(text) + 1;
    zeno_text_top = mark;
    char *answer = zeno_alloc_text(size);
    memmove(answer, text, size);
    return answer;
}

/* What a program built with `zeno build --memory` prints as it ends. A
 * program that ran to its last line should have no blocks left. */
#ifdef ZENO_MEMORY_REPORT
static void zeno_memory_report(void) {
    fflush(stdout);
    /* "1 block", "2 blocks": the report is read by people. */
    fprintf(stderr, "memory: %zu block%s made (%zu byte%s), at most %zu byte%s in use at once, no collector, %zu block%s left at the end\n",
            zeno_blocks_made, zeno_blocks_made == 1 ? "" : "s", zeno_bytes_made, zeno_bytes_made == 1 ? "" : "s",
            zeno_most_in_use, zeno_most_in_use == 1 ? "" : "s", zeno_blocks_held, zeno_blocks_held == 1 ? "" : "s");
#ifdef ZENO_HEAP
    fprintf(stderr, "pool: %zu byte%s taken from the system, in %d piece%s\n", zeno_pool_taken, zeno_pool_taken == 1 ? "" : "s",
            zeno_piece_count, zeno_piece_count == 1 ? "" : "s");
    fprintf(stderr, "text: at most %zu byte%s in use at once, in %d room%s\n", zeno_most_text,
            zeno_most_text == 1 ? "" : "s", zeno_room_count, zeno_room_count == 1 ? "" : "s");
#else /* ZENO_HEAP */
    fprintf(stderr, "pool: %zu byte%s taken, in a pool of %d\n", zeno_pool_top, zeno_pool_top == 1 ? "" : "s", (int)ZENO_POOL);
    fprintf(stderr, "text: at most %zu byte%s in use at once, in a room of %d\n", zeno_most_text,
            zeno_most_text == 1 ? "" : "s", (int)ZENO_TEXT_ROOM);
#endif /* ZENO_HEAP */
}
#endif

#ifdef ZENO_HEAP
/* The items of a small list come from the pool like anything else, and
 * those of a big one from the system (see zeno_rows_free). */
static void zeno_rows_made(size_t size, size_t was) {
#ifdef ZENO_MEMORY_REPORT
    zeno_blocks_made++;
    zeno_bytes_made += size;
    zeno_in_use += size - was;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
#else
    (void)size;
    (void)was;
#endif
}

static char *zeno_alloc_rows(size_t size) {
    if (size < ZENO_BIG_ROWS) return zeno_alloc(size);
    size_t *start = calloc(1, 2 * sizeof(size_t) + size);
    if (start == NULL) zeno_pool_full();
    start[0] = size;
    zeno_head *head = (zeno_head *)(start + 2) - 1;
    head->step = ZENO_FROM_SYSTEM;
    head->asked = 1;
#ifdef ZENO_MEMORY_REPORT
    zeno_blocks_held++;
#endif
    zeno_rows_made(size, 0);
    return (char *)(start + 2);
}

/* Rows that came from the system, grown with `realloc`. */
static char *zeno_rows_regrow(char *rows, size_t size) {
    if (rows == NULL || size < ZENO_BIG_ROWS || ((zeno_head *)rows - 1)->step != ZENO_FROM_SYSTEM) return NULL;
    size_t *start = (size_t *)rows - 2;
    size_t was = start[0];
    start = realloc(start, 2 * sizeof(size_t) + size);
    if (start == NULL) zeno_pool_full();
    start[0] = size;
    zeno_rows_made(size, was);
    return (char *)(start + 2);
}
#else /* ZENO_HEAP */
/* The items of a list come from the pool like anything else. */
static char *zeno_alloc_rows(size_t size) {
    return zeno_alloc(size);
}
#endif /* ZENO_HEAP */

#endif /* ZENO_NO_COLLECTOR */

/* The items of a list, given room for `size` bytes, of which the first
 * `used` are kept. */
static char *zeno_rows_grow(char *rows, size_t used, size_t size) {
#ifdef ZENO_HEAP
    char *grown = zeno_rows_regrow(rows, size);
    if (grown != NULL) return grown;
#endif /* ZENO_HEAP */
    char *moved = zeno_alloc_rows(size);
    if (used > 0) memcpy(moved, rows, used);
    zeno_free(rows);
    return moved;
}

/* ------------------------------------------------------------------- Errors
 *
 * An error value is what `ValueError("bad")` makes, what `raise` sends and
 * what `except ValueError as err` receives.
 */
typedef struct {
    const char *kind;    /* "ValueError", or the name of a class */
    const char *message; /* what `print(err)` shows */
    void *object;        /* the object itself, when an object was raised */
#ifdef ZENO_NO_COLLECTOR
    void (*drop)(void *); /* what frees `object`, when it is one */
#endif /* ZENO_NO_COLLECTOR */
} zeno_error;

#ifndef ZENO_NO_COLLECTOR

static zeno_error *zeno_error_new(const char *kind, const char *message, void *object) {
    zeno_error *error = (zeno_error *)zeno_alloc(sizeof(zeno_error));
    error->kind = kind;
    error->message = message;
    error->object = object;
    return error;
}

#else /* ZENO_NO_COLLECTOR */

/* With no collector, an error is one block that is freed when the `except`
 * that caught it ends. The message is often text that was built for it,
 * which will not last that long, so the error keeps a copy of its own,
 * in the same block. */
static zeno_error *zeno_error_new(const char *kind, const char *message, void *object) {
    size_t length = strlen(message) + 1;
    zeno_error *error = (zeno_error *)zeno_alloc(sizeof(zeno_error) + length);
    char *kept = (char *)(error + 1);
    memcpy(kept, message, length);
    error->kind = kind;
    error->message = kept;
    error->object = object;
    return error;
}

/* `raise Overload(channel, name)`: an object travelling as an error.
 * `drop` is the function the compiler wrote to free such an object, with
 * whatever text it keeps. */
static zeno_error *zeno_error_of(const char *kind, const char *message, void *object, void (*drop)(void *)) {
    zeno_error *error = zeno_error_new(kind, message, object);
    error->drop = drop;
    return error;
}

/* The `except` block that caught `error` is over: free the error, and the
 * object that was raised, if there was one. */
static void zeno_error_drop(zeno_error *error) {
    if (error == NULL) return;
    if (error->drop != NULL) {
        error->drop(error->object);
    } else {
        zeno_free(error->object);
    }
    zeno_free(error);
}

#endif /* ZENO_NO_COLLECTOR */

static bool zeno_error_same(zeno_error *a, zeno_error *b) {
    return strcmp(a->kind, b->kind) == 0 && strcmp(a->message, b->message) == 0;
}

/* The error on its way to an `except`, and where it was raised. */
static zeno_error *zeno_raised = NULL;
#ifndef ZENO_MINIMAL
static int zeno_raised_line = 0;
static int zeno_raised_col = 0;
#endif /* ZENO_MINIMAL */

/* Nobody caught it: print what `zeno run` prints, and stop. */
__attribute__((noreturn)) static void zeno_uncaught(void) {
    fflush(stdout);
#ifndef ZENO_MINIMAL
    fprintf(stderr, "error: uncaught %s: %s\n --> %s:%d:%d\n", zeno_kind_shown(zeno_raised->kind), zeno_raised->message,
            zeno_file_of(zeno_raised_line), zeno_line_of(zeno_raised_line), zeno_raised_col);
#else /* ZENO_MINIMAL */
    fprintf(stderr, "error: uncaught %s: %s\n", zeno_kind_shown(zeno_raised->kind), zeno_raised->message);
#endif /* ZENO_MINIMAL */
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
#ifndef ZENO_MINIMAL
    zeno_raised_line = line;
    zeno_raised_col = col;
#endif /* ZENO_MINIMAL */
    if (zeno_trying == NULL) zeno_uncaught();
    zeno_try *handler = zeno_trying;
    zeno_trying = handler->outer; /* that `try` is over, whatever happens next */
    zeno_longjmp(handler->place);
}

/* No `except` of this `try` wanted the error: pass it outward unchanged. */
__attribute__((noreturn)) static void zeno_raise_again(void) {
#ifndef ZENO_MINIMAL
    zeno_raise(zeno_raised, zeno_raised_line, zeno_raised_col);
#else /* ZENO_MINIMAL */
    zeno_raise(zeno_raised);
#endif /* ZENO_MINIMAL */
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
#ifndef ZENO_MINIMAL
    zeno_raised_line = line;
    zeno_raised_col = col;
#endif /* ZENO_MINIMAL */
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
#ifndef ZENO_MINIMAL
__attribute__((noreturn)) static void zeno_bug(const char *message, int line, int col) {
    fflush(stdout);
    fprintf(stderr, "error: %s\n --> %s:%d:%d\n", message, zeno_file_of(line), zeno_line_of(line), col);
    exit(1);
}
#endif /* ZENO_MINIMAL */

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
#ifndef ZENO_MINIMAL
    int64_t result;
    if (__builtin_add_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
#else /* ZENO_MINIMAL */
    return a + b;
#endif /* ZENO_MINIMAL */
}

static int64_t zeno_sub(int64_t a, int64_t b, int line, int col) {
#ifndef ZENO_MINIMAL
    int64_t result;
    if (__builtin_sub_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
#else /* ZENO_MINIMAL */
    return a - b;
#endif /* ZENO_MINIMAL */
}

static int64_t zeno_mul(int64_t a, int64_t b, int line, int col) {
#ifndef ZENO_MINIMAL
    int64_t result;
    if (__builtin_mul_overflow(a, b, &result)) zeno_overflow(line, col);
    return result;
#else /* ZENO_MINIMAL */
    return a * b;
#endif /* ZENO_MINIMAL */
}

static int64_t zeno_neg(int64_t a, int line, int col) {
#ifndef ZENO_MINIMAL
    return zeno_sub(0, a, line, col);
#else /* ZENO_MINIMAL */
    return -a;
#endif /* ZENO_MINIMAL */
}

static int64_t zeno_abs(int64_t a, int line, int col) {
#ifndef ZENO_MINIMAL
    return a < 0 ? zeno_neg(a, line, col) : a;
#else /* ZENO_MINIMAL */
    return a < 0 ? -a : a;
#endif /* ZENO_MINIMAL */
}

/* `//` rounds down. C rounds toward zero, so step down once more when the
 * signs differ and the division was not exact. */
static int64_t zeno_floordiv(int64_t a, int64_t b, int line, int col) {
#ifndef ZENO_MINIMAL
    if (b == 0) zeno_by_zero(line, col);
    if (a == INT64_MIN && b == -1) zeno_overflow(line, col);
#endif /* ZENO_MINIMAL */
    int64_t q = a / b;
    if ((a ^ b) < 0 && a % b != 0) q -= 1;
    return q;
}

/* `a // 2`, `a // 4` and so on: `a` with its bits moved `by` places
 * down, which rounds down for a number below zero too. C leaves the
 * shift of a number below zero to the compiler, so it is done on the
 * number turned over; a C compiler makes one instruction of it. */
static int64_t zeno_halved(int64_t a, int by) {
    return a < 0 ? ~(~a >> by) : a >> by;
}

/* `%` takes the sign of the right side, to pair with `//`. */
static int64_t zeno_mod(int64_t a, int64_t b, int line, int col) {
#ifndef ZENO_MINIMAL
    if (b == 0) zeno_by_zero(line, col);
    if (b == -1) return 0; /* also avoids INT64_MIN % -1, which C leaves undefined */
#else /* ZENO_MINIMAL */
    if (b == -1) return 0; /* INT64_MIN % -1 is one C leaves undefined */
#endif /* ZENO_MINIMAL */
    int64_t r = a % b;
    /* Signs first: a branch on them is almost always guessed right,
       while one on `r != 0` is not (decision 23.5). */
    if ((r ^ b) < 0 && r != 0) r += b;
    return r;
}

static int64_t zeno_pow(int64_t base, int64_t power, int line, int col) {
#ifndef ZENO_MINIMAL
    if (power < 0) {
        zeno_fail("ValueError",
                  "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`",
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    /* These never overflow, however big the power, so answer them directly
     * instead of looping. */
    if (base == 1 || power == 0) return 1;
    if (base == 0) return 0;
    if (base == -1) return power % 2 == 0 ? 1 : -1;
    int64_t result = 1;
#ifndef ZENO_MINIMAL
    for (int64_t i = 0; i < power; i++) result = zeno_mul(result, base, line, col);
#else /* ZENO_MINIMAL */
    for (int64_t i = 0; i < power; i++) result *= base;
#endif /* ZENO_MINIMAL */
    return result;
}

/* How many numbers `range(start, stop, step)` holds. */
static int64_t zeno_range_count(int64_t start, int64_t stop, int64_t step, int line, int col) {
#ifndef ZENO_MINIMAL
    if (step == 0) zeno_fail("ValueError", "the step of a range cannot be 0", line, col);
#endif /* ZENO_MINIMAL */
    if (step > 0 && start < stop) {
        return (int64_t)(((uint64_t)stop - (uint64_t)start - 1) / (uint64_t)step) + 1;
    }
    if (step < 0 && start > stop) {
        return (int64_t)(((uint64_t)start - (uint64_t)stop - 1) / (0 - (uint64_t)step)) + 1;
    }
    return 0;
}

/* ------------------------------------------------------------------- Floats */

/* `ZENO_UNCHECKED` is `zeno build --fast` (decision 23.1): the checks a
 * correct program never fails are left out, as C and Rust leave them out.
 * A float divided by zero is then infinity, the square root of a number
 * below zero is NaN, and a list read out of range reads whatever memory
 * is there. A whole number divided by zero is still caught: C would stop
 * the program with a signal, or worse. */
static double zeno_div(double a, double b, int line, int col) {
#ifndef ZENO_UNCHECKED
    if (b == 0.0) zeno_by_zero(line, col);
#else
    (void)line;
    (void)col;
#endif
    return a / b;
}

static double zeno_floordiv_float(double a, double b, int line, int col) {
#ifndef ZENO_MINIMAL
    return floor(zeno_div(a, b, line, col));
#else /* ZENO_MINIMAL */
    return floor(a / b);
#endif /* ZENO_MINIMAL */
}

static double zeno_mod_float(double a, double b, int line, int col) {
#ifndef ZENO_MINIMAL
    if (b == 0.0) zeno_by_zero(line, col);
#endif /* ZENO_MINIMAL */
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

/* A piece of text that a new list is about to hold: one word of
 * `text.split()`, one character of `for c in text`. With a collector it
 * is text like any other. With none, a list keeps its own copies of its
 * items (docs/text.md), so the piece is made in the pool to begin with
 * and never passes through the text room. */
/* Every text of one ASCII character, made once. `for ch in text` and
 * `text.split("")` hand these out instead of making a block for each
 * character: a block costs a trip to the allocator (and, with a collector,
 * a place in its table), where this costs nothing. They are written into
 * the program like a literal, so with no collector nothing frees them,
 * which is right: only text in the pool is ever given back. */
static const char zeno_ascii[128][2] = {
#define ZENO_A(n) {(char)(n), 0}, {(char)(n + 1), 0}, {(char)(n + 2), 0}, {(char)(n + 3), 0}
#define ZENO_B(n) ZENO_A(n), ZENO_A(n + 4), ZENO_A(n + 8), ZENO_A(n + 12)
    ZENO_B(0), ZENO_B(16), ZENO_B(32), ZENO_B(48), ZENO_B(64), ZENO_B(80), ZENO_B(96), ZENO_B(112)
#undef ZENO_B
#undef ZENO_A
};

static const char *zeno_piece(const char *text, size_t length) {
    if (length == 1 && (unsigned char)text[0] < 128 && text[0] != '\0') return zeno_ascii[(unsigned char)text[0]];
#ifdef ZENO_NO_COLLECTOR
    char *piece = zeno_alloc(length + 1); /* wiped, so it ends in a zero */
    memcpy(piece, text, length);
    return piece;
#else /* ZENO_NO_COLLECTOR */
    return zeno_copy(text, length);
#endif /* ZENO_NO_COLLECTOR */
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

/* Digits written from the right, two at a time from a table: snprintf
 * reads a format first, and on a program that turns millions of numbers
 * into text that was most of its time (decision 23.2). */
static const char zeno_digit_pairs[] =
    "00010203040506070809101112131415161718192021222324252627282930313233343536373839"
    "40414243444546474849505152535455565758596061626364656667686970717273747576777879"
    "8081828384858687888990919293949596979899";

static const char *zeno_str_int(int64_t n) {
    char buffer[24];
    char *end = buffer + sizeof buffer, *p = end;
    uint64_t u = n < 0 ? 0 - (uint64_t)n : (uint64_t)n;
    while (u >= 100) {
        unsigned pair = (unsigned)(u % 100) * 2;
        u /= 100;
        *--p = zeno_digit_pairs[pair + 1];
        *--p = zeno_digit_pairs[pair];
    }
    if (u >= 10) {
        *--p = zeno_digit_pairs[u * 2 + 1];
        *--p = zeno_digit_pairs[u * 2];
    } else {
        *--p = (char)('0' + u);
    }
    if (n < 0) *--p = '-';
    return zeno_copy(p, (size_t)(end - p));
}

static const char *zeno_str_bool(bool b) {
    return b ? "true" : "false";
}

/* A float as text: the fewest digits that read back as the same float, and
 * always with a `.0` or an exponent so it cannot be mistaken for an int.
 * 0.1 + 0.2 prints as 0.30000000000000004, and 3.0 as 3.0. */
#ifndef ZENO_MINIMAL
static const char *zeno_str_float(double x) {
#else /* ZENO_MINIMAL */
static const char *zeno_float_text(double x, char out[64]) {
#endif /* ZENO_MINIMAL */
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

#ifndef ZENO_MINIMAL
    char out[64];
#endif /* ZENO_MINIMAL */
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
#ifndef ZENO_MINIMAL
    return zeno_copy(out, (size_t)(o - out));
}
#else /* ZENO_MINIMAL */
    *o = '\0';
    return out;
}

static const char *zeno_str_float(double x) {
    char out[64];
    const char *text = zeno_float_text(x, out);
    return text == out ? zeno_copy(out, strlen(out)) : text;
}

static const char *zeno_quantity_text(double x, char out[64]) {
    if (x == floor(x) && fabs(x) < 1e15) {
        snprintf(out, 64, "%lld", (long long)x);
        return out;
    }
    return zeno_float_text(x, out);
}
#endif /* ZENO_MINIMAL */

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

/* `{angle:.1f}`: a quantity with a fixed number of decimals, and its
 * unit after it as always: `30.5 deg`. */
static const char *zeno_fixed_quantity(double x, int decimals, const char *unit) {
    return zeno_join(3, zeno_fixed(x, decimals), " ", unit);
}

/* `{n:x}`, `{n:X}`, `{n:b}`: hexadecimal or binary. */
#ifndef ZENO_MINIMAL
static const char *zeno_radix(int64_t n, char kind) {
    char buffer[72];
    char *end = buffer + sizeof buffer;
#else /* ZENO_MINIMAL */
static const char *zeno_radix_text(int64_t n, char kind, int zeros, char buffer[72]) {
    char *end = buffer + 71;
    *end = '\0';
#endif /* ZENO_MINIMAL */
    char *p = end;
    uint64_t size = n < 0 ? 0 - (uint64_t)n : (uint64_t)n;
    unsigned base = kind == 'b' ? 2 : 16;
    const char *symbols = kind == 'X' ? "0123456789ABCDEF" : "0123456789abcdef";
    do {
        *--p = symbols[size % base];
        size /= base;
    } while (size != 0);
#ifndef ZENO_MINIMAL
    if (n < 0) *--p = '-';
    return zeno_copy(p, (size_t)(end - p));
}
#else /* ZENO_MINIMAL */
    while (end - p < zeros - (n < 0)) *--p = '0';
    if (n < 0) *--p = '-';
    return p;
}

static const char *zeno_radix(int64_t n, char kind) {
    char buffer[72];
    const char *text = zeno_radix_text(n, kind, 0, buffer);
    return zeno_copy(text, strlen(text));
}
#endif /* ZENO_MINIMAL */

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

/* `a << n` and `a >> n`. An int has 64 bits, so `n` is 0 to 63. */
static void zeno_shift_count(int64_t n, int line, int col) {
#ifndef ZENO_MINIMAL
    if (n < 0 || n > 63) {
        zeno_fail("ValueError",
                  zeno_join(3, "cannot shift by ", zeno_str_int(n), ". An int has 64 bits, so a shift is 0 to 63"),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
}

/* `a << n` is `a` times two, `n` times, and like `*` it must fit. The
 * shift is done on the unsigned number, where C says what happens to the
 * bits that leave. Shifting back gives `a` again unless some did. */
static int64_t zeno_shl(int64_t a, int64_t n, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_shift_count(n, line, col);
    int64_t shifted = (int64_t)((uint64_t)a << n);
    if ((shifted < 0 ? ~(~shifted >> n) : shifted >> n) != a) zeno_overflow(line, col);
    return shifted;
#else /* ZENO_MINIMAL */
    return (int64_t)((uint64_t)a << n);
#endif /* ZENO_MINIMAL */
}

/* `a >> n` halves `a`, `n` times, rounding down as `//` does. C leaves
 * `>>` on a negative number to the compiler, so that case is done on the
 * number with every bit turned over, which is not negative. */
static int64_t zeno_shr(int64_t a, int64_t n, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_shift_count(n, line, col);
#endif /* ZENO_MINIMAL */
    return a < 0 ? ~(~a >> n) : a >> n;
}

/* `u8(n)`: `n` itself if the size can hold it. `size` is the size in
 * words, "a u8", for the message. A sized whole number is an ordinary
 * int64_t that has been through here, or through zeno_wrap. */
static int64_t zeno_fit(int64_t n, int64_t min, int64_t max, const char *size, int line, int col) {
#ifndef ZENO_MINIMAL
    if (n < min || n > max) {
        zeno_fail("OverflowError",
                  zeno_join(8, zeno_str_int(n), " does not fit in ", size, " (", zeno_str_int(min), " to ",
                            zeno_str_int(max), ")"),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    return n;
}

/* `u8.wrap(n)`: the low `bits` bits of `n`. In a signed size the top one
 * of them counts as minus, so `i8.wrap(200)` is -56. Done on the unsigned
 * number, where C says exactly what happens. */
static int64_t zeno_wrap(int64_t n, int bits, bool is_signed) {
    uint64_t low = (uint64_t)n & ((UINT64_C(1) << bits) - 1);
    if (is_signed && (low >> (bits - 1)) != 0) return (int64_t)low - (INT64_C(1) << bits);
    return (int64_t)low;
}

static double zeno_sqrt(double x, int line, int col) {
#ifndef ZENO_UNCHECKED
    if (x < 0.0) {
        zeno_fail("ValueError", zeno_join(2, "cannot take the square root of ", zeno_str_float(x)), line, col);
    }
#else
    (void)line;
    (void)col;
#endif
    return sqrt(x);
}

/* A logarithm is of a number above zero. `base` is 2 or 10, or 0 for
 * the natural one. */
static double zeno_log(double x, int base, int line, int col) {
#ifndef ZENO_MINIMAL
    if (x <= 0.0) {
        zeno_fail("ValueError", zeno_join(2, "cannot take the logarithm of ", zeno_str_float(x)), line, col);
    }
#endif /* ZENO_MINIMAL */
    return base == 2 ? log2(x) : base == 10 ? log10(x) : log(x);
}

/* `asin` and `acos` are given the sine or cosine of some angle, which
 * lies from -1 to 1. */
static void zeno_ratio(const char *name, double x, int line, int col) {
#ifndef ZENO_MINIMAL
    if (x < -1.0 || x > 1.0) {
        zeno_fail("ValueError",
                  zeno_join(4, "`", name, "` needs a number from -1 to 1, and this is ", zeno_str_float(x)), line, col);
    }
#endif /* ZENO_MINIMAL */
}

static double zeno_asin(double x, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_ratio("asin", x, line, col);
#endif /* ZENO_MINIMAL */
    return asin(x);
}

static double zeno_acos(double x, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_ratio("acos", x, line, col);
#endif /* ZENO_MINIMAL */
    return acos(x);
}

/* ---------------------------------------------------------------- The board
 *
 * `from board import led` and `from time import sleep` end up here. A
 * Zeno program touches the world outside it through four functions, and
 * only these four:
 *
 *   zeno_board_write(pin, level)   drive a pin high or low
 *   zeno_board_read(pin)           is the pin high?
 *   zeno_board_sleep(seconds)      wait
 *   zeno_board_now()               a clock that only goes forward, in ms
 *
 * together with ZENO_PINS, the number of pins. Below is a simulated
 * board, for a program that runs on a computer: it keeps the level of
 * each pin in an array, and prints a line when a level changes, as a
 * logic analyser on the pin would show it.
 *
 * To build for a real board, write the four functions for it in a
 * header, and compile with -DZENO_BOARD='"my_board.h"'. Nothing else in
 * this file knows what a pin is made of. See docs/board.md. */

/* The pin the LED is on, as a Zeno program numbers it. A real board is
 * free to wire that number to whichever pin its LED is really on. */
#define ZENO_LED ((int64_t)13)

#ifdef ZENO_BOARD
#include ZENO_BOARD
#else

#include <time.h>

#define ZENO_PINS 64

static bool zeno_levels[ZENO_PINS];

static bool zeno_board_read(int pin) {
    return zeno_levels[pin];
}

static void zeno_board_write(int pin, bool level) {
    /* A pin that is already there does not move, and shows nothing. */
    if (zeno_levels[pin] == level) return;
    zeno_levels[pin] = level;
    if (pin == ZENO_LED) {
        printf("led %s\n", level ? "on" : "off");
    } else {
        printf("pin %d %s\n", pin, level ? "on" : "off");
    }
}

/* Waiting and reading the clock are the two things here that the C
 * language leaves to the system, and there are two families of system.
 *
 * Windows has `Sleep`, in milliseconds, and a counter that goes up
 * steadily, with a second call that says how many counts make a second.
 * They are declared here by hand instead of through <windows.h>, which
 * is very large and defines thousands of names of its own, `min` and
 * `max` among them. The three lines are exactly what that header says.
 *
 * Everything else (Linux, macOS, the BSDs) has `nanosleep` and
 * `clock_gettime`. MinGW on Windows has those too, but Microsoft's own
 * C library does not, so on Windows the Windows calls are used whichever
 * compiler it is. */
#ifdef _WIN32

__declspec(dllimport) void __stdcall Sleep(unsigned long milliseconds);
__declspec(dllimport) int __stdcall QueryPerformanceCounter(int64_t *count);
__declspec(dllimport) int __stdcall QueryPerformanceFrequency(int64_t *counts_a_second);

static void zeno_board_sleep(double seconds) {
    /* What has been printed so far should be seen before the wait, also
     * when the output goes to a file or another program. */
    fflush(stdout);
    Sleep((unsigned long)(seconds * 1000.0 + 0.5));
}

static double zeno_board_now(void) {
    int64_t count, counts_a_second;
    QueryPerformanceCounter(&count);
    QueryPerformanceFrequency(&counts_a_second);
    return (double)count * 1000.0 / (double)counts_a_second;
}

#else

static void zeno_board_sleep(double seconds) {
    /* What has been printed so far should be seen before the wait, also
     * when the output goes to a file or another program. */
    fflush(stdout);
    struct timespec wait;
    wait.tv_sec = (time_t)seconds;
    wait.tv_nsec = (long)((seconds - (double)wait.tv_sec) * 1e9);
    nanosleep(&wait, NULL);
}

static double zeno_board_now(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (double)now.tv_sec * 1000.0 + (double)now.tv_nsec / 1e6;
}

#endif /* _WIN32 */

#endif /* ZENO_BOARD */

/* Registers: `register(u32, 0x40020014)`, then `.read()` and `.write(v)`
 * inside `unsafe { }`.
 *
 * On a real board a register is a place in memory at a fixed address,
 * and reading or writing it is one `volatile` access: the compiler may
 * not skip it, repeat it or move it, because the hardware is watching.
 * A board header can supply its own two functions and define
 * ZENO_BOARD_REGISTERS, for a board that reaches its registers some
 * other way.
 *
 * The simulated board has no such memory, and writing to address
 * 0x40020014 of a computer would stop the program. So it remembers
 * what was last written to each address, and prints a line for every
 * write. A register nobody has written to reads as 0. */
#ifdef ZENO_BOARD
#ifndef ZENO_BOARD_REGISTERS

static int64_t zeno_register_read(int64_t address, int bits) {
    switch (bits) {
    case 8: return *(volatile uint8_t *)(uintptr_t)address;
    case 16: return *(volatile uint16_t *)(uintptr_t)address;
    default: return *(volatile uint32_t *)(uintptr_t)address;
    }
}

static void zeno_register_write(int64_t address, int bits, int64_t value) {
    switch (bits) {
    case 8: *(volatile uint8_t *)(uintptr_t)address = (uint8_t)value; break;
    case 16: *(volatile uint16_t *)(uintptr_t)address = (uint16_t)value; break;
    default: *(volatile uint32_t *)(uintptr_t)address = (uint32_t)value; break;
    }
}

#endif /* ZENO_BOARD_REGISTERS */
#else /* ZENO_BOARD */

#define ZENO_REGISTERS 256

static int64_t zeno_register_at[ZENO_REGISTERS];
static int64_t zeno_register_holds[ZENO_REGISTERS];
static int zeno_register_count = 0;

static int64_t zeno_register_read(int64_t address, int bits) {
    (void)bits;
    for (int i = 0; i < zeno_register_count; i++) {
        if (zeno_register_at[i] == address) return zeno_register_holds[i];
    }
    return 0;
}

static void zeno_register_write(int64_t address, int bits, int64_t value) {
    int i = 0;
    while (i < zeno_register_count && zeno_register_at[i] != address) i++;
    if (i == zeno_register_count) {
        if (i == ZENO_REGISTERS) {
            fflush(stdout);
            fprintf(stderr, "error: the simulated board remembers %d registers, and this is one more\n", ZENO_REGISTERS);
            exit(1);
        }
        zeno_register_at[i] = address;
        zeno_register_count++;
    }
    zeno_register_holds[i] = value;
    /* Every write is printed, also one that changes nothing: on real
     * hardware a write is an event, whatever was there before. */
    printf("register 0x%08" PRIx64 " = 0x%0*" PRIx64 "\n", (uint64_t)address, bits / 4, (uint64_t)value);
}

#endif /* ZENO_BOARD */

/* A register as `print` shows it. */
static const char *zeno_str_register(int64_t address) {
    char buffer[40];
    int length = snprintf(buffer, sizeof buffer, "Register(0x%08" PRIx64 ")", (uint64_t)address);
    return zeno_copy(buffer, (size_t)length);
}

/* ------------------------------------------------------------ Manual memory
 *
 * `allocate` and `free`, for the lines inside `unsafe { }`. See
 * docs/unsafe.md.
 *
 * `allocate(u8, 16)` gives sixteen bytes, all zero. They come from the
 * heap, and the collector never looks at them: the program said it would
 * free them itself. Where there is no collector there is no heap either,
 * so they come from the pool, like everything else.
 *
 * Nothing here checks how the memory is used. A pointer in a compiled
 * program is an address and nothing more, as in C. `zeno run` is where a
 * read past the end or a use after `free` is caught. */
static void *zeno_allocate(int64_t count, size_t size, int line, int col) {
#ifndef ZENO_MINIMAL
    if (count < 1) {
        zeno_bug(zeno_join(2, "`allocate` needs at least one item, and this asks for ", zeno_str_int(count)), line, col);
    }
#endif /* ZENO_MINIMAL */
#ifdef ZENO_NO_COLLECTOR
    return zeno_alloc((size_t)count * size);
#else /* ZENO_NO_COLLECTOR */
    void *memory = calloc((size_t)count, size);
#ifndef ZENO_MINIMAL
    if (memory == NULL) zeno_bug("there is no memory left for this `allocate`", line, col);
#else /* ZENO_MINIMAL */
    if (memory == NULL) zeno_out_of_memory();
#endif /* ZENO_MINIMAL */
    zeno_by_hand_made++;
    return memory;
#endif /* ZENO_NO_COLLECTOR */
}

/* The other half: `free` gives the memory back, to the heap or the pool. */
static void zeno_release(void *memory) {
#ifdef ZENO_NO_COLLECTOR
    zeno_free(memory);
#else /* ZENO_NO_COLLECTOR */
    if (memory != NULL) zeno_by_hand_freed++;
    free(memory);
#endif /* ZENO_NO_COLLECTOR */
}

/* `pin(7)`: the pin with that number, if the board has one. */
static int64_t zeno_pin(int64_t number, int line, int col) {
#ifndef ZENO_MINIMAL
    if (number < 0 || number >= ZENO_PINS) {
        zeno_fail("ValueError",
                  zeno_join(4, "this board has no pin ", zeno_str_int(number), ". Its pins are 0 to ",
                            zeno_str_int(ZENO_PINS - 1)),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    return number;
}

static void zeno_pin_on(int64_t pin) { zeno_board_write((int)pin, true); }
static void zeno_pin_off(int64_t pin) { zeno_board_write((int)pin, false); }
static void zeno_pin_toggle(int64_t pin) { zeno_board_write((int)pin, !zeno_board_read((int)pin)); }
static bool zeno_pin_is_on(int64_t pin) { return zeno_board_read((int)pin); }

/* A pin as `print` shows it. */
static const char *zeno_str_pin(int64_t pin) {
    return zeno_join(3, "Pin(", zeno_str_int(pin), ")");
}

/* `sleep(500 ms)`. The compiler has already turned the time into
 * seconds. A time that is zero, negative or not a number waits for
 * nothing; one too long to count is cut to about thirty years. */
static void zeno_sleep(double seconds) {
    if (!(seconds > 0.0)) return;
    if (seconds > 1e9) seconds = 1e9;
    zeno_board_sleep(seconds);
}

/* `now()`, in milliseconds. */
static double zeno_now(void) {
    return zeno_board_now();
}

/* `int(x)` on a float drops the fraction. Infinity has no int to become. */
static int64_t zeno_int(double x, int line, int col) {
#ifndef ZENO_MINIMAL
    if (x != x || x == INFINITY || x == -INFINITY) {
        zeno_fail("ValueError", zeno_join(3, "cannot turn ", zeno_str_float(x), " into an int"), line, col);
    }
#endif /* ZENO_MINIMAL */
    return zeno_whole(x);
}

/* A str the way it is shown inside a list or an object: in quotes. */
static const char *zeno_repr_str(const char *text) {
    static const char hex[] = "0123456789abcdef";
    /* The longest a byte becomes is the six of `\u{1f}`. */
    char *out = zeno_alloc_text(strlen(text) * 6 + 3);
    char *o = out;
    *o++ = '"';
    for (const unsigned char *p = (const unsigned char *)text; *p != '\0'; p++) {
        unsigned c = *p;
        /* U+0080 to U+009F are the two bytes C2 80 to C2 9F. */
        bool high = c == 0xC2 && p[1] >= 0x80 && p[1] <= 0x9F;
        if (high) c = *++p;
        switch (c) {
        case '"': *o++ = '\\'; *o++ = '"'; break;
        case '\\': *o++ = '\\'; *o++ = '\\'; break;
        case '\n': *o++ = '\\'; *o++ = 'n'; break;
        case '\t': *o++ = '\\'; *o++ = 't'; break;
        case '\r': *o++ = '\\'; *o++ = 'r'; break;
        default:
            if (c < 0x20 || c == 0x7F || high) {
                *o++ = '\\'; *o++ = 'u'; *o++ = '{';
                if (c >= 0x10) *o++ = hex[c >> 4];
                *o++ = hex[c & 15];
                *o++ = '}';
            } else {
                *o++ = (char)c;
            }
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
    list->items = room > 0 ? (zeno_slot *)zeno_alloc_rows((size_t)room * sizeof(zeno_slot)) : NULL;
    return list;
}

/* A full list moves to a place twice the size. Only the list itself ever
 * points at its `items`, so the old place can go back at once.
 *
 * This is kept out of line. It runs once in many appends, and with it
 * apart, what is left of `zeno_list_append` is a compare, a store and a
 * count, which the C compiler puts where the program appends, as a C
 * programmer writes a growing array by hand (decision 23.8). */
__attribute__((noinline)) static void zeno_list_grow(zeno_list *list) {
    int64_t room = list->room == 0 ? 4 : list->room * 2;
    list->items = (zeno_slot *)zeno_rows_grow((char *)list->items, (size_t)list->len * sizeof(zeno_slot),
                                              (size_t)room * sizeof(zeno_slot));
    list->room = room;
}

static void zeno_list_append(zeno_list *list, zeno_slot item) {
    if (list->len == list->room) zeno_list_grow(list);
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

#ifdef ZENO_NO_COLLECTOR

/* A list of text, in a build with no collector. Each item that is not
 * text written in the program is the list's own copy (zeno_text_keep),
 * so the end of the list frees those too. */
static void zeno_list_drop_text(zeno_list *list) {
    if (list == NULL) return;
    for (int64_t i = 0; i < list->len; i++) zeno_text_drop(list->items[i].s);
    zeno_list_drop(list);
}

/* `list` was just made, and its items are still somebody else's text: a
 * copy of another list, or two lists joined by `+`. Give it copies of
 * its own. */
static zeno_list *zeno_list_keep_text(zeno_list *list) {
    for (int64_t i = 0; i < list->len; i++) list->items[i].s = zeno_text_keep(list->items[i].s);
    return list;
}

/* The copy of such a list that a `for` loop walks. It has copies of the
 * items as well, so that the body of the loop can give the list new
 * items while the loop is still looking at the old ones. */
static zeno_list *zeno_list_copy_text(zeno_list *list) {
    return zeno_list_keep_text(zeno_list_copy(list));
}

#endif /* ZENO_NO_COLLECTOR */

static zeno_list *zeno_list_concat(zeno_list *a, zeno_list *b) {
    zeno_list *joined = zeno_list_new(a->len + b->len);
    for (int64_t i = 0; i < a->len; i++) zeno_list_append(joined, a->items[i]);
    for (int64_t i = 0; i < b->len; i++) zeno_list_append(joined, b->items[i]);
    return joined;
}

/* Every use of `list[i]` goes through here, so a position that is not in
 * the list is an IndexError and never a read of someone else's memory. */
static zeno_slot *zeno_list_at(zeno_list *list, int64_t i, int line, int col) {
#ifndef ZENO_UNCHECKED
    if (i < 0 || i >= list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "index ", zeno_str_int(i), " is out of range for a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
#else
    (void)line;
    (void)col;
#endif
    return &list->items[i];
}

static zeno_slot zeno_list_pop(zeno_list *list, int line, int col) {
#ifndef ZENO_MINIMAL
    if (list->len == 0) zeno_fail("IndexError", "cannot pop from an empty list", line, col);
#endif /* ZENO_MINIMAL */
    zeno_slot item = list->items[--list->len];
    /* Wipe the place it was in, or the collector would go on thinking the
     * list holds it. */
    memset(&list->items[list->len], 0, sizeof(zeno_slot));
    return item;
}

/* The three below need to know how to show or compare one item, which
 * depends on its kind. The compiler passes a small function that does. */

#ifdef ZENO_NO_COLLECTOR
/* `text` is the newest thing in the text room, and everything else written
 * since `start` was only needed to build it. Slide it down to `start` and
 * give the rest back. Showing a list joins one item at a time; without
 * this, a list of a hundred items would leave a hundred half-built texts
 * in the room. */
static const char *zeno_text_settle(size_t start, const char *text) {
    size_t size = strlen(text) + 1;
#ifdef ZENO_HEAP
    /* The text may be in a later room than `start`, and may not fit in
     * start's room. zeno_alloc_text finds where it goes. The two places
     * can only overlap within one room, where memmove copes. */
    zeno_text_top = start;
    char *settled = zeno_alloc_text(size);
    memmove(settled, text, size);
    return settled;
#else /* ZENO_HEAP */
    memmove(zeno_text_room + start, text, size);
    zeno_text_top = start + size;
    return zeno_text_room + start;
#endif /* ZENO_HEAP */
}
#endif /* ZENO_NO_COLLECTOR */

static const char *zeno_list_str(zeno_list *list, const char *(*show)(zeno_slot)) {
#ifdef ZENO_NO_COLLECTOR
    size_t start = zeno_text_top;
#endif /* ZENO_NO_COLLECTOR */
    const char *text = "[";
    for (int64_t i = 0; i < list->len; i++) {
        text = zeno_join(3, text, i > 0 ? ", " : "", show(list->items[i]));
#ifdef ZENO_NO_COLLECTOR
        text = zeno_text_settle(start, text);
#endif /* ZENO_NO_COLLECTOR */
    }
    text = zeno_join(2, text, "]");
#ifdef ZENO_NO_COLLECTOR
    text = zeno_text_settle(start, text);
#endif /* ZENO_NO_COLLECTOR */
    return text;
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

/* `list.find(item)`: the position of the first item equal to `item`, or
 * -1 when there is none. The compiled program turns the answer into an
 * `int or none` (zeno_found, further down). */
static int64_t zeno_list_find(zeno_list *list, zeno_slot item, bool (*same)(zeno_slot, zeno_slot)) {
    for (int64_t i = 0; i < list->len; i++) {
        if (same(list->items[i], item)) return i;
    }
    return -1;
}

/* `list.insert(at, item)`: `item` goes to position `at`, and everything
 * from there on moves up one place. `at` may be the length of the list,
 * which puts the item at the end. */
static void zeno_list_insert(zeno_list *list, int64_t at, zeno_slot item, int line, int col) {
#ifndef ZENO_MINIMAL
    if (at < 0 || at > list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "cannot insert at ", zeno_str_int(at), " in a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    /* Putting it at the end first makes the room, if room is needed. */
    zeno_list_append(list, item);
    for (int64_t i = list->len - 1; i > at; i--) list->items[i] = list->items[i - 1];
    list->items[at] = item;
}

/* `list.pop(at)`: the item at position `at` leaves the list, and
 * everything after it moves down one place. */
static zeno_slot zeno_list_pop_at(zeno_list *list, int64_t at, int line, int col) {
    zeno_slot item = *zeno_list_at(list, at, line, col);
    for (int64_t i = at; i < list->len - 1; i++) list->items[i] = list->items[i + 1];
    list->len--;
    memset(&list->items[list->len], 0, sizeof(zeno_slot));
    return item;
}

static void zeno_list_reverse(zeno_list *list) {
    for (int64_t i = 0, j = list->len - 1; i < j; i++, j--) {
        zeno_slot item = list->items[i];
        list->items[i] = list->items[j];
        list->items[j] = item;
    }
}

/* `list.clear()`: the list holds nothing, and keeps its room for what
 * comes next. Where the items have one owner, the compiled program has
 * freed each of them before it calls this. */
static void zeno_list_clear(zeno_list *list) {
    if (list->len > 0) memset(list->items, 0, (size_t)list->len * sizeof(zeno_slot));
    list->len = 0;
}

/* `list.sort()`: smallest first.
 *
 * `kind` says what the items are, so that two of them can be compared:
 * 0 for whole numbers, 1 for floats, 2 for text.
 *
 * The way of sorting is heapsort. It moves the items about inside the
 * list, so it needs no second list, and it is a loop and never a
 * function that calls itself, so it needs no deep stack. Both matter on
 * a small board. It takes about n log n steps whatever the order was to
 * begin with.
 *
 * A heap is the list seen as a tree: the items at 2i + 1 and 2i + 2 are
 * the two "below" the item at i, and in a heap no item is smaller than
 * one below it. So the biggest is at the front. Sorting is: make the
 * list a heap, then again and again swap the front (the biggest left)
 * with the last item of the heap, and make what is left a heap again. */
static bool zeno_before(zeno_slot a, zeno_slot b, int kind) {
    switch (kind) {
        case 0: return a.i < b.i;
        case 1: return a.f < b.f;
        default: return strcmp(a.s, b.s) < 0;
    }
}

/* Move the item at `root` down until neither item below it is bigger.
 * `end` is where the heap ends. */
static void zeno_sift(zeno_slot *items, int64_t root, int64_t end, int kind) {
    for (;;) {
        int64_t child = 2 * root + 1;
        if (child >= end) return;
        if (child + 1 < end && zeno_before(items[child], items[child + 1], kind)) child++;
        if (!zeno_before(items[root], items[child], kind)) return;
        zeno_slot item = items[root];
        items[root] = items[child];
        items[child] = item;
        root = child;
    }
}

static void zeno_list_sort(zeno_list *list, int kind) {
    for (int64_t root = list->len / 2 - 1; root >= 0; root--) zeno_sift(list->items, root, list->len, kind);
    for (int64_t end = list->len - 1; end > 0; end--) {
        zeno_slot item = list->items[0];
        list->items[0] = list->items[end];
        list->items[end] = item;
        zeno_sift(list->items, 0, end, kind);
    }
}

/* `people.sort(age_of)`: `keys` has what the function gave for each
 * item, and the items go in the order of their keys, smallest first.
 * Two items with keys that are neither before the other stay in the
 * order they were in: `places` is where each item was, and it settles
 * those. The same heapsort as above, with three lists that move
 * together, and the same as `sort_by` in src/builtins.rs. */
static bool zeno_before_by(zeno_slot *keys, zeno_slot *places, int64_t a, int64_t b, int kind) {
    if (zeno_before(keys[a], keys[b], kind)) return true;
    if (zeno_before(keys[b], keys[a], kind)) return false;
    return places[a].i < places[b].i;
}

static void zeno_swap_by(zeno_slot *items, zeno_slot *keys, zeno_slot *places, int64_t a, int64_t b) {
    zeno_slot item = items[a], key = keys[a], place = places[a];
    items[a] = items[b];
    keys[a] = keys[b];
    places[a] = places[b];
    items[b] = item;
    keys[b] = key;
    places[b] = place;
}

static void zeno_sift_by(zeno_slot *items, zeno_slot *keys, zeno_slot *places, int64_t root, int64_t end, int kind) {
    for (;;) {
        int64_t child = 2 * root + 1;
        if (child >= end) return;
        if (child + 1 < end && zeno_before_by(keys, places, child, child + 1, kind)) child++;
        if (!zeno_before_by(keys, places, root, child, kind)) return;
        zeno_swap_by(items, keys, places, root, child);
        root = child;
    }
}

static void zeno_list_sort_by(zeno_list *list, zeno_list *keys, int kind, int line, int col) {
#ifndef ZENO_MINIMAL
    if (keys->len != list->len) {
        zeno_fail("ValueError", "the function given to `sort` changed how many items the list has", line, col);
    }
#endif /* ZENO_MINIMAL */
    zeno_list *places = zeno_list_new(list->len);
    for (int64_t i = 0; i < list->len; i++) places->items[i].i = i;
    places->len = list->len;
    for (int64_t root = list->len / 2 - 1; root >= 0; root--) {
        zeno_sift_by(list->items, keys->items, places->items, root, list->len, kind);
    }
    for (int64_t end = list->len - 1; end > 0; end--) {
        zeno_swap_by(list->items, keys->items, places->items, 0, end);
        zeno_sift_by(list->items, keys->items, places->items, 0, end, kind);
    }
    zeno_list_drop(places);
}

/* `slice(from)` and `slice(from, to)` take the items (or characters)
 * from position `from` up to `to`, which is not taken itself, as in
 * `range`. With `to_end` there was no `to`, and the slice goes on to the
 * end. Anything that is not inside what is being sliced is an
 * IndexError. */
static void zeno_slice_ends(int64_t from, int64_t *to, bool to_end, int64_t length, const char *what, int line,
                            int col) {
    if (to_end) *to = length;
#ifndef ZENO_MINIMAL
    if (from < 0 || *to < from || *to > length) {
        zeno_fail("IndexError",
                  zeno_join(8, "slice ", zeno_str_int(from), " to ", zeno_str_int(*to), " is out of range for a ",
                            what, " of length ", zeno_str_int(length)),
                  line, col);
    }
#else /* ZENO_MINIMAL */
    (void)what;
#endif /* ZENO_MINIMAL */
}

/* A new list with a copy of each slot. The items themselves are shared
 * with the first list, as they are after `+`. */
static zeno_list *zeno_list_slice(zeno_list *list, int64_t from, int64_t to, bool to_end, int line, int col) {
    zeno_slice_ends(from, &to, to_end, list->len, "list", line, col);
    zeno_list *slice = zeno_list_new(to - from);
    if (to > from) memcpy(slice->items, list->items + from, (size_t)(to - from) * sizeof(zeno_slot));
    slice->len = to - from;
    return slice;
}

/* `words.join(", ")`: the items of a list of text, with `separator`
 * between each two. The length is added up first, so the text is made
 * in one piece. */
static const char *zeno_list_join(zeno_list *list, const char *separator) {
    size_t gap = strlen(separator);
    size_t length = 0;
    for (int64_t i = 0; i < list->len; i++) length += strlen(list->items[i].s) + (i > 0 ? gap : 0);
    char *joined = zeno_alloc_text(length + 1);
    char *end = joined;
    /* Each item is copied a byte at a time up to its end, which finds the
     * end and copies in one pass: most items are a few bytes long. */
    for (int64_t i = 0; i < list->len; i++) {
        if (i > 0) {
            memcpy(end, separator, gap);
            end += gap;
        }
        for (const char *p = list->items[i].s; *p != '\0'; p++) *end++ = *p;
    }
    *end = '\0';
    return joined;
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
#ifndef ZENO_MINIMAL
    zeno_fail("IndexError",
              zeno_join(4, "index ", zeno_str_int(i), " is out of range for a str of length ",
                        zeno_str_int(zeno_len(text))),
              line, col);
#else /* ZENO_MINIMAL */
    return "";
#endif /* ZENO_MINIMAL */
}

/* `for ch in text`, walked where the text is: the character that starts
 * at `p`, as a str. An ASCII one is in `zeno_ascii`, made once. Any other
 * is copied into a block. With a collector the block is left to it. With
 * no collector the loop holds the last such block in `*held` and frees it
 * when it makes the next one, or when the loop is over: whatever keeps
 * the character keeps a copy of its own. */
static const char *zeno_char_here(const char *p, const char **held) {
    if ((unsigned char)*p < 128) return zeno_ascii[(unsigned char)*p];
#ifdef ZENO_NO_COLLECTOR
    zeno_text_drop(*held);
    *held = zeno_piece(p, zeno_char_size(p));
    return *held;
#else /* ZENO_NO_COLLECTOR */
    (void)held;
    return zeno_piece(p, zeno_char_size(p));
#endif /* ZENO_NO_COLLECTOR */
}

/* `for c in text`: the characters of `text`, each as a str. */
static zeno_list *zeno_chars(const char *text) {
    zeno_list *chars = zeno_list_new(0);
    for (const char *p = text; *p != '\0'; p += zeno_char_size(p)) {
        zeno_list_append(chars, (zeno_slot){.s = zeno_piece(p, zeno_char_size(p))});
    }
    return chars;
}

/* `sum`, `min` and `max` of a list of numbers. */

static int64_t zeno_list_sum_int(zeno_list *list, int line, int col) {
    int64_t total = 0;
#ifndef ZENO_MINIMAL
    for (int64_t i = 0; i < list->len; i++) total = zeno_add(total, list->items[i].i, line, col);
#else /* ZENO_MINIMAL */
    for (int64_t i = 0; i < list->len; i++) total += list->items[i].i;
#endif /* ZENO_MINIMAL */
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
#ifndef ZENO_MINIMAL
    zeno_list_not_empty(list, name, line, col);
#else /* ZENO_MINIMAL */
    (void)name;
#endif /* ZENO_MINIMAL */
    int64_t best = list->items[0].i;
    for (int64_t i = 1; i < list->len; i++) {
        int64_t item = list->items[i].i;
        if (smallest ? item < best : item > best) best = item;
    }
    return best;
}

static double zeno_list_best_float(zeno_list *list, bool smallest, const char *name, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_list_not_empty(list, name, line, col);
#else /* ZENO_MINIMAL */
    (void)name;
#endif /* ZENO_MINIMAL */
    double best = list->items[0].f;
    for (int64_t i = 1; i < list->len; i++) {
        double item = list->items[i].f;
        if (smallest ? item < best : item > best) best = item;
    }
    return best;
}

/* ---------------------------------------------------------- Lists of bytes
 *
 * A `list[u8]` is a zeno_list whose `items` are bytes and not slots, so
 * that each item takes one byte and not eight (decision 36.2). `len` and
 * `room` count bytes. The struct is the same, so a list of bytes is made
 * an owner's, given back and freed as every other list is; only what
 * reads or writes the items needs to know, and the compiler, which knows
 * the type, calls the functions below for it. */

#define ZENO_BYTES(list) ((uint8_t *)(list)->items)

static zeno_list *zeno_bytes_new(int64_t room) {
    zeno_list *list = (zeno_list *)zeno_alloc(sizeof(zeno_list));
    list->len = 0;
    list->room = room;
    list->items = room > 0 ? (zeno_slot *)zeno_alloc_rows((size_t)room) : NULL;
    return list;
}

__attribute__((noinline)) static void zeno_bytes_grow(zeno_list *list) {
    int64_t room = list->room < 8 ? 8 : list->room * 2;
    list->items = (zeno_slot *)zeno_rows_grow((char *)list->items, (size_t)list->len, (size_t)room);
    list->room = room;
}

/* A byte goes in and comes out as an int64_t, which is how the compiled
 * program holds every whole number; the checker has seen that it fits. */
static void zeno_bytes_append(zeno_list *list, int64_t item) {
    if (list->len == list->room) zeno_bytes_grow(list);
    ZENO_BYTES(list)[list->len++] = (uint8_t)item;
}

/* `[1, 2, 3]` where a list[u8] is wanted. Each byte travels through
 * `...` as an int64_t. */
static zeno_list *zeno_bytes_of(int count, ...) {
    zeno_list *list = zeno_bytes_new(count);
    va_list items;
    va_start(items, count);
    for (int i = 0; i < count; i++) ZENO_BYTES(list)[i] = (uint8_t)va_arg(items, int64_t);
    va_end(items);
    list->len = count;
    return list;
}

static zeno_list *zeno_bytes_copy(zeno_list *list) {
    zeno_list *copy = zeno_bytes_new(list->len);
    if (list->len > 0) memcpy(copy->items, list->items, (size_t)list->len);
    copy->len = list->len;
    return copy;
}

static zeno_list *zeno_bytes_concat(zeno_list *a, zeno_list *b) {
    zeno_list *joined = zeno_bytes_new(a->len + b->len);
    if (a->len > 0) memcpy(ZENO_BYTES(joined), a->items, (size_t)a->len);
    if (b->len > 0) memcpy(ZENO_BYTES(joined) + a->len, b->items, (size_t)b->len);
    joined->len = a->len + b->len;
    return joined;
}

static uint8_t *zeno_bytes_at(zeno_list *list, int64_t i, int line, int col) {
#ifndef ZENO_UNCHECKED
    if (i < 0 || i >= list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "index ", zeno_str_int(i), " is out of range for a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
#else
    (void)line;
    (void)col;
#endif
    return &ZENO_BYTES(list)[i];
}

static int64_t zeno_bytes_pop(zeno_list *list, int line, int col) {
#ifndef ZENO_MINIMAL
    if (list->len == 0) zeno_fail("IndexError", "cannot pop from an empty list", line, col);
#endif /* ZENO_MINIMAL */
    return ZENO_BYTES(list)[--list->len];
}

static int64_t zeno_bytes_pop_at(zeno_list *list, int64_t at, int line, int col) {
    int64_t item = *zeno_bytes_at(list, at, line, col);
    memmove(ZENO_BYTES(list) + at, ZENO_BYTES(list) + at + 1, (size_t)(list->len - at - 1));
    list->len--;
    return item;
}

static void zeno_bytes_insert(zeno_list *list, int64_t at, int64_t item, int line, int col) {
#ifndef ZENO_MINIMAL
    if (at < 0 || at > list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "cannot insert at ", zeno_str_int(at), " in a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    zeno_bytes_append(list, item);
    memmove(ZENO_BYTES(list) + at + 1, ZENO_BYTES(list) + at, (size_t)(list->len - 1 - at));
    ZENO_BYTES(list)[at] = (uint8_t)item;
}

static const char *zeno_bytes_str(zeno_list *list) {
    /* At most "255, " for each, the brackets and the end. */
    char *text = zeno_alloc_text((size_t)list->len * 5 + 3);
    char *end = text;
    *end++ = '[';
    for (int64_t i = 0; i < list->len; i++) {
        if (i > 0) {
            *end++ = ',';
            *end++ = ' ';
        }
        unsigned n = ZENO_BYTES(list)[i];
        if (n >= 100) *end++ = (char)('0' + n / 100);
        if (n >= 10) *end++ = (char)('0' + n / 10 % 10);
        *end++ = (char)('0' + n % 10);
    }
    *end++ = ']';
    *end = '\0';
    return text;
}

static bool zeno_bytes_eq(zeno_list *a, zeno_list *b) {
    return a->len == b->len && (a->len == 0 || memcmp(a->items, b->items, (size_t)a->len) == 0);
}

/* `bytes == [1, 2]`, where the list written out is a list of ints. */
static bool zeno_bytes_eq_ints(zeno_list *bytes, zeno_list *ints) {
    if (bytes->len != ints->len) return false;
    for (int64_t i = 0; i < bytes->len; i++) {
        if (ZENO_BYTES(bytes)[i] != ints->items[i].i) return false;
    }
    return true;
}

static int64_t zeno_bytes_find(zeno_list *list, int64_t item) {
    if (item < 0 || item > 255 || list->len == 0) return -1;
    const uint8_t *at = memchr(list->items, (int)item, (size_t)list->len);
    return at == NULL ? -1 : (int64_t)(at - ZENO_BYTES(list));
}

static bool zeno_bytes_contains(zeno_list *list, int64_t item) {
    return zeno_bytes_find(list, item) >= 0;
}

static void zeno_bytes_reverse(zeno_list *list) {
    for (int64_t i = 0, j = list->len - 1; i < j; i++, j--) {
        uint8_t item = ZENO_BYTES(list)[i];
        ZENO_BYTES(list)[i] = ZENO_BYTES(list)[j];
        ZENO_BYTES(list)[j] = item;
    }
}

static void zeno_bytes_clear(zeno_list *list) {
    list->len = 0;
}

/* Smallest first. With only 256 values a byte can have, counting how
 * many there are of each is quicker than any sort that compares, and
 * needs no room but the counts. */
static void zeno_bytes_sort(zeno_list *list) {
    int64_t counts[256] = {0};
    for (int64_t i = 0; i < list->len; i++) counts[ZENO_BYTES(list)[i]]++;
    int64_t at = 0;
    for (int value = 0; value < 256; value++) {
        for (int64_t k = 0; k < counts[value]; k++) ZENO_BYTES(list)[at++] = (uint8_t)value;
    }
}

/* `bytes.sort(key)`: the bytes go into a list of slots for the heapsort
 * that every list uses, and come back in their new order. */
static void zeno_bytes_sort_by(zeno_list *list, zeno_list *keys, int kind, int line, int col) {
    zeno_list *slots = zeno_list_new(list->len);
    for (int64_t i = 0; i < list->len; i++) slots->items[i].i = ZENO_BYTES(list)[i];
    slots->len = list->len;
    zeno_list_sort_by(slots, keys, kind, line, col);
    for (int64_t i = 0; i < list->len; i++) ZENO_BYTES(list)[i] = (uint8_t)slots->items[i].i;
    zeno_list_drop(slots);
}

static zeno_list *zeno_bytes_slice(zeno_list *list, int64_t from, int64_t to, bool to_end, int line, int col) {
    zeno_slice_ends(from, &to, to_end, list->len, "list", line, col);
    zeno_list *slice = zeno_bytes_new(to - from);
    if (to > from) memcpy(slice->items, ZENO_BYTES(list) + from, (size_t)(to - from));
    slice->len = to - from;
    return slice;
}

/* No sum of bytes can overflow an int: a list would need more than
 * 2^55 of them. */
static int64_t zeno_bytes_sum(zeno_list *list) {
    int64_t total = 0;
    for (int64_t i = 0; i < list->len; i++) total += ZENO_BYTES(list)[i];
    return total;
}

static int64_t zeno_bytes_best(zeno_list *list, bool smallest, const char *name, int line, int col) {
#ifndef ZENO_MINIMAL
    zeno_list_not_empty(list, name, line, col);
#else /* ZENO_MINIMAL */
    (void)name;
#endif /* ZENO_MINIMAL */
    int64_t best = ZENO_BYTES(list)[0];
    for (int64_t i = 1; i < list->len; i++) {
        int64_t item = ZENO_BYTES(list)[i];
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
        if (p > start) zeno_list_append(parts, (zeno_slot){.s = zeno_piece(start, (size_t)(p - start))});
    }
    return parts;
}

/* `text.split(",")`: the pieces of `text` between each `separator`. */
static zeno_list *zeno_split(const char *text, const char *separator, int line, int col) {
    size_t step = strlen(separator);
#ifndef ZENO_MINIMAL
    if (step == 0) zeno_fail("ValueError", "cannot split on an empty str", line, col);
#endif /* ZENO_MINIMAL */
    zeno_list *parts = zeno_list_new(0);
    const char *start = text;
    for (;;) {
        const char *found = strstr(start, separator);
        size_t length = found != NULL ? (size_t)(found - start) : strlen(start);
        zeno_list_append(parts, (zeno_slot){.s = zeno_piece(start, length)});
        if (found == NULL) break;
        start = found + step;
    }
    return parts;
}

/* `text.lines()`: the lines of `text`. A line ends at "\n", and a "\r"
 * just before that "\n" (a file from Windows) goes with it. The last
 * line need not end, and text that ends in "\n" has no empty line after
 * it: "a\nb\n" and "a\nb" are both the two lines "a" and "b". */
static zeno_list *zeno_lines(const char *text) {
    zeno_list *lines = zeno_list_new(0);
    const char *start = text;
    while (*start != '\0') {
        const char *end = strchr(start, '\n');
        size_t length = end != NULL ? (size_t)(end - start) : strlen(start);
        const char *next = start + length + (end != NULL ? 1 : 0);
        if (end != NULL && length > 0 && start[length - 1] == '\r') length--;
        zeno_list_append(lines, (zeno_slot){.s = zeno_piece(start, length)});
        start = next;
    }
    return lines;
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

/* `text.find(part)`: where `part` first begins in `text`, counted in
 * characters, or -1 when it is not there. */
static int64_t zeno_find(const char *text, const char *part) {
    const char *found = strstr(text, part);
    if (found == NULL) return -1;
    int64_t position = 0;
    for (const char *p = text; p < found; p++) {
        if (((unsigned char)*p & 0xC0) != 0x80) position++;
    }
    return position;
}

/* `text.slice(from, to)`: the characters from position `from` up to
 * `to`. Positions count characters, so each end is found by walking. */
static const char *zeno_slice(const char *text, int64_t from, int64_t to, bool to_end, int line, int col) {
    zeno_slice_ends(from, &to, to_end, zeno_len(text), "str", line, col);
    const char *start = text;
    for (int64_t i = 0; i < from; i++) start += zeno_char_size(start);
    const char *end = start;
    for (int64_t i = from; i < to; i++) end += zeno_char_size(end);
    return zeno_copy(start, (size_t)(end - start));
}

/* `"A".code()`: the number Unicode gives to the one character in `text`.
 *
 * UTF-8 spreads the bits of that number over one to four bytes. The
 * first byte says how many there are by how many 1s it begins with, and
 * each byte after it begins with 10 and carries six bits:
 *
 *     0xxxxxxx                              up to 127
 *     110xxxxx 10xxxxxx                     up to 2047
 *     1110xxxx 10xxxxxx 10xxxxxx            up to 65535
 *     11110xxx 10xxxxxx 10xxxxxx 10xxxxxx   the rest
 */
static int64_t zeno_char_code(const char *text, int line, int col) {
#ifndef ZENO_MINIMAL
    if (zeno_len(text) != 1) {
        zeno_fail("ValueError",
                  zeno_join(2, "`code` needs a str of exactly one character, and this one has ",
                            zeno_str_int(zeno_len(text))),
                  line, col);
    }
#endif /* ZENO_MINIMAL */
    const unsigned char *p = (const unsigned char *)text;
    if (p[0] < 0x80) return p[0];
    if (p[0] < 0xE0) return ((int64_t)(p[0] & 0x1F) << 6) | (p[1] & 0x3F);
    if (p[0] < 0xF0) return ((int64_t)(p[0] & 0x0F) << 12) | ((int64_t)(p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    return ((int64_t)(p[0] & 0x07) << 18) | ((int64_t)(p[1] & 0x3F) << 12) | ((int64_t)(p[2] & 0x3F) << 6)
           | (p[3] & 0x3F);
}

/* `char(65)`: the other way round. 0 is refused, because a zero byte is
 * where text ends. So are the numbers from 0xD800 to 0xDFFF, which
 * Unicode keeps free, and anything above its last character. */
static const char *zeno_char(int64_t code, int line, int col) {
#ifndef ZENO_MINIMAL
    if (code < 1 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
        zeno_fail("ValueError", zeno_join(2, zeno_str_int(code), " is not the code of a character"), line, col);
    }
#endif /* ZENO_MINIMAL */
    char bytes[4];
    size_t size;
    if (code < 0x80) {
        bytes[0] = (char)code;
        size = 1;
    } else if (code < 0x800) {
        bytes[0] = (char)(0xC0 | (code >> 6));
        bytes[1] = (char)(0x80 | (code & 0x3F));
        size = 2;
    } else if (code < 0x10000) {
        bytes[0] = (char)(0xE0 | (code >> 12));
        bytes[1] = (char)(0x80 | ((code >> 6) & 0x3F));
        bytes[2] = (char)(0x80 | (code & 0x3F));
        size = 3;
    } else {
        bytes[0] = (char)(0xF0 | (code >> 18));
        bytes[1] = (char)(0x80 | ((code >> 12) & 0x3F));
        bytes[2] = (char)(0x80 | ((code >> 6) & 0x3F));
        bytes[3] = (char)(0x80 | (code & 0x3F));
        size = 4;
    }
    return zeno_copy(bytes, size);
}

/* -------------------------------------------------------------------- Files
 *
 * The `files` module: a whole file read as text, text written to a file,
 * and two questions about a path. Each function has a twin in
 * builtins.rs, for `zeno run`, and the two are written to give the same
 * answer and the same message in every case.
 *
 * A file is opened in binary mode ("rb", "wb"), so that the bytes of the
 * text are the bytes of the file on every system.
 */

/* What is at a path: 0 for nothing, 1 for a file that can be read, and 2
 * for anything else (a folder, or a file this program may not read).
 * A folder can be opened on some systems and only fails when it is read,
 * so one byte is read to find out. */
static int zeno_file_at(const char *path) {
    FILE *file = fopen(path, "rb");
    if (file == NULL) return errno == ENOENT ? 0 : 2;
    int first = fgetc(file);
    int at = (first == EOF && ferror(file) != 0) ? 2 : 1;
    (void)fclose(file);
    return at;
}

/* Is this text? Text is well-formed UTF-8 with no zero byte in it, since
 * a zero is where a str ends.
 *
 * The first byte of a character says how many bytes follow, and each of
 * those must look like 10xxxxxx. For four first bytes the second byte is
 * held tighter still (`low` and `high`), which rules out a character
 * written in more bytes than it needs, the numbers Unicode keeps free,
 * and anything past its last character. */
static bool zeno_is_text(const unsigned char *bytes, size_t size) {
    size_t i = 0;
    while (i < size) {
        unsigned char first = bytes[i];
        size_t more;
        unsigned char low = 0x80;
        unsigned char high = 0xBF;
        if (first == 0) return false;
        if (first < 0x80) {
            i++;
            continue;
        }
        if (first >= 0xC2 && first <= 0xDF) {
            more = 1;
        } else if (first == 0xE0) {
            more = 2;
            low = 0xA0;
        } else if (first == 0xED) {
            more = 2;
            high = 0x9F;
        } else if (first >= 0xE1 && first <= 0xEF) {
            more = 2;
        } else if (first == 0xF0) {
            more = 3;
            low = 0x90;
        } else if (first >= 0xF1 && first <= 0xF3) {
            more = 3;
        } else if (first == 0xF4) {
            more = 3;
            high = 0x8F;
        } else {
            return false;
        }
        if (size - i <= more) return false;
        if (bytes[i + 1] < low || bytes[i + 1] > high) return false;
        for (size_t k = 2; k <= more; k++) {
            if ((bytes[i + k] & 0xC0) != 0x80) return false;
        }
        i += more + 1;
    }
    return true;
}

/* The three below do the work. Each gives back NULL when all went well,
 * or the message of the FileError. */

static const char *zeno_file_read(const char *path, const char **result) {
    int at = zeno_file_at(path);
    if (at == 0) return zeno_join(3, "there is no file `", path, "`");
    FILE *file = at == 1 ? fopen(path, "rb") : NULL;
    /* How long it is: go to the end, and ask where that is. */
    long size = -1;
    if (file != NULL && fseek(file, 0, SEEK_END) == 0) size = ftell(file);
    if (file == NULL || size < 0 || fseek(file, 0, SEEK_SET) != 0) {
        if (file != NULL) (void)fclose(file);
        return zeno_join(3, "cannot read `", path, "`");
    }
    char *text = zeno_alloc_text((size_t)size + 1);
    size_t got = fread(text, 1, (size_t)size, file);
    (void)fclose(file);
    if (got != (size_t)size) return zeno_join(3, "cannot read `", path, "`");
    text[size] = '\0';
    if (!zeno_is_text((const unsigned char *)text, (size_t)size)) {
        return zeno_join(3, "`", path, "` does not hold text");
    }
    *result = text;
    return NULL;
}

/* The bytes of a file, one to an item of a list[u8] (decisions 36.1, 36.2). */
static const char *zeno_file_read_bytes(const char *path, zeno_list **result) {
    int at = zeno_file_at(path);
    if (at == 0) return zeno_join(3, "there is no file `", path, "`");
    FILE *file = at == 1 ? fopen(path, "rb") : NULL;
    long size = -1;
    if (file != NULL && fseek(file, 0, SEEK_END) == 0) size = ftell(file);
    if (file == NULL || size < 0 || fseek(file, 0, SEEK_SET) != 0) {
        if (file != NULL) (void)fclose(file);
        return zeno_join(3, "cannot read `", path, "`");
    }
    /* The bytes go straight into the list, which has room for them all. */
    zeno_list *bytes = zeno_bytes_new((int64_t)size);
    size_t got = size > 0 ? fread(bytes->items, 1, (size_t)size, file) : 0;
    (void)fclose(file);
    if (got != (size_t)size) return zeno_join(3, "cannot read `", path, "`");
    bytes->len = (int64_t)size;
    *result = bytes;
    return NULL;
}

/* A list[u8] written out, one byte an item. Its items fit a byte: the
 * checker saw to that. */
static const char *zeno_file_put_bytes(const char *path, zeno_list *bytes) {
    FILE *file = fopen(path, "wb");
    bool good = file != NULL && (bytes->len == 0 || fwrite(bytes->items, 1, (size_t)bytes->len, file) == (size_t)bytes->len);
    if (file != NULL && fclose(file) != 0) good = false;
    return good ? NULL : zeno_join(3, "cannot write `", path, "`");
}

/* `mode` is "wb" to replace what the file held, and "ab" to add to it. */
static const char *zeno_file_put(const char *path, const char *text, const char *mode) {
    FILE *file = fopen(path, mode);
    size_t size = strlen(text);
    bool good = file != NULL && fwrite(text, 1, size, file) == size;
    if (file != NULL && fclose(file) != 0) good = false;
    return good ? NULL : zeno_join(3, "cannot write `", path, "`");
}

static const char *zeno_file_remove(const char *path) {
    int at = zeno_file_at(path);
    if (at == 0) return zeno_join(3, "there is no file `", path, "`");
    if (at == 2 || remove(path) != 0) return zeno_join(3, "cannot remove `", path, "`");
    return NULL;
}

static bool zeno_files_exists(const char *path) {
    return zeno_file_at(path) == 1;
}

/* What the compiled program calls. A file can be missing however careful
 * the program is, so a FileError is raised as an error that can be
 * caught in every kind of build: by a jump in an ordinary one, and as a
 * value in a firmware one, like `int("x")`. */
#ifndef ZENO_FIRMWARE

static void zeno_file_done(const char *wrong, int line, int col) {
    if (wrong != NULL) zeno_raise(zeno_error_new("FileError", wrong, NULL), line, col);
}

static const char *zeno_files_read(const char *path, int line, int col) {
    const char *text = "";
    zeno_file_done(zeno_file_read(path, &text), line, col);
    return text;
}

static void zeno_files_write(const char *path, const char *text, int line, int col) {
    zeno_file_done(zeno_file_put(path, text, "wb"), line, col);
}

static void zeno_files_append(const char *path, const char *text, int line, int col) {
    zeno_file_done(zeno_file_put(path, text, "ab"), line, col);
}

static void zeno_files_remove(const char *path, int line, int col) {
    zeno_file_done(zeno_file_remove(path), line, col);
}

static zeno_list *zeno_files_read_bytes(const char *path, int line, int col) {
    zeno_list *bytes = NULL;
    zeno_file_done(zeno_file_read_bytes(path, &bytes), line, col);
    return bytes != NULL ? bytes : zeno_bytes_new(0);
}

static void zeno_files_write_bytes(const char *path, zeno_list *bytes, int line, int col) {
    zeno_file_done(zeno_file_put_bytes(path, bytes), line, col);
}

#else /* ZENO_FIRMWARE */

static bool zeno_file_done(const char *wrong, int line, int col) {
    if (wrong == NULL) return true;
    zeno_note(zeno_error_new("FileError", wrong, NULL), line, col);
    return false;
}

static bool zeno_files_read(const char *path, int line, int col, const char **result) {
    return zeno_file_done(zeno_file_read(path, result), line, col);
}

static bool zeno_files_write(const char *path, const char *text, int line, int col) {
    return zeno_file_done(zeno_file_put(path, text, "wb"), line, col);
}

static bool zeno_files_append(const char *path, const char *text, int line, int col) {
    return zeno_file_done(zeno_file_put(path, text, "ab"), line, col);
}

static bool zeno_files_remove(const char *path, int line, int col) {
    return zeno_file_done(zeno_file_remove(path), line, col);
}

static bool zeno_files_read_bytes(const char *path, int line, int col, zeno_list **result) {
    return zeno_file_done(zeno_file_read_bytes(path, result), line, col);
}

static bool zeno_files_write_bytes(const char *path, zeno_list *bytes, int line, int col) {
    return zeno_file_done(zeno_file_put_bytes(path, bytes), line, col);
}

#endif /* ZENO_FIRMWARE */

/* ------------------------------------- The keyboard and the command line
 *
 * `input()` reads one line of standard input: the keyboard, or a file
 * that was piped in. `program.arguments()` gives the words the program
 * was started with. Both come from the C library, so a board gets them
 * from whatever its C library reads: a serial line, as a rule. */

/* A line that is being read grows by a byte at a time, and nobody knows
 * how long it will be. `line` is where it begins, `used` how many bytes
 * it has, and `room` how many it has room for. */
#ifdef ZENO_NO_COLLECTOR
/* With no collector the line grows in the text room, which is a stack:
 * each byte goes on top of the one before, so the line is in one piece
 * and takes exactly its own length. */
static void zeno_line_add(char **line, size_t *used, size_t *room, char c) {
    char *top = zeno_alloc_text(1);
    if (*line == NULL) *line = top;
#ifdef ZENO_HEAP
    /* A text room that grows is a chain of rooms. If this byte had to
     * start the next room, the line so far moves there, to stay in one
     * piece. (What it leaves behind is given back with the statement.) */
    if (top != *line + *used) {
        zeno_text_top = zeno_text_place(top);
        char *moved = zeno_alloc_text(*used + 1);
        memcpy(moved, *line, *used);
        *line = moved;
        top = moved + *used;
    }
#endif /* ZENO_HEAP */
    *top = c;
    (*used)++;
    (void)room;
}
#else /* ZENO_NO_COLLECTOR */
/* With a collector the line is a block. When it is full it moves to a
 * block twice the size, and the collector takes the old one. Doubling
 * keeps the copying small: all the moves of a line together copy fewer
 * bytes than the line is long. */
static void zeno_line_add(char **line, size_t *used, size_t *room, char c) {
    if (*used == *room) {
        *room = *room == 0 ? 64 : *room * 2;
        char *more = zeno_alloc_text(*room);
        if (*used > 0) memcpy(more, *line, *used);
        *line = more;
    }
    (*line)[(*used)++] = c;
}
#endif /* ZENO_NO_COLLECTOR */

/* `input()`, `input("Name: ")`: the next line, without its line end,
 * or NULL (which is `none`) when there are no more lines. A "\r" just
 * before the "\n" goes with it, as in `lines()`. A zero byte is left
 * out, because text ends at one. */
static const char *zeno_input(const char *question) {
    if (question != NULL) {
        fputs(question, stdout);
        fflush(stdout); /* on the screen before the program waits */
    }
    char *line = NULL;
    size_t used = 0;
    size_t room = 0;
    bool carriage = false; /* a "\r" is held back until the next byte is seen */
    bool got = false;      /* was there anything at all, even an empty line? */
    for (;;) {
        int c = fgetc(stdin);
        if (c == '\n') {
            got = true;
            break;
        }
        if (carriage) {
            zeno_line_add(&line, &used, &room, '\r'); /* it was not before a "\n": it stays */
            carriage = false;
        }
        if (c == EOF) break;
        got = true;
        if (c == '\r') {
            carriage = true;
        } else if (c != '\0') {
            zeno_line_add(&line, &used, &room, (char)c);
        }
    }
    if (!got) return NULL;
    zeno_line_add(&line, &used, &room, '\0');
    return line;
}

/* What `main` was given, kept for `program.arguments()`. */
static int zeno_argc = 0;
static char **zeno_argv = NULL;

static void zeno_program_start(int argc, char **argv) {
    zeno_argc = argc;
    zeno_argv = argv;
}

/* On Windows the words that `main` is given have been through the
 * machine's "code page", an older way of writing text that holds a few
 * hundred characters: which ones depends on the country the machine was
 * set up for. A word in Tamil on a machine set up for English arrives as
 * question marks. Windows itself keeps the command line whole, two bytes
 * or four to a character (UTF-16), and will hand it over and cut it into
 * words. Each is turned into UTF-8 here, which is what Zeno text is.
 *
 * The functions are declared by hand for the reason `Sleep` is, above.
 * 65001 is Windows' number for UTF-8. The one that cuts the line into
 * words lives in a library that Visual Studio's linker has to be told
 * of, which the `#pragma` does. gcc on Windows links it unasked. */
#ifdef _WIN32
#ifdef _MSC_VER
#pragma comment(lib, "shell32")
#endif
__declspec(dllimport) wchar_t *__stdcall GetCommandLineW(void);
__declspec(dllimport) wchar_t **__stdcall CommandLineToArgvW(const wchar_t *line, int *count);
__declspec(dllimport) void *__stdcall LocalFree(void *memory);
__declspec(dllimport) int __stdcall WideCharToMultiByte(unsigned int page, unsigned long flags, const wchar_t *wide,
                                                        int wide_length, char *bytes, int room, const char *instead,
                                                        int *used_instead);
#endif

/* `program.arguments()`: a new list of the words after the program's
 * own name. */
static zeno_list *zeno_program_arguments(void) {
    zeno_list *words = zeno_list_new(0);
#ifdef _WIN32
    int count = 0;
    wchar_t **wide = CommandLineToArgvW(GetCommandLineW(), &count);
    if (wide != NULL) {
        for (int i = 1; i < count; i++) {
            /* Asked with no room, it says how much room the word needs,
             * the zero at its end included. */
            int room = WideCharToMultiByte(65001, 0, wide[i], -1, NULL, 0, NULL, NULL);
            if (room < 1) room = 1;
            char word[room];
            word[0] = '\0';
            WideCharToMultiByte(65001, 0, wide[i], -1, word, room, NULL, NULL);
            zeno_list_append(words, (zeno_slot){.s = zeno_piece(word, strlen(word))});
        }
        LocalFree(wide);
        return words;
    }
    /* Windows could not say: `main`'s own words are better than none. */
#endif
    for (int i = 1; i < zeno_argc; i++) {
        zeno_list_append(words, (zeno_slot){.s = zeno_piece(zeno_argv[i], strlen(zeno_argv[i]))});
    }
    return words;
}

/* `program.exit(code)`: the program ends here, and whatever started it
 * is given `code`. `exit` sends what was printed first, and runs what
 * was asked to run at the end (the memory report). */
static void zeno_program_exit(int64_t code, int line, int col) {
#ifndef ZENO_MINIMAL
    if (code < 0 || code > 255) {
        zeno_fail("ValueError", zeno_join(2, "a program ends with a number from 0 to 255, not ", zeno_str_int(code)), line,
                  col);
    }
#endif /* ZENO_MINIMAL */
    exit((int)code);
}

/* `program.environment(name)`: a copy of what the environment says,
 * or NULL, which is `none`. The copy is the program's to keep: the
 * environment's own text can change under it. */
static const char *zeno_program_environment(const char *name) {
    const char *value = getenv(name);
    if (value == NULL) return NULL;
    return zeno_copy(value, strlen(value));
}

/* `program.warn(text)`: one line on the error stream, after what was
 * printed so far, so that the two keep their order on a screen. */
static void zeno_program_warn(const char *text) {
    fflush(stdout);
    fputs(text, stderr);
    fputc('\n', stderr);
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

/* What `find` found: a position, or `none` where the runtime answered -1. */
static zeno_opt_int zeno_found(int64_t at) {
    return (zeno_opt_int){at >= 0, at >= 0 ? at : 0};
}

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

/* -------------------------------------------------------------------- Dicts
 *
 * A dict keeps its keys in one list and its values in another, side by
 * side, in the order the keys were put in: values->items[i] belongs to
 * keys->items[i]. That order is what a loop walks and what `print` shows.
 *
 * Finding a key by walking the list would take longer the bigger the dict
 * got. So beside the two lists there is an index: a table of positions
 * with at least twice as many places as there are keys. A key's hash (a
 * number worked out from the key) says where in the table to start
 * looking. From there the search steps forward until it meets the key or
 * an empty place. With the table at most half full that is a step or two
 * on average, whether the dict holds ten keys or ten thousand.
 *
 *     keys     "Ada"   "Alan"   "Grace"
 *     values    36      41       45
 *     index    [0, 3, 0, 0, 1, 0, 2, 0]     a place holds position + 1,
 *                                           and 0 means "empty"
 *
 * The dict knows what its keys and its values are ('s' text, 'i' whole
 * number, 'f' float, 'b' bool), so it can compare and show them itself.
 *
 * With no collector the dict has a copy of its own of every key and
 * value that is text, like a list of text, and frees them when it ends
 * (zeno_dict_drop). A value arrives already copied by whoever stores it,
 * exactly as an item arrives at a list. A key is copied here, and only
 * when it is new: storing under a key the dict already has needs no
 * second copy. */

typedef struct {
    zeno_list *keys;
    zeno_list *values;
    int64_t *index; /* position + 1 of a key in `keys`, or 0 for an empty place */
    int64_t room;   /* how many places `index` has: 0, or a power of two */
    char key_kind;
    char value_kind;
} zeno_dict;

static zeno_dict *zeno_dict_new(int key_kind, int value_kind) {
    zeno_dict *dict = (zeno_dict *)zeno_alloc(sizeof(zeno_dict));
    dict->keys = zeno_list_new(0);
    dict->values = zeno_list_new(0);
    dict->index = NULL;
    dict->room = 0;
    dict->key_kind = (char)key_kind;
    dict->value_kind = (char)value_kind;
    return dict;
}

/* `[:]` is made before the place it arrives at has said what it will
 * hold. This is that place saying so. */
static zeno_dict *zeno_dict_kinds(zeno_dict *dict, int key_kind, int value_kind) {
    dict->key_kind = (char)key_kind;
    dict->value_kind = (char)value_kind;
    return dict;
}

static uint64_t zeno_dict_hash(const zeno_dict *dict, zeno_slot key) {
    if (dict->key_kind == 's') {
        /* FNV-1a: fold in one byte at a time, multiplying by a prime in
         * between so that every byte ends up changing every bit. */
        uint64_t hash = UINT64_C(14695981039346656037);
        for (const unsigned char *p = (const unsigned char *)key.s; *p != '\0'; p++) {
            hash = (hash ^ *p) * UINT64_C(1099511628211);
        }
        return hash;
    }
    /* Whole numbers are often 1, 2, 3 or multiples of something. One
     * multiplication by a big odd number spreads them over the table. */
    uint64_t hash = (uint64_t)key.i * UINT64_C(0x9E3779B97F4A7C15);
    return hash ^ (hash >> 32);
}

static bool zeno_dict_same_key(const zeno_dict *dict, zeno_slot a, zeno_slot b) {
    return dict->key_kind == 's' ? strcmp(a.s, b.s) == 0 : a.i == b.i;
}

/* The place in the index that holds `key`, or the empty place where it
 * would go. The index must have room (it is never full, so this ends). */
static int64_t zeno_dict_place(const zeno_dict *dict, zeno_slot key) {
    uint64_t mask = (uint64_t)dict->room - 1;
    uint64_t place = zeno_dict_hash(dict, key) & mask;
    while (dict->index[place] != 0 &&
           !zeno_dict_same_key(dict, dict->keys->items[dict->index[place] - 1], key)) {
        place = (place + 1) & mask;
    }
    return (int64_t)place;
}

/* Where `key` is in the two lists, or -1 if the dict does not have it. */
static int64_t zeno_dict_find(zeno_dict *dict, zeno_slot key) {
    if (dict->room == 0) return -1;
    return dict->index[zeno_dict_place(dict, key)] - 1;
}

/* Write the index again from the list of keys, with `room` places. */
static void zeno_dict_index(zeno_dict *dict, int64_t room) {
    if (room != dict->room) {
        zeno_free(dict->index);
        dict->index = (int64_t *)zeno_alloc((size_t)room * sizeof(int64_t));
        dict->room = room;
    }
    if (room > 0) memset(dict->index, 0, (size_t)room * sizeof(int64_t));
    for (int64_t i = 0; i < dict->keys->len; i++) {
        dict->index[zeno_dict_place(dict, dict->keys->items[i])] = i + 1;
    }
}

/* `dict[key] = value`. A key the dict has gets the new value and stays
 * where it is in the order. A new key goes at the end. */
static void zeno_dict_set(zeno_dict *dict, zeno_slot key, zeno_slot value) {
    int64_t at = zeno_dict_find(dict, key);
    if (at >= 0) {
#ifdef ZENO_NO_COLLECTOR
        if (dict->value_kind == 's') zeno_text_drop(dict->values->items[at].s);
#endif /* ZENO_NO_COLLECTOR */
        dict->values->items[at] = value;
        return;
    }
#ifdef ZENO_NO_COLLECTOR
    if (dict->key_kind == 's') key.s = zeno_text_keep(key.s);
#endif /* ZENO_NO_COLLECTOR */
    zeno_list_append(dict->keys, key);
    zeno_list_append(dict->values, value);
    if (dict->keys->len * 2 > dict->room) {
        /* More than half full: twice the places, and every key finds
         * its place again. */
        zeno_dict_index(dict, dict->room == 0 ? 8 : dict->room * 2);
    } else {
        dict->index[zeno_dict_place(dict, key)] = dict->keys->len;
    }
}

/* `["Ada": 36, "Alan": 41]`: `count` keys, each followed by its value. */
static zeno_dict *zeno_dict_of(int key_kind, int value_kind, int count, ...) {
    zeno_dict *dict = zeno_dict_new(key_kind, value_kind);
    va_list entries;
    va_start(entries, count);
    for (int i = 0; i < count; i++) {
        zeno_slot key = va_arg(entries, zeno_slot);
        zeno_slot value = va_arg(entries, zeno_slot);
        zeno_dict_set(dict, key, value);
    }
    va_end(entries);
    return dict;
}

/* One key or value as `print` shows it inside a dict. */
static const char *zeno_dict_show(char kind, zeno_slot item) {
    switch (kind) {
    case 's': return zeno_repr_str(item.s);
    case 'f': return zeno_str_float(item.f);
    case 'b': return zeno_str_bool(item.b);
    default: return zeno_str_int(item.i);
    }
}

__attribute__((noreturn)) static void zeno_dict_no_key(zeno_dict *dict, zeno_slot key, int line, int col) {
    zeno_fail("KeyError", zeno_join(2, "this dict has no key ", zeno_dict_show(dict->key_kind, key)), line, col);
}

/* `dict[key]`. Asking for a key that is not there is a KeyError, as a
 * position that is not in a list is an IndexError. */
static zeno_slot zeno_dict_at(zeno_dict *dict, zeno_slot key, int line, int col) {
    int64_t at = zeno_dict_find(dict, key);
#ifndef ZENO_MINIMAL
    if (at < 0) zeno_dict_no_key(dict, key, line, col);
#endif /* ZENO_MINIMAL */
    return dict->values->items[at];
}

/* The value at a position that zeno_dict_find gave, and a slot of zeros
 * where it gave -1. `dict.get(key)` is built from the two. */
static zeno_slot zeno_dict_value(zeno_dict *dict, int64_t at) {
    zeno_slot nothing;
    memset(&nothing, 0, sizeof nothing);
    return at < 0 ? nothing : dict->values->items[at];
}

/* `dict.get(key, instead)`. */
static zeno_slot zeno_dict_get(zeno_dict *dict, zeno_slot key, zeno_slot instead) {
    int64_t at = zeno_dict_find(dict, key);
    return at < 0 ? instead : dict->values->items[at];
}

/* `dict.get(key)` where the values are numbers or bools: the value and a
 * flag that says whether there was one. */
static zeno_opt_int zeno_dict_get_int(zeno_dict *dict, zeno_slot key) {
    int64_t at = zeno_dict_find(dict, key);
    return at < 0 ? (zeno_opt_int){false, 0} : (zeno_opt_int){true, dict->values->items[at].i};
}

static zeno_opt_float zeno_dict_get_float(zeno_dict *dict, zeno_slot key) {
    int64_t at = zeno_dict_find(dict, key);
    return at < 0 ? (zeno_opt_float){false, 0.0} : (zeno_opt_float){true, dict->values->items[at].f};
}

static zeno_opt_bool zeno_dict_get_bool(zeno_dict *dict, zeno_slot key) {
    int64_t at = zeno_dict_find(dict, key);
    return at < 0 ? (zeno_opt_bool){false, false} : (zeno_opt_bool){true, dict->values->items[at].b};
}

/* The same where the values are text: NULL is `none`. */
static const char *zeno_dict_get_str(zeno_dict *dict, zeno_slot key) {
    int64_t at = zeno_dict_find(dict, key);
    return at < 0 ? NULL : dict->values->items[at].s;
}

/* `dict.pop(key)`: the key and its value leave the dict, and the value is
 * the answer. Everything after them moves down one place, so the index
 * is written again. With no collector the value's text now belongs to
 * whoever asked (the compiled program wraps this in zeno_text_take). */
static zeno_slot zeno_dict_pop(zeno_dict *dict, zeno_slot key, int line, int col) {
    int64_t at = zeno_dict_find(dict, key);
#ifndef ZENO_MINIMAL
    if (at < 0) zeno_dict_no_key(dict, key, line, col);
#endif /* ZENO_MINIMAL */
    zeno_slot gone = zeno_list_pop_at(dict->keys, at, line, col);
    zeno_slot value = zeno_list_pop_at(dict->values, at, line, col);
#ifdef ZENO_NO_COLLECTOR
    if (dict->key_kind == 's') zeno_text_drop(gone.s);
#else /* ZENO_NO_COLLECTOR */
    (void)gone;
#endif /* ZENO_NO_COLLECTOR */
    zeno_dict_index(dict, dict->room);
    return value;
}

/* `dict.keys()` and `dict.values()`: a new list, which is the caller's.
 * It is also what a `for` loop over a dict walks, so the body of the
 * loop can add to the dict or take from it without confusing the loop. */
static zeno_list *zeno_dict_keys(zeno_dict *dict) {
    zeno_list *keys = zeno_list_copy(dict->keys);
#ifdef ZENO_NO_COLLECTOR
    if (dict->key_kind == 's') zeno_list_keep_text(keys);
#endif /* ZENO_NO_COLLECTOR */
    return keys;
}

static zeno_list *zeno_dict_values(zeno_dict *dict) {
    zeno_list *values = zeno_list_copy(dict->values);
#ifdef ZENO_NO_COLLECTOR
    if (dict->value_kind == 's') zeno_list_keep_text(values);
#endif /* ZENO_NO_COLLECTOR */
    return values;
}

/* `dict.clear()`: the dict holds nothing, and keeps its room. */
static void zeno_dict_clear(zeno_dict *dict) {
#ifdef ZENO_NO_COLLECTOR
    for (int64_t i = 0; i < dict->keys->len; i++) {
        if (dict->key_kind == 's') zeno_text_drop(dict->keys->items[i].s);
        if (dict->value_kind == 's') zeno_text_drop(dict->values->items[i].s);
    }
#endif /* ZENO_NO_COLLECTOR */
    zeno_list_clear(dict->keys);
    zeno_list_clear(dict->values);
    if (dict->room > 0) memset(dict->index, 0, (size_t)dict->room * sizeof(int64_t));
}

/* The end of a dict that has one owner: its owner's block is over, or its
 * owner was given another. NULL means the owner had already given the
 * dict away, and there is nothing to do. */
static void zeno_dict_drop(zeno_dict *dict) {
    if (dict == NULL) return;
    zeno_dict_clear(dict);
    zeno_list_drop(dict->keys);
    zeno_list_drop(dict->values);
    zeno_free(dict->index);
    zeno_free(dict);
}

/* A dict as `print` shows it, which is also how it is written:
 * `["Ada": 36, "Alan": 41]`, and `[:]` when it holds nothing. */
static const char *zeno_dict_str_with(zeno_dict *dict, const char *(*show)(zeno_slot));

static const char *zeno_dict_str(zeno_dict *dict) {
    return zeno_dict_str_with(dict, NULL);
}

/* The same for a dict whose values the runtime cannot show by itself:
 * a list, an object, another dict. `show` is a function the compiler
 * wrote that shows one of them. */
static const char *zeno_dict_str_with(zeno_dict *dict, const char *(*show)(zeno_slot)) {
    if (dict->keys->len == 0) return "[:]";
#ifdef ZENO_NO_COLLECTOR
    size_t start = zeno_text_top;
#endif /* ZENO_NO_COLLECTOR */
    const char *text = "[";
    for (int64_t i = 0; i < dict->keys->len; i++) {
        text = zeno_join(3, text, i > 0 ? ", " : "", zeno_dict_show(dict->key_kind, dict->keys->items[i]));
        zeno_slot value = dict->values->items[i];
        text = zeno_join(3, text, ": ", show != NULL ? show(value) : zeno_dict_show(dict->value_kind, value));
#ifdef ZENO_NO_COLLECTOR
        text = zeno_text_settle(start, text);
#endif /* ZENO_NO_COLLECTOR */
    }
    text = zeno_join(2, text, "]");
#ifdef ZENO_NO_COLLECTOR
    text = zeno_text_settle(start, text);
#endif /* ZENO_NO_COLLECTOR */
    return text;
}

/* Two dicts are equal when they have the same keys with the same values.
 * The order the keys were put in does not count. */
static bool zeno_dict_eq_with(zeno_dict *a, zeno_dict *b, bool (*same_value)(zeno_slot, zeno_slot));

static bool zeno_dict_eq(zeno_dict *a, zeno_dict *b) {
    return zeno_dict_eq_with(a, b, NULL);
}

/* The same for values the runtime cannot compare by itself: `same_value`
 * is a function the compiler wrote that compares two of them. */
static bool zeno_dict_eq_with(zeno_dict *a, zeno_dict *b, bool (*same_value)(zeno_slot, zeno_slot)) {
    if (a->keys->len != b->keys->len) return false;
    for (int64_t i = 0; i < a->keys->len; i++) {
        int64_t at = zeno_dict_find(b, a->keys->items[i]);
        if (at < 0) return false;
        zeno_slot x = a->values->items[i];
        zeno_slot y = b->values->items[at];
        bool same;
        if (same_value != NULL) {
            if (!same_value(x, y)) return false;
            continue;
        }
        switch (a->value_kind) {
        case 's': same = strcmp(x.s, y.s) == 0; break;
        case 'f': same = x.f == y.f; break;
        case 'b': same = x.b == y.b; break;
        default: same = x.i == y.i; break;
        }
        if (!same) return false;
    }
    return true;
}

/* A variable that other code can change may have become `none` again since
 * it was checked. These look once more before the value is used. */
#ifndef ZENO_MINIMAL
__attribute__((noreturn)) static void zeno_gone(int line, int col) {
    zeno_bug("this was checked with `is not none`, but it has been set to `none` since", line, col);
}
#endif /* ZENO_MINIMAL */

static void *zeno_still(const void *pointer, int line, int col) {
#ifndef ZENO_MINIMAL
    if (pointer == NULL) zeno_gone(line, col);
#endif /* ZENO_MINIMAL */
    return (void *)pointer;
}

static int64_t zeno_still_int(zeno_opt_int value, int line, int col) {
#ifndef ZENO_MINIMAL
    if (!value.present) zeno_gone(line, col);
#endif /* ZENO_MINIMAL */
    return value.value;
}

static double zeno_still_float(zeno_opt_float value, int line, int col) {
#ifndef ZENO_MINIMAL
    if (!value.present) zeno_gone(line, col);
#endif /* ZENO_MINIMAL */
    return value.value;
}

static bool zeno_still_bool(zeno_opt_bool value, int line, int col) {
#ifndef ZENO_MINIMAL
    if (!value.present) zeno_gone(line, col);
#endif /* ZENO_MINIMAL */
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
#ifdef ZENO_NO_COLLECTOR
    /* What frees the block this is in. NULL for a function of the program,
     * which is not in a block. */
    void (*drop)(struct zeno_closure *);
#endif /* ZENO_NO_COLLECTOR */
} zeno_closure;

#ifdef ZENO_NO_COLLECTOR

/* With no collector a function value has one owner, which calls this when
 * it is done with it. A function that was written where a value goes is in
 * a block, and says how to free it. A function of the program is not, and
 * is left alone. */
static void zeno_closure_drop(zeno_closure *function) {
    if (function == NULL || function->drop == NULL) return;
    function->drop(function);
}

/* The `drop` of a function value that keeps no text. */
static void zeno_closure_release(zeno_closure *function) {
    zeno_free(function);
}

#endif /* ZENO_NO_COLLECTOR */

static const char *zeno_show_function(zeno_closure *function) {
    if (function->name[0] == '\0') return "<function>";
    return zeno_join(3, "<function ", function->name, ">");
}

/* ------------------------------------------------------------- The program */
