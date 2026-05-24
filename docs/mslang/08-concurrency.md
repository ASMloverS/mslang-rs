# 并发模型

## 概述

mslang 的并发模型基于 **async/await** 协程，辅以 **channel** 进行协程间通信。

| 特性 | 说明 |
|---|---|
| 并发单元 | 协程 (coroutine) |
| 调度方式 | 协作式调度（yield 点让出控制权） |
| 通信方式 | async/await + channel |
| 启动并发 | `go fn() { ... }` |

## async/await

### async 函数

```
async_fn = "async" "fn" IDENTIFIER "(" param_list? ")" block
```

```ms
async fn fetch(url) {
    resp = await http_get(url)
    return resp.body
}
```

- `async fn` 定义的函数返回一个 **Future** 对象
- 调用 async 函数不会立即执行，而是返回 Future
- `await` 暂停当前协程，等待 Future 完成

### await 表达式

```
await_expr = "await" expression
```

```ms
async fn parallel_fetch(urls) {
    futures = []
    for url in urls {
        futures.push(fetch(url))
    }
    results = []
    for f in futures {
        results.push(await f)
    }
    return results
}
```

- `await` 只能在 `async fn` 内使用
- `await` 暂停当前协程，让出执行权给事件循环
- 当 Future 完成后，协程从暂停点恢复

### Future

Future 表示一个异步操作的最终结果。

```ms
f = fetch("http://example.com")    # 返回 Future
body = await f                      # 等待结果
```

Future 状态：
- **Pending** — 未完成
- **Resolved** — 成功完成，持有返回值
- **Rejected** — 失败，持有错误

### 顶层 await

在脚本顶层可以直接使用 `await`（无需包裹在 async 函数中）：

```ms
# top-level await
data = await fetch("http://api.example.com/data")
print(data)
```

> **实现注意**：顶层 await 意味着主脚本的执行环境必须支持协程暂停/恢复。这要求 Phase 2 的 VM 核心在初始设计时就预留协程基础设施（可暂停的执行帧），而非在 Phase 7 补丁式添加。

## go 关键字

```
go_expr = "go" expression
```

`go` 启动一个新的并发协程：

```ms
go fn() {
    result = heavy_computation()
    print(result)
}

# 也可以启动 async 函数
go async_fetch("http://example.com")
```

`go` 表达式：
- 立即返回，不等待协程完成
- 被启动的协程在事件循环中并发执行
- 协程的返回值被丢弃（除非通过 channel 传递）

## Channel

### 创建 channel

```ms
ch = channel()       # 无缓冲 channel
ch = channel(10)     # 缓冲区大小为 10 的 channel
```

### 发送与接收

```ms
# 发送
ch <- value

# 接收
value = <-ch
```

### 无缓冲 channel

无缓冲 channel 的发送和接收**同步进行**：
- 发送方阻塞直到有接收方
- 接收方阻塞直到有发送方

```ms
ch = channel()

go fn() {
    ch <- 42       # 发送方等待接收方
}

val = <-ch          # 接收方等待发送方
print(val)          # 42
```

### 有缓冲 channel

有缓冲 channel 有一个内部队列：
- 缓冲区未满时，发送不阻塞
- 缓冲区非空时，接收不阻塞
- 缓冲区满时，发送阻塞等待
- 缓冲区空时，接收阻塞等待

```ms
ch = channel(3)

ch <- 1    # 不阻塞
ch <- 2    # 不阻塞
ch <- 3    # 不阻塞（缓冲区已满）

go fn() {
    time.sleep(100)
    val = <-ch    # 消费一个，腾出空间
}

ch <- 4    # 阻塞，等待缓冲区有空位
```

### channel 操作

```ms
ch = channel(10)

# 发送
ch <- value

# 接收
value = <-ch

# 关闭
ch.close()

# 检查是否关闭
ch.closed()

# 遍历（接收直到关闭）
for val in ch {
    print(val)
}
```

### 关闭 channel

- 发送方可以关闭 channel 表示不再发送数据
- 接收方仍可接收缓冲区中剩余的数据
- 从已关闭的空 channel 接收返回 `nil`
- 向已关闭的 channel 发送抛出运行时错误

```ms
ch = channel(3)

go fn() {
    for i in range(3) {
        ch <- i
    }
    ch.close()
}

for val in ch {
    print(val)    # 0, 1, 2
}
```

## select（保留）

多 channel 复用的 `select` 语法保留给后续版本：

```ms
# 保留语法（暂不实现）
select {
    case val = <-ch1 {
        print("from ch1: " + str(val))
    }
    case ch2 <- data {
        print("sent to ch2")
    }
    default {
        print("no activity")
    }
}
```

> **注意**：`select`、`case`、`default` 为保留关键字（见 [01-lexical](01-lexical.md)），不可用作变量名。

## 事件循环

### 运行机制

mslang 内置事件循环 (EventLoop)：

1. 程序启动时，事件循环自动创建
2. 主脚本代码在主协程中执行
3. `go` 启动的协程注册到事件循环
4. 当主协程遇到 `await` 时，事件循环调度其他就绪协程
5. 程序在所有协程完成后退出

### 调度策略

- 协作式调度：协程在 `await`、channel 操作时主动让出
- 公平调度：就绪协程按 FIFO 顺序执行
- 无抢占：没有时间片轮转

### 与 defer 的交互

`defer` 在协程结束（正常或异常）时执行。每个协程维护自己的 defer 栈。

```ms
async fn worker(id) {
    defer print("worker " + str(id) + " done")
    await some_async_op()
}
```

## 完整示例

### 并发 HTTP 请求

```ms
async fn fetch_all(urls) {
    results = []
    ch = channel(len(urls))

    for url in urls {
        go fn(u) {
            resp = await http_get(u)
            ch <- resp
        }(url)
    }

    for i in range(len(urls)) {
        results.push(<-ch)
    }

    return results
}

data = await fetch_all([
    "http://api1.example.com",
    "http://api2.example.com",
    "http://api3.example.com"
])
```

### 生产者-消费者

```ms
ch = channel(5)

# 生产者
go fn() {
    for i in range(20) {
        ch <- i
        print("produced: " + str(i))
    }
    ch.close()
}()

# 消费者
for item in ch {
    print("consumed: " + str(item))
}
```

### 超时控制

```ms
async fn fetch_with_timeout(url, timeout_ms) {
    ch = channel(1)

    go fn() {
        resp = await http_get(url)
        ch <- resp
    }()

    go fn() {
        await sleep(timeout_ms)
        ch <- nil
    }()

    result = <-ch
    return result
}
```
