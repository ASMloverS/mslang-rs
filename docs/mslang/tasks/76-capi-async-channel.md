# C API — Async/Channel/Generator

## 所属阶段

Phase 7 后 — 并发特性完成后

## 前置任务

- 53-async-await（Future 对象、AWAIT 指令、EventLoop 事件循环、协程调度）
- 54-channel（Channel 对象、SEND/RECEIVE 指令、有缓冲/无缓冲通信、关闭语义）
- 55-go-concurrency（go 关键字、GO 指令、JoinHandle）
- 65-capi-infrastructure（cbindgen + 手写类型头文件 + 构建集成）
- 70-capi-call（msCall 同步调用、NativeFunction 桥接、CallFrame 执行）

## 目标

实现 call.h 的异步部分 API：异步调用（`msCallAsync`/`msAwait`）、Future 操作（`msFutureState`/`msFutureResolve`/`msFutureReject`）、C 侧 async 函数桥接（`MsAsyncFunction`）、Channel 操作（`msChannel`/`msChannelSend`/`msChannelRecv`/`msChannelClose`/`msChannelIsClosed`）、Generator 操作（`msGeneratorIter`/`msGeneratorNext`）。使 C 程序能异步调用 mslang 函数、创建和操作 Channel 与 Generator。

## 设计规格

参照 [13-capi.md](../13-capi.md) § call.h — 异步调用、Channel 操作、生成器操作。

### 异步调用

```c
MS_API MsValue* msCallAsync(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
MS_API MsValue* msAwait(MsVM* vm, MsValue* future);
```

- `msCallAsync`：异步调用函数，立即返回 `Future` 对象。`func` 为可调用对象，参数语义与 `msCall` 一致
- `msAwait`：阻塞等待 Future 完成。Resolved 返回结果（新引用），Rejected 返回 NULL 并设置异常

### Future 操作

```c
typedef enum MsFutureState {
    MS_FUTURE_PENDING,
    MS_FUTURE_RESOLVED,
    MS_FUTURE_REJECTED,
} MsFutureState;

MS_API MsFutureState msFutureState(MsVM* vm, MsValue* future);
MS_API void msFutureResolve(MsVM* vm, MsValue* future, MsValue* result);
MS_API void msFutureReject(MsVM* vm, MsValue* future, MsValue* error);
```

- `msFutureState`：查询 Future 当前状态
- `msFutureResolve`：将 Future 设为 Resolved 并设置结果值，唤醒等待者
- `msFutureReject`：将 Future 设为 Rejected 并设置错误值，唤醒等待者

### C 侧 async 函数

```c
typedef void (*MsAsyncFunction)(MsVM* vm, MsValue* const* args, int nargs, MsValue* future);
```

C async 函数接收参数和一个 Future。C 函数负责在异步操作完成后调用 `msFutureResolve` 或 `msFutureReject`。通过 `msModuleAddAsyncFunc`（Task 72 已定义）注册到模块。

### Channel 操作

```c
MS_API MsValue*  msChannel(MsVM* vm, int bufferSize);
MS_API MsStatus  msChannelSend(MsVM* vm, MsValue* ch, MsValue* val);
MS_API MsValue*  msChannelRecv(MsVM* vm, MsValue* ch);
MS_API MsStatus  msChannelClose(MsVM* vm, MsValue* ch);
MS_API int       msChannelIsClosed(MsVM* vm, MsValue* ch);
```

- `msChannel`：创建指定缓冲区大小的 Channel 对象
- `msChannelSend`：发送值到 Channel。缓冲区满时线程级阻塞
- `msChannelRecv`：从 Channel 接收值。缓冲区空时线程级阻塞；Channel 已关闭且为空时返回 `nil`
- `msChannelClose`：关闭 Channel
- `msChannelIsClosed`：查询 Channel 是否已关闭

### Generator 操作

```c
MS_API MsValue*  msGeneratorIter(MsVM* vm, MsValue* generator);
MS_API MsStatus  msGeneratorNext(MsVM* vm, MsValue* generator, MsValue** out);
```

- `msGeneratorIter`：返回 Generator 的迭代器（Generator 自身即为迭代器，返回 `generator` 本身）
- `msGeneratorNext`：恢复 Generator 执行，获取下一个 yield 值。迭代结束时返回 `MS_ERROR`

## 实现细节

### 文件结构

```
变更文件:
  src/capi/call.rs                    // 追加异步/Channel/Generator 函数（#[cfg(feature = "capi")]）
  include/mslang/call.h               // cbindgen 重新生成（新增函数声明）
  include/mslang/mslang.h             // 无变更（call.h 已在 Task 70 启用）

依赖（前置任务已完成）:
  src/capi/vm.rs                      // CApiVM 包装、mutex、vm 内部引用
  src/capi/value.rs                   // MsValue 包装、wrap_value/unwrap_value
  src/capi/call.rs                    // msCall 同步调用（Task 70）
  src/vm/object.rs                    // TypeTag::FUTURE/CHANNEL/GENERATOR、堆对象分配
  src/vm/mod.rs                       // VM 内部结构、EventLoop、execute_call_from_capi
  src/async_runtime/mod.rs            // EventLoop、Coroutine、PausedCoroutine
  src/async_runtime/channel.rs        // Channel 对象实现
```

