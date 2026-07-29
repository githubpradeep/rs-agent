# Broken on purpose — agent should fix the off-by-one.
def add(a, b):
    return a + b + 1


if __name__ == "__main__":
    assert add(2, 3) == 5, add(2, 3)
    print("OK")
