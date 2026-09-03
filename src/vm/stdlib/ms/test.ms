# test — 嵌入式 .ms 标准库（task 84，16-stdlib-expansion.md §4.15）
# 9 个断言函数；失败统一抛 AssertionError，消息含 str(a)/str(b) 与可选 msg。
# assert_raises 类匹配用 e.type（GET_ATTR EXC 分支返回类名字符串），
# 精确类名比对、不含父类链（与 except 的 CATCH 父类链语义有差异，
# 见 10-builtins.md test 章）；仅捕获 .ms 侧 throw 的异常（原生错误不可捕获）。
# 注：模块函数体在导入方全局上下文执行，各断言自包含（不引用模块顶层 fail）。

fn fail(msg = nil) {
    throw AssertionError(msg or "fail() called")
}

fn assert_eq(a, b, msg = nil) {
    if not (a == b) {
        throw AssertionError(msg or ("assert_eq failed: " + str(a) + " != " + str(b)))
    }
}

fn assert_ne(a, b, msg = nil) {
    if a == b {
        throw AssertionError(msg or ("assert_ne failed: " + str(a) + " == " + str(b)))
    }
}

fn assert_true(cond, msg = nil) {
    if not cond {
        throw AssertionError(msg or "assert_true failed: condition is false")
    }
}

fn assert_false(cond, msg = nil) {
    if cond {
        throw AssertionError(msg or "assert_false failed: condition is true")
    }
}

fn assert_almost_eq(a, b, eps = 1e-9, msg = nil) {
    if eps == nil {
        eps = 1e-9
    }
    if abs(a - b) > eps {
        throw AssertionError(msg or ("assert_almost_eq failed: |" + str(a) + " - " + str(b) + "| > " + str(eps)))
    }
}

fn assert_raises(f, exc_class, msg = nil) {
    try {
        f()
    } except as e {
        if e.type != exc_class {
            throw AssertionError(msg or ("expected " + exc_class + ", got " + e.type))
        }
        return nil
    }
    throw AssertionError(msg or "expected exception not raised")
}

fn assert_len(v, n, msg = nil) {
    if len(v) != n {
        throw AssertionError(msg or ("assert_len failed: len == " + str(len(v)) + ", expected " + str(n)))
    }
}

fn assert_contains(coll, item, msg = nil) {
    if not (item in coll) {
        throw AssertionError(msg or "assert_contains failed: item not in collection")
    }
}
