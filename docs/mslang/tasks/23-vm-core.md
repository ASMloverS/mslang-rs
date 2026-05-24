# 虚拟机核心执行循环

## 所属阶段
Phase 2.4a - 字节码编译 + VM 核心

## 前置任务
- 21-object-system-operations
- 19-compile-statements

## 目标

实现虚拟机核心结构体和执行循环，支持基本脚本的运行。包括值栈、全局变量表、CallFrame、以及核心指令的执行。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) VM 核心结构：

### VM 结构

```rust
struct VM {
    stack: Vec<Object>,
    stack_base: usize,
    call_stack: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    defer_stack: Vec<DeferEntry>,
    open_upvalues: Vec<Gc<Upvalue>>,
    gc: GarbageCollector,
}
```

### CallFrame

```rust
struct CallFrame {
    closure: Gc<Closure>,
    ip: usize,
    stack_base: usize,
    defer_stack_base: usize,
}
```

> **注意**（引用 [12-implementation-plan.md](../12-implementation-plan.md) Phase 2 备注）：由于 mslang 支持顶层 await，CallFrame 设计需要预留帧快照/恢复能力，以便 Phase 7 无缝集成 async/await。

### 执行循环

```rust
fn run(&mut self) {
    loop {
        let opcode = self.read_byte();
        match opcode {
            OpCode::CONSTANT => { ... }
            OpCode::ADD => { ... }
            // ...
            OpCode::HALT => return,
        }
    }
}
```

## 实现细节

### 文件位置

- `src/vm/mod.rs`（VM 主循环）
- `src/vm/frame.rs`（CallFrame）
- `src/vm/gc.rs`（GC 基础框架）

### CallFrame 设计（预留快照/恢复）

```rust
#[derive(Clone)]
pub struct CallFrame {
    pub chunk: Chunk,
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
}

impl CallFrame {
    pub fn snapshot(&self, stack: &[Object]) -> FrameSnapshot {
        FrameSnapshot {
            ip: self.ip,
            stack_base: self.stack_base,
            stack_slice: stack[self.stack_base..].to_vec(),
        }
    }

    pub fn restore(&mut self, snapshot: &FrameSnapshot) -> Vec<Object> {
        self.ip = snapshot.ip;
        snapshot.stack_slice.clone()
    }
}

pub struct FrameSnapshot {
    pub ip: usize,
    pub stack_base: usize,
    pub stack_slice: Vec<Object>,
}
```

### VM 结构体

```rust
const STACK_MAX: usize = 1024;

pub struct VM {
    stack: Vec<Object>,
    frames: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    defer_stack: Vec<DeferEntry>,
}

pub struct VMResult {
    pub output: Vec<String>,
}
```

### VM 初始化

```rust
impl VM {
    pub fn new() -> Self {
        VM {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::new(),
            globals: HashMap::new(),
            defer_stack: Vec::new(),
        }
    }

    pub fn interpret(&mut self, chunk: Chunk) -> Result<Object, String> {
        let frame = CallFrame {
            chunk,
            ip: 0,
            stack_base: 0,
            defer_stack_base: 0,
        };
        self.frames.push(frame);
        self.run()
    }
}
```

### 栈操作

```rust
impl VM {
    fn push(&mut self, value: Object) {
        if self.stack.len() >= STACK_MAX {
            panic!("Stack overflow");
        }
        self.stack.push(value);
    }

    fn pop(&mut self) -> Object {
        self.stack.pop().expect("Stack underflow")
    }

    fn peek(&self, distance: usize) -> &Object {
        &self.stack[self.stack.len() - 1 - distance]
    }

    fn peek_mut(&mut self, distance: usize) -> &mut Object {
        let len = self.stack.len();
        &mut self.stack[len - 1 - distance]
    }
}
```

### 指令读取

```rust
impl VM {
    fn read_byte(&mut self) -> u8 {
        let frame = self.frames.last_mut().unwrap();
        let byte = frame.chunk.code[frame.ip];
        frame.ip += 1;
        byte
    }

    fn read_u16(&mut self) -> u16 {
        let frame = self.frames.last_mut().unwrap();
        let bytes = [frame.chunk.code[frame.ip], frame.chunk.code[frame.ip + 1]];
        frame.ip += 2;
        u16::from_be_bytes(bytes)
    }

    fn read_u8(&mut self) -> u8 {
        let frame = self.frames.last_mut().unwrap();
        let byte = frame.chunk.code[frame.ip];
        frame.ip += 1;
        byte
    }
}
```

