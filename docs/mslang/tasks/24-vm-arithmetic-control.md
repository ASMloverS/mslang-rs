# VM 算术运算与控制流

## 所属阶段
Phase 2.4b - 字节码编译 + VM 核心

## 前置任务
- 23-vm-core

## 目标

在 VM 核心执行循环中实现算术运算、位运算、比较运算、逻辑运算和基本控制流（if/while）指令的执行。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) 指令集，[02-types.md](../02-types.md) 运算符类型规则，[05-control-flow.md](../05-control-flow.md) 控制流语义。

### 本任务实现的指令

| 类别 | 指令 |
|---|---|
| 算术 | `ADD`, `SUBTRACT`, `MULTIPLY`, `DIVIDE`, `FLOOR_DIV`, `MODULO`, `POWER`, `NEGATE` |
| 位运算 | `BIT_AND`, `BIT_OR`, `BIT_XOR`, `BIT_NOT`, `LEFT_SHIFT`, `RIGHT_SHIFT` |
| 比较 | `EQUAL`, `NOT_EQUAL`, `LESS`, `GREATER`, `LESS_EQUAL`, `GREATER_EQUAL`, `IS`, `IN` |
| 逻辑 | `NOT`, `JUMP_IF_FALSE`, `JUMP_IF_TRUE`, `JUMP` |
| 控制流 | `JUMP_BACK`（while 回边）、`BREAK`（跳出循环）、`CONTINUE`（跳到循环头） |

### 跳转指令语义

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) 指令集。**偏移量编码**：2 字节无符号量值（`u16`，大端），方向由 opcode 决定——前向跳转（`JUMP`/`JUMP_IF_FALSE`/`JUMP_IF_TRUE`/`BREAK`）用 `ip += offset`，后向跳转（`JUMP_BACK`/`CONTINUE`）用 `ip -= offset`。基址为跳转指令之后的那条指令（即 `read_u16` 返回后 `ip` 所指），与编译器 `patch_jump`（`src/compiler/mod.rs:198`，`jump = code_len - offset - 2`）/ `patch_jump_back`（`:215`，`backward = (offset+2) - loop_start`）一致。

> 注：`11-bytecode-vm.md:105` 将偏移量描述为「有符号 16 位」，实际实现为 u16 量值 + opcode 定方向（`opcode.rs` 反汇编器亦按 u16）。该措辞差异建议后续修订 11-bytecode-vm.md，非本 task 阻塞项。

- `JUMP offset`：无条件向前跳转 offset 字节
- `JUMP_IF_FALSE offset`：栈顶为 falsy 则跳转，否则继续（**不弹出栈顶**；编译器在 if/while 条件后显式发射 `POP`，见 `src/compiler/statement.rs:242,288`）
- `JUMP_IF_TRUE offset`：栈顶为 truthy 则跳转，否则继续（不弹出栈顶）
- `JUMP_BACK offset`：向后跳转 offset 字节（while 回边）
- `BREAK offset`：前向跳到循环出口（编译器 `patch_jump` patch 到循环结尾，见 `src/compiler/statement.rs:387-395`）
- `CONTINUE offset`：后向跳到循环头（编译器 `patch_jump_back`，见 `src/compiler/statement.rs:398-407`）

## 实现细节

### 文件位置

`src/vm/mod.rs`（扩展任务 23 的 `run()` 方法）

### 算术运算指令

```rust
OpCode::Add => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.add(&b)?;
    self.push(result)?;
}

OpCode::Subtract => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.subtract(&b)?;
    self.push(result)?;
}

OpCode::Multiply => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.multiply(&b)?;
    self.push(result)?;
}

OpCode::Divide => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.divide(&b)?;
    self.push(result)?;
}

OpCode::FloorDiv => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.floor_divide(&b)?;
    self.push(result)?;
}

OpCode::Modulo => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.modulo(&b)?;
    self.push(result)?;
}

OpCode::Power => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.power(&b)?;
    self.push(result)?;
}

OpCode::Negate => {
    let value = self.pop()?;
    let result = value.negate()?;
    self.push(result)?;
}
```

### 位运算指令

```rust
OpCode::BitAnd => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.bit_and(&b)?;
    self.push(result)?;
}

OpCode::BitOr => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.bit_or(&b)?;
    self.push(result)?;
}

OpCode::BitXor => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.bit_xor(&b)?;
    self.push(result)?;
}

OpCode::BitNot => {
    let value = self.pop()?;
    let result = value.bit_not()?;
    self.push(result)?;
}

OpCode::LeftShift => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.left_shift(&b)?;
    self.push(result)?;
}

OpCode::RightShift => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.right_shift(&b)?;
    self.push(result)?;
}
```

