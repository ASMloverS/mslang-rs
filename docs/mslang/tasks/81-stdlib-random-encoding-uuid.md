# 标准库 - random / encoding / uuid 模块

## 所属阶段
Phase 9 - 标准库扩展（M3）

## 前置任务
78-stdlib-split

> **依赖说明**：random 复用既有 rand 0.8 依赖（StdRng）；encoding/uuid 纯手写零依赖。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.4-4.6。

## 目标

新增三个原生模块：`random`（随机数）、`encoding`（base64/hex/url 编解码）、
`uuid`（UUID v4 生成）。

## 设计规格

### random

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.4（对齐 Python random 常用集）：

| 函数 | 签名 | 说明 |
|---|---|---|
| random | () -> Float | [0,1) 均匀 |
| randint | (a, b) -> Int | 闭区间 [a,b]；a>b → ValueError；非 Int → TypeError |
| uniform | (a, b) -> Float | 端点不保证（Python 语义） |
| gauss | (mu, sigma) -> Float | Box–Muller；sigma<0 → ValueError |
| choice | (seq) -> value | list/tuple/string（string 返回单字符 string）；空 → ValueError |
| shuffle | (lst) -> nil | 原地 Fisher–Yates；非 list → TypeError |
| sample | (pop, n) -> list | 不放回；n<0 或 n>len → ValueError |
| seed | (n?) -> nil | 重置生成器；缺省系统熵播种；arity MAX |

### encoding

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.5（手写实现）：

| 函数 | 说明 |
|---|---|
| base64_encode(s) | RFC 4648 标准字母表 + `=` padding |
| base64_decode(s) | 非法字符/长度 → ValueError |
| hex_encode(s) | 字节十六进制小写 |
| hex_decode(s) | 奇数长度/非 hex → ValueError |
| url_encode(s, safe="/") | 百分号编码 UTF-8；保留 `A-Za-z0-9-_.~` 与 safe；arity MAX |
| url_decode(s) | %XX 解码；非法序列 → ValueError；`+` 保持字面 |

### uuid

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.6：

- `uuid4() -> string`：36 字符小写连字符；
  版本 4（高 4 位 of time_hi_and_version = 0100）与 variant（clock_seq_hi 2 位 = 10）置位；
  rand 生成 122 位熵。

## 实现细节

### 文件位置

- `src/vm/stdlib/random.rs` — `register_random_module` + 8 个 native 函数
- `src/vm/stdlib/encoding.rs` — `register_encoding_module` + 6 个 native 函数
- `src/vm/stdlib/uuid.rs` — `register_uuid_module` + `native_uuid4`
- `src/vm/stdlib/mod.rs` — `pub use` 转发
- `src/vm/mod.rs` — `VM::new` 注册三个模块 + `native_arities` 登记

### random 生成器持有

```rust
thread_local! {
    static RNG: RefCell<StdRng> = RefCell::new(StdRng::from_entropy());
}
```

- `seed(n)` 重置为 `SeedableRng::seed_from_u64(n)`（缺省 `from_entropy`）。
- 与 `vm/mod.rs:5148` 的 `rand::thread_rng()`（select 随机分派）**互不影响**：
  该处沿用 thread_rng，不与本模块状态共享。

### gauss（Box–Muller）

```
u1, u2 ∈ (0,1]（拒绝 0，防 log(0)）
z = sqrt(-2 ln u1) * cos(2π u2)
返回 mu + sigma * z
```

### base64 细节

- 编码：3 字节 → 4 字符，尾部 `=` padding（1 或 2 个）。
- 解码：剔除输入中 ASCII 空白后校验长度 %4==0（末组 `=` 仅允许 0/1/2 个且在尾部）；
  非法字母表字符 → ValueError（附位置）。

### url 编码细节

- 按 char 迭代，非保留字符逐 UTF-8 字节 `%HH`（大写十六进制）。
- 解码遇 `%` 后不足 2 位或非 hex → ValueError；解码出的字节序列按 UTF-8 还原 string
  （非法 UTF-8 → ValueError）。

### uuid4 细节

- `rng.fill_bytes(&mut [u8;16])`，置 version/variant 位，
  格式化 `xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`。

## GC 安全

- 三个模块全部无状态或 thread_local 纯 Rust 状态，无堆对象引用，无新根集。
- shuffle 原地交换 list 元素（`read_list_mut`），写屏障由既有 SET_INDEX 路径语义
  覆盖——native 内直接 Vec 交换不经过屏障，但元素均为已有堆内对象且 list 本身
  存活，无跨代引用变化（元素集合不变，仅重排）。保守起见实现后与 GC 负责人确认
  （ms_corpus gc 用例回归覆盖）。

## 验证标准

1. `random.seed(42)` 后序列确定：两次 seed(42) 的 `random()`/`randint(1,100)` 序列一致
2. randint 边界：randint(1,1)=1；randint(2,1) → ValueError；randint(1.5, 2) → TypeError
3. uniform(0,0)=0.0；gauss(0,-1) → ValueError
4. choice("")/sample([1],2) → ValueError；shuffle("abc") → TypeError
5. base64 编解码往返：`base64_decode(base64_encode(s)) == s`（含中文/空串）
6. base64_decode("A") → ValueError；hex_decode("abc") → ValueError
7. url_encode("a b/c") == "a%20b/c"（safe="/"）；url_decode 往返一致
8. url_decode("%ZZ") → ValueError
9. uuid4 格式与版本位：匹配 36 字符模式、第 13 位 = '4'、第 17 位 ∈ 89ab；
   连续生成 100 个互不相同
10. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_random.ms

seed 确定性 / 边界错误（验证标准 1-4）。

### tests/ms/stdlib/test_encoding.ms

往返与非法输入（验证标准 5-8），配 `.expected` 不必要（assert 输出 ALL PASSED）。

### tests/ms/stdlib/test_uuid.ms

格式/版本位/唯一性（验证标准 9）。
