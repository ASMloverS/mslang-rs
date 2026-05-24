# go 关键字与并发执行

## 所属阶段
Phase 7.3 - 并发

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
- 立即返回，不等待协程完成
- 被启动的协程在事件循环中并发执行
- 协程的返回值被丢弃（除非通过 channel 传递）

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

- `go fn() { ... }()` 编译为：
  1. 编译闭包/函数表达式
  2. 发出 `GO` 指令
  3. `GO` 指令将栈顶的可调用对象包装为协程并注册到事件循环

```rust
// 编译 go 表达式
fn compile_go(&mut self, expr: &Expr) {
    self.compile_expression(expr);  // 编译被 go 启动的表达式
    self.emit(OpCode::GO);
}
```

### 2. GO 指令处理

`src/vm/mod.rs`：

```rust
OpCode::GO => {
    let callable = self.stack.pop();

    // 创建新协程
    let frame = self.create_call_frame_for(&callable, vec![]);
    let coroutine = Coroutine {
        frame,
        defer_stack: Vec::new(),
    };

    // 注册到事件循环就绪队列
    self.event_loop.ready_queue.push(coroutine);

    // go 返回 nil（不等待结果）
    self.stack.push(Object::Nil);
}
```

### 3. 协程执行集成

EventLoop 需要支持多协程轮流执行：

```rust
impl EventLoop {
    fn run(&mut self, vm: &mut VM) -> Result<Object> {
        // 主协程
        let main_coroutine = self.ready_queue.pop_front().unwrap();
        let mut main_result = None;

        loop {
            // 从就绪队列取协程
            while let Some(mut coroutine) = self.ready_queue.pop_front() {
                vm.restore_frame(&coroutine.frame);
                vm.defer_stack = coroutine.defer_stack;

                let result = vm.run_until_yield();

                match result {
                    YieldReason::Completed(val) => {
                        // 协程完成
                        // 如果是 async fn，完成其 Future
                        if let Some(future) = &coroutine.future {
                            future.resolve(val);
                        }
                        // 执行协程的 defer 栈
                        vm.exec_defer_for(&mut coroutine.defer_stack);
                    }
                    YieldReason::Awaited(future) => {
                        // 协程因 await 暂停
                        coroutine.frame = vm.snapshot_frame();
                        coroutine.defer_stack = vm.defer_stack.clone();
                        self.paused.push(PausedCoroutine {
                            coroutine,
                            waiting_on: future,
                        });
                    }
                    YieldReason::ChannelSend(channel) => {
                        // 协程因 channel 发送阻塞
                        coroutine.frame = vm.snapshot_frame();
                        channel.waiting_senders.borrow_mut().push(coroutine);
                    }
                    YieldReason::ChannelRecv(channel) => {
                        // 协程因 channel 接收阻塞
                        coroutine.frame = vm.snapshot_frame();
                        channel.waiting_receivers.borrow_mut().push(coroutine);
                    }
                    YieldReason::Error(err) => {
                        // 执行 defer 后传播错误
                        vm.exec_defer_for(&mut coroutine.defer_stack);
                        return Err(err);
                    }
                }
            }

            // 尝试唤醒暂停的协程
            self.check_paused();

            if self.ready_queue.is_empty() && self.paused.is_empty() {
                break; // 所有协程完成
            }

            if self.ready_queue.is_empty() && !self.paused.is_empty() {
                return Err(MspError::RuntimeError("deadlock".into()));
            }
        }

        main_result.unwrap_or(Object::Nil)
    }
}
```

### 4. 协程 defer 栈

每个协程维护独立的 defer 栈：

```rust
struct Coroutine {
    frame: CallFrame,
    defer_stack: Vec<DeferEntry>,
}
```

- 协程正常完成或出错时，执行其 defer 栈（LIFO）
- defer 栈不与其他协程共享

### 5. 公平调度

使用 `VecDeque` 实现就绪队列，`pop_front` 取队首，`push_back` 加入队尾，保证 FIFO：

```rust
ready_queue: VecDeque<Coroutine>,
```

## 验证标准

1. `go` 启动新协程并立即返回
2. 多个 `go` 协程并发执行
3. 协程通过 channel 正确通信
4. 主协程等待所有 go 协程完成后程序退出
5. 每个 go 协程有独立的 defer 栈
6. 生产者-消费者模式正确工作
7. 死锁检测：所有协程暂停时报错

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
