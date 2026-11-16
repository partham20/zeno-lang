import sys
N = int(sys.argv[1]) if len(sys.argv) > 1 else 16
class Node:
    __slots__ = ("left", "right")
    def __init__(self, left, right):
        self.left = left; self.right = right
def make(d):
    return Node(None, None) if d == 0 else Node(make(d - 1), make(d - 1))
def check(n):
    return 0 if n is None else 1 + check(n.left) + check(n.right)
deepest = N
long_lived = make(deepest)
depth = 4
while depth <= deepest:
    iterations = 1 << (deepest - depth + 4)
    total = 0
    for k in range(iterations):
        total += check(make(depth))
    print(iterations, "trees of depth", depth, "check", total)
    depth += 2
print("long lived tree of depth", deepest, "check", check(long_lived))
