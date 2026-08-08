//! task 62：混合写屏障（Dijkstra 插入 + Yuasa 删除）+ 并发标记期间分配标黑。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § 混合写屏障（501-533 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §5、§17。
//!
//! 非并发标记阶段（phase != ConcurrentMark）所有写屏障为**零开销**：槽位式直接写回并
//! 返回，对象式直接返回。仅当并发标记进行中才执行着色 + 入灰色队列 + card marking。

use super::header::{
    color_atomic, generation_atomic, set_color_atomic, GcPhase,
};
use super::runtime::GcRuntime;
use super::{Color, Generation, MsObjHeader};

/// 混合写屏障（槽位式）。非并发标记阶段零开销。
///
/// 在写入 `*slot = new_val` 时：
/// 1. 若 old_val 非 null 且 White → 标灰 + 入灰色队列（Yuasa 删除屏障）
/// 2. 若 new_val 非 null 且 White → 标灰 + 入灰色队列（Dijkstra 插入屏障）
///
/// # Safety
/// `slot` 必须指向有效的 `*mut MsObjHeader` 槽（堆对象内部引用字段）。
pub unsafe fn write_barrier(
    gc: &GcRuntime,
    slot: *mut *mut MsObjHeader,
    new_val: *mut MsObjHeader,
) {
    if !gc.phase_is_concurrent_mark() {
        // SAFETY: 调用方保证 slot 指向有效槽位。
        unsafe {
            *slot = new_val;
        }
        return;
    }
    // SAFETY: 调用方保证 slot 指向有效槽位。
    let old_val = unsafe { *slot };
    shade_if_white(gc, old_val);
    shade_if_white(gc, new_val);
    // SAFETY: 调用方保证 slot 指向有效槽位。
    unsafe {
        *slot = new_val;
    }
}

/// 混合写屏障（对象式）。用于 HashMap/Vec 等容器写入**之后**调用。
/// `old_val` 为被覆盖的旧值指针（null 表示无旧值，如 append/add）。
///
/// # Safety
/// `parent` 必须指向有效 MsObjHeader；old_val/new_val 为 null 或有效 MsObjHeader。
pub unsafe fn write_barrier_obj(
    gc: &GcRuntime,
    parent: *mut MsObjHeader,
    old_val: *mut MsObjHeader,
    new_val: *mut MsObjHeader,
) {
    // task 63：跨代 card marking 常驻（任意阶段）。Old parent 写入 Young 引用 → dirty。
    // 修正 Task 62 仅在 ConcurrentMark 期标记 card 的限制：Old→Young 引用可在任意阶段建立，
    // 未记录会导致 Minor GC 漏扫。此判定为 Minor GC 扫描 dirty cards 正确性的前提
    //（14-gc.md § Remembered Set，326-344 行）。
    if !new_val.is_null()
        && unsafe { generation_atomic(parent) } == Generation::Old
        && unsafe { generation_atomic(new_val) } == Generation::Young
    {
        gc.card_table.mark_dirty(parent);
    }

    // 着色仅在并发标记期（三色不变性维护）。
    if !gc.phase_is_concurrent_mark() {
        return;
    }
    shade_if_white(gc, old_val);
    shade_if_white(gc, new_val);
}

/// 若 obj 非 null 且 White → 原子标灰 + 入灰色队列。
fn shade_if_white(gc: &GcRuntime, obj: *mut MsObjHeader) {
    if obj.is_null() {
        return;
    }
    // SAFETY: 调用方保证 obj 为 null 或有效 MsObjHeader（此处已排除 null）。
    if unsafe { color_atomic(obj) } == Color::White {
        // SAFETY: 同上。
        unsafe {
            set_color_atomic(obj, Color::Gray);
        }
        gc.gray_queue.push(obj);
    }
}

