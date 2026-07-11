# async/await 协程

## 所属阶段
Phase 7.1 - 并发

## 前置任务
52-gc, 28-closures, 36-defer, 37-try-except-finally

## 目标
实现 async/await 协程系统，包括 Future 对象、AWAIT 指令、EventLoop 事件循环和协程调度机制。

## 设计规格

参照 [08-concurrency](../08-concurrency.md) § async/await、[11-bytecode-vm](../11-bytecode-vm.md) § 异步执行模型：

### Future 对象

Future 状态：
- **Pending** — 未完成
- **Resolved(value)** — 成功完成，持有返回值
- **Rejected(error)** — 失败，持有错误

### AWAIT 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `AWAIT` | — | await Future |

AWAIT 流程：
1. 如果 Future 是 **Resolved**：直接将结果压栈，继续执行
2. 如果 Future 是 **Rejected**：抛出异常
3. 如果 Future 是 **Pending**：快照当前 CallFrame，暂停协程，调度下一个就绪协程

### EventLoop

```rust
use std::collections::VecDeque;

struct EventLoop {
    ready_queue: VecDeque<Coroutine>,  // FIFO：pop_front 取协程，push 追加
    paused: Vec<PausedCoroutine>,
}
```

### Coroutine

```rust
struct Coroutine {
    frame: CallFrame,
    value_stack: Vec<Object>,   // 帧关联的值栈区间 [stack_base..stack_top) 副本
    defer_stack: Vec<DeferEntry>,
    tlab: TLAB,
    future: Option<*mut MsObjHeader>,  // async fn 协程关联的 Future（TypeTag::FUTURE）；普通协程为 None
    handle: Option<*mut MsObjHeader>,  // go 启动时关联的 JoinHandle（TypeTag::JOIN_HANDLE）；task 55 启用
}
```

> - `value_stack`：暂停时必须保存值栈区间（见 `11-bytecode-vm.md` CallFrame 可暂停帧设计），恢复时写回 VM 栈。仅 clone CallFrame 不够——CallFrame 的 `ip`/`stack_base`/`defer_stack_base` 为值类型，但实际 Object 值在 `VM.stack` 中。
> - `future`：async fn 创建的协程完成时，EventLoop 通过此字段找到并 resolve 对应 Future。
> - `handle`：本 task 不使用，预留供 task 55（go 关键字）填充。

### PausedCoroutine

```rust
struct PausedCoroutine {
    coroutine: Coroutine,
    waiting_on: *mut MsObjHeader,  // 指向 MsFuture（TypeTag::FUTURE）
}
```

> `PausedCoroutine` 不单独存储 `frame` 字段——`Coroutine` 已包含 `frame`，无需重复。

### 顶层 await

主脚本作为主协程在事件循环中执行。遇到 await 时主协程暂停，事件循环调度其他协程。

### async fn 语义

- `async fn` 调用时**不立即执行**，而是返回 Future 对象
- 只有 `await` 时才触发执行

## 实现细节

### 1. Future 对象

`src/vm/object.rs`：

Future 为 GC 管理的堆对象（`TypeTag::FUTURE = 13`，含 `MsObjHeader`）。等待者管理由 EventLoop 的 `paused` 列表集中负责，Future 自身不存储等待者列表。

```rust
struct Future {
    header: MsObjHeader,
    state: RefCell<FutureState>,
}

enum FutureState {
    Pending,
    Resolved(Object),
    Rejected(Object),  // 异常对象（Exception 实例），与异常系统一致
}
```

- `header`：统一对象头（`type_tag = TypeTag::FUTURE`）
- `state`：当前状态，使用 `RefCell` 允许内部可变性。`Rejected` 持有异常 Object（而非 String），以便 AWAIT 时直接抛出带类型的异常，与 try/except 类型匹配机制集成

> **FUTURE TypeDescriptor**（`trace` 函数）：遍历 `state` 中的引用——`Resolved(Object::Ref(ptr))` 时 trace `ptr`；`Rejected(Object::Ref(ptr))` 时 trace `ptr`；`Pending` 无引用。无 `finalize`（Future 无 `__del__`）。

> **RefCell 与 GC 安全**：AWAIT 是 GC 安全点（见 `14-gc.md` 安全点位置表）。所有 `state` 的 RefCell borrow guard 必须在安全点检查之前释放，避免 GC trace 时重复 borrow 导致 panic。

### 2. async fn 编译

`src/compiler/mod.rs`：

- 解析器在 Phase 1 已将 `async`/`await`/`go` 识别为关键字 Token（见 `01-lexical.md`）；`async fn` 语法的 AST 构建在 Phase 1 已完成（Function 节点带 `is_async` 标志）
- 编译 `async fn` 时：
  1. 编译函数体为普通字节码
  2. 在函数对象上标记 `is_async = true`
  3. 调用 async fn 时，不直接 CALL，而是创建 Future 并返回

