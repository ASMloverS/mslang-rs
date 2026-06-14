# 语句编译

## 所属阶段
Phase 2.2c - 字节码编译 + VM 核心

## 前置任务
- 18-compile-expressions

## 目标

实现所有语句类型的编译逻辑，将 AST 语句节点翻译为字节码指令序列。重点处理控制流的跳转指令和循环的 patch 机制。

## 设计规格

引用 [03-syntax.md](../03-syntax.md) 语句语法，[05-control-flow.md](../05-control-flow.md) 控制流语义。

### 语句编译规则

| 语句类型 | 编译策略 | 产生的指令 |
|---|---|---|
| var/const 声明 | 求值右值 → 存入局部/全局 | 表达式... `STORE_LOCAL`/`STORE_GLOBAL` |
| 赋值语句 | 求值右值 → 存入目标 | 表达式... `STORE_*` |
| 表达式语句 | 求值表达式 → 弹出结果 | 表达式... `POP` |
| if/elif/else | 条件跳转 + 无条件跳转 | `JUMP_IF_FALSE` ... `JUMP` ... |
| while | 跳转到条件检查 → 循环体 → `JUMP_BACK` | `JUMP_IF_FALSE` ... `JUMP_BACK` |
| for..in | 迭代器 + 条件跳转 | `ITERATOR` `FOR_ITER` ... `JUMP_BACK` |
| break | 跳出循环 | `BREAK`（待 patch） |
| continue | 跳到循环头 | `CONTINUE`（待 patch） |
| return | 求值 → 返回 | 表达式... `RETURN` |
| block | 顺序编译内部语句 | 各语句的字节码 |

## 实现细节

### 文件位置

`src/compiler/statement.rs`

### 编译 var/const 声明

```rust
fn compile_var_decl(&mut self, name: &str, init: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(init)?;
    self.declare_local(name, line);
    let slot = self.resolve_local(name).unwrap();
    self.emit_byte(OpCode::StoreLocal as u8, line);
    self.emit_byte(slot as u8, line);
    Ok(())
}

fn compile_global_assign(&mut self, name: &str, value: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(value)?;
    let name_idx = self.add_constant(alloc_string(name));
    self.emit_byte(OpCode::StoreGlobal as u8, line);
    self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
    Ok(())
}
```

### 编译赋值语句（含复合赋值）

```rust
fn compile_assign_stmt(&mut self, target: &AssignTarget, op: Option<&CompoundOp>, value: &Expr, line: usize) -> Result<(), String> {
    if let Some(compound_op) = op {
        // 复合赋值: x += 5 => x = x + 5
        self.compile_assign_load(target)?;
        self.compile_expression(value)?;
        let opcode = self.compound_opcode(compound_op);
        self.emit_byte(opcode as u8, line);
    } else {
        self.compile_expression(value)?;
    }
    self.compile_assign_store(target, line)
}

fn compound_opcode(&self, op: &CompoundOp) -> OpCode {
    match op {
        CompoundOp::Add => OpCode::Add,
        CompoundOp::Sub => OpCode::Subtract,
        CompoundOp::Mul => OpCode::Multiply,
        CompoundOp::Div => OpCode::Divide,
        CompoundOp::FloorDiv => OpCode::FloorDiv,
        CompoundOp::Mod => OpCode::Modulo,
        CompoundOp::Power => OpCode::Power,
        CompoundOp::BitAnd => OpCode::BitAnd,
        CompoundOp::BitOr => OpCode::BitOr,
        CompoundOp::BitXor => OpCode::BitXor,
        CompoundOp::LeftShift => OpCode::LeftShift,
        CompoundOp::RightShift => OpCode::RightShift,
    }
}
```

### 编译表达式语句

```rust
fn compile_expr_stmt(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(expr)?;
    self.emit_byte(OpCode::Pop as u8, line);
    Ok(())
}
```

### 编译 if/elif/else

```rust
fn compile_if(&mut self, branches: &[(Expr, Vec<Stmt>)], else_body: &Option<Vec<Stmt>>, line: usize) -> Result<(), String> {
    let mut end_jumps = Vec::new();

    for (condition, body) in branches {
        self.compile_expression(condition)?;
        let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line);
        for stmt in body {
            self.compile_statement(stmt)?;
        }
        end_jumps.push(self.emit_jump(OpCode::Jump, line));
        self.patch_jump(else_jump);
        self.emit_byte(OpCode::Pop as u8, line);
    }

    if let Some(else_body) = else_body {
        for stmt in else_body {
            self.compile_statement(stmt)?;
        }
    }

    for jump in end_jumps {
        self.patch_jump(jump);
    }
    Ok(())
}
```

### 编译 while 循环

```rust
struct LoopContext {
    loop_start: usize,
    break_jumps: Vec<usize>,
}

fn compile_while(&mut self, condition: &Expr, body: &[Stmt], line: usize) -> Result<(), String> {
    let loop_start = self.unit.chunk.code.len();

    self.compile_expression(condition)?;
    let exit_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.emit_byte(OpCode::Pop as u8, line);

    let mut loop_ctx = LoopContext {
        loop_start,
        break_jumps: Vec::new(),
    };
    self.current_loop = Some(&mut loop_ctx);

    for stmt in body {
        self.compile_statement(stmt)?;
    }

    self.emit_loop(loop_start, line);
    self.patch_jump(exit_jump);
    self.emit_byte(OpCode::Pop as u8, line);

    for jump in &loop_ctx.break_jumps {
        self.patch_jump(*jump);
    }
    self.current_loop = None;
    Ok(())
}

fn emit_loop(&mut self, loop_start: usize, line: usize) {
    self.emit_byte(OpCode::JumpBack as u8, line);
    let offset = (self.unit.chunk.code.len() - loop_start + 2) as u16;
    let bytes = offset.to_be_bytes();
    self.emit_byte(bytes[0], line);
    self.emit_byte(bytes[1], line);
}
```

