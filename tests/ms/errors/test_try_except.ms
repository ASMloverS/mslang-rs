# test_try_except.ms — try/except/finally 与 throw
# 覆盖: task 37（05-control-flow.md § 异常处理；task 48/49 原生错误不可捕获除外）

# throw 字符串 → RuntimeError，e.message 取消息
fn risky(n) {
    if n < 0 {
        throw "negative: " + str(n)
    }
    return n * 2
}

# 无异常路径：except 不执行
path = []
result = 0
try {
    result = risky(5)
    path.push("try")
} except as e {
    path.push("except")
}
assert(result == 10, "try 正常返回值经赋值")
assert(path == ["try"], "无异常不进 except")

# 异常路径：except 捕获，try 块剩余语句跳过
path2 = []
try {
    path2.push("before")
    risky(-3)
    path2.push("after")    # 不应执行
} except as e {
    path2.push("except:" + e.message)
}
assert(path2 == ["before", "except:negative: -3"], "异常中断 try 块")

# finally：正常与异常路径都执行
fin1 = []
try {
    fin1.push("t")
} finally {
    fin1.push("f")
}
assert(fin1 == ["t", "f"], "finally 正常路径")

fin2 = []
reached = false
try {
    try {
        throw "boom"
    } finally {
        fin2.push("fin")
    }
} except as e {
    reached = true
}
assert(fin2 == ["fin"], "finally 异常路径")
assert(reached, "异常穿透 finally 后被外层捕获")

# except 匹配特定类型（沿内置异常层级：throw → RuntimeError，基类 Error）
fn type_route(v) {
    try {
        throw v
    } except ValueError as e {
        return "VE:" + e.message
    } except RuntimeError as e {
        return "RT:" + e.message
    }
}
assert(type_route("runtime-case") == "RT:runtime-case", "RuntimeError 子句命中")

# Error 基类匹配所有异常
try {
    throw "base-match"
} except Error as e {
    assert(e.message == "base-match", "Error 基类子句命中")
}

# 未匹配类型落到后续子句 / 兜底
fn unmatched() {
    try {
        throw "plain"
    } except ValueError as e {
        return "VE"
    } except {
        return "BARE"
    }
}
assert(unmatched() == "BARE", "类型未命中走兜底 except")

# 多个 except 子句顺序匹配（首个命中生效）
fn multi(v) {
    try {
        throw v
    } except RuntimeError as e {
        return "RT"
    } except Error as e {
        return "ERR"
    } except {
        return "FALLBACK"
    }
}
assert(multi("a") == "RT", "首子句命中（具体类型）")
assert(multi("b") == "RT", "throw 均为 RuntimeError")
# Error 在 RuntimeError 之后不可达（顺序语义验证）

# 嵌套 try：内层处理，外层不受影响
nested = ""
try {
    try {
        throw "inner"
    } except as e {
        nested = "inner-caught"
    }
    nested = nested + "+continued"
} except as e2 {
    nested = nested + "+outer"
}
assert(nested == "inner-caught+continued", "内层捕获后继续执行")

# 重抛与包装
fn wrap_throw() {
    try {
        throw "root-cause"
    } except as e {
        throw "wrapped[" + e.message + "]"
    }
}
outer_msg = ""
try {
    wrap_throw()
} except as e3 {
    outer_msg = e3.message
}
assert(outer_msg == "wrapped[root-cause]", "异常包装重抛")

# 异常从被调函数向上传播到调用者 try
fn deep_throw() {
    helper()
}
fn helper() {
    throw "deep"
}
deep_msg = ""
try {
    deep_throw()
} except as e4 {
    deep_msg = e4.message
}
assert(deep_msg == "deep", "跨函数传播")

# try 表达式值：try/except/finally 作为语句，无值语义（不测）

# e.type / e.traceback 属性存在
try {
    throw "tmsg"
} except as e5 {
    assert(e5.message == "tmsg", "e.message")
    assert(type(e5.type) == "string", "e.type 为字符串")
}

# finally 中的 return 不覆盖异常语义（规范 §3），此处仅验证 finally 顺序
order = []
fn finally_order() {
    try {
        order.push("1")
        return "early"
    } finally {
        order.push("2")
    }
}
assert(finally_order() == "early", "try 内 return 值")
assert(order == ["1", "2"], "return 后 finally 仍执行")

print("ALL PASSED")
