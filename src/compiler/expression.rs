//! mslang 表达式编译（task 18）。
//!
//! 将 AST 表达式节点翻译为栈式字节码指令序列。表达式编译的核心原则：
//! 每条表达式编译后，在栈顶留下一个结果值。
//!
//! 参照 [18-compile-expressions](../../../docs/mslang/tasks/18-compile-expressions.md)。

use crate::ast::node::{AssignOp, BinaryOp, Expr, ForClause, Literal, Stmt, UnaryOp};
use crate::vm::object::{alloc_function, alloc_string, Function, Object};

use super::{Chunk, CompilationUnit, Compiler, Local, OpCode};

// ---- 表达式编译入口与访问器 ----

impl Compiler {
    /// 获取当前字节码偏移量。
    pub fn current_offset(&self) -> usize {
        self.unit.chunk.code.len()
    }

    /// 获取已编译的字节码块引用（测试用）。
    pub fn chunk(&self) -> &Chunk {
        &self.unit.chunk
    }

    /// 编译表达式。编译后栈顶留下一个结果值。
    ///
    /// 根据 `Expr` 变体路由到对应的编译方法。尚未实现的类型返回错误。
    pub fn compile_expression(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
        match expr {
            Expr::Literal(lit) => self.compile_literal(lit, line),
            Expr::Identifier(name) => self.compile_identifier(name, line),
            Expr::Binary { left, op, right } => match op {
                BinaryOp::And => self.compile_logical_and(left, right, line),
                BinaryOp::Or => self.compile_logical_or(left, right, line),
                _ => self.compile_binary(left, op, right, line),
            },
            Expr::Unary { op, operand } => self.compile_unary(op, operand, line),
            Expr::Assign { target, op, value } => self.compile_assignment(target, op, value, line),
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => self.compile_ternary(condition, then_expr, else_expr, line),
            Expr::Call { callee, args } => self.compile_call(callee, args, line),
            Expr::Index { object, index } => self.compile_index(object, index, line),
            Expr::Dot { object, name } => self.compile_dot(object, name, line),
            Expr::Slice {
                object,
                start,
                stop,
                step,
            } => self.compile_slice(
                object,
                start.as_deref(),
                stop.as_deref(),
                step.as_deref(),
                line,
            ),
            Expr::ListLiteral { elements } => self.compile_list_literal(elements, line),
            Expr::DictLiteral { pairs } => self.compile_dict_literal(pairs, line),
            Expr::SetLiteral { elements } => self.compile_set_literal(elements, line),
            Expr::TupleLiteral { elements } => self.compile_tuple_literal(elements, line),
            Expr::Grouping { expr } => self.compile_expression(expr, line),
            Expr::FnLiteral { params, body } => self.compile_fn_literal(params, body, line),
            // 以下类型由后续 task 实现
            Expr::ListComprehension {
                expr,
                for_clauses,
                condition,
            } => self.compile_list_comprehension(expr, for_clauses, condition, line),
            Expr::DictComprehension {
                key_expr,
                value_expr,
                for_clauses,
                condition,
            } => {
                self.compile_dict_comprehension(key_expr, value_expr, for_clauses, condition, line)
            }
            Expr::SetComprehension {
                expr,
                for_clauses,
                condition,
            } => self.compile_set_comprehension(expr, for_clauses, condition, line),
            Expr::GeneratorExpression {
                expr,
                for_clauses,
                condition,
            } => self.compile_generator_expression(expr, for_clauses, condition, line),
            Expr::SuperAccess { name } => {
                let class_name = self
                    .current_class
                    .as_ref()
                    .ok_or_else(|| "'super' used outside of class method".to_string())?;
                let class_idx = self.add_constant(alloc_string(class_name));
                let class_idx = u16::try_from(class_idx)
                    .map_err(|_| "constant pool overflow".to_string())?;
                let name_idx = self.add_constant(alloc_string(name));
                let name_idx = u16::try_from(name_idx)
                    .map_err(|_| "constant pool overflow".to_string())?;
                self.emit_byte(OpCode::GetSuper as u8, line);
                self.emit_bytes(&class_idx.to_be_bytes(), line);
                self.emit_bytes(&name_idx.to_be_bytes(), line);
                Ok(())
            }
            Expr::Yield { value } => {
                self.unit.is_generator = true;
                match value {
                    None => self.emit_byte(OpCode::Nil as u8, line),
                    Some(e) => self.compile_expression(e, line)?,
                }
                self.emit_byte(OpCode::Yield as u8, line);
                Ok(())
            }
            Expr::YieldFrom { iterable } => {
                self.unit.is_generator = true;
                self.compile_expression(iterable, line)?;
                self.emit_byte(OpCode::YieldFrom as u8, line);
                self.emit_byte(OpCode::YieldFromResume as u8, line);
                Ok(())
            }
            Expr::Await { .. } => {
                Err("await compilation not yet implemented (task 53)".to_string())
            }
            Expr::Go { .. } => Err("go compilation not yet implemented (task 55)".to_string()),
        }
    }
}

// ---- 字面量与标识符 ----

impl Compiler {
    fn compile_literal(&mut self, lit: &Literal, line: usize) -> Result<(), String> {
        match lit {
            Literal::Int(n) => self.emit_constant(Object::Int(*n), line)?,
            Literal::Float(f) => self.emit_constant(Object::Float(*f), line)?,
            Literal::String(s) => self.emit_constant(alloc_string(s), line)?,
            Literal::Bool(true) => self.emit_byte(OpCode::True as u8, line),
            Literal::Bool(false) => self.emit_byte(OpCode::False as u8, line),
            Literal::Nil => self.emit_byte(OpCode::Nil as u8, line),
        }
        Ok(())
    }

    fn compile_identifier(&mut self, name: &str, line: usize) -> Result<(), String> {
        if let Some(slot) = self.resolve_local(name) {
            self.emit_byte(OpCode::LoadLocal as u8, line);
            self.emit_byte(slot as u8, line);
        } else if let Some(idx) = self.resolve_upvalue(name) {
            self.emit_byte(OpCode::LoadUpvalue as u8, line);
            self.emit_byte(idx as u8, line);
        } else {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::LoadGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        }
        Ok(())
    }
}

// ---- 二元与一元运算 ----

impl Compiler {
    fn compile_binary(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        line: usize,
    ) -> Result<(), String> {
        self.compile_expression(left, line)?;
        self.compile_expression(right, line)?;
        let opcode = match op {
            BinaryOp::Add => OpCode::Add,
            BinaryOp::Subtract => OpCode::Subtract,
            BinaryOp::Multiply => OpCode::Multiply,
            BinaryOp::Divide => OpCode::Divide,
            BinaryOp::FloorDiv => OpCode::FloorDiv,
            BinaryOp::Modulo => OpCode::Modulo,
            BinaryOp::Power => OpCode::Power,
            BinaryOp::BitAnd => OpCode::BitAnd,
            BinaryOp::BitOr => OpCode::BitOr,
            BinaryOp::BitXor => OpCode::BitXor,
            BinaryOp::LeftShift => OpCode::LeftShift,
            BinaryOp::RightShift => OpCode::RightShift,
            BinaryOp::Equal => OpCode::Equal,
            BinaryOp::NotEqual => OpCode::NotEqual,
            BinaryOp::Less => OpCode::Less,
            BinaryOp::Greater => OpCode::Greater,
            BinaryOp::LessEqual => OpCode::LessEqual,
            BinaryOp::GreaterEqual => OpCode::GreaterEqual,
            BinaryOp::In => OpCode::In,
            BinaryOp::Is => OpCode::Is,
            // And/Or 由 compile_expression 拦截走短路路径，不应到达此处。
            _ => return Err(format!("Unsupported binary op: {:?}", op)),
        };
        self.emit_byte(opcode as u8, line);
        Ok(())
    }

