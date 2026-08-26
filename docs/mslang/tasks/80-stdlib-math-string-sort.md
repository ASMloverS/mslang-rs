# 标准库 - math/string 扩充与排序增强

## 所属阶段
Phase 9 - 标准库扩展（M2）

## 前置任务
78-stdlib-split

> **依赖说明**：在拆分后的 `src/vm/stdlib/math.rs` / `string.rs` 上扩充；
> 排序增强涉及 `src/vm/builtins.rs` 的 `builtin_sorted` 与 task 51 的 list 方法表。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.1-4.3。

## 目标

1. math 模块扩充 3 常量 + 28 个函数（对齐 Go math / Python math 常用集）。
2. string 模块扩充 18 个函数 + `format` 支持 `{:.Nf}` 精度与 `{{`/`}}` 转义。
3. `sorted(iterable, key?, reverse?)` 扩展 + `sorted_by` + `list.sort(key?, reverse?)`
   + `list.sort_by(key)`。

## 设计规格

### math 扩充

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.1：

- 常量：`tau` `inf` `nan`（Float inline，同 pi/e 模式）
- 反三角/双曲：asin/acos/atan/atan2/sinh/cosh/tanh/asinh/acosh/atanh
  （域外返回 NaN，与现状 sqrt/log 一致，不抛错）
- 数值：cbrt/hypot/trunc/sign/fmod/modf/copysign
- 角度：degrees/radians
- 整数：gcd/lcm/factorial/comb/perm/isqrt（参数非法 → ValueError；
  factorial 范围 0-20，21! 溢出 → OverflowError；gcd/lcm/comb/perm 全程
  checked 运算，中间值或结果溢出 i64 → OverflowError；gcd/lcm 负数取
  绝对值（Python math.gcd/lcm 语义，§2.3 Python 对齐））
- 谓词：is_nan/is_inf
- **log 升级**：`log(x, base?)`（base 缺省 e；base=1 → ValueError；
  arity 1 → MAX，自校验 1-2 参，见 §2.2 同名冲突治理）

### string 扩充

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.2：

count/find/title/capitalize/pad_start/pad_end/center/zfill/split_lines/trim_start/
trim_end/is_alnum/is_space/is_upper/is_lower/cut/fields/join（模块级）。

- `count` arity MAX（与 gc.count=0 共享名，各自自校验）
- `find` 未找到返回 -1（与 `s.index()` 抛 ValueError 区分）
- `pad_start/pad_end/center` 的 n 为结果总长（Python rjust/ljust 语义），arity MAX
- `cut` 返回 tuple(s0, s1)；无 sep → (s, "")（Go strings.Cut 去 found 布尔）
- `fields` 按连续空白分割（Go strings.Fields）
- `join(sep, list)` 与 `sep.join(list)` 方法等价，arity MAX（与 path.join 共享名）

### format 增强（`string.format`）

- `{}` 顺序替换（现状保留）；`{{` / `}}` 输出字面花括号（新增）
- `{:.Nf}` 定点（N ∈ 0..=9；超出/非法规格 → ValueError 附原文片段；
  接受 Float 与 Int —— Int 按 Float 格式化（`{:.2f}` 于 3 → "3.00"），
  其余类型 → TypeError）
- 不实现宽度/对齐/符号（开放问题 6 留白）

### 排序增强

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.3：

| 入口 | 签名 | arity |
|---|---|---|
| sorted | (iterable, key?, reverse?) -> list | MAX，自校验 1-3 参 |
| sorted_by | (iterable, key, reverse?) -> list | MAX，自校验 2-3 参 |
| list.sort | (key?, reverse?) | MAX |
| list.sort_by | (key) | 1 |

- 稳定排序（Rust `sort_by`）；key 抛错上抛调用方
- reverse=true 反转比较器实现（等值元素保持原序，Python 语义）
- decorate-sort-undecorate：单次 sort 每元素仅调用 key 一次

## 实现细节

### 文件位置

- `src/vm/stdlib/math.rs` — 常量 + 新函数 + `native_math_log` 重写（自校验）
- `src/vm/stdlib/string.rs` — 新函数 + `native_string_format` 扩展
- `src/vm/builtins.rs` — `builtin_sorted` 重写（key/reverse 可选参 + DSU）；
  新增 `builtin_sorted_by`
- `src/vm/mod.rs` — `native_arities` 更新：
  `log → MAX`、`count → MAX`、`sorted → MAX`（覆盖 builtins 表登记的 1）、
  `sorted_by → MAX`、新函数逐个登记
- `src/vm/stdlib/list.rs` — `native_list_sort` 扩展（key/reverse）+ 新增
  `native_list_sort_by`。**方法调用经 BoundMethod→FUNCTION 路径（mod.rs
  call_value）不查 `native_arities`**，故 `list.sort(key?, reverse?)` /
  `list.sort_by(key)` 须在 native 内自校验用户参数个数（0-2 / 1，不含
  receiver），违规 → TypeError；sort/sort_by 现未亦无需登记 native_arities
- `src/vm/stdlib/mod.rs` — helper 无新增（`float_to_int` 复用）

### key 调用机制（native 内调用脚本函数）

