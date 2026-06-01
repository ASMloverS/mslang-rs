# 类型系统

## 概述

mslang 采用**纯动态类型**系统：

- 变量不需要声明类型
- 变量可以在运行时改变其持有的值的类型
- 类型检查在运行时进行
- 每个**值**（而非变量）有明确的类型

```ms
s2 = set()           # 空集合（{} 是空 dict）
```

## 内置类型

### Nil

```
type: nil
值: nil
```

表示"无值"或"不存在"。

- 未初始化的变量默认为 `nil`
- 函数无显式 `return` 时返回 `nil`
- 字典中不存在的键返回 `nil`（而非报错）
- `nil` 是 falsy 值

### Bool

```
type: bool
值: true, false
```

布尔类型，仅两个值。

**Truthy 值**：`true`、非零数值、非空字符串、非空集合  
**Falsy 值**：`false`、`nil`、`0`、`0.0`、`""`、空集合（`[]`、`{}`、`()`、`set()`）

### Int

```
type: int
值: 64位有符号整数
范围: -2^63 ~ 2^63 - 1
```

整数类型，支持十进制、十六进制、二进制、八进制字面量。

```
a = 42
b = 0xFF
c = 0b1010
d = 0o755
```

**运算**：

| 运算 | 示例 | 结果 |
|---|---|---|
| 加减乘除 | `10 + 3` | `13` |
| 整除 | `10 // 3` | `3` |
| 取模 | `10 % 3` | `1` |
| 幂运算 | `2 ** 10` | `1024` |
| 位运算 | `5 & 3` | `1` |
| 左移/右移 | `1 << 4` | `16` |

**整除规则**：向负无穷方向取整（与 Python 一致）。

```
-7 // 2 == -4    # 向负无穷取整
7 // 2 == 3
```

**溢出行为**：int 为 64 位有符号整数（i64），溢出时抛出 `OverflowError`。算术运算结果超出 i64 范围时触发。

### Float

```
type: float
值: 64位双精度浮点数 (IEEE 754)
```

```
pi = 3.14159
e = 2.71828
speed = 1.5e8
```

**注意**：int 和 float 混合运算结果为 float。

**特殊浮点值**：
- `NaN`：`0.0 / 0.0` 产生 NaN。NaN 不等于任何值（包括自身）：`NaN == NaN` 为 `false`。使用 `x != x` 检测 NaN。
- `Infinity`：`1.0 / 0.0` 产生正无穷，`-1.0 / 0.0` 产生负无穷。
- `-0.0`：负零与 `0.0` 相等（`0.0 == -0.0` 为 `true`）。

```
1 + 2.0   # 3.0 (float)
10 / 3    # 3.3333... (float)
10 // 3   # 3 (int)
```

### String

```
type: string
值: UTF-8 编码的不可变字符串
```

- 仅双引号字面量
- 不可变（修改操作返回新字符串）
- 支持 `+` 拼接和 `*` 重复
- 支持下标访问和切片（见切片规范）
- 支持成员运算符 `in`

```ms
s = "hello"
s2 = s + " world"    # "hello world"
s3 = "ab" * 3        # "ababab"
ch = s[0]            # "h"
len = s.length()     # 5
```

### List

```
type: list
值: 有序可变序列，可存储任意类型元素
```

```ms
nums = [1, 2, 3, 4, 5]
mixed = [1, "two", true, nil]
nested = [[1, 2], [3, 4]]
```

**操作**：

| 操作 | 示例 | 说明 |
|---|---|---|
| 下标访问 | `lst[0]` | 从0开始 |
| 切片 | `lst[1:3]` | 见切片规范 |
| 追加 | `lst.push(val)` | 尾部追加 |
| 弹出 | `lst.pop()` | 弹出末尾元素 |
| 长度 | `lst.length()` | 元素个数 |
| 包含 | `val in lst` | 是否包含 |
| 拼接 | `lst1 + lst2` | 返回新列表 |

**List 是可变的**：

```ms
lst = [1, 2, 3]
lst[0] = 99     # lst 现在是 [99, 2, 3]
```

### Dict

```
type: dict
值: 有序可变映射，键值对集合
```

键必须为可哈希类型（int, float, bool, string, nil, tuple）。

```ms
person = {
    "name": "Alice",
    "age": 30,
    0: "zero key"
}
```

**操作**：

| 操作 | 示例 | 说明 |
|---|---|---|
| 访问 | `d["key"]` | 不存在返回 nil |
| 设置 | `d["key"] = val` | 设置或覆盖 |
| 删除 | `d.remove("key")` | 删除键 |
| 包含 | `"key" in d` | 键是否存在 |
| 长度 | `d.length()` | 键值对数 |

**访问语义**：`d["key"]` 访问不存在的键返回 `nil`（不抛异常）。`d.remove("key")` 删除不存在的键抛出 `KeyError`。需要区分"键不存在"和"值为 nil"时，使用 `d.contains("key")` 或 `d.get("key", sentinel)`。

