# Channel 通信

## 所属阶段
Phase 7.2 - 并发

## 前置任务
53-async-await

## 目标
实现 Channel 对象，支持有缓冲和无缓冲模式，提供协程间通信机制，包括发送、接收、关闭和遍历操作。

## 设计规格

参照 [08-concurrency](../08-concurrency.md) § Channel、[11-bytecode-vm](../11-bytecode-vm.md) § CHANNEL/SEND/RECEIVE 指令：

### Channel 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CHANNEL` | `buffer_size(1)` | 创建 channel |
| `SEND` | — | channel 发送（`ch <- value`） |
| `RECEIVE` | — | channel 接收（`value = <-ch`） |

### 无缓冲 Channel

- 发送方和接收方**同步进行**
- 发送方阻塞直到有接收方
- 接收方阻塞直到有发送方

### 有缓冲 Channel

- 内部队列，容量为 `buffer_size`
- 缓冲区未满时，发送不阻塞
- 缓冲区非空时，接收不阻塞
- 缓冲区满时，发送阻塞
- 缓冲区空时，接收阻塞

### Channel 操作

| 操作 | 语法 | 说明 |
|---|---|---|
| 创建 | `ch = channel(n)` | 创建缓冲区大小为 n 的 channel |
| 发送 | `ch <- value` | 发送值到 channel |
| 接收 | `value = <-ch` | 从 channel 接收值 |
| 关闭 | `ch.close()` | 关闭 channel |
| 检查 | `ch.closed()` | 检查是否关闭 |
| 遍历 | `for val in ch { ... }` | 接收直到关闭 |

### 关闭语义

- 发送方可关闭 channel 表示不再发送数据
- 接收方可继续接收缓冲区中剩余的数据
- 从已关闭的空 channel 接收返回 `nil`
- 向已关闭的 channel 发送抛出运行时错误

## 实现细节

### 1. Channel 对象

`src/async_runtime/channel.rs`：

```rust
enum ChannelState {
    Open,
    Closed,
}

struct Channel {
    header: MsObjHeader,                                   // type_tag = TypeTag::CHANNEL(14)
    buffer: RefCell<VecDeque<Object>>,
    capacity: usize,
    state: RefCell<ChannelState>,
    waiting_senders: RefCell<VecDeque<WaitingSender>>,
    waiting_receivers: RefCell<VecDeque<WaitingReceiver>>,
}

struct WaitingSender {
    coroutine: Coroutine,   // 暂停期间由 channel 持有（move 语义，参照 task 53 EventLoop）
    value: Object,           // 待发送的值（从发送方栈弹出后存于此）
}

struct WaitingReceiver {
    coroutine: Coroutine,   // 暂停期间由 channel 持有
}
```

- `header`：统一对象头（`type_tag = TypeTag::CHANNEL`），GC 管理的堆对象
- `buffer`：内部队列
- `capacity`：缓冲区容量（0 为无缓冲）
- `state`：通道状态（Open/Closed）
- `waiting_senders`：等待发送的协程及其待发送值（FIFO 队列）
- `waiting_receivers`：等待接收的协程（FIFO 队列）

> **Coroutine 所有权模型**（与 task 53 一致）：Coroutine 为普通 struct（非 GC 堆对象，无 TypeTag，见 `14-gc.md` TypeTag 枚举）。协程在不同状态间通过 move 转移所有权：`ready_queue` → 运行 → channel 等待列表 / `paused` → `ready_queue`。channel 的等待列表直接持有 Coroutine 值，不使用裸指针。

### 2. CHANNEL 指令

`src/vm/mod.rs`：

```rust
OpCode::CHANNEL => {
    let buffer_size = self.read_byte() as usize;
    // buffer_size == 0 → 无缓冲 channel（发送方和接收方同步）
    let channel = Channel::new(buffer_size);
    // 通过堆分配为 Channel 对象（Ref + TypeTag::CHANNEL），参照 Task 20 对象模型
    self.stack.push(alloc_channel(channel));
}
```

### 3. SEND 指令

编译器将 `ch <- value` 编译为：`value` → `channel` → `SEND`

