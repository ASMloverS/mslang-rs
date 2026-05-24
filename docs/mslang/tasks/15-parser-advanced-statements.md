# 高级语句解析（defer/try/with/class/import）

## 所属阶段
Phase 1.5e - 基础设施

## 前置任务
13-parser-statements

## 目标
实现高级语句解析：defer、try/except/finally、throw、with、class 定义、async 函数、go 表达式、yield 表达式。

## 设计规格

### defer 语句

参照 [05-control-flow](../05-control-flow.md) § defer：

```
defer_stmt = "defer" expression
```

### try / except / finally

参照 [05-control-flow](../05-control-flow.md) § try / except / finally：

```
try_stmt = "try" block except_clause* finally_clause?
except_clause = "except" type_spec? ("as" IDENTIFIER)? block
type_spec = IDENTIFIER ("." IDENTIFIER)*
finally_clause = "finally" block
```

### throw 语句

```
throw_stmt = "throw" expression
```

### with 语句

参照 [05-control-flow](../05-control-flow.md) § with 语句：

```
with_stmt = "with" expression ("as" IDENTIFIER)? block
```

### class 定义

参照 [06-oop](../06-oop.md) § 类定义：

```
class_def = "class" IDENTIFIER ("<" IDENTIFIER)? "{" class_body "}"
class_body = (method_def | class_var)*
method_def = "fn" IDENTIFIER "(" param_list? ")" block
class_var = "var"? IDENTIFIER "=" expression
```

### async 函数

参照 [08-concurrency](../08-concurrency.md)：

```
async_fn = "async" "fn" IDENTIFIER "(" param_list? ")" block
```

### go 表达式

```
go_expr = "go" expression
```

### yield 表达式

参照 [07-advanced](../07-advanced.md) § yield：

```
yield_expr = "yield" expression?
           | "yield" "from" expression
```

## 实现细节

### 文件位置

`src/parser/statement.rs` 中添加相应解析方法。

### parse_defer()

```rust
fn parse_defer(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'defer'
    let expr = self.parse_expression()?;
    self.consume_newline();
    Ok(Stmt::Defer { expr })
}
```

### parse_try()

```rust
fn parse_try(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'try'
    let try_block = self.parse_block()?;
    self.skip_newlines();

    let mut except_clauses = Vec::new();
    while self.match_token(&[TokenKind::Except]) {
        let type_name = if self.check(&TokenKind::Identifier(String::new())) {
            match &self.peek().kind {
                TokenKind::Identifier(name) => {
                    let mut path = name.clone();
                    self.advance();
                    while self.match_token(&[TokenKind::Dot]) {
                        if let TokenKind::Identifier(n) = &self.peek().kind {
                            path.push('.');
                            path.push_str(n);
                            self.advance();
                        }
                    }
                    Some(path)
                }
                _ => None,
            }
        } else {
            None
        };

        let alias = if self.match_token(&[TokenKind::As]) {
            Some(self.expect_identifier("expected variable name after 'as'")?)
        } else {
            None
        };

        let body = self.parse_block()?;
        self.skip_newlines();

        except_clauses.push(ExceptClause {
            type_name,
            alias,
            body,
        });
    }

    let finally_block = if self.match_token(&[TokenKind::Finally]) {
        Some(self.parse_block()?)
    } else {
        None
    };

    Ok(Stmt::Try {
        try_block,
        except_clauses,
        finally_block,
    })
}
```

### parse_throw()

```rust
fn parse_throw(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'throw'

    // 支持 bare throw（重新抛出当前异常）
    if self.check(&TokenKind::Newline) || self.check(&TokenKind::RightBrace) {
        self.consume_newline();
        return Ok(Stmt::Throw {
            expr: Expr::Literal(Literal::Nil), // bare throw 标记
        });
    }

    let expr = self.parse_expression()?;
    self.consume_newline();
    Ok(Stmt::Throw { expr })
}
```

### parse_with()

