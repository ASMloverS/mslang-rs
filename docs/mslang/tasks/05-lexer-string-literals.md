# 字符串字面量解析

## 所属阶段
Phase 1.3c - 基础设施

## 前置任务
03-lexer-core

## 目标
实现双引号字符串字面量解析，支持全部转义序列，禁止跨行字符串。

## 设计规格

参照 [01-lexical](../01-lexical.md) § 字符串字面量：

```
string = '"' ( [^"\\] | escape )* '"'
escape = '\' ( '"' | '\' | 'n' | 't' | 'r' | '0' )
```

转义序列：

| 转义 | 含义 |
|---|---|
| `\"` | 双引号 `"` |
| `\\` | 反斜杠 `\` |
| `\n` | 换行 |
| `\t` | 制表符 |
| `\r` | 回车 |
| `\0` | 空字符 |

规则：
- 仅支持双引号，不支持单引号
- 字符串不可跨行（未转义的换行为错误）
- 不支持多行字符串（`"""..."""` 后续版本考虑）

## 实现细节

### 文件位置

`src/lexer/mod.rs` 中添加 `read_string()` 方法。

### read_string()

```rust
fn read_string(&mut self, start: Position) -> Result<Token> {
    let mut value = String::new();
    let mut lexeme = String::from("\"");

    loop {
        match self.advance() {
            None => {
                return Err(MspError::LexError {
                    line: start.line,
                    column: start.column,
                    message: "unterminated string".into(),
                });
            }
            Some('"') => {
                lexeme.push('"');
                break;
            }
            Some('\n') => {
                return Err(MspError::LexError {
                    line: start.line,
                    column: start.column,
                    message: "unterminated string (newline in string literal)".into(),
                });
            }
            Some('\\') => {
                lexeme.push('\\');
                match self.advance() {
                    Some('"') => { value.push('"'); lexeme.push('"'); }
                    Some('\\') => { value.push('\\'); lexeme.push('\\'); }
                    Some('n') => { value.push('\n'); lexeme.push('n'); }
                    Some('t') => { value.push('\t'); lexeme.push('t'); }
                    Some('r') => { value.push('\r'); lexeme.push('r'); }
                    Some('0') => { value.push('\0'); lexeme.push('0'); }
                    Some(c) => {
                        return Err(MspError::LexError {
                            line: self.line,
                            column: self.column,
                            message: format!("unknown escape sequence: \\{}", c),
                        });
                    }
                    None => {
                        return Err(MspError::LexError {
                            line: start.line,
                            column: start.column,
                            message: "unterminated string (end of file in escape)".into(),
                        });
                    }
                }
            }
            Some(c) => {
                value.push(c);
                lexeme.push(c);
            }
        }
    }

    Ok(Token {
        kind: TokenKind::String(value),
        lexeme,
        span: Span {
            start,
            end: self.current_position(),
        },
    })
}
```

### 注意事项

- `lexeme` 保留原始文本（含引号和转义），`value` 存储解析后的值
- 遇到未转义的 `\n` 或 EOF 时报 unterminated string 错误
- 未知转义序列（如 `\x`）报错
- 空字符串 `""` → `TokenKind::String("")` 合法

## 验证标准

1. 普通字符串正确解析
2. 所有 6 种转义序列正确处理
3. 未终止字符串报错
4. 字符串中出现未转义换行报错
5. 空字符串合法

## 测试用例

```ms
a = "hello world"
b = "line1\nline2"
c = "path: C:\\Users"
d = "quotes: \"hello\""
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
    fn test_simple_string() {
        let tokens = tokenize("a = \"hello world\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "hello world")));
    }

    #[test]
    fn test_newline_escape() {
        let tokens = tokenize("b = \"line1\\nline2\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "line1\nline2")));
    }

    #[test]
    fn test_backslash_escape() {
        let tokens = tokenize("c = \"path: C:\\\\Users\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "path: C:\\Users")));
    }

    #[test]
    fn test_quote_escape() {
        let tokens = tokenize("d = \"quotes: \\\"hello\\\"\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "quotes: \"hello\"")));
    }

    #[test]
    fn test_empty_string() {
        let tokens = tokenize("e = \"\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "")));
    }

    #[test]
    fn test_unterminated_string() {
        let result = Lexer::new("x = \"unterminated\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_escape() {
        let result = Lexer::new("x = \"\\x\"\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_tab_and_null_escape() {
        let tokens = tokenize("x = \"\\t\\0\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "\t\0")));
    }
}
```
