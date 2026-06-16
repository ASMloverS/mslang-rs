pub mod token;

use crate::error::{MspError, Result};
use crate::lexer::token::{keyword_table, reserved_words, Position, Span, Token, TokenKind};

pub struct Lexer {
    #[allow(dead_code)]  // 保留用于调试/错误报告；chars 用于所有字符访问
    source: String,
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
    paren_depth: usize,    // () 深度
    bracket_depth: usize,  // [] 深度
    brace_depth: usize,    // {} 深度
    prev_token_kind: Option<TokenKind>,  // 最近发出的真实 token（注释跳过时不更新）
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
            paren_depth: 0,
            bracket_depth: 0,
            brace_depth: 0,
            prev_token_kind: None,
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
        // next_token() 的循环以 continue 重新进入，由其换行分支处理该换行。
        while let Some(c) = self.peek_char() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    /// 续行判断：当前行尾 token 是否期待后续操作数/元素，使得换行不终止语句。
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

        // 规则 3: 行尾是左括号（与括号深度跟踪互补）
        if matches!(
            prev,
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace
        ) {
            return true;
        }

        false
    }

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
                    // 空行合并：前一 token 已是 Newline 时跳过（连续空行只产生一个 Newline）
                    if self.prev_token_kind.as_ref() == Some(&TokenKind::Newline) {
                        continue;
                    }
                    // 括号内换行直接跳过（不产生 token）
                    if self.paren_depth > 0 || self.bracket_depth > 0 || self.brace_depth > 0 {
                        continue;
                    }
                    // 隐式续行：行尾运算符/逗号/左括号
                    if self.is_continuation() {
                        continue;
                    }
                    self.make_token(TokenKind::Newline, start, "\n")
                }
                '#' => { self.skip_comment(); continue; }
                '(' => {
                    self.paren_depth += 1;
                    self.make_token(TokenKind::LeftParen, start, "(")
                }
                ')' => {
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.make_token(TokenKind::RightParen, start, ")")
                }
                '[' => {
                    self.bracket_depth += 1;
                    self.make_token(TokenKind::LeftBracket, start, "[")
                }
                ']' => {
                    self.bracket_depth = self.bracket_depth.saturating_sub(1);
                    self.make_token(TokenKind::RightBracket, start, "]")
                }
                '{' => {
                    self.brace_depth += 1;
                    self.make_token(TokenKind::LeftBrace, start, "{")
                }
                '}' => {
                    self.brace_depth = self.brace_depth.saturating_sub(1);
                    self.make_token(TokenKind::RightBrace, start, "}")
                }
                c if c.is_ascii_digit() => self.read_number(c, start)?,
                '"' => self.read_string(start)?,
                c if c.is_ascii_alphabetic() || c == '_' => self.read_identifier(c, start)?,
                '+' => self.read_plus(start)?,
                '-' => self.read_minus(start)?,
                '*' => self.read_star(start)?,
                '/' => self.read_slash(start)?,
                '%' => self.read_percent(start)?,
                '=' => self.read_equal(start)?,
                '!' => self.read_bang(start)?,
                '<' => self.read_less(start)?,
                '>' => self.read_greater(start)?,
                '&' => self.read_ampersand(start)?,
                '|' => self.read_pipe(start)?,
                '^' => self.read_caret(start)?,
                '~' => self.make_token(TokenKind::Tilde, start, "~"),
                ',' => self.make_token(TokenKind::Comma, start, ","),
                '.' => self.read_dot(start)?,
                ':' => self.read_colon(start)?,
                ';' => self.make_token(TokenKind::Semicolon, start, ";"),
                '@' => self.make_token(TokenKind::At, start, "@"),
                _ => {
                    return Err(MspError::LexError {
                        line: start.line,
                        column: start.column,
                        message: format!("unexpected character '{}'", ch),
                    });
                }
            };

            self.prev_token_kind = Some(token.kind.clone());
            return Ok(token);
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
        let kind = if let Some(kw) = keyword_table().get(lexeme.as_str()) {
            kw.clone()
        } else if reserved_words().contains(&lexeme.as_str()) {
            return Err(MspError::LexError {
                line: start.line,
                column: start.column,
                message: format!(
                    "'{}' is a reserved word and cannot be used as identifier",
                    lexeme
                ),
            });
        } else {
            TokenKind::Identifier(lexeme.clone())
        };
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
                        Some('"') => {
                            value.push('"');
                            lexeme.push('"');
                        }
                        Some('\\') => {
                            value.push('\\');
                            lexeme.push('\\');
                        }
                        Some('n') => {
                            value.push('\n');
                            lexeme.push('n');
                        }
                        Some('t') => {
                            value.push('\t');
                            lexeme.push('t');
                        }
                        Some('r') => {
                            value.push('\r');
                            lexeme.push('r');
                        }
                        Some('0') => {
                            value.push('\0');
                            lexeme.push('0');
                        }
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

    fn read_plus(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::PlusEqual, start, "+="))
            }
            _ => Ok(self.make_token(TokenKind::Plus, start, "+")),
        }
    }

    fn read_minus(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::MinusEqual, start, "-="))
            }
            Some('>') => {
                self.advance();
                Ok(self.make_token(TokenKind::Arrow, start, "->"))
            }
            _ => Ok(self.make_token(TokenKind::Minus, start, "-")),
        }
    }

    fn read_star(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::StarEqual, start, "*="))
            }
            Some('*') => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok(self.make_token(TokenKind::DoubleStarEqual, start, "**="))
                } else {
                    Ok(self.make_token(TokenKind::DoubleStar, start, "**"))
                }
            }
            _ => Ok(self.make_token(TokenKind::Star, start, "*")),
        }
    }

    fn read_slash(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::SlashEqual, start, "/="))
            }
            Some('/') => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok(self.make_token(TokenKind::DoubleSlashEqual, start, "//="))
                } else {
                    Ok(self.make_token(TokenKind::DoubleSlash, start, "//"))
                }
            }
            _ => Ok(self.make_token(TokenKind::Slash, start, "/")),
        }
    }

    fn read_percent(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::PercentEqual, start, "%="))
            }
            _ => Ok(self.make_token(TokenKind::Percent, start, "%")),
        }
    }

    fn read_less(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::LessEqual, start, "<="))
            }
            Some('<') => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok(self.make_token(TokenKind::LeftShiftEqual, start, "<<="))
                } else {
                    Ok(self.make_token(TokenKind::LeftShift, start, "<<"))
                }
            }
            Some('-') => {
                self.advance();
                Ok(self.make_token(TokenKind::LeftArrow, start, "<-"))
            }
            _ => Ok(self.make_token(TokenKind::Less, start, "<")),
        }
    }

    fn read_greater(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::GreaterEqual, start, ">="))
            }
            Some('>') => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok(self.make_token(TokenKind::RightShiftEqual, start, ">>="))
                } else {
                    Ok(self.make_token(TokenKind::RightShift, start, ">>"))
                }
            }
            _ => Ok(self.make_token(TokenKind::Greater, start, ">")),
        }
    }

    fn read_ampersand(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::AmpersandEqual, start, "&="))
            }
            _ => Ok(self.make_token(TokenKind::Ampersand, start, "&")),
        }
    }

    fn read_pipe(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::PipeEqual, start, "|="))
            }
            _ => Ok(self.make_token(TokenKind::Pipe, start, "|")),
        }
    }

    fn read_caret(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::CaretEqual, start, "^="))
            }
            _ => Ok(self.make_token(TokenKind::Caret, start, "^")),
        }
    }

    fn read_dot(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('.') => {
                self.advance();
                if self.peek_char() == Some('.') {
                    self.advance();
                    Ok(self.make_token(TokenKind::DotDotDot, start, "..."))
                } else {
                    Ok(self.make_token(TokenKind::DotDot, start, ".."))
                }
            }
            _ => Ok(self.make_token(TokenKind::Dot, start, ".")),
        }
    }

    fn read_colon(&mut self, start: Position) -> Result<Token> {
        match self.peek_char() {
            Some('=') => {
                self.advance();
                Ok(self.make_token(TokenKind::ColonEqual, start, ":="))
            }
            _ => Ok(self.make_token(TokenKind::Colon, start, ":")),
        }
    }
}