**`await` 作用域校验**（编译期）：`await` 仅允许出现在 `async fn` 函数体内部或脚本顶层（顶层 await，见 `08-concurrency.md`）。编译器维护当前编译单元的 `is_async_context` 标志：
- 进入 `async fn` 函数体 → 设为 `true`
- 脚本顶层编译单元 → 设为 `true`（支持顶层 await）
- 普通 `fn` 函数体 → 设为 `false`
- 遇到 `await` 表达式时若 `is_async_context == false`，报编译错误："`await' outside async function"

### 3. async fn 调用机制

在 VM 的 CALL 指令处理中：

```rust
if function.is_async {
    // 创建 Future（Object::Ref，TypeTag::FUTURE）
    let future_obj = alloc_future(Future::new());
    let Object::Ref(future_ptr) = future_obj.clone() else { unreachable!() };

    // 创建协程执行函数体，关联 Future
    let coroutine = Coroutine {
        frame: call_frame,
        value_stack: self.snapshot_value_stack(),
        defer_stack: Vec::new(),
        tlab: TLAB::new(),
        future: Some(future_ptr),
        handle: None,
    };

    // 将协程加入就绪队列
    vm.event_loop.ready_queue.push_back(coroutine);

    // 返回 Future 给调用者
    self.stack.push(future_obj);
} else {
    // 普通 CALL
}
```

### 4. AWAIT 指令处理

`src/vm/mod.rs`：

```rust
OpCode::AWAIT => {
    let future_val = self.stack.pop().expect("stack empty");
    let future_ptr = expect_future(&future_val)?;  // 返回 *mut MsObjHeader

    // 安全点检查（AWAIT 是 GC 安全点）
    self.check_safepoint();

    // 读取 Future 状态（borrow 在 match 结束后释放，不跨越安全点）
    let state_copy = future_state_clone(future_ptr);  // 克隆状态值，立即释放 borrow
    match state_copy {
        FutureState::Resolved(val) => {
            self.stack.push(val);
            // 继续执行下一条指令
        }
        FutureState::Rejected(exc_obj) => {
            // 直接抛出异常 Object（Exception 实例），复用异常系统
            return Err(MspError::Exception(exc_obj));
        }
        FutureState::Pending => {
            // 返回 YieldReason 让 EventLoop 快照状态并调度下一个就绪协程
            // 暂停状态的快照由 EventLoop 的 Awaited 分支统一完成（避免双重创建）
            return YieldReason::Awaited(future_ptr);
        }
    }
}
```

> **值栈快照**：`snapshot_value_stack()` 提取当前帧的 `[stack_base..stack_top)` 区间副本。恢复时 `restore_value_stack()` 将副本写回 VM 栈并设置 `stack_base`。这是 `11-bytecode-vm.md` CallFrame 可暂停帧设计的要求。

> **defer 栈 move**：使用 `std::mem::take` 将 defer 栈所有权转移到暂停协程，恢复时再 move 回 VM。避免每次 await 深拷贝。

> **GC 根集保护**：`waiting_on` 指向的 Future 必须在协程暂停期间不被回收。EventLoop 的 `paused` 列表中所有 `waiting_on` 指针纳入 GC 根集扫描（见下方"GC 安全"章节）。

### 5. EventLoop 实现

`src/async_runtime/mod.rs`：

