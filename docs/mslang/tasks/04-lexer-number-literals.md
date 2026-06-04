# 数值字面量解析

## 所属阶段
Phase 1.3b - 基础设施

## 前置任务
03-lexer-core

## 目标
在 Lexer 核心框架上实现完整的数值字面量解析，支持十进制、十六进制、二进制、八进制整数及浮点数。

## 设计规格

参照 [01-lexical](../01-lexical.md) § 字面量 / 整数字面量 / 浮点字面量：

```
integer = decimal | hex | binary | octal
decimal = [1-9][0-9]* | 0
hex     = 0[xX][0-9a-fA-F]+
binary  = 0[bB][01]+
octal   = 0[oO][0-7]+

float = [0-9]+ "." [0-9]+ ([eE][+-]?[0-9]+)?
      | [0-9]+ [eE][+-]?[0-9]+
```

数值字面量优先级（见 [01-lexical](../01-lexical.md) § 数值字面量优先级）：

1. `0x` / `0X` 开头 → 十六进制
2. `0b` / `0B` 开头 → 二进制
3. `0o` / `0O` 开头 → 八进制
4. 包含 `.` 或 `e`/`E` → 浮点数
5. 其他 → 十进制整数

## 实现细节

### 文件位置

`src/lexer/mod.rs` 中添加 `read_number()` 及辅助方法。

### read_number()

```rust
fn read_number(&mut self, first: char, start: Position) -> Result<Token> {
    if first == '0' {
        match self.peek_char() {
            Some('x') | Some('X') => {
                self.advance();
                return self.read_hex(start);
            }
            Some('b') | Some('B') => {
                self.advance();
                return self.read_binary(start);
            }
            Some('o') | Some('O') => {
                self.advance();
                return self.read_octal(start);
            }
            Some('.') => {
                self.advance();
                return self.read_float_after_dot("0".to_string(), start);
            }
            _ => {
                return Ok(self.make_token(TokenKind::Int(0), start, "0"));
            }
        }
    }
    let mut digits = String::new();
    digits.push(first);
    while let Some(c) = self.peek_char() {
        if c.is_ascii_digit() {
            digits.push(c);
            self.advance();
        } else if c == '.' {
            self.advance();
            return self.read_float_after_dot(digits, start);
        } else if c == 'e' || c == 'E' {
            self.advance();
            return self.read_float_exponent(digits, start);
        } else {
            break;
        }
    }
    let value: i64 = digits.parse().map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid integer literal: {}", digits),
    })?;
    Ok(self.make_token(TokenKind::Int(value), start, &digits))
}
```

### read_hex()

```rust
fn read_hex(&mut self, start: Position) -> Result<Token> {
    let mut digits = String::new();
    while let Some(c) = self.peek_char() {
        if c.is_ascii_hexdigit() {
            digits.push(c);
            self.advance();
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "expected hex digits after '0x'".into(),
        });
    }
    let value = i64::from_str_radix(&digits, 16).map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid hex literal: 0x{}", digits),
    })?;
    Ok(self.make_token(TokenKind::Int(value), start, &format!("0x{}", digits)))
}
```

### read_binary()

```rust
fn read_binary(&mut self, start: Position) -> Result<Token> {
    let mut digits = String::new();
    while let Some(c) = self.peek_char() {
        if c == '0' || c == '1' {
            digits.push(c);
            self.advance();
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "expected binary digits after '0b'".into(),
        });
    }
    let value = i64::from_str_radix(&digits, 2).map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid binary literal: 0b{}", digits),
    })?;
    Ok(self.make_token(TokenKind::Int(value), start, &format!("0b{}", digits)))
}
```

### read_octal()

```rust
fn read_octal(&mut self, start: Position) -> Result<Token> {
    let mut digits = String::new();
    while let Some(c) = self.peek_char() {
        if c >= '0' && c <= '7' {
            digits.push(c);
            self.advance();
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "expected octal digits after '0o'".into(),
        });
    }
    let value = i64::from_str_radix(&digits, 8).map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid octal literal: 0o{}", digits),
    })?;
    Ok(self.make_token(TokenKind::Int(value), start, &format!("0o{}", digits)))
}
```

### read_float_after_dot()

