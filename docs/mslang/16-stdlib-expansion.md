# 标准库扩展设计（stdlib expansion）

> 状态：**草案，待逐节确认**。本文档为 M0-M8 全部标准库扩展的设计与实现方案总纲；
> 确认后按 §6 里程碑拆分为 task 78-86 逐一实现。
> 关联：[10-builtins](10-builtins.md)（现行 stdlib API）、[12-implementation-plan](12-implementation-plan.md) Phase 6、
> tasks 46-49 / 60 / 61（io/math/os/string/time/path/json/gc/async 现有实现）。

## 0. 摘要

- 拆分 `src/vm/stdlib.rs`（5010 行）为 `src/vm/stdlib/` 目录，每模块一文件（M0）。
- 新增嵌入式 `.ms` 标准库机制：`include_str!` 内置源码模块，单二进制自足（M1）。
- 零依赖新增/扩充 12 项：math、string、排序（sorted key）、random、encoding、uuid、
  fs、os、sys、time、heapq（原生）；collections、itertools、functools、test（.ms 嵌入）（M2-M6）。
- 引入 4 个 crate：`regex`、`md-5`、`sha1`、`sha2`，新增 regex / hash 模块（M7）。
- http 客户端：手写 HTTP/1.1（无 TLS），配套**后台线程 + Future 完成**基础设施（M8）。

## 1. 背景与现状

现有原生模块（`src/vm/stdlib.rs`，注册于 `src/vm/mod.rs:415-509`）：

| 模块 | 已有 API |
|---|---|
| io | open/read_file/write_file/exists + FileHandle.read/write/lines/close |
| math | pi/e + sqrt/pow/abs/sin/cos/tan/log/log2/log10/exp/ceil/floor/round |
| os | getenv/setenv/getcwd/chdir/exec/exit/args |
| string | format(仅 {})/repeat/reverse/is_alpha/is_digit |
| time | now/sleep/format(单一 UTC 格式) |
| path | join/ext/base/dir |
| json | parse/stringify |
| gc | collect/调优/统计 14 个 |
| async | sleep/timeout |

缺口：`10-builtins.md` §未文档化的标准库模块 承诺未实现的 regex/http/net/collections/fs/test；
另缺 random/encoding/uuid/heapq/itertools/functools/sys；sorted 无 key；string/math/time/os 本体过薄。

约束事实（设计据此展开）：

- 语言无命名参数，仅位置参数 + 默认值 + `*args`（task 31）。
- 无 bytes 类型，String 为唯一文本载体（UTF-8）。
- `native_arities` 为**按函数名共享**的全局校验表（`src/vm/mod.rs:254`）：同名函数必须同
  arity，不同 arity 时以 `usize::MAX` 注册并各自自校验。
- 事件循环单线程协作式；现有阻塞调用仅 os.exec。
- `ModuleResolver` 支持 `.ms` 磁盘模块（`@std:` 前缀 + stdlib/ 目录），但从未启用。
- 测试基建：`tests/ms/**/*.ms` 语料由 `tests/ms_corpus.rs` 自动执行（`.expected` stdout 对比）。

## 2. 总体设计原则

### 2.1 模块形态判据（原生 Rust vs .ms）

| 判据 | 原生 Rust | .ms（嵌入） |
|---|---|---|
| 系统/OS 资源 | fs/sys/os 扩充/http | — |
| 外部 crate 或手写算法密集 | regex/hash/encoding/uuid/random | — |
| 直接操作 GC 堆容器 | heapq（操作 list） | — |
| 纯算法、可用 class/生成器表达 | — | collections/itertools/functools/test |

`.ms` 模块由 `include_str!` 嵌入（§3.2），无需发行 `stdlib/` 目录。

### 2.2 注册模式与 native_arities 冲突规则

沿用现状：`register_<mod>_module()` + `VM::new` 注册 + `native_arities` 登记。

**同名冲突治理**（新增函数名与既有注册同名时）：

