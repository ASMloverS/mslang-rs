# test_operators_compare.ms — 比较、逻辑与成员运算符
# 覆盖: task 21/24（02-types.md），01-lexical.md § 比较/逻辑/成员运算符

# 基本比较
assert(1 == 1, "== 相等")
assert(1 != 2, "!= 不等")
assert(2 < 3, "< 小于")
assert(3 <= 3, "<= 小于等于")
assert(4 > 3, "> 大于")
assert((4 >= 5) == false, ">= 结果为 false")

# 跨数值类型比较（int/float 混合）
assert(1 == 1.0, "int 与 float 数值相等")
assert(0.5 < 1, "float 与 int")

# 字符串比较（字典序）
assert("abc" < "abd", "字符串字典序")
assert("a" < "ab", "前缀更短更小")
assert("ABC" < "abc", "大写在前")

# 链式比较
assert(1 < 2 < 3, "链式比较为真")
assert((1 < 2 > 3) == false, "链式比较为假")
assert(1 < 2 == 2, "链式比较混合运算符")

# 逻辑运算与短路求值（右侧不求证无副作用）
calls = 0
fn bump(v) {
    calls += 1
    return v
}
assert((false and bump(true)) == false, "and 假值短路")
assert(calls == 0, "and 短路未调用右操作数")
assert((true or bump(false)) == true, "or 真值短路")
assert(calls == 0, "or 短路未调用右操作数")
assert((true and bump("r")) == "r", "and 返回右操作数值")
assert(calls == 1, "and 右操作数被求值")
assert((false or "rhs") == "rhs", "or 返回右操作数值")
assert((nil and 1) == nil, "nil and 短路返回 nil")
assert((0 or "zero") == "zero", "0 为假触发 or 右侧")

# not
assert(not true == false, "not true")
assert(not 0, "not 0 为真")
assert(not "" , "not 空串为真")
assert(not nil, "not nil 为真")
assert(not not 5, "双重取反")

# and/or 优先级：and 高于 or
assert(true or false and false, "and 先于 or 结合")
assert((true or false) and false == false, "括号改变逻辑结合")

# in 成员判断（字符串子串 + 集合成员，02-types.md）
assert("x" in "ax", "子串存在")
assert(("z" in "ax") == false, "子串不存在")
assert(1 in [1, 2], "list 成员")
assert((5 in [1, 2]) == false, "list 非成员")
assert(2 in (1, 2, 3), "tuple 成员")
assert("a" in {"a": 1}, "dict 键成员")
assert(("z" in {"a": 1}) == false, "dict 键非成员")
assert(3 in {1, 2, 3}, "set 成员")

# is 身份比较（仅堆对象；同一对象为真，相等副本为假）
lst_a = [1, 2]
lst_b = lst_a
assert(lst_a is lst_b, "同一对象")
assert((lst_a is [1, 2]) == false, "相等但不同对象")
assert((not (lst_a is [1, 2])) == true, "not is 等价 is not")

print("ALL PASSED")
