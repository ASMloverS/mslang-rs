# test_anonymous.ms — 匿名函数
# 覆盖: task 29（04-functions.md § 匿名函数）

# 基本形式与赋值
sq = fn(x) {
    return x * x
}
assert(sq(9) == 81, "匿名函数赋值调用")

# 立即调用（IIFE）
assert((fn(x) { return x + 1 })(41) == 42, "立即调用")

# 多参匿名函数
adder = fn(a, b) {
    return a + b
}
assert(adder(3, 4) == 7, "多参匿名函数")

# 隐式返回表达式体单行
one = fn() { return 1 }
assert(one() == 1, "单行函数体")

# 作为高阶函数实参
apply2 = fn(f, v) {
    return f(f(v))
}
assert(apply2(fn(x) { return x * 2 }, 3) == 12, "匿名实参复合调用")

# 与内置 map/filter 组合
doubled = map(fn(x) { return x * 2 }, [1, 2, 3])
assert(doubled == [2, 4, 6], "map + 匿名函数")
evens = filter(fn(x) { return x % 2 == 0 }, [1, 2, 3, 4])
assert(evens == [2, 4], "filter + 匿名函数")

# 与列表方法组合
mapped = [1, 2, 3].map(fn(x) { return x + 10 })
assert(mapped == [11, 12, 13], "list.map + 匿名函数")
reduced = [1, 2, 3, 4].reduce(fn(a, b) { return a + b }, 0)
assert(reduced == 10, "list.reduce + 匿名函数")

# 匿名函数捕获外层变量（闭包语义）
base = 100
make = fn() {
    return fn(x) {
        return base + x
    }
}
capture = make()
assert(capture(1) == 101, "匿名函数闭包捕获")

# sorted 使用自定义键？规范 sorted(iterable) 无键参，跳过
# 匿名函数递归（经具名绑定）
fact_fn = fn(n) {
    if n <= 1 {
        return 1
    }
    return n * fact_fn(n - 1)
}
assert(fact_fn(5) == 120, "经具名绑定递归")

print("ALL PASSED")
