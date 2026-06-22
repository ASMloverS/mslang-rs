# 语句解析

## 所属阶段
Phase 1.5c - 基础设施

## 前置任务
11-parser-core

## 目标
实现核心语句的解析：变量声明、常量声明、赋值（含多目标赋值）、控制流、函数声明、nonlocal/global 声明。

> **范围说明**：`import` / `from...import` 语句归属于 task 15（高级语句解析），见 `tasks/README.md:23`。task 13 不实现 import 解析。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 语句：

### 变量声明

```
var_stmt  = "var" IDENTIFIER "=" expression
short_var = IDENTIFIER ":=" expression
assign    = IDENTIFIER "=" expression
```

三种方式等价但语义不同：`var` 和 `:=` 总是创建新变量，`=` 赋值给已有变量或创建新变量。

### 常量声明

```
const_stmt = "const" IDENTIFIER "=" expression
```

### 赋值语句

```
assign_stmt = target ("=" | "+=" | "-=" | "*=" | "/=" | "//=" | "%=" | "**=" |
              "&=" | "|=" | "^=" | "<<=" | ">>=") expression

target = IDENTIFIER
       | target "." IDENTIFIER          // 属性赋值
       | target "[" expression "]"      // 下标赋值
```

赋值 target 必须是合法 lvalue（标识符、属性访问、下标访问）。其他表达式形式（如 `1 + 2 = 3`）解析期拒绝。

### 多目标赋值

```
multi_assign = target_list "=" expression_list
target_list = target ("," target)*
```

右侧表达式先全部求值，构造元组，再按位置解包到左侧各目标。解包数量必须匹配（`03-syntax.md:140`）。

在 AST 中表示为 `Stmt::Assign`，target 与 value 均为 `Expr::TupleLiteral`。

### 条件语句

```
if_stmt = "if" expression block ("elif" expression block)* ("else" block)?
```

### 循环语句

```
while_stmt = "while" expression block
for_stmt = "for" IDENTIFIER "in" expression block
        | "for" IDENTIFIER "," IDENTIFIER "in" expression block
```

### break / continue / return

```
break_stmt    = "break"
continue_stmt = "continue"
return_stmt   = "return" expression_list?
```

### 函数声明

```
fn_def = "fn" IDENTIFIER "(" param_list? ")" block
```

## 实现细节

### 文件位置

`src/parser/statement.rs`

### parse_var_decl()

```rust
fn parse_var_decl(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'var'
    let name = self.expect_identifier("expected variable name after 'var'")?;
    self.expect(TokenKind::Equal, "expected '=' after variable name")?;
    let initializer = self.parse_expression()?;
    self.consume_newline();
    Ok(Stmt::VarDecl { name, initializer })
}
```

### parse_const_decl()

```rust
fn parse_const_decl(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'const'
    let name = self.expect_identifier("expected constant name after 'const'")?;
    self.expect(TokenKind::Equal, "expected '=' after constant name")?;
    let initializer = self.parse_expression()?;
    self.consume_newline();
    Ok(Stmt::ConstDecl { name, initializer })
}
```

### parse_expr_or_assignment()

处理裸赋值、短声明、表达式语句、多目标赋值：

