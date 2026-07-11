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
#[repr(C)]
pub struct MsChannel {
    pub header: MsObjHeader,
    pub buffer: RefCell<VecDeque<Object>>,
    pub capacity: usize,
    pub state: RefCell<ChannelState>,
    pub waiting_senders: RefCell<VecDeque<WaitingSender>>,
    pub waiting_receivers: RefCell<VecDeque<WaitingReceiver>>,
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
        }
    }

    /// 是否已关闭。
    pub fn is_closed(&self) -> bool {
        *self.state.borrow() == ChannelState::Closed
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
