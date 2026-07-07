import json

# 浮点数 round-trip
f = json.parse("3.14")
print(type(f) == "float")
print(json.stringify(f))

# 负零保留（-0.0 字面量，与 0.0 == 相等但 JSON 文本不同）
neg_zero = json.parse("-0.0")
print(json.stringify(neg_zero))

# 超出 i64 的整数退化为 float（与 Python JSON 一致）
big = json.parse("99999999999999999999")
print(type(big) == "float")

# 负数与混合
mixed = json.parse("[-1, 2.5, 1000000]")
print(mixed[0])
print(mixed[1])
print(mixed[2])
