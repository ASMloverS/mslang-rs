//! 语句解析（task 13）：变量/常量声明、赋值（含多目标与短声明）、控制流
//! （if/while/for）、break/continue/return、函数声明、nonlocal/global 声明。
//! 完整规格见 docs/mslang/tasks/13-parser-statements.md。
//!
//! 分发目标方法（parse_var_decl 等供 parse_statement 调用）声明为 pub(super)；
//! 内部辅助方法保持私有。is_fn_literal 由 task 12（expression.rs）提供，此处复用。

use crate::ast::{AssignOp, ExceptClause, Expr, Param, Stmt};
use crate::error::{MspError, Result};
use crate::lexer::token::TokenKind;

use super::Parser;

impl Parser {
    // ---- 声明 ----

    pub(super) fn parse_var_decl(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'var'
        let name = self.expect_identifier("expected variable name after 'var'")?;
        self.expect(TokenKind::Equal, "expected '=' after variable name")?;
        let initializer = self.parse_expression()?;
        self.consume_newline();
        Ok(Stmt::VarDecl { name, initializer })
    }

    pub(super) fn parse_const_decl(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'const'
        let name = self.expect_identifier("expected constant name after 'const'")?;
        self.expect(TokenKind::Equal, "expected '=' after constant name")?;
        let initializer = self.parse_expression()?;
        self.consume_newline();
        Ok(Stmt::ConstDecl { name, initializer })
    }

    // ---- 赋值 / 短声明 / 表达式语句 ----

