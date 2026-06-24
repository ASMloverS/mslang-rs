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

### 作用范围

本 task 覆盖上表所列语句类型（含 `var`/`:=`/`const`、赋值、表达式语句、if/elif/else、while、for..in、break、continue、return、block、nonlocal、global）。

以下语句类型**由后续 task 实现**，dispatcher 中返回 `Err` 标注对应 task：

| 语句 | 实现 task |
|---|---|
| `FnDecl` | 27（调用帧）/ 29（匿名函数） |
| `ClassDecl` | 40 |
| `Defer` | 36 |
| `Try` | 37 |
| `With` | 38 |
| `Import`/`FromImport` | 45 |
| `Throw` | 37 |

> **多目标赋值**（`a, b = 1, 2`，`03-syntax.md:140`）与**多返回值**（`return a, b, c`）的完整解包语义由 task 30 实现。本 task 的赋值/return 编译对 `TupleLiteral` 目标仅做最小处理或显式推迟，不实现 `UNPACK` 数量校验。

## 实现细节

### 文件位置

`src/compiler/statement.rs`

### Compiler 结构体扩展

本 task 需修改 `src/compiler/mod.rs` 中 task 17 定义的 `Compiler` 结构体，新增循环上下文栈与 nonlocal/global 标记：

```rust
pub struct Compiler<'a> {
    unit: CompilationUnit<'a>,
    source_file: Option<String>,
    source_lines: Vec<String>,
    exports: Vec<String>,
    // —— task 19 新增 ——
    /// 循环上下文栈，支持 break/continue 与嵌套循环（最内层在栈顶）。
    current_loop: Vec<LoopContext>,
    /// 标记为 nonlocal 的变量名（当前函数作用域内有效）。
    nonlocal_names: std::collections::HashSet<String>,
    /// 标记为 global 的变量名（当前函数作用域内有效）。
    global_names: std::collections::HashSet<String>,
}

/// 循环上下文。break 跳到循环出口（前向），continue 跳到循环头（后向）。
struct LoopContext {
    /// 循环头指令偏移（continue 目标）。
    loop_start: usize,
    /// 待 patch 的 break 跳转操作数位置列表。
    break_jumps: Vec<usize>,
}
```

> 用栈（`Vec<LoopContext>`）而非单个引用：循环可能嵌套（`05-control-flow.md:144`），且 break/continue 只影响最内层循环。栈顶即当前循环。`current_loop` 必须是值类型字段，**不可**存储指向局部变量的 `&mut`（否则与循环体内 `&mut self` 调用冲突，无法通过借用检查）。

### 编译 var/const 声明

`var x = expr` 与 `x := expr` 语义等价（`03-syntax.md:48-60`），均创建新局部变量，共用本方法。`const` 除额外校验外编译路径相同。

```rust
/// 编译 var/短声明/const 声明。三者均：求值右值 → 声明局部 → 存入 slot。
/// `is_const` 为 true 时先做常量表达式校验（见下文）。
fn compile_var_decl(&mut self, name: &str, init: &Expr, is_const: bool, line: usize) -> Result<(), String> {
    if is_const {
        self.validate_const_expr(init, line)?;
    }
    self.compile_expression(init, line)?;
    self.declare_local(name, line)?;
    let slot = self.resolve_local(name)
        .ok_or_else(|| format!("internal: local '{}' not found after declare", name))?;
    self.emit_byte(OpCode::StoreLocal as u8, line);
    self.emit_byte(slot as u8, line);
    Ok(())
}
```

**常量表达式校验**（`03-syntax.md:73-81`）：`const` 右侧仅允许字面量、其他 const 引用、一元取反（`-`/`~`）、二元算术（`+ - * / // % ** & | ^ << >>`）与括号分组。出现函数调用、变量引用（非 const）、方法调用、比较/逻辑运算、集合构造时返回编译错误。完整常量折叠求值可在 task 17 的常量池基础上扩展，本 task 至少做语法形式校验。

