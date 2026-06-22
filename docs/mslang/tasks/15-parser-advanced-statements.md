# 高级语句解析（defer/try/with/class/import）

## 所属阶段
Phase 1.5e - 基础设施

## 前置任务
13-parser-statements

## 目标
实现高级语句解析：defer、try/except/finally、throw、with、class 定义、import 语句、async 函数、go 表达式、yield 表达式。

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

### import 语句

参照 [09-modules](../09-modules.md) § import 语法：

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_targets

module_path = IDENTIFIER ("." IDENTIFIER)*
import_targets = import_target ("," import_target)*
import_target = IDENTIFIER ("as" IDENTIFIER)?
```

`@std` 前缀（`09-modules.md:60-61`）：`import @std math` 强制加载标准库模块（跳过当前目录搜索）。`@` 后必须跟 `std` 标识符。

## 实现细节

### 文件位置

涉及三个文件：

- **`src/parser/statement.rs`** — 新增 `parse_defer`、`parse_try`、`parse_throw`、`parse_with`、`parse_class`、`parse_class_method`、`parse_async_fn`、`parse_import`、`parse_from_import`、`parse_module_path`。
- **`src/parser/expression.rs`** — 替换 `parse_yield_expr` stub（第 819-820 行）；替换 `parse_primary` 中 `TokenKind::Go` 分支（第 475-481 行，`parse_unary` → `parse_postfix`）。
- **`src/parser/mod.rs`** — 删除 7 个 stub（`parse_import`、`parse_from_import`、`parse_class`、`parse_defer`、`parse_try`、`parse_with`、`parse_throw`）；在 `parse_statement` 分发中添加 `TokenKind::Async` 分支。

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
        let type_name = if self.peek().is_identifier() {
            match &self.peek().kind {
                TokenKind::Identifier(name) => {
                    let mut path = vec![name.clone()];
                    self.advance();
                    while self.match_token(&[TokenKind::Dot]) {
                        if let TokenKind::Identifier(n) = &self.peek().kind {
                            path.push(n.clone());
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
        return Ok(Stmt::Throw { expr: None });
    }

    let expr = self.parse_expression()?;
    self.consume_newline();
    Ok(Stmt::Throw { expr: Some(expr) })
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
    Ok(Stmt::FnDecl { name, params, body, is_async: false })
}
```

### parse_import()

```rust
fn parse_import(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'import'
    let is_stdlib = self.match_token(&[TokenKind::At]);
    if is_stdlib {
        let std_tok = self.peek();
        match &std_tok.kind {
            TokenKind::Identifier(name) if name == "std" => {
                self.advance();
            }
            _ => {
                return Err(MspError::ParseError {
                    line: std_tok.span.start.line,
                    column: std_tok.span.start.column,
                    message: "expected 'std' after '@' in import".into(),
                });
            }
        }
    }
    let module_path = self.parse_module_path()?;
    let alias = if self.match_token(&[TokenKind::As]) {
        Some(self.expect_identifier("expected alias name")?)
    } else {
        None
    };
    self.consume_newline();
    Ok(Stmt::Import { module_path, alias, is_stdlib })
}
```

### parse_from_import()

```rust
fn parse_from_import(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'from'
    let is_stdlib = self.match_token(&[TokenKind::At]);
    if is_stdlib {
        let std_tok = self.peek();
        match &std_tok.kind {
            TokenKind::Identifier(name) if name == "std" => {
                self.advance();
            }
            _ => {
                return Err(MspError::ParseError {
                    line: std_tok.span.start.line,
                    column: std_tok.span.start.column,
                    message: "expected 'std' after '@' in from-import".into(),
                });
            }
        }
    }
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
    Ok(Stmt::FromImport { module_path, targets, is_stdlib })
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

### parse_yield_expr()

> **替换** `src/parser/expression.rs` 中的 `parse_yield_expr` stub（当前返回 `unimplemented_expr`）。此方法留在 expression.rs（被 `parse_primary` 调用）。
>
> **简化说明**：`from` 是关键字（`TokenKind::From`，由词法分析器 `keyword_table()` 映射），不可能作为标识符出现。`yield from_module.import_name` 中的 `from_module` 被词法分析器整体识别为 `Identifier("from_module")`，而非 `From` + 后续 token。因此 `Yield` 后跟 `From` token 时**始终**为 `yield from`（委托），无需消歧逻辑（`is_expression_start` / `backup`）。`07-advanced.md:185` 的消歧规则是针对 `from_module`（单个标识符）而非 `From` 关键字。

```rust
fn parse_yield_expr(&mut self) -> Result<Expr> {
    self.advance(); // consume 'yield'

    // yield from expr — From 是关键字，始终为委托语义
    if self.match_token(&[TokenKind::From]) {
        let iterable = self.parse_expression()?;
        return Ok(Expr::YieldFrom {
            iterable: Box::new(iterable),
        });
    }

    // bare yield
    if self.check(&TokenKind::Newline) || self.check(&TokenKind::RightBrace) || self.is_at_end() {
        return Ok(Expr::Yield { value: None });
    }

    // yield expr
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
    let mut fn_decl = self.parse_fn_decl()?;
    if let Stmt::FnDecl { is_async, .. } = &mut fn_decl {
        *is_async = true;
    }
    Ok(fn_decl)
}
```

对于 `go` 表达式，**替换** `expression.rs:475-481` 中 task 12 的 `Go` 分支（当前使用 `parse_unary`，改为 `parse_postfix` 以限制 `go` 后只能跟函数调用 / 匿名函数，不允许一元前缀运算符）：

```rust
TokenKind::Go => {
    self.advance();
    let expr = self.parse_postfix()?;
    Ok(Expr::Go { expr: Box::new(expr) })
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
8. `import` 和 `from...import` 正确解析（含 `@std` 前缀）

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
                assert_eq!(except_clauses[0].type_name.as_ref().unwrap(), &vec!["ValueError".to_string()]);
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

    #[test]
    fn test_yield_from() {
        let prog = parse("fn gen() {\n    yield from items\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::ExprStmt { expr: Expr::YieldFrom { .. } }));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_bare_yield() {
        let prog = parse("fn gen() {\n    yield\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::ExprStmt { expr: Expr::Yield { value: None } }));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_async_fn() {
        let prog = parse("async fn fetch(url) {\n    return url\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { name, is_async, .. } => {
                assert_eq!(name, "fetch");
                assert!(*is_async);
            }
            _ => panic!("expected async fn"),
        }
    }

    #[test]
    fn test_go_expr() {
        let prog = parse("go worker()\n").unwrap();
        match &prog.statements[0] {
            Stmt::ExprStmt { expr: Expr::Go { .. } } => {}
            _ => panic!("expected go expression"),
        }
    }

    #[test]
    fn test_class_var_with_var_keyword() {
        let prog = parse("class C {\n    var count = 0\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ClassDecl { class_vars, .. } => {
                assert_eq!(class_vars.len(), 1);
                assert_eq!(class_vars[0].0, "count");
            }
            _ => panic!("expected class"),
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
}
```
