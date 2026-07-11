# go 关键字与并发执行

## 所属阶段
Phase 7.4 - 并发

## 前置任务
54-channel

## 目标
实现 `go` 关键字，支持启动并发协程，与事件循环和 channel 集成，实现完整的生产者-消费者等并发模式。

## 设计规格

参照 [08-concurrency](../08-concurrency.md) § go 关键字、[11-bytecode-vm](../11-bytecode-vm.md) § GO 指令：

### GO 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `GO` | — | 启动新协程 |

### go 语义

```
go_expr = "go" expression
```

- `go expression`：启动新协程执行表达式（通常为函数调用或闭包）
- 立即返回一个 **JoinHandle** 对象，不等待协程完成（`08-concurrency.md:104`）
- 被启动的协程在事件循环中并发执行
- 协程的返回值可通过 `await handle.join()` 获取（`08-concurrency.md:106`）
- 不持有 JoinHandle 的 go 协程（如 `go fn() { ... }` 不赋值给变量）返回值被丢弃，panic 不会传播到主协程（`08-concurrency.md:129`）

### JoinHandle 方法

| 方法 | 说明 |
|---|---|
| `await handle.join()` | 等待协程完成，返回结果（或抛出异常） |
| `handle.is_done()` | 协程是否已完成 |
| `handle.cancel()` | 请求取消协程（协程在下一个 `await`/channel 操作点终止） |

### 调度策略

- 协作式调度：协程在 `await`、channel 操作时主动让出
- 公平调度：就绪协程按 FIFO 顺序执行
- 无抢占

### defer 交互

`defer` 在协程结束时执行。每个协程维护自己的 defer 栈。

### 程序退出

程序在所有协程完成后退出（主协程 + go 启动的协程）。

## 实现细节

### 1. GO 指令编译

`src/compiler/mod.rs`：

GO 指令无操作数（`11-bytecode-vm.md:189`），仅弹出栈顶可调用对象包装为协程。对于带参数的 go 表达式，编译器生成零参数 thunk 闭包绑定实参：

- `go fn() { ... }()` 编译为：编译闭包表达式 → 发出 `GO`
- `go fn(x) { ... }(expr)` 编译为：生成 thunk `fn() { fn(x) { ... }(expr) }()` → 发出 `GO`
- `go my_func(args)` 编译为：生成 thunk `fn() { my_func(args) }()` → 发出 `GO`

```rust
fn compile_go(&mut self, expr: &Expr) {
    // 将 go 后的表达式包装为零参数 thunk（若已有参数）
    let thunk = self.wrap_as_thunk_if_needed(expr);
    self.compile_expression(&thunk);  // 编译闭包到栈顶
    self.emit(OpCode::GO);
}
```

> **参数传递**（`08-concurrency.md:307-311`）：`go fn(u) { ... }(url)` 中 `url` 通过 thunk 闭包捕获传入。GO 指令始终操作零参数可调用对象，与 `11-bytecode-vm.md:189` 的 `GO | —`（无操作数）一致。

### 2. GO 指令处理

`src/vm/mod.rs`：

```rust
OpCode::GO => {
    let callable = self.stack.pop();

    // 创建 JoinHandle 堆对象（TypeTag::JOIN_HANDLE = 16）
    let handle_obj = alloc_join_handle(JoinHandle::new());
    let Object::Ref(handle_ptr) = &handle_obj else { unreachable!() };

    // 创建新协程（使用 task 53 完整 Coroutine 定义）
    let frame = self.create_call_frame_for(&callable, vec![]);
    let coroutine = Coroutine {
        frame,
        value_stack: Vec::new(),       // 新协程初始空栈
        defer_stack: Vec::new(),
        tlab: TLAB::new(),
        future: None,                  // go 协程非 async fn
        handle: Some(*handle_ptr),     // 关联 JoinHandle（task 53 预留字段）
    };

    // 注册到事件循环就绪队列
    self.event_loop.ready_queue.push_back(coroutine);

    // go 返回 JoinHandle（不是 nil）
    self.stack.push(handle_obj);
}
```

> **Coroutine 结构体**：沿用 task 53 定义（`53-async-await.md:48-55`），不重新定义。GO 创建的协程设 `future: None`、`handle: Some(handle_ptr)`、`value_stack: Vec::new()`。

### 3. JoinHandle 对象

`src/async_runtime/join_handle.rs`：