```rust
OpCode::SEND => {
    // 安全点检查（SEND 是 GC 安全点，见 14-gc.md 安全点位置表）
    self.check_safepoint();

    let value = self.stack.pop();
    let channel_obj = self.stack.pop();
    let channel_ptr = expect_channel(&channel_obj)?;  // *mut MsObjHeader
    let channel = unsafe { &mut *channel_ptr };

    if channel.is_closed() {
        return Err(MspError::RuntimeError("send on closed channel".into()));
    }

    if channel.capacity == 0 {
        // 无缓冲：尝试匹配等待的接收者（rendezvous）
        let matched = {
            let mut receivers = channel.waiting_receivers.borrow_mut();
            if let Some(receiver) = receivers.pop_front() {
                // 直接将值传递给接收者：压入接收者协程的值栈快照
                // 接收者恢复后从 RECEIVE 下一条指令继续，栈顶即为此值
                receiver.coroutine.value_stack.push(value.clone());
                self.event_loop.ready_queue.push_back(receiver.coroutine);
                true
            } else {
                false
            }
        }; // RefCell borrow guard 在此释放
        if !matched {
            // 无接收者，暂停当前协程
            return YieldReason::ChannelSend { channel: channel_ptr, value };
        }
    } else {
        // 有缓冲
        let pushed = {
            let mut buffer = channel.buffer.borrow_mut();
            if buffer.len() < channel.capacity {
                // 写屏障（SEND 需写屏障，见 14-gc.md 写屏障插入点表）
                self.write_barrier(&mut buffer, value.clone());
                buffer.push_back(value);
                true
            } else {
                false
            }
        }; // RefCell borrow guard 在此释放
        if !pushed {
            // 缓冲区满，暂停当前协程
            return YieldReason::ChannelSend { channel: channel_ptr, value };
        }
    }
}
```

> **RefCell borrow 约束**（与 task 53 `53-async-await.md:108` 一致）：所有 `buffer` / `waiting_senders` / `waiting_receivers` 的 `borrow_mut()` guard 必须在 `return YieldReason` 之前释放。使用 `{ ... }` 块限定 guard 生命周期，避免暂停期间跨协程访问导致 `RefCell` panic。
>
> **YieldReason::ChannelSend**：携带 `channel` 指针和待发送的 `value`。EventLoop 接收后快照当前协程，创建 `WaitingSender { coroutine, value }` 并 push 到 channel 的 `waiting_senders`（见下方"EventLoop 集成"章节）。此变体扩展了 task 39 `39-generator-yield.md:767` 的占位定义。

### 4. RECEIVE 指令

编译器将 `<-ch` 编译为：`channel` → `RECEIVE`

```rust
OpCode::RECEIVE => {
    // 安全点检查（RECEIVE 是 GC 安全点，见 14-gc.md 安全点位置表）
    self.check_safepoint();

    let channel_obj = self.stack.pop();
    let channel_ptr = expect_channel(&channel_obj)?;
    let channel = unsafe { &mut *channel_ptr };

    // 1. 先尝试从缓冲区取值（有缓冲 channel）
    let from_buffer = {
        let mut buffer = channel.buffer.borrow_mut();
        buffer.pop_front()
    }; // RefCell borrow guard 释放

    if let Some(val) = from_buffer {
        // 缓冲区有数据：如果存在等待的发送者，将其数据移入缓冲区并唤醒
        let woken_sender = {
            let mut senders = channel.waiting_senders.borrow_mut();
            senders.pop_front()
        }; // guard 释放
        if let Some(sender) = woken_sender {
            // 发送者的值进入缓冲区（腾出空位）
            let mut buffer = channel.buffer.borrow_mut();
            buffer.push_back(sender.value);
            drop(buffer); // guard 释放后再操作 EventLoop
            // 唤醒发送者协程（其 SEND 已完成）
            self.event_loop.ready_queue.push_back(sender.coroutine);
        }
        self.stack.push(val);
    } else if channel.is_closed() {
        // 已关闭且缓冲区为空
        self.stack.push(Object::Nil);
    } else {
        // 2. 无缓冲或缓冲区空：检查是否有等待的发送者（rendezvous）
        let woken_sender = {
            let mut senders = channel.waiting_senders.borrow_mut();
            senders.pop_front()
        }; // guard 释放
        if let Some(sender) = woken_sender {
            // 直接从发送者获取值，唤醒发送者
            self.stack.push(sender.value);
            self.event_loop.ready_queue.push_back(sender.coroutine);
        } else {
            // 无等待发送者，暂停当前协程
            return YieldReason::ChannelRecv { channel: channel_ptr };
        }
    }
}
```

