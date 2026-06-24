# Object 运算符实现

## 所属阶段
Phase 2.3b - 字节码编译 + VM 核心

## 前置任务
- 20-object-system-basic

## 目标

在 Object 基础类型上实现所有运算符操作，包括算术、比较、位运算、逻辑和类型转换函数。运算符实现是 VM 执行指令的核心依赖。

## 设计规格

引用 [02-types.md](../02-types.md) 运算符类型规则：

### 算术运算规则

| 左 | 运算 | 右 | 结果类型 |
|---|---|---|---|
| int | `+ - * // % ** & \| ^ << >>` | int | int |
| int | `/` | int | float |
| int | `+ - * / // % **` | float | float |
| int | `& \| ^ << >>` | float | TypeError |
| float | `+ - * / // % **` | float | float |
| float | `& \| ^ << >>` | any | TypeError |
| string | `+` | string | string（拼接） |
| string | `*` | int | string（重复） |
| list | `+` | list | list（拼接） — 由 task 22 实现 |
| list | `*` | int | list（重复） — 由 task 22 实现 |

### 整除规则

向负无穷方向取整（与 Python 一致）：

```
-7 // 2 == -4    # 向负无穷取整
7 // 2 == 3
```

### 比较运算规则

| 左 | 比较 | 右 | 行为 |
|---|---|---|---|
| int | `== !=` | float | 数值比较 |
| int | `< > <= >=` | float | 数值比较 |
| 其他不同类型 | 比较 | — | `==` 返回 false，`< >` 抛出 TypeError |

### 逻辑运算规则

```
a and b    # a 为 falsy 则返回 a，否则返回 b
a or b     # a 为 truthy 则返回 a，否则返回 b
not a      # 返回 bool
```

### 位运算规则

仅 `int` 类型支持：`& | ^ ~ << >>`

## 实现细节

### 文件位置

`src/vm/object.rs`（扩展任务 20 的 Object 实现）

### 算术运算

```rust
impl Object {
    pub fn add(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => {
                a.checked_add(*b)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer addition overflow".to_string())
            }
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 + b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a + *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a + b)),
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (*(*a)).type_tag } == TypeTag::STRING as u8
                && unsafe { (*(*b)).type_tag } == TypeTag::STRING as u8 =>
            {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let result = unsafe { read_str(*a) }.to_owned() + unsafe { read_str(*b) };
                Ok(alloc_string(&result))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for +: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn subtract(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => {
                a.checked_sub(*b)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer subtraction overflow".to_string())
            }
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 - b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a - *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a - b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for -: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn multiply(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => {
                a.checked_mul(*b)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer multiplication overflow".to_string())
            }
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 * b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a * *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a * b)),
            (Object::Ref(a), Object::Int(b)) | (Object::Int(b), Object::Ref(a))
                if unsafe { (*(*a)).type_tag } == TypeTag::STRING as u8 =>
            {
                debug_assert!(!a.is_null(), "null Object::Ref");
                if *b < 0 {
                    return Err("TypeError: can't multiply string by negative int".to_string());
                }
                // 防止 `*b as usize` 触发 OOM abort：限制结果总长度（DoS 缓解）
                const MAX_REPEAT_LEN: usize = 1 << 30; // 1 GiB 上限
                let unit = unsafe { read_str(*a) }.len();
                let total = unit
                    .checked_mul(*b as usize)
                    .ok_or_else(|| "OverflowError: string repeat count too large".to_string())?;
                if total > MAX_REPEAT_LEN {
                    return Err("MemoryError: string repeat result too large".to_string());
                }
                let repeated = unsafe { read_str(*a) }.repeat(*b as usize);
                Ok(alloc_string(&repeated))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for *: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn divide(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) => {
                Err("ZeroDivisionError: division by zero".to_string())
            }
            (Object::Int(a), Object::Int(b)) => Ok(Object::Float(*a as f64 / *b as f64)),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 / b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a / *b as f64)),
            // Float 除零遵循 IEEE 754：1.0/0.0 = +inf, -1.0/0.0 = -inf, 0.0/0.0 = NaN
            // 参照 02-types.md § 特殊浮点值
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a / b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for /: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }
}
```

