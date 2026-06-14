# 字节码指令集定义

## 所属阶段
Phase 2.1 - 字节码编译 + VM 核心

## 前置任务
- 15-parser-advanced-statements

## 目标

定义 mslang 虚拟机使用的完整字节码指令集（OpCode 枚举），实现指令编码/解码，并编写反汇编器用于调试输出。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) 中的 OpCode 设计：

- **设计原则**：栈式虚拟机，操作数从栈顶弹出，结果压入栈顶
- **指令格式**：1 字节操作码 + 可变长度操作数
- **常量池**：字符串、数字等常量存储在独立的常量池中，通过索引引用

### 指令集完整列表

#### 常量加载（无操作数或 2 字节索引）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CONSTANT` | `idx(2)` | 将常量池[idx]压栈 |
| `NIL` | — | 压入 nil |
| `TRUE` | — | 压入 true |
| `FALSE` | — | 压入 false |

#### 局部变量（1 字节 slot/idx，或 2 字节 name_idx）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `LOAD_LOCAL` | `slot(1)` | 将局部变量[slot]压栈 |
| `STORE_LOCAL` | `slot(1)` | 将栈顶存入局部变量[slot] |
| `LOAD_UPVALUE` | `idx(1)` | 将上值[idx]压栈 |
| `STORE_UPVALUE` | `idx(1)` | 将栈顶存入上值[idx] |
| `LOAD_GLOBAL` | `name_idx(2)` | 将全局变量压栈 |
| `STORE_GLOBAL` | `name_idx(2)` | 将栈顶存入全局变量 |

#### 属性与下标（2 字节 name_idx 或无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `GET_ATTR` | `name_idx(2)` | obj.attr |
| `SET_ATTR` | `name_idx(2)` | obj.attr = val |
| `GET_INDEX` | — | obj[key] |
| `SET_INDEX` | — | obj[key] = val |
| `GET_SLICE` | `flags(1)` | obj[start:stop:step] |

#### 算术运算（无操作数）

| OpCode | 说明 |
|---|---|
| `ADD` | a + b |
| `SUBTRACT` | a - b |
| `MULTIPLY` | a * b |
| `DIVIDE` | a / b |
| `FLOOR_DIV` | a // b |
| `MODULO` | a % b |
| `POWER` | a ** b |
| `NEGATE` | -a |

#### 位运算（无操作数）

| OpCode | 说明 |
|---|---|
| `BIT_AND` | a & b |
| `BIT_OR` | a \| b |
| `BIT_XOR` | a ^ b |
| `BIT_NOT` | ~a |
| `LEFT_SHIFT` | a << b |
| `RIGHT_SHIFT` | a >> b |

#### 比较运算（无操作数）

| OpCode | 说明 |
|---|---|
| `EQUAL` | a == b |
| `NOT_EQUAL` | a != b |
| `LESS` | a < b |
| `GREATER` | a > b |
| `LESS_EQUAL` | a <= b |
| `GREATER_EQUAL` | a >= b |
| `IS` | a is b |
| `IN` | a in b |

#### 逻辑运算（2 字节 offset 或无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `NOT` | — | not a（逻辑取反） |
| `JUMP_IF_FALSE` | `offset(2)` | 为 falsy 则跳转 |
| `JUMP_IF_TRUE` | `offset(2)` | 为 truthy 则跳转 |
| `JUMP` | `offset(2)` | 无条件跳转 |
| `POP` | — | 弹出栈顶 |
| `DUP` | — | 复制栈顶 |

#### 控制流（2 字节 offset）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `JUMP_BACK` | `offset(2)` | 向后跳转（循环用） |
| `BREAK` | `offset(2)` | 跳出循环 |
| `CONTINUE` | `offset(2)` | 跳到循环开头 |

> 跳转偏移量为有符号 16 位整数，相对于当前指令位置。

#### 函数调用（1 字节 argc 或无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CALL` | `argc(1)` | 调用函数（argc 个参数） |
| `RETURN` | — | 从函数返回 |
| `TAIL_CALL` | `argc(1)` | 尾调用（优化） |

