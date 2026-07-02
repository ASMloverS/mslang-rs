# defer 语句

## 所属阶段
Phase 4.4 - 控制流 + 高级语法

## 前置任务
27-call-frame

> defer 核心逻辑（LIFO 顺序、参数声明时求值）不依赖 try/except。
> defer 与 try/except 的交互语义（异常时 defer 也执行）在 Task 37 完成后补充集成测试验证。
> Task 37 的前置任务包含本任务，形成单向依赖。

## 目标
实现 defer 语句，支持在函数返回前按 LIFO 顺序执行延迟调用，参数在声明时求值，异常时也执行。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § defer：

### 语法

```
defer_stmt = "defer" expression
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § defer：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `DEFER` | — | 注册 defer 调用 |
| `EXEC_DEFER` | — | 执行所有 defer（函数返回前） |

### defer 执行规则

1. defer 在函数**返回前**执行（包括正常返回和异常返回）
2. 多个 defer 按 **LIFO**（后进先出）顺序执行
3. defer 的参数在 **defer 声明时**求值，不是在执行时

### 与 RETURN 的关系

参照 [11-bytecode-vm](../11-bytecode-vm.md) § CallFrame，每个 CallFrame 维护 `defer_stack_base`，用于追踪当前帧的 defer 栈范围。RETURN 指令在返回前必须执行 EXEC_DEFER。

## 实现细节

### 1. 解析 defer 语句

`src/parser/statement.rs`：

```rust
fn parse_defer(&mut self) -> Result<Stmt> {
    self.consume(TokenKind::Defer)?;
    let expr = self.parse_expression()?;
    // defer 后面应该是一个函数调用表达式
    Ok(Stmt::Defer { expr })
}
```

defer 后的表达式应为函数调用形式（如 `print("msg")`、`fin.close()`）。

### 2. 编译 defer

`src/compiler/statement.rs`：`Stmt::Defer { expr }` 要求 `expr` 为 **Call 表达式**（形如 `defer f(args...)`），否则编译报错「defer requires a call expression」。

采用**值绑定**方案（满足规则 3：参数在 defer 声明时求值；不依赖 upvalue，规避循环变量单 slot 复用导致的「全部 defer 看到终值」错误——见本节末注）：

```
编译 defer f(arg1, ..., argN):

1. 求值 callee f         → 栈: [f]
2. 依次求值 arg1..argN   → 栈: [f, arg1, ..., argN]   ← 参数在此刻求值（规则 3）
3. emit BUILD_TUPLE count=N+1 → 栈: [tuple(f, arg1, ..., argN)]
4. emit DEFER（无操作数） → VM 弹出 tuple，入当前帧 defer 栈
```

> `DEFER` 操作数编码严格遵循 `11-bytecode-vm.md:161`（无操作数）；callee 与 args 经 `BUILD_TUPLE` 打成单个 tuple Object 传递，无需新增操作数或修改设计文档。

**RETURN 前刷新 defer**：编译端在每个 `RETURN` 前（含函数末尾隐式 `RETURN`、模块顶层返回点）`emit EXEC_DEFER`，由 `EXEC_DEFER` 按规则 2（LIFO）执行当前帧全部 defer。这与本文档「设计规格 → 与 RETURN 的关系」（行 43）一致；`RETURN` handler 本身不再处理 defer（职责分离，避免与 `EXEC_DEFER` 双重执行）。

> **为何不用「闭包 + upvalue」**：`compile_for_in`（`src/compiler/statement.rs:460-526`）的循环变量 `i` 是**单一跨轮复用 slot**（仅 declare 一次，非每轮新 slot）。若 `defer print(i)` 以 upvalue 捕获 `i`，同一 slot 被所有 defer 共享，执行时全部读到循环结束后的终值，直接违反规则 3（对 `with_params()` 会输出 `2,2,2`）。值绑定方案在注册时即把 `i` 的当前值拷进 tuple，彻底规避。

### 3. DEFER 指令实现

`src/vm/mod.rs`：弹出栈顶的 `call_tuple`（编译端已把 callee+args 打成 tuple），入当前帧的 defer 区间。`defer_stack_base`（`src/vm/frame.rs:10` 已存在）已按帧分区，entry 内无需再存 frame_base。

```rust
OpCode::Defer => {
    let call_tuple = self.pop()?;
    self.defer_stack.push(DeferEntry { call_tuple });
}
```

### 4. EXEC_DEFER 指令实现

`EXEC_DEFER` 是函数返回路径上唯一的 defer 刷新点（编译端在每个 `RETURN` 前 emit，见 §2）。按 LIFO 逆序执行当前帧 defer 区间内的全部条目；每个条目的返回值被丢弃（`POP`）。函数返回值此刻在栈顶下方，defer 的 call setup/POP 须保持栈平衡、不得扰动它（满足规则 5）。

```rust
OpCode::ExecDefer => {
    let base = self.call_stack.last().ok_or("no frame".to_string())?.defer_stack_base;
    while self.defer_stack.len() > base {
        let entry = self.defer_stack.pop().unwrap();
        self.run_defer_entry(entry.call_tuple)?;
    }
}

