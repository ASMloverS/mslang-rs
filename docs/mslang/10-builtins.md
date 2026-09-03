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

**安全警示**：`os.exec` 的命令字符串经 shell（Windows `cmd /C`、Unix `sh -c`）
执行，拼接不可信输入存在命令注入风险。结构化场景（命令与参数固定、含用户
数据）请改用 `os.run`（argv 列表不经 shell，无注入面）。

task 82 扩充（5 函数）：

```ms
import os

os.getpid()                              # 当前进程 ID（int）
os.hostname()                            # 主机名（env COMPUTERNAME/HOSTNAME）
e = os.environ()                         # 全量环境变量快照（dict）
os.unsetenv("KEY")                       # 删除环境变量
r = os.run(["cmd", "/C", "echo", "hi"])  # 不经 shell 执行
# r == {"status": 0, "stdout": "hi\r\n", "stderr": ""}
```

#### API（task 82 扩充）

| 函数 | 签名 | 说明 |
|---|---|---|
| `getpid` | `() -> int` | 当前进程 ID（正整数） |
| `hostname` | `() -> string` | env COMPUTERNAME/HOSTNAME；均缺失 → IOError（Linux 非交互 shell/CI 下 HOSTNAME 常未导出，已知限制） |
| `environ` | `() -> dict` | 全量环境变量快照（string→string）；经 `vars_os` + `to_string_lossy` 构建，无效 Unicode 项不 panic；Windows 键统一大写（与 Python os.environ 对齐，保证 `e["PATH"]` 命中），Unix 保留原样 |
| `unsetenv` | `(key) -> nil` | 删除环境变量（键不存在亦成功） |
| `run` | `(argv) -> dict` | `{"status","stdout","stderr"}`；argv 为非空 string list，不经 shell（无注入面） |

os.run 注意事项：

- 参数校验：argv 非 list / 空列表 / 含非 string 元素 → TypeError；启动失败
  （可执行不存在）→ IOError。
- status 序列化：正常退出为退出码（int）；Unix 下被信号杀死
  （`ExitStatus::code()` 为 None）统一 -1（platform 特定，不区分信号编号）。
- stdout/stderr 经 `from_utf8_lossy`（与 os.exec 一致）。
- 同步阻塞（与 os.exec 一致）：单线程协作事件循环下长命令会饿死其他协程。

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

time.now()               # 当前时间戳（秒，Float）
time.now_ms()            # 当前时间戳（毫秒，Int）
time.monotonic()         # 单调秒（Float，进程启动为 0 点；用于计时非报时）
time.sleep(1)            # 休眠 1 秒（阻塞）
time.sleep_ms(100)       # 休眠 100 毫秒（Int，阻塞；协程场景用 async.sleep）
time.format(timestamp)   # 格式化时间戳（固定 "YYYY-MM-DD HH:MM:SS"）
time.iso(1700000000)     # "2023-11-14T22:13:20Z"（缺省当前时间）
time.date_parts(0)       # {year:1970, month:1, ..., weekday:3}（缺省当前时间）
time.format_ts(0, "%Y-%m-%d %H:%M:%S")   # "1970-01-01 00:00:00"
time.parse("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S")  # → 1700000000.0
```

#### API（task 83 扩充）

| 函数 | 签名 | 说明 |
|---|---|---|
| `now_ms` | `() -> int` | Unix 毫秒 |
| `monotonic` | `() -> float` | 单调秒，进程启动为 0 点；用于计时非报时（不受系统时钟回拨影响） |
| `iso` | `(ts?) -> string` | `"YYYY-MM-DDTHH:MM:SSZ"`（UTC）；缺省当前时间 |
| `date_parts` | `(ts?) -> dict` | `{year, month, day, hour, minute, second, weekday}`；weekday 0=周一…6=周日（Python 约定；1970-01-01 为周四=3）；缺省当前时间 |
| `sleep_ms` | `(ms) -> nil` | 阻塞指定毫秒（Int）；负数 → ValueError。协程场景用 `async.sleep`（非阻塞），阻塞 sleep 会饿死其他协程 |
| `format_ts` | `(ts, fmt) -> string` | 指令集 `%Y %m %d %H %M %S %%`（UTC；%Y 4 位、其余 2 位零填充，字面段原样输出） |
| `parse` | `(s, fmt) -> float` | 同指令集解析为 Unix 秒；字面不匹配 / 域越界 / 多余输入 / 1970 前日期 → ValueError（与 json.parse 同名，各自自校验参数个数） |

- 时间一律 UTC、秒为 Float、毫秒为 Int；闰秒忽略；`time.format(ts)`（无 fmt）保留不动。
- ts 校验（iso / date_parts / format_ts）：接受 Int 与 Float；负数 → ValueError；
  Float 非有限值（NaN/±Inf）→ ValueError、超 i64 范围 → OverflowError（禁止静默饱和）。
- parse 域校验对齐 Python strptime：月 1-12、时 0-23、分/秒 0-59、日 1-31 且不超
  当月天数（含闰年规则，2 月 30 → ValueError）；`%Y` 贪婪 1-4 位、其余 1-2 位。

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

### random

```ms
import random

