# test_error_defer_interact.ms — 异常与 defer 协作
# 覆盖: task 36/37（05-control-flow.md § defer 与异常交互、异常链）

# defer 在异常传播途中执行
trace = []
fn thrower() {
    defer trace.push("defer-1")
    defer trace.push("defer-2")
    trace.push("body")
    throw "fail"
}
caught = ""
try {
    thrower()
} except as e {
    caught = e.message
    trace.push("caught")
}
assert(caught == "fail", "异常被外层捕获")
assert(trace == ["body", "defer-2", "defer-1", "caught"], "defer LIFO 在异常时执行")

# try 内 return 时 defer 先于 return 生效
order = []
fn ret_defer() {
    defer order.push("d")
    try {
        order.push("t")
        return "v"
    } finally {
        order.push("f")
    }
}
assert(ret_defer() == "v", "返回值传递")
assert(order == ["t", "f", "d"], "finally 先于 defer")

# defer 中抛异常：defer 后接单表达式，经函数调用触发 throw
fn defer_throw() {
    throw "defer-error"
}
fn defer_throws() {
    defer defer_throw()
    throw "original-error"
}
msg2 = ""
try {
    defer_throws()
} except as e {
    msg2 = e.message
}
assert(msg2 == "defer-error", "defer 异常向外传播")

# 嵌套函数各自的 defer 逐层执行
seq = []
fn level_inner() {
    defer seq.push("inner-defer")
    throw "deep"
}
fn level_outer() {
    defer seq.push("outer-defer")
    level_inner()
}
last = ""
try {
    level_outer()
} except as e {
    last = e.message
}
assert(last == "deep", "异常跨两层传播")
assert(seq == ["inner-defer", "outer-defer"], "各层 defer 依序执行")

# 捕获后继续执行（错误恢复）
state = "start"
fn flaky(ok2) {
    if not ok2 {
        throw "flaky-fail"
    }
    return "flaky-ok"
}
try {
    state = flaky(false)
} except as e {
    state = "recovered"
}
assert(state == "recovered", "捕获后恢复执行")

state2 = flaky(true)
assert(state2 == "flaky-ok", "恢复后再次调用成功")

# 循环内 try：每轮异常各自处理
results = []
for i in range(5) {
    try {
        if i % 2 == 0 {
            throw "even"
        }
        results.push(i)
    } except as e {
        results.push(-1)
    }
}
assert(results == [-1, 1, -1, 3, -1], "循环内逐轮捕获")

print("ALL PASSED")