```rust
/// 全局变量赋值：`=` 赋值给全局变量（仅在全局作用域或变量已标记 global 时）。
/// 函数作用域内的 `=` 赋值默认创建/更新局部变量，不走此路径（见 dispatcher）。
fn compile_global_assign(&mut self, name: &str, value: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(value, line)?;
    let name_idx = self.add_constant(alloc_string(name));
    let name_idx = u16::try_from(name_idx)
        .map_err(|_| "constant pool overflow".to_string())?;
    self.emit_byte(OpCode::StoreGlobal as u8, line);
    self.emit_bytes(&name_idx.to_be_bytes(), line);
    Ok(())
}
```

### 编译赋值语句（含复合赋值）

赋值语句复用 task 18 的 `compile_assignment`（表达式语义，已处理复合赋值与 `DUP`），编译后栈顶留有赋值结果值；语句需额外 `POP` 丢弃该值以保持栈平衡。

```rust
/// 编译赋值语句（含复合赋值与属性/下标目标）。
/// 复合赋值、目标类型（Identifier/Index/Dot）的运算与存储逻辑全部由
/// task 18 的 compile_assignment 统一实现，本方法仅做语句包装 + POP。
fn compile_assign_stmt(&mut self, target: &Expr, op: &AssignOp, value: &Expr, line: usize) -> Result<(), String> {
    let assign_expr = Expr::Assign {
        target: Box::new(target.clone()),
        op: *op,
        value: Box::new(value.clone()),
    };
    self.compile_expression(&assign_expr, line)?;
    // 赋值表达式在栈顶留下结果值；作为语句需丢弃
    self.emit_byte(OpCode::Pop as u8, line);
    Ok(())
}
```

> 多目标赋值 `a, b = 1, 2`（target/value 均为 `TupleLiteral`，`03-syntax.md:140`）的 `UNPACK` 数量校验由 task 30 实现。本方法对 `TupleLiteral` 目标不做特殊处理，由 `compile_assignment` 内部的 `compile_store_target` 按现有逻辑处理（或返回错误推迟到 task 30）。

### 编译表达式语句

```rust
fn compile_expr_stmt(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(expr, line)?;
    self.emit_byte(OpCode::Pop as u8, line);
    Ok(())
}
```

### 编译 if/elif/else

签名直接匹配 `Stmt::If` 的 AST 字段（`src/ast/node.rs:102`）：首个 `condition`/`then_block` 单独处理，`elif_clauses` 循环处理。

```rust
fn compile_if(
    &mut self,
    condition: &Expr,
    then_block: &[Stmt],
    elif_clauses: &[(Expr, Vec<Stmt>)],
    else_block: &Option<Vec<Stmt>>,
    line: usize,
) -> Result<(), String> {
    let mut end_jumps = Vec::new();

    // 首个 if 分支
    self.compile_expression(condition, line)?;
    let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.emit_byte(OpCode::Pop as u8, line); // 弹出条件（true 路径）
    for stmt in then_block {
        self.compile_statement(stmt, line)?;
    }
    end_jumps.push(self.emit_jump(OpCode::Jump, line));
    self.patch_jump(else_jump)?;
    self.emit_byte(OpCode::Pop as u8, line); // 弹出条件（false 路径）

    // elif 分支
    for (cond, body) in elif_clauses {
        self.compile_expression(cond, line)?;
        let next_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line);
        for stmt in body {
            self.compile_statement(stmt, line)?;
        }
        end_jumps.push(self.emit_jump(OpCode::Jump, line));
        self.patch_jump(next_jump)?;
        self.emit_byte(OpCode::Pop as u8, line);
    }

    // else 分支
    if let Some(else_body) = else_block {
        for stmt in else_body {
            self.compile_statement(stmt, line)?;
        }
    }

    // 所有分支汇合点
    for jump in end_jumps {
        self.patch_jump(jump)?;
    }
    Ok(())
}
```

### 编译 while 循环

`LoopContext` 已移入 `Compiler` 结构体（栈式 `current_loop` 字段），不再用指向局部变量的引用。循环头记录在栈顶 `LoopContext` 中。