```rust
fn parse_expr_or_assignment(&mut self) -> Result<Stmt> {
    let expr = self.parse_expression()?;

    // 多目标赋值：a, b = 1, 2  （03-syntax.md:140-151）
    // 首个表达式后跟逗号即进入多目标路径。
    // 注意：parse_expression 不消费逗号（逗号不是二元运算符），
    // 因此 expr 是第一个 target，逗号仍在流中。
    if self.check(&TokenKind::Comma) {
        return self.parse_multi_assign(expr);
    }

    // 短声明：IDENTIFIER := expression
    if self.check(&TokenKind::ColonEqual) {
        self.advance();
        let value = self.parse_expression()?;
        self.consume_newline();
        if let Expr::Identifier(name) = expr {
            return Ok(Stmt::ShortVarDecl { name, initializer: value });
        }
        return Err(MspError::ParseError {
            line: self.previous().span.start.line,
            column: self.previous().span.start.column,
            message: "invalid target for :=".into(),
        });
    }

    // 赋值表达式（含复合赋值）：由 parse_assignment 产出 Expr::Assign
    if let Expr::Assign { target, op, value } = expr {
        // lvalue 校验：target 必须是标识符、属性访问或下标访问
        // （03-syntax.md:128-131）
        if !Self::is_valid_lvalue(&target) {
            let tok = self.previous();
            return Err(MspError::ParseError {
                line: tok.span.start.line,
                column: tok.span.start.column,
                message: "invalid assignment target".into(),
            });
        }
        self.consume_newline();
        if op == AssignOp::Assign {
            if let Expr::Identifier(name) = *target {
                return Ok(Stmt::VarDecl { name, initializer: *value });
            }
        }
        return Ok(Stmt::Assign {
            target: *target,
            op,
            value: *value,
        });
    }

    self.consume_newline();
    Ok(Stmt::ExprStmt { expr })
}

/// 多目标赋值解析：expr 为已解析的第一个 target。
fn parse_multi_assign(&mut self, first_target: Expr) -> Result<Stmt> {
    let mut targets = vec![first_target];
    while self.match_token(&[TokenKind::Comma]) {
        targets.push(self.parse_expression()?);
    }
    self.expect(TokenKind::Equal, "expected '=' in multi-assignment")?;

    let mut values = vec![self.parse_expression()?];
    while self.match_token(&[TokenKind::Comma]) {
        values.push(self.parse_expression()?);
    }
    self.consume_newline();

    // 校验所有 target 均为合法 lvalue
    for t in &targets {
        if !Self::is_valid_lvalue(t) {
            let tok = self.previous();
            return Err(MspError::ParseError {
                line: tok.span.start.line,
                column: tok.span.start.column,
                message: "invalid assignment target in multi-assignment".into(),
            });
        }
    }

    Ok(Stmt::Assign {
        target: Expr::TupleLiteral { elements: targets },
        op: AssignOp::Assign,
        value: Expr::TupleLiteral { elements: values },
    })
}

/// 判断表达式是否为合法赋值目标（03-syntax.md:128-131）。
fn is_valid_lvalue(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Identifier(_) | Expr::Dot { .. } | Expr::Index { .. }
    )
}
```

注意：
- 需要区分 `Expr::Assign` 和 `Stmt::Assign`。在 `parse_expression()` 中赋值表达式已经被构建为 `Expr::Assign`，这里需要将其转换为对应的 `Stmt`。
- 多目标赋值 `a, b = 1, 2` 在 AST 中表示为 `Stmt::Assign`，target 与 value 均为 `Expr::TupleLiteral`。编译期需校验两侧元素数量匹配（`03-syntax.md:140`，抛出 `ValueError`）。
- 裸 `=` 赋值（`x = 5`）当前归一化为 `Stmt::VarDecl`，与显式 `var x = 5` 同构。两者语义差异（`var`/`:=` 强制新建绑定 vs `=` 先查再建，见 `03-syntax.md:59-60`）需在编译期通过作用域查找顺序处理。

### parse_if()

```rust
fn parse_if(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'if'
    let condition = self.parse_expression()?;
    let then_block = self.parse_block()?;
    self.skip_newlines();

    let mut elif_clauses = Vec::new();
    while self.match_token(&[TokenKind::Elif]) {
        let cond = self.parse_expression()?;
        let block = self.parse_block()?;
        self.skip_newlines();
        elif_clauses.push((cond, block));
    }

    let mut else_block = None;
    if self.match_token(&[TokenKind::Else]) {
        else_block = Some(self.parse_block()?);
        self.skip_newlines();
    }

    Ok(Stmt::If {
        condition,
        then_block,
        elif_clauses,
        else_block,
    })
}
```

### parse_while()

```rust
fn parse_while(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'while'
    let condition = self.parse_expression()?;
    let body = self.parse_block()?;
    Ok(Stmt::While { condition, body })
}
```

### parse_for()

```rust
fn parse_for(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'for'
    let first_var = self.expect_identifier("expected variable name after 'for'")?;

    let (variable, second_variable) = if self.match_token(&[TokenKind::Comma]) {
        let second = self.expect_identifier("expected second variable name")?;
        (first_var, Some(second))
    } else {
        (first_var, None)
    };

    self.expect(TokenKind::In, "expected 'in' after for variable")?;
    let iterable = self.parse_expression()?;
    let body = self.parse_block()?;

    Ok(Stmt::ForIn {
        variable,
        second_variable,
        iterable,
        body,
    })
}
```

### parse_return()

```rust
fn parse_return(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'return'
    let mut values = Vec::new();

    if !self.check(&TokenKind::Newline) && !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
        values.push(self.parse_expression()?);
        while self.match_token(&[TokenKind::Comma]) {
            values.push(self.parse_expression()?);
        }
    }

    self.consume_newline();
    Ok(Stmt::Return { values })
}
```

