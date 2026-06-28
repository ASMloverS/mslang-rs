//! mslang 语句编译（task 19）。
//!
//! 将 AST 语句节点翻译为字节码指令序列。语句编译的核心是控制流的
//! 跳转指令与循环的 patch 机制。
//!
//! 参照 [19-compile-statements](../../../docs/mslang/tasks/19-compile-statements.md)。

use crate::ast::node::{AssignOp, BinaryOp, Expr, Stmt, UnaryOp};
use crate::vm::object::{alloc_function, alloc_string, Function};

use super::{CompilationUnit, Compiler, Local, OpCode};

// ---- 语句编译分发器 ----

impl Compiler {
    /// 语句编译入口。根据 Stmt 变体路由到对应编译方法。
    pub fn compile_statement(&mut self, stmt: &Stmt, line: usize) -> Result<(), String> {
        match stmt {
            Stmt::VarDecl { name, initializer } | Stmt::ShortVarDecl { name, initializer } => {
                self.compile_var_decl(name, initializer, false, line)
            }
            Stmt::ConstDecl { name, initializer } => {
                self.compile_var_decl(name, initializer, true, line)
            }
            Stmt::Assign { target, op, value } => self.compile_assign_stmt(target, op, value, line),
            Stmt::ExprStmt { expr } => self.compile_expr_stmt(expr, line),
            Stmt::Block { statements } => self.compile_block(statements, line),
            Stmt::If {
                condition,
                then_block,
                elif_clauses,
                else_block,
            } => self.compile_if(condition, then_block, elif_clauses, else_block, line),
            Stmt::While { condition, body } => self.compile_while(condition, body, line),
            Stmt::ForIn {
                variable,
                second_variable,
                iterable,
                body,
            } => self.compile_for_in(variable, second_variable.as_deref(), iterable, body, line),
            Stmt::Break => self.compile_break(line),
            Stmt::Continue => self.compile_continue(line),
            Stmt::Return { values } => self.compile_return(values, line),
            Stmt::Nonlocal { names } => self.compile_nonlocal(names, line),
            Stmt::Global { names } => self.compile_global(names, line),
            Stmt::FnDecl {
                name, params, body, ..
            } => self.compile_fn_decl(name, params, body, line),
            Stmt::ClassDecl { .. } => Err("class compilation not yet implemented (task 40)".into()),
            Stmt::Defer { .. } => Err("defer compilation not yet implemented (task 36)".into()),
            Stmt::Try { .. } => {
                Err("try/except/finally compilation not yet implemented (task 37)".into())
            }
            Stmt::With { .. } => Err("with compilation not yet implemented (task 38)".into()),
            Stmt::Import { .. } | Stmt::FromImport { .. } => {
                Err("import compilation not yet implemented (task 45)".into())
            }
            Stmt::Throw { .. } => Err("throw compilation not yet implemented (task 37)".into()),
        }
    }
}

// ---- 声明、赋值、表达式语句、block、return、nonlocal/global ----

