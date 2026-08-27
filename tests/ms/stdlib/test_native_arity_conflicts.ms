# test_native_arity_conflicts.ms — 同名冲突回归（task 80）
# 覆盖: gc.count × string.count 同脚本并存正确（§2.2 同名冲突治理）
# （count 升级 MAX 后各自自校验：gc.count 恰 0 参、string.count 恰 2 参；
#   参数个数违规的原生 Err 用例由 Rust 单测覆盖）

import gc
import string

# gc.count()：0 参
n = gc.count()
assert(n >= 0, "gc.count() 为非负 int")

# string.count(s, sub)：2 参
assert(string.count("aa", "a") == 2, "string.count 2 参")
assert(string.count("aaa", "aa") == 1, "非重叠")
assert(string.count("abc", "") == 0, "空 sub")

# 同脚本交替调用互不干扰
gc.collect()
assert(gc.count() > n, "collect 后 count 递增")
assert(string.count("aaaa", "aa") == 2, "gc 调用后 string.count 仍正确")
assert(gc.count() >= 1, "string.count 调用后 gc.count 仍正确")

# join 同名（path.join MAX × string.join 自校验 2 参）并存
import path
pj = path.join("a", "b")
assert(pj.endswith("b") and pj.length() == 3, "path.join")
assert(string.join("-", ["x", "y"]) == "x-y", "string.join")

print("ALL PASSED")
