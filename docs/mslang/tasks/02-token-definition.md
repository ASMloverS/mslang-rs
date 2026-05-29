# Token 类型定义

## 所属阶段
Phase 1.2 - 基础设施

## 前置任务
01-project-skeleton

## 目标
定义 `TokenKind` 枚举、`Token` 结构体、`Span` / `Position` 结构体，完整覆盖 [01-lexical](../01-lexical.md) 中的所有词法元素。

## 设计规格

参照 [01-lexical](../01-lexical.md) § Token 完整列表：

### TokenKind 枚举

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // 字面量
    Int(i64),
    Float(f64),
    String(String),

    // 标识符
    Identifier(String),

    // 35 个关键字
    Var, Const, Fn, Return,
    If, Elif, Else,
    While, For, In, Break, Continue,
    Class, Self, Super,
    True, False, Nil,
    And, Or, Not,
    Try, Except, Finally, Defer, With, Throw,
    Async, Await, Go,
    Import, From, As,
    Yield, Nonlocal,

    // 算术运算符 (+ - * / // % **)
    Plus, Minus, Star, Slash, DoubleSlash, Percent, DoubleStar,
    // 比较运算符 (== != < > <= >=)
    EqualEqual, BangEqual, Less, Greater, LessEqual, GreaterEqual,
    // 位运算符 (& | ^ << >> ~)
    Ampersand, Pipe, Caret, LeftShift, RightShift, Tilde,
    // 成员运算符 (in is) — In/Is 既作为关键字 token，也在表达式解析中作为比较运算符使用
    // 词法分析器统一返回 TokenKind::In / TokenKind::Is（关键字身份）
    // 表达式解析器在 parse_comparison() 中将其视为比较运算符（双重角色）
    // In 同时也作为 for..in 语法的关键字使用
    // 赋值运算符 (= += -= *= /= //= %= **= &= |= ^= <<= >>=)
    Equal, PlusEqual, MinusEqual, StarEqual, SlashEqual,
    DoubleSlashEqual, PercentEqual, DoubleStarEqual,
    AmpersandEqual, PipeEqual, CaretEqual, LeftShiftEqual, RightShiftEqual,
    // 短声明 (:=)
    ColonEqual,
    // 分隔符
    LeftParen, RightParen,
    LeftBracket, RightBracket,
    LeftBrace, RightBrace,
    Comma, Dot, Colon, Semicolon, Arrow,
    // 特殊符号
    At, LeftArrow,
    // 范围运算符
    DotDot, DotDotDot,
    // 换行（语句终止）
    Newline,
    // EOF
    Eof,
}
```

### Token 结构体

```rust
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
}

impl Token {
    pub fn is_identifier(&self) -> bool {
        matches!(self.kind, TokenKind::Identifier(_))
    }
}
```

### Span 结构体

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}
```

### Position 结构体

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}
```

## 实现细节

### 文件位置

`src/lexer/token.rs`

### 关键实现

1. **TokenKind::Display**：为每个变体提供人类可读的字符串表示

```rust
impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenKind::Int(v) => write!(f, "Int({})", v),
            TokenKind::Float(v) => write!(f, "Float({})", v),
            TokenKind::String(v) => write!(f, "String(\"{}\")", v),
            TokenKind::Identifier(v) => write!(f, "Identifier(\"{}\")", v),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::DoubleSlash => write!(f, "//"),
            // ... 所有变体
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}
```

2. **Token::Display**：格式化为 `Token(kind, lexeme, line:col)`

```rust
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Token({}, \"{}\", {}:{})",
            self.kind, self.lexeme,
            self.span.start.line, self.span.start.column
        )
    }
}
```

3. **关键字查找表**：`HashMap<&'static str, TokenKind>`，用于词法分析阶段

