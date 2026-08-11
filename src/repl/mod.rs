//! REPL 交互式命令行（task 56）。
//!
//! 多行输入（结构化括号平衡 + 未终结字面量检测）、表达式求值（打印结果）、
//! 语句执行（不打印）、上下文持久化（VM globals/module_resolver 跨输入保持）、
//! 行编辑（rustyline）。参照 [56-repl](../../../docs/mslang/tasks/56-repl.md)。

use crate::ast::Stmt;
use crate::lexer::token::TokenKind;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::vm::VM;

use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::hint::Hinter;
use rustyline::highlight::Highlighter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};

/// 行编辑辅助。Helper 要求 Completer + Hinter + Highlighter + Validator 四者齐全。
/// Tab 补全与历史提示可后续迭代扩展（task 56 §5）。
struct ReplHelper;

impl Completer for ReplHelper {
    type Candidate = String;
}

impl Hinter for ReplHelper {
    type Hint = String;
}

impl Highlighter for ReplHelper {}
impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

/// Read-Eval-Print Loop。持有持久化 VM 与输入缓冲区。
pub struct Repl {
    pub vm: VM,
    editor: Editor<ReplHelper, DefaultHistory>,
    pub buffer: String,
}

impl Repl {
    pub fn new() -> Result<Self, String> {
        let vm = VM::new();
        let mut editor: Editor<ReplHelper, DefaultHistory> =
            Editor::new().map_err(|e| format!("{}", e))?;
        editor.set_helper(Some(ReplHelper));
        Ok(Self {
            vm,
            editor,
            buffer: String::new(),
        })
    }