```rust
/// run_until_yield 的返回原因
enum YieldReason {
    Completed(Object),          // 协程执行完毕（RETURN），携带返回值
    Awaited(*mut MsObjHeader),  // 协程因 AWAIT Pending 暂停，指向等待的 Future
    Error(MspError),            // 协程抛出异常
    // 注：task 54/55 追加 ChannelSend / ChannelRecv 变体
}

/// VM 执行循环直到协程让出（AWAIT Pending 或 RETURN）
/// 返回 YieldReason 告知 EventLoop 调度决策
fn run_until_yield(&mut self) -> YieldReason { ... }

impl EventLoop {
    fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            paused: Vec::new(),
        }
    }

    fn run(&mut self, vm: &mut VM) -> Result<Object> {
        // 主协程
        let main = Coroutine {
            frame: vm.current_frame().clone(),
            value_stack: vm.snapshot_value_stack(),
            defer_stack: std::mem::take(&mut vm.defer_stack),
            tlab: vm.tlab.take(),
            future: None,
            handle: None,
        };
        self.ready_queue.push_back(main);

        let mut main_result = Object::Nil;

        while !self.ready_queue.is_empty() || !self.paused.is_empty() {
            if let Some(mut coroutine) = self.ready_queue.pop_front() {
                // 恢复协程状态到 VM
                vm.restore_frame(&coroutine.frame);
                vm.restore_value_stack(&coroutine.value_stack);
                vm.defer_stack = std::mem::take(&mut coroutine.defer_stack);
                vm.tlab = coroutine.tlab.take();

                let result = vm.run_until_yield();

                match result {
                    YieldReason::Completed(val) => {
                        // 执行协程的 defer 栈（LIFO）
                        vm.exec_defer();

                        // 如果是 async fn 协程，resolve 其 Future
                        if let Some(future_ptr) = coroutine.future {
                            resolve_future(future_ptr, FutureState::Resolved(val.clone()));
                            // 唤醒等待此 Future 的暂停协程
                            self.wake_waiters(future_ptr, &val);
                        } else {
                            // 主协程或普通协程：记录返回值
                            main_result = val;
                        }
                    }
                    YieldReason::Awaited(future_ptr) => {
                        // 协程因 AWAIT Pending 暂停——快照已由 AWAIT 处理完成
                        // 从 VM 重新提取暂停状态
                        coroutine.frame = vm.current_frame().clone();
                        coroutine.value_stack = vm.snapshot_value_stack();
                        coroutine.defer_stack = std::mem::take(&mut vm.defer_stack);
                        coroutine.tlab = vm.tlab.take();
                        self.paused.push(PausedCoroutine {
                            coroutine,
                            waiting_on: future_ptr,
                        });
                    }
                    YieldReason::Error(err) => {
                        // 执行 defer 后传播错误
                        vm.exec_defer();
                        return Err(err);
                    }
                }
            } else {
                // 无就绪协程但有暂停协程 → 死锁
                return Err(MspError::RuntimeError(
                    "deadlock: all coroutines paused".into(),
                ));
            }
        }
        Ok(main_result)
    }

    /// 唤醒等待指定 Future 的暂停协程
    /// 将 Future 的 resolved 值压入恢复协程的值栈
    fn wake_waiters(&mut self, resolved_future: *mut MsObjHeader, val: &Object) {
        let mut still_paused = Vec::new();
        for paused in self.paused.drain(..) {
            if paused.waiting_on == resolved_future {
                // 将 resolved 值压入协程的值栈快照（AWAIT 恢复后栈顶即为结果）
                let mut coro = paused.coroutine;
                coro.value_stack.push(val.clone());
                self.ready_queue.push_back(coro);
            } else {
                still_paused.push(paused);
            }
        }
        self.paused = still_paused;
    }
}
```

> **Future resolve 机制**：协程完成时（`YieldReason::Completed`），若 `coroutine.future` 存在，调用 `resolve_future` 将 Future 状态设为 `Resolved(val)`，然后调用 `wake_waiters` 唤醒所有等待此 Future 的暂停协程。这是 async/await 闭环的关键——Future 的 Pending → Resolved 转换由协程完成触发。

> **wake_waiters 传值**：被唤醒的协程恢复后将从 AWAIT 指令的下一条继续执行。AWAIT 的 Resolved 路径期望栈顶为结果值。因此在唤醒时将 `val` 压入协程的 `value_stack` 快照，恢复后 VM 栈顶即为正确结果。

> **主协程返回值**：`run()` 返回主协程的完成值（而非固定 `Object::Nil`），与 task 55 保持一致。

### 6. 协程调度策略

- **协作式调度**：协程在 `await` 时主动让出
- **公平调度**：就绪协程按 FIFO 顺序执行
- **无抢占**：没有时间片轮转

### 7. 顶层 await 支持

主脚本作为第一个协程运行。VM 启动时：

```rust
fn run_script(&mut self, source: &str) -> Result<Object> {
    let unit = self.compile(source)?;
    let main_frame = CallFrame::new(unit);
    let main_coroutine = Coroutine {
        frame: main_frame,
        value_stack: Vec::new(),
        defer_stack: Vec::new(),
        tlab: TLAB::new(),
        future: None,
        handle: None,
    };
    self.event_loop.ready_queue.push_back(main_coroutine);
    self.event_loop.run(self)
}
```

## GC 安全

### 根集扩展

本 task 引入的 VM 状态必须纳入 GC 根集扫描（见 `14-gc.md` 根集章节）：

| 新增根集来源 | 扫描内容 |
|---|---|
| `EventLoop.ready_queue` | 每个协程的 `value_stack` 中的 `Object::Ref`、`frame.closure`、`defer_stack` 中的 Ref、`future`/`handle` 指针 |
| `EventLoop.paused` | 每个 PausedCoroutine 的 `coroutine`（同上）+ `waiting_on` 指针 |

> **关键**：`waiting_on` 指向的 Future 对象在协程暂停期间不可被回收。若 Future 唯一的引用来源是 `paused` 列表中的 `waiting_on` 指针，GC 必须通过根集扫描保留该 Future，否则将导致 use-after-free。

