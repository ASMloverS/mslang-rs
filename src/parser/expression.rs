//! 表达式解析（task 12）：递归下降 + 优先级爬升，15 级运算符优先级。
//! 完整规格见 docs/mslang/tasks/12-parser-expressions.md。
//!
//! 这些方法在非测试构建中尚无调用方（`parse_expr_or_assignment` 仍为 task 13 占位），
//! 故整个模块允许 dead_code；task 13 接入 `parse_expression` 后自然消除。

#![allow(dead_code)]

use crate::ast::{AssignOp, BinaryOp, Expr, Literal, UnaryOp};
use crate::error::{MspError, Result};
use crate::lexer::token::TokenKind;

use super::Parser;

impl Parser {
    /// 表达式入口（优先级 1 起步）。
    pub(super) fn parse_expression(&mut self) -> Result<Expr> {
        self.parse_assignment()
    }

    // ---- 优先级 1：赋值（右结合）----

    fn parse_assignment(&mut self) -> Result<Expr> {
        let expr = self.parse_ternary()?;

        let assign_ops = [
            (TokenKind::Equal, AssignOp::Assign),
            (TokenKind::PlusEqual, AssignOp::PlusAssign),
            (TokenKind::MinusEqual, AssignOp::MinusAssign),
            (TokenKind::StarEqual, AssignOp::StarAssign),
            (TokenKind::SlashEqual, AssignOp::SlashAssign),
            (TokenKind::DoubleSlashEqual, AssignOp::DoubleSlashAssign),
            (TokenKind::PercentEqual, AssignOp::PercentAssign),
            (TokenKind::DoubleStarEqual, AssignOp::DoubleStarAssign),
            (TokenKind::AmpersandEqual, AssignOp::BitAndAssign),
            (TokenKind::PipeEqual, AssignOp::BitOrAssign),
            (TokenKind::CaretEqual, AssignOp::BitXorAssign),
            (TokenKind::LeftShiftEqual, AssignOp::LeftShiftAssign),
            (TokenKind::RightShiftEqual, AssignOp::RightShiftAssign),
        ];

        for (token_kind, op) in &assign_ops {
            if self.check(token_kind) {
                self.advance();
                let value = self.parse_assignment()?;
                return Ok(Expr::Assign {
                    target: Box::new(expr),
                    op: *op,
                    value: Box::new(value),
                });
            }
        }

        // `:=` 短声明为语句级构造（03-syntax.md:48 short_var），由 task 13
        // parse_expr_or_assignment 在语句层检测 ColonEqual 并产出 Stmt::ShortVarDecl；
        // 此处不消费 :=，否则会使 task 13 的 := 分支成为死代码。
        Ok(expr)
    }

    // ---- 优先级 2：三元（右结合）----

    fn parse_ternary(&mut self) -> Result<Expr> {
        let expr = self.parse_or()?;

        if self.check(&TokenKind::If) {
            self.advance();
            let condition = self.parse_or()?;
            self.expect(TokenKind::Else, "expected 'else' in ternary expression")?;
            let else_expr = self.parse_ternary()?;
            return Ok(Expr::Ternary {
                condition: Box::new(condition),
                then_expr: Box::new(expr),
                else_expr: Box::new(else_expr),
            });
        }

        Ok(expr)
    }

    // ---- 优先级 3：or（左结合）----