> **除零行为差异（有意为之）**：`/` 对 Float 除零遵循 IEEE 754（`1.0/0.0 → +inf`、`0.0/0.0 → NaN`，不报错，`02-types.md:98`）；而 `//` 与 `%` 对 Float(0.0) 除数报 `ZeroDivisionError`（与 Python 一致）。理由：`//`/`%` 的数学定义不容许无穷结果。
>
> **GC 泄漏提示**：`add`（String 拼接）与 `multiply`（String 重复）每次调用 `alloc_string` 产生 `Box` 分配，MVP 阶段不回收（task 52 GC 接管）。循环内反复拼接（如构建大字符串）会导致内存无界增长；推荐 `.ms` 层用 `join`（task 50）等可变缓冲方案。

### 整除与取余

```rust
impl Object {
    pub fn floor_divide(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) | (_, Object::Float(0.0)) => {
                Err("ZeroDivisionError: integer division or modulo by zero".to_string())
            }
            // 整数 floor division 须为精确整数运算（02-types.md:32）。div_euclid 的商即
            // Python 的 floor division（向负无穷取整），避免 f64 中转对 >2^53 整数丢精度。
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a.div_euclid(*b))),
            (Object::Int(a), Object::Float(b)) => {
                Ok(Object::Float((*a as f64 / b).floor()))
            }
            (Object::Float(a), Object::Int(b)) => {
                Ok(Object::Float((a / *b as f64).floor()))
            }
            (Object::Float(a), Object::Float(b)) => {
                Ok(Object::Float((a / b).floor()))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for //: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn modulo(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) | (_, Object::Float(0.0)) => {
                Err("ZeroDivisionError: integer division or modulo by zero".to_string())
            }
            // floor-mod：与 floor_divide 自洽（a == (a//b)*b + (a%b)），结果取除数符号。
            // rem_euclid 给出 Euclid 余数（恒 ≥ 0 当除数 > 0），等价 Python 的 %。
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a.rem_euclid(*b))),
            (Object::Float(a), Object::Float(b)) => {
                Ok(Object::Float(a - (a / b).floor() * b))
            }
            (Object::Int(a), Object::Float(b)) => {
                let a = *a as f64;
                Ok(Object::Float(a - (a / b).floor() * b))
            }
            (Object::Float(a), Object::Int(b)) => {
                let b = *b as f64;
                Ok(Object::Float(a - (a / b).floor() * b))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for %: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn power(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) if *b >= 0 => {
                // i64 的 ** ：指数 ≥ 64 必溢出（|a|≥2 时），且 checked_pow 取 u32 指数，
                // 超大指数会被 `as u32` 截断导致静默错误值。先按溢出处理。
                if *b >= 64 {
                    return Err("OverflowError: integer power overflow".to_string());
                }
                a.checked_pow(*b as u32)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer power overflow".to_string())
            }
            (Object::Int(a), Object::Int(b)) => {
                Ok(Object::Float((*a as f64).powf(*b as f64)))
            }
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float((*a as f64).powf(*b))),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a.powf(*b as f64))),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a.powf(*b))),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for **: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn negate(&self) -> Result<Object, String> {
        match self {
            // checked_neg：-i64::MIN 溢出，须报 OverflowError（02-types.md:79）
            Object::Int(n) => n
                .checked_neg()
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer negation overflow".to_string()),
            Object::Float(n) => Ok(Object::Float(-n)),
            _ => Err(format!(
                "TypeError: bad operand type for unary -: '{}'",
                self.type_name()
            )),
        }
    }
}
```

### 比较运算

> **架构说明**：`compare` 不直接接受编译器的 `OpCode`，避免 VM Object 层反向依赖字节码指令集（compiler 依赖 vm::object，不可反向）。改用 VM 本地的 `CmpOp` 枚举；VM 执行循环（task 23/24）负责 `OpCode` → `CmpOp` 映射。

```rust
/// 比较算子（VM 本地定义，与 compiler::OpCode 解耦）。
#[derive(Debug, Clone, Copy)]
pub enum CmpOp { Equal, NotEqual, Less, Greater, LessEqual, GreaterEqual }

impl Object {
    pub fn compare(&self, other: &Object, op: CmpOp) -> Result<Object, String> {
        let result = match op {
            CmpOp::Equal => self == other,
            CmpOp::NotEqual => self != other,
            CmpOp::Less => self.try_less(other)?,
            CmpOp::Greater => self.try_greater(other)?,
            CmpOp::LessEqual => self.try_less_equal(other)?,
            CmpOp::GreaterEqual => self.try_greater_equal(other)?,
        };
        Ok(Object::Bool(result))
    }

    fn try_less(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a < b),
            (Object::Float(a), Object::Float(b)) => Ok(a < b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) < *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a < (*b as f64)),
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    Ok(unsafe { read_str(*a) } < unsafe { read_str(*b) })
                } else {
                    Err(format!(
                        "TypeError: '{}' not supported between instances of '{}' and '{}'",
                        "<", self.type_name(), other.type_name()
                    ))
                }
            }
            _ => Err(format!(
                "TypeError: '<' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }
    // try_greater / try_less_equal / try_greater_equal 同构，省略
}
```

