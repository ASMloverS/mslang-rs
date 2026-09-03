# functools — 嵌入式 .ms 标准库（task 84，16-stdlib-expansion.md §4.14）
# partial / memoize / reduce。
#
# 实现注记：
# - mslang 无调用点展开（f(*args)，03-syntax.md 调用文法仅位置实参），
#   动态实参以 0-4 参阶梯分派；超出 → TypeError。
# - 模块函数体在导入方全局上下文执行（LOAD_GLOBAL 查调用方 globals），
#   故各导出实体自包含（嵌套闭包捕获，不引用模块顶层名）。

# partial(fn, *args)：可调用实例（类即工厂——构造即绑定前缀实参），
# 调用时 args 在前、*more 在后。
class partial {
    fn __init__(self, f, *args) {
        self.func = f
        self.args = args
    }
    fn __call__(self, *more) {
        all = self.args + more
        n = all.length()
        if n == 0 {
            return self.func()
        }
        if n == 1 {
            return self.func(all[0])
        }
        if n == 2 {
            return self.func(all[0], all[1])
        }
        if n == 3 {
            return self.func(all[0], all[1], all[2])
        }
        if n == 4 {
            return self.func(all[0], all[1], all[2], all[3])
        }
        throw TypeError("partial: functions of arity > 4 are not supported (no call-site spread)")
    }
}

# memoize(fn)：dict 缓存，键 tuple(args)；unhashable → TypeError（dict 行为上抛）；
# 无 LRU 上限（开放问题 5 留白）。
fn memoize(f) {
    cache = {}
    call_with = fn(args) {
        n = args.length()
        if n == 0 {
            return f()
        }
        if n == 1 {
            return f(args[0])
        }
        if n == 2 {
            return f(args[0], args[1])
        }
        if n == 3 {
            return f(args[0], args[1], args[2])
        }
        if n == 4 {
            return f(args[0], args[1], args[2], args[3])
        }
        throw TypeError("memoize: functions of arity > 4 are not supported (no call-site spread)")
    }
    return fn(*args) {
        key = tuple(args)
        if cache.contains(key) {
            return cache[key]
        }
        val = call_with(args)
        cache[key] = val
        return val
    }
}

# reduce(fn, iter, init?)：iterable 级归约（list.reduce 方法保留不动）。
# init=nil 视为未提供（首元素作种子）；空迭代 + 无 init → TypeError。
fn reduce(f, iterable, init = nil) {
    has_init = init != nil
    acc = init
    first = true
    for x in iterable {
        if first and not has_init {
            acc = x
        } else {
            acc = f(acc, x)
        }
        first = false
    }
    if first and not has_init {
        throw TypeError("reduce(): empty iterable with no initial value")
    }
    return acc
}
