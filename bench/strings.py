import sys
N = int(sys.argv[1]) if len(sys.argv) > 1 else 1000000
parts = []
for i in range(N):
    parts.append(str(i))
text = ",".join(parts)
commas = 0
for ch in text:
    if ch == ",":
        commas += 1
print(len(text), commas)
