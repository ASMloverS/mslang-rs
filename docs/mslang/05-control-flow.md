# 控制流

## 条件语句

### if / elif / else

```
if_stmt = "if" expression block ("elif" expression block)* ("else" block)?
```

```ms
if score >= 90 {
    grade = "A"
} elif score >= 80 {
    grade = "B"
} elif score >= 70 {
    grade = "C"
} else {
    grade = "F"
}
```

**语义**：
- 条件表达式求值，若为 truthy 则执行对应块
- 按顺序检查 `if` → `elif` → `else`
- 最多执行一个分支
- 条件不需要括号

### 三元表达式

```
ternary = expr "if" expr "else" expr
```

```ms
status = "pass" if score >= 60 else "fail"
max_val = a if a > b else b
```

## 循环语句

### while

```
while_stmt = "while" expression block
```

```ms
i = 0
while i < 10 {
    print(i)
    i += 1
}

while true {
    # 无限循环
    if should_stop() {
        break
    }
}
```

### for..in

```
for_stmt = "for" IDENTIFIER "in" expression block
         | "for" IDENTIFIER "," IDENTIFIER "in" expression block
```

```ms
# 遍历列表
for item in [1, 2, 3] {
    print(item)
}

# 遍历 range
for i in range(10) {
    print(i)
}

# 遍历字典
for key in dict {
    print(key)
}

# 遍历键值对
for key, value in dict.items() {
    print(key + ": " + str(value))
}

# 遍历字符串
for ch in "hello" {
    print(ch)
}

# 遍历生成器
for val in generator() {
    print(val)
}
```

**可迭代对象**：list, tuple, dict, set, string, range, 生成器。

**for..in 语义**：
- 每次迭代从可迭代对象取下一个值
- 赋值给循环变量
- 迭代结束后退出循环
- 循环变量在循环结束后**保持最后一次的值**

### break

```
break_stmt = "break"
```

立即跳出当前循环。

```ms
for item in items {
    if item == target {
        print("found")
        break
    }
}
```

### continue

```
continue_stmt = "continue"
```

跳过当前迭代的剩余部分，进入下一次迭代。

```ms
for i in range(10) {
    if i % 2 == 0 {
        continue
    }
    print(i)    # 只打印奇数
}
```

### 循环嵌套

break/continue 只影响最内层循环。不支持带标签的 break/continue。

```ms
for i in range(5) {
    for j in range(5) {
        if j == 3 {
            break    # 只跳出内层循环
        }
    }
}
```

## 错误处理

### try / except / finally

```
try_stmt = "try" block except_clause* finally_clause?

except_clause = "except" type_spec? ("as" IDENTIFIER)? block
type_spec     = IDENTIFIER ("." IDENTIFIER)*
finally_clause = "finally" block
```

```ms
# 基本用法
try {
    result = risky()
} except {
    print("caught an error")
}

# 捕获特定异常类型
try {
    val = int("abc")
} except ValueError as e {
    print("not a number: " + e.message)
}

# 多个 except
try {
    operation()
} except ValueError as e {
    print("value error")
} except IOError as e {
    print("io error")
} except as e {
    print("unknown error: " + e.message)
} finally {
    cleanup()
}
```

**语义**：

1. 执行 `try` 块
2. 若发生异常，按顺序检查 `except` 子句：
   - 不带类型：匹配所有异常
   - 带类型：匹配该类型及其子类型
   - `as name`：将异常对象绑定到 `name`
3. 无论是否发生异常，`finally` 块总是执行
4. `finally` 块中的异常会覆盖之前的异常

5. `dict[key]` 访问不存在的键返回 `nil`（不抛 KeyError）；`dict.remove(key)` 删除不存在的键抛 KeyError

### 异常对象

异常对象是特殊的 class 实例，包含以下属性：

```
Error
├── message      # 错误消息（string）
├── type         # 错误类型名（string）
├── traceback    # 堆栈跟踪（string）
```

### 内置异常类型

```
Error                      # 所有异常的基类
├── ValueError             # 值错误
├── TypeError              # 类型错误
├── IndexError             # 下标越界
├── KeyError               # 键不存在
├── AttributeError         # 属性不存在
├── NameError              # 变量未定义
├── RuntimeError           # 运行时错误
├── IOError                # IO 错误
├── ZeroDivisionError      # 除零错误
├── OverflowError          # 溢出错误
└── StopIteration          # 迭代结束
```

### 自定义异常类

用户可通过继承内置异常类创建自定义异常：

```ms
class MyError < ValueError {
    fn __init__(self, message, code) {
        super.__init__(message)
        self.code = code
    }
}

throw MyError("something went wrong", 42)
```

自定义异常类：
- 必须继承自 `Error` 或其子类
- 可在 `except` 中按类型匹配
- 异常对象的 `message`、`type`、`traceback` 属性自动可用

### 抛出异常

```ms
fn assert_positive(n) {
    if n < 0 {
        throw ValueError("expected positive, got " + str(n))
    }
}
```

`throw` 关键字用于抛出异常：

```
throw_stmt = "throw" expression?
```

`throw` 后跟表达式时，表达式应为 Error 实例或子类实例。裸 `throw`（无表达式）用于 re-throw：在 `except` 块内重新抛出当前捕获的异常。在 `except` 块外使用裸 `throw` 抛出 `RuntimeError`。

### 异常传播

异常沿调用栈向上传播，直到被 `try/except` 捕获。若未被捕获，程序终止并打印堆栈跟踪。

## defer

```
defer_stmt = "defer" expression
```

defer 注册一个在函数返回前执行的调用。

```ms
fn copy_file(src, dst) {
    fin = open(src, "r")
    defer fin.close()

    fout = open(dst, "w")
    defer fout.close()

    content = fin.read()
    fout.write(content)
}
```

### defer 执行规则

1. defer 在函数**返回前**执行（包括正常返回和异常返回）
2. 多个 defer 按 **LIFO**（后进先出）顺序执行
3. defer 的参数在 **defer 声明时**求值，不是在执行时

```ms
fn example() {
    for i in range(3) {
        defer print(i)
    }
    # 输出: 2, 1, 0（LIFO，且 i 在 defer 时求值）
}
```

### defer 与 try/finally 的关系

defer 的行为等价于：

```ms
fn example() {
    # defer action1
    # defer action2
    try {
        # 函数体
    } finally {
        action2()   # 后注册的先执行
        action1()
    }
}
```

## with 语句

```
with_stmt = "with" expression ("as" IDENTIFIER)? block
```

```ms
with open("data.txt") as f {
    content = f.read()
    print(content)
}
# f.__exit__() 在这里自动调用
```

### 上下文管理器协议

一个对象要支持 `with` 语句，必须实现：

```ms
class MyResource {
    fn __enter__(self) {
        # 获取资源，返回 self 或其他对象
        return self
    }

    fn __exit__(self, error_type, error_msg, traceback) {
        # 清理资源
        # 如果返回 true，异常被吞掉
        # 如果返回 false 或 nil，异常继续传播
        return false
    }
}
```

### with 执行流程

1. 求值 `with` 后的表达式
2. 调用结果的 `__enter__()` 方法
3. 如果有 `as name`，将 `__enter__()` 返回值绑定到 `name`
4. 执行块体
5. 离开块时（正常或异常），调用 `__exit__()`
6. 如果块内有异常，异常信息传递给 `__exit__()`

### 嵌套 with

```ms
with open("input.txt") as fin {
    with open("output.txt") as fout {
        fout.write(fin.read())
    }
}
```
