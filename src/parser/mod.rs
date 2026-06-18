// 语法分析器核心框架（task 11）。
// 递归下降 Parser：token 流遍历、语句分发、块解析、错误恢复原语。
// 语句/表达式的实际解析由 task 12-15 替换下方 stub 实现。
// 完整规格见 docs/mslang/tasks/11-parser-core.md。

use crate::ast::{Expr, Program, Stmt};
use crate::error::{MspError, Result};
use crate::lexer::token::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Parser {
        Parser { tokens, current: 0 }
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current.saturating_sub(1)]
    }

    fn is_at_end(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn check(&self, kind: &TokenKind) -> bool {
        if self.is_at_end() {
            return false;
        }
        self.peek().kind == *kind
    }

    // task 12-15 将频繁使用；task 11 无调用方故标记。
    #[allow(dead_code)]
    fn match_token(&mut self, kinds: &[TokenKind]) -> bool {
        for kind in kinds {
            if self.check(kind) {
                self.advance();
                return true;
            }
        }
        false
    }

    // task 12-15 的块解析将调用；task 11 经 parse_block 测试覆盖，非测试构建无调用方。
    #[allow(dead_code)]
    fn expect(&mut self, kind: TokenKind, message: &str) -> Result<&Token> {
        if self.check(&kind) {
            return Ok(self.advance());
        }
        let tok = self.peek();
        Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: message.into(),
        })
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) {
            self.advance();
        }
    }

    pub fn parse(mut self) -> Result<Program> {
        let mut statements = Vec::new();
        self.skip_newlines();

        while !self.is_at_end() {
            self.skip_newlines();
            if self.is_at_end() {
                break;
            }
            statements.push(self.parse_statement()?);
            self.skip_newlines();
        }

        Ok(Program { statements })
    }

    fn parse_statement(&mut self) -> Result<Stmt> {
        self.skip_newlines();

        if self.check(&TokenKind::Var) {
            self.parse_var_decl()
        } else if self.check(&TokenKind::Const) {
            self.parse_const_decl()
        } else if self.check(&TokenKind::Fn) {
            self.parse_fn_or_expr()
        } else if self.check(&TokenKind::If) {
            self.parse_if()
        } else if self.check(&TokenKind::While) {
            self.parse_while()
        } else if self.check(&TokenKind::For) {
            self.parse_for()
        } else if self.check(&TokenKind::Break) {
            self.advance();
            self.consume_newline();
            Ok(Stmt::Break)
        } else if self.check(&TokenKind::Continue) {
            self.advance();
            self.consume_newline();
            Ok(Stmt::Continue)
        } else if self.check(&TokenKind::Return) {
            self.parse_return()
        } else if self.check(&TokenKind::Import) {
            self.parse_import()
        } else if self.check(&TokenKind::From) {
            self.parse_from_import()
        } else if self.check(&TokenKind::Class) {
            self.parse_class()
        } else if self.check(&TokenKind::Defer) {
            self.parse_defer()
        } else if self.check(&TokenKind::Try) {
            self.parse_try()
        } else if self.check(&TokenKind::With) {
            self.parse_with()
        } else if self.check(&TokenKind::Throw) {
            self.parse_throw()
        } else {
            self.parse_expr_or_assignment()
        }
    }

    // task 13（parse_if 等）与 task 14（匿名函数体）调用；task 11 经测试覆盖。
    #[allow(dead_code)]
    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.expect(TokenKind::LeftBrace, "expected '{'")?;
        self.skip_newlines();

        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            self.skip_newlines();
            if self.check(&TokenKind::RightBrace) {
                break;
            }
            statements.push(self.parse_statement()?);
            self.skip_newlines();
        }

        self.expect(TokenKind::RightBrace, "expected '}'")?;
        Ok(statements)
    }

    fn consume_newline(&mut self) {
        if self.check(&TokenKind::Newline) {
            self.advance();
        }
    }

    // 错误恢复（panic mode）原语：供 REPL/IDE/LSP 调用以收集多个错误。
    #[allow(dead_code)]
    fn synchronize(&mut self) {
        self.advance();

        while !self.is_at_end() {
            if self.previous().kind == TokenKind::Newline {
                return;
            }

            match self.peek().kind {
                TokenKind::Var
                | TokenKind::Const
                | TokenKind::Fn
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Class
                | TokenKind::Return
                | TokenKind::Import
                | TokenKind::From
                | TokenKind::Nonlocal
                | TokenKind::Global
                | TokenKind::Async
                | TokenKind::Try
                | TokenKind::With
                | TokenKind::Defer
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Throw => return,
                _ => {}
            }

            self.advance();
        }
    }

    #[allow(dead_code)]
    fn parse_statement_safe(&mut self) -> Option<Stmt> {
        match self.parse_statement() {
            Ok(stmt) => Some(stmt),
            Err(e) => {
                eprintln!("{}", e);
                self.synchronize();
                None
            }
        }
    }

    // ---- 占位 stubs：由 task 12-15 替换 ----

    // 占位：由 task 13 替换
    fn parse_var_decl(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_var_decl")
    }

    // 占位：由 task 13 替换
    fn parse_const_decl(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_const_decl")
    }

    // 占位：由 task 14 替换
    fn parse_fn_or_expr(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_fn_or_expr")
    }

    // 占位：由 task 13 替换
    fn parse_if(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_if")
    }

    // 占位：由 task 13 替换
    fn parse_while(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_while")
    }

    // 占位：由 task 13 替换
    fn parse_for(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_for")
    }

    // 占位：由 task 13 替换
    fn parse_return(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_return")
    }

    // 占位：由 task 15 替换
    fn parse_import(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_import")
    }

    // 占位：由 task 15 替换
    fn parse_from_import(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_from_import")
    }

    // 占位：由 task 13 替换（裸表达式/赋值）
    fn parse_expr_or_assignment(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_expr_or_assignment")
    }

    // 占位：由 task 15 替换
    fn parse_class(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_class")
    }

    // 占位：由 task 15 替换
    fn parse_defer(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_defer")
    }

    // 占位：由 task 15 替换
    fn parse_try(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_try")
    }

    // 占位：由 task 15 替换
    fn parse_with(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_with")
    }

    // 占位：由 task 15 替换
    fn parse_throw(&mut self) -> Result<Stmt> {
        self.unimplemented("parse_throw")
    }

    // 占位：由 task 12 替换（被 parse_expr_or_assignment 调用）
    #[allow(dead_code)]
    fn parse_expression(&mut self) -> Result<Expr> {
        self.unimplemented_expr("parse_expression")
    }

    fn unimplemented(&mut self, name: &str) -> Result<Stmt> {
        let tok = self.peek();
        Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: format!("{} not yet implemented", name),
        })
    }

    #[allow(dead_code)]
    fn unimplemented_expr(&mut self, name: &str) -> Result<Expr> {
        let tok = self.peek();
        Err(MspError::ParseError {
            line: tok.span.start.line,
            column: tok.span.start.column,
            message: format!("{} not yet implemented", name),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::ast::{Program, Stmt};
    use crate::error::{MspError, Result};
    use crate::lexer::token::TokenKind;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Result<Program> {
        let tokens = Lexer::new(source).tokenize_all()?;
        let parser = Parser::new(tokens);
        parser.parse()
    }

    fn parser_from(source: &str) -> Parser {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        Parser::new(tokens)
    }

    // ---- task 13 回归基线：依赖实际解析，stub 阶段返回错误故暂忽略 ----

    #[test]
    #[ignore = "task 13: requires parse_expr_or_assignment/parse_expression"]
    fn test_simple_program() {
        let prog = parse("x = 10\ny = 20\nprint(x + y)\n").unwrap();
        assert_eq!(prog.statements.len(), 3);
    }

    #[test]
    #[ignore = "task 13: requires parse_if/parse_block with real statement parsing"]
    fn test_block() {
        let prog = parse("if true {\n    x = 1\n}\n").unwrap();
        assert_eq!(prog.statements.len(), 1);
        match &prog.statements[0] {
            Stmt::If { then_block, .. } => {
                assert_eq!(then_block.len(), 1);
            }
            _ => panic!("expected if statement"),
        }
    }

    #[test]
    #[ignore = "task 13: requires parse_expr_or_assignment/parse_expression"]
    fn test_newline_handling() {
        let prog = parse("x = 1\n\n\ny = 2\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
    }

    // ---- 框架原语（task 11 验证范围）----

    #[test]
    fn test_empty_program() {
        let prog = parse("").unwrap();
        assert!(prog.statements.is_empty());
    }

    #[test]
    fn test_only_newlines() {
        let prog = parse("\n\n\n").unwrap();
        assert!(prog.statements.is_empty());
    }

    #[test]
    fn test_parse_error() {
        // `if` 走 parse_if stub，返回 ParseError 而非 panic。
        let result = parse("if {\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_break_continue_dispatch() {
        // break/continue 由 parse_statement 直接处理（非 stub），验证分发 + consume_newline。
        let prog = parse("break\ncontinue\n").unwrap();
        assert_eq!(prog.statements.len(), 2);
        assert!(matches!(prog.statements[0], Stmt::Break));
        assert!(matches!(prog.statements[1], Stmt::Continue));
    }

    #[test]
    fn test_break_at_eof_no_newline() {
        // consume_newline 在 EOF 时安全无操作（不 panic）。
        let prog = parse("break").unwrap();
        assert_eq!(prog.statements.len(), 1);
        assert!(matches!(prog.statements[0], Stmt::Break));
    }

    #[test]
    fn test_parse_block_empty() {
        let mut p = parser_from("{}\n");
        let block = p.parse_block().unwrap();
        assert!(block.is_empty());
    }

    #[test]
    fn test_parse_block_missing_close() {
        let mut p = parser_from("{\n");
        assert!(p.parse_block().is_err());
    }

    #[test]
    fn test_parse_block_with_break() {
        // 块内 break 由 parse_statement 直接处理；验证块循环 + skip_newlines 边界。
        let mut p = parser_from("{\nbreak\n}\n");
        let block = p.parse_block().unwrap();
        assert_eq!(block.len(), 1);
        assert!(matches!(block[0], Stmt::Break));
    }

    #[test]
    fn test_match_token() {
        let mut p = parser_from("break\n");
        assert!(p.match_token(&[TokenKind::Continue, TokenKind::Break]));
        // 命中后已前移到 Newline。
        assert!(p.check(&TokenKind::Newline));

        let mut p2 = parser_from("x\n");
        assert!(!p2.match_token(&[TokenKind::Break]));
        // 未命中时不前移。
        assert!(matches!(p2.peek().kind, TokenKind::Identifier(_)));
    }

    #[test]
    fn test_synchronize_to_newline() {
        // var 走 stub 返回错误；synchronize 跳到换行后的下一语句边界。
        let mut p = parser_from("var x\ny = 2\n");
        assert!(p.parse_statement().is_err());
        p.synchronize();
        assert!(matches!(&p.peek().kind, TokenKind::Identifier(s) if s == "y"));
    }

    #[test]
    fn test_parse_expression_stub() {
        let mut p = parser_from("x\n");
        match p.parse_expression() {
            Err(MspError::ParseError { message, .. }) => {
                assert!(message.contains("parse_expression not yet implemented"));
            }
            _ => panic!("expected ParseError from parse_expression stub"),
        }
    }
}
