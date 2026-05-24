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
    buffer: RefCell<VecDeque<Object>>,
    capacity: usize,
    state: RefCell<ChannelState>,
    waiting_senders: RefCell<Vec<Gc<Coroutine>>>,
    waiting_receivers: RefCell<Vec<Gc<Coroutine>>>,
}
```

- `buffer`：内部队列
- `capacity`：缓冲区容量（0 为无缓冲）
- `state`：通道状态（Open/Closed）
- `waiting_senders`：等待发送的协程列表
- `waiting_receivers`：等待接收的协程列表

### 2. CHANNEL 指令

`src/vm/mod.rs`：

```rust
OpCode::CHANNEL => {
    let buffer_size = self.read_byte() as usize;
    // buffer_size == 0 → 无缓冲 channel（发送方和接收方同步）
    let channel = Channel::new(buffer_size);
    self.stack.push(Object::Channel(Gc::new(channel)));
}
```

### 3. SEND 指令

编译器将 `ch <- value` 编译为：`value` → `channel` → `SEND`

```rust
OpCode::SEND => {
    let value = self.stack.pop();
    let channel_obj = self.stack.pop();
    let channel = expect_channel(&channel_obj)?;

    if channel.is_closed() {
        return Err(MspError::RuntimeError("send on closed channel".into()));
    }

    if channel.capacity == 0 {
        // 无缓冲：尝试匹配等待的接收者
        if let Some(receiver) = channel.waiting_receivers.borrow_mut().pop() {
            // 直接传递给接收者
            // 唤醒接收者协程
            self.event_loop.ready_queue.push(receiver);
        } else {
            // 没有等待的接收者，暂停当前协程
            channel.waiting_senders.borrow_mut().push(current_coroutine);
            self.yield_coroutine();
        }
    } else {
        // 有缓冲
        let mut buffer = channel.buffer.borrow_mut();
        if buffer.len() < channel.capacity {
            buffer.push_back(value);
        } else {
            // 缓冲区满，暂停当前协程
            channel.waiting_senders.borrow_mut().push(current_coroutine);
            self.yield_coroutine();
        }
    }
}
```

### 4. RECEIVE 指令

编译器将 `<-ch` 编译为：`channel` → `RECEIVE`

```rust
OpCode::RECEIVE => {
    let channel_obj = self.stack.pop();
    let channel = expect_channel(&channel_obj)?;

    let mut buffer = channel.buffer.borrow_mut();
    if let Some(val) = buffer.pop_front() {
        // 缓冲区有数据
        // 如果有等待的发送者，将其数据移入缓冲区
        if let Some(sender) = channel.waiting_senders.borrow_mut().pop() {
            self.event_loop.ready_queue.push(sender);
        }
        self.stack.push(val);
    } else if channel.is_closed() {
        // 已关闭且缓冲区为空
        self.stack.push(Object::Nil);
    } else {
        // 缓冲区空且未关闭，暂停当前协程
        channel.waiting_receivers.borrow_mut().push(current_coroutine);
        self.yield_coroutine();
    }
}
```

### 5. Channel 方法

`close()`、`closed()` 通过方法分派实现：

```rust
"close" => {
    channel.state.replace(ChannelState::Closed);
    // 唤醒所有等待的接收者（它们会收到 nil）
    for receiver in channel.waiting_receivers.borrow_mut().drain(..) {
        self.event_loop.ready_queue.push(receiver);
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
- `__next__`：如果 channel 已关闭且缓冲区为空，返回迭代结束信号；否则接收下一个值

### 7. 编译器改动

- `channel(n)` 表达式编译为 `CONSTANT n` + `CHANNEL`
- `ch <- value` 编译为 `value` + `ch` + `SEND`
- `<-ch` 编译为 `ch` + `RECEIVE`

## 验证标准

1. 有缓冲 channel 正确发送和接收
2. 无缓冲 channel 同步发送和接收
3. 缓冲区满时发送阻塞
4. 缓冲区空时接收阻塞
5. 关闭 channel 后接收返回剩余数据然后 nil
6. 向已关闭 channel 发送抛出错误
7. `for val in ch` 正确遍历直到关闭
8. 多协程通过 channel 正确通信

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

```ms
ch = channel()

go fn() {
    ch <- 42
}()

val = <-ch
print(val)
```

预期输出：
```
42
```