> **YieldReason::ChannelRecv**：携带 `channel` 指针。EventLoop 接收后快照当前协程，创建 `WaitingReceiver { coroutine }` 并 push 到 channel 的 `waiting_receivers`。当后续有发送者匹配或 channel 关闭时，接收者被唤醒。
>
> **无缓冲 channel rendezvous 路径**：RECEIVE 在缓冲区为空时检查 `waiting_senders`。若有等待发送者，直接取其 `value`，压入当前栈，并唤醒发送者协程。这是无缓冲 channel 同步交接的核心路径——与 SEND 中匹配 `waiting_receivers` 的路径互补，保证无论发送方还是接收方先到达，值都能正确交接。

### 5. Channel 方法

`close()`、`closed()` 通过方法分派实现：

```rust
"close" => {
    // 幂等：重复 close 不报错（08-concurrency.md:221）
    channel.state.replace(ChannelState::Closed);

    // 唤醒所有等待的接收者（它们恢复后会从缓冲区取剩余数据，
    // 缓冲区空时 RECEIVE 返回 nil）
    let drained_receivers: Vec<_> =
        channel.waiting_receivers.borrow_mut().drain(..).collect();
    for receiver in drained_receivers {
        self.event_loop.ready_queue.push_back(receiver.coroutine);
    }

    // 唤醒所有等待的发送者（它们恢复后重新执行 SEND →
    // 检测 is_closed → 抛出 "send on closed channel" 错误）
    let drained_senders: Vec<_> =
        channel.waiting_senders.borrow_mut().drain(..).collect();
    for sender in drained_senders {
        // 将待发送值放回发送者值栈快照，恢复后 SEND 重试时
        // 能从栈重新弹出（或直接标记发送者以错误终止）
        sender.coroutine.value_stack.push(sender.value);
        self.event_loop.ready_queue.push_back(sender.coroutine);
    }
    Ok(Object::Nil)
}

"closed" => {
    Ok(Object::Bool(matches!(channel.state.borrow(), ChannelState::Closed)))
}
```

### 6. Channel 迭代

`for val in ch { ... }` 需要为 Channel 实现 `__iter__` 和 `__next__`：

- `__iter__`：返回 self
- `__next__`：如果 channel 已关闭且缓冲区为空，返回迭代结束信号（`FOR_ITER` 跳出循环）；否则执行一次 RECEIVE 操作

> **`__next__` 阻塞语义**：Channel 的 `__next__` 等价于 `<-ch`。当 channel 未关闭且缓冲区为空（无缓冲 channel 无等待发送者）时，`__next__` 会阻塞当前协程——通过 `return YieldReason::ChannelRecv` 暂停协程。`FOR_ITER` 指令调用 `__next__` 时，若协程暂停，EventLoop 快照当前帧并调度其他就绪协程；接收者被唤醒后从 `__next__` 恢复点继续，`FOR_ITER` 将接收到的值绑定到循环变量。此机制与 task 53 的可暂停帧设计（`11-bytecode-vm.md:335`）一致——`FOR_ITER` 本身不在安全点列表中，但 `__next__` 内部的 RECEIVE 指令是安全点。

### 7. 编译器改动

- `channel(n)`（n 为 0-255 字面量）编译为 `CHANNEL <n as u8>`（操作码 + 1 字节内联操作数，与 `11-bytecode-vm.md:186` `buffer_size(1)` 一致）
- `channel()`（无参数，即无缓冲）编译为 `CHANNEL 0x00`
- `ch <- value` 编译为 `value` + `ch` + `SEND`
- `<-ch` 编译为 `ch` + `RECEIVE`

> **buffer_size 编译期校验**：`CHANNEL` 操作数为单字节无符号整数（0-255，见 `11-bytecode-vm.md:186`）。若 `channel(n)` 的 n 超过 255，编译器报编译错误："`channel' buffer size exceeds 255"。动态 buffer_size（如 `channel(var)`）不支持——与 `BUILD_LIST count(1)` 等指令的操作数约束一致。

## EventLoop 集成

本 task 扩展 task 53 的 `YieldReason` 枚举，新增两个 channel 相关变体（占位定义见 `39-generator-yield.md:767-768`，task 53 `53-async-await.md:204` 注释预告）：

