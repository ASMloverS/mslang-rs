pub mod frame;
pub mod object;

use crate::compiler::opcode::OpCode;
use crate::compiler::Chunk;
use crate::gc::GarbageCollector;
use crate::vm::object::{read_str, Object, TypeTag};
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
}
