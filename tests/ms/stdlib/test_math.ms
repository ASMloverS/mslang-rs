# test_math.ms — math 模块
# 覆盖: task 47（10-builtins.md § math）

import math

# 常量
assert(math.pi > 3.14159 and math.pi < 3.14160, "pi 值域")
assert(math.e > 2.71828 and math.e < 2.71829, "e 值域")

# 幂与根
assert(math.sqrt(16) == 4.0, "sqrt 完全平方")
assert(math.sqrt(2) > 1.41421 and math.sqrt(2) < 1.415, "sqrt(2) 值域")
assert(math.pow(2, 10) == 1024.0, "pow 整数幂")
assert(math.pow(4, 0.5) == 2.0, "pow 分数幂")
assert(math.exp(0) == 1.0, "exp(0)")

# 对数
assert(math.log(math.e) == 1.0, "log(e) = 1")
assert(math.log2(8) == 3.0, "log2")
assert(math.log10(100) == 2.0, "log10")
assert(math.log(100) > 4.605 and math.log(100) < 4.606, "log(100) 值域")

# 三角函数（特殊角）
assert(math.sin(0) == 0.0, "sin(0)")
assert(math.cos(0) == 1.0, "cos(0)")
assert(math.tan(0) == 0.0, "tan(0)")
half_pi = math.pi / 2
assert(math.sin(half_pi) > 0.99999 and math.sin(half_pi) < 1.00001, "sin(π/2) 值域")

# 取整族（round 半远离零）
assert(math.floor(2.7) == 2, "floor 正数")
assert(math.floor(-2.7) == -3, "floor 负数")
assert(math.ceil(2.1) == 3, "ceil 正数")
assert(math.ceil(-2.1) == -2, "ceil 负数")
assert(math.round(2.5) == 3, "round 半向上")
assert(math.round(3.5) == 4, "round 3.5")
assert(math.round(-2.5) == -3, "round 负半远离零")
assert(math.round(2.4) == 2, "round 向下")

# abs
assert(math.abs(-5) == 5, "abs 负整数")
assert(math.abs(5) == 5, "abs 正整数")
assert(math.abs(-2.5) == 2.5, "abs 浮点")

# 全局 min/max（非 math 成员）
assert(min(3, 1, 2) == 1, "全局 min")
assert(max(3, 1, 2) == 3, "全局 max")
assert(min(4, 2, 9) == 2, "min 多参数")

print("ALL PASSED")