```rust
fn read_float_after_dot(&mut self, int_part: String, start: Position) -> Result<Token> {
    let mut frac = String::new();
    while let Some(c) = self.peek_char() {
        if c.is_ascii_digit() {
            frac.push(c);
            self.advance();
        } else {
            break;
        }
    }
    if frac.is_empty() {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "expected digits after decimal point".into(),
        });
    }
    let full = if let Some(c) = self.peek_char() {
        if c == 'e' || c == 'E' {
            self.advance();
            let exp = self.read_exponent(start)?;
            format!("{}.{}e{}", int_part, frac, exp)
        } else {
            format!("{}.{}", int_part, frac)
        }
    } else {
        format!("{}.{}", int_part, frac)
    };
    let value: f64 = full.parse().map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid float literal: {}", full),
    })?;
    Ok(self.make_token(TokenKind::Float(value), start, &full))
}
```

### read_exponent()

```rust
fn read_exponent(&mut self, start: Position) -> Result<String> {
    let mut exp = String::new();
    if let Some(c) = self.peek_char() {
        if c == '+' || c == '-' {
            exp.push(c);
            self.advance();
        }
    }
    while let Some(c) = self.peek_char() {
        if c.is_ascii_digit() {
            exp.push(c);
            self.advance();
        } else {
            break;
        }
    }
    if exp.is_empty() || (exp == "+" || exp == "-") {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "expected digits in exponent".into(),
        });
    }
    Ok(exp)
}
```

### read_float_exponent()

处理无小数点的科学计数法形式（如 `1e10`, `2E-5`）：

```rust
fn read_float_exponent(&mut self, int_part: String, start: Position) -> Result<Token> {
    let exp = self.read_exponent(start)?;
    let full = format!("{}e{}", int_part, exp);
    let value: f64 = full.parse().map_err(|_| MspError::LexError {
        line: start.line,
        column: start.column,
        message: format!("invalid float literal: {}", full),
    })?;
    Ok(self.make_token(TokenKind::Float(value), start, &full))
}
```

### 注意事项

- `0` 后面不跟 `x`/`b`/`o`/`.` 时，解析为十进制 `0`
- `0xFF` → `Int(255)`，`0b1010` → `Int(10)`，`0o755` → `Int(493)`
- `3.14` → `Float(3.14)`，`1.5e-3` → `Float(0.0015)`，`1e10` → `Float(10000000000.0)`
- 浮点数至少有一个小数位：`3.` 是非法的（缺少小数部分）
- `0.` 同样非法
- `1e10` 形式（无小数点）通过 `read_float_exponent()` 处理

## 验证标准

1. 十进制整数、十六进制、二进制、八进制正确解析
2. 浮点数（含科学计数法）正确解析
3. 非法数值格式返回 `LexError`
4. 边界值：`0`、大数、前导零处理

## 测试用例

```ms
a = 42
b = 0xFF
c = 0b1010
d = 0o755
e = 3.14
f = 1.5e-3
g = 1e10
```

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::TokenKind;

    fn tokenize(source: &str) -> Vec<Token> {
        Lexer::new(source).tokenize_all().unwrap()
    }

    fn find_kind(tokens: &[Token], target: TokenKind) -> bool {
        tokens.iter().any(|t| t.kind == target)
    }

    #[test]
    fn test_decimal() {
        let tokens = tokenize("a = 42\n");
        assert!(find_kind(&tokens, TokenKind::Int(42)));
    }

    #[test]
    fn test_hex() {
        let tokens = tokenize("b = 0xFF\n");
        assert!(find_kind(&tokens, TokenKind::Int(255)));
    }

    #[test]
    fn test_binary() {
        let tokens = tokenize("c = 0b1010\n");
        assert!(find_kind(&tokens, TokenKind::Int(10)));
    }

    #[test]
    fn test_octal() {
        let tokens = tokenize("d = 0o755\n");
        assert!(find_kind(&tokens, TokenKind::Int(493)));
    }

    #[test]
    fn test_float_decimal() {
        let tokens = tokenize("e = 3.14\n");
        assert!(find_kind(&tokens, TokenKind::Float(3.14)));
    }

    #[test]
    fn test_float_exponent_negative() {
        let tokens = tokenize("f = 1.5e-3\n");
        assert!(find_kind(&tokens, TokenKind::Float(0.0015)));
    }

    #[test]
    fn test_float_exponent_only() {
        let tokens = tokenize("g = 1e10\n");
        assert!(find_kind(&tokens, TokenKind::Float(1e10)));
    }

    #[test]
    fn test_zero() {
        let tokens = tokenize("z = 0\n");
        assert!(find_kind(&tokens, TokenKind::Int(0)));
    }

    #[test]
    fn test_invalid_float_no_frac() {
        let result = Lexer::new("x = 3.\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_hex_no_digits() {
        let result = Lexer::new("x = 0x\n").tokenize_all();
        assert!(result.is_err());
    }
}
```
