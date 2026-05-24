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
    &self.tokens[self.current - 1]
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

## 验证标准

1. `cargo build` 编译通过
2. 能解析简单程序（变量赋值 + 表达式语句）
3. 错误输入不 panic，返回 `ParseError`
4. 块语句正确解析 `{ }` 内的语句列表

## 测试用例

```ms
x = 10
y = 20
print(x + y)
```

预期 AST：

```
Program {
    statements: [
        Assign(x, =, Literal(10)),
        Assign(y, =, Literal(20)),
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
