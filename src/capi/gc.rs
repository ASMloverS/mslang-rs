//! C API — GC 交互（task 74 → task 77 升级）。
//!
//! 参照 [74-capi-gc](../../docs/mslang/tasks/74-capi-gc.md) 与
//! [77-capi-concurrent-gc](../../docs/mslang/tasks/77-capi-concurrent-gc.md)。
//!
//! 实现写屏障（msWriteBarrier）、Finalizer 注册（msOnFinalize）、
//! GC 控制（msGcCollect/msGcEnable/msGcIsEnabled）、GC 调优
//! （msGcSetThreshold/msGcSetPromotionAge/msGcSetGcThreads）、
//! GC 调试模式（msGcSetDebug）、GC 统计（msGcStats）。
//!
//! task 77 升级：msWriteBarrier 委托 VM 内部写屏障（并发标记着色 + card marking），
//! msGcSetGcThreads 控制 Worker 线程池大小，msGcStats 扩展并发 GC 指标。
//!
//! msRoot/msUnroot 由 task 67 实现，保留在此文件中。

use crate::capi::types::{MsGcStats, MsGcType, MsStatus, MsValue};
use crate::capi::vm::{lock_vm, MsVM, VmInner};
use crate::vm::object::{MsObjHeader, Object};
use std::ffi::c_void;

// ---------------------------------------------------------------------------
// C finalizer 注册表条目
// ---------------------------------------------------------------------------

/// C 侧 finalizer 注册条目。msOnFinalize 注册，msGcCollect 时执行。
///
/// SAFETY: 仅在持有 VmInner 互斥锁时访问（注册、查找、执行）。
/// finalizer 回调在 mutator 线程、持有锁的状态下执行。
/// 裸指针不跨线程并发访问。
pub(crate) struct CFinalizerEntry {
    /// 对象的 MsObjHeader 地址（可达性判定键）。
    pub obj_header: *mut MsObjHeader,
    /// C finalizer 回调函数指针。
    pub fn_ptr: extern "C" fn(*mut MsVM, *mut MsValue, *mut c_void),
    /// C 侧 userdata 透传指针。
    pub userdata: *mut c_void,
}

unsafe impl Send for CFinalizerEntry {}
unsafe impl Sync for CFinalizerEntry {}

// ---------------------------------------------------------------------------
// GC Root 注册（task 67）
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 写屏障（task 74 → task 77 升级）
// ---------------------------------------------------------------------------

/// 写屏障。task 77 升级为委托 VM 内部 `write_barrier_obj`（Go 1.8+ 风格混合写屏障）。
///
/// **保守近似**：C API 签名 `msWriteBarrier(vm, parent, new_val)` 缺少 `old_val` 参数
/// （13-capi.md:642 已固化），故仅走插入屏障侧（shade new_val）+ 无条件 card marking，
/// 不走删除屏障侧（无法 shade 被覆盖的旧值）。详见 task 77 § 写屏障。
///
/// 行为：
/// - 非并发标记阶段：仅 card marking（Old parent → Young new_val，开销极小），着色逻辑返回。
/// - 并发标记阶段：额外将 White 的 new_val 原子标灰 + 入灰色队列。
///
/// 仅对 Ref 类型（堆对象）有效；内联值（Int/Float/Bool/Nil）为安全 no-op。NULL 安全。
#[no_mangle]
pub extern "C" fn msWriteBarrier(
    vm: *mut MsVM,
    parent: *mut MsValue,
    new_val: *mut MsValue,
) {
    if vm.is_null() || parent.is_null() || new_val.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };

    // 提取堆对象指针（与 msOnFinalize 一致的 inner.inner 模式）。
    // 非堆对象（Int/Float/Bool/Nil 内联值）不需写屏障。
    let parent_obj = match unsafe { &(*parent).inner } {
        Object::Ref(h) => *h,
        _ => return,
    };
    let new_val_obj = match unsafe { &(*new_val).inner } {
        Object::Ref(h) => *h,
        _ => return,
    };

    // 委托 VM 内部写屏障（barrier.rs::write_barrier_obj）：
    //   - old_val = null（C API 签名缺 old_val 参数 → 仅走插入屏障 + card marking）
    //   - 内部含 card marking（无条件）+ 并发标记期原子着色 new_val
    // SAFETY: parent_obj/new_val_obj 由 MsValue 持有，GC 已知可达；VM lock 期间对象不被释放。
    //   write_barrier_obj 仅读 gc_meta 原子位 + push gray_queue（内部 Mutex）+
    //   card_table.mark_dirty（内部 Mutex），不构造 &mut MsObjHeader，无 UB。
    //   VM lock 不阻止并发清扫（Coordinator 经 Arc 共享 GcRuntime），但 gray_queue/card_table
    //   自身有 Mutex 保护，C 侧 push 与 Worker pop 互斥安全。
    unsafe {
        crate::vm::gc::write_barrier_obj(
            &inner.vm.gc_runtime,
            parent_obj,
            std::ptr::null_mut(), // old_val 未知 → 仅走插入屏障 + card marking
            new_val_obj,
        );
    }
}