```rust
struct JoinHandle {
    header: MsObjHeader,                    // type_tag = TypeTag::JOIN_HANDLE(16)
    result: RefCell<Option<Object>>,        // 协程返回值（完成后设值）
    error: RefCell<Option<Object>>,         // 协程异常（panic 时设值）
    done: RefCell<bool>,                    // 是否已完成
    waiters: RefCell<Vec<PausedCoroutine>>, // 等待 join 的协程（await handle.join() 暂停）
    cancel_requested: RefCell<bool>,        // cancel() 请求标志
}
```

（参照 `11-bytecode-vm.md:481-487`。增加 `cancel_requested` 字段供 cancel 机制使用。）

> **waiters 类型说明**：`waiters` 存储 `PausedCoroutine`（与 EventLoop 的 `paused` 列表一致），每个 waiter 的 `waiting_on` 指向此 JoinHandle。协程完成时唤醒所有 waiters。

### 4. JoinHandle 方法

通过 GET_ATTR 方法分派实现：

```rust
// await handle.join()
"join" => {
    // join 返回一个 Future；EventLoop 通过 JoinHandle.waiters 管理等待
    // 简化方案：join() 返回 JoinHandle 自身，AWAIT 指令识别 JoinHandle 类型
    // 若 done=true：直接返回 result（或抛出 error）
    // 若 done=false：暂停当前协程，加入 waiters
    if *handle.done.borrow() {
        if let Some(err) = handle.error.borrow().as_ref() {
            return Err(MspError::Exception(err.clone()));
        }
        Ok(handle.result.borrow().clone().unwrap_or(Object::Nil))
    } else {
        // 暂停当前协程等待 join
        return YieldReason::Awaited(handle_ptr);
    }
}

// handle.is_done()
"is_done" => {
    Ok(Object::Bool(*handle.done.borrow()))
}

// handle.cancel()
"cancel" => {
    *handle.cancel_requested.borrow_mut() = true;
    Ok(Object::Nil)
}
```

> **join 与 AWAIT 集成**：`await handle.join()` 中，`join()` 方法在 `done=false` 时返回 `YieldReason::Awaited(handle_ptr)`。EventLoop 将当前协程加入 `handle.waiters`。当 go 协程完成时（见下方第 5 节），唤醒所有 waiters，将 result 压入恢复协程的值栈。

### 5. 协程完成处理与 JoinHandle 填充

在 task 53 的 EventLoop.run() 中，`YieldReason::Completed(val)` 分支已处理 async fn 协程的 Future resolve。本 task 在同一分支中追加 JoinHandle 填充逻辑：

```rust
YieldReason::Completed(val) => {
    vm.exec_defer();  // 执行协程 defer 栈（LIFO）

    if let Some(future_ptr) = coroutine.future {
        // async fn 协程：resolve Future（task 53 已实现）
        resolve_future(future_ptr, FutureState::Resolved(val.clone()));
        self.wake_waiters(future_ptr, &val);
    }

    if let Some(handle_ptr) = coroutine.handle {
        // go 协程：填充 JoinHandle
        let handle = unsafe { &*handle_ptr };
        *handle.result.borrow_mut() = Some(val);
        *handle.done.borrow_mut() = true;
        // 唤醒所有等待 join 的协程
        self.wake_join_waiters(handle_ptr);
    }
}

YieldReason::Error(err) => {
    vm.exec_defer();  // 执行协程 defer 栈

    if let Some(handle_ptr) = coroutine.handle {
        // go 协程异常：填入 JoinHandle.error，不传播到主协程
        let handle = unsafe { &*handle_ptr };
        let exc_obj = err.to_exception_object();
        *handle.error.borrow_mut() = Some(exc_obj.clone());
        *handle.done.borrow_mut() = true;
        self.wake_join_waiters(handle_ptr);
        // 不 return Err —— 异常被 JoinHandle 捕获，不传播
    } else {
        // 主协程或无 handle 的 async fn 协程：传播错误
        return Err(err);
    }
}
```

> **panic 隔离**（`08-concurrency.md:129`）：有 JoinHandle 的 go 协程 panic 时，异常存入 `handle.error`，不传播到 EventLoop 顶层。调用者通过 `await handle.join()` 获取异常。无 JoinHandle 的 go 协程（不赋值给变量）panic 时，异常被静默丢弃——当前实现中这类协程的 `handle` 为 `Some`（GO 总是创建 JoinHandle），但若 JoinHandle 未被引用则 GC 可回收，异常随之丢失。

