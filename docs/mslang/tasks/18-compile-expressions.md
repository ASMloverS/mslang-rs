# 表达式编译

## 所属阶段
Phase 2.2b - 字节码编译 + VM 核心

## 前置任务
- 17-compiler-core
- 20-object-system-basic

## 目标

实现所有表达式类型的编译逻辑，将 AST 表达式节点翻译为栈式字节码指令序列。表达式编译的核心原则：**每条表达式编译后，在栈顶留下一个结果值**。

## 设计规格

引用 [03-syntax.md](../03-syntax.md) 表达式优先级与运算符类型规则：
- [02-types.md](../02-types.md) 运算符类型规则

### 表达式编译规则

| 表达式类型 | 编译策略 | 产生的指令 |
|---|---|---|
| 字面量 (Int/Float/String) | 加载常量 | `CONSTANT idx` |
| Bool (true/false) | 直接压栈 | `TRUE` / `FALSE` |
| Nil | 直接压栈 | `NIL` |
| 标识符 (局部变量) | 查找 slot | `LOAD_LOCAL slot` |
| 标识符 (全局变量) | 查找 name_idx | `LOAD_GLOBAL name_idx` |
| 二元运算 | 左操作数 → 右操作数 → 运算 | 左... 右... `OP` |
| 一元运算 | 操作数 → 运算 | 操作数... `NEGATE`/`NOT`/`BIT_NOT` |
| 赋值 | 右值 → 存储 | 右值... `STORE_*` |
| 三元 (if...else) | 条件跳转 | `JUMP_IF_FALSE` + `JUMP` |
| 后缀 (call/index/dot/slice) | 目标 → 参数 → 操作 | `CALL`/`GET_INDEX`/`GET_ATTR`/`GET_SLICE` |

## 实现细节

### 文件位置

`src/compiler/expression.rs`

### 编译字面量

```rust
fn compile_literal(&mut self, lit: &Literal, line: usize) -> Result<(), String> {
    match lit {
        Literal::Int(n) => self.emit_constant(Object::Int(*n), line),
        Literal::Float(f) => self.emit_constant(Object::Float(*f), line),
        Literal::String(s) => self.emit_constant(alloc_string(s), line),
        Literal::Bool(true) => self.emit_byte(OpCode::True as u8, line),
        Literal::Bool(false) => self.emit_byte(OpCode::False as u8, line),
        Literal::Nil => self.emit_byte(OpCode::Nil as u8, line),
    }
    Ok(())
}
```

### 编译标识符

```rust
fn compile_identifier(&mut self, name: &str, line: usize) -> Result<(), String> {
    if let Some(slot) = self.resolve_local(name) {
        self.emit_byte(OpCode::LoadLocal as u8, line);
        self.emit_byte(slot as u8, line);
    } else if let Some(idx) = self.resolve_upvalue(name) {
        self.emit_byte(OpCode::LoadUpvalue as u8, line);
        self.emit_byte(idx as u8, line);
    } else {
        let name_idx = self.add_constant(alloc_string(name));
        self.emit_byte(OpCode::LoadGlobal as u8, line);
        self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
    }
    Ok(())
}
```

### 编译二元表达式

```rust
fn compile_binary(&mut self, left: &Expr, op: &BinaryOp, right: &Expr, line: usize) -> Result<(), String> {
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
        _ => return Err(format!("Unsupported binary op: {:?}", op)),
    };
    self.emit_byte(opcode as u8, line);
    Ok(())
}
```

### 编译一元表达式

```rust
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
```

### 编译比较表达式（支持链式比较）

引用 [03-syntax.md](../03-syntax.md) 链式比较：`1 < x < 10` 等价于 `(1 < x) and (x < 10)`

编译策略：对每个中间操作数生成额外加载，避免依赖 Swap 指令。

```rust
/// 将 BinaryOp 比较运算符映射到 OpCode。
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

/// 编译链式比较：`1 < x < 10` 等价于 `(1 < x) and (x < 10)`。
/// comparisons 为 (op, right_expr) 对列表。
fn compile_comparison(&mut self, first: &Expr, comparisons: &[(BinaryOp, Expr)], line: usize) -> Result<(), String> {
    self.compile_expression(first, line)?;
    if comparisons.len() == 1 {
        let (op, right) = &comparisons[0];
        self.compile_expression(right, line)?;
        self.emit_byte(self.comparison_opcode(op) as u8, line);
        return Ok(());
    }
    // 链式比较：a op1 b op2 c => (a op1 b) and (b op2 c) and ...
    // 策略：对每段，加载右操作数→比较→如果为 false 短路跳到结束
    let mut end_jumps: Vec<usize> = Vec::new();
    for (i, (op, right)) in comparisons.iter().enumerate() {
        if i > 0 {
            // 重新加载上一个操作数作为本次左操作数
            self.compile_expression(&comparisons[i - 1].1, line)?;
        }
        self.compile_expression(right, line)?;
        self.emit_byte(self.comparison_opcode(op) as u8, line);
        // 如果比较结果为 false，短路到结束（合并为 and）
        let jump = self.emit_jump(OpCode::JumpIfFalse, line);
        end_jumps.push(jump);
        self.emit_byte(OpCode::Pop as u8, line); // 弹出 bool true
    }
    // 所有比较都为 true：压入 true
    self.emit_byte(OpCode::True as u8, line);
    for jump in &end_jumps {
        self.patch_jump(*jump)?;
    }
    Ok(())
}
```