    fn compile_unary(&mut self, op: &UnaryOp, operand: &Expr, line: usize) -> Result<(), String> {
        self.compile_expression(operand, line)?;
        let opcode = match op {
            UnaryOp::Negate => OpCode::Negate,
            UnaryOp::Not => OpCode::Not,
            UnaryOp::BitNot => OpCode::BitNot,
            UnaryOp::ChannelReceive => OpCode::Receive,
        };
        self.emit_byte(opcode as u8, line);
        Ok(())
    }
}

// ---- 链式比较（预留） ----
//
// 注意：当前解析器（task 12）在解析阶段已将链式比较反糖为 `and` 链
// （`1 < x < 10` → `(1 < x) and (x < 10)`），因此以下方法暂无调用方。
// 保留作为编译策略参考，待后续如需直接编译链式比较节点时使用。

impl Compiler {
    /// 将 BinaryOp 比较运算符映射到 OpCode。
    #[allow(dead_code)]
    fn comparison_opcode(&self, op: &BinaryOp) -> OpCode {
        match op {
            BinaryOp::Equal => OpCode::Equal,
            BinaryOp::NotEqual => OpCode::NotEqual,
            BinaryOp::Less => OpCode::Less,
            BinaryOp::Greater => OpCode::Greater,
            BinaryOp::LessEqual => OpCode::LessEqual,
            BinaryOp::GreaterEqual => OpCode::GreaterEqual,
            _ => unreachable!("non-comparison op in chain"),
        }
    }

    /// 编译链式比较：`a op1 b op2 c` 等价于 `(a op1 b) and (b op2 c)`。
    #[allow(dead_code)]
    fn compile_comparison(
        &mut self,
        first: &Expr,
        comparisons: &[(BinaryOp, Expr)],
        line: usize,
    ) -> Result<(), String> {
        self.compile_expression(first, line)?;
        if comparisons.len() == 1 {
            let (op, right) = &comparisons[0];
            self.compile_expression(right, line)?;
            self.emit_byte(self.comparison_opcode(op) as u8, line);
            return Ok(());
        }
        // 链式比较：对每段，加载右操作数 → 比较 → 若 false 短路跳到结束。
        let mut end_jumps: Vec<usize> = Vec::new();
        for (i, (op, right)) in comparisons.iter().enumerate() {
            if i > 0 {
                // 重新加载上一个操作数作为本次左操作数。
                self.compile_expression(&comparisons[i - 1].1, line)?;
            }
            self.compile_expression(right, line)?;
            self.emit_byte(self.comparison_opcode(op) as u8, line);
            let jump = self.emit_jump(OpCode::JumpIfFalse, line);
            end_jumps.push(jump);
            self.emit_byte(OpCode::Pop as u8, line); // 弹出 bool true
        }
        // 所有比较都为 true：压入 true。
        self.emit_byte(OpCode::True as u8, line);
        for jump in &end_jumps {
            self.patch_jump(*jump)?;
        }
        Ok(())
    }
}

// ---- 赋值表达式 ----

impl Compiler {
    fn compile_assignment(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        value: &Expr,
        line: usize,
    ) -> Result<(), String> {
        use AssignOp::*;
        let is_compound = !matches!(op, Assign);

        if is_compound {
            // 复合赋值：x += 5 → 加载 x、编译右值、运算、DUP、存储。
            self.compile_load_target(target, line)?;
            self.compile_expression(value, line)?;
            let arith_op = match op {
                PlusAssign => OpCode::Add,
                MinusAssign => OpCode::Subtract,
                StarAssign => OpCode::Multiply,
                SlashAssign => OpCode::Divide,
                DoubleSlashAssign => OpCode::FloorDiv,
                PercentAssign => OpCode::Modulo,
                DoubleStarAssign => OpCode::Power,
                BitAndAssign => OpCode::BitAnd,
                BitOrAssign => OpCode::BitOr,
                BitXorAssign => OpCode::BitXor,
                LeftShiftAssign => OpCode::LeftShift,
                RightShiftAssign => OpCode::RightShift,
                Assign => unreachable!("guarded by is_compound check"),
            };
            self.emit_byte(arith_op as u8, line);
        } else {
            // 简单赋值：仅编译右值。
            self.compile_expression(value, line)?;
        }
        // DUP：保留赋值结果值在栈顶（赋值表达式返回被赋的值）。
        self.emit_byte(OpCode::Dup as u8, line);
        self.compile_store_target(target, line)?;
        Ok(())
    }

    /// 加载赋值目标的当前值（用于复合赋值的读取）。
    fn compile_load_target(&mut self, target: &Expr, line: usize) -> Result<(), String> {
        match target {
            Expr::Identifier(name) => self.compile_identifier(name, line),
            Expr::Index { object, index } => self.compile_index(object, index, line),
            Expr::Dot { object, name } => self.compile_dot(object, name, line),
            Expr::TupleLiteral { .. } => {
                Err("compound assignment cannot target a tuple".to_string())
            }
            _ => Err("Invalid assignment target".to_string()),
        }
    }

    /// 将栈顶值存储到赋值目标。
    fn compile_store_target(&mut self, target: &Expr, line: usize) -> Result<(), String> {
        match target {
            Expr::Identifier(name) => {
                if self.nonlocal_names.contains(name) {
                    // nonlocal 写语义：强制走上值路径，不存在则编译错误
                    // （04-functions.md："no binding for nonlocal 'x'"）。
                    let idx = self
                        .resolve_upvalue(name)
                        .ok_or_else(|| format!("no binding for nonlocal '{}'", name))?;
                    self.emit_byte(OpCode::StoreUpvalue as u8, line);
                    self.emit_byte(idx as u8, line);
                } else if let Some(slot) = self.resolve_local(name) {
                    self.emit_byte(OpCode::StoreLocal as u8, line);
                    self.emit_byte(slot as u8, line);
                } else if let Some(idx) = self.resolve_upvalue(name) {
                    self.emit_byte(OpCode::StoreUpvalue as u8, line);
                    self.emit_byte(idx as u8, line);
                } else {
                    let name_idx = self.add_constant(alloc_string(name));
                    let name_idx = u16::try_from(name_idx)
                        .map_err(|_| "constant pool overflow".to_string())?;
                    self.emit_byte(OpCode::StoreGlobal as u8, line);
                    self.emit_bytes(&name_idx.to_be_bytes(), line);
                }
            }
            Expr::Index { object, index } => {
                self.compile_expression(object, line)?;
                self.compile_expression(index, line)?;
                self.emit_byte(OpCode::SetIndex as u8, line);
            }
            Expr::Dot { object, name } => {
                let name_idx = self.add_constant(alloc_string(name));
                let name_idx =
                    u16::try_from(name_idx).map_err(|_| "constant pool overflow".to_string())?;
                self.compile_expression(object, line)?;
                self.emit_byte(OpCode::SetAttr as u8, line);
                self.emit_bytes(&name_idx.to_be_bytes(), line);
            }
            Expr::TupleLiteral { elements: targets } => {
                let count = u8::try_from(targets.len()).map_err(|_| {
                    format!("too many unpack targets (max 255, got {})", targets.len())
                })?;
                self.emit_byte(OpCode::Unpack as u8, line);
                self.emit_byte(count, line);
                // UNPACK（mod.rs）逆序压入 elements，使 elements[0] 位于栈顶。
                // 因此按正序迭代 targets：targets[0] 在栈顶，先 store。
                for target in targets {
                    self.compile_store_target(target, line)?;
                }
            }
            _ => return Err("Invalid assignment target".to_string()),
        }
        Ok(())
    }
}

// ---- 三元、调用、下标、属性 ----

impl Compiler {
    fn compile_ternary(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        line: usize,
    ) -> Result<(), String> {
        self.compile_expression(condition, line)?;
        let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.compile_expression(then_expr, line)?;
        let end_jump = self.emit_jump(OpCode::Jump, line);
        self.patch_jump(else_jump)?;
        self.compile_expression(else_expr, line)?;
        self.patch_jump(end_jump)?;
        Ok(())
    }