### 编译 for..in 循环

```rust
fn compile_for_in(&mut self, var_name: &str, iterable: &Expr, body: &[Stmt], line: usize) -> Result<(), String> {
    self.compile_expression(iterable)?;
    self.emit_byte(OpCode::Iterator as u8, line);

    let loop_start = self.unit.chunk.code.len();
    self.emit_byte(OpCode::ForIter as u8, line);
    let exit_jump_placeholder = self.unit.chunk.code.len();
    self.emit_byte(0xff, line);
    self.emit_byte(0xff, line);

    self.declare_local(var_name, line);
    let slot = self.resolve_local(var_name).unwrap();
    self.emit_byte(OpCode::StoreLocal as u8, line);
    self.emit_byte(slot as u8, line);

    let mut loop_ctx = LoopContext {
        loop_start,
        break_jumps: Vec::new(),
    };
    self.current_loop = Some(&mut loop_ctx);

    for stmt in body {
        self.compile_statement(stmt)?;
    }

    self.current_loop = None;
    self.emit_loop(loop_start, line);

    let exit_offset = (self.unit.chunk.code.len() - exit_jump_placeholder - 2) as u16;
    let bytes = exit_offset.to_be_bytes();
    self.unit.chunk.code[exit_jump_placeholder] = bytes[0];
    self.unit.chunk.code[exit_jump_placeholder + 1] = bytes[1];

    for jump in &loop_ctx.break_jumps {
        self.patch_jump(*jump);
    }
    Ok(())
}
```

### 编译 break/continue

```rust
fn compile_break(&mut self, line: usize) -> Result<(), String> {
    let loop_ctx = self.current_loop.as_ref().expect("break outside loop");
    let jump = self.emit_jump(OpCode::Jump, line);
    loop_ctx.break_jumps.push(jump);
    Ok(())
}

fn compile_continue(&mut self, line: usize) -> Result<(), String> {
    let loop_ctx = self.current_loop.as_ref().expect("continue outside loop");
    self.emit_loop(loop_ctx.loop_start, line);
    Ok(())
}
```

### 编译 return

```rust
fn compile_return(&mut self, value: &Option<Expr>, line: usize) -> Result<(), String> {
    if let Some(expr) = value {
        self.compile_expression(expr)?;
    } else {
        self.emit_byte(OpCode::Nil as u8, line);
    }
    self.emit_byte(OpCode::Return as u8, line);
    Ok(())
}
```

### 编译 nonlocal / global 声明

`nonlocal` 和 `global` 声明不产生字节码指令，仅在编译器的符号表中标记变量绑定语义：

```rust
fn compile_nonlocal(&mut self, names: &[String]) -> Result<(), String> {
    for name in names {
        self.mark_nonlocal(name);
    }
    Ok(())
}

fn compile_global(&mut self, names: &[String]) -> Result<(), String> {
    for name in names {
        self.mark_global(name);
    }
    Ok(())
}
```

- `mark_nonlocal(name)` 将变量标记为绑定到外层函数作用域
- `mark_global(name)` 将变量标记为绑定到全局作用域
- 后续对该变量的 `LOAD`/`STORE` 指令选择将依据这些标记决定

### 编译 block 语句

```rust
fn compile_block(&mut self, stmts: &[Stmt]) -> Result<(), String> {
    for stmt in stmts {
        self.compile_statement(stmt)?;
    }
    Ok(())
}
```

## 验证标准

1. `var x = 10` 编译为 `CONSTANT` + `STORE_LOCAL`
2. `if/elif/else` 编译为正确的条件跳转链，所有跳转被正确 patch
3. `while` 编译为循环条件检查 → 循环体 → `JUMP_BACK`
4. `for..in` 编译为 `ITERATOR` + `FOR_ITER` 循环
5. `break` 跳转到循环出口，`continue` 跳到循环头
6. `return` 编译为求值 + `RETURN`
7. 所有跳转偏移在编译结束时被正确 patch

## 测试用例

```ms
# test_compile_statements.ms
x = 10

if x > 5 {
    y = 1
} else {
    y = 2
}

i = 0
while i < 3 {
    i += 1
}
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_if_else() {
        let source = r#"
            x = 10
            if x > 5 {
                y = 1
            } else {
                y = 2
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        assert!(chunk.code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(chunk.code.contains(&(OpCode::Jump as u8)));
        assert!(chunk.code.contains(&(OpCode::Halt as u8)));
    }

    #[test]
    fn test_compile_while() {
        let source = r#"
            i = 0
            while i < 3 {
                i += 1
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        assert!(chunk.code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(chunk.code.contains(&(OpCode::JumpBack as u8)));
    }

    #[test]
    fn test_all_jumps_patched() {
        let source = r#"
            x = 1
            if x > 0 {
                y = 1
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        for (i, &byte) in chunk.code.iter().enumerate() {
            if byte == OpCode::Jump as u8 || byte == OpCode::JumpIfFalse as u8 {
                let offset = u16::from_be_bytes([chunk.code[i + 1], chunk.code[i + 2]]);
                assert_ne!(offset, 0xffff, "Unpatched jump at offset {}", i);
            }
        }
    }
}
```
