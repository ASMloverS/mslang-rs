# 标准库 - async 模块

## 所属阶段
Phase 7.5 - 并发（标准库）

## 前置任务
53-async-await, 45-module-system

> **依赖说明**：本 task 复用 task 53 的 Future 对象（`MsFuture` / `FutureState`）与 EventLoop 协程调度机制。`async.timeout` 通过 task 53 的 async fn CALL 路径创建子协程（CALL 识别 `Function.is_async` 时自动 spawn 协程，无需 GO 关键字）；故不依赖 task 55。timer 状态以 EventLoop **side table**（`timers: BinaryHeap<TimerEntry>`）形式存储，不修改 `FutureState` 枚举——沿用 task 53 trace 函数。

> **同步设计文档更新**：本 task 同时修订：
> - `05-control-flow.md` 内置异常清单：追加 `TimeoutError`（父类 `Error`）
> - `10-builtins.md` § async：补 `TimeoutError` 语义说明
> - `src/vm/mod.rs`：`BUILTIN_EXCEPTION_NAMES` 追加 `"TimeoutError"`；`EXCEPTION_PARENTS` 追加 `("TimeoutError", "Error")`

## 目标

实现 mslang 标准库 `async` 模块，提供异步定时器和超时控制工具函数。

## 设计规格

参照 [10-builtins](../10-builtins.md) § async、[08-concurrency](../08-concurrency.md) § async/await：

### API 列表

| 函数 | 签名 | 说明 |
|---|---|---|
| `async.sleep(ms)` | `sleep(ms: int) -> Future<nil>` | 异步休眠指定毫秒数（让出协程执行权）；返回 Pending Future，由 EventLoop timer 推进 |
| `async.timeout(fn, ms)` | `timeout(fn: function, ms: int) -> Future<value>` | 带超时执行函数；返回 Pending Future，子协程与定时器竞争 resolve |

### TimeoutError（新增内置异常）

本 task 新增 `TimeoutError`，父类为 `Error`：

- 注册到 `BUILTIN_EXCEPTION_NAMES`（`src/vm/mod.rs:141`）与 `EXCEPTION_PARENTS`（`:123`）
- `async.timeout` 超时时 Future 被 reject 为 `TimeoutError` 实例，调用方通过 `except TimeoutError { ... }` 捕获

### async.sleep(ms)

```ms
import async

async fn delayed_greet() {
    print("waiting...")
    await async.sleep(1000)
    print("done!")
}
```

- 参数 `ms`：休眠毫秒数（int，**非负**，上限 86_400_000 即 24 小时）
- **返回值**：`Future` 对象（`Object::Ref` + `type_tag=FUTURE`，初始状态 `Pending`）
- **调度流程**：
  1. `async_sleep` native 函数分配 Future（Pending），并在 EventLoop 的 `timers` 优先队列注册一条 `TimerEntry { deadline, future_ptr, action: Resolve(Nil) }`
  2. 返回 Future 给调用者
  3. 调用者 `await` 此 Future → AWAIT 指令看到 Pending → 暂停协程（task 53 既有路径）
  4. EventLoop 调度循环每轮 `check_timers`：弹出 `deadline <= now` 的 entries，对每条 `Resolve(val)` 调用 `resolve_future(fp, FutureState::Resolved(val))` + `wake_waiters(fp)`
- **`async.sleep(0)` 语义**：deadline = now，下一轮 `check_timers` 立即 resolve。但因协程已通过 AWAIT 暂停并移入 `paused`，wake_waiters 将其 push 到 `ready_queue` **末尾**——其他就绪协程先运行，实现「主动让出」语义
- **输入校验**：`ms < 0` 或非 Int 类型 → 抛 `TypeError("async.sleep expects non-negative int")`；`ms > 86_400_000` → 截断为 `86_400_000`（避免 `Instant + Duration` 溢出）

### async.timeout(fn, ms)