    fn parse_or(&mut self) -> Result<Expr> {
        let mut expr = self.parse_and()?;
        while self.match_token(&[TokenKind::Or]) {
            let right = self.parse_and()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 4：and（左结合）----

    fn parse_and(&mut self) -> Result<Expr> {
        let mut expr = self.parse_not()?;
        while self.match_token(&[TokenKind::And]) {
            let right = self.parse_not()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 5：not（一元前缀，右结合）----

    fn parse_not(&mut self) -> Result<Expr> {
        if self.match_token(&[TokenKind::Not]) {
            let operand = self.parse_not()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
        }
        self.parse_comparison()
    }

    // ---- 优先级 6：比较（链式比较在解析期反糖为 and）----

    fn parse_comparison(&mut self) -> Result<Expr> {
        let expr = self.parse_bit_or()?;

        let comp_ops = [
            (TokenKind::EqualEqual, BinaryOp::Equal),
            (TokenKind::BangEqual, BinaryOp::NotEqual),
            (TokenKind::Less, BinaryOp::Less),
            (TokenKind::Greater, BinaryOp::Greater),
            (TokenKind::LessEqual, BinaryOp::LessEqual),
            (TokenKind::GreaterEqual, BinaryOp::GreaterEqual),
            (TokenKind::In, BinaryOp::In),
            (TokenKind::Is, BinaryOp::Is),
        ];

        // 匹配首个比较运算符；无则直接返回（单初等表达式）
        let mut first_op = None;
        for (token_kind, op) in &comp_ops {
            if self.check(token_kind) {
                first_op = Some(*op);
                break;
            }
        }
        let first_op = match first_op {
            Some(op) => op,
            None => return Ok(expr),
        };
        self.advance();
        let mut right = self.parse_bit_or()?;
        let mut result = Expr::Binary {
            left: Box::new(expr),
            op: first_op,
            right: Box::new(right.clone()),
        };

        // 后续比较运算符：在解析阶段反糖为 and 链（见 task 09 § 链式比较）
        // a < b < c  =>  (a < b) and (b < c)，中间操作数 clone 复用到两侧
        loop {
            let mut next_op = None;
            for (token_kind, op) in &comp_ops {
                if self.check(token_kind) {
                    next_op = Some(*op);
                    break;
                }
            }
            match next_op {
                Some(op) => {
                    self.advance();
                    let next = self.parse_bit_or()?;
                    result = Expr::Binary {
                        left: Box::new(result),
                        op: BinaryOp::And,
                        right: Box::new(Expr::Binary {
                            left: Box::new(right),
                            op,
                            right: Box::new(next.clone()),
                        }),
                    };
                    right = next;
                }
                None => break,
            }
        }

        Ok(result)
    }

    // ---- 优先级 7：位或 ----

    fn parse_bit_or(&mut self) -> Result<Expr> {
        let mut expr = self.parse_bit_xor()?;
        while self.match_token(&[TokenKind::Pipe]) {
            let right = self.parse_bit_xor()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::BitOr,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 8：位异或 ----

    fn parse_bit_xor(&mut self) -> Result<Expr> {
        let mut expr = self.parse_bit_and()?;
        while self.match_token(&[TokenKind::Caret]) {
            let right = self.parse_bit_and()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::BitXor,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 9：位与 ----

    fn parse_bit_and(&mut self) -> Result<Expr> {
        let mut expr = self.parse_shift()?;
        while self.match_token(&[TokenKind::Ampersand]) {
            let right = self.parse_shift()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::BitAnd,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 10：位移 ----

    fn parse_shift(&mut self) -> Result<Expr> {
        let mut expr = self.parse_addition()?;
        loop {
            let op = if self.match_token(&[TokenKind::LeftShift]) {
                BinaryOp::LeftShift
            } else if self.match_token(&[TokenKind::RightShift]) {
                BinaryOp::RightShift
            } else {
                break;
            };
            let right = self.parse_addition()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 11：加减 ----

    fn parse_addition(&mut self) -> Result<Expr> {
        let mut expr = self.parse_multiplication()?;
        loop {
            let op = if self.match_token(&[TokenKind::Plus]) {
                BinaryOp::Add
            } else if self.match_token(&[TokenKind::Minus]) {
                BinaryOp::Subtract
            } else {
                break;
            };
            let right = self.parse_multiplication()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 12：乘除模 ----

    fn parse_multiplication(&mut self) -> Result<Expr> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.match_token(&[TokenKind::Star]) {
                BinaryOp::Multiply
            } else if self.match_token(&[TokenKind::Slash]) {
                BinaryOp::Divide
            } else if self.match_token(&[TokenKind::DoubleSlash]) {
                BinaryOp::FloorDiv
            } else if self.match_token(&[TokenKind::Percent]) {
                BinaryOp::Modulo
            } else {
                break;
            };
            let right = self.parse_unary()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    // ---- 优先级 13：一元前缀（- ~ <-），右结合 ----

    fn parse_unary(&mut self) -> Result<Expr> {
        if self.match_token(&[TokenKind::Minus]) {
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Negate,
                operand: Box::new(operand),
            });
        }
        if self.match_token(&[TokenKind::Tilde]) {
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::BitNot,
                operand: Box::new(operand),
            });
        }
        if self.match_token(&[TokenKind::LeftArrow]) {
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::ChannelReceive,
                operand: Box::new(operand),
            });
        }
        self.parse_power()
    }

    // ---- 优先级 14：幂（右结合）----
    // 右侧调用 parse_unary()：既允许 2 ** -3，又经 parse_unary → parse_power
    // 递归实现右结合 2 ** 3 ** 2 = 2 ** (3 ** 2)。

    fn parse_power(&mut self) -> Result<Expr> {
        let base = self.parse_postfix()?;
        if self.match_token(&[TokenKind::DoubleStar]) {
            let exponent = self.parse_unary()?;
            return Ok(Expr::Binary {
                left: Box::new(base),
                op: BinaryOp::Power,
                right: Box::new(exponent),
            });
        }
        Ok(base)
    }

    // ---- 优先级 15：后缀（() [] .）----

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.match_token(&[TokenKind::LeftParen]) {
                let args = self.parse_arguments()?;
                self.expect(TokenKind::RightParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else if self.match_token(&[TokenKind::LeftBracket]) {
                if self.is_slice() {
                    expr = self.parse_slice(expr)?;
                } else {
                    let index = self.parse_expression()?;
                    self.expect(TokenKind::RightBracket, "expected ']'")?;
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
            } else if self.match_token(&[TokenKind::Dot]) {
                // 不可用 expect(TokenKind::Identifier(String::new()))：TokenKind 派生的
                // PartialEq 按内层 String 比较，与 check 的 == 配合会使任意真实标识符都
                // 不匹配 Identifier("")。改用模式匹配。
                let name = match &self.peek().kind {
                    TokenKind::Identifier(n) => n.clone(),
                    _ => {
                        let tok = self.peek();
                        return Err(MspError::ParseError {
                            line: tok.span.start.line,
                            column: tok.span.start.column,
                            message: "expected property name after '.'".into(),
                        });
                    }
                };
                self.advance();
                expr = Expr::Dot {
                    object: Box::new(expr),
                    name,
                };
            } else {
                break;
            }
        }

        Ok(expr)
    }

    // ---- 优先级 15（续）：初等表达式 ----
    // 先克隆 kind 再 match，避免持有 self 的不可变借用而无法在分支内调用 advance/expect。

    fn parse_primary(&mut self) -> Result<Expr> {
        let kind = self.peek().kind.clone();
        match &kind {
            TokenKind::Int(v) => {
                let v = *v;
                self.advance();
                Ok(Expr::Literal(Literal::Int(v)))
            }
            TokenKind::Float(v) => {
                let v = *v;
                self.advance();
                Ok(Expr::Literal(Literal::Float(v)))
            }
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::Literal(Literal::String(s)))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            TokenKind::Nil => {
                self.advance();
                Ok(Expr::Literal(Literal::Nil))
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Identifier(name))
            }
            TokenKind::Zelf => {
                // `self` 关键字在方法体内作为初等表达式；按 task 13 约定映射为
                // Expr::Identifier("self")，使 self.attr 解析路径与普通标识符一致。
                self.advance();
                Ok(Expr::Identifier("self".into()))
            }
            TokenKind::Super => {
                self.advance();
                self.expect(TokenKind::Dot, "expected '.' after 'super'")?;
                let name = match &self.peek().kind {
                    TokenKind::Identifier(n) => n.clone(),
                    _ => {
                        let tok = self.peek();
                        return Err(MspError::ParseError {
                            line: tok.span.start.line,
                            column: tok.span.start.column,
                            message: "expected method name after 'super.'".into(),
                        });
                    }
                };
                self.advance();
                Ok(Expr::SuperAccess { name })
            }
            TokenKind::LeftParen => self.parse_grouping_or_tuple(),
            TokenKind::LeftBracket => self.parse_list_literal(),
            TokenKind::LeftBrace => self.parse_dict_or_set(),
            TokenKind::Fn if self.is_fn_literal() => self.parse_fn_literal(),
            TokenKind::Yield => self.parse_yield_expr(),
            TokenKind::Await => {
                self.advance();
                let expr = self.parse_power()?;
                Ok(Expr::Await {
                    expr: Box::new(expr),
                })
            }
            TokenKind::Go => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Go {
                    expr: Box::new(expr),
                })
            }
            _ => {
                let tok = self.peek();
                Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: format!("unexpected token: {}", tok.kind),
                })
            }
        }
    }

    // ---- 辅助方法 ----

    /// 函数调用参数列表（不含外层括号）。完整归属 task 13；
    /// 此处提供最小可用实现以支持 Call 后缀解析。
    fn parse_arguments(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        self.skip_newlines();
        if !self.check(&TokenKind::RightParen) {
            loop {
                self.skip_newlines();
                args.push(self.parse_assignment()?);
                self.skip_newlines();
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        self.skip_newlines();
        Ok(args)
    }

    /// 分组 `(expr)` 与元组 `(a, b, ...)`。完整归属集合字面量解析 task；
    /// 此处提供最小可用实现以支持分组表达式（spec 测试 test_not_unary 需要）。
    fn parse_grouping_or_tuple(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LeftParen, "expected '('")?;
        self.skip_newlines();
        if self.check(&TokenKind::RightParen) {
            self.advance();
            return Ok(Expr::TupleLiteral { elements: vec![] });
        }
        let first = self.parse_expression()?;
        self.skip_newlines();
        if self.match_token(&[TokenKind::Comma]) {
            let mut elements = vec![first];
            self.skip_newlines();
            while !self.check(&TokenKind::RightParen) {
                elements.push(self.parse_expression()?);
                self.skip_newlines();
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
                self.skip_newlines();
            }
            self.expect(TokenKind::RightParen, "expected ')' after tuple")?;
            return Ok(Expr::TupleLiteral { elements });
        }
        self.skip_newlines();
        self.expect(TokenKind::RightParen, "expected ')' after expression")?;
        Ok(Expr::Grouping {
            expr: Box::new(first),
        })
    }

    /// 区分 `fn name(` 声明与 `fn(` 字面量：
    /// 当前为 Fn 且下一个 token 为 LeftParen 则为字面量。
    fn is_fn_literal(&self) -> bool {
        if !self.check(&TokenKind::Fn) {
            return false;
        }
        let next = self.tokens.get(self.current + 1);
        matches!(next.map(|t| &t.kind), Some(TokenKind::LeftParen))
    }

    /// 判断 `[` 之后的内容是否为切片（含顶层 `:`）而非纯下标。
    /// 调用前 `[` 已被消费；扫描至匹配 `]`，若在嵌套深度 0 处遇到 `:` 则为切片。
    fn is_slice(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.current;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::LeftBracket | TokenKind::LeftParen | TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBracket if depth == 0 => return false,
                TokenKind::RightBracket | TokenKind::RightParen | TokenKind::RightBrace => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                TokenKind::Colon if depth == 0 => return true,
                _ => {}
            }
            i += 1;
        }
        false
    }

    // ---- 以下初等表达式由后续 task 实现，当前为占位 ----

    fn parse_list_literal(&mut self) -> Result<Expr> {
        self.unimplemented_expr("parse_list_literal")
    }

    fn parse_dict_or_set(&mut self) -> Result<Expr> {
        self.unimplemented_expr("parse_dict_or_set")
    }

    fn parse_fn_literal(&mut self) -> Result<Expr> {
        self.unimplemented_expr("parse_fn_literal")
    }

    fn parse_yield_expr(&mut self) -> Result<Expr> {
        self.unimplemented_expr("parse_yield_expr")
    }

    fn parse_slice(&mut self, _object: Expr) -> Result<Expr> {
        self.unimplemented_expr("parse_slice")
    }
}

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::ast::{BinaryOp, Expr, Literal, UnaryOp};
    use crate::error::Result;
    use crate::lexer::Lexer;

    fn parse_expr(source: &str) -> Result<Expr> {
        let tokens = Lexer::new(source).tokenize_all()?;
        let mut parser = Parser::new(tokens);
        parser.parse_expression()
    }

    #[test]
    fn test_precedence_mul_over_add() {
        let expr = parse_expr("1 + 2 * 3").unwrap();
        match expr {
            Expr::Binary {
                left,
                op: BinaryOp::Add,
                right,
            } => {
                assert!(matches!(*left, Expr::Literal(Literal::Int(1))));
                assert!(matches!(
                    *right,
                    Expr::Binary {
                        op: BinaryOp::Multiply,
                        ..
                    }
                ));
            }
            _ => panic!("expected add at top level"),
        }
    }

    #[test]
    fn test_power_right_assoc() {
        let expr = parse_expr("2 ** 3 ** 2").unwrap();
        match expr {
            Expr::Binary {
                left: _,
                op: BinaryOp::Power,
                right,
            } => {
                assert!(matches!(
                    *right,
                    Expr::Binary {
                        op: BinaryOp::Power,
                        ..
                    }
                ));
            }
            _ => panic!("expected power at top level"),
        }
    }

    #[test]
    fn test_and_or_precedence() {
        let expr = parse_expr("a > 1 and b < 512").unwrap();
        assert!(matches!(
            expr,
            Expr::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
    }

    #[test]
    fn test_ternary() {
        let expr = parse_expr("\"yes\" if c else \"no\"").unwrap();
        assert!(matches!(expr, Expr::Ternary { .. }));
    }

    #[test]
    fn test_not_unary() {
        let expr = parse_expr("not (a > 10)").unwrap();
        assert!(matches!(
            expr,
            Expr::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
    }

    #[test]
    fn test_chained_comparison() {
        let expr = parse_expr("1 < a < 10").unwrap();
        // 解析阶段反糖为 (1 < a) and (a < 10)
        match expr {
            Expr::Binary {
                left,
                op: BinaryOp::And,
                right,
            } => {
                assert!(matches!(
                    *left,
                    Expr::Binary {
                        op: BinaryOp::Less,
                        ..
                    }
                ));
                assert!(matches!(
                    *right,
                    Expr::Binary {
                        op: BinaryOp::Less,
                        ..
                    }
                ));
            }
            _ => panic!("expected and-desugared chained comparison"),
        }
    }

    #[test]
    fn test_postfix_chain() {
        // 验证后缀表达式正确链接：obj.method(args)[index]（验证标准 5）
        let expr = parse_expr("obj.method(42)[0]").unwrap();
        match expr {
            Expr::Index { object, .. } => {
                assert!(matches!(*object, Expr::Call { .. }));
            }
            _ => panic!("expected index over call"),
        }
    }
}
