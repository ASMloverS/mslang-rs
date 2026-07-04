# 生成器与 yield

## 所属阶段
Phase 4.7 - 控制流 + 高级语法

## 前置任务
- 28-closures（闭包 / upvalue 机制；生成器表达式编译为闭包）
- 32-for-in-iterator（FOR_ITER / compile_for_in 基础设施）
- 26-builtins-iterators（ITERATOR / IteratorState / `to_iterator`）
- 37-try-except-finally（异常处理器栈、`throw()`、GeneratorExit 不可被用户捕获）

## 目标
实现生成器函数与 yield/yield from，支持帧快照/恢复机制（基于 task 23/27 已确立的"值栈按帧分段"不变量），生成器对象可作为迭代器在 for..in 中使用。同时实现生成器表达式（圆括号推导式）的编译与执行。

> **解析依赖（已完成，不在本任务范围）**：
> - `yield` / `yield from` 解析在 [15-parser-advanced-statements](./15-parser-advanced-statements.md) § `parse_yield_expr`（`From` 为关键字，无需消歧；`07-advanced.md:185` 的消歧规则针对 `from_module` 单个标识符，由词法分析器整体识别为 `Identifier`）
> - 生成器表达式 AST 节点 `GeneratorExpression { expr, for_clauses, condition }` 在 [09-ast-expression-nodes](./09-ast-expression-nodes.md) 已定义，[14-parser-collection-literals](./14-parser-collection-literals.md) § `parse_generator_expression` 已解析（仅支持显式双层括号 `(expr for x in iter)` 形式）
> - `YIELD` / `YIELD_FROM` / `CLOSE_GENERATOR` opcode 在 [16-opcode-definition](./16-opcode-definition.md) 已定义
> - **未覆盖解析缺口**（B11）：`f(x for x in iter)` 单实参省略外层括号的生成器表达式语法（`07-advanced.md:192` 范例 `sum(x * x for x in range(1000000))`），task 14 的 `parse_generator_expression` 仅在 `parse_grouping_or_tuple`（已消费 `(`）内触发；由 `parse_call` 解析实参时不会构造 `GeneratorExpression`。本任务**不补此解析缺口**，标记为已知遗漏，待后续 task 修复（影响范围仅限语法糖，不影响核心语义）。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 生成器：

### yield 语法

包含 `yield` 的函数自动成为生成器函数。

```ms
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}
```

### 生成器语义

- 调用生成器函数**不执行函数体**，而是返回一个 Generator 对象
- Generator 实现 `__iter__`（返回 self）和 `__next__`（恢复执行）
- `yield expr` 暂停执行并返回 `expr` 的值
- 函数体执行完毕时抛出 `StopIteration`

### yield from

```ms
yield from flatten(item)  // 委托给另一个可迭代对象
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 迭代：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `YIELD` | — | yield 暂停 |
| `YIELD_FROM` | — | yield from 委托 |

### Generator 对象

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 生成器执行模型。**沿用 task 23（vm-core §56）与 task 27（call-frame §360）已确立的"值栈按帧分段管理"不变量**：生成器快照保存的是 VM 主值栈的 `[stack_base..stack_top)` 区间拷贝，**不是**独立栈。Phase 7 async/await（task 53）的 `YieldReason::Generator` 暂停机制与本任务共用同一份快照/恢复代码。

```
MsGenerator {
    header:          MsObjHeader            // TypeTag::GENERATOR = 12
    frame:           CallFrame              // 帧的 ip / stack_base / defer_stack_base（值类型，可拷贝）
    stack_snapshot:  Vec<Object>            // 该帧 [stack_base..stack_top) 区间的快照
    state:           GeneratorState         // 状态
    receiver:        Option<*mut MsObjHeader>   // yield from 子迭代器（MsIterator 或 MsGenerator）
    gen_exit_pending: bool                  // close_generator 注入 GeneratorExit 标志
}

