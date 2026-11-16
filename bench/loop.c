#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    int64_t n = argc > 1 ? atoll(argv[1]) : 100000000, total = 0;
    for (int64_t i = 0; i < n; i++) total += (i * i) % 7;
    printf("%lld\n", (long long)total);
    return 0;
}
