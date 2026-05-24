# async/await 协程

## 所属阶段
Phase 7.1 - 并发

## 前置任务
52-gc

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
struct EventLoop {
    ready_queue: Vec<Coroutine>,
    paused: Vec<PausedCoroutine>,
}
```

### Coroutine

```rust
struct Coroutine {
    frame: CallFrame,
    defer_stack: Vec<DeferEntry>,
}
```

### PausedCoroutine

```rust
struct PausedCoroutine {
    coroutine: Coroutine,
    waiting_on: Gc<Future>,
    frame: CallFrame,
}
```

### 顶层 await

主脚本作为主协程在事件循环中执行。遇到 await 时主协程暂停，事件循环调度其他协程。

### async fn 语义

- `async fn` 调用时**不立即执行**，而是返回 Future 对象
- 只有 `await` 时才触发执行

## 实现细节

### 1. Future 对象

`src/vm/object.rs`：

```rust
enum FutureState {
    Pending,
    Resolved(Object),
    Rejected(String),
}

struct Future {
    state: RefCell<FutureState>,
    waiters: RefCell<Vec<Gc<Coroutine>>>,
}
```

- `state`：当前状态，使用 `RefCell` 允许内部可变性
- `waiters`：等待此 Future 完成的协程列表

### 2. async fn 编译

`src/compiler/mod.rs`：

- 解析器在 Phase 1.5 已处理 `async fn` 语法
- 编译 `async fn` 时：
  1. 编译函数体为普通字节码
  2. 在函数对象上标记 `is_async = true`
  3. 调用 async fn 时，不直接 CALL，而是创建 Future 并返回

### 3. async fn 调用机制

在 VM 的 CALL 指令处理中：

```rust
if function.is_async {
    // 创建 Future
    let future = Future::new();
    let future_gc = Gc::new(future);

    // 创建协程执行函数体
    let coroutine = Coroutine::new(call_frame);
    coroutine.future = Some(future_gc.clone());

    // 将协程加入就绪队列
    vm.event_loop.ready_queue.push(coroutine);

    // 返回 Future 给调用者
    self.stack.push(Object::Future(future_gc));
} else {
    // 普通 CALL
}
```

### 4. AWAIT 指令处理

`src/vm/mod.rs`：

```rust
OpCode::AWAIT => {
    let future = self.stack.pop().expect("stack empty");
    let future = expect_future(&future)?;

    match future.state() {
        FutureState::Resolved(val) => {
            self.stack.push(val);
            // 继续执行当前指令
        }
        FutureState::Rejected(err) => {
            return Err(MspError::RuntimeError(err));
        }
        FutureState::Pending => {
            // 快照当前帧
            let frame_snapshot = self.current_frame().clone();

            // 创建暂停协程
            let paused = PausedCoroutine {
                coroutine: Coroutine {
                    frame: frame_snapshot,
                    defer_stack: self.defer_stack.clone(),
                },
                waiting_on: future.clone(),
                frame: frame_snapshot,
            };
            self.event_loop.paused.push(paused);

            // 切换到下一个就绪协程
            self.switch_to_next_coroutine()?;
        }
    }
}
```

### 5. EventLoop 实现

`src/async_runtime/mod.rs`：

```rust
impl EventLoop {
    fn new() -> Self {
        Self {
            ready_queue: Vec::new(),
            paused: Vec::new(),
        }
    }

    fn run(&mut self, vm: &mut VM) -> Result<Object> {
        // 主协程
        let main = Coroutine::new(vm.current_frame().clone());
        self.ready_queue.push(main);

        while !self.ready_queue.is_empty() || !self.paused.is_empty() {
            // 取出就绪协程执行
            if let Some(coroutine) = self.ready_queue.pop_front() {
                vm.restore_frame(coroutine.frame);
                let result = vm.run_until_yield();

                match result {
                    YieldReason::Completed(val) => {
                        // 协程完成，检查 Future
                        self.wake_waiters(&val);
                    }
                    YieldReason::Awaited(future) => {
                        // 协程暂停
                        self.paused.push(PausedCoroutine { ... });
                    }
                    YieldReason::Error(err) => {
                        return Err(err);
                    }
                }
            } else {
                // 无就绪协程但有暂停协程 → 死锁
                return Err(MspError::RuntimeError("deadlock: all coroutines paused".into()));
            }
        }
        Ok(Object::Nil)
    }

    fn wake_waiters(&mut self, result: &Object) {
        // 检查暂停列表，唤醒等待的协程
        let mut still_paused = Vec::new();
        for paused in self.paused.drain(..) {
            if paused.waiting_on.state() == FutureState::Resolved(_) {
                // 唤醒：将结果放入栈
                self.ready_queue.push(paused.coroutine);
            } else {
                still_paused.push(paused);
            }
        }
        self.paused = still_paused;
    }
}
```

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
    let main_coroutine = Coroutine::new(main_frame);
    self.event_loop.ready_queue.push(main_coroutine);
    self.event_loop.run(self)
}
```

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