```ms
import async

async fn risky() {
    await async.sleep(5000)
    return "completed"
}

result = await async.timeout(fn() {
    return await risky()
}, 1000)
```

- 参数 `fn`：可调用对象（async fn / 普通 fn / 闭包），通过 `Object::Ref` + `type_tag=FUNCTION` 识别
- 参数 `ms`：超时毫秒数（int，非负，同 sleep 上限）
- **返回值**：外层 Future（Pending）；其状态由两条路径竞争推进：
  - **子协程先完成**：外层 Future resolve 为子协程返回值；同步移除对应 TimerEntry（标记 cancelled）
  - **定时器先到**：外层 Future reject 为 `TimeoutError`；子协程标记为「孤儿」（结果/异常被丢弃，下次 safepoint 终止）
- **异常传播**：若 `fn` 抛出非 TimeoutError 异常（如 `ValueError`），外层 Future 立即 reject 为该异常，定时器取消——即 fn 内部异常优先于超时
- **`fn` 异步 vs 同步**：`fn` 必须是 async fn 或最终返回 Future 的闭包（内部使用 await）。若 `fn` 是同步函数立即返回值，timeout 退化为「立即完成」，定时器无机会触发——可接受语义

## 实现细节

### 文件位置

- `src/stdlib/async.rs` — `register_async_module` + `async_sleep` / `async_timeout` native 实现
- `src/vm/mod.rs` — `VM::new` 中追加注册调用 + `BUILTIN_EXCEPTION_NAMES` / `EXCEPTION_PARENTS` 扩展 + EventLoop 集成（`timers` 字段、`check_timers` 方法、主循环 `sleep_until` 防忙等）
- `src/vm/object.rs` — 无需改动（沿用 task 53 `MsFuture` / `alloc_future`）

### 注册方式

参照 task 46-51、60 既有模式（`stdlib.rs:27 register_io_module`、`stdlib.rs:410 register_math_module` 等）：

`src/stdlib/async.rs`：

```rust
use crate::vm::object::{alloc_native_function, MsObjHeader, NativeFunction, Object, TypeTag};
use crate::vm::VM;
use std::collections::HashMap;

/// 注册 `async` 内置模块，返回 MsModule 堆对象指针（TypeTag::MODULE）。
/// 参照 register_io_module / register_math_module 既有模式。
pub fn register_async_module() -> *mut MsObjHeader {
    let mut exports = HashMap::new();
    // 裸函数名（与 io/math 等模块风格一致；CALL 通过 native_arities 查表校验）
    exports.insert("sleep".to_string(), alloc_native_function(NativeFunction {
        name: "sleep".to_string(),
        arity: 1,
        func: async_sleep,
    }));
    exports.insert("timeout".to_string(), alloc_native_function(NativeFunction {
        name: "timeout".to_string(),
        arity: 2,
        func: async_timeout,
    }));
    alloc_module("async", exports)
}
```

`src/vm/mod.rs` 的 `VM::new`（参照 `:302-305`、`:368-372` 模式）：

```rust
// task 61：注册原生 async 模块 + 模块函数 arity。
let async_ptr = stdlib::register_async_module();
vm.module_resolver
    .native_modules
    .insert("async".to_string(), async_ptr);
// CALL 按 fn name 查 native_arities；与 sleep(time 模块) / timeout 同名冲突时
// 优先匹配模块前缀调用（async.sleep / async.timeout）。time.sleep 与 async.sleep
// 调用形式不同（time.sleep vs async.sleep），native_arities 共享无歧义。
vm.native_arities.insert("sleep".to_string(), 1);
vm.native_arities.insert("timeout".to_string(), 2);

// 注册 TimeoutError 内置异常（task 61）
// BUILTIN_EXCEPTION_NAMES 追加 "TimeoutError"
// EXCEPTION_PARENTS 追加 ("TimeoutError", "Error")
```