### 比较运算指令

```rust
// CmpOp 与 OpCode 解耦（task 21 设计决策，见 src/vm/object.rs:378）。
// 需 `use crate::vm::object::CmpOp;`（CmpOp 为 Copy，按值传递）。
OpCode::Equal => {
    let b = self.pop()?;
    let a = self.pop()?;
    self.push(Object::Bool(a == b))?;
}

OpCode::NotEqual => {
    let b = self.pop()?;
    let a = self.pop()?;
    self.push(Object::Bool(a != b))?;
}

OpCode::Less => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.compare(&b, CmpOp::Less)?;
    self.push(result)?;
}

OpCode::Greater => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.compare(&b, CmpOp::Greater)?;
    self.push(result)?;
}

OpCode::LessEqual => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.compare(&b, CmpOp::LessEqual)?;
    self.push(result)?;
}

OpCode::GreaterEqual => {
    let b = self.pop()?;
    let a = self.pop()?;
    let result = a.compare(&b, CmpOp::GreaterEqual)?;
    self.push(result)?;
}
```

### 逻辑运算指令

引用 [02-types.md](../02-types.md)：`and`/`or` 短路求值返回实际值，`not` 返回 bool。

```rust
OpCode::Not => {
    let value = self.pop()?;
    self.push(value.logical_not())?;
}
```

> **注意**：`and`/`or` 的短路求值在编译器阶段通过 `JUMP_IF_FALSE`/`JUMP_IF_TRUE` + `POP` 实现（见任务 18），不需要单独的 AND/OR 指令。

### 跳转指令

```rust
OpCode::Jump => {
    let offset = self.read_u16()? as usize;
    let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
    frame.ip += offset;
}

OpCode::JumpIfFalse => {
    let offset = self.read_u16()? as usize;
    if !self.peek(0)?.is_truthy() {
        let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
        frame.ip += offset;
    }
}

OpCode::JumpIfTrue => {
    let offset = self.read_u16()? as usize;
    if self.peek(0)?.is_truthy() {
        let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
        frame.ip += offset;
    }
}

OpCode::JumpBack => {
    let offset = self.read_u16()? as usize;
    let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
    frame.ip = frame
        .ip
        .checked_sub(offset)
        .ok_or_else(|| "jump back underflow".to_string())?;
}

// BREAK：前向跳到循环出口（编译器 patch_jump）
OpCode::Break => {
    let offset = self.read_u16()? as usize;
    let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
    frame.ip += offset;
}

// CONTINUE：后向跳到循环头（编译器 patch_jump_back）
OpCode::Continue => {
    let offset = self.read_u16()? as usize;
    let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
    frame.ip = frame
        .ip
        .checked_sub(offset)
        .ok_or_else(|| "continue underflow".to_string())?;
}
```

### 身份比较指令

引用 [02-types.md](../02-types.md) § `is` 运算符：仅适用于引用类型（list, dict, set, class instance, string, function, module）。对内联值（int, float, bool, nil）使用 `is` 抛出 `TypeError`。

> `is_identity`（`src/vm/object.rs:727`）**已内置** inline 类型的 `TypeError` 检查（无须在 VM 侧重复判断），返回 `Result<Object, String>`（`Object::Bool`）。

```rust
OpCode::Is => {
    let b = self.pop()?;
    let a = self.pop()?;
    self.push(a.is_identity(&b)?)?;
}

OpCode::In => {
    let b = self.pop()?;
    let a = self.pop()?;
    // 当前仅支持 String 子串判断（contains_str，src/vm/object.rs:744）。
    // List/Dict/Set 的成员判断由 task 22 扩展（或 task 26 容器函数）补全。
    let result = b.contains_str(&a)?;
    self.push(result)?;
}
```

### 错误处理

所有运算操作可能返回 `Err(String)`，run 循环中使用 `?` 传播错误（栈操作 `pop`/`push`/`peek`、读取 `read_u16` 同样返回 `Result`，见 task 23）：

```rust
fn run(&mut self) -> Result<Object, String> {
    loop {
        // ...
        match opcode {
            OpCode::Add => {
                let b = self.pop()?;
                let a = self.pop()?;
                let result = a.add(&b)?;  // 错误自动传播
                self.push(result)?;
            }
            // ...
        }
    }
}
```

