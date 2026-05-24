# 运算符与分隔符解析

## 所属阶段
Phase 1.3e - 基础设施

## 前置任务
03-lexer-core

## 目标
实现全部运算符和分隔符的解析，遵循最大匹配原则。

## 设计规格

参照 [01-lexical](../01-lexical.md) § 运算符 / 分隔符 / 特殊语法符号：

### 算术运算符
```
+  -  *  /  //  %  **
```

### 比较运算符
```
==  !=  <  >  <=  >=
```

### 位运算符
```
&  |  ^  <<  >>  ~
```

### 赋值运算符
```
=  +=  -=  *=  /=  //=  %=  **=  &=  |=  ^=  <<=  >>=
```

### 范围运算符
```
..  ...
```

### 分隔符
```
(  )  [  ]  {  }  ,  .  :  ;  ->
```

### 特殊符号
```
@  <-  :=
```

### 最大匹配原则

- `**` 优先于 `*`
- `//` 优先于 `/`（注释 `#` 不与整除冲突）
- `<<` 优先于 `<`，`<=` 优先于 `<`
- `>>` 优先于 `>`，`>=` 优先于 `>`
- `==` 优先于 `=`
- `!=` 是唯一合法的 `!` 开头运算符
- `...` 优先于 `..` 优先于 `.`
- `:=` 优先于 `:`
- `<-` 优先于 `<`（但 `<-` 在 `<` 之后检查，需特别注意顺序）
- `->` 优先于 `-`

## 实现细节

### 文件位置

`src/lexer/mod.rs` 中的各个运算符读取方法。

### 多字符运算符匹配模式

每个运算符读取方法使用 `peek_char()` 前瞻来决定匹配长度：

```rust
fn read_plus(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::PlusEqual, start, "+="))
        }
        _ => Ok(self.make_token(TokenKind::Plus, start, "+")),
    }
}

fn read_minus(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::MinusEqual, start, "-="))
        }
        Some('>') => {
            self.advance();
            Ok(self.make_token(TokenKind::Arrow, start, "->"))
        }
        _ => Ok(self.make_token(TokenKind::Minus, start, "-")),
    }
}

fn read_star(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::StarEqual, start, "*="))
        }
        Some('*') => {
            self.advance();
            if self.peek_char() == Some('=') {
                self.advance();
                Ok(self.make_token(TokenKind::DoubleStarEqual, start, "**="))
            } else {
                Ok(self.make_token(TokenKind::DoubleStar, start, "**"))
            }
        }
        _ => Ok(self.make_token(TokenKind::Star, start, "*")),
    }
}

fn read_slash(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::SlashEqual, start, "/="))
        }
        Some('/') => {
            self.advance();
            if self.peek_char() == Some('=') {
                self.advance();
                Ok(self.make_token(TokenKind::DoubleSlashEqual, start, "//="))
            } else {
                Ok(self.make_token(TokenKind::DoubleSlash, start, "//"))
            }
        }
        _ => Ok(self.make_token(TokenKind::Slash, start, "/")),
    }
}

fn read_percent(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::PercentEqual, start, "%="))
        }
        _ => Ok(self.make_token(TokenKind::Percent, start, "%")),
    }
}

fn read_equal(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::EqualEqual, start, "=="))
        }
        _ => Ok(self.make_token(TokenKind::Equal, start, "=")),
    }
}

fn read_bang(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::BangEqual, start, "!="))
        }
        _ => Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "unexpected character '!'".into(),
        }),
    }
}

fn read_less(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::LessEqual, start, "<="))
        }
        Some('<') => {
            self.advance();
            if self.peek_char() == Some('=') {
                self.advance();
                Ok(self.make_token(TokenKind::LeftShiftEqual, start, "<<="))
            } else {
                Ok(self.make_token(TokenKind::LeftShift, start, "<<"))
            }
        }
        Some('-') => {
            self.advance();
            Ok(self.make_token(TokenKind::LeftArrow, start, "<-"))
        }
        _ => Ok(self.make_token(TokenKind::Less, start, "<")),
    }
}

fn read_greater(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::GreaterEqual, start, ">="))
        }
        Some('>') => {
            self.advance();
            if self.peek_char() == Some('=') {
                self.advance();
                Ok(self.make_token(TokenKind::RightShiftEqual, start, ">>="))
            } else {
                Ok(self.make_token(TokenKind::RightShift, start, ">>"))
            }
        }
        _ => Ok(self.make_token(TokenKind::Greater, start, ">")),
    }
}

fn read_ampersand(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::AmpersandEqual, start, "&="))
        }
        _ => Ok(self.make_token(TokenKind::Ampersand, start, "&")),
    }
}

fn read_pipe(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::PipeEqual, start, "|="))
        }
        _ => Ok(self.make_token(TokenKind::Pipe, start, "|")),
    }
}

fn read_caret(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::CaretEqual, start, "^="))
        }
        _ => Ok(self.make_token(TokenKind::Caret, start, "^")),
    }
}

fn read_dot(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('.') => {
            self.advance();
            if self.peek_char() == Some('.') {
                self.advance();
                Ok(self.make_token(TokenKind::DotDotDot, start, "..."))
            } else {
                Ok(self.make_token(TokenKind::DotDot, start, ".."))
            }
        }
        _ => Ok(self.make_token(TokenKind::Dot, start, ".")),
    }
}

fn read_colon(&mut self, start: Position) -> Result<Token> {
    match self.peek_char() {
        Some('=') => {
            self.advance();
            Ok(self.make_token(TokenKind::ColonEqual, start, ":="))
        }
        _ => Ok(self.make_token(TokenKind::Colon, start, ":")),
    }
}
```

