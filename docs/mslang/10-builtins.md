# 内置函数与标准库

## 内置函数

以下函数全局可用，无需 import。

### 类型转换

| 函数 | 签名 | 说明 |
|---|---|---|
| `int` | `int(val) -> int` | 转换为整数 |
| `float` | `float(val) -> float` | 转换为浮点数 |
| `str` | `str(val) -> string` | 转换为字符串 |
| `bool` | `bool(val) -> bool` | 转换为布尔值 |
| `list` | `list(val) -> list` | 转换为列表 |
| `tuple` | `tuple(val) -> tuple` | 转换为元组 |
| `set` | `set(val) -> set` | 转换为集合 |
| `dict` | `dict(val) -> dict` | 转换为字典 |

```ms
int("42")        # 42
int(3.7)         # 3
float("3.14")    # 3.14
str(42)          # "42"
bool(0)          # false
list("abc")      # ["a", "b", "c"]
tuple([1,2])     # (1, 2)
set([1,2,2])     # {1, 2}
```

### 类型检查

| 函数 | 签名 | 说明 |
|---|---|---|
| `type` | `type(val) -> string` | 返回类型名称 |
| `isinstance` | `isinstance(val, type) -> bool` | 是否为指定类型 |

```ms
type(42)              # "int"
type("hello")         # "string"
type([1,2])           # "list"
isinstance(42, int)      # true
isinstance("x", int)     # false
```

### I/O

| 函数 | 签名 | 说明 |
|---|---|---|
| `print` | `print(*args)` | 打印参数（空格分隔），追加换行 |
| `println` | `println(*args)` | **已弃用**。`print` 的别名，行为完全一致。推荐统一使用 `print` |
| `input` | `input(prompt?) -> string` | 读取用户输入 |
| `open` | `open(path, mode?) -> File` | 打开文件，返回文件对象（支持 `with` 语句） |

```ms
print("hello")                  # hello（末尾换行）
print("a", "b", "c")           # a b c（末尾换行）

name = input("Enter name: ")   # 带提示的输入

# open() 是全局内置函数，无需 import
with open("data.txt") as f {
    content = f.read()
}
# open() 等价于 io.open()，后者提供额外的选项
```

### 数学

| 函数 | 签名 | 说明 |
|---|---|---|
| `abs` | `abs(n) -> number` | 绝对值 |
| `max` | `max(*args) -> number` | 最大值 |
| `min` | `min(*args) -> number` | 最小值 |
| `sum` | `sum(iterable) -> number` | 求和 |
| `ceil` | `ceil(n) -> int` | 向上取整 |
| `floor` | `floor(n) -> int` | 向下取整 |
| `round` | `round(n, digits?) -> number` | 四舍五入 |

```ms
abs(-5)          # 5
max(1, 2, 3)    # 3
sum([1,2,3])    # 6
ceil(3.2)       # 4
floor(3.8)      # 3
round(3.14159, 2)  # 3.14
```

### 容器

| 函数 | 签名 | 说明 |
|---|---|---|
| `len` | `len(val) -> int` | 长度 |

`len(val)` 返回长度。推荐使用 `len()` 内置函数。`.length()` 方法作为遗留兼容保留，新代码应统一使用 `len()`。

| `range` | `range(end) -> iterator` | 0 到 end-1 |
| `range` | `range(start, end) -> iterator` | start 到 end-1 |
| `range` | `range(start, end, step) -> iterator` | 带步长 |
| `sorted` | `sorted(iterable, key?, reverse?) -> list` | 稳定排序（新 list）；key 为 1 参函数；reverse=true 反转比较器（等值元素保持原序） |
| `sorted_by` | `sorted_by(iterable, key, reverse?) -> list` | sorted 的 key 显式版 |
| `reversed` | `reversed(iterable) -> iterator` | 反转 |
| `enumerate` | `enumerate(iterable) -> iterator` | (index, value) 对 |
| `zip` | `zip(*iterables) -> iterator` | 并行迭代 |
| `map` | `map(fn, iterable) -> list` | 映射 |
| `filter` | `filter(fn, iterable) -> list` | 过滤 |
| `any` | `any(iterable) -> bool` | 任一为 truthy |
| `all` | `all(iterable) -> bool` | 全部为 truthy |

