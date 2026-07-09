//! C API — GC Root 注册（task 67）。
//!
//! 参照 [67-capi-value-creation](../../docs/mslang/tasks/67-capi-value-creation.md)。
//!
//! `msRoot` / `msUnroot` 让 C 侧持有跨调用帧的值引用。仅对 Ref 类型（堆对象）
//! 有效；内联值（Nil/Bool/Int/Float）为安全 no-op。

use crate::capi::types::MsValue;
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::object::Object;

/// 将对象注册为 GC 根，返回 `val` 本身。注册后 GC 不会回收此对象。
/// 仅对 Ref 类型（堆对象）有效。内联值为安全 no-op。NULL 安全。
#[no_mangle]
pub extern "C" fn msRoot(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || val.is_null() {
        return val;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    if let Object::Ref(header_ptr) = unsafe { &(*val).inner } {
        inner.vm.c_roots_mut().insert(*header_ptr);
    }
    val
}

/// 注销 GC 根。注销后对象可能被 GC 回收。
/// 仅对 Ref 类型（堆对象）有效。内联值为安全 no-op。NULL 安全。
#[no_mangle]
pub extern "C" fn msUnroot(vm: *mut MsVM, val: *mut MsValue) {
    if vm.is_null() || val.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    if let Object::Ref(header_ptr) = unsafe { &(*val).inner } {
        inner.vm.c_roots_mut().remove(header_ptr);
    }
}
