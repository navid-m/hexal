#!/usr/bin/env python3

with open("hexal_test.txt", "w", encoding="utf-8") as f:
    for i in range(10_000_000):
        f.write(f"This is test line {i + 1}: Hexal performance testing.\n")

print("Created hexal_test.txt with 1,000,000 lines.")
