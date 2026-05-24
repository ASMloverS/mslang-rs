# 表达式解析（优先级爬升）

## 所属阶段
Phase 1.5b - 基础设施

## 前置任务
11-parser-core

## 目标
实现完整的表达式解析器，使用递归下降 + 优先级爬升法处理 15 级运算符优先级。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 运算符优先级总表：

| 优先级 | 运算符 | 结合性 | 解析方法 |
|---|---|---|---|
| 1（最低） | `=` `+=` `-=` 等 | 右 | `parse_assignment()` |
| 2 | `if...else`（三元） | 右 | `parse_ternary()` |
| 3 | `or` | 左 | `parse_or()` |
| 4 | `and` | 左 | `parse_and()` |
| 5 | `not` | 右（一元） | `parse_not()` |
| 6 | `== != < > <= >= in is` | 左 | `parse_comparison()` |
| 7 | `\|` | 左 | `parse_bit_or()` |
| 8 | `^` | 左 | `parse_bit_xor()` |
| 9 | `&` | 左 | `parse_bit_and()` |
| 10 | `<< >>` | 左 | `parse_shift()` |
| 11 | `+ -` | 左 | `parse_addition()` |
| 12 | `* / // %` | 左 | `parse_multiplication()` |
| 13 | `- ~`（一元） | 右 | `parse_unary()` |
| 14 | `**` | 右 | `parse_power()` |
| 15（最高） | `() [] .`（后缀） | 左 | `parse_postfix()` |

参照 [03-syntax](../03-syntax.md) § 比较运算 — 链式比较：

```ms
1 < x < 10     # 等价于 (1 < x) and (x < 10)
```

## 实现细节

### 文件位置

`src/parser/expression.rs`

### 入口方法

```rust
pub fn parse_expression(&mut self) -> Result<Expr> {
    self.parse_assignment()
}
```

### parse_assignment() — 优先级 1

```rust
fn parse_assignment(&mut self) -> Result<Expr> {
    let expr = self.parse_ternary()?;

    let assign_ops = [
        (TokenKind::Equal, AssignOp::Assign),
        (TokenKind::PlusEqual, AssignOp::PlusAssign),
        (TokenKind::MinusEqual, AssignOp::MinusAssign),
        (TokenKind::StarEqual, AssignOp::StarAssign),
        (TokenKind::SlashEqual, AssignOp::SlashAssign),
        (TokenKind::DoubleSlashEqual, AssignOp::DoubleSlashAssign),
        (TokenKind::PercentEqual, AssignOp::PercentAssign),
        (TokenKind::DoubleStarEqual, AssignOp::DoubleStarAssign),
        (TokenKind::AmpersandEqual, AssignOp::BitAndAssign),
        (TokenKind::PipeEqual, AssignOp::BitOrAssign),
        (TokenKind::CaretEqual, AssignOp::BitXorAssign),
        (TokenKind::LeftShiftEqual, AssignOp::LeftShiftAssign),
        (TokenKind::RightShiftEqual, AssignOp::RightShiftAssign),
    ];

    for (token_kind, op) in &assign_ops {
        if self.check(token_kind) {
            self.advance();
            let value = self.parse_assignment()?;
            return Ok(Expr::Assign {
                target: Box::new(expr.clone()),
                op: op.clone(),
                value: Box::new(value),
            });
        }
    }

    // 检查 := 短声明
    if self.check(&TokenKind::ColonEqual) {
        self.advance();
        let value = self.parse_assignment()?;
        if let Expr::Identifier(name) = &expr {
            return Ok(Expr::Assign {
                target: Box::new(expr.clone()),
                op: AssignOp::Assign,
                value: Box::new(value),
            });
        }
        return Err(MspError::ParseError {
            line: self.previous().span.start.line,
            column: self.previous().span.start.column,
            message: "invalid assignment target for :=".into(),
        });
    }

    Ok(expr)
}
```

### parse_ternary() — 优先级 2

```rust
fn parse_ternary(&mut self) -> Result<Expr> {
    let expr = self.parse_or()?;

    if self.check(&TokenKind::If) {
        self.advance();
        let condition = self.parse_or()?;
        self.expect(TokenKind::Else, "expected 'else' in ternary expression")?;
        let else_expr = self.parse_ternary()?;
        return Ok(Expr::Ternary {
            condition: Box::new(condition),
            then_expr: Box::new(expr),
            else_expr: Box::new(else_expr),
        });
    }

    Ok(expr)
}
```

### parse_or() — 优先级 3