```ms
len("hello")          # 5
len([1,2,3])          # 3
range(5)              # 0,1,2,3,4
range(2, 8)           # 2,3,4,5,6,7
range(0, 10, 2)       # 0,2,4,6,8

sorted([3,1,2])       # [1,2,3]
reversed([1,2,3])     # [3,2,1]

# key / reverse（task 80）：稳定排序，等 key 元素保持原序
sorted(["bb","a","ccc"], fn(w) { return len(w) })   # ["a","bb","ccc"]
sorted([3,1,2], nil, true)                           # [3,2,1]
sorted_by(["bb","a","ccc"], fn(w) { return len(w) }) # 同 sorted(iter, key)

for i, v in enumerate(["a","b"]) {
    print(i, v)       # 0 a, 1 b
}

for a, b in zip([1,2], ["x","y"]) {
    print(a, b)       # 1 x, 2 y
}

map(fn(x) { return x*2 }, [1,2,3])   # [2,4,6]
filter(fn(x) { return x > 2 }, [1,2,3,4])  # [3,4]

any([false, false, true])    # true
all([true, true, false])     # false
```

### 其他

| 函数 | 签名 | 说明 |
|---|---|---|
| `id` | `id(val) -> int` | 对象唯一标识 |
| `hash` | `hash(val) -> int` | 哈希值 |
| `copy` | `copy(val) -> value` | 浅拷贝 |
| `deepcopy` | `deepcopy(val) -> value` | 深拷贝 |
| `assert` | `assert(cond, msg?)` | 断言 |
| `channel` | `channel(buffer_size?) -> Channel` | 创建 channel（见 [08-concurrency](08-concurrency.md)） |

```ms
ch = channel()       # 无缓冲 channel
ch = channel(10)     # 缓冲区大小为 10
```

## 内置方法

### string 方法

| 方法 | 说明 |
|---|---|
| `length()` | 字符串长度 |
| `upper()` | 转大写 |
| `lower()` | 转小写 |
| `strip()` | 去除两端空白 |
| `split(sep?)` | 分割为列表 |
| `join(list)` | 用字符串连接列表 |
| `replace(old, new)` | 替换子串 |
| `contains(sub)` | 是否包含子串 |
| `startswith(prefix)` | 是否以指定前缀开头 |
| `endswith(suffix)` | 是否以指定后缀结尾 |
| `index(sub)` | 查找子串位置 |
| `slice(start, end?)` | 切片 |

```ms
"Hello World".lower()           # "hello world"
"  trim  ".strip()              # "trim"
"a,b,c".split(",")             # ["a", "b", "c"]
"-".join(["a","b","c"])        # "a-b-c"
"hello".replace("l", "r")      # "herro"
"hello".contains("ell")        # true
"hello".startswith("hel")      # true
```

### list 方法

| 方法 | 说明 |
|---|---|
| `length()` | 列表长度 |
| `push(val)` | 尾部追加 |
| `pop()` | 弹出末尾元素 |
| `pop(index)` | 弹出指定位置元素 |
| `insert(index, val)` | 插入元素 |
| `remove(val)` | 删除第一个匹配 |
| `index(val)` | 查找元素位置 |
| `contains(val)` | 是否包含 |
| `sort(key?, reverse?)` | 原地稳定排序（key 为 1 参函数；reverse=true 反转比较器，等值保序） |
| `sort_by(key)` | 原地按键排序（sort 的 key 显式版） |
| `reverse()` | 原地反转 |
| `slice(start, end?)` | 切片 |
| `map(fn)` | 映射 |
| `filter(fn)` | 过滤 |
| `reduce(fn, init?)` | 归约 |

```ms
lst = [3, 1, 4, 1, 5]
lst.sort()                    # [1, 1, 3, 4, 5]
lst.push(9)                   # [1, 1, 3, 4, 5, 9]
lst.pop()                     # 9
lst.insert(0, 0)              # [0, 1, 1, 3, 4, 5]
lst.remove(1)                 # [0, 1, 3, 4, 5]

# key / reverse（task 80）
words = ["bb", "a", "ccc"]
words.sort(fn(w) { return len(w) })   # ["a", "bb", "ccc"]
words.sort_by(fn(w) { return -len(w) })
nums = [3, 1, 2]
nums.sort(nil, true)                  # [3, 2, 1]

[1,2,3].map(fn(x) { return x*2 })      # [2,4,6]
[1,2,3,4].filter(fn(x) { return x>2 }) # [3,4]
[1,2,3].reduce(fn(a,b) { return a+b }, 0) # 6
```

### dict 方法