### 编译赋值表达式

引用 [03-syntax.md](../03-syntax.md) 赋值语句产生式。

赋值是表达式，返回被赋的值。编译时在 STORE 前发射 `DUP` 以保留结果值在栈顶。

```rust
fn compile_assignment(&mut self, target: &Expr, op: &AssignOp, value: &Expr, line: usize) -> Result<(), String> {
    use AssignOp::*;
    // 判断是否复合赋值（x += y 等）
    let is_compound = !matches!(op, Assign);

    if is_compound {
        // 复合赋值：x += 5 => 先加载 x 的当前值，再编译右值，执行运算，最后存储
        // 1. 加载当前值到栈顶
        self.compile_load_target(target, line)?;
        // 2. 编译右值
        self.compile_expression(value, line)?;
        // 3. 发射运算 opcode
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
            Assign => unreachable!(),
        };
        self.emit_byte(arith_op as u8, line);
    } else {
        // 简单赋值：仅编译右值
        self.compile_expression(value, line)?;
    }
    // DUP：保留赋值结果值在栈顶（赋值表达式返回被赋的值）
    self.emit_byte(OpCode::Dup as u8, line);
    // 存储到目标
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
            // 栈顶布局（DUP 后）：[value]
            // 需要变为：[object, index, value] → SET_INDEX
            // 先弹出 value，编译 object/index，再重新压入 value
            // 简化：编译 object、index，然后从栈布局调整（VM 层面 SET_INDEX 从栈弹出 value, index, object）
            self.compile_expression(object, line)?;
            self.compile_expression(index, line)?;
            self.emit_byte(OpCode::SetIndex as u8, line);
        }
        Expr::Dot { object, name } => {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow".to_string())?;
            self.compile_expression(object, line)?;
            self.emit_byte(OpCode::SetAttr as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        }
        _ => return Err("Invalid assignment target".to_string()),
    }
    Ok(())
}
```

### 编译三元表达式

```rust
fn compile_ternary(&mut self, condition: &Expr, then_expr: &Expr, else_expr: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(condition, line)?;
    let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.compile_expression(then_expr, line)?;
    let end_jump = self.emit_jump(OpCode::Jump, line);
    self.patch_jump(else_jump)?;
    self.compile_expression(else_expr, line)?;
    self.patch_jump(end_jump)?;
    Ok(())
}
```

### 编译后缀表达式

```rust
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
    let name_idx = u16::try_from(name_idx)
        .map_err(|_| "constant pool overflow".to_string())?;
    self.emit_byte(OpCode::GetAttr as u8, line);
    self.emit_bytes(&name_idx.to_be_bytes(), line);
    Ok(())
}
```

### 编译逻辑表达式（短路求值）

引用 [02-types.md](../02-types.md)：`and`/`or` 返回实际值（短路），`not` 返回 bool

```rust
fn compile_logical_and(&mut self, left: &Expr, right: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(left, line)?;
    let end_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.emit_byte(OpCode::Pop as u8, line);
    self.compile_expression(right, line)?;
    self.patch_jump(end_jump)?;
    Ok(())
}

fn compile_logical_or(&mut self, left: &Expr, right: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(left, line)?;
    let end_jump = self.emit_jump(OpCode::JumpIfTrue, line);
    self.emit_byte(OpCode::Pop as u8, line);
    self.compile_expression(right, line)?;
    self.patch_jump(end_jump)?;
    Ok(())
}
```

### 表达式编译分发器

核心入口方法，根据 `Expr` 变体路由到对应编译方法。未实现的类型返回错误。

