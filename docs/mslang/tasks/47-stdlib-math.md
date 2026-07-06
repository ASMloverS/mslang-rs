# 标准库 - math 模块

## 所属阶段
Phase 6.2b - 标准库

## 前置任务
45-module-system

## 目标
实现 `math` 标准库模块，提供常用数学常量和函数。

## 设计规格

参照 [10-builtins](../10-builtins.md) § math：

### math 模块 API

| 名称 | 类型 | 说明 |
|---|---|---|
| `math.pi` | const | 3.141592653589793 |
| `math.e` | const | 2.718281828459045 |
| `math.sqrt(x)` | fn | 平方根 |
| `math.pow(x, y)` | fn | 幂运算 |
| `math.abs(x)` | fn | 绝对值 |
| `math.sin(x)` | fn | 正弦（弧度） |
| `math.cos(x)` | fn | 余弦（弧度） |
| `math.tan(x)` | fn | 正切（弧度） |
| `math.log(x)` | fn | 自然对数 |
| `math.log2(x)` | fn | 以 2 为底对数 |
| `math.log10(x)` | fn | 以 10 为底对数 |
| `math.exp(x)` | fn | e 的 x 次方 |
| `math.ceil(x)` | fn | 向上取整 |
| `math.floor(x)` | fn | 向下取整 |
| `math.round(x)` | fn | 四舍五入 |

## 实现细节

### 1. 原生 Rust 模块注册

`src/vm/stdlib.rs` 中注册 `math` 模块。

> **对象模型约束**（task 20/25/46）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec；task 25:102、task 46）。Module 经 task 45 的 `alloc_module(name)` 构造（无 `Module::new(name, exports)`）。

```rust
/// 构造 `math` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 2 个 inline Float 常量（pi/e）+ 13 个原生函数。
pub fn register_math_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();

    // 常量（inline Object::Float，无需堆分配）
    exports.insert("pi".to_string(), Object::Float(std::f64::consts::PI));
    exports.insert("e".to_string(), Object::Float(std::f64::consts::E));

    // 函数（alloc_native_function → Object::Ref + TypeTag::FUNCTION）
    exports.insert("sqrt".to_string(),  alloc_native_function(NativeFunction{ name: "sqrt".to_string(),  func: native_math_sqrt }));
    exports.insert("pow".to_string(),   alloc_native_function(NativeFunction{ name: "pow".to_string(),   func: native_math_pow }));
    exports.insert("abs".to_string(),   alloc_native_function(NativeFunction{ name: "abs".to_string(),   func: native_math_abs }));
    exports.insert("sin".to_string(),   alloc_native_function(NativeFunction{ name: "sin".to_string(),   func: native_math_sin }));
    exports.insert("cos".to_string(),   alloc_native_function(NativeFunction{ name: "cos".to_string(),   func: native_math_cos }));
    exports.insert("tan".to_string(),   alloc_native_function(NativeFunction{ name: "tan".to_string(),   func: native_math_tan }));
    exports.insert("log".to_string(),   alloc_native_function(NativeFunction{ name: "log".to_string(),   func: native_math_log }));
    exports.insert("log2".to_string(),  alloc_native_function(NativeFunction{ name: "log2".to_string(),  func: native_math_log2 }));
    exports.insert("log10".to_string(), alloc_native_function(NativeFunction{ name: "log10".to_string(), func: native_math_log10 }));
    exports.insert("exp".to_string(),   alloc_native_function(NativeFunction{ name: "exp".to_string(),   func: native_math_exp }));
    exports.insert("ceil".to_string(),  alloc_native_function(NativeFunction{ name: "ceil".to_string(),  func: native_math_ceil }));
    exports.insert("floor".to_string(), alloc_native_function(NativeFunction{ name: "floor".to_string(), func: native_math_floor }));
    exports.insert("round".to_string(), alloc_native_function(NativeFunction{ name: "round".to_string(), func: native_math_round }));

    let m = alloc_module("math");  // task 45：返回空壳 MsModule 的 Object::Ref
    match m {
        Object::Ref(p) => {
            unsafe { read_module_mut(p).exports = exports; }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}
```

### 1b. 原生模块与 ModuleResolver 集成

复用 task 46 建立的 `native_modules` 注册表（`src/module/resolver.rs`）。math 模块注册路径与 io 完全对称：

1. `VM::new`（`src/vm/mod.rs`，task 46 注册 io 之后）调用 `register_math_module()` 并 `module_resolver.native_modules.insert("math".to_string(), ptr)`。
2. `import math` / `import @std math` / `from math import sqrt, pi` 均经 `VM::load_module` 顶部查 `native_modules` 命中，跳过磁盘搜索（task 46 §1b 已建立该路径，无需重复实现）。
3. `@std` 前缀经 `parse_std_prefix` 剥离后得 `"math"`，同样命中（原生模块不区分 @std）。