    fn compile_call(&mut self, callee: &Expr, args: &[Expr], line: usize) -> Result<(), String> {
        self.compile_expression(callee, line)?;
        for arg in args {
            self.compile_expression(arg, line)?;
        }
        let argc = u8::try_from(args.len())
            .map_err(|_| format!("too many arguments (max 255, got {})", args.len()))?;
        self.emit_byte(OpCode::Call as u8, line);
        self.emit_byte(argc, line);
        Ok(())
    }

    fn compile_index(&mut self, object: &Expr, index: &Expr, line: usize) -> Result<(), String> {
        self.compile_expression(object, line)?;
        self.compile_expression(index, line)?;
        self.emit_byte(OpCode::GetIndex as u8, line);
        Ok(())
    }

    fn compile_dot(&mut self, object: &Expr, name: &str, line: usize) -> Result<(), String> {
        self.compile_expression(object, line)?;
        let name_idx = self.add_constant(alloc_string(name));
        let name_idx = u16::try_from(name_idx).map_err(|_| "constant pool overflow".to_string())?;
        self.emit_byte(OpCode::GetAttr as u8, line);
        self.emit_bytes(&name_idx.to_be_bytes(), line);
        Ok(())
    }
}

// ---- 逻辑短路求值 ----

impl Compiler {
    /// `and` 短路求值：左操作数为假时直接跳过右操作数。
    fn compile_logical_and(
        &mut self,
        left: &Expr,
        right: &Expr,
        line: usize,
    ) -> Result<(), String> {
        self.compile_expression(left, line)?;
        let end_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line); // 弹出左操作数的假值
        self.compile_expression(right, line)?;
        self.patch_jump(end_jump)?;
        Ok(())
    }

    /// `or` 短路求值：左操作数为真时直接跳过右操作数。
    fn compile_logical_or(&mut self, left: &Expr, right: &Expr, line: usize) -> Result<(), String> {
        self.compile_expression(left, line)?;
        let end_jump = self.emit_jump(OpCode::JumpIfTrue, line);
        self.emit_byte(OpCode::Pop as u8, line); // 弹出左操作数的真值
        self.compile_expression(right, line)?;
        self.patch_jump(end_jump)?;
        Ok(())
    }
}

// ---- 切片 ----

impl Compiler {
    /// 编译切片 `obj[start:stop:step]`。
    /// flags 位域：bit 0 = has_start, bit 1 = has_stop, bit 2 = has_step。
    fn compile_slice(
        &mut self,
        object: &Expr,
        start: Option<&Expr>,
        stop: Option<&Expr>,
        step: Option<&Expr>,
        line: usize,
    ) -> Result<(), String> {
        self.compile_expression(object, line)?;
        let mut flags: u8 = 0;
        if let Some(s) = start {
            flags |= 0b001;
            self.compile_expression(s, line)?;
        }
        if let Some(s) = stop {
            flags |= 0b010;
            self.compile_expression(s, line)?;
        }
        if let Some(s) = step {
            flags |= 0b100;
            self.compile_expression(s, line)?;
        }
        self.emit_byte(OpCode::GetSlice as u8, line);
        self.emit_byte(flags, line);
        Ok(())
    }
}

// ---- 集合字面量 ----

impl Compiler {
    fn compile_list_literal(&mut self, elements: &[Expr], line: usize) -> Result<(), String> {
        for elem in elements {
            self.compile_expression(elem, line)?;
        }
        let count = u8::try_from(elements.len())
            .map_err(|_| format!("too many list elements (max 255, got {})", elements.len()))?;
        self.emit_byte(OpCode::BuildList as u8, line);
        self.emit_byte(count, line);
        Ok(())
    }

    fn compile_dict_literal(&mut self, pairs: &[(Expr, Expr)], line: usize) -> Result<(), String> {
        for (key, val) in pairs {
            self.compile_expression(key, line)?;
            self.compile_expression(val, line)?;
        }
        let count = u8::try_from(pairs.len())
            .map_err(|_| format!("too many dict entries (max 255, got {})", pairs.len()))?;
        self.emit_byte(OpCode::BuildDict as u8, line);
        self.emit_byte(count, line);
        Ok(())
    }

    fn compile_set_literal(&mut self, elements: &[Expr], line: usize) -> Result<(), String> {
        for elem in elements {
            self.compile_expression(elem, line)?;
        }
        let count = u8::try_from(elements.len())
            .map_err(|_| format!("too many set elements (max 255, got {})", elements.len()))?;
        self.emit_byte(OpCode::BuildSet as u8, line);
        self.emit_byte(count, line);
        Ok(())
    }

    fn compile_tuple_literal(&mut self, elements: &[Expr], line: usize) -> Result<(), String> {
        for elem in elements {
            self.compile_expression(elem, line)?;
        }
        let count = u8::try_from(elements.len())
            .map_err(|_| format!("too many tuple elements (max 255, got {})", elements.len()))?;
        self.emit_byte(OpCode::BuildTuple as u8, line);
        self.emit_byte(count, line);
        Ok(())
    }

    /// 编译匿名函数字面量（task 29）。
    /// 镜像 compile_fn_decl（statement.rs），差异仅两点：(1) name="<anonymous>"；
    /// (2) 不发 STORE_GLOBAL —— 匿名函数是表达式，闭包值留栈作为表达式结果，由外层
    /// 赋值/传参/集合构造消费。上值机制（parent 链接、is_captured 回填、CLOSURE 发射）
    /// 与具名函数完全一致，确保匿名闭包正确捕获外层变量。
    fn compile_fn_literal(
        &mut self,
        params: &[crate::ast::node::Param],
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        let mut func_unit = CompilationUnit {
            chunk: super::Chunk::new(),
            // slot 0 预留给被调用者（closure 自身），与 CALL 的 stack_base=callee_idx 自洽。
            // 参数从 slot 1 起（与 compile_fn_decl 一致 — task 27 订正 A3/V1）。
            locals: vec![Local {
                name: "<self>".to_string(),
                depth: 0,
                is_captured: false,
            }],
            upvalues: Vec::new(),
            scope_depth: 0,
            is_generator: false,
            parent: std::ptr::null(),
        };
        // task 31：参数顺序校验 + 默认/可变分类（镜像 compile_fn_decl）。
        super::validate_param_order(params)?;
        let mut required_arity = 0usize;
        let mut default_values = Vec::new();
        let mut has_variadic = false;
        for param in params {
            if param.is_variadic {
                has_variadic = true;
            } else if param.default.is_some() {
                let val = super::eval_default(param.default.as_ref().unwrap())?;
                default_values.push(val);
            } else {
                required_arity += 1;
            }
            func_unit.locals.push(Local {
                name: param.name.clone(),
                depth: 0,
                is_captured: false,
            });
        }

        // 换出父单元，编译函数体。parent 指向 saved_unit（裸指针，规避 self-referential
        // 借用冲突 — task 28 方案），使 resolve_upvalue_recursive 可攀爬外层。
        let saved_unit = std::mem::replace(&mut self.unit, func_unit);
        self.unit.parent = std::ptr::addr_of!(saved_unit);
        self.compile_block(body, line)?;
        self.emit_byte(OpCode::Nil as u8, line); // 隐式 return nil
        self.emit_return(line);
        let func_unit = std::mem::replace(&mut self.unit, saved_unit);

        // 上值捕获回填（task 28）：is_local=true 的上值对应父单元局部变量，
        // 标记 is_captured 驱动 end_scope 发射 CLOSE_UPVALUE。先收集再写回以避开借用。
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

        // 存 Function 入常量池，发 CLOSURE(func_idx) + 逐上值操作数。
        let function = Function {
            name: "<anonymous>".to_string(),
            // 固定参数总数（普通 + 默认，不含可变）。
            arity: params.iter().filter(|p| !p.is_variadic).count(),
            code: func_unit.chunk.code,
            constants: func_unit.chunk.constants,
            upvalue_count: func_unit.upvalues.len(),
            source_file: self.source_file.clone(),
            default_values,
            has_variadic,
            required_arity,
            is_generator: func_unit.is_generator,
            locals_count: func_unit.locals.len(),
        };
        let func_idx = self.add_constant(alloc_function(function));
        let func_idx = u16::try_from(func_idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;

        self.emit_byte(OpCode::Closure as u8, line);
        self.emit_bytes(&func_idx.to_be_bytes(), line);
        for uv in &func_unit.upvalues {
            self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
            let idx = u8::try_from(uv.index).map_err(|_| {
                format!(
                    "upvalue index {} exceeds 255 (function too large)",
                    uv.index
                )
            })?;
            self.emit_byte(idx, line);
        }
        // 不发 STORE_GLOBAL —— 闭包值留栈，作为表达式结果供外层消费。
        Ok(())
    }
}

// ---- 推导式（task 33 list / task 34 dict·set）----

/// 推导式单个 for 子句的预留 slot 信息：iterable 引用 + 迭代器 slot + 各目标 slot。
/// 在 `compile_comprehension` 入口一次性收集，供递归编译嵌套循环时复用。
struct CompClauseSlots<'a> {
    iterable: &'a Expr,
    iter: usize,
    targets: Vec<usize>,
}