### `is` 运算符（身份比较）

引用 [02-types.md](../02-types.md) § `is` 运算符语义：身份比较（两个引用是否指向同一堆对象），仅适用 Ref 类型；inline 类型（int/float/bool/nil）抛 `TypeError`。

```rust
impl Object {
    /// `is`：身份比较。Ref↔Ref 比指针；inline 类型抛 TypeError（02-types.md:313）。
    pub fn is_identity(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                Ok(Object::Bool(*a == *b))
            }
            // 任意一侧为 inline 类型：is 不可用
            _ => Err(format!(
                "TypeError: 'is' cannot be used with inline type '{}'/'{}'",
                self.type_name(), other.type_name()
            )),
        }
    }
}
```

> **`in` 运算符**（成员运算符，`OpCode::In`）依赖集合类型，String 子串 `in` 可在本 task 顺手实现（`read_str(haystack).contains(needle)`），List/Dict/Set 的 `in` 由 task 22 实现。本 task 至少为 String `in` 提供方法 `contains_str(&self, needle) -> Result<Object, String>`，并在文档注明集合 `in` 推迟。

### 位运算

```rust
impl Object {
    pub fn bit_and(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a & b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for &: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn bit_or(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a | b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for |: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn bit_xor(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a ^ b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for ^: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn bit_not(&self) -> Result<Object, String> {
        match self {
            Object::Int(n) => Ok(Object::Int(!n)),
            _ => Err(format!(
                "TypeError: bad operand type for unary ~: '{}'",
                self.type_name()
            )),
        }
    }

    pub fn left_shift(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(_), Object::Int(b)) if *b < 0 => {
                Err("ValueError: negative shift count".to_string())
            }
            (Object::Int(_), Object::Int(b)) if *b >= 64 => {
                Err("ValueError: shift count too large".to_string())
            }
            // 注：b ∈ [0,63] 时 checked_shl 必返回 Some（仅校验位移量，已由上面守卫保证）。
            // 位移结果若越过 i64 范围（如 1<<63 得 i64::MIN 负值）按 i64 回绕返回——
            // 02-types.md 未规定左移溢出语义，此处采用"回绕"而非 OverflowError。
            // 若后续设计要求溢出报错，改为：if b >= 64 - a.leading_zeros() { Err(...) }。
            (Object::Int(a), Object::Int(b)) => {
                a.checked_shl(*b as u32)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer left shift overflow".to_string())
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for <<: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn right_shift(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) if *b < 0 => {
                Err("ValueError: negative shift count".to_string())
            }
            (Object::Int(a), Object::Int(b)) if *b >= 64 => {
                Err("ValueError: shift count too large".to_string())
            }
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a >> b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for >>: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }
}
```

### 逻辑运算

```rust
impl Object {
    pub fn logical_not(&self) -> Object {
        Object::Bool(!self.is_truthy())
    }

    pub fn logical_and(&self, other: &Object) -> Object {
        if self.is_truthy() {
            other.clone()
        } else {
            self.clone()
        }
    }

    pub fn logical_or(&self, other: &Object) -> Object {
        if self.is_truthy() {
            self.clone()
        } else {
            other.clone()
        }
    }
}
```

### 类型转换

