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

### 1. 模块注册

`src/vm/stdlib.rs` 中注册 `math` 模块：

```rust
fn register_math_module(vm: &mut VM) -> Gc<Module> {
    let mut exports = HashMap::new();

    // 常量
    exports.insert("pi".into(), Object::Float(std::f64::consts::PI));
    exports.insert("e".into(), Object::Float(std::f64::consts::E));

    // 函数
    exports.insert("sqrt".into(), Object::NativeFn(native_math_sqrt));
    exports.insert("pow".into(), Object::NativeFn(native_math_pow));
    exports.insert("abs".into(), Object::NativeFn(native_math_abs));
    exports.insert("sin".into(), Object::NativeFn(native_math_sin));
    exports.insert("cos".into(), Object::NativeFn(native_math_cos));
    exports.insert("tan".into(), Object::NativeFn(native_math_tan));
    exports.insert("log".into(), Object::NativeFn(native_math_log));
    exports.insert("log2".into(), Object::NativeFn(native_math_log2));
    exports.insert("log10".into(), Object::NativeFn(native_math_log10));
    exports.insert("exp".into(), Object::NativeFn(native_math_exp));
    exports.insert("ceil".into(), Object::NativeFn(native_math_ceil));
    exports.insert("floor".into(), Object::NativeFn(native_math_floor));
    exports.insert("round".into(), Object::NativeFn(native_math_round));

    Module::new("math", exports)
}
```

### 2. 各函数实现

每个函数使用 Rust `std::f64` 内置数学方法：

```rust
fn native_math_sqrt(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let x = expect_number(&args[0])?;
    Ok(Object::Float(x.sqrt()))
}

fn native_math_pow(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let base = expect_number(&args[0])?;
    let exp = expect_number(&args[1])?;
    Ok(Object::Float(base.powf(exp)))
}

fn native_math_sin(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let x = expect_number(&args[0])?;
    Ok(Object::Float(x.sin()))
}
```

- `expect_number`：从 Object 提取 f64（Int 自动转 Float）
- 返回值统一为 `Object::Float`
- 对负数 `sqrt` 返回 `NaN`

### 3. ceil/floor/round 返回值

- `ceil/floor/round` 返回 `Object::Int`（因为结果是整数）
- `round` 四舍五入到最近整数

## 验证标准

1. `math.pi` 和 `math.e` 常量值正确
2. 所有三角函数结果正确
3. 对数函数结果正确
4. `ceil/floor/round` 正确取整
5. 整数参数自动转换为浮点数
6. 无效输入（如负数 sqrt）返回 NaN 而非崩溃

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
42
```