```rust
fn compile_while(&mut self, condition: &Expr, body: &[Stmt], line: usize) -> Result<(), String> {
    let loop_start = self.current_offset(); // 循环头：条件检查起点

    self.compile_expression(condition, line)?;
    let exit_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.emit_byte(OpCode::Pop as u8, line); // 弹出条件（true 路径）

    // 压入循环上下文（break/continue 在此栈顶取目标）
    self.current_loop.push(LoopContext {
        loop_start,
        break_jumps: Vec::new(),
    });

    for stmt in body {
        self.compile_statement(stmt, line)?;
    }

    // 回边：跳回循环头重新检查条件
    let back_edge = self.emit_jump(OpCode::JumpBack, line);
    self.patch_jump_back(back_edge, loop_start)?;

    // 正常出口：条件为 false，跳到此处
    self.patch_jump(exit_jump)?;
    self.emit_byte(OpCode::Pop as u8, line); // 弹出条件（false 路径）

    // 取出本循环的 break 跳转，patch 到出口（条件 POP 之后，break 时栈上无条件值）
    let loop_ctx = self.current_loop.pop()
        .ok_or("internal: loop context stack underflow")?;
    for jump in &loop_ctx.break_jumps {
        self.patch_jump(*jump)?;
    }
    Ok(())
}
```

> `patch_jump_back`（task 17 已提供，`src/compiler/mod.rs:192`）负责后向跳转偏移的边界检查（`u16::try_from().map_err()`），不再在本 task 内手工计算偏移。`current_offset()` 由 task 18 提供（`18-compile-expressions.md:354`）。

### 编译 for..in 循环

支持单变量 `for x in ...` 与双变量 `for k, v in ...`（`03-syntax.md:209`、`05-control-flow.md:66`）。双变量时用 `UNPACK 2` 将迭代器产出的二元组拆分到两个 slot。

```rust
fn compile_for_in(
    &mut self,
    variable: &str,
    second_variable: Option<&str>,
    iterable: &Expr,
    body: &[Stmt],
    line: usize,
) -> Result<(), String> {
    // 求值可迭代对象 → 创建迭代器（迭代器常驻栈上直至循环结束）
    self.compile_expression(iterable, line)?;
    self.emit_byte(OpCode::Iterator as u8, line);

    let loop_start = self.current_offset(); // FOR_ITER 位置（continue 目标）
    let for_iter_exit = self.emit_jump(OpCode::ForIter, line); // FOR_ITER + 占位偏移

    // FOR_ITER 每次迭代在栈顶压入下一个值
    if let Some(var2) = second_variable {
        // 双变量：UNPACK 2 拆出两个值，分别存入两个局部
        self.emit_byte(OpCode::Unpack as u8, line);
        self.emit_byte(2, line);
        self.declare_local(variable, line)?;
        let slot1 = self.resolve_local(variable)
            .ok_or("internal: loop var not found after declare")?;
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot1 as u8, line);
        self.declare_local(var2, line)?;
        let slot2 = self.resolve_local(var2)
            .ok_or("internal: loop var not found after declare")?;
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot2 as u8, line);
    } else {
        // 单变量：直接存入局部
        self.declare_local(variable, line)?;
        let slot = self.resolve_local(variable)
            .ok_or("internal: loop var not found after declare")?;
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot as u8, line);
    }

    self.current_loop.push(LoopContext {
        loop_start,
        break_jumps: Vec::new(),
    });

    for stmt in body {
        self.compile_statement(stmt, line)?;
    }

    let loop_ctx = self.current_loop.pop()
        .ok_or("internal: loop context stack underflow")?;

    // 回边：跳回 FOR_ITER
    let back_edge = self.emit_jump(OpCode::JumpBack, line);
    self.patch_jump_back(back_edge, loop_start)?;

    // 出口（FOR_ITER 耗尽时跳到此处）：break 也跳到此处，统一弹出迭代器
    let exit_target = self.current_offset();
    self.patch_jump(for_iter_exit)?;
    for jump in &loop_ctx.break_jumps {
        self.patch_jump(*jump)?;
    }
    // 迭代器常驻栈上，循环结束（正常退出或 break）必须弹出以保持栈平衡
    self.emit_byte(OpCode::Pop as u8, line);
    Ok(())
}
```

