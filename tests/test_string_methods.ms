# 内置 String 方法 - 基础（task 50）
# 覆盖 length/upper/lower/strip/split/join/replace/contains/startswith/endswith。
s = "Hello World"
print(s.lower())
print(s.upper())
print("  trim  ".strip())
print("a,b,c".split(","))
print("-".join(["a", "b", "c"]))
print("hello".replace("l", "r"))
print("hello".contains("ell"))
print("hello".startswith("hel"))
print("hello".endswith("llo"))
print("hello".length())
