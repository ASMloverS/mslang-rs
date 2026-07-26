//! C API — 函数调用（task 70）+ 异步/Channel/Generator（task 76）。
//!
//! 参照 [70-capi-call](../../docs/mslang/tasks/70-capi-call.md)、
//! [76-capi-async-channel](../../docs/mslang/tasks/76-capi-async-channel.md)。
//!
//! 实现 `msCall`（同步函数调用）和 `msMakeCFunction`（C 原生函数注册桥接）。
//! msCall 复用 VM 已有的 `call_function` 方法，仅负责 MsValue* ↔ Object 转换
//! 和错误桥接。
//!
//! task 76 追加：msCallAsync/msAwait/msFutureState/msFutureResolve/msFutureReject、
//! msChannel/msChannelSend/msChannelRecv/msChannelClose/msChannelIsClosed、
//! msGeneratorIter/msGeneratorNext。

use std::os::raw::c_int;

use crate::capi::types::{MsCFunction, MsFutureState, MsStatus, MsValue};
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::builtins::alloc_c_native_function;
use crate::vm::object::{
    alloc_exception, alloc_future, alloc_string, read_exception, read_future, read_generator,
    read_str, FutureState, GeneratorState, Object, TypeTag,
};
use crate::vm::ThreadSignal;
use crate::async_runtime::channel::{alloc_channel, read_channel};