| 名字 | 现有 | 新增 | 处理 |
|---|---|---|---|
| format | string.format = MAX | — | 已 MAX，兼容 |
| join | path.join = MAX | string.join | 保持 MAX，各自自校验 |
| log | math.log = 1 | math.log(x, base?) | 升级 MAX + 自校验 1-2 参 |
| count | gc.count = 0 | string.count | 升级 MAX；gc.count / string.count 各自自校验 |
| parse | json.parse = 1 | time.parse | 升级 MAX；两者各自自校验 |
| repeat | string.repeat = 2 | itertools.repeat | 升级 MAX；两者各自自校验 |
| sorted | builtin sorted = 1 | sorted(iter, key?, reverse?) | 升级 MAX + 自校验 1-3 参 |
| copy | builtin copy = 1 | fs.copy | 升级 MAX；两侧各自自校验（task 82 审核发现，回写） |

规则：凡注册为 MAX 的原生函数**必须**自校验参数个数并返回 TypeError；实现期加入
「同名函数 arity 交叉调用」回归用例（如 `gc.count()` 与 `string.count("aa","a")` 并存）。
### 2.3 错误与命名约定

- 错误字符串沿用 `前缀: 消息` 约定：`TypeError`（类型不符）/ `ValueError`（值域非法）/
  `IOError`（文件与系统资源、网络）/ `IndexError` / `KeyError` / `OverflowError`。
- 参数校验消息带签名提示（沿用 `expect_string(args.get(0), "open(path, mode?)")` 风格）。
- 数值边界：f64→i64 转换必须经 `float_to_int`（NaN/溢出显式报错，禁止静默饱和）。
- 命名：模块函数 snake_case；与 Python 同名者语义对齐 Python，与 Go 同名者对齐 Go；
  两者冲突时优先 Python 语义（语言观感更接近 Python）。
- 时间一律 UTC；秒为 Float、毫秒为 Int（与现有 time.now / async.sleep 一致）。

### 2.4 验收约定（每个里程碑）

1. Rust 单元测试（各模块文件内 `#[cfg(test)]`，模式沿用 stdlib.rs:2496 现有 tests 模块）。
2. `tests/ms/stdlib/test_<模块>.ms` 语料；输出确定的部分配 `.expected`。
3. 同步更新 `docs/mslang/10-builtins.md`（API 表）与 `docs/mslang/tasks/README.md` 索引。
4. `cargo test` 全绿（含 ms_corpus 全量回归）。

### 2.5 依赖策略

| crate | 版本 | 用途 | 引入时机 |
|---|---|---|---|
| regex | 1 | regex 模块 | M7 |
| md-5 | 0.10 | hash.md5 | M7 |
| sha1 | 0.10 | hash.sha1 | M7 |
| sha2 | 0.10 | hash.sha256 / sha512 | M7 |

不引入：网络库（http 手写，§4.18）、chrono（time 手写，沿用 unix_to_ymdhms）、
rand_distr（gauss 用 Box–Muller 手写）。random 复用既有 rand 依赖。

## 3. 代码组织

### 3.1 stdlib.rs 拆分（M0）

```
src/vm/stdlib/
├── mod.rs        # 公共 helper（expect_string/expect_number/float_to_int/expect_int/hash_key/expect_list_ref 等）
│                 # + pub use 各子模块 register_* 与 lookup_*；vm/mod.rs 引用路径不变
├── io.rs  math.rs  os.rs  string.rs  time.rs  path.rs  json.rs  gc.rs  async.rs
├── list.rs  dict.rs  set.rs   # 内建类型方法（lookup_list/dict/set_method + native_list/dict/set_*）
└── ms/           # 嵌入式 .ms 源码（M1 起）
    ├── collections.ms  itertools.ms  functools.ms  test.ms
```

- 纯移动零行为变更：函数原样迁移，跨文件引用的私有项提升为 `pub(super)`/`pub(crate)`；
  模块私有 helper 留在各模块文件。