    pub fn run(&mut self) -> Result<(), String> {
        println!("mslang {} REPL", env!("CARGO_PKG_VERSION"));
        println!("Type :quit to exit");
        loop {
            let prompt = if self.buffer.is_empty() { "> " } else { ". " };
            let line = match self.editor.readline(prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    // Ctrl+C：取消当前输入，清空 buffer 继续读
                    self.buffer.clear();
                    continue;
                }
                Err(ReadlineError::Eof) => break, // Ctrl+D：退出 REPL
                Err(e) => return Err(format!("{}", e)),
            };
            if line == ":quit" {
                break;
            }
            self.buffer.push_str(&line);
            self.buffer.push('\n');
            if self.is_complete() {
                self.evaluate_buffer();
                self.buffer.clear();
            }
        }
        Ok(())
    }

    /// 输入是否为完整语句块。判定：未终结字符串 → 否；括号不匹配 → 否；
    /// 末尾反斜杠 → 否；其余 → 是（语法错误交执行阶段报告）。
    fn is_complete(&self) -> bool {
        const MAX_BUFFER: usize = 64 * 1024;
        if self.buffer.len() > MAX_BUFFER {
            return true;
        }
        let tokens = match Lexer::new(&self.buffer).tokenize_all() {
            Ok(t) => t,
            Err(e) => return !format!("{}", e).contains("unterminated"),
        };
        let (mut parens, mut brackets, mut braces) = (0i32, 0i32, 0i32);
        for tok in &tokens {
            match tok.kind {
                TokenKind::LeftParen => parens += 1,
                TokenKind::RightParen => parens -= 1,
                TokenKind::LeftBracket => brackets += 1,
                TokenKind::RightBracket => brackets -= 1,
                TokenKind::LeftBrace => braces += 1,
                TokenKind::RightBrace => braces -= 1,
                _ => {}
            }
        }
        if parens > 0 || brackets > 0 || braces > 0 {
            return false;
        }
        if self.buffer.trim_end().ends_with('\\') {
            return false;
        }
        true
    }

    fn evaluate_buffer(&mut self) {
        let source = self.buffer.clone(); // clone 避免 &self.buffer 与 &mut self.vm 借用冲突
        if Self::is_expression(&source) {
            match self.vm.eval_expression(&source) {
                Ok(val) => {
                    println!("{}", val.display());
                    let _ = self.editor.add_history_entry(&source);
                }
                Err(e) => self.print_error(&e),
            }
        } else {
            match self.vm.exec(&source) {
                Ok(_) => {
                    let _ = self.editor.add_history_entry(&source);
                }
                Err(e) => self.print_error(&e),
            }
        }
    }

    /// 顶层是否为单个裸表达式（ExprStmt）。是 → 表达式求值并打印；否 → 语句执行。
    fn is_expression(source: &str) -> bool {
        let tokens = match Lexer::new(source).tokenize_all() {
            Ok(t) => t,
            Err(_) => return false,
        };
        match Parser::new(tokens).parse() {
            Ok(program) => {
                program.statements.len() == 1
                    && matches!(program.statements[0], Stmt::ExprStmt { .. })
            }
            Err(_) => false,
        }
    }

    fn print_error(&self, e: &str) {
        eprintln!("Error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::object::{alloc_string, Object};

    #[test]
    fn test_repl_expression() {
        let mut repl = Repl::new().unwrap();
        let result = repl.vm.eval_expression("1 + 2").unwrap();
        assert_eq!(result, Object::Int(3));
    }

    #[test]
    fn test_repl_persistence() {
        let mut repl = Repl::new().unwrap();
        repl.vm.exec("x = 42").unwrap();
        let result = repl.vm.eval_expression("x").unwrap();
        assert_eq!(result, Object::Int(42));
    }

    #[test]
    fn test_repl_persistence_var_keyword() {
        // 镜像 REPL 流程：带 `var` 关键字 + 行尾换行（buffer 实际内容）。
        let mut repl = Repl::new().unwrap();
        repl.vm.exec("var x = 10\n").unwrap();
        let result = repl.vm.eval_expression("x * 3\n").unwrap();
        assert_eq!(result, Object::Int(30));
    }

    #[test]
    fn test_repl_multiline() {
        let mut repl = Repl::new().unwrap();
        repl.buffer = "fn add(a, b) {".to_string();
        assert!(!repl.is_complete());
        repl.buffer = "fn add(a, b) {\n    return a + b\n}".to_string();
        assert!(repl.is_complete());
    }

    #[test]
    fn test_is_expression() {
        assert!(Repl::is_expression("1 + 2"));
        assert!(Repl::is_expression("x"));
        assert!(Repl::is_expression("print(1)")); // 函数调用 = ExprStmt → 按表达式求值
        assert!(!Repl::is_expression("x = 1")); // 赋值语句
        assert!(!Repl::is_expression("var x = 1")); // 声明语句
        assert!(!Repl::is_expression("if x { 1 }")); // 控制流
    }

    #[test]
    fn test_is_complete_unclosed_collection() {
        let mut repl = Repl::new().unwrap();
        repl.buffer = "[1, 2,".to_string();
        assert!(!repl.is_complete());
        repl.buffer = "[1, 2, 3]".to_string();
        assert!(repl.is_complete());
    }

    #[test]
    fn test_is_complete_unterminated_string() {
        let mut repl = Repl::new().unwrap();
        repl.buffer = "x = \"abc".to_string();
        assert!(!repl.is_complete());
    }

    #[test]
    fn test_eval_expression_rejects_statement() {
        let mut repl = Repl::new().unwrap();
        assert!(repl.vm.eval_expression("var x = 1").is_err());
    }

    #[test]
    fn test_exec_runtime_error_is_recoverable() {
        // 运行时错误后 VM 应能继续执行（reset_execution_state 清理中间态）。
        let mut repl = Repl::new().unwrap();
        assert!(repl.vm.eval_expression("10 / 0").is_err());
        let result = repl.vm.eval_expression("2 + 3").unwrap();
        assert_eq!(result, Object::Int(5));
    }

    #[test]
    fn test_object_display_quotes_string() {
        assert_eq!(alloc_string("hi").display(), "\"hi\"");
        assert_eq!(Object::Int(7).display(), "7");
        assert_eq!(Object::Nil.display(), "nil");
    }
}

