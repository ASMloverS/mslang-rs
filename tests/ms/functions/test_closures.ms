# test_closures.ms — 闭包与上值
# 覆盖: task 28（04-functions.md § 闭包与上值）

# 基本捕获：闭包引用外层参数
fn make_multiplier(n) {
    return fn(x) {
        return x * n
    }
}
mul3 = make_multiplier(3)
mul10 = make_multiplier(10)
assert(mul3(7) == 21, "捕获参数")
assert(mul10(7) == 70, "另一捕获实例独立")

# nonlocal 写外层变量（计数器）
fn make_counter() {
    count = 0
    return fn() {
        nonlocal count
        count += 1
        return count
    }
}
c1 = make_counter()
c1()
c1()
assert(c1() == 3, "nonlocal 计数器累加")

c2 = make_counter()
c2()
assert(c2() == 2 and c1() == 4, "两个计数器互不干扰")

# 未声明 nonlocal 时在内层创建新局部（不影响外层）
fn shadow_test() {
    v = "outer"
    inner = fn() {
        v = "inner-local"
        return v
    }
    result = inner()
    return result + "/" + v
}
assert(shadow_test() == "inner-local/outer", "未声明 nonlocal 时内层赋值仅遮蔽")

# 多层捕获（两层上值链）
fn outer(a) {
    fn middle(b) {
        fn inner(c) {
            return a + b + c
        }
        return inner
    }
    return middle
}
chain = outer(100)(20)
assert(chain(3) == 123, "两层上值链")

# 闭包捕获循环变量（函数级作用域同 Python：所有闭包共享同一变量，
# 循环结束后捕获值为末次迭代值）
fn make_adders() {
    adders = []
    for i in range(3) {
        adders.push(fn(x) {
            return x + i
        })
    }
    return adders
}
fns = make_adders()
assert(fns[0](100) == 102, "循环变量共享（首闭包见末值）")
assert(fns[2](100) == 102, "循环变量共享（末闭包一致）")

# global 声明写全局
counter = 0
fn bump_global() {
    global counter
    counter += 5
}
bump_global()
bump_global()
assert(counter == 10, "global 声明写全局")

# 闭包共享上值（同一外层的两个闭包互通）
fn make_pair() {
    shared = 0
    inc = fn() {
        nonlocal shared
        shared += 1
        return shared
    }
    get = fn() {
        return shared
    }
    return inc, get
}
inc_fn, get_fn = make_pair()
inc_fn()
inc_fn()
assert(get_fn() == 2, "两闭包共享同一上值")

print("ALL PASSED")
