//! task 55：JoinHandle 堆对象。
//!
//! 参照 [08-concurrency](../../docs/mslang/08-concurrency.md) § JoinHandle、
//! [11-bytecode-vm](../../docs/mslang/11-bytecode-vm.md) § JOIN_HANDLE。
//!
//! JoinHandle 为 GC 管理的堆对象（TypeTag::JOIN_HANDLE = 16），由 `go` 表达式
//! 返回。提供协程生命周期控制：join（经 await）/ is_done / cancel。
//! 等待 join 的协程由 EventLoop 的 `paused` 列表集中管理（与 Future 一致），
//! JoinHandle 自身不存储 waiters 列表。

use std::cell::RefCell;

use crate::vm::object::{MsObjHeader, Object, TypeTag};

/// JoinHandle 堆对象（TypeTag::JOIN_HANDLE = 16）。
///
/// - `result`：协程正常完成时的返回值（完成后设值）
/// - `error`：协程异常时的异常对象（panic 时设值）
/// - `done`：协程是否已完成（正常或异常）
/// - `cancel_requested`：cancel() 请求标志，协程在安全点检查后终止
///
/// 等待 join 的协程存储在 EventLoop.paused（waiting_on = handle_ptr），
/// 与 Future await 的暂停机制一致，故此处无 waiters 字段。
#[repr(C)]
pub struct MsJoinHandle {
    pub header: MsObjHeader,
    pub result: RefCell<Option<Object>>,
    pub error: RefCell<Option<Object>>,
    pub done: RefCell<bool>,
    pub cancel_requested: RefCell<bool>,
}

impl MsJoinHandle {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            header: MsObjHeader {
                gc_meta: 0,
                type_tag: TypeTag::JOIN_HANDLE as u8,
                size: std::mem::size_of::<MsJoinHandle>() as u16,
                _padding: 0,
                class_ptr: 0,
            },
            result: RefCell::new(None),
            error: RefCell::new(None),
            done: RefCell::new(false),
            cancel_requested: RefCell::new(false),
        }
    }

    /// 协程是否已完成。
    pub fn is_done(&self) -> bool {
        *self.done.borrow()
    }
}

/// 分配 MsJoinHandle 堆对象（TypeTag::JOIN_HANDLE），返回 Object::Ref。
/// MVP：Box 分配（与既有 alloc_* 一致，VM 日常分配暂未接入 GC 堆）。
pub fn alloc_join_handle() -> Object {
    let handle = Box::new(MsJoinHandle::new());
    Object::Ref(Box::into_raw(handle) as *mut MsObjHeader)
}

/// 读取 MsJoinHandle（不可变引用）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_join_handle` 分配的、在 `'a` 期间有效的 `MsJoinHandle`。
pub unsafe fn read_join_handle<'a>(ptr: *mut MsObjHeader) -> &'a MsJoinHandle {
    debug_assert_eq!(
        (*ptr).type_tag,
        TypeTag::JOIN_HANDLE as u8,
        "read_join_handle on non-JOIN_HANDLE"
    );
    &*(ptr as *const MsJoinHandle)
}