```rust
// VM::new（src/vm/mod.rs，紧随 task 46 的 io 注册）
let math_ptr = stdlib::register_math_module();
vm.module_resolver
    .native_modules
    .insert("math".to_string(), math_ptr);
```

### 1c. native_arities 注册

参照 task 46 §1（`src/vm/mod.rs:163-165`），所有原生函数须在 `native_arities` 表登记 arity，供 CALL 路径校验（经 `module.fn(...)` 走 GET_ATTR→CALL）：

```rust
// VM::new（紧随 io 的 arity 注册）
vm.native_arities.insert("sqrt".to_string(), 1);
vm.native_arities.insert("pow".to_string(), 2);
vm.native_arities.insert("abs".to_string(), 1);
vm.native_arities.insert("sin".to_string(), 1);
vm.native_arities.insert("cos".to_string(), 1);
vm.native_arities.insert("tan".to_string(), 1);
vm.native_arities.insert("log".to_string(), 1);
vm.native_arities.insert("log2".to_string(), 1);
vm.native_arities.insert("log10".to_string(), 1);
vm.native_arities.insert("exp".to_string(), 1);
vm.native_arities.insert("ceil".to_string(), 1);
vm.native_arities.insert("floor".to_string(), 1);
vm.native_arities.insert("round".to_string(), 1);
```

### 2. 全局函数与 math.* 的关系

`10-builtins.md:72-78` 已定义全局 `abs` / `ceil` / `floor` / `round`（task 25 注册于 `src/vm/builtins.rs:105-111`）。本 task 的 `math.abs` / `math.ceil` / `math.floor` / `math.round` 是 **模块限定版**，与全局版语义一致但经 `math.` 前缀访问：

| 函数 | 全局签名 | math 签名 | 关系 |
|---|---|---|---|
| `abs` | `abs(n) -> number` | `math.abs(x) -> number` | 同语义；保留入参类型（Int→Int, Float→Float） |
| `ceil` | `ceil(n) -> int` | `math.ceil(x) -> int` | 同语义；返回 Int |
| `floor` | `floor(n) -> int` | `math.floor(x) -> int` | 同语义；返回 Int |
| `round` | `round(n, digits?) -> number` | `math.round(x) -> int` | math 版固定取整（无 digits），返回 Int；等价全局 `round(x, 0)` 但返回类型不同（全局保留 Float，math 返回 Int） |

> 本 task 不修改全局版本（task 25 已实现），仅在 math 模块中添加模块限定版。`math.pi` / `math.e` / `math.sqrt` / `math.pow` / `sin` / `cos` / `tan` / `log` / `log2` / `log10` / `exp` 是 math 模块独有（全局无对应）。

### 3. expect_number 辅助函数

参照 task 46 `expect_string`（`src/vm/stdlib.rs:242-254`）的 `Option<&Object>` + None→TypeError 模式：

```rust
/// 从预期为数值的参数提取 f64（Int 自动转 Float）。
/// None（缺参）或非数值 → TypeError。
fn expect_number(arg: Option<&Object>, who: &str) -> Result<f64, String> {
    match arg {
        Some(Object::Int(n)) => Ok(*n as f64),
        Some(Object::Float(x)) => Ok(*x),
        Some(Object::Bool(b)) => Ok(if *b { 1.0 } else { 0.0 }),  // bool 真值=1/0
        other => Err(format!(
            "TypeError: {} expects number, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}
```

> **Bool 入参**：`02-types.md:41` Bool 是数值（truthy/falsy）。`math.sqrt(true)` → `1.0`。若不希望接受 Bool，可移除该分支（参照全局 abs 的实际实现选择）。

### 4. 各函数实现

签名统一为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（task 25:102、task 46）。**必须用 `args.get(N)`**（返回 `Option<&Object>`），不得直接 `args[N]` 索引（缺参时 panic，违反异常传播约定）：

```rust
fn native_math_sqrt(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sqrt(x)")?;
    Ok(Object::Float(x.sqrt()))
}

fn native_math_pow(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let base = expect_number(args.get(0), "pow(base, exp)")?;
    let exp = expect_number(args.get(1), "pow(base, exp)")?;
    Ok(Object::Float(base.powf(exp)))
}

fn native_math_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 保留入参类型：Int→Int, Float→Float（与全局 abs(n)->number 一致）
    match args.get(0) {
        Some(Object::Int(n)) => Ok(Object::Int(n.wrapping_abs())),
        Some(Object::Float(x)) => Ok(Object::Float(x.abs())),
        Some(Object::Bool(true)) => Ok(Object::Int(1)),
        Some(Object::Bool(false)) => Ok(Object::Int(0)),
        other => Err(format!(
            "TypeError: abs(x) expects number, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

fn native_math_sin(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sin(x)")?;
    Ok(Object::Float(x.sin()))
}
// cos/tan/log/log2/log10/exp 同 sin 模式，返回 Object::Float。
```