```rust
enum YieldReason {
    Completed(Object),
    Awaited(*mut MsObjHeader),
    Error(MspError),
    ChannelSend { channel: *mut MsObjHeader, value: Object },  // 本 task 新增
    ChannelRecv { channel: *mut MsObjHeader },                  // 本 task 新增
}
```

### EventLoop 处理 channel 暂停

在 task 53 的 `EventLoop::run` 循环中，`run_until_yield` 的返回值 match 增加：

```rust
YieldReason::ChannelSend { channel, value } => {
    // 协程因 SEND 阻塞——快照协程状态
    coroutine.frame = vm.current_frame().clone();
    coroutine.value_stack = vm.snapshot_value_stack();
    coroutine.defer_stack = std::mem::take(&mut vm.defer_stack);
    coroutine.tlab = vm.tlab.take();

    // 创建 WaitingSender 并加入 channel 的等待列表
    let ch = unsafe { &mut *channel };
    ch.waiting_senders.borrow_mut().push_back(WaitingSender {
        coroutine,
        value,
    });
}

YieldReason::ChannelRecv { channel } => {
    // 协程因 RECEIVE 阻塞——快照协程状态
    coroutine.frame = vm.current_frame().clone();
    coroutine.value_stack = vm.snapshot_value_stack();
    coroutine.defer_stack = std::mem::take(&mut vm.defer_stack);
    coroutine.tlab = vm.tlab.take();

    // 创建 WaitingReceiver 并加入 channel 的等待列表
    let ch = unsafe { &mut *channel };
    ch.waiting_receivers.borrow_mut().push_back(WaitingReceiver {
        coroutine,
    });
}
```

> **协程快照**（与 task 53 `53-async-await.md:258-268` Awaited 分支一致）：暂停时快照帧、值栈、defer 栈、TLAB。恢复时从 `ready_queue` 取出，写回 VM。SEND/RECEIVE 唤醒操作将协程 move 到 `ready_queue` 即可，恢复路径复用 task 53 已有逻辑。

## GC 安全

### CHANNEL TypeDescriptor

Channel 为 GC 管理的堆对象（`TypeTag::CHANNEL = 14`），必须定义 `trace` 函数（参照 `14-gc.md:122-135` TypeDescriptor、task 53 `53-async-await.md:106` FUTURE trace 规格）：

```rust
// CHANNEL trace 函数
fn trace_channel(header: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let channel = unsafe { &*(header as *const Channel) };

    // 1. 遍历缓冲区中的 Object::Ref
    for obj in channel.buffer.borrow().iter() {
        if let Object::Ref(ptr) = obj {
            callback(ptr);
        }
    }

    // 2. 遍历等待发送者协程的 value_stack + 待发送 value
    for sender in channel.waiting_senders.borrow().iter() {
        if let Object::Ref(ptr) = &sender.value {
            callback(ptr);
        }
        for obj in sender.coroutine.value_stack.iter() {
            if let Object::Ref(ptr) = obj {
                callback(ptr);
            }
        }
        // trace 协程帧中的闭包引用
        if let Object::Ref(ptr) = &sender.coroutine.frame.closure {
            callback(ptr);
        }
    }

    // 3. 遍历等待接收者协程的 value_stack
    for receiver in channel.waiting_receivers.borrow().iter() {
        for obj in receiver.coroutine.value_stack.iter() {
            if let Object::Ref(ptr) = obj {
                callback(ptr);
            }
        }
        if let Object::Ref(ptr) = &receiver.coroutine.frame.closure {
            callback(ptr);
        }
    }
}
```

- 无 `finalize`（Channel 无 `__del__`）

### 根集扩展

本 task 引入的 VM 状态必须纳入 GC 根集扫描（见 `14-gc.md:606-626`，对比 task 53 `53-async-await.md:340-349`）：

| 新增根集来源 | 扫描内容 |
|---|---|
| Channel 对象的 `buffer` | `VecDeque<Object>` 中的 `Object::Ref` |
| Channel 对象的 `waiting_senders` | 每个等待发送者协程的 `value_stack` Ref + `frame.closure` + `value` |
| Channel 对象的 `waiting_receivers` | 每个等待接收者协程的 `value_stack` Ref + `frame.closure` |

> **关键**：等待列表中的协程在阻塞期间不被回收——CHANNEL 的 `trace` 函数遍历等待协程的值栈和闭包引用，确保所有可达对象保留。若 channel 本身不可达（无根引用），则整个 channel 及其等待协程将被 GC 回收。

