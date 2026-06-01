# 高级特性

## 装饰器

### 语法

```
decorator = "@" expression newline
decorated = decorator+ (fn_def | class_def)
```

```ms
@log
fn greet(name) {
    return "Hello, " + name
}

# 等价于：
fn greet(name) {
    return "Hello, " + name
}
greet = log(greet)
```

### 函数装饰器

```ms
fn timer(func) {
    return fn(*args) {
        start = time.now()
        result = func(*args)
        elapsed = time.now() - start
        print("took " + str(elapsed) + "s")
        return result
    }
}

@timer
fn slow_function() {
    # 耗时操作
}
```

### 带参数的装饰器

```ms
fn retry(times) {
    return fn(func) {
        return fn(*args) {
            for i in range(times) {
                try {
                    return func(*args)
                } except {
                    if i == times - 1 {
                        throw
                    }
                }
            }
        }
    }
}

@retry(3)
fn unreliable_request() {
    # 可能失败的网络请求
}
```

### 多重装饰器

```ms
@decorator1
@decorator2
fn func() {
    # ...
}

# 等价于：
fn func() { ... }
func = decorator1(decorator2(func))
```

装饰器从下到上应用（靠近函数的先执行）。

### 类装饰器

```ms
fn add_repr(cls) {
    cls.__repr__ = fn(self) {
        return cls.__name__ + "()"
    }
    return cls
}

@add_repr
class Foo {
}

print(Foo())    # "Foo()"
```

## 生成器 (Generator)

### yield 语法

包含 `yield` 的函数自动成为生成器函数。

```ms
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}

for i in countdown(5) {
    print(i)    # 5, 4, 3, 2, 1
}
```

### 生成器函数

- 调用生成器函数不执行函数体，而是返回一个**生成器对象**
- 生成器对象是迭代器，实现了 `__iter__` 和 `__next__`
- 每次调用 `__next__()`（或 `for..in`）执行到下一个 `yield`
- `yield expr` 暂停执行并返回 `expr` 的值
- 函数体执行完毕时抛出 `StopIteration`

### 生成器资源清理

生成器被 GC 回收但尚未耗尽时，VM 自动注入 `GeneratorExit` 异常（内部异常，不可被用户 `except` 捕获）并恢复生成器帧执行，触发生成器内部的 `defer` 和 `finally` 块执行。这确保文件句柄等资源被正确释放。

```ms
fn read_lines(path) {
    with open(path) as f {
        for line in f.lines() {
            yield line.strip()
        }
    }
}

gen = read_lines("big.txt")
gen.__next__()    # 读一行
gen = nil         # 丢弃生成器 → GC 回收时自动关闭文件
```

也可手动调用 `gen.close()` 提前关闭生成器（立即触发清理，不等 GC）。

```ms
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}

fib = fibonacci()
fib.__next__()    # 0
fib.__next__()    # 1
fib.__next__()    # 1
fib.__next__()    # 2
```

### yield from

`yield from` 委托给另一个可迭代对象：

```ms
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
    print(v)    # 1, 2, 3, 4, 5, 6
}
```

> **消歧规则**：`yield from` 中的 `from` 作为关键字解析（委托生成器语义），仅在 `yield` 紧后跟 `from` 时触发。`yield from_module.import_name` 等场景中 `from` 后不跟表达式，仍解析为 `yield` 后跟标识符表达式 `from_module.import_name`。解析器通过检查 `from` 后是否跟随表达式来区分。

### 生成器表达式

类似列表推导式但用圆括号，惰性求值：

```ms
total = sum(x * x for x in range(1000000))
```

生成器表达式的形式文法见 [03-syntax](03-syntax.md) 推导式章节。圆括号内的推导式惰性求值，不在内存中构建完整列表。

### 生成器与 with

```ms
fn lines(filename) {
    with open(filename) as f {
        for line in f.lines() {
            yield line.strip()
        }
    }
}
```

## 列表推导式

### 基本语法

```
list_comp = "[" expression "for" IDENTIFIER "in" expression ("if" expression)? "]"
```

```ms
squares = [x * x for x in range(10)]
# [0, 1, 4, 9, 16, 25, 36, 49, 64, 81]

evens = [x for x in range(20) if x % 2 == 0]
# [0, 2, 4, 6, 8, 10, 12, 14, 16, 18]
```

### 带过滤条件

```ms
names = ["Alice", "Bob", "Charlie", "David"]
long_names = [n for n in names if n.length() > 3]
# ["Alice", "Charlie", "David"]
```

### 嵌套推导式

```ms
matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
flat = [x for row in matrix for x in row]
# [1, 2, 3, 4, 5, 6, 7, 8, 9]

pairs = [(x, y) for x in range(3) for y in range(3) if x != y]
# [(0,1), (0,2), (1,0), (1,2), (2,0), (2,1)]
```

### dict 推导式

```ms
squares_dict = {x: x*x for x in range(5)}
# {0:0, 1:1, 2:4, 3:9, 4:16}
```

### set 推导式

```ms
unique_lengths = {w.length() for w in ["a", "bb", "ccc", "bb"]}
# {1, 2, 3}
```

## 切片

### 语法

```
slice = expression "[" slice_part? ":" slice_part? (":" slice_part)? "]"
slice_part = expression | (空)
```

格式：`seq[start:stop:step]`

### 参数规则

| 参数 | 默认值 | 含义 |
|---|---|---|
| `start` | `0` | 起始索引（含） |
| `stop` | `seq.length()` | 结束索引（不含） |
| `step` | `1` | 步长 |

### 示例

```ms
lst = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

lst[2:5]       # [2, 3, 4]
lst[:3]        # [0, 1, 2]
lst[7:]        # [7, 8, 9]
lst[::2]       # [0, 2, 4, 6, 8]
lst[1::2]      # [1, 3, 5, 7, 9]
lst[::-1]      # [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
lst[8:2:-1]    # [8, 7, 6, 5, 4, 3]
```

### 负索引

```ms
lst[-1]        # 9（最后一个元素）
lst[-3:]       # [7, 8, 9]
lst[-5:-2]     # [5, 6, 7]
```

负索引 `-n` 等价于 `length - n`。

### 适用类型

切片适用于以下类型：

| 类型 | 返回类型 |
|---|---|
| list | list |
| string | string |
| tuple | tuple |

切片总是返回**新对象**（不修改原对象）。

### 越界处理

切片操作不会越界报错，会自动调整到有效范围：

```ms
lst = [0, 1, 2]
lst[0:100]     # [0, 1, 2]
lst[-100:100]  # [0, 1, 2]
lst[100:200]   # []
```

## with 语句（上下文管理器）

详见 [05-control-flow](05-control-flow.md#with-语句)。

协议：

```ms
fn __enter__(self) -> value     # 进入 with 块时调用，返回值绑定到 as 变量
fn __exit__(self, err, msg, tb) -> bool  # 离开 with 块时调用
```

## defer 语句

详见 [05-control-flow](05-control-flow.md#defer)。

- 在函数返回前执行
- LIFO 顺序
- 参数在声明时求值
- 即使异常也会执行
