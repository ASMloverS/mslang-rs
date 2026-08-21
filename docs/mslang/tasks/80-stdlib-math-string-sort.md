# 标准库 - math/string 扩充与排序增强

## 所属阶段
Phase 9 - 标准库扩展（M2）

## 前置任务
78-stdlib-split

> **依赖说明**：在拆分后的 `src/vm/stdlib/math.rs` / `string.rs` 上扩充；
> 排序增强涉及 `src/vm/builtins.rs` 的 `builtin_sorted` 与 task 51 的 list 方法表。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.1-4.3。

## 目标

1. math 模块扩充 3 常量 + 约 25 个函数（对齐 Go math / Python math 常用集）。
2. string 模块扩充约 17 个函数 + `format` 支持 `{:.Nf}` 精度与 `{{`/`}}` 转义。
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
  factorial 范围 0-20，21! 溢出 → OverflowError）
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
- `{:.Nf}` 定点（N ∈ 0..=9；超出/非法规格 → ValueError 附原文片段）
- 不实现宽度/对齐/符号（开放问题 6 留白）

### 排序增强

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.3：

| 入口 | 签名 | arity |
|---|---|---|
| sorted | (iterable, key?, reverse?) -> list | MAX，自校验 1-3 参 |
| sorted_by | (iterable, key, reverse?) -> list | MAX |
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
  `log → MAX`、`count → MAX`、`sorted → MAX`、`sorted_by → MAX`、
  `sort_by → 1`、`sort → MAX`（若 list.sort 现登记为 0）、新函数逐个登记
- `src/vm/stdlib/mod.rs` — helper 无新增（`float_to_int` 复用）

### key 调用机制（native 内调用脚本函数）

参照 `builtin_map` / `builtin_filter` 既有模式：native 函数内经 `vm.call_value`
（或既有等价入口）同步调用 key 函数取回返回值；key 非 function → TypeError。

> **注意**：同步调用期间可能触发 GC / 深层调用；实现须确认所用调用入口
> 与 map/filter 的嵌套调用路径一致（复用其已验证的模式）。

### format 解析状态机

单遍字符扫描：`{` 后 peek `}` → 占位；peek `:` → 进入格式段，读至 `}`，
段内须为 `.` + 1 位数字（0-9）；其余任何字符（含 `{` 嵌套、`:` 后非法、未闭合）
→ ValueError 附片段。`}` 单独出现按字面输出（与 Python 容忍度一致）。

## GC 安全

- 全部新函数返回值经既有 `alloc_*` 分配，无新根集。
- DSU 排序的中间 `Vec<(Object, Object)>` 在 native 栈上，函数返回即弃，
  无跨调用存活（与 builtin_sorted 现状一致）。

## 验证标准

1. math 新函数值域抽查（asin(1)=π/2、hypot(3,4)=5、gcd(12,18)=6、factorial(5)=120、
   isqrt(17)=4、log(8,2)=3.0、log(100)=log10(100)）
2. factorial(21) → OverflowError；factorial(-1)/isqrt(-1)/gcd 负数 → ValueError
3. log(8, 1) → ValueError；log("x") → TypeError
4. string 新函数抽查（count("aaa","a")=3、find 未找到 -1、zfill 符号保留、
   cut/fields 语义、split_lines 三种行尾）
5. format：`{:.2f}` 于 3.14159 → "3.14"；`{{}}` → "{}"；`{:x}` → ValueError；
   `{:.10f}` → ValueError
6. sorted([3,1,2]) == [1,2,3]（无 key 兼容旧用例）；sorted(words, fn(w){ len(w) })
   稳定；sorted(..., reverse=true) 等值保序；key 抛错上抛
7. sorted_by 与 sorted(iter, key) 等价；list.sort/sort_by 原地生效
8. 同名冲突回归：`gc.count()` 与 `string.count("aa","a")` 同脚本并存正确
9. `cargo test` 全绿（含 ms_corpus 全量）

## 测试用例

### tests/ms/stdlib/test_math_ext.ms

覆盖验证标准 1-3（assert + "ALL PASSED"）。

### tests/ms/stdlib/test_string_ext.ms

覆盖验证标准 4-5。

### tests/ms/stdlib/test_sort_key.ms

覆盖验证标准 6-7；含稳定性用例（等 key 元素保原序）与 key 异常上抛用例
（try/except 捕获 key 内抛出的 ValueError）。

### tests/ms/stdlib/test_native_arity_conflicts.ms

覆盖验证标准 8（gc.count × string.count 并存）。