| 方法 | 说明 |
|---|---|
| `length()` | 键值对数量 |
| `keys()` | 返回键列表 |
| `values()` | 返回值列表 |
| `items()` | 返回 (key, value) 对列表 |
| `get(key, default?)` | 获取值（不存在返回默认值） |
| `set(key, val)` | 设置键值 |
| `remove(key)` | 删除键 |
| `contains(key)` | 是否包含键 |
| `merge(other)` | 合并另一个 dict |

```ms
d = {"a": 1, "b": 2}
d.keys()               # ["a", "b"]
d.values()             # [1, 2]
d.items()              # [("a", 1), ("b", 2)]
d.get("c", 0)          # 0（键不存在返回默认值）
d.merge({"c": 3})      # {"a": 1, "b": 2, "c": 3}
```

### set 方法

| 方法 | 说明 |
|---|---|
| `length()` | 元素数量 |
| `add(val)` | 添加元素 |
| `remove(val)` | 删除元素 |
| `contains(val)` | 是否包含 |
| `union(other)` | 并集 |
| `intersection(other)` | 交集 |
| `difference(other)` | 差集 |

`set.remove(val)` 在元素不存在时抛出 `KeyError`。

## 标准库

### io

```ms
import io

# io.open() 提供与全局 open() 相同的文件打开功能
# 全局 open() 是 io.open() 的快捷方式，无需 import
f = io.open("file.txt", "r")    # 等价于 open("file.txt", "r")
content = f.read()              # 读取全部
lines = f.lines()               # 按行读取
f.close()                       # 关闭

f = io.open("file.txt", "w")
f.write("hello\n")              # 写入
f.close()

# 更推荐 with 方式
with io.open("file.txt") as f {
    print(f.read())
}

io.read_file("file.txt")        # 一次性读取
io.write_file("file.txt", content) # 一次性写入
io.exists("file.txt")           # 文件是否存在
```

### math

```ms
import math

math.pi             # 3.141592653589793
math.e              # 2.718281828459045
math.sqrt(16)       # 4.0
math.pow(2, 10)     # 1024.0
math.sin(math.pi/2) # 1.0
math.cos(0)         # 1.0
math.tan(0)         # 0.0
math.log(100)       # 4.605...
math.log2(8)        # 3.0
math.log10(100)     # 2.0
math.exp(1)         # 2.718...
```

task 80 扩充（3 常量 + 28 函数）：

```ms
import math

# 常量
math.tau               # 2π
math.inf               # 正无穷
math.nan               # NaN

# 反三角 / 双曲（域外返回 NaN，不抛错）
math.asin(1)           # π/2
math.atan2(1, 1)       # π/4
math.sinh(0)           # 0.0
math.acosh(1)          # 0.0

# 数值
math.cbrt(27)          # 3.0（立方根，负数域可用）
math.hypot(3, 4)       # 5.0（无中间溢出）
math.trunc(-3.7)       # -3（向零截断）
math.sign(-2.5)        # -1（-1/0/1；NaN → 0）
math.fmod(-7, 3)       # -1.0（C 截断取余，与 % 的地板取整区分）
math.modf(1.25)        # (0.25, 1.0)
math.copysign(3, -1)   # -3.0
math.degrees(math.pi)  # 180.0
math.radians(180)      # π

# 整数族（参数非法 → ValueError；checked 溢出 → OverflowError）
math.gcd(12, 18)       # 6（负数取绝对值；gcd(0,n)=n）
math.lcm(4, 6)         # 12（lcm(0,n)=0）
math.factorial(5)      # 120（范围 0-20，21 溢出）
math.comb(5, 2)        # 10（k>n → 0）
math.perm(5, 2)        # 20
math.isqrt(17)         # 4（⌊√n⌋）

# 谓词
math.is_nan(math.nan)  # true
math.is_inf(math.inf)  # true

# log 升级：log(x, base?)
math.log(8, 2)         # 3.0（base 2/10 走精确路径）
math.log(100, 10)      # 2.0 == math.log10(100)
# base=1 或 base<=0 → ValueError
```

#### API

