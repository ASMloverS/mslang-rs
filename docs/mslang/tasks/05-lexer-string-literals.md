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
            Some('\n') | Some('\r') => {
                // 裸 \n 与裸 \r 均视为跨行（\r\n 已在 Lexer::new 归一化为 \n，
                // 此处的 \r 为非 CRLF 的孤立回车，与词法器整体将其视为行终止一致）
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
                            line: start.line,
                            column: start.column,
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

    Ok(self.make_token(TokenKind::String(value), start, &lexeme))
}
```

### 注意事项

- `lexeme` 保留原始文本（含引号和转义），`value` 存储解析后的值
- 遇到未转义的 `\n`、`\r` 或 EOF 时报 unterminated string 错误
- 未知转义序列（如 `\x`）报错
- 空字符串 `""` → `TokenKind::String("")` 合法
- 使用现有 `make_token` 辅助方法构造 Token（与文件内其他 `read_*` 一致）
- `\0` 转义产生 NUL 字节：在 Rust `String` 内合法；但 Phase 6 C API 经 `CString` 传递时内部 NUL 会截断，需在 task 67/68 处理（当前不修复，仅记录）

## 验证标准

1. 普通字符串正确解析
2. 所有 6 种转义序列正确处理（`\"` `\\` `\n` `\t` `\r` `\0`）
3. 未终止字符串报错（含纯 EOF 与 `\` 后 EOF 两种路径）
4. 字符串中出现未转义换行（`\n` 或裸 `\r`）报错
5. 空字符串合法

## 测试用例

```ms
a = "hello world"
b = "line1\nline2"
c = "path: C:\\Users"
d = "quotes: \"hello\""
```

Rust 单元测试：

> **集成说明**：以下测试函数添加到 `src/lexer/mod.rs` 现有 `#[cfg(test)] mod tests` 模块。复用既有 `tokenize` 助手（`Lexer::new(src).tokenize_all().unwrap()`）与 `find_kind` 助手，**勿重复定义 `mod tests` / `tokenize`**（否则触发 E0201 重复定义）。

```rust
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
        // 未转义换行路径
        let result = Lexer::new("x = \"unterminated\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_unterminated_string_eof() {
        // 纯 EOF 路径：无换行、无闭合引号
        let result = Lexer::new("x = \"abc").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_eof_in_escape() {
        // '\' 后立即 EOF（转义序列未完成）
        let result = Lexer::new("x = \"abc\\").tokenize_all();
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

    #[test]
    fn test_carriage_return_escape() {
        // \r 转义 → 回车符（验证标准 #2 的第 5 种转义，此前遗漏）
        let tokens = tokenize("x = \"a\\rb\"\n");
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "a\rb")));
    }

    #[test]
    fn test_bare_carriage_return_error() {
        // 字符串内裸 \r（非 CRLF）视为跨行，与 \n 一致报错
        let result = Lexer::new("x = \"abc\rdef\"\n").tokenize_all();
        assert!(result.is_err());
    }
```
