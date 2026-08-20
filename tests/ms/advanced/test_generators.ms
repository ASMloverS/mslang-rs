# test_generators.ms — 生成器与 yield
# 覆盖: task 39（07-advanced.md § 生成器）

# 基本生成器：for..in 消费
fn countdown(n) {
    while n > 0 {
        yield n
        n -= 1
    }
}
acc = []
for v in countdown(3) {
    acc.push(v)
}
assert(acc == [3, 2, 1], "for 消费生成器")

# 手动迭代 __next__
fn gen3() {
    yield "a"
    yield "b"
    yield "c"
}
g = gen3()
assert(g.__next__() == "a", "第一次 next")
assert(g.__next__() == "b", "第二次 next")
assert(g.__next__() == "c", "第三次 next")

# 生成器惰性：未迭代不执行（写全局须经 global 声明）
executed = false
fn lazy_gen() {
    global executed
    executed = true
    yield 1
}
_ = lazy_gen()
assert((executed) == false, "创建生成器不执行函数体")
lg = lazy_gen()
lg.__next__()
assert(executed, "首次 next 才执行到第一个 yield")

# 生成器保持局部状态
fn make_counter(start) {
    n = start
    while true {
        yield n
        n += 1
    }
}
cnt = make_counter(100)
assert(cnt.__next__() == 100, "计数器 1")
assert(cnt.__next__() == 101, "计数器 2")
assert(cnt.__next__() == 102, "计数器 3")

# 无限生成器（斐波那契，取前几个）
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}
fib = fibonacci()
fib_vals = []
for f in range(8) {
    fib_vals.push(fib.__next__())
}
assert(fib_vals == [0, 1, 1, 2, 3, 5, 8, 13], "无限生成器按需取值")

# yield from 委托
fn inner_gen() {
    yield 1
    yield 2
}
fn outer_gen() {
    yield 0
    yield from inner_gen()
    yield 3
}
delegated = []
for v in outer_gen() {
    delegated.push(v)
}
assert(delegated == [0, 1, 2, 3], "yield from 委托")

# yield from 递归展平
fn flatten(nested) {
    for item in nested {
        if type(item) == "list" {
            yield from flatten(item)
        } else {
            yield item
        }
    }
}
flat = []
for v in flatten([1, [2, 3], [4, [5, [6]]]]) {
    flat.push(v)
}
assert(flat == [1, 2, 3, 4, 5, 6], "递归展平")

# 生成器表达式（圆括号形式，惰性）
ge = (x * 2 for x in range(4))
ge_vals = []
for v in ge {
    ge_vals.push(v)
}
assert(ge_vals == [0, 2, 4, 6], "生成器表达式")

ge2 = (x for x in range(10) if x % 3 == 0)
ge2_vals = []
for v in ge2 {
    ge2_vals.push(v)
}
assert(ge2_vals == [0, 3, 6, 9], "生成器表达式带过滤")

# 两个生成器实例相互独立
ga = gen3()
gb = gen3()
assert(ga.__next__() == "a", "实例 a 首值")
assert(gb.__next__() == "a", "实例 b 首值独立")
assert(ga.__next__() == "b", "实例 a 次值")

print("ALL PASSED")
