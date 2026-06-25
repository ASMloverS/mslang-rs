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

引自 [11-bytecode-vm.md](../11-bytecode-vm.md) VM 核心结构（完整字段）。本 task 仅启用标注 ✅ 的字段，其余为后续 task 预留：

```rust
struct VM {
    stack: Vec<Object>,                    // 值栈（内联值 + Ref 指针）         ✅ task 23
    stack_base: usize,                     // 当前帧栈基址（实现中存于 CallFrame） ✅ task 23
    call_stack: Vec<CallFrame>,            // 调用栈                            ✅ task 23
    globals: HashMap<String, Object>,      // 全局变量                          ✅ task 23
    defer_stack: Vec<DeferEntry>,          // defer 栈                          ⬜ task 36
    open_upvalues: Vec<*mut MsObjHeader>,  // 开放上值（原 *mut RuntimeUpvalue 已对齐 11-bytecode-vm.md） ⬜ task 28
    event_loop: EventLoop,                 // 事件循环（并发用）                ⬜ task 53
    heap: MsHeap,                          // 堆（Young/Old/Large Object Space） ⬜ task 52
    gc_config: GcConfig,                   // GC 配置                           ⬜ task 52
    gc_phase: AtomicU8,                    // GC 状态机当前阶段                 ⬜ task 52
    safepoint_requested: AtomicBool,       // 安全点请求标志                    ⬜ task 52
    c_roots: HashSet<*mut MsObjHeader>,    // C 侧注册的 GC 根                  ⬜ task 65（C API）
}
```

> **GC 存根**：本 task 引入最小 `GarbageCollector` 存根（`src/gc/mod.rs`），仅提供 `should_collect`/`collect`（no-op）与触发阈值，供主循环预留触发点；真实实现见 task 52。

### CallFrame

引自 [11-bytecode-vm.md](../11-bytecode-vm.md) CallFrame（目标形态）：

```rust
struct CallFrame {
    closure: Gc<Closure>,    // 被调用的闭包（目标形态，task 28）
    ip: usize,               // 程序计数器
    stack_base: usize,       // 栈基址
    defer_stack_base: usize, // defer 栈基址（task 36）
}
```

> **Phase 2 MVP 偏差（已声明）**：`Gc<T>` 与 `Closure` 在 task 28 才引入。本 task 实现细节（见下）暂以 `chunk: Chunk` 直接持有字节码块代替 `closure` 指针；**task 27（调用帧）/task 28（闭包）须将其重构为 closure 指针**。`defer_stack_base` 同理在 task 36 启用。

> **可暂停帧要求**（引用 [12-implementation-plan.md](../12-implementation-plan.md) Phase 2 备注 与 [11-bytecode-vm.md](../11-bytecode-vm.md) CallFrame 节）：为支持顶层 `await`（[08-concurrency](../08-concurrency.md)）与生成器 `yield`，Phase 2 须保证**值栈按帧分段管理**（每帧的 `[stack_base..stack_top)` 区间独立可复制），避免 Phase 7 大规模重构。本 task 通过 `stack_base` 满足该不变量；帧的 `snapshot`/`restore` 方法推迟到 task 39（生成器）/task 53（async）实现。

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

> GC 基础框架位于已有的 `src/gc/mod.rs`（crate 根 GC 模块），**不在 `src/vm/` 下新建**；VM 通过 `crate::gc::GarbageCollector` 引用。真实标记-清除见 task 52。

### CallFrame 设计

```rust
#[derive(Clone)]
pub struct CallFrame {
    pub chunk: Chunk,
    pub ip: usize,
    pub stack_base: usize,
}
```

> - `chunk: Chunk`：Phase 2 MVP 直接持有字节码块（见上「Phase 2 MVP 偏差」），task 27/28 改为 `closure` 指针。
> - `defer_stack_base`：task 36 引入（届时连同 `defer_stack` 一起加入）。
> - 帧快照 `snapshot`/`restore` 与 `FrameSnapshot` 推迟到 task 39（生成器）/task 53（async）。Phase 2 仅需保证「值栈按帧分段」不变量——已由 `stack_base` 满足。

### VM 结构体（MVP 子集）

仅含 task 23 启用的字段（完整设计见上方「VM 结构」）：

```rust
const STACK_MAX: usize = 1024;

pub struct VM {
    stack: Vec<Object>,
    frames: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    gc: GarbageCollector,   // src/gc/mod.rs 存根；task 52 实现真实 GC
}
```

> - `defer_stack`、`open_upvalues`、`event_loop`、`heap` 等字段分别由 task 36/28/53/52 引入，MVP 不启用。
> - 原 `VMResult`/`output` 移除：`print` 为内置函数（task 25），本 task 不产生输出缓冲；届时再按需引入。

### VM 初始化

```rust
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
```

> **错误类型约定**：`interpret` 返回 `Result<Object, String>`，与已合并的编译器（task 17-19 的 `compile() -> Result<Chunk, String>`）保持一致，作为过渡约定。`src/error.rs` 中 `MspError::RuntimeError(String)` 的统一接入是跨模块的后续工作，不属本 task 范围。

