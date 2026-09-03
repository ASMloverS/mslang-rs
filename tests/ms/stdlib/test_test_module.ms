# test_test_module.ms — test 嵌入模块（task 84）
# 覆盖: 验证标准 9（用 test 模块断言 test 模块自身；失败路径以
#       try/except AssertionError 包裹验证）
# 注意：assert_raises 仅捕获 .ms 侧 throw 的异常（原生错误不可捕获）。

import test

# ---------------------------------------------------------------------------
# 成功路径
# ---------------------------------------------------------------------------

test.assert_eq(1, 1, "eq 命中")
test.assert_eq("a", "a")
test.assert_ne(1, 2, "ne 命中")
test.assert_true(1 < 2, "true 命中")
test.assert_false(1 > 2, "false 命中")
test.assert_almost_eq(0.1 + 0.2, 0.3, 1e-9, "浮点 eps 命中")
test.assert_almost_eq(0.1 + 0.2, 0.3, nil, "nil 显式 eps 走 or 回退")
test.assert_almost_eq(1.0, 1.0 + 5e-10, 1e-9, "1e-9 容差内")
test.assert_almost_eq(1.0, 1.05, 0.1, "自定义 eps")
test.assert_len([1, 2, 3], 3, "len 命中")
test.assert_len("hello", 5)
test.assert_contains([1, 2], 1, "contains list")
test.assert_contains("hello", "ell", "contains string")
test.assert_contains({"k": 1}, "k", "contains dict 键")

# assert_raises：捕获路径（类匹配）
test.assert_raises(fn() { throw ValueError("boom") }, "ValueError", "匹配捕获")
test.assert_raises(fn() { throw KeyError("k") }, "KeyError")

# ---------------------------------------------------------------------------
# 失败路径（try/except AssertionError 包裹）
# ---------------------------------------------------------------------------

caught = 0

# assert_eq 失败消息含双值
try {
    test.assert_eq(1, 2)
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("1"), "消息含 str(a)")
    assert(e.message.contains("2"), "消息含 str(b)")
}

# 自定义 msg 优先
try {
    test.assert_eq(1, 2, "custom msg")
} except AssertionError as e {
    caught += 1
    assert(e.message == "custom msg", "自定义 msg")
}

# assert_ne 失败
try {
    test.assert_ne(3, 3)
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("3"), "ne 消息含值")
}

# assert_true / assert_false 失败
try {
    test.assert_true(false)
} except AssertionError as e {
    caught += 1
}
try {
    test.assert_true(nil)
} except AssertionError as e {
    caught += 1
}
try {
    test.assert_false(true)
} except AssertionError as e {
    caught += 1
}

# assert_almost_eq 默认 eps 失败
try {
    test.assert_almost_eq(1.0, 1.1)
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("1.0"), "almost_eq 消息含双值")
}

# assert_len 失败
try {
    test.assert_len([1], 2)
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("2"), "len 消息含期望值")
}

# assert_contains 失败
try {
    test.assert_contains([1, 2], 9)
} except AssertionError as e {
    caught += 1
}

# fail
try {
    test.fail()
} except AssertionError as e {
    caught += 1
}
try {
    test.fail("reason")
} except AssertionError as e {
    caught += 1
    assert(e.message == "reason", "fail(msg)")
}

# assert_raises：未抛异常
try {
    test.assert_raises(fn() { return 1 }, "ValueError")
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("not raised"), "未抛异常消息")
}

# assert_raises：类不匹配（e.type 精确比对，不含父类链）
try {
    test.assert_raises(fn() { throw KeyError("k") }, "ValueError")
} except AssertionError as e {
    caught += 1
    assert(e.message.contains("ValueError"), "不匹配消息含期望类")
    assert(e.message.contains("KeyError"), "不匹配消息含实际类")
}

# assert_raises：父类名不匹配子类异常（精确比对语义）
try {
    test.assert_raises(fn() { throw ValueError("v") }, "Error")
} except AssertionError as e {
    caught += 1
}

# 成功的 assert_raises 不抛
ok = true
try {
    test.assert_raises(fn() { throw AssertionError("inner") }, "AssertionError")
} except AssertionError as e {
    ok = false
}
assert(ok, "assert_raises 匹配不外抛")

assert(caught == 14, "失败路径全数包裹捕获，got " + str(caught))

print("ALL PASSED")