- stdlib.rs:2496 起的 `mod tests` 按模块拆入对应文件。
- 验收：`cargo test` 全绿，无语义 diff。

### 3.2 嵌入式 .ms 标准库（M1）

- 源码置于 `src/vm/stdlib/ms/*.ms`，`include_str!` 编入二进制。
- `ModuleResolver` 新增 `embedded_modules: HashMap<String, &'static str>`（名字 → 源码）。
- **解析顺序**：native_modules → 磁盘（当前目录 → stdlib/ → MS_PATH）→ embedded。
  用户当前目录同名 `.ms` 可覆盖嵌入版（便于调试/热修）。
- 缓存键：伪路径 `@embedded/<name>.ms`（PathBuf），与磁盘模块共用 cache 机制。
- 加载复用 `compile_module_source` + `VM::execute_module`（阶段化编排不变）。
- safe_mode：embedded 视同 `@std`，允许导入。
- 验收：单测注册临时嵌入模块，验证 import / 缓存命中 / 磁盘覆盖优先级。
## 4. 模块设计明细

### 4.1 math 扩充（M2）

常量：`tau` `inf` `nan`（Float inline）。

| 函数 | 签名 | 说明 |
|---|---|---|
| asin/acos/atan | (x) -> Float | 域外返回 NaN（与现状 sqrt/log 行为一致，不抛错） |
| atan2 | (y, x) -> Float | |
| sinh/cosh/tanh/asinh/acosh/atanh | (x) -> Float | |
| cbrt | (x) -> Float | 立方根 |
| hypot | (x, y) -> Float | √(x²+y²)，无中间溢出 |
| trunc | (x) -> Int | 向零截断（经 float_to_int 校验） |
| sign | (x) -> Int | -1/0/1；NaN → 0（Go math.Sign 语义） |
| fmod | (x, y) -> Float | C 语义取余（与 `%` 的地板取整区分） |
| modf | (x) -> tuple(Float, Float) | (小数部分, 整数部分) |
| copysign | (x, y) -> Float | 取 x 幅值 + y 符号 |
| degrees/radians | (x) -> Float | 角度/弧度互转 |
| gcd/lcm | (a, b) -> Int | 非负；gcd(0,n)=n、lcm(0,n)=0 |
| factorial | (n) -> Int | 0≤n≤20（21! 溢出 i64 → OverflowError）；负数 ValueError |
| comb/perm | (n, k) -> Int | 组合数/排列数；k>n → 0；参数非法 ValueError |
| isqrt | (n) -> Int | ⌊√n⌋；负数 ValueError |
| is_nan/is_inf | (x) -> Bool | |
| log | (x, base?) -> Float | base 缺省 e；base=1 → ValueError；arity MAX（§2.2） |

### 4.2 string 扩充（M2）

| 函数 | 签名 | 说明 |
|---|---|---|
| count | (s, sub) -> Int | 非重叠出现次数；空 sub → 0；arity MAX（§2.2 与 gc.count 共享名） |
| find | (s, sub) -> Int | 首个字符索引；未找到 -1（与 `s.index()` 抛错语义区分） |
| title | (s) -> string | 每个词首字母大写其余小写（Python 语义） |
| capitalize | (s) -> string | 首字符大写其余小写 |
| pad_start/pad_end | (s, n, pad=" ") -> string | n 为结果**总长**（Python rjust/ljust）；pad 取首字符循环；已长于 n 返回 s 副本；arity MAX |
| center | (s, n, pad=" ") -> string | 居中，左短右长；arity MAX |
| zfill | (s, n) -> string | 左补零至长 n，保留符号位（"-42" → "-0042"） |
| split_lines | (s) -> list | 按行分割去除行尾；`\n`/`\r\n`/`\r` 均识别 |
| trim_start/trim_end | (s) -> string | |
| is_alnum/is_space/is_upper/is_lower | (s) -> Bool | 空串 false；is_upper/is_lower 要求至少一个有大小写字母（Python 语义） |
| cut | (s, sep) -> tuple(s0, s1) | 以第一个 sep 切两段；无 sep → (s, "")（Go strings.Cut 去 found 布尔） |
| fields | (s) -> list | 连续空白分割（Go strings.Fields） |
| join | (sep, list) -> string | 模块级，与 `sep.join(list)` 方法等价；arity MAX |

