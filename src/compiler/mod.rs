//! mslang 编译器核心框架（task 17）。
//!
//! 实现 Chunk（字节码块）、CompilationUnit（编译单元）、Compiler（编译器），
//! 包括常量池管理、字节码发射、跳转补丁、作用域管理和局部变量表。
//!
//! 参照 [11-bytecode-vm](../../../docs/mslang/11-bytecode-vm.md) 编译单元设计，
//! [03-syntax](../../../docs/mslang/03-syntax.md) 作用域规则。

pub mod opcode;

use crate::ast::node::{Program, Stmt};
use crate::vm::object::Object;

pub use opcode::OpCode;

/// 字节码块。包含常量池、字节码序列和行号信息。
#[derive(Debug, Clone)]
pub struct Chunk {
    /// 常量池
    pub constants: Vec<Object>,
    /// 字节码序列
    pub code: Vec<u8>,
    /// 行号信息（指令偏移, 源码行号）
    pub lines: Vec<(usize, usize)>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),
            code: Vec::new(),
            lines: Vec::new(),
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// 编译单元。每个编译单元对应一个函数或脚本顶层。
pub struct CompilationUnit<'a> {
    /// 字节码块（常量池 + 字节码 + 行号）
    pub chunk: Chunk,
    /// 局部变量表
    pub locals: Vec<Local>,
    /// 上值表
    pub upvalues: Vec<Upvalue>,
    /// 当前作用域深度
    pub scope_depth: usize,
    /// 父编译单元（用于闭包上值解析）
    pub parent: Option<&'a CompilationUnit<'a>>,
}

/// 局部变量。
#[derive(Debug, Clone)]
pub struct Local {
    /// 变量名
    pub name: String,
    /// 作用域深度
    pub depth: usize,
    /// 是否被闭包捕获
    pub is_captured: bool,
}

/// 上值。
#[derive(Debug, Clone)]
pub struct Upvalue {
    /// 外层局部变量或上值的索引
    pub index: usize,
    /// true = 直接的外层局部变量；false = 外层的上值
    pub is_local: bool,
}

/// 编译器。将 AST 编译为字节码。
pub struct Compiler<'a> {
    unit: CompilationUnit<'a>,
    source_file: Option<String>,
    source_lines: Vec<String>,
    exports: Vec<String>,
}

impl<'a> Compiler<'a> {
    pub fn new() -> Self {
        let unit = CompilationUnit {
            chunk: Chunk::new(),
            // slot 0 预留给函数自身（脚本顶层为空名）
            locals: vec![Local {
                name: String::new(),
                depth: 0,
                is_captured: false,
            }],
            upvalues: Vec::new(),
            scope_depth: 0,
            parent: None,
        };
        Compiler {
            unit,
            source_file: None,
            source_lines: Vec::new(),
            exports: Vec::new(),
        }
    }

    pub fn with_source(source: &str, file: Option<String>) -> Self {
        let mut compiler = Self::new();
        compiler.source_file = file;
        compiler.source_lines = source.lines().map(|l| l.to_string()).collect();
        compiler
    }
}

impl Default for Compiler<'_> {
    fn default() -> Self {
        Self::new()
    }
}

// ---- 常量池管理 ----

impl Compiler<'_> {
    /// 添加常量到常量池（自动去重），返回索引。
    pub fn add_constant(&mut self, value: Object) -> usize {
        if let Some(idx) = self.unit.chunk.constants.iter().position(|c| c == &value) {
            return idx;
        }
        let idx = self.unit.chunk.constants.len();
        self.unit.chunk.constants.push(value);
        idx
    }

    /// 发射常量加载指令。CONSTANT + 2 字节索引（大端序）。
    pub fn emit_constant(&mut self, value: Object, line: usize) -> Result<(), String> {
        let idx = self.add_constant(value);
        let idx = u16::try_from(idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
        self.emit_byte(OpCode::Constant as u8, line);
        self.emit_bytes(&idx.to_be_bytes(), line);
        Ok(())
    }
}

// ---- 字节码发射 ----

