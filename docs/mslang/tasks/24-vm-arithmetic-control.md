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
| 比较 | `EQUAL`, `NOT_EQUAL`, `LESS`, `GREATER`, `LESS_EQUAL`, `GREATER_EQUAL` |
| 逻辑 | `NOT`, `JUMP_IF_FALSE`, `JUMP_IF_TRUE`, `JUMP` |
| 控制流 | `JUMP_BACK`（while 循环用） |

### 跳转指令语义

引用 [11-bytecode-vm.md](../11-bytecode-vm.md)：跳转偏移量为有符号 16 位整数，相对于当前指令位置。

- `JUMP offset`：无条件向前跳转 offset 字节
- `JUMP_IF_FALSE offset`：栈顶为 falsy 则跳转，否则继续（不弹出栈顶）
- `JUMP_IF_TRUE offset`：栈顶为 truthy 则跳转，否则继续
- `JUMP_BACK offset`：向后跳转 offset 字节（循环回跳）

## 实现细节

### 文件位置

`src/vm/mod.rs`（扩展任务 23 的 `run()` 方法）

### 算术运算指令

```rust
OpCode::Add => {
    let b = self.pop();
    let a = self.pop();
    let result = a.add(&b)?;
    self.push(result);
}

OpCode::Subtract => {
    let b = self.pop();
    let a = self.pop();
    let result = a.subtract(&b)?;
    self.push(result);
}

OpCode::Multiply => {
    let b = self.pop();
    let a = self.pop();
    let result = a.multiply(&b)?;
    self.push(result);
}

OpCode::Divide => {
    let b = self.pop();
    let a = self.pop();
    let result = a.divide(&b)?;
    self.push(result);
}

OpCode::FloorDiv => {
    let b = self.pop();
    let a = self.pop();
    let result = a.floor_divide(&b)?;
    self.push(result);
}

OpCode::Modulo => {
    let b = self.pop();
    let a = self.pop();
    let result = a.modulo(&b)?;
    self.push(result);
}

OpCode::Power => {
    let b = self.pop();
    let a = self.pop();
    let result = a.power(&b)?;
    self.push(result);
}

OpCode::Negate => {
    let value = self.pop();
    let result = value.negate()?;
    self.push(result);
}
```

### 位运算指令

```rust
OpCode::BitAnd => {
    let b = self.pop();
    let a = self.pop();
    let result = a.bit_and(&b)?;
    self.push(result);
}

OpCode::BitOr => {
    let b = self.pop();
    let a = self.pop();
    let result = a.bit_or(&b)?;
    self.push(result);
}

OpCode::BitXor => {
    let b = self.pop();
    let a = self.pop();
    let result = a.bit_xor(&b)?;
    self.push(result);
}

OpCode::BitNot => {
    let value = self.pop();
    let result = value.bit_not()?;
    self.push(result);
}

OpCode::LeftShift => {
    let b = self.pop();
    let a = self.pop();
    let result = a.left_shift(&b)?;
    self.push(result);
}

OpCode::RightShift => {
    let b = self.pop();
    let a = self.pop();
    let result = a.right_shift(&b)?;
    self.push(result);
}
```

### 比较运算指令

```rust
OpCode::Equal => {
    let b = self.pop();
    let a = self.pop();
    self.push(Object::Bool(a == b));
}

OpCode::NotEqual => {
    let b = self.pop();
    let a = self.pop();
    self.push(Object::Bool(a != b));
}

OpCode::Less => {
    let b = self.pop();
    let a = self.pop();
    let result = a.compare(&b, &OpCode::Less)?;
    self.push(result);
}

OpCode::Greater => {
    let b = self.pop();
    let a = self.pop();
    let result = a.compare(&b, &OpCode::Greater)?;
    self.push(result);
}

OpCode::LessEqual => {
    let b = self.pop();
    let a = self.pop();
    let result = a.compare(&b, &OpCode::LessEqual)?;
    self.push(result);
}

OpCode::GreaterEqual => {
    let b = self.pop();
    let a = self.pop();
    let result = a.compare(&b, &OpCode::GreaterEqual)?;
    self.push(result);
}
```

### 逻辑运算指令

引用 [02-types.md](../02-types.md)：`and`/`or` 短路求值返回实际值，`not` 返回 bool。

```rust
OpCode::Not => {
    let value = self.pop();
    self.push(value.logical_not());
}
```

> **注意**：`and`/`or` 的短路求值在编译器阶段通过 `JUMP_IF_FALSE`/`JUMP_IF_TRUE` + `POP` 实现（见任务 18），不需要单独的 AND/OR 指令。

### 跳转指令

```rust
OpCode::Jump => {
    let offset = self.read_u16() as usize;
    let frame = self.frames.last_mut().unwrap();
    frame.ip += offset;
}

OpCode::JumpIfFalse => {
    let offset = self.read_u16() as usize;
    if !self.peek(0).is_truthy() {
        let frame = self.frames.last_mut().unwrap();
        frame.ip += offset;
    }
}

OpCode::JumpIfTrue => {
    let offset = self.read_u16() as usize;
    if self.peek(0).is_truthy() {
        let frame = self.frames.last_mut().unwrap();
        frame.ip += offset;
    }
}

OpCode::JumpBack => {
    let offset = self.read_u16() as usize;
    let frame = self.frames.last_mut().unwrap();
    frame.ip -= offset;
}
```

### 错误处理

所有运算操作可能返回 `Err(String)`，run 循环中使用 `?` 传播错误：

```rust
fn run(&mut self) -> Result<Object, String> {
    loop {
        // ...
        match opcode {
            OpCode::Add => {
                let b = self.pop();
                let a = self.pop();
                let result = a.add(&b)?;  // 错误自动传播
                self.push(result);
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
9. 以下脚本输出正确

## 测试用例

```ms
# test_vm_arithmetic_control.ms
x = 10
y = 20

if x > y {
    print("x is bigger")
} else {
    print("y is bigger")
}

i = 0
result = 0
while i < 5 {
    result += i
    i += 1
}
print(result)
```

预期输出：
```
y is bigger
10
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn run_source(source: &str) -> (VM, Result<Object, String>) {
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
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
