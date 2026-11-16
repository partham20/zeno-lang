import sys
N = int(sys.argv[1]) if len(sys.argv) > 1 else 32
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
print(fib(N))