impl Compiler<'_> {
    /// 发射单字节并记录行号。
    pub fn emit_byte(&mut self, byte: u8, line: usize) {
        self.unit.chunk.code.push(byte);
        self.unit
            .chunk
            .lines
            .push((self.unit.chunk.code.len() - 1, line));
    }

    /// 发射多字节。
    pub fn emit_bytes(&mut self, bytes: &[u8], line: usize) {
        for &b in bytes {
            self.emit_byte(b, line);
        }
    }

    /// 发射跳转指令（占位 0xFFFF），返回跳转操作数的起始偏移（供 patch_jump 使用）。
    pub fn emit_jump(&mut self, opcode: OpCode, line: usize) -> usize {
        self.emit_byte(opcode as u8, line);
        self.emit_byte(0xff, line);
        self.emit_byte(0xff, line);
        self.unit.chunk.code.len() - 2
    }

    /// 补丁前向跳转（JUMP / JUMP_IF_FALSE / JUMP_IF_TRUE）。
    /// `offset` 为 [`emit_jump`](Self::emit_jump) 返回的操作数起始位置。
    pub fn patch_jump(&mut self, offset: usize) -> Result<(), String> {
        let code_len = self.unit.chunk.code.len();
        if offset + 2 > code_len {
            return Err(format!("invalid jump offset: {} out of range", offset));
        }
        let jump = code_len - offset - 2;
        let jump =
            u16::try_from(jump).map_err(|_| "forward jump distance exceeds 65535".to_string())?;
        let bytes = jump.to_be_bytes();
        self.unit.chunk.code[offset] = bytes[0];
        self.unit.chunk.code[offset + 1] = bytes[1];
        Ok(())
    }

    /// 后向跳转补丁（JUMP_BACK / CONTINUE）。跳转目标在当前代码位置之前。
    /// `offset` 为 [`emit_jump`](Self::emit_jump) 返回的操作数起始位置，
    /// `loop_start` 为循环起始指令偏移。
    pub fn patch_jump_back(&mut self, offset: usize, loop_start: usize) -> Result<(), String> {
        if offset + 2 > self.unit.chunk.code.len() {
            return Err(format!("invalid jump offset: {} out of range", offset));
        }
        // 后向跳转：偏移量为目标到操作数末尾的距离
        let backward = (offset + 2) - loop_start;
        let backward = u16::try_from(backward)
            .map_err(|_| "backward jump distance exceeds 65535".to_string())?;
        let bytes = backward.to_be_bytes();
        self.unit.chunk.code[offset] = bytes[0];
        self.unit.chunk.code[offset + 1] = bytes[1];
        Ok(())
    }
}

// ---- 作用域管理 ----
//
// mslang 使用函数级作用域（`if`/`while`/`for` 块不创建新作用域）。
// begin_scope / end_scope 仅在函数边界和推导式隐式作用域中使用。

impl Compiler<'_> {
    /// 进入新作用域。
    pub fn begin_scope(&mut self) {
        self.unit.scope_depth += 1;
    }

    /// 离开作用域，弹出该作用域的局部变量。
    pub fn end_scope(&mut self) {
        self.unit.scope_depth = self.unit.scope_depth.saturating_sub(1);
        while let Some(local) = self.unit.locals.last() {
            if local.depth > self.unit.scope_depth {
                self.unit.locals.pop();
            } else {
                break;
            }
        }
    }
}

// ---- 局部变量表管理 ----

impl Compiler<'_> {
    /// 在当前作用域声明局部变量。重复声明返回错误。
    pub fn declare_local(&mut self, name: &str, line: usize) -> Result<(), String> {
        let depth = self.unit.scope_depth;
        if self
            .unit
            .locals
            .iter()
            .rev()
            .take_while(|l| l.depth == depth)
            .any(|l| l.name == name)
        {
            return Err(format!(
                "line {}: variable '{}' already declared in this scope",
                line, name
            ));
        }
        self.unit.locals.push(Local {
            name: name.to_string(),
            depth,
            is_captured: false,
        });
        Ok(())
    }

    /// 解析局部变量，返回在局部变量表中的索引（最内层优先）。
    pub fn resolve_local(&self, name: &str) -> Option<usize> {
        self.unit.locals.iter().rposition(|l| l.name == name)
    }

    /// 解析上值。先递归查找 parent 链，再在上值表中去重。
    pub fn resolve_upvalue(&mut self, name: &str) -> Option<usize> {
        let upvalue = self.unit.resolve_upvalue_recursive(name)?;
        if let Some(existing) = self
            .unit
            .upvalues
            .iter()
            .position(|u| u.index == upvalue.0 && u.is_local == upvalue.1)
        {
            return Some(existing);
        }
        let idx = self.unit.upvalues.len();
        self.unit.upvalues.push(Upvalue {
            index: upvalue.0,
            is_local: upvalue.1,
        });
        Some(idx)
    }
}

