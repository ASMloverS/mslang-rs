// test_generator.ms — 生成器与 yield

// 基本生成器
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}

for i in countdown(5) {
    print(i)
}

// 手动迭代
fn gen3() {
    yield 10
    yield 20
    yield 30
}

g = gen3()
print(g.__next__())
print(g.__next__())
print(g.__next__())

// 无限生成器
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}

fib = fibonacci()
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())

// yield from
fn flatten(nested) {
    for item in nested {
        if type(item) == "list" {
            yield from flatten(item)
        } else {
            yield item
        }
    }
}

for v in flatten([1, [2, 3], [4, [5, 6]]]) {
    print(v)
}

// 生成器中引用外部变量
fn make_counter(start) {
    n = start
    while true {
        yield n
        n += 1
    }
}

c = make_counter(100)
print(c.__next__())
print(c.__next__())
print(c.__next__())