### 栈操作

```rust
impl VM {
    fn push(&mut self, value: Object) -> Result<(), String> {
        if self.stack.len() >= STACK_MAX {
            return Err("stack overflow".to_string());
        }
        self.stack.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Object, String> {
        self.stack.pop().ok_or_else(|| "stack underflow".to_string())
    }

    fn peek(&self, distance: usize) -> Result<&Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack.get(idx).ok_or_else(|| "stack underflow".to_string())
    }

    fn peek_mut(&mut self, distance: usize) -> Result<&mut Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack.get_mut(idx).ok_or_else(|| "stack underflow".to_string())
    }
}
```

> - `push`/`pop`/`peek`/`peek_mut` 均以 `Result` 表达失败，避免 host `panic!`（语言层栈溢出/下溢异常在 task 37 引入；当前以 `Err` 上报）。
> - 索引用 `checked_sub(distance + 1)` + `get`/`get_mut`，杜绝 `distance >= len` 时的 `usize` 下溢与越界（见漏洞 D3）。
> - 修正：原 `pop(&self)` 无法通过借用检查（`Vec::pop` 需 `&mut self`），已改为 `&mut self`。

### 指令读取

```rust
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
```

> - 所有读取返回 `Result` 并用 `code.get(ip)` 边界检查，避免损坏或缺 `HALT` 的字节码导致越界 panic（见风险 C2）。
> - `read_u8` 与 `read_byte` 等价，已合并；1 字节操作数（`slot`/`argc`/`count`）统一用 `read_byte`。
> - `from_be_bytes` 大端序与编译器发射、`src/compiler/opcode.rs` 反汇编器一致（见风险 C6，已核对）。

### 核心指令执行

```rust
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
```

### GC 基础框架（MVP 存根）

位于 `src/gc/mod.rs`（已存在的 crate 根 GC 模块）。仅提供触发阈值与 no-op `collect`，供 `run()` 循环预留 GC 触发点（见下方「核心指令执行」）；真实标记-清除见 task 52。

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

> 仅覆盖本 task 已实现的 opcode（`Constant/Nil/True/False/LoadLocal/StoreLocal/LoadGlobal/StoreGlobal/Pop/Dup/Halt`）。算术/比较/控制流指令见 task 24；`print` 等内置见 task 25；函数调用见 task 27。

1. VM 能执行空程序（仅 `HALT`），返回 `Ok(Object::Nil)`（栈空时 `HALT` 返回 Nil，不 panic）。
2. `CONSTANT` 指令正确加载常量到栈。
3. `LOAD_GLOBAL` / `STORE_GLOBAL` 正确读写全局变量（缺键读取返回 `Nil`）。
4. `LOAD_LOCAL` / `STORE_LOCAL` 正确读写局部变量。
5. `POP` / `DUP` 正确操作栈。
6. 错误路径：未知 opcode 返回 `Err`；非法全局名常量（非 STRING）返回 `Err`；`ip` 越过字节码末尾返回 `Err`。
7. 栈溢出保护：压入超过 `STACK_MAX` 个值时 `push` 返回 `Err("stack overflow")`（非 panic）。

## 测试用例

> 端到端 `.ms` 脚本测试（如 `z = x + y; print(z)`）依赖 task 24（算术 `Add`）、task 25（`print` 内置）、task 27（`CALL`），**不在本 task 范围**。本 task 的可执行验证以「Rust 单元测试」为准（见下）。

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::opcode::OpCode;
    use crate::compiler::Chunk;

    fn compile_and_run(source: &str) -> Result<Object, String> {
        let ast = parse(source).unwrap();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&ast).unwrap();
        let mut vm = VM::new();
        vm.interpret(chunk)
    }

    #[test]
    fn test_empty_program() {
        // 空程序 = 仅 HALT；栈空 → 返回 Nil，不 panic
        assert_eq!(compile_and_run("").unwrap(), Object::Nil);
    }

    #[test]
    fn test_constant_loading() {
        // x = 42（裸赋值 → StoreGlobal）；HALT 后栈空 → Nil
        assert_eq!(compile_and_run("x = 42").unwrap(), Object::Nil);
    }

    #[test]
    fn test_global_store_and_load() {
        // 仅 LoadGlobal/StoreGlobal（不含算术 Add，那是 task 24）
        let mut vm = VM::new();
        let chunk = Compiler::new()
            .compile(&parse("x = 10\ny = x").unwrap())
            .unwrap();
        vm.interpret(chunk).unwrap();
        assert_eq!(vm.globals.get("x"), Some(&Object::Int(10)));
        assert_eq!(vm.globals.get("y"), Some(&Object::Int(10)));
    }

    #[test]
    fn test_unknown_opcode_returns_err() {
        let mut vm = VM::new();
        // 人造非法 opcode 字节 0xFF（超出 Halt=79）
        let chunk = Chunk { code: vec![0xFF], constants: vec![], lines: vec![] };
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
        let chunk = Chunk { code, constants: vec![], lines: vec![] };
        assert!(vm.interpret(chunk).is_err());
    }
}
```