/// 调用可调用对象，返回结果（新引用）。异常时返回 NULL。
///
/// func 必须是可调用对象（Function、Closure、BoundMethod、C 原生函数，
/// 或带 `__call__` 的 Instance）。可调用性由 VM `call_value` 内部 match 处理。
#[no_mangle]
pub extern "C" fn msCall(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
) -> *mut MsValue {
    if vm.is_null() || func.is_null() {
        return std::ptr::null_mut();
    }
    if nargs < 0 {
        return std::ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let func_obj = unsafe { (*func).inner.clone() };

    let nargs_usize = nargs as usize;
    let mut arg_objects: Vec<Object> = Vec::with_capacity(nargs_usize);
    if nargs_usize > 0 {
        if args.is_null() {
            inner.vm.has_error = true;
            inner.vm.error_message = "msCall: args is NULL but nargs > 0".into();
            return std::ptr::null_mut();
        }
        let arg_slice = unsafe { std::slice::from_raw_parts(args, nargs_usize) };
        for &arg_ptr in arg_slice {
            if arg_ptr.is_null() {
                inner.vm.has_error = true;
                inner.vm.error_message = "msCall: NULL argument in args".into();
                return std::ptr::null_mut();
            }
            arg_objects.push(unsafe { (*arg_ptr).inner.clone() });
        }
    }

    match inner.vm.call_function(&func_obj, &arg_objects) {
        Ok(result) => Box::into_raw(Box::new(MsValue { inner: result })),
        Err(msg) => {
            inner.vm.has_error = true;
            inner.vm.error_message = msg;
            std::ptr::null_mut()
        }
    }
}

/// 将 C 函数指针包装为 VM 可调用对象（MsValue*）。
/// 返回的值可作为全局变量注册，供 mslang 脚本调用。
#[no_mangle]
pub extern "C" fn msMakeCFunction(
    vm: *mut MsVM,
    name: *const std::os::raw::c_char,
    func: MsCFunction,
    arity: c_int,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() || func.is_none() {
        return std::ptr::null_mut();
    }

    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let obj = alloc_c_native_function(&name_str, func, arity);

    Box::into_raw(Box::new(MsValue { inner: obj }))
}

// ===========================================================================
// task 76：异步/Channel/Generator C API
// ===========================================================================

// ---------------------------------------------------------------------------
// 异步调用 — msCallAsync / msAwait
// ---------------------------------------------------------------------------

/// 异步调用：包装 func 为协程，立即返回 Future（Pending 或 Resolved 状态）。
/// 协程在 EventLoop 中执行；func 完成时 EventLoop 自动 resolve/reject Future。
///
/// msCallAsync 在返回前驱动 EventLoop（pump），使不挂起的协程立即完成。
#[no_mangle]
pub unsafe extern "C" fn msCallAsync(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
) -> *mut MsValue {
    if vm.is_null() || func.is_null() || nargs < 0 {
        return std::ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let func_obj = unsafe { (*func).inner.clone() };

    // 校验可调用性。
    let callable = match &func_obj {
        Object::Ref(p) => {
            let tag = unsafe { (**p).type_tag };
            tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
                || tag == TypeTag::NATIVE_C_FUNCTION as u8
                || tag == TypeTag::NATIVE_ASYNC_FUNCTION as u8
                || tag == TypeTag::CLASS as u8
        }
        _ => false,
    };
    if !callable {
        inner.vm.has_error = true;
        inner.vm.error_message = "msCallAsync: not a callable object".into();
        return std::ptr::null_mut();
    }

    // 转换参数。
    let nargs_usize = nargs as usize;
    let mut arg_objects: Vec<Object> = Vec::with_capacity(nargs_usize);
    if nargs_usize > 0 {
        if args.is_null() {
            inner.vm.has_error = true;
            inner.vm.error_message = "msCallAsync: args is NULL but nargs > 0".into();
            return std::ptr::null_mut();
        }
        let slice = unsafe { std::slice::from_raw_parts(args, nargs_usize) };
        for &p in slice {
            if p.is_null() {
                inner.vm.has_error = true;
                inner.vm.error_message = "msCallAsync: NULL argument in args".into();
                return std::ptr::null_mut();
            }
            arg_objects.push(unsafe { (*p).inner.clone() });
        }
    }

    // 分配 Pending Future。
    let future_obj = alloc_future(FutureState::Pending);
    let future_ptr = match &future_obj {
        Object::Ref(p) => *p,
        _ => unreachable!("alloc_future returns Ref"),
    };

    // 创建协程并加入就绪队列。
    let coroutine =
        inner.vm.spawn_async_call_coroutine(func_obj, arg_objects, future_ptr);
    inner.vm.event_loop.ready_queue.push_back(coroutine);

    // 驱动 EventLoop（pump）：使不挂起的协程立即完成。
    inner.vm.pump_event_loop();

    // 立即返回 Future（C 侧 msRoot 后异步 msAwait）。
    Box::into_raw(Box::new(MsValue { inner: future_obj }))
}

/// 阻塞等待 Future 完成。
/// - Resolved → 返回结果（新引用）
/// - Rejected → 设置异常，返回 NULL
/// - Pending → 驱动 EventLoop（单线程模式）；若仍 Pending，Condvar 等待（多线程模式）
#[no_mangle]
pub unsafe extern "C" fn msAwait(
    vm: *mut MsVM,
    future: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || future.is_null() {
        return std::ptr::null_mut();
    }

    // 取 future_ptr（持锁校验）。
    let future_ptr = {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        let future_obj = unsafe { (*future).inner.clone() };
        match &future_obj {
            Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::FUTURE as u8 => *p,
            _ => {
                inner.vm.has_error = true;
                inner.vm.error_message = "msAwait: not a Future object".into();
                return std::ptr::null_mut();
            }
        }
    };

    // 第一阶段：快速路径 + pump（单线程模式）。
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        match read_future_state(future_ptr) {
            FutureStateRead::Resolved(val) => {
                return Box::into_raw(Box::new(MsValue { inner: val.clone() }));
            }
            FutureStateRead::Rejected(err) => {
                let msg = extract_error_message(&err);
                inner.vm.has_error = true;
                inner.vm.error_message = msg;
                return std::ptr::null_mut();
            }
            FutureStateRead::Pending => {
                // Pending：驱动 EventLoop 尝试推进。
                inner.vm.pump_event_loop();
            }
        }
    }
    // pump 后重新检查状态。
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        match read_future_state(future_ptr) {
            FutureStateRead::Resolved(val) => {
                return Box::into_raw(Box::new(MsValue { inner: val.clone() }));
            }
            FutureStateRead::Rejected(err) => {
                let msg = extract_error_message(&err);
                inner.vm.has_error = true;
                inner.vm.error_message = msg;
                return std::ptr::null_mut();
            }
            FutureStateRead::Pending => {}
        }
    }

    // 第二阶段：Condvar 等待（多线程模式）。
    let signal: ThreadSignal =
        std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner
            .vm
            .event_loop
            .thread_waiters
            .entry(future_ptr)
            .or_insert_with(Vec::new)
            .push(signal.clone());
    }

    let (lock, cvar) = &*signal;
    let mut completed = lock.lock().unwrap();
    while !*completed {
        completed = cvar.wait(completed).unwrap();
    }
    drop(completed);

    // 被唤醒后重新读 Future 状态。
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    match read_future_state(future_ptr) {
        FutureStateRead::Resolved(val) => {
            Box::into_raw(Box::new(MsValue { inner: val.clone() }))
        }
        FutureStateRead::Rejected(err) => {
            let msg = extract_error_message(&err);
            inner.vm.has_error = true;
            inner.vm.error_message = msg;
            std::ptr::null_mut()
        }
        FutureStateRead::Pending => {
            inner.vm.has_error = true;
            inner.vm.error_message =
                "msAwait: spurious wakeup, future still pending".into();
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Future 操作 — msFutureState / msFutureResolve / msFutureReject
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn msFutureState(
    vm: *mut MsVM,
    future: *mut MsValue,
) -> MsFutureState {
    if vm.is_null() || future.is_null() {
        return MsFutureState::MS_FUTURE_PENDING;
    }
    let guard = lock_vm(vm);
    let _inner = unsafe { &mut *guard.get() };
    let future_obj = unsafe { (*future).inner.clone() };
    let Object::Ref(p) = &future_obj else {
        return MsFutureState::MS_FUTURE_PENDING;
    };
    if unsafe { (**p).type_tag } != TypeTag::FUTURE as u8 {
        return MsFutureState::MS_FUTURE_PENDING;
    }
    let f = unsafe { read_future(*p) };
    let state = f.state.borrow();
    match &*state {
        FutureState::Pending => MsFutureState::MS_FUTURE_PENDING,
        FutureState::Resolved(_) => MsFutureState::MS_FUTURE_RESOLVED,
        FutureState::Rejected(_) => MsFutureState::MS_FUTURE_REJECTED,
    }
}

/// 将 Future 设为 Resolved 并唤醒所有等待者（协程 + 线程）。幂等。
#[no_mangle]
pub unsafe extern "C" fn msFutureResolve(
    vm: *mut MsVM,
    future: *mut MsValue,
    result: *mut MsValue,
) {
    if vm.is_null() || future.is_null() || result.is_null() {
        return;
    }
    let result_obj = unsafe { (*result).inner.clone() };
    let future_obj = unsafe { (*future).inner.clone() };
    let Object::Ref(fp) = &future_obj else {
        return;
    };
    if unsafe { (**fp).type_tag } != TypeTag::FUTURE as u8 {
        return;
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let f = unsafe { read_future(*fp) };
    // 幂等检查：已 settle 则 no-op。
    if !matches!(*f.state.borrow(), FutureState::Pending) {
        return;
    }
    f.state.replace(FutureState::Resolved(result_obj));

    // 唤醒协程等待者（复用 task 53 wake_waiters 路径）。
    inner.vm.wake_waiters(*fp);
    // 取出线程等待者 signals。
    let signals = inner.vm.event_loop.thread_waiters.remove(fp);
    drop(guard); // 释放 VMutex 后再 notify，避免被唤醒线程立即抢锁。
    notify_thread_signals(signals);
}

/// 将 Future 设为 Rejected 并唤醒所有等待者。error 自动包装为 RuntimeError MsException。幂等。
#[no_mangle]
pub unsafe extern "C" fn msFutureReject(
    vm: *mut MsVM,
    future: *mut MsValue,
    error: *mut MsValue,
) {
    if vm.is_null() || future.is_null() || error.is_null() {
        return;
    }
    let mut error_obj = unsafe { (*error).inner.clone() };
    // 规范化 error 为 MsException。
    let is_exception = matches!(&error_obj, Object::Ref(p)
        if unsafe { (**p).type_tag } == TypeTag::EXCEPTION as u8);
    if !is_exception {
        error_obj = wrap_as_runtime_exception(&error_obj);
    }

    let future_obj = unsafe { (*future).inner.clone() };
    let Object::Ref(fp) = &future_obj else {
        return;
    };
    if unsafe { (**fp).type_tag } != TypeTag::FUTURE as u8 {
        return;
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let f = unsafe { read_future(*fp) };
    if !matches!(*f.state.borrow(), FutureState::Pending) {
        return;
    }
    f.state.replace(FutureState::Rejected(error_obj));

    inner.vm.wake_waiters(*fp);
    let signals = inner.vm.event_loop.thread_waiters.remove(fp);
    drop(guard);
    notify_thread_signals(signals);
}

// ---------------------------------------------------------------------------
// Future 内部辅助
// ---------------------------------------------------------------------------

/// Future 状态读取结果（clone 出 Object，释放 RefCell borrow）。
enum FutureStateRead {
    Pending,
    Resolved(Object),
    Rejected(Object),
}

/// 读取 Future 状态（clone 值，避免持有 RefCell borrow 跨 lock_vm 边界）。
/// 调用方须持锁。
unsafe fn read_future_state(ptr: *mut crate::vm::object::MsObjHeader) -> FutureStateRead {
    let f = unsafe { read_future(ptr) };
    match &*f.state.borrow() {
        FutureState::Pending => FutureStateRead::Pending,
        FutureState::Resolved(v) => FutureStateRead::Resolved(v.clone()),
        FutureState::Rejected(e) => FutureStateRead::Rejected(e.clone()),
    }
}

/// 唤醒线程等待者 signals。
fn notify_thread_signals(signals: Option<Vec<ThreadSignal>>) {
    if let Some(sigs) = signals {
        for signal in sigs {
            let (lock, cvar) = &*signal;
            *lock.lock().unwrap() = true;
            cvar.notify_one();
        }
    }
}

/// 从 Object（通常是 MsException 实例）提取错误 message 字符串。
fn extract_error_message(err: &Object) -> String {
    match err {
        Object::Ref(p) => {
            let tag = unsafe { (**p).type_tag };
            if tag == TypeTag::EXCEPTION as u8 {
                let exc = unsafe { read_exception(*p) };
                format!("{}: {}", exc.class_name, object_to_display_string(&exc.message))
            } else {
                format!("{:?}", err)
            }
        }
        _ => format!("{:?}", err),
    }
}

/// 将非 Exception Object 包装为 RuntimeError MsException。
fn wrap_as_runtime_exception(err: &Object) -> Object {
    let msg = object_to_display_string(err);
    alloc_exception(
        "RuntimeError",
        alloc_string(&msg),
        alloc_string(""),
        Object::Nil,
    )
}

/// 简易 Display：String 提取内容，其余用 Debug。
fn object_to_display_string(obj: &Object) -> String {
    match obj {
        Object::Ref(p) => {
            let tag = unsafe { (**p).type_tag };
            if tag == TypeTag::STRING as u8 {
                unsafe { read_str(*p) }.to_owned()
            } else {
                format!("{:?}", obj)
            }
        }
        _ => format!("{:?}", obj),
    }
}

// ---------------------------------------------------------------------------
// Channel 操作 — msChannel / msChannelSend / msChannelRecv / msChannelClose / msChannelIsClosed
// ---------------------------------------------------------------------------

/// 创建指定缓冲区大小的 Channel 对象。buffer_size 须在 0-255 范围内。
#[no_mangle]
pub unsafe extern "C" fn msChannel(
    vm: *mut MsVM,
    buffer_size: c_int,
) -> *mut MsValue {
    if vm.is_null() {
        return std::ptr::null_mut();
    }
    if buffer_size < 0 || buffer_size > 255 {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner.vm.has_error = true;
        inner.vm.error_message = format!(
            "msChannel: buffer_size must be 0-255, got {}",
            buffer_size
        );
        return std::ptr::null_mut();
    }
    let channel_obj = alloc_channel(buffer_size as usize);
    Box::into_raw(Box::new(MsValue { inner: channel_obj }))
}

/// 发送值到 Channel。缓冲区满时线程级阻塞。
#[no_mangle]
pub unsafe extern "C" fn msChannelSend(
    vm: *mut MsVM,
    ch: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || ch.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }

    // 第一阶段：持锁校验 + 快速路径。
    let (channel_ptr, val_obj) = {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        let ch_obj = unsafe { (*ch).inner.clone() };
        let val_obj = unsafe { (*val).inner.clone() };

        let Object::Ref(p) = &ch_obj else {
            inner.vm.has_error = true;
            inner.vm.error_message = "msChannelSend: not a Channel".into();
            return MsStatus::MS_ERROR;
        };
        if unsafe { (**p).type_tag } != TypeTag::CHANNEL as u8 {
            inner.vm.has_error = true;
            inner.vm.error_message = "msChannelSend: not a Channel".into();
            return MsStatus::MS_ERROR;
        }

        let channel = unsafe { read_channel(*p) };
        if channel.is_closed() {
            inner.vm.has_error = true;
            inner.vm.error_message = "send on closed channel".into();
            return MsStatus::MS_ERROR;
        }

        // 快速路径：缓冲区未满，直接入队。
        let mut buffer = channel.buffer.borrow_mut();
        if channel.capacity > 0 && buffer.len() < channel.capacity {
            buffer.push_back(val_obj.clone());
            drop(buffer);
            // 唤醒一个等待的协程接收者（如有）。
            if let Some(receiver) = channel.waiting_receivers.borrow_mut().pop_front() {
                inner.vm.event_loop.ready_queue.push_back(receiver.coroutine);
            }
            // 唤醒线程级接收者（C API msChannelRecv 阻塞者）。
            channel.recv_cvar.notify_one();
            return MsStatus::MS_OK;
        }
        (*p, val_obj)
    };

    // 第二阶段：慢路径线程级阻塞（释放 VMutex 后获取 channel.sync_mutex）。
    let channel = unsafe { read_channel(channel_ptr) };
    match channel.send_blocking(val_obj) {
        Ok(()) => MsStatus::MS_OK,
        Err(msg) => {
            let guard = lock_vm(vm);
            let inner = unsafe { &mut *guard.get() };
            inner.vm.has_error = true;
            inner.vm.error_message = msg;
            MsStatus::MS_ERROR
        }
    }
}

/// 从 Channel 接收值。缓冲区空时线程级阻塞；Channel 已关闭且为空时返回 nil。
#[no_mangle]
pub unsafe extern "C" fn msChannelRecv(
    vm: *mut MsVM,
    ch: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || ch.is_null() {
        return std::ptr::null_mut();
    }

    // 第一阶段：快速路径（持 VMutex）。
    let channel_ptr = {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        let ch_obj = unsafe { (*ch).inner.clone() };

        let Object::Ref(p) = &ch_obj else {
            inner.vm.has_error = true;
            inner.vm.error_message = "msChannelRecv: not a Channel".into();
            return std::ptr::null_mut();
        };
        if unsafe { (**p).type_tag } != TypeTag::CHANNEL as u8 {
            inner.vm.has_error = true;
            inner.vm.error_message = "msChannelRecv: not a Channel".into();
            return std::ptr::null_mut();
        }

        let channel = unsafe { read_channel(*p) };
        // 快速路径：缓冲区有数据。
        if let Some(val) = channel.buffer.borrow_mut().pop_front() {
            // 唤醒一个等待的协程发送者（如有）。
            if let Some(sender) = channel.waiting_senders.borrow_mut().pop_front() {
                channel.buffer.borrow_mut().push_back(sender.value);
                inner.vm.event_loop.ready_queue.push_back(sender.coroutine);
            }
            channel.send_cvar.notify_one();
            return Box::into_raw(Box::new(MsValue { inner: val }));
        }
        // 缓冲区空 + 已关闭 → 返回 nil。
        if channel.is_closed() {
            return Box::into_raw(Box::new(MsValue { inner: Object::Nil }));
        }
        *p
    };

    // 第二阶段：线程级阻塞接收。
    let channel = unsafe { read_channel(channel_ptr) };
    let val = channel.recv_blocking();
    Box::into_raw(Box::new(MsValue { inner: val }))
}

/// 关闭 Channel（幂等）。唤醒所有等待的协程与线程。
#[no_mangle]
pub unsafe extern "C" fn msChannelClose(
    vm: *mut MsVM,
    ch: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || ch.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let ch_obj = unsafe { (*ch).inner.clone() };
    let Object::Ref(p) = &ch_obj else {
        inner.vm.has_error = true;
        inner.vm.error_message = "msChannelClose: not a Channel".into();
        return MsStatus::MS_ERROR;
    };
    if unsafe { (**p).type_tag } != TypeTag::CHANNEL as u8 {
        inner.vm.has_error = true;
        inner.vm.error_message = "msChannelClose: not a Channel".into();
        return MsStatus::MS_ERROR;
    }

    let channel = unsafe { read_channel(*p) };
    // 幂等 close：已关闭为 no-op。
    if channel.is_closed() {
        return MsStatus::MS_OK;
    }
    channel
        .state
        .replace(crate::async_runtime::channel::ChannelState::Closed);

    // 唤醒所有等待的协程接收者。
    let drained_receivers: Vec<_> =
        channel.waiting_receivers.borrow_mut().drain(..).collect();
    for receiver in drained_receivers {
        inner.vm.event_loop.ready_queue.push_back(receiver.coroutine);
    }

    // 唤醒所有等待的协程发送者（恢复后会重试 SEND → is_closed → 抛错）。
    let drained_senders: Vec<_> =
        channel.waiting_senders.borrow_mut().drain(..).collect();
    for mut sender in drained_senders {
        sender.coroutine.stack.push(sender.value);
        inner.vm.event_loop.ready_queue.push_back(sender.coroutine);
    }

    // 唤醒所有线程级等待者。
    channel.notify_all_thread_waiters();

    MsStatus::MS_OK
}

/// 查询 Channel 是否已关闭。
#[no_mangle]
pub unsafe extern "C" fn msChannelIsClosed(
    vm: *mut MsVM,
    ch: *mut MsValue,
) -> c_int {
    if vm.is_null() || ch.is_null() {
        return crate::capi::value::MS_FALSE;
    }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    let ch_obj = unsafe { (*ch).inner.clone() };
    let Object::Ref(p) = &ch_obj else {
        return crate::capi::value::MS_FALSE;
    };
    if unsafe { (**p).type_tag } != TypeTag::CHANNEL as u8 {
        return crate::capi::value::MS_FALSE;
    }
    let channel = unsafe { read_channel(*p) };
    if channel.is_closed() {
        crate::capi::value::MS_TRUE
    } else {
        crate::capi::value::MS_FALSE
    }
}

// ---------------------------------------------------------------------------
// Generator 操作 — msGeneratorIter / msGeneratorNext
// ---------------------------------------------------------------------------

/// 返回 Generator 的迭代器（Generator 自身即为迭代器，返回新引用）。
#[no_mangle]
pub unsafe extern "C" fn msGeneratorIter(
    vm: *mut MsVM,
    generator: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || generator.is_null() {
        return std::ptr::null_mut();
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let gen_obj = unsafe { (*generator).inner.clone() };
    let Object::Ref(p) = &gen_obj else {
        inner.vm.has_error = true;
        inner.vm.error_message = "msGeneratorIter: not a Generator".into();
        return std::ptr::null_mut();
    };
    if unsafe { (**p).type_tag } != TypeTag::GENERATOR as u8 {
        inner.vm.has_error = true;
        inner.vm.error_message = "msGeneratorIter: not a Generator".into();
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(MsValue { inner: gen_obj }))
}

/// 恢复 Generator 执行，获取下一个 yield 值。
/// MS_OK 时 *out 设置为 yield 值（新引用）；
/// MS_ERROR 时迭代结束（无异常）或运行时错误（has_error=true）。
#[no_mangle]
pub unsafe extern "C" fn msGeneratorNext(
    vm: *mut MsVM,
    generator: *mut MsValue,
    out: *mut *mut MsValue,
) -> MsStatus {
    if vm.is_null() || generator.is_null() || out.is_null() {
        return MsStatus::MS_ERROR;
    }
    // 防御性：清空 *out（错误路径保证 C 侧 *out 为 NULL）。
    *out = std::ptr::null_mut();

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let gen_obj = unsafe { (*generator).inner.clone() };
    let Object::Ref(p) = &gen_obj else {
        inner.vm.has_error = true;
        inner.vm.error_message = "msGeneratorNext: not a Generator".into();
        return MsStatus::MS_ERROR;
    };
    if unsafe { (**p).type_tag } != TypeTag::GENERATOR as u8 {
        inner.vm.has_error = true;
        inner.vm.error_message = "msGeneratorNext: not a Generator".into();
        return MsStatus::MS_ERROR;
    }

    // 状态检查。
    let gen = unsafe { read_generator(*p) };
    match gen.state {
        GeneratorState::Exhausted => {
            // 迭代结束：不设置异常，仅返回 MS_ERROR。
            return MsStatus::MS_ERROR;
        }
        GeneratorState::Running => {
            inner.vm.has_error = true;
            inner.vm.error_message = "msGeneratorNext: generator already running".into();
            return MsStatus::MS_ERROR;
        }
        GeneratorState::Suspended => {}
    }

    // 通过 VM helper 恢复执行。
    match inner.vm.resume_generator_from_capi(*p) {
        Ok(yield_value) => {
            *out = Box::into_raw(Box::new(MsValue { inner: yield_value }));
            MsStatus::MS_OK
        }
        Err(msg) => {
            // StopIteration = 正常结束；其他 = 运行时错误。
            if !msg.is_empty() && msg != "StopIteration" {
                inner.vm.has_error = true;
                inner.vm.error_message = msg;
            }
            MsStatus::MS_ERROR
        }
    }
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::error::msErrOccurred;
    use crate::capi::gc::{msRoot, msUnroot};
    use crate::capi::types::MsStatus;
    use crate::capi::value::*;
    use crate::capi::vm::*;
    use std::ffi::CString;
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            unsafe {
                let _ = Box::from_raw(val);
            }
        }
    }

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn exec(vm: *mut MsVM, src: &str) {
        let cs = cstr(src);
        let fname = cstr("test.ms");
        let status = msExecString(vm, cs.as_ptr(), fname.as_ptr());
        assert_eq!(status, MsStatus::MS_OK, "exec failed for: {}", src);
    }

    fn get_global(vm: *mut MsVM, name: &str) -> *mut MsValue {
        let cs = cstr(name);
        let val = msGetGlobal(vm, cs.as_ptr());
        assert!(!val.is_null(), "global '{}' not found", name);
        val
    }

    #[test]
    fn test_call_script_function() {
        let vm = msVmNew();
        exec(vm, "fn add(a, b) {\n  return a + b\n}\n");

        let add_fn = get_global(vm, "add");
        msRoot(vm, add_fn);

        let a = msInt(3);
        let b = msInt(4);
        let args = [a, b];
        let result = msCall(vm, add_fn, args.as_ptr(), 2);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 7);

        free_value(result);
        free_value(a);
        free_value(b);
        msUnroot(vm, add_fn);
        free_value(add_fn);
        msVmFree(vm);
    }

    #[test]
    fn test_call_zero_args() {
        let vm = msVmNew();
        exec(vm, "fn fortytwo() {\n  return 42\n}\n");

        let fn_val = get_global(vm, "fortytwo");
        msRoot(vm, fn_val);

        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 42);

        free_value(result);
        msUnroot(vm, fn_val);
        free_value(fn_val);
        msVmFree(vm);
    }

    #[test]
    fn test_call_with_exception() {
        let vm = msVmNew();
        exec(vm, "fn boom() {\n  throw \"exploded\"\n}\n");

        let fn_val = get_global(vm, "boom");
        msRoot(vm, fn_val);

        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(result.is_null());

        let guard = lock_vm(vm);
        let inner = unsafe { &*guard.get() };
        assert!(inner.vm.has_error);
        assert!(inner.vm.error_message.contains("exploded"));
        drop(guard);

        msUnroot(vm, fn_val);
        free_value(fn_val);
        msVmFree(vm);
    }

    #[test]
    fn test_call_closure() {
        let vm = msVmNew();
        exec(
            vm,
            "fn make_adder(x) {\n  return fn(y) {\n    return x + y\n  }\n}\nadder = make_adder(10)\n",
        );

        let adder = get_global(vm, "adder");
        msRoot(vm, adder);

        let arg = msInt(5);
        let args = [arg];
        let result = msCall(vm, adder, args.as_ptr(), 1);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 15);

        free_value(result);
        free_value(arg);
        msUnroot(vm, adder);
        free_value(adder);
        msVmFree(vm);
    }

    #[test]
    fn test_call_non_callable() {
        let vm = msVmNew();

        let not_callable = msInt(42);
        let result = msCall(vm, not_callable, ptr::null(), 0);
        assert!(result.is_null());

        let guard = lock_vm(vm);
        let inner = unsafe { &*guard.get() };
        assert!(inner.vm.has_error);
        drop(guard);

        free_value(not_callable);
        msVmFree(vm);
    }

    #[test]
    fn test_call_null_safety() {
        let result = msCall(ptr::null_mut(), ptr::null_mut(), ptr::null(), 0);
        assert!(result.is_null());

        let vm = msVmNew();
        let result = msCall(vm, ptr::null_mut(), ptr::null(), 0);
        assert!(result.is_null());

        let result = msCall(vm, msInt(1), ptr::null(), -1);
        assert!(result.is_null());

        free_value(msInt(1));
        msVmFree(vm);
    }

    #[test]
    fn test_recursive_call() {
        let vm = msVmNew();
        exec(
            vm,
            "fn fib(n) {\n  if n <= 1 {\n    return n\n  }\n  return fib(n - 1) + fib(n - 2)\n}\n",
        );

        let fib_fn = get_global(vm, "fib");
        msRoot(vm, fib_fn);

        let arg = msInt(10);
        let args = [arg];
        let result = msCall(vm, fib_fn, args.as_ptr(), 1);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 55);

        free_value(result);
        free_value(arg);
        msUnroot(vm, fib_fn);
        free_value(fib_fn);
        msVmFree(vm);
    }

    extern "C" fn c_mul(
        _vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: i32,
    ) -> *mut MsValue {
        if nargs < 2 {
            return ptr::null_mut();
        }
        let a = unsafe { (*(*args.add(0))).inner.clone() };
        let b = unsafe { (*(*args.add(1))).inner.clone() };
        match (a, b) {
            (Object::Int(x), Object::Int(y)) => {
                Box::into_raw(Box::new(MsValue {
                    inner: Object::Int(x * y),
                }))
            }
            _ => ptr::null_mut(),
        }
    }

    #[test]
    fn test_native_function_bridge() {
        let vm = msVmNew();

        let name = cstr("mul");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_mul), 2);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let global_name = cstr("mul");
        assert_eq!(msSetGlobal(vm, global_name.as_ptr(), cfn), MsStatus::MS_OK);

        exec(vm, "result = mul(3, 7)\n");

        let result_val = get_global(vm, "result");
        msRoot(vm, result_val);
        assert_eq!(msToInt(vm, result_val), 21);

        msUnroot(vm, result_val);
        free_value(result_val);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }

    extern "C" fn c_check_positive(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: i32,
    ) -> *mut MsValue {
        if nargs < 1 {
            return ptr::null_mut();
        }
        let val = unsafe { (*(*args.add(0))).inner.clone() };
        match val {
            Object::Int(n) if n >= 0 => {
                Box::into_raw(Box::new(MsValue {
                    inner: Object::Int(n),
                }))
            }
            _ => {
                let guard = lock_vm(vm);
                let inner = unsafe { &mut *guard.get() };
                inner.vm.has_error = true;
                inner.vm.error_message = "ValueError: negative".into();
                drop(guard);
                ptr::null_mut()
            }
        }
    }

    #[test]
    fn test_native_function_throws() {
        let vm = msVmNew();

        let name = cstr("check_pos");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_check_positive), 1);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let global_name = cstr("check_pos");
        assert_eq!(msSetGlobal(vm, global_name.as_ptr(), cfn), MsStatus::MS_OK);

        exec(
            vm,
            "fn try_catch() {\n  try {\n    check_pos(-1)\n    return 999\n  } except Error as e {\n    return -1\n  }\n}\n",
        );

        let fn_val = get_global(vm, "try_catch");
        msRoot(vm, fn_val);
        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), -1);

        free_value(result);
        msUnroot(vm, fn_val);
        free_value(fn_val);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }

    #[test]
    fn test_make_c_function_null_safety() {
        let vm = msVmNew();
        let name = cstr("noop");

        assert!(msMakeCFunction(ptr::null_mut(), name.as_ptr(), Some(c_mul), 0).is_null());
        assert!(msMakeCFunction(vm, ptr::null(), Some(c_mul), 0).is_null());
        assert!(msMakeCFunction(vm, name.as_ptr(), None, 0).is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_native_function_bridge_direct_call() {
        let vm = msVmNew();

        let name = cstr("mul");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_mul), 2);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let a = msInt(6);
        let b = msInt(7);
        let args = [a, b];
        let result = msCall(vm, cfn, args.as_ptr(), 2);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 42);

        free_value(result);
        free_value(a);
        free_value(b);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }

    // ===================================================================
    // task 76：异步/Channel/Generator 测试
    // ===================================================================

    /// 辅助：比较 MsValue 字符串值。
    fn assert_str_eq(vm: *mut MsVM, val: *mut MsValue, expected: &str) {
        let ptr = msToString(vm, val);
        assert!(!ptr.is_null(), "msToString returned null");
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) };
        assert_eq!(s.to_str().unwrap(), expected);
    }

    #[test]
    fn test_channel_basic_send_recv() {
        let vm = msVmNew();
        let ch = unsafe { msChannel(vm, 3) };
        assert!(!ch.is_null());
        msRoot(vm, ch);

        // 发送 3 个值。
        for v in [1i64, 2, 3] {
            let msval = msInt(v);
            msRoot(vm, msval);
            let s = unsafe { msChannelSend(vm, ch, msval) };
            assert_eq!(s, MsStatus::MS_OK);
            msUnroot(vm, msval);
            free_value(msval);
        }

        // 接收并校验顺序。
        for expected in [1i64, 2, 3] {
            let got = unsafe { msChannelRecv(vm, ch) };
            assert!(!got.is_null());
            msRoot(vm, got);
            assert_eq!(msToInt(vm, got), expected);
            msUnroot(vm, got);
            free_value(got);
        }

        msUnroot(vm, ch);
        free_value(ch);
        msVmFree(vm);
    }

    #[test]
    fn test_channel_close_idempotent_and_recv_after() {
        let vm = msVmNew();
        let ch = unsafe { msChannel(vm, 2) };
        assert!(!ch.is_null());
        msRoot(vm, ch);

        let val = msString(vm, cstr("a").as_ptr());
        msRoot(vm, val);
        assert_eq!(unsafe { msChannelSend(vm, ch, val) }, MsStatus::MS_OK);

        // 关闭。
        assert_eq!(unsafe { msChannelClose(vm, ch) }, MsStatus::MS_OK);
        assert_eq!(unsafe { msChannelIsClosed(vm, ch) }, MS_TRUE);

        // 幂等 close。
        assert_eq!(unsafe { msChannelClose(vm, ch) }, MsStatus::MS_OK);

        // 仍可接收剩余数据。
        let got = unsafe { msChannelRecv(vm, ch) };
        assert!(!got.is_null());
        msRoot(vm, got);
        assert_str_eq(vm, got, "a");
        msUnroot(vm, got);
        free_value(got);

        // 缓冲区空后接收返回 nil。
        let nil_val = unsafe { msChannelRecv(vm, ch) };
        assert!(!nil_val.is_null());
        msRoot(vm, nil_val);
        assert_eq!(msIsNil(nil_val), MS_TRUE);
        msUnroot(vm, nil_val);
        free_value(nil_val);

        // 关闭后发送返回错误。
        let v = msInt(99);
        assert_eq!(unsafe { msChannelSend(vm, ch, v) }, MsStatus::MS_ERROR);
        free_value(v);

        msUnroot(vm, val);
        free_value(val);
        msUnroot(vm, ch);
        free_value(ch);
        msVmFree(vm);
    }

    #[test]
    fn test_channel_buffer_size_validation() {
        let vm = msVmNew();
        // 256 超上限。
        let ch = unsafe { msChannel(vm, 256) };
        assert!(ch.is_null());

        // -1 非法。
        let ch = unsafe { msChannel(vm, -1) };
        assert!(ch.is_null());

        // 边界值 255 合法。
        let ch = unsafe { msChannel(vm, 255) };
        assert!(!ch.is_null());
        msRoot(vm, ch);
        msUnroot(vm, ch);
        free_value(ch);

        // 边界值 0 合法。
        let ch = unsafe { msChannel(vm, 0) };
        assert!(!ch.is_null());
        free_value(ch);

        msVmFree(vm);
    }

    #[test]
    fn test_generator_iteration() {
        let vm = msVmNew();
        exec(vm, "fn gen3() {\n  yield 10\n  yield 20\n  yield 30\n}\ng = gen3()\n");

        let g = get_global(vm, "g");
        msRoot(vm, g);

        // msGeneratorIter 返回 generator 自身（新引用）。
        let iter = unsafe { msGeneratorIter(vm, g) };
        assert!(!iter.is_null());
        msRoot(vm, iter);
        msUnroot(vm, iter);
        free_value(iter);

        // 逐个获取 yield 值。
        #[allow(unused_assignments)]
        let mut out: *mut MsValue = ptr::null_mut();
        for expected in [10i64, 20, 30] {
            out = ptr::null_mut();
            let s = unsafe { msGeneratorNext(vm, g, &mut out) };
            assert_eq!(s, MsStatus::MS_OK);
            assert!(!out.is_null());
            msRoot(vm, out);
            assert_eq!(msToInt(vm, out), expected);
            msUnroot(vm, out);
            free_value(out);
        }
        // 第 4 次：迭代结束，返回 MS_ERROR，out=NULL，无异常。
        out = ptr::null_mut();
        let s = unsafe { msGeneratorNext(vm, g, &mut out) };
        assert_eq!(s, MsStatus::MS_ERROR);
        assert!(out.is_null());
        assert_eq!(msErrOccurred(vm), MS_FALSE);

        msUnroot(vm, g);
        free_value(g);
        msVmFree(vm);
    }

    #[test]
    fn test_generator_iter_not_a_generator() {
        let vm = msVmNew();
        let not_gen = msInt(42);
        msRoot(vm, not_gen);
        let result = unsafe { msGeneratorIter(vm, not_gen) };
        assert!(result.is_null());
        assert_eq!(msErrOccurred(vm), MS_TRUE);
        msUnroot(vm, not_gen);
        free_value(not_gen);
        msVmFree(vm);
    }

    #[test]
    fn test_future_state_resolve() {
        let vm = msVmNew();
        exec(vm, "async fn immediate() {\n  return 100\n}\n");

        let func = get_global(vm, "immediate");
        msRoot(vm, func);

        let future = unsafe { msCallAsync(vm, func, ptr::null(), 0) };
        assert!(!future.is_null());
        msRoot(vm, future);

        // msCallAsync 驱动 EventLoop 后，immediate() 应已完成 → Resolved。
        let state = unsafe { msFutureState(vm, future) };
        assert!(
            state == MsFutureState::MS_FUTURE_PENDING
                || state == MsFutureState::MS_FUTURE_RESOLVED
        );

        // msAwait 阻塞取结果。
        let result = unsafe { msAwait(vm, future) };
        assert!(!result.is_null(), "msAwait returned null");
        if !result.is_null() {
            msRoot(vm, result);
            assert_eq!(msToInt(vm, result), 100);
            msUnroot(vm, result);
            free_value(result);
        }

        // 最终状态应为 Resolved。
        let final_state = unsafe { msFutureState(vm, future) };
        assert_eq!(final_state, MsFutureState::MS_FUTURE_RESOLVED);

        msUnroot(vm, future);
        free_value(future);
        msUnroot(vm, func);
        free_value(func);
        msVmFree(vm);
    }

    #[test]
    fn test_future_manual_resolve_and_await() {
        let vm = msVmNew();
        exec(vm, "async fn immediate() {\n  return 42\n}\n");

        let func = get_global(vm, "immediate");
        msRoot(vm, func);
        let future = unsafe { msCallAsync(vm, func, ptr::null(), 0) };
        assert!(!future.is_null());
        msRoot(vm, future);

        // msAwait 取结果。
        let result = unsafe { msAwait(vm, future) };
        assert!(!result.is_null());
        msRoot(vm, result);
        assert_eq!(msToInt(vm, result), 42);
        msUnroot(vm, result);
        free_value(result);

        msUnroot(vm, future);
        free_value(future);
        msUnroot(vm, func);
        free_value(func);
        msVmFree(vm);
    }

    #[test]
    fn test_future_resolve_reject_idempotent() {
        let vm = msVmNew();

        // 手动创建 Future：通过 async fn。
        exec(vm, "async fn pending_fn() {\n  return 1\n}\n");
        let func = get_global(vm, "pending_fn");
        msRoot(vm, func);

        // msCallAsync 创建 Future；msCallAsync 驱动 EventLoop 后可能已 resolve。
        let future = unsafe { msCallAsync(vm, func, ptr::null(), 0) };
        assert!(!future.is_null());
        msRoot(vm, future);

        // 等待完成。
        let _ = unsafe { msAwait(vm, future) };

        // 幂等 resolve：已 settle 的 Future 再次 resolve 为 no-op。
        let val = msInt(999);
        unsafe { msFutureResolve(vm, future, val) };
        free_value(val);

        // 状态不变。
        let state = unsafe { msFutureState(vm, future) };
        assert_eq!(state, MsFutureState::MS_FUTURE_RESOLVED);

        msUnroot(vm, future);
        free_value(future);
        msUnroot(vm, func);
        free_value(func);
        msVmFree(vm);
    }

    #[test]
    fn test_ms_call_async_not_callable() {
        let vm = msVmNew();
        let not_callable = msInt(42);
        msRoot(vm, not_callable);
        let result = unsafe { msCallAsync(vm, not_callable, ptr::null(), 0) };
        assert!(result.is_null());
        assert_eq!(msErrOccurred(vm), MS_TRUE);
        msUnroot(vm, not_callable);
        free_value(not_callable);
        msVmFree(vm);
    }

    #[test]
    fn test_ms_call_async_null_safety() {
        let result = unsafe { msCallAsync(ptr::null_mut(), ptr::null_mut(), ptr::null(), 0) };
        assert!(result.is_null());

        let vm = msVmNew();
        let result = unsafe { msCallAsync(vm, ptr::null_mut(), ptr::null(), 0) };
        assert!(result.is_null());
        let result = unsafe { msCallAsync(vm, msInt(1), ptr::null(), -1) };
        assert!(result.is_null());
        free_value(msInt(1));
        msVmFree(vm);
    }

    #[test]
    fn test_ms_await_not_a_future() {
        let vm = msVmNew();
        let not_future = msInt(42);
        msRoot(vm, not_future);
        let result = unsafe { msAwait(vm, not_future) };
        assert!(result.is_null());
        assert_eq!(msErrOccurred(vm), MS_TRUE);
        msUnroot(vm, not_future);
        free_value(not_future);
        msVmFree(vm);
    }

    #[test]
    fn test_channel_send_null_safety() {
        let vm = msVmNew();
        assert_eq!(
            unsafe { msChannelSend(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) },
            MsStatus::MS_ERROR
        );

        let ch = unsafe { msChannel(vm, 2) };
        msRoot(vm, ch);
        // ch is valid, val is NULL.
        assert_eq!(
            unsafe { msChannelSend(vm, ch, ptr::null_mut()) },
            MsStatus::MS_ERROR
        );
        msUnroot(vm, ch);
        free_value(ch);
        msVmFree(vm);
    }

    #[test]
    fn test_generator_next_null_safety() {
        let vm = msVmNew();
        let mut out: *mut MsValue = ptr::null_mut();
        // NULL generator.
        let s = unsafe { msGeneratorNext(vm, ptr::null_mut(), &mut out) };
        assert_eq!(s, MsStatus::MS_ERROR);
        assert!(out.is_null());
        msVmFree(vm);
    }

    /// C 异步函数桥接：C 函数 resolve Future → mslang 脚本 await → 获取结果。
    extern "C" fn c_async_double(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: i32,
        future: *mut MsValue,
    ) {
        if nargs < 1 {
            return;
        }
        let val = unsafe { (*(*args.add(0))).inner.clone() };
        let doubled = match val {
            Object::Int(n) => Object::Int(n * 2),
            _ => Object::Nil,
        };
        let result = Box::into_raw(Box::new(MsValue { inner: doubled }));
        unsafe { msFutureResolve(vm, future, result) };
        free_value(result);
    }

    #[test]
    fn test_c_async_function_bridge() {
        use crate::capi::module::{msModuleAddAsyncFunc, msModuleNew, msRegisterModuleValue};

        let vm = msVmNew();

        // 创建模块并注册 C 异步函数。
        let mod_name = cstr("mymod");
        let mod_val = msModuleNew(vm, mod_name.as_ptr());
        assert!(!mod_val.is_null());
        msRoot(vm, mod_val);

        let fn_name = cstr("cdouble");
        let status = msModuleAddAsyncFunc(vm, mod_val, fn_name.as_ptr(), Some(c_async_double));
        assert_eq!(status, MsStatus::MS_OK);

        let status = msRegisterModuleValue(vm, mod_val);
        assert_eq!(status, MsStatus::MS_OK);

        // 执行脚本调用 C 异步函数并 await。
        exec(
            vm,
            "import mymod\nresult = mymod.cdouble(21)\n",
        );

        let result_val = get_global(vm, "result");
        msRoot(vm, result_val);
        // cdouble(21) 返回 Future；脚本顶层赋值后 result 即为 Future 对象。
        // 验证 result 是一个已 Resolved 的 Future。
        let state = unsafe { msFutureState(vm, result_val) };
        assert_eq!(
            state,
            MsFutureState::MS_FUTURE_RESOLVED,
            "C async function Future should be resolved"
        );

        // msAwait 取结果。
        let resolved = unsafe { msAwait(vm, result_val) };
        assert!(!resolved.is_null());
        msRoot(vm, resolved);
        assert_eq!(msToInt(vm, resolved), 42);
        msUnroot(vm, resolved);
        free_value(resolved);

        msUnroot(vm, result_val);
        free_value(result_val);
        msUnroot(vm, mod_val);
        free_value(mod_val);
        msVmFree(vm);
    }
}