#### 闭包（2 字节 func_idx 或无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLOSURE` | `func_idx(2)` | 创建闭包 |
| `CLOSE_UPVALUE` | — | 关闭上值 |

#### 迭代（2 字节 offset 或无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `ITERATOR` | — | 创建迭代器 |
| `FOR_ITER` | `offset(2)` | 迭代下一步，结束则跳转 |
| `YIELD` | — | yield 暂停 |
| `YIELD_FROM` | — | yield from 委托 |
| `CLOSE_GENERATOR` | — | 关闭生成器（注入 GeneratorExit，触发 defer/finally） |

#### 构造器（1 字节 count）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `BUILD_LIST` | `count(1)` | 从栈顶 count 个元素构建 list |
| `BUILD_DICT` | `count(1)` | 从栈顶 count 对元素构建 dict |
| `BUILD_TUPLE` | `count(1)` | 从栈顶 count 个元素构建 tuple |
| `BUILD_SET` | `count(1)` | 从栈顶 count 个元素构建 set |
| `UNPACK` | `count(1)` | 解包序列到栈 |

#### 类与实例

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLASS` | `name_idx(2)` | 创建类 |
| `METHOD` | `name_idx(2)` | 定义方法 |
| `INHERIT` | — | 继承父类 |
| `GET_SUPER` | `name_idx(2)` | 获取父类方法 |
| `INVOKE` | `name_idx(2), argc(1)` | 直接调用方法（优化） |

#### defer（无操作数）

| OpCode | 操作数 | 说明 |
|---|---|---|
| `DEFER` | — | 注册 defer 调用 |
| `EXEC_DEFER` | — | 执行所有 defer（函数返回前） |

#### 异常

| OpCode | 操作数 | 说明 |
|---|---|---|
| `THROW` | — | 抛出异常 |
| `TRY_ENTER` | `handler_offset(2)` | 进入 try 块 |
| `TRY_EXIT` | — | 离开 try 块 |
| `CATCH` | `type_idx(2)` | 捕获异常 |

#### 其他

| OpCode | 操作数 | 说明 |
|---|---|---|
| `ASSERT` | — | 断言 |
| `IMPORT` | `module_idx(2)` | 导入模块 |
| `CHANNEL` | `buffer_size(1)` | 创建 channel（0 = 无缓冲） |
| `SEND` | — | channel 发送 |
| `RECEIVE` | — | channel 接收 |
| `GO` | — | 启动协程 |
| `AWAIT` | — | await Future |
| `HALT` | — | 程序结束 |

## 实现细节

### 文件位置

`src/compiler/opcode.rs`

### OpCode 枚举

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Constant = 0,
    Nil,
    True,
    False,
    LoadLocal,
    StoreLocal,
    LoadUpvalue,
    StoreUpvalue,
    LoadGlobal,
    StoreGlobal,
    GetAttr,
    SetAttr,
    GetIndex,
    SetIndex,
    GetSlice,
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDiv,
    Modulo,
    Power,
    Negate,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    LeftShift,
    RightShift,
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Is,
    In,
    Not,
    JumpIfFalse,
    JumpIfTrue,
    Jump,
    Pop,
    Dup,
    JumpBack,
    Break,
    Continue,
    Call,
    Return,
    TailCall,
    Closure,
    CloseUpvalue,
    Iterator,
    ForIter,
    Yield,
    YieldFrom,
    CloseGenerator,
    BuildList,
    BuildDict,
    BuildTuple,
    BuildSet,
    Unpack,
    Class,
    Method,
    Inherit,
    GetSuper,
    Invoke,
    Defer,
    ExecDefer,
    Throw,
    TryEnter,
    TryExit,
    Catch,
    Assert,
    Import,
    Channel,
    Send,
    Receive,
    Go,
    Await,
    Halt,
}
```

### 操作数编码

每条指令的编码格式为：`[opcode: u8][operands: variable]`

