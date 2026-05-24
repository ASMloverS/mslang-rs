# 换行与语句终止规则

## 所属阶段
Phase 1.3f - 基础设施

## 前置任务
07-lexer-operators-delimiters

## 目标
在 Lexer 中实现换行与语句终止规则，处理隐式续行，确保多行语句正确 token 化。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 语句终止：

### 规则

1. 换行符终止当前语句
2. **续行规则** — 以下情况换行不终止语句：
   - 行尾是运算符（`+`, `-`, `*`, `/`, `//`, `%`, `**`, `=`, `==`, `!=`, `<`, `>`, `<=`, `>=`, `&`, `|`, `^`, `<<`, `>>`, `and`, `or`, `not`, `in`, `is`）
   - 行尾是逗号 `,`
   - 行尾是左括号 `(`, `[`, `{`
   - 行首是运算符
   - 字符串字面量内（已由引号界定，不允许跨行）

### 换行符统一

`\r\n` 和 `\n` 统一处理为 `\n`（已在 03-lexer-core 中实现）。

### 实现：隐式续行

当满足续行条件时，跳过 `Newline` token，不插入语句终止符。

## 实现细节

### 文件位置

`src/lexer/mod.rs`

### 方案：在 next_token() 后处理 Newline

在 `next_token()` 返回 `Newline` 时，检查前一个 token 和后一个字符，决定是否跳过：

```rust
pub fn next_token(&mut self) -> Result<Token> {
    loop {
        self.skip_whitespace();

        let start = self.current_position();
        let ch = match self.advance() {
            Some(c) => c,
            None => return Ok(self.make_token(TokenKind::Eof, start, "")),
        };

        let token = match ch {
            '\n' => {
                if self.is_continuation() {
                    continue;
                }
                self.make_token(TokenKind::Newline, start, "\n")
            }
            '#' => { self.skip_comment(); continue; }
            // ... 其他 token 处理
        };

        self.prev_token_kind = Some(token.kind.clone());
        return Ok(token);
    }
}
```

### is_continuation() — 续行判断

```rust
fn is_continuation(&self) -> bool {
    let prev = match &self.prev_token_kind {
        Some(k) => k,
        None => return false,
    };

    // 规则 1: 行尾是运算符
    if is_binary_operator(prev) {
        return true;
    }

    // 规则 2: 行尾是逗号
    if matches!(prev, TokenKind::Comma) {
        return true;
    }

    // 规则 3: 行尾是左括号
    if matches!(prev, TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace) {
        return true;
    }

    // 规则 4: 行首是运算符（检查下一行第一个非空白字符）
    if self.next_non_whitespace_is_operator() {
        return true;
    }

    false
}
```

### is_binary_operator()

```rust
fn is_binary_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Plus | TokenKind::Minus | TokenKind::Star | TokenKind::Slash
        | TokenKind::DoubleSlash | TokenKind::Percent | TokenKind::DoubleStar
        | TokenKind::Equal | TokenKind::EqualEqual | TokenKind::BangEqual
        | TokenKind::Less | TokenKind::Greater | TokenKind::LessEqual | TokenKind::GreaterEqual
        | TokenKind::Ampersand | TokenKind::Pipe | TokenKind::Caret
        | TokenKind::LeftShift | TokenKind::RightShift
        | TokenKind::And | TokenKind::Or | TokenKind::Not
        | TokenKind::In | TokenKind::Is
        | TokenKind::Dot | TokenKind::Arrow
    )
}
```

### next_non_whitespace_is_operator()

```rust
fn next_non_whitespace_is_operator(&self) -> bool {
    let mut i = self.pos;
    while i < self.chars.len() {
        let c = self.chars[i];
        if c == ' ' || c == '\t' {
            i += 1;
            continue;
        }
        // 检查是否是运算符开头的字符
        return matches!(
            c,
            '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>'
            | '&' | '|' | '^' | '~' | '.'
        );
    }
    false
}
```

### 括号深度跟踪

为正确处理续行，Lexer 需要跟踪括号深度：

```rust
pub struct Lexer {
    // ... 已有字段
    paren_depth: usize,    // () 深度
    bracket_depth: usize,  // [] 深度
    brace_depth: usize,    // {} 深度
    prev_token_kind: Option<TokenKind>,
}
```

当括号深度 > 0 时，所有 `Newline` 都被跳过（不产生 token）：

```rust
'\n' => {
    if self.paren_depth > 0 || self.bracket_depth > 0 || self.brace_depth > 0 {
        continue;  // 括号内换行直接跳过
    }
    if self.is_continuation() {
        continue;
    }
    self.make_token(TokenKind::Newline, start, "\n")
}
```

遇到 `(`、`[`、`{` 时递增深度，遇到 `)`、`]`、`}` 时递减深度。

### 注意事项

- 注释行后的换行：如果前一个非注释 token 触发了续行，注释后的换行也应被跳过
- 空行（连续换行）不应产生多个 `Newline` token
- 括号深度跟踪与续行规则互补

## 验证标准

1. 基本换行产生 `Newline` token
2. 行尾运算符后的换行被跳过
3. 行尾逗号后的换行被跳过
4. 行尾左括号后的换行被跳过
5. 括号内换行全部被跳过
6. 连续空行只产生一个 `Newline`
7. `\r\n` 统一为 `\n`

## 测试用例

```ms
total = a +
        b +
        c

names = [
    "Alice",
    "Bob",
    "Charlie"
]

result = some_function(
    arg1,
    arg2
)
```

预期：第一段产生 1 个 `Newline`（在 `c` 之后），列表和函数调用内不产生 `Newline`。

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::TokenKind;

    fn tokenize(source: &str) -> Vec<Token> {
        Lexer::new(source).tokenize_all().unwrap()
    }

    fn newline_count(tokens: &[Token]) -> usize {
        tokens.iter().filter(|t| t.kind == TokenKind::Newline).count()
    }

    #[test]
    fn test_basic_newline() {
        let tokens = tokenize("x = 1\ny = 2\n");
        assert!(newline_count(&tokens) >= 2);
    }

    #[test]
    fn test_operator_continuation() {
        let tokens = tokenize("total = a +\n        b +\n        c\n");
        // + 后的换行被跳过，只有末尾换行
        let operators: Vec<_> = tokens.iter()
            .filter(|t| t.kind == TokenKind::Plus)
            .collect();
        assert_eq!(operators.len(), 2);
    }

    #[test]
    fn test_list_continuation() {
        let tokens = tokenize("names = [\n    \"Alice\",\n    \"Bob\"\n]\n");
        // [] 内换行被跳过
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftBracket));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::RightBracket));
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "Alice")));
        assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::String(s) if s == "Bob")));
    }

    #[test]
    fn test_function_call_continuation() {
        let tokens = tokenize("result = fn(\n    arg1,\n    arg2\n)\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Identifier(s) if s == "arg1" => true));
    }

    #[test]
    fn test_bracket_depth_balanced() {
        let lexer = Lexer::new("x = [\n1\n]\n");
        let tokens = lexer.tokenize_all().unwrap();
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Int(1)));
    }
}
```
