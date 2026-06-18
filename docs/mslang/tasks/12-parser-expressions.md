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
| 13 | `- ~ <-`（一元） | 右 | `parse_unary()` |
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

    // `:=` 短声明为语句级构造（03-syntax.md:48 short_var），由 task 13
    // parse_expr_or_assignment 在语句层检测 ColonEqual 并产出 Stmt::ShortVarDecl，
    // 不在表达式层消费——此处若处理会使 task 13 的 := 分支成为死代码。
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

    // 匹配首个比较运算符；无则直接返回（单初等表达式）
    let mut first_op = None;
    for (token_kind, op) in &comp_ops {
        if self.check(token_kind) {
            first_op = Some(*op);
            break;
        }
    }
    let first_op = match first_op {
        Some(op) => op,
        None => return Ok(expr),
    };
    self.advance();
    let mut right = self.parse_bit_or()?;
    let mut result = Expr::Binary {
        left: Box::new(expr),
        op: first_op,
        right: Box::new(right.clone()),
    };

    // 后续比较运算符：在解析阶段反糖为 and 链（见 task 09 § 链式比较）
    // a < b < c  =>  (a < b) and (b < c)，中间操作数 clone 复用到两侧
    loop {
        let mut next_op = None;
        for (token_kind, op) in &comp_ops {
            if self.check(token_kind) {
                next_op = Some(*op);
                break;
            }
        }
        match next_op {
            Some(op) => {
                self.advance();
                let next = self.parse_bit_or()?;
                result = Expr::Binary {
                    left: Box::new(result),
                    op: BinaryOp::And,
                    right: Box::new(Expr::Binary {
                        left: Box::new(right),
                        op,
                        right: Box::new(next.clone()),
                    }),
                };
                right = next;
            }
            None => break,
        }
    }

    Ok(result)
}
```

链式比较 `1 < x < 10` 在解析阶段即反糖为 `(1 < x) and (x < 10)`（见 [09-ast-expression-nodes](09-ast-expression-nodes.md) § 链式比较），中间操作数 `x` 被 clone 复用到两侧比较中。单比较 `a < b` 仍为普通 `Binary(a, Less, b)`。此反糖**必须在解析阶段完成**：若保留原始左结合嵌套 `Binary(Binary(a, op, b), op, c)`，其 AST 形状与合法左结合链 `(a op b) op c` 完全相同，编译器无法区分二者，且按字面执行会把布尔结果再次比较（按 `02-types.md` 触发 TypeError）。

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
            BinaryOp::Subtract
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
            BinaryOp::Multiply
        } else if self.match_token(&[TokenKind::Slash]) {
            BinaryOp::Divide
        } else if self.match_token(&[TokenKind::DoubleSlash]) {
            BinaryOp::FloorDiv
        } else if self.match_token(&[TokenKind::Percent]) {
            BinaryOp::Modulo
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
    if self.match_token(&[TokenKind::LeftArrow]) {
        let operand = self.parse_unary()?;
        return Ok(Expr::Unary {
            op: UnaryOp::ChannelReceive,
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

`**` 的右侧调用 `parse_unary()`：既允许 `2 ** -3`（一元负号作为指数），又通过 `parse_unary() → parse_power()` 的递归实现右结合 `2 ** 3 ** 2` = `2 ** (3 ** 2)`。解析过程：`parse_power()` 解析 `2`，遇到 `**`，右侧调用 `parse_unary()`；`parse_unary()` 在无前缀运算符时落入 `parse_power()`，从而继续解析 `3 ** 2`，实现右结合。

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
            // 注意：不可用 expect(TokenKind::Identifier(String::new())) —— TokenKind 派生的
            // PartialEq（task 02）按内层 String 比较，与 check 的 ==（task 11）配合会使任意
            // 真实标识符都不匹配 Identifier("")。改用模式匹配。
            let tok = self.peek();
            if let TokenKind::Identifier(n) = &tok.kind {
                let name = n.clone();
                self.advance();
                expr = Expr::Dot {
                    object: Box::new(expr),
                    name,
                };
            } else {
                return Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "expected property name after '.'".into(),
                });
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
        TokenKind::Zelf => {
            // `self` 是关键字（task 02 Zelf），但在方法体内作为初等表达式使用。
            // 按 task 13 parse_param_name 的约定映射为 Expr::Identifier("self")，
            // 使 self.attr 的解析路径与普通标识符一致（06-oop.md:48-58）。
            self.advance();
            Ok(Expr::Identifier("self".into()))
        }
        TokenKind::Super => {
            self.advance();
            self.expect(TokenKind::Dot, "expected '.' after 'super'")?;
            // 用模式匹配而非 expect(Identifier(""))，原因见 parse_postfix 的 . 分支注释
            let name_tok = self.peek();
            if let TokenKind::Identifier(n) = &name_tok.kind {
                let name = n.clone();
                self.advance();
                return Ok(Expr::SuperAccess { name });
            }
            Err(MspError::ParseError {
                line: name_tok.span.start.line,
                column: name_tok.span.start.column,
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
        TokenKind::Go => {
            // go 表达式（08-concurrency.md:88 go_expr = "go" expression）。
            // 与 await/yield 同属前缀表达式族，操作数取 parse_unary()，
            // 使 `go f(x)`、`go fn(){...}` 正确解析为 Expr::Go（09-ast-expression-nodes.md）。
            self.advance();
            let expr = self.parse_unary()?;
            Ok(Expr::Go { expr: Box::new(expr) })
        }
        _ => Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: format!("unexpected token: {}", tok.kind),
        }),
    }
}
```