### 1. msCallAsync — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msCallAsync(
    vm: *mut crate::capi::vm::CApiVM,
    func: *mut crate::capi::value::MsValue,
    args: *const *mut crate::capi::value::MsValue,
    nargs: c_int,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || func.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let func_obj = match capi_vm.unwrap_value(func) {
        Some(obj) => obj,
        None => return std::ptr::null_mut(),
    };

    // 检查可调用性（与 msCall 一致）
    let callable = match &func_obj {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
                || tag == TypeTag::NATIVE_FUNCTION as u8
                || tag == TypeTag::CLASS as u8
        }
        _ => false,
    };
    if !callable {
        capi_vm.set_error("not a callable object");
        return std::ptr::null_mut();
    }

    // 转换参数
    let nargs_usize = nargs as usize;
    let arg_objects: Vec<Object> = if nargs_usize > 0 && !args.is_null() {
        std::slice::from_raw_parts(args, nargs_usize)
            .iter()
            .filter_map(|&arg_ptr| {
                if arg_ptr.is_null() { None } else { capi_vm.unwrap_value(arg_ptr) }
            })
            .collect()
    } else {
        Vec::new()
    };

    // 创建 Future
    let future_obj = alloc_future();
    let Object::Ref(future_ptr) = &future_obj else { return std::ptr::null_mut() };

    // 创建协程执行函数调用
    let vm_inner = &mut capi_vm.vm;
    vm_inner.stack.push(func_obj);
    for arg in &arg_objects {
        vm_inner.stack.push(arg.clone());
    }

    // 创建 Coroutine 并加入 EventLoop 就绪队列
    let coroutine = vm_inner.create_coroutine_for_capi(nargs_usize, *future_ptr);
    vm_inner.event_loop.ready_queue.push_back(coroutine);

    // 立即返回 Future
    capi_vm.wrap_value(future_obj.clone())
}
```

核心逻辑：

1. Lock VM，校验参数
2. 检查 func 可调用性
3. 转换 MsValue* 参数为内部 Object
4. 创建 Future 对象（TypeTag::FUTURE，初始状态 Pending）
5. 创建协程包装函数调用，将协程加入 EventLoop ready_queue
6. 立即返回 Future（不等协程完成）
7. 协程执行完成后，内部自动 resolve/reject 该 Future

### 2. msAwait — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msAwait(
    vm: *mut crate::capi::vm::CApiVM,
    future: *mut crate::capi::value::MsValue,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || future.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let future_obj = match capi_vm.unwrap_value(future) {
        Some(obj) => obj,
        None => return std::ptr::null_mut(),
    };

    let Object::Ref(ptr) = &future_obj else {
        capi_vm.set_error("not a Future object");
        return std::ptr::null_mut();
    };

    if unsafe { (**ptr).type_tag } != TypeTag::FUTURE as u8 {
        capi_vm.set_error("not a Future object");
        return std::ptr::null_mut();
    }

    let future_inner = unsafe { read_future(*ptr) };

    // 检查当前状态
    let state = future_inner.state.borrow();
    match &*state {
        FutureState::Resolved(val) => {
            drop(state);
            capi_vm.wrap_value(val.clone())
        }
        FutureState::Rejected(err) => {
            let err_msg = err.clone();
            drop(state);
            capi_vm.set_error(&err_msg);
            std::ptr::null_mut()
        }
        FutureState::Pending => {
            drop(state);
            // 阻塞等待 Future 完成
            // 使用 thread parking 机制
            // 注册一个线程唤醒器到 Future 的 waiters 列表
            let thread_handle = std::thread::current();
            let thread_id = thread_handle.id();

            // 注册线程唤醒回调
            future_inner.add_thread_waiter(thread_id);

            // 释放锁后 parking 等待
            drop(_lock);

            // 阻塞当前线程直到 Future 被 resolve/reject
            // EventLoop 中的协程执行 resolve 时会 unpark 此线程
            std::thread::park();

            // 被唤醒后重新获取锁
            let _lock = capi_vm.mutex.lock();

            let state = future_inner.state.borrow();
            match &*state {
                FutureState::Resolved(val) => {
                    capi_vm.wrap_value(val.clone())
                }
                FutureState::Rejected(err) => {
                    let err_msg = err.clone();
                    drop(state);
                    capi_vm.set_error(&err_msg);
                    std::ptr::null_mut()
                }
                FutureState::Pending => {
                    // 不应发生
                    std::ptr::null_mut()
                }
            }
        }
    }
}
```

核心逻辑：

1. Lock VM，校验参数类型为 Future
2. 检查 Future 状态：
   - Resolved → 直接返回结果
   - Rejected → 设置异常，返回 NULL
   - Pending → 阻塞当前线程