/// 拆开 call_tuple = (callee, arg1, ..., argN)：按序压栈后走标准 CALL 流程，
/// 调用返回值（1 个）立即 POP 丢弃。主 dispatch 的 CALL 分支须抽出公共 helper
/// `call_value(callee, argc)` 供此处复用（task 36 顺带重构 CALL 子流程）。
fn run_defer_entry(&mut self, call_tuple: Object) -> Result<(), String> {
    // call_tuple 须为 tuple Object（编译端 BUILD_TUPLE 保证）。
    let items = match &call_tuple {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            unsafe { read_tuple(*ptr) }
        }
        _ => return Err("internal: defer call_tuple is not a tuple".into()),
    };
    let callee = items[0].clone();
    let argc = items.len() - 1;
    self.push(callee)?;
    for a in &items[1..] {
        self.push(a.clone())?;
    }
    self.call_value(argc)?;   // 复用 CALL 子流程
    self.pop()?;              // 丢弃 defer 调用的返回值
    Ok(())
}
```

> defer 抛异常的完整传播（规则 1/2/4）依赖 task 37 异常机制：`run_defer_entry` 返回 `Err` 时 `EXEC_DEFER` 直接向上传播，此时函数返回值尚未压回栈即被遗弃，自然满足规则 2。`__cause__` 异常链与 unwind 路径在 task 37 接入（见 §6）。

### 5. RETURN 指令修改

defer 执行已由编译端在 `RETURN` 前 emit 的 `EXEC_DEFER` 完成（§4），故 `RETURN` handler 自身不再处理 defer，仅保留既有职责。须在截断值栈**前**关闭本帧开放上值（与 `src/vm/mod.rs:1240` 现有实现一致：close_upvalues 依赖栈区间仍有效）。

```rust
OpCode::Return => {
    let return_value = self.stack.pop().unwrap_or(Object::Nil);
    // defer 已由前置 EXEC_DEFER 执行完毕，本帧 defer 区间为空。
    let old_base = self.call_stack
        .last()
        .ok_or("return outside function".to_string())?
        .stack_base;
    self.close_upvalues_from(old_base);   // 必须在 truncate 前（slot 仍有效）
    self.stack.truncate(old_base);
    self.call_stack.pop();
    self.stack.push(return_value);
    // 顶层帧 RETURN 后无更多调用者帧 → 终止（顶层 defer 已由顶层 EXEC_DEFER 执行）。
    if self.call_stack.is_empty() {
        return Ok(self.stack.pop().unwrap_or(Object::Nil));
    }
}
```

> 规则 5（defer 不能修改返回值）：返回值在 `EXEC_DEFER` 运行前已求值并入栈，defer 无法访问该栈位（非具名变量），修改不到。规则 2（defer 抛异常→丢弃返回值）：见 §4 末注，task 37 落实。

### 6. 异常时的 defer（task 37 集成）

defer 与异常的全部交互（规则 1/3/4，`__cause__` 异常链构建、unwind 路径执行 defer）依赖 task 37 的异常基础设施（`THROW`/`TRY_ENTER`/`CATCH`、异常对象、`find_exception_handler`）。task 36 范围内**不实现**，统一在 task 37 接入：

- `throw_exception`：沿调用栈 unwind 前执行当前帧 defer（LIFO）；
- defer 抛新异常时把原异常附加为新异常的 `__cause__`（规则 1/4）；
- 函数因异常返回时执行 defer 后原异常继续传播（规则 3）；
- `set_cause`：须用 task 37 届时定义的真实异常对象 API。注意 `05-control-flow.md:278` 允许 `throw "string"` 自动包装为 `RuntimeError`——`set_cause` 必须对所有 Error 子类生效，**不得仅限 Instance**（旧版本伪代码 `read_instance_mut` + `TypeTag::INSTANCE` 守卫会在非 Instance 异常上静默丢失异常链，且 `read_instance_mut` 当前在源码中不存在、Instance 属 Phase 5）。

**task 36 范围内**：defer 闭包体不应抛异常（无异常机制）；验证标准第 6/7/8 项（异常交互）在 task 37 完成后补充集成测试（见测试用例末 `[需 task 37]` 段）。

### 7. DeferEntry 结构

```rust
struct DeferEntry {
    call_tuple: Object,   // tuple(callee, arg1, ..., argN)；GC 须作根扫描（见 §9）
}
```

> 不再保留 `frame_base`：`CallFrame.defer_stack_base`（`src/vm/frame.rs:10`）已按帧分区，entry 内冗余。

### 8. 模块顶层 defer

`05-control-flow.md:346` 与 `03-syntax.md:280` 规定：`defer` 在模块顶层使用时，在模块执行完毕时执行（等价于整个模块被包裹在隐式函数中）。mslang 的脚本顶层本身就在一个 entry `CallFrame` 中执行（`src/vm/mod.rs:68` 推入 entry 帧），故顶层 `defer` 自然落入该帧的 defer 区间，顶层 `RETURN`（前置 `EXEC_DEFER`）会按 LIFO 执行它们——无需特殊代码路径，仅需：
- 编译端：顶层编译单元的每个返回点前同样 `emit EXEC_DEFER`；
- 测试覆盖（见测试用例 `// 模块顶层 defer`）。

