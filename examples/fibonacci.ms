# fibonacci.ms — 生成器与多返回值示例

# 递归版
fn fib(n) {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

# 生成器版（无限序列，按需取值）
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}

for i in range(10) {
    print(fib(i))
}

gen = fibonacci()
first10 = []
for i in range(10) {
    first10.push(gen.__next__())
}
print(first10)
