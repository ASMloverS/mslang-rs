# 语法分析器核心框架

## 所属阶段
Phase 1.5a - 基础设施

## 前置任务
10-ast-statement-nodes, 08-lexer-statement-termination

## 目标
实现 Parser 核心框架，包括 token 流遍历、语句分发、块解析、错误恢复机制。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 程序结构：

```
program = statement*
```

参照 [12-implementation-plan](../12-implementation-plan.md) § 1.5 语法分析器：

- Parser 框架（递归下降）
- 块解析（花括号）
- 错误恢复（panic mode）和错误报告

## 实现细节

### 文件位置

`src/parser/mod.rs`

### Parser 结构体

```rust
pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}
```

### 核心方法

#### new()

```rust
pub fn new(tokens: Vec<Token>) -> Parser {
    Parser { tokens, current: 0 }
}
```

#### advance() / peek() / previous() / check() / match_token() / expect()

```rust
fn advance(&mut self) -> &Token {
    if !self.is_at_end() {
        self.current += 1;
    }
    self.previous()
}

fn peek(&self) -> &Token {
    &self.tokens[self.current]
}

fn previous(&self) -> &Token {
    &self.tokens[self.current.saturating_sub(1)]
}

fn is_at_end(&self) -> bool {
    self.peek().kind == TokenKind::Eof
}

fn check(&self, kind: &TokenKind) -> bool {
    if self.is_at_end() { return false; }
    &self.peek().kind == kind
}

fn match_token(&mut self, kinds: &[TokenKind]) -> bool {
    for kind in kinds {
        if self.check(kind) {
            self.advance();
            return true;
        }
    }
    false
}

fn expect(&mut self, kind: TokenKind, message: &str) -> Result<&Token> {
    if self.check(&kind) {
        return Ok(self.advance());
    }
    let tok = self.peek();
    Err(MspError::ParseError {
        line: tok.span.start.line,
        column: tok.span.start.column,
        message: message.into(),
    })
}
```

#### skip_newlines()

跳过连续的 `Newline` token：

```rust
fn skip_newlines(&mut self) {
    while self.check(&TokenKind::Newline) {
        self.advance();
    }
}
```

### 入口方法

#### parse()

```rust
pub fn parse(mut self) -> Result<Program> {
    let mut statements = Vec::new();
    self.skip_newlines();

    while !self.is_at_end() {
        self.skip_newlines();
        if self.is_at_end() { break; }
        statements.push(self.parse_statement()?);
        self.skip_newlines();
    }

    Ok(Program { statements })
}
```

### parse_statement() — 语句分发

```rust
fn parse_statement(&mut self) -> Result<Stmt> {
    self.skip_newlines();

    if self.check(&TokenKind::Var) {
        self.parse_var_decl()
    } else if self.check(&TokenKind::Const) {
        self.parse_const_decl()
    } else if self.check(&TokenKind::Fn) {
        self.parse_fn_or_expr()
    } else if self.check(&TokenKind::If) {
        self.parse_if()
    } else if self.check(&TokenKind::While) {
        self.parse_while()
    } else if self.check(&TokenKind::For) {
        self.parse_for()
    } else if self.check(&TokenKind::Break) {
        self.advance();
        self.consume_newline();
        Ok(Stmt::Break)
    } else if self.check(&TokenKind::Continue) {
        self.advance();
        self.consume_newline();
        Ok(Stmt::Continue)
    } else if self.check(&TokenKind::Return) {
        self.parse_return()
    } else if self.check(&TokenKind::Import) {
        self.parse_import()
    } else if self.check(&TokenKind::From) {
        self.parse_from_import()
    } else if self.check(&TokenKind::Class) {
        self.parse_class()
    } else if self.check(&TokenKind::Defer) {
        self.parse_defer()
    } else if self.check(&TokenKind::Try) {
        self.parse_try()
    } else if self.check(&TokenKind::With) {
        self.parse_with()
    } else if self.check(&TokenKind::Throw) {
        self.parse_throw()
    } else {
        self.parse_expr_or_assignment()
    }
}
```

### parse_block()

解析 `{ statement* }` 块：

```rust
fn parse_block(&mut self) -> Result<Vec<Stmt>> {
    self.expect(TokenKind::LeftBrace, "expected '{'")?;
    self.skip_newlines();

    let mut statements = Vec::new();
    while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
        self.skip_newlines();
        if self.check(&TokenKind::RightBrace) { break; }
        statements.push(self.parse_statement()?);
        self.skip_newlines();
    }

    self.expect(TokenKind::RightBrace, "expected '}'")?;
    Ok(statements)
}
```

### consume_newline()

在语句结束后消费换行符：

```rust
fn consume_newline(&mut self) {
    if self.check(&TokenKind::Newline) {
        self.advance();
    }
}
```

### 错误恢复 — Panic Mode

当遇到语法错误时，跳过 token 直到找到同步点（synchronization point）：

```rust
fn synchronize(&mut self) {
    self.advance();

    while !self.is_at_end() {
        if self.previous().kind == TokenKind::Newline {
            return;
        }

        match self.peek().kind {
            TokenKind::Var | TokenKind::Const | TokenKind::Fn
            | TokenKind::If | TokenKind::While | TokenKind::For
            | TokenKind::Class | TokenKind::Return | TokenKind::Import
            | TokenKind::From | TokenKind::Nonlocal | TokenKind::Global
            | TokenKind::Async
            | TokenKind::Try | TokenKind::With | TokenKind::Defer
            | TokenKind::Break | TokenKind::Continue | TokenKind::Throw
            => return,
            _ => {}
        }

        self.advance();
    }
}
```