| 函数/常量 | 签名 | 说明 |
|---|---|---|
| `tau` / `inf` / `nan` | `-> float` | 常量（inline Float） |
| `asin` / `acos` / `atan` | `(x) -> float` | 反三角；域外返回 NaN |
| `atan2` | `(y, x) -> float` | |
| `sinh` / `cosh` / `tanh` / `asinh` / `acosh` / `atanh` | `(x) -> float` | 双曲；域外返回 NaN |
| `cbrt` | `(x) -> float` | 立方根 |
| `hypot` | `(x, y) -> float` | √(x²+y²)，无中间溢出 |
| `trunc` | `(x) -> int` | 向零截断 |
| `sign` | `(x) -> int` | -1/0/1；NaN → 0 |
| `fmod` | `(x, y) -> float` | C 截断取余 |
| `modf` | `(x) -> tuple(float, float)` | (小数部分, 整数部分) |
| `copysign` | `(x, y) -> float` | x 幅值 + y 符号 |
| `degrees` / `radians` | `(x) -> float` | 角度/弧度互转 |
| `gcd` / `lcm` | `(a, b) -> int` | 非负；负数取绝对值；溢出 → OverflowError |
| `factorial` | `(n) -> int` | 0..=20；超范围 → OverflowError、负数 → ValueError |
| `comb` / `perm` | `(n, k) -> int` | k>n → 0；负数 → ValueError；溢出 → OverflowError |
| `isqrt` | `(n) -> int` | ⌊√n⌋；负数 → ValueError |
| `is_nan` / `is_inf` | `(x) -> bool` | 谓词 |
| `log` | `(x, base?) -> float` | base 缺省 e；base=1 / base<=0 → ValueError |

### os

```ms
import os

os.getenv("PATH")          # 环境变量
os.setenv("KEY", "val")    # 设置环境变量
os.getcwd()                # 当前工作目录
os.chdir("/tmp")           # 改变目录
os.exec("ls -la")          # 执行命令（注意：避免拼接不可信输入，防止命令注入）
os.exit(0)                 # 退出程序
os.args                    # 命令行参数列表
```

### string

```ms
import string

string.format("{} + {} = {}", 1, 2, 3)   # "1 + 2 = 3"
string.repeat("ab", 3)                    # "ababab"
string.reverse("hello")                   # "olleh"
string.is_alpha("abc")                    # true
string.is_digit("123")                    # true
```

task 80 扩充（18 函数 + format 增强）：

```ms
import string

# 查找
string.count("aaa", "a")          # 3（非重叠；空 sub → 0）
string.find("hello", "ll")        # 2（字符位置；未找到 -1）

# 大小写
string.title("hello world")       # "Hello World"
string.capitalize("hello WORLD")  # "Hello world"

# 填充（n 为结果总长，Python rjust/ljust 语义；pad 取首字符循环）
string.pad_start("42", 5)         # "   42"
string.pad_start("42", 5, "0")    # "00042"
string.pad_end("42", 5, "*")      # "42***"
string.center("abc", 7, "-")      # "--abc--"（左短右长）
string.zfill("-42", 5)            # "-0042"（符号保留）

# 行分割 / 修剪（\n / \r\n / \r 均识别；尾行尾不产生空行）
string.split_lines("a\nb\r\nc")   # ["a", "b", "c"]
string.trim_start("  x  ")        # "x  "
string.trim_end("  x  ")          # "  x"

# 谓词（空串均 false；is_upper/is_lower 需至少一个有大小写字母）
string.is_alnum("abc123")         # true
string.is_space(" \t\n")          # true
string.is_upper("ABC1")           # true
string.is_lower("abc1")           # true

# 切分 / 连接
before, after = string.cut("a,b,c", ",")   # ("a", "b,c")；无 sep → (s, "")
string.fields("  a \t b  ")       # ["a", "b"]（连续空白分割）
string.join("-", ["a", "b"])      # "a-b"（与 "-".join(list) 等价）

# format 增强：{{ / }} 字面转义 + {:.Nf} 定点（N ∈ 0..=9）
string.format("{{}}")             # "{}"
string.format("{:.2f}", 3.14159)  # "3.14"
string.format("{:.2f}", 3)        # "3.00"（Int 按 Float 格式化）
# 非数值 / 非法规格（{:x}、{:.10f}、单独 }、未闭合）→ ValueError/TypeError
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `count` | `(s, sub) -> int` | 非重叠出现次数；空 sub → 0 |
| `find` | `(s, sub) -> int` | 首个字符索引；未找到 -1（与 `s.index()` 抛错区分） |
| `title` | `(s) -> string` | 每词首字母大写其余小写 |
| `capitalize` | `(s) -> string` | 首字符大写其余小写 |
| `pad_start` / `pad_end` | `(s, n, pad=" ") -> string` | 左/右填充至总长 n |
| `center` | `(s, n, pad=" ") -> string` | 居中（左短右长） |
| `zfill` | `(s, n) -> string` | 左补零，符号保留 |
| `split_lines` | `(s) -> list` | 按行分割（\n / \r\n / \r） |
| `trim_start` / `trim_end` | `(s) -> string` | 去除首/尾空白 |
| `is_alnum` / `is_space` | `(s) -> bool` | 谓词（空串 false） |
| `is_upper` / `is_lower` | `(s) -> bool` | 谓词（需至少一个有大小写字母） |
| `cut` | `(s, sep) -> tuple(s0, s1)` | 首个 sep 切两段；无 sep → (s, "") |
| `fields` | `(s) -> list` | 连续空白分割 |
| `join` | `(sep, list) -> string` | 模块级，与 `sep.join(list)` 等价 |
| `format` | `(template, *args) -> string` | `{}` 顺序替换 + `{{`/`}}` 转义 + `{:.Nf}` 定点（N ∈ 0..=9） |

### time

```ms
import time

