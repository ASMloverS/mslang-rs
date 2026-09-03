# itertools — 嵌入式 .ms 标准库（task 84，16-stdlib-expansion.md §4.13）
# 14 个惰性生成器函数；无限序列（count/cycle/repeat 无 n）配合
# take_while/islice 消费；combinations/permutations/product 输入先物化。
# 注：模块函数体在导入方全局上下文执行（LOAD_GLOBAL 查调用方 globals），
# 递归辅助经局部闭包自引用，不依赖模块顶层名。

fn count(start = 0, step = 1) {
    n = start
    while true {
        yield n
        n += step
    }
}

fn cycle(iterable) {
    items = list(iterable)
    if items.length() == 0 {
        return
    }
    i = 0
    while true {
        yield items[i]
        i = (i + 1) % items.length()
    }
}

fn repeat(x, n = nil) {
    if n == nil {
        while true {
            yield x
        }
    }
    i = 0
    while i < n {
        yield x
        i += 1
    }
}

fn chain(*iters) {
    for it in iters {
        for x in it {
            yield x
        }
    }
}

fn take_while(pred, it) {
    for x in it {
        if not pred(x) {
            return
        }
        yield x
    }
}

fn drop_while(pred, it) {
    dropping = true
    for x in it {
        if dropping and pred(x) {
            continue
        }
        dropping = false
        yield x
    }
}

fn pairwise(it) {
    first = true
    prev = nil
    for x in it {
        if first {
            first = false
        } else {
            yield (prev, x)
        }
        prev = x
    }
}

fn accumulate(it, f = nil) {
    first = true
    acc = nil
    for x in it {
        if first {
            acc = x
            first = false
        } else {
            if f == nil {
                acc = acc + x
            } else {
                acc = f(acc, x)
            }
        }
        yield acc
    }
}

fn zip_longest(*iters) {
    seqs = []
    for it in iters {
        seqs.push(list(it))
    }
    if seqs.length() == 0 {
        return
    }
    n = 0
    for s in seqs {
        if s.length() > n {
            n = s.length()
        }
    }
    for i in range(n) {
        row = []
        for s in seqs {
            if i < s.length() {
                row.push(s[i])
            } else {
                row.push(nil)
            }
        }
        yield tuple(row)
    }
}

fn product(*iters) {
    seqs = []
    for it in iters {
        seqs.push(list(it))
    }
    # product() 零参 → 产出一个空 tuple 后结束（Python 语义）。
    if seqs.length() == 0 {
        yield tuple([])
        return
    }
    lens = []
    empty = false
    for s in seqs {
        lens.push(s.length())
        if s.length() == 0 {
            empty = true
        }
    }
    # 任一输入为空 → 不产出（笛卡尔积为空）。
    if empty {
        return
    }
    n = seqs.length()
    idx = []
    for i in range(n) {
        idx.push(0)
    }
    # 索引里程表：从末位进位，全溢出即穷尽。
    while true {
        row = []
        for i in range(n) {
            row.push(seqs[i][idx[i]])
        }
        yield tuple(row)
        i = n - 1
        while i >= 0 {
            idx[i] += 1
            if idx[i] < lens[i] {
                break
            }
            idx[i] = 0
            i -= 1
        }
        if i < 0 {
            return
        }
    }
}

# 组合类：r 校验为急切（Python itertools 语义：调用即抛，可被 try/except 捕获
# ——生成器体内 throw 在 resume 时不可捕获）；r>len 置为 len+1 使递归零产出。
# 递归生成器经局部闭包自引用（模块函数在导入方全局上下文执行，不引用模块顶层名）。
fn combinations(iterable, r) {
    items = list(iterable)
    if r < 0 {
        throw ValueError("combinations(): r must be non-negative")
    }
    if r > items.length() {
        r = items.length() + 1
    }
    gen = nil
    gen = fn(start, prefix) {
        if prefix.length() == r {
            yield tuple(prefix)
            return
        }
        for i in range(start, items.length()) {
            prefix.push(items[i])
            yield from gen(i + 1, prefix)
            prefix.pop()
        }
    }
    return gen(0, [])
}

fn permutations(iterable, r = nil) {
    items = list(iterable)
    if r == nil {
        r = items.length()
    }
    if r < 0 {
        throw ValueError("permutations(): r must be non-negative")
    }
    if r > items.length() {
        r = items.length() + 1
    }
    used = []
    for i in range(items.length()) {
        used.push(false)
    }
    gen = nil
    gen = fn(prefix) {
        if prefix.length() == r {
            yield tuple(prefix)
            return
        }
        for i in range(items.length()) {
            if not used[i] {
                used[i] = true
                prefix.push(items[i])
                yield from gen(prefix)
                prefix.pop()
                used[i] = false
            }
        }
    }
    return gen([])
}

# stop=nil 表示无限（islice(it, start, nil) 位置传参）。
# 参数校验为急切（Python 语义，调用即抛）；产出经嵌套生成器闭包。
fn islice(iterable, start, stop, step = 1) {
    if start < 0 {
        throw ValueError("islice(): start must be non-negative")
    }
    if stop != nil and stop < 0 {
        throw ValueError("islice(): stop must be non-negative or nil")
    }
    if step < 1 {
        throw ValueError("islice(): step must be at least 1")
    }
    gen = fn() {
        i = 0
        for x in iterable {
            if stop != nil and i >= stop {
                return
            }
            if i >= start and (i - start) % step == 0 {
                yield x
            }
            i += 1
        }
    }
    return gen()
}

# batched(it, n)：n<1 → ValueError（Python 3.12 语义，急切校验）；
# 尾批可不满；产出 tuple。
fn batched(iterable, n) {
    if n < 1 {
        throw ValueError("batched(): n must be at least 1")
    }
    gen = fn() {
        batch = []
        for x in iterable {
            batch.push(x)
            if batch.length() == n {
                yield tuple(batch)
                batch = []
            }
        }
        if batch.length() > 0 {
            yield tuple(batch)
        }
    }
    return gen()
}