> **wake_join_waiters**：遍历 `handle.waiters`，将每个等待协程 move 到 `ready_queue`。若 `handle.error` 非空，将异常对象压入恢复协程的值栈（AWAIT 恢复后抛出）；否则将 `result` 压入。

### 6. EventLoop 集成（增量扩展）

本 task **不重写** task 53 的 `EventLoop::run()`（`53-async-await.md:219-284`）。GO 创建的协程已被 `push_back` 到 `ready_queue`，task 53 的调度循环自动处理它们。仅需在 EventLoop 的 `YieldReason::Completed` 和 `YieldReason::Error` 分支中追加 JoinHandle 填充逻辑（见第 5 节）。

task 53 的协程快照/恢复机制（`snapshot_value_stack` / `restore_value_stack` / `std::mem::take` for defer_stack）对 go 协程同样适用，无需修改。

### 7. cancel 机制

`handle.cancel()` 设 `cancel_requested = true`。被取消的协程在下一个安全点（`AWAIT`、`SEND`、`RECEIVE`，见 `14-gc.md:586`）检测此标志并终止：

```rust
// 在 AWAIT / SEND / RECEIVE 安全点检查中追加：
fn check_cancel(&mut self, coroutine: &Coroutine) -> Option<YieldReason> {
    if let Some(handle_ptr) = coroutine.handle {
        let handle = unsafe { &*handle_ptr };
        if *handle.cancel_requested.borrow() {
            // 注入取消异常，终止协程
            return Some(YieldReason::Error(
                MspError::RuntimeError("coroutine cancelled".into())
            ));
        }
    }
    None
}
```

> **cancel 语义**（`08-concurrency.md:127`）：cancel 不是立即终止——协程在下一个 `await`/channel 操作点（安全点）检查标志后终止。这保证 defer 栈正常执行。

### 8. 公平调度

沿用 task 53 的 `ready_queue: VecDeque<Coroutine>`（`53-async-await.md:40`），`pop_front` 取队首，`push_back` 加入队尾，保证 FIFO。本 task 无需新增调度逻辑。

## GC 安全

### JOIN_HANDLE TypeDescriptor

JoinHandle 为 GC 管理的堆对象（`TypeTag::JOIN_HANDLE = 16`），必须定义 `trace` 函数（参照 `14-gc.md:122-135`、task 53 FUTURE trace `53-async-await.md:106`、task 54 CHANNEL trace）：

```rust
fn trace_join_handle(header: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let handle = unsafe { &*(header as *const JoinHandle) };

    // 1. trace result 中的 Ref
    if let Some(Object::Ref(ptr)) = handle.result.borrow().as_ref() {
        callback(ptr);
    }

    // 2. trace error 中的 Ref（异常实例）
    if let Some(Object::Ref(ptr)) = handle.error.borrow().as_ref() {
        callback(ptr);
    }

    // 3. trace waiters 中每个暂停协程的值栈和闭包
    for waiter in handle.waiters.borrow().iter() {
        for obj in waiter.coroutine.value_stack.iter() {
            if let Object::Ref(ptr) = obj {
                callback(ptr);
            }
        }
        if let Object::Ref(ptr) = &waiter.coroutine.frame.closure {
            callback(ptr);
        }
    }
}
```

- 无 `finalize`（JoinHandle 无 `__del__`）

### 根集扩展

本 task 引入的 VM 状态必须纳入 GC 根集扫描（见 `14-gc.md:606-626`，对比 task 53 `53-async-await.md:340-349`）：

| 新增根集来源 | 扫描内容 |
|---|---|
| JoinHandle 对象的 `result` | `Option<Object>` 中的 `Object::Ref` |
| JoinHandle 对象的 `error` | `Option<Object>` 中的 `Object::Ref`（异常实例） |
| JoinHandle 对象的 `waiters` | 每个等待 join 协程的 `value_stack` Ref + `frame.closure` |
| go 协程的 `handle` | `Coroutine.handle: Option<*mut MsObjHeader>` 指向的 JoinHandle |

> **Coroutine.handle 根集覆盖**：task 53 的根集扫描（`53-async-await.md:346`）已包含 `future`/`handle` 指针。本 task 填充 `handle` 字段后，GC 自动覆盖。JoinHandle 本身通过栈变量（`handle = go ...`）或 `waiters` 中的协程引用保持可达。

### GC 移动对象的指针更新

Minor GC 半空间复制移动 Young 代对象时（见 `14-gc.md:351-359`），以下指针需 forwarding 更新：

