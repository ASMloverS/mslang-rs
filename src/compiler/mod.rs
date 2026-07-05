//! mslang 编译器核心框架（task 17）。
//!
//! 实现 Chunk（字节码块）、CompilationUnit（编译单元）、Compiler（编译器），
//! 包括常量池管理、字节码发射、跳转补丁、作用域管理和局部变量表。
//!
//! 参照 [11-bytecode-vm](../../../docs/mslang/11-bytecode-vm.md) 编译单元设计，
//! [03-syntax](../../../docs/mslang/03-syntax.md) 作用域规则。

pub mod expression;
pub mod opcode;
pub mod statement;

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
pub struct CompilationUnit {
    /// 字节码块（常量池 + 字节码 + 行号）
    pub chunk: Chunk,
    /// 局部变量表
    pub locals: Vec<Local>,
    /// 上值表
    pub upvalues: Vec<Upvalue>,
    /// 当前作用域深度
    pub scope_depth: usize,
    /// task 39：函数体编译期间是否出现 yield / yield from。函数编译完成时据此
    /// 设置 Function.is_generator。父单元的该字段不被读取（仅当前子单元有效）。
    pub is_generator: bool,
    /// 父编译单元（用于闭包上值解析）。
    ///
    /// 采用裸指针（clox 风格）规避 `self.unit` 经 `mem::replace` 换出/换入时
    /// 的 self-referential 借用冲突：`compile_fn_decl` 将 parent 指向被换出的
    /// `saved_unit`（编译函数体期间该局部存活，指针有效）。空指针表示无父单元。
    /// SAFETY: 仅在编译子函数体期间解引用（parent 指向存活的 saved_unit）。
    pub parent: *const CompilationUnit,
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
pub struct Compiler {
    unit: CompilationUnit,
    source_file: Option<String>,
    source_lines: Vec<String>,
    exports: Vec<String>,
    /// 循环上下文栈，支持 break/continue 与嵌套循环（最内层在栈顶）。
    current_loop: Vec<LoopContext>,
    /// task 37：当前正编译其 try body 的嵌套 try 数量。return/break/continue 在
    /// try body 内的 early-exit 出口须先 emit 等量 TRY_EXIT（注销已注册的 handler）。
    try_depth: usize,
    /// task 38：with 语句临时局部（保存上下文管理器）的唯一名计数器。每条 with 分配
    /// `_with_ctx_N`，使同函数作用域内嵌套 with 不冲突（with 不创建新作用域）。
    with_temp_counter: usize,
    gen_expr_counter: usize,
    /// 标记为 nonlocal 的变量名（当前函数作用域内有效）。
    nonlocal_names: std::collections::HashSet<String>,
    /// 标记为 global 的变量名（当前函数作用域内有效）。
    global_names: std::collections::HashSet<String>,
    /// task 42：当前正编译其方法体的类名（None = 不在类方法内）。
    /// compile_class_decl 进入方法编译前设置，Expr::SuperAccess 据此发射 GET_SUPER。
    current_class: Option<String>,
}

/// 循环上下文。break 跳到循环出口（前向），continue 跳到循环头（后向）。
struct LoopContext {
    /// 循环头指令偏移（continue 目标）。
    loop_start: usize,
    /// 待 patch 的 break 跳转操作数位置列表。
    break_jumps: Vec<usize>,
}

impl Compiler {
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
            is_generator: false,
            parent: std::ptr::null(),
        };
        Compiler {
            unit,
            source_file: None,
            source_lines: Vec::new(),
            exports: Vec::new(),
            current_loop: Vec::new(),
            try_depth: 0,
            with_temp_counter: 0,
            gen_expr_counter: 0,
            nonlocal_names: std::collections::HashSet::new(),
            global_names: std::collections::HashSet::new(),
            current_class: None,
        }
    }

    pub fn with_source(source: &str, file: Option<String>) -> Self {
        let mut compiler = Self::new();
        compiler.source_file = file;
        compiler.source_lines = source.lines().map(|l| l.to_string()).collect();
        compiler
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

// ---- 常量池管理 ----

impl Compiler {
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

impl Compiler {
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

    /// 发射函数返回点（task 36）：先 EXEC_DEFER 刷新本帧 defer（LIFO），再 RETURN。
    /// 所有 RETURN 站点（显式 return、函数末尾隐式 return、fn 字面量末尾）统一经此。
    pub fn emit_return(&mut self, line: usize) {
        self.emit_byte(OpCode::ExecDefer as u8, line);
        self.emit_byte(OpCode::Return as u8, line);
    }

    /// 发射跳转指令（占位 0xFFFF），返回跳转操作数的起始偏移（供 patch_jump 使用）。
    pub fn emit_jump(&mut self, opcode: OpCode, line: usize) -> usize {
        self.emit_byte(opcode as u8, line);
        self.emit_byte(0xff, line);
        self.emit_byte(0xff, line);
        self.unit.chunk.code.len() - 2
    }

    /// 发射 FOR_ITER 指令（iter_slot + 2 字节 exit_offset 占位）。
    /// 返回 exit_offset 操作数起始位置，供 [`patch_jump`](Self::patch_jump) 补丁。
    pub fn emit_for_iter(&mut self, iter_slot: u8, line: usize) -> usize {
        self.emit_byte(OpCode::ForIter as u8, line);
        self.emit_byte(iter_slot, line);
        self.emit_byte(0xff, line); // exit_offset placeholder
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

impl Compiler {
    /// 进入新作用域。
    pub fn begin_scope(&mut self) {
        self.unit.scope_depth += 1;
    }

    /// 离开作用域，弹出该作用域的局部变量。
    /// 被闭包捕获的局部发射 `CLOSE_UPVALUE`（关闭对应开放上值再弹栈），
    /// 其余发射 `POP`。
    pub fn end_scope(&mut self, line: usize) {
        self.unit.scope_depth = self.unit.scope_depth.saturating_sub(1);
        while let Some(local) = self.unit.locals.last() {
            if local.depth > self.unit.scope_depth {
                let local = self.unit.locals.pop().unwrap();
                if local.is_captured {
                    self.emit_byte(OpCode::CloseUpvalue as u8, line);
                } else {
                    self.emit_byte(OpCode::Pop as u8, line);
                }
            } else {
                break;
            }
        }
    }
}

// ---- 局部变量表管理 ----

impl Compiler {
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

    /// 解析上值。委托给自由函数 `resolve_upvalue_in_unit`，后者沿 parent 链递归
    /// 查找变量，并在中间单元中创建上值条目（clox 语义）。
    ///
    /// SAFETY: `self.unit.parent` 为裸指针。仅在编译子函数体期间解引用（此时 parent
    /// 指向存活的 `saved_unit`）。空指针表示无父单元。
    pub fn resolve_upvalue(&mut self, name: &str) -> Option<usize> {
        let unit_ptr: *mut CompilationUnit = &mut self.unit;
        // SAFETY: 编译子函数体期间，仅 self.unit 处于活跃使用。parent 链中的
        // CompilationUnit 经 mem::replace 保存，仅通过 parent 指针访问，无别名。
        unsafe { resolve_upvalue_in_unit(unit_ptr, name) }
    }
}

/// 向 `unit.upvalues` 去重添加上值条目，返回索引。
fn add_upvalue(unit: &mut CompilationUnit, index: usize, is_local: bool) -> usize {
    if let Some(existing) = unit
        .upvalues
        .iter()
        .position(|u| u.index == index && u.is_local == is_local)
    {
        return existing;
    }
    let idx = unit.upvalues.len();
    unit.upvalues.push(Upvalue { index, is_local });
    idx
}

/// 递归解析上值（clox resolveUpvalue 等价实现）。
///
/// 1. 检查直接 parent 的 locals → 命中则向 **当前** unit 添加 is_local=true 上值。
/// 2. 未命中 → 递归在 parent 中解析（可能向 parent 添加上值），再向当前 unit
///    添加 is_local=false 上值（复用 parent 的上值索引）。
///
/// SAFETY: `unit` 及其 parent 链中的 CompilationUnit 在编译期均存活，且仅通过
/// 此函数的裸指针访问（无别名竞争）。
unsafe fn resolve_upvalue_in_unit(unit: *mut CompilationUnit, name: &str) -> Option<usize> {
    let unit = &mut *unit;
    let parent_ptr = unit.parent;
    if parent_ptr.is_null() {
        return None;
    }
    let parent = &mut *(parent_ptr as *mut CompilationUnit);

    // 1. 在直接 parent 的 locals 中查找
    if let Some(idx) = parent.locals.iter().rposition(|l| l.name == name) {
        return Some(add_upvalue(unit, idx, true));
    }

    // 2. 递归在 parent 的上值链中查找
    resolve_upvalue_in_unit(parent_ptr as *mut CompilationUnit, name)
        .map(|upvalue_idx| add_upvalue(unit, upvalue_idx, false))
}

// ---- task 31：默认参数 / 可变参数辅助 ----

/// 校验参数顺序：普通 → 默认 → 可变（`04-functions.md:75`）。
/// 违序时返回编译期错误。
pub fn validate_param_order(params: &[crate::ast::node::Param]) -> Result<(), String> {
    // 0=normal, 1=default, 2=variadic
    let mut state = 0u8;
    for p in params {
        match (p.is_variadic, &p.default, state) {
            (false, None, _) => {
                if state > 0 {
                    return Err(
                        "positional parameter after default or variadic parameter".to_string()
                    );
                }
            }
            (false, Some(_), _) => {
                if state > 1 {
                    return Err("default parameter after variadic parameter".to_string());
                }
                state = 1;
            }
            // *rest 不应有 default（解析器不会产出 is_variadic=true && default=Some）
            (true, _, _) => state = 2,
        }
    }
    Ok(())
}

/// 编译期求值默认参数表达式。仅支持常量字面量（`04-functions.md:44`）。
/// 非常量默认值（如 `items = []`）暂不支持，返回编译期错误。
pub fn eval_default(expr: &crate::ast::node::Expr) -> Result<Object, String> {
    use crate::ast::node::{Expr, Literal};
    match expr {
        Expr::Literal(Literal::Int(n)) => Ok(Object::Int(*n)),
        Expr::Literal(Literal::Float(n)) => Ok(Object::Float(*n)),
        Expr::Literal(Literal::String(s)) => Ok(crate::vm::object::alloc_string(s)),
        Expr::Literal(Literal::Bool(b)) => Ok(Object::Bool(*b)),
        Expr::Literal(Literal::Nil) => Ok(Object::Nil),
        _ => Err(
            "default parameter value must be a constant literal (non-literal defaults not yet supported)"
                .to_string(),
        ),
    }
}

// ---- 编译入口 ----

impl Compiler {
    /// 编译程序，返回字节码块。
    pub fn compile(&mut self, program: &Program) -> Result<Chunk, String> {
        for stmt in &program.statements {
            self.compile_statement(stmt, 0)?;

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
        self.emit_byte(OpCode::ExecDefer as u8, 0);
        self.emit_byte(OpCode::Halt as u8, 0);
        Ok(std::mem::take(&mut self.unit.chunk))
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
        // 顶层返回点：EXEC_DEFER（task 36 顶层 defer）+ HALT。
        assert_eq!(chunk.code.len(), 2);
        assert_eq!(chunk.code[0], OpCode::ExecDefer as u8);
        assert_eq!(chunk.code[1], OpCode::Halt as u8);
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