3. 阻塞机制：将当前线程 ID 注册到 Future 的 `thread_waiters` 列表，释放 VM 锁，调用 `std::thread::park()`
4. 当 EventLoop 中协程 resolve/reject 该 Future 时，`msFutureResolve`/`msFutureReject` 遍历 `thread_waiters`，对每个线程调用 `std::thread::unpark()`
5. 线程被唤醒后重新获取锁，读取最终状态返回

> **设计决策**：C 侧没有协程上下文，`msAwait` 使用线程级阻塞。这与 mslang 内部 `AWAIT` 指令的协程暂停机制不同。C 调用者需注意：不要在 mslang EventLoop 线程中调用 `msAwait`，否则会死锁。

### 3. msFutureState — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msFutureState(
    vm: *mut crate::capi::vm::CApiVM,
    future: *mut crate::capi::value::MsValue,
) -> MsFutureState {
    if vm.is_null() || future.is_null() {
        return MsFutureState::MS_FUTURE_PENDING;
    }

    let capi_vm = &mut *vm;

    let future_obj = match capi_vm.unwrap_value(future) {
        Some(obj) => obj,
        None => return MsFutureState::MS_FUTURE_PENDING,
    };

    let Object::Ref(ptr) = &future_obj else {
        return MsFutureState::MS_FUTURE_PENDING;
    };

    if unsafe { (**ptr).type_tag } != TypeTag::FUTURE as u8 {
        return MsFutureState::MS_FUTURE_PENDING;
    }

    let future_inner = unsafe { read_future(*ptr) };
    let state = future_inner.state.borrow();

    match &*state {
        FutureState::Pending => MsFutureState::MS_FUTURE_PENDING,
        FutureState::Resolved(_) => MsFutureState::MS_FUTURE_RESOLVED,
        FutureState::Rejected(_) => MsFutureState::MS_FUTURE_REJECTED,
    }
}
```

### 4. msFutureResolve / msFutureReject — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msFutureResolve(
    vm: *mut crate::capi::vm::CApiVM,
    future: *mut crate::capi::value::MsValue,
    result: *mut crate::capi::value::MsValue,
) {
    if vm.is_null() || future.is_null() || result.is_null() {
        return;
    }

    let capi_vm = &mut *vm;
    let result_obj = match capi_vm.unwrap_value(result) {
        Some(obj) => obj,
        None => return,
    };

    let future_obj = match capi_vm.unwrap_value(future) {
        Some(obj) => obj,
        None => return,
    };

    let Object::Ref(ptr) = &future_obj else { return };
    if unsafe { (**ptr).type_tag } != TypeTag::FUTURE as u8 {
        return;
    }

    let future_inner = unsafe { read_future(*ptr) };

    // 设置状态
    future_inner.state.replace(FutureState::Resolved(result_obj));

    // 唤醒协程等待者（加入 EventLoop ready_queue）
    for waiter_ptr in future_inner.waiters.borrow_mut().drain(..) {
        let tag = unsafe { (*waiter_ptr).type_tag };
        if tag == TypeTag::GENERATOR as u8 {
            // 暂停的协程，加入就绪队列
            capi_vm.vm.event_loop.ready_queue.push_back(waiter_ptr);
        }
    }

    // 唤醒线程等待者（msAwait 阻塞的线程）
    for thread_id in future_inner.thread_waiters.borrow_mut().drain(..) {
        // 遍历注册的线程 ID 并 unpark
        // 注意：无法直接从 ThreadId 获取 Thread 句柄
        // 使用 Arc<Condvar> 方案替代（见下方设计）
    }
}
```

**线程唤醒替代方案**：由于 `ThreadId` 无法直接用于 `unpark`，改用 `Arc<(Mutex<bool>, Condvar)>` 作为线程阻塞原语：

```rust
// Future 对象增加 thread_waiters 字段
struct Future {
    state: RefCell<FutureState>,
    waiters: RefCell<Vec<*mut MsObjHeader>>,           // 协程等待者
    thread_waiters: RefCell<Vec<Arc<(Mutex<bool>, Condvar)>>>, // 线程等待者
}
```

`msAwait` 阻塞逻辑改为：

```rust
// 注册线程等待器
let signal = Arc::new((Mutex::new(false), Condvar::new()));
future_inner.thread_waiters.borrow_mut().push(signal.clone());

drop(state);
drop(_lock);

// 等待信号
let (lock, cvar) = &*signal;
let mut completed = lock.lock().unwrap();
while !*completed {
    completed = cvar.wait(completed).unwrap();
}
```

`msFutureResolve`/`msFutureReject` 唤醒逻辑：

```rust
// 唤醒线程等待者
for signal in future_inner.thread_waiters.borrow_mut().drain(..) {
    let (lock, cvar) = &*signal;
    *lock.lock().unwrap() = true;
    cvar.notify_one();
}
```