### 9. GC 与并发注意事项

**GC 根集义务**：`defer_stack` 中每个 `DeferEntry.call_tuple` 持有 tuple（及内部 callee/args），在 defer 执行前必须被 GC 视为根。`src/vm/gc.rs:723` 已留 `// [task 36] defer_stack` TODO：task 36 引入该字段后，Minor GC 的根转发（`forward_slot`）须遍历 `defer_stack` 每个条目的 `call_tuple`（task 37 后可能还有 closure）。task 52 GC 全面启用前（当前 TLAB/堆未启用）此项可延后，但须在 task 52 落实。

**协程私有 defer 栈（Phase 7 前向兼容）**：`08-concurrency.md:288` 与 `11-bytecode-vm.md:468` 要求每个协程维护自己的 `defer_stack`。task 36 采用 VM 全局单 `defer_stack`（Phase 4 MVP），依赖 `defer_stack_base` 按帧分区隔离嵌套调用。Phase 7（task 53）并发接入时须将 `defer_stack` 迁移到 `Coroutine` 字段；本任务的 `defer_stack_base` 分区设计使其可平滑迁移（帧快照时一并复制对应区间）。

## 验证标准

1. defer 在函数返回前执行
2. 多个 defer 按 LIFO 顺序执行
3. defer 参数在声明时求值（非执行时）——须用循环变量场景验证（`with_params` 输出 `2,1,0`）
4. defer 栈与函数帧正确关联，嵌套调用互不干扰
5. 模块顶层 defer 在脚本执行完毕时按 LIFO 执行（§8）
6. [task 37] 异常发生时 defer 也执行（规则 1/3）
7. [task 37] 函数正常返回 + defer 抛异常 → 返回值被丢弃，defer 异常传播（规则 2）
8. [task 37] 多个 defer 抛异常时构建 `__cause__` 异常链（规则 1/4）

> 第 6/7/8 项统一标记 [task 37]：task 36 无异常机制，异常路径在 task 37 集成验证（见 §6）。

## 测试用例

```ms
// test_defer.ms — defer 语句

// 基本 LIFO 顺序
fn example() {
    defer print("first")
    defer print("second")
    defer print("third")
    print("body")
}
example()

// 参数在声明时求值（规则 3）：循环变量 i 在每次 defer 注册时拷贝其当前值
fn with_params() {
    for i in range(3) {
        defer print(i)
    }
}
with_params()

// defer 与 return（规则 5：defer 不能修改返回值）
fn with_return() {
    defer print("deferred")
    return 42
}
result = with_return()
print(result)

// 嵌套函数的 defer 互不干扰
fn outer() {
    defer print("outer defer")
    fn inner() {
        defer print("inner defer")
    }
    inner()
    print("after inner")
}
outer()

// 模块顶层 defer（§8）：脚本结束时按 LIFO 执行
defer print("top defer 1")
defer print("top defer 2")
```

预期输出（task 36 可验证部分）：

```
body
third
second
first
2
1
0
deferred
42
inner defer
after inner
outer defer
top defer 2
top defer 1
```

> `with_params` 输出 `2, 1, 0`：LIFO（规则 2）且每次 defer 捕获注册时的 `i` 值（规则 3）。顶层 defer 在脚本末尾执行，`top defer 2` 先于 `top defer 1`（LIFO）。

```ms
// [需 task 37] defer 在异常时也执行（规则 1/3/4，__cause__ 异常链）
fn with_error() {
    defer print("cleanup")
    throw ValueError("oops")
}

try {
    with_error()
} except ValueError as e {
    print("caught: " + e.message)
}
// 预期（task 37 集成后）：
//   cleanup
//   caught: oops
```