format 增强（`string.format`；语法实现与 print 无耦合，print 不变）：

- `{}` 顺序替换（现状保留）；`{{` / `}}` 字面转义（**新增**）。
- `{:.Nf}` 定点（N ∈ 0..=9，超出 ValueError）；非法规格（如 `{:x}`、`{:` 未闭合）→
  ValueError 并附原文片段。
- 不支持宽度/对齐/符号等完整 Python format spec（§7 开放问题 6）。

### 4.3 排序增强（M2）

| 入口 | 签名 | 说明 |
|---|---|---|
| sorted | (iterable, key?, reverse?) -> list | 现签名扩展；key 为 1 参函数；reverse 缺省 false；返回新 list；arity MAX |
| sorted_by | (iterable, key, reverse?) -> list | 语义别名（key 显式版） |
| list.sort | (key?, reverse?) | 方法增强 |
| list.sort_by | (key) | 方法别名 |

- 稳定排序（Rust `sort_by`）；key 抛错上抛调用方。
- reverse=True 以**反转比较器**实现（等值元素保持原序，Python 语义）。
- decorate-sort-undecorate：单次 sort 每元素仅调用 key 一次。
### 4.4 random（M3，原生）

实现：`thread_local!` 持 `RefCell<StdRng>`（rand 0.8，可种子）。GC 无关。

| 函数 | 签名 | 说明 |
|---|---|---|
| random | () -> Float | [0,1) 均匀 |
| randint | (a, b) -> Int | 闭区间 [a,b]；a>b → ValueError；非 Int → TypeError |
| uniform | (a, b) -> Float | [a,b)/[b,a)（Python 语义，端点不保证） |
| gauss | (mu, sigma) -> Float | Box–Muller；sigma<0 → ValueError |
| choice | (seq) -> value | list/tuple/string（string 返回单字符 string）；空 → ValueError |
| shuffle | (lst) -> nil | 原地 Fisher–Yates；非 list → TypeError |
| sample | (pop, n) -> list | 不放回；n<0 或 n>len → ValueError；pop 为 list/tuple/string |
| seed | (n?) -> nil | 重置生成器；缺省以系统熵播种 |

### 4.5 encoding（M3，原生，手写零依赖）

| 函数 | 签名 | 说明 |
|---|---|---|
| base64_encode | (s) -> string | RFC 4648 标准字母表 + padding |
| base64_decode | (s) -> string | 非法字符/长度 → ValueError |
| hex_encode | (s) -> string | 字节十六进制小写 |
| hex_decode | (s) -> string | 奇数长度/非 hex → ValueError |
| url_encode | (s, safe="/") -> string | 百分号编码 UTF-8；保留 `A-Za-z0-9-_.~` 与 safe；arity MAX |
| url_decode | (s) -> string | %XX 解码；非法序列 → ValueError；`+` 保持字面（非 form 语义） |

### 4.6 uuid（M3，原生）

| 函数 | 签名 | 说明 |
|---|---|---|
| uuid4 | () -> string | 36 字符小写连字符；版本 4 / variant 位正确；rand 生成 122 位熵 |

### 4.7 fs（M4，原生；错误一律 IOError 前缀）

