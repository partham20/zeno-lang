import sys
sys.setrecursionlimit(10000)
def quicksort(xs, lo, hi):
    while lo < hi:
        pivot = xs[(lo + hi) // 2]
        i, j = lo, hi
        while i <= j:
            while xs[i] < pivot: i += 1
            while xs[j] > pivot: j -= 1
            if i <= j:
                xs[i], xs[j] = xs[j], xs[i]
                i += 1; j -= 1
        if j - lo < hi - i:
            quicksort(xs, lo, j); lo = i
        else:
            quicksort(xs, i, hi); hi = j
n = int(sys.argv[1]) if len(sys.argv) > 1 else 1000000
xs = []
seed = 42
for k in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    xs.append(seed)
quicksort(xs, 0, n - 1)
ok = all(xs[k - 1] <= xs[k] for k in range(1, n))
print("true" if ok else "false", xs[0], xs[n // 2], xs[n - 1])
