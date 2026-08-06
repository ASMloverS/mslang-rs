//! task 62：线程安全灰色队列 + Arc 共享 GC 运行时状态。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § GC 与协程交互（631-674 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §3-4。

use super::cardtable::CardTable;
use super::header::GcPhase;
use super::safepoint::SafepointCoordinator;
use super::MsObjHeader;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// 默认 GC Worker 线程数：available_parallelism()/4，min 1。
fn default_gc_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 4).max(1) as u32)
        .unwrap_or(1)
}

/// 本轮 GC 管辖的对象指针集合（old_objects + los_objects）。
///
/// 裸 `*mut MsObjHeader` 不自动实现 Send/Sync。GC 内部经安全点协议 + 原子着色 + Mutex
/// 自行协调跨线程访问（Coordinator/Worker 仅在并发标记期间读取此集合，mutator 此时
/// 不修改 old/los_objects 向量），故显式声明 Send/Sync。
#[derive(Clone, Default)]
pub struct GcManagedSet(pub HashSet<*mut MsObjHeader>);

// SAFETY: GcManagedSet 在 GC 周期内被 Coordinator/Worker 线程只读共享；mutator 不在并发
// 标记期间修改 old/los_objects。指针本身的别名安全由 GC 的安全点 + 写屏障不变性保证。
unsafe impl Send for GcManagedSet {}
unsafe impl Sync for GcManagedSet {}

impl GcManagedSet {
    pub fn contains(&self, p: *mut MsObjHeader) -> bool {
        self.0.contains(&p)
    }
}

