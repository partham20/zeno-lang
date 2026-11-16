#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    size_t n = argc > 1 ? (size_t)atoll(argv[1]) : 1000000, cap = 16, len = 0;
    char **parts = malloc(n * sizeof *parts);
    for (size_t i = 0; i < n; i++) { char b[24]; int k = snprintf(b, sizeof b, "%zu", i); parts[i] = malloc(k + 1); memcpy(parts[i], b, k + 1); }
    char *text = malloc(cap);
    for (size_t i = 0; i < n; i++) {
        size_t k = strlen(parts[i]);
        while (len + k + 2 > cap) { cap *= 2; text = realloc(text, cap); }
        if (i) text[len++] = ',';
        memcpy(text + len, parts[i], k); len += k;
    }
    text[len] = 0;
    size_t commas = 0;
    for (size_t i = 0; i < len; i++) if (text[i] == ',') commas++;
    printf("%zu %zu\n", len, commas);
    for (size_t i = 0; i < n; i++) free(parts[i]);
    free(parts); free(text);
    return 0;
}