`msFutureReject` 与 `msFutureResolve` 结构对称，唯一区别是将状态设为 `FutureState::Rejected` 并存储错误值：

```rust
#[no_mangle]
pub unsafe extern "C" fn msFutureReject(
    vm: *mut crate::capi::vm::CApiVM,
    future: *mut crate::capi::value::MsValue,
    error: *mut crate::capi::value::MsValue,
) {
    // 与 msFutureResolve 对称
    // future_inner.state.replace(FutureState::Rejected(error_str));
    // 唤醒 waiters + thread_waiters
}
```

### 5. MsAsyncFunction 桥接

当 C async 函数通过 `msModuleAddAsyncFunc`（Task 72）注册时，内部创建 `NativeAsyncFunction` 堆对象：

```rust
// src/vm/object.rs 新增 TypeTag 变体
TypeTag::NATIVE_ASYNC_FUNCTION = 17,

#[repr(C)]
pub struct NativeAsyncFunction {
    pub header: MsObjHeader,
    pub name: String,
    pub func: unsafe extern "C" fn(
        *mut MsVM,
        *const *mut MsValue,
        i32,
        *mut MsValue,           // Future
    ),
    pub arity: i32,
}
```

当 VM 的 CALL 指令遇到 `NATIVE_ASYNC_FUNCTION` 时：

1. 创建 Future 对象
2. 调用 C async 函数，传入 Future
3. 将 Future 压入调用者栈（调用者通过 await 等待结果）
4. C 函数在后台线程或异步操作完成后调用 `msFutureResolve`/`msFutureReject`

```rust
// CALL 指令中新增分支
Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::NATIVE_ASYNC_FUNCTION as u8 => {
    let native_async = unsafe { read_native_async_function(ptr) };

    // 创建 Future
    let future_obj = alloc_future();
    let future_msvalue = object_to_msvalue_ptr(&future_obj);

    // 构建参数数组
    let arg_ptrs: Vec<*mut MsValue> = /* ... */;

    // 调用 C async 函数（非阻塞，C 函数负责异步完成）
    unsafe {
        (native_async.func)(
            vm_as_capi_ptr(self),
            arg_ptrs.as_ptr(),
            argc as i32,
            future_msvalue,
        );
    }

    // 清理栈，压入 Future
    self.stack.truncate(callee_idx);
    self.stack.push(future_obj);
}
```

### 6. msChannel — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msChannel(
    vm: *mut crate::capi::vm::CApiVM,
    buffer_size: c_int,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let channel = Channel::new(buffer_size.max(0) as usize);
    let channel_obj = alloc_channel(channel);

    capi_vm.wrap_value(channel_obj)
}
```

### 7. msChannelSend — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msChannelSend(
    vm: *mut crate::capi::vm::CApiVM,
    ch: *mut crate::capi::value::MsValue,
    val: *mut crate::capi::value::MsValue,
) -> MsStatus {
    if vm.is_null() || ch.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let ch_obj = match capi_vm.unwrap_value(ch) {
        Some(obj) => obj,
        None => return MsStatus::MS_ERROR,
    };

    let val_obj = match capi_vm.unwrap_value(val) {
        Some(obj) => obj,
        None => return MsStatus::MS_ERROR,
    };

    let Object::Ref(ptr) = &ch_obj else { return MsStatus::MS_ERROR };
    if unsafe { (**ptr).type_tag } != TypeTag::CHANNEL as u8 {
        capi_vm.set_error("not a Channel object");
        return MsStatus::MS_ERROR;
    }

    let channel = unsafe { read_channel(*ptr) };

    if channel.is_closed() {
        capi_vm.set_error("send on closed channel");
        return MsStatus::MS_ERROR;
    }

    // 线程级阻塞发送
    // C 调用者没有协程上下文，使用 Condvar 阻塞
    let mut buffer = channel.buffer.borrow_mut();

    if channel.capacity == 0 {
        // 无缓冲 Channel：需要等待接收者
        // 简化实现：对无缓冲 Channel 从 C API 使用线程级同步原语
        drop(buffer);
        drop(_lock);

        // 使用 channel 内部的 sync primitives 阻塞
        channel.send_blocking(val_obj);

        MsStatus::MS_OK
    } else if buffer.len() < channel.capacity {
        // 缓冲区未满，直接入队
        buffer.push_back(val_obj);

        // 如果有等待的协程接收者，唤醒
        if let Some(waiter) = channel.waiting_receivers.borrow_mut().pop() {
            capi_vm.vm.event_loop.ready_queue.push_back(waiter);
        }

        MsStatus::MS_OK
    } else {
        // 缓冲区满：阻塞等待
        drop(buffer);
        drop(_lock);

        channel.send_blocking(val_obj);

        MsStatus::MS_OK
    }
}
```

**Channel 线程级阻塞**：Channel 对象需要增加线程级同步原语（`Mutex` + `Condvar`），供 C API 的 `msChannelSend`/`msChannelRecv` 使用。这与内部协程暂停机制独立：