random.random()            # [0, 1) 均匀 Float
random.randint(1, 6)       # 闭区间 [1, 6] Int
random.uniform(5, 10)      # Float（端点不保证，Python 语义）
random.gauss(0, 1)         # 正态分布（Box–Muller）
random.choice("abc")       # 随机单字符 string（list/tuple/string 均可）
lst = [1, 2, 3]
random.shuffle(lst)        # 原地 Fisher–Yates 洗牌，返回 nil
random.sample([1, 2, 3], 2)    # 不放回采样，返回新 list
random.seed(42)            # 重置生成器（缺省系统熵）；此后序列确定
```

生成器为 thread_local `StdRng`：`seed(n)` 后序列确定（可测试），与 select 语句
随机分派的内部 `thread_rng` 互不影响。负 seed 按补码位模式转 u64。

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `random` | `() -> float` | [0,1) 均匀 |
| `randint` | `(a, b) -> int` | 闭区间 [a,b]（含 i64 全区间）；a>b → ValueError；非 int → TypeError |
| `uniform` | `(a, b) -> float` | a + (b-a)*random()；端点不保证（Python 语义） |
| `gauss` | `(mu, sigma) -> float` | Box–Muller 正态；sigma<0 → ValueError |
| `choice` | `(seq) -> value` | list/tuple/string（string 返回单字符 string）；空 → ValueError |
| `shuffle` | `(lst) -> nil` | 原地 Fisher–Yates；非 list → TypeError |
| `sample` | `(pop, n) -> list` | 不放回；string 总体返回单字符 string 的 list；n<0 或 n>len → ValueError |
| `seed` | `(n?) -> nil` | 重置生成器；缺省（或 nil）系统熵播种 |

### encoding

```ms
import encoding

encoding.base64_encode("foobar")   # "Zm9vYmFy"（RFC 4648 + '=' padding）
encoding.base64_decode("Zm9vYg==") # "foob"（ASCII 空白剔除后解码）
encoding.hex_encode("mslang")      # "6d736c616e67"（小写）
encoding.hex_decode("4D53")        # "MS"（大写输入等价）
encoding.url_encode("a b/c")       # "a%20b/c"（safe="/" 缺省，%HH 大写）
encoding.url_encode("a b/c", "")   # "a%20b%2Fc"（自定义 safe）
encoding.url_decode("a+b")         # "a+b"（`+` 保持字面，非 form 语义）
```

语言无 bytes 类型：解码结果须为合法 UTF-8，否则 ValueError（如
`url_decode("%FF")`）。非法字符/长度错误均附位置。

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `base64_encode` | `(s) -> string` | RFC 4648 标准字母表 + `=` padding |
| `base64_decode` | `(s) -> string` | 剔除 ASCII 空白；长度非 4 倍数 / 非法字符 / padding >2 → ValueError |
| `hex_encode` | `(s) -> string` | 字节十六进制小写 |
| `hex_decode` | `(s) -> string` | 大小写输入均接受；奇数长度 / 非 hex → ValueError |
| `url_encode` | `(s, safe="/") -> string` | 保留 `A-Za-z0-9-_.~` 与 safe；其余逐 UTF-8 字节 `%HH` 大写 |
| `url_decode` | `(s) -> string` | %XX 解码（大小写 hex 均接受）；`+` 字面；非法序列/UTF-8 → ValueError |

### uuid

```ms
import uuid

