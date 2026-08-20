# test_decorators.ms — 装饰器
# 覆盖: task 44（07-advanced.md § 装饰器）

# 基本装饰器：包装函数
fn shout(fn0) {
    return fn(s) {
        return fn0(s).upper() + "!"
    }
}
@shout
fn hello(name) {
    return "hello " + name
}
assert(hello("bob") == "HELLO BOB!", "基本装饰器")

# 装饰后函数名被替换（原函数不可直接访问）
@shout
fn hi(name) {
    return "hi " + name
}
assert(hi("x") == "HI X!", "第二个装饰函数独立")

# 带参数的装饰器（三层）
fn tag_wrap(tag) {
    return fn(inner) {
        return fn(s) {
            return "[" + tag + "] " + inner(s)
        }
    }
}
@tag_wrap("ms")
fn greet(name) {
    return "hello " + name
}
assert(greet("al") == "[ms] hello al", "带参装饰器")

@tag_wrap("py")
fn bye(name) {
    return "bye " + name
}
assert(bye("al") == "[py] bye al", "不同参数的装饰器实例")

# 装饰器保持被包装函数的调用参数传递
fn trace(fn0) {
    return fn(a, b) {
        return fn0(a, b) * 10
    }
}
@trace
fn add(a, b) {
    return a + b
}
assert(add(1, 2) == 30, "多参函数装饰")

# 装饰带默认参数的函数
fn keep_default(fn0) {
    return fn(x, y = 5) {
        return fn0(x, y) + 1
    }
}
@keep_default
fn mult(a, b = 5) {
    return a * b
}
assert(mult(2) == 11, "默认参数经装饰器传递")
assert(mult(2, 3) == 7, "覆盖默认参数")

# 装饰器计数副作用
calls = 0
fn counting(fn0) {
    return fn(x) {
        global calls
        calls += 1
        return fn0(x)
    }
}
@counting
fn ident(x) {
    return x
}
ident(1)
ident(2)
ident(3)
assert(calls == 3, "装饰器记录调用次数")

# 匿名函数经变量接收（等效装饰）
plain = fn(x) { return x + 1 }
wrapped = counting(plain)
wrapped(0)
assert(calls == 4, "手动应用装饰器")

# 装饰生成器函数
fn listify(gen0) {
    return fn(n) {
        items = []
        for v in gen0(n) {
            items.push(v)
        }
        return items
    }
}
@listify
fn upto(n) {
    for i in range(n) {
        yield i
    }
}
assert(upto(4) == [0, 1, 2, 3], "装饰生成器函数")

print("ALL PASSED")
