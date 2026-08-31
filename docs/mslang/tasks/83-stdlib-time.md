# 标准库 - time 模块扩充

## 所属阶段
Phase 9 - 标准库扩展（M5）

## 前置任务
78-stdlib-split

> **依赖说明**：在拆分后的 `src/vm/stdlib/time.rs` 上扩充，复用既有
> `unix_to_ymdhms` 历法算法；格式化/解析指令集为手写，零新依赖（不引 chrono）。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.10。

## 目标

time 模块扩充 7 个函数：now_ms / monotonic / iso / date_parts / sleep_ms /
format_ts / parse。

## 设计规格

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.10
（时间一律 UTC；秒为 Float、毫秒为 Int，与现状一致）：

| 函数 | 签名 | 说明 |
|---|---|---|
| now_ms | () -> Int | Unix 毫秒 |
| monotonic | () -> Float | 单调秒，进程启动为 0 点；用于计时非报时 |
| iso | (ts?) -> string | "YYYY-MM-DDTHH:MM:SSZ"；缺省当前时间；arity MAX |
| date_parts | (ts?) -> dict | `{year,month,day,hour,minute,second,weekday}`，weekday 0=周一（Python）；arity MAX |
| sleep_ms | (ms) | Int 毫秒；负数 → ValueError |
| format_ts | (ts, fmt) -> string | 指令集 `%Y %m %d %H %M %S %%`；UTC |
| parse | (s, fmt) -> Float | 同指令集解析为 Unix 秒；不匹配 → ValueError；arity MAX（与 json.parse 共享名） |

闰秒忽略。`time.format(ts)`（既有，无 fmt）保留不动；`format_ts` 为带格式版。

## 实现细节

### 文件位置

- `src/vm/stdlib/time.rs` — 7 个新函数入 `register_time_module` exports
- `src/vm/mod.rs` — `native_arities` 登记：`now_ms → 0`、`monotonic → 0`、
  `iso → MAX`、`date_parts → MAX`、`sleep_ms → 1`、`format_ts → 2`、
  `parse → MAX`（自校验 2 参，json.parse=1 升级为 MAX 同步处理——见 §同名冲突）

> **注意 parse arity 迁移**：`native_arities["parse"]` 现为 1（json.parse）。
  本 task 将其升级为 MAX 后，`native_json_parse` 必须同步补自校验（恰好 1 参，
  否则 TypeError）——升级动作归并到本 task 一次性完成，并加交叉回归用例
  （json.parse 单参与 time.parse 双参同脚本并存）。

### monotonic 基线

```rust
static MONO_BASE: OnceLock<Instant> = OnceLock::new();
monotonic() = Instant::now().duration_since(*MONO_BASE.get_or_init(Instant::now)).as_secs_f64()
```

### ts 参数校验（iso / date_parts / format_ts）

- ts 接受 Int 与 Float（与既有 time.format 一致）。
- Float ts 禁止 `as u64` 静默饱和（§2.3）：NaN/±Inf → ValueError、
  超出可表示范围 → OverflowError（经 `float_to_int` 语义校验），再截断取整秒。
- ts < 0 → ValueError（沿用 time.format 既有校验）。

### format_ts / parse 共享指令扫描

- 格式串逐字符：`%` + 指令字符为一段；其余为字面字符。
- 指令集外指令字符（如 `%q`）与孤立 `%` → ValueError（format_ts 与 parse 同）。
- `format_ts`：字面段原样输出；指令段替换为对应零填充数字（`%Y` 4 位、
  `%m/%d/%H/%M/%S` 2 位、`%%` 输出 `%`）。
- `parse`：格式串与输入串双指针推进；字面段须精确匹配；指令段按位数贪婪扫描数字
  （`%Y` 1-4 位、其余 1-2 位），扫完构造 (year,month,day,hour,min,sec)；
  月份 1-12 / 时 0-23 / 分秒 0-59 越界 → ValueError；日 1-31 且不超当月天数
  （含闰年规则，对齐 Python strptime，2 月 30 → ValueError）；多余输入 → ValueError；
  结果 ts < 0（1970 前日期）→ ValueError（与 ts ≥ 0 约束对称，保证往返一致）。
- ts→ymdhms 与反向均复用/补写 civil 历法（既有 `unix_to_ymdhms` 为正向；
  反向 `ymdhms_to_unix` 补写 days_from_civil，同 Howard Hinnant 算法）。

### date_parts weekday

`(days_since_epoch + 3) % 7`（1970-01-01 为周四=3），结果 0=周一…6=周日；
负数情形不存在（ts ≥ 0 校验，与现 time.format 一致：ts<0 → ValueError）。

### iso / date_parts 缺省 ts

缺省取 `time.now()` 当前值；`iso` 输出 `{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z`。

## GC 安全

- 返回值为 string / dict / Float，经既有 alloc 路径；无新根集。

## 验证标准

1. now_ms() 与 time.now()*1000 差 < 50ms
2. monotonic() 两次调用递增；sleep(0.05) 前后差 ≥ 0.05
3. iso(0) == "1970-01-01T00:00:00Z"；iso(1700000000) == "2023-11-14T22:13:20Z"
4. date_parts(0) == {year:1970, month:1, day:1, hour:0, minute:0, second:0, weekday:3}
   （1970-01-01 为周四）
5. date_parts(0).weekday == 3；date_parts(86400).weekday == 4（周五）
6. format_ts(0, "%Y-%m-%d %H:%M:%S") == "1970-01-01 00:00:00"
7. parse("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S") == 1700000000.0
8. parse/format_ts 往返一致（多组样本）
9. parse 非法：字面不匹配 / 月 13 / 多余尾部 → ValueError
10. sleep_ms(-1) → ValueError；sleep_ms("x") → TypeError
11. iso(NaN) → ValueError（Float ts 非有限值显式报错，禁止静默饱和）
12. 未知指令：format_ts(0, "%q") / parse("x", "%q") → ValueError
13. parse("2023-02-30", "%Y-%m-%d") → ValueError（日越界当月天数）
14. parse("1969-12-31 23:59:59", "%Y-%m-%d %H:%M:%S") → ValueError（结果 ts < 0）
15. 同名冲突回归：json.parse('1') 与 time.parse(...) 同脚本并存正确
16. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_time_ext.ms

验证标准 3-15（assert + ALL PASSED；1-2 因时序抖动仅作宽松断言；11 的 NaN 经
math.nan 常量构造）。

### Rust 单测（time.rs 内）

- `unix_to_ymdhms` / `ymdhms_to_unix` 往返（含闰日 2000-02-29 / 2024-02-29 / 2100-02-28）
- format_ts / parse 指令扫描边界（%% 结尾、孤立 % 结尾、未知指令 %q → ValueError）

## 文档更新

- `docs/mslang/10-builtins.md` — time 章节扩表（7 新函数 + ts 校验说明；
  sleep_ms 注明协程场景用 async.sleep）
- `docs/mslang/tasks/README.md` — task 83 状态 ⬜ → ✅