> **命名一致性**：使用裸名 `"sleep"` / `"timeout"`（与 `register_math_module` 用 `"sqrt"` 而非 `"math.sqrt"` 一致）。模块前缀由 `module.fn()` 调用语法隐式提供，native 函数 name 字段无需包含模块名。
>
> **字段路径**：使用 `vm.module_resolver.native_modules`（不是 `vm.builtin_modules`，后者不存在）。返回 `*mut MsObjHeader`（TypeTag::MODULE），不是 `Module` struct。

### async_sleep 实现

```rust
/// async.sleep(ms) native 实现。
/// 返回 Pending Future，并在 EventLoop.timers 注册一条 TimerEntry。
fn async_sleep(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 1 {
        return Err("async.sleep expects 1 argument".to_string());
    }
    let ms = match &args[0] {
        Object::Int(n) => *n,
        _ => return Err("async.sleep expects non-negative int argument".to_string()),
    };
    // 输入校验：负数拒绝；超上限截断（避免 Instant + Duration 溢出）
    if ms < 0 {
        return Err(format!("async.sleep expects non-negative int, got {}", ms));
    }
    const MAX_MS: i64 = 86_400_000; // 24 小时
    let ms_clamped = if ms > MAX_MS { MAX_MS } else { ms };

    // 分配 Pending Future（沿用 task 53 alloc_future，无需扩展 FutureState）
    let future_obj = alloc_future(FutureState::Pending);
    let Object::Ref(future_ptr) = future_obj.clone() else { unreachable!() };

    // 在 EventLoop 的 timer 队列注册一条 Resolve(Nil) entry
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(ms_clamped as u64);
    vm.event_loop.timers.push(std::cmp::Reverse(TimerEntry {
        deadline,
        future_ptr,
        action: TimerAction::Resolve(Object::Nil),
    }));

    Ok(future_obj)
}
```

> **关键修正点**（针对原伪代码漏洞）：
> 1. **`Object::Int` + 负数校验**：避免 `as u64` 回绕导致 deadline 溢出。
> 2. **`alloc_future` 返回 `Object::Ref`**：不使用不存在的 `Object::Future` 变体。
> 3. **timer 状态在 EventLoop side table**：不修改 `MsFuture` 结构体、不扩展 `FutureState`，沿用 task 53 trace 函数。
> 4. **args.len() 防御**：避免索引越界。

### async_timeout 实现

```rust
/// async.timeout(fn, ms) native 实现。
/// 创建外层 Future + 子协程（async fn CALL 路径）+ 竞争 TimerEntry。
fn async_timeout(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err("async.timeout expects 2 arguments".to_string());
    }
    // fn 必须是 Function 堆对象（Object::Ref + type_tag=FUNCTION）
    let func_ptr = match &args[0] {
        Object::Ref(p) => {
            let tag = unsafe { (**p).type_tag };
            if tag != TypeTag::FUNCTION as u8 && tag != TypeTag::CLOSURE as u8 {
                return Err("async.timeout expects function as first argument".to_string());
            }
            *p
        }
        _ => return Err("async.timeout expects function as first argument".to_string()),
    };
    let ms = match &args[1] {
        Object::Int(n) => *n,
        _ => return Err("async.timeout expects non-negative int as second argument".to_string()),
    };
    if ms < 0 {
        return Err(format!("async.timeout expects non-negative int, got {}", ms));
    }
    const MAX_MS: i64 = 86_400_000;
    let ms_clamped = if ms > MAX_MS { MAX_MS } else { ms };

    // 1. 分配外层 Future（Pending）——子协程与定时器竞争 resolve/reject
    let outer_future = alloc_future(FutureState::Pending);
    let Object::Ref(outer_ptr) = outer_future.clone() else { unreachable!() };

    // 2. 创建子协程运行 fn（复用 task 53 CALL 处理 async fn 的内部机制）
    //    构造一个 thunk 调用：thunk = fn() { return await fn_body() }
    //    thunk 关联此 outer_ptr 作为其 future 字段——子协程完成时 EventLoop
    //    在 YieldReason::Completed 分支识别 outer_ptr 并 resolve/reject
    let sub_coro = vm.spawn_timeout_subcoroutine(func_ptr, outer_ptr);
    let sub_coro_handle = sub_coro.handle.expect("timeout subcoro must have handle");
    vm.event_loop.ready_queue.push_back(sub_coro);

    // 3. 注册竞争 timer：到时 reject outer Future 为 TimeoutError
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(ms_clamped as u64);
    vm.event_loop.timers.push(std::cmp::Reverse(TimerEntry {
        deadline,
        future_ptr: outer_ptr,
        action: TimerAction::Reject {
            error_class: "TimeoutError",
            sub_coro_handle: sub_coro_handle,
        },
    }));

    Ok(outer_future)
}
```