    pub(super) fn parse_expr_or_assignment(&mut self) -> Result<Stmt> {
        let expr = self.parse_expression()?;

        // 多目标赋值：a, b = 1, 2（03-syntax.md:140-151）。
        // parse_expression 不消费逗号（逗号非二元运算符），故 expr 是首个 target。
        if self.check(&TokenKind::Comma) {
            return self.parse_multi_assign(expr);
        }

        // 短声明：IDENTIFIER := expression
        if self.check(&TokenKind::ColonEqual) {
            self.advance();
            let value = self.parse_expression()?;
            self.consume_newline();
            if let Expr::Identifier(name) = expr {
                return Ok(Stmt::ShortVarDecl {
                    name,
                    initializer: value,
                });
            }
            let tok = self.previous();
            return Err(MspError::ParseError {
                line: tok.span.start.line,
                column: tok.span.start.column,
                message: "invalid target for :=".into(),
            });
        }

        // 赋值表达式（含复合赋值）：parse_assignment 产出 Expr::Assign。
        if let Expr::Assign { target, op, value } = expr {
            // lvalue 校验（03-syntax.md:128-131）。
            if !Self::is_valid_lvalue(&target) {
                let tok = self.previous();
                return Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "invalid assignment target".into(),
                });
            }
            self.consume_newline();
            if op == AssignOp::Assign {
                if let Expr::Identifier(name) = *target {
                    return Ok(Stmt::VarDecl {
                        name,
                        initializer: *value,
                    });
                }
            }
            return Ok(Stmt::Assign {
                target: *target,
                op,
                value: *value,
            });
        }

        self.consume_newline();
        Ok(Stmt::ExprStmt { expr })
    }

    /// 多目标赋值：first_target 为已解析的第一个目标。
    ///
    /// target 必须用 parse_ternary 解析（而非 parse_expression）：parse_expression
    /// 经 parse_assignment 会消费 `=`，导致 `b` 误吞 `b = 1`。target 不是赋值表达式，
    /// 用赋值之下的优先级入口即可；lvalue 合法性由 is_valid_lvalue 校验。
    fn parse_multi_assign(&mut self, first_target: Expr) -> Result<Stmt> {
        let mut targets = vec![first_target];
        while self.match_token(&[TokenKind::Comma]) {
            targets.push(self.parse_ternary()?);
        }
        self.expect(TokenKind::Equal, "expected '=' in multi-assignment")?;

        let mut values = vec![self.parse_expression()?];
        while self.match_token(&[TokenKind::Comma]) {
            values.push(self.parse_expression()?);
        }
        self.consume_newline();

        for t in &targets {
            if !Self::is_valid_lvalue(t) {
                let tok = self.previous();
                return Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "invalid assignment target in multi-assignment".into(),
                });
            }
        }

        Ok(Stmt::Assign {
            target: Expr::TupleLiteral { elements: targets },
            op: AssignOp::Assign,
            value: Expr::TupleLiteral { elements: values },
        })
    }

    /// 合法赋值目标：标识符、属性访问、下标访问（03-syntax.md:128-131）。
    fn is_valid_lvalue(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Identifier(_) | Expr::Dot { .. } | Expr::Index { .. }
        )
    }

    // ---- 控制流 ----

    pub(super) fn parse_if(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'if'
        let condition = self.parse_expression()?;
        let then_block = self.parse_block()?;
        self.skip_newlines();

        let mut elif_clauses = Vec::new();
        while self.match_token(&[TokenKind::Elif]) {
            let cond = self.parse_expression()?;
            let block = self.parse_block()?;
            self.skip_newlines();
            elif_clauses.push((cond, block));
        }

        let mut else_block = None;
        if self.match_token(&[TokenKind::Else]) {
            else_block = Some(self.parse_block()?);
            self.skip_newlines();
        }

        Ok(Stmt::If {
            condition,
            then_block,
            elif_clauses,
            else_block,
        })
    }

    pub(super) fn parse_while(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'while'
        let condition = self.parse_expression()?;
        let body = self.parse_block()?;
        Ok(Stmt::While { condition, body })
    }

    pub(super) fn parse_for(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'for'
        let first_var = self.expect_identifier("expected variable name after 'for'")?;

        let (variable, second_variable) = if self.match_token(&[TokenKind::Comma]) {
            let second = self.expect_identifier("expected second variable name")?;
            (first_var, Some(second))
        } else {
            (first_var, None)
        };

        self.expect(TokenKind::In, "expected 'in' after for variable")?;
        let iterable = self.parse_expression()?;
        let body = self.parse_block()?;

        Ok(Stmt::ForIn {
            variable,
            second_variable,
            iterable,
            body,
        })
    }

    pub(super) fn parse_return(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'return'
        let mut values = Vec::new();

        if !self.check(&TokenKind::Newline)
            && !self.check(&TokenKind::RightBrace)
            && !self.is_at_end()
        {
            values.push(self.parse_expression()?);
            while self.match_token(&[TokenKind::Comma]) {
                values.push(self.parse_expression()?);
            }
        }

        self.consume_newline();
        Ok(Stmt::Return { values })
    }

    // ---- 函数声明 ----

    /// 区分 `fn name(...)` 声明与 `fn(...)` 匿名函数字面量。
    pub(super) fn parse_fn_or_expr(&mut self) -> Result<Stmt> {
        if self.is_fn_literal() {
            let expr = self.parse_expression()?;
            self.consume_newline();
            Ok(Stmt::ExprStmt { expr })
        } else {
            self.parse_fn_decl()
        }
    }

    fn parse_fn_decl(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'fn'
        let name = self.expect_identifier("expected function name")?;
        self.expect(TokenKind::LeftParen, "expected '(' after function name")?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RightParen, "expected ')' after parameters")?;
        let body = self.parse_block()?;

        Ok(Stmt::FnDecl {
            name,
            params,
            body,
            is_async: false,
        })
    }

    pub(super) fn parse_param_list(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if self.check(&TokenKind::RightParen) {
            return Ok(params);
        }

        loop {
            if self.match_token(&[TokenKind::Star]) {
                let name = self.expect_identifier("expected parameter name after '*'")?;
                params.push(Param {
                    name,
                    default: None,
                    is_variadic: true,
                });
            } else {
                let name = self.parse_param_name()?;
                let default = if self.match_token(&[TokenKind::Equal]) {
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                params.push(Param {
                    name,
                    default,
                    is_variadic: false,
                });
            }

            if !self.match_token(&[TokenKind::Comma]) {
                break;
            }
        }

        Ok(params)
    }

    fn parse_param_name(&mut self) -> Result<String> {
        let kind = self.peek().kind.clone();
        match &kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            TokenKind::Zelf => {
                self.advance();
                Ok("self".to_string())
            }
            _ => {
                let tok = self.peek();
                Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "expected parameter name".into(),
                })
            }
        }
    }

    // ---- nonlocal / global ----

    pub(super) fn parse_nonlocal(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'nonlocal'
        Ok(Stmt::Nonlocal {
            names: self.parse_name_list("expected identifier after 'nonlocal'")?,
        })
    }

    pub(super) fn parse_global(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'global'
        Ok(Stmt::Global {
            names: self.parse_name_list("expected identifier after 'global'")?,
        })
    }

    /// 逗号分隔的标识符列表（nonlocal/global 声明共用）。
    fn parse_name_list(&mut self, first_msg: &str) -> Result<Vec<String>> {
        let mut names = vec![self.expect_identifier(first_msg)?];
        while self.match_token(&[TokenKind::Comma]) {
            names.push(self.expect_identifier("expected identifier after ','")?);
        }
        self.consume_newline();
        Ok(names)
    }

    // ---- 辅助 ----

    pub(super) fn expect_identifier(&mut self, msg: &str) -> Result<String> {
        let kind = self.peek().kind.clone();
        match &kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => {
                let tok = self.peek();
                Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: msg.into(),
                })
            }
        }
    }

    // ---- 高级语句（task 15）----

    pub(super) fn parse_defer(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'defer'
        let expr = self.parse_expression()?;
        self.consume_newline();
        Ok(Stmt::Defer { expr })
    }

    pub(super) fn parse_try(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'try'
        let try_block = self.parse_block()?;
        self.skip_newlines();

        let mut except_clauses = Vec::new();
        while self.match_token(&[TokenKind::Except]) {
            let type_name = if self.peek().is_identifier() {
                match &self.peek().kind {
                    TokenKind::Identifier(name) => {
                        let mut path = vec![name.clone()];
                        self.advance();
                        while self.match_token(&[TokenKind::Dot]) {
                            if let TokenKind::Identifier(n) = &self.peek().kind {
                                path.push(n.clone());
                                self.advance();
                            }
                        }
                        Some(path)
                    }
                    _ => None,
                }
            } else {
                None
            };

            let alias = if self.match_token(&[TokenKind::As]) {
                Some(self.expect_identifier("expected variable name after 'as'")?)
            } else {
                None
            };

            let body = self.parse_block()?;
            self.skip_newlines();

            except_clauses.push(ExceptClause {
                type_name,
                alias,
                body,
            });
        }

        let finally_block = if self.match_token(&[TokenKind::Finally]) {
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(Stmt::Try {
            try_block,
            except_clauses,
            finally_block,
        })
    }

    pub(super) fn parse_throw(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'throw'

        // 支持 bare throw（重新抛出当前异常）
        if self.check(&TokenKind::Newline)
            || self.check(&TokenKind::RightBrace)
            || self.is_at_end()
        {
            self.consume_newline();
            return Ok(Stmt::Throw { expr: None });
        }

        let expr = self.parse_expression()?;
        self.consume_newline();
        Ok(Stmt::Throw { expr: Some(expr) })
    }

    pub(super) fn parse_with(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'with'
        let expression = self.parse_expression()?;

        let alias = if self.match_token(&[TokenKind::As]) {
            Some(self.expect_identifier("expected variable name after 'as'")?)
        } else {
            None
        };

        let body = self.parse_block()?;
        Ok(Stmt::With {
            expression,
            alias,
            body,
        })
    }

    pub(super) fn parse_class(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'class'
        let name = self.expect_identifier("expected class name")?;

        let parent = if self.match_token(&[TokenKind::Less]) {
            Some(self.expect_identifier("expected parent class name")?)
        } else {
            None
        };

        self.expect(TokenKind::LeftBrace, "expected '{' after class name")?;
        self.skip_newlines();

        let mut methods = Vec::new();
        let mut class_vars = Vec::new();

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            self.skip_newlines();
            if self.check(&TokenKind::RightBrace) {
                break;
            }

            if self.check(&TokenKind::Fn) {
                methods.push(self.parse_class_method()?);
            } else {
                let _is_var = self.match_token(&[TokenKind::Var]);
                let var_name = self.expect_identifier("expected class variable or method name")?;
                self.expect(TokenKind::Equal, "expected '=' in class variable")?;
                let value = self.parse_expression()?;
                class_vars.push((var_name, value));
            }
            self.skip_newlines();
        }

        self.expect(TokenKind::RightBrace, "expected '}'")?;
        Ok(Stmt::ClassDecl {
            name,
            parent,
            methods,
            class_vars,
        })
    }

    fn parse_class_method(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'fn'
        let name = self.expect_identifier("expected method name")?;
        self.expect(TokenKind::LeftParen, "expected '(' after method name")?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RightParen, "expected ')'")?;
        let body = self.parse_block()?;
        Ok(Stmt::FnDecl {
            name,
            params,
            body,
            is_async: false,
        })
    }

    pub(super) fn parse_async_fn(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'async'
        let mut fn_decl = self.parse_fn_decl()?;
        if let Stmt::FnDecl { is_async, .. } = &mut fn_decl {
            *is_async = true;
        }
        Ok(fn_decl)
    }

    pub(super) fn parse_import(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'import'
        let is_stdlib = self.match_token(&[TokenKind::At]);
        if is_stdlib {
            let std_tok = self.peek();
            match &std_tok.kind {
                TokenKind::Identifier(name) if name == "std" => {
                    self.advance();
                }
                _ => {
                    return Err(MspError::ParseError {
                        line: std_tok.span.start.line,
                        column: std_tok.span.start.column,
                        message: "expected 'std' after '@' in import".into(),
                    });
                }
            }
        }
        let module_path = self.parse_module_path()?;
        let alias = if self.match_token(&[TokenKind::As]) {
            Some(self.expect_identifier("expected alias name")?)
        } else {
            None
        };
        self.consume_newline();
        Ok(Stmt::Import {
            module_path,
            alias,
            is_stdlib,
        })
    }

    pub(super) fn parse_from_import(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'from'
        let is_stdlib = self.match_token(&[TokenKind::At]);
        if is_stdlib {
            let std_tok = self.peek();
            match &std_tok.kind {
                TokenKind::Identifier(name) if name == "std" => {
                    self.advance();
                }
                _ => {
                    return Err(MspError::ParseError {
                        line: std_tok.span.start.line,
                        column: std_tok.span.start.column,
                        message: "expected 'std' after '@' in from-import".into(),
                    });
                }
            }
        }
        let module_path = self.parse_module_path()?;
        self.expect(TokenKind::Import, "expected 'import' after module path")?;

        let mut targets = Vec::new();
        loop {
            let name = self.expect_identifier("expected import name")?;
            let alias = if self.match_token(&[TokenKind::As]) {
                Some(self.expect_identifier("expected alias name")?)
            } else {
                None
            };
            targets.push((name, alias));
            if !self.match_token(&[TokenKind::Comma]) {
                break;
            }
        }

        self.consume_newline();
        Ok(Stmt::FromImport {
            module_path,
            targets,
            is_stdlib,
        })
    }

    fn parse_module_path(&mut self) -> Result<Vec<String>> {
        let mut path = vec![self.expect_identifier("expected module name")?];
        while self.match_token(&[TokenKind::Dot]) {
            path.push(self.expect_identifier("expected module name after '.'")?);
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::ast::{AssignOp, Expr, Program, Stmt};
    use crate::error::Result;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Result<Program> {
        let tokens = Lexer::new(source).tokenize_all()?;
        Parser::new(tokens).parse()
    }

    #[test]
    fn test_const_and_var() {
        let prog =
            parse("const PI = 3.14159\nvar radius = 10\narea = PI * radius * radius\n").unwrap();
        assert_eq!(prog.statements.len(), 3);
        assert!(matches!(&prog.statements[0], Stmt::ConstDecl { name, .. } if name == "PI"));
        assert!(matches!(&prog.statements[1], Stmt::VarDecl { name, .. } if name == "radius"));
    }

    #[test]
    fn test_short_var_decl() {
        let prog = parse("x := 5\n").unwrap();
        assert!(matches!(
            &prog.statements[0],
            Stmt::ShortVarDecl { name, .. } if name == "x"
        ));
    }

    #[test]
    fn test_if_elif_else() {
        let prog = parse("if x > 0 {\n    print(\"pos\")\n} elif x == 0 {\n    print(\"zero\")\n} else {\n    print(\"neg\")\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::If {
                elif_clauses,
                else_block,
                ..
            } => {
                assert_eq!(elif_clauses.len(), 1);
                assert!(else_block.is_some());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn test_while() {
        let prog = parse("while x > 0 {\n    x = x - 1\n}\n").unwrap();
        assert!(matches!(&prog.statements[0], Stmt::While { .. }));
    }

    #[test]
    fn test_for_in() {
        let prog = parse("for i in range(5) {\n    print(i)\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn {
                variable,
                second_variable,
                ..
            } => {
                assert_eq!(variable, "i");
                assert!(second_variable.is_none());
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_for_in_dual_var() {
        let prog = parse("for k, v in dict.items() {\n    print(k)\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn {
                variable,
                second_variable,
                ..
            } => {
                assert_eq!(variable, "k");
                assert_eq!(second_variable.as_deref(), Some("v"));
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_fn_decl() {
        let prog = parse("fn add(a, b) {\n    return a + b\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl {
                name, params, body, ..
            } => {
                assert_eq!(name, "add");
                assert_eq!(params.len(), 2);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_fn_with_defaults() {
        let prog =
            parse("fn greet(name, prefix = \"Hello\") {\n    return prefix + name\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { params, .. } => {
                assert_eq!(params.len(), 2);
                assert!(params[1].default.is_some());
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_fn_variadic() {
        let prog = parse("fn f(*args) {\n    return args\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { params, .. } => {
                assert_eq!(params.len(), 1);
                assert!(params[0].is_variadic);
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_break_continue() {
        let prog = parse("for i in x {\n    break\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ForIn { body, .. } => {
                assert!(matches!(body[0], Stmt::Break));
            }
            _ => panic!("expected for"),
        }
    }

    #[test]
    fn test_return_no_value() {
        let prog = parse("fn f() {\n    return\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::Return { values } if values.is_empty()));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_multi_return() {
        let prog = parse("fn f() {\n    return 1, 2, 3\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => match &body[0] {
                Stmt::Return { values } => assert_eq!(values.len(), 3),
                _ => panic!("expected return"),
            },
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_compound_assign() {
        let prog = parse("x += 5\ny *= 3\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        assert!(matches!(
            &prog.statements[0],
            Stmt::Assign {
                op: AssignOp::PlusAssign,
                ..
            }
        ));
        assert!(matches!(
            &prog.statements[1],
            Stmt::Assign {
                op: AssignOp::StarAssign,
                ..
            }
        ));
    }

    #[test]
    fn test_attribute_and_index_assign() {
        let prog = parse("obj.attr = 1\narr[0] = 2\n").unwrap();
        assert!(matches!(
            &prog.statements[0],
            Stmt::Assign {
                target: Expr::Dot { .. },
                ..
            }
        ));
        assert!(matches!(
            &prog.statements[1],
            Stmt::Assign {
                target: Expr::Index { .. },
                ..
            }
        ));
    }

    #[test]
    fn test_multi_assign() {
        let prog = parse("a, b = 1, 2\n").unwrap();
        match &prog.statements[0] {
            Stmt::Assign {
                target,
                op: AssignOp::Assign,
                value,
            } => {
                assert!(matches!(target, Expr::TupleLiteral { elements } if elements.len() == 2));
                assert!(matches!(value, Expr::TupleLiteral { elements } if elements.len() == 2));
            }
            _ => panic!("expected multi-assign"),
        }
    }

    #[test]
    fn test_invalid_lvalue() {
        let result = parse("1 + 2 = 3\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_short_decl_target() {
        let result = parse("a + b := 3\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_nonlocal() {
        let prog = parse("fn f() {\n    nonlocal x, y\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::Nonlocal { names } if names.len() == 2));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_global() {
        let prog = parse("fn f() {\n    global counter\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(&body[0], Stmt::Global { names } if names.len() == 1));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_expr_statement() {
        let prog = parse("print(x + y)\n").unwrap();
        assert!(matches!(&prog.statements[0], Stmt::ExprStmt { .. }));
    }

    #[test]
    fn test_integration_example() {
        let src = "const PI = 3.14159\nvar radius = 10\narea = PI * radius * radius\n\nif area > 100 {\n    print(\"big circle\")\n} elif area > 50 {\n    print(\"medium circle\")\n} else {\n    print(\"small circle\")\n}\n\nfor i in range(5) {\n    if i == 3 {\n        continue\n    }\n    print(i)\n}\n";
        let prog = parse(src).unwrap();
        assert_eq!(prog.statements.len(), 5);
        assert!(matches!(&prog.statements[0], Stmt::ConstDecl { name, .. } if name == "PI"));
        assert!(matches!(&prog.statements[2], Stmt::VarDecl { name, .. } if name == "area"));
        assert!(matches!(&prog.statements[4], Stmt::ForIn { body, .. } if body.len() == 2));
    }

    // ---- task 15：高级语句 ----

    #[test]
    fn test_class_basic() {
        let prog = parse("class Animal {\n    kingdom = \"Animalia\"\n    fn speak(self) {\n        return self.name\n    }\n}\n").unwrap();
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0] {
            Stmt::ClassDecl {
                name,
                parent,
                methods,
                class_vars,
            } => {
                assert_eq!(name, "Animal");
                assert!(parent.is_none());
                assert_eq!(methods.len(), 1);
                assert_eq!(class_vars.len(), 1);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn test_class_inheritance() {
        let prog = parse(
            "class Dog < Animal {\n    fn speak(self) {\n        return \"bark\"\n    }\n}\n",
        )
        .unwrap();
        match &prog.statements[0] {
            Stmt::ClassDecl {
                name,
                parent,
                methods,
                ..
            } => {
                assert_eq!(name, "Dog");
                assert_eq!(parent.as_deref(), Some("Animal"));
                assert_eq!(methods.len(), 1);
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn test_defer() {
        let prog = parse("fn f() {\n    defer print(\"done\")\n    x = 1\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(body[0], Stmt::Defer { .. }));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_try_except_finally() {
        let prog = parse("try {\n    x()\n} except ValueError as e {\n    print(e)\n} finally {\n    cleanup()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::Try {
                try_block,
                except_clauses,
                finally_block,
            } => {
                assert_eq!(try_block.len(), 1);
                assert_eq!(except_clauses.len(), 1);
                assert_eq!(
                    except_clauses[0].type_name.as_ref().unwrap(),
                    &vec!["ValueError".to_string()]
                );
                assert_eq!(except_clauses[0].alias.as_deref(), Some("e"));
                assert!(finally_block.is_some());
            }
            _ => panic!("expected try"),
        }
    }

    #[test]
    fn test_try_catch_all() {
        let prog = parse("try {\n    x()\n} except {\n    print(\"error\")\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::Try { except_clauses, .. } => {
                assert_eq!(except_clauses.len(), 1);
                assert!(except_clauses[0].type_name.is_none());
                assert!(except_clauses[0].alias.is_none());
            }
            _ => panic!("expected try"),
        }
    }

    #[test]
    fn test_throw() {
        let prog = parse("throw ValueError(\"bad\")\n").unwrap();
        match &prog.statements[0] {
            Stmt::Throw { .. } => {}
            _ => panic!("expected throw"),
        }
    }

    #[test]
    fn test_with() {
        let prog = parse("with open(\"file.txt\") as f {\n    f.read()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::With {
                expression: _,
                alias,
                body,
            } => {
                assert!(alias.as_deref() == Some("f"));
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected with"),
        }
    }

    #[test]
    fn test_with_no_alias() {
        let prog = parse("with lock.acquire() {\n    work()\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::With { alias, .. } => {
                assert!(alias.is_none());
            }
            _ => panic!("expected with"),
        }
    }

    #[test]
    fn test_yield() {
        let prog = parse("fn gen() {\n    yield 1\n    yield 2\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert_eq!(body.len(), 2);
                assert!(matches!(
                    &body[0],
                    Stmt::ExprStmt {
                        expr: Expr::Yield { value: Some(_) }
                    }
                ));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_yield_from() {
        let prog = parse("fn gen() {\n    yield from items\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(
                    &body[0],
                    Stmt::ExprStmt {
                        expr: Expr::YieldFrom { .. }
                    }
                ));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_bare_yield() {
        let prog = parse("fn gen() {\n    yield\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { body, .. } => {
                assert!(matches!(
                    &body[0],
                    Stmt::ExprStmt {
                        expr: Expr::Yield { value: None }
                    }
                ));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_async_fn() {
        let prog = parse("async fn fetch(url) {\n    return url\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::FnDecl { name, is_async, .. } => {
                assert_eq!(name, "fetch");
                assert!(*is_async);
            }
            _ => panic!("expected async fn"),
        }
    }

    #[test]
    fn test_go_expr() {
        let prog = parse("go worker()\n").unwrap();
        match &prog.statements[0] {
            Stmt::ExprStmt {
                expr: Expr::Go { .. },
            } => {}
            _ => panic!("expected go expression"),
        }
    }

    #[test]
    fn test_class_var_with_var_keyword() {
        let prog = parse("class C {\n    var count = 0\n}\n").unwrap();
        match &prog.statements[0] {
            Stmt::ClassDecl { class_vars, .. } => {
                assert_eq!(class_vars.len(), 1);
                assert_eq!(class_vars[0].0, "count");
            }
            _ => panic!("expected class"),
        }
    }

    #[test]
    fn test_import() {
        let prog = parse("import math\nimport os.path as pathutil\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
    }

    #[test]
    fn test_from_import() {
        let prog = parse("from os import path\nfrom io import open, print as log\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        match &prog.statements[1] {
            Stmt::FromImport { targets, .. } => {
                assert_eq!(targets.len(), 2);
            }
            _ => panic!("expected from import"),
        }
    }
}
