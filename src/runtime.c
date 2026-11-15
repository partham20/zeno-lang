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

#include <errno.h>
#include <inttypes.h>
#include <math.h>
#ifndef ZENO_FIRMWARE
#include <setjmp.h>
#endif /* ZENO_FIRMWARE */
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
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

#ifndef ZENO_NO_COLLECTOR

static void zeno_out_of_memory(void) {
    fflush(stdout);
    fprintf(stderr, "error: out of memory\n");
    exit(1);
}

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
#define ZENO_POOL 32768
#endif

/* How many sizes there are. The largest holds 16 << 23 bytes, which is
 * more than a pool is likely to be. */
#ifndef ZENO_POOL_STEPS
#define ZENO_POOL_STEPS 24
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

/* Memory for a list, an object or an error. It starts as all zeros. */
static char *zeno_alloc(size_t size) {
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
    memset(memory, 0, room);
#ifdef ZENO_MEMORY_REPORT
    zeno_blocks_made++;
    zeno_blocks_held++;
    zeno_bytes_made += size;
    zeno_in_use += size;
    if (zeno_in_use > zeno_most_in_use) zeno_most_in_use = zeno_in_use;
#endif
    return memory;
}

/* Give a block back. `memory` is what zeno_alloc returned, or NULL, which
 * does nothing. The compiler writes one of these for each block, at the
 * line where its owner ends. If a block ever came here twice, the list of
 * spare blocks would hand it to two owners, so that is looked for. */
