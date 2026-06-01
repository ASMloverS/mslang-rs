# 语法规范

## 语句终止

换行符作为语句终止符。规则：

1. 换行符终止当前语句
2. **续行规则**：以下情况换行不终止语句：
   - 行尾是运算符（`+`, `-`, `*`, 等）
   - 行尾是逗号 `,`
   - 行尾是左括号 `(`, `[`, `{`
   - 行首是运算符
   - 字符串字面量内（已由引号界定）

```ms
# 这些是合法的多行语句
total = a +
        b +
        c

names = [
    "Alice",
    "Bob",
    "Charlie"
]

result = some_function(
    arg1,
    arg2
)
```

## 程序结构

mslang 程序由一系列顶层语句组成，从上到下顺序执行：

```
program = statement*
```

## 语句 (Statement)

### 变量声明

```
var_stmt  = "var" IDENTIFIER "=" expression
short_var = IDENTIFIER ":=" expression
assign    = IDENTIFIER "=" expression
```

三种变量声明方式等价：

```ms
x = 10           # 直接赋值
var x = 10       # var 关键字
x := 10          # 短声明
```

`var` 和 `:=` 总是创建新变量（在同一作用域）。`=` 赋值给已有变量，若不存在则在当前作用域创建（精确语义见 [作用域规则](#作用域规则) § `=` 赋值行为）。

### 常量声明

```
const_stmt = "const" IDENTIFIER "=" expression
```

```ms
const PI = 3.14159
const MAX_SIZE = 100
```

常量必须在声明时初始化，之后不可修改。常量值仅允许以下形式：字面量（整数、浮点、字符串、布尔、nil）、其他常量引用、二元算术运算（`+` `-` `*` `/` `//` `%` `**`）。运行时表达式（函数调用、变量引用等）不允许出现在 `const` 右侧。

### nonlocal 声明

```
nonlocal_stmt = "nonlocal" IDENTIFIER ("," IDENTIFIER)*
```

```ms
fn make_counter() {
    count = 0
    return fn() {
        nonlocal count
        count += 1
        return count
    }
}
```

`nonlocal` 声明当前函数内的变量名绑定到外层（非全局）函数作用域的同名变量。必须在变量使用前声明。未声明 `nonlocal` 时，`=` 赋值在当前函数作用域创建新局部变量。

### global 声明

```
global_stmt = "global" IDENTIFIER ("," IDENTIFIER)*
```

```ms
counter = 0

fn increment() {
    global counter
    counter += 1
}

increment()
print(counter)    # 1
```

`global` 声明当前函数内的变量名绑定到全局作用域的同名变量。必须在变量使用前声明。未声明 `global` 时，`=` 赋值在当前函数作用域创建新局部变量，不会修改全局变量。

### 赋值语句

```
assign_stmt = target ("=" | "+=" | "-=" | "*=" | "/=" | "//=" | "%=" | "**=" |
              "&=" | "|=" | "^=" | "<<=" | ">>=") expression

target = IDENTIFIER
       | target "." IDENTIFIER          // 属性赋值
       | target "[" expression "]"      // 下标赋值
```

```ms
x = 10
x += 5         # x = 15
arr[0] = 99
obj.name = "Alice"
```

### 多目标赋值

```
multi_assign = target_list "=" expression_list
target_list = target ("," target)*
```

```ms
a, b = 1, 2           # a=1, b=2
a, b = b, a           # 交换
a, b, c = fn()        # 元组解包
```

### 表达式语句

```
expr_stmt = expression
```

单独的表达式作为语句。常用于函数调用：

```ms
print("hello")
items.push(42)
```

### 块语句

```
block = "{" statement* "}"
```

### 条件语句

```
if_stmt = "if" expression block ("elif" expression block)* ("else" block)?
```

```ms
if x > 0 {
    print("positive")
} elif x == 0 {
    print("zero")
} else {
    print("negative")
}
```

条件不需要括号（但可以加，因为括号是分组表达式）。

### 循环语句

#### while

```
while_stmt = "while" expression block
```

```ms
i = 0
while i < 10 {
    print(i)
    i += 1
}
```

#### for..in

```
for_stmt = "for" IDENTIFIER ("," IDENTIFIER)? "in" expression block
```

```ms
for item in [1, 2, 3] {
    print(item)
}

for i in range(10) {
    print(i)
}

for key, value in dict.items() {
    print(key, value)
}
```

`for..in` 遍历可迭代对象（list, tuple, dict, set, string, 生成器, range）。

遍历 dict 时默认遍历键。使用 `dict.items()` 遍历键值对。

> **注意**：在 `for..in` 迭代期间修改被迭代的容器（增删元素）行为未定义。建议避免此操作，或在迭代前复制容器：`for item in list.copy() { ... }`。

### break / continue / return

```
break_stmt    = "break"
continue_stmt = "continue"
return_stmt   = "return" expression_list?
```

```ms
break                   # 跳出当前循环
continue                # 跳到下一次迭代
return                  # 返回 nil
return x                # 返回 x
return a, b, c          # 返回元组 (a, b, c)
```

### defer 语句

```
defer_stmt = "defer" expression
```

```ms
fn process(path) {
    f = open(path)
    defer f.close()
    # ... 即使发生异常，f.close() 也会在函数返回前执行
}
```

defer 的语义：
- 参数在 defer 声明时求值
- 函数体在 defer 语句后执行
- defer 注册的调用在函数返回前按 LIFO 顺序执行
- 多个 defer 按**后进先出**执行

```ms
fn example() {
    defer print("first")
    defer print("second")
    defer print("third")
    # 输出: third, second, first
}
```

> **注意**：`defer` 在模块顶层使用时，在模块执行完毕时执行（等价于整个模块被包裹在隐式函数中）。

### try / except / finally

```
try_stmt = "try" block ("except" type_spec? ("as" IDENTIFIER)? block)* ("finally" block)?
type_spec = IDENTIFIER ("." IDENTIFIER)*
```

```ms
try {
    risky_operation()
} except {
    # 捕获所有异常
    print("something went wrong")
}

try {
    risky_operation()
} except ValueError as e {
    print("value error: " + e.message)
} except IOError as e {
    print("io error: " + e.message)
} finally {
    cleanup()
}
```

- `except` 不带类型匹配所有异常
- `except SomeError as e` 捕获特定类型
- `finally` 块总是执行

### with 语句

```
with_stmt = "with" expression ("as" IDENTIFIER)? block
```

```ms
with open("file.txt") as f {
    content = f.read()
}

with lock.acquire() {
    critical_section()
}
```

语义：调用对象的 `__enter__()` 方法，执行块，退出时调用 `__exit__()`。

### import 语句

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_list

module_path = IDENTIFIER ("." IDENTIFIER)*
import_list = IDENTIFIER ("as" IDENTIFIER)? ("," IDENTIFIER ("as" IDENTIFIER)?)*
```

```ms
import math
import os.path as pathutil
from os import path
from io import open, read_file
```

### 空语句

```
empty_stmt = (换行)
```

空行被忽略。

## 表达式 (Expression)

按优先级从低到高：

### 1. 赋值表达式

```
assign_expr = ternary_expr (("=" | "+=" | ...) ternary_expr)*
```

赋值是右结合的。

### 2. 三元表达式

```
ternary_expr = or_expr ("if" or_expr "else" ternary_expr)?
```

```ms
result = "yes" if ok else "no"
```

### 3. 逻辑 or

```
or_expr = and_expr ("or" and_expr)*
```

### 4. 逻辑 and

```
and_expr = not_expr ("and" not_expr)*
```

### 5. 逻辑 not

```
not_expr = "not" not_expr | comparison_expr
```

### 6. 比较

```
comparison_expr = bit_or (( "==" | "!=" | "<" | ">" | "<=" | ">=" | "in" | "is" ) bit_or)*
```

支持链式比较：

```ms
1 < x < 10     # 等价于 (1 < x) and (x < 10)
```

### 7. 位运算 or

```
bit_or = bit_xor ("|" bit_xor)*
```

### 8. 位运算 xor

```
bit_xor = bit_and ("^" bit_and)*
```

### 9. 位运算 and

```
bit_and = shift ("&" shift)*
```

### 10. 位移

```
shift = addition (("<<" | ">>") addition)*
```

### 11. 加减

```
addition = multiplication (("+" | "-") multiplication)*
```

### 12. 乘除

```
multiplication = unary (("*" | "/" | "//" | "%") unary)*
```

### 13. 一元运算

```
unary = ("-" | "not" | "~" | "<-") unary | power_expr
```

`<-` 为 channel 接收前缀运算符：`<-ch` 从 channel 中接收一个值。

### 14. 幂运算

```
power_expr = postfix ("**" unary)?
```

右结合：`2 ** 3 ** 2` = `2 ** (3 ** 2)` = `512`

### 15. 后缀表达式

```
postfix = primary (call | index | dot | slice)*

call   = "(" (expression ("," expression)*)? ")"
index  = "[" expression "]"
dot    = "." IDENTIFIER
slice  = "[" slice_part? ":" slice_part? (":" slice_part)? "]"

slice_part = expression?
```

```ms
func(a, b)          # 函数调用
arr[0]              # 下标
obj.name            # 属性访问
arr[1:3]            # 切片
arr[::2]            # 步长切片
```

### 16. 初等表达式

```
primary = literal
        | IDENTIFIER
        | "super" "." IDENTIFIER                 // 父类属性/方法访问（可后接调用）
        | "(" expression ("," expression)* ")"   # 分组或元组
        | "[" list_content? "]"                  // 列表
        | "{" (dict_content | set_content)? "}"  // dict 或 set
        | fn_literal                             // 匿名函数
        | comprehension                          // 推导式

literal = INT | FLOAT | STRING | "true" | "false" | "nil"
```

#### 列表字面量

```
list_content = expression ("," expression)* ","?
```

```ms
[1, 2, 3]
["a", "b"]
[]
```

#### 推导式

```
comprehension = list_comp | dict_comp | set_comp | gen_expr

list_comp = "[" expression "for" IDENTIFIER ("," IDENTIFIER)? "in" expression ("if" expression)? "]"

dict_comp = "{" expression ":" expression "for" IDENTIFIER ("," IDENTIFIER)? "in" expression ("if" expression)? "}"

set_comp = "{" expression "for" IDENTIFIER ("," IDENTIFIER)? "in" expression ("if" expression)? "}"

gen_expr = "(" expression "for" IDENTIFIER ("," IDENTIFIER)? "in" expression ("if" expression)? ")"
```

嵌套推导式：

```
list_comp = "[" expression for_clause+ ("if" expression)? "]"
for_clause = "for" IDENTIFIER ("," IDENTIFIER)? "in" expression
```

> **作用域规则**：推导式内部创建隐式作用域。推导式中的循环变量（`for x in ...` 中的 `x`）不会泄漏到外层作用域。

#### Dict 字面量

```
dict_content = (expression ":" expression) ("," (expression ":" expression))* ","?
```

```ms
{"a": 1, "b": 2}
{}
```

#### Set 字面量

```
set_content = expression ("," expression)+ ","?
            | expression ","                        # 单元素 set（需尾部逗号）

非空花括号，且元素没有冒号分隔，解析为 set。

```ms
{1, 2, 3}
```

注意：`{}` 是空 dict，空 set 用 `set()`。单元素花括号 `{1}` 不匹配 set 语法，将被解析为分组表达式（等价于 `1`）。单元素 set 使用 `{1,}`（尾部逗号）或 `set([1])`。

#### 元组

```
tuple = "(" expression ("," expression)+ ","? ")"
      | expression "," expression ("," expression)*
```

```ms
(1, 2, 3)
1, 2, 3           # 裸元组（在允许的上下文中）
(42,)             # 单元素元组
()
```

### 运算符优先级总表

| 优先级 | 运算符 | 结合性 |
|---|---|---|
| 1（最低） | `=` `+=` `-=` 等 | 右 |
| 2 | `if...else`（三元） | 右 |
| 3 | `or` | 左 |
| 4 | `and` | 左 |
| 5 | `not` | 右（一元） |
| 6 | `== != < > <= >= in is` | 左 |
| 7 | `\|` | 左 |
| 8 | `^` | 左 |
| 9 | `&` | 左 |
| 10 | `<< >>` | 左 |
| 11 | `+ -` | 左 |
| 12 | `* / // %` | 左 |
| 13 | `- ~ <-`（一元） | 右 |
| 14 | `**` | 右 |
| 15（最高） | `() [] .`（后缀） | 左 |

## 作用域规则

mslang 使用**函数级作用域**（类似 Python）：

- 顶层是全局作用域
- 每个 `fn` 创建新作用域
- `if`/`while`/`for`/`with` 块**不创建**新作用域
- 内层函数可以访问外层变量（闭包）
- `var` 和 `:=` 在当前函数作用域创建新变量
- `=` 赋值时：仅在当前函数作用域内查找，找不到则在当前作用域创建新变量（不穿透外层）；使用 `nonlocal` 声明可绑定外层函数变量，使用 `global` 声明可绑定全局变量

```ms
x = 10               # 全局

fn foo() {
    print(x)          # 访问全局 x = 10
    var y = 20        # foo 作用域的局部变量
    z := 30           # foo 作用域的局部变量
}

fn bar() {
    x = 99            # 在 bar 作用域创建新局部 x（不修改全局）
    var x = 1         # 同上，创建局部变量
}
```

### 闭包与循环变量陷阱

`for` 循环不创建新作用域，循环变量在循环结束后保持最后一次的值。如果在循环内创建闭包捕获循环变量，所有闭包将共享同一个变量引用：

```ms
fns = []
for i in range(3) {
    fns.push(fn() { return i })
}
# 警告：所有 fns 调用都返回 2（循环结束值），不是 0, 1, 2
```

若需捕获每次迭代的值，使用默认参数技巧：

```ms
fns = []
for i in range(3) {
    fns.push(fn(captured = i) { return captured })
}
# 现在 fns[0]() 返回 0, fns[1]() 返回 1, fns[2]() 返回 2
```