/// gc_alloc_* 内部：分配后检查 GC 阶段，并发标记/清扫期间新分配对象直接标黑，
/// 避免被本轮标记漏扫后 Sweep 误回收（§17）。
///
/// # Safety
/// `obj` 必须指向有效的 `MsObjHeader`（刚由 `gc_alloc_*` 经 `Box::into_raw` 分配）。
pub unsafe fn alloc_during_gc(gc: &GcRuntime, obj: *mut MsObjHeader) {
    let phase = gc.phase();
    if phase == GcPhase::ConcurrentMark || phase == GcPhase::ConcurrentSweep {
        // SAFETY: 调用方保证 obj 指向有效 MsObjHeader。
        unsafe {
            set_color_atomic(obj, Color::Black);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::gc::runtime::GcRuntime;
    use crate::vm::object::TypeTag;

    fn make_obj(color: Color) -> *mut MsObjHeader {
        let mut h = Box::new(MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::STRING as u8,
            size: 0,
            _padding: 0,
            class_ptr: 0,
        });
        h.set_color(color);
        Box::into_raw(h)
    }

    #[test]
    fn test_write_barrier_noop_outside_mark() {
        // gc_phase = Idle → 写屏障直接写入，不着色。
        let gc = GcRuntime::new(); // phase = Idle
        let mut slot = std::ptr::null_mut();
        let new_val = make_obj(Color::White);
        // SAFETY: slot 指向栈上有效槽。
        unsafe {
            write_barrier(&gc, &mut slot, new_val);
        }
        assert_eq!(slot, new_val);
        // SAFETY: new_val 有效。
        assert_eq!(unsafe { color_atomic(new_val) }, Color::White); // 未着色
        unsafe {
            drop(Box::from_raw(new_val));
        }
    }

    #[test]
    fn test_write_barrier_shades_old_and_new() {
        // 并发标记期间：old_val(White) + new_val(White) → 均标灰 + 入队。
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let old_val = make_obj(Color::White);
        let new_val = make_obj(Color::White);
        let mut slot = old_val;
        // SAFETY: slot 指向栈上有效槽。
        unsafe {
            write_barrier(&gc, &mut slot, new_val);
        }
        // SAFETY: 两指针有效。
        assert_eq!(unsafe { color_atomic(old_val) }, Color::Gray);
        assert_eq!(unsafe { color_atomic(new_val) }, Color::Gray);
        assert_eq!(gc.gray_queue.len(), 2);
        unsafe {
            drop(Box::from_raw(old_val));
            drop(Box::from_raw(new_val));
        }
    }

    #[test]
    fn test_write_barrier_skips_non_white() {
        // old_val/new_val 已为 Gray/Black → 不重复入队。
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let old_val = make_obj(Color::Black);
        let new_val = make_obj(Color::Gray);
        let mut slot = old_val;
        // SAFETY: slot 指向栈上有效槽。
        unsafe {
            write_barrier(&gc, &mut slot, new_val);
        }
        assert!(gc.gray_queue.is_empty()); // 均非 White，不入队
        unsafe {
            drop(Box::from_raw(old_val));
            drop(Box::from_raw(new_val));
        }
    }

    #[test]
    fn test_write_barrier_obj_card_marking() {
        // Old parent 写入 Young 引用 → card dirty。
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let parent = make_obj(Color::Black);
        unsafe {
            (*parent).set_generation(Generation::Old);
        }
        let young = make_obj(Color::White);
        unsafe {
            (*young).set_generation(Generation::Young);
        }
        // SAFETY: parent/young 有效。
        unsafe {
            write_barrier_obj(&gc, parent, std::ptr::null_mut(), young);
        }
        assert_eq!(gc.card_table.len(), 1); // parent 被标 dirty
        unsafe {
            drop(Box::from_raw(parent));
            drop(Box::from_raw(young));
        }
    }

    #[test]
    fn test_alloc_during_mark_marked_black() {
        // 并发标记期间分配的新对象被标黑，不被误回收。
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let obj = make_obj(Color::White);
        // SAFETY: obj 有效。
        unsafe {
            alloc_during_gc(&gc, obj);
        }
        // SAFETY: obj 有效。
        assert_eq!(unsafe { color_atomic(obj) }, Color::Black);
        unsafe {
            drop(Box::from_raw(obj));
        }
    }

    #[test]
    fn test_alloc_during_idle_not_marked() {
        let gc = GcRuntime::new(); // Idle
        let obj = make_obj(Color::White);
        // SAFETY: obj 有效。
        unsafe {
            alloc_during_gc(&gc, obj);
        }
        // SAFETY: obj 有效。
        assert_eq!(unsafe { color_atomic(obj) }, Color::White); // 未着色
        unsafe {
            drop(Box::from_raw(obj));
        }
    }
}