// ---------------------------------------------------------------------------
// Finalizer 注册（task 74）
// ---------------------------------------------------------------------------

/// 注册 C finalizer 回调。对象被 GC 回收前在 mutator 线程中调用回调。
/// MsFinalizerFn 签名：`void (*)(MsVM* vm, MsValue* obj, void* userdata)`。
///
/// 失败时（vm/obj 为 NULL、obj 非 Ref 类型、fn 为 NULL）返回 MS_ERROR。
#[no_mangle]
pub extern "C" fn msOnFinalize(
    vm: *mut MsVM,
    obj: *mut MsValue,
    fn_ptr: Option<extern "C" fn(*mut MsVM, *mut MsValue, *mut c_void)>,
    userdata: *mut c_void,
) -> MsStatus {
    if vm.is_null() || obj.is_null() {
        return MsStatus::MS_ERROR;
    }
    let fn_ptr = match fn_ptr {
        Some(f) => f,
        None => return MsStatus::MS_ERROR,
    };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    // 验证 obj 是 Ref 类型（堆对象），提取 MsObjHeader 指针
    let header = match unsafe { &(*obj).inner } {
        Object::Ref(h) => *h,
        _ => return MsStatus::MS_ERROR,
    };

    // 设置 has_finalizer 标志位（GC sweep 阶段据此入队复活）
    unsafe {
        (*header).set_has_finalizer(true);
    }

    // 注册 finalizer 回调
    inner.c_finalizers.push(CFinalizerEntry {
        obj_header: header,
        fn_ptr,
        userdata,
    });

    MsStatus::MS_OK
}

// ---------------------------------------------------------------------------
// GC 控制（task 74）
// ---------------------------------------------------------------------------

