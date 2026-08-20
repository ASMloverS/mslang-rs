# test_functions_basic.ms — 函数声明、调用与返回
# 覆盖: task 27（04-functions.md），task 23/24 调用帧

# 无参无返回（隐式 nil）
fn noop() {
    x = 1
}
assert(noop() == nil, "无 return 返回 nil")

# 单参单返回
fn square(x) {
    return x * x
}
assert(square(6) == 36, "单参")

# 多参与默认求值顺序
fn sub3(a, b, c) {
    return a - b - c
}
assert(sub3(10, 3, 2) == 5, "多参左结合")

# 提前 return
fn abs2(v) {
    if v < 0 {
        return -v
    }
    return v
}
assert(abs2(-7) == 7 and abs2(7) == 7, "提前 return")

# 嵌套调用
fn add1(x) {
    return x + 1
}
assert(add1(add1(add1(0))) == 3, "嵌套调用")

# 递归：阶乘与斐波那契
fn fact(n) {
    if n <= 1 {
        return 1
    }
    return n * fact(n - 1)
}
assert(fact(10) == 3628800, "递归阶乘")

fn fib(n) {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
assert(fib(15) == 610, "递归斐波那契")

# 较深递归（调用帧深度限制内：300 层安全，1000 层触发 stack overflow 由 VM 帧上限保护）
fn sum_to(n, acc) {
    if n == 0 {
        return acc
    }
    return sum_to(n - 1, acc + n)
}
assert(sum_to(300, 0) == 45150, "深递归")

# 函数是一等值：赋值、作为参数、作为返回值
fn apply(f, v) {
    return f(v)
}
fn triple(x) {
    return 3 * x
}
alias = triple
assert(alias(5) == 15, "函数赋值给变量")
assert(apply(square, 4) == 16, "函数作为参数")

fn make_adder(n) {
    return fn(x) {
        return x + n
    }
}
add5 = make_adder(5)
assert(add5(10) == 15, "函数作为返回值")

# 参数求值顺序（从左到右）
order = []
fn observe(tag, val) {
    order.push(tag)
    return val
}
fn three(a, b, c) {
    return a + b + c
}
three(observe("a", 1), observe("b", 2), observe("c", 3))
assert(order == ["a", "b", "c"], "参数从左到右求值")

print("ALL PASSED")