```rust
// src/async_runtime/channel.rs 扩展
struct Channel {
    buffer: RefCell<VecDeque<Object>>,
    capacity: usize,
    state: RefCell<ChannelState>,
    waiting_senders: RefCell<Vec<*mut MsObjHeader>>,
    waiting_receivers: RefCell<Vec<*mut MsObjHeader>>,

    // 线程级同步原语（供 C API 使用）
    sync_mutex: Mutex<()>,
    send_cvar: Condvar,     // 缓冲区非满时通知
    recv_cvar: Condvar,     // 缓冲区非空时通知
}
```

### 8. msChannelRecv — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msChannelRecv(
    vm: *mut crate::capi::vm::CApiVM,
    ch: *mut crate::capi::value::MsValue,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || ch.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let ch_obj = match capi_vm.unwrap_value(ch) {
        Some(obj) => obj,
        None => return std::ptr::null_mut(),
    };

    let Object::Ref(ptr) = &ch_obj else { return std::ptr::null_mut() };
    if unsafe { (**ptr).type_tag } != TypeTag::CHANNEL as u8 {
        return std::ptr::null_mut();
    }

    let channel = unsafe { read_channel(*ptr) };

    let mut buffer = channel.buffer.borrow_mut();
    if let Some(val) = buffer.pop_front() {
        // 缓冲区有数据
        // 如果有等待的协程发送者，唤醒
        if let Some(waiter) = channel.waiting_senders.borrow_mut().pop() {
            capi_vm.vm.event_loop.ready_queue.push_back(waiter);
        }
        drop(buffer);
        return capi_vm.wrap_value(val);
    } else if channel.is_closed() {
        // 已关闭且缓冲区为空
        drop(buffer);
        return capi_vm.wrap_value(Object::Nil);
    } else {
        // 缓冲区空且未关闭：阻塞等待
        drop(buffer);
        drop(_lock);

        let val = channel.recv_blocking();
        capi_vm.wrap_value(val)
    }
}
```

### 9. msChannelClose / msChannelIsClosed — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msChannelClose(
    vm: *mut crate::capi::vm::CApiVM,
    ch: *mut crate::capi::value::MsValue,
) -> MsStatus {
    if vm.is_null() || ch.is_null() {
        return MsStatus::MS_ERROR;
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let ch_obj = match capi_vm.unwrap_value(ch) {
        Some(obj) => obj,
        None => return MsStatus::MS_ERROR,
    };

    let Object::Ref(ptr) = &ch_obj else { return MsStatus::MS_ERROR };
    if unsafe { (**ptr).type_tag } != TypeTag::CHANNEL as u8 {
        return MsStatus::MS_ERROR;
    }

    let channel = unsafe { read_channel(*ptr) };

    channel.state.replace(ChannelState::Closed);

    // 唤醒所有等待的协程接收者（它们会收到 nil）
    for waiter in channel.waiting_receivers.borrow_mut().drain(..) {
        capi_vm.vm.event_loop.ready_queue.push_back(waiter);
    }

    // 唤醒所有线程级等待者
    channel.recv_cvar.notify_all();
    channel.send_cvar.notify_all();

    MsStatus::MS_OK
}

#[no_mangle]
pub unsafe extern "C" fn msChannelIsClosed(
    vm: *mut crate::capi::vm::CApiVM,
    ch: *mut crate::capi::value::MsValue,
) -> c_int {
    if vm.is_null() || ch.is_null() {
        return MS_FALSE;
    }

    let capi_vm = &mut *vm;

    let ch_obj = match capi_vm.unwrap_value(ch) {
        Some(obj) => obj,
        None => return MS_FALSE,
    };

    let Object::Ref(ptr) = &ch_obj else { return MS_FALSE };
    if unsafe { (**ptr).type_tag } != TypeTag::CHANNEL as u8 {
        return MS_FALSE;
    }

    let channel = unsafe { read_channel(*ptr) };
    if matches!(*channel.state.borrow(), ChannelState::Closed) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}
```

