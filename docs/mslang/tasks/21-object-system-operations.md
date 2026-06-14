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
| int | `+ - * /` | float | float |
| float | `+ - * / // % **` | float | float |
| string | `+` | string | string（拼接） |
| string | `*` | int | string（重复） |
| list | `+` | list | list（拼接） |
| list | `*` | int | list（重复） |

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
                if *b < 0 {
                    return Err("TypeError: can't multiply string by negative int".to_string());
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

    pub fn floor_divide(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) | (_, Object::Float(0.0)) => {
                Err("ZeroDivisionError: integer division or modulo by zero".to_string())
            }
            (Object::Int(a), Object::Int(b)) => {
                Ok(Object::Int((*a as f64 / *b as f64).floor() as i64))
            }
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
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a % b)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a % b)),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 % b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a % *b as f64)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for %: '{}' and '{}'",
                self.type_name(), other.type_name()
            )),
        }
    }

    pub fn power(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) if *b >= 0 => {
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
            Object::Int(n) => Ok(Object::Int(-n)),
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

```rust
impl Object {
    pub fn compare(&self, other: &Object, op: &OpCode) -> Result<Object, String> {
        let result = match op {
            OpCode::Equal => self == other,
            OpCode::NotEqual => self != other,
            OpCode::Less => self.try_less(other)?,
            OpCode::Greater => self.try_greater(other)?,
            OpCode::LessEqual => self.try_less_equal(other)?,
            OpCode::GreaterEqual => self.try_greater_equal(other)?,
            _ => return Err("Invalid comparison op".to_string()),
        };
        Ok(Object::Bool(result))
    }

    fn try_less(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a < b),
            (Object::Float(a), Object::Float(b)) => Ok(a < b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) < *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a < (*b as f64)),
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (*(*a)).type_tag } == TypeTag::STRING as u8
                && unsafe { (*(*b)).type_tag } == TypeTag::STRING as u8 =>
            {
                Ok(unsafe { read_str(*a) } < unsafe { read_str(*b) })
            }
            _ => Err(format!(
                "TypeError: '{}' not supported between instances of '{}' and '{}'",
                "<", self.type_name(), other.type_name()
            )),
        }
    }
}
```

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
            (Object::Int(a), Object::Int(b)) if *b < 0 => {
                Err("ValueError: negative shift count".to_string())
            }
            (Object::Int(a), Object::Int(b)) if *b >= 64 => {
                Err("ValueError: shift count too large".to_string())
            }
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
            Object::Float(f) => Ok(Object::Int(*f as i64)),
            Object::Bool(b) => Ok(Object::Int(if *b { 1 } else { 0 })),
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
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
3. 整除 `-7 // 2 == -4`（向负无穷取整）
4. `2 ** 10 == 1024`
5. Int/Float 交叉比较正确
6. 位运算仅 Int 支持，其他类型报 TypeError
7. `and`/`or` 返回实际值（非 bool），`not` 返回 bool
8. 类型转换正确：`int("42") == 42`，`float("3.14")`，`str(42)`
9. Int 除零报 ZeroDivisionError；Float 除零遵循 IEEE 754（`1.0 / 0.0` → Infinity）
10. Int 算术溢出抛 OverflowError（`9223372036854775807 + 1` → OverflowError）

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
}
```