```rust
use std::collections::HashMap;

pub fn keyword_table() -> HashMap<&'static str, TokenKind> {
    let mut m = HashMap::new();
    m.insert("var", TokenKind::Var);
    m.insert("const", TokenKind::Const);
    m.insert("fn", TokenKind::Fn);
    m.insert("return", TokenKind::Return);
    m.insert("if", TokenKind::If);
    m.insert("elif", TokenKind::Elif);
    m.insert("else", TokenKind::Else);
    m.insert("while", TokenKind::While);
    m.insert("for", TokenKind::For);
    m.insert("in", TokenKind::In);
    m.insert("break", TokenKind::Break);
    m.insert("continue", TokenKind::Continue);
    m.insert("class", TokenKind::Class);
    m.insert("self", TokenKind::Self);
    m.insert("super", TokenKind::Super);
    m.insert("true", TokenKind::True);
    m.insert("false", TokenKind::False);
    m.insert("nil", TokenKind::Nil);
    m.insert("and", TokenKind::And);
    m.insert("or", TokenKind::Or);
    m.insert("not", TokenKind::Not);
    m.insert("try", TokenKind::Try);
    m.insert("except", TokenKind::Except);
    m.insert("finally", TokenKind::Finally);
    m.insert("defer", TokenKind::Defer);
    m.insert("with", TokenKind::With);
    m.insert("throw", TokenKind::Throw);
    m.insert("async", TokenKind::Async);
    m.insert("await", TokenKind::Await);
    m.insert("go", TokenKind::Go);
    m.insert("import", TokenKind::Import);
    m.insert("from", TokenKind::From);
    m.insert("as", TokenKind::As);
    m.insert("yield", TokenKind::Yield);
    m.insert("nonlocal", TokenKind::Nonlocal);
    m
}
```

4. **保留字集合**：`select`, `default`, `case`, `export`, `match`（不可用作标识符）

```rust
pub fn reserved_words() -> &'static [&'static str] {
    &["select", "default", "case", "export", "match"]
}
```

### 完整性检查清单

对照 [01-lexical](../01-lexical.md)：

- [ ] 3 种字面量类型：Int, Float, String
- [ ] Identifier
- [ ] 35 个关键字（Var, Const, Fn, Return, If, Elif, Else, While, For, In, Break, Continue, Class, Self, Super, True, False, Nil, And, Or, Not, Try, Except, Finally, Defer, With, Throw, Async, Await, Go, Import, From, As, Yield, Nonlocal）
- [ ] 7 个算术运算符
- [ ] 6 个比较运算符
- [ ] 6 个位运算符
- [ ] 14 个赋值运算符（含 = 和 :=）
- [ ] 12 个分隔符
- [ ] 3 个特殊符号（@, <-, :=）— 注意 := 已在赋值运算符中列为 ColonEqual
- [ ] 2 个范围运算符（.., ...）
- [ ] Newline（语句终止）
- [ ] Eof

## 验证标准

1. `cargo build` 编译通过
2. TokenKind 变体数量覆盖 [01-lexical](../01-lexical.md) Token 完整列表中所有元素
3. 关键字表包含 35 个条目
4. Display 实现正确输出

## 测试用例

无 `.ms` 测试。Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_construction() {
        let tok = Token {
            kind: TokenKind::Int(42),
            lexeme: "42".into(),
            span: Span {
                start: Position { line: 1, column: 1, offset: 0 },
                end: Position { line: 1, column: 3, offset: 2 },
            },
        };
        assert_eq!(tok.kind, TokenKind::Int(42));
        assert_eq!(tok.lexeme, "42");
    }

    #[test]
    fn test_keyword_count() {
        let kw = keyword_table();
        assert_eq!(kw.len(), 35);
    }

    #[test]
    fn test_reserved_words() {
        let reserved = reserved_words();
        assert!(reserved.contains(&"select"));
        assert!(reserved.contains(&"match"));
    }

    #[test]
    fn test_token_display() {
        let tok = Token {
            kind: TokenKind::Identifier("x".into()),
            lexeme: "x".into(),
            span: Span {
                start: Position { line: 1, column: 1, offset: 0 },
                end: Position { line: 1, column: 2, offset: 1 },
            },
        };
        let s = format!("{}", tok);
        assert!(s.contains("Identifier"));
        assert!(s.contains("x"));
    }

    #[test]
    fn test_position_copy() {
        let p = Position { line: 1, column: 5, offset: 4 };
        let p2 = p;
        assert_eq!(p, p2);
    }
}
```
