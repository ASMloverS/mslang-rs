//! task 54：Channel 通信对象。
//!
//! 参照 [08-concurrency](../../docs/mslang/08-concurrency.md) § Channel、
//! [11-bytecode-vm](../../docs/mslang/11-bytecode-vm.md) CHANNEL/SEND/RECEIVE。
//!
//! Channel 为 GC 管理的堆对象（TypeTag::CHANNEL = 14），支持有缓冲与无缓冲模式，
//! 提供协程间通信机制。Coroutine 为普通 struct（非 GC 堆对象），在状态间通过 move
//! 转移所有权——channel 的等待列表直接持有 Coroutine 值。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex as StdMutex};

use crate::vm::object::{MsObjHeader, Object, TypeTag};
use crate::vm::Coroutine;

/// Channel 状态。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Open,
    Closed,
}

/// 等待发送的协程及其待发送值（move 语义，暂停期间由 channel 持有）。
pub struct WaitingSender {
    pub coroutine: Coroutine,
    /// 待发送的值（SEND 时从栈弹出后存于此）。
    pub value: Object,
}

/// 等待接收的协程（move 语义，暂停期间由 channel 持有）。
pub struct WaitingReceiver {
    pub coroutine: Coroutine,
}

/// Channel 堆对象（TypeTag::CHANNEL = 14）。
///
/// - `buffer`：内部队列（RefCell 允许内部可变性）
/// - `capacity`：缓冲区容量（0 为无缓冲）
/// - `state`：通道状态（Open/Closed）
/// - `waiting_senders`：等待发送的协程及其值（FIFO）
/// - `waiting_receivers`：等待接收的协程（FIFO）
/// - `sync_mutex` / `send_cvar` / `recv_cvar`：task 76 线程级同步原语
///   （C API msChannelSend/msChannelRecv 使用）。协程侧 SEND/RECEIVE 不使用
///   这些字段。GC trace 跳过这三字段（Mutex/Condvar 与对象图无关）。
#[repr(C)]
pub struct MsChannel {
    pub header: MsObjHeader,
    pub buffer: RefCell<VecDeque<Object>>,
    pub capacity: usize,
    pub state: RefCell<ChannelState>,
    pub waiting_senders: RefCell<VecDeque<WaitingSender>>,
    pub waiting_receivers: RefCell<VecDeque<WaitingReceiver>>,
    /// task 76：线程级同步互斥锁（C API 阻塞 send/recv 使用）。
    pub sync_mutex: StdMutex<()>,
    /// task 76：缓冲区非满时通知（唤醒阻塞的发送线程）。
    pub send_cvar: Condvar,
    /// task 76：缓冲区非空时通知（唤醒阻塞的接收线程）。
    pub recv_cvar: Condvar,
}

impl MsChannel {
    pub fn new(capacity: usize) -> Self {
        Self {
            header: MsObjHeader {
                gc_meta: 0,
                type_tag: TypeTag::CHANNEL as u8,
                size: std::mem::size_of::<MsChannel>() as u16,
                _padding: 0,
                class_ptr: 0,
            },
            buffer: RefCell::new(VecDeque::new()),
            capacity,
            state: RefCell::new(ChannelState::Open),
            waiting_senders: RefCell::new(VecDeque::new()),
            waiting_receivers: RefCell::new(VecDeque::new()),
            sync_mutex: StdMutex::new(()),
            send_cvar: Condvar::new(),
            recv_cvar: Condvar::new(),
        }
    }

    /// 是否已关闭。
    pub fn is_closed(&self) -> bool {
        *self.state.borrow() == ChannelState::Closed
    }

    /// task 76：线程级阻塞发送。C API 调用者无协程上下文。
    /// 返回 Result：Ok(()) 成功；Err(String) channel 已关闭。
    ///
    /// 注意：capacity == 0（无缓冲）channel 在纯线程级模式下会阻塞直到 close
    /// （rendezvous 需要协程级配合）。C API 推荐使用 capacity > 0 的有缓冲 channel。
    pub fn send_blocking(&self, val: Object) -> Result<(), String> {
        let mut guard = self.sync_mutex.lock().unwrap();
        while self.buffer.borrow().len() >= self.capacity && !self.is_closed() {
            guard = self.send_cvar.wait(guard).unwrap();
        }
        if self.is_closed() {
            return Err("send on closed channel".to_string());
        }
        self.buffer.borrow_mut().push_back(val);
        self.recv_cvar.notify_one();
        Ok(())
    }

    /// task 76：线程级阻塞接收。channel 关闭且缓冲区空时返回 Object::Nil。
    pub fn recv_blocking(&self) -> Object {
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

    /// task 76：close 时唤醒所有线程级等待者（C API msChannelClose 调用）。
    pub fn notify_all_thread_waiters(&self) {
        self.send_cvar.notify_all();
        self.recv_cvar.notify_all();
    }
}

/// 分配 MsChannel 堆对象（TypeTag::CHANNEL），返回 Object::Ref。
/// MVP：Box 分配（与既有 alloc_* 一致，VM 日常分配暂未接入 GC 堆）。
pub fn alloc_channel(capacity: usize) -> Object {
    let ch = Box::new(MsChannel::new(capacity));
    Object::Ref(Box::into_raw(ch) as *mut MsObjHeader)
}

/// 读取 MsChannel（不可变引用）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_channel` 分配的、在 `'a` 期间有效的 `MsChannel`。
pub unsafe fn read_channel<'a>(ptr: *mut MsObjHeader) -> &'a MsChannel {
    debug_assert_eq!(
        (*ptr).type_tag,
        TypeTag::CHANNEL as u8,
        "read_channel on non-CHANNEL"
    );
    &*(ptr as *const MsChannel)
}