> **VM 契约**：`FOR_ITER` 退出（迭代耗尽）时**不**自动弹出迭代器对象，由编译器在出口处显式 `POP`。task 23/24 的 VM 实现须遵守此约定。
>
> **双变量数量校验**：`for k, v in ...` 要求迭代器每次产出恰好两个元素，否则运行时抛 `ValueError`（`03-syntax.md:228`）。该校验在 VM 的 `UNPACK` 执行处完成（task 24），编译期不校验。

### 编译 break/continue

使用专门的 `BREAK`/`CONTINUE` opcode（`11-bytecode-vm.md:102-103`），便于 VM 区分循环退出（后续 defer/finally 展开、生成器清理需识别）。从 `current_loop` 栈顶取目标；循环外使用返回编译错误而非 panic。

```rust
fn compile_break(&mut self, line: usize) -> Result<(), String> {
    // 先发射跳转（&mut self 借用随即释放），再取栈顶上下文 push，
    // 避免对 self.current_loop 的可变借用与 emit_jump 的 &mut self 跨语句并存。
    let jump = self.emit_jump(OpCode::Break, line);
    let ctx = self.current_loop.last_mut()
        .ok_or_else(|| format!("line {}: 'break' outside loop", line))?;
    ctx.break_jumps.push(jump);
    Ok(())
}

fn compile_continue(&mut self, line: usize) -> Result<(), String> {
    // last() 取出 loop_start（Copy，借用立即结束），再发射跳转并 patch
    let loop_start = self.current_loop.last()
        .map(|ctx| ctx.loop_start)
        .ok_or_else(|| format!("line {}: 'continue' outside loop", line))?;
    let back = self.emit_jump(OpCode::Continue, line);
    self.patch_jump_back(back, loop_start)?;
    Ok(())
}
```

> `last_mut()`/`last()` 在单次表达式内完成借用，避免 `as_ref()` 与后续 `&mut self` 调用的冲突。break 的前向跳转偏移在循环编译末尾由 `compile_while`/`compile_for_in` 统一 patch；continue 的后向偏移此处立即 patch。

### 编译 return

签名匹配 `Stmt::Return { values: Vec<Expr> }`（`src/ast/node.rs:120`）。无值返回 `NIL`；单值直接求值；多值构造元组（`return a, b, c`，`03-syntax.md:239`）。多返回值的完整解包语义由 task 30 实现，本 task 对多值打包为元组即可。

```rust
fn compile_return(&mut self, values: &[Expr], line: usize) -> Result<(), String> {
    match values.len() {
        0 => self.emit_byte(OpCode::Nil as u8, line),
        1 => self.compile_expression(&values[0], line)?,
        _ => {
            for v in values {
                self.compile_expression(v, line)?;
            }
            let count = u8::try_from(values.len())
                .map_err(|_| format!("too many return values (max 255, got {})", values.len()))?;
            self.emit_byte(OpCode::BuildTuple as u8, line);
            self.emit_byte(count, line);
        }
    }
    self.emit_byte(OpCode::Return as u8, line);
    Ok(())
}
```

### 编译 nonlocal / global 声明

`nonlocal` 和 `global` 声明不产生字节码指令，仅在编译器的符号表中标记变量绑定语义。标记存储在 `Compiler` 的 `nonlocal_names`/`global_names`（`HashSet<String>`）字段中，供 `compile_identifier`（task 18）与 `compile_store_target`（task 18）查询。

```rust
fn compile_nonlocal(&mut self, names: &[String], line: usize) -> Result<(), String> {
    for name in names {
        if self.global_names.contains(name) {
            return Err(format!("line {}: '{}' declared both nonlocal and global", line, name));
        }
        self.nonlocal_names.insert(name.clone());
    }
    Ok(())
}

fn compile_global(&mut self, names: &[String], line: usize) -> Result<(), String> {
    for name in names {
        if self.nonlocal_names.contains(name) {
            return Err(format!("line {}: '{}' declared both nonlocal and global", line, name));
        }
        self.global_names.insert(name.clone());
    }
    Ok(())
}
```