### 10. msGeneratorIter — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msGeneratorIter(
    vm: *mut crate::capi::vm::CApiVM,
    generator: *mut crate::capi::value::MsValue,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || generator.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;

    let gen_obj = match capi_vm.unwrap_value(generator) {
        Some(obj) => obj,
        None => return std::ptr::null_mut(),
    };

    let Object::Ref(ptr) = &gen_obj else { return std::ptr::null_mut() };
    if unsafe { (**ptr).type_tag } != TypeTag::GENERATOR as u8 {
        capi_vm.set_error("not a Generator object");
        return std::ptr::null_mut();
    }

    // Generator 自身即为迭代器（__iter__ 返回 self）
    generator
}
```

### 11. msGeneratorNext — src/capi/call.rs

```rust
#[no_mangle]
pub unsafe extern "C" fn msGeneratorNext(
    vm: *mut crate::capi::vm::CApiVM,
    generator: *mut crate::capi::value::MsValue,
    out: *mut *mut crate::capi::value::MsValue,
) -> MsStatus {
    if vm.is_null() || generator.is_null() || out.is_null() {
        return MsStatus::MS_ERROR;
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let gen_obj = match capi_vm.unwrap_value(generator) {
        Some(obj) => obj,
        None => return MsStatus::MS_ERROR,
    };

    let Object::Ref(ptr) = &gen_obj else { return MsStatus::MS_ERROR };
    if unsafe { (**ptr).type_tag } != TypeTag::GENERATOR as u8 {
        capi_vm.set_error("not a Generator object");
        return MsStatus::MS_ERROR;
    }

    let gen = unsafe { read_generator(*ptr) };

    match gen.state {
        GeneratorState::Exhausted => {
            // 迭代结束，不设置异常，仅返回 MS_ERROR 表示 StopIteration
            return MsStatus::MS_ERROR;
        }
        GeneratorState::Running => {
            capi_vm.set_error("generator already running");
            return MsStatus::MS_ERROR;
        }
        GeneratorState::Suspended => {}
    }

    // 恢复 Generator 执行
    let result = capi_vm.vm.resume_generator(*ptr);

    match result {
        Ok(value) => {
            *out = capi_vm.wrap_value(value);
            MsStatus::MS_OK
        }
        Err(_) => {
            // StopIteration 或运行时错误
            // 区分 StopIteration（正常结束）和真正的异常
            if capi_vm.vm.is_stop_iteration() {
                // 正常结束，不设置错误
                MsStatus::MS_ERROR
            } else {
                // 运行时错误，错误已在 VM 内部设置
                MsStatus::MS_ERROR
            }
        }
    }
}
```

### 12. Future 对象扩展 — src/vm/object.rs

Future 对象需要增加线程等待者字段，供 `msAwait` 使用：

```rust
struct Future {
    state: RefCell<FutureState>,
    waiters: RefCell<Vec<*mut MsObjHeader>>,                          // 协程等待者
    thread_waiters: RefCell<Vec<Arc<(Mutex<bool>, Condvar)>>>,       // 线程等待者
}
```

`thread_waiters` 仅在 C API `msAwait` 场景中使用。内部协程的 `AWAIT` 指令仍使用 `waiters` 字段和 EventLoop 调度。

### 13. Channel 对象扩展 — src/async_runtime/channel.rs

Channel 对象增加线程级同步原语：

```rust
struct Channel {
    buffer: RefCell<VecDeque<Object>>,
    capacity: usize,
    state: RefCell<ChannelState>,
    waiting_senders: RefCell<Vec<*mut MsObjHeader>>,
    waiting_receivers: RefCell<Vec<*mut MsObjHeader>>,

    // 线程级同步（C API 使用）
    sync_mutex: Mutex<()>,
    send_cvar: Condvar,
    recv_cvar: Condvar,
}
```

新增阻塞方法：

```rust
impl Channel {
    fn send_blocking(&self, val: Object) {
        let mut guard = self.sync_mutex.lock().unwrap();
        while self.buffer.borrow().len() >= self.capacity && !self.is_closed() {
            guard = self.send_cvar.wait(guard).unwrap();
        }
        if self.is_closed() {
            return; // 或 panic
        }
        self.buffer.borrow_mut().push_back(val);
        self.recv_cvar.notify_one();
    }

    fn recv_blocking(&self) -> Object {
        let mut guard = self.sync_mutex.lock().unwrap();
        while self.buffer.borrow().is_empty() && !self.is_closed() {
            guard = self.recv_cvar.wait(guard).unwrap();
        }
        if let Some(val) = self.buffer.borrow_mut().pop_front() {
            self.send_cvar.notify_one();
            val
        } else {
            Object::Nil
        }
    }
}
```

### 14. CApiVM 辅助方法

`src/capi/call.rs` 依赖 `CApiVM` 已有方法（Task 66-70 定义）：

| 方法 | 说明 |
|---|---|
| `unwrap_value(MsValue*) -> Option<Object>` | 从 C 侧 MsValue* 提取内部 Object |
| `wrap_value(Object) -> *mut MsValue` | 将 Object 包装为 C 侧 MsValue* |
| `set_error(&str)` | 设置 VM 异常状态 |
| `mutex` | 内部 Mutex 字段 |
| `vm` | 内部 VM 实例引用 |

### 15. mslang.h 更新

无需变更。`call.h` 已在 Task 70 启用。cbindgen 重新生成 `call.h` 时会自动包含新增的函数声明。

## 验证标准

1. `msCallAsync` 调用 mslang 函数后立即返回 Future（PENDING 状态）
2. `msAwait` 阻塞并正确返回 Future 的结果值
3. `msFutureState` 正确反映 PENDING / RESOLVED / REJECTED 三种状态
4. `msFutureReject` 设置 Rejected 状态后，`msAwait` 返回 NULL 且异常可查
5. C async 函数桥接：C 函数 resolve Future → mslang 脚本 await → 获取结果
6. C async 函数桥接：C 函数 reject Future → mslang 脚本 await → 捕获异常
7. `msChannel` 创建指定缓冲区大小的 Channel
8. `msChannelSend`/`msChannelRecv` 在 C 与 mslang 之间正确传递数据
9. 缓冲区满时 `msChannelSend` 阻塞，缓冲区空时 `msChannelRecv` 阻塞
10. `msChannelClose` 关闭后 `msChannelIsClosed` 返回 MS_TRUE
11. 关闭后 `msChannelSend` 返回 MS_ERROR
12. 关闭后缓冲区清空前 `msChannelRecv` 仍可获取剩余数据
13. `msGeneratorIter` 返回 Generator 自身
14. `msGeneratorNext` 逐个获取 yield 值
15. Generator 迭代结束后 `msGeneratorNext` 返回 MS_ERROR
16. 多个异步调用可通过 `msAwait` 依次等待
17. 所有 API 线程安全（per-VM 互斥锁保护）
18. `cargo build --features capi` 编译无错误
19. `cargo test --features capi` 全部通过

## 测试用例

### Rust 单元测试

```rust
#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    use super::*;
    use crate::capi::vm::*;
    use crate::capi::value::*;

    #[test]
    fn test_async_call_and_await() {
        // msExecString: async fn compute(x) { return x * 2 }
        // MsValue* compute = msGetGlobal(vm, "compute")
        // MsValue* future = msCallAsync(vm, compute, [msInt(21)], 1)
        // 断言 msFutureState(vm, future) == MS_FUTURE_PENDING 或 MS_FUTURE_RESOLVED
        // MsValue* result = msAwait(vm, future)
        // 断言 msToInt(vm, result) == 42
    }

    #[test]
    fn test_future_resolve_reject() {
        // 创建 Future:
        //   MsValue* future = msCallAsync(vm, someFunc, NULL, 0)
        //
        // 测试 resolve:
        //   msFutureResolve(vm, future, msInt(100))
        //   断言 msFutureState(vm, future) == MS_FUTURE_RESOLVED
        //   MsValue* result = msAwait(vm, future)
        //   断言 msToInt(vm, result) == 100
        //
        // 测试 reject:
        //   MsValue* future2 = msCallAsync(vm, errorFunc, NULL, 0)
        //   msFutureReject(vm, future2, msString(vm, "fail"))
        //   断言 msFutureState(vm, future2) == MS_FUTURE_REJECTED
        //   MsValue* result2 = msAwait(vm, future2)
        //   断言 result2 == NULL
        //   断言 msErrOccurred(vm)
    }

    #[test]
    fn test_c_async_function() {
        // 注册 C async 函数 async_read:
        //   void async_read(MsVM* vm, MsValue* const* args, int nargs, MsValue* future) {
        //       // 模拟异步操作
        //       msFutureResolve(vm, future, msString(vm, "data"));
        //   }
        //
        // mslang 脚本:
        //   result = await async_read("file.txt")
        //   print(result)
        //
        // 验证输出 "data"
    }

    #[test]
    fn test_channel_send_recv() {
        // MsValue* ch = msChannel(vm, 3)
        // msChannelSend(vm, ch, msInt(1)) → MS_OK
        // msChannelSend(vm, ch, msInt(2)) → MS_OK
        // msChannelSend(vm, ch, msInt(3)) → MS_OK
        //
        // MsValue* v1 = msChannelRecv(vm, ch) → 1
        // MsValue* v2 = msChannelRecv(vm, ch) → 2
        // MsValue* v3 = msChannelRecv(vm, ch) → 3
        //
        // 断言 msToInt(vm, v1) == 1
        // 断言 msToInt(vm, v2) == 2
        // 断言 msToInt(vm, v3) == 3
    }

    #[test]
    fn test_channel_close() {
        // MsValue* ch = msChannel(vm, 5)
        // msChannelSend(vm, ch, msString(vm, "a")) → MS_OK
        // msChannelClose(vm, ch) → MS_OK
        // 断言 msChannelIsClosed(vm, ch) == MS_TRUE
        //
        // 关闭后仍可接收缓冲区数据
        // MsValue* v = msChannelRecv(vm, ch) → "a"
        //
        // 缓冲区空后接收返回 nil
        // MsValue* nil_val = msChannelRecv(vm, ch)
        // 断言 msIsNil(nil_val)
        //
        // 关闭后发送返回错误
        // MsStatus s = msChannelSend(vm, ch, msInt(1))
        // 断言 s == MS_ERROR
    }

    #[test]
    fn test_generator_iteration() {
        // msExecString:
        //   fn gen3() { yield 10; yield 20; yield 30 }
        //   g = gen3()
        //
        // MsValue* g = msGetGlobal(vm, "g")
        // MsValue* iter = msGeneratorIter(vm, g)
        // 断言 iter == g (同一指针)
        //
        // MsValue* out = NULL;
        // 断言 msGeneratorNext(vm, g, &out) == MS_OK; msToInt(vm, out) == 10
        // 断言 msGeneratorNext(vm, g, &out) == MS_OK; msToInt(vm, out) == 20
        // 断言 msGeneratorNext(vm, g, &out) == MS_OK; msToInt(vm, out) == 30
        // 断言 msGeneratorNext(vm, g, &out) == MS_ERROR (迭代结束)
    }
}
```

### C 集成测试 — test_async_channel.c

```c
#include <mslang.h>
#include <stdio.h>
#include <assert.h>

