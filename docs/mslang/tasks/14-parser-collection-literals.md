# 集合字面量与匿名函数解析

## 所属阶段
Phase 1.5d - 基础设施

## 前置任务
12-parser-expressions

## 目标
实现列表、Dict、Set、元组字面量、匿名函数和推导式的解析。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 列表字面量 / Dict 字面量 / Set 字面量 / 元组 / 推导式：

### 列表

```
list_content = expression ("," expression)* ","?
```

```ms
[1, 2, 3]
["a", "b"]
[]
```

### Dict

```
dict_content = (expression ":" expression) ("," (expression ":" expression))* ","?
```

```ms
{"a": 1, "b": 2}
{}
```

### Set

```
set_content = expression ("," expression)+ ","?
```

非空花括号，元素没有冒号分隔 → set。

```ms
{1, 2, 3}
```

`{}` 是空 dict，空 set 用 `set()`。

### 元组

```
tuple = "(" expression ("," expression)+ ","? ")"
      | expression "," expression ("," expression)*
```

```ms
(1, 2, 3)
1, 2, 3           # 裸元组
(42,)             # 单元素元组
()                # 空元组
```

### 匿名函数

```
fn_literal = "fn" "(" param_list? ")" block
```

```ms
double = fn(x) { return x * 2 }
```

### 推导式

```
list_comp = "[" expression "for" IDENTIFIER "in" expression ("if" expression)? "]"
dict_comp = "{" expression ":" expression "for" IDENTIFIER "in" expression ("if" expression)? "}"
set_comp  = "{" expression "for" IDENTIFIER "in" expression ("if" expression)? "}"
```

## 实现细节

### 文件位置

`src/parser/expression.rs` 中添加相应解析方法。

### parse_list_literal()

```rust
fn parse_list_literal(&mut self) -> Result<Expr> {
    self.advance(); // consume '['
    self.skip_newlines();

    if self.check(&TokenKind::RightBracket) {
        self.advance();
        return Ok(Expr::ListLiteral { elements: vec![] });
    }

    // 检查是否是推导式: [expr for x in iter ...]
    let first = self.parse_expression()?;
    if self.check(&TokenKind::For) {
        return self.parse_list_comprehension(first);
    }

    let mut elements = vec![first];
    while self.match_token(&[TokenKind::Comma]) {
        self.skip_newlines();
        if self.check(&TokenKind::RightBracket) { break; }
        elements.push(self.parse_expression()?);
        self.skip_newlines();
    }
    self.skip_newlines();
    self.expect(TokenKind::RightBracket, "expected ']'")?;
    Ok(Expr::ListLiteral { elements })
}
```

### parse_list_comprehension()

```rust
fn parse_list_comprehension(&mut self, expr: Expr) -> Result<Expr> {
    self.advance(); // consume 'for'
    let target = self.expect_identifier("expected variable name after 'for'")?;
    self.expect(TokenKind::In, "expected 'in' in comprehension")?;
    let iterable = self.parse_expression()?;

    let condition = if self.match_token(&[TokenKind::If]) {
        Some(Box::new(self.parse_expression()?))
    } else {
        None
    };

    self.skip_newlines();
    self.expect(TokenKind::RightBracket, "expected ']'")?;
    Ok(Expr::ListComprehension {
        expr: Box::new(expr),
        target,
        iterable: Box::new(iterable),
        condition,
    })
}
```

### parse_dict_or_set()