| 函数 | 签名 | 说明 |
|---|---|---|
| mkdir | (path) | 单级；已存在 → IOError |
| mkdirs | (path) | 递归；幂等（已存在目录成功） |
| rmdir | (path) | 仅空目录 |
| remove | (path) | 删除文件（目录 → IOError） |
| remove_all | (path) | 递归删除；路径不存在返回 nil（幂等，Go RemoveAll） |
| rename | (old, new) | |
| copy | (src, dst) | 文件→文件；dst 存在则覆盖 |
| list_dir | (path) -> list | 子项文件名（不含 `.`/`..`），**排序后返回**（跨平台确定性） |
| walk | (path) -> list | 递归先序全路径（目录+文件）；不跟随符号链接 |
| is_dir/is_file/is_abs | (p) -> Bool | |
| abs | (p) -> string | 绝对化（不解析符号链接、不规范化 `..`） |
| size | (path) -> Int | 字节 |
| mtime | (path) -> Float | Unix 秒 |
| temp_dir | () -> string | 系统临时目录 |
| home_dir | () -> string | env USERPROFILE/HOME；缺失 IOError |

注：read_file/write_file/exists 保留在 io 模块，不在 fs 重复。

### 4.8 os 扩充（M4）

| 函数 | 签名 | 说明 |
|---|---|---|
| getpid | () -> Int | |
| hostname | () -> string | env COMPUTERNAME/HOSTNAME；缺失 → IOError |
| environ | () -> dict | 全量环境变量快照 |
| unsetenv | (key) -> nil | |
| run | (argv) -> dict | `{"status","stdout","stderr"}`；argv 为 string list，**不经 shell**（无注入面）；空列表/非 string 元素 → TypeError；启动失败 → IOError |

os.exec（shell 字符串）保留不动，文档继续警示注入风险；结构化场景引导至 os.run。

### 4.9 sys（M4，原生）

| 函数 | 签名 | 说明 |
|---|---|---|
| platform | () -> string | "windows" / "linux" / "macos" |
| version | () -> string | "mslang 0.1.0"（与 Cargo.toml 同步维护） |
| executable | () -> string | current_exe 绝对路径 |
| stdin_read_all | () -> string | 读 stdin 至 EOF（管道/重定向场景） |
### 4.10 time 扩充（M5）

| 函数 | 签名 | 说明 |
|---|---|---|
| now_ms | () -> Int | Unix 毫秒 |
| monotonic | () -> Float | 单调秒，进程启动为 0 点（OnceLock<Instant> 基线）；用于计时非报时 |
| iso | (ts?) -> string | "YYYY-MM-DDTHH:MM:SSZ"；缺省当前时间；arity MAX |
| date_parts | (ts?) -> dict | `{year,month,day,hour,minute,second,weekday}`，weekday 0=周一（Python）；arity MAX |
| sleep_ms | (ms) | Int 毫秒；负数 → ValueError |
| format_ts | (ts, fmt) -> string | 指令集 `%Y %m %d %H %M %S %%`；UTC |
| parse | (s, fmt) -> Float | 同指令集解析为 Unix 秒；不匹配 → ValueError；arity MAX（§2.2 与 json.parse 共享名） |

闰秒忽略、时区固定 UTC（与现 time.format 一致）。

### 4.11 heapq（M6，原生；最小堆，Python heapq 语义）

| 函数 | 签名 | 说明 |
|---|---|---|
| heapify | (lst) -> nil | 原地建堆 |
| heap_push | (lst, v) -> nil | |
| heap_pop | (lst) -> value | 弹出最小；空 → IndexError |
| push_pop | (lst, v) -> value | push 后立即 pop 最小（合并语义） |
| n_largest/n_smallest | (lst, n) -> list | 前 n 大（降序）/小（升序）；不改原 list；n≤0 → [] |

比较沿用对象 `compare`（同 sorted）；跨类型比较错误上抛。

### 4.12 collections（M6，.ms 嵌入）

三个 class（class 实例而非 dict 子类——dict 为内建类型不可继承；经 `__len__`/`__getitem__`/
`__iter__` 魔术方法接通 `len()`/`[]`/for-in）：

- **deque**：循环缓冲实现（内部 list 容量倍增 + head 偏移），两端均摊 O(1)。
  `push_back/push_front/pop_back/pop_front/front/back/extend(iter)/to_list/is_empty/__len__/__iter__`；
  空弹出 → IndexError。