/// 按 MsGcType 触发 GC（Minor / Major / Full）。手动触发不受 gc_enabled 影响。
#[no_mangle]
pub extern "C" fn msGcCollect(vm: *mut MsVM, gc_type: MsGcType) {
    if vm.is_null() {
        return;
    }
    // 防御非法枚举值（C 侧可传入越界整数）
    if (gc_type as i32) > MsGcType::MS_GC_FULL as i32 || (gc_type as i32) < 0 {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    match gc_type {
        MsGcType::MS_GC_MINOR => {
            inner.vm.gc_minor_only();
        }
        MsGcType::MS_GC_MAJOR => {
            inner.vm.gc_major_only();
        }
        MsGcType::MS_GC_FULL => {
            inner.vm.gc_full();
        }
    }

    // GC 后执行 C 侧 finalizer（检查可达性，调用不可达对象回调）
    run_c_finalizers(vm, inner);
}

/// 启用（enable=1）或禁用（enable=0）自动 GC。
#[no_mangle]
pub extern "C" fn msGcEnable(vm: *mut MsVM, enable: i32) {
    if vm.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.heap.gc_enabled = enable != 0;
}

/// 返回当前自动 GC 状态（1=启用，0=禁用）。
#[no_mangle]
pub extern "C" fn msGcIsEnabled(vm: *mut MsVM) -> i32 {
    if vm.is_null() {
        return 0;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    if inner.vm.heap.gc_enabled {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// GC 调优（task 74）
// ---------------------------------------------------------------------------

/// 设置 GC 触发阈值。
/// - MS_GC_MAJOR / MS_GC_FULL：threshold 为 Old GC 触发比率
/// - MS_GC_MINOR：threshold 为 Young 代大小（MB），clamp [0.5, 64.0]
#[no_mangle]
pub extern "C" fn msGcSetThreshold(vm: *mut MsVM, gc_type: MsGcType, threshold: f64) {
    if vm.is_null() || threshold <= 0.0 {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    match gc_type {
        MsGcType::MS_GC_MAJOR | MsGcType::MS_GC_FULL => {
            let allocated = inner.vm.heap.bytes_allocated;
            inner.vm.heap.next_major_gc =
                ((allocated as f64 * threshold) as usize).max(1);
        }
        MsGcType::MS_GC_MINOR => {
            // threshold 作为 Young 代大小（MB → bytes）
            // 限制范围 [0.5, 64.0] MB
            let clamped = threshold.clamp(0.5, 64.0);
            let young_bytes = (clamped * 1024.0 * 1024.0) as usize;
            inner.vm.heap.next_minor_gc = young_bytes;
        }
    }
}

/// 设置晋升年龄（1-3，超出范围被 clamp）。默认值为 2。
#[no_mangle]
pub extern "C" fn msGcSetPromotionAge(vm: *mut MsVM, age: u32) {
    if vm.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let clamped = age.clamp(1, 3);
    inner.vm.heap.promotion_age = clamped as u8;
}

/// 设置 GC 线程数。task 77 升级：写入 `heap.gc_threads_setting`，下次并发 GC 周期 Init
/// 阶段（`init_concurrent_mark`）读取此字段写入 `gc_runtime.gc_threads`（AtomicU32），
/// 供 Coordinator 启动 Worker 池。当前周期不生效。
///
/// clamp 到 `gc_threads_max()`（CPU 核数上限），与脚本侧 `gc.set_gc_threads` 一致
/// （Task 64 C5）。Task 64 自适应调整可能在下次 GC 收尾单调上调此值。
#[no_mangle]
pub extern "C" fn msGcSetGcThreads(vm: *mut MsVM, threads: u32) {
    if vm.is_null() || threads == 0 {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // clamp 到 gc_threads_max()，与 stdlib gc.set_gc_threads 上限一致（Task 64 C5）。
    let max = crate::vm::gc::tuning::gc_threads_max();
    inner.vm.heap.gc_threads_setting = threads.min(max);
}

// ---------------------------------------------------------------------------
// GC 调试模式（task 74）
// ---------------------------------------------------------------------------

/// 启用/禁用 GC 调试模式。仅 debug_assertions 构建中有实际效果。
#[no_mangle]
pub extern "C" fn msGcSetDebug(vm: *mut MsVM, enable: i32) {
    if vm.is_null() {
        return;
    }
    #[cfg(debug_assertions)]
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner.vm.heap.debug = enable != 0;
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = enable;
    }
}

// ---------------------------------------------------------------------------
// GC 统计（task 74）
// ---------------------------------------------------------------------------

/// 返回 GC 统计快照。NULL vm 返回全零 MsGcStats。
///
/// task 77：扩展并发 GC 指标。从 `MsHeap`（mutator 独占字段）+ `GcRuntime`
/// （Arc deref 原子 load）直接组装——无 `MsHeap::get_stats()` 方法（沿用 Task 74 模式）。
///
/// `gc_threads`：并发模式（`concurrent_enabled=true`）返回 `gc_runtime.gc_threads` 真实值；
/// 降级模式（默认）返回 1（STW 单线程实际值）。
#[no_mangle]
pub extern "C" fn msGcStats(vm: *mut MsVM) -> MsGcStats {
    if vm.is_null() {
        return MsGcStats::default();
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    let h = &inner.vm.heap;
    let g = &inner.vm.gc_runtime;
    use std::sync::atomic::Ordering::Relaxed;
    // gc_threads：并发模式返回 gc_runtime.gc_threads 真实值；降级模式返回 1（STW 单线程）。
    let gc_threads = if g.concurrent_enabled.load(Relaxed) {
        g.gc_threads.load(Relaxed) as u64
    } else {
        1
    };
    MsGcStats {
        minor_gc_count: h.minor_count,
        major_gc_count: h.major_count,
        total_pause_ns: h.total_pause_ns,
        last_pause_ns: h.last_pause_ns,
        young_size: h.young_size() as u64,
        old_size: h.old_size() as u64,
        los_size: h.los_size() as u64,
        bytes_freed: h.bytes_freed,
        concurrent_mark_ns: g.concurrent_mark_ns.load(Relaxed),
        concurrent_sweep_ns: g.concurrent_sweep_ns.load(Relaxed),
        init_stw_ns: g.init_stw_ns.load(Relaxed),
        term_stw_ns: g.term_stw_ns.load(Relaxed),
        gray_queue_peak: g.gray_queue_peak.load(Relaxed),
        gc_threads,
    }
}

// ---------------------------------------------------------------------------
// 内部：C finalizer 执行
// ---------------------------------------------------------------------------

/// GC 后检查 C finalizer 注册表，对不可达对象执行回调。
///
/// 可达性判定：检查 stack + globals + c_roots + call_stack current_exc（浅层）。
/// 不可达对象：构造临时 MsValue 调用回调，从注册表移除。
///
/// 重入保护：执行回调期间禁用自动 GC（防止回调内分配递归触发 GC）。
fn run_c_finalizers(vm_ptr: *mut MsVM, inner: &mut VmInner) {
    // 收集不可达的 finalizer 条目
    let entries = std::mem::take(&mut inner.c_finalizers);
    let vm_ref = &inner.vm;

    let mut keep = Vec::new();
    let mut to_call = Vec::new();
    for entry in entries {
        if vm_ref.is_obj_reachable(entry.obj_header) {
            keep.push(entry);
        } else {
            to_call.push(entry);
        }
    }
    inner.c_finalizers = keep;

    // 重入保护：回调执行期间禁用自动 GC
    let was_enabled = inner.vm.heap.gc_enabled;
    inner.vm.heap.gc_enabled = false;

    for entry in to_call {
        // 构造临时 MsValue 传递给回调
        let temp_val = Box::new(MsValue {
            inner: Object::Ref(entry.obj_header),
        });
        let val_ptr = Box::into_raw(temp_val);
        (entry.fn_ptr)(vm_ptr, val_ptr, entry.userdata);
        // 回收临时 MsValue（不释放底层堆对象，由 GC/VM 管理）
        unsafe {
            drop(Box::from_raw(val_ptr));
        }
    }

    // 恢复自动 GC 状态
    inner.vm.heap.gc_enabled = was_enabled;
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::{msVmFree, msVmNew};
    use std::ffi::CString;

    /// No-op finalizer for tests that only check error returns.
    extern "C" fn noop_finalizer(
        _vm: *mut MsVM,
        _obj: *mut MsValue,
        _userdata: *mut c_void,
    ) {
    }

    #[test]
    fn test_gc_collect_minor() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_collect_major() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_collect_full() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_enable_disable() {
        let vm = msVmNew();

        // 默认启用
        assert_eq!(msGcIsEnabled(vm), 1);

        // 禁用
        msGcEnable(vm, 0);
        assert_eq!(msGcIsEnabled(vm), 0);

        // 重新启用
        msGcEnable(vm, 1);
        assert_eq!(msGcIsEnabled(vm), 1);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_stats() {
        let vm = msVmNew();

        // 初始状态
        let stats = msGcStats(vm);
        assert_eq!(stats.minor_gc_count, 0);
        assert_eq!(stats.major_gc_count, 0);

        // 执行 GC 后检查统计
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        let stats = msGcStats(vm);
        assert!(stats.minor_gc_count > 0 || stats.major_gc_count > 0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_stats_after_multiple_collects() {
        let vm = msVmNew();

        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);

        let stats = msGcStats(vm);
        assert!(stats.minor_gc_count >= 2);
        assert!(stats.major_gc_count >= 1);
        assert!(stats.total_pause_ns > 0);
        assert!(stats.last_pause_ns > 0);

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer() {
        use std::sync::{Arc, Mutex};

        let called: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        // Box::into_raw stores the Arc itself on the heap; the raw pointer
        // is to Arc<Mutex<bool>>, not to the inner Mutex<bool>.
        // (Arc::into_raw would return a pointer to inner T, not to Arc<T>.)
        let called_ptr = Box::into_raw(Box::new(called)) as *mut c_void;

        extern "C" fn my_finalizer(
            _vm: *mut MsVM,
            _obj: *mut MsValue,
            userdata: *mut c_void,
        ) {
            let called = unsafe { &*(userdata as *const Arc<Mutex<bool>>) };
            *called.lock().unwrap() = true;
        }

        let vm = msVmNew();

        // 执行脚本创建一个对象
        let source = CString::new("obj = [1, 2, 3]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        // 获取对象并注册 finalizer
        let name = CString::new("obj").unwrap();
        let obj = crate::capi::vm::msGetGlobal(vm, name.as_ptr());
        assert!(!obj.is_null());

        let status = msOnFinalize(vm, obj, Some(my_finalizer), called_ptr);
        assert_eq!(status, MsStatus::MS_OK);

        // 删除全局引用，触发 GC
        crate::capi::vm::msDelGlobal(vm, name.as_ptr());
        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let called = unsafe { *Box::from_raw(called_ptr as *mut Arc<Mutex<bool>>) };
        assert!(*called.lock().unwrap());

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer_userdata() {
        use std::sync::{Arc, Mutex};

        let captured: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
        let captured_ptr = Box::into_raw(Box::new(captured)) as *mut c_void;

        extern "C" fn my_finalizer(
            _vm: *mut MsVM,
            _obj: *mut MsValue,
            userdata: *mut c_void,
        ) {
            let captured = unsafe { &*(userdata as *const Arc<Mutex<Option<usize>>>) };
            *captured.lock().unwrap() = Some(userdata as usize);
        }

        let vm = msVmNew();

        let source = CString::new("obj = [1, 2, 3]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name = CString::new("obj").unwrap();
        let obj = crate::capi::vm::msGetGlobal(vm, name.as_ptr());
        assert!(!obj.is_null());

        let status = msOnFinalize(vm, obj, Some(my_finalizer), captured_ptr);
        assert_eq!(status, MsStatus::MS_OK);

        crate::capi::vm::msDelGlobal(vm, name.as_ptr());
        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let captured = unsafe { *Box::from_raw(captured_ptr as *mut Arc<Mutex<Option<usize>>>) };
        let got = captured.lock().unwrap();
        assert_eq!(*got, Some(captured_ptr as usize));

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer_kept_if_reachable() {
        use std::sync::{Arc, Mutex};

        let called: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let called_ptr = Box::into_raw(Box::new(called)) as *mut c_void;

        extern "C" fn my_finalizer(
            _vm: *mut MsVM,
            _obj: *mut MsValue,
            userdata: *mut c_void,
        ) {
            let called = unsafe { &*(userdata as *const Arc<Mutex<bool>>) };
            *called.lock().unwrap() = true;
        }

        let vm = msVmNew();

        let source = CString::new("obj = [1, 2, 3]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name = CString::new("obj").unwrap();
        let obj = crate::capi::vm::msGetGlobal(vm, name.as_ptr());
        assert!(!obj.is_null());

        let status = msOnFinalize(vm, obj, Some(my_finalizer), called_ptr);
        assert_eq!(status, MsStatus::MS_OK);

        // 不删除全局引用 → 对象仍可达 → finalizer 不应被调用
        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let called = unsafe { *Box::from_raw(called_ptr as *mut Arc<Mutex<bool>>) };
        assert!(!*called.lock().unwrap());

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer_null_fn() {
        let vm = msVmNew();

        let source = CString::new("obj = [1, 2, 3]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name = CString::new("obj").unwrap();
        let obj = crate::capi::vm::msGetGlobal(vm, name.as_ptr());
        assert!(!obj.is_null());

        // fn 为 NULL → 返回 MS_ERROR
        let status = msOnFinalize(vm, obj, None, std::ptr::null_mut());
        assert_eq!(status, MsStatus::MS_ERROR);

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer_non_ref_type() {
        let vm = msVmNew();

        // Int 是内联值，不是 Ref 类型
        let expr = CString::new("42").unwrap();
        let val = crate::capi::vm::msEval(vm, expr.as_ptr());
        assert!(!val.is_null());

        let status = msOnFinalize(vm, val, Some(noop_finalizer), std::ptr::null_mut());
        assert_eq!(status, MsStatus::MS_ERROR);

        crate::capi::vm::msValueFree(val);
        msVmFree(vm);
    }

    #[test]
    fn test_write_barrier_noop() {
        let vm = msVmNew();

        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm, name_a.as_ptr());

        let source2 = CString::new("b = [2]").unwrap();
        crate::capi::vm::msExecString(vm, source2.as_ptr(), filename.as_ptr());
        let name_b = CString::new("b").unwrap();
        let b = crate::capi::vm::msGetGlobal(vm, name_b.as_ptr());

        // MVP no-op，不应崩溃
        msWriteBarrier(vm, a, b);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_threshold() {
        let vm = msVmNew();

        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, 3.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_MINOR, 8.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_FULL, 2.5);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_threshold_invalid() {
        let vm = msVmNew();

        // 无效值（<= 0）应被忽略，不崩溃
        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, 0.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, -1.0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_promotion_age() {
        let vm = msVmNew();

        msGcSetPromotionAge(vm, 1);
        msGcSetPromotionAge(vm, 2);
        msGcSetPromotionAge(vm, 3);

        // 超出范围应被 clamp
        msGcSetPromotionAge(vm, 0);
        msGcSetPromotionAge(vm, 10);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_gc_threads() {
        let vm = msVmNew();

        msGcSetGcThreads(vm, 1);
        msGcSetGcThreads(vm, 4);
        msGcSetGcThreads(vm, 8);

        // 0 应被忽略
        msGcSetGcThreads(vm, 0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_debug_mode() {
        let vm = msVmNew();

        msGcSetDebug(vm, 1);
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        msGcSetDebug(vm, 0);

        msVmFree(vm);
    }

    #[test]
    fn test_null_vm_safe() {
        assert_eq!(msGcIsEnabled(std::ptr::null_mut()), 0);

        msGcCollect(std::ptr::null_mut(), MsGcType::MS_GC_FULL);
        msGcEnable(std::ptr::null_mut(), 1);
        msGcSetThreshold(std::ptr::null_mut(), MsGcType::MS_GC_MAJOR, 2.0);
        msGcSetPromotionAge(std::ptr::null_mut(), 2);
        msGcSetGcThreads(std::ptr::null_mut(), 4);
        msGcSetDebug(std::ptr::null_mut(), 1);
        msWriteBarrier(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );

        let stats = msGcStats(std::ptr::null_mut());
        assert_eq!(stats.minor_gc_count, 0);
        // task 77：NULL vm → 全零并发指标
        assert_eq!(stats.concurrent_mark_ns, 0);
        assert_eq!(stats.concurrent_sweep_ns, 0);
        assert_eq!(stats.gc_threads, 0);

        // msOnFinalize with NULL vm
        let status = msOnFinalize(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            Some(noop_finalizer),
            std::ptr::null_mut(),
        );
        assert_eq!(status, MsStatus::MS_ERROR);
    }

    #[test]
    fn test_gc_stats_pause_time() {
        let vm = msVmNew();

        // 分配一些对象以产生有意义的数据
        let source = CString::new(
            "
            for i in range(100) {
                x = [1, 2, 3, 4, 5]
            }
        ",
        )
        .unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let stats = msGcStats(vm);
        assert!(stats.last_pause_ns > 0);
        assert!(stats.total_pause_ns >= stats.last_pause_ns);

        msVmFree(vm);
    }

    #[test]
    fn test_multi_vm_isolation() {
        let vm1 = msVmNew();
        let vm2 = msVmNew();

        // vm1 禁用 GC，vm2 保持启用
        msGcEnable(vm1, 0);
        assert_eq!(msGcIsEnabled(vm1), 0);
        assert_eq!(msGcIsEnabled(vm2), 1);

        // vm1 设置 promotion_age=1，vm2 设置 promotion_age=3
        msGcSetPromotionAge(vm1, 1);
        msGcSetPromotionAge(vm2, 3);

        // 各自执行 GC，统计互不影响
        msGcCollect(vm1, MsGcType::MS_GC_MINOR);
        msGcCollect(vm2, MsGcType::MS_GC_MAJOR);

        let s1 = msGcStats(vm1);
        let s2 = msGcStats(vm2);
        assert!(s1.minor_gc_count > 0);
        assert!(s2.major_gc_count > 0);
        // vm1 未执行 Major GC
        assert_eq!(s1.major_gc_count, 0);
        // vm2 未执行 Minor GC
        assert_eq!(s2.minor_gc_count, 0);

        msVmFree(vm1);
        msVmFree(vm2);
    }

    // -----------------------------------------------------------------------
    // task 77 新增测试
    // -----------------------------------------------------------------------

    /// task 77：验证 msWriteBarrier 委托 write_barrier_obj——并发标记期调用，
    /// White 的 new_val 被原子标灰并入灰色队列（保守近似：不对 parent 着色）。
    #[test]
    fn test_write_barrier_shades_gray() {
        let vm = msVmNew();
        let filename = CString::new("test.ms").unwrap();
        // 分两次 msExecString 创建 a/b（合并 "a = [1]; b = [2]" 形式下第二个全局不持久，
        // 与 test_write_barrier_noop 同手法：逐语句执行）。
        let src_a = CString::new("a = [1]").unwrap();
        let src_b = CString::new("b = [2]").unwrap();
        crate::capi::vm::msExecString(vm, src_a.as_ptr(), filename.as_ptr());
        crate::capi::vm::msExecString(vm, src_b.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let name_b = CString::new("b").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm, name_a.as_ptr());
        let b = crate::capi::vm::msGetGlobal(vm, name_b.as_ptr());
        assert!(!a.is_null() && !b.is_null());

        // 进入并发标记阶段 + 清空队列（释放锁后再调 msWriteBarrier，避免重入死锁）。
        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            inner
                .vm
                .gc_runtime
                .set_phase(crate::vm::gc::GcPhase::ConcurrentMark);
            inner.vm.gc_runtime.gray_queue.clear();
        }

        msWriteBarrier(vm, a, b);

        // b 为 White（alloc_list 默认 gc_meta=0）→ 应被标灰入队。
        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            assert!(
                !inner.vm.gc_runtime.gray_queue.is_empty(),
                "并发标记期写屏障应将 new_val 标灰入队"
            );
            // 清理 + 恢复 Idle，避免干扰 msVmFree 的内部状态。
            inner.vm.gc_runtime.gray_queue.clear();
            inner.vm.gc_runtime.set_phase(crate::vm::gc::GcPhase::Idle);
        }

        msVmFree(vm);
    }

    /// task 77：Old parent → Young new_val 应标记 dirty card（card marking 无条件，任意阶段）。
    /// 注：脚本对象经 alloc_* 分配、未接入 GC 堆（gc.rs 注释），不经 GC 晋升；故直接设置
    /// parent 代数为 Old 模拟晋升后状态，验证 C API 委托的 card-marking 路径。
    #[test]
    fn test_write_barrier_card_table() {
        let vm = msVmNew();
        let filename = CString::new("test.ms").unwrap();
        let source = CString::new("a = [1]").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        let source2 = CString::new("b = [2]").unwrap();
        crate::capi::vm::msExecString(vm, source2.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let name_b = CString::new("b").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm, name_a.as_ptr());
        let b = crate::capi::vm::msGetGlobal(vm, name_b.as_ptr());

        // 将 parent (a) 设为 Old 代（模拟晋升）；new_val (b) 保持 Young（alloc 默认）。
        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            let header = match unsafe { &(*a).inner } {
                Object::Ref(h) => *h,
                _ => unreachable!("a 必为 Ref"),
            };
            unsafe {
                (*header).set_generation(crate::vm::gc::Generation::Old);
            }
            inner.vm.gc_runtime.card_table.drain();
        }

        msWriteBarrier(vm, a, b);

        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            assert_eq!(
                inner.vm.gc_runtime.card_table.len(),
                1,
                "Old parent 写入 Young 引用应标记 dirty card"
            );
            inner.vm.gc_runtime.card_table.drain();
        }

        msVmFree(vm);
    }

    /// task 77：非 Ref 类型（Int 内联值）作为 new_val → 写屏障 early-return，不崩溃。
    #[test]
    fn test_write_barrier_non_ref_values() {
        let vm = msVmNew();
        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm, name_a.as_ptr());

        let int_val = crate::capi::value::msInt(42);
        msWriteBarrier(vm, a, int_val);
        crate::capi::vm::msValueFree(int_val);

        msVmFree(vm);
    }

    /// task 77：压力测试——反复触发 GC 并调用写屏障，不崩溃（降级模式下写屏障
    /// 仅 card-marking 检查后 early-return；concurrent_enabled=false 时着色逻辑零开销）。
    /// 注：脚本对象经 alloc_* 分配、未接入 GC 堆，分配数量不产生实际 GC 工作；
    /// 此测试核心是「反复 GC + 写屏障不崩溃」。用单语句脚本（合并 `;` 形式不绑定全局）。
    #[test]
    fn test_concurrent_gc_with_c_extension() {
        let vm = msVmNew();
        let source = CString::new("data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name_data = CString::new("data").unwrap();
        let data = crate::capi::vm::msGetGlobal(vm, name_data.as_ptr());
        assert!(!data.is_null(), "data 全局必须存在");

        for _ in 0..10 {
            msGcCollect(vm, MsGcType::MS_GC_MAJOR);
            msWriteBarrier(vm, data, data);
        }

        msVmFree(vm);
    }

    /// task 77：gc_threads 统计。降级模式（默认）→ 恒为 1；并发模式可能被自适应上调。
    /// 故仅断言 >= 1（精确断言需并发模式 + 关自适应，归集成测试）。
    #[test]
    fn test_gc_thread_count() {
        let vm = msVmNew();

        msGcSetGcThreads(vm, 4);
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        let stats = msGcStats(vm);
        assert!(stats.gc_threads >= 1);

        msGcSetGcThreads(vm, 2);
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);
        let stats = msGcStats(vm);
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    /// task 77：基础 GC 统计 + 并发字段可读。
    /// 注：降级模式下手动 msGcCollect 走 STW major_gc，不触发并发周期 → 并发指标原子量
    /// 保持 0。并发指标的写入/读取路径由 test_gc_stats_reflects_concurrent_runtimes 覆盖。
    #[test]
    fn test_gc_stats_concurrent() {
        let vm = msVmNew();
        let source = CString::new("for i in range(1000) { x = [1, 2, 3, 4, 5] }").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let stats = msGcStats(vm);
        assert!(stats.minor_gc_count > 0 || stats.major_gc_count > 0);
        assert!(stats.total_pause_ns > 0);
        assert!(stats.last_pause_ns > 0);
        // 降级模式 → 并发指标为 0（确定性，见 test_gc_stats_reflects_concurrent_runtimes）。
        assert_eq!(stats.concurrent_mark_ns, 0);
        assert_eq!(stats.concurrent_sweep_ns, 0);
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    /// task 77（验证标准 10）：多线程经 per-VM 外层锁 + msWriteBarrier 内部 VmInner 锁
    /// 双重串行。验证锁不死锁、不 panic。任意时刻仅一线程持锁进入写屏障（非真正并发入队）。
    #[test]
    fn test_write_barrier_vm_lock_serialized() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        // 裸指针非 Send/Sync；包一层 newtype 显式声明 Send 供 Arc<Mutex<_>> 跨线程共享。
        struct SendState {
            vm: *mut MsVM,
            a: *mut MsValue,
            b: *mut MsValue,
        }
        unsafe impl Send for SendState {}

        let vm_raw = msVmNew();
        let filename = CString::new("test.ms").unwrap();
        // 分两次 msExecString 创建 a/b（合并 "a = [1]; b = [2]" 形式下第二个全局不持久）。
        let src_a = CString::new("a = [1]").unwrap();
        let src_b = CString::new("b = [2]").unwrap();
        crate::capi::vm::msExecString(vm_raw, src_a.as_ptr(), filename.as_ptr());
        crate::capi::vm::msExecString(vm_raw, src_b.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let name_b = CString::new("b").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm_raw, name_a.as_ptr());
        let b = crate::capi::vm::msGetGlobal(vm_raw, name_b.as_ptr());
        assert!(!a.is_null() && !b.is_null(), "a/b 全局必须存在");

        let state = Arc::new(Mutex::new(SendState { vm: vm_raw, a, b }));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let st = Arc::clone(&state);
                thread::spawn(move || {
                    let g = st.lock().unwrap();
                    msWriteBarrier(g.vm, g.a, g.b);
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        {
            let g = state.lock().unwrap();
            msVmFree(g.vm);
        }
        // state (Arc) 在此函数末尾 drop，引用计数归零，Arc + Mutex 内存释放。
    }

    /// task 77（验证标准 8 修正）：STW 暂停分解。降级模式下 init/term STW 为 0，
    /// 故 total_pause_ns >= init+term 平凡成立。含 Minor GC 时 Minor 暂停独立累加。
    #[test]
    fn test_concurrent_gc_stats_stw_decomposition() {
        let vm = msVmNew();
        let source = CString::new("for i in range(500) { x = {'key': i} }").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        msGcCollect(vm, MsGcType::MS_GC_MAJOR);

        let stats = msGcStats(vm);
        if stats.major_gc_count > 0 {
            let stw_total = stats.init_stw_ns + stats.term_stw_ns;
            // total_pause_ns 应 >= STW 分解之和（Minor 暂停独立累加时也成立）。
            assert!(stats.total_pause_ns >= stw_total);
        }

        msVmFree(vm);
    }

    /// task 77：传入 0 应被忽略（不崩溃，不改变当前值）。
    #[test]
    fn test_gc_set_gc_threads_min_one() {
        let vm = msVmNew();

        msGcSetGcThreads(vm, 0);
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        let stats = msGcStats(vm);
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    /// task 77（验证标准 12）：parent/new_val/vm 为 NULL 的组合均不崩溃。
    #[test]
    fn test_write_barrier_null_pointers() {
        let vm = msVmNew();
        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());

        let name_a = CString::new("a").unwrap();
        let a = crate::capi::vm::msGetGlobal(vm, name_a.as_ptr());

        msWriteBarrier(vm, std::ptr::null_mut(), a); // parent NULL
        msWriteBarrier(vm, a, std::ptr::null_mut()); // new_val NULL
        msWriteBarrier(std::ptr::null_mut(), a, a); // vm NULL

        msVmFree(vm);
    }

    /// task 77：确定性验证 msGcStats 正确读取 6 个并发指标 + gc_threads 两分支逻辑。
    /// 直接写 gc_runtime 原子量（与 tuning.rs 单测同手法），避开并发周期触发的不确定性。
    #[test]
    fn test_gc_stats_reflects_concurrent_runtimes() {
        use std::sync::atomic::Ordering::Relaxed;

        let vm = msVmNew();

        // 分支 A：concurrent_enabled=true → gc_threads 返回 gc_runtime.gc_threads 真实值。
        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            let g = &inner.vm.gc_runtime;
            g.concurrent_mark_ns.store(123, Relaxed);
            g.concurrent_sweep_ns.store(456, Relaxed);
            g.init_stw_ns.store(789, Relaxed);
            g.term_stw_ns.store(321, Relaxed);
            g.gray_queue_peak.store(42, Relaxed);
            g.concurrent_enabled.store(true, Relaxed);
            g.gc_threads.store(7, Relaxed);
        }
        let stats = msGcStats(vm);
        assert_eq!(stats.concurrent_mark_ns, 123);
        assert_eq!(stats.concurrent_sweep_ns, 456);
        assert_eq!(stats.init_stw_ns, 789);
        assert_eq!(stats.term_stw_ns, 321);
        assert_eq!(stats.gray_queue_peak, 42);
        assert_eq!(stats.gc_threads, 7); // concurrent_enabled=true → 真实值

        // 分支 B：concurrent_enabled=false → gc_threads 返回 1（降级 STW 单线程）。
        {
            let guard = lock_vm(vm);
            let inner = unsafe { &*guard.get() };
            inner.vm.gc_runtime.concurrent_enabled.store(false, Relaxed);
        }
        let stats = msGcStats(vm);
        assert_eq!(stats.gc_threads, 1);

        msVmFree(vm);
    }
}