### 核心指令执行

```rust
impl VM {
    fn run(&mut self) -> Result<Object, String> {
        loop {
            let opcode_byte = self.read_byte();
            let opcode = OpCode::from_byte(opcode_byte)
                .ok_or_else(|| format!("Unknown opcode: {}", opcode_byte))?;

            match opcode {
                OpCode::Constant => {
                    let idx = self.read_u16() as usize;
                    let frame = self.frames.last().unwrap();
                    let value = frame.chunk.constants[idx].clone();
                    self.push(value);
                }

                OpCode::Nil => self.push(Object::Nil),
                OpCode::True => self.push(Object::Bool(true)),
                OpCode::False => self.push(Object::Bool(false)),

                OpCode::LoadLocal => {
                    let slot = self.read_u8() as usize;
                    let frame = self.frames.last().unwrap();
                    let stack_base = frame.stack_base;
                    self.push(self.stack[stack_base + slot].clone());
                }

                OpCode::StoreLocal => {
                    let slot = self.read_u8() as usize;
                    let frame = self.frames.last().unwrap();
                    let stack_base = frame.stack_base;
                    let value = self.pop();
                    self.stack[stack_base + slot] = value;
                }

                OpCode::LoadGlobal => {
                    let name_idx = self.read_u16() as usize;
                    let frame = self.frames.last().unwrap();
                    let name = match &frame.chunk.constants[name_idx] {
                        Object::String(s) => s.borrow().data.clone(),
                        _ => return Err("Invalid global name".to_string()),
                    };
                    let value = self.globals.get(&name)
                        .cloned()
                        .unwrap_or(Object::Nil);
                    self.push(value);
                }

                OpCode::StoreGlobal => {
                    let name_idx = self.read_u16() as usize;
                    let frame = self.frames.last().unwrap();
                    let name = match &frame.chunk.constants[name_idx] {
                        Object::String(s) => s.borrow().data.clone(),
                        _ => return Err("Invalid global name".to_string()),
                    };
                    let value = self.pop();
                    self.globals.insert(name, value);
                }

                OpCode::Pop => {
                    self.pop();
                }

                OpCode::Dup => {
                    let value = self.peek(0).clone();
                    self.push(value);
                }

                OpCode::Halt => {
                    return Ok(self.pop());
                }

                _ => {
                    return Err(format!("Unimplemented opcode: {:?}", opcode));
                }
            }
        }
    }
}
```

### GC 基础框架（MVP 存根）

```rust
pub struct GarbageCollector {
    bytes_allocated: usize,
    next_gc: usize,
}

impl GarbageCollector {
    pub fn new() -> Self {
        GarbageCollector {
            bytes_allocated: 0,
            next_gc: 1024 * 1024,
        }
    }

    pub fn should_collect(&self) -> bool {
        self.bytes_allocated >= self.next_gc
    }

    pub fn collect(&mut self) {
        // MVP: no-op，Phase 后续实现标记-清除
    }
}
```

## 验证标准

1. VM 能执行空程序（仅 HALT）
2. `CONSTANT` 指令正确加载常量到栈
3. `LOAD_GLOBAL` / `STORE_GLOBAL` 正确读写全局变量
4. `LOAD_LOCAL` / `STORE_LOCAL` 正确读写局部变量
5. `POP` / `DUP` 正确操作栈
6. 以下脚本能正确执行并输出 `30`：

```ms
x = 10
y = 20
z = x + y
print(z)
```

## 测试用例

```ms
# test_vm_core.ms
x = 10
y = 20
z = x + y
print(z)
```

预期输出：`30`

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn compile_and_run(source: &str) -> Result<Object, String> {
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        let mut vm = VM::new();
        vm.interpret(chunk)
    }

    #[test]
    fn test_empty_program() {
        let result = compile_and_run("");
        assert!(result.is_ok());
    }

    #[test]
    fn test_constant_loading() {
        let source = "x = 42";
        let result = compile_and_run(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_global_variable() {
        let source = r#"
            x = 10
            y = 20
            z = x + y
        "#;
        let mut vm = VM::new();
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        vm.interpret(chunk).unwrap();
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(30)));
    }

    #[test]
    fn test_stack_operations() {
        let source = r#"
            x = 1
            y = 2
            z = x + y
        "#;
        let mut vm = VM::new();
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        vm.interpret(chunk).unwrap();
        assert_eq!(vm.globals.get("z"), Some(&Object::Int(3)));
    }
}
```
