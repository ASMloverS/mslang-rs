pub mod builtins;
pub mod frame;
pub mod gc;
pub mod object;

use crate::compiler::opcode::OpCode;
use crate::compiler::Chunk;
use crate::vm::builtins::{read_native_function, to_iterator};
use crate::vm::object::{
    alloc_closure, alloc_function, alloc_iterator, alloc_tuple, alloc_upvalue, read_closure,
    read_function, read_iterator, read_list, read_str, read_tuple, read_upvalue, CmpOp, Function,
    MsObjHeader, MsUpvalue, Object, TypeTag,
};
use frame::CallFrame;
use std::collections::HashMap;

const STACK_MAX: usize = 1024;
/// 调用栈最大深度（对齐 Python 默认；task 28/31/36/37/70 共用此常量）。
pub const MAX_CALL_DEPTH: usize = 1000;

pub struct VM {
    stack: Vec<Object>,
    call_stack: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    /// 内置函数参数个数表（`usize::MAX` = 可变参数），供 CALL 校验。
    native_arities: HashMap<String, usize>,
    /// 开放上值列表（task 28）。每项指向 MsUpvalue（TypeTag::UPVALUE）。
    /// **不变量**：按 `location` **升序**维护（最小在前）。
    /// `close_upvalues_from` 从末尾向前扫描，依赖此序保证正确性（见 §6/§8）。
    open_upvalues: Vec<*mut MsObjHeader>,
    /// GC 堆（task 52）。MVP 经 `gc::maybe_gc` 在主循环触发；当前 VM 日常分配
    /// （`object.rs`/`builtins.rs` 的 `alloc_*`）尚未接入 GC 堆，故 GC 保持 dormant。
    heap: gc::MsHeap,
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            // 预留 slot 0（callee 占位），修复 task 26 发现的「顶层预留 slot 0
            // 但 VM 栈未预分配 → StoreLocal 1 越界」bug（订正 A3）。
            stack: vec![Object::Nil],
            call_stack: Vec::new(),
            globals: HashMap::new(),
            native_arities: HashMap::new(),
            open_upvalues: Vec::new(),
            heap: gc::MsHeap::new(),
        };
        vm.register_builtins();
        vm
    }

    pub fn interpret(&mut self, chunk: Chunk) -> Result<Object, String> {
        let function = Function {
            name: "<main>".to_string(),
            arity: 0,
            code: chunk.code,
            constants: chunk.constants,
            upvalue_count: 0,
            source_file: None,
        };
        let Object::Ref(closure_ptr) = alloc_closure(alloc_function(function), Vec::new()) else {
            unreachable!()
        };
        self.call_stack.push(CallFrame::new(closure_ptr, 0));
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
        let frame = self
            .call_stack
            .last_mut()
            .ok_or("no call frame".to_string())?;
        // SAFETY：frame.closure 由 alloc_closure 分配（CLOSURE），closure.function
        // 指向 alloc_function 分配的 MsFunction，在帧生命周期内有效。
        let code = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function).function.code.as_slice()
        };
        let b = *code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let frame = self
            .call_stack
            .last_mut()
            .ok_or("no call frame".to_string())?;
        let code = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function).function.code.as_slice()
        };
        let lo = *code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        let hi = *code
            .get(frame.ip + 1)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 2;
        Ok(u16::from_be_bytes([lo, hi]))
    }

    /// 读取当前帧的常量池中的常量（克隆）。供 CONSTANT/LoadGlobal/StoreGlobal 使用。
    fn read_constant(&self, idx: usize) -> Result<Object, String> {
        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
        // SAFETY：同 read_byte，closure→MsFunction 在帧生命周期内有效。
        let constants = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function)
                .function
                .constants
                .as_slice()
        };
        constants
            .get(idx)
            .ok_or_else(|| "constant index out of range".to_string())
            .cloned()
    }
}

impl VM {
    /// 捕获（或复用）指向栈槽 `location` 的开放上值，返回 `*mut MsObjHeader` (MsUpvalue)。
    /// 插入时维持 `open_upvalues` 按 `location` **升序**（最小在前）。
    ///
    /// 升序不变量是 `close_upvalues_from` 正确性的前提：它从末尾（最大 location）
    /// 向前扫描，遇 `location < last` 即 break。若非升序，close 会提前中断或遗漏。
    fn capture_upvalue(&mut self, location: usize) -> *mut MsObjHeader {
        // 升序表中，第一个 location >= 新 location 的位置即插入点。
        let insert_at = self.open_upvalues.iter().position(|&ptr| {
            // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let loc = unsafe { (*(ptr as *const MsUpvalue)).location };
            loc >= location
        });

        if let Some(i) = insert_at {
            let existing = self.open_upvalues[i];
            // SAFETY: existing 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let loc = unsafe { (*(existing as *const MsUpvalue)).location };
            if loc == location {
                return existing; // 复用已存在的开放上值（多个闭包共享同一变量）
            }
            // 插入新上值于 i（保持升序：新 location < existing[i].location）
            let Object::Ref(ptr) = alloc_upvalue(location) else {
                unreachable!()
            };
            self.open_upvalues.insert(i, ptr);
            return ptr;
        }

        // 新 location 大于所有现存上值 → 追加末尾（升序保持）
        let Object::Ref(ptr) = alloc_upvalue(location) else {
            unreachable!()
        };
        self.open_upvalues.push(ptr);
        ptr
    }

    /// 关闭所有 `location >= last` 的开放上值：将栈槽当前值拷贝到 `closed`。
    /// 依赖 `open_upvalues` 按 location 升序：从末尾（最大 location）向前扫，
    /// 遇 `location < last` 即停止（升序保证其前所有 location 更小，确属作用域外）。
    ///
    /// **必须在栈截断前调用**（`close` 读取 `stack[location]`，截断后越界）。
    fn close_upvalues_from(&mut self, last: usize) {
        let mut i = self.open_upvalues.len();
        while i > 0 {
            i -= 1;
            let ptr = self.open_upvalues[i];
            // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let location = unsafe { (*(ptr as *const MsUpvalue)).location };
            if location < last {
                break;
            }
            // SAFETY: ptr 指向有效 MsUpvalue；close 仅读写其 closed 字段与（借）栈。
            unsafe {
                read_upvalue(ptr).close(&self.stack);
            }
            self.open_upvalues.remove(i);
        }
    }
}