**LOAD/STORE 指令选择规则**（task 18 的标识符/存储编译须查询这两个集合，本 task 在此明确契约）：

对变量名 `name` 的读取/存储，按以下优先级选择指令（`03-syntax.md:598`）：

1. 若 `nonlocal_names` 含 `name` → `LOAD_UPVALUE`/`STORE_UPVALUE`（绑定外层函数作用域）
2. 否则若 `global_names` 含 `name` → `LOAD_GLOBAL`/`STORE_GLOBAL`（绑定全局作用域）
3. 否则若 `resolve_local(name)` 命中 → `LOAD_LOCAL`/`STORE_LOCAL`
4. 否则若 `resolve_upvalue(name)` 命中 → `LOAD_UPVALUE`/`STORE_UPVALUE`（隐式闭包捕获）
5. 否则 → `LOAD_GLOBAL`/`STORE_GLOBAL`（顶层脚本的 `=` 创建/更新全局；函数内的 `=` 在当前作用域创建新局部——见 dispatcher）

> nonlocal/global 的完整闭包语义（上值分配、`CLOSE_UPVALUE` 时机）由 task 28 实现。本 task 仅维护标记集合并提供查询契约；在顶层脚本作用域（无 parent）下 nonlocal 声明应报错（无外层函数作用域），此校验由 task 28 补全。

### 编译 block 语句

```rust
fn compile_block(&mut self, stmts: &[Stmt], line: usize) -> Result<(), String> {
    for stmt in stmts {
        self.compile_statement(stmt, line)?;
    }
    Ok(())
}
```

### 语句编译分发器

task 17 在 `src/compiler/mod.rs` 提供的 `compile_statement` 是返回 `Err` 的 stub。**本 task 必须替换该 stub 为下列分发器**（在 `src/compiler/statement.rs` 实现，`mod.rs` 中将其改为委托或直接内联实现）。

> **行号来源**：当前 AST 的 `Stmt` 变体不携带行号字段（`src/ast/node.rs`）。MVP 阶段 dispatcher 暂以 `line = 0` 传入各编译方法；完整的行号跟踪（基于 token span）由 task 57 补全。本 task 的函数签名保留 `line` 参数以兼容后续。

```rust
/// 语句编译入口。根据 Stmt 变体路由到对应编译方法。
pub fn compile_statement(&mut self, stmt: &Stmt, line: usize) -> Result<(), String> {
    match stmt {
        Stmt::VarDecl { name, initializer } | Stmt::ShortVarDecl { name, initializer } => {
            self.compile_var_decl(name, initializer, false, line)
        }
        Stmt::ConstDecl { name, initializer } => {
            self.compile_var_decl(name, initializer, true, line)
        }
        Stmt::Assign { target, op, value } => {
            self.compile_assign_stmt(target, op, value, line)
        }
        Stmt::ExprStmt { expr } => self.compile_expr_stmt(expr, line),
        Stmt::Block { statements } => self.compile_block(statements, line),
        Stmt::If { condition, then_block, elif_clauses, else_block } => {
            self.compile_if(condition, then_block, elif_clauses, else_block, line)
        }
        Stmt::While { condition, body } => self.compile_while(condition, body, line),
        Stmt::ForIn { variable, second_variable, iterable, body } => {
            self.compile_for_in(variable, second_variable.as_deref(), iterable, body, line)
        }
        Stmt::Break => self.compile_break(line),
        Stmt::Continue => self.compile_continue(line),
        Stmt::Return { values } => self.compile_return(values, line),
        Stmt::Nonlocal { names } => self.compile_nonlocal(names, line),
        Stmt::Global { names } => self.compile_global(names, line),
        // 以下类型由后续 task 实现
        Stmt::FnDecl { .. } => Err("fn declaration compilation not yet implemented (task 27/29)".into()),
        Stmt::ClassDecl { .. } => Err("class compilation not yet implemented (task 40)".into()),
        Stmt::Defer { .. } => Err("defer compilation not yet implemented (task 36)".into()),
        Stmt::Try { .. } => Err("try/except/finally compilation not yet implemented (task 37)".into()),
        Stmt::With { .. } => Err("with compilation not yet implemented (task 38)".into()),
        Stmt::Import { .. } | Stmt::FromImport { .. } => {
            Err("import compilation not yet implemented (task 45)".into())
        }
        Stmt::Throw { .. } => Err("throw compilation not yet implemented (task 37)".into()),
    }
}
```