- 无操作数指令：1 字节（仅 opcode）
- 1 字节操作数（slot, argc, count, flags）：2 字节
- 2 字节操作数（idx, offset, name_idx）：3 字节（opcode + 2 字节大端序操作数）
- `INVOKE` 特殊：4 字节（opcode + 2 字节 name_idx + 1 字节 argc）

```rust
impl OpCode {
    pub fn operand_size(&self) -> usize {
        match self {
            Self::Constant
            | Self::LoadGlobal | Self::StoreGlobal
            | Self::GetAttr | Self::SetAttr
            | Self::JumpIfFalse | Self::JumpIfTrue | Self::Jump
            | Self::JumpBack | Self::Break | Self::Continue
            | Self::Closure
            | Self::ForIter
            | Self::Class | Self::Method | Self::GetSuper
            | Self::TryEnter | Self::Catch
            | Self::Import => 2,

            Self::LoadLocal | Self::StoreLocal
            | Self::LoadUpvalue | Self::StoreUpvalue
            | Self::GetSlice
            | Self::Call | Self::TailCall
            | Self::BuildList | Self::BuildDict | Self::BuildTuple | Self::BuildSet
            | Self::Unpack
            | Self::Channel => 1,

            Self::Invoke => 3,

            _ => 0,
        }
    }
}
```

### 反汇编器

反汇编器输出格式参照 [11-bytecode-vm.md](../11-bytecode-vm.md) 调试信息：

```
== main.ms ==
0000 CONSTANT     0   "hello"
0002 CONSTANT     1   "world"
0004 ADD
0005 HALT
```

```rust
pub fn disassemble(unit: &CompilationUnit, name: &str) {
    println!("== {} ==", name);
    let mut offset = 0;
    while offset < unit.code.len() {
        offset = disassemble_instruction(unit, offset);
    }
}

fn disassemble_instruction(unit: &CompilationUnit, offset: usize) -> usize {
    print!("{:04} ", offset);
    let opcode = OpCode::from(unit.code[offset]);
    match opcode.operand_size() {
        0 => {
            println!("{:?}", opcode);
            offset + 1
        }
        1 => {
            let operand = unit.code[offset + 1];
            println!("{:?} {}", opcode, operand);
            offset + 2
        }
        2 => {
            let operand = u16::from_be_bytes([unit.code[offset + 1], unit.code[offset + 2]]);
            let constant_display = match opcode {
                OpCode::Constant => format!(" {:?}", unit.constants[operand as usize]),
                OpCode::LoadGlobal | OpCode::StoreGlobal => format!(" {:?}", unit.constants[operand as usize]),
                _ => String::new(),
            };
            println!("{:?} {}{}", opcode, operand, constant_display);
            offset + 3
        }
        3 => {
            let name_idx = u16::from_be_bytes([unit.code[offset + 1], unit.code[offset + 2]]);
            let argc = unit.code[offset + 3];
            println!("{:?} {} {}", opcode, name_idx, argc);
            offset + 4
        }
        _ => offset + 1,
    }
}
```

### OpCode 与 u8 的转换

```rust
impl OpCode {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Self::try_from(byte).ok()
    }
}

impl TryFrom<u8> for OpCode {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Constant),
            1 => Ok(Self::Nil),
            // ... 依次映射
            _ => Err(()),
        }
    }
}
```

## 验证标准

1. 所有 OpCode 变体有对应的 `u8` 值，且可通过 `from_byte` 反向转换
2. 所有 OpCode 变体有字符串表示（`Debug` trait 可满足）
3. `operand_size()` 对每个 OpCode 返回正确的操作数字节数
4. 反汇编器能正确输出格式化文本
5. 指令编码后再解码可还原原始信息

## 测试用例

