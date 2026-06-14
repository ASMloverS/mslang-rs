pub mod token;

use crate::error::{MspError, Result};
use crate::lexer::token::{keyword_table, Position, Span, Token, TokenKind};

pub struct Lexer {
    #[allow(dead_code)]  // 保留用于调试/错误报告；chars 用于所有字符访问
    source: String,
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Lexer {
        let normalized = source.replace("\r\n", "\n");
        Lexer {
            source: normalized.clone(),
            chars: normalized.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    #[allow(dead_code)]  // task 04+ 多字符前瞻将使用
    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if let Some(c) = ch {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == ' ' || c == '\t' || c == '\r' {  // \r: 防御性处理裸 \r（标准仅定义 \r\n 归一化）
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        // 注意：此方法在 '\n' 处停止但不消费换行符。
        // next_token() 中注释后递归调用会处理该换行。
        // 大量连续注释行会导致递归深度增加（已知限制）。
        while let Some(c) = self.peek_char() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    pub fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace();

        let start = self.current_position();

        let ch = match self.advance() {
            Some(c) => c,
            None => return Ok(self.make_token(TokenKind::Eof, start, "")),
        };

        match ch {
            '\n' => Ok(self.make_token(TokenKind::Newline, start, "\n")),
            '#' => { self.skip_comment(); self.next_token() }
            c if c.is_ascii_digit() => self.read_number(c, start),
            '"' => self.read_string(start),
            c if c.is_ascii_alphabetic() || c == '_' => self.read_identifier(c, start),
            '+' => self.read_plus(start),
            '-' => self.read_minus(start),
            '*' => self.read_star(start),
            '/' => self.read_slash(start),
            '%' => self.read_percent(start),
            '=' => self.read_equal(start),
            '!' => self.read_bang(start),
            '<' => self.read_less(start),
            '>' => self.read_greater(start),
            '&' => self.read_ampersand(start),
            '|' => self.read_pipe(start),
            '^' => self.read_caret(start),
            '~' => Ok(self.make_token(TokenKind::Tilde, start, "~")),
            '(' => Ok(self.make_token(TokenKind::LeftParen, start, "(")),
            ')' => Ok(self.make_token(TokenKind::RightParen, start, ")")),
            '[' => Ok(self.make_token(TokenKind::LeftBracket, start, "[")),
            ']' => Ok(self.make_token(TokenKind::RightBracket, start, "]")),
            '{' => Ok(self.make_token(TokenKind::LeftBrace, start, "{")),
            '}' => Ok(self.make_token(TokenKind::RightBrace, start, "}")),
            ',' => Ok(self.make_token(TokenKind::Comma, start, ",")),
            '.' => self.read_dot(start),
            ':' => self.read_colon(start),
            ';' => Ok(self.make_token(TokenKind::Semicolon, start, ";")),
            '@' => Ok(self.make_token(TokenKind::At, start, "@")),
            _ => {
                Err(MspError::LexError {
                    line: start.line,
                    column: start.column,
                    message: format!("unexpected character '{}'", ch),
                })
            }
        }
    }

    pub fn tokenize_all(mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof { break; }
        }
        Ok(tokens)
    }

    fn make_token(&self, kind: TokenKind, start: Position, lexeme: &str) -> Token {
        Token {
            kind,
            lexeme: lexeme.to_string(),
            span: Span {
                start,
                end: self.current_position(),
            },
        }
    }

    fn current_position(&self) -> Position {
        Position {
            line: self.line,
            column: self.column,
            offset: self.pos,
        }
    }

    fn read_identifier(&mut self, first: char, start: Position) -> Result<Token> {
        let mut lexeme = String::new();
        lexeme.push(first);
        while let Some(c) = self.peek_char() {
            if c.is_ascii_alphanumeric() || c == '_' {
                lexeme.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let kind = keyword_table()
            .get(lexeme.as_str())
            .cloned()
            .unwrap_or_else(|| TokenKind::Identifier(lexeme.clone()));
        Ok(self.make_token(kind, start, &lexeme))
    }

    fn read_number(&mut self, first: char, start: Position) -> Result<Token> {
        let mut lexeme = String::new();
        lexeme.push(first);
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                lexeme.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let value: i64 = lexeme.parse().map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid integer literal '{}'", lexeme),
        })?;
        Ok(self.make_token(TokenKind::Int(value), start, &lexeme))
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

    fn read_string(&mut self, start: Position) -> Result<Token> {
        Err(MspError::LexError {
            line: start.line,
            column: start.column,
            message: "string literals not yet implemented (task 05)".into(),
        })
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

    // 以下方法在 task 03 中返回基础单字符 token。
    // task 07 将增强为完整的多字符运算符匹配（+=, -=, **, //, <<, >>=, .., ... 等）。
    fn read_plus(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Plus, start, "+")) }
    fn read_minus(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Minus, start, "-")) }
    fn read_star(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Star, start, "*")) }
    fn read_slash(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Slash, start, "/")) }
    fn read_percent(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Percent, start, "%")) }
    fn read_less(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Less, start, "<")) }
    fn read_greater(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Greater, start, ">")) }
    fn read_ampersand(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Ampersand, start, "&")) }
    fn read_pipe(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Pipe, start, "|")) }
    fn read_caret(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Caret, start, "^")) }
    fn read_dot(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Dot, start, ".")) }
    fn read_colon(&mut self, start: Position) -> Result<Token> { Ok(self.make_token(TokenKind::Colon, start, ":")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::TokenKind;

    #[test]
    fn test_comment_and_assignment() {
        let source = "# this is a comment\nx = 10\n";
        let lexer = Lexer::new(source);
        let tokens = lexer.tokenize_all().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert!(kinds.contains(&TokenKind::Identifier("x".into())));
        assert!(kinds.contains(&TokenKind::Equal));
        assert!(kinds.contains(&TokenKind::Int(10)));
        assert!(kinds.contains(&TokenKind::Eof));
    }

    #[test]
    fn test_unexpected_char() {
        let source = "x $ y";
        let mut lexer = Lexer::new(source);
        let first = lexer.next_token();
        assert!(first.is_ok());
        let second = lexer.next_token();
        assert!(second.is_err());
    }

    #[test]
    fn test_newline_normalization() {
        let source = "x\r\ny\n";
        let lexer = Lexer::new(source);
        let tokens = lexer.tokenize_all().unwrap();
        let newline_count = tokens.iter()
            .filter(|t| t.kind == TokenKind::Newline)
            .count();
        assert_eq!(newline_count, 2);
    }

    #[test]
    fn test_keyword_vs_identifier() {
        let source = "var myvar";
        let lexer = Lexer::new(source);
        let tokens = lexer.tokenize_all().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert!(kinds.contains(&TokenKind::Var));
        assert!(kinds.contains(&TokenKind::Identifier("myvar".into())));
    }
}