> **`spawn_timeout_subcoroutine`**：VM 辅助方法，包装 fn 为带 outer_ptr 关联的子协程。子协程通过 JoinHandle（task 55）与外层 timer 关联——若 timer 先触发，EventLoop 通过 handle 标记 sub-coroutine 为「孤儿」（下个 safepoint 终止）。
>
> **JoinHandle 与 cancel**：本 task 复用 task 55 的 JoinHandle.cancel() 机制。子协程创建时分配 handle（即使外部不持有），timer reject 时调用 `handle.cancel_requested = true`，子协程下个 safepoint（AWAIT/SEND/RECEIVE）检测并终止。
>
> **子协程异常优先**：子协程抛非 TimeoutError 异常时，EventLoop `YieldReason::Error` 分支优先 reject outer Future 为原异常，并从 timers 队列移除对应 entry（按 future_ptr 匹配）。

### 竞争解决规则

| 触发事件 | 外层 Future 终态 | 子协程处理 | TimerEntry 处理 |
|---|---|---|---|
| 子协程 Resolved(val) | `Resolved(val)` | 自然结束 | 从 timers 中移除（按 future_ptr 匹配） |
| 子协程 Rejected(exc) | `Rejected(exc)` | 自然结束 | 从 timers 中移除 |
| Timer 先到（仍在运行） | `Rejected(TimeoutError)` | 标记 cancel_requested，下个 safepoint 终止 | 弹出消费 |

### EventLoop 集成

#### Timer 数据结构

`src/vm/mod.rs`：

```rust
use std::collections::BinaryHeap;
use std::time::Instant;

/// 优先队列按 deadline 升序排列（Reverse 包裹实现最小堆）
struct TimerEntry {
    deadline: Instant,
    future_ptr: *mut MsObjHeader,        // 目标 Future
    action: TimerAction,
}

enum TimerAction {
    /// sleep 到期：resolve Future 为 val
    Resolve(Object),
    /// timeout 到期：reject Future 为指定异常类，并取消关联子协程
    Reject {
        error_class: &'static str,        // "TimeoutError"
        sub_coro_handle: *mut MsObjHeader, // JoinHandle（用于 cancel 子协程）
    },
}

pub struct EventLoop {
    pub ready_queue: std::collections::VecDeque<Coroutine>,
    pub paused: Vec<PausedCoroutine>,
    /// task 61：timer 优先队列（按 deadline 升序）。BinaryHeap 默认最大堆，
    /// 用 Reverse 包裹实现最小堆，pop() 取最早到期的 entry。
    pub timers: BinaryHeap<std::cmp::Reverse<TimerEntry>>,
}
```

#### check_timers 实现