u = uuid.uuid4()   # "xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx"（36 字符小写）
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `uuid4` | `() -> string` | RFC 4122 版本 4；122 位熵（复用 random 模块生成器，seed 后可复现）；version/variant 位正确置位 |

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

### fs

```ms
import fs

fs.mkdir("a")                    # 单级创建；已存在 → IOError
fs.mkdirs("a/b/c")               # 递归创建；幂等（已存在目录成功）
fs.rmdir("a")                    # 删除空目录（非空/不存在 → IOError）
fs.remove("f.txt")               # 删除文件（目录 → IOError）
fs.remove_all("dir")             # 递归删除；不存在返回 nil（幂等，Go RemoveAll）
fs.rename("old", "new")          # 重命名/移动
fs.copy("src.txt", "dst.txt")    # 文件复制；dst 存在则覆盖，dst 为目录 → IOError
fs.list_dir("dir")               # 子项文件名 list（排序返回，不含 `.`/`..`）
fs.walk("dir")                   # 递归先序全路径扁平 list（含 dir 自身；不跟随符号链接）
fs.is_dir("p")                   # 谓词（is_file / is_abs 同型）
fs.abs("p")                      # 词法绝对化（不解析符号链接）
fs.size("f.txt")                 # 字节数（int）
fs.mtime("f.txt")                # 修改时间（Unix 秒，float）
fs.temp_dir()                    # 系统临时目录
fs.home_dir()                    # env USERPROFILE/HOME；均缺失 → IOError
```

与 io 模块分工：read_file/write_file/exists/open（内容读写）保留在 io，
不在 fs 重复。错误一律 IOError 前缀。

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `mkdir` | `(path) -> nil` | 单级创建；已存在（文件/目录）→ IOError |
| `mkdirs` | `(path) -> nil` | 递归创建；幂等 |
| `rmdir` | `(path) -> nil` | 仅空目录；非空 → IOError |
| `remove` | `(path) -> nil` | 删除文件；目录 → IOError（目录用 remove_all） |
| `remove_all` | `(path) -> nil` | 递归删除；路径不存在返回 nil（幂等） |
| `rename` | `(old, new) -> nil` | 重命名/移动 |
| `copy` | `(src, dst) -> nil` | 文件→文件；dst 存在则覆盖；dst 为目录 → IOError（不自动拼接文件名，显式优于隐式） |
| `list_dir` | `(path) -> list` | 子项文件名排序返回（跨平台确定；不含 `.`/`..`） |
| `walk` | `(path) -> list` | DFS 严格先序全路径扁平 list（含 path 首元素）；与 list_dir 同排序；不跟随符号链接 |
| `is_dir` / `is_file` / `is_abs` | `(path) -> bool` | 谓词 |
| `abs` | `(path) -> string` | `std::path::absolute` 词法绝对化；不解析符号链接 |
| `size` | `(path) -> int` | 字节数 |
| `mtime` | `(path) -> float` | 修改时间（Unix 秒） |
| `temp_dir` | `() -> string` | 系统临时目录 |
| `home_dir` | `() -> string` | env USERPROFILE/HOME；均缺失 → IOError |

注意事项：

- `copy` 与全局内置 `copy(val)` 同名不同 arity：native_arities 升级 MAX 后
  两者各自自校验（fs.copy 恰 2 参、全局 copy 恰 1 参），并存无冲突（§2.2）。