**返回值规则**（订正原 §2/§3 矛盾）：
- `math.abs(x)`：保留入参类型（Int→Int, Float→Float, Bool→Int）。
- `math.ceil/floor/round(x)`：返回 `Object::Int`（见 §5）。
- 其余（sqrt/pow/sin/cos/tan/log/log2/log10/exp）：返回 `Object::Float`。

### 5. ceil/floor/round 的 Int 转换与溢出保护

`f64::ceil()` 等返回 f64，需转 i64。Rust `as i64` 在溢出/NaN 时**静默饱和**（`1e30 → i64::MAX`，`NaN → 0`），会导致静默错误结果。须显式校验：

```rust
/// f64 → Object::Int，校验 NaN 与 i64 范围溢出。
fn float_to_int(x: f64, who: &str) -> Result<Object, String> {
    if x.is_nan() {
        return Err(format!("ValueError: {} input is NaN", who));
    }
    if x >= 9.223372036854776e18 || x < -9.223372036854776e18 {
        return Err(format!("OverflowError: {} result out of int range", who));
    }
    Ok(Object::Int(x as i64))
}

fn native_math_ceil(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "ceil(x)")?;
    float_to_int(x.ceil(), "ceil")
}
// floor/round 同模式，调用 float_to_int。
```

### 6. round 舍入方向

采用 Rust `f64::round()` 语义：**半远离零**（half-away-from-zero）。

- `round(3.5)` → `4`，`round(2.5)` → `3`，`round(-2.5)` → `-3`，`round(0.5)` → `1`。
- 这与中文「四舍五入」传统一致，但**不同于 Python `round()` 的银行家舍入**（半偶数：`round(2.5)` → `2`）。mslang 多处对标 Python，此处刻意区分。

### 7. math 域错误行为

math 函数的域错误（如负数 sqrt、负数 log、溢出）返回 **NaN / ±Infinity**，**不抛异常**（IEEE 754 一致，`02-types.md:96-99`）：

| 调用 | 结果 | 说明 |
|---|---|---|
| `math.sqrt(-1)` | `NaN` | 负数平方根 |
| `math.log(0)` | `-Infinity` | 零的对数 |
| `math.log(-1)` | `NaN` | 负数对数 |
| `math.log2(-1)` | `NaN` | 同上 |
| `math.log10(-1)` | `NaN` | 同上 |
| `math.exp(710)` | `Infinity` | 指数溢出 |
| `math.pow(0, -1)` | `Infinity` | 零的负幂 |
| `math.pow(-1, 0.5)` | `NaN` | 负底非整数幂 |

> NaN 检测：`02-types.md:97` 使用 `x != x`（NaN 不等于自身）。`NaN` 不可作为 dict 键（`02-types.md:352`）。

### 8. GC 集成

math 模块无 Rust 资源（不持 `std::fs::File` 等），**无需 finalizer**，无需新增 TypeTag（不同于 task 46 的 FILE_HANDLE）。

- **MODULE trace**：task 45 已实现，遍历 `exports` 中的 `Object::Ref`（13 个 FUNCTION 对象指针）。inline `Object::Float` 常量（pi/e）不参与 GC 扫描（内联值无需标记，`14-gc.md:54`）。
- **alloc_module 代数**：默认 Immortal（task 45），不进入 Young 代半空间复制。

## 验证标准

1. `math.pi` 和 `math.e` 常量值正确
2. 所有三角函数结果正确
3. 对数函数结果正确
4. `ceil/floor/round` 正确取整
5. 整数参数自动转换为浮点数
6. 无效输入（如负数 sqrt）返回 NaN 而非崩溃
7. `math.abs` 保留入参类型（`abs(-42)` → Int `42`，`abs(-3.14)` → Float `3.14`）
8. `math.round(2.5)` → `3`（半远离零，非银行家舍入）
9. `math.ceil(NaN)` 抛 `ValueError`；`math.ceil(1e30)` 抛 `OverflowError`

## 测试用例

### test_math.ms

```ms
import math

print(math.pi)
print(math.sqrt(16))
print(math.pow(2, 10))
print(math.sin(math.pi / 2))
print(math.log(100))
print(math.log2(8))
print(math.log10(100))
```

预期输出：
```
3.141592653589793
4.0
1024.0
1.0
4.605170185988091
3.0
2.0
```

### test_math_extra.ms

```ms
import math

print(math.cos(0))
print(math.tan(0))
print(math.exp(1))
print(math.ceil(3.2))
print(math.floor(3.8))
print(math.round(3.5))
print(math.round(2.5))
print(math.abs(-42))
```

预期输出：
```
1.0
0.0
2.718281828459045
4
3
4
3
42
```
