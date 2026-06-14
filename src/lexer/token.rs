use std::collections::HashMap;
use std::fmt;

// 注意：Float(f64) 使 PartialEq 执行浮点精确比较，测试中应使用容差比较而非 ==
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // 字面量
    Int(i64),
    Float(f64),
    String(String),

    // 标识符
    Identifier(String),

    // 36 个关键字
    Var, Const, Fn, Return,
    If, Elif, Else,
    While, For, In, Break, Continue,
    Class, Zelf, Super,  // Zelf = 'self' keyword (Self is Rust reserved word)
    True, False, Nil,
    And, Or, Not,
    Try, Except, Finally, Defer, With, Throw,
    Async, Await, Go,
    Import, From, As,
    Yield, Nonlocal, Global,

    // 算术运算符 (+ - * / // % **)
    Plus, Minus, Star, Slash, DoubleSlash, Percent, DoubleStar,
    // 比较运算符 (== != < > <= >=)
    EqualEqual, BangEqual, Less, Greater, LessEqual, GreaterEqual,
    // 位运算符 (& | ^ << >> ~)
    Ampersand, Pipe, Caret, LeftShift, RightShift, Tilde,
    // 身份比较 — Is 与 In 类似：词法分析器通过关键字查找表返回 TokenKind::Is
    // 表达式解析器在 parse_comparison() 中将其视为比较运算符（双重角色）
    Is,
    // 赋值运算符 (= += -= *= /= //= %= **= &= |= ^= <<= >>=)
    Equal, PlusEqual, MinusEqual, StarEqual, SlashEqual,
    DoubleSlashEqual, PercentEqual, DoubleStarEqual,
    AmpersandEqual, PipeEqual, CaretEqual, LeftShiftEqual, RightShiftEqual,
    // 短声明 (:=)
    ColonEqual,
    // 分隔符
    LeftParen, RightParen,
    LeftBracket, RightBracket,
    LeftBrace, RightBrace,
    Comma, Dot, Colon, Semicolon, Arrow,
    // 特殊符号
    At, LeftArrow,
    // 范围运算符（标准 01-lexical.md:184-193 定义但 TokenKind 枚举遗漏，此处补充）
    DotDot, DotDotDot,
    // 换行/语句终止（标准 01-lexical.md:244 定义但 TokenKind 枚举遗漏，此处补充）
    Newline,
    // EOF
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
}

impl Token {
    pub fn is_identifier(&self) -> bool {
        matches!(self.kind, TokenKind::Identifier(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // 字面量
            TokenKind::Int(v) => write!(f, "Int({})", v),
            TokenKind::Float(v) => write!(f, "Float({})", v),
            TokenKind::String(v) => write!(f, "String(\"{}\")", v),
            TokenKind::Identifier(v) => write!(f, "Identifier(\"{}\")", v),
            // 关键字
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
            TokenKind::Break => write!(f, "break"),
            TokenKind::Continue => write!(f, "continue"),
            TokenKind::Class => write!(f, "class"),
            TokenKind::Zelf => write!(f, "self"),
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
            TokenKind::Import => write!(f, "import"),
            TokenKind::From => write!(f, "from"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Yield => write!(f, "yield"),
            TokenKind::Nonlocal => write!(f, "nonlocal"),
            TokenKind::Global => write!(f, "global"),
            TokenKind::Is => write!(f, "is"),
            // 算术运算符
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::DoubleSlash => write!(f, "//"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::DoubleStar => write!(f, "**"),
            // 比较运算符
            TokenKind::EqualEqual => write!(f, "=="),
            TokenKind::BangEqual => write!(f, "!="),
            TokenKind::Less => write!(f, "<"),
            TokenKind::Greater => write!(f, ">"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::GreaterEqual => write!(f, ">="),
            // 位运算符
            TokenKind::Ampersand => write!(f, "&"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::Caret => write!(f, "^"),
            TokenKind::LeftShift => write!(f, "<<"),
            TokenKind::RightShift => write!(f, ">>"),
            TokenKind::Tilde => write!(f, "~"),
            // 赋值运算符
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
            // 分隔符
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
            // 特殊符号
            TokenKind::At => write!(f, "@"),
            TokenKind::LeftArrow => write!(f, "<-"),
            // 范围运算符
            TokenKind::DotDot => write!(f, ".."),
            TokenKind::DotDotDot => write!(f, "..."),
            // 换行/语句终止
            TokenKind::Newline => write!(f, "\\n"),
            // EOF
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Token({}, \"{}\", {}:{})",
            self.kind, self.lexeme, self.span.start.line, self.span.start.column
        )
    }
}

/// 关键字查找表：词法分析阶段用于将标识符区分为关键字。
/// 包含 36 个关键字 + `is`（身份比较，双重角色），共 37 个条目。
pub fn keyword_table() -> HashMap<&'static str, TokenKind> {
    let mut m = HashMap::new();
    m.insert("var", TokenKind::Var);
    m.insert("const", TokenKind::Const);
    m.insert("fn", TokenKind::Fn);
    m.insert("return", TokenKind::Return);
    m.insert("if", TokenKind::If);
    m.insert("elif", TokenKind::Elif);
    m.insert("else", TokenKind::Else);
    m.insert("while", TokenKind::While);
    m.insert("for", TokenKind::For);
    m.insert("in", TokenKind::In);
    m.insert("break", TokenKind::Break);
    m.insert("continue", TokenKind::Continue);
    m.insert("class", TokenKind::Class);
    m.insert("self", TokenKind::Zelf);
    m.insert("super", TokenKind::Super);
    m.insert("true", TokenKind::True);
    m.insert("false", TokenKind::False);
    m.insert("nil", TokenKind::Nil);
    m.insert("and", TokenKind::And);
    m.insert("or", TokenKind::Or);
    m.insert("not", TokenKind::Not);
    m.insert("try", TokenKind::Try);
    m.insert("except", TokenKind::Except);
    m.insert("finally", TokenKind::Finally);
    m.insert("defer", TokenKind::Defer);
    m.insert("with", TokenKind::With);
    m.insert("throw", TokenKind::Throw);
    m.insert("async", TokenKind::Async);
    m.insert("await", TokenKind::Await);
    m.insert("go", TokenKind::Go);
    m.insert("import", TokenKind::Import);
    m.insert("from", TokenKind::From);
    m.insert("as", TokenKind::As);
    m.insert("yield", TokenKind::Yield);
    m.insert("nonlocal", TokenKind::Nonlocal);
    m.insert("global", TokenKind::Global);
    m.insert("is", TokenKind::Is);
    m
}

/// 保留字集合：不可用作标识符，但当前版本尚无语义（预留未来使用）。
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
                start: Position { line: 1, column: 1, offset: 0 },
                end: Position { line: 1, column: 3, offset: 2 },
            },
        };
        assert_eq!(tok.kind, TokenKind::Int(42));
        assert_eq!(tok.lexeme, "42");
    }

    #[test]
    fn test_keyword_count() {
        let kw = keyword_table();
        assert_eq!(kw.len(), 37);
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
                start: Position { line: 1, column: 1, offset: 0 },
                end: Position { line: 1, column: 2, offset: 1 },
            },
        };
        let s = format!("{}", tok);
        assert!(s.contains("Identifier"));
        assert!(s.contains("x"));
    }

    #[test]
    fn test_position_copy() {
        let p = Position { line: 1, column: 5, offset: 4 };
        let p2 = p;
        assert_eq!(p, p2);
    }
}
