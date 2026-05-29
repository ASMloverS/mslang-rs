# 标识符与关键字解析

## 所属阶段
Phase 1.3d - 基础设施

## 前置任务
03-lexer-core

## 目标
实现标识符解析和关键字识别，覆盖 [01-lexical](../01-lexical.md) 中的 35 个关键字和 5 个保留字。

## 设计规格

参照 [01-lexical](../01-lexical.md) § 标识符 / 关键字 / 保留字：

```
identifier = [a-zA-Z_][a-zA-Z0-9_]*
```

- 首字符必须是字母或下划线
- 区分大小写
- 关键字不可用作标识符
- 保留字（select, default, case, export, match）不可用作标识符

### 35 个关键字

```
var        const      fn         return
if         elif       else
while      for        in         break      continue
class      self       super
true       false      nil
and        or         not
try        except     finally    defer      with     throw
async      await      go
import     from       as
yield      nonlocal
```

### 5 个保留字

```
select     default    case    export    match
```

## 实现细节

### 文件位置

`src/lexer/mod.rs` 中添加 `read_identifier()` 方法，`src/lexer/token.rs` 中已定义关键字表和保留字集合。

### read_identifier()

```rust
fn read_identifier(&mut self, first: char, start: Position) -> Result<Token> {
    let mut ident = String::new();
    ident.push(first);

    while let Some(c) = self.peek_char() {
        if c.is_ascii_alphanumeric() || c == '_' {
            ident.push(c);
            self.advance();
        } else {
            break;
        }
    }

    let kind = if let Some(kw) = keyword_table().get(ident.as_str()) {
        kw.clone()
    } else if reserved_words().contains(&ident.as_str()) {
        return Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("'{}' is a reserved word and cannot be used as identifier", ident),
        });
    } else {
        TokenKind::Identifier(ident.clone())
    };

    Ok(self.make_token(kind, start, &ident))
}
```

### 优化：关键字表缓存

`keyword_table()` 每次调用都创建 `HashMap`。优化方案：使用 `lazy_static` 或 `phf` 编译时生成：

```rust
use std::collections::HashMap;
use std::sync::OnceLock;

fn keywords() -> &'static HashMap<&'static str, TokenKind> {
    static KEYWORDS: OnceLock<HashMap<&str, TokenKind>> = OnceLock::new();
    KEYWORDS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("var", TokenKind::Var);
        m.insert("const", TokenKind::Const);
        // ... 全部 35 个
        m
    })
}
```

### 关键字与保留字判断逻辑

1. 匹配标识符规则 `[a-zA-Z_][a-zA-Z0-9_]*`
2. 查关键字表 → 命中则返回对应关键字 Token
3. 查保留字集合 → 命中则报错
4. 未命中 → 返回 `Identifier(name)` Token

## 验证标准

1. 所有 35 个关键字正确识别为对应 TokenKind
2. 普通标识符正确识别为 `Identifier(name)`
3. 保留字使用时报错
4. 大小写敏感：`True` 是标识符，`true` 是关键字
5. 下划线开头合法：`_foo` 是标识符

## 测试用例

```ms
var x = 10
const PI = 3.14
fn greet(name) {
    return name
}
if true {
    print("yes")
}
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
    fn test_keywords() {
        let tokens = tokenize("var const fn return if elif else\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Var));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Const));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Fn));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Return));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::If));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Elif));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Else));
    }

    #[test]
    fn test_identifier() {
        let tokens = tokenize("myVar _foo bar123\n");
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "myVar")));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "_foo")));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "bar123")));
    }

    #[test]
    fn test_case_sensitive() {
        let tokens = tokenize("True False NIL\n");
        assert!(tokens.iter().all(|t| matches!(&t.kind, TokenKind::Identifier(_))));
    }

    #[test]
    fn test_boolean_keywords() {
        let tokens = tokenize("true false nil\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::True));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::False));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Nil));
    }

    #[test]
    fn test_reserved_word_error() {
        let result = Lexer::new("select = 1\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_all_35_keywords() {
        let source = "var const fn return if elif else while for in break continue \
                      class self super true false nil and or not \
                      try except finally defer with throw \
                      async await go import from as yield nonlocal\n";
        let tokens = tokenize(source);
        let keyword_tokens: Vec<_> = tokens.iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
            .collect();
        assert_eq!(keyword_tokens.len(), 35);
    }
}
```