- **Counter**：构造 `(iterable?)`；`__getitem__` 缺失返回 0（不写入）；`update(other)`；
  `most_common(n?)`（依赖 M2 sorted_by，按频次降序）；`elements()` 生成器；
  `items()/get(k, d=0)`。
- **defaultdict**：构造 `(default_factory)`；`__getitem__` 缺失 → 调 factory() 存入并返回；
  factory 为 nil → KeyError；`get` 不触发 factory（与 Python 一致）。

### 4.13 itertools（M6，.ms 嵌入；全部生成器/惰性）

| 函数 | 说明 |
|---|---|
| count(start=0, step=1) | 无限计数 |
| cycle(iter) | 无限循环（先物化为 list） |
| repeat(x, n?) | 重复 n 次；缺省无限；arity MAX（§2.2 与 string.repeat 共享名） |
| chain(*iters) | 串接 |
| take_while(pred, it) / drop_while(pred, it) | 谓词截断/跳过 |
| pairwise(it) | 相邻对 |
| accumulate(it, fn?) | 前缀累积（缺省 `+`） |
| zip_longest(*iters) | fill=nil 固定（无命名参数） |
| product(*iters) / combinations(it, r) / permutations(it, r?) | 笛卡尔积/组合/排列（输入物化） |
| islice(it, start, stop, step=1) | 切片迭代 |
| batched(it, n) | 按 n 分批 |

### 4.14 functools（M6，.ms 嵌入）

| 函数 | 说明 |
|---|---|
| partial(fn, *args) | 返回 `__call__(*more)` 实例，调用时 args 在前 |
| memoize(fn) | dict 缓存，键 `tuple(args)`；unhashable → TypeError（dict 行为上抛）；**无 LRU 上限**（§7 开放问题 5） |
| reduce(fn, iter, init?) | iterable 级归约（list.reduce 方法保留） |

### 4.15 test（M6，.ms 嵌入）

| 函数 | 说明 |
|---|---|
| assert_eq(a, b, msg?) / assert_ne(a, b, msg?) | 失败抛 AssertionError，消息含 `str(a)`/`str(b)` |
| assert_true(cond, msg?) / assert_false(cond, msg?) | |
| assert_almost_eq(a, b, eps=1e-9, msg?) | 数值；`|a-b| <= eps` |
| assert_raises(fn, exc_class, msg?) | 调 fn()：未抛 → AssertionError；抛出但类不匹配 → AssertionError（匹配机制见 §7 开放问题 2） |
| assert_len(v, n, msg?) / assert_contains(coll, item, msg?) | 复用 len()/contains 语义 |
| fail(msg) | 直接抛 AssertionError |
### 4.16 regex（M7，依赖 regex crate）

函数式（pattern 在前）：

| 函数 | 签名 | 说明 |
|---|---|---|
| match | (pattern, s) -> Match/nil | 锚定开头（Python re.match） |
| search | (pattern, s) -> Match/nil | 首个匹配 |
| findall | (pattern, s) -> list | 0/1 组 → list[string]；多组 → list[tuple] |
| sub | (pattern, repl, s, count=0) -> string | count=0 全替换；repl 为 string 或函数；arity MAX |
| split | (pattern, s) -> list | |
| compile | (pattern) -> Regex | 对象式；方法 `match(s)/search(s)/findall(s)/sub(repl,s,count?)/split(s)/pattern()` |

- 替换串 `${1}` 分组引用（仅索引；无命名组 v1）；repl 为函数时接收 Match 返回 string。
- 非法 pattern → ValueError（附 regex::Error 详情）。
- 新堆类型：`TypeTag::REGEX`（MsRegex{pattern, Box<regex::Regex>}）与
  `TypeTag::MATCH`（MsMatch{text, spans: Vec<Option<(start,end)>>}）；
  均为纯数据（无 Ref 字段），trace noop，正常参与分代 GC。
- Match 方法：`group(i)`（越界 → IndexError）/ `groups()`（tuple，未参组为 nil）/
  `start()/end()/span()`（偏移为字符偏移，与 `s.index` 语义一致）。

