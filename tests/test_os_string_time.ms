import os
import time
import path
import string

print(os.getcwd())
print(time.now())
print(path.join("a", "b", "c"))
print(path.ext("file.txt"))
print(path.base("a/b/c.txt"))
print(path.dir("a/b/c.txt"))
print(string.format("{} + {} = {}", 1, 2, 3))
print(string.repeat("ab", 3))
print(string.reverse("hello"))
print(string.is_alpha("abc"))
print(string.is_digit("123"))