GeneratorState {
    Suspended,
    Running,
    Exhausted,    // 包括自然结束 + 被 close 两种情形，统一用此状态
}
```

> **不再单独设 `Closed` 状态**（A1 修复）：`11-bytecode-vm.md:423-427` 仅定义 3 状态；close 后的生成器视为 `Exhausted`，再次 `__next__()` 抛 StopIteration、再次 close 静默返回。

> **`stack_snapshot` 与标准 `stack`/`locals` 字段的关系**（A2 修复）：标准 `11-bytecode-vm.md:415-421` 列出 `stack` + `locals` 两字段是基于"独立栈"的概念表述；本任务按 task 23/27 的分段栈不变量，统一为 `stack_snapshot`（覆盖整个帧区间，locals 即前缀部分），**不再单列 `locals`**。这是对标准实现策略的具体化，须回写 `11-bytecode-vm.md`（见 §设计规格回写）。

> **`receiver` 字段为标准扩展**（A3 修复）：`11-bytecode-vm.md:415-421` 未列出该字段；本任务须回写标准（见 §设计规格回写），并同步通知 task 52 的 `GENERATOR_DESC.size_base / copy_for_gc` 据此调整。

> **`gen_exit_pending` 标志**：`close_generator` 设置此标志后调用 `resume_generator_with_exception`；恢复执行的首个安全点检查若发现此标志，立即经 task 37 `throw(GeneratorExit)` 路径注入异常。GeneratorExit 不可被用户 `except` 捕获（`05-control-flow.md:238`、task 37 §5 `exception_matches` 显式 `return false`），故会跑完 defer/finally 后再次回到 close 路径，把 state 置 `Exhausted`。

## 实现细节

### 0. 堆对象 MsGenerator（B2 / B3）

`src/vm/object.rs` 新增 `MsGenerator`（参照 task 28 的 `MsUpvalue` 模板）。TypeTag 复用 [20-object-system-basic](./20-object-system-basic.md) §1 中已定义的 `GENERATOR = 12`（`14-gc.md:103` 权威定义）。

```rust
#[repr(C)]
pub struct MsGenerator {
    pub header:           MsObjHeader,
    pub frame:            CallFrame,                      // 拷贝自当前帧（值类型）
    pub stack_snapshot:   Vec<Object>,                    // [stack_base..stack_top) 区间拷贝
    pub state:            GeneratorState,
    pub receiver:         Option<*mut MsObjHeader>,       // yield from 子迭代器（MsIterator / MsGenerator）
    pub gen_exit_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeneratorState { Suspended, Running, Exhausted }

impl MsGenerator {
    pub fn new(frame: CallFrame, stack_snapshot: Vec<Object>) -> Self {
        Self {
            header: MsObjHeader {
                gc_meta: 0,
                type_tag: TypeTag::GENERATOR as u8,
                size: std::mem::size_of::<MsGenerator>() as u16,
                _padding: 0,
                class_ptr: 0,
            },
            frame,
            stack_snapshot,
            state: GeneratorState::Suspended,
            receiver: None,
            gen_exit_pending: false,
        }
    }
}

/// 分配 MsGenerator（TypeTag::GENERATOR）。MVP：Box 分配；task 52-gc 替换为 TLAB bump。
pub fn alloc_generator(gen: MsGenerator) -> Object {
    let boxed = Box::new(gen);
    Object::Ref(Box::into_raw(boxed) as *mut MsObjHeader)
}

/// # Safety
/// `ptr` 必须指向由 `alloc_generator` 分配的、在 `'a` 期间有效的 `MsGenerator`。
pub unsafe fn read_generator<'a>(ptr: *mut MsObjHeader) -> &'a MsGenerator {
    &*(ptr as *mut MsGenerator)
}

/// # Safety
/// 同 read_generator；调用方须保证无其它 `&MsGenerator` 或 `&mut MsGenerator` 同时存活。
pub unsafe fn read_generator_mut<'a>(ptr: *mut MsObjHeader) -> &'a mut MsGenerator {
    &mut *(ptr as *mut MsGenerator)
}
```

> **命名约定（A4 修复）**：本任务下文统一用 task 25-28 既有 API：`self.push(v)` / `self.pop()` / `self.peek(n)` / `self.read_byte()` / `self.read_u16()`。不再使用 `stack_push`/`stack_pop`/`stack_peek`/`call_function` 等非标准名。

### 1. 编译期检测生成器函数

`src/compiler/mod.rs`：

在编译函数体时，检测是否包含 `yield` 或 `yield from` 语句。如果包含，将 Function 标记为 `is_generator: true`。

```rust
struct Function {
    name: String,
    arity: usize,
    code: Vec<u8>,
    constants: Vec<Value>,
    upvalue_count: usize,
    is_generator: bool,    // 新增标记
}
```

编译器在遇到 `yield` 时设置当前编译单元的 `is_generator` 标志，函数编译完成后将该标志写入 Function。

### 2. 生成器函数调用

当 VM 执行 `CALL` 且目标是 `is_generator == true` 的函数时，不立即执行，而是创建 Generator 对象。

> **插入位置**：在 task 28 §9 CALL handler 的 CLOSURE 分支（`28-closures.md:533-549`）读完 `arity` 并完成 `argc != arity` 校验**之后**、`call_stack.push(CallFrame::new(...))` **之前**插入 `is_generator` 预检。这样 task 28 已有的 `argc + 1 <= self.stack.len()` 隐式校验（CALL 进入时栈至少含 callee + argc 个参数）先于本任务代码生效，避免 V1 栈下溢。

```rust
// task 28 CALL handler 内，arity 校验通过后：
let is_generator = unsafe { read_function(func.function_ptr) }.function.is_generator;
if is_generator {
    return self.call_generator(closure_ptr, argc);
}
// 普通 CALL 流程（task 28 原有逻辑）...
```

```rust
fn call_generator(&mut self, closure_ptr: *mut MsObjHeader, argc: u8) -> Result<(), String> {
    // V1 修复：显式栈下溢校验（防御性，即便 task 28 已隐式校验）
    let argc_usize = argc as usize;
    if argc_usize + 1 > self.stack.len() {
        return Err("stack underflow in call_generator".into());
    }

    // 拷贝 callee + args 作为生成器帧的初始值栈（[stack_base..stack_base+argc+1)）
    let callee_idx = self.stack.len() - argc_usize - 1;
    let stack_base = callee_idx;            // 生成器帧的 stack_base = callee 在主栈的位置
    let initial_stack: Vec<Object> = self.stack[callee_idx..callee_idx + argc_usize + 1].to_vec();

    // R6/V6 修复：locals_count 上限校验
    let closure = unsafe { read_closure(closure_ptr) };
    let func = unsafe { read_function(closure.function) };
    const MAX_GENERATOR_LOCALS: usize = 65536;
    if func.function.locals_count > MAX_GENERATOR_LOCALS {
        return Err(format!(
            "generator locals_count {} exceeds MAX_GENERATOR_LOCALS {}",
            func.function.locals_count, MAX_GENERATOR_LOCALS
        ));
    }

    let frame = CallFrame {
        closure: closure_ptr,
        ip: 0,
        stack_base,
        defer_stack_base: self.defer_stack.len(),
    };

    let generator = MsGenerator::new(frame, initial_stack);

    // 弹出 callee + args，压入 Generator
    for _ in 0..=argc_usize { self.stack.pop(); }
    self.push(alloc_generator(generator));
    Ok(())
}
```

### 3. YIELD 指令

> **辅助函数定义（V2 修复）**：本节定义生成器专用辅助函数，统一在此处实现（task 39 范围内）：

```rust
impl VM {
    /// 取当前活动生成器对象的可变引用。
    /// 约定：生成器帧运行时，CallFrame.closure 字段被临时替换为指向 MsGenerator 的 Ref
    /// （而非原 Closure），使主循环 self.call_stack.last().closure 即可定位生成器。
    /// 通过 `frame.gen_owner: Option<*mut MsObjHeader>` 标记（CallFrame 扩展字段，见下文）。
    fn current_generator_mut(&mut self) -> &mut MsGenerator {
        let frame = self.call_stack.last_mut().expect("no frame");
        let gen_ptr = frame.gen_owner.expect("not in generator frame");
        unsafe { read_generator_mut(gen_ptr) }
    }

