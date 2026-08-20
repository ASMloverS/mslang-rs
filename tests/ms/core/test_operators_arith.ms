# test_operators_arith.ms — 算术与位运算
# 覆盖: task 21（02-types.md § 运算），01-lexical.md § 算术运算符

# 整数四则
assert(7 + 3 == 10, "加法")
assert(7 - 3 == 4, "减法")
assert(7 * 3 == 21, "乘法")
assert(7 / 2 == 3.5, "除法产生浮点")
assert(7 // 2 == 3, "整除向下取整")
assert(-7 // 2 == -4, "负数整除向负无穷")
assert(7 % 3 == 1, "取模")
assert(2 ** 10 == 1024, "幂运算")

# 浮点混合
assert(1 + 0.5 == 1.5, "int + float")
assert(2 * 1.5 == 3.0, "int * float")

# 一元运算符
assert(-5 == 0 - 5, "一元负号")

# 位运算（整数）
assert(5 & 3 == 1, "按位与")
assert(5 | 3 == 7, "按位或")
assert(5 ^ 3 == 6, "按位异或")
assert(5 << 2 == 20, "左移")
assert(20 >> 2 == 5, "右移")
assert(~5 == -6, "按位取反")

# 优先级：** 高于一元负号高于 * / 高于 + -
assert(2 + 3 * 4 == 14, "乘法优先于加法")
assert((2 + 3) * 4 == 20, "括号改变优先级")
assert(-2 ** 2 == -(2 ** 2), "一元负号作用于幂结果")
assert(2 * 3 ** 2 == 18, "幂优先于乘法")
assert(1 + 2 << 1 == 6, "移位低于加减")

# 复合赋值
n = 10
n += 5
assert(n == 15, "+=")
n -= 3
assert(n == 12, "-=")
n *= 2
assert(n == 24, "*=")
n //= 5
assert(n == 4, "//=")
n %= 3
assert(n == 1, "%=")
n += 1
n **= 3
assert(n == 8, "**=")
m = 0b1100
m |= 0b0011
assert(m == 15, "|=")
m &= 0b1110
assert(m == 14, "&=")
m ^= 0b1111
assert(m == 1, "^=")
m <<= 3
assert(m == 8, "<<=")
m >>= 2
assert(m == 2, ">>=")

# 赋值表达式返回被赋的值
q = 0
assert((q = 7) == 7, "赋值表达式的值")

print("ALL PASSED")