// C async 函数示例
static void async_compute(MsVM* vm, MsValue* const* args, int nargs, MsValue* future) {
    if (nargs < 1) {
        msFutureReject(vm, future, msString(vm, "need 1 arg"));
        return;
    }
    int64_t val = msToInt(vm, args[0]);
    MsValue* result = msInt(val * 2);
    msFutureResolve(vm, future, result);
}

int main(void) {
    MsVM* vm = msVmNew();

    /* === 异步调用测试 === */
    const char* script =
        "async fn double_it(x) {\n"
        "  return x * 2\n"
        "}\n";

    assert(msExecString(vm, script, "test.ms") == MS_OK);

    MsValue* double_fn = msGetGlobal(vm, "double_it");
    msRoot(vm, double_fn);

    MsValue* arg = msInt(21);
    MsValue* future = msCallAsync(vm, double_fn, (MsValue* const[]){arg}, 1);
    assert(future != NULL);

    MsValue* result = msAwait(vm, future);
    assert(result != NULL);
    assert(msToInt(vm, result) == 42);

    msUnroot(vm, result);
    msUnroot(vm, arg);
    msUnroot(vm, future);
    msUnroot(vm, double_fn);

    /* === Future 状态测试 === */
    MsValue* future2 = msCallAsync(vm, double_fn, (MsValue* const[]){msInt(5)}, 1);
    msRoot(vm, future2);
    assert(msFutureState(vm, future2) != MS_FUTURE_REJECTED);
    result = msAwait(vm, future2);
    assert(msToInt(vm, result) == 10);
    assert(msFutureState(vm, future2) == MS_FUTURE_RESOLVED);
    msUnroot(vm, result);
    msUnroot(vm, future2);

    /* === Channel 测试 === */
    MsValue* ch = msChannel(vm, 3);
    msRoot(vm, ch);

    assert(msChannelSend(vm, ch, msInt(1)) == MS_OK);
    assert(msChannelSend(vm, ch, msInt(2)) == MS_OK);
    assert(msChannelSend(vm, ch, msInt(3)) == MS_OK);

    MsValue* v1 = msChannelRecv(vm, ch);
    MsValue* v2 = msChannelRecv(vm, ch);
    MsValue* v3 = msChannelRecv(vm, ch);
    assert(msToInt(vm, v1) == 1);
    assert(msToInt(vm, v2) == 2);
    assert(msToInt(vm, v3) == 3);

    assert(msChannelClose(vm, ch) == MS_OK);
    assert(msChannelIsClosed(vm, ch) == MS_TRUE);

    /* 关闭后发送应失败 */
    assert(msChannelSend(vm, ch, msInt(99)) == MS_ERROR);

    /* 关闭后缓冲区为空，接收返回 nil */
    MsValue* nil_val = msChannelRecv(vm, ch);
    assert(msIsNil(nil_val));

    msUnroot(vm, v1);
    msUnroot(vm, v2);
    msUnroot(vm, v3);
    msUnroot(vm, nil_val);
    msUnroot(vm, ch);

    /* === Generator 测试 === */
    const char* gen_script =
        "fn countdown(n) {\n"
        "  while n > 0 {\n"
        "    yield n\n"
        "    n = n - 1\n"
        "  }\n"
        "}\n"
        "gen = countdown(3)\n";

    assert(msExecString(vm, gen_script, "gen.ms") == MS_OK);

    MsValue* gen = msGetGlobal(vm, "gen");
    msRoot(vm, gen);

    MsValue* iter = msGeneratorIter(vm, gen);
    assert(iter == gen);

    MsValue* out = NULL;
    assert(msGeneratorNext(vm, gen, &out) == MS_OK);
    assert(msToInt(vm, out) == 3);
    msUnroot(vm, out);

    assert(msGeneratorNext(vm, gen, &out) == MS_OK);
    assert(msToInt(vm, out) == 2);
    msUnroot(vm, out);

    assert(msGeneratorNext(vm, gen, &out) == MS_OK);
    assert(msToInt(vm, out) == 1);
    msUnroot(vm, out);

    assert(msGeneratorNext(vm, gen, &out) == MS_ERROR);

    msUnroot(vm, gen);
    msVmFree(vm);

    printf("test_async_channel: all passed\n");
    return 0;
}
```

编译与运行：

```bash
cc -I include -o test_async_channel test_async_channel.c -L target/debug -lmslang
./test_async_channel
```
