//! task 62：Old → Young 跨代引用记录集（写侧）。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § Remembered Set（326-344 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §7。
//!
//! 适配说明：14-gc.md 假设 Old 代为连续内存（按 512 字节分 card）。当前 Old 代为散布
//! Box 分配（`old_objects: Vec<*mut>`），无连续区域，故用 `HashSet<*mut>` 记录含 Young
//! 引用的 Old 对象指针。Minor GC（Task 63）扫描此集合而非全量 Old 对象。语义等价，
//! 粒度更细（per-object）。扫描侧（drain）为 Task 63；本任务仅实现写侧 mark_dirty。

use super::MsObjHeader;
use std::collections::HashSet;
use std::sync::Mutex;

/// Old → Young 跨代引用记录集（写侧）。
pub struct CardTable {
    dirty: Mutex<HashSet<*mut MsObjHeader>>,
}

impl CardTable {
    pub fn new() -> Self {
        Self {
            dirty: Mutex::new(HashSet::new()),
        }
    }

    /// 标记一个 Old 对象含有 Young 引用（写屏障调用）。
    pub fn mark_dirty(&self, old_obj: *mut MsObjHeader) {
        self.dirty.lock().unwrap().insert(old_obj);
    }

    /// Minor GC 扫描后清空（Task 63 调用）。
    pub fn drain(&self) -> Vec<*mut MsObjHeader> {
        self.dirty.lock().unwrap().drain().collect()
    }

    pub fn len(&self) -> usize {
        self.dirty.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirty.lock().unwrap().is_empty()
    }

    /// Sweep 后清理已释放对象的悬垂指针（防止 Minor GC drain 后 UAF）。
    /// 保留仍存在于 old_objects 中的指针，移除其余。
    pub fn retain_valid(&self, old_objects: &[*mut MsObjHeader]) {
        let live: HashSet<*mut MsObjHeader> = old_objects.iter().copied().collect();
        self.dirty.lock().unwrap().retain(|p| live.contains(p));
    }
}

impl Default for CardTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::object::TypeTag;

    fn make_obj() -> *mut MsObjHeader {
        Box::into_raw(Box::new(MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::STRING as u8,
            size: 0,
            _padding: 0,
            class_ptr: 0,
        }))
    }

    #[test]
    fn test_card_table_mark_drain_retain() {
        let ct = CardTable::new();
        let a = make_obj();
        let b = make_obj();
        let c = make_obj();
        ct.mark_dirty(a);
        ct.mark_dirty(b);
        ct.mark_dirty(a); // 去重
        assert_eq!(ct.len(), 2);

        // retain_valid：仅保留仍存活的 c。
        ct.retain_valid(&[c]);
        assert_eq!(ct.len(), 0);

        ct.mark_dirty(c);
        assert_eq!(ct.len(), 1);
        let drained = ct.drain();
        assert_eq!(drained.len(), 1);
        assert!(ct.is_empty());

        unsafe {
            drop(Box::from_raw(a));
            drop(Box::from_raw(b));
            drop(Box::from_raw(c));
        }
    }
}