### parse_fn_decl()

```rust
fn parse_fn_decl(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'fn'
    let name = self.expect_identifier("expected function name")?;
    self.expect(TokenKind::LeftParen, "expected '(' after function name")?;
    let params = self.parse_param_list()?;
    self.expect(TokenKind::RightParen, "expected ')' after parameters")?;
    let body = self.parse_block()?;

    Ok(Stmt::FnDecl { name, params, body, is_async: false })
}
```

> **async fn 推迟**：`is_async` 当前硬编码为 `false`。`async fn name() { }` 声明的解析推迟至 Phase 7（并发，task 53-55）。届时需在 `parse_statement` 增加 `TokenKind::Async` 分支：消费 `async` 后调用 `parse_fn_decl`，并将结果 `is_async` 置为 `true`。

### parse_param_list()

```rust
fn parse_param_list(&mut self) -> Result<Vec<Param>> {
    let mut params = Vec::new();
    if self.check(&TokenKind::RightParen) {
        return Ok(params);
    }

    loop {
        if self.match_token(&[TokenKind::Star]) {
            let name = self.expect_identifier("expected parameter name after '*'")?;
            params.push(Param {
                name,
                default: None,
                is_variadic: true,
            });
        } else {
            let name = self.parse_param_name()?;
            let default = if self.match_token(&[TokenKind::Equal]) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            params.push(Param {
                name,
                default,
                is_variadic: false,
            });
        }

        if !self.match_token(&[TokenKind::Comma]) {
            break;
        }
    }

    Ok(params)
}

fn parse_param_name(&mut self) -> Result<String> {
    let tok = self.peek();
    match &tok.kind {
        TokenKind::Identifier(name) => {
            let name = name.clone();
            self.advance();
            Ok(name)
        }
        TokenKind::Zelf => {
            self.advance();
            Ok("self".to_string())
        }
        _ => Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: "expected parameter name".into(),
        }),
    }
}
```

### parse_import() / parse_from_import()

> **归属 task 15**，task 13 不实现。参见 `tasks/README.md:23`。

### 辅助方法

```rust
fn expect_identifier(&mut self, msg: &str) -> Result<String> {
    let tok = self.peek();
    match &tok.kind {
        TokenKind::Identifier(name) => {
            let name = name.clone();
            self.advance();
            Ok(name)
        }
        _ => Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: msg.into(),
        }),
    }
}
```

### parse_fn_or_expr()

区分 `fn name(...)` 函数声明和 `fn(...)` 匿名函数。

> **Task 边界说明**：task 13 拥有 `parse_fn_or_expr`（语句分发）与 `parse_fn_decl`（命名函数声明）。
> task 14 仅拥有 `parse_fn_literal`（匿名函数表达式，当前为 expression.rs stub）。
> 实施时需将 `src/parser/mod.rs` 中 `parse_fn_or_expr` stub 注释从"由 task 14 替换"改为"由 task 13 替换"。
>
> `is_fn_literal` 已由 task 12 在 `src/parser/expression.rs:548` 实现，task 13 直接复用，不可重复定义。

```rust
fn parse_fn_or_expr(&mut self) -> Result<Stmt> {
    if self.is_fn_literal() {
        let expr = self.parse_expression()?;
        self.consume_newline();
        Ok(Stmt::ExprStmt { expr })
    } else {
        self.parse_fn_decl()
    }
}
```

### parse_nonlocal()

参照 [03-syntax](../03-syntax.md) § nonlocal 声明：

```rust
fn parse_nonlocal(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'nonlocal'
    let mut names = vec![self.expect_identifier("expected identifier after 'nonlocal'")?];
    while self.match_token(&[TokenKind::Comma]) {
        names.push(self.expect_identifier("expected identifier after ','")?);
    }
    self.consume_newline();
    Ok(Stmt::Nonlocal { names })
}
```

在 `parse_statement()` 分发中添加 `TokenKind::Nonlocal` 分支调用 `parse_nonlocal()`。

### parse_global()

参照 [03-syntax](../03-syntax.md) § global 声明：

```rust
fn parse_global(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'global'
    let mut names = vec![self.expect_identifier("expected identifier after 'global'")?];
    while self.match_token(&[TokenKind::Comma]) {
        names.push(self.expect_identifier("expected identifier after ','")?);
    }
    self.consume_newline();
    Ok(Stmt::Global { names })
}
```

在 `parse_statement()` 分发中添加 `TokenKind::Global` 分支调用 `parse_global()`。

## 验证标准