```rust
fn parse_with(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'with'
    let expression = self.parse_expression()?;

    let alias = if self.match_token(&[TokenKind::As]) {
        Some(self.expect_identifier("expected variable name after 'as'")?)
    } else {
        None
    };

    let body = self.parse_block()?;
    Ok(Stmt::With { expression, alias, body })
}
```

### parse_class()

```rust
fn parse_class(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'class'
    let name = self.expect_identifier("expected class name")?;

    let parent = if self.match_token(&[TokenKind::Less]) {
        Some(self.expect_identifier("expected parent class name")?)
    } else {
        None
    };

    self.expect(TokenKind::LeftBrace, "expected '{' after class name")?;
    self.skip_newlines();

    let mut methods = Vec::new();
    let mut class_vars = Vec::new();

    while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
        self.skip_newlines();
        if self.check(&TokenKind::RightBrace) { break; }

        if self.check(&TokenKind::Fn) {
            methods.push(self.parse_class_method()?);
        } else {
            let is_var = self.match_token(&[TokenKind::Var]);
            let var_name = self.expect_identifier("expected class variable or method name")?;
            self.expect(TokenKind::Equal, "expected '=' in class variable")?;
            let value = self.parse_expression()?;
            class_vars.push((var_name, value));
        }
        self.skip_newlines();
    }

    self.expect(TokenKind::RightBrace, "expected '}'")?;
    Ok(Stmt::ClassDecl {
        name,
        parent,
        methods,
        class_vars,
    })
}
```

### parse_class_method()

```rust
fn parse_class_method(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'fn'
    let name = self.expect_identifier("expected method name")?;
    self.expect(TokenKind::LeftParen, "expected '(' after method name")?;
    let params = self.parse_param_list()?;
    self.expect(TokenKind::RightParen, "expected ')'")?;
    let body = self.parse_block()?;
    Ok(Stmt::FnDecl { name, params, body })
}
```

### parse_yield_expr()

```rust
fn parse_yield_expr(&mut self) -> Result<Expr> {
    self.advance(); // consume 'yield'

    if self.match_token(&[TokenKind::From]) {
        let iterable = self.parse_expression()?;
        return Ok(Expr::YieldFrom {
            iterable: Box::new(iterable),
        });
    }

    if self.check(&TokenKind::Newline) || self.check(&TokenKind::RightBrace) {
        return Ok(Expr::Yield { value: None });
    }

    let value = self.parse_expression()?;
    Ok(Expr::Yield { value: Some(Box::new(value)) })
}
```

### async 函数和 go 表达式

在 `parse_statement()` 分发中添加：

> **Phase 1 范围**：`async fn` 在本阶段仅做语法识别和 AST 构建。编译器在 AST 节点上标记 `is_async`，但运行时语义（Future 创建、事件循环调度）推迟到 Phase 7（Task 53）实现。Phase 1 编译器遇到 `async fn` 时正常编译函数体字节码，仅在 Function 对象上设置 `is_async = true` 标记。

```rust
// async fn ...
if self.check(&TokenKind::Async) {
    return self.parse_async_fn();
}
```

```rust
fn parse_async_fn(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'async'
    let fn_decl = self.parse_fn_decl()?;
    // fn_decl 已是 Stmt::FnDecl，async 标记可在 AST 中添加 is_async 字段
    // 或保持 FnDecl 不变，在编译阶段根据上下文处理
    Ok(fn_decl)
}
```

对于 `go` 表达式，在 `parse_primary()` 中添加：

```rust
TokenKind::Go => {
    self.advance();
    let expr = self.parse_postfix()?;
    // go 表达式包装为 Expr::Go（可在 Expr 枚举中添加 Go 变体）
    // 或作为 ExprStmt(Call(...)) 处理
    Ok(expr)
}
```

### Stmt 扩展

需要在 `Stmt` 枚举中确认 `ClassDecl` 中 `methods` 字段的类型为 `Vec<Stmt>`，其中每个元素为 `Stmt::FnDecl`。