参照 `builtin_map` / `builtin_filter` 既有模式：native 函数内经 `vm.call_value`
（或既有等价入口，如 `vm.call_function`）同步调用 key 函数取回返回值；
key 非 function → TypeError。

> **注意**：同步调用期间可能触发 GC / 深层调用（见下文「GC 安全」的根化
> 要求；map/filter 的调用入口可复用，但其**未根化的结果 Vec 写法不可复刻**）。

### format 解析状态机

单遍字符扫描：`{` 后 peek `}` → 占位；peek `{` → 输出字面 `{`（消费两个
字符）；peek `:` → 进入格式段，读至 `}`，段内须为 `.` + 1 位数字（0-9）；
`{` 后其余任何字符（含 `{` 嵌套、`:` 后非法、未闭合）→ ValueError 附片段。
`}` 后 peek `}` → 输出字面 `}`（消费两个字符）；`}` 单独出现 → ValueError
（Python 对齐：Single '}' encountered）。

## GC 安全

- 全部新函数返回值经既有 `alloc_*` 分配，无新根集。
- **DSU / key 回调的根化（关键）**：native 入口实参已被弹出 vm.stack
  （call_value FUNCTION 分支），key 调用经 `call_function` 再入解释器循环时，
  每条字节码前均执行 `maybe_gc` —— native 栈上的 Vec **不在 GC 根集**，
  直接持有 Object::Ref 会在 key 分配触发 GC（对象移动/回收）时悬垂。实现须：
  1. native 入口将源 iterable 与 key 压入 vm.stack 作临时根，返回前 pop；
  2. DSU 中间对放入 heap list 并将其 Ref 压入 vm.stack 根化（list 被 trace，
     元素随之存活），物化结果后 pop；
  3. 现状 builtin_sorted 比较器不重入 VM，无此问题（不可类比）；builtin_map/
     filter 与 list.map/filter/reduce 的结果 Vec 存在同构隐患，本 task 不修复，
     仅禁止新代码复刻该未根化写法。

## 验证标准

1. math 新函数值域抽查（asin(1)=π/2、hypot(3,4)=5、gcd(12,18)=6、
   gcd(-12,18)=6、factorial(5)=120、isqrt(17)=4、log(8,2)=3.0、
   log(100,10)=log10(100)=2.0）
2. factorial(21) → OverflowError；factorial(-1)/isqrt(-1) → ValueError；
   comb(100,50) → OverflowError（i64 checked 溢出）
3. log(8, 1) → ValueError；log("x") → TypeError
4. string 新函数抽查（count("aaa","a")=3、find 未找到 -1、zfill 符号保留、
   cut/fields 语义、split_lines 三种行尾）
5. format：`{:.2f}` 于 3.14159 → "3.14"；`{:.2f}` 于 3 → "3.00"（Int 按
   Float）；`{:.2f}` 于 "x" → TypeError；`{{}}` → "{}"；单独 `}` →
   ValueError；`{:x}` → ValueError；`{:.10f}` → ValueError
6. sorted([3,1,2]) == [1,2,3]（无 key 兼容旧用例）；sorted(words, fn(w){ len(w) })
   稳定；sorted(..., reverse=true) 等值保序；key 抛错上抛
7. sorted_by 与 sorted(iter, key) 等价；list.sort/sort_by 原地生效
8. 同名冲突回归：`gc.count()` 与 `string.count("aa","a")` 同脚本并存正确
9. GC 压力：sorted 大 list + key 内逐元素分配新对象（触发 Minor GC）+
   `gc.collect()` 后逐项校验，无崩溃/错值（根化方案有效性）
10. `cargo test` 全绿（含 ms_corpus 全量）

## 测试用例

### tests/ms/stdlib/test_math_ext.ms

覆盖验证标准 1-3（assert + "ALL PASSED"）。

### tests/ms/stdlib/test_string_ext.ms

覆盖验证标准 4-5。

### tests/ms/stdlib/test_sort_key.ms

覆盖验证标准 6-7、9；含稳定性用例（等 key 元素保原序）、key 异常上抛用例
（try/except 捕获 key 内抛出的 ValueError）与 GC 压力用例
（key 内分配 + gc.collect 后逐项校验）。

### tests/ms/stdlib/test_native_arity_conflicts.ms

覆盖验证标准 8（gc.count × string.count 并存）。

### Rust 单元测试（§2.4.1）

- `src/vm/stdlib/math.rs`：新函数值域、错误路径（OverflowError / ValueError /
  TypeError）
- `src/vm/stdlib/string.rs`：format 状态机全分支（`{{`/`}}`/`{:.Nf}`/非法
  规格/单独 `}`）、新函数边界（空串、空 sub、n 短于串长）
- `src/vm/builtins.rs` / `src/vm/stdlib/list.rs`：sorted/sorted_by/sort/
  sort_by 参数个数自校验、DSU 稳定性与 reverse 等值保序

## 文档更新

- `docs/mslang/10-builtins.md`：math 章节扩表（3 常量 + 28 函数）、string
  章节扩表（18 函数 + format 增强）、`sorted` 签名改为
  `(iterable, key?, reverse?)` 并补 `sorted_by`、list 方法表补
  `sort(key?, reverse?)` 与 `sort_by(key)`
- `docs/mslang/tasks/README.md`：task 80 状态标记 ✅
