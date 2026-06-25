pub mod frame;
pub mod object;

use crate::compiler::opcode::OpCode;
use crate::compiler::Chunk;
use crate::gc::GarbageCollector;
use crate::vm::object::{read_str, CmpOp, Object, TypeTag};
use frame::CallFrame;
use std::collections::HashMap;

const STACK_MAX: usize = 1024;

pub struct VM {
    stack: Vec<Object>,
    frames: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    gc: GarbageCollector,
}

impl VM {
    pub fn new() -> Self {
        VM {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::new(),
            globals: HashMap::new(),
            gc: GarbageCollector::new(),
        }
    }

    pub fn interpret(&mut self, chunk: Chunk) -> Result<Object, String> {
        let frame = CallFrame {
            chunk,
            ip: 0,
            stack_base: 0,
        };
        self.frames.push(frame);
        self.run()
    }
}

impl VM {
    fn push(&mut self, value: Object) -> Result<(), String> {
        if self.stack.len() >= STACK_MAX {
            return Err("stack overflow".to_string());
        }
        self.stack.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Object, String> {
        self.stack
            .pop()
            .ok_or_else(|| "stack underflow".to_string())
    }

    fn peek(&self, distance: usize) -> Result<&Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack
            .get(idx)
            .ok_or_else(|| "stack underflow".to_string())
    }

    #[allow(dead_code)]
    fn peek_mut(&mut self, distance: usize) -> Result<&mut Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack
            .get_mut(idx)
            .ok_or_else(|| "stack underflow".to_string())
    }
}

impl VM {
    fn read_byte(&mut self) -> Result<u8, String> {
        let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
        let b = *frame
            .chunk
            .code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let frame = self.frames.last_mut().ok_or("no call frame".to_string())?;
        let lo = *frame
            .chunk
            .code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        let hi = *frame
            .chunk
            .code
            .get(frame.ip + 1)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 2;
        Ok(u16::from_be_bytes([lo, hi]))
    }
}

