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
        if first == '0' {
            match self.peek_char() {
                Some('x') | Some('X') => {
                    self.advance();
                    return self.read_hex(start);
                }
                Some('b') | Some('B') => {
                    self.advance();
                    return self.read_binary(start);
                }
                Some('o') | Some('O') => {
                    self.advance();
                    return self.read_octal(start);
                }
                Some('.') if self.peek_next().is_some_and(|nc| nc.is_ascii_digit()) => {
                    self.advance();
                    return self.read_float_after_dot("0".to_string(), start);
                }
                Some('e') | Some('E') => {
                    self.advance();
                    return self.read_float_exponent("0".to_string(), start);
                }
                _ => {
                    // decimal 语法禁止前导零：[1-9][0-9]* | 0
                    // (0e5 已由上方 e/E 分支处理，0.5 已由 . 分支处理)
                    if self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                        return Err(MspError::LexError {
                            line: start.line,
                            column: start.column,
                            message: "leading zeros are not allowed in decimal integer literal"
                                .into(),
                        });
                    }
                    return Ok(self.make_token(TokenKind::Int(0), start, "0"));
                }
            }
        }
        let mut digits = String::new();
        digits.push(first);
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                digits.push(c);
                self.advance();
            } else if c == '.' && self.peek_next().is_some_and(|nc| nc.is_ascii_digit()) {
                self.advance();
                return self.read_float_after_dot(digits, start);
            } else if c == 'e' || c == 'E' {
                self.advance();
                return self.read_float_exponent(digits, start);
            } else {
                break;
            }
        }
        let value: i64 = digits.parse().map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid integer literal: {}", digits),
        })?;
        Ok(self.make_token(TokenKind::Int(value), start, &digits))
    }

    fn read_hex(&mut self, start: Position) -> Result<Token> {
        let mut digits = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_ascii_hexdigit() {
                digits.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: "expected hex digits after '0x'".into(),
            });
        }
        let value = i64::from_str_radix(&digits, 16).map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid hex literal: 0x{}", digits),
        })?;
        Ok(self.make_token(TokenKind::Int(value), start, &format!("0x{}", digits)))
    }

    fn read_binary(&mut self, start: Position) -> Result<Token> {
        let mut digits = String::new();
        while let Some(c) = self.peek_char() {
            if c == '0' || c == '1' {
                digits.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: "expected binary digits after '0b'".into(),
            });
        }
        let value = i64::from_str_radix(&digits, 2).map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid binary literal: 0b{}", digits),
        })?;
        Ok(self.make_token(TokenKind::Int(value), start, &format!("0b{}", digits)))
    }

    fn read_octal(&mut self, start: Position) -> Result<Token> {
        let mut digits = String::new();
        while let Some(c) = self.peek_char() {
            if ('0'..='7').contains(&c) {
                digits.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: "expected octal digits after '0o'".into(),
            });
        }
        let value = i64::from_str_radix(&digits, 8).map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid octal literal: 0o{}", digits),
        })?;
        Ok(self.make_token(TokenKind::Int(value), start, &format!("0o{}", digits)))
    }

    fn read_float_after_dot(&mut self, int_part: String, start: Position) -> Result<Token> {
        let mut frac = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                frac.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if frac.is_empty() {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: "expected digits after decimal point".into(),
            });
        }
        let full = if matches!(self.peek_char(), Some('e') | Some('E')) {
            self.advance();
            let exp = self.read_exponent(start)?;
            format!("{}.{}e{}", int_part, frac, exp)
        } else {
            format!("{}.{}", int_part, frac)
        };
        let value: f64 = full.parse().map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid float literal: {}", full),
        })?;
        Ok(self.make_token(TokenKind::Float(value), start, &full))
    }

    fn read_exponent(&mut self, start: Position) -> Result<String> {
        let mut exp = String::new();
        if let Some(c) = self.peek_char() {
            if c == '+' || c == '-' {
                exp.push(c);
                self.advance();
            }
        }
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                exp.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if exp.is_empty() || (exp == "+" || exp == "-") {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: "expected digits in exponent".into(),
            });
        }
        Ok(exp)
    }

    fn read_float_exponent(&mut self, int_part: String, start: Position) -> Result<Token> {
        let exp = self.read_exponent(start)?;
        let full = format!("{}e{}", int_part, exp);
        let value: f64 = full.parse().map_err(|_| MspError::LexError {
            line: start.line,
            column: start.column,
            message: format!("invalid float literal: {}", full),
        })?;
        Ok(self.make_token(TokenKind::Float(value), start, &full))
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

    fn tokenize(source: &str) -> Vec<Token> {
        Lexer::new(source).tokenize_all().unwrap()
    }

    fn find_kind(tokens: &[Token], target: TokenKind) -> bool {
        tokens.iter().any(|t| t.kind == target)
    }

    #[test]
    fn test_decimal() {
        let tokens = tokenize("a = 42\n");
        assert!(find_kind(&tokens, TokenKind::Int(42)));
    }

    #[test]
    fn test_hex() {
        let tokens = tokenize("b = 0xFF\n");
        assert!(find_kind(&tokens, TokenKind::Int(255)));
    }

    #[test]
    fn test_binary() {
        let tokens = tokenize("c = 0b1010\n");
        assert!(find_kind(&tokens, TokenKind::Int(10)));
    }

    #[test]
    fn test_octal() {
        let tokens = tokenize("d = 0o755\n");
        assert!(find_kind(&tokens, TokenKind::Int(493)));
    }

    #[allow(clippy::approx_constant)] // 3.14 is the spec test value, not an approximation of PI
    #[test]
    fn test_float_decimal() {
        let tokens = tokenize("e = 3.14\n");
        assert!(find_kind(&tokens, TokenKind::Float(3.14)));
    }

    #[test]
    fn test_float_exponent_negative() {
        let tokens = tokenize("f = 1.5e-3\n");
        assert!(find_kind(&tokens, TokenKind::Float(0.0015)));
    }

    #[test]
    fn test_float_exponent_only() {
        let tokens = tokenize("g = 1e10\n");
        assert!(find_kind(&tokens, TokenKind::Float(1e10)));
    }

    #[test]
    fn test_zero() {
        let tokens = tokenize("z = 0\n");
        assert!(find_kind(&tokens, TokenKind::Int(0)));
    }

    #[test]
    fn test_float_exponent_on_zero() {
        // 0e5 是合法浮点字面量（float 语法第二分支 [0-9]+ [eE][+-]?[0-9]+）
        let tokens = tokenize("x = 0e5\n");
        assert!(find_kind(&tokens, TokenKind::Float(0.0)));
    }

    #[test]
    fn test_int_dot_no_float() {
        // 3. 后无数字 → 不消费 '.'：Int(3) + Dot（由 parser 决定语义）
        // spec 01-lexical 语法 float 要求小数点后至少一位；此消歧规则见 task 04 §注意事项
        let tokens = tokenize("x = 3.\n");
        assert!(find_kind(&tokens, TokenKind::Int(3)));
        assert!(find_kind(&tokens, TokenKind::Dot));
    }

    #[test]
    fn test_invalid_hex_no_digits() {
        let result = Lexer::new("x = 0x\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_binary_no_digits() {
        let result = Lexer::new("x = 0b\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_octal_no_digits() {
        let result = Lexer::new("x = 0o\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_exponent_no_digits() {
        let result = Lexer::new("x = 1e\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_int_followed_by_dot() {
        // 42.foo 不应被当作浮点数尝试；. 后无数字时不消费 '.'
        let tokens = tokenize("42.foo\n");
        assert!(find_kind(&tokens, TokenKind::Int(42)));
        assert!(find_kind(&tokens, TokenKind::Dot));
        assert!(find_kind(&tokens, TokenKind::Identifier("foo".into())));
    }

    #[test]
    fn test_invalid_leading_zero() {
        // decimal = [1-9][0-9]* | 0，前导零非法
        let result = Lexer::new("x = 07\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_double_leading_zero() {
        let result = Lexer::new("x = 00\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_large_number_and_overflow() {
        // i64 最大值合法
        let tokens = tokenize("x = 9223372036854775807\n");
        assert!(find_kind(&tokens, TokenKind::Int(9223372036854775807)));
        // i64 最大值 + 1 溢出 → LexError
        let result = Lexer::new("x = 9223372036854775808\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_uppercase_prefix() {
        let tokens = tokenize("x = 0XFF\n");
        assert!(find_kind(&tokens, TokenKind::Int(255)));
    }

    #[test]
    fn test_float_with_positive_exponent() {
        let tokens = tokenize("x = 2.0e+3\n");
        assert!(find_kind(&tokens, TokenKind::Float(2000.0)));
    }

    #[test]
    fn test_float_dot_after_zero() {
        // 0.5 → Float(0.5)
        let tokens = tokenize("x = 0.5\n");
        assert!(find_kind(&tokens, TokenKind::Float(0.5)));
    }
}