### 匹配顺序注意事项

`<` 的处理需要特别关注：
- `<=` 优先于 `<`
- `<<` / `<<=` 优先于 `<`
- `<-` 优先于 `<`

当前实现中 `<` 先检查 `=` 和 `<`，再检查 `-`。`<-` 是三字符运算符中最长的匹配，符合最大匹配原则。

## 验证标准

1. 所有单字符和多字符运算符正确识别
2. 最大匹配原则：`**=` 不会被拆为 `**` + `=`
3. 赋值变体运算符正确识别（`+=`, `//=`, `**=`, `<<=` 等）
4. `!` 单独出现报错
5. 分隔符和特殊符号正确识别

## 测试用例

```ms
x = 10 + 3
y = x // 3
z = x ** 2
a = x >= 5
b = x != 0
c = x & 0xFF
d = x << 2
x += 1
```

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::TokenKind;

    fn tokenize(source: &str) -> Vec<Token> {
        Lexer::new(source).tokenize_all().unwrap()
    }

    #[test]
    fn test_arithmetic() {
        let tokens = tokenize("x = 10 + 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Plus));
    }

    #[test]
    fn test_floor_div() {
        let tokens = tokenize("y = x // 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleSlash));
    }

    #[test]
    fn test_power() {
        let tokens = tokenize("z = x ** 2\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleStar));
    }

    #[test]
    fn test_comparison() {
        let tokens = tokenize("a = x >= 5\nb = x != 0\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::GreaterEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::BangEqual));
    }

    #[test]
    fn test_bitwise() {
        let tokens = tokenize("c = x & 0xFF\nd = x << 2\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Ampersand));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftShift));
    }

    #[test]
    fn test_compound_assignment() {
        let tokens = tokenize("x += 1\ny **= 2\nz <<= 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::PlusEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleStarEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftShiftEqual));
    }

    #[test]
    fn test_special_symbols() {
        let tokens = tokenize("@ deco\nx := 10\nch <- val\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::At));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::ColonEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftArrow));
    }

    #[test]
    fn test_range_operators() {
        let tokens = tokenize("a .. b\nx ...\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DotDot));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DotDotDot));
    }

    #[test]
    fn test_arrow() {
        let tokens = tokenize("fn foo() -> int {}\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Arrow));
    }

    #[test]
    fn test_bang_alone_is_error() {
        let result = Lexer::new("x = ! y\n").tokenize_all();
        assert!(result.is_err());
    }
}
```