```rust
impl Object {
    pub fn to_int(&self) -> Result<Object, String> {
        match self {
            Object::Int(_) => Ok(self.clone()),
            Object::Float(f) => {
                // 拒绝 NaN / ±Infinity / 越界（Python 报 ValueError/OverflowError），
                // 避免 `*f as i64` 静默饱和或 NaN→0。
                if f.is_nan() {
                    return Err("ValueError: cannot convert NaN to int".to_string());
                }
                if f.is_infinite() || *f < i64::MIN as f64 || *f > i64::MAX as f64 {
                    return Err("OverflowError: float too large to convert to int".to_string());
                }
                Ok(Object::Int(*f as i64))
            }
            Object::Bool(b) => Ok(Object::Int(if *b { 1 } else { 0 })),
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let s = unsafe { read_str(*ptr) };
                s.parse::<i64>()
                    .map(Object::Int)
                    .map_err(|_| format!("ValueError: invalid literal for int(): '{}'", s))
            }
            Object::Nil => Err("TypeError: cannot convert nil to int".to_string()),
        }
    }

    pub fn to_float(&self) -> Result<Object, String> {
        match self {
            Object::Float(_) => Ok(self.clone()),
            Object::Int(n) => Ok(Object::Float(*n as f64)),
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let s = unsafe { read_str(*ptr) };
                s.parse::<f64>()
                    .map(Object::Float)
                    .map_err(|_| format!("ValueError: invalid literal for float(): '{}'", s))
            }
            _ => Err(format!("TypeError: cannot convert {} to float", self.type_name())),
        }
    }

    pub fn to_str(&self) -> Object {
        alloc_string(&format!("{}", self))
    }

    pub fn to_bool(&self) -> Object {
        Object::Bool(self.is_truthy())
    }
}
```

## 验证标准

1. `Int + Int` → `Int`，`Int / Int` → `Float`，`Int + Float` → `Float`
2. `String + String` → 拼接，`String * Int` → 重复
3. 整除 `-7 // 2 == -4`（向负无穷取整）；`-7 % 2 == 1`（floor-mod，与 `//` 自洽，`a == (a//b)*b + (a%b)`）
4. `2 ** 10 == 1024`；指数 ≥ 64 报 OverflowError（不静默截断）
5. Int/Float 交叉比较正确
6. 位运算仅 Int 支持，其他类型报 TypeError
7. `and`/`or` 返回实际值（非 bool），`not` 返回 bool
8. 类型转换正确：`int("42") == 42`，`float("3.14")`，`str(42)`；`int(NaN)`/`int(1e20)` 报错（不饱和）
9. Int 除零报 ZeroDivisionError；Float `/` 除零遵循 IEEE 754（`1.0 / 0.0` → Infinity）；Float `//`/`%` 除零报 ZeroDivisionError
10. Int 算术溢出抛 OverflowError（`9223372036854775807 + 1` → OverflowError）；`-i64::MIN` → OverflowError
11. 大整数 floor division 精确（`9007199254740993 // 1 == 9007199254740993`，不走 f64）
12. `is`：Ref↔Ref 比身份（同对象 true、等值不同对象 false），inline 类型抛 TypeError
13. `compare` 解耦 `OpCode`（使用 `CmpOp`），String 重复有上限（OOM 守卫）

## 测试用例

```ms
# test_object_operations.ms
a = 10 + 3
b = 10 / 3
c = 10 // 3
d = -7 // 2
e = 2 ** 10
f = "hello" + " world"
g = "ab" * 3
```