impl CompilationUnit<'_> {
    /// 递归查找上值。先在直接 parent 的 locals 中查找（is_local=true）；
    /// 若未找到，递归在 parent 的上值链中查找（is_local=false）。
    fn resolve_upvalue_recursive(&self, name: &str) -> Option<(usize, bool)> {
        let parent = self.parent?;
        if let Some(idx) = parent.locals.iter().rposition(|l| l.name == name) {
            Some((idx, true))
        } else {
            parent
                .resolve_upvalue_recursive(name)
                .map(|(idx, _)| (idx, false))
        }
    }
}

// ---- 编译入口 ----

impl Compiler<'_> {
    /// 编译程序，返回字节码块。
    pub fn compile(&mut self, program: &Program) -> Result<Chunk, String> {
        for stmt in &program.statements {
            self.compile_statement(stmt)?;

            // 记录顶层导出名（供模块系统使用）
            if self.unit.scope_depth == 0 {
                match stmt {
                    Stmt::FnDecl { name, .. } | Stmt::ClassDecl { name, .. } => {
                        self.exports.push(name.clone());
                    }
                    Stmt::ConstDecl { name, .. } => {
                        self.exports.push(name.clone());
                    }
                    // var 声明私有，不导出；其他语句不产生导出
                    _ => {}
                }
            }
        }
        self.emit_byte(OpCode::Halt as u8, 0);
        Ok(std::mem::take(&mut self.unit.chunk))
    }

    /// 语句编译入口。task 17 提供空实现（仅空程序可通过），
    /// 完整实现由 task 18（表达式编译）和 task 19（语句编译）逐步填充。
    fn compile_statement(&mut self, _stmt: &Stmt) -> Result<(), String> {
        // task 17 框架仅确保编译入口可调用；具体语句编译见 task 18/19。
        // 空程序（statements 为空）不触发此方法。
        Err("compile_statement not yet implemented (task 18/19)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::node::Program;
    use crate::vm::object::Object;

    #[test]
    fn test_empty_program_compiles() {
        let mut compiler = Compiler::new();
        let program = Program { statements: vec![] };
        let chunk = compiler.compile(&program).unwrap();
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(chunk.code[0], OpCode::Halt as u8);
    }

    #[test]
    fn test_add_constant_deduplicates() {
        let mut compiler = Compiler::new();
        let idx1 = compiler.add_constant(Object::Int(42));
        let idx2 = compiler.add_constant(Object::Int(42));
        assert_eq!(idx1, idx2);
        assert_eq!(idx1, 0);
    }

    #[test]
    fn test_emit_constant() {
        let mut compiler = Compiler::new();
        compiler.emit_constant(Object::Int(10), 1).unwrap();
        assert_eq!(compiler.unit.chunk.code[0], OpCode::Constant as u8);
        assert_eq!(compiler.unit.chunk.constants.len(), 1);
        assert_eq!(compiler.unit.chunk.constants[0], Object::Int(10));
    }

    #[test]
    fn test_jump_patch() {
        let mut compiler = Compiler::new();
        let jump_offset = compiler.emit_jump(OpCode::Jump, 1);
        compiler.emit_byte(OpCode::Nil as u8, 1);
        compiler.emit_byte(OpCode::Nil as u8, 1);
        compiler.patch_jump(jump_offset).unwrap();
        let offset = u16::from_be_bytes([
            compiler.unit.chunk.code[jump_offset],
            compiler.unit.chunk.code[jump_offset + 1],
        ]);
        assert_eq!(offset, 2);
    }

    #[test]
    fn test_local_variable_management() {
        let mut compiler = Compiler::new();
        compiler.declare_local("x", 1).unwrap();
        compiler.declare_local("y", 1).unwrap();
        assert_eq!(compiler.resolve_local("x"), Some(1));
        assert_eq!(compiler.resolve_local("y"), Some(2));
        assert_eq!(compiler.resolve_local("z"), None);
    }

    #[test]
    fn test_declare_local_duplicate_returns_error() {
        let mut compiler = Compiler::new();
        compiler.declare_local("x", 1).unwrap();
        let result = compiler.declare_local("x", 2);
        assert!(result.is_err());
    }
}