- `abs` 平台差异：Unix 保留 `..`（仅前置 cwd 词法拼接）；Windows 经
  GetFullPathNameW 会词法归一 `..`。可移植代码只依赖「结果为绝对路径」不变量。
- `walk` 顺序与 Go filepath.Walk 一致：父目录条目先于其子目录内容，兄弟按
  字典序，后继兄弟排在先前兄弟的整个子树之后。

### sys

```ms
import sys

sys.platform()         # "windows" / "linux" / "macos"（cfg! 编译期映射）
sys.version()          # "mslang 0.1.0"（与 Cargo.toml 自动同步）
sys.executable()       # 当前解释器绝对路径；失败（二进制已删等）→ IOError
sys.stdin_read_all()   # 读 stdin 至 EOF（管道/重定向场景）
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `platform` | `() -> string` | "windows" / "linux" / "macos"（`cfg!` 编译期映射） |
| `version` | `() -> string` | "mslang {CARGO_PKG_VERSION}"（env! 编译期读取，与 Cargo.toml 自动同步） |
| `executable` | `() -> string` | current_exe 绝对路径；失败（二进制已删等）→ IOError |
| `stdin_read_all` | `() -> string` | 读 stdin 至 EOF；非 UTF-8 → IOError（lossy 不可逆，宁可报错）；交互 REPL 下阻塞至 EOF，仅面向 `ms run script.ms < input` 管道/重定向用法 |

### heapq

```ms
import heapq

lst = [7, 2, 9, 4, 3]
heapq.heapify(lst)             # 原地建堆（最小堆）→ nil
heapq.heap_push(lst, 1)        # 尾插 + 上浮 → nil
smallest = heapq.heap_pop(lst) # 弹出最小（1）；空堆 → IndexError
v = heapq.push_pop(lst, 0)     # 合并语义：0 ≤ 堆顶直返 0（一次 sift）
top2 = heapq.n_largest(lst, 2) # 前 2 大（降序），不改原 list
low2 = heapq.n_smallest(lst, 2)# 前 2 小（升序）；n≤0 → []
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `heapify` | `(lst) -> nil` | 原地建堆（sift-down 自底向上，O(n)） |
| `heap_push` | `(lst, v) -> nil` | 尾插 + sift-up |
| `heap_pop` | `(lst) -> value` | 首位弹出（尾元素补首 + sift-down）；空 → IndexError |
| `push_pop` | `(lst, v) -> value` | push 后立即 pop 最小（合并语义，一次 sift）：lst 空直返 v；v ≤ 堆顶直返 v；否则弹原堆顶、v 入堆 |
| `n_largest` | `(lst, n) -> list` | 前 n 大（降序）；n≤0 → []；n≥len 全量排序返回；不改原 list |
| `n_smallest` | `(lst, n) -> list` | 前 n 小（升序）；边界同上 |

注意事项：

- **排序类型限制**：比较走对象 `compare`（同 `sorted`），仅 Int/Float/String
  可排序；Instance 等其余类型 → TypeError（原生错误不可捕获）。`<` 运算符
  经 `__lt__` 分派可支持自定义类，但 heapq 不走该路径——两者语义有差异。
- 混合 Int/Float 数值可比较；int 与 string 混合 → TypeError。

### collections

```ms
import collections

# deque：两端均摊 O(1)（list 容量倍增 + head 偏移循环缓冲）
d = collections.deque()
d.push_back(2)
d.push_front(1)
d.extend([3, 4])
d.to_list()          # [1, 2, 3, 4]
len(d)               # 4（__len__）
for x in d { ... }   # for-in（__iter__ 生成器）
d.front(); d.back()  # 两端窥视；空 → IndexError
d.pop_front()        # 空弹出 → IndexError
d.pop_back()
d.is_empty()

# Counter：缺失键读 0 不写入（Python 语义）
c = collections.Counter(["a", "b", "a"])
c["a"]               # 2
c["z"]               # 0（不写入）
c.update("xya")      # iterable 逐个计数
c.most_common()      # 按频次降序 [(a,3),(b,1)...]（等频保插入序）
c.most_common(2)     # 前 n 项
c.elements()         # 生成器：按计数重复展开
c.items(); c.get("z")   # get 缺省 0

# defaultdict：__getitem__ 缺失触发 factory
dd = collections.defaultdict(fn() { return [] })
dd["k"].push(1)      # factory() 存入并返回
dd.get("nope")       # nil（get 不触发 factory，Python 一致）
dn = collections.defaultdict(nil)
dn["x"]              # factory 为 nil → KeyError
```