无 `.ms` 测试文件，使用 Rust 单元测试验证：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_roundtrip() {
        let mut code = Vec::new();
        code.push(OpCode::Constant as u8);
        code.extend(&42u16.to_be_bytes());
        code.push(OpCode::Add as u8);
        code.push(OpCode::Halt as u8);

        assert_eq!(OpCode::from_byte(code[0]), Some(OpCode::Constant));
        assert_eq!(OpCode::from_byte(code[3]), Some(OpCode::Add));
        assert_eq!(OpCode::from_byte(code[4]), Some(OpCode::Halt));
    }

    #[test]
    fn test_all_opcodes_have_string_repr() {
        let opcodes: Vec<OpCode> = vec![
            OpCode::Constant, OpCode::Nil, OpCode::True, OpCode::False,
            OpCode::LoadLocal, OpCode::StoreLocal,
            OpCode::LoadUpvalue, OpCode::StoreUpvalue,
            OpCode::LoadGlobal, OpCode::StoreGlobal,
            OpCode::GetAttr, OpCode::SetAttr,
            OpCode::GetIndex, OpCode::SetIndex, OpCode::GetSlice,
            OpCode::Add, OpCode::Subtract, OpCode::Multiply, OpCode::Divide,
            OpCode::FloorDiv, OpCode::Modulo, OpCode::Power, OpCode::Negate,
            OpCode::BitAnd, OpCode::BitOr, OpCode::BitXor, OpCode::BitNot,
            OpCode::LeftShift, OpCode::RightShift,
            OpCode::Equal, OpCode::NotEqual,
            OpCode::Less, OpCode::Greater,
            OpCode::LessEqual, OpCode::GreaterEqual,
            OpCode::Is, OpCode::In,
            OpCode::Not, OpCode::JumpIfFalse, OpCode::JumpIfTrue, OpCode::Jump,
            OpCode::Pop, OpCode::Dup,
            OpCode::JumpBack, OpCode::Break, OpCode::Continue,
            OpCode::Call, OpCode::Return, OpCode::TailCall,
            OpCode::Closure, OpCode::CloseUpvalue,
            OpCode::Iterator, OpCode::ForIter, OpCode::Yield, OpCode::YieldFrom,
            OpCode::CloseGenerator,
            OpCode::BuildList, OpCode::BuildDict, OpCode::BuildTuple,
            OpCode::BuildSet, OpCode::Unpack,
            OpCode::Class, OpCode::Method, OpCode::Inherit,
            OpCode::GetSuper, OpCode::Invoke,
            OpCode::Defer, OpCode::ExecDefer,
            OpCode::Throw, OpCode::TryEnter, OpCode::TryExit, OpCode::Catch,
            OpCode::Assert, OpCode::Import,
            OpCode::Channel, OpCode::Send, OpCode::Receive,
            OpCode::Go, OpCode::Await, OpCode::Halt,
        ];
        for op in &opcodes {
            let s = format!("{:?}", op);
            assert!(!s.is_empty(), "OpCode {:?} has no string repr", op);
        }
    }

    #[test]
    fn test_operand_sizes() {
        assert_eq!(OpCode::Constant.operand_size(), 2);
        assert_eq!(OpCode::Nil.operand_size(), 0);
        assert_eq!(OpCode::LoadLocal.operand_size(), 1);
        assert_eq!(OpCode::LoadGlobal.operand_size(), 2);
        assert_eq!(OpCode::Add.operand_size(), 0);
        assert_eq!(OpCode::Jump.operand_size(), 2);
        assert_eq!(OpCode::Call.operand_size(), 1);
        assert_eq!(OpCode::Invoke.operand_size(), 3);
        assert_eq!(OpCode::BuildList.operand_size(), 1);
        assert_eq!(OpCode::Halt.operand_size(), 0);
    }

    #[test]
    fn test_encode_decode_constant() {
        let mut code = Vec::new();
        code.push(OpCode::Constant as u8);
        code.extend(&100u16.to_be_bytes());

        let op = OpCode::from_byte(code[0]).unwrap();
        assert_eq!(op, OpCode::Constant);
        let idx = u16::from_be_bytes([code[1], code[2]]);
        assert_eq!(idx, 100);
    }
}
```
