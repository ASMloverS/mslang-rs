use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum MspError {
    #[error("lexer error at {line}:{column}: {message}")]
    LexError {
        line: usize,
        column: usize,
        message: String,
    },

    #[error("parse error at {line}:{column}: {message}")]
    ParseError {
        line: usize,
        column: usize,
        message: String,
    },

    #[error("compile error: {message}")]
    CompileError { message: String },

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MspError>;

// ---------------------------------------------------------------------------
// task 57：友好错误信息与堆栈跟踪
//
// 参照 docs/mslang/tasks/57-error-messages.md §3/§4/§6/§7。
// SourceLocation / StackTrace / RuntimeError / CompileError 为纯展示结构，
// 与上方 MspError（编译/词法/解析错误枚举）独立。RuntimeError 容纳两类输入
// （§0.1）：MsException 对象（titled=true，输出 "Error: <msg>"）与 VM 内部
// Result<_, String> 错误（titled=false，逐字输出消息字符串）。
// ---------------------------------------------------------------------------

/// 源码位置。运行时错误 column 恒为 None（§1.2）；编译时错误 column = Some。
#[derive(Clone)]
pub struct SourceLocation {
    pub file: String,
    pub line: usize,
    pub column: Option<usize>,
}

/// 单个栈帧：函数名 + 源码位置。
#[derive(Clone)]
pub struct StackTraceFrame {
    pub function_name: String,
    pub location: SourceLocation,
}

/// 调用堆栈快照（栈底→栈顶顺序存储；Display 倒序输出，最内层在前）。
#[derive(Clone, Default)]
pub struct StackTrace {
    pub frames: Vec<StackTraceFrame>,
}

/// 运行时错误展示对象。`titled` 控制首行：true → "Error: {message}"
/// （MsException 路径，§0.1 case 1）；false → 逐字输出 message（VM 内部
/// String 错误，§0.1 case 2，message 已含类型前缀如 "ZeroDivisionError: ..."）。
pub struct RuntimeError {
    pub message: String,
    pub titled: bool,
    pub stack_trace: StackTrace,
    pub source_line: Option<String>,
    pub location: Option<SourceLocation>,
}

/// 编译时错误展示对象。column 来自 lexer/parser（恒为 Some）。
pub struct CompileError {
    pub message: String,
    pub location: Option<SourceLocation>,
    pub source_line: Option<String>,
}

impl RuntimeError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            titled: true,
            stack_trace: StackTrace::default(),
            source_line: None,
            location: None,
        }
    }
}

/// 行号动态宽度（≥3，避免 ≥1000 行源文件破坏 | 列对齐）。
fn line_width(line: usize) -> usize {
    line.to_string().len().max(3)
}

impl std::fmt::Display for RuntimeError {
    /// 参照 §6。列号缺失（运行时错误）→ 不显示 :col、不画 ^ 行。
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // 错误标题（消息内容不改写，见 §0.1）
        if self.titled {
            writeln!(f, "Error: {}", self.message)?;
        } else {
            writeln!(f, "{}", self.message)?;
        }

        // 源码行和错误指示（仅当 location 可用时输出）
        if let Some(ref loc) = self.location {
            let width = line_width(loc.line);
            if let Some(col) = loc.column {
                writeln!(
                    f,
                    "{:>width$} --> {}:{}:{}",
                    "",
                    loc.file,
                    loc.line,
                    col,
                    width = width
                )?;
                if let Some(ref source_line) = self.source_line {
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(
                        f,
                        "{:>width$} | {}",
                        loc.line.to_string(),
                        source_line,
                        width = width
                    )?;
                    let caret_pad = col.saturating_sub(1).min(source_line.chars().count());
                    writeln!(
                        f,
                        "{:>width$} | {}^",
                        "",
                        " ".repeat(caret_pad),
                        width = width
                    )?;
                }
            } else {
                // 列号缺失（运行时错误，见 §1.2）：仅显示 file:line
                if let Some(ref source_line) = self.source_line {
                    writeln!(
                        f,
                        "{:>width$} --> {}:{}",
                        "",
                        loc.file,
                        loc.line,
                        width = width
                    )?;
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(
                        f,
                        "{:>width$} | {}",
                        loc.line.to_string(),
                        source_line,
                        width = width
                    )?;
                } else {
                    writeln!(
                        f,
                        "{:>width$} --> {}:{}",
                        "",
                        loc.file,
                        loc.line,
                        width = width
                    )?;
                }
            }
        }

        // 堆栈跟踪（spec 格式扩展，加 "Stack trace:" 标题，见 §0.3）
        if !self.stack_trace.frames.is_empty() {
            writeln!(f, "Stack trace:")?;
            for frame in &self.stack_trace.frames {
                writeln!(
                    f,
                    "    at {} ({}:{})",
                    frame.function_name, frame.location.file, frame.location.line
                )?;
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for CompileError {
    /// 参照 §7。编译时错误 column 来自 lexer/parser（恒为 Some），画 :col 与 ^ 行。
    /// 行号动态宽度、^ 上界保护、源码不可读时回退均与 RuntimeError 一致。
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "Compile error: {}", self.message)?;
        if let Some(ref loc) = self.location {
            let width = line_width(loc.line);
            if let Some(col) = loc.column {
                writeln!(
                    f,
                    "{:>width$} --> {}:{}:{}",
                    "",
                    loc.file,
                    loc.line,
                    col,
                    width = width
                )?;
                if let Some(ref source_line) = self.source_line {
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(
                        f,
                        "{:>width$} | {}",
                        loc.line.to_string(),
                        source_line,
                        width = width
                    )?;
                    let caret_pad = col.saturating_sub(1).min(source_line.chars().count());
                    writeln!(
                        f,
                        "{:>width$} | {}^--- here",
                        "",
                        " ".repeat(caret_pad),
                        width = width
                    )?;
                }
            } else {
                writeln!(
                    f,
                    "{:>width$} --> {}:{}",
                    "",
                    loc.file,
                    loc.line,
                    width = width
                )?;
            }
        }
        Ok(())
    }
}

impl MspError {
    /// 词法/解析错误的 (line, column)；其余返回 None。
    pub fn line_col(&self) -> Option<(usize, usize)> {
        match self {
            MspError::LexError { line, column, .. } | MspError::ParseError { line, column, .. } => {
                Some((*line, *column))
            }
            _ => None,
        }
    }