注意事项：

- 三个 class 均为 class 实例（dict 为内建类型不可继承），经 `__len__`/
  `__getitem__`/`__iter__` 魔术方法接通 `len()`/`[]`/for-in。
- Counter 的 `c[k]` 读缺失不写入；`most_common` 依赖 `sorted_by`（等频稳定）。

### itertools

```ms
import itertools

# 无限序列配合 islice/take_while 消费
for x in itertools.islice(itertools.count(10, 5), 0, 3) { }   # 10, 15, 20
for x in itertools.islice(itertools.count(1), 3, nil) { }     # stop=nil 无限（跳过前 3 项）
for x in itertools.islice(itertools.cycle([1, 2]), 0, 5) { }  # 1, 2, 1, 2, 1
for x in itertools.repeat(7, 3) { }                           # 7, 7, 7

itertools.chain([1, 2], "ab")            # 串接（零参 → 空迭代）
itertools.take_while(fn(x) { return x < 3 }, [1, 2, 3, 1])   # 1, 2
itertools.drop_while(fn(x) { return x < 3 }, [1, 2, 3, 1])   # 3, 1
itertools.pairwise([1, 2, 3])            # (1,2), (2,3)
itertools.accumulate([1, 2, 3], fn(a, b) { return a * b })   # 缺省 +；1, 2, 6
itertools.zip_longest([1, 2], [3])       # (1,3), (2,nil)（fill=nil 固定）
itertools.product([1, 2], "xy")          # 笛卡尔积（右端变化最快）
itertools.combinations([1, 2, 3], 2)     # C(3,2) 字典序
itertools.permutations([1, 2, 3], 2)     # P(3,2)；r 缺省 = len
itertools.islice(it, 1, 6, 2)            # 索引切片
itertools.batched([1, 2, 3, 4, 5], 2)    # (1,2), (3,4), (5)（尾批可不满）
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `count` | `(start=0, step=1)` | 无限计数（生成器） |
| `cycle` | `(iter)` | 无限循环（先物化为 list；空输入立即结束） |
| `repeat` | `(x, n?)` | 重复 n 次；缺省无限。与 string.repeat 同名共存（.ms 闭包不经 native_arities） |
| `chain` | `(*iters)` | 串接；零参 → 空迭代 |
| `take_while` | `(pred, it)` | 谓词假即止 |
| `drop_while` | `(pred, it)` | 一次失假全放行 |
| `pairwise` | `(it)` | 相邻对 (prev, cur) |
| `accumulate` | `(it, fn?)` | 前缀累积；缺省 `+`（判 nil） |
| `zip_longest` | `(*iters)` | 补 nil 对齐；fill=nil 固定（无命名参数） |
| `product` | `(*iters)` | 笛卡尔积；零参产出一个空 tuple；含空输入不产出 |
| `combinations` | `(it, r)` | 组合（输入先物化，字典序）；r<0 → ValueError（急切） |
| `permutations` | `(it, r?)` | 排列；r 缺省 = len；r<0 → ValueError（急切） |
| `islice` | `(it, start, stop, step=1)` | 索引切片；**stop=nil 无限**；start/stop 负或 step<1 → ValueError（急切） |
| `batched` | `(it, n)` | 按 n 分批 yield tuple；尾批可不满；n<1 → ValueError（Python 3.12，急切） |

注意事项：

- 全部惰性生成器；参数校验为**急切**（调用即抛，Python 语义，可 try/except
  捕获）——生成器体内 throw 在 resume 时不可捕获。
- `repeat` 与 string 方法 `repeat` 同名共存无冲突（前者 .ms 闭包、后者原生）。

### functools

```ms
import functools