```rust
impl EventLoop {
    /// 弹出到期 timer，执行 action（resolve / reject）并唤醒等待协程。
    /// 必须在 EventLoop 主循环每轮调用。
    fn check_timers(&mut self) {
        let now = Instant::now();
        // 收集到期 entries（避免在循环中持有 self.timers 的 &mut 同时调 wake_waiters）
        let mut due = Vec::new();
        while let Some(std::cmp::Reverse(entry)) = self.timers.peek() {
            if entry.deadline > now { break; }
            let std::cmp::Reverse(entry) = self.timers.pop().unwrap();
            due.push(entry);
        }

        for entry in due {
            let fp = entry.future_ptr;
            // 检查 Future 是否已被其他路径 resolve/reject（子协程先完成的情况）
            let already_settled = {
                let f = unsafe { read_future(fp) };
                !matches!(*f.state.borrow(), FutureState::Pending)
            };
            if already_settled { continue; }

            match entry.action {
                TimerAction::Resolve(val) => {
                    resolve_future(fp, FutureState::Resolved(val));
                }
                TimerAction::Reject { error_class, sub_coro_handle } => {
                    // 构造异常实例（参照 task 37 MsException 创建路径）
                    let exc = alloc_exception(error_class, format!("timeout after deadline"));
                    resolve_future(fp, FutureState::Rejected(exc));
                    // 标记子协程 cancel_requested
                    let handle = unsafe { read_join_handle(sub_coro_handle) };
                    *handle.cancel_requested.borrow_mut() = true;
                }
            }
            // 复用 task 53 wake_waiters 路径，将等待此 Future 的协程 move 到 ready_queue
            self.wake_waiters(fp);
        }
    }

    /// 计算下一个 timer 的 deadline，用于主循环空闲时 sleep_until。
    /// 返回 None 表示无 timer。
    fn next_timer_deadline(&self) -> Option<Instant> {
        self.timers.peek().map(|std::cmp::Reverse(e)| e.deadline)
    }
}
```

> **关键修正点**（针对原伪代码漏洞）：
> 1. **借用分离**：先在 `while let peek` 中收集 `due`（持有 `&mut self.timers`），循环外再迭代 `due` 调用 `wake_waiters`（持有 `&mut self`）。避免 `retain` 闭包内的双重 `&mut self` 借用冲突。
> 2. **不 clone 协程**：完全沿用 task 53 `wake_waiters` 路径（`53-async-await.md:286-301`）——从 `paused` 移出协程 move 到 `ready_queue`，不复制。
> 3. **复用既有 resolve/wake**：调用 `resolve_future` 写入 `FutureState::Resolved/Rejected`，然后 `wake_waiters(fp)` 唤醒所有等待协程（task 53 已实现）。
> 4. **already_settled 检查**：timeout 的 timer 弹出时若 Future 已被子协程完成，跳过 reject。这是竞争情况下的正确处理。
> 5. **BinaryHeap 最小堆**：`Reverse` 包裹 + 标准 `BinaryHeap`（最大堆）实现按 deadline 升序 pop。复杂度 O(log N) push / O(log N) pop，远优于原伪代码的 O(N) `paused.retain`。

#### 主循环防忙等

task 53 `EventLoop::run`（`53-async-await.md:233-282`）的主循环在 `ready_queue.is_empty() && paused.is_empty()` 时退出。但引入 timer 后，`paused` 非空（协程在 await Future）且 `ready_queue` 空（全部等 timer）时，主循环会 busy-spin 调用 `check_timers` 烧 CPU。

修改 `event_loop_run` 主循环开头（参照 `src/vm/mod.rs:606-630`）：

```rust
while !self.event_loop.ready_queue.is_empty() || !self.event_loop.paused.is_empty() {
    // task 61：每轮先推进 timer
    self.event_loop.check_timers();

    let coro = match self.event_loop.ready_queue.pop_front() {
        Some(c) => c,
        None => {
            // 无就绪协程——按需 select / sleep
            if self.try_wake_selects() { continue; }   // task 59

            let all_empty_select = /* 同 task 59 */;
            if all_empty_select && !self.event_loop.paused.is_empty() {
                return Ok(Object::Nil);
            }

            // task 61：若仍有 timer 等待，sleep_until 下个 deadline（防忙等）
            if let Some(deadline) = self.event_loop.next_timer_deadline() {
                let now = Instant::now();
                if deadline > now {
                    std::thread::sleep(deadline - now);
                }
                self.event_loop.check_timers();
                continue;
            }

            return Err("deadlock: all coroutines paused".to_string());
        }
    };
    // ...（原有调度逻辑）
}
```