impl VM {
    fn run(&mut self) -> Result<Object, String> {
        loop {
            // GC 触发点（task 52 接入真实回收；当前 collect 为 no-op）
            if self.gc.should_collect() {
                self.gc.collect();
            }

            let opcode_byte = self.read_byte()?;
            let opcode = OpCode::from_byte(opcode_byte)
                .ok_or_else(|| format!("unknown opcode: {}", opcode_byte))?;

            match opcode {
                OpCode::Constant => {
                    let idx = self.read_u16()? as usize;
                    let frame = self.frames.last().unwrap();
                    let value = frame
                        .chunk
                        .constants
                        .get(idx)
                        .ok_or_else(|| "constant index out of range".to_string())?
                        .clone();
                    self.push(value)?;
                }

                OpCode::Nil => self.push(Object::Nil)?,
                OpCode::True => self.push(Object::Bool(true))?,
                OpCode::False => self.push(Object::Bool(false))?,

                OpCode::LoadLocal => {
                    let slot = self.read_byte()? as usize;
                    let frame = self.frames.last().unwrap();
                    let idx = frame
                        .stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    let value = self
                        .stack
                        .get(idx)
                        .ok_or_else(|| "local slot out of range".to_string())?
                        .clone();
                    self.push(value)?;
                }

                OpCode::StoreLocal => {
                    let slot = self.read_byte()? as usize;
                    let value = self.pop()?;
                    let frame = self.frames.last().unwrap();
                    let idx = frame
                        .stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    *self
                        .stack
                        .get_mut(idx)
                        .ok_or_else(|| "local slot out of range".to_string())? = value;
                }

                OpCode::LoadGlobal => {
                    let name_idx = self.read_u16()? as usize;
                    let frame = self.frames.last().unwrap();
                    let constant = frame
                        .chunk
                        .constants
                        .get(name_idx)
                        .ok_or_else(|| "constant index out of range".to_string())?;
                    let name = match constant {
                        // SAFETY：type_tag 守卫确认常量为 STRING，且由编译器经
                        // alloc_string 分配，生命周期与 Chunk/VM 一致；read_str
                        // 的借用仅用于 to_owned，立即结束。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            debug_assert!(!(*ptr).is_null());
                            unsafe { read_str(*ptr) }.to_owned()
                        }
                        _ => return Err("invalid global name constant".to_string()),
                    };
                    let value = self.globals.get(&name).cloned().unwrap_or(Object::Nil);
                    self.push(value)?;
                }

                OpCode::StoreGlobal => {
                    let name_idx = self.read_u16()? as usize;
                    let value = self.pop()?;
                    let frame = self.frames.last().unwrap();
                    let constant = frame
                        .chunk
                        .constants
                        .get(name_idx)
                        .ok_or_else(|| "constant index out of range".to_string())?;
                    let name = match constant {
                        // SAFETY：同 LoadGlobal。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            debug_assert!(!(*ptr).is_null());
                            unsafe { read_str(*ptr) }.to_owned()
                        }
                        _ => return Err("invalid global name constant".to_string()),
                    };
                    self.globals.insert(name, value);
                }

                OpCode::Pop => {
                    self.pop()?;
                }

                OpCode::Dup => {
                    let value = self.peek(0)?.clone();
                    self.push(value)?;
                }

                OpCode::Halt => return Ok(self.pop().unwrap_or(Object::Nil)),

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

                OpCode::Not => {
                    let value = self.pop()?;
                    self.push(value.logical_not())?;
                }

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

                _ => {
                    return Err(format!("unimplemented opcode: {:?}", opcode));
                }
            }
        }
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::node::Program;
    use crate::compiler::opcode::OpCode;
    use crate::compiler::{Chunk, Compiler};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::object::{alloc_string, Object};

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    fn compile_and_run(source: &str) -> Result<Object, String> {
        let program = parse(source);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.interpret(chunk)
    }

    // 合成字节码测试：直接构造 Chunk 验证单个 opcode 语义，绕开编译器
    // 顶层=全局作用域的已知 bug（spec line 334-338，task 23 既有先例）。
    fn run_chunk(code: Vec<u8>, constants: Vec<Object>) -> Result<Object, String> {
        let mut vm = VM::new();
        vm.interpret(Chunk {
            code,
            constants,
            lines: vec![],
        })
    }

    #[test]
    fn test_empty_program() {
        // 空程序 = 仅 HALT；栈空 → 返回 Nil，不 panic
        assert_eq!(compile_and_run("").unwrap(), Object::Nil);
    }

    #[test]
    fn test_constant_expr_stmt() {
        // 表达式语句 `42`：CONSTANT 加载后 POP 丢弃；HALT 栈空 → Nil
        assert_eq!(compile_and_run("42").unwrap(), Object::Nil);
    }

    #[test]
    fn test_constant_pushes_to_stack() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![OpCode::Constant as u8, 0x00, 0x00, OpCode::Halt as u8],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));
    }

    #[test]
    fn test_nil_true_false() {
        let run = |code: Vec<u8>| {
            let mut vm = VM::new();
            let chunk = Chunk {
                code,
                constants: vec![],
                lines: vec![],
            };
            vm.interpret(chunk).unwrap()
        };
        assert_eq!(
            run(vec![OpCode::Nil as u8, OpCode::Halt as u8]),
            Object::Nil
        );
        assert_eq!(
            run(vec![OpCode::True as u8, OpCode::Halt as u8]),
            Object::Bool(true)
        );
        assert_eq!(
            run(vec![OpCode::False as u8, OpCode::Halt as u8]),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_load_local_store_local() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::True as u8, // 占位 slot 0
                OpCode::True as u8, // 占位 slot 1
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(42)
                OpCode::StoreLocal as u8,
                0x01, // stack[1] = 42
                OpCode::LoadLocal as u8,
                0x01, // push stack[1] = 42
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));
    }

    #[test]
    fn test_global_store_and_load() {
        // 顶层 `x = 10` 经 compile_var_decl 发射 StoreLocal（局部变量），
        // 无法端到端触发全局路径，故合成字节码直接测试 LoadGlobal/StoreGlobal。
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(10)   (const[0])
                OpCode::StoreGlobal as u8,
                0x00,
                0x01, // globals["x"] = 10 (name const[1])
                OpCode::LoadGlobal as u8,
                0x00,
                0x01, // push globals["x"]
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(10), alloc_string("x")],
            lines: vec![],
        };
        let result = vm.interpret(chunk).unwrap();
        assert_eq!(result, Object::Int(10));
        assert_eq!(vm.globals.get("x"), Some(&Object::Int(10)));
    }

    #[test]
    fn test_load_global_missing_returns_nil() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![OpCode::LoadGlobal as u8, 0x00, 0x00, OpCode::Halt as u8],
            constants: vec![alloc_string("undefined")],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Nil);
    }

    #[test]
    fn test_store_global_invalid_name_returns_err() {
        let mut vm = VM::new();
        // name 常量指向 Int（非 STRING）→ Err
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) (const[0])
                OpCode::StoreGlobal as u8,
                0x00,
                0x01, // name const[1]=Int → Err
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(0), Object::Int(1)],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_pop_and_dup() {
        // Dup 复制栈顶
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Dup as u8,
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));

        // Pop 弹出栈顶，露出下方值
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Pop as u8,
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(7)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(7));
    }

    #[test]
    fn test_unknown_opcode_returns_err() {
        let mut vm = VM::new();
        // 人造非法 opcode 字节 0xFF（超出 Halt=79）
        let chunk = Chunk {
            code: vec![0xFF],
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_ip_past_end_returns_err() {
        let mut vm = VM::new();
        // CONSTANT 缺操作数 → read_u16 越界 → Err
        let chunk = Chunk {
            code: vec![OpCode::Constant as u8],
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_stack_overflow_returns_err() {
        let mut vm = VM::new();
        let mut code = Vec::new();
        for _ in 0..(STACK_MAX + 1) {
            code.push(OpCode::True as u8);
        }
        code.push(OpCode::Halt as u8);
        let chunk = Chunk {
            code,
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    // ---- task 24：算术 / 除法 / 取模 / 幂 / 取反 ----

    #[test]
    fn test_arithmetic_add_subtract_multiply() {
        let op = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(op(OpCode::Add as u8, 10, 3), Object::Int(13));
        assert_eq!(op(OpCode::Subtract as u8, 10, 3), Object::Int(7));
        assert_eq!(op(OpCode::Multiply as u8, 10, 3), Object::Int(30));
    }

    #[test]
    fn test_divide_returns_float() {
        // 10 / 3 → Float（真除法总返回 float，02-types.md）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Divide as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(10), Object::Int(3)],
        )
        .unwrap();
        assert!(matches!(result, Object::Float(_)));
    }

    #[test]
    fn test_floor_division_toward_negative_infinity() {
        // -7 // 2 == -4（向负无穷取整）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::FloorDiv as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(-7), Object::Int(2)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(-4));
    }

    #[test]
    fn test_modulo_floor_semantics() {
        // 10 % 3 == 1；-7 % 2 == 1（Python floor-mod，符号跟随除数）
        let m = |a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    OpCode::Modulo as u8,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(m(10, 3), Object::Int(1));
        assert_eq!(m(-7, 2), Object::Int(1));
    }

    #[test]
    fn test_power() {
        // 2 ** 10 == 1024
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Power as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(2), Object::Int(10)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(1024));
    }

    #[test]
    fn test_negate() {
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Negate as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(-5));
    }

    // ---- task 24：比较（含 int/float 交叉） ----

    #[test]
    fn test_comparison_ops() {
        let cmp = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(cmp(OpCode::Less as u8, 3, 5), Object::Bool(true));
        assert_eq!(cmp(OpCode::Greater as u8, 3, 5), Object::Bool(false));
        assert_eq!(cmp(OpCode::LessEqual as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::GreaterEqual as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::Equal as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::NotEqual as u8, 3, 3), Object::Bool(false));
    }

    #[test]
    fn test_comparison_int_float_cross() {
        // Less/Greater 等经 compare 支持跨类型数值比较
        let le = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::LessEqual as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(3), Object::Float(3.0)],
        )
        .unwrap();
        assert_eq!(le, Object::Bool(true));
        let gt = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Greater as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(3), Object::Float(3.0)],
        )
        .unwrap();
        assert_eq!(gt, Object::Bool(false));
    }

    // ---- task 24：位运算（仅 int） ----

    #[test]
    fn test_bitwise_ops() {
        let op2 = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(op2(OpCode::BitAnd as u8, 5, 3), Object::Int(1));
        assert_eq!(op2(OpCode::BitOr as u8, 5, 3), Object::Int(7));
        assert_eq!(op2(OpCode::BitXor as u8, 5, 3), Object::Int(6));
        assert_eq!(op2(OpCode::LeftShift as u8, 1, 2), Object::Int(4));
        assert_eq!(op2(OpCode::RightShift as u8, 4, 1), Object::Int(2));
        let not = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::BitNot as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5)],
        )
        .unwrap();
        assert_eq!(not, Object::Int(-6));
    }

    #[test]
    fn test_bitwise_type_error_on_float() {
        // 位运算仅支持 int；float 操作数 → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::BitAnd as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5), Object::Float(3.0)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_logical_not() {
        let n = |code: Vec<u8>, constants: Vec<Object>| run_chunk(code, constants).unwrap();
        assert_eq!(
            n(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Not as u8,
                    OpCode::Halt as u8
                ],
                vec![Object::Int(0)]
            ),
            Object::Bool(true)
        );
        assert_eq!(
            n(
                vec![OpCode::True as u8, OpCode::Not as u8, OpCode::Halt as u8],
                vec![]
            ),
            Object::Bool(false)
        );
        assert_eq!(
            n(
                vec![OpCode::Nil as u8, OpCode::Not as u8, OpCode::Halt as u8],
                vec![]
            ),
            Object::Bool(true)
        );
    }

    // ---- task 24：身份比较 `is`（Ref↔Ref 比指针；inline 抛 TypeError） ----

    #[test]
    fn test_is_identity_same_pointer() {
        // 同一常量指针 → is 为 true
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("abc")],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(true));
    }

    #[test]
    fn test_is_identity_different_pointer() {
        // 内容相同但两次独立分配 → 不同指针 → is 为 false
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("abc"), alloc_string("abc")],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(false));
    }

    #[test]
    fn test_is_inline_type_error() {
        // inline 类型（int）使用 is → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(2)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：`in`（当前仅 String 子串） ----

    #[test]
    fn test_in_string() {
        // "ell" in "hello" → true；"xyz" in "hello" → false
        let t = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("ell"), alloc_string("hello")],
        )
        .unwrap();
        assert_eq!(t, Object::Bool(true));
        let f = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("xyz"), alloc_string("hello")],
        )
        .unwrap();
        assert_eq!(f, Object::Bool(false));
    }

    #[test]
    fn test_in_string_type_error() {
        // needle 为 int → TypeError（要求 str in str）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), alloc_string("hello")],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：错误路径 ----

    #[test]
    fn test_division_by_zero_error() {
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Divide as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(0)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ZeroDivisionError"));
    }

    #[test]
    fn test_power_overflow_error() {
        // 2 ** 100 → OverflowError（指数 ≥ 64 必溢出）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Power as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(2), Object::Int(100)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("OverflowError"));
    }

    #[test]
    fn test_arithmetic_type_error() {
        // int + nil → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Nil as u8,
                OpCode::Add as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：控制流（端到端，真实 Lexer+Parser+Compiler）----
    // 注：编译器顶层=局部（非全局）的已知 bug 只影响 vm.globals 读取；
    // 局部变量的存取自洽，故 if/while/break/continue 的执行路径可端到端验证。
    // 用「错误注入」使分支/循环选择可观测（错误分支未执行即证明选择正确）。

    #[test]
    fn test_if_else_then_branch() {
        // 3 > 2 为真 → then 分支；else 中 1/0 不执行 → Ok
        let result = compile_and_run("if 3 > 2 { 1 } else { 1/0 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_else_else_branch() {
        // 2 > 3 为假 → else 分支；then 中 1/0 不执行 → Ok
        let result = compile_and_run("if 2 > 3 { 1/0 } else { 1 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_else_error_when_condition_true() {
        // 条件为真且 then 含 1/0 → 必触发 ZeroDivisionError（证明 then 被选中）
        let result = compile_and_run("if 3 > 2 { 1/0 } else { 1 }");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ZeroDivisionError"));
    }

    #[test]
    fn test_while_loop_iterations() {
        // 合成 while 循环：slot 0 为计数器（初始 0），每轮 +1，i<3 为限。
        // 经 JumpBack 回边 3 轮后退出；Halt 弹出 slot 0 的最终值 → Int(3)。
        // 用合成 Chunk 绕开顶层局部槽未预分配的限制（task 23 既有先例）。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 0 占位/初值
                OpCode::LoadLocal as u8,
                0x00, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(3)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x00, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x00, // i = i + 1
                OpCode::JumpBack as u8,
                0x00,
                0x15,              // → loop_start
                OpCode::Pop as u8, // exit: 弹出条件
                OpCode::Halt as u8,
            ],
            vec![Object::Int(0), Object::Int(3), Object::Int(1)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(3));
    }

    #[test]
    fn test_break_end_to_end() {
        // 端到端（无变量，故不受顶层局部槽限制）：while true 体首句 break 即跳出，
        // 跳过后续不可达的 1/0 → Ok。若 break 失效（前向跳转未生效）则 1/0 必触发错误。
        let result = compile_and_run("while true { break\n1/0 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_continue_backward_jump() {
        // 合成循环：与 while 测试同构，但回边用 Continue（checked_sub 后向跳）。
        // 限界 2 → 两轮后退出 → Int(2)，验证 Continue 的后向跳转执行。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 0
                OpCode::LoadLocal as u8,
                0x00, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(2)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x00, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x00, // i = i + 1
                OpCode::Continue as u8,
                0x00,
                0x15,              // → loop_start
                OpCode::Pop as u8, // exit
                OpCode::Halt as u8,
            ],
            vec![Object::Int(0), Object::Int(2), Object::Int(1)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(2));
    }
}
