# 标准库 - regex / hash 模块

## 所属阶段
Phase 9 - 标准库扩展（M7）

## 前置任务
78-stdlib-split
80-stdlib-math-string-sort（sub repl 函数回调复用其同步调用入口）

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
| split | (pattern, s) -> list | arity MAX（§2.2 与 `s.split` 同名，自校验恰好 2 参） |
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
- sub 的 count：缺省 0 = 全替换；**count < 0 → ValueError**（"count must be
  non-negative"）；count 非 Int → TypeError。
- Match.group(i)：越界 → IndexError；**负索引 v1 按越界处理 → IndexError**
  （不支持 Python 的负索引回绕，10-builtins.md 注明差异）。
- repl 展开错误（`${99}` 越界、`${1x` / `${` 畸形格式）→ ValueError，
  **消息附 repl 原文片段**（对齐 string.format 的错误风格）。
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

### 前置扩展（词法/解析）：保留字作属性名

> **审核发现（A1）**：`match` 为保留字（[01-lexical](../01-lexical.md) §保留字），
> 现行词法器对任何位置出现的 `match` 直接 LexError（`src/lexer/mod.rs` read_identifier），
> `regex.match(...)` 与 `re.match(s)` 无法通过词法分析。本 task 须先落地以下变更
> （决策：保留 API 名 `match`，与 Python 对齐；不采用改名方案）：

- 词法器：保留字（`export` / `match`）不再报 LexError，改为产出
  `TokenKind::Reserved(String)`。
- 解析器：`.` 后的属性名接受 `Identifier` 与 `Reserved` 两种 token；其余
  绑定/初等表达式位置（变量声明、赋值目标、参数名、函数名、class 名、
  import 绑定、for-in 变量）遇到 `Reserved` 报 ParseError，消息沿用
  "'match' is a reserved word and cannot be used as identifier"。
- 影响面：`src/lexer/mod.rs`、`src/lexer/token.rs`、`src/parser/`（属性名位置
  与各绑定校验点）、`test_all_reserved_words_error` 断言迁移
  （LexError → ParseError）。
- 语义不变量：`match = 1` 仍报错（错误种类由词法变解析，消息与行号语义保持）；
  属性位置 `regex.match` 解析通过。
- 文档回写（本 task 交付物）：`01-lexical.md` §保留字「词法处理」段——
  保留字改为产出 Reserved token、由解析器在绑定位置拒绝、成员访问位置放行。

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
  findall=2, sub=MAX, **split=MAX**, compile=1；hash 侧各 1）。
  > **同名冲突（§2.2，审核 A2）**：string 方法 `s.split(sep?)`（1-2 参）经
  > GET_ATTR 创建名为 "split" 的 native，CALL 按名查全局 native_arities；
  > 注册 split=2 会使 `s.split()`（空白分割，argc=1）抛 TypeError。
  > 故 split 必须注册 **MAX**，regex.split 自校验恰好 2 参（TypeError），
  > 并回写 16-stdlib-expansion.md §2.2 冲突表补 split 行。
  > match/search/findall/sub/compile 经查无既有同名注册，无冲突。
- `src/vm/gc.rs` / 各 GC 子模块 — 新 TypeTag 的 trace noop 注册与 sweep 大小
  （若按 tag 分派则需要登记；实现期以既有 tag 接入点清单为准逐一核对）
- `docs/mslang/10-builtins.md` — 新增 regex / hash 章节（API 表）；「未文档化的
  标准库模块」表移除 regex 行；注明 regex crate Rust 方言差异（Unicode 类超集、
  Perl 反向引用支持子集，见上文语义细则）
- `docs/mslang/14-gc.md` — TypeTag 权威枚举回写：REGEX = 23、MATCH = 24
- `docs/mslang/01-lexical.md` — 保留字词法处理规则回写（见前置扩展）
- `docs/mslang/tasks/README.md` — task 85 状态勾选

### 字节偏移 ↔ 字符偏移

regex crate 的 span 为**字节**偏移；Match 方法语义为**字符**偏移
（与 `s.index` 一致）。转换：缓存 `text` 的字节→字符映射
（构建时一次 `char_indices` 前缀和，`Vec<usize>` 长度 = 字节数 + 1），
spans 存储时预转换为字符偏移（Match 构造期完成，查询 O(1)）。
**字节 spans 同时保留**（MsMatch 增设镜像字段或双向映射）：
`group(i)` / findall / sub 提取子串按字节偏移 O(1) 切片，
避免每次调用对 text 做字符→字节线性回转。

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
  晋升、回收）——与 MsStr 同类（纯数据）。MsRegex 深拷贝依赖
  `regex::Regex: Clone`（1.13 已实现，内部 Arc 共享、廉价；若版本不含 Clone
  则改存 `Arc<regex::Regex>`）。
- REPL/函数回调期间的 Match 对象由调用方 native 栈持有（VM 根集含调用栈），
  与 map/filter 回调同模型。
- 新 TypeTag 接入点清单（实现期逐项核对）：
  `alloc` 尺寸计算、trace 表、**copy_for_gc（TypeDescriptor 字段，Young 半空间
  复制/晋升路径深拷贝载荷）**、sweep/free 路径（drop Rust 所有的
  String/Vec/Box 字段）、GET_ATTR 方法分派、`type()` 名称（"regex"/"match"）、
  object_to_string（regex → `/<pattern>/`；match → 类似 Python `<match>` 简化文本）。

## 验证标准

1. `regex.search("l+", "hello")` 的 group(0)=="ll"、start()==2、end()==4、span()==(2,4)
2. `regex.match("h", "hello")` 命中；`regex.match("e", "hello")` → nil
3. findall 单组/多组/零组三态正确（含中文输入的字符偏移正确性）
4. sub("${1}-${2}", ...) 分组展开；count 参数截断；repl 函数回调返回替换串
5. sub repl 函数返回非 string → TypeError；`${99}` 越界 → ValueError（附原文）
6. split 未匹配返回 [s]；多分隔正确；**同名交叉回归**：`s.split()`（空白）、
   `s.split(",")`、`regex.split(",", s)` 三者并存正确（§2.2 split=MAX）
7. compile 对象与函数式结果等价；pattern() 回读
8. 非法 pattern（如 `(`）→ ValueError 含 regex::Error 信息
9. hash 四函数输出长度/大小写/已知向量（md5("")=d41d8cd98f00b204e9800998ecf8427e、
   sha256("abc")=ba7816bf... 等）正确
10. sub count=-1 → ValueError；group(-1) → IndexError（负索引按越界）
11. 前置扩展：`regex.match("h", "hello")` 可解析执行；`match = 1` 仍报错
    （保留字绑定拒绝）
12. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_regex.ms

验证标准 1-8、10（assert + ALL PASSED）。

### tests/ms 负面语料（保留字）

验证标准 11 错误侧：`match = 1` → 编译错误（配 `.expected` 的 stderr 子串）。

### tests/ms/stdlib/test_hash.ms

验证标准 9（空串与 "abc" 标准向量断言）。

### Rust 单测（regex.rs / hash.rs 内）

- 字节↔字符偏移转换（中文/emoji 混合样本）
- repl 状态机边界（`$` 结尾字面、`{0}` 整体引用）