> **设计注**：`d["key"]` 返回 nil 的设计意味着无法通过下标操作区分"键不存在"和"值为 nil"两种情况。这是有意的取舍（简化常见路径）。需要严格区分的场景应使用 `d.contains()` 或 `d.get()` with sentinel 模式。

**Dict 保持插入顺序**（与 Python 3.7+ 一致）。

### Tuple

```
type: tuple
值: 有序不可变序列
```

```ms
point = (1, 2)
single = (42,)       # 单元素元组必须有逗号
empty = ()
```

- 不可变：创建后不能修改
- 可哈希（当所有元素都可哈希时）
- 主要用于多返回值和 dict 键

**元组解包**：

```ms
a, b, c = (1, 2, 3)
a, b = b, a          # 交换
q, r = divmod(10, 3)
```

### Set

```
type: set
值: 无序可变集合，元素唯一
```

元素必须为可哈希类型。

```ms
s = {1, 2, 3}
s2 = set()           # 空集合（{} 是空 dict）
```

**操作**：

| 操作 | 示例 | 说明 |
|---|---|---|
| 添加 | `s.add(val)` | 添加元素 |
| 删除 | `s.remove(val)` | 删除元素 |
| 包含 | `val in s` | 是否存在 |
| 并集 | `s1 | s2` | 返回新集合 |
| 交集 | `s1 & s2` | 返回新集合 |
| 差集 | `s1 - s2` | 返回新集合 |
| 对称差 | `s1 ^ s2` | 返回新集合 |

## 类型转换

内置函数进行显式类型转换：

```ms
int("42")        # 42
int(3.7)         # 3
float("3.14")    # 3.14
float(42)        # 42.0
str(42)          # "42"
bool(0)          # false
bool("")         # false
list("abc")      # ["a", "b", "c"]
tuple([1,2])     # (1, 2)
set([1,2,2])     # {1, 2}
```

## 类型判断

```ms
type(42)              # "int"
type("hello")         # "string"
type([1,2])           # "list"
isinstance(42, int)            # true
isinstance("hello", string)    # true
```

**`isinstance` 类型参数**：`isinstance(val, type_obj)` 的第二个参数接受类型名称（如 `int`、`string`）。这些名称既是内置转换函数，也是类型对象。当用作 `isinstance` 的第二个参数时，按类型对象语义解释。用户自定义类的类名也可用于 `isinstance` 检查。

```ms
class Animal {}
a = Animal()
isinstance(a, Animal)    # true
isinstance(a, Object)    # true（所有实例隐式继承 Object）
```

## 运算符类型规则

### 算术运算

| 左 | 运算 | 右 | 结果类型 |
|---|---|---|---|
| int | `+ - * // % ** & \| ^ << >>` | int | int |
| int | `/` | int | float |
| int | `+ - * / // % **` | float | float |
| int | `& \| ^ << >>` | float | TypeError |
| float | `+ - * / // % **` | float | float |
| float | `& \| ^ << >>` | any | TypeError |
| string | `+` | string | string（拼接） |
| string | `*` | int | string（重复） |
| list | `+` | list | list（拼接） |
| list | `*` | int | list（重复） |

> **注**：int 与 float 混合的 `//`、`%`、`**` 运算结果为 float。例如 `7 // 2.0` → `3.0`，`2 ** 0.5` → `1.4142...`。位运算符（`& | ^ << >> ~`）仅接受 int 操作数，float 或其他类型操作数抛出 `TypeError`。

### 比较运算

同类型比较直接比较值。不同类型的比较规则：

| 左 | 比较 | 右 | 行为 |
|---|---|---|---|
| int | `== !=` | float | 数值比较 |
| int | `< > <= >=` | float | 数值比较 |
| 其他不同类型 | 比较 | — | `==` 返回 false，`< >` 抛出运行时错误 |

### `is` 运算符语义

`is` 执行**身份比较**（两个引用是否指向同一个堆对象），仅适用于引用类型（String、List、Dict、Tuple、Set、Instance、Class、Function 等）。

对内联值（int、float、bool、nil），`is` **抛出 `TypeError`**。这些类型没有堆身份的概念，应使用 `==` 进行值比较。

```ms
a = [1, 2]
b = [1, 2]
a == b     # true（值相等）
a is b     # false（不同对象）

x = 42
y = 42
x == y     # true
x is y     # TypeError: 'is' cannot be used with inline type 'int'
```

### 逻辑运算

短路求值：

```ms
a and b    # a 为 falsy 则返回 a，否则返回 b
a or b     # a 为 truthy 则返回 a，否则返回 b
not a      # 返回 bool
```

注意：`and`/`or` 返回的是实际值（不一定是 bool），这与 Python 一致。

## 可哈希类型

以下类型可以作为 dict 的键或 set 的元素：

- int
- float
- bool
- string
- nil
- tuple（所有元素可哈希）

不可哈希的类型：list, dict, set, class 实例。

> **注意**：`NaN` 不可作为 dict 键或 set 元素（`hash(NaN)` 抛出 `TypeError`）。`-0.0` 和 `0.0` 的哈希值相同，视为同一键（`{-0.0: 1, 0.0: 2}` 结果为 `{0.0: 2}`）。