```rust
/// 获取当前字节码偏移量。
pub fn current_offset(&self) -> usize {
    self.unit.chunk.code.len()
}

/// 编译表达式。编译后栈顶留下一个结果值。
pub fn compile_expression(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
    match expr {
        Expr::Literal(lit) => self.compile_literal(lit, line),
        Expr::Identifier(name) => self.compile_identifier(name, line),
        Expr::Binary { left, op, right } => {
            match op {
                // 逻辑运算符短路求值（不走 compile_binary）
                BinaryOp::And => {
                    self.compile_logical_and(left, right, line)
                }
                BinaryOp::Or => {
                    self.compile_logical_or(left, right, line)
                }
                _ => self.compile_binary(left, op, right, line),
            }
        }
        Expr::Unary { op, operand } => self.compile_unary(op, operand, line),
        Expr::Assign { target, op, value } => self.compile_assignment(target, op, value, line),
        Expr::Ternary { condition, then_expr, else_expr } => {
            self.compile_ternary(condition, then_expr, else_expr, line)
        }
        Expr::Call { callee, args } => self.compile_call(callee, args, line),
        Expr::Index { object, index } => self.compile_index(object, index, line),
        Expr::Dot { object, name } => self.compile_dot(object, name, line),
        Expr::Slice { object, start, stop, step } => {
            self.compile_slice(object, start.as_deref(), stop.as_deref(), step.as_deref(), line)
        }
        Expr::ListLiteral { elements } => self.compile_list_literal(elements, line),
        Expr::DictLiteral { pairs } => self.compile_dict_literal(pairs, line),
        Expr::SetLiteral { elements } => self.compile_set_literal(elements, line),
        Expr::TupleLiteral { elements } => self.compile_tuple_literal(elements, line),
        Expr::Grouping { expr } => self.compile_expression(expr, line),
        // 以下类型由后续 task 实现
        Expr::FnLiteral { .. } => Err("fn literal compilation not yet implemented (task 29)".to_string()),
        Expr::ListComprehension { .. } | Expr::DictComprehension { .. }
        | Expr::SetComprehension { .. } | Expr::GeneratorExpression { .. } => {
            Err("comprehension compilation not yet implemented (task 33/34)".to_string())
        }
        Expr::SuperAccess { .. } => Err("super compilation not yet implemented (task 42)".to_string()),
        Expr::Yield { .. } | Expr::YieldFrom { .. } => {
            Err("yield compilation not yet implemented (task 39)".to_string())
        }
        Expr::Await { .. } => Err("await compilation not yet implemented (task 53)".to_string()),
        Expr::Go { .. } => Err("go compilation not yet implemented (task 55)".to_string()),
    }
}
```

### 编译切片

引用 [03-syntax.md](../03-syntax.md) 切片语法：`obj[start:stop:step]`。

```rust
fn compile_slice(
    &mut self,
    object: &Expr,
    start: Option<&Expr>,
    stop: Option<&Expr>,
    step: Option<&Expr>,
    line: usize,
) -> Result<(), String> {
    self.compile_expression(object, line)?;
    // flags 位域：bit 0 = has_start, bit 1 = has_stop, bit 2 = has_step
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
```

### 编译集合字面量

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) BUILD_LIST/BUILD_DICT/BUILD_TUPLE/BUILD_SET 指令。count 为单字节（0-255），超过需分段构建。

```rust
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
```

### chunk 访问器

测试用。Compiler 内部通过 `self.unit.chunk` 访问。

```rust
pub fn chunk(&self) -> &Chunk {
    &self.unit.chunk
}
```

## 验证标准

1. 整数字面量编译为 `CONSTANT idx`
2. 标识符编译为 `LOAD_LOCAL slot` 或 `LOAD_GLOBAL name_idx`
3. 二元运算编译为：左、右、运算符（栈式求值）
4. 赋值编译为：右值计算、`STORE_LOCAL`/`STORE_GLOBAL`
5. 三元表达式编译为条件跳转
6. 函数调用编译为：callee、args、`CALL argc`
7. 逻辑运算实现短路求值

## 测试用例

```ms
# test_compile_expressions.ms
x = 10
y = 20
z = x + y * 3
a = x > 5
b = "yes" if a else "no"
```

预期字节码（简化表示）：

```
0000 CONSTANT     0   10
0003 STORE_GLOBAL 1   "x"
0006 CONSTANT     0   10
0009 LOAD_GLOBAL  1   "x"
0012 CONSTANT     2   20
0015 STORE_GLOBAL 3   "y"
0018 LOAD_GLOBAL  1   "x"
0021 LOAD_GLOBAL  3   "y"
0024 CONSTANT     4   3
0027 MULTIPLY
0028 ADD
0029 STORE_GLOBAL 5   "z"
...
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::node::{Expr, Literal, BinaryOp};

    #[test]
    fn test_compile_int_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::Literal(Literal::Int(42));
        compiler.compile_expression(&expr, 1).unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
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
    fn test_compile_ternary() {
        let mut compiler = Compiler::new();
        let expr = Expr::Ternary {
            condition: Box::new(Expr::Literal(Literal::Bool(true))),
            then_expr: Box::new(Expr::Literal(Literal::String("yes".to_string()))),
            else_expr: Box::new(Expr::Literal(Literal::String("no".to_string()))),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        assert!(code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(code.contains(&(OpCode::Jump as u8)));
    }
}
```
