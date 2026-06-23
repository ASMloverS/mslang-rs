# 编译器核心框架

## 所属阶段
Phase 2.2a - 字节码编译 + VM 核心

## 前置任务
- 16-opcode-definition
- 20-object-system-basic

## 目标

实现编译器核心框架，包括 CompilationUnit、常量池管理、字节码发射、行号跟踪和局部变量表管理。编译器负责将 AST 节点翻译为字节码指令序列。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) 中的编译单元设计：

### CompilationUnit

```
CompilationUnit {
    constants: Vec<Value>          // 常量池
    code: Vec<u8>                  // 字节码
    lines: Vec<(usize, usize)>     // 行号信息（指令偏移, 源码行号）
    locals: Vec<Local>             // 局部变量表
    upvalues: Vec<Upvalue>         // 上值表
    parent: Option<&CompilationUnit>
}
```

### Local

```
Local {
    name: String
    depth: usize          // 作用域深度
    is_captured: bool     // 是否被闭包捕获
}
```

### Upvalue

```
Upvalue {
    index: usize          // 外层局部变量索引
    is_local: bool        // 是直接的外层局部变量，还是外层的上值
}
```

引用 [03-syntax.md](../03-syntax.md) 作用域规则：
- 函数级作用域（类似 Python）
- `if`/`while`/`for` 块不创建新作用域
- `var` 和 `:=` 在当前函数作用域创建新变量
- `=` 赋值时仅在当前函数作用域内查找，找不到则在当前作用域创建新变量

## 实现细节

### 文件位置

`src/compiler/mod.rs`（核心框架）
`src/compiler/opcode.rs`（指令集，任务 16 已定义）

### Chunk 结构体

```rust
pub struct Chunk {
    pub constants: Vec<Object>,
    pub code: Vec<u8>,
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
```

### CompilationUnit

```rust
pub struct CompilationUnit<'a> {
    pub chunk: Chunk,
    pub locals: Vec<Local>,
    pub upvalues: Vec<Upvalue>,
    pub scope_depth: usize,
    pub parent: Option<&'a CompilationUnit<'a>>,
}

pub struct Local {
    pub name: String,
    pub depth: usize,
    pub is_captured: bool,
}

pub struct Upvalue {
    pub index: usize,
    pub is_local: bool,
}
```

### Compiler 结构体

```rust
pub struct Compiler<'a> {
    unit: CompilationUnit<'a>,
    source_file: Option<String>,
    source_lines: Vec<String>,
    exports: Vec<String>,
}

impl<'a> Compiler<'a> {
    pub fn new() -> Self {
        let mut unit = CompilationUnit {
            chunk: Chunk::new(),
            locals: vec![Local {
                name: String::new(),
                depth: 0,
                is_captured: false,
            }],
            upvalues: Vec::new(),
            scope_depth: 0,
            parent: None,
        };
        Compiler { unit, source_file: None, source_lines: Vec::new(), exports: Vec::new() }
    }

    pub fn with_source(source: &str, file: Option<String>) -> Self {
        let mut compiler = Self::new();
        compiler.source_file = file;
        compiler.source_lines = source.lines().map(|l| l.to_string()).collect();
        compiler
    }
}
```

### 常量池管理

```rust
impl Compiler<'_> {
    pub fn add_constant(&mut self, value: Object) -> usize {
        if let Some(idx) = self.unit.chunk.constants.iter().position(|c| c == &value) {
            return idx;
        }
        let idx = self.unit.chunk.constants.len();
        self.unit.chunk.constants.push(value);
        idx
    }

    pub fn emit_constant(&mut self, value: Object, line: usize) -> Result<(), String> {
        let idx = self.add_constant(value);
        let idx = u16::try_from(idx)
            .map_err(|_| format!("constant pool overflow: more than 65535 constants"))?;
        self.emit_byte(OpCode::Constant as u8, line);
        self.emit_bytes(&idx.to_be_bytes(), line);
        Ok(())
    }
}
```

### 字节码发射