> **注意**：task 17 的 `compile` 方法（`src/compiler/mod.rs:302`）调用 `self.compile_statement(stmt)?`——签名需同步改为 `compile_statement(&mut self, stmt: &Stmt, line: usize)`，调用处传入 `line = 0`（或从 `source_lines` 查找）。



## 验证标准

1. `var x = 10` / `x := 10` / `const X = 10` 编译为 `CONSTANT` + `STORE_LOCAL`（const 额外做常量表达式校验）
2. `if/elif/else` 编译为正确的条件跳转链，所有 `JUMP`/`JUMP_IF_FALSE` 跳转被正确 patch
3. `while` 编译为循环条件检查 → 循环体 → `JUMP_BACK`（回边）
4. `for..in`（单变量与双变量）编译为 `ITERATOR` + `FOR_ITER` 循环；双变量额外产生 `UNPACK 2`
5. `break` 编译为 `BREAK`（前向，跳到循环出口）；`continue` 编译为 `CONTINUE`（后向，跳到循环头）
6. `return` 编译为求值 + `RETURN`；无值时压入 `NIL`
7. 所有跳转偏移在编译结束时被正确 patch（无残留 `0xFFFF` 占位）
8. 循环结束后迭代器对象被 `POP`（for..in 栈平衡）
9. 循环外 `break`/`continue` 返回编译错误（非 panic）
10. 未实现的语句类型（fn/class/defer/try/with/import/throw）返回标注对应 task 的错误

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
    fn test_compile_for_in_single_var() {
        let source = r#"
            for i in [1, 2, 3] {
                print(i)
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        assert!(chunk.code.contains(&(OpCode::Iterator as u8)));
        assert!(chunk.code.contains(&(OpCode::ForIter as u8)));
    }

    #[test]
    fn test_compile_for_in_two_vars() {
        let source = r#"
            for k, v in d.items() {
                print(k)
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        assert!(chunk.code.contains(&(OpCode::Unpack as u8)));
    }

    #[test]
    fn test_break_continue_use_dedicated_opcodes() {
        let source = r#"
            while true {
                break
            }
            for i in [1] {
                continue
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        assert!(chunk.code.contains(&(OpCode::Break as u8)));
        assert!(chunk.code.contains(&(OpCode::Continue as u8)));
    }

    #[test]
    fn test_break_outside_loop_is_error() {
        let ast = parse("break").unwrap();
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&ast).is_err());
    }

    #[test]
    fn test_all_jumps_patched() {
        // 覆盖 if / while / for..in 的全部 2 字节跳转指令：无残留 0xFFFF
        let source = r#"
            x = 1
            if x > 0 {
                y = 1
            }
            for i in [1, 2] {
                if i == 1 {
                    continue
                }
                break
            }
        "#;
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        let two_byte_jumps = [
            OpCode::Jump, OpCode::JumpIfFalse, OpCode::JumpIfTrue,
            OpCode::JumpBack, OpCode::Break, OpCode::Continue, OpCode::ForIter,
        ];
        for (i, &byte) in chunk.code.iter().enumerate() {
            if two_byte_jumps.iter().any(|op| *op as u8 == byte) {
                let offset = u16::from_be_bytes([chunk.code[i + 1], chunk.code[i + 2]]);
                assert_ne!(offset, 0xffff, "Unpatched jump {:?} at offset {}", byte, i);
            }
        }
    }
}
```