    /// 提取消息文本（去掉位置前缀），供 CompileError 使用。
    pub fn bare_message(&self) -> String {
        match self {
            MspError::LexError { message, .. }
            | MspError::ParseError { message, .. }
            | MspError::CompileError { message } => message.clone(),
            MspError::RuntimeError(m) => m.clone(),
            MspError::IoError(e) => e.to_string(),
        }
    }
}

/// 将词法/解析错误格式化为 CompileError 展示字符串。
/// `source` 为源码全文（用于抽取出错行）；`file` 为显示用文件名。
pub fn format_compile_error(file: &str, source: &str, e: &MspError) -> String {
    let (line, column) = match e.line_col() {
        Some(lc) => lc,
        None => return format!("Compile error: {}", e.bare_message()),
    };
    let source_line = source
        .lines()
        .nth(line.saturating_sub(1))
        .map(|s| s.to_string());
    let ce = CompileError {
        message: e.bare_message(),
        location: Some(SourceLocation {
            file: file.to_string(),
            line,
            column: Some(column),
        }),
        source_line,
    };
    ce.to_string()
}

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
        fn sample() -> Result<i32> {
            Ok(42)
        }
        assert_eq!(sample().unwrap(), 42);
    }

    // ---- task 57：Display 单元测试 ----

    use super::{CompileError, RuntimeError, SourceLocation, StackTrace, StackTraceFrame};

    #[test]
    fn test_runtime_error_titled() {
        let re = RuntimeError::new("oops".into());
        assert_eq!(format!("{}", re), "Error: oops\n");
    }

    #[test]
    fn test_runtime_error_raw_message() {
        // VM 内部 String 错误（§0.1 case 2）：逐字输出，不加 "Error:" 前缀。
        let mut re = RuntimeError::new("ZeroDivisionError: division by zero".into());
        re.titled = false;
        assert_eq!(format!("{}", re), "ZeroDivisionError: division by zero\n");
    }

    #[test]
    fn test_runtime_error_with_location_and_source() {
        // 运行时错误：column=None → 显示 file:line（无 :col），不画 ^ 行。
        let mut re = RuntimeError::new("ZeroDivisionError: division by zero".into());
        re.titled = false;
        re.location = Some(SourceLocation {
            file: "test.ms".into(),
            line: 1,
            column: None,
        });
        re.source_line = Some("x = 10 / 0".into());
        let out = format!("{}", re);
        assert!(out.contains("ZeroDivisionError: division by zero"));
        assert!(out.contains("--> test.ms:1"));
        assert!(out.contains("1 | x = 10 / 0"));
        // 运行时错误不画 ^ 行
        assert!(!out.contains('^'));
    }

    #[test]
    fn test_runtime_error_with_stack_trace() {
        let mut re = RuntimeError::new("oops".into());
        re.stack_trace = StackTrace {
            frames: vec![
                StackTraceFrame {
                    function_name: "bar".into(),
                    location: SourceLocation {
                        file: "main.ms".into(),
                        line: 6,
                        column: None,
                    },
                },
                StackTraceFrame {
                    function_name: "foo".into(),
                    location: SourceLocation {
                        file: "main.ms".into(),
                        line: 2,
                        column: None,
                    },
                },
            ],
        };
        let out = format!("{}", re);
        assert!(out.contains("Error: oops"));
        assert!(out.contains("Stack trace:"));
        assert!(out.contains("at bar (main.ms:6)"));
        assert!(out.contains("at foo (main.ms:2)"));
    }

    #[test]
    fn test_runtime_error_no_source_line_fallback() {
        // 源码行不可读（§9 回退）：仅输出 --> file:line，省略源码与 ^ 行。
        let mut re = RuntimeError::new("oops".into());
        re.titled = false;
        re.location = Some(SourceLocation {
            file: "x.ms".into(),
            line: 5,
            column: None,
        });
        re.source_line = None;
        let out = format!("{}", re);
        assert!(out.contains("--> x.ms:5"));
        assert!(!out.contains('|'));
    }

    #[test]
    fn test_compile_error_with_caret() {
        let ce = CompileError {
            message: "expected expression after '+'".into(),
            location: Some(SourceLocation {
                file: "test.ms".into(),
                line: 1,
                column: Some(7),
            }),
            source_line: Some("x = 1 + ".into()),
        };
        let out = format!("{}", ce);
        assert!(out.contains("Compile error: expected expression after '+'"));
        assert!(out.contains("--> test.ms:1:7"));
        assert!(out.contains("1 | x = 1 + "));
        assert!(out.contains("^--- here"));
    }

    #[test]
    fn test_format_compile_error_helper() {
        let e = MspError::ParseError {
            line: 1,
            column: 7,
            message: "expected expression after '+'".into(),
        };
        let out = super::format_compile_error("test.ms", "x = 1 + ", &e);
        assert!(out.contains("Compile error: expected expression after '+'"));
        assert!(out.contains("--> test.ms:1:7"));
        assert!(out.contains("^--- here"));
    }
}