运行时错误示例：

```
TypeError: unsupported operand type(s) for +: 'int' and 'string'
ZeroDivisionError: division by zero
```

## 验证标准

1. 算术运算产生正确类型和值的结果
2. `10 / 3` 返回 float，`10 // 3` 返回 int
3. `-7 // 2 == -4`（向负无穷取整）
4. `2 ** 10 == 1024`
5. 比较运算正确处理 int/float 交叉比较
6. 位运算仅 int 支持，其他类型抛出 TypeError
7. if/else 控制流正确执行
8. while 循环正确执行和退出
9. break/continue 正确跳出/跳到循环头
10. `is` 对引用类型比较指针，对 inline 类型抛 TypeError；`in` 支持 String 子串
11. 错误路径：除零 → `ZeroDivisionError`；整数溢出 → `OverflowError`；类型不匹配 → `TypeError`

> 注：端到端 `.ms` 脚本（含 `print`）依赖 task 25（内置）/task 27（CALL），不在本 task 范围。本 task 以「Rust 单元测试」为准（见下）。

## 测试用例

> 端到端 `.ms` 脚本（如 `z = x + y; print(z)`、含 `print` 的 if/while）依赖 task 24 尚未具备的 `print` 内置（task 25）与 `CALL`（task 27），**不在本 task 范围**。本 task 的可执行验证以「Rust 单元测试」为准（见下）。

### Rust 单元测试

> **⚠️ 前置依赖（编译器 bug）**：下列端到端测试检查 `vm.globals`，这**符合设计**（`03-syntax.md:593`「顶层是全局作用域」、task 19 spec `:414`「顶层脚本的 `=` 创建/更新全局」）。但已实现的解析器 `src/parser/statement.rs:77-83` 将语句级 `name = expr` 一律转为 `Stmt::VarDecl` → `StoreLocal`（local），**未实现顶层=全局作用域规则**。故这些测试在编译器修复前会失败（`vm.globals.get(...)` 为 `None`）。
>
> **实现期建议**：(1) 先修编译器（`parse_statement`/`compile_var_decl`：顶层作用域 `=` → `StoreGlobal`）；或 (2) 本 task 的 opcode 验收改用**合成 Chunk**（push 操作数 → 执行 opcode → 检查返回值/栈顶），与 task 23 一致，绕开该依赖。
>
> 另注：无公开 `parse()` 函数；测试 helper 须用 `Lexer` + `Parser`（见 `src/compiler/statement.rs:418-427` 的既有模式）。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::Compiler;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run_source(source: &str) -> (VM, Result<Object, String>) {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        let ast = Parser::new(tokens).parse().unwrap();
        let chunk = Compiler::new().compile(&ast).unwrap();
        let mut vm = VM::new();
        let result = vm.interpret(chunk);
        (vm, result)
    }

    #[test]
    fn test_arithmetic() {
        let (vm, result) = run_source("z = 10 + 3 * 2");
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(16)));
    }

    #[test]
    fn test_division_returns_float() {
        let (vm, result) = run_source("z = 10 / 3");
        assert!(result.is_ok());
        assert!(matches!(vm.globals.get("z"), Some(Object::Float(_))));
    }

    #[test]
    fn test_floor_division() {
        let (vm, result) = run_source("z = -7 // 2");
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(-4)));
    }

    #[test]
    fn test_if_else() {
        let source = r#"
            x = 10
            if x > 5 {
                y = 1
            } else {
                y = 2
            }
        "#;
        let (vm, result) = run_source(source);
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("y"), Some(&Object::Int(1)));
    }

    #[test]
    fn test_while_loop() {
        let source = r#"
            i = 0
            sum = 0
            while i < 5 {
                sum += i
                i += 1
            }
        "#;
        let (vm, result) = run_source(source);
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("sum"), Some(&Object::Int(10)));
    }

    #[test]
    fn test_comparison() {
        let (vm, result) = run_source("z = 3 > 2");
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("z"), Some(&Object::Bool(true)));
    }

    #[test]
    fn test_bitwise() {
        let (vm, result) = run_source("z = 5 & 3");
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(1)));
    }

    #[test]
    fn test_power() {
        let (vm, result) = run_source("z = 2 ** 10");
        assert!(result.is_ok());
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(1024)));
    }
}
```