> **sleep_until 防忙等**：当所有协程都在 timer 上暂停时，主线程 `std::thread::sleep` 直到下个 deadline，CPU 占用为 0。这是 task 61 必须实现的关键性能/正确性修正。
>
> **timer 精度**：sleep_until 后再 check_timers，最坏延迟 = 一次 `thread::sleep` 系统调用唤醒抖动（通常 < 1ms）。可接受。

## GC 安全

### 设计选择：side table 不污染 FutureState

本 task **不修改** `MsFuture` 结构体、不扩展 `FutureState` 枚举——timer 元数据（deadline、action、sub_coro_handle）全部存放在 EventLoop 的 `timers: BinaryHeap<TimerEntry>` 中。这意味着：

- **Future 的 trace 函数无需调整**：task 53 `53-async-await.md:106` 已覆盖 `FutureState::Pending/Resolved(Object::Ref)/Rejected(Object::Ref)` 全部变体。
- **timer entries 不被 GC 直接 trace**：TimerEntry 持有的 `future_ptr` / `sub_coro_handle` 是裸指针，由 EventLoop 作为「运行时根」集中扫描。

### 根集扩展

新增根集来源（参照 `14-gc.md:606-626`、task 53 `53-async-await.md:340-349`）：

| 新增根集来源 | 扫描内容 |
|---|---|
| `EventLoop.timers` | 每个 TimerEntry 的 `future_ptr`（指向 Pending Future）+ `action` 中的 Ref（Resolve(val) 中的 Object::Ref；Reject 的 sub_coro_handle 指向 JoinHandle） |

> **关键不变量**：timer 注册后，对应的 Future 在 `timers` 中被引用；同时若已有协程 await 此 Future，paused.waiting_on 也指向它。两处引用确保 GC 不会误回收 Pending Future。

### GC 移动对象的指针更新

Minor GC 半空间复制移动 Young 代对象时（`14-gc.md:351-359`），以下裸指针需 forwarding 更新：

- TimerEntry.future_ptr（指向 Future）
- TimerEntry.action 中 Resolve(Object::Ref) 的 Ref 指针
- TimerEntry.action 中 Reject.sub_coro_handle（指向 JoinHandle）

GC 标记/复制阶段遍历 `event_loop.timers` 时执行 forwarding 更新。

### RefCell borrow 约束

- `check_timers` 中 `read_future(fp).state.borrow()` guard 在 `match entry.action` 之前必须释放（参照 task 53 `53-async-await.md:362-366`、task 54 `54-channel.md:408-413`）
- `resolve_future` / `alloc_exception` 内部使用 `borrow_mut()`，仅在 EventLoop 单线程上下文调用
- GC trace 函数访问 `timers` 中 future_ptr 时使用 `try_borrow()`，失败时标灰待重扫

### TimerEntry 生命周期

- sleep 类 timer：fire 时被 pop 消费，不再持有 future_ptr 引用
- timeout 类 timer：可能被子协程先完成而提前移除——EventLoop 在 `YieldReason::Completed/Error` 分支检测到 outer Future 已被 settle 时，须扫描 `timers` 移除匹配 entry（按 future_ptr）

> **移除复杂度**：BinaryHeap 不支持随机删除。两种实现策略：
> - **A（推荐）**：lazy 删除——entry 保留在堆中，弹出时通过 `already_settled` 检查跳过。代价是堆中可能堆积 stale entries（每个 timeout 最多 1 条），可接受。
> - **B**：维护 `cancelled_timers: HashSet<*mut MsObjHeader>`，弹出时检查是否在集合中。复杂度更高但堆更紧凑。

## 验证标准