```rust
fn parse_dict_or_set(&mut self) -> Result<Expr> {
    self.advance(); // consume '{'
    self.skip_newlines();

    if self.check(&TokenKind::RightBrace) {
        self.advance();
        return Ok(Expr::DictLiteral { pairs: vec![] }); // {} is empty dict
    }

    let first = self.parse_expression()?;

    if self.check(&TokenKind::Colon) {
        // Dict: {key: value, ...}
        self.advance();
        let value = self.parse_expression()?;

        // 检查是否是 dict 推导式
        if self.check(&TokenKind::For) {
            return self.parse_dict_comprehension(first, value);
        }

        let mut pairs = vec![(first, value)];
        while self.match_token(&[TokenKind::Comma]) {
            self.skip_newlines();
            if self.check(&TokenKind::RightBrace) { break; }
            let key = self.parse_expression()?;
            self.expect(TokenKind::Colon, "expected ':' in dict literal")?;
            let val = self.parse_expression()?;
            pairs.push((key, val));
            self.skip_newlines();
        }
        self.skip_newlines();
        self.expect(TokenKind::RightBrace, "expected '}'")?;
        Ok(Expr::DictLiteral { pairs })
    } else {
        // Set: {elem, ...}

        // 检查是否是 set 推导式
        if self.check(&TokenKind::For) {
            return self.parse_set_comprehension(first);
        }

        let mut elements = vec![first];
        while self.match_token(&[TokenKind::Comma]) {
            self.skip_newlines();
            if self.check(&TokenKind::RightBrace) { break; }
            elements.push(self.parse_expression()?);
            self.skip_newlines();
        }
        self.skip_newlines();
        self.expect(TokenKind::RightBrace, "expected '}'")?;
        Ok(Expr::SetLiteral { elements })
    }
}
```

### parse_dict_comprehension()

```rust
fn parse_dict_comprehension(&mut self, key_expr: Expr, value_expr: Expr) -> Result<Expr> {
    self.advance(); // consume 'for'
    let target = self.expect_identifier("expected variable name")?;
    self.expect(TokenKind::In, "expected 'in'")?;
    let iterable = self.parse_expression()?;

    let condition = if self.match_token(&[TokenKind::If]) {
        Some(Box::new(self.parse_expression()?))
    } else {
        None
    };

    self.skip_newlines();
    self.expect(TokenKind::RightBrace, "expected '}'")?;
    Ok(Expr::DictComprehension {
        key_expr: Box::new(key_expr),
        value_expr: Box::new(value_expr),
        target,
        iterable: Box::new(iterable),
        condition,
    })
}
```

### parse_set_comprehension()

```rust
fn parse_set_comprehension(&mut self, expr: Expr) -> Result<Expr> {
    self.advance(); // consume 'for'
    let target = self.expect_identifier("expected variable name")?;
    self.expect(TokenKind::In, "expected 'in'")?;
    let iterable = self.parse_expression()?;

    let condition = if self.match_token(&[TokenKind::If]) {
        Some(Box::new(self.parse_expression()?))
    } else {
        None
    };

    self.skip_newlines();
    self.expect(TokenKind::RightBrace, "expected '}'")?;
    Ok(Expr::SetComprehension {
        expr: Box::new(expr),
        target,
        iterable: Box::new(iterable),
        condition,
    })
}
```

### parse_grouping_or_tuple()

```rust
fn parse_grouping_or_tuple(&mut self) -> Result<Expr> {
    self.advance(); // consume '('

    if self.check(&TokenKind::RightParen) {
        self.advance();
        return Ok(Expr::TupleLiteral { elements: vec![] }); // ()
    }

    let first = self.parse_expression()?;

    if self.check(&TokenKind::Comma) {
        // Tuple: (expr, expr, ...)
        let mut elements = vec![first];
        while self.match_token(&[TokenKind::Comma]) {
            if self.check(&TokenKind::RightParen) { break; } // trailing comma
            elements.push(self.parse_expression()?);
        }
        self.expect(TokenKind::RightParen, "expected ')'")?;
        return Ok(Expr::TupleLiteral { elements });
    }

    // Grouping: (expr)
    self.expect(TokenKind::RightParen, "expected ')'")?;
    Ok(Expr::Grouping { expr: Box::new(first) })
}
```

### parse_fn_literal()

```rust
fn parse_fn_literal(&mut self) -> Result<Expr> {
    self.advance(); // consume 'fn'
    self.expect(TokenKind::LeftParen, "expected '(' in anonymous function")?;
    let params = self.parse_param_list()?;
    self.expect(TokenKind::RightParen, "expected ')'")?;
    let body = self.parse_block()?;

    Ok(Expr::FnLiteral { params, body })
}
```

### is_fn_literal()

判断 `fn` 后面跟的是 `(`（匿名函数）还是标识符（函数声明）：