### 4.17 hash（M7，依赖 md-5/sha1/sha2）

| 函数 | 签名 | 说明 |
|---|---|---|
| md5/sha1 | (s) -> string | 32/40 位小写 hex；**非安全用途**（文档警示） |
| sha256/sha512 | (s) -> string | 64/128 位小写 hex |

仅 string 输入 v1；文件哈希留待（§7 开放问题 5）。

### 4.18 http（M8，手写 HTTP/1.1，无 TLS）

| 函数 | 签名 | 说明 |
|---|---|---|
| get | (url, headers?, timeout_ms=30000) -> Future<dict> | headers 为 dict |
| post | (url, body, headers?, timeout_ms=30000) -> Future<dict> | body: string；默认 Content-Type `text/plain; charset=utf-8` |
| request | (method, url, body?, headers?, timeout_ms=30000) -> Future<dict> | method 大小写不敏感；arity MAX |

- 响应 dict：`{"status": int, "headers": dict(键小写，同名逗号拼接), "body": string(lossy UTF-8)}`。
- 仅 `http://`；`https://` → Future reject ValueError（TLS 见 §7 开放问题 4）。
- 实现范围：URL 解析（scheme/host/port/path/query，IPv4 字面 host；不含 userinfo）、
  Content-Length 与 **chunked** 传输解码、重定向跟随 ≤5（301/302/303 → GET 且丢 body；
  307/308 保持方法与 body）、默认头 Host / User-Agent: mslang-http/0.1 / Connection: close。
- 超时覆盖连接与单次读（TcpStream::set_read_timeout）；超时 → reject IOError。
- 返回 Future：Pending 分配即返回，可 await 可丢弃（fire-and-forget）；完成机制见 §5。
## 5. 后台线程 + Future 完成架构（http 专用，可复用）

事件循环单线程，阻塞 IO 必须移出；同时后台线程不得触碰 GC 堆。设计：

```
脚本线程（VM/事件循环）                       后台线程
─────────────────────────                   ─────────────────────
http.get(url)
  ├─ alloc Pending Future
  ├─ inflight_futures.push(ptr)   ──GC 根──
  └─ spawn thread(请求参数, Arc<Mutex<Vec<Completion>>>)
                                             执行请求（纯 Rust 数据）
                                             lock queue.push(Completion{
                                                 future: ptr,   // 仅作 tag 传递，绝不解引用
                                                 result: Ok/Err(纯数据),
                                             })
事件循环每轮（timer 处理后）
  └─ drain completions:
       resolve / reject（VM 线程分配 dict/string）
       wake_waiters(ptr)
       inflight_futures.remove(ptr)
```

- `Completion.result`：`Ok(HttpResponseData{status, headers, body})` / `Err(String)`
  （错误消息，VM 线程转 reject）。
- **GC 安全**：`inflight_futures: Vec<*mut MsObjHeader>` 为 VM 级根集扩展（trace 扫描），
  保证 fire-and-forget 期间 Future 不被回收；resolve 后移除。
- **线程安全**：后台线程闭包只携带 `Arc<Mutex<Vec>>` 与请求参数（String）；
  唯一的堆指针是作为完成标记的 future 裸指针，仅 VM 线程解引用。
- **通用性**：机制命名 `external completion`；后续 net、大文件 hash 等阻塞任务可复用。
- **关机**：线程 detached；capi `msVmDestroy` 时未完成结果随队列 Arc 丢弃
  （风险验证见 §7 开放问题 3）。

## 6. 实施里程碑与验收

