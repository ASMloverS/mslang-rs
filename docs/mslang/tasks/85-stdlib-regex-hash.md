# 标准库 - regex / hash 模块

## 所属阶段
Phase 9 - 标准库扩展（M7）

## 前置任务
78-stdlib-split

> **依赖说明**：本 task 引入 4 个新 crate（regex / md-5 / sha1 / sha2，
> 见 16-stdlib-expansion.md §2.5）；regex 模块新增两个堆类型
> TypeTag::REGEX 与 TypeTag::MATCH。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.16-4.17。

## 目标

1. `regex` 模块：函数式 + `compile()` 对象式双入口，Match 对象带分组。
2. `hash` 模块：md5 / sha1 / sha256 / sha512（string → 小写 hex）。

## 设计规格

### regex

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.16：

**函数式**（pattern 在前）：

| 函数 | 签名 | 说明 |
|---|---|---|
| match | (pattern, s) -> Match/nil | 锚定开头（Python re.match） |
| search | (pattern, s) -> Match/nil | 首个匹配 |
| findall | (pattern, s) -> list | 0/1 组 → list[string]；多组 → list[tuple] |
| sub | (pattern, repl, s, count=0) -> string | count=0 全替换；repl 为 string 或函数；arity MAX |
| split | (pattern, s) -> list | |
| compile | (pattern) -> Regex | 对象式入口 |

**Regex 对象方法**：`match(s)` / `search(s)` / `findall(s)` /
`sub(repl, s, count?)` / `split(s)` / `pattern()`。

**Match 对象方法**：

| 方法 | 说明 |
|---|---|
| group(i) | 第 i 组（0 = 整体）；越界 → IndexError |
| groups() | tuple；未参与匹配的组为 nil |
| start() / end() / span() | 字符偏移（与 `s.index` 语义一致），span() → tuple(start, end) |

**语义细则**：

- 替换串 `${1}` 分组引用（仅索引，无命名组 v1）；repl 为函数时接收 Match 返回 string。
- 非法 pattern → ValueError（附 regex::Error 详情）。
- findall 空/未匹配返回空 list；split 未匹配返回 `[s]`。
- regex crate 语法为 Rust 方言（Unicode 类等超集）；mslang 直接透传 pattern，
  差异在 10-builtins.md 注明（如 Perl 反向引用支持子集）。

### hash

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.17：

| 函数 | 签名 | 输出 |
|---|---|---|
| md5(s) | -> string | 32 位小写 hex，**非安全用途**（文档警示） |
| sha1(s) | -> string | 40 位小写 hex，同上警示 |
| sha256(s) | -> string | 64 位小写 hex |
| sha512(s) | -> string | 128 位小写 hex |

仅 string 输入（UTF-8 字节）；文件哈希留白（开放问题 5）。

## 实现细节

### 文件位置

- `Cargo.toml` — `+regex = "1"` `+md-5 = "0.10"` `+sha1 = "0.10"` `+sha2 = "0.10"`
- `src/vm/object.rs` — TypeTag::REGEX / MATCH 枚举值（沿用递增：23 / 24）；
  `MsRegex { pattern: String, compiled: Box<regex::Regex> }`、
  `MsMatch { text: String, spans: Vec<Option<(usize, usize)>> }`（spans[0] 为整体，
  字符偏移）、`alloc_regex` / `alloc_match`、trace 函数 noop（无 Ref 字段）、
  方法表 lookup（`lookup_regex_method` / `lookup_match_method`，参照
  `lookup_string_method` 的 GET_ATTR 模式）
- `src/vm/stdlib/regex.rs` — `register_regex_module` + 6 个 native 函数 +
  对象方法实现 + repl 展开（`${N}` 与函数回调）
- `src/vm/stdlib/hash.rs` — `register_hash_module` + 4 个 native 函数
- `src/vm/mod.rs` — 注册两模块 + `native_arities`（regex 侧 match=2, search=2,
  findall=2, sub=MAX, split=2, compile=1；hash 侧各 1）
- `src/vm/gc.rs` / 各 GC 子模块 — 新 TypeTag 的 trace noop 注册与 sweep 大小
  （若按 tag 分派则需要登记；实现期以既有 tag 接入点清单为准逐一核对）

### 字节偏移 ↔ 字符偏移

regex crate 的 span 为**字节**偏移；Match 方法语义为**字符**偏移
（与 `s.index` 一致）。转换：缓存 `text` 的字节→字符映射
（构建时一次 `char_indices` 前缀和，`Vec<usize>` 长度 = 字节数 + 1），
spans 存储时预转换为字符偏移（Match 构造期完成，查询 O(1)）。

### repl 展开状态机

- `${N}`：`{` 后读数字至 `}`；越界组引用 / 非法格式 → ValueError。
- repl 为 FUNCTION/CLOSURE 堆对象：每处匹配调用（复用 task 80 key 调用的
  同步调用入口），返回值须为 string（否则 TypeError）。
- repl 为 string：`${N}` 展开 + 字面透传。

### findall 组策略

- pattern 组数 = 0：返回全部整体匹配 string。
- 组数 = 1：返回该组内容 string。
- 组数 ≥ 2：每组匹配返回 tuple（未参组 nil）。

### sha/md 实现骨架

```rust
use md5::Md5; use sha1::Sha1; use sha2::{Sha256, Sha512};
use md5::Digest as _;   // trait 名冲突处理
// hash.md5: Md5::digest(s.as_bytes()) → hex 小写
```

> md-5/sha1/sha2 0.10 系 `digest` 方法来自 `digest::Digest` trait，
> 三个 crate 共存时用 `as _` 别名或全限定调用避免 trait 冲突。

## GC 安全

- MsRegex / MsMatch 无 Ref 字段，trace noop；参与正常分代 GC（Young 分配、
  晋升、回收）——与 MsStr 同类（纯数据）。
- REPL/函数回调期间的 Match 对象由调用方 native 栈持有（VM 根集含调用栈），
  与 map/filter 回调同模型。
- 新 TypeTag 接入点清单（实现期逐项核对）：
  `alloc` 尺寸计算、trace 表、sweep/free 路径、
  GET_ATTR 方法分派、`type()` 名称（"regex"/"match"）、object_to_string
  （regex → `/<pattern>/`；match → 类似 Python `<match>` 简化文本）。

## 验证标准

1. `regex.search("l+", "hello")` 的 group(0)=="ll"、start()==2、end()==4、span()==(2,4)
2. `regex.match("h", "hello")` 命中；`regex.match("e", "hello")` → nil
3. findall 单组/多组/零组三态正确（含中文输入的字符偏移正确性）
4. sub("${1}-${2}", ...) 分组展开；count 参数截断；repl 函数回调返回替换串
5. sub repl 函数返回非 string → TypeError；`${99}` 越界 → ValueError
6. split 未匹配返回 [s]；多分隔正确
7. compile 对象与函数式结果等价；pattern() 回读
8. 非法 pattern（如 `(`）→ ValueError 含 regex::Error 信息
9. hash 四函数输出长度/大小写/已知向量（md5("")=d41d8cd98f00b204e9800998ecf8427e、
   sha256("abc")=ba7816bf... 等）正确
10. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_regex.ms

验证标准 1-8（assert + ALL PASSED）。

### tests/ms/stdlib/test_hash.ms

验证标准 9（空串与 "abc" 标准向量断言）。

### Rust 单测（regex.rs / hash.rs 内）

- 字节↔字符偏移转换（中文/emoji 混合样本）
- repl 状态机边界（`$` 结尾字面、`{0}` 整体引用）