> **辅助方法归属**：`parse_primary` 与 `parse_postfix` 调用的下列方法不在本 task 实现，由后续 task 提供并替换（遵循 task 11 的占位模式，归属 task 完成前以 stub 返回 `ParseError`，集成后由对应 task 替换）：
>
> | 方法 | 归属 | 说明 |
> |---|---|---|
> | `parse_arguments` | task 13（或本 task 同期的语句解析） | 函数调用参数列表 |
> | `is_slice` / `parse_slice` | Phase 4.3（切片） | 切片语法检测与解析 |
> | `parse_grouping_or_tuple` | 集合字面量解析 task | `(...)` 分组或元组 |
> | `parse_list_literal` | 集合字面量解析 task | `[...]` 列表字面量 |
> | `parse_dict_or_set` | 集合字面量解析 task | `{...}` dict 或 set |
> | `parse_fn_literal` | task 14（匿名函数） | `fn(...){...}` 字面量 |
> | `parse_yield_expr` | Phase 4.7（生成器） | `yield` / `yield from` 表达式 |
> | `is_fn_literal` | task 13（建议下沉至 task 11 核心原语） | 区分 `fn name(` 声明与 `fn(` 字面量 |
>
> 本 task 仅提供上述方法的**分发调用点**与 15 级优先级爬升主体；这些方法的最终实现需在 [12-implementation-plan](../12-implementation-plan.md) 中明确对应 task，避免归属悬空。

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
                assert!(matches!(*right, Expr::Binary { op: BinaryOp::Multiply, .. }));
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
        // 解析阶段反糖为 (1 < a) and (a < 10)（见 task 09 § 链式比较）
        match expr {
            Expr::Binary { left, op: BinaryOp::And, right } => {
                assert!(matches!(*left, Expr::Binary { op: BinaryOp::Less, .. }));
                assert!(matches!(*right, Expr::Binary { op: BinaryOp::Less, .. }));
            }
            _ => panic!("expected and-desugared chained comparison"),
        }
    }

    #[test]
    fn test_postfix_chain() {
        // 验证后缀表达式正确链接：obj.method(args)[index]（验证标准 5）
        let expr = parse_expr("obj.method(42)[0]").unwrap();
        match expr {
            Expr::Index { object, .. } => {
                assert!(matches!(*object, Expr::Call { .. }));
            }
            _ => panic!("expected index over call"),
        }
    }
}
```
