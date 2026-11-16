import sys
N = int(sys.argv[1]) if len(sys.argv) > 1 else 100000000
total = 0
i = 0
while i < N:
    total += (i * i) % 7
    i += 1
print(total)