add3 = fn(a, b, c) { return a + b + c }
p = functools.partial(add3, 1, 2)
p(3)                 # 6（args 在前，*more 在后）

slow = fn(x) { ... }
fast = functools.memoize(slow)     # dict 缓存，键 tuple(args)
fast(21); fast(21)                 # 命中：副作用仅执行一次

functools.reduce(fn(a, b) { return a + b }, [1, 2, 3], 100)  # 106
functools.reduce(fn(a, b) { return a + b }, [1, 2, 3])       # 6（首元素作种子）
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `partial` | `(fn, *args)` | 可调用实例（类即工厂），调用时 args 在前、*more 在后 |
| `memoize` | `(fn)` | dict 缓存，键 `tuple(args)`；unhashable → TypeError（dict 行为上抛，原生不可捕获）；无 LRU 上限 |
| `reduce` | `(fn, iter, init?)` | iterable 级归约（list.reduce 方法保留不动）；init=nil 视为未提供；空迭代无 init → TypeError |

注意事项：

- mslang 无调用点展开（`f(*args)`），partial/memoize 以 **0-4 参阶梯**分派
  动态实参；目标函数 arity > 4 → TypeError。

### test

```ms
import test as testmod

testmod.assert_eq(1, 1, "可选 msg")
testmod.assert_ne(1, 2)
testmod.assert_true(cond)
testmod.assert_false(cond)
testmod.assert_almost_eq(0.1 + 0.2, 0.3)          # 默认 eps=1e-9
testmod.assert_almost_eq(a, b, 0.01, "msg")       # 自定义 eps（nil 回退默认）
testmod.assert_raises(fn() { throw ValueError("x") }, "ValueError")
testmod.assert_len([1, 2], 2)
testmod.assert_contains("hello", "ell")
testmod.fail("直接失败")
```

#### API

| 函数 | 签名 | 说明 |
|---|---|---|
| `assert_eq` | `(a, b, msg?)` | 失败抛 AssertionError，消息含 `str(a)`/`str(b)` 与可选 msg |
| `assert_ne` | `(a, b, msg?)` | 同上（`!=`） |
| `assert_true` | `(cond, msg?)` / `assert_false` | 谓词断言 |
| `assert_almost_eq` | `(a, b, eps=1e-9, msg?)` | 数值；`abs(a-b) <= eps`；eps 显式 nil 回退默认 |
| `assert_raises` | `(fn, exc_class, msg?)` | 调 fn()：未抛 → AssertionError；类不匹配 → AssertionError |
| `assert_len` | `(v, n, msg?)` | `len(v) == n` |
| `assert_contains` | `(coll, item, msg?)` | `item in coll` |
| `fail` | `(msg?)` | 直接抛 AssertionError |

注意事项：

- **assert_raises 类匹配为精确类名比对，不含父类链**：匹配机制用 `e.type`
  （异常实例的类名字符串），`exc_class` 传类名字符串（如 `"ValueError"`）。
  `throw ValueError(...)` 不能被断言为 `"Error"` 命中——与 `except` 的
  CATCH 父类链语义有差异。
- 仅捕获 .ms 侧 `throw` 的异常；原生错误（heapq/IO 等）不可捕获。
- `assert_almost_eq` 的 eps 为位置参数，不支持关键字跳过；显式传 nil 回退默认。

### 未文档化的标准库模块

以下模块已列入标准库结构但尚未定义完整 API，将在后续版本补充：

| 模块 | 说明 |
|---|---|
| `regex` | 正则表达式匹配 |
| `http` | HTTP 客户端/服务端 |
| `net` | 网络操作（TCP/UDP） |