1. `var`, `:=`, `=` 三种声明方式正确解析
2. `const` 声明正确解析
3. 复合赋值运算符正确解析（`+=` `-=` `*=` 等）
4. `if/elif/else` 正确解析多个分支
5. `while` 和 `for..in`（单变量和双变量）正确解析
6. `break`, `continue`, `return` 正确解析
7. 函数声明（含默认参数和可变参数）正确解析
8. 多目标赋值正确解析（`a, b = 1, 2`，AST 表示为 TupleLiteral 赋值）
9. 非法赋值目标被拒绝（`1 + 2 = 3` 返回 ParseError）
10. `nonlocal` 声明正确解析（含多个变量名）
11. `global` 声明正确解析（含多个变量名）

> **注**：`import` / `from...import` 解析归属于 task 15，不在本 task 验证范围内。

## 测试用例

```ms
const PI = 3.14159
var radius = 10
area = PI * radius * radius

if area > 100 {
    print("big circle")
} elif area > 50 {
    print("medium circle")
} else {
    print("small circle")
}

for i in range(5) {
    if i == 3 {
        continue
    }
    print(i)
}
```

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Result<Program> {
        let tokens = Lexer::new(source).tokenize_all()?;
        Parser::new(tokens).parse()
    }

    #[test]
    fn test_const_and_var() {
        let prog = parse("const PI = 3.14159\nvar radius = 10\narea = PI * radius * radius\n").unwrap();
        assert_eq!(prog.statements.len(), 3);
        assert!(matches!(&prog.statements[0], Stmt::ConstDecl { name, .. } if name == "PI"));
        assert!(matches!(&prog.statements[1], Stmt::VarDecl { name, .. } if name == "radius"));
    }

    #[test]
    fn test_if_elif_else() {
        let prog = parse("if x > 0 {\n    print(\"pos\")\n} elif x == 0 {\n    print(\"zero\")\n} else {\n    print(\"neg\")\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::If { elif_clauses, else_block, .. } => {
                assert_eq!(elif_clauses.len(), 1);
                assert!(else_block.is_some());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn test_for_in() {
        let prog = parse("for i in range(5) {\n    print(i)\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn { variable, second_variable, .. } => {
                assert_eq!(variable, "i");
                assert!(second_variable.is_none());
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_for_in_dual_var() {
        let prog = parse("for k, v in dict.items() {\n    print(k)\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn { variable, second_variable, .. } => {
                assert_eq!(variable, "k");
                assert_eq!(second_variable.as_deref(), Some("v"));
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_fn_decl() {
        let prog = parse("fn add(a, b) {\n    return a + b\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { name, params, body, .. } => {
                assert_eq!(name, "add");
                assert_eq!(params.len(), 2);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_fn_with_defaults() {
        let prog = parse("fn greet(name, prefix = \"Hello\") {\n    return prefix + name\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { params, .. } => {
                assert_eq!(params.len(), 2);
                assert!(params[1].default.is_some());
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_break_continue() {
        let prog = parse("for i in x {\n    break\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn { body, .. } => {
                assert!(matches!(body[0], Stmt::Break));
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_multi_return() {
        let prog = parse("fn f() {\n    return 1, 2, 3\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                match &body[0] {
                    Stmt::Return { values } => assert_eq!(values.len(), 3),
                    _ => panic!("expected return"),
                }
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_compound_assign() {
        let prog = parse("x += 5\ny *= 3\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        assert!(matches!(
            &prog.statements[0],
            Stmt::Assign { op: AssignOp::PlusAssign, .. }
        ));
    }

    #[test]
    fn test_multi_assign() {
        let prog = parse("a, b = 1, 2\n").unwrap();
        match &prog.statements[0] {
            Stmt::Assign { target, op: AssignOp::Assign, value } => {
                assert!(matches!(target, Expr::TupleLiteral { elements } if elements.len() == 2));
                assert!(matches!(value, Expr::TupleLiteral { elements } if elements.len() == 2));
            }
            _ => panic!("expected multi-assign"),
        }
    }

    #[test]
    fn test_invalid_lvalue() {
        // 1 + 2 = 3 不是合法赋值，应在解析期拒绝
        let result = parse("1 + 2 = 3\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_nonlocal() {
        let prog = parse("fn f() {\n    nonlocal x, y\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::Nonlocal { names } if names.len() == 2));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_global() {
        let prog = parse("fn f() {\n    global counter\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::Global { names } if names.len() == 1));
            }
            _ => panic!("expected fn"),
        }
    }
}
```