- `Coroutine.handle` 指向的 JoinHandle
- JoinHandle `waiters` 中协程的 `value_stack` 所有 `Object::Ref`
- JoinHandle `result` / `error` 中的 `Object::Ref`

### RefCell borrow 约束

JoinHandle 的 `result` / `error` / `done` / `waiters` / `cancel_requested` 均使用 `RefCell`。约束（与 task 53/54 一致）：所有 `borrow_mut()` guard 必须在安全点检查和 `return YieldReason` 之前释放。GC trace 函数使用 `try_borrow()`。

## 验证标准

1. `go` 启动新协程并立即返回 JoinHandle
2. 多个 `go` 协程并发执行
3. 协程通过 channel 正确通信
4. 主协程等待所有 go 协程完成后程序退出
5. 每个 go 协程有独立的 defer 栈
6. 生产者-消费者模式正确工作
7. 死锁检测：所有协程暂停时报错
8. `await handle.join()` 正确获取协程返回值（`08-concurrency.md:125`）
9. `handle.is_done()` 正确反映协程完成状态（`08-concurrency.md:126`）
10. go 协程 panic 时通过 `await handle.join()` 抛出异常（`08-concurrency.md:120`）
11. 无 JoinHandle 引用时 panic 不传播到主协程（`08-concurrency.md:129`）
12. `handle.cancel()` 在下一个安全点终止协程（`08-concurrency.md:127`）

## 测试用例

### test_go_basic.ms

```ms
ch = channel(5)

go fn() {
    for i in range(5) {
        ch <- i
    }
    ch.close()
}()

for item in ch {
    print(item)
}
```

预期输出：
```
0
1
2
3
4
```

### test_go_multiple.ms

```ms
ch = channel(6)

go fn() {
    ch <- "A1"
    ch <- "A2"
}

go fn() {
    ch <- "B1"
    ch <- "B2"
}

# 接收 4 条消息（顺序不确定，但数量正确）
for i in range(4) {
    print(<-ch)
}
```

预期输出（顺序可能不同）：
```
A1
B1
A2
B2
```

### test_go_defer.ms

```ms
go fn() {
    defer print("deferred in goroutine")
    print("goroutine running")
}()

print("main done")
```

预期输出：
```
main done
goroutine running
deferred in goroutine
```

### test_go_producer_consumer.ms

```ms
ch = channel(3)

# 生产者
go fn() {
    for i in range(10) {
        ch <- i
    }
    ch.close()
}()

# 消费者
result = []
for item in ch {
    result.push(item)
}
print(result.length())
print(result)
```

预期输出：
```
10
[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
```

### test_go_wait_all.ms

```ms
ch = channel(3)

go fn() {
    ch <- "worker1 done"
}()

go fn() {
    ch <- "worker2 done"
}()

go fn() {
    ch <- "worker3 done"
}()

# 等待所有 worker 完成
for i in range(3) {
    print(<-ch)
}

print("all workers done")
```

预期输出：
```
worker1 done
worker2 done
worker3 done
all workers done
```

### test_go_join_handle.ms

> 验证 `go` 返回 JoinHandle，`await handle.join()` 获取返回值（`08-concurrency.md:113-121`）。

```ms
fn compute() {
    return 42
}

async fn main() {
    handle = go fn() {
        return compute()
    }

    result = await handle.join()
    print(result)
}

main()
```

预期输出：
```
42
```

### test_go_is_done.ms

> 验证 `handle.is_done()` 反映协程完成状态（`08-concurrency.md:126`）。

```ms
ch = channel(1)

handle = go fn() {
    ch <- "done"
}

# 协程可能尚未完成
print(handle.is_done())

# 接收数据，确保协程完成
val = <-ch

# 现在协程应已完成
print(handle.is_done())
```

预期输出：
```
false
true
```

### test_go_panic_isolation.ms

> 验证 go 协程 panic 通过 JoinHandle 传播，无 JoinHandle 时静默丢弃（`08-concurrency.md:120,129`）。

```ms
async fn main() {
    # 有 JoinHandle：panic 通过 join 传播
    handle = go fn() {
        throw RuntimeError("goroutine failed")
    }

    try {
        await handle.join()
    } except e {
        print("caught: " + str(e))
    }

    # 无 JoinHandle 引用：panic 静默丢弃
    go fn() {
        throw RuntimeError("silent failure")
    }()

    print("main survived")
}

main()
```

预期输出：
```
caught: goroutine failed
main survived
```