```rust
fn parse_or(&mut self) -> Result<Expr> {
    let mut expr = self.parse_and()?;

    while self.match_token(&[TokenKind::Or]) {
        let right = self.parse_and()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::Or,
            right: Box::new(right),
        };
    }

    Ok(expr)
}
```

### parse_and() — 优先级 4

```rust
fn parse_and(&mut self) -> Result<Expr> {
    let mut expr = self.parse_not()?;

    while self.match_token(&[TokenKind::And]) {
        let right = self.parse_not()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::And,
            right: Box::new(right),
        };
    }

    Ok(expr)
}
```

### parse_not() — 优先级 5

```rust
fn parse_not(&mut self) -> Result<Expr> {
    if self.match_token(&[TokenKind::Not]) {
        let operand = self.parse_not()?;
        return Ok(Expr::Unary {
            op: UnaryOp::Not,
            operand: Box::new(operand),
        });
    }
    self.parse_comparison()
}
```

### `is` / `in` 双角色说明

`TokenKind::Is` 和 `TokenKind::In` 既是关键字 token，又是比较运算符。词法分析器始终返回关键字形式的 token，表达式解析器在 `parse_comparison()` 中将其作为二元比较运算符处理。这与 Python 的处理方式一致。

### parse_comparison() — 优先级 6（支持链式比较）

```rust
fn parse_comparison(&mut self) -> Result<Expr> {
    let mut expr = self.parse_bit_or()?;

    let comp_ops = [
        (TokenKind::EqualEqual, BinaryOp::Equal),
        (TokenKind::BangEqual, BinaryOp::NotEqual),
        (TokenKind::Less, BinaryOp::Less),
        (TokenKind::Greater, BinaryOp::Greater),
        (TokenKind::LessEqual, BinaryOp::LessEqual),
        (TokenKind::GreaterEqual, BinaryOp::GreaterEqual),
        (TokenKind::In, BinaryOp::In),
        (TokenKind::Is, BinaryOp::Is),
    ];

    let mut found_op = true;
    while found_op {
        found_op = false;
        for (token_kind, op) in &comp_ops {
            if self.check(token_kind) {
                self.advance();
                let right = self.parse_bit_or()?;
                expr = Expr::Binary {
                    left: Box::new(expr),
                    op: op.clone(),
                    right: Box::new(right),
                };
                found_op = true;
                break;
            }
        }
    }

    Ok(expr)
}
```

链式比较 `1 < x < 10` 解析为嵌套的 `Binary(Binary(1, <, x), and, Binary(x, <, 10))`，此转换可在编译阶段完成，Parser 层保持原始嵌套即可。

### parse_bit_or() — 优先级 7