    /// 恢复生成器：把 stack_snapshot 拷回主栈、push 生成器 CallFrame、置 Running。
    fn push_generator_frame(&mut self, gen_ptr: *mut MsObjHeader) {
        let gen = unsafe { read_generator_mut(gen_ptr) };
        gen.state = GeneratorState::Running;
        let frame = gen.frame.clone();
        let snapshot = std::mem::take(&mut gen.stack_snapshot);
        // 在主栈上重建帧的值栈区间
        let new_base = self.stack.len();
        for v in snapshot { self.stack.push(v); }
        let mut new_frame = frame;
        new_frame.stack_base = new_base;
        new_frame.gen_owner = Some(gen_ptr);
        self.call_stack.push(new_frame);
    }

    /// yield 或 return 时：把当前帧的 [stack_base..stack_top) 拷回生成器快照、pop 帧。
    fn pop_generator_frame(&mut self, gen_ptr: *mut MsObjHeader) {
        let frame = self.call_stack.pop().expect("no generator frame");
        let stack_base = frame.stack_base;
        let snapshot: Vec<Object> = self.stack[stack_base..].to_vec();
        self.stack.truncate(stack_base);
        let gen = unsafe { read_generator_mut(gen_ptr) };
        gen.frame.ip = frame.ip;
        gen.frame.defer_stack_base = frame.defer_stack_base;
        // frame.stack_base 在下次 push_generator_frame 时重设，此处保留旧值无害
        gen.stack_snapshot = snapshot;
    }
}
```

> **CallFrame 字段扩展（spec writeback）**：`CallFrame` 新增 `gen_owner: Option<*mut MsObjHeader>` 字段（`11-bytecode-vm.md:325-331`），普通帧为 `None`，生成器帧为 `Some(gen_ptr)`。须回写标准。

```rust
OpCode::YIELD => {
    let value = self.pop()?;

    // 取生成器对象（在 yield 之前栈顶即产出值；当前帧是生成器帧）
    let gen_ptr = self.call_stack.last().expect("no frame").gen_owner
        .expect("YIELD outside generator frame");

    // V8 修复：经 pop_generator_frame 保存快照（内含 ip / stack 区间）
    self.pop_generator_frame(gen_ptr);

    // V7 修复：删除冗余的 caller_frame.clone()——pop_generator_frame 已恢复调用者帧
    // （生成器帧被 pop 后，call_stack.last() 自然是调用者帧）

    // 置 Suspended（pop_generator_frame 已 pop 帧，state 仍为 Running）
    unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Suspended;

    // 产出值压入调用者栈顶
    self.push(value);
}
```

### 4. __next__ 调用（恢复生成器）

> **恢复入口的双重路径（R5 修复）**：
> - **FOR_ITER 内部循环**（§6）：检测 `state == Exhausted` 时直接 jump 到循环出口，**不抛异常**（for..in 静默退出）。
> - **显式 `gen.__next__()` 调用**（CALL handler 分派）：见 §4a，检测 `state == Exhausted` 时经 `throw()` 抛 `StopIteration`。

```rust
/// 恢复生成器执行。返回 `Ok(Some(value))` 表示 yield 了一个值；
/// `Ok(None)` 表示生成器已结束（state → Exhausted）。
/// 调用方（FOR_ITER 或 __next__）按返回值决定压栈 / 抛 StopIteration / 跳出循环。
fn resume_generator(&mut self, gen_ptr: *mut MsObjHeader) -> Result<Option<Object>, String> {
    let state = unsafe { read_generator(gen_ptr) }.state;
    match state {
        GeneratorState::Exhausted => return Ok(None),   // 调用方决定语义
        GeneratorState::Running => {
            return Err("RuntimeError: generator already executing".into());
        }
        GeneratorState::Suspended => {}
    }

    // 若 close 路径设置了 gen_exit_pending，恢复后第一件事是注入 GeneratorExit
    let inject_exit = unsafe { read_generator(gen_ptr) }.gen_exit_pending;

    // V7 修复：不再 clone caller_frame（push_generator_frame/pop_generator_frame 已正确管理）
    self.push_generator_frame(gen_ptr);

    if inject_exit {
        // V4 修复：见 §resume_generator_with_exception
        let exc = self.alloc_generator_exit_exception();
        // task 37 的 throw() 会找当前生成器帧内的 handler（try/except/finally）
        // GeneratorExit 不可被用户 except 捕获（task 37 §5 exception_matches 返回 false），
        // 故会跑完 defer/finally 后再次抛出，被 close_generator 的内层循环捕获。
        self.throw(exc)?;
        // throw() 若未终止程序，说明生成器内 except 捕获了（不应发生）或 finally 跑完
        return Ok(None);
    }

    // 主循环继续执行直到 YIELD（pop_generator_frame 已保存快照）或 RETURN（generator_return）
    // 复用 task 53 的 run_until_yield 基础设施（R3 修复，见 §task 53 集成说明）
    self.run_until_generator_yield(gen_ptr)
}
```

> **B8 修复（defer 与 yield 的交互）**：
> - **yield**（`OpCode::YIELD`）：仅调用 `pop_generator_frame` 保存快照，**不**执行该帧的 defer（defer 仍在 `defer_stack` 中，`defer_stack_base` 边界保留）。
> - **生成器结束**（`RETURN` 或函数末尾，§7 `generator_return`）：执行 `EXEC_DEFER` 跑当前帧 defer（LIFO），然后 pop 帧。
> - **close 注入 GeneratorExit**：`throw()`（task 37 §7）在 unwind 当前生成器帧时按规则 1/3/4 跑完 defer，链式异常挂 `__cause__`。GeneratorExit 不可捕获保证 defer 必跑。
> - **GC finalizer 路径**（§9）：mutator 线程在 GC 结束后调用 `vm.close_generator(obj)`，与 Instance `__del__` 同路径（task 52 §`run_finalizers`）。

### 4a. 显式 `gen.__next__()` 与 `gen.close()` 的 CALL 分派（B4 / B6）

`src/vm/mod.rs` CALL handler，在 task 28 CLOSURE 分派与 task 37 EXCEPTION_CLASS 分派之间新增 GENERATOR 特判：

```rust
// CALL 分派中，callee 类型检查：
Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 => {
    // 形如 gen.__next__() / gen.close() / gen.__iter__()
    // 由 INVOKE 路径或显式方法查找触发；本任务在 CALL 之前由 GET_ATTR 解析方法名时
    // 返回一个特殊的 BOUND_METHOD（TypeTag::BOUND_METHOD，task 41 落地）。
    // Phase 4.7 阶段 Instance 尚未实现，采用最小特判：
    //   - 编译期：gen.__next__() 仍按 Call(GetAttr(gen, "__next__"), []) 编译
    //   - 运行期 GET_ATTR handler 对 GENERATOR 类型 + 方法名 in {"__next__", "close", "__iter__"}
    //     返回一个特殊 Object::Ref 指向 gen 自身并附 method_id（占位策略，Phase 5 由 BOUND_METHOD 替换）
    unreachable!("CALL on raw GENERATOR should be intercepted by GET_ATTR/BoundMethod")
}
```

> **Phase 4.7 最小可用方案**（不依赖 task 41 BOUND_METHOD）：GET_ATTR handler 增加 GENERATOR 特判分支——
> ```rust
> OpCode::GetAttr => {
>     let name_idx = self.read_u16()?;
>     let obj = self.pop()?;
>     if let Object::Ref(ptr) = &obj {
>         if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 {
>             let name = self.read_string_constant(name_idx)?;
>             return match name.as_str() {
>                 "__iter__" => Ok(()),  // 返回 gen 自身（已在栈底，push 一次副本）
>                 "__next__" | "close" => {
>                     // 用一个 sentinel Closure（preallocated，绑定 gen_ptr 与 method_id）
>                     // 简化方案：直接在 GET_ATTR 内 return，并在 CALL 处对 GENERATOR 特判
>                     self.push(obj.clone())?;   // gen 自身
>                     // method_id 通过 CallFrame 临时字段或额外 opcode 传递（见下）
>                 }
>                 _ => return Err(format!("AttributeError: 'generator' has no attribute '{}'", name)),
>             };
>         }
>     }
>     // 原有 EXCEPTION / INSTANCE 分派（task 37 / 41）...
> }
> ```
> **推荐**：直接新增 `OpCode::InvokeGenMethod`（操作数 `method_id(1)`），编译期 `gen.__next__()` / `gen.close()` 编译为 `LOAD gen; INVOKE_GEN_METHOD 1 / 2`。这样避免 GET_ATTR / BOUND_METHOD 的 Phase 4.7 空窗。具体由实现者选定，但**必须**在验证标准 #2、#10 中分别覆盖 `gen.__next__()` 与 `gen.close()`。

### 5. YIELD_FROM 指令

> **V3 修复（完全重写）**：原方案的 `sub_iter.next()` 把 mslang 运行时对象当 Rust `Iterator` 用——类型混淆，不可编译。正确做法：经 task 26 的 `to_iterator` 把 iterable 转为 `MsIterator`（或对 GENERATOR 直接用），存入 `gen.receiver`，每次恢复时通过 `iterator_next` / `resume_generator` 取下一个值。

```rust
OpCode::YIELD_FROM => {
    let iterable = self.pop()?;
    let gen_ptr = self.call_stack.last().expect("no frame").gen_owner
        .expect("YIELD_FROM outside generator frame");

    // 经 task 26 的 to_iterator 转为迭代器；GENERATOR 类型直接用（生成器本身即迭代器）
    let sub_iter_obj = match &iterable {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 => {
            iterable   // 生成器自身即迭代器
        }
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::ITERATOR as u8 => {
            iterable   // 已是迭代器
        }
        _ => self.to_iterator(iterable)?,   // task 26：list/tuple/dict/set/string/range → MsIterator
    };

    // R4 修复：task 62 并发 GC 启用后，下方赋值是堆引用写入，须经 write_barrier
    // MVP（task 52 STW）无并发标记，本任务不插屏障
    unsafe { read_generator_mut(gen_ptr) }.receiver = match sub_iter_obj {
        Object::Ref(r) => Some(r),
        _ => return Err("yield from requires an iterable".into()),
    };

    // 立即产出第一个值（若子迭代器为空则继续当前生成器）
    self.yield_from_step(gen_ptr)?;
}
```

```rust
/// 从 gen.receiver 取下一个值；有值则按 YIELD 流程产出、耗尽则清 receiver 继续。
/// 在每次 resume_generator 进入主循环后，于 YIELD_FROM 后续指令处调用此函数。
impl VM {
    fn yield_from_step(&mut self, gen_ptr: *mut MsObjHeader) -> Result<(), String> {
        let sub_iter_ptr = unsafe { read_generator(gen_ptr) }.receiver
            .ok_or("internal: yield_from_step with no receiver")?;

        // 根据子迭代器类型取下一个值
        let next: Option<Object> = unsafe { (**sub_iter_ptr).type_tag } {
            tag if tag == TypeTag::ITERATOR as u8 => {
                // task 26 的 iterator_next（不可变借用，返回 Option<Object>）
                self.iterator_next(sub_iter_ptr)?
            }
            tag if tag == TypeTag::GENERATOR as u8 => {
                // 子生成器：调用 resume_generator
                match self.resume_generator(sub_iter_ptr)? {
                    Some(v) => Some(v),
                    None => None,   // 子生成器耗尽
                }
            }
            _ => return Err("yield from receiver corrupted".into()),
        };

        match next {
            Some(value) => {
                // 把 value 当作 YIELD 的产出值（沿用 YIELD handler 逻辑）
                self.pop_generator_frame(gen_ptr);
                unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Suspended;
                self.push(value);
                Ok(())
            }
            None => {
                // 子迭代器耗尽，清 receiver，继续当前生成器（控制流回到 YIELD_FROM 的下一条指令）
                unsafe { read_generator_mut(gen_ptr) }.receiver = None;
                Ok(())
            }
        }
    }
}
```

> **YIELD_FROM 恢复路径**：生成器下次被 `resume_generator` 恢复时，主循环从 YIELD_FROM 的下一条指令继续——编译器在 YIELD_FROM 后立即发射一条 `YIELD_FROM_RESUME` opcode（无操作数），其 handler 调用 `yield_from_step`：若 receiver 仍有值则再次 yield、若已清空则 fall-through 进入生成器体的后续语句。本任务新增 `YIELD_FROM_RESUME` opcode（须回写 `11-bytecode-vm.md`，见 §设计规格回写）。

### 6. FOR_ITER 与 Generator 集成

> **R5 修复**：FOR_ITER 路径**静默退出**（不抛 StopIteration），与显式 `gen.__next__()`（§4a 抛 StopIteration）语义分流。本节仅展示 FOR_ITER 对 GENERATOR 类型的特判分支；其余 list/tuple/string/range 等迭代逻辑由 task 26/32 既有的 `ForIter` handler 处理，本任务**只追加 GENERATOR 分支**。

```rust
// 在 task 32 的 OpCode::ForIter handler 中，iter 类型分派新增分支：
if let Object::Ref(ptr) = &self.peek(0)? {
    if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 {
        let gen_ptr = *ptr;
        match self.resume_generator(gen_ptr)? {
            Some(value) => {
                self.push(value)?;   // 压入循环变量
                // fall-through 到循环体（task 32 既有逻辑）
            }
            None => {
                // 生成器耗尽：弹出 generator、跳到循环出口（不抛 StopIteration）
                self.pop()?;
                let offset = i16::from(self.read_u16()?) as isize;
                self.call_stack.last_mut().unwrap().ip =
                    (self.call_stack.last().unwrap().ip as isize + offset) as usize;
                // 跳过 task 32 handler 的常规迭代路径
                return Ok(());
            }
        }
        // resume 后 yield 值已压栈，跳过 task 32 原有的 iterator_next 调用
        return Ok(());
    }
}
// task 32 的 MsIterator 分支（既有逻辑）...
```

> **B5 修复**：task 26 的 `to_iterator` 须识别 `TypeTag::GENERATOR` 并返回生成器自身（生成器本身即迭代器，无需转换）。在 `to_iterator`（`src/vm/builtins.rs`）的类型 match 中新增分支：
> ```rust
> Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 => obj,
> ```

### 7. 生成器函数结束时

当生成器函数执行到末尾或 `return` 时：

> **B9 修复**：`return expr` 的 `expr` 求值后**丢弃**（与 Python 的 `StopIteration.value` 不同；mslang 当前不支持 `send()` 协议，故返回值无意义）。编译期在生成器函数的 RETURN 前不需要额外处理——直接走标准 RETURN handler，但 RETURN handler 须识别当前是生成器帧（`frame.gen_owner.is_some()`）并走 `generator_return` 路径。

```rust
// 在 task 28 §8 RETURN handler 的开头新增生成器特判：
OpCode::RETURN => {
    let return_value = self.stack.pop().unwrap_or(Object::Nil);

    if let Some(gen_ptr) = self.call_stack.last().expect("no frame").gen_owner {
        // 生成器帧的 RETURN：丢弃返回值（B9），置 Exhausted，pop 帧
        let old_base = self.call_stack.last().unwrap().stack_base;
        self.close_upvalues_from(old_base);   // task 28 §8：truncate 前关闭上值
        // 跑当前帧 defer（B8：defer 在生成器结束时执行）
        self.exec_defer()?;                   // task 36 §EXEC_DEFER
        self.pop_generator_frame(gen_ptr);
        unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Exhausted;
        // 不压入返回值；调用方（resume_generator 的 run_until_generator_yield）检测到
        // 帧已 pop 且 gen.state == Exhausted，返回 Ok(None)
        let _ = return_value;   // 显式丢弃
        return Ok(());
    }

    // 普通 RETURN 流程（task 28 §8 原有逻辑）...
}
```

### 8. 生成器表达式编译

参照 [03-syntax](../03-syntax.md) § gen_expr、[07-advanced](../07-advanced.md) § 生成器表达式：

生成器表达式 `(expr for x in iter if cond)` 在编译时被变换为一个匿名生成器闭包：

> **R7 修复（free variable 捕获）**：表达式中的 `expr` 与 `cond` 可能引用外层词法作用域的变量。**复用 task 28 的 upvalue 机制**——把生成器表达式编译为闭包（外层变量经 upvalue 链访问），`iter` 作为唯一形参传入。具体：调用 task 28 的 `compile_fn_decl` 等价流程（新建 `CompilationUnit`，`parent` 链接外层），`resolve_upvalue` 自动解析 `expr`/`cond` 中的自由变量。
>
> **R8 修复（命名唯一性）**：用单调计数器为每个生成器表达式生成唯一名 `__gen_expr_0`、`__gen_expr_1`、…，避免与 task 28 的 `StoreGlobal` 冲突。`Compiler` 维护 `self.gen_expr_counter: usize`。

> **B7 修复（bare yield 编译）**：编译 `Expr::Yield { value: None }` 时先 emit `OpCode::Nil` 再 emit `OpCode::Yield`；编译 `Expr::Yield { value: Some(e) }` 时按普通表达式编译 `e` 后 emit `Yield`；编译 `Expr::YieldFrom { iterable }` 时编译 `iterable` 后 emit `YIELD_FROM` + `YIELD_FROM_RESUME`。

```rust
fn compile_generator_expression(&mut self, gen_expr: &GeneratorExpression) -> Result<(), String> {
    let unique_name = format!("__gen_expr_{}", self.gen_expr_counter);
    self.gen_expr_counter += 1;

    // 变换为（闭包，捕获外层 upvalue）：
    //   fn __gen_expr_N(iter) {
    //       for x in iter {
    //           if cond { yield expr }
    //       }
    //   }
    // 多个 for_clause 编译为嵌套循环；condition 编译为 if 包裹的 yield。
    self.begin_function(&unique_name, /*arity=*/1, /*is_generator=*/true);
    // 形参 iter 占 slot 0；for_clause 中的 targets 占 slot 1..；多变量解包时按需扩展
    self.compile_gen_expr_body(gen_expr)?;
    self.end_function(/*is_generator=*/true)
}
```

- 生成的函数标记为 `is_generator = true`
- 多个 `for_clause` 编译为嵌套循环
- `condition` 编译为 `if` 包裹的 `yield`

### 9. 生成器关闭与 GC 清理（CLOSE_GENERATOR）

参照 [05-control-flow](../05-control-flow.md) § GeneratorExit、[07-advanced](../07-advanced.md) § 生成器关闭、[11-bytecode-vm](../11-bytecode-vm.md) § CLOSE_GENERATOR：

当生成器被 GC 回收但尚未耗尽（state != Exhausted）时，VM 必须自动注入 `GeneratorExit` 异常并恢复生成器帧执行，以触发 defer/finally 资源清理（如关闭文件句柄）。

#### CLOSE_GENERATOR 指令

```rust
OpCode::CloseGenerator => {
    let gen_obj = self.pop()?;
    if let Object::Ref(ptr) = gen_obj {
        if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 {
            self.close_generator(*ptr)?;
        }
    }
    Ok(())
}
```

#### close_generator 实现

> **A1 修复**：不再使用 `Closed` 状态——已关闭的生成器统一为 `Exhausted`。
> **V5 修复**：`Running` 状态经显式 `gen.close()` 调用须抛 RuntimeError；GC finalizer 路径在调用前自行判断 `state == Suspended`，避免进入此分支。
> **V4 修复**：`resume_generator_with_exception` 见下方单独定义。
> **R1 修复**：finalizer 由 mutator 线程在 GC 结束后执行（task 52 `run_finalizers`），**不**在 STW 期间。
> **R2 修复**：`run_finalizers` 期间设置 `vm.gc_disabled = true` 防止重入 GC。

```rust
fn close_generator(&mut self, gen_ptr: *mut MsObjHeader) -> Result<(), String> {
    let state = unsafe { read_generator(gen_ptr) }.state;
    match state {
        GeneratorState::Exhausted => return Ok(()),    // A1：已耗尽（含已 close），幂等
        GeneratorState::Running => {
            // V5：显式 gen.close() 路径抛错；GC finalizer 不会进入此分支（见 finalize_generator）
            return Err("RuntimeError: generator already executing".into());
        }
        GeneratorState::Suspended => {}
    }

    // 设置 gen_exit_pending 标志，恢复后由 resume_generator 注入 GeneratorExit
    unsafe { read_generator_mut(gen_ptr) }.gen_exit_pending = true;

    // V4 修复：恢复执行并注入异常
    self.resume_generator_with_exception(gen_ptr, "GeneratorExit", "generator closed")?;

    // 恢复后生成器应执行完 defer/finally 并到达 Exhausted 状态
    // 若生成器内部捕获了 GeneratorExit（task 37 §5 保证不可捕获，返回 false），此处防御性检查
    let final_state = unsafe { read_generator(gen_ptr) }.state;
    debug_assert!(final_state == GeneratorState::Exhausted,
                  "generator did not exhaust after GeneratorExit injection");
    Ok(())
}

/// V4 修复：注入指定异常类型并恢复生成器执行。
/// 复用 task 37 §7 throw() 机制：设 frame.current_exc、跳到 except 分派器。
fn resume_generator_with_exception(
    &mut self,
    gen_ptr: *mut MsObjHeader,
    class_name: &str,
    message: &str,
) -> Result<(), String> {
    let exc = self.alloc_exception(class_name, message);

    // push_generator_frame 把生成器快照恢复到主栈并 push CallFrame
    self.push_generator_frame(gen_ptr);

    // 直接调用 task 37 的 throw()：在当前（生成器）帧内找 handler
    // GeneratorExit 不可被用户 except 捕获（task 37 §5 exception_matches 对 "GeneratorExit" 返回 false），
    // 故 throw() 会跑完 defer/finally 后再次抛出，被 close_generator 的调用栈捕获（或传播到顶层）
    self.throw(exc)
}

fn alloc_generator_exit_exception(&mut self) -> Object {
    self.alloc_exception("GeneratorExit", "generator closed")
}
```

#### GC finalizer 钩子

> **R1 修复（重要）**：finalizer 由 **mutator 线程**在 GC 结束后经 `run_finalizers(&mut VM)` 调用（task 52 §`run_finalizers`，参照 `14-gc.md:469-489` Finalize 阶段），**不**在 GC 安全点 / STW 期间执行。原因：close_generator 需要恢复生成器帧执行 defer/finally，可能分配对象、运行用户代码——在 STW 期间做这些会破坏 GC 状态机。
>
> **R2 修复**：task 52 的 `run_finalizers` 在执行 finalizer 队列期间设置 `vm.gc_disabled = true`（或等价标志），防止 close_generator 内部触发重入 GC。finalizer 队列清空后统一复位。
>
> **task 52 集成**：在 task 52 的 `run_finalizers`（`52-gc.md:578-594`）中新增 GENERATOR 分支：
> ```rust
> // run_finalizers 内，对每个 obj：
> if tag == TypeTag::GENERATOR as u8 {
>     let gen = unsafe { read_generator(obj) };
>     if gen.state == GeneratorState::Suspended {
>         vm.close_generator(obj).ok();   // 失败静默（如内部抛未被捕获的异常）
>     }
>     // close 后清 has_finalizer，下次 GC 正常回收
>     header.gc_meta &= !MsObjHeader::HAS_FINALIZER;
>     header.set_color(Color::White);
>     continue;
> }
> ```

```rust
/// GENERATOR 的 finalizer 钩子（C 侧，无 VM 访问）。
/// 实际关闭逻辑由 run_finalizers 在 mutator 线程通过 vm.close_generator(obj) 执行（见上）。
/// 此处仅做防御性清理（state 已是 Exhausted 时无操作）。
fn finalize_generator(obj: *mut MsObjHeader) {
    let gen = unsafe { read_generator(obj) };
    // 若 state == Suspended，由 run_finalizers 调用 vm.close_generator；
    // 若 state == Exhausted，无需操作
    let _ = gen.state;
}
```

#### GC trace / forward_fields / copy_for_gc（B1 修复）

`src/vm/gc.rs`：替换 task 52 的 `trace_generator` 占位（`52-gc.md:228` `// TODO task 39`），并补充 `forward_fields_generator` / `copy_for_gc_generator`：

```rust
/// 遍历 MsGenerator 内所有 Ref 槽：frame.closure + stack_snapshot + receiver。
fn trace_generator(obj: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let gen = unsafe { read_generator(obj) };
    callback(gen.frame.closure);
    for v in gen.stack_snapshot.iter() {
        if let Object::Ref(r) = v { callback(*r); }
    }
    if let Some(r) = gen.receiver { callback(r); }
}

/// Cheney 复制时修正 MsGenerator 内的 Ref 槽指针（Minor GC）。
fn forward_fields_generator(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let gen = unsafe { read_generator_mut(obj) };
    // frame.closure 是裸指针，需单独 forward
    let mut closure_obj = Object::Ref(gen.frame.closure);
    forwarder(&mut closure_obj);
    if let Object::Ref(new_ptr) = closure_obj { gen.frame.closure = new_ptr; }
    // stack_snapshot 中的每个 Object 槽
    for v in gen.stack_snapshot.iter_mut() { forwarder(v); }
    // receiver 裸指针
    if let Some(r) = gen.receiver {
        let mut recv_obj = Object::Ref(r);
        forwarder(&mut recv_obj);
        if let Object::Ref(new_ptr) = recv_obj { gen.receiver = Some(new_ptr); }
    }
}

/// Minor GC 复制：MsGenerator 含 Vec<Object>（堆分配的独立缓冲），不可盲字节拷贝。
/// 重新分配 Vec 并逐元素复制（子 Ref 由 forward_fields 统一修正）。
fn copy_for_gc_generator(src: *mut MsObjHeader, dst: *mut MsObjHeader) -> usize {
    let src_gen = unsafe { read_generator(src) };
    let dst_gen = unsafe { read_generator_mut(dst) };
    dst_gen.frame = src_gen.frame.clone();
    dst_gen.stack_snapshot = src_gen.stack_snapshot.clone();   // Vec 深拷贝
    dst_gen.state = src_gen.state;
    dst_gen.receiver = src_gen.receiver;
    dst_gen.gen_exit_pending = src_gen.gen_exit_pending;
    std::mem::size_of::<MsGenerator>()
}
```

> **`14-gc.md` 同步**：本任务的 `TypeDescriptor` 注册（task 52 `type_descriptor` match `12 => &GENERATOR_DESC`）须把 `trace` / `forward_fields` / `copy_for_gc` / `finalize` 四个字段全部填实（不再为 noop）。`has_finalizer` 标志：所有 MsGenerator 在 `alloc_generator` 时设 `gc_meta |= HAS_FINALIZER`（确保 GC 回收前进入 finalizer 队列）。

## 验证标准

1. 调用生成器函数返回 Generator 对象，不执行函数体
2. **显式 `gen.__next__()`** 恢复执行到下一个 yield；在生成器已耗尽时抛 `StopIteration`（R5：与 FOR_ITER 静默退出区分）
3. for..in 能正确遍历生成器（FOR_ITER 检测 Exhausted 后静默跳出，不抛 StopIteration）
4. 生成器自然结束时 state 置 Exhausted；for..in 与显式 `__next__()` 都能正确感知
5. `yield from` 正确委托给子可迭代对象（list / range / MsIterator / 另一个 Generator）
6. 生成器帧快照/恢复正确保留整个 `[stack_base..stack_top)` 区间（含 locals / 临时值）
7. 无限生成器（如 fibonacci）可惰性求值
8. 生成器表达式 `(x*x for x in range(10))` 返回惰性 Generator 对象
9. 带过滤的生成器表达式 `(x for x in nums if x > 0)` 正确过滤
10. **未耗尽的生成器被 GC 回收时**自动注入 GeneratorExit，触发 defer/finally 清理（mutator 线程、GC 结束后由 `run_finalizers` 调用 `vm.close_generator`）
11. **显式 `gen.close()`**：手动调用立即触发清理（不等 GC）；在 Running 状态调用抛 `RuntimeError: generator already executing`（V5）
12. `gen.close()` 后再调用 `gen.__next__()` 抛 StopIteration（A1：close 后统一为 Exhausted）
13. bare `yield`（无值）等价于 `yield nil`（B7）
14. `return expr` 在生成器中：`expr` 求值后丢弃，state 置 Exhausted（B9）
15. 生成器内的 defer 在 yield 时**不**执行；在生成器结束 / close 时**执行**（B8）
16. 生成器表达式引用外层变量时经 upvalue 正确捕获（R7）
17. 多个生成器表达式在同一作用域使用唯一函数名（R8）
18. `MsGenerator` 的 GC trace 覆盖 `frame.closure` + `stack_snapshot` + `receiver` 三个引用来源（B1）
19. `call_generator` 在 argc 与实际压栈参数数不匹配时返回栈下溢错误（V1）
20. `func.locals_count > MAX_GENERATOR_LOCALS` 时返回错误（V6/R6）

## 设计规格回写（spec writeback）

本任务对设计文档的扩展（参照 task 28 / 37 的回写惯例）：

- **`11-bytecode-vm.md` CallFrame 结构**：新增字段 `gen_owner: Option<*mut MsObjHeader>`（普通帧为 None，生成器帧为 Some(gen_ptr)）。
- **`11-bytecode-vm.md` Generator 结构**：扩展为 `{ frame, stack_snapshot, state, receiver, gen_exit_pending }`（替换原 `{ frame, stack, locals, state }` 概念表述）。明确"值栈按帧分段"实现策略，与 task 23/27 一致。
- **`11-bytecode-vm.md` GeneratorState**：仍为 `Suspended / Running / Exhausted` 3 状态，**不引入 `Closed`**。
- **`11-bytecode-vm.md` 迭代 opcode 表**：新增 `YIELD_FROM_RESUME`（—）一条（YIELD_FROM 的配套恢复指令）。
- **`14-gc.md` TypeDescriptor 表（GENERATOR 行）**：填充 trace / forward_fields / copy_for_gc / finalize 四字段实际实现（替换 task 52 占位 noop）。
- **`14-gc.md` has_finalizer 注册**：所有 MsGenerator 在 `alloc_generator` 时置 `gc_meta |= HAS_FINALIZER`。
- **`14-gc.md` finalizer 队列**：明确 GENERATOR 类型的 finalizer 在 mutator 线程、GC 结束后经 `run_finalizers(&mut VM)` 调用 `vm.close_generator(obj)`（与 Instance `__del__` 同路径）；执行期间 `vm.gc_disabled = true` 防重入。
- **`07-advanced.md`**：无需改动（语义未变）。
- **`05-control-flow.md`**：无需改动。

## 与 task 53（async-await）的集成约定（R3 修复）

本任务的 `run_until_generator_yield(gen_ptr)` 为过渡实现。task 53（async-await）已规划统一的 `run_until_yield(&mut self) -> YieldReason` 机制（见 `53-async-await.md:192`、`55-go-concurrency.md:107`）：

```rust
enum YieldReason {
    Completed(Object),         // 函数 RETURN
    GeneratorYield(Object),    // 生成器 YIELD（本任务对应）
    Awaited(Future),           // task 53：await 暂停
    ChannelSend(Channel),      // task 54：channel 发送阻塞
    ChannelRecv(Channel),      // task 54：channel 接收阻塞
    Error(Object),             // 异常传播
}
```

本任务的过渡方案：

1. `resume_generator` 内调用 `run_until_generator_yield`——内部是 `loop { match self.run_one_instruction()? { Continue => {}, Yield(v) => return Ok(Some(v)), Return => return Ok(None), Throw(e) => return Err(e) } }`
2. task 53 落地时，把 `run_until_generator_yield` 重命名为 `run_until_yield` 并扩展 YieldReason 变体；本任务的 `GeneratorYield` 分支保持兼容。
3. task 53 实现者须在本任务的 `YieldReason::GeneratorYield` 基础上扩展，**不得**重新设计第二套暂停/恢复机制（避免 `11-bytecode-vm.md:334` 所警告的"Phase 7 大规模重构"）。

## 测试用例

```ms
// test_generator.ms — 生成器与 yield

// 基本生成器
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}

for i in countdown(5) {
    print(i)
}

// 手动迭代
fn gen3() {
    yield 10
    yield 20
    yield 30
}

g = gen3()
print(g.__next__())
print(g.__next__())
print(g.__next__())

// 无限生成器
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}

fib = fibonacci()
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())

// yield from
fn flatten(nested) {
    for item in nested {
        if type(item) == "list" {
            yield from flatten(item)
        } else {
            yield item
        }
    }
}

for v in flatten([1, [2, 3], [4, [5, 6]]]) {
    print(v)
}

// 生成器中引用外部变量
fn make_counter(start) {
    n = start
    while true {
        yield n
        n += 1
    }
}

c = make_counter(100)
print(c.__next__())
print(c.__next__())
print(c.__next__())
```

预期输出：

```
5
4
3
2
1
10
20
30
0
1
1
2
1
2
3
4
5
6
100
101
102
```

### test_generator_expression.ms

```ms
# 生成器表达式 — 惰性求值
squares = (x * x for x in range(5))
for s in squares {
    print(s)
}

# 带过滤的生成器表达式
nums = [1, -2, 3, -4, 5]
positives = (x for x in nums if x > 0)
for p in positives {
    print(p)
}
```

预期输出：

```
0
1
4
9
16
1
3
5
```
