# test_ternary.ms — 三元表达式
# 覆盖: 03-syntax.md § 三元表达式（优先级 2，右结合）
# 注：含嵌套位置的回归用例（编译器曾缺失两支 POP 导致栈残留）

ok = true

# 基本形式
assert(("yes" if ok else "no") == "yes", "真分支")
assert(("yes" if not ok else "no") == "no", "假分支")

# 右结合嵌套
n = 2
label = "one" if n == 1 else "two" if n == 2 else "many"
assert(label == "two", "右结合嵌套命中第二分支")

n3 = 9
label3 = "one" if n3 == 1 else "two" if n3 == 2 else "many"
assert(label3 == "many", "右结合嵌套命中末分支")

# 三元在元组非首元素（回归：条件残留曾破坏 BuildTuple 操作数）
pair = ("t", "yes" if ok else "no")
assert(pair == ("t", "yes"), "元组第二元素")

# 三元在调用参数（回归：条件残留曾顶替被调函数）
fn pick(v) {
    return v
}
assert(pick("a" if ok else "b") == "a", "调用首参")
fn second(x, y) {
    return y
}
assert(second("x", "a" if ok else "b") == "a", "调用第二参数")

# 三元在列表字面量与二元运算右操作数
assert(["k", 5 if ok else 6] == ["k", 5], "列表第二元素")
assert(10 + (1 if ok else 2) == 11, "二元右操作数")

# 条件为任意真值表达式
assert(("v" if 1 else "w") == "v", "整数条件真")
assert(("v" if "" else "w") == "w", "空串条件假")

# 与语句级 if 的边界：块内表达式语句后跟 if 语句（回归：
# 块内换行被抑制时 if 曾被误并入前一表达式成三元）
fn classify(x) {
    tag = "small"
    if x >= 100 {
        tag = "big"
    }
    return tag
}
assert(classify(5) == "small", "表达式语句后接 if 语句不并入三元")
assert(classify(500) == "big", "if 语句正常分支")

print("ALL PASSED")