预期值：
- `a` = 13 (int)
- `b` ≈ 3.333... (float)
- `c` = 3 (int)
- `d` = -4 (int)
- `e` = 1024 (int)
- `f` = "hello world" (string)
- `g` = "ababab" (string)

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_add() {
        let result = Object::Int(10).add(&Object::Int(3)).unwrap();
        assert_eq!(result, Object::Int(13));
    }

    #[test]
    fn test_int_div_returns_float() {
        let result = Object::Int(10).divide(&Object::Int(3)).unwrap();
        assert!(matches!(result, Object::Float(_)));
    }

    #[test]
    fn test_floor_div_negative() {
        let result = Object::Int(-7).floor_divide(&Object::Int(2)).unwrap();
        assert_eq!(result, Object::Int(-4));
    }

    #[test]
    fn test_power() {
        let result = Object::Int(2).power(&Object::Int(10)).unwrap();
        assert_eq!(result, Object::Int(1024));
    }

    #[test]
    fn test_string_concat() {
        let result = alloc_string("hello")
            .add(&alloc_string(" world")).unwrap();
        assert_eq!(result, alloc_string("hello world"));
    }

    #[test]
    fn test_string_repeat() {
        let result = alloc_string("ab")
            .multiply(&Object::Int(3)).unwrap();
        assert_eq!(result, alloc_string("ababab"));
    }

    #[test]
    fn test_division_by_zero() {
        // Int 除零 → ZeroDivisionError
        let result = Object::Int(10).divide(&Object::Int(0));
        assert!(result.is_err());

        // Float 除零 → IEEE 754（参照 02-types.md § 特殊浮点值）
        let result = Object::Float(1.0).divide(&Object::Float(0.0)).unwrap();
        assert_eq!(result, Object::Float(f64::INFINITY));

        let result = Object::Float(-1.0).divide(&Object::Float(0.0)).unwrap();
        assert_eq!(result, Object::Float(f64::NEG_INFINITY));
    }

    #[test]
    fn test_integer_overflow() {
        let max_int = Object::Int(i64::MAX);
        let result = max_int.add(&Object::Int(1));
        assert!(result.is_err());

        let result = Object::Int(2).power(&Object::Int(63));
        assert!(result.is_err());
    }

    #[test]
    fn test_bitwise_int_only() {
        assert!(Object::Int(5).bit_and(&Object::Int(3)).is_ok());
        assert!(Object::Float(5.0).bit_and(&Object::Float(3.0)).is_err());
    }

    #[test]
    fn test_logical_short_circuit() {
        let result = Object::Int(0).logical_and(&Object::Int(42));
        assert_eq!(result, Object::Int(0));

        let result = Object::Int(1).logical_and(&Object::Int(42));
        assert_eq!(result, Object::Int(42));
    }

    #[test]
    fn test_type_conversion() {
        assert_eq!(alloc_string("42").to_int().unwrap(), Object::Int(42));
        assert_eq!(Object::Int(42).to_float().unwrap(), Object::Float(42.0));
        assert_eq!(Object::Int(0).to_bool(), Object::Bool(false));
    }

    #[test]
    fn test_floor_div_and_mod_consistency() {
        // // 与 % 自洽：a == (a//b)*b + (a%b)，且 % 取除数符号（floor-mod）
        // 负数场景（02-types.md:72-77，与 Python 一致）
        assert_eq!(Object::Int(-7).floor_divide(&Object::Int(2)).unwrap(), Object::Int(-4));
        assert_eq!(Object::Int(-7).modulo(&Object::Int(2)).unwrap(), Object::Int(1));
        assert_eq!(Object::Int(7).modulo(&Object::Int(-2)).unwrap(), Object::Int(-1));
        // 不变式验证
        for (a, b) in [(-7i64, 2), (7, -2), (-7, -2), (7, 2), (1_000_003, 7)] {
            let av = Object::Int(a);
            let bv = Object::Int(b);
            let q = if let Object::Int(q) = av.floor_divide(&bv).unwrap() { q } else { unreachable!() };
            let r = if let Object::Int(r) = av.modulo(&bv).unwrap() { r } else { unreachable!() };
            assert_eq!(q * b + r, a, "a={} b={} 不满足 (a//b)*b + a%b == a", a, b);
            // floor-mod 余数符号跟随除数（或为 0）
            assert!(r == 0 || (r < 0) == (b < 0), "a={} b={} 余数符号错误: r={}", a, b, r);
        }
    }

    #[test]
    fn test_floor_div_large_int_no_f64_loss() {
        // > 2^53 的整数 floor division 必须精确（不走 f64 路径）
        let big = 9_007_199_254_740_993i64; // 2^53 + 1
        assert_eq!(
            Object::Int(big).floor_divide(&Object::Int(1)).unwrap(),
            Object::Int(big)
        );
    }

    #[test]
    fn test_float_mod_floor_semantics() {
        // Float % 与 // 自洽，符号跟随除数
        assert_eq!(Object::Float(-7.0).modulo(&Object::Float(2.0)).unwrap(), Object::Float(1.0));
    }

    #[test]
    fn test_negate_overflow() {
        // -i64::MIN 溢出 → OverflowError（02-types.md:79）
        assert!(Object::Int(i64::MIN).negate().is_err());
        assert_eq!(Object::Int(5).negate().unwrap(), Object::Int(-5));
    }

    #[test]
    fn test_power_huge_exponent() {
        // 指数 ≥ 64 必溢出（i64），不因 `as u32` 截断返回静默错误值
        assert!(Object::Int(2).power(&Object::Int(64)).is_err());
        assert!(Object::Int(2).power(&Object::Int(1_000_000)).is_err());
    }

    #[test]
    fn test_is_identity() {
        // Ref↔Ref：同对象 → true，不同对象 → false（身份比较）
        let s1 = alloc_string("x");
        let s2 = alloc_string("x");
        assert_eq!(s1.clone().is_identity(&s1).unwrap(), Object::Bool(true));
        assert_eq!(s1.is_identity(&s2).unwrap(), Object::Bool(false));
        // inline 类型 → TypeError（02-types.md:313）
        assert!(Object::Int(42).is_identity(&Object::Int(42)).is_err());
    }
}
```
