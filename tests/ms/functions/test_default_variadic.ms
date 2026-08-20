# test_default_variadic.ms — 默认参数与可变参数
# 覆盖: task 31（04-functions.md § 默认参数与可变参数）

# 默认参数：省略与覆盖
fn greet(name, greeting = "hello", punct = "!") {
    return greeting + ", " + name + punct
}
assert(greet("bob") == "hello, bob!", "全部默认")
assert(greet("bob", "hi") == "hi, bob!", "覆盖第一个默认")
assert(greet("bob", "hi", "?") == "hi, bob?", "覆盖全部")

# 默认值须为常量字面量（编译器限制：非字面量默认值报编译错误）
fn scaled(v, factor = 6) {
    return v * factor
}
assert(scaled(2) == 12, "默认值为字面量")

# 可变参数 *args 收集为 list
fn collect(*args) {
    return args
}
assert(collect() == [], "无参空收集")
assert(collect(1, 2, 3) == [1, 2, 3], "收集为列表")
assert(type(collect(1)) == "list", "args 类型为 list")

# 常规参数 + 可变参数
fn head_then_rest(first, *rest) {
    return first * 100 + rest.length()
}
assert(head_then_rest(2, "a", "b", "c") == 203, "定位参 + 收集参")

# 可变参数求和
fn sum_all(*nums) {
    total = 0
    for n in nums {
        total += n
    }
    return total
}
assert(sum_all(1, 2, 3, 4) == 10, "可变参数求和")
assert(sum_all() == 0, "空可变参数")

# 默认参数 + 可变参数混合
fn tagged(tag = "none", *items) {
    return tag + ":" + str(items.length())
}
assert(tagged() == "none:0", "默认 + 空 varargs")
assert(tagged("x", 1, 2) == "x:2", "覆盖默认 + varargs")

# 可变参数传递函数值
fn call_all(*fns) {
    total = 0
    for f in fns {
        total += f()
    }
    return total
}
assert(call_all(fn() { return 1 }, fn() { return 2 }, fn() { return 3 }) == 6, "收集函数值并调用")

print("ALL PASSED")