```rust
fn is_fn_literal(&self) -> bool {
    if !self.check(&TokenKind::Fn) { return false; }
    let next = self.tokens.get(self.current + 1);
    matches!(next.map(|t| &t.kind), Some(TokenKind::LeftParen))
}
```

## 验证标准

1. 空列表 `[]`、空 dict `{}`、空元组 `()` 正确解析
2. 单元素元组 `(42,)` 正确解析
3. 分组表达式 `(x + y)` 正确解析为 Grouping 而非 Tuple
4. Dict 和 Set 正确区分（冒号区分）
5. 匿名函数正确解析
6. 列表推导式（含过滤条件）正确解析
7. Dict 和 Set 推导式正确解析

## 测试用例

```ms
nums = [1, 2, 3, 4, 5]
person = {"name": "Alice", "age": 30}
unique = {1, 2, 3}
point = (1, 2)
double = fn(x) { return x * 2 }
squares = [x * x for x in nums if x > 2]
```

Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_expr(source: &str) -> Result<Expr> {
        let tokens = Lexer::new(source).tokenize_all()?;
        let mut parser = Parser::new(tokens);
        parser.parse_expression()
    }

    #[test]
    fn test_list_literal() {
        let expr = parse_expr("[1, 2, 3]").unwrap();
        match expr {
            Expr::ListLiteral { elements } => assert_eq!(elements.len(), 3),
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn test_empty_list() {
        let expr = parse_expr("[]").unwrap();
        match expr {
            Expr::ListLiteral { elements } => assert!(elements.is_empty()),
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn test_dict_literal() {
        let expr = parse_expr("{\"a\": 1, \"b\": 2}").unwrap();
        match expr {
            Expr::DictLiteral { pairs } => assert_eq!(pairs.len(), 2),
            _ => panic!("expected dict"),
        }
    }

    #[test]
    fn test_empty_dict() {
        let expr = parse_expr("{}").unwrap();
        match expr {
            Expr::DictLiteral { pairs } => assert!(pairs.is_empty()),
            _ => panic!("expected empty dict"),
        }
    }

    #[test]
    fn test_set_literal() {
        let expr = parse_expr("{1, 2, 3}").unwrap();
        match expr {
            Expr::SetLiteral { elements } => assert_eq!(elements.len(), 3),
            _ => panic!("expected set"),
        }
    }

    #[test]
    fn test_tuple() {
        let expr = parse_expr("(1, 2, 3)").unwrap();
        match expr {
            Expr::TupleLiteral { elements } => assert_eq!(elements.len(), 3),
            _ => panic!("expected tuple"),
        }
    }

    #[test]
    fn test_single_element_tuple() {
        let expr = parse_expr("(42,)").unwrap();
        match expr {
            Expr::TupleLiteral { elements } => assert_eq!(elements.len(), 1),
            _ => panic!("expected single-element tuple"),
        }
    }

    #[test]
    fn test_grouping() {
        let expr = parse_expr("(x + y)").unwrap();
        match expr {
            Expr::Grouping { .. } => {}
            _ => panic!("expected grouping, got tuple or other"),
        }
    }

    #[test]
    fn test_fn_literal() {
        let expr = parse_expr("fn(x) { return x * 2 }").unwrap();
        match expr {
            Expr::FnLiteral { params, body } => {
                assert_eq!(params.len(), 1);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected fn literal"),
        }
    }

    #[test]
    fn test_list_comprehension() {
        let expr = parse_expr("[x * x for x in nums if x > 2]").unwrap();
        match expr {
            Expr::ListComprehension { target, condition, .. } => {
                assert_eq!(target, "x");
                assert!(condition.is_some());
            }
            _ => panic!("expected list comprehension"),
        }
    }

    #[test]
    fn test_list_comprehension_no_filter() {
        let expr = parse_expr("[x * x for x in nums]").unwrap();
        match expr {
            Expr::ListComprehension { condition, .. } => {
                assert!(condition.is_none());
            }
            _ => panic!("expected list comprehension"),
        }
    }

    #[test]
    fn test_empty_tuple() {
        let expr = parse_expr("()").unwrap();
        match expr {
            Expr::TupleLiteral { elements } => assert!(elements.is_empty()),
            _ => panic!("expected empty tuple"),
        }
    }
}
```
