# test_math_ext.ms — math 扩充（task 80）
# 覆盖: 16-stdlib-expansion.md §4.1 值域抽查
# （错误路径 factorial(21)/isqrt(-1)/comb(100,50)/log(8,1) 等为原生 Err，
#   不经 try/except，由 Rust 单测 test_integration_math_ext_error_paths 覆盖）

import math

# 常量
assert(math.tau == 2 * math.pi, "tau = 2π")
assert(math.inf > 1e308, "inf")
assert(math.is_nan(math.nan), "nan 常量")

# 反三角 / 双曲
assert(abs(math.asin(1) - math.pi / 2) < 1e-12, "asin(1) = π/2")
assert(math.acos(1) == 0.0, "acos(1)")
assert(abs(math.atan(1) - math.pi / 4) < 1e-12, "atan(1)")
assert(abs(math.atan2(1, 1) - math.pi / 4) < 1e-12, "atan2(1, 1)")
assert(math.sinh(0) == 0.0, "sinh(0)")
assert(math.cosh(0) == 1.0, "cosh(0)")
assert(math.tanh(0) == 0.0, "tanh(0)")
assert(math.asinh(0) == 0.0, "asinh(0)")
assert(math.acosh(1) == 0.0, "acosh(1)")
assert(math.atanh(0) == 0.0, "atanh(0)")
assert(math.is_nan(math.asin(2)), "asin 域外 NaN")
assert(math.is_nan(math.acosh(0.5)), "acosh 域外 NaN")
assert(math.is_nan(math.atanh(2)), "atanh 域外 NaN")

# 数值
assert(math.cbrt(27) == 3.0, "cbrt(27)")
assert(math.cbrt(-8) == -2.0, "cbrt(-8)")
assert(math.hypot(3, 4) == 5.0, "hypot(3, 4)")
assert(math.trunc(3.7) == 3, "trunc 向零")
assert(math.trunc(-3.7) == -3, "trunc 负数")
assert(math.sign(-2.5) == -1, "sign 负")
assert(math.sign(0) == 0, "sign 零")
assert(math.sign(2.5) == 1, "sign 正")
assert(math.sign(math.nan) == 0, "sign(NaN) = 0")
assert(math.fmod(7, 3) == 1.0, "fmod")
assert(math.fmod(-7, 3) == -1.0, "fmod C 截断语义")
frac, whole = math.modf(1.25)
assert(frac == 0.25, "modf 小数部")
assert(whole == 1.0, "modf 整数部")
assert(math.copysign(3, -1) == -3.0, "copysign")
assert(abs(math.degrees(math.pi) - 180) < 1e-12, "degrees")
assert(abs(math.radians(180) - math.pi) < 1e-12, "radians")

# 整数族
assert(math.gcd(12, 18) == 6, "gcd")
assert(math.gcd(-12, 18) == 6, "gcd 负数取绝对值")
assert(math.gcd(0, 5) == 5, "gcd(0, n)")
assert(math.lcm(4, 6) == 12, "lcm")
assert(math.lcm(0, 5) == 0, "lcm(0, n)")
assert(math.factorial(5) == 120, "factorial")
assert(math.factorial(0) == 1, "factorial(0)")
assert(math.comb(5, 2) == 10, "comb")
assert(math.comb(3, 5) == 0, "comb k>n = 0")
assert(math.perm(5, 2) == 20, "perm")
assert(math.isqrt(17) == 4, "isqrt(17)")
assert(math.isqrt(16) == 4, "isqrt(16)")

# 谓词
assert(math.is_nan(1.0) == false, "is_nan 数值")
assert(math.is_inf(math.inf), "is_inf")
assert(math.is_inf(1.0) == false, "is_inf 有限值")

# log 升级（base 缺省 e；2/10 走精确路径）
assert(math.log(8, 2) == 3.0, "log(8, 2) = 3.0")
assert(math.log(100, 10) == 2.0, "log(100, 10) = 2.0")
assert(math.log(100, 10) == math.log10(100), "log(b=10) = log10")
assert(math.log(math.e) == 1.0, "log 缺省 base e")
assert(math.log(9, 3) == 2.0, "log(9, 3)")

print("ALL PASSED")
