#include <stdio.h>
#include <stdlib.h>
typedef struct Node { struct Node *left, *right; } Node;
static Node *make(int d) { Node *n = malloc(sizeof *n); if (d == 0) { n->left = n->right = NULL; } else { n->left = make(d - 1); n->right = make(d - 1); } return n; }
static long check(Node *n) { return n ? 1 + check(n->left) + check(n->right) : 0; }
static void drop(Node *n) { if (n) { drop(n->left); drop(n->right); free(n); } }
int main(int argc, char **argv) {
    int deepest = argc > 1 ? atoi(argv[1]) : 16;
    Node *long_lived = make(deepest);
    for (int depth = 4; depth <= deepest; depth += 2) {
        long iterations = 1L << (deepest - depth + 4), total = 0;
        for (long k = 0; k < iterations; k++) { Node *t = make(depth); total += check(t); drop(t); }
        printf("%ld trees of depth %d check %ld\n", iterations, depth, total);
    }
    printf("long lived tree of depth %d check %ld\n", deepest, check(long_lived));
    drop(long_lived);
    return 0;
}
