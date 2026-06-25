//! mslang 表达式编译（task 18）。
//!
//! 将 AST 表达式节点翻译为栈式字节码指令序列。表达式编译的核心原则：
//! 每条表达式编译后，在栈顶留下一个结果值。
//!
//! 参照 [18-compile-expressions](../../../docs/mslang/tasks/18-compile-expressions.md)。

use crate::ast::node::{AssignOp, BinaryOp, Expr, Literal, UnaryOp};
use crate::vm::object::{alloc_string, Object};

use super::{Chunk, Compiler, OpCode};

// ---- 表达式编译入口与访问器 ----

impl Compiler<'_> {
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
            // 以下类型由后续 task 实现
            Expr::FnLiteral { .. } => {
                Err("fn literal compilation not yet implemented (task 29)".to_string())
            }
            Expr::ListComprehension { .. }
            | Expr::DictComprehension { .. }
            | Expr::SetComprehension { .. }
            | Expr::GeneratorExpression { .. } => {
                Err("comprehension compilation not yet implemented (task 33/34)".to_string())
            }
            Expr::SuperAccess { .. } => {
                Err("super compilation not yet implemented (task 42)".to_string())
            }
            Expr::Yield { .. } | Expr::YieldFrom { .. } => {
                Err("yield compilation not yet implemented (task 39)".to_string())
            }
            Expr::Await { .. } => {
                Err("await compilation not yet implemented (task 53)".to_string())
            }
            Expr::Go { .. } => Err("go compilation not yet implemented (task 55)".to_string()),
        }
    }
}

// ---- 字面量与标识符 ----

impl Compiler<'_> {
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

impl Compiler<'_> {
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

impl Compiler<'_> {
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

impl Compiler<'_> {
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
            _ => Err("Invalid assignment target".to_string()),
        }
    }

    /// 将栈顶值存储到赋值目标。
    fn compile_store_target(&mut self, target: &Expr, line: usize) -> Result<(), String> {
        match target {
            Expr::Identifier(name) => {
                if let Some(slot) = self.resolve_local(name) {
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
            _ => return Err("Invalid assignment target".to_string()),
        }
        Ok(())
    }
}

// ---- 三元、调用、下标、属性 ----

impl Compiler<'_> {
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

impl Compiler<'_> {
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

impl Compiler<'_> {
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

impl Compiler<'_> {
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
}

#[cfg(test)]
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
mod tests {
    use crate::ast::node::{AssignOp, BinaryOp, Expr, Literal, UnaryOp};
    use crate::compiler::{Compiler, OpCode};

    /// 在编译产物中查找指定 opcode 的字节码偏移。
    fn find_opcode(compiler: &Compiler<'_>, opcode: OpCode) -> Option<usize> {
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
    fn test_compile_fn_literal_returns_error() {
        let mut compiler = Compiler::new();
        let expr = Expr::FnLiteral {
            params: vec![],
            body: vec![],
        };
        assert!(compiler.compile_expression(&expr, 1).is_err());
    }

    #[test]
    fn test_compile_await_returns_error() {
        let mut compiler = Compiler::new();
        let expr = Expr::Await {
            expr: Box::new(Expr::Literal(Literal::Nil)),
        };
        assert!(compiler.compile_expression(&expr, 1).is_err());
    }
}
