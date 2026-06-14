use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum MspError {
    #[error("lexer error at {line}:{column}: {message}")]
    LexError { line: usize, column: usize, message: String },

    #[error("parse error at {line}:{column}: {message}")]
    ParseError { line: usize, column: usize, message: String },

    #[error("compile error: {message}")]
    CompileError { message: String },

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MspError>;

#[cfg(test)]
mod tests {
    use crate::error::{MspError, Result};

    #[test]
    fn test_error_display() {
        let err = MspError::LexError {
            line: 1,
            column: 5,
            message: "unexpected character".into(),
        };
        assert_eq!(
            format!("{}", err),
            "lexer error at 1:5: unexpected character"
        );
    }

    #[test]
    fn test_result_alias() {
        fn sample() -> Result<i32> { Ok(42) }
        assert_eq!(sample().unwrap(), 42);
    }
}