impl Compiler {
    /// 编译 var/短声明/const 声明。三者均：求值右值 → 声明局部 → 存入 slot。
    /// `is_const` 为 true 时先做常量表达式校验。
    ///
    /// 解析器对 `name = value`（标识符目标 + 简单赋值）也产出 VarDecl，
    /// 与 `var`/`:=` 同走此方法。mslang 为函数级作用域，重复"声明"同名变量
    /// 视为更新现有 slot（仅首次创建新局部），匹配赋值语义。
    fn compile_var_decl(
        &mut self,
        name: &str,
        init: &Expr,
        is_const: bool,
        line: usize,
    ) -> Result<(), String> {
        if is_const {
            self.validate_const_expr(init, line)?;
        }
        self.compile_expression(init, line)?;
        // nonlocal 写语义（04-functions.md）：声明为 nonlocal 的名字强制走上值路径，
        // 不在当前作用域创建新局部（否则会遮蔽外层变量而非写入它）。
        if self.nonlocal_names.contains(name) {
            return match self.resolve_upvalue(name) {
                Some(idx) => {
                    self.emit_byte(OpCode::StoreUpvalue as u8, line);
                    self.emit_byte(idx as u8, line);
                    Ok(())
                }
                None => Err(format!("no binding for nonlocal '{}'", name)),
            };
        }
        let slot = match self.resolve_local(name) {
            Some(slot) => slot,
            None => {
                self.declare_local(name, line)?;
                self.resolve_local(name)
                    .ok_or_else(|| format!("internal: local '{}' not found after declare", name))?
            }
        };
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot as u8, line);
        Ok(())
    }

    /// const 表达式语法形式校验：仅允许字面量、一元取反（`-`/`~`）、
    /// 二元算术与位运算、括号分组。完整常量折叠求值由后续 task 扩展。
    fn validate_const_expr(&self, expr: &Expr, line: usize) -> Result<(), String> {
        match expr {
            Expr::Literal(_) => Ok(()),
            Expr::Grouping { expr } => self.validate_const_expr(expr, line),
            Expr::Unary { op, operand } => match op {
                UnaryOp::Negate | UnaryOp::BitNot => self.validate_const_expr(operand, line),
                _ => Err(format!(
                    "line {}: const initializer contains unsupported unary op",
                    line
                )),
            },
            Expr::Binary { left, op, right } => {
                if matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::FloorDiv
                        | BinaryOp::Modulo
                        | BinaryOp::Power
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::LeftShift
                        | BinaryOp::RightShift
                ) {
                    self.validate_const_expr(left, line)?;
                    self.validate_const_expr(right, line)
                } else {
                    Err(format!(
                        "line {}: const initializer must be a constant expression",
                        line
                    ))
                }
            }
            _ => Err(format!(
                "line {}: const initializer must be a constant expression",
                line
            )),
        }
    }

    /// 编译赋值语句（含复合赋值与属性/下标目标）。
    /// 复合赋值、目标类型的运算与存储逻辑全部由 task 18 的 compile_assignment
    /// 统一实现，本方法仅做语句包装 + POP 丢弃结果值。
    fn compile_assign_stmt(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        value: &Expr,
        line: usize,
    ) -> Result<(), String> {
        let assign_expr = Expr::Assign {
            target: Box::new(target.clone()),
            op: *op,
            value: Box::new(value.clone()),
        };
        self.compile_expression(&assign_expr, line)?;
        self.emit_byte(OpCode::Pop as u8, line);
        Ok(())
    }

    /// 编译表达式语句：求值表达式 → 弹出结果。
    fn compile_expr_stmt(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
        self.compile_expression(expr, line)?;
        self.emit_byte(OpCode::Pop as u8, line);
        Ok(())
    }

    /// 编译 block 语句：顺序编译内部语句。
    ///
    /// `pub(super)`：除本模块外，task 29 的 `compile_fn_literal`（expression.rs）也需
    /// 编译匿名函数体（语句向量），与具名函数体编译路径一致。
    pub(super) fn compile_block(&mut self, stmts: &[Stmt], line: usize) -> Result<(), String> {
        for stmt in stmts {
            self.compile_statement(stmt, line)?;
        }
        Ok(())
    }

    /// 编译函数声明（task 27/28）。
    /// 创建独立编译单元（parent 链接父单元以启用上值解析），预留 slot 0 给被调用者
    /// （closure 自身），参数从 slot 1 起。编译函数体后追加隐式 `NIL + RETURN`。
    /// task 28 改造：存 **Function**（非 Closure）入常量池，发 `CLOSURE` 指令 +
    /// 逐上值操作数（运行期包装），并写真值 `upvalue_count`。
    fn compile_fn_decl(
        &mut self,
        name: &str,
        params: &[crate::ast::node::Param],
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        let mut func_unit = CompilationUnit {
            chunk: super::Chunk::new(),
            // 订正 A3/V1：预留 slot 0 给被调用者（closure 自身），与 CALL 的
            // stack_base = callee_idx 自洽（slot 0 = stack[stack_base] = callee）。
            // 参数从 slot 1 起注册（slot 1..arity）。
            locals: vec![Local {
                name: "<self>".to_string(),
                depth: 0,
                is_captured: false,
            }],
            upvalues: Vec::new(),
            scope_depth: 0,
            parent: std::ptr::null(),
        };
        for param in params {
            func_unit.locals.push(Local {
                name: param.name.clone(),
                depth: 0,
                is_captured: false,
            });
        }

        // 换出父单元，编译函数体。期间 func_unit.parent 指向 saved_unit（裸指针，
        // 规避 self-referential 借用冲突，见 CompilationUnit.parent 字段注释）。
        let saved_unit = std::mem::replace(&mut self.unit, func_unit);
        self.unit.parent = std::ptr::addr_of!(saved_unit);
        self.compile_block(body, line)?;
        self.emit_byte(OpCode::Nil as u8, line);
        self.emit_byte(OpCode::Return as u8, line);
        let func_unit = std::mem::replace(&mut self.unit, saved_unit);

        // 上值捕获回填：函数体中 is_local=true 的上值对应父单元的局部变量，
        // 标记其 is_captured（驱动 end_scope 发射 CLOSE_UPVALUE）。此时 self.unit
        // 已恢复为父单元，func_unit 为刚编译的子单元。先收集再写回以避开借用。
        let captured_locals: Vec<usize> = func_unit
            .upvalues
            .iter()
            .filter(|uv| uv.is_local)
            .map(|uv| uv.index)
            .collect();
        for idx in captured_locals {
            if idx < self.unit.locals.len() {
                self.unit.locals[idx].is_captured = true;
            }
        }

        // 存 Function（非 Closure）入常量池 —— CLOSURE 指令运行期包装。
        let function = Function {
            name: name.to_string(),
            arity: params.len(),
            code: func_unit.chunk.code,
            constants: func_unit.chunk.constants,
            upvalue_count: func_unit.upvalues.len(),
            source_file: self.source_file.clone(),
        };
        let func_idx = self.add_constant(alloc_function(function));
        let func_idx = u16::try_from(func_idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;

        // 发 CLOSURE(func_idx) + 逐上值操作数（is_local:1 + index:1 每上值）。
        self.emit_byte(OpCode::Closure as u8, line);
        self.emit_bytes(&func_idx.to_be_bytes(), line);
        for uv in &func_unit.upvalues {
            self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
            let idx = u8::try_from(uv.index).map_err(|_| {
                format!("upvalue index {} exceeds 255 (function too large)", uv.index)
            })?;
            self.emit_byte(idx, line);
        }

        // 绑定函数名到全局（与 task 27 一致）
        let name_idx = self.add_constant(alloc_string(name));
        let name_idx = u16::try_from(name_idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
        self.emit_byte(OpCode::StoreGlobal as u8, line);
        self.emit_bytes(&name_idx.to_be_bytes(), line);
        Ok(())
    }

    /// 编译 return：无值压入 NIL；单值直接求值；多值打包为元组。
    fn compile_return(&mut self, values: &[Expr], line: usize) -> Result<(), String> {
        match values.len() {
            0 => self.emit_byte(OpCode::Nil as u8, line),
            1 => self.compile_expression(&values[0], line)?,
            _ => {
                for v in values {
                    self.compile_expression(v, line)?;
                }
                let count = u8::try_from(values.len()).map_err(|_| {
                    format!("too many return values (max 255, got {})", values.len())
                })?;
                self.emit_byte(OpCode::BuildTuple as u8, line);
                self.emit_byte(count, line);
            }
        }
        self.emit_byte(OpCode::Return as u8, line);
        Ok(())
    }

    /// 编译 nonlocal 声明：仅在符号表中标记，不产生字节码。
    fn compile_nonlocal(&mut self, names: &[String], line: usize) -> Result<(), String> {
        for name in names {
            if self.global_names.contains(name) {
                return Err(format!(
                    "line {}: '{}' declared both nonlocal and global",
                    line, name
                ));
            }
            self.nonlocal_names.insert(name.clone());
        }
        Ok(())
    }

    /// 编译 global 声明：仅在符号表中标记，不产生字节码。
    fn compile_global(&mut self, names: &[String], line: usize) -> Result<(), String> {
        for name in names {
            if self.nonlocal_names.contains(name) {
                return Err(format!(
                    "line {}: '{}' declared both nonlocal and global",
                    line, name
                ));
            }
            self.global_names.insert(name.clone());
        }
        Ok(())
    }
}

// ---- 控制流：if / while / for..in / break / continue ----

impl Compiler {
    /// 编译 if/elif/else。
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
        self.emit_byte(OpCode::Pop as u8, line);
        for stmt in then_block {
            self.compile_statement(stmt, line)?;
        }
        end_jumps.push(self.emit_jump(OpCode::Jump, line));
        self.patch_jump(else_jump)?;
        self.emit_byte(OpCode::Pop as u8, line);

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

    /// 编译 while 循环。
    fn compile_while(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        let loop_start = self.current_offset();

        self.compile_expression(condition, line)?;
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line);

        self.current_loop.push(super::LoopContext {
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
        self.emit_byte(OpCode::Pop as u8, line);

        // 取出本循环的 break 跳转，patch 到出口（条件 POP 之后）
        let loop_ctx = self
            .current_loop
            .pop()
            .ok_or("internal: loop context stack underflow")?;
        for jump in &loop_ctx.break_jumps {
            self.patch_jump(*jump)?;
        }
        Ok(())
    }

    /// 编译 for..in 循环（单变量与双变量）。
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

        let loop_start = self.current_offset();
        let for_iter_exit = self.emit_jump(OpCode::ForIter, line);

        if let Some(var2) = second_variable {
            // 双变量：UNPACK 2 拆出两个值，分别存入两个局部
            self.emit_byte(OpCode::Unpack as u8, line);
            self.emit_byte(2, line);
            self.declare_local(variable, line)?;
            let slot1 = self
                .resolve_local(variable)
                .ok_or("internal: loop var not found after declare")?;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot1 as u8, line);
            self.declare_local(var2, line)?;
            let slot2 = self
                .resolve_local(var2)
                .ok_or("internal: loop var not found after declare")?;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot2 as u8, line);
        } else {
            // 单变量：直接存入局部
            self.declare_local(variable, line)?;
            let slot = self
                .resolve_local(variable)
                .ok_or("internal: loop var not found after declare")?;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot as u8, line);
        }

        self.current_loop.push(super::LoopContext {
            loop_start,
            break_jumps: Vec::new(),
        });

        for stmt in body {
            self.compile_statement(stmt, line)?;
        }

        let loop_ctx = self
            .current_loop
            .pop()
            .ok_or("internal: loop context stack underflow")?;

        // 回边：跳回 FOR_ITER
        let back_edge = self.emit_jump(OpCode::JumpBack, line);
        self.patch_jump_back(back_edge, loop_start)?;

        // 出口：FOR_ITER 耗尽与 break 跳到此处，统一弹出迭代器
        self.patch_jump(for_iter_exit)?;
        for jump in &loop_ctx.break_jumps {
            self.patch_jump(*jump)?;
        }
        self.emit_byte(OpCode::Pop as u8, line);
        Ok(())
    }

    /// 编译 break：前向跳转到循环出口（由循环编译末尾统一 patch）。
    fn compile_break(&mut self, line: usize) -> Result<(), String> {
        let jump = self.emit_jump(OpCode::Break, line);
        let ctx = self
            .current_loop
            .last_mut()
            .ok_or_else(|| format!("line {}: 'break' outside loop", line))?;
        ctx.break_jumps.push(jump);
        Ok(())
    }

    /// 编译 continue：后向跳转到循环头（立即 patch）。
    fn compile_continue(&mut self, line: usize) -> Result<(), String> {
        let loop_start = self
            .current_loop
            .last()
            .map(|ctx| ctx.loop_start)
            .ok_or_else(|| format!("line {}: 'continue' outside loop", line))?;
        let back = self.emit_jump(OpCode::Continue, line);
        self.patch_jump_back(back, loop_start)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::node::Program;
    use crate::compiler::{Chunk, Compiler, OpCode};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    fn compile(source: &str) -> Chunk {
        let program = parse(source);
        let mut compiler = Compiler::new();
        compiler.compile(&program).unwrap()
    }

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
        let chunk = compile(source);
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
        let chunk = compile(source);
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
        let chunk = compile(source);
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
        let chunk = compile(source);
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
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::Break as u8)));
        assert!(chunk.code.contains(&(OpCode::Continue as u8)));
    }

    #[test]
    fn test_break_outside_loop_is_error() {
        let program = parse("break");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_continue_outside_loop_is_error() {
        let program = parse("continue");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_all_jumps_patched() {
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
        let chunk = compile(source);
        let two_byte_jumps = [
            OpCode::Jump,
            OpCode::JumpIfFalse,
            OpCode::JumpIfTrue,
            OpCode::JumpBack,
            OpCode::Break,
            OpCode::Continue,
            OpCode::ForIter,
        ];
        for (i, &byte) in chunk.code.iter().enumerate() {
            if two_byte_jumps.iter().any(|op| *op as u8 == byte) {
                let offset = u16::from_be_bytes([chunk.code[i + 1], chunk.code[i + 2]]);
                assert_ne!(offset, 0xffff, "Unpatched jump {:?} at offset {}", byte, i);
            }
        }
    }

    #[test]
    fn test_var_decl_uses_store_local() {
        // var x = 10 → CONSTANT ... + STORE_LOCAL
        let chunk = compile("var x = 10");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::StoreLocal as u8)));
    }

    #[test]
    fn test_const_decl_valid_literal() {
        // const X = 10 通过常量表达式校验
        let chunk = compile("const X = 10");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::StoreLocal as u8)));
    }

    #[test]
    fn test_const_decl_valid_arithmetic() {
        // const 支持 + - * 等二元算术与括号
        let chunk = compile("const X = (1 + 2) * 3");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
    }

    #[test]
    fn test_const_decl_rejects_call() {
        // const 右侧出现函数调用 → 编译错误
        let program = parse("const X = foo()");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_const_decl_rejects_identifier() {
        // const 右侧出现变量引用 → 编译错误
        let program = parse("const X = y");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_return_with_no_value_emits_nil() {
        let chunk = compile("return");
        assert!(chunk.code.contains(&(OpCode::Nil as u8)));
        assert!(chunk.code.contains(&(OpCode::Return as u8)));
    }

    #[test]
    fn test_return_with_value() {
        let chunk = compile("return 42");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::Return as u8)));
    }

    #[test]
    fn test_deferred_statement_types_return_error() {
        // fn 声明已由 task 27 实现，不再报错；其余语句类型仍为 stub。
        for source in ["defer foo()", "throw e", "import os"] {
            let program = parse(source);
            let mut compiler = Compiler::new();
            assert!(
                compiler.compile(&program).is_err(),
                "expected error for deferred statement: {:?}",
                source
            );
        }
    }
}