/// 判断 token 是否为二元/复合运算符（行尾时期待后续操作数，触发隐式续行）。
fn is_binary_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        // 算术
        TokenKind::Plus | TokenKind::Minus | TokenKind::Star | TokenKind::Slash
        | TokenKind::DoubleSlash | TokenKind::Percent | TokenKind::DoubleStar
        // 赋值（含复合赋值与海象运算符 :=）
        | TokenKind::Equal | TokenKind::ColonEqual
        | TokenKind::PlusEqual | TokenKind::MinusEqual | TokenKind::StarEqual
        | TokenKind::SlashEqual | TokenKind::DoubleSlashEqual | TokenKind::PercentEqual
        | TokenKind::DoubleStarEqual
        | TokenKind::AmpersandEqual | TokenKind::PipeEqual | TokenKind::CaretEqual
        | TokenKind::LeftShiftEqual | TokenKind::RightShiftEqual
        // 比较
        | TokenKind::EqualEqual | TokenKind::BangEqual
        | TokenKind::Less | TokenKind::Greater | TokenKind::LessEqual | TokenKind::GreaterEqual
        // 位运算
        | TokenKind::Ampersand | TokenKind::Pipe | TokenKind::Caret
        | TokenKind::LeftShift | TokenKind::RightShift
        // 逻辑关键字
        | TokenKind::And | TokenKind::Or | TokenKind::Not
        // 成员/身份
        | TokenKind::In | TokenKind::Is
        // 成员访问 / 箭头（行尾时期待后续操作数）
        | TokenKind::Dot | TokenKind::Arrow
    )
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
        let idents: Vec<_> = tokens.iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
            .collect();
        assert!(idents.iter().all(|t| matches!(&t.kind, TokenKind::Identifier(_))));
    }

    #[test]
    fn test_boolean_keywords() {
        let tokens = tokenize("true false nil\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::True));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::False));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Nil));
    }

    #[test]
    fn test_all_reserved_words_error() {
        for word in &["select", "default", "case", "export", "match"] {
            let result = Lexer::new(&format!("{} = 1\n", word)).tokenize_all();
            assert!(result.is_err(), "reserved word '{}' should error", word);
        }
    }

    #[test]
    fn test_keyword_prefix_is_identifier() {
        let tokens = tokenize("varx iffy returnx\n");
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "varx")));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "iffy")));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "returnx")));
    }

    #[test]
    fn test_underscore_only_identifier() {
        let tokens = tokenize("_ = 1\n");
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "_")));
    }

    #[test]
    fn test_is_operator() {
        let tokens = tokenize("x is y\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Is));
    }

    #[test]
    fn test_all_36_keywords() {
        let source = "var const fn return if elif else while for in break continue \
                      class self super true false nil and or not \
                      try except finally defer with throw \
                      async await go import from as yield nonlocal global\n";
        let tokens = tokenize(source);
        let keyword_tokens: Vec<_> = tokens.iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
            .collect();
        assert_eq!(keyword_tokens.len(), 36);
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

    #[test]
    fn test_simple_string() {
        let tokens = tokenize("a = \"hello world\"\n");
        assert!(find_kind(&tokens, TokenKind::String("hello world".into())));
    }

    #[test]
    fn test_newline_escape() {
        let tokens = tokenize("b = \"line1\\nline2\"\n");
        assert!(find_kind(&tokens, TokenKind::String("line1\nline2".into())));
    }

    #[test]
    fn test_backslash_escape() {
        let tokens = tokenize("c = \"path: C:\\\\Users\"\n");
        assert!(find_kind(
            &tokens,
            TokenKind::String("path: C:\\Users".into())
        ));
    }

    #[test]
    fn test_quote_escape() {
        let tokens = tokenize("d = \"quotes: \\\"hello\\\"\"\n");
        assert!(find_kind(
            &tokens,
            TokenKind::String("quotes: \"hello\"".into())
        ));
    }

    #[test]
    fn test_empty_string() {
        let tokens = tokenize("e = \"\"\n");
        assert!(find_kind(&tokens, TokenKind::String("".into())));
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
        assert!(find_kind(&tokens, TokenKind::String("\t\0".into())));
    }

    #[test]
    fn test_carriage_return_escape() {
        // \r 转义 → 回车符（验证标准 #2 的第 5 种转义，此前遗漏）
        let tokens = tokenize("x = \"a\\rb\"\n");
        assert!(find_kind(&tokens, TokenKind::String("a\rb".into())));
    }

    #[test]
    fn test_bare_carriage_return_error() {
        // 字符串内裸 \r（非 CRLF）视为跨行，与 \n 一致报错
        let result = Lexer::new("x = \"abc\rdef\"\n").tokenize_all();
        assert!(result.is_err());
    }

    // ---- task 07: 运算符与分隔符解析 ----

    #[test]
    fn test_arithmetic() {
        let tokens = tokenize("x = 10 + 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Plus));
    }

    #[test]
    fn test_floor_div() {
        let tokens = tokenize("y = x // 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleSlash));
    }

    #[test]
    fn test_power() {
        let tokens = tokenize("z = x ** 2\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleStar));
    }

    #[test]
    fn test_comparison() {
        let tokens = tokenize("a = x >= 5\nb = x != 0\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::GreaterEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::BangEqual));
    }

    #[test]
    fn test_bitwise() {
        let tokens = tokenize("c = x & 0xFF\nd = x << 2\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Ampersand));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftShift));
    }

    #[test]
    fn test_compound_assignment() {
        let tokens = tokenize("x += 1\ny **= 2\nz <<= 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::PlusEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleStarEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftShiftEqual));
    }

    #[test]
    fn test_special_symbols() {
        let tokens = tokenize("@ deco\nx := 10\nch <- val\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::At));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::ColonEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftArrow));
    }

    #[test]
    fn test_range_operators() {
        let tokens = tokenize("a .. b\nx ...\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DotDot));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DotDotDot));
    }

    #[test]
    fn test_arrow() {
        let tokens = tokenize("fn foo() -> int {}\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Arrow));
    }

    #[test]
    fn test_bang_alone_is_error() {
        let result = Lexer::new("x = ! y\n").tokenize_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_comparison_full() {
        // 注意：spec 原文 input 仅含 "<"，却断言 LessEqual —— 该断言不可能成立。
        // 此处修正 input 同时包含 "<" 与 "<="，使 Less 与 LessEqual 断言均可验证。
        let tokens = tokenize("a == b\nc < d\nf <= g\ne > f\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::EqualEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LessEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Less));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Greater));
    }

    #[test]
    fn test_right_shift() {
        let tokens = tokenize("x = a >> 2\nb >>= 3\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::RightShift));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::RightShiftEqual));
    }

    #[test]
    fn test_tilde() {
        let tokens = tokenize("x = ~y\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Tilde));
    }

    #[test]
    fn test_semicolon() {
        let tokens = tokenize("x = 1;\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Semicolon));
    }

    #[test]
    fn test_remaining_compound_assignments() {
        let tokens = tokenize("x -= 1\nx *= 2\nx /= 3\nx //= 4\nx %= 5\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::MinusEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::StarEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::SlashEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DoubleSlashEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::PercentEqual));
        let tokens2 = tokenize("x &= 6\nx |= 7\nx ^= 8\n");
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::AmpersandEqual));
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::PipeEqual));
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::CaretEqual));
    }

    #[test]
    fn test_dot_number_interaction() {
        // 42.foo → Int(42) + Dot + Identifier（read_number 不消费后无数字的 .）
        let tokens = tokenize("42.foo\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Int(42)));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Dot));
        // 42..50 → Int(42) + DotDot + Int(50)
        let tokens2 = tokenize("42..50\n");
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::Int(42)));
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::DotDot));
        assert!(tokens2.iter().any(|t| t.kind == TokenKind::Int(50)));
    }

    // ---- task 08: 换行与语句终止规则 ----

    fn newline_count(tokens: &[Token]) -> usize {
        tokens.iter().filter(|t| t.kind == TokenKind::Newline).count()
    }

    #[test]
    fn test_basic_newline() {
        let tokens = tokenize("x = 1\ny = 2\n");
        assert_eq!(newline_count(&tokens), 2);
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
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::String(s) if s == "Alice")));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::String(s) if s == "Bob")));
    }

    #[test]
    fn test_function_call_continuation() {
        let tokens = tokenize("result = fn(\n    arg1,\n    arg2\n)\n");
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "arg1")));
    }

    #[test]
    fn test_bracket_depth_balanced() {
        let lexer = Lexer::new("x = [\n1\n]\n");
        let tokens = lexer.tokenize_all().unwrap();
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Int(1)));
    }

    #[test]
    fn test_comma_continuation() {
        // 行尾逗号后的换行被跳过
        let tokens = tokenize("x = foo,\n    bar\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Comma));
        assert!(tokens.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "bar")));
    }

    #[test]
    fn test_brace_continuation() {
        // {} 内换行被跳过
        let tokens = tokenize("d = {\n    key: 1\n}\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::LeftBrace));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::RightBrace));
    }

    #[test]
    fn test_compound_assignment_continuation() {
        // += 后的换行被跳过（复合赋值也是续行运算符）
        let tokens = tokenize("x +=\n    5\n");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::PlusEqual));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Int(5)));
    }

    #[test]
    fn test_consecutive_blank_lines() {
        // 连续空行只产生一个 Newline
        let tokens = tokenize("x = 1\n\n\ny = 2\n");
        assert_eq!(newline_count(&tokens), 2);
    }
}
