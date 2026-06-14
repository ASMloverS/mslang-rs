# 标准库 - async 模块

## 所属阶段
Phase 7.2 - 并发

## 前置任务
53-async-await, 45-module-system

## 目标

实现 mslang 标准库 `async` 模块，提供异步定时器和超时控制工具函数。

## 设计规格

参照 [10-builtins](../10-builtins.md) § async：

### API 列表

| 函数 | 签名 | 说明 |
|---|---|---|
| `async.sleep(ms)` | `sleep(ms: int) -> nil` | 异步休眠指定毫秒数（让出协程执行权） |
| `async.timeout(fn, ms)` | `timeout(fn: function, ms: int) -> value` | 带超时执行函数，超时抛出 `TimeoutError` |

### async.sleep(ms)

```ms
import async

async fn delayed_greet() {
    print("waiting...")
    await async.sleep(1000)
    print("done!")
}
```

- 参数 `ms` 为休眠毫秒数（int）
- 该函数返回一个 Future，`await` 时暂停当前协程
- 到达指定时间后，事件循环唤醒协程继续执行
- `await async.sleep(0)` 可用于主动让出执行权（防止饿死其他协程）

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

- `fn` 为需要执行的异步函数或闭包
- `ms` 为超时毫秒数
- 如果 `fn` 在 `ms` 毫秒内完成，返回其结果
- 如果超时，抛出 `TimeoutError`

## 实现细节

### 文件位置

`src/stdlib/async.rs`

### 注册方式

在 VM 初始化时，将 `async` 模块注册为内置模块：

```rust
pub fn register_async_module(vm: &mut VM) {
    let mut exports = HashMap::new();

    // 原生函数通过 alloc_native_function 分配为堆对象（Ref + TypeTag::FUNCTION）
    exports.insert("sleep".into(), alloc_native_function(NativeFunction {
        name: "async.sleep".into(), arity: 1, func: async_sleep,
    }));
    exports.insert("timeout".into(), alloc_native_function(NativeFunction {
        name: "async.timeout".into(), arity: 2, func: async_timeout,
    }));

    vm.builtin_modules.insert("async".into(), Module { name: "async".into(), exports, globals: HashMap::new() });
}
```

### async_sleep 实现

```rust
fn async_sleep(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ms = match &args[0] {
        Object::Int(n) => *n,
        _ => return Err("async.sleep expects int argument".to_string()),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
    Ok(Object::Future(Future::new_timer(deadline)))
}
```

`Future::new_timer(deadline)` 创建一个定时器 Future：
- 事件循环在每个调度周期检查定时器是否到期
- 到期后 Future 变为 `Resolved(Nil)`，唤醒等待的协程

### async_timeout 实现

```rust
fn async_timeout(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let func = match &args[0] {
        Object::Function(f) => f.clone(),
        _ => return Err("async.timeout expects function as first argument".to_string()),
    };
    let ms = match &args[1] {
        Object::Int(n) => *n,
        _ => return Err("async.timeout expects int as second argument".to_string()),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
    Ok(Object::Future(Future::new_timeout(func, deadline)))
}
```

`Future::new_timeout(func, deadline)` 创建一个带超时的 Future：
- 启动目标函数作为子协程
- 设置超时定时器
- 如果子协程先完成，Future 变为 `Resolved(result)`
- 如果定时器先到期，Future 变为 `Rejected(TimeoutError)`

### 事件循环集成

事件循环需要扩展以支持定时器检查：

```rust
impl EventLoop {
    fn check_timers(&mut self) {
        let now = std::time::Instant::now();
        self.paused.retain(|coro| {
            if let Some(deadline) = coro.timer_deadline {
                if now >= deadline {
                    // 定时器到期，唤醒协程
                    self.ready_queue.push_back(coro.clone());
                    return false;
                }
            }
            true
        });
    }
}
```

## 验证标准

1. `await async.sleep(100)` 正确暂停约 100ms 后恢复
2. `await async.sleep(0)` 立即让出执行权
3. `await async.timeout(fn, 5000)` 在函数正常完成时返回结果
4. `await async.timeout(fn, 10)` 在超时时抛出 `TimeoutError`

## 测试用例

```ms
import async

async fn test_sleep() {
    print("before sleep")
    await async.sleep(100)
    print("after sleep")
}

async fn test_timeout() {
    result = await async.timeout(fn() {
        return 42
    }, 5000)
    print(result)
}

async fn test_timeout_error() {
    try {
        await async.timeout(fn() {
            await async.sleep(10000)
        }, 50)
    } except TimeoutError {
        print("timed out as expected")
    }
}

await test_sleep()
await test_timeout()
await test_timeout_error()
```

预期输出：
```
before sleep
after sleep
42
timed out as expected
```