## 验证标准

1. defer 语句正确解析
2. try/except/finally 各种组合正确解析
3. throw 语句正确解析（含 bare throw）
4. with 语句（含/不含 as）正确解析
5. class 定义（含继承、方法、类属性）正确解析
6. async fn 正确解析
7. yield 和 yield from 正确解析

## 测试用例

```ms
class Animal {
    kingdom = "Animalia"
    
    fn __init__(self, name) {
        self.name = name
    }
    
    fn speak(self) {
        return self.name + " speaks"
    }
}

class Dog < Animal {
    fn speak(self) {
        return self.name + " barks"
    }
}

fn safe_divide(a, b) {
    defer print("done")
    if b == 0 {
        throw ValueError("division by zero")
    }
    return a / b
}

try {
    result = safe_divide(10, 0)
} except ValueError as e {
    print("error: " + e.message)
} finally {
    print("cleanup")
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
    fn test_class_basic() {
        let prog = parse("class Animal {\n    kingdom = \"Animalia\"\n    fn speak(self) {\n        return self.name\n    }\n}\n").unwrap();
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0] {
            Stmt::ClassDecl { name, parent, methods, class_vars } => {
                assert_eq!(name, "Animal");
                assert!(parent.is_none());
                assert_eq!(methods.len(), 1);
                assert_eq!(class_vars.len(), 1);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn test_class_inheritance() {
        let prog = parse("class Dog < Animal {\n    fn speak(self) {\n        return \"bark\"\n    }\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ClassDecl { name, parent, methods, .. } => {
                assert_eq!(name, "Dog");
                assert_eq!(parent.as_deref(), Some("Animal"));
                assert_eq!(methods.len(), 1);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn test_defer() {
        let prog = parse("fn f() {\n    defer print(\"done\")\n    x = 1\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(body[0], Stmt::Defer { .. }));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_try_except_finally() {
        let prog = parse("try {\n    x()\n} except ValueError as e {\n    print(e)\n} finally {\n    cleanup()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::Try { try_block, except_clauses, finally_block } => {
                assert_eq!(try_block.len(), 1);
                assert_eq!(except_clauses.len(), 1);
                assert_eq!(except_clauses[0].type_name.as_deref(), Some("ValueError"));
                assert_eq!(except_clauses[0].alias.as_deref(), Some("e"));
                assert!(finally_block.is_some());
            }
            _ => panic!("expected try"),
        }
    }

    #[test]
    fn test_try_catch_all() {
        let prog = parse("try {\n    x()\n} except {\n    print(\"error\")\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::Try { except_clauses, .. } => {
                assert_eq!(except_clauses.len(), 1);
                assert!(except_clauses[0].type_name.is_none());
                assert!(except_clauses[0].alias.is_none());
            }
            _ => panic!("expected try"),
        }
    }

    #[test]
    fn test_throw() {
        let prog = parse("throw ValueError(\"bad\")\n").unwrap();
        match &prog.statements[0] {
            Stmt::Throw { .. } => {}
            _ => panic!("expected throw"),
        }
    }

    #[test]
    fn test_with() {
        let prog = parse("with open(\"file.txt\") as f {\n    f.read()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::With { expression, alias, body } => {
                assert!(alias.as_deref() == Some("f"));
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected with"),
        }
    }

    #[test]
    fn test_with_no_alias() {
        let prog = parse("with lock.acquire() {\n    work()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::With { alias, .. } => {
                assert!(alias.is_none());
            }
            _ => panic!("expected with"),
        }
    }

    #[test]
    fn test_yield() {
        let prog = parse("fn gen() {\n    yield 1\n    yield 2\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert_eq!(body.len(), 2);
                assert!(matches!(&body[0], Stmt::ExprStmt { expr: Expr::Yield { value: Some(_) } }));
            }
            _ => panic!("expected fn"),
        }
    }
}
```
