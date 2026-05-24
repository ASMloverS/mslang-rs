# 语句解析

## 所属阶段
Phase 1.5c - 基础设施

## 前置任务
11-parser-core

## 目标
实现核心语句的解析：变量声明、常量声明、赋值、控制流、函数声明、import 语句。

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
assign_stmt = target ("=" | "+=" | ...) expression
target = IDENTIFIER | target "." IDENTIFIER | target "[" expression "]"
```

### 多目标赋值

```
multi_assign = target_list "=" expression_list
```

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

### import 语句

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_list
module_path = IDENTIFIER ("." IDENTIFIER)*
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

    if let Expr::Assign { target, op, value } = expr {
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
```

注意：需要区分 `Expr::Assign` 和 `Stmt::Assign`。在 `parse_expression()` 中赋值表达式已经被构建为 `Expr::Assign`，这里需要将其转换为对应的 `Stmt`。

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

    Ok(Stmt::FnDecl { name, params, body })
}
```

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
            let name = self.expect_identifier("expected parameter name")?;
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
```

### parse_import()

```rust
fn parse_import(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'import'
    let module_path = self.parse_module_path()?;
    let alias = if self.match_token(&[TokenKind::As]) {
        Some(self.expect_identifier("expected alias name")?)
    } else {
        None
    };
    self.consume_newline();
    Ok(Stmt::Import { module_path, alias })
}
```

### parse_from_import()

```rust
fn parse_from_import(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'from'
    let module_path = self.parse_module_path()?;
    self.expect(TokenKind::Import, "expected 'import' after module path")?;

    let mut targets = Vec::new();
    loop {
        let name = self.expect_identifier("expected import name")?;
        let alias = if self.match_token(&[TokenKind::As]) {
            Some(self.expect_identifier("expected alias name")?)
        } else {
            None
        };
        targets.push((name, alias));
        if !self.match_token(&[TokenKind::Comma]) {
            break;
        }
    }

    self.consume_newline();
    Ok(Stmt::FromImport { module_path, targets })
}
```

### parse_module_path()

```rust
fn parse_module_path(&mut self) -> Result<Vec<String>> {
    let mut path = vec![self.expect_identifier("expected module name")?];
    while self.match_token(&[TokenKind::Dot]) {
        path.push(self.expect_identifier("expected module name after '.'")?);
    }
    Ok(path)
}
```

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

区分 `fn name(...)` 函数声明和 `fn(...)` 匿名函数：

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

fn is_fn_literal(&self) -> bool {
    if !self.check(&TokenKind::Fn) { return false; }
    let next = self.tokens.get(self.current + 1);
    matches!(next.map(|t| &t.kind), Some(TokenKind::LeftParen))
}
```

## 验证标准

1. `var`, `:=`, `=` 三种声明方式正确解析
2. `const` 声明正确解析
3. 复合赋值运算符正确解析
4. `if/elif/else` 正确解析多个分支
5. `while` 和 `for..in`（单变量和双变量）正确解析
6. `break`, `continue`, `return` 正确解析
7. 函数声明（含默认参数和可变参数）正确解析
8. `import` 和 `from...import` 正确解析

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
            Stmt::FnDecl { name, params, body } => {
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
    fn test_import() {
        let prog = parse("import math\nimport os.path as pathutil\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
    }

    #[test]
    fn test_from_import() {
        let prog = parse("from os import path\nfrom io import open, print as log\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        match &prog.statements[1] {
            Stmt::FromImport { targets, .. } => {
                assert_eq!(targets.len(), 2);
            }
            _ => panic!("expected from import"),
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
}
```
