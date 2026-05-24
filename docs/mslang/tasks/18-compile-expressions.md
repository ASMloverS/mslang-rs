# 表达式编译

## 所属阶段
Phase 2.2b - 字节码编译 + VM 核心

## 前置任务
- 17-compiler-core

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
fn compile_literal(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
    match expr {
        Expr::Int(n) => self.emit_constant(Object::Int(*n), line),
        Expr::Float(f) => self.emit_constant(Object::Float(*f), line),
        Expr::String(s) => self.emit_constant(Object::String(Gc::new(s.clone())), line),
        Expr::Bool(true) => self.emit_byte(OpCode::True as u8, line),
        Expr::Bool(false) => self.emit_byte(OpCode::False as u8, line),
        Expr::Nil => self.emit_byte(OpCode::Nil as u8, line),
        _ => return Err("Not a literal".to_string()),
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
        let name_idx = self.add_constant(Object::String(Gc::new(name.to_string())));
        self.emit_byte(OpCode::LoadGlobal as u8, line);
        self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
    }
    Ok(())
}
```

### 编译二元表达式

```rust
fn compile_binary(&mut self, left: &Expr, op: &BinaryOp, right: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(left)?;
    self.compile_expression(right)?;
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
        _ => return Err(format!("Unsupported binary op: {:?}", op)),
    };
    self.emit_byte(opcode as u8, line);
    Ok(())
}
```

### 编译一元表达式

```rust
fn compile_unary(&mut self, op: &UnaryOp, operand: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(operand)?;
    let opcode = match op {
        UnaryOp::Negate => OpCode::Negate,
        UnaryOp::Not => OpCode::Not,
        UnaryOp::BitNot => OpCode::BitNot,
    };
    self.emit_byte(opcode as u8, line);
    Ok(())
}
```

### 编译比较表达式（支持链式比较）

引用 [03-syntax.md](../03-syntax.md) 链式比较：`1 < x < 10` 等价于 `(1 < x) and (x < 10)`

```rust
fn compile_comparison(&mut self, first: &Expr, comparisons: &[(BinaryOp, Expr)], line: usize) -> Result<(), String> {
    self.compile_expression(first)?;
    if comparisons.len() == 1 {
        let (op, right) = &comparisons[0];
        self.compile_expression(right)?;
        self.emit_byte(self.comparison_opcode(op) as u8, line);
        return Ok(());
    }
    for (i, (op, right)) in comparisons.iter().enumerate() {
        self.compile_expression(right)?;
        let opcode = self.comparison_opcode(op);
        self.emit_byte(opcode as u8, line);
        if i < comparisons.len() - 1 {
            self.emit_byte(OpCode::Dup as u8, line);
            self.emit_byte(OpCode::Swap as u8, line);
        }
    }
    for _ in 1..comparisons.len() {
        self.emit_byte(OpCode::BitAnd as u8, line);
    }
    Ok(())
}
```

### 编译赋值表达式

```rust
fn compile_assignment(&mut self, target: &Expr, value: &Expr, op: &Option<CompoundOp>, line: usize) -> Result<(), String> {
    self.compile_expression(value)?;
    if let Some(_compound) = op {
        // 复合赋值需要先加载当前值再运算
        // x += 5 => x = x + 5
    }
    match target {
        Expr::Identifier(name) => {
            if let Some(slot) = self.resolve_local(name) {
                self.emit_byte(OpCode::StoreLocal as u8, line);
                self.emit_byte(slot as u8, line);
            } else {
                let name_idx = self.add_constant(Object::String(Gc::new(name.to_string())));
                self.emit_byte(OpCode::StoreGlobal as u8, line);
                self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
            }
        }
        Expr::Index { object, index } => {
            self.compile_expression(object)?;
            self.compile_expression(index)?;
            self.emit_byte(OpCode::SetIndex as u8, line);
        }
        Expr::Dot { object, name } => {
            self.compile_expression(object)?;
            let name_idx = self.add_constant(Object::String(Gc::new(name.to_string())));
            self.emit_byte(OpCode::SetAttr as u8, line);
            self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
        }
        _ => return Err("Invalid assignment target".to_string()),
    }
    Ok(())
}
```

### 编译三元表达式

```rust
fn compile_ternary(&mut self, condition: &Expr, then_expr: &Expr, else_expr: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(condition)?;
    let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.compile_expression(then_expr)?;
    let end_jump = self.emit_jump(OpCode::Jump, line);
    self.patch_jump(else_jump);
    self.compile_expression(else_expr)?;
    self.patch_jump(end_jump);
    Ok(())
}
```

### 编译后缀表达式

```rust
fn compile_call(&mut self, callee: &Expr, args: &[Expr], line: usize) -> Result<(), String> {
    self.compile_expression(callee)?;
    for arg in args {
        self.compile_expression(arg)?;
    }
    self.emit_byte(OpCode::Call as u8, line);
    self.emit_byte(args.len() as u8, line);
    Ok(())
}

fn compile_index(&mut self, object: &Expr, index: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(object)?;
    self.compile_expression(index)?;
    self.emit_byte(OpCode::GetIndex as u8, line);
    Ok(())
}

fn compile_dot(&mut self, object: &Expr, name: &str, line: usize) -> Result<(), String> {
    self.compile_expression(object)?;
    let name_idx = self.add_constant(Object::String(Gc::new(name.to_string())));
    self.emit_byte(OpCode::GetAttr as u8, line);
    self.emit_bytes(&(name_idx as u16).to_be_bytes(), line);
    Ok(())
}
```

### 编译逻辑表达式（短路求值）

引用 [02-types.md](../02-types.md)：`and`/`or` 返回实际值（短路），`not` 返回 bool

```rust
fn compile_logical_and(&mut self, left: &Expr, right: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(left)?;
    let end_jump = self.emit_jump(OpCode::JumpIfFalse, line);
    self.emit_byte(OpCode::Pop as u8, line);
    self.compile_expression(right)?;
    self.patch_jump(end_jump);
    Ok(())
}

fn compile_logical_or(&mut self, left: &Expr, right: &Expr, line: usize) -> Result<(), String> {
    self.compile_expression(left)?;
    let end_jump = self.emit_jump(OpCode::JumpIfTrue, line);
    self.emit_byte(OpCode::Pop as u8, line);
    self.compile_expression(right)?;
    self.patch_jump(end_jump);
    Ok(())
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

    #[test]
    fn test_compile_int_literal() {
        let mut compiler = Compiler::new();
        let expr = Expr::Int(42);
        compiler.compile_expression(&expr, 1).unwrap();
        assert_eq!(compiler.chunk().code[0], OpCode::Constant as u8);
    }

    #[test]
    fn test_compile_binary_add() {
        let mut compiler = Compiler::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Int(1)),
            op: BinaryOp::Add,
            right: Box::new(Expr::Int(2)),
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
            condition: Box::new(Expr::Bool(true)),
            then_expr: Box::new(Expr::String("yes".to_string())),
            else_expr: Box::new(Expr::String("no".to_string())),
        };
        compiler.compile_expression(&expr, 1).unwrap();
        let code = &compiler.chunk().code;
        assert!(code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(code.contains(&(OpCode::Jump as u8)));
    }
}
```
