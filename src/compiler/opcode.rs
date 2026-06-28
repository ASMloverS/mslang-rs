//! mslang 字节码指令集（OpCode）与反汇编器。
//!
//! 参照 [11-bytecode-vm](../../../docs/mslang/11-bytecode-vm.md) 的 OpCode 设计：
//! - 栈式虚拟机：操作数从栈顶弹出，结果压入栈顶
//! - 指令格式：1 字节操作码 + 可变长度操作数
//! - 常量池：字符串、数字等常量存储在独立的常量池中，通过索引引用

use crate::compiler::Chunk;

/// 字节码操作码。
///
/// `#[repr(u8)]` 且判别值从 0 连续递增至 `Halt`(79)，
/// 因此可通过 `transmute` 与 `u8` 之间安全转换（见 [`OpCode::from_byte`]）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    // 常量加载
    Constant = 0,
    Nil,
    True,
    False,
    // 局部变量
    LoadLocal,
    StoreLocal,
    LoadUpvalue,
    StoreUpvalue,
    LoadGlobal,
    StoreGlobal,
    // 属性与下标
    GetAttr,
    SetAttr,
    GetIndex,
    SetIndex,
    GetSlice,
    // 算术运算
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDiv,
    Modulo,
    Power,
    Negate,
    // 位运算
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    LeftShift,
    RightShift,
    // 比较运算
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Is,
    In,
    // 逻辑运算
    Not,
    JumpIfFalse,
    JumpIfTrue,
    Jump,
    Pop,
    Dup,
    // 控制流
    JumpBack,
    Break,
    Continue,
    // 函数调用
    Call,
    Return,
    TailCall,
    // 闭包
    Closure,
    CloseUpvalue,
    // 迭代
    Iterator,
    ForIter,
    Yield,
    YieldFrom,
    CloseGenerator,
    // 构造器
    BuildList,
    BuildDict,
    BuildTuple,
    BuildSet,
    Unpack,
    // 类与实例
    Class,
    Method,
    Inherit,
    GetSuper,
    Invoke,
    // defer
    Defer,
    ExecDefer,
    // 异常
    Throw,
    TryEnter,
    TryExit,
    Catch,
    // 其他
    Assert,
    Import,
    Channel,
    Send,
    Receive,
    Go,
    Await,
    Halt,
}

impl OpCode {
    /// 该指令操作数占用的字节数。
    ///
    /// - `0`：无操作数指令（仅 1 字节 opcode）
    /// - `1`：`slot` / `argc` / `count` / `flags` / `buffer_size`
    /// - `2`：`idx` / `offset` / `name_idx`（大端序）
    /// - `3`：`INVOKE` 特殊（2 字节 `name_idx` + 1 字节 `argc`）
    pub fn operand_size(&self) -> usize {
        match self {
            // 2 字节操作数：idx / offset / name_idx
            Self::Constant
            | Self::LoadGlobal
            | Self::StoreGlobal
            | Self::GetAttr
            | Self::SetAttr
            | Self::JumpIfFalse
            | Self::JumpIfTrue
            | Self::Jump
            | Self::JumpBack
            | Self::Break
            | Self::Continue
            | Self::Closure
            | Self::Class
            | Self::Method
            | Self::GetSuper
            | Self::TryEnter
            | Self::Catch
            | Self::Import => 2,

            // FOR_ITER 特殊：iter_slot(1) + exit_offset(2)。
            // 迭代器存储在局部 slot 中（非栈顶），使嵌套 for..in 不冲突。
            Self::ForIter => 3,

            // 1 字节操作数：slot / argc / count / flags / buffer_size
            Self::LoadLocal
            | Self::StoreLocal
            | Self::LoadUpvalue
            | Self::StoreUpvalue
            | Self::GetSlice
            | Self::Call
            | Self::TailCall
            | Self::BuildList
            | Self::BuildDict
            | Self::BuildTuple
            | Self::BuildSet
            | Self::Unpack
            | Self::Channel => 1,

            // INVOKE 特殊：name_idx(2) + argc(1)
            Self::Invoke => 3,

            // 无操作数
            _ => 0,
        }
    }