impl std::ops::Deref for GcManagedSet {
    type Target = HashSet<*mut MsObjHeader>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 线程安全灰色队列。写屏障 push 极短（1 次 Mutex lock + Vec push），锁竞争低。
/// 性能敏感时可替换为无锁队列，接口不变。
pub struct GrayQueue {
    inner: Mutex<Vec<*mut MsObjHeader>>,
}

impl GrayQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }
    pub fn push(&self, obj: *mut MsObjHeader) {
        self.inner.lock().unwrap().push(obj);
    }
    pub fn extend(&self, objs: impl IntoIterator<Item = *mut MsObjHeader>) {
        self.inner.lock().unwrap().extend(objs);
    }
    pub fn pop(&self) -> Option<*mut MsObjHeader> {
        self.inner.lock().unwrap().pop()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

impl Default for GrayQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// GC 运行时状态。Arc 共享给 VM 线程（mutator）与 GC Coordinator/Worker 线程。
///
/// 设计要点（[62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §15）：
/// Coordinator 线程**不**直接访问 VM 结构体——根集扫描与 Sweep 由 mutator 在安全点
/// 执行（拥有 `&mut VM`）。Coordinator 仅操作本结构（灰色队列 + 并发 Worker 池），
/// 经安全点协议（request_and_wait / release）与 mutator 协调 Mark Termination 的 STW。
/// 这样规避了跨线程 `&mut VM` 的别名 UB。
pub struct GcRuntime {
    phase: AtomicU8, // GcPhase
    pub gray_queue: GrayQueue,
    pub safepoint: SafepointCoordinator,
    pub card_table: CardTable,
    /// 本轮 GC 管辖的对象集合（old_objects + los_objects）。mutator 在 Init 前构建并
    /// 存入；Coordinator Worker 读取以过滤非 GC 堆对象（alloc_* 分配，布局不兼容）。
    gc_managed: Mutex<Option<Arc<GcManagedSet>>>,
    /// 并发 GC 启用开关。false → major_collect 走 Task 52 STW 路径
    /// （14-gc.md § Phase 7.5 降级路径）。默认 false，确保合并不改变现有行为。
    pub concurrent_enabled: AtomicBool,
    /// Coordinator 完成 Sweep 后置 true，mutator 在安全点恢复后执行 run_finalizers。
    pub finalize_pending: AtomicBool,
    /// Coordinator 完成并发标记后置 true，mutator 在安全点恢复后执行 Mark Termination
    /// 重扫 + Sweep（拥有 `&mut VM`）。
    pub closure_pending: AtomicBool,
    /// 并发标记 Worker 线程数（mutator 在 Init 前从 heap.gc_threads_setting 写入；
    /// 默认 available_parallelism()/4，min 1）。
    pub gc_threads: AtomicU32,

    // 并发 GC 统计（Task 77 C API 读取）。
    pub concurrent_mark_ns: AtomicU64,
    pub init_stw_ns: AtomicU64,
    pub term_stw_ns: AtomicU64,
    pub gray_queue_peak: AtomicU64,
}

impl GcRuntime {
    pub fn new() -> Self {
        Self {
            phase: AtomicU8::new(GcPhase::Idle as u8),
            gray_queue: GrayQueue::new(),
            safepoint: SafepointCoordinator::new(),
            card_table: CardTable::new(),
            gc_managed: Mutex::new(None),
            concurrent_enabled: AtomicBool::new(false),
            finalize_pending: AtomicBool::new(false),
            closure_pending: AtomicBool::new(false),
            gc_threads: AtomicU32::new(default_gc_threads()),
            concurrent_mark_ns: AtomicU64::new(0),
            init_stw_ns: AtomicU64::new(0),
            term_stw_ns: AtomicU64::new(0),
            gray_queue_peak: AtomicU64::new(0),
        }
    }

    pub fn phase(&self) -> GcPhase {
        match self.phase.load(Ordering::Relaxed) {
            x if x == GcPhase::Idle as u8 => GcPhase::Idle,
            x if x == GcPhase::Init as u8 => GcPhase::Init,
            x if x == GcPhase::ConcurrentMark as u8 => GcPhase::ConcurrentMark,
            x if x == GcPhase::MarkTermination as u8 => GcPhase::MarkTermination,
            x if x == GcPhase::ConcurrentSweep as u8 => GcPhase::ConcurrentSweep,
            x if x == GcPhase::Finalize as u8 => GcPhase::Finalize,
            _ => GcPhase::Idle,
        }
    }
    pub fn set_phase(&self, p: GcPhase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }
    pub fn phase_is_concurrent_mark(&self) -> bool {
        self.phase.load(Ordering::Relaxed) == GcPhase::ConcurrentMark as u8
    }

    /// 存入本轮 gc_managed 集合（mutator 在 Init 前调用）。
    pub fn set_gc_managed(&self, managed: Arc<GcManagedSet>) {
        *self.gc_managed.lock().unwrap() = Some(managed);
    }
    /// 取出 gc_managed 的 Arc 克隆（Coordinator 启动 Worker 池时调用）。
    pub fn gc_managed_clone(&self) -> Option<Arc<GcManagedSet>> {
        self.gc_managed.lock().unwrap().clone()
    }
    /// 清空 gc_managed（周期结束后调用，释放内存）。
    pub fn clear_gc_managed(&self) {
        *self.gc_managed.lock().unwrap() = None;
    }
}

impl Default for GcRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: GcRuntime 的可变共享状态（gray_queue / card_table / gc_managed）均由 Mutex 保护；
// phase/flags/gc_meta 经 Atomic 访问；裸指针的别名安全由 GC 的安全点协议 + 写屏障不变性
// 保证（mutator 与 GC 不同时写同一对象图区域）。故可跨 mutator/Coordinator/Worker 线程共享。
unsafe impl Send for GcRuntime {}
unsafe impl Sync for GcRuntime {}

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
    fn test_gray_queue_basic() {
        let q = GrayQueue::new();
        assert!(q.is_empty());
        let a = make_obj();
        let b = make_obj();
        q.push(a);
        q.push(b);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(b)); // LIFO
        assert_eq!(q.pop(), Some(a));
        assert!(q.is_empty());
        unsafe {
            drop(Box::from_raw(a));
            drop(Box::from_raw(b));
        }
    }

    #[test]
    fn test_gray_queue_thread_safety() {
        // 多线程并发 push/pop，不 panic、不丢数据。每线程在自身内分配对象并 push，
        // 裸 *mut 不跨 spawn 边界（raw 指针非 Send），仅经 Mutex 保护队列交互。
        let gc = Arc::new(GcRuntime::new());
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let gc = Arc::clone(&gc);
                std::thread::spawn(move || {
                    let local: Vec<*mut MsObjHeader> = (0..250).map(|_| make_obj()).collect();
                    for o in local {
                        gc.gray_queue.push(o);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(gc.gray_queue.len(), 1000);
        // 回收队列中全部对象。
        while let Some(o) = gc.gray_queue.pop() {
            unsafe {
                drop(Box::from_raw(o));
            }
        }
    }

    #[test]
    fn test_gc_runtime_defaults_and_phase() {
        let gc = GcRuntime::new();
        assert!(!gc.concurrent_enabled.load(Ordering::Relaxed)); // 默认 false
        assert_eq!(gc.phase(), GcPhase::Idle);
        gc.set_phase(GcPhase::ConcurrentMark);
        assert!(gc.phase_is_concurrent_mark());
        assert_eq!(gc.phase(), GcPhase::ConcurrentMark);
        gc.set_phase(GcPhase::Idle);
        assert!(!gc.phase_is_concurrent_mark());
    }

    #[test]
    fn test_gc_managed_set_get_clear() {
        let gc = GcRuntime::new();
        assert!(gc.gc_managed_clone().is_none());
        let s = Arc::new(GcManagedSet(
            std::iter::once(make_obj()).collect::<HashSet<_>>(),
        ));
        gc.set_gc_managed(s);
        assert!(gc.gc_managed_clone().is_some());
        gc.clear_gc_managed();
        assert!(gc.gc_managed_clone().is_none());
    }
}