```rust
fn parse_bit_or(&mut self) -> Result<Expr> {
    let mut expr = self.parse_bit_xor()?;
    while self.match_token(&[TokenKind::Pipe]) {
        let right = self.parse_bit_xor()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::BitOr,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_bit_xor() — 优先级 8

```rust
fn parse_bit_xor(&mut self) -> Result<Expr> {
    let mut expr = self.parse_bit_and()?;
    while self.match_token(&[TokenKind::Caret]) {
        let right = self.parse_bit_and()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::BitXor,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_bit_and() — 优先级 9

```rust
fn parse_bit_and(&mut self) -> Result<Expr> {
    let mut expr = self.parse_shift()?;
    while self.match_token(&[TokenKind::Ampersand]) {
        let right = self.parse_shift()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::BitAnd,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_shift() — 优先级 10

```rust
fn parse_shift(&mut self) -> Result<Expr> {
    let mut expr = self.parse_addition()?;
    loop {
        let op = if self.match_token(&[TokenKind::LeftShift]) {
            BinaryOp::LeftShift
        } else if self.match_token(&[TokenKind::RightShift]) {
            BinaryOp::RightShift
        } else {
            break;
        };
        let right = self.parse_addition()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_addition() — 优先级 11

```rust
fn parse_addition(&mut self) -> Result<Expr> {
    let mut expr = self.parse_multiplication()?;
    loop {
        let op = if self.match_token(&[TokenKind::Plus]) {
            BinaryOp::Add
        } else if self.match_token(&[TokenKind::Minus]) {
            BinaryOp::Sub
        } else {
            break;
        };
        let right = self.parse_multiplication()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_multiplication() — 优先级 12

```rust
fn parse_multiplication(&mut self) -> Result<Expr> {
    let mut expr = self.parse_unary()?;
    loop {
        let op = if self.match_token(&[TokenKind::Star]) {
            BinaryOp::Mul
        } else if self.match_token(&[TokenKind::Slash]) {
            BinaryOp::Div
        } else if self.match_token(&[TokenKind::DoubleSlash]) {
            BinaryOp::FloorDiv
        } else if self.match_token(&[TokenKind::Percent]) {
            BinaryOp::Mod
        } else {
            break;
        };
        let right = self.parse_unary()?;
        expr = Expr::Binary {
            left: Box::new(expr),
            op,
            right: Box::new(right),
        };
    }
    Ok(expr)
}
```

### parse_unary() — 优先级 13

```rust
fn parse_unary(&mut self) -> Result<Expr> {
    if self.match_token(&[TokenKind::Minus]) {
        let operand = self.parse_unary()?;
        return Ok(Expr::Unary {
            op: UnaryOp::Negate,
            operand: Box::new(operand),
        });
    }
    if self.match_token(&[TokenKind::Tilde]) {
        let operand = self.parse_unary()?;
        return Ok(Expr::Unary {
            op: UnaryOp::BitNot,
            operand: Box::new(operand),
        });
    }
    self.parse_power()
}
```

### parse_power() — 优先级 14（右结合）

```rust
fn parse_power(&mut self) -> Result<Expr> {
    let base = self.parse_postfix()?;
    if self.match_token(&[TokenKind::DoubleStar]) {
        let exponent = self.parse_unary()?;
        return Ok(Expr::Binary {
            left: Box::new(base),
            op: BinaryOp::Power,
            right: Box::new(exponent),
        });
    }
    Ok(base)
}
```

注意：`**` 的右侧调用 `parse_unary()` 而非 `parse_power()`，因为一元运算符优先级低于幂运算。但实际上 `2 ** -3` 应该是合法的（`2 ** (-3)`），所以右侧应调用 `parse_unary()`。然而为了实现右结合 `2 ** 3 ** 2` = `2 ** (3 ** 2)`，右侧实际上应该调用 `parse_power()` 或自身。

正确实现：

```rust
fn parse_power(&mut self) -> Result<Expr> {
    let base = self.parse_postfix()?;
    if self.match_token(&[TokenKind::DoubleStar]) {
        let exponent = self.parse_unary()?;
        return Ok(Expr::Binary {
            left: Box::new(base),
            op: BinaryOp::Power,
            right: Box::new(exponent),
        });
    }
    Ok(base)
}
```

`2 ** 3 ** 2` 的解析过程：`parse_power()` 解析 `2`，遇到 `**`，递归调用 `parse_unary()`，`parse_unary()` 调用 `parse_power()` 解析 `3 ** 2`，实现右结合。

### parse_postfix() — 优先级 15

```rust
fn parse_postfix(&mut self) -> Result<Expr> {
    let mut expr = self.parse_primary()?;

    loop {
        if self.match_token(&[TokenKind::LeftParen]) {
            let args = self.parse_arguments()?;
            self.expect(TokenKind::RightParen, "expected ')' after arguments")?;
            expr = Expr::Call {
                callee: Box::new(expr),
                args,
            };
        } else if self.match_token(&[TokenKind::LeftBracket]) {
            if self.is_slice() {
                expr = self.parse_slice(expr)?;
            } else {
                let index = self.parse_expression()?;
                self.expect(TokenKind::RightBracket, "expected ']'")?;
                expr = Expr::Index {
                    object: Box::new(expr),
                    index: Box::new(index),
                };
            }
        } else if self.match_token(&[TokenKind::Dot]) {
            let name = self.expect(TokenKind::Identifier(String::new()), "expected property name")?;
            if let TokenKind::Identifier(n) = &name.kind {
                expr = Expr::Dot {
                    object: Box::new(expr),
                    name: n.clone(),
                };
            }
        } else {
            break;
        }
    }

    Ok(expr)
}
```

### parse_primary() — 初等表达式

> **注意**：`yield` 和 `await` 作为一元前缀表达式，其优先级与后缀表达式同级（优先级 15）。
> `yield expr` 在 `parse_yield_expr()` 中处理，`await expr` 在 `parse_primary()` 中处理。
> `await` 的操作数调用 `parse_power()` 而非 `parse_postfix()`，确保 `await x.y()` 解析为 `await (x.y())` 而非 `(await x).y()`。

```rust
fn parse_primary(&mut self) -> Result<Expr> {
    let tok = self.peek();

    match &tok.kind {
        TokenKind::Int(v) => {
            let v = *v;
            self.advance();
            Ok(Expr::Literal(Literal::Int(v)))
        }
        TokenKind::Float(v) => {
            let v = *v;
            self.advance();
            Ok(Expr::Literal(Literal::Float(v)))
        }
        TokenKind::String(s) => {
            let s = s.clone();
            self.advance();
            Ok(Expr::Literal(Literal::String(s)))
        }
        TokenKind::True => {
            self.advance();
            Ok(Expr::Literal(Literal::Bool(true)))
        }
        TokenKind::False => {
            self.advance();
            Ok(Expr::Literal(Literal::Bool(false)))
        }
        TokenKind::Nil => {
            self.advance();
            Ok(Expr::Literal(Literal::Nil))
        }
        TokenKind::Identifier(name) => {
            let name = name.clone();
            self.advance();
            Ok(Expr::Identifier(name))
        }
        TokenKind::Super => {
            self.advance();
            self.expect(TokenKind::Dot, "expected '.' after 'super'")?;
            let name_tok = self.expect(TokenKind::Identifier(String::new()), "expected method name")?;
            if let TokenKind::Identifier(n) = &name_tok.kind {
                return Ok(Expr::SuperAccess { name: n.clone() });
            }
            Err(MspError::ParseError {
                line: tok.span.start.line,
                column: tok.span.start.column,
                message: "expected method name after 'super.'".into(),
            })
        }
        TokenKind::LeftParen => self.parse_grouping_or_tuple(),
        TokenKind::LeftBracket => self.parse_list_literal(),
        TokenKind::LeftBrace => self.parse_dict_or_set(),
        TokenKind::Fn if self.is_fn_literal() => self.parse_fn_literal(),
        TokenKind::Yield => self.parse_yield_expr(),
        TokenKind::Await => {
            self.advance();
            let expr = self.parse_power()?;
            Ok(Expr::Await { expr: Box::new(expr) })
        }
        _ => Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: format!("unexpected token: {}", tok.kind),
        }),
    }
}
```

## 验证标准

1. `cargo build` 编译通过
2. 15 级优先级全部正确实现
3. 右结合运算符（`**`, 赋值, 三元）正确处理
4. 链式比较正确解析
5. 后缀表达式正确链接（`obj.method(args)[index]`）

## 测试用例

```ms
a = 1 + 2 * 3
b = 2 ** 3 ** 2
c = a > 1 and b < 512
d = "yes" if c else "no"
e = not (a > 10)
f = 1 < a < 10
```

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_expr(source: &str) -> Result<Expr> {
        let tokens = Lexer::new(source).tokenize_all()?;
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expression()?;
        Ok(expr)
    }

    #[test]
    fn test_precedence_mul_over_add() {
        let expr = parse_expr("1 + 2 * 3").unwrap();
        match expr {
            Expr::Binary { left, op: BinaryOp::Add, right } => {
                assert!(matches!(*left, Expr::Literal(Literal::Int(1))));
                assert!(matches!(*right, Expr::Binary { op: BinaryOp::Mul, .. }));
            }
            _ => panic!("expected add at top level"),
        }
    }

    #[test]
    fn test_power_right_assoc() {
        let expr = parse_expr("2 ** 3 ** 2").unwrap();
        match expr {
            Expr::Binary { left: _, op: BinaryOp::Power, right } => {
                assert!(matches!(*right, Expr::Binary { op: BinaryOp::Power, .. }));
            }
            _ => panic!("expected power at top level"),
        }
    }

    #[test]
    fn test_and_or_precedence() {
        let expr = parse_expr("a > 1 and b < 512").unwrap();
        assert!(matches!(expr, Expr::Binary { op: BinaryOp::And, .. }));
    }

    #[test]
    fn test_ternary() {
        let expr = parse_expr("\"yes\" if c else \"no\"").unwrap();
        assert!(matches!(expr, Expr::Ternary { .. }));
    }

    #[test]
    fn test_not_unary() {
        let expr = parse_expr("not (a > 10)").unwrap();
        assert!(matches!(expr, Expr::Unary { op: UnaryOp::Not, .. }));
    }

    #[test]
    fn test_chained_comparison() {
        let expr = parse_expr("1 < a < 10").unwrap();
        match expr {
            Expr::Binary { left, op: BinaryOp::Less, right: _ } => {
                assert!(matches!(*left, Expr::Binary { op: BinaryOp::Less, .. }));
            }
            _ => panic!("expected chained less-than"),
        }
    }
}
```