    /// 将字节反向转换为 [`OpCode`]。
    pub fn from_byte(byte: u8) -> Option<Self> {
        Self::try_from(byte).ok()
    }
}

impl TryFrom<u8> for OpCode {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= Self::Halt as u8 {
            // SAFETY: OpCode is #[repr(u8)] with sequential discriminants
            // Constant=0 through Halt=79. value <= Halt guarantees validity.
            Ok(unsafe { core::mem::transmute::<u8, Self>(value) })
        } else {
            Err(())
        }
    }
}

/// 将字节码反汇编到标准输出（调试用）。
pub fn disassemble(chunk: &Chunk, name: &str) {
    println!("== {} ==", name);
    let mut offset = 0;
    while offset < chunk.code.len() {
        offset = disassemble_instruction(chunk, offset);
    }
}

/// 反汇编偏移量 `offset` 处的单条指令并打印，返回下一条指令的偏移量。
fn disassemble_instruction(chunk: &Chunk, offset: usize) -> usize {
    let (line, next) = format_instruction(chunk, offset);
    println!("{}", line);
    next
}

/// 格式化偏移量 `offset` 处的单条指令为一行文本，
/// 返回 `(该行文本, 下一条指令偏移量)`。
fn format_instruction(chunk: &Chunk, offset: usize) -> (String, usize) {
    let opcode = OpCode::from_byte(chunk.code[offset]).expect("invalid opcode in bytecode");
    match opcode.operand_size() {
        0 => (format!("{:04} {:?}", offset, opcode), offset + 1),
        1 => {
            let operand = chunk.code[offset + 1];
            (
                format!("{:04} {:?} {}", offset, opcode, operand),
                offset + 2,
            )
        }
        2 => {
            let operand = u16::from_be_bytes([chunk.code[offset + 1], chunk.code[offset + 2]]);
            // 带常量池索引的指令额外显示常量内容（如 CONSTANT / 全局变量名）
            let constant_display = match opcode {
                OpCode::Constant | OpCode::LoadGlobal | OpCode::StoreGlobal => {
                    format!(" {}", chunk.constants[operand as usize])
                }
                _ => String::new(),
            };
            (
                format!("{:04} {:?} {}{}", offset, opcode, operand, constant_display),
                offset + 3,
            )
        }
        3 => {
            if opcode == OpCode::ForIter {
                // FOR_ITER: iter_slot(1) + exit_offset(2)
                let iter_slot = chunk.code[offset + 1];
                let exit_offset =
                    u16::from_be_bytes([chunk.code[offset + 2], chunk.code[offset + 3]]);
                (
                    format!(
                        "{:04} {:?} slot={} offset={}",
                        offset, opcode, iter_slot, exit_offset
                    ),
                    offset + 4,
                )
            } else {
                // INVOKE: name_idx(2) + argc(1)
                let name_idx = u16::from_be_bytes([chunk.code[offset + 1], chunk.code[offset + 2]]);
                let argc = chunk.code[offset + 3];
                (
                    format!("{:04} {:?} {} {}", offset, opcode, name_idx, argc),
                    offset + 4,
                )
            }
        }
        _ => (format!("{:04} {:?}", offset, opcode), offset + 1),
    }
}

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
            OpCode::Constant,
            OpCode::Nil,
            OpCode::True,
            OpCode::False,
            OpCode::LoadLocal,
            OpCode::StoreLocal,
            OpCode::LoadUpvalue,
            OpCode::StoreUpvalue,
            OpCode::LoadGlobal,
            OpCode::StoreGlobal,
            OpCode::GetAttr,
            OpCode::SetAttr,
            OpCode::GetIndex,
            OpCode::SetIndex,
            OpCode::GetSlice,
            OpCode::Add,
            OpCode::Subtract,
            OpCode::Multiply,
            OpCode::Divide,
            OpCode::FloorDiv,
            OpCode::Modulo,
            OpCode::Power,
            OpCode::Negate,
            OpCode::BitAnd,
            OpCode::BitOr,
            OpCode::BitXor,
            OpCode::BitNot,
            OpCode::LeftShift,
            OpCode::RightShift,
            OpCode::Equal,
            OpCode::NotEqual,
            OpCode::Less,
            OpCode::Greater,
            OpCode::LessEqual,
            OpCode::GreaterEqual,
            OpCode::Is,
            OpCode::In,
            OpCode::Not,
            OpCode::JumpIfFalse,
            OpCode::JumpIfTrue,
            OpCode::Jump,
            OpCode::Pop,
            OpCode::Dup,
            OpCode::JumpBack,
            OpCode::Break,
            OpCode::Continue,
            OpCode::Call,
            OpCode::Return,
            OpCode::TailCall,
            OpCode::Closure,
            OpCode::CloseUpvalue,
            OpCode::Iterator,
            OpCode::ForIter,
            OpCode::Yield,
            OpCode::YieldFrom,
            OpCode::CloseGenerator,
            OpCode::BuildList,
            OpCode::BuildDict,
            OpCode::BuildTuple,
            OpCode::BuildSet,
            OpCode::Unpack,
            OpCode::Class,
            OpCode::Method,
            OpCode::Inherit,
            OpCode::GetSuper,
            OpCode::Invoke,
            OpCode::Defer,
            OpCode::ExecDefer,
            OpCode::Throw,
            OpCode::TryEnter,
            OpCode::TryExit,
            OpCode::Catch,
            OpCode::Assert,
            OpCode::Import,
            OpCode::Channel,
            OpCode::Send,
            OpCode::Receive,
            OpCode::Go,
            OpCode::Await,
            OpCode::Halt,
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

    #[test]
    fn test_all_opcodes_byte_roundtrip() {
        // 0..=Halt(79) 每个字节都应能往返转换
        for b in 0u8..=OpCode::Halt as u8 {
            let op =
                OpCode::from_byte(b).unwrap_or_else(|| panic!("from_byte({}) should succeed", b));
            assert_eq!(op as u8, b);
        }
        // 超出范围的字节应转换失败
        for b in (OpCode::Halt as u8 + 1)..=u8::MAX {
            assert_eq!(
                OpCode::from_byte(b),
                None,
                "from_byte({}) should be None",
                b
            );
        }
    }

    #[test]
    fn test_disassemble() {
        use crate::vm::object::Object;
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // CONSTANT 0
                OpCode::Constant as u8,
                0x00,
                0x01, // CONSTANT 1
                OpCode::LoadLocal as u8,
                0x02, // LOAD_LOCAL 2
                OpCode::Invoke as u8,
                0x00,
                0x05,
                0x01,               // INVOKE 5 1
                OpCode::Add as u8,  // ADD
                OpCode::Halt as u8, // HALT
            ],
            constants: vec![Object::Int(42), Object::Int(99)],
            lines: vec![],
        };

        let (line, next) = format_instruction(&chunk, 0);
        assert_eq!(line, "0000 Constant 0 42");
        assert_eq!(next, 3);

        let (line, next) = format_instruction(&chunk, 3);
        assert_eq!(line, "0003 Constant 1 99");
        assert_eq!(next, 6);

        let (line, next) = format_instruction(&chunk, 6);
        assert_eq!(line, "0006 LoadLocal 2");
        assert_eq!(next, 8);

        let (line, next) = format_instruction(&chunk, 8);
        assert_eq!(line, "0008 Invoke 5 1");
        assert_eq!(next, 12);

        let (line, next) = format_instruction(&chunk, 12);
        assert_eq!(line, "0012 Add");
        assert_eq!(next, 13);

        let (line, next) = format_instruction(&chunk, 13);
        assert_eq!(line, "0013 Halt");
        assert_eq!(next, 14);
    }
}
