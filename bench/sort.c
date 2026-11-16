#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
static void quicksort(int64_t *xs, int64_t lo, int64_t hi) {
    while (lo < hi) {
        int64_t pivot = xs[(lo + hi) / 2], i = lo, j = hi;
        while (i <= j) {
            while (xs[i] < pivot) i++;
            while (xs[j] > pivot) j--;
            if (i <= j) { int64_t t = xs[i]; xs[i] = xs[j]; xs[j] = t; i++; j--; }
        }
        if (j - lo < hi - i) { quicksort(xs, lo, j); lo = i; }
        else { quicksort(xs, i, hi); hi = j; }
    }
}
int main(int argc, char **argv) {
    int64_t n = argc > 1 ? atoll(argv[1]) : 1000000, seed = 42, cap = 8, len = 0;
    int64_t *xs = malloc(cap * sizeof *xs);
    for (int64_t k = 0; k < n; k++) {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        if (len == cap) { cap *= 2; xs = realloc(xs, cap * sizeof *xs); }
        xs[len++] = seed;
    }
    quicksort(xs, 0, n - 1);
    int ok = 1;
    for (int64_t k = 1; k < n; k++) if (xs[k - 1] > xs[k]) ok = 0;
    printf("%s %lld %lld %lld\n", ok ? "true" : "false", (long long)xs[0], (long long)xs[n / 2], (long long)xs[n - 1]);
    free(xs);
    return 0;
}