/// 推导式最内层载荷：list/set 追加单个表达式，dict 插入 key→value。
/// 三种推导式共用同一循环骨架（[`Compiler::compile_comprehension`]），仅此最内层步骤不同。
#[derive(Clone, Copy)]
enum CompPayload<'a> {
    List(&'a Expr),
    Set(&'a Expr),
    Dict { key: &'a Expr, value: &'a Expr },
}

impl Compiler {
    /// 编译推导式公共骨架（list/set/dict 共用，task 33/34）。
    ///
    /// 语法糖：创建空容器，从左到右展开嵌套 for 循环，最内层按可选条件执行
    /// 插入/追加（由 `payload` 决定）。三条不变量（task 33/34 §2）：
    /// (1) iter 与结果容器存局部 slot，复用 `emit_for_iter`；
    /// (2) 整个 codegen 包裹 `begin_scope`/`end_scope`，循环变量不泄漏；
    /// (3) `end_scope` 前发 `LOAD_LOCAL container_slot` 留结果于栈顶。
    ///
    /// **对 §2 伪代码的修正**：所有 for 子句的 iter/target slot 在进入任何循环之前一次性
    /// 预留（Nil 占位 + declare_local），而非伪代码的逐子句延迟声明。必要性：(a) 嵌套子句
    /// 的占位指令若位于外层循环体内，会随每次外层迭代重入而反复执行，导致栈泄漏并破坏
    /// `end_scope` 的 POP 清理；(b) 多变量子句的 UNPACK 展开要求目标 slot 已位于栈低部，
    /// 否则展开元素与目标 slot 位置重叠而互相覆盖。一次性预留使占位指令仅执行一次，循环体内
    /// 仅 StoreLocal 写入固定 slot（净零栈效应），杜绝泄漏。
    fn compile_comprehension(
        &mut self,
        build_op: OpCode,
        payload: CompPayload,
        for_clauses: &[ForClause],
        condition: &Option<Box<Expr>>,
        line: usize,
    ) -> Result<(), String> {
        self.begin_scope();

        // 结果容器：BUILD_<KIND> 0 压空容器，声明局部占据该栈位。
        self.emit_byte(build_op as u8, line);
        self.emit_byte(0, line);
        self.declare_local("__comp_container", line)?;
        let container_slot = self
            .resolve_local("__comp_container")
            .ok_or("internal: __comp_container not found after declare")?;

        // 一次性预留所有子句的 iter/target slot（Nil 占位 + 声明），收集其 slot 索引。
        let mut clauses: Vec<CompClauseSlots> = Vec::with_capacity(for_clauses.len());
        for (i, clause) in for_clauses.iter().enumerate() {
            let iter_name = format!("__comp_iter_{}", i);
            self.emit_byte(OpCode::Nil as u8, line);
            self.declare_local(&iter_name, line)?;
            let iter = self
                .resolve_local(&iter_name)
                .ok_or("internal: comp iter slot not found after declare")?;
            let mut targets = Vec::with_capacity(clause.targets.len());
            for target in &clause.targets {
                self.emit_byte(OpCode::Nil as u8, line);
                self.declare_local(target, line)?;
                targets.push(
                    self.resolve_local(target)
                        .ok_or("internal: comp target slot not found after declare")?,
                );
            }
            clauses.push(CompClauseSlots {
                iterable: clause.iterable.as_ref(),
                iter,
                targets,
            });
        }

        // 递归编译嵌套循环（最内层执行过滤 + 插入/追加）。
        self.compile_comp_clause(payload, condition, 0, container_slot, &clauses, line)?;

        // 结果副本留栈顶；end_scope 依次 POP 本作用域所有 local（容器 slot 上方的
        // iter/target 与此 LOAD_LOCAL 副本），恰好留下原始 container slot 之值作为结果。
        self.emit_byte(OpCode::LoadLocal as u8, line);
        self.emit_byte(container_slot as u8, line);
        self.end_scope(line);
        Ok(())
    }

    /// 编译列表推导式 `[expr for ... (if cond)?]`（task 33）。
    fn compile_list_comprehension(
        &mut self,
        expr: &Expr,
        for_clauses: &[ForClause],
        condition: &Option<Box<Expr>>,
        line: usize,
    ) -> Result<(), String> {
        self.compile_comprehension(
            OpCode::BuildList,
            CompPayload::List(expr),
            for_clauses,
            condition,
            line,
        )
    }

    /// 编译字典推导式 `{k: v for ... (if cond)?}`（task 34）。
    fn compile_dict_comprehension(
        &mut self,
        key_expr: &Expr,
        value_expr: &Expr,
        for_clauses: &[ForClause],
        condition: &Option<Box<Expr>>,
        line: usize,
    ) -> Result<(), String> {
        self.compile_comprehension(
            OpCode::BuildDict,
            CompPayload::Dict {
                key: key_expr,
                value: value_expr,
            },
            for_clauses,
            condition,
            line,
        )
    }

    /// 编译集合推导式 `{expr for ... (if cond)?}`（task 34）。
    fn compile_set_comprehension(
        &mut self,
        expr: &Expr,
        for_clauses: &[ForClause],
        condition: &Option<Box<Expr>>,
        line: usize,
    ) -> Result<(), String> {
        self.compile_comprehension(
            OpCode::BuildSet,
            CompPayload::Set(expr),
            for_clauses,
            condition,
            line,
        )
    }

    /// 编译第 `i` 个 for 子句的循环结构（递归至最内层）。所有 slot 已由调用方预留并打包进
    /// `clauses`。
    fn compile_comp_clause(
        &mut self,
        payload: CompPayload,
        condition: &Option<Box<Expr>>,
        i: usize,
        container_slot: usize,
        clauses: &[CompClauseSlots],
        line: usize,
    ) -> Result<(), String> {
        let clause = &clauses[i];
        // 设置迭代器：编译 iterable → ITERATOR → 写入预留的 iter_slot。
        self.compile_expression(clause.iterable, line)?;
        self.emit_byte(OpCode::Iterator as u8, line);
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(clause.iter as u8, line);

        // 循环头：FOR_ITER iter_slot exit_offset(2)。
        let loop_start = self.current_offset();
        let exit = self.emit_for_iter(clause.iter as u8, line);

        // 存储循环目标（参照 compile_for_in：单变量直接存；多变量 UNPACK 后正序存）。
        let targets = &clause.targets;
        if targets.len() == 1 {
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(targets[0] as u8, line);
        } else {
            let n = u8::try_from(targets.len())
                .map_err(|_| format!("too many unpack targets (max 255, got {})", targets.len()))?;
            self.emit_byte(OpCode::Unpack as u8, line);
            self.emit_byte(n, line);
            for &slot in targets {
                self.emit_byte(OpCode::StoreLocal as u8, line);
                self.emit_byte(slot as u8, line);
            }
        }

        if i + 1 < clauses.len() {
            // 嵌套下一层 for 子句。
            self.compile_comp_clause(payload, condition, i + 1, container_slot, clauses, line)?;
        } else if let Some(cond) = condition {
            // 最内层 + 过滤：JUMP_IF_FALSE 跳过插入/追加。JumpIfFalse 仅 peek 不弹 cond，
            // 故真/假两支各需一次 POP；真支插入后须 JUMP 越过假支的清理 POP。
            // （§2 伪代码遗漏此 JUMP，会使真支跌入假支 POP 弹错值。）
            self.compile_expression(cond, line)?;
            let skip = self.emit_jump(OpCode::JumpIfFalse, line);
            self.emit_byte(OpCode::Pop as u8, line);
            self.comp_do_build(payload, container_slot, line)?;
            let end_jump = self.emit_jump(OpCode::Jump, line);
            self.patch_jump(skip)?;
            self.emit_byte(OpCode::Pop as u8, line);
            self.patch_jump(end_jump)?;
        } else {
            // 最内层无过滤：直接插入/追加。
            self.comp_do_build(payload, container_slot, line)?;
        }

        // 回边：跳回本子句循环头。
        let back = self.emit_jump(OpCode::JumpBack, line);
        self.patch_jump_back(back, loop_start)?;
        // 出口：FOR_ITER 耗尽落地处。
        self.patch_jump(exit)?;
        Ok(())
    }

    /// 最内层插入/追加（task 33/34 共用）：
    /// - list：编译 expr 后 `LIST_APPEND slot`；
    /// - set：编译 expr 后 `SET_ADD slot`；
    /// - dict：编译 key、value 后 `DICT_INSERT slot`（key 先压栈，val 在栈顶）。
    ///
    /// 操作数均为 container slot，弹出栈顶值原地修改，不 push 返回值。
    fn comp_do_build(
        &mut self,
        payload: CompPayload,
        slot: usize,
        line: usize,
    ) -> Result<(), String> {
        match payload {
            CompPayload::List(expr) => {
                self.compile_expression(expr, line)?;
                self.emit_byte(OpCode::ListAppend as u8, line);
            }
            CompPayload::Set(expr) => {
                self.compile_expression(expr, line)?;
                self.emit_byte(OpCode::SetAdd as u8, line);
            }
            CompPayload::Dict { key, value } => {
                self.compile_expression(key, line)?;
                self.compile_expression(value, line)?;
                self.emit_byte(OpCode::DictInsert as u8, line);
            }
        }
        self.emit_byte(slot as u8, line);
        Ok(())
    }
}

struct GenClause {
    iter: usize,
    targets: Vec<usize>,
}

impl Compiler {
    /// 编译生成器表达式 `(expr for x in iter (if cond)?)`（task 39）。
    ///
    /// 变换为匿名生成器闭包并立即调用：
    ///   fn __gen_expr_N(iter) {
    ///       for x in iter {           // 首子句用参数 iter
    ///           for y in iter2 { ... } // 后续子句编译各自 iterable
    ///           if cond { yield expr }
    ///       }
    ///   }
    /// 外层变量经 upvalue 捕获（R7）。函数名用单调计数器保证唯一（R8）。
    fn compile_generator_expression(
        &mut self,
        expr: &Expr,
        for_clauses: &[ForClause],
        condition: &Option<Box<Expr>>,
        line: usize,
    ) -> Result<(), String> {
        let name = format!("__gen_expr_{}", self.gen_expr_counter);
        self.gen_expr_counter += 1;

        // 新建编译单元 fn __gen_expr_N(iter)。
        let func_unit = super::CompilationUnit {
            chunk: super::Chunk::new(),
            locals: vec![
                super::Local {
                    name: "<self>".to_string(),
                    depth: 0,
                    is_captured: false,
                },
                super::Local {
                    name: "iter".to_string(),
                    depth: 0,
                    is_captured: false,
                },
            ],
            upvalues: Vec::new(),
            scope_depth: 0,
            is_generator: true,
            parent: std::ptr::null(),
        };
        let saved_unit = std::mem::replace(&mut self.unit, func_unit);
        self.unit.parent = std::ptr::addr_of!(saved_unit);

        // 预留所有子句的 iter/target slot（Nil 占位 + declare_local）。
        let mut clauses: Vec<GenClause> = Vec::with_capacity(for_clauses.len());
        for (i, clause) in for_clauses.iter().enumerate() {
            let iter_name = format!("__gen_iter_{}", i);
            self.emit_byte(OpCode::Nil as u8, line);
            self.declare_local(&iter_name, line)?;
            let iter = self
                .resolve_local(&iter_name)
                .ok_or("internal: gen-expr iter slot")?;
            let mut targets = Vec::with_capacity(clause.targets.len());
            for target in &clause.targets {
                self.emit_byte(OpCode::Nil as u8, line);
                self.declare_local(target, line)?;
                targets.push(
                    self.resolve_local(target)
                        .ok_or("internal: gen-expr target slot")?,
                );
            }
            clauses.push(GenClause { iter, targets });
        }

        // 递归编译嵌套循环（首子句用参数 iter，后续子句编译各自 iterable）。
        self.compile_gen_expr_loop(expr, condition, 0, &clauses, for_clauses, line)?;

        // 隐式 return nil + emit_return（含 EXEC_DEFER）。
        self.emit_byte(OpCode::Nil as u8, line);
        self.emit_return(line);

        // 换回父单元 + 上值回填。
        let func_unit = std::mem::replace(&mut self.unit, saved_unit);
        let captured: Vec<usize> = func_unit
            .upvalues
            .iter()
            .filter(|uv| uv.is_local)
            .map(|uv| uv.index)
            .collect();
        for idx in captured {
            if idx < self.unit.locals.len() {
                self.unit.locals[idx].is_captured = true;
            }
        }

        // 存 Function + 发 CLOSURE + 上值操作数。
        // 注意：CLOSURE 先发（压闭包于栈），再编译首子句 iterable（压实参于栈），
        // 使 CALL 1 的栈布局为 [closure, arg]（callee 在底，实参在顶）。
        let function = crate::vm::object::Function {
            name: name.clone(),
            arity: 1,
            code: func_unit.chunk.code,
            constants: func_unit.chunk.constants,
            upvalue_count: func_unit.upvalues.len(),
            source_file: self.source_file.clone(),
            default_values: Vec::new(),
            has_variadic: false,
            required_arity: 1,
            is_generator: true,
            locals_count: func_unit.locals.len(),
        };
        let func_idx = self.add_constant(crate::vm::object::alloc_function(function));
        let func_idx = u16::try_from(func_idx).map_err(|_| "constant pool overflow".to_string())?;
        self.emit_byte(OpCode::Closure as u8, line);
        self.emit_bytes(&func_idx.to_be_bytes(), line);
        for uv in &func_unit.upvalues {
            self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
            let idx =
                u8::try_from(uv.index).map_err(|_| "upvalue index exceeds 255".to_string())?;
            self.emit_byte(idx, line);
        }

        // 外层：编译首子句 iterable（压栈，作为 CALL 1 的实参）。
        self.compile_expression(&for_clauses[0].iterable, line)?;

        // CALL 1：用首子句 iterable 调用闭包 → 返回 Generator 对象。
        self.emit_byte(OpCode::Call as u8, line);
        self.emit_byte(1, line);
        Ok(())
    }

    /// 生成器表达式嵌套循环递归编译（task 39）。
    fn compile_gen_expr_loop(
        &mut self,
        expr: &Expr,
        condition: &Option<Box<Expr>>,
        i: usize,
        clauses: &[GenClause],
        for_clauses: &[ForClause],
        line: usize,
    ) -> Result<(), String> {
        let clause = &clauses[i];

        // 设置迭代器：首子句用参数 iter（slot 1），后续子句编译各自 iterable。
        if i == 0 {
            self.emit_byte(OpCode::LoadLocal as u8, line);
            self.emit_byte(1, line); // slot 1 = iter parameter
        } else {
            self.compile_expression(&for_clauses[i].iterable, line)?;
        }
        self.emit_byte(OpCode::Iterator as u8, line);
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(clause.iter as u8, line);

        // 循环头。
        let loop_start = self.current_offset();
        let exit = self.emit_for_iter(clause.iter as u8, line);

        // 存储循环目标。
        let targets = &clause.targets;
        if targets.len() == 1 {
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(targets[0] as u8, line);
        } else {
            let n =
                u8::try_from(targets.len()).map_err(|_| "too many unpack targets".to_string())?;
            self.emit_byte(OpCode::Unpack as u8, line);
            self.emit_byte(n, line);
            for &slot in targets {
                self.emit_byte(OpCode::StoreLocal as u8, line);
                self.emit_byte(slot as u8, line);
            }
        }

        if i + 1 < clauses.len() {
            self.compile_gen_expr_loop(expr, condition, i + 1, clauses, for_clauses, line)?;
        } else if let Some(cond) = condition {
            // 条件过滤 + yield。
            self.compile_expression(cond, line)?;
            let skip = self.emit_jump(OpCode::JumpIfFalse, line);
            self.emit_byte(OpCode::Pop as u8, line);
            self.compile_expression(expr, line)?;
            self.emit_byte(OpCode::Yield as u8, line);
            let end_jump = self.emit_jump(OpCode::Jump, line);
            self.patch_jump(skip)?;
            self.emit_byte(OpCode::Pop as u8, line);
            self.patch_jump(end_jump)?;
        } else {
            // 无条件 yield。
            self.compile_expression(expr, line)?;
            self.emit_byte(OpCode::Yield as u8, line);
        }

        // 回边。
        let back = self.emit_jump(OpCode::JumpBack, line);
        self.patch_jump_back(back, loop_start)?;
        // 出口。
        self.patch_jump(exit)?;
        Ok(())
    }
}

#[cfg(test)]
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
mod tests {
    use crate::ast::node::{AssignOp, BinaryOp, Expr, ForClause, Literal, UnaryOp};
    use crate::compiler::{Compiler, OpCode};

    /// 在编译产物中查找指定 opcode 的字节码偏移。
    fn find_opcode(compiler: &Compiler, opcode: OpCode) -> Option<usize> {
        compiler
            .chunk()
            .code
            .iter()
            .position(|&b| b == opcode as u8)
    }

    #[test]
    fn test_compile_int_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::Literal(Literal::Int(42));
        compiler.compile_expression(&expr, 1).unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
        assert_eq!(compiler.chunk().constants.len(), 1);
    }

    #[test]
    fn test_compile_float_literal() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Literal(Literal::Float(3.14)), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
    }

    #[test]
    fn test_compile_string_literal() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Literal(Literal::String("hi".into())), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
    }

    #[test]
    fn test_compile_bool_true() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Literal(Literal::Bool(true)), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::True as u8);
    }

    #[test]
    fn test_compile_bool_false() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Literal(Literal::Bool(false)), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::False as u8);
    }

    #[test]
    fn test_compile_nil() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Literal(Literal::Nil), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Nil as u8);
    }

    #[test]
    fn test_compile_identifier_global() {
        let mut compiler = Compiler::new();
        compiler
            .compile_expression(&Expr::Identifier("x".into()), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::LoadGlobal as u8);
    }

    #[test]
    fn test_compile_identifier_local() {
        let mut compiler = Compiler::new();
        compiler.declare_local("x", 1).unwrap();
        compiler
            .compile_expression(&Expr::Identifier("x".into()), 1)
            .unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::LoadLocal as u8);
        // slot 1 (slot 0 reserved for the implicit function-local).
        assert_eq!(compiler.chunk().code[1], 1);
    }

    #[test]
    fn test_compile_binary_add() {
        let mut compiler = Compiler::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Literal::Int(1))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Literal::Int(2))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        assert_eq!(code[0], OpCode::Constant as u8);
        assert_eq!(code[3], OpCode::Constant as u8);
        assert_eq!(code[6], OpCode::Add as u8);
    }

    #[test]
    fn test_compile_binary_comparison() {
        let mut compiler = Compiler::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Literal::Int(1))),
            op: BinaryOp::Less,
            right: Box::new(Expr::Literal(Literal::Int(2))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert_eq!(*compiler.chunk().code.last().unwrap(), OpCode::Less as u8);
    }

    #[test]
    fn test_compile_unary_negate() {
        let mut compiler = Compiler::new();
        let expr = Expr::Unary {
            op: UnaryOp::Negate,
            operand: Box::new(Expr::Literal(Literal::Int(5))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        assert_eq!(code[0], OpCode::Constant as u8);
        assert_eq!(code[3], OpCode::Negate as u8);
    }

    #[test]
    fn test_compile_unary_not() {
        let mut compiler = Compiler::new();
        let expr = Expr::Unary {
            op: UnaryOp::Not,
            operand: Box::new(Expr::Identifier("flag".into())),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::Not).is_some());
    }

    #[test]
    fn test_compile_ternary() {
        let mut compiler = Compiler::new();
        let expr = Expr::Ternary {
            condition: Box::new(Expr::Literal(Literal::Bool(true))),
            then_expr: Box::new(Expr::Literal(Literal::String("yes".into()))),
            else_expr: Box::new(Expr::Literal(Literal::String("no".into()))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::JumpIfFalse).is_some());
        assert!(find_opcode(&compiler, OpCode::Jump).is_some());
    }

    #[test]
    fn test_compile_logical_and_short_circuit() {
        let mut compiler = Compiler::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Identifier("a".into())),
            op: BinaryOp::And,
            right: Box::new(Expr::Identifier("b".into())),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::JumpIfFalse).is_some());
        assert!(find_opcode(&compiler, OpCode::Pop).is_some());
    }

    #[test]
    fn test_compile_logical_or_short_circuit() {
        let mut compiler = Compiler::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Identifier("a".into())),
            op: BinaryOp::Or,
            right: Box::new(Expr::Identifier("b".into())),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::JumpIfTrue).is_some());
        assert!(find_opcode(&compiler, OpCode::Pop).is_some());
    }

    #[test]
    fn test_compile_call() {
        let mut compiler = Compiler::new();
        let expr = Expr::Call {
            callee: Box::new(Expr::Identifier("f".into())),
            args: vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
            ],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::Call).unwrap();
        assert_eq!(compiler.chunk().code[pos + 1], 2); // argc
    }

    #[test]
    fn test_compile_index() {
        let mut compiler = Compiler::new();
        let expr = Expr::Index {
            object: Box::new(Expr::Identifier("arr".into())),
            index: Box::new(Expr::Literal(Literal::Int(0))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::GetIndex).is_some());
    }

    #[test]
    fn test_compile_dot() {
        let mut compiler = Compiler::new();
        let expr = Expr::Dot {
            object: Box::new(Expr::Identifier("obj".into())),
            name: "field".into(),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::GetAttr).is_some());
    }

    #[test]
    fn test_compile_slice() {
        let mut compiler = Compiler::new();
        let expr = Expr::Slice {
            object: Box::new(Expr::Identifier("lst".into())),
            start: Some(Box::new(Expr::Literal(Literal::Int(1)))),
            stop: Some(Box::new(Expr::Literal(Literal::Int(5)))),
            step: None,
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::GetSlice).unwrap();
        // flags: has_start(1) | has_stop(2) = 0b011
        assert_eq!(compiler.chunk().code[pos + 1], 0b011);
    }

    #[test]
    fn test_compile_slice_step_only() {
        let mut compiler = Compiler::new();
        let expr = Expr::Slice {
            object: Box::new(Expr::Identifier("lst".into())),
            start: None,
            stop: None,
            step: Some(Box::new(Expr::Literal(Literal::Int(2)))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::GetSlice).unwrap();
        // flags: has_step(4) = 0b100
        assert_eq!(compiler.chunk().code[pos + 1], 0b100);
    }

    #[test]
    fn test_compile_list_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::ListLiteral {
            elements: vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
                Expr::Literal(Literal::Int(3)),
            ],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::BuildList).unwrap();
        assert_eq!(compiler.chunk().code[pos + 1], 3);
    }

    #[test]
    fn test_compile_dict_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::DictLiteral {
            pairs: vec![(
                Expr::Literal(Literal::String("k".into())),
                Expr::Literal(Literal::Int(1)),
            )],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::BuildDict).unwrap();
        assert_eq!(compiler.chunk().code[pos + 1], 1);
    }

    #[test]
    fn test_compile_set_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::SetLiteral {
            elements: vec![Expr::Literal(Literal::Int(1))],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::BuildSet).is_some());
    }

    #[test]
    fn test_compile_tuple_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::TupleLiteral {
            elements: vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
            ],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let pos = find_opcode(&compiler, OpCode::BuildTuple).unwrap();
        assert_eq!(compiler.chunk().code[pos + 1], 2);
    }

    #[test]
    fn test_compile_store_target_tuple_emits_unpack_forward() {
        // task 30：TupleLiteral 赋值目标发射 UNPACK + 正序 StoreLocal。
        // slot 0 预留（callee），故 a→slot 1、b→slot 2。
        let mut compiler = Compiler::new();
        compiler.declare_local("a", 1).unwrap();
        compiler.declare_local("b", 1).unwrap();
        let target = Expr::TupleLiteral {
            elements: vec![Expr::Identifier("a".into()), Expr::Identifier("b".into())],
        };
        compiler.compile_store_target(&target, 1).unwrap();
        let code = &compiler.chunk().code;
        let pos = find_opcode(&compiler, OpCode::Unpack).unwrap();
        // UNPACK 2，随后 StoreLocal a(槽1)、StoreLocal b(槽2)——正序。
        assert_eq!(code[pos], OpCode::Unpack as u8);
        assert_eq!(code[pos + 1], 2);
        assert_eq!(code[pos + 2], OpCode::StoreLocal as u8);
        assert_eq!(code[pos + 3], 1); // a = 槽 1
        assert_eq!(code[pos + 4], OpCode::StoreLocal as u8);
        assert_eq!(code[pos + 5], 2); // b = 槽 2
    }

    #[test]
    fn test_compile_store_target_tuple_too_many_targets() {
        // 超过 255 个解包目标 → 编译错误（u8 溢出）。
        let mut compiler = Compiler::new();
        let targets: Vec<Expr> = (0..256)
            .map(|i| Expr::Identifier(format!("v{}", i)))
            .collect();
        let target = Expr::TupleLiteral { elements: targets };
        let r = compiler.compile_store_target(&target, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("too many unpack targets"));
    }

    #[test]
    fn test_compile_load_target_tuple_errors() {
        // task 30：复合赋值不能以 tuple 为目标（防御性）。
        let mut compiler = Compiler::new();
        let target = Expr::TupleLiteral { elements: vec![] };
        let r = compiler.compile_load_target(&target, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("tuple"));
    }

    #[test]
    fn test_compile_assignment_simple_global() {
        let mut compiler = Compiler::new();
        let expr = Expr::Assign {
            target: Box::new(Expr::Identifier("x".into())),
            op: AssignOp::Assign,
            value: Box::new(Expr::Literal(Literal::Int(42))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::StoreGlobal).is_some());
        assert!(find_opcode(&compiler, OpCode::Dup).is_some());
    }

    #[test]
    fn test_compile_assignment_local() {
        let mut compiler = Compiler::new();
        compiler.declare_local("x", 1).unwrap();
        let expr = Expr::Assign {
            target: Box::new(Expr::Identifier("x".into())),
            op: AssignOp::Assign,
            value: Box::new(Expr::Literal(Literal::Int(42))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::StoreLocal).is_some());
    }

    #[test]
    fn test_compile_assignment_compound() {
        let mut compiler = Compiler::new();
        let expr = Expr::Assign {
            target: Box::new(Expr::Identifier("x".into())),
            op: AssignOp::PlusAssign,
            value: Box::new(Expr::Literal(Literal::Int(1))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        // 复合赋值应产生：LOAD_GLOBAL ... ADD ... DUP STORE_GLOBAL
        assert!(find_opcode(&compiler, OpCode::LoadGlobal).is_some());
        assert!(find_opcode(&compiler, OpCode::Add).is_some());
        assert!(find_opcode(&compiler, OpCode::StoreGlobal).is_some());
    }

    #[test]
    fn test_compile_assignment_index_target() {
        let mut compiler = Compiler::new();
        let expr = Expr::Assign {
            target: Box::new(Expr::Index {
                object: Box::new(Expr::Identifier("arr".into())),
                index: Box::new(Expr::Literal(Literal::Int(0))),
            }),
            op: AssignOp::Assign,
            value: Box::new(Expr::Literal(Literal::Int(99))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::SetIndex).is_some());
    }

    #[test]
    fn test_compile_assignment_dot_target() {
        let mut compiler = Compiler::new();
        let expr = Expr::Assign {
            target: Box::new(Expr::Dot {
                object: Box::new(Expr::Identifier("obj".into())),
                name: "field".into(),
            }),
            op: AssignOp::Assign,
            value: Box::new(Expr::Literal(Literal::Int(99))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::SetAttr).is_some());
    }

    #[test]
    fn test_compile_grouping() {
        let mut compiler = Compiler::new();
        let expr = Expr::Grouping {
            expr: Box::new(Expr::Literal(Literal::Int(42))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
    }

    #[test]
    fn test_compile_fn_literal() {
        // task 29：匿名函数字面量编译为 CLOSURE 指令（闭包值留栈作为表达式结果，
        // 不发 STORE_GLOBAL —— 与具名函数声明的关键差异）。
        use crate::ast::node::Param;
        let mut compiler = Compiler::new();
        let expr = Expr::FnLiteral {
            params: vec![Param {
                name: "x".to_string(),
                default: None,
                is_variadic: false,
            }],
            body: vec![],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::Closure).is_some());
        // 匿名函数是表达式，不绑定全局名 → 不发 STORE_GLOBAL
        assert!(find_opcode(&compiler, OpCode::StoreGlobal).is_none());
    }

    #[test]
    fn test_compile_anon_fn_in_dict_literal() {
        // spec §验证标准 #6 编译端覆盖：匿名函数作为集合（dict）值编译为 CLOSURE，
        // 随后由 BuildDict 打包。注：dict/list 集合的「运行期」执行（BuildDict/
        // GetIndex 操作码）尚未在 VM 实装（独立任务，非 task 29），故此处仅验证
        // 编译端正确生成 CLOSURE + BuildDict，端到端 dict 下标调用待 VM 实装。
        use crate::ast::node::Param;
        let mut compiler = Compiler::new();
        let expr = Expr::DictLiteral {
            pairs: vec![(
                Expr::Literal(Literal::String("add".into())),
                Expr::FnLiteral {
                    params: vec![
                        Param {
                            name: "a".to_string(),
                            default: None,
                            is_variadic: false,
                        },
                        Param {
                            name: "b".to_string(),
                            default: None,
                            is_variadic: false,
                        },
                    ],
                    body: vec![],
                },
            )],
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::Closure).is_some());
        assert!(find_opcode(&compiler, OpCode::BuildDict).is_some());
    }

    #[test]
    fn test_compile_await_returns_error() {
        let mut compiler = Compiler::new();
        let expr = Expr::Await {
            expr: Box::new(Expr::Literal(Literal::Nil)),
        };
        assert!(compiler.compile_expression(&expr, 1).is_err());
    }

    // ---- task 33/34：推导式（list / dict / set）----

    fn make_comp(expr: Expr, clauses: Vec<(&str, Expr)>, condition: Option<Expr>) -> Expr {
        Expr::ListComprehension {
            expr: Box::new(expr),
            for_clauses: clauses
                .into_iter()
                .map(|(t, it)| ForClause {
                    targets: vec![t.to_string()],
                    iterable: Box::new(it),
                })
                .collect(),
            condition: condition.map(Box::new),
        }
    }

    /// 构造 dict 推导式 AST `{key: value for ...}`（单变量子句）。
    fn make_dict_comp(
        key: Expr,
        value: Expr,
        clauses: Vec<(&str, Expr)>,
        condition: Option<Expr>,
    ) -> Expr {
        Expr::DictComprehension {
            key_expr: Box::new(key),
            value_expr: Box::new(value),
            for_clauses: clauses
                .into_iter()
                .map(|(t, it)| ForClause {
                    targets: vec![t.to_string()],
                    iterable: Box::new(it),
                })
                .collect(),
            condition: condition.map(Box::new),
        }
    }

    /// 构造 set 推导式 AST `{expr for ...}`（单变量子句）。
    fn make_set_comp(expr: Expr, clauses: Vec<(&str, Expr)>, condition: Option<Expr>) -> Expr {
        Expr::SetComprehension {
            expr: Box::new(expr),
            for_clauses: clauses
                .into_iter()
                .map(|(t, it)| ForClause {
                    targets: vec![t.to_string()],
                    iterable: Box::new(it),
                })
                .collect(),
            condition: condition.map(Box::new),
        }
    }

    #[test]
    fn test_compile_list_comprehension_emits_core_opcodes() {
        // [x for x in xs]：应发射 BUILD_LIST、ITERATOR、FOR_ITER、LIST_APPEND，
        // 并以 LOAD_LOCAL 收尾（结果留栈顶）。
        let mut compiler = Compiler::new();
        let expr = make_comp(
            Expr::Identifier("x".into()),
            vec![("x", Expr::Identifier("xs".into()))],
            None,
        );
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::BuildList).is_some());
        assert!(find_opcode(&compiler, OpCode::Iterator).is_some());
        assert!(find_opcode(&compiler, OpCode::ForIter).is_some());
        assert!(find_opcode(&compiler, OpCode::ListAppend).is_some());
        assert!(find_opcode(&compiler, OpCode::LoadLocal).is_some());
    }

    #[test]
    fn test_compile_list_comprehension_filter_emits_jump_if_false() {
        // [x for x in xs if x > 0]：过滤分支发射 JUMP_IF_FALSE + JUMP（修正后的平衡结构）。
        let mut compiler = Compiler::new();
        let cond = Expr::Binary {
            left: Box::new(Expr::Identifier("x".into())),
            op: BinaryOp::Greater,
            right: Box::new(Expr::Literal(Literal::Int(0))),
        };
        let expr = make_comp(
            Expr::Identifier("x".into()),
            vec![("x", Expr::Identifier("xs".into()))],
            Some(cond),
        );
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::JumpIfFalse).is_some());
        assert!(find_opcode(&compiler, OpCode::Jump).is_some());
        assert!(find_opcode(&compiler, OpCode::ListAppend).is_some());
    }

    #[test]
    fn test_compile_list_comprehension_nested_two_clauses() {
        // [x for row in m for x in row]：两子句 → 两个 FOR_ITER + 两个 ITERATOR。
        let mut compiler = Compiler::new();
        let expr = make_comp(
            Expr::Identifier("x".into()),
            vec![
                ("row", Expr::Identifier("m".into())),
                ("x", Expr::Identifier("row".into())),
            ],
            None,
        );
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        let iter_count = code.iter().filter(|&&b| b == OpCode::ForIter as u8).count();
        assert_eq!(iter_count, 2);
    }

    // ---- task 34：dict / set 推导式 ----

    #[test]
    fn test_compile_dict_comprehension_emits_core_opcodes() {
        // {x: x*x for x in xs}：应发射 BUILD_DICT、ITERATOR、FOR_ITER、DICT_INSERT，
        // 并以 LOAD_LOCAL 收尾（结果留栈顶）。
        let mut compiler = Compiler::new();
        let expr = make_dict_comp(
            Expr::Identifier("x".into()),
            Expr::Identifier("x".into()),
            vec![("x", Expr::Identifier("xs".into()))],
            None,
        );
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::BuildDict).is_some());
        assert!(find_opcode(&compiler, OpCode::Iterator).is_some());
        assert!(find_opcode(&compiler, OpCode::ForIter).is_some());
        assert!(find_opcode(&compiler, OpCode::DictInsert).is_some());
        assert!(find_opcode(&compiler, OpCode::LoadLocal).is_some());
        // 不应发射 list/set 构造/追加指令
        assert!(find_opcode(&compiler, OpCode::BuildSet).is_none());
        assert!(find_opcode(&compiler, OpCode::SetAdd).is_none());
    }

    #[test]
    fn test_compile_set_comprehension_emits_core_opcodes() {
        // {x for x in xs}：应发射 BUILD_SET、ITERATOR、FOR_ITER、SET_ADD、LOAD_LOCAL。
        let mut compiler = Compiler::new();
        let expr = make_set_comp(
            Expr::Identifier("x".into()),
            vec![("x", Expr::Identifier("xs".into()))],
            None,
        );
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::BuildSet).is_some());
        assert!(find_opcode(&compiler, OpCode::Iterator).is_some());
        assert!(find_opcode(&compiler, OpCode::ForIter).is_some());
        assert!(find_opcode(&compiler, OpCode::SetAdd).is_some());
        assert!(find_opcode(&compiler, OpCode::LoadLocal).is_some());
        // 不应发射 dict 构造/插入指令
        assert!(find_opcode(&compiler, OpCode::BuildDict).is_none());
        assert!(find_opcode(&compiler, OpCode::DictInsert).is_none());
    }

    #[test]
    fn test_compile_dict_comprehension_filter_emits_jump_if_false() {
        // {x: x*x for x in xs if x > 0}：过滤分支发射 JUMP_IF_FALSE + JUMP + DICT_INSERT。
        let mut compiler = Compiler::new();
        let cond = Expr::Binary {
            left: Box::new(Expr::Identifier("x".into())),
            op: BinaryOp::Greater,
            right: Box::new(Expr::Literal(Literal::Int(0))),
        };
        let expr = make_dict_comp(
            Expr::Identifier("x".into()),
            Expr::Identifier("x".into()),
            vec![("x", Expr::Identifier("xs".into()))],
            Some(cond),
        );
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::JumpIfFalse).is_some());
        assert!(find_opcode(&compiler, OpCode::Jump).is_some());
        assert!(find_opcode(&compiler, OpCode::DictInsert).is_some());
    }

    #[test]
    fn test_compile_set_comprehension_nested_two_clauses() {
        // {x for row in m for x in row}：两子句 → 两个 FOR_ITER + 两个 ITERATOR。
        let mut compiler = Compiler::new();
        let expr = make_set_comp(
            Expr::Identifier("x".into()),
            vec![
                ("row", Expr::Identifier("m".into())),
                ("x", Expr::Identifier("row".into())),
            ],
            None,
        );
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        let iter_count = code.iter().filter(|&&b| b == OpCode::ForIter as u8).count();
        assert_eq!(iter_count, 2);
    }

    #[test]
    fn test_compile_dict_comprehension_multi_var_emits_unpack() {
        // {k: v for k, v in pairs}：多变量 → UNPACK + 两个 STORE_LOCAL 目标。
        let mut compiler = Compiler::new();
        let expr = Expr::DictComprehension {
            key_expr: Box::new(Expr::Identifier("k".into())),
            value_expr: Box::new(Expr::Identifier("v".into())),
            for_clauses: vec![ForClause {
                targets: vec!["k".to_string(), "v".to_string()],
                iterable: Box::new(Expr::Identifier("pairs".into())),
            }],
            condition: None,
        };
        compiler.compile_expression(&expr, 1).unwrap();
        assert!(find_opcode(&compiler, OpCode::Unpack).is_some());
        assert!(find_opcode(&compiler, OpCode::DictInsert).is_some());
    }
}
