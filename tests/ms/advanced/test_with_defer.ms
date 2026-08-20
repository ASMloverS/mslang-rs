# test_with_defer.ms — with 语句与 defer
# 覆盖: task 36/38（07-advanced.md § defer、with）
# io 副作用：临时文件写 temp 目录（io 模块无删除 API，以覆写空内容收尾）

import io

# defer：LIFO 执行顺序
order = []
fn basic_defer() {
    defer order.push("first")
    defer order.push("second")
    defer order.push("third")
    order.push("body")
}
basic_defer()
assert(order == ["body", "third", "second", "first"], "defer LIFO")

# defer 在 return 之后仍执行
trace = []
fn defer_after_return() {
    defer trace.push("cleanup")
    trace.push("before-return")
    return 42
}
assert(defer_after_return() == 42, "return 值不受 defer 影响")
assert(trace == ["before-return", "cleanup"], "return 后执行 defer")

# defer 捕获定义时刻的值（参数快照）
snaps = []
fn snapshot(v) {
    defer snaps.push(v)
    v = v * 100
    return v
}
assert(snapshot(7) == 700, "函数体修改参数")
assert(snaps == [7], "defer 捕获定义时刻的值")

# 多函数 defer 独立
a_trace = []
b_trace = []
fn fa() {
    defer a_trace.push("a-defer")
    a_trace.push("a-body")
}
fn fb() {
    defer b_trace.push("b-defer")
    b_trace.push("b-body")
}
fa()
fb()
assert(a_trace == ["a-body", "a-defer"] and b_trace == ["b-body", "b-defer"], "函数间 defer 独立")

# with：文件句柄自动关闭（task 46 io）
tmp = "mslang_with_test.txt"
io.write_file(tmp, "payload")
content = ""
with f = io.open(tmp) {
    content = f.read()
}
assert(content == "payload", "with 内读取")
# with 结束后句柄已关闭：再次 read 报错（由 negative 覆盖），这里验证读到了内容
io.write_file(tmp, "")

# with + defer 混合：defer 先注册后于 with 收尾执行
mixed = []
fn with_and_defer(path) {
    defer mixed.push("defer-last")
    with h = io.open(path) {
        mixed.push(h.read())
    }
    mixed.push("after-with")
}
io.write_file(tmp, "inside")
with_and_defer(tmp)
assert(mixed == ["inside", "after-with", "defer-last"], "with 结束后 defer 才收尾")
io.write_file(tmp, "")

# 嵌套 with（两文件）
p1 = "mslang_with_a.txt"
p2 = "mslang_with_b.txt"
io.write_file(p1, "A")
io.write_file(p2, "B")
got = []
with fa2 = io.open(p1) {
    got.push(fa2.read())
    with fb2 = io.open(p2) {
        got.push(fb2.read())
    }
}
assert(got == ["A", "B"], "嵌套 with")
io.write_file(p1, "")
io.write_file(p2, "")

print("ALL PASSED")