| # | task 文档 | 内容 | 涉及章节 | 规模估计 |
|---|---|---|---|---|
| M0 | 78-stdlib-split.md | stdlib.rs 拆分为目录 + 测试迁移 | §3.1 | ~5000 行移动 |
| M1 | 79-embedded-ms.md | 嵌入式 .ms 机制（注册/优先级/缓存/单测） | §3.2 | ~150 行 |
| M2 | 80-stdlib-math-string-sort.md | math/string 扩充 + sorted key | §4.1-4.3 | ~700 行 |
| M3 | 81-stdlib-random-encoding-uuid.md | random/encoding/uuid | §4.4-4.6 | ~450 行 |
| M4 | 82-stdlib-fs-os-sys.md | fs / os 扩充 / sys | §4.7-4.9 | ~500 行 |
| M5 | 83-stdlib-time.md | time 扩充 | §4.10 | ~250 行 |
| M6 | 84-stdlib-collections-itertools-functools-test.md | heapq + 4 个 .ms 模块 | §4.11-4.15 | ~800 行 |
| M7 | 85-stdlib-regex-hash.md | regex / hash + 新 TypeTag | §4.16-4.17 | ~500 行 |
| M8 | 86-stdlib-http.md | http + external completion | §4.18, §5 | ~600 行 |

- 每里程碑验收 = §2.4 全套 + `cargo test` 全量回归。
- M7 额外验证新 TypeTag 注册不影响 GC trace 表；M8 需并发场景用例（超时/重定向/chunked），
  网络成功路径由 Rust 单测内置 `TcpListener` 固定响应服务器覆盖，ms 语料仅覆盖错误路径
  （https 拒绝/非法 URL），避免测试依赖外网。
- 顺序依赖：M1、M2 先于 M6（Counter.most_common 用 sorted_by、.ms 模块需嵌入机制）；
  其余可并行。
## 7. 风险与开放问题

1. **native_arities 同名冲突**：count/parse/repeat/log/sorted 升级 MAX 后靠自校验兜底；
   实现期补「同名交叉调用」回归用例（gc.count × string.count、json.parse × time.parse 等）。
2. **assert_raises 类匹配**：`type(e)` 对异常实例的返回值待实现期验证；必要时暴露
   `__class__` 或增强 type()，在 M6 内闭环并回写本文档。
3. **http detached 线程生命周期**：解释器销毁后入队结果被丢弃；若 capi 场景出现
   future 指针悬垂风险，以「销毁前 join in-flight 线程」兜底，M8 内验证。
4. **TLS**：https 不支持；未来引入 rustls（新依赖决策，另行确认）。
5. **后续增强留白**（均不在本次范围）：hash 文件输入、regex 命名组、memoize LRU 上限、
   deque 原生 TypeTag（当前 .ms 实现常数较大）、完整 Python format spec、net 模块。
6. **format 精度集极小**（仅 `{:.Nf}`）；复杂格式先用字符串拼接过渡。
7. **GC**：MsRegex/MsMatch trace noop（无 Ref 字段）；http inflight 根集正确性依赖
   「resolve 后即移除」不变量，M8 单测覆盖（fire-and-forget + 强制 gc.collect）。
8. **磁盘 stdlib 目录**：嵌入机制落地后发行不再依赖 `stdlib/` 目录；MS_STDLIB 语义保留。

## 8. 既有文档修订点

| 文档 | 修订 |
|---|---|
| 10-builtins.md | 新增 random/encoding/uuid/fs/sys/heapq/collections/itertools/functools/test/regex/hash/http 章节；math/string/time/os 章节扩表；「未文档化的标准库模块」表移除已兑现项（net 保留） |
| 12-implementation-plan.md | 项目结构树更新（src/vm/stdlib/ 目录 + ms/ 嵌入源码）；Phase 6 补记 |
| 09-modules.md | 补嵌入模块解析顺序（native → 磁盘 → embedded）与 `@embedded/` 缓存键说明 |
| tasks/README.md | Phase 6 追加 task 78-86 索引（实现时逐个建行） |
| Cargo.toml | +regex +md-5 +sha1 +sha2（M7 时） |

---

*确认方式：按节回复（如「§4.1 确认；§4.4 randint 改为左闭右开」）。全部确认后按 §6 拆分
task 文档（78-86）并开始实现；实现期若推翻本文档某项决策，回写对应章节并注明修订原因。*
