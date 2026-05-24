use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i64),
    Float(f64),
    String(String),
    Identifier(String),

    Var,
    Const,
    Fn,
    Return,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Is,
    Break,
    Continue,
    Class,
    Self_,
    Super,
    True,
    False,
    Nil,
    And,
    Or,
    Not,
    Try,
    Except,
    Finally,
    Defer,
    With,
    Throw,
    Async,
    Await,
    Go,
    Channel,
    Import,
    From,
    As,
    Yield,

    Plus,
    Minus,
    Star,
    Slash,
    DoubleSlash,
    Percent,
    DoubleStar,

    EqualEqual,
    BangEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,

    Ampersand,
    Pipe,
    Caret,
    LeftShift,
    RightShift,
    Tilde,

    Equal,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    DoubleSlashEqual,
    PercentEqual,
    DoubleStarEqual,
    AmpersandEqual,
    PipeEqual,
    CaretEqual,
    LeftShiftEqual,
    RightShiftEqual,
    ColonEqual,

    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Dot,
    Colon,
    Semicolon,
    Arrow,

    At,
    LeftArrow,
    DotDot,
    DotDotDot,

    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Int(v) => write!(f, "Int({})", v),
            TokenKind::Float(v) => write!(f, "Float({})", v),
            TokenKind::String(v) => write!(f, "String(\"{}\")", v),
            TokenKind::Identifier(v) => write!(f, "Identifier(\"{}\")", v),

            TokenKind::Var => write!(f, "var"),
            TokenKind::Const => write!(f, "const"),
            TokenKind::Fn => write!(f, "fn"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Elif => write!(f, "elif"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::While => write!(f, "while"),
            TokenKind::For => write!(f, "for"),
            TokenKind::In => write!(f, "in"),
            TokenKind::Is => write!(f, "is"),
            TokenKind::Break => write!(f, "break"),
            TokenKind::Continue => write!(f, "continue"),
            TokenKind::Class => write!(f, "class"),
            TokenKind::Self_ => write!(f, "self"),
            TokenKind::Super => write!(f, "super"),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Nil => write!(f, "nil"),
            TokenKind::And => write!(f, "and"),
            TokenKind::Or => write!(f, "or"),
            TokenKind::Not => write!(f, "not"),
            TokenKind::Try => write!(f, "try"),
            TokenKind::Except => write!(f, "except"),
            TokenKind::Finally => write!(f, "finally"),
            TokenKind::Defer => write!(f, "defer"),
            TokenKind::With => write!(f, "with"),
            TokenKind::Throw => write!(f, "throw"),
            TokenKind::Async => write!(f, "async"),
            TokenKind::Await => write!(f, "await"),
            TokenKind::Go => write!(f, "go"),
            TokenKind::Channel => write!(f, "channel"),
            TokenKind::Import => write!(f, "import"),
            TokenKind::From => write!(f, "from"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Yield => write!(f, "yield"),

            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::DoubleSlash => write!(f, "//"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::DoubleStar => write!(f, "**"),

            TokenKind::EqualEqual => write!(f, "=="),
            TokenKind::BangEqual => write!(f, "!="),
            TokenKind::Less => write!(f, "<"),
            TokenKind::Greater => write!(f, ">"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::GreaterEqual => write!(f, ">="),

            TokenKind::Ampersand => write!(f, "&"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::Caret => write!(f, "^"),
            TokenKind::LeftShift => write!(f, "<<"),
            TokenKind::RightShift => write!(f, ">>"),
            TokenKind::Tilde => write!(f, "~"),

            TokenKind::Equal => write!(f, "="),
            TokenKind::PlusEqual => write!(f, "+="),
            TokenKind::MinusEqual => write!(f, "-="),
            TokenKind::StarEqual => write!(f, "*="),
            TokenKind::SlashEqual => write!(f, "/="),
            TokenKind::DoubleSlashEqual => write!(f, "//="),
            TokenKind::PercentEqual => write!(f, "%="),
            TokenKind::DoubleStarEqual => write!(f, "**="),
            TokenKind::AmpersandEqual => write!(f, "&="),
            TokenKind::PipeEqual => write!(f, "|="),
            TokenKind::CaretEqual => write!(f, "^="),
            TokenKind::LeftShiftEqual => write!(f, "<<="),
            TokenKind::RightShiftEqual => write!(f, ">>="),
            TokenKind::ColonEqual => write!(f, ":="),

            TokenKind::LeftParen => write!(f, "("),
            TokenKind::RightParen => write!(f, ")"),
            TokenKind::LeftBracket => write!(f, "["),
            TokenKind::RightBracket => write!(f, "]"),
            TokenKind::LeftBrace => write!(f, "{{"),
            TokenKind::RightBrace => write!(f, "}}"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Semicolon => write!(f, ";"),
            TokenKind::Arrow => write!(f, "->"),

            TokenKind::At => write!(f, "@"),
            TokenKind::LeftArrow => write!(f, "<-"),
            TokenKind::DotDot => write!(f, ".."),
            TokenKind::DotDotDot => write!(f, "..."),

            TokenKind::Newline => write!(f, "\\n"),
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Token({}, \"{}\", {}:{})",
            self.kind,
            self.lexeme,
            self.span.start.line,
            self.span.start.column
        )
    }
}

pub fn keyword_table() -> HashMap<&'static str, TokenKind> {
    HashMap::from([
        ("var", TokenKind::Var),
        ("const", TokenKind::Const),
        ("fn", TokenKind::Fn),
        ("return", TokenKind::Return),
        ("if", TokenKind::If),
        ("elif", TokenKind::Elif),
        ("else", TokenKind::Else),
        ("while", TokenKind::While),
        ("for", TokenKind::For),
        ("in", TokenKind::In),
        ("is", TokenKind::Is),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        ("class", TokenKind::Class),
        ("self", TokenKind::Self_),
        ("super", TokenKind::Super),
        ("true", TokenKind::True),
        ("false", TokenKind::False),
        ("nil", TokenKind::Nil),
        ("and", TokenKind::And),
        ("or", TokenKind::Or),
        ("not", TokenKind::Not),
        ("try", TokenKind::Try),
        ("except", TokenKind::Except),
        ("finally", TokenKind::Finally),
        ("defer", TokenKind::Defer),
        ("with", TokenKind::With),
        ("throw", TokenKind::Throw),
        ("async", TokenKind::Async),
        ("await", TokenKind::Await),
        ("go", TokenKind::Go),
        ("channel", TokenKind::Channel),
        ("import", TokenKind::Import),
        ("from", TokenKind::From),
        ("as", TokenKind::As),
        ("yield", TokenKind::Yield),
    ])
}

pub fn reserved_words() -> &'static [&'static str] {
    &["select", "default", "case", "export", "match"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_construction() {
        let tok = Token {
            kind: TokenKind::Int(42),
            lexeme: "42".into(),
            span: Span {
                start: Position {
                    line: 1,
                    column: 1,
                    offset: 0,
                },
                end: Position {
                    line: 1,
                    column: 3,
                    offset: 2,
                },
            },
        };
        assert_eq!(tok.kind, TokenKind::Int(42));
        assert_eq!(tok.lexeme, "42");
    }

    #[test]
    fn test_keyword_count() {
        let kw = keyword_table();
        assert_eq!(kw.len(), 36);
    }

    #[test]
    fn test_reserved_words() {
        let reserved = reserved_words();
        assert!(reserved.contains(&"select"));
        assert!(reserved.contains(&"match"));
    }

    #[test]
    fn test_token_display() {
        let tok = Token {
            kind: TokenKind::Identifier("x".into()),
            lexeme: "x".into(),
            span: Span {
                start: Position {
                    line: 1,
                    column: 1,
                    offset: 0,
                },
                end: Position {
                    line: 1,
                    column: 2,
                    offset: 1,
                },
            },
        };
        let s = format!("{}", tok);
        assert!(s.contains("Identifier"));
        assert!(s.contains("x"));
    }

    #[test]
    fn test_position_copy() {
        let p = Position {
            line: 1,
            column: 5,
            offset: 4,
        };
        let p2 = p;
        assert_eq!(p, p2);
    }
}