time.now()               # 当前时间戳（秒）
time.sleep(1)            # 休眠 1 秒
time.format(timestamp)   # 格式化时间
```

### json

```ms
import json

data = json.parse('{"name": "Alice"}')   # 解析 JSON
text = json.stringify(data)              # 序列化为 JSON
```

### path

```ms
import path

path.join("a", "b", "c")     # "a/b/c"
path.ext("file.txt")         # ".txt"
path.base("a/b/c.txt")       # "c.txt"
path.dir("a/b/c.txt")        # "a/b"
```

### async

```ms
import async

async.sleep(1000)            # 异步休眠（毫秒），返回 Future<nil>
async.timeout(fn, 5000)      # 带超时执行，返回 Future<value>；超时抛 TimeoutError
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `async.sleep(ms)` | `sleep(ms: int) -> Future<nil>` | 异步休眠指定毫秒数；`ms` 必须为非负 int（上限 24 小时），负数抛 `TypeError` |
| `async.timeout(fn, ms)` | `timeout(fn: function, ms: int) -> Future<value>` | 带超时执行 `fn`（async fn / 闭包）；超时 Future 被 reject 为 `TimeoutError`，fn 内部异常优先于超时 |

#### TimeoutError

`async.timeout` 超时时抛出的内置异常，父类为 `Error`：

```ms
try {
    await async.timeout(fn() {
        await async.sleep(10000)
    }, 50)
} except TimeoutError {
    print("timed out")
}
```

- 注册到内置异常表（`src/vm/mod.rs` `BUILTIN_EXCEPTION_NAMES` / `EXCEPTION_PARENTS`）
- `except TimeoutError` 直接匹配；与 `except Error` 也匹配（父类链）

### gc

```ms
import gc

gc.collect()                    # 触发 Full GC（Major + Minor）
gc.collect_minor()              # 仅触发 Minor GC
gc.enable()                     # 启用自动 GC
gc.disable()                    # 禁用自动 GC
gc.is_enabled()                 # 返回 bool

gc.set_threshold("major", 2.0)  # Old GC 触发比率（allocated > live * ratio 时触发）
gc.set_threshold("minor", 4)    # Young 代大小（MB）
gc.set_promotion_age(3)         # Young→Old 晋升年龄（范围 1-3）
gc.set_gc_threads(4)            # GC Worker 线程数

gc.stats()                      # 返回统计信息 dict
gc.count()                      # GC 总次数（minor + major）
gc.mem_alloc()                  # 当前堆分配字节数
gc.mem_live()                   # 当前存活字节数
```

`gc.stats()` 返回的 dict：

```ms
{
    "minor_count": 42,
    "major_count": 3,
    "total_pause_ns": 1520000,
    "last_pause_ns": 23000,
    "young_size": 4194304,
    "old_size": 1048576,
    "los_size": 0,
    "bytes_freed": 8388608,
    "promotion_age": 2,
    "gc_threads": 8,
    "gc_enabled": true,
}
```

GC 系统设计详见 [14-gc](14-gc.md)。

### 未文档化的标准库模块

以下模块已列入标准库结构但尚未定义完整 API，将在后续版本中补充：

| 模块 | 说明 |
|---|---|
| `regex` | 正则表达式匹配 |
| `http` | HTTP 客户端/服务端 |
| `net` | 网络操作（TCP/UDP） |
| `collections` | 高级数据结构（deque, heap 等） |
| `fs` | 文件系统操作（目录遍历、文件元数据等） |
| `test` | 测试框架（assert 辅助、mock 等） |