impl VM {
    fn run(&mut self) -> Result<Object, String> {
        loop {
            // GC 触发点（task 52）。MVP：VM 日常分配未接入 GC 堆，bytes_allocated 保持
            // 0，此调用为 no-op；接入后在此按阈值触发 minor/major GC（STW）。
            gc::maybe_gc(&mut self.heap, &mut self.stack, &mut self.globals);

            let opcode_byte = self.read_byte()?;
            let opcode = OpCode::from_byte(opcode_byte)
                .ok_or_else(|| format!("unknown opcode: {}", opcode_byte))?;

            match opcode {
                OpCode::Constant => {
                    let idx = self.read_u16()? as usize;
                    let value = self.read_constant(idx)?;
                    self.push(value)?;
                }

                OpCode::Nil => self.push(Object::Nil)?,
                OpCode::True => self.push(Object::Bool(true))?,
                OpCode::False => self.push(Object::Bool(false))?,

                OpCode::LoadLocal => {
                    let slot = self.read_byte()? as usize;
                    let frame = self.call_stack.last().unwrap();
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
                    let frame = self.call_stack.last().unwrap();
                    let idx = frame
                        .stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    // task 28：新局部变量声明时（VarDecl/compile_var_decl），slot 尚不存在
                    // 于栈上（StoreLocal 先 pop 再写）。以 Nil 填充至 idx 保证写入成功。
                    // 已有 slot（赋值/复合赋值）不受影响：while 条件为 false，直接覆写。
                    while self.stack.len() <= idx {
                        self.stack.push(Object::Nil);
                    }
                    self.stack[idx] = value;
                }

                OpCode::LoadGlobal => {
                    let name_idx = self.read_u16()? as usize;
                    let constant = self.read_constant(name_idx)?;
                    let name = match &constant {
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
                    let constant = self.read_constant(name_idx)?;
                    let name = match &constant {
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
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip += offset;
                }

                OpCode::JumpIfFalse => {
                    let offset = self.read_u16()? as usize;
                    if !self.peek(0)?.is_truthy() {
                        let frame = self
                            .call_stack
                            .last_mut()
                            .ok_or("no call frame".to_string())?;
                        frame.ip += offset;
                    }
                }

                OpCode::JumpIfTrue => {
                    let offset = self.read_u16()? as usize;
                    if self.peek(0)?.is_truthy() {
                        let frame = self
                            .call_stack
                            .last_mut()
                            .ok_or("no call frame".to_string())?;
                        frame.ip += offset;
                    }
                }

                OpCode::JumpBack => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip = frame
                        .ip
                        .checked_sub(offset)
                        .ok_or_else(|| "jump back underflow".to_string())?;
                }

                // BREAK：前向跳到循环出口（编译器 patch_jump）
                OpCode::Break => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip += offset;
                }

                // CONTINUE：后向跳到循环头（编译器 patch_jump_back）
                OpCode::Continue => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip = frame
                        .ip
                        .checked_sub(offset)
                        .ok_or_else(|| "continue underflow".to_string())?;
                }

                // ITERATOR（task 26）：弹出可迭代对象，压入其迭代器。
                // 编译器在 for..in 头部发射（statement.rs:329）。
                OpCode::Iterator => {
                    let iterable = self.pop()?;
                    let iter_state =
                        to_iterator(&iterable).map_err(|e| format!("RuntimeError: {}", e))?;
                    self.push(alloc_iterator(iter_state))?;
                }

                // FOR_ITER（task 26）：迭代器常驻栈顶；取下一值压入栈顶之上供
                // StoreLocal/Unpack 消费。耗尽时 ip += offset 跳到循环出口
                // （offset 为相对「操作数后一字节」的前向偏移，与 patch_jump 同口径）。
                OpCode::ForIter => {
                    let offset = self.read_u16()? as usize;
                    // 先取出 next 值并结束 &mut stack 借用，再 push，避免借用冲突。
                    let next_val: Option<Object> = {
                        let top = self.stack.len() - 1;
                        match &mut self.stack[top] {
                            Object::Ref(ptr)
                                if unsafe { (**ptr).type_tag } == TypeTag::ITERATOR as u8 =>
                            {
                                unsafe { read_iterator(*ptr) }.state.next()
                            }
                            _ => return Err("RuntimeError: not an iterator".to_string()),
                        }
                    };
                    match next_val {
                        Some(v) => self.push(v)?,
                        None => {
                            let frame = self
                                .call_stack
                                .last_mut()
                                .ok_or("no call frame".to_string())?;
                            frame.ip += offset;
                        }
                    }
                }

                // UNPACK n（task 26）：弹出顶部集合（tuple/list），将其 n 个元素
                // 压入栈，使元素 0 位于栈顶（编译器随后按序 StoreLocal 各循环变量）。
                // for..in 双变量循环（statement.rs:336）依赖此指令。
                OpCode::Unpack => {
                    let n = self.read_byte()? as usize;
                    let val = self.pop()?;
                    let elements: Vec<Object> = match &val {
                        Object::Ref(ptr) => {
                            debug_assert!(!ptr.is_null(), "null Object::Ref");
                            let tag = unsafe { (**ptr).type_tag };
                            if tag == TypeTag::TUPLE as u8 {
                                unsafe { read_tuple(*ptr) }.clone()
                            } else if tag == TypeTag::LIST as u8 {
                                unsafe { read_list(*ptr) }.clone()
                            } else {
                                return Err(format!(
                                    "TypeError: cannot unpack non-iterable '{}' object",
                                    val.type_name()
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "TypeError: cannot unpack non-iterable '{}' object",
                                val.type_name()
                            ))
                        }
                    };
                    if elements.len() != n {
                        return Err(format!(
                            "ValueError: not enough values to unpack (expected {}, got {})",
                            n,
                            elements.len()
                        ));
                    }
                    // 逆序压入，使 elements[0] 落在栈顶。
                    for e in elements.into_iter().rev() {
                        self.push(e)?;
                    }
                }

                // BUILD_TUPLE count：从栈顶弹出 count 个元素，构建 tuple 对象并压栈。
                // 编译端由 compile_tuple_literal（expression.rs）与 compile_return
                // 多返回值（statement.rs）发射。
                OpCode::BuildTuple => {
                    let count = self.read_byte()? as usize;
                    let start = self.stack
                        .len()
                        .checked_sub(count)
                        .ok_or("RuntimeError: stack underflow in BUILD_TUPLE")?;
                    let elements: Vec<Object> = self.stack.drain(start..).collect();
                    self.push(alloc_tuple(elements))?;
                }

                // CLOSURE（task 28）：从常量池取出 Function，捕获上值，创建 Closure。
                // 操作数：func_idx(2) + 每上值 (is_local:1, index:1)。
                OpCode::Closure => {
                    let func_idx = self.read_u16()? as usize;
                    let func_obj = self.read_constant(func_idx)?;

                    let func_ptr = match func_obj {
                        Object::Ref(ptr)
                            if unsafe { (*ptr).type_tag } == TypeTag::FUNCTION as u8 =>
                        {
                            ptr
                        }
                        _ => return Err("CLOSURE expects a Function".to_string()),
                    };

                    // task 27 嵌套布局：read_function(ptr).function.upvalue_count
                    let upvalue_count = unsafe { read_function(func_ptr) }.function.upvalue_count;
                    let mut upvalues: Vec<*mut MsObjHeader> = Vec::with_capacity(upvalue_count);

                    for _ in 0..upvalue_count {
                        let is_local = self.read_byte()? == 1;
                        let index = self.read_byte()? as usize; // 编译期已断言 ≤ 255

                        if is_local {
                            // 直接外层局部变量：捕获当前帧栈槽。
                            let stack_base = self.call_stack.last().unwrap().stack_base;
                            let location = stack_base + index;
                            upvalues.push(self.capture_upvalue(location));
                        } else {
                            // 外层上值：复用当前闭包的上值（上值链穿透）。
                            let closure_ptr = self.call_stack.last().unwrap().closure;
                            // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                            let closure = unsafe { read_closure(closure_ptr) };
                            upvalues.push(closure.upvalues[index]);
                        }
                    }

                    let closure_obj = alloc_closure(Object::Ref(func_ptr), upvalues);
                    self.push(closure_obj)?;
                }

                // LOAD_UPVALUE（task 28）：将当前闭包的上值[idx]压栈。
                OpCode::LoadUpvalue => {
                    let idx = self.read_byte()? as usize;
                    let closure_ptr = self.call_stack.last().unwrap().closure;
                    // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                    let closure = unsafe { read_closure(closure_ptr) };
                    let upvalue_ptr = closure.upvalues[idx];
                    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
                    let value = unsafe { read_upvalue(upvalue_ptr) }.get(&self.stack);
                    self.push(value)?;
                }

                // STORE_UPVALUE（task 28）：将栈顶存入当前闭包的上值[idx]（peek，不弹）。
                OpCode::StoreUpvalue => {
                    let idx = self.read_byte()? as usize;
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .unwrap_or(Object::Nil); // peek 栈顶（不弹）
                    let closure_ptr = self.call_stack.last().unwrap().closure;
                    // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                    let closure = unsafe { read_closure(closure_ptr) };
                    let upvalue_ptr = closure.upvalues[idx];
                    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
                    // read_upvalue 返回独立 &mut（源自裸指针，不与 self.stack 借用重叠）。
                    unsafe { read_upvalue(upvalue_ptr) }.set(&mut self.stack, value);
                }

                // CLOSE_UPVALUE（task 28）：关闭栈顶位置对应的开放上值，再弹栈。
                OpCode::CloseUpvalue => {
                    let stack_top = self.stack.len() - 1;
                    self.close_upvalues_from(stack_top);
                    self.stack.pop();
                }

                // CALL（task 25/27）：native 分支（TypeTag::FUNCTION）+ 用户函数分支（TypeTag::CLOSURE）。
                OpCode::Call => {
                    let argc = self.read_byte()? as usize;
                    // 边界检查（D1）：防止 argc 过大导致下溢/越界。
                    if argc + 1 > self.stack.len() {
                        return Err("stack underflow for CALL arguments".to_string());
                    }
                    let callee_idx = self.stack.len() - argc - 1;
                    let callee = self.stack[callee_idx].clone();
                    match &callee {
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8 =>
                        {
                            // 读出函数指针与参数个数信息（借用堆对象，不借用 self）。
                            let (func, arity, name) = {
                                debug_assert!(!ptr.is_null(), "null Object::Ref");
                                // SAFETY: type_tag 为 FUNCTION，指针由 alloc_native_function 分配。
                                let native = unsafe { read_native_function(*ptr) };
                                let arity = self.native_arities.get(native.name()).copied();
                                (native.func, arity, native.name().to_owned())
                            };
                            // 参数个数校验（C2）：固定 arity 须严格匹配。
                            if let Some(arity) = arity {
                                if arity != usize::MAX && arity != argc {
                                    return Err(format!(
                                        "TypeError: {}() takes exactly {} argument{} but {} were given",
                                        name,
                                        arity,
                                        if arity == 1 { "" } else { "s" },
                                        argc
                                    ));
                                }
                            }
                            let args = self.stack[self.stack.len() - argc..].to_vec();
                            self.stack.truncate(self.stack.len() - argc - 1);
                            let result = func(self, &args)?;
                            self.push(result)?;
                        }
                        // 用户函数（task 27）：CLOSURE 分支，与 native FUNCTION 分支并列。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 =>
                        {
                            // 经 closure.function 取 MsFunction 读 arity（不借用 self）。
                            let arity = {
                                debug_assert!(!ptr.is_null(), "null Object::Ref");
                                // SAFETY: type_tag 为 CLOSURE，指针由 alloc_closure 分配。
                                let closure = unsafe { read_closure(*ptr) };
                                // SAFETY: closure.function 由 alloc_function 分配。
                                let func = unsafe { read_function(closure.function) };
                                func.function.arity
                            };
                            if argc != arity {
                                return Err(format!(
                                    "TypeError: expected {} arguments, got {}",
                                    arity, argc
                                ));
                            }
                            if self.call_stack.len() >= MAX_CALL_DEPTH {
                                return Err("RecursionError: stack overflow".to_string());
                            }
                            // stack_base = callee_idx：slot 0 = callee（closure 自身），
                            // 参数在 slot 1..argc（与 compile_fn_decl 的 slot-0 预留约定自洽）。
                            self.call_stack.push(CallFrame::new(*ptr, callee_idx));
                        }
                        _ => {
                            return Err(format!(
                                "TypeError: '{}' object is not callable",
                                callee.type_name()
                            ))
                        }
                    }
                }

                // RETURN（task 27/28）：弹出返回值，关闭本帧开放上值，恢复调用者帧，截断值栈。
                OpCode::Return => {
                    let return_value = self.stack.pop().unwrap_or(Object::Nil);
                    // TODO(task 36)：弹出本帧前须执行其 defer 条目（EXEC_DEFER，LIFO）。
                    // Phase 3.1 defer_stack 恒空，此处暂不做。
                    let old_base = self
                        .call_stack
                        .last()
                        .ok_or("return outside function".to_string())?
                        .stack_base;
                    // task 28：先关闭当前帧的所有开放上值（栈尚未截断，location 仍有效）。
                    self.close_upvalues_from(old_base);
                    self.stack.truncate(old_base); // 移除 callee(slot0)+args+locals
                    self.call_stack.pop();
                    self.stack.push(return_value);

                    // 顶层帧 RETURN 后无更多调用者帧 → 终止执行（等价于隐式 HALT）。
                    if self.call_stack.is_empty() {
                        return Ok(self.stack.pop().unwrap_or(Object::Nil));
                    }
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
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::ast::node::Program;
    use crate::compiler::opcode::OpCode;
    use crate::compiler::{Chunk, Compiler};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::object::{
        alloc_dict, alloc_iterator, alloc_list, alloc_set, alloc_string, alloc_tuple, DictMap,
        IteratorState, Object,
    };
    use std::collections::HashSet;

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
        // 合成 while 循环：slot 1 为计数器（初始 0），每轮 +1，i<3 为限。
        // slot 0 为 VM 预分配的 callee 占位（Nil，task 27 订正 A3）。
        // 经 JumpBack 回边 3 轮后退出；Halt 弹出 slot 1 的最终值 → Int(3)。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 1 初值
                OpCode::LoadLocal as u8,
                0x01, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(3)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x01, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x01, // i = i + 1
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
        // slot 0 为 VM 预分配的 callee 占位（Nil，task 27 订正 A3）。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 1
                OpCode::LoadLocal as u8,
                0x01, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(2)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x01, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x01, // i = i + 1
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

    // ---- task 25：内置函数 CALL 集成（LoadGlobal → CALL native dispatch）----
    //
    // register_builtins 在 VM::new() 中注入全部内置函数；call_builtin 构造
    // 合成字节码（LoadGlobal name → push args → CALL → Halt）端到端验证
    // 原生函数调用分支、arity 校验与全局解析。

    fn call_builtin(name: &str, args: &[Object]) -> Result<Object, String> {
        let mut code = Vec::new();
        let mut constants = vec![alloc_string(name)];
        code.push(OpCode::LoadGlobal as u8);
        code.extend(&0u16.to_be_bytes()); // const[0] = name
        for arg in args {
            let idx = constants.len();
            constants.push(arg.clone());
            code.push(OpCode::Constant as u8);
            code.extend(&(idx as u16).to_be_bytes());
        }
        code.push(OpCode::Call as u8);
        code.push(args.len() as u8);
        code.push(OpCode::Halt as u8);
        run_chunk(code, constants)
    }

    /// 仅加载全局内置函数对象（不调用），用于 isinstance 第二参数等场景。
    fn load_builtin(name: &str) -> Object {
        run_chunk(
            vec![OpCode::LoadGlobal as u8, 0x00, 0x00, OpCode::Halt as u8],
            vec![alloc_string(name)],
        )
        .unwrap()
    }

    #[test]
    fn test_builtin_type_returns_name() {
        // type(42) -> "int"；type([1,2]) -> "list"（B3 扩展）
        assert_eq!(
            call_builtin("type", &[Object::Int(42)]).unwrap(),
            alloc_string("int")
        );
        assert_eq!(
            call_builtin("type", &[alloc_string("hello")]).unwrap(),
            alloc_string("string")
        );
        assert_eq!(
            call_builtin("type", &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_string("list")
        );
    }

    #[test]
    fn test_builtin_len() {
        assert_eq!(
            call_builtin("len", &[alloc_string("hello")]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            call_builtin(
                "len",
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(3)
        );
    }

    #[test]
    fn test_builtin_abs_max_min_sum() {
        assert_eq!(
            call_builtin("abs", &[Object::Int(-5)]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            call_builtin("max", &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(3)
        );
        assert_eq!(
            call_builtin("min", &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            call_builtin(
                "sum",
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(6)
        );
    }

    #[test]
    fn test_builtin_conversions() {
        assert_eq!(
            call_builtin("int", &[alloc_string("42")]).unwrap(),
            Object::Int(42)
        );
        assert_eq!(
            call_builtin("float", &[alloc_string("3.14")]).unwrap(),
            Object::Float(3.14)
        );
        assert_eq!(
            call_builtin("str", &[Object::Int(42)]).unwrap(),
            alloc_string("42")
        );
        assert_eq!(
            call_builtin("bool", &[Object::Int(0)]).unwrap(),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_builtin_ceil_floor_round() {
        assert_eq!(
            call_builtin("ceil", &[Object::Float(3.7)]).unwrap(),
            Object::Int(4)
        );
        assert_eq!(
            call_builtin("floor", &[Object::Float(3.7)]).unwrap(),
            Object::Int(3)
        );
        assert_eq!(
            call_builtin("round", &[Object::Float(3.5)]).unwrap(),
            Object::Float(4.0)
        );
        // round(3.14159, 2) -> 3.14
        assert_eq!(
            call_builtin("round", &[Object::Float(3.14159), Object::Int(2)]).unwrap(),
            Object::Float(3.14)
        );
    }

    #[test]
    fn test_builtin_round_digits_out_of_range() {
        // round(x, 20) -> ValueError（digits 越界，D3）
        let r = call_builtin("round", &[Object::Float(1.0), Object::Int(20)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_builtin_isinstance() {
        // isinstance(42, int) -> true（int 全局即内置转换函数，充当类型对象）
        assert_eq!(
            call_builtin("isinstance", &[Object::Int(42), load_builtin("int")]).unwrap(),
            Object::Bool(true)
        );
        // isinstance("hi", int) -> false
        assert_eq!(
            call_builtin("isinstance", &[alloc_string("hi"), load_builtin("int")]).unwrap(),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_builtin_arity_check() {
        // type() arity=1；传 0 参 → TypeError
        let r = call_builtin("type", &[]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_builtin_collection_conversions() {
        // list("abc") -> ["a","b","c"]
        assert_eq!(
            call_builtin("list", &[alloc_string("abc")]).unwrap(),
            alloc_list(vec![
                alloc_string("a"),
                alloc_string("b"),
                alloc_string("c")
            ])
        );
        // tuple([1,2]) -> (1,2)
        assert_eq!(
            call_builtin("tuple", &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_tuple(vec![Object::Int(1), Object::Int(2)])
        );
    }

    #[test]
    fn test_builtin_range() {
        // range 现返回迭代器（task 26）；list(range(5)) 消费为 [0,1,2,3,4]。
        // 合成嵌套调用字节码：CALL 约定要求 callee 在其参数之下，
        // 故 list 先入栈，再求值其参数 range(5)。
        let constants = vec![alloc_string("list"), alloc_string("range"), Object::Int(5)];
        let code = vec![
            OpCode::LoadGlobal as u8,
            0x00,
            0x00, // push list（外层 callee）
            OpCode::LoadGlobal as u8,
            0x00,
            0x01, // push range（内层 callee）
            OpCode::Constant as u8,
            0x00,
            0x02, // push Int(5)
            OpCode::Call as u8,
            0x01, // range(5) -> iterator（留在 list 之上）
            OpCode::Call as u8,
            0x01, // list(iter) -> [0,1,2,3,4]
            OpCode::Halt as u8,
        ];
        assert_eq!(
            run_chunk(code, constants).unwrap(),
            alloc_list(vec![
                Object::Int(0),
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4)
            ])
        );
    }

    #[test]
    fn test_builtin_print_returns_nil() {
        // print 不报错且返回 nil（输出至测试 stdout，格式由 Display 保证，已单测）
        assert_eq!(
            call_builtin("print", &[alloc_string("hello")]).unwrap(),
            Object::Nil
        );
    }

    #[test]
    fn test_call_non_callable_type_error() {
        // 调用非函数全局（nil 局部无此名）→ "object is not callable"
        // 注：未定义名经 LoadGlobal 返回 Nil；CALL Nil → TypeError。
        let r = call_builtin("does_not_exist", &[]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not callable"));
    }

    // ---- task 25：真实编译器端到端（Lexer+Parser+Compiler+VM）----
    //
    // 证明真实编译器对内置标识符发射 LoadGlobal（而非误判为局部），
    // CALL 经原生函数分支正确分派。用「错误注入」使结果可观测
    // （裸表达式语句被 POP，返回值恒为 Nil，故靠是否报错判断）。

    #[test]
    fn test_end_to_end_builtin_resolves_as_global() {
        // len(5) → 内置函数被解析并调用 → TypeError（int 无 len）
        let r = compile_and_run("len(5)");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
        // abs(-5) → 解析 + 调用成功，无报错
        assert!(compile_and_run("abs(-5)").is_ok());
    }

    #[test]
    fn test_end_to_end_nested_builtin_calls() {
        // print(type(42))：嵌套调用；type 内层成功 → print 外层成功 → Ok
        assert!(compile_and_run("print(type(42))").is_ok());
        // max(1, 2, 3) → Ok
        assert!(compile_and_run("max(1, 2, 3)").is_ok());
    }

    // ---- task 26：ITERATOR / FOR_ITER / UNPACK VM 执行 ----
    //
    // 注：顶层=局部作用域的已知 bug（spec line 334-338，task 23 既有先例）使
    // 真实编译的顶层 for..in 因 slot 1 越界而失败，故此处用合成 Chunk 直接
    // 验证 opcode 语义（与 test_while_loop_iterations 同策略）；编译器侧由
    // test_compiler_emits_for_in_opcodes 验证发射。

    /// 构造「统计可迭代对象元素个数」的 for..in 循环字节码并运行，返回计数。
    /// 覆盖 ITERATOR + FOR_ITER 对各可迭代类型的执行。
    fn count_iterations(iterable: Object) -> Result<Object, String> {
        // 布局：slot1=count（slot0 为 callee 占位）；push iterable→ITERATOR；
        // loop: FOR_ITER→Pop(弃值)→count+=1→JUMP_BACK；exit: Pop(iter)→LoadLocal 1→HALT。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00, // Int(0) → slot1
            OpCode::Constant as u8,
            0x00,
            0x01,                   // iterable
            OpCode::Iterator as u8, // → iter
            OpCode::ForIter as u8,
            0x00,
            0x0C,              // → exit (offset 12)
            OpCode::Pop as u8, // 弃迭代值
            OpCode::LoadLocal as u8,
            0x01,
            OpCode::Constant as u8,
            0x00,
            0x02, // Int(1)
            OpCode::Add as u8,
            OpCode::StoreLocal as u8,
            0x01,
            OpCode::JumpBack as u8,
            0x00,
            0x0F,              // → loop_start (offset 15)
            OpCode::Pop as u8, // exit: 弹出迭代器
            OpCode::LoadLocal as u8,
            0x01,
            OpCode::Halt as u8,
        ];
        run_chunk(code, vec![Object::Int(0), iterable, Object::Int(1)])
    }

    #[test]
    fn test_for_iter_counts_each_iterable_type() {
        // list / string / dict(键) / set / range 迭代器 均可被 FOR_ITER 遍历
        assert_eq!(
            count_iterations(alloc_list(vec![
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4),
                Object::Int(5)
            ]))
            .unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            count_iterations(alloc_string("hello")).unwrap(),
            Object::Int(5)
        );
        let mut m = DictMap::new();
        m.insert(alloc_string("a"), Object::Int(1));
        m.insert(alloc_string("b"), Object::Int(2));
        assert_eq!(count_iterations(alloc_dict(m)).unwrap(), Object::Int(2));
        let mut s = HashSet::new();
        s.insert(Object::Int(1));
        s.insert(Object::Int(2));
        s.insert(Object::Int(3));
        assert_eq!(count_iterations(alloc_set(s)).unwrap(), Object::Int(3));
        // range 迭代器（Iterator 对迭代器克隆状态，见 to_iterator ITERATOR 分支）
        let range_it = alloc_iterator(IteratorState::Range {
            current: 0,
            end: 5,
            step: 1,
        });
        assert_eq!(count_iterations(range_it).unwrap(), Object::Int(5));
        // 空可迭代对象 → 0 次
        assert_eq!(
            count_iterations(alloc_list(vec![])).unwrap(),
            Object::Int(0)
        );
    }

    #[test]
    fn test_for_iter_sums_list_values() {
        // 验证 FOR_ITER 推送的值确为各元素（累加 [0,1,2,3,4] = 10）。
        // 布局：slot1=sum（slot0 为 callee 占位）；push list→ITERATOR；
        // loop: FOR_ITER→LoadLocal1→Add→StoreLocal1→JUMP_BACK；
        // exit: Pop→LoadLocal1→HALT。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00, // Int(0) → slot1 (sum)
            OpCode::Constant as u8,
            0x00,
            0x01, // list [0,1,2,3,4]
            OpCode::Iterator as u8,
            OpCode::ForIter as u8,
            0x00,
            0x08, // → exit (offset 8)
            OpCode::LoadLocal as u8,
            0x01,              // push sum
            OpCode::Add as u8, // value + sum
            OpCode::StoreLocal as u8,
            0x01, // sum = value + sum
            OpCode::JumpBack as u8,
            0x00,
            0x0B,              // → loop_start (offset 11)
            OpCode::Pop as u8, // exit: 弹出迭代器
            OpCode::LoadLocal as u8,
            0x01,
            OpCode::Halt as u8,
        ];
        let list = alloc_list(vec![
            Object::Int(0),
            Object::Int(1),
            Object::Int(2),
            Object::Int(3),
            Object::Int(4),
        ]);
        assert_eq!(
            run_chunk(code, vec![Object::Int(0), list]).unwrap(),
            Object::Int(10)
        );
    }

    #[test]
    fn test_unpack_opcode_isolated() {
        // UNPACK 2：tuple(5,7) → 逆序压入，元素 0 落在栈顶 → HALT 返回 5。
        let tuple = alloc_tuple(vec![Object::Int(5), Object::Int(7)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        assert_eq!(run_chunk(code, vec![tuple]).unwrap(), Object::Int(5));
        // 个数不匹配 → ValueError
        let tuple2 = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x03, // 期望 3，实际 2
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![tuple2]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_for_iter_with_unpack_sums_first_elements() {
        // 端到端模拟双变量 for..in：遍历 [(1,10),(2,20),(3,30)]，累加每对首元素 = 6。
        // 验证 FOR_ITER + UNPACK + 多轮迭代协同。布局：slot1=sum（slot0 为 callee 占位）；
        // push list→ITERATOR；loop: FOR_ITER→UNPACK 2→LoadLocal1→Add→StoreLocal1→
        // Pop(弃次元素)→JUMP_BACK；exit: Pop→LoadLocal1→HALT。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00, // Int(0) → slot1
            OpCode::Constant as u8,
            0x00,
            0x01, // list of tuples
            OpCode::Iterator as u8,
            OpCode::ForIter as u8,
            0x00,
            0x0B, // → exit (offset 11)
            OpCode::Unpack as u8,
            0x02, // → [iter, second, first]
            OpCode::LoadLocal as u8,
            0x01,              // push sum
            OpCode::Add as u8, // first + sum
            OpCode::StoreLocal as u8,
            0x01,              // sum = first + sum
            OpCode::Pop as u8, // 弃 second
            OpCode::JumpBack as u8,
            0x00,
            0x0E,              // → loop_start (offset 14)
            OpCode::Pop as u8, // exit: 弹出迭代器
            OpCode::LoadLocal as u8,
            0x01,
            OpCode::Halt as u8,
        ];
        let tuples = alloc_list(vec![
            alloc_tuple(vec![Object::Int(1), Object::Int(10)]),
            alloc_tuple(vec![Object::Int(2), Object::Int(20)]),
            alloc_tuple(vec![Object::Int(3), Object::Int(30)]),
        ]);
        assert_eq!(
            run_chunk(code, vec![Object::Int(0), tuples]).unwrap(),
            Object::Int(6)
        );
    }

    // ---- task 30: BUILD_TUPLE VM handler + 多返回值/元组解包 ----

    #[test]
    fn test_build_tuple_opcode_constructs_tuple() {
        // BUILD_TUPLE 3：弹出栈顶 3 个元素，构建 tuple 并压栈 → HALT 返回该 tuple。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::Constant as u8,
            0x00,
            0x02,
            OpCode::BuildTuple as u8,
            0x03,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap();
        assert_eq!(
            result,
            alloc_tuple(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
    }

    #[test]
    fn test_build_tuple_opcode_empty() {
        // BUILD_TUPLE 0：构建空 tuple。
        let code = vec![OpCode::BuildTuple as u8, 0x00, OpCode::Halt as u8];
        let result = run_chunk(code, vec![]).unwrap();
        assert_eq!(result, alloc_tuple(vec![]));
        assert_eq!(format!("{}", result), "()");
    }

    #[test]
    fn test_build_tuple_opcode_preserves_order() {
        // 栈顶顺序保持：先压 1、再压 2 → BUILD_TUPLE 2 → tuple(1, 2)（非逆序）。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::BuildTuple as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(1), Object::Int(2)]).unwrap();
        assert_eq!(result, alloc_tuple(vec![Object::Int(1), Object::Int(2)]));
    }

    #[test]
    fn test_build_tuple_opcode_underflow() {
        // 栈上不足 count 个元素 → RuntimeError（stack underflow）。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::BuildTuple as u8,
            0x03, // 仅 1 个元素，却要 3 个
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![Object::Int(1)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("underflow"));
    }

    #[test]
    fn test_multi_return_build_then_unpack_roundtrip() {
        // 模拟多返回值完整字节码流：return a, b → BUILD_TUPLE 2 → DUP → UNPACK 2。
        // UNPACK 逆序压栈使 elements[0] 位于栈顶 → HALT 返回 elements[0]。
        // 注：q, r = func() 的端到端形式受解析器限制（单值右值被包成 1 元 tuple），
        // 故以合成字节码验证 BUILD_TUPLE + UNPACK 协同。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::BuildTuple as u8,
            0x02,
            OpCode::Dup as u8,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(3), Object::Int(1)]).unwrap();
        assert_eq!(result, Object::Int(3)); // elements[0] 在栈顶
    }

    #[test]
    fn test_swap_via_unpack_e2e() {
        // 交换：a, b = b, a（右值为多值 TupleLiteral，解析器支持）。
        let src = "a = 1\nb = 2\na, b = b, a\nif a != 2 {\n1/0\n}\nif b != 1 {\n1/0\n}";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_multi_value_rhs_unpack_e2e() {
        // 多目标赋值 a, b = 10, 20（右值为多值 TupleLiteral，解析器支持）。
        // 验证 compile_store_target 的 TupleLiteral 分支（UNPACK + 正序 store）。
        let src = "a, b = 10, 20\nif a != 10 {\n1/0\n}\nif b != 20 {\n1/0\n}";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_unpack_count_mismatch_synthetic() {
        // 解包数量不匹配 → ValueError（UNPACK handler 运行时校验，task 26 已实现）。
        // 用合成字节码解包真正的 2 元素 tuple（解析器对单值右值会包成 1 元 tuple）。
        let tuple = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x03, // 期望 3，实际 2
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![tuple]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_unpack_list_synthetic() {
        // 对 list 解包：UNPACK 支持 tuple/list（task 26 已实现）。
        // 用合成字节码解包 2 元素 list，验证 elements[0] 在栈顶（逆序压栈）。
        let list = alloc_list(vec![Object::Int(10), Object::Int(20)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8, // 返回栈顶 = elements[0]
        ];
        let result = run_chunk(code, vec![list]).unwrap();
        assert_eq!(result, Object::Int(10));
    }

    #[test]
    fn test_unpack_regression_for_in_still_works() {
        // 回归：for..in 双变量循环依赖 UNPACK，task 30 不应破坏。
        let prog = parse("s = 0\nfor i, v in [(1, 10), (2, 20)] {\ns = s + i\n}");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::Unpack as u8)));
    }

    #[test]
    fn test_iterator_opcode_not_iterable_errors() {
        // ITERATOR 对不可迭代对象（int）→ RuntimeError（TypeError 包装）
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Iterator as u8,
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![Object::Int(1)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not iterable"));
    }

    #[test]
    fn test_compiler_emits_for_in_opcodes() {
        // 证明真实编译器对 for..in 发射 ITERATOR + FOR_ITER（编译器侧已实现，
        // task 19）；本 task 在 VM 侧补齐执行。双变量额外发 UNPACK。
        let prog = parse("for i in [1] {\n}");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::Iterator as u8)));
        assert!(chunk.code.contains(&(OpCode::ForIter as u8)));

        let prog2 = parse("for a, b in [(1, 2)] {\n}");
        let chunk2 = Compiler::new().compile(&prog2).unwrap();
        assert!(chunk2.code.contains(&(OpCode::Unpack as u8)));
    }

    // ---- task 27：调用帧与函数调用 ----

    #[test]
    fn test_function_decl_compiles_to_global_binding() {
        // task 28：fn 声明编译为存 Function 入常量池 + 发 CLOSURE 指令（运行期包装）
        // + STORE_GLOBAL(name)。常量池存 FUNCTION（非 CLOSURE）。
        let prog = parse("fn f(x) { return x }");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::StoreGlobal as u8)));
        assert!(chunk.code.contains(&(OpCode::Closure as u8)));
        // 常量池中应存在 MsFunction（CLOSURE 指令运行期包装为 Closure）
        assert!(chunk.constants.iter().any(|c| {
            matches!(
                c,
                Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8
            )
        }));
    }

    #[test]
    fn test_msfunction_msclosure_alloc_and_read() {
        use crate::vm::object::{
            alloc_closure, alloc_function, read_closure, read_function, Function,
        };
        let func = Function {
            name: "test".to_string(),
            arity: 2,
            code: vec![OpCode::Halt as u8],
            constants: vec![],
            upvalue_count: 0,
            source_file: None,
        };
        let func_obj = alloc_function(func);
        let Object::Ref(func_ptr) = func_obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*func_ptr).type_tag, TypeTag::FUNCTION as u8);
            let f = read_function(func_ptr);
            assert_eq!(f.function.arity, 2);
            assert_eq!(f.function.name, "test");
        }

        let closure_obj = alloc_closure(func_obj, Vec::new());
        let Object::Ref(cl_ptr) = closure_obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*cl_ptr).type_tag, TypeTag::CLOSURE as u8);
            let cl = read_closure(cl_ptr);
            let f = read_function(cl.function);
            assert_eq!(f.function.arity, 2);
        }
    }

    #[test]
    fn test_user_function_call_returns_value() {
        // fn add(a, b) { return a + b }；验证 add(3,4) == 7。
        // 这同时验证 param0 != callee（若 slot 1 指向闭包则返回闭包而非 3）。
        let result = compile_and_run(
            r#"
            fn add(a, b) {
                return a + b
            }
            assert(add(3, 4) == 7)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_user_function_single_param() {
        // fn id(x) { return x }；验证 id(42) == 42（param0 = slot1 ≠ callee）。
        let result = compile_and_run(
            r#"
            fn id(x) {
                return x
            }
            assert(id(42) == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_user_function_string_concat() {
        // fn greet(name) { return "Hello, " + name }
        let result = compile_and_run(
            r#"
            fn greet(name) {
                return "Hello, " + name
            }
            assert(greet("World") == "Hello, World")
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_implicit_nil_return() {
        // 无显式 return → 隐式 NIL + RETURN → 返回 nil。
        let result = compile_and_run(
            r#"
            fn noop() {
            }
            assert(noop() == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_implicit_nil_return_with_body() {
        // 函数体有语句但无 return → 隐式返回 nil。
        let result = compile_and_run(
            r#"
            fn side_effect(x) {
                assert(x > 0)
            }
            assert(side_effect(1) == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_recursion_factorial() {
        // 递归调用：factorial(10) == 3628800。
        let result = compile_and_run(
            r#"
            fn factorial(n) {
                if n <= 1 {
                    return 1
                }
                return n * factorial(n - 1)
            }
            assert(factorial(10) == 3628800)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_arity_mismatch_too_many() {
        let result = compile_and_run(
            r#"
            fn one(x) {
                return x
            }
            one(1, 2)
            "#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected 1 arguments, got 2"));
    }

    #[test]
    fn test_arity_mismatch_too_few() {
        let result = compile_and_run(
            r#"
            fn two(a, b) {
                return a
            }
            two(1)
            "#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected 2 arguments, got 1"));
    }

    #[test]
    fn test_stack_overflow_at_max_depth() {
        // 无限递归 → MAX_CALL_DEPTH(1000) 触发栈溢出。
        let result = compile_and_run(
            r#"
            fn recurse() {
                return recurse()
            }
            recurse()
            "#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("stack overflow"));
    }

    #[test]
    fn test_call_non_callable_error() {
        // 调用 int → TypeError: not callable。
        let result = compile_and_run("42()");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not callable"));
    }

    #[test]
    fn test_native_builtin_still_works_with_user_fn() {
        // print 等内置函数仍正常工作（经 FUNCTION native 分支），与用户函数共存。
        let result = compile_and_run(
            r#"
            fn double(x) {
                return x + x
            }
            print(double(21))
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_spec_end_to_end_program() {
        // spec 验证标准 §10 的完整测试程序（greet/add/factorial）须无错执行。
        let result = compile_and_run(
            r#"
            fn greet(name) {
                return "Hello, " + name
            }

            fn add(a, b) {
                return a + b
            }

            print(greet("World"))
            print(add(3, 4))

            fn factorial(n) {
                if n <= 1 {
                    return 1
                }
                return n * factorial(n - 1)
            }
            print(factorial(10))
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_calls() {
        // 嵌套调用：add(add(1, 2), add(3, 4)) == 10。
        let result = compile_and_run(
            r#"
            fn add(a, b) {
                return a + b
            }
            assert(add(add(1, 2), add(3, 4)) == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    // ---- task 28：闭包与上值机制 ----
    //
    // 注：匿名 fn 字面量（`fn() {...}` 表达式）由 task 29 实现；本任务的闭包机制
    // 经「命名函数声明」（编译期发 CLOSURE 指令）即可完整验证——CLOSURE 指令对命名/
    // 匿名函数一致地捕获上值并运行期包装。

    #[test]
    fn test_closure_make_counter() {
        // 经典计数器：nonlocal 写捕获。counter() 返回 1、2、3。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            counter = make_counter()
            assert(counter() == 1)
            assert(counter() == 2)
            assert(counter() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_read_capture() {
        // 只读捕获：内层函数读取外层局部（无需 nonlocal）。
        let result = compile_and_run(
            r#"
            fn make_reader() {
                value = 42
                fn reader() {
                    return value
                }
                return reader
            }
            r = make_reader()
            assert(r() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_shared_upvalue_getter_setter() {
        // 多个闭包共享同一上值：setter 经 nonlocal 修改后 getter 可见。
        // 验证 capture_upvalue 对相同 location 复用同一 MsUpvalue。
        let result = compile_and_run(
            r#"
            fn make_pair() {
                x = 10
                fn getter() {
                    return x
                }
                fn setter(v) {
                    nonlocal x
                    x = v
                }
                assert(getter() == 10)
                setter(42)
                assert(getter() == 42)
                return getter()
            }
            assert(make_pair() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_nested_upvalue_chain() {
        // 三层嵌套：最内层经 is_local=false 复用中间层的上值（上值链穿透）。
        let result = compile_and_run(
            r#"
            fn outer() {
                x = 1
                fn middle() {
                    fn inner() {
                        return x
                    }
                    return inner()
                }
                return middle()
            }
            assert(outer() == 1)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_survives_return_close() {
        // 外层函数返回后，被捕获变量仍存活：RETURN 在 truncate 前关闭上值（拷到堆）。
        let result = compile_and_run(
            r#"
            fn make() {
                x = 5
                fn reader() {
                    return x
                }
                return reader
            }
            g = make()
            assert(g() == 5)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_two_independent_counters() {
        // 两次 make_counter 产生独立计数器（各自独立的 count 上值）。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            a = make_counter()
            b = make_counter()
            assert(a() == 1)
            assert(a() == 2)
            assert(b() == 1)
            assert(a() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nonlocal_write_modifies_outer() {
        // nonlocal 写入对外层可见（经 STORE_UPVALUE 修改共享上值）。
        let result = compile_and_run(
            r#"
            fn outer() {
                total = 0
                fn add_one() {
                    nonlocal total
                    total += 1
                }
                add_one()
                add_one()
                add_one()
                return total
            }
            assert(outer() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_nonlocal_creates_local_does_not_penetrate() {
        // 无 nonlocal 声明时，赋值在内层创建新局部，不穿透外层（04-functions.md）。
        let result = compile_and_run(
            r#"
            fn outer() {
                x = 10
                fn inner() {
                    x = 99
                    return x
                }
                assert(inner() == 99)
                assert(x == 10)
                return x
            }
            assert(outer() == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nonlocal_missing_binding_is_compile_error() {
        // nonlocal 声明的名字在外层作用域不存在 → 编译错误。
        let program = parse(
            "fn bad() {\n    nonlocal zzz\n    zzz = 1\n}\n",
        );
        let mut compiler = Compiler::new();
        let result = compiler.compile(&program);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("nonlocal"),
            "expected nonlocal error"
        );
    }

    #[test]
    fn test_closure_recursion_regression() {
        // 回归：递归仍正常工作（闭包机制不影响调用帧）。
        let result = compile_and_run(
            r#"
            fn fact(n) {
                if n <= 1 {
                    return 1
                }
                return n * fact(n - 1)
            }
            assert(fact(5) == 120)
            assert(fact(10) == 3628800)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_builtins_regression() {
        // 回归：内置函数（len/abs/range 等）仍正常工作。
        // 注：for-in 循环经编译器在顶层有已知栈布局限制（spec 334-338，task 23
        // 既有先例），此处不使用 for-in，仅验证 builtin 调用不受闭包机制影响。
        let result = compile_and_run(
            r#"
            assert(len("hello") == 5)
            assert(abs(-7) == 7)
            assert(abs(7) == 7)
            assert(min(3, 8) == 3)
            assert(max(3, 8) == 8)
            "#,
        );
        assert!(result.is_ok());
    }

    // ---- task 29：匿名函数（函数字面量）----
    //
    // 匿名函数是一等公民：可赋值、传参、做返回值、存于集合。其编译为 name="<anonymous>"
    // 的 Function + CLOSURE 指令，闭包值留栈作为表达式结果（不发 STORE_GLOBAL）。

    #[test]
    fn test_anon_fn_assignment_and_call() {
        // double = fn(x) { return x * 2 }；通过变量名调用。
        let result = compile_and_run(
            r#"
            double = fn(x) { return x * 2 }
            assert(double(5) == 10)
            assert(double(0) == 0)
            assert(double(-3) == -6)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_higher_order() {
        // 用户定义的高阶函数：匿名函数作为参数传递。
        // apply(fn(x){ return x*x }, 4) == 16
        let result = compile_and_run(
            r#"
            fn apply(f, x) { return f(x) }
            assert(apply(fn(x) { return x * x }, 4) == 16)
            assert(apply(fn(x) { return x + 1 }, 9) == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_stored_value_called() {
        // 匿名函数值存储后（赋值给变量）经变量取出调用 —— 一等公民「存储后调用」语义。
        // 注：spec §验证标准 #6 的 dict/list 集合存储 + 下标调用（如 ops["add"](3,4)）
        // 无法端到端验证：VM 尚未实装 BuildDict/BuildList/GetIndex 等集合构造与下标
        // 操作码（属独立 VM 实装任务，非 task 29 范畴）。此处用变量存储验证同等的
        // 「值存储 → 取出 → 调用」语义；集合场景的编译端由
        // test_compile_anon_fn_in_dict_literal（expression.rs）覆盖。
        let result = compile_and_run(
            r#"
            add = fn(a, b) { return a + b }
            mul = fn(a, b) { return a * b }
            assert(add(3, 4) == 7)
            assert(mul(3, 4) == 12)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_closure_counter() {
        // 匿名闭包经 nonlocal 捕获外层变量：counter() 返回 1、2。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                return fn() {
                    nonlocal count
                    count += 1
                    return count
                }
            }
            counter = make_counter()
            assert(counter() == 1)
            assert(counter() == 2)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_read_capture() {
        // 匿名闭包只读捕获外层局部（无需 nonlocal）。
        let result = compile_and_run(
            r#"
            fn make_reader() {
                value = 42
                return fn() { return value }
            }
            r = make_reader()
            assert(r() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_implicit_return_nil() {
        // 无显式 return 的匿名函数返回 nil。
        let result = compile_and_run(
            r#"
            side = fn() { x = 1 }
            assert(side() == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_as_return_value() {
        // 具名函数返回匿名函数，调用结果。
        let result = compile_and_run(
            r#"
            fn make_adder(n) {
                return fn(x) { return x + n }
            }
            add5 = make_adder(5)
            assert(add5(10) == 15)
            assert(add5(0) == 5)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_named_decl_regression() {
        // 回归：具名函数声明 + 闭包（task 27/28）仍正常工作。
        let result = compile_and_run(
            r#"
            fn fact(n) {
                if n <= 1 {
                    return 1
                }
                return n * fact(n - 1)
            }
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            assert(fact(5) == 120)
            c = make_counter()
            assert(c() == 1)
            assert(c() == 2)
            # 匿名与具名共存
            sq = fn(x) { return x * x }
            assert(sq(fact(3)) == 36)
            "#,
        );
        assert!(result.is_ok());
    }
}
