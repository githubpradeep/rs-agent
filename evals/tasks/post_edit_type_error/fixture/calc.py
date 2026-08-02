def add(a, b):
    return a - b  # intentional bug for agent to fix via edit after diagnostics/run

if __name__ == "__main__":
    assert add(2, 3) == 5, "add is wrong"
    print("OK")