### GC 移动对象的指针更新

Minor GC 的半空间复制会移动 Young 代对象（见 `14-gc.md:351-359`）。以下指针需由 GC forwarding 机制更新：

- Channel `buffer` 中的所有 `Object::Ref`
- 等待协程 `value_stack` 中的所有 `Object::Ref`
- `WaitingSender.value` 若为 `Object::Ref`

### 写屏障

`SEND` 指令将值写入 channel buffer（堆数据），在并发 GC 标记期间需触发混合写屏障（见 `14-gc.md:546` 写屏障插入点表）。写屏障实现为内联检查：`if gc_phase.is_concurrent_mark() { barrier(...) }`，非 GC 期间零开销。

### RefCell borrow 约束

Channel 的 `buffer` / `state` / `waiting_senders` / `waiting_receivers` 均使用 `RefCell`。约束（与 task 53 `53-async-await.md:362-366` 一致）：

- 所有 `borrow_mut()` guard 必须在安全点检查和 `return YieldReason` 之前释放（使用 `{ ... }` 块限定生命周期）
- GC trace 函数访问 RefCell 时使用 `try_borrow()`，若失败（mutate 进行中）则将对象标灰待重扫

## 验证标准

1. 有缓冲 channel 正确发送和接收
2. 无缓冲 channel 同步发送和接收
3. 缓冲区满时发送阻塞
4. 缓冲区空时接收阻塞
5. 关闭 channel 后接收返回剩余数据然后 nil
6. 向已关闭 channel 发送抛出错误
7. `close()` 幂等：重复调用不报错（`08-concurrency.md:221`）
8. 关闭 channel 时唤醒所有等待的发送者和接收者
9. `for val in ch` 正确遍历直到关闭
10. 多协程通过 channel 正确通信

## 测试用例

### test_channel_buffered.ms

```ms
ch = channel(3)

ch <- 1
ch <- 2
ch <- 3

print(<-ch)
print(<-ch)
print(<-ch)
```

预期输出：
```
1
2
3
```

### test_channel_close.ms

```ms
ch = channel(5)

ch <- "a"
ch <- "b"
ch <- "c"
ch.close()

print(ch.closed())

for item in ch {
    print(item)
}

# 从已关闭的空 channel 接收
result = <-ch
print(result)
```

预期输出：
```
true
a
b
c
nil
```

### test_channel_send_closed.ms

```ms
ch = channel(1)
ch.close()

try {
    ch <- 42
} except e {
    print("caught: " + str(e))
}
```

预期输出：
```
caught: send on closed channel
```

### test_channel_unbuffered.ms

> 无缓冲 channel 测试需在独立协程中发送（否则同协程顺序执行必然死锁）。本测试使用 async fn + 顶层 await（task 53 已提供），不依赖 task 55 的 `go` 关键字。

```ms
# 发送方协程：调用 async fn 创建 Future + 协程，加入 ready_queue
async fn sender(ch) {
    ch <- 42
}

ch = channel()

# 调用 sender(ch) 返回 Future，对应协程加入就绪队列
f = sender(ch)

# 主协程接收：阻塞时 EventLoop 调度 sender 协程执行
val = <-ch
print(val)

# 等待 sender 协程完成
await f
```

预期输出：
```
42
```

### test_channel_close_wakes_sender.ms

> 验证 close() 唤醒阻塞的发送者，使其收到 "send on closed channel" 错误。

```ms
async fn blocked_sender(ch) {
    ch <- "data"
}

ch = channel(1)
ch <- "first"       # 填满缓冲区

f = blocked_sender(ch)  # sender 协程：SEND 阻塞（缓冲区满）

ch.close()          # 唤醒 sender → 重试 SEND → is_closed → 抛出错误

try {
    await f
} except e {
    print("caught: " + str(e))
}
```

预期输出：
```
caught: send on closed channel
```

### test_channel_close_idempotent.ms

> 验证 `close()` 幂等性（`08-concurrency.md:221`）。

```ms
ch = channel(2)
ch <- 1
ch.close()
ch.close()          # 重复 close 不报错
ch.close()          # 仍不报错

print(ch.closed())  # true
print(<-ch)         # 1（缓冲区剩余数据）
print(<-ch)         # nil（已关闭且空）
```

预期输出：
```
true
1
nil
```
