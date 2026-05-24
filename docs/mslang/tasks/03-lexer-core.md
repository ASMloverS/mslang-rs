# 词法分析器核心框架

## 所属阶段
Phase 1.3a - 基础设施

## 前置任务
02-token-definition

## 目标
实现 `Lexer` 核心框架，包括字符遍历、空白符跳过、注释跳过、标识符/关键字/运算符的基础 token 化，以及错误恢复机制。

## 设计规格

参照 [01-lexical](../01-lexical.md) § 词法分析规则：

- 最大匹配原则：`**` 优先于 `*`，`//` 优先于 `/`，`<=` 优先于 `<`
- 关键字 vs 标识符：先匹配标识符规则 `[a-zA-Z_][a-zA-Z0-9_]*`，再查关键字表
- 注释：`#` 开始到行尾，直接跳过
- 空白符：空格、制表符用于分隔 token，不影响语义
- 换行符：`\n` 和 `\r\n` 统一处理为 `\n`，产生 `Newline` token
- 错误恢复：遇到非法字符时报告错误，跳过该字符，继续分析

## 实现细节

### 文件位置

`src/lexer/mod.rs`

### Lexer 结构体

```rust
pub struct Lexer {
    source: String,
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}
```

### 核心方法

#### new()

```rust
pub fn new(source: &str) -> Lexer {
    let normalized = source.replace("\r\n", "\n");
    Lexer {
        source: normalized.clone(),
        chars: normalized.chars().collect(),
        pos: 0,
        line: 1,
        column: 1,
    }
}
```

- 将 `\r\n` 统一为 `\n`

#### peek_char() / advance()

```rust
fn peek_char(&self) -> Option<char> {
    self.chars.get(self.pos).copied()
}

fn peek_next(&self) -> Option<char> {
    self.chars.get(self.pos + 1).copied()
}

fn advance(&mut self) -> Option<char> {
    let ch = self.chars.get(self.pos).copied();
    if let Some(c) = ch {
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }
    ch
}
```

#### skip_whitespace() / skip_comment()

```rust
fn skip_whitespace(&mut self) {
    while let Some(c) = self.peek_char() {
        if c == ' ' || c == '\t' || c == '\r' {
            self.advance();
        } else {
            break;
        }
    }
}

fn skip_comment(&mut self) {
    while let Some(c) = self.peek_char() {
        if c == '\n' { break; }
        self.advance();
    }
}
```

#### next_token()

主入口，返回 `Result<Token>`：

```rust
pub fn next_token(&mut self) -> Result<Token> {
    self.skip_whitespace();

    let start = self.current_position();

    let ch = match self.advance() {
        Some(c) => c,
        None => return Ok(self.make_token(TokenKind::Eof, start, "")),
    };

    match ch {
        '\n' => Ok(self.make_token(TokenKind::Newline, start, "\n")),
        '#' => { self.skip_comment(); self.next_token() }
        c if c.is_ascii_digit() => self.read_number(c, start),
        '"' => self.read_string(start),
        c if c.is_ascii_alphabetic() || c == '_' => self.read_identifier(c, start),
        '+' => self.read_plus(start),
        '-' => self.read_minus(start),
        '*' => self.read_star(start),
        '/' => self.read_slash(start),
        '%' => self.read_percent(start),
        '=' => self.read_equal(start),
        '!' => self.read_bang(start),
        '<' => self.read_less(start),
        '>' => self.read_greater(start),
        '&' => self.read_ampersand(start),
        '|' => self.read_pipe(start),
        '^' => self.read_caret(start),
        '~' => Ok(self.make_token(TokenKind::Tilde, start, "~")),
        '(' => Ok(self.make_token(TokenKind::LeftParen, start, "(")),
        ')' => Ok(self.make_token(TokenKind::RightParen, start, ")")),
        '[' => Ok(self.make_token(TokenKind::LeftBracket, start, "[")),
        ']' => Ok(self.make_token(TokenKind::RightBracket, start, "]")),
        '{' => Ok(self.make_token(TokenKind::LeftBrace, start, "{")),
        '}' => Ok(self.make_token(TokenKind::RightBrace, start, "}")),
        ',' => Ok(self.make_token(TokenKind::Comma, start, ",")),
        '.' => self.read_dot(start),
        ':' => self.read_colon(start),
        ';' => Ok(self.make_token(TokenKind::Semicolon, start, ";")),
        '@' => Ok(self.make_token(TokenKind::At, start, "@")),
        _ => {
            Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: format!("unexpected character '{}'", ch),
            })
        }
    }
}
```

#### tokenize_all()

便利方法，一次性返回全部 token：

```rust
pub fn tokenize_all(mut self) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    loop {
        let tok = self.next_token()?;
        let is_eof = tok.kind == TokenKind::Eof;
        tokens.push(tok);
        if is_eof { break; }
    }
    Ok(tokens)
}
```

#### 错误恢复

在 `next_token()` 中，遇到非法字符时返回 `LexError`。调用方（如 REPL 或 IDE 集成）可以捕获错误后调用 `next_token()` 继续。对于批量 token 化（`tokenize_all`），遇到第一个错误即停止。

### 辅助方法

#### make_token()

```rust
fn make_token(&self, kind: TokenKind, start: Position, lexeme: &str) -> Token {
    Token {
        kind,
        lexeme: lexeme.to_string(),
        span: Span {
            start,
            end: self.current_position(),
        },
    }
}
```

#### current_position()

```rust
fn current_position(&self) -> Position {
    Position {
        line: self.line,
        column: self.column,
        offset: self.pos,
    }
}
```

## 验证标准

1. `cargo build` 编译通过
2. 能正确 token 化含注释、空白符的简单程序
3. 换行产生 `Newline` token
4. 非法字符返回 `LexError`
5. `#` 注释被跳过

## 测试用例

```ms
# this is a comment
x = 10
```

预期 token 序列：

| Token | Kind |
|---|---|
| `x` | Identifier("x") |
| `=` | Equal |
| `10` | Int(10) |
| _(换行)_ | Newline |
| _EOF_ | Eof |

注释 `# this is a comment` 及其所在行的换行被跳过（注释后的换行是否产生 Newline 由实现决定，但不应影响语义）。

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::TokenKind;

    #[test]
    fn test_comment_and_assignment() {
        let source = "# this is a comment\nx = 10\n";
        let lexer = Lexer::new(source);
        let tokens = lexer.tokenize_all().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert!(kinds.contains(&TokenKind::Identifier("x".into())));
        assert!(kinds.contains(&TokenKind::Equal));
        assert!(kinds.contains(&TokenKind::Int(10)));
        assert!(kinds.contains(&TokenKind::Eof));
    }

    #[test]
    fn test_unexpected_char() {
        let source = "x $ y";
        let mut lexer = Lexer::new(source);
        let first = lexer.next_token();
        assert!(first.is_ok());
        let second = lexer.next_token();
        assert!(second.is_err());
    }

    #[test]
    fn test_newline_normalization() {
        let source = "x\r\ny\n";
        let lexer = Lexer::new(source);
        let tokens = lexer.tokenize_all().unwrap();
        let newline_count = tokens.iter()
            .filter(|t| t.kind == TokenKind::Newline)
            .count();
        assert_eq!(newline_count, 2);
    }
}
```