上层可在 `parse_statement()` 中捕获错误后调用 `synchronize()`：

```rust
fn parse_statement_safe(&mut self) -> Option<Stmt> {
    match self.parse_statement() {
        Ok(stmt) => Some(stmt),
        Err(e) => {
            eprintln!("{}", e);
            self.synchronize();
            None
        }
    }
}
```

> **错误恢复策略**：入口 `parse()` 采用**严格模式**——通过 `parse_statement()?` 在首个语法错误即返回 `Err`，适合一次性编译/执行场景。`synchronize()` 与 `parse_statement_safe()` 作为**原语**提供，供 REPL、IDE、LSP 等需要继续解析以收集多个错误的上下文调用（例如把 `parse()` 主循环改为调用 `parse_statement_safe()` 并聚合错误）。本 task 只提供原语，不在 `parse()` 中强制启用 panic mode。

### 占位方法（stubs）

`parse_statement()` 分发引用的子解析器（`parse_var_decl`、`parse_const_decl`、`parse_fn_or_expr`、`parse_if`、`parse_while`、`parse_for`、`parse_return`、`parse_import`、`parse_from_import`、`parse_class`、`parse_defer`、`parse_try`、`parse_with`、`parse_throw`、`parse_expr_or_assignment`）**不在本 task 范围内**——它们由 task 12（`parse_expression`）、task 13（变量/控制流/import/赋值）、task 14（匿名函数）、task 15（class/defer/try/with/throw/async）实现。

为保证 task 11 可**独立编译**（遵循 task 03 `read_string`、task 09 `Stmt::Placeholder` 的前置占位模式），本 task 需为上述每个方法提供 stub，由后续 task 替换：

```rust
// 占位：由 task 13 替换
fn parse_var_decl(&mut self) -> Result<Stmt> {
    self.unimplemented("parse_var_decl")
}
// ... parse_const_decl / parse_fn_or_expr / parse_if / parse_while / parse_for /
//     parse_return / parse_import / parse_from_import / parse_expr_or_assignment
//     同样返回 self.unimplemented(...)，分别由 task 13/14 替换。

// 占位：由 task 15 替换
fn parse_class(&mut self) -> Result<Stmt> {
    self.unimplemented("parse_class")
}
// ... parse_defer / parse_try / parse_with / parse_throw 同上。

// 占位：由 task 12 替换（被 parse_expr_or_assignment 调用）
fn parse_expression(&mut self) -> Result<Expr> {
    self.unimplemented_expr("parse_expression")
}

fn unimplemented(&mut self, name: &str) -> Result<Stmt> {
    let tok = self.peek();
    Err(MspError::ParseError {
        line: tok.span.start.line,
        column: tok.span.start.column,
        message: format!("{} not yet implemented", name),
    })
}

fn unimplemented_expr(&mut self, name: &str) -> Result<Expr> {
    let tok = self.peek();
    Err(MspError::ParseError {
        line: tok.span.start.line,
        column: tok.span.start.column,
        message: format!("{} not yet implemented", name),
    })
}
```

> **测试归属**：因 stub 在 task 11 阶段对所有非空输入返回 `ParseError`，依赖实际解析的测试（`test_simple_program`、`test_block`、`test_newline_handling`）**在 task 13 完成后方可通过**；本 task 验证范围仅覆盖框架原语（空程序、错误路径不 panic、块边界）。这些解析测试的断言保留于此作为 task 13 的回归基线。

## 验证标准

1. `cargo build` 编译通过（含上述 stub 方法）
2. 框架原语可用：`parse("")` 返回空 `Program`，错误输入返回 `ParseError` 且不 panic
3. `parse_block()` 正确解析 `{ }` 边界（缺 `}` 时报错）
4. `skip_newlines` / `consume_newline` / `synchronize` 行为正确
5. 实际语句/表达式解析（`test_simple_program`、`test_block`、`test_newline_handling`）由 task 12-15 接管 stub 后通过

## 测试用例

```ms
x = 10
y = 20
print(x + y)
```

预期 AST（变体以 task 13 为准；裸 `=` + 标识符 target 由 task 13 转为 `Stmt::VarDecl`）：

```
Program {
    statements: [
        VarDecl(x, Literal(10)),
        VarDecl(y, Literal(20)),
        ExprStmt(Call(print, [Binary(x, +, y)])),
    ]
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
        let parser = Parser::new(tokens);
        parser.parse()
    }

    #[test]
    fn test_simple_program() {
        let prog = parse("x = 10\ny = 20\nprint(x + y)\n").unwrap();
        assert_eq!(prog.statements.len(), 3);
    }

    #[test]
    fn test_block() {
        let prog = parse("if true {\n    x = 1\n}\n").unwrap();
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0] {
            Stmt::If { then_block, .. } => {
                assert_eq!(then_block.len(), 1);
            }
            _ => panic!("expected if statement"),
        }
    }

    #[test]
    fn test_empty_program() {
        let prog = parse("").unwrap();
        assert!(prog.statements.is_empty());
    }

    #[test]
    fn test_parse_error() {
        let result = parse("if {\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_newline_handling() {
        let prog = parse("x = 1\n\n\ny = 2\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
    }
}
```