### GC 移动对象的指针更新

Minor GC 的半空间复制会移动 Young 代对象。以下裸指针需要由 GC 的 forwarding 机制更新：

- `Coroutine.future` / `Coroutine.handle`
- `PausedCoroutine.waiting_on`
- 协程 `value_stack` 中所有 `Object::Ref`

GC 标记/复制阶段遍历 ready_queue 和 paused 列表时，对这些指针执行 forwarding 指针更新。

### RefCell borrow 约束

`Future.state` 使用 `RefCell`。约束：
- AWAIT 指令处理中的 `state` borrow 必须在安全点检查之前释放
- `resolve_future` 使用 `borrow_mut()`，仅在协程完成时（非 GC 线程）调用
- GC trace 函数访问 `state` 时使用 `try_borrow()`，若失败（mutate 进行中）则将对象标灰待重扫

### 值栈隔离

暂停协程的 `value_stack` 快照中可能包含 `Object::Ref` 指针。这些指针在 GC 根集扫描中被遍历，确保暂停期间其指向的对象不被回收。恢复协程时，快照写回 VM 栈。

## 验证标准

1. `async fn` 调用返回 Future（不立即执行）
2. `await` 正确等待 Future 完成并获取结果
3. 多个协程交替执行（await 时让出）
4. 顶层 await 正常工作
5. Rejected Future 正确抛出异常
6. 事件循环在所有协程完成后退出
7. 死锁检测（所有协程暂停时报错）

## 测试用例

### test_async_basic.ms

```ms
async fn fetch_data() {
    return "data"
}

async fn main() {
    result = await fetch_data()
    print(result)
}

main()
```

预期输出：
```
data
```

### test_async_multiple.ms

```ms
async fn compute(x) {
    return x * 2
}

async fn main() {
    a = await compute(3)
    b = await compute(5)
    print(a + b)
}

main()
```

预期输出：
```
16
```

### test_async_toplevel.ms

```ms
async fn greet(name) {
    return "Hello, " + name
}

# 顶层 await
msg = await greet("World")
print(msg)
```

预期输出：
```
Hello, World
```

### test_async_chain.ms

```ms
async fn step1() {
    return 10
}

async fn step2(x) {
    return x + 5
}

async fn pipeline() {
    a = await step1()
    b = await step2(a)
    return b
}

result = await pipeline()
print(result)
```

预期输出：
```
15
```

### test_async_interleave.ms

验证多个协程在 await 点交替执行（非串行）。

```ms
order = []

async fn task_a() {
    order.push("A1")
    # 此处让出后，task_b 开始执行
    await yield_once()
    order.push("A2")
    return "a"
}

async fn task_b() {
    order.push("B1")
    await yield_once()
    order.push("B2")
    return "b"
}

# 启动两个 async fn（创建 Future + 协程，加入就绪队列）
fa = task_a()
fb = task_b()

# await fa 触发调度：task_a 先执行到 await，让出；task_b 执行到 await，让出
ra = await fa
rb = await fb

print(ra + rb)
print(order)
```

预期输出（order 体现交替）：
```
ab
["A1", "B1", "A2", "B2"]
```

### test_async_rejected.ms

验证 Rejected Future 正确抛出异常，且 try/except 可捕获。

```ms
async fn fail() {
    throw RuntimeError("intentional failure")
}

async fn main() {
    try {
        result = await fail()
        print("should not reach here")
    } except RuntimeError {
        print("caught error")
    }
    print("done")
}

main()
```

预期输出：
```
caught error
done
```

### test_async_deadlock.ms

验证死锁检测：协程间循环 await 导致所有协程暂停且无就绪协程时报错。

```ms
# 两个 async fn 互相 await 对方的 Future（循环等待）
var fa_ref = nil
var fb_ref = nil

async fn coro_a() {
    return await fb_ref
}

async fn coro_b() {
    return await fa_ref
}

fa_ref = coro_a()
fb_ref = coro_b()

# 主协程 await fa_ref → coro_a 运行 → await fb_ref → coro_b 运行
# → await fa_ref → 三个协程全部暂停，无就绪协程 → 死锁
result = await fa_ref
```

预期输出：
```
Error: deadlock: all coroutines paused
```

### test_async_defer.ms

验证协程结束时 defer 正常执行（每个协程独立 defer 栈）。

```ms
async fn worker(id) {
    defer print("worker " + id + " cleanup")
    print("worker " + id + " running")
    return id
}

async fn main() {
    a = await worker("1")
    b = await worker("2")
    print("all done")
}

main()
```

预期输出：
```
worker 1 running
worker 1 cleanup
worker 2 running
worker 2 cleanup
all done
```
