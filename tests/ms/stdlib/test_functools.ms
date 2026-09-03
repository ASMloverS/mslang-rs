# test_functools.ms — functools 嵌入模块（task 84）
# 覆盖: 验证标准 8（partial 参数顺序、memoize 命中仅执行一次、
#       reduce 与 list.reduce 等价）+ 边界（init/空迭代/arity 阶梯）

import functools

# ---------------------------------------------------------------------------
# partial：args 在前、*more 在后
# ---------------------------------------------------------------------------

add3 = fn(a, b, c) { return a + b + c }

p0 = functools.partial(add3)
assert(p0(1, 2, 3) == 6, "partial 无前缀实参")

p1 = functools.partial(add3, 1)
assert(p1(2, 3) == 6, "partial 1 个前缀实参")

p2 = functools.partial(add3, 1, 2)
assert(p2(3) == 6, "partial 2 个前缀实参")

p3 = functools.partial(add3, 1, 2, 3)
assert(p3() == 6, "partial 齐参调用")

# 参数顺序语义：前缀实参在前
sub = fn(a, b) { return a - b }
psub = functools.partial(sub, 10)
assert(psub(3) == 7, "partial(10)(3) == 10-3")

# 0 参函数
zero = functools.partial(fn() { return "z" })
assert(zero() == "z", "partial 0 参函数")

# 4 参阶梯上界
add4 = fn(a, b, c, d) { return a + b + c + d }
p4 = functools.partial(add4, 1, 2)
assert(p4(3, 4) == 10, "partial 4 参函数")

# arity > 4 → TypeError（语言无调用点展开）
p5 = functools.partial(fn(a, b, c, d, e) { return a + b + c + d + e }, 1)
hit = false
try {
    p5(2, 3, 4, 5)
} except TypeError as e {
    hit = true
}
assert(hit, "partial arity>4 → TypeError")

# ---------------------------------------------------------------------------
# memoize：命中路径副作用仅一次
# ---------------------------------------------------------------------------

calls = [0]
slow = fn(x) {
    calls[0] = calls[0] + 1
    return x * 2
}
fast = functools.memoize(slow)
assert(fast(21) == 42, "memoize 首调（未命中）")
assert(fast(21) == 42, "memoize 命中")
assert(calls[0] == 1, "副作用函数仅执行一次")
assert(fast(20) == 40, "不同参数未命中")
assert(calls[0] == 2, "不同参数触发执行")

# 多参键 tuple(args) 区分
calls2 = [0]
keyed = functools.memoize(fn(a, b) {
    calls2[0] = calls2[0] + 1
    return a * 100 + b
})
assert(keyed(1, 2) == 102, "双参首调")
assert(keyed(2, 1) == 201, "参数序不同不命中")
assert(keyed(1, 2) == 102, "双参命中")
assert(calls2[0] == 2, "双参仅两次执行")

# unhashable 键 → TypeError（dict 行为上抛；原生错误不可捕获，
# 由 heapq.rs 外的 Rust 单测与整体失败语义覆盖，此处不展开）

# 0 参函数（空 tuple 键）
m0 = functools.memoize(fn() { return 7 })
assert(m0() == 7 and m0() == 7, "memoize 0 参")

# ---------------------------------------------------------------------------
# reduce：与 list.reduce 等价
# ---------------------------------------------------------------------------

addf = fn(a, b) { return a + b }
data = [1, 2, 3, 4]

assert(functools.reduce(addf, data) == 10, "reduce 无 init（首元素作种子）")
assert(data.reduce(addf) == 10, "list.reduce 等价")
assert(functools.reduce(addf, data, 100) == 110, "reduce init")
assert(data.reduce(addf, 100) == 110, "list.reduce init 等价")

assert(functools.reduce(fn(a, b) { return a * b }, [2, 3, 4]) == 24, "reduce 乘法")

# init 与空迭代
assert(functools.reduce(addf, [], 5) == 5, "空迭代 + init 返回 init")

# 空迭代无 init → TypeError
hit = false
try {
    functools.reduce(addf, [])
} except TypeError as e {
    hit = true
}
assert(hit, "reduce 空迭代无 init → TypeError")

# 字符串迭代（可迭代对象通用）
assert(functools.reduce(fn(a, b) { return a + b }, "abc", "z") == "zabc", "reduce 字符串")

print("ALL PASSED")