```rust
impl Compiler<'_> {
    pub fn emit_byte(&mut self, byte: u8, line: usize) {
        self.unit.chunk.code.push(byte);
        self.unit.chunk.lines.push((self.unit.chunk.code.len() - 1, line));
    }

    pub fn emit_bytes(&mut self, bytes: &[u8], line: usize) {
        for &b in bytes {
            self.emit_byte(b, line);
        }
    }

    pub fn emit_jump(&mut self, opcode: OpCode, line: usize) -> usize {
        self.emit_byte(opcode as u8, line);
        self.emit_byte(0xff, line);
        self.emit_byte(0xff, line);
        self.unit.chunk.code.len() - 2
    }

    pub fn patch_jump(&mut self, offset: usize) -> Result<(), String> {
        let code_len = self.unit.chunk.code.len();
        if offset + 2 > code_len {
            return Err(format!("invalid jump offset: {} out of range", offset));
        }
        let jump = code_len - offset - 2;
        let jump = u16::try_from(jump)
            .map_err(|_| format!("forward jump distance exceeds 65535"))?;
        let bytes = jump.to_be_bytes();
        self.unit.chunk.code[offset] = bytes[0];
        self.unit.chunk.code[offset + 1] = bytes[1];
        Ok(())
    }

    /// 后向跳转补丁（JUMP_BACK / CONTINUE）。跳转目标在当前代码位置之前。
    /// offset 为 emit_jump 返回的操作数起始位置，loop_start 为循环起始指令偏移。
    pub fn patch_jump_back(&mut self, offset: usize, loop_start: usize) -> Result<(), String> {
        if offset + 2 > self.unit.chunk.code.len() {
            return Err(format!("invalid jump offset: {} out of range", offset));
        }
        // 后向跳转：偏移量为负，用 i16 的补码表示
        let backward = (offset + 2) - loop_start;
        let backward = u16::try_from(backward)
            .map_err(|_| format!("backward jump distance exceeds 65535"))?;
        let bytes = backward.to_be_bytes();
        self.unit.chunk.code[offset] = bytes[0];
        self.unit.chunk.code[offset + 1] = bytes[1];
        Ok(())
    }
}

### 作用域管理

mslang 使用函数级作用域（`if`/`while`/`for` 块不创建新作用域）。`begin_scope` / `end_scope` 仅在函数边界和推导式隐式作用域（`03-syntax.md:528`）中使用。

```rust
impl Compiler<'_> {
    pub fn begin_scope(&mut self) {
        self.unit.scope_depth += 1;
    }

    pub fn end_scope(&mut self) {
        self.unit.scope_depth = self.unit.scope_depth.saturating_sub(1);
        // 弹出当前作用域的局部变量
        while let Some(local) = self.unit.locals.last() {
            if local.depth > self.unit.scope_depth {
                self.unit.locals.pop();
            } else {
                break;
            }
        }
    }
}
```

### 局部变量表管理

```rust
impl Compiler<'_> {
    pub fn declare_local(&mut self, name: &str, line: usize) -> Result<(), String> {
        let depth = self.unit.scope_depth;
        if self.unit.locals.iter().rev().take_while(|l| l.depth == depth).any(|l| l.name == name) {
            return Err(format!("line {}: variable '{}' already declared in this scope", line, name));
        }
        self.unit.locals.push(Local {
            name: name.to_string(),
            depth,
            is_captured: false,
        });
        Ok(())
    }

    pub fn resolve_local(&self, name: &str) -> Option<usize> {
        self.unit.locals.iter().rposition(|l| l.name == name)
    }

    pub fn resolve_upvalue(&mut self, name: &str) -> Option<usize> {
        let upvalue = match self.resolve_upvalue_recursive(name) {
            Some(idx) => idx,
            None => return None,
        };
        if let Some(existing) = self.unit.upvalues.iter().position(|u| u.index == upvalue.0 && u.is_local == upvalue.1) {
            return Some(existing);
        }
        let idx = self.unit.upvalues.len();
        self.unit.upvalues.push(Upvalue {
            index: upvalue.0,
            is_local: upvalue.1,
        });
        Some(idx)
    }

    /// 递归查找上值。先在直接 parent 的 locals 中查找（is_local=true）；
    /// 若未找到，递归在 parent 的 upvalues 中查找（is_local=false）。
    fn resolve_upvalue_recursive(&self, name: &str) -> Option<(usize, bool)> {
        let parent = self.unit.parent?;
        if let Some(idx) = parent.locals.iter().rposition(|l| l.name == name) {
            Some((idx, true))
        } else {
            // 递归：在 parent 的上值链中查找
            parent.resolve_upvalue_recursive(name).map(|(idx, _)| (idx, false))
        }
    }
}
```

### 编译入口

```rust
impl Compiler<'_> {
    pub fn compile(&mut self, program: &Program) -> Result<Chunk, String> {
        for stmt in &program.statements {
            self.compile_statement(stmt)?;

            // 记录顶层导出名（供模块系统使用）
            if self.unit.scope_depth == 0 {
                match stmt {
                    Stmt::FnDecl { name, .. } | Stmt::ClassDecl { name, .. } => {
                        self.exports.push(name.clone());
                    }
                    Stmt::VarDecl { name, .. } => {} // var 声明私有，不导出
                    Stmt::ConstDecl { name, .. } => {
                        self.exports.push(name.clone());
                    }
                    _ => {}
                }
            }
        }
        self.emit_byte(OpCode::Halt as u8, 0);
        Ok(std::mem::replace(&mut self.unit.chunk, Chunk::new()))
    }

    /// 语句编译入口。task 17 提供空实现（仅 ExprStmt/空程序可通过），
    /// 完整实现由 task 18（表达式编译）和 task 19（语句编译）逐步填充。
    fn compile_statement(&mut self, stmt: &Stmt) -> Result<(), String> {
        // task 17 框架仅确保编译入口可调用；具体语句编译见 task 18/19。
        // 空程序（statements 为空）不触发此方法。
        match stmt {
            _ => Err(format!("compile_statement not yet implemented (task 18/19)")),
        }
    }
}
```

### 导出列表

`exports` 字段（已在上方 Compiler 结构体中定义）记录顶层 fn/class/const 声明的名称。模块系统（Task 45）通过此列表过滤导出内容。

`source_file` 和 `source_lines` 保留用于 task 57（友好错误信息与堆栈跟踪）的行号查找。

## 验证标准

1. 能创建 Compiler 实例并编译空程序（仅产生 HALT 指令）
2. 常量池能正确去重和索引
3. `emit_constant` 正确生成 CONSTANT + 2 字节索引
4. `emit_jump` / `patch_jump` 正确处理跳转偏移
5. `declare_local` / `resolve_local` 正确管理局部变量
6. 行号信息正确记录

## 测试用例

```ms
# empty program - should compile to just HALT
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
```