1. `await async.sleep(100)` 正确暂停约 100ms 后恢复
2. `await async.sleep(0)` 立即让出执行权
3. `await async.timeout(fn, 5000)` 在函数正常完成时返回结果
4. `await async.timeout(fn, 10)` 在超时时抛出 `TimeoutError`
5. `await async.sleep(0)` 让出执行权给其他就绪协程（不立即恢复，排到 ready_queue 末尾）
6. 多协程 sleep 不同时长时按 deadline 顺序唤醒（短 sleep 先恢复）
7. `async.timeout` 内 fn 抛非 TimeoutError 异常时，外层 Future reject 为原异常（不被超时吞掉）
8. 超时触发后子协程被 cancel（下个 safepoint 终止，defer 正常执行）
9. `async.sleep(-1)` 抛 `TypeError`
10. `async.sleep(10**20)` 不溢出（截断为 24h 上限）
11. `async.timeout(fn, -1)` 抛 `TypeError`
12. EventLoop 全部协程在 timer 上暂停时不 busy-loop（CPU 占用接近 0）
13. 多次 sleep 的 GC 安全：sleep 期间触发的 GC 不回收 Pending Future（根集覆盖 timer 队列）

## 测试用例

### test_async_sleep_basic.ms

```ms
import async

async fn test_sleep() {
    print("before sleep")
    await async.sleep(100)
    print("after sleep")
}

await test_sleep()
```

预期输出：
```
before sleep
after sleep
```

### test_async_sleep_zero.ms

> 验证 sleep(0) 让出执行权（不立即恢复）。

```ms
import async

order = []

async fn yielder() {
    order.push("A1")
    await async.sleep(0)
    order.push("A2")
}

async fn other() {
    order.push("B")
}

f1 = yielder()
f2 = other()

await f1
await f2

print(order)
```

预期输出：`["A1", "B", "A2"]`（A1 后 yielder 让出，B 先执行，A2 最后）

### test_async_sleep_order.ms

> 验证多 timer 按 deadline 顺序唤醒。

```ms
import async

results = []

async fn sleeper(name, ms) {
    await async.sleep(ms)
    results.push(name)
}

# 三个 sleep：100ms / 50ms / 200ms，应按 50 → 100 → 200 顺序唤醒
f1 = sleeper("long", 200)
f2 = sleeper("short", 50)
f3 = sleeper("mid", 100)

await f1
await f2
await f3

print(results)
```

预期输出：`["short", "mid", "long"]`

### test_async_timeout_success.ms

```ms
import async

async fn compute() {
    await async.sleep(20)
    return 42
}

result = await async.timeout(fn() {
    return await compute()
}, 5000)

print(result)
```

预期输出：`42`

### test_async_timeout fires.ms

```ms
import async

try {
    await async.timeout(fn() {
        await async.sleep(10000)
    }, 50)
    print("should not reach")
} except TimeoutError {
    print("timed out as expected")
}
```

预期输出：`timed out as expected`

### test_async_timeout_fn_exception.ms

> 验证 fn 内部异常优先于 timeout。

```ms
import async

try {
    await async.timeout(fn() {
        throw ValueError("bad input")
    }, 5000)
} except ValueError {
    print("caught ValueError")
} except TimeoutError {
    print("should not be TimeoutError")
}
```

预期输出：`caught ValueError`

### test_async_sleep_negative.ms

> 验证输入校验。

```ms
import async

try {
    await async.sleep(-1)
    print("should not reach")
} except TypeError {
    print("caught TypeError")
}
```

预期输出：`caught TypeError`

### test_async_sleep_no_busy_loop.ms

> 验证 EventLoop 空闲时 sleep_until 而非忙等。
> 测试通过外部观测 CPU 占用——单测中改为验证总执行时间近似 sleep 时长（不允许多倍）。

```ms
import async

async fn main() {
    start = time.now()
    await async.sleep(200)
    elapsed = (time.now() - start) * 1000
    # 允许 ±50ms 抖动
    assert(elapsed >= 200 and elapsed < 300, "sleep took too long: " + str(elapsed))
    print("ok")
}

await main()
```

预期输出：`ok`