static void zeno_free(void *memory) {
    if (memory == NULL) return;
    zeno_spare *block = (zeno_spare *)((char *)memory - sizeof(zeno_head));
    if (block->head.asked == 0) {
        fflush(stdout);
        fprintf(stderr, "error: a block of memory was given back twice\n");
        exit(1);
    }
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
static bool zeno_in_pool(const char *text) {
    return (uintptr_t)text - (uintptr_t)zeno_pool.bytes < (uintptr_t)ZENO_POOL;
}

static bool zeno_in_room(const char *text) {
    return (uintptr_t)text - (uintptr_t)zeno_text_room < (uintptr_t)ZENO_TEXT_ROOM;
}

/* `text` is about to be kept in a variable, a field or a list. Text
 * written in the program can be kept as it is. Anything else will not
 * last, or is somebody else's, so the keeper gets a copy of its own. */
static const char *zeno_text_keep(const char *text) {
    if (text == NULL || !(zeno_in_pool(text) || zeno_in_room(text))) return text;
    size_t size = strlen(text) + 1;
    char *kept = zeno_alloc(size);
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
    bool above = text != NULL && zeno_in_room(text) && (size_t)(text - zeno_text_room) >= mark;
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
    fprintf(stderr, "pool: %zu byte%s taken, in a pool of %d\n", zeno_pool_top, zeno_pool_top == 1 ? "" : "s", (int)ZENO_POOL);
    fprintf(stderr, "text: at most %zu byte%s in use at once, in a room of %d\n", zeno_most_text,
            zeno_most_text == 1 ? "" : "s", (int)ZENO_TEXT_ROOM);
}
#endif

#endif /* ZENO_NO_COLLECTOR */

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

/* A piece of text that a new list is about to hold: one word of
 * `text.split()`, one character of `for c in text`. With a collector it
 * is text like any other. With none, a list keeps its own copies of its
 * items (docs/text.md), so the piece is made in the pool to begin with
 * and never passes through the text room. */
static const char *zeno_piece(const char *text, size_t length) {
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

/* `a << n` and `a >> n`. An int has 64 bits, so `n` is 0 to 63. */
static void zeno_shift_count(int64_t n, int line, int col) {
    if (n < 0 || n > 63) {
        zeno_fail("ValueError",
                  zeno_join(3, "cannot shift by ", zeno_str_int(n), ". An int has 64 bits, so a shift is 0 to 63"),
                  line, col);
    }
}

/* `a << n` is `a` times two, `n` times, and like `*` it must fit. The
 * shift is done on the unsigned number, where C says what happens to the
 * bits that leave. Shifting back gives `a` again unless some did. */
static int64_t zeno_shl(int64_t a, int64_t n, int line, int col) {
    zeno_shift_count(n, line, col);
    int64_t shifted = (int64_t)((uint64_t)a << n);
    if ((shifted < 0 ? ~(~shifted >> n) : shifted >> n) != a) zeno_overflow(line, col);
    return shifted;
}

/* `a >> n` halves `a`, `n` times, rounding down as `//` does. C leaves
 * `>>` on a negative number to the compiler, so that case is done on the
 * number with every bit turned over, which is not negative. */
static int64_t zeno_shr(int64_t a, int64_t n, int line, int col) {
    zeno_shift_count(n, line, col);
    return a < 0 ? ~(~a >> n) : a >> n;
}

/* `u8(n)`: `n` itself if the size can hold it. `size` is the size in
 * words, "a u8", for the message. A sized whole number is an ordinary
 * int64_t that has been through here, or through zeno_wrap. */
static int64_t zeno_fit(int64_t n, int64_t min, int64_t max, const char *size, int line, int col) {
    if (n < min || n > max) {
        zeno_fail("OverflowError",
                  zeno_join(8, zeno_str_int(n), " does not fit in ", size, " (", zeno_str_int(min), " to ",
                            zeno_str_int(max), ")"),
                  line, col);
    }
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
    if (x < 0.0) {
        zeno_fail("ValueError", zeno_join(2, "cannot take the square root of ", zeno_str_float(x)), line, col);
    }
    return sqrt(x);
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
    if (count < 1) {
        zeno_bug(zeno_join(2, "`allocate` needs at least one item, and this asks for ", zeno_str_int(count)), line, col);
    }
#ifdef ZENO_NO_COLLECTOR
    return zeno_alloc((size_t)count * size);
#else /* ZENO_NO_COLLECTOR */
    void *memory = calloc((size_t)count, size);
    if (memory == NULL) zeno_bug("there is no memory left for this `allocate`", line, col);
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
    if (number < 0 || number >= ZENO_PINS) {
        zeno_fail("ValueError",
                  zeno_join(4, "this board has no pin ", zeno_str_int(number), ". Its pins are 0 to ",
                            zeno_str_int(ZENO_PINS - 1)),
                  line, col);
    }
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

#ifdef ZENO_NO_COLLECTOR
/* `text` is the newest thing in the text room, and everything else written
 * since `start` was only needed to build it. Slide it down to `start` and
 * give the rest back. Showing a list joins one item at a time; without
 * this, a list of a hundred items would leave a hundred half-built texts
 * in the room. */
static const char *zeno_text_settle(size_t start, const char *text) {
    size_t size = strlen(text) + 1;
    memmove(zeno_text_room + start, text, size);
    zeno_text_top = start + size;
    return zeno_text_room + start;
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
    if (at < 0 || at > list->len) {
        zeno_fail("IndexError",
                  zeno_join(4, "cannot insert at ", zeno_str_int(at), " in a list of length ",
                            zeno_str_int(list->len)),
                  line, col);
    }
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

/* `slice(from)` and `slice(from, to)` take the items (or characters)
 * from position `from` up to `to`, which is not taken itself, as in
 * `range`. With `to_end` there was no `to`, and the slice goes on to the
 * end. Anything that is not inside what is being sliced is an
 * IndexError. */
static void zeno_slice_ends(int64_t from, int64_t *to, bool to_end, int64_t length, const char *what, int line,
                            int col) {
    if (to_end) *to = length;
    if (from < 0 || *to < from || *to > length) {
        zeno_fail("IndexError",
                  zeno_join(8, "slice ", zeno_str_int(from), " to ", zeno_str_int(*to), " is out of range for a ",
                            what, " of length ", zeno_str_int(length)),
                  line, col);
    }
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
    for (int64_t i = 0; i < list->len; i++) {
        if (i > 0) {
            memcpy(end, separator, gap);
            end += gap;
        }
        size_t size = strlen(list->items[i].s);
        memcpy(end, list->items[i].s, size);
        end += size;
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
    zeno_fail("IndexError",
              zeno_join(4, "index ", zeno_str_int(i), " is out of range for a str of length ",
                        zeno_str_int(zeno_len(text))),
              line, col);
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
        if (p > start) zeno_list_append(parts, (zeno_slot){.s = zeno_piece(start, (size_t)(p - start))});
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
    if (zeno_len(text) != 1) {
        zeno_fail("ValueError",
                  zeno_join(2, "`code` needs a str of exactly one character, and this one has ",
                            zeno_str_int(zeno_len(text))),
                  line, col);
    }
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
    if (code < 1 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
        zeno_fail("ValueError", zeno_join(2, zeno_str_int(code), " is not the code of a character"), line, col);
    }
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

#endif /* ZENO_FIRMWARE */

/* ------------------------------------- The keyboard and the command line
 *
 * `input()` reads one line of standard input: the keyboard, or a file
 * that was piped in. `program.arguments()` gives the words the program
 * was started with. Both come from the C library, so a board gets them
 * from whatever its C library reads: a serial line, as a rule. */

/* `input()`, `input("Name: ")`: the next line, without its line end,
 * or NULL (which is `none`) when there are no more lines. A "\r" just
 * before the "\n" goes with it, as in `lines()`. A zero byte is left
 * out, because text ends at one. */
static const char *zeno_input(const char *question) {
    if (question != NULL) {
        fputs(question, stdout);
        fflush(stdout); /* on the screen before the program waits */
    }
    char piece[128];
    size_t used = 0;
    const char *line = NULL;
    bool carriage = false; /* a "\r" is held back until the next byte is seen */
    bool got = false;      /* was there anything at all, even an empty line? */
    for (;;) {
        /* Room for two more bytes? If not, move the piece to the line. */
        if (used + 2 >= sizeof piece) {
            const char *more = zeno_copy(piece, used);
            line = line == NULL ? more : zeno_join(2, line, more);
            used = 0;
        }
        int c = fgetc(stdin);
        if (c == '\n') {
            got = true;
            break;
        }
        if (carriage) {
            piece[used++] = '\r'; /* it was not before a "\n": it stays */
            carriage = false;
        }
        if (c == EOF) break;
        got = true;
        if (c == '\r') {
            carriage = true;
        } else if (c != '\0') {
            piece[used++] = (char)c;
        }
    }
    if (!got) return NULL;
    const char *rest = zeno_copy(piece, used);
    return line == NULL ? rest : zeno_join(2, line, rest);
}

/* What `main` was given, kept for `program.arguments()`. */
static int zeno_argc = 0;
static char **zeno_argv = NULL;

static void zeno_program_start(int argc, char **argv) {
    zeno_argc = argc;
    zeno_argv = argv;
}

/* `program.arguments()`: a new list of the words after the program's
 * own name. */
static zeno_list *zeno_program_arguments(void) {
    zeno_list *words = zeno_list_new(0);
    for (int i = 1; i < zeno_argc; i++) {
        zeno_list_append(words, (zeno_slot){.s = zeno_piece(zeno_argv[i], strlen(zeno_argv[i]))});
    }
    return words;
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
