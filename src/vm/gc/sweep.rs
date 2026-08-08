//! task 63：并发清扫（Coordinator sweep + mutator reconcile）。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § Concurrent Sweep（443-475 行）与
//! [63-concurrent-sweep-compaction](../../../docs/mslang/tasks/63-concurrent-sweep-compaction.md) §6。
//!
//! Coordinator 线程在 Mark Termination 完成后（phase=ConcurrentSweep）遍历 `gc_managed`
//! 快照释放 White Old 对象。LOS 与 finalizer 对象交由 mutator 在 `reconcile_sweep` 处理
//!（LOS dealloc 需 `los_sizes` 侧表，Coordinator 无 `&mut MsHeap`）。

use super::header::{color_atomic, generation_atomic, GcPhase};
use super::runtime::GcRuntime;
use super::{type_descriptor, Color, Generation};
use crate::vm::object::TypeTag;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Coordinator 并发清扫：遍历 gc_managed 快照，释放 White Old 对象。
///
/// # Safety 前提
/// - phase == ConcurrentSweep（mutator 已完成 Mark Termination，颜色稳定）
/// - 写屏障已关闭（mutator 写入不改颜色）
/// - White 对象不可达（mutator 无法引用 → free 无别名 UB）
pub fn concurrent_sweep(gc: &Arc<GcRuntime>) {
    debug_assert_eq!(gc.phase(), GcPhase::ConcurrentSweep);
    let t0 = std::time::Instant::now();

    let Some(managed) = gc.gc_managed_clone() else {
        return; // 无快照（空周期）
    };

    for &obj in managed.0.iter() {
        // SAFETY: obj 在 gc_managed 中，为有效 MsObjHeader。
        let color = unsafe { color_atomic(obj) };
        if color != Color::White {
            continue; // Black|Gray 存活，reconcile 时重置 White
        }

        // SAFETY: obj 有效。
        let h = unsafe { &*obj };
        // task 63：仅处理 Old 代。LOS（type_tag=LARGE_OBJECT）显式跳过——LOS dealloc 需
        // los_sizes 侧表（MsHeap 独占），交 mutator reconcile 序贯处理。双重过滤（tag + gen）
        // 无副作用：当前 alloc_los 写 gc_meta=0（generation=Young），故 LOS 亦被 gen 过滤跳过。
        if h.type_tag == TypeTag::LARGE_OBJECT as u8 {
            continue;
        }
        if unsafe { generation_atomic(obj) } != Generation::Old {
            continue;
        }

        if h.has_finalizer() {
            // White + finalizer → 复活（不释放），交 mutator 入 finalizer_queue。
            gc.sweep_finalizers.lock().unwrap().push(obj);
            continue;
        }
        if h.is_pinned() {
            // C 侧 pin 的 White 对象保留（14-gc.md 84-85 行）。
            continue;
        }

        // 释放：typed free（Box::from_raw + Drop 载荷）。对象不可达 → 无别名。
        let size = h.size as u64;
        let tag = h.type_tag;
        (type_descriptor(tag).free)(obj);
        gc.sweep_dead_old.lock().unwrap().push(obj);
        gc.swept_bytes.fetch_add(size, Ordering::Relaxed);
    }

    gc.concurrent_sweep_ns
        .store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::gc::header::set_color_atomic;
    use crate::vm::gc::runtime::{GcManagedSet, GcRuntime};
    use crate::vm::gc::{GcList, header_for};
    use crate::vm::object::{MsObjHeader, TypeTag};
    use std::sync::Arc;

    fn make_old_list(color: Color) -> *mut MsObjHeader {
        let obj = Box::new(GcList {
            header: header_for(TypeTag::LIST, std::mem::size_of::<GcList>() as u16),
            items: vec![],
        });
        let ptr = Box::into_raw(obj) as *mut MsObjHeader;
        // SAFETY: ptr 有效。
        unsafe {
            (*ptr).set_generation(Generation::Old);
            set_color_atomic(ptr, color);
        }
        ptr
    }

    #[test]
    fn test_concurrent_sweep_frees_white_old() {
        let gc = Arc::new(GcRuntime::new());
        let live = make_old_list(Color::Black);
        let dead = make_old_list(Color::White);
        gc.set_gc_managed(Arc::new(GcManagedSet(
            [live, dead].into_iter().collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        // dead 入 sweep_dead_old；live 不入。
        assert_eq!(gc.sweep_dead_old.lock().unwrap().len(), 1);
        assert!(gc.sweep_dead_old.lock().unwrap().contains(&dead));
        // SAFETY: live 未被释放，仍可读。
        unsafe {
            assert_eq!(crate::vm::gc::header::color_atomic(live), Color::Black);
            drop(Box::from_raw(live as *mut GcList));
        }
        // dead 已被 free（Box::from_raw），不可再访问。
    }

    #[test]
    fn test_concurrent_sweep_keeps_finalizer_white() {
        let gc = Arc::new(GcRuntime::new());
        let fin = make_old_list(Color::White);
        // SAFETY: fin 有效。
        unsafe {
            (*fin).set_has_finalizer(true);
        }
        gc.set_gc_managed(Arc::new(GcManagedSet(
            std::iter::once(fin).collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        // finalizer 对象入 sweep_finalizers，未被 free（仍可访问）。
        assert_eq!(gc.sweep_finalizers.lock().unwrap().len(), 1);
        assert!(gc.sweep_dead_old.lock().unwrap().is_empty());
        unsafe {
            drop(Box::from_raw(fin as *mut GcList));
        }
    }

    #[test]
    fn test_concurrent_sweep_skips_pinned_white() {
        let gc = Arc::new(GcRuntime::new());
        let pinned = make_old_list(Color::White);
        // SAFETY: pinned 有效。本任务已补 set_pinned 访问器（gc.rs impl MsObjHeader）。
        unsafe {
            (*pinned).set_pinned(true);
        }
        assert!(unsafe { (*pinned).is_pinned() });
        gc.set_gc_managed(Arc::new(GcManagedSet(
            std::iter::once(pinned).collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        assert!(gc.sweep_dead_old.lock().unwrap().is_empty());
        unsafe {
            drop(Box::from_raw(pinned as *mut GcList));
        }
    }
}
