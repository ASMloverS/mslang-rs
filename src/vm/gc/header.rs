//! task 62：GC 阶段状态机 + gc_meta 原子访问。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § Major GC 状态机（360-400 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §1-2。
//!
//! Task 52 的 `MsObjHeader::set_color(&mut self, ...)` 为非原子 `&mut` 访问。并发标记
//! 期间 GC Worker 与 mutator 写屏障同时修改颜色位会触发数据竞争（UB）。本模块提供经
//! `AtomicU8` 指针 cast 操作裸 `gc_meta` 字节的原子 RMW，`AtomicU8::from_mut_ptr` 自
//! Rust 1.70 稳定。颜色/代数为 GC 内部一致性标志，`Ordering::Relaxed` 足够；happens-before
//! 由安全点协议（safepoint.rs 的 atomic + condvar）提供。

use super::{Color, Generation, MsObjHeader};
use std::sync::atomic::{AtomicU8, Ordering};

/// 把指向 `gc_meta` 字节的裸指针转为 `&AtomicU8` 共享引用。
/// AtomicU8 与 u8 同尺寸同对齐，原子 RMW 仅需共享引用（内部 UnsafeCell）。
///
/// # Safety
/// `meta_ptr` 必须指向有效的 u8 字节（存活期间不被释放/移动）。
unsafe fn atomic_meta(meta_ptr: *const u8) -> &'static AtomicU8 {
    // SAFETY: 调用方保证 meta_ptr 指向有效 u8；AtomicU8 与 u8 布局兼容（repr(transparent)）。
    unsafe { &*(meta_ptr as *const AtomicU8) }
}

/// GC 周期阶段。以 AtomicU8 存储，供 mutator 与 GC 线程原子读取。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcPhase {
    Idle = 0,
    Init = 1, // STW：扫描根集，开启写屏障
    ConcurrentMark = 2, // 并发：GC Workers 标记，mutator 继续运行
    MarkTermination = 3, // STW：重扫协程栈/全局表，关闭写屏障
    ConcurrentSweep = 4, // 过渡 STW（Task 63 升级为真正的并发）
    Finalize = 5, // mutator 线程执行 pending finalizers
}

impl GcPhase {
    pub fn is_concurrent_mark(self) -> bool {
        self == GcPhase::ConcurrentMark
    }
    #[allow(dead_code)]
    pub fn is_stw(self) -> bool {
        matches!(
            self,
            GcPhase::Init | GcPhase::MarkTermination | GcPhase::ConcurrentSweep
        )
    }
}

/// 原子读取颜色。
///
/// # Safety
/// `obj` 必须指向有效的 `MsObjHeader`（`gc_meta` 字节可读）。
pub unsafe fn color_atomic(obj: *const MsObjHeader) -> Color {
    // SAFETY: 调用方保证 obj 指向有效 MsObjHeader；gc_meta 位于偏移 0 的 u8。
    let meta = unsafe { atomic_meta(obj as *const u8) };
    match meta.load(Ordering::Relaxed) & 0b11 {
        0 => Color::White,
        1 => Color::Gray,
        2 => Color::Black,
        _ => Color::White, // 位值 3 越界防御（与 Task 52 color() 一致）
    }
}

/// 原子着色（CAS 循环保留 gen/age/finalizer/pinned 位）。
///
/// # Safety
/// `obj` 必须指向有效的 `MsObjHeader`。
pub unsafe fn set_color_atomic(obj: *mut MsObjHeader, c: Color) {
    // SAFETY: 调用方保证 obj 指向有效 MsObjHeader；gc_meta 位于偏移 0。
    let meta = unsafe { atomic_meta(obj as *const u8) };
    let mut cur = meta.load(Ordering::Relaxed);
    loop {
        let new = (cur & !0b11) | (c as u8);
        match meta.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// 原子读取代数。
///
/// # Safety
/// `obj` 必须指向有效的 `MsObjHeader`。
pub unsafe fn generation_atomic(obj: *const MsObjHeader) -> Generation {
    // SAFETY: 调用方保证 obj 指向有效 MsObjHeader。
    let meta = unsafe { atomic_meta(obj as *const u8) };
    match (meta.load(Ordering::Relaxed) >> 2) & 0b11 {
        0 => Generation::Young,
        1 => Generation::Old,
        2 => Generation::Immortal,
        _ => Generation::Young,
    }
}

/// CAS 式着色转换：仅当当前颜色 == from 时原子改为 to。返回是否成功。
/// 多 Worker 竞争同一对象时仅一个成功 → 保证只入队一次。
///
/// # Safety
/// `obj` 必须指向有效的 `MsObjHeader`。
pub unsafe fn try_color_transition(obj: *mut MsObjHeader, from: Color, to: Color) -> bool {
    // SAFETY: 调用方保证 obj 指向有效 MsObjHeader。
    let meta = unsafe { atomic_meta(obj as *const u8) };
    let mut cur = meta.load(Ordering::Relaxed);
    loop {
        if (cur & 0b11) != from as u8 {
            return false;
        }
        let new = (cur & !0b11) | (to as u8);
        match meta.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => cur = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::object::TypeTag;

    /// 构造一个 gc_meta=0 的对象头（堆分配，便于原子 RMW）。
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
    fn test_color_atomic_roundtrip() {
        for c in [Color::White, Color::Gray, Color::Black] {
            let obj = make_obj(Color::White);
            unsafe {
                set_color_atomic(obj, c);
                assert_eq!(color_atomic(obj), c);
            }
            unsafe {
                drop(Box::from_raw(obj));
            }
        }
    }

    #[test]
    fn test_color_atomic_preserves_other_bits() {
        // 原子着色不丢失 gen/age/finalizer 位。
        let obj = make_obj(Color::White);
        unsafe {
            (*obj).set_generation(Generation::Old);
            (*obj).inc_age();
            (*obj).set_has_finalizer(true);
            set_color_atomic(obj, Color::Black);
            assert_eq!(color_atomic(obj), Color::Black);
            assert_eq!((*obj).generation(), Generation::Old); // 保留
            assert_eq!((*obj).age(), 1); // 保留
            assert!((*obj).has_finalizer()); // 保留
            assert_eq!(generation_atomic(obj), Generation::Old);
        }
        unsafe {
            drop(Box::from_raw(obj));
        }
    }

    #[test]
    fn test_try_color_transition_cas() {
        // 成功：White→Gray。
        let obj = make_obj(Color::White);
        unsafe {
            assert!(try_color_transition(obj, Color::White, Color::Gray));
            assert_eq!(color_atomic(obj), Color::Gray);
            // 失败：当前为 Gray，期望 from=White。
            assert!(!try_color_transition(obj, Color::White, Color::Black));
            assert_eq!(color_atomic(obj), Color::Gray); // 未变
        }
        unsafe {
            drop(Box::from_raw(obj));
        }
    }

    #[test]
    fn test_try_color_transition_contended_only_one_succeeds() {
        // 多线程并发 White→Gray，CAS 保证只一个成功（其余返回 false）。
        let obj = make_obj(Color::White);
        let successes = std::sync::atomic::AtomicUsize::new(0);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let obj = obj as usize;
                let s = &successes as *const _ as usize;
                std::thread::spawn(move || {
                    let obj = obj as *mut MsObjHeader;
                    let s = unsafe { &*(s as *const std::sync::atomic::AtomicUsize) };
                    if unsafe { try_color_transition(obj, Color::White, Color::Gray) } {
                        s.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(successes.load(Ordering::Relaxed), 1);
        unsafe {
            assert_eq!(color_atomic(obj), Color::Gray);
            drop(Box::from_raw(obj));
        }
    }

    #[test]
    fn test_gcphase_predicate() {
        assert!(GcPhase::ConcurrentMark.is_concurrent_mark());
        assert!(!GcPhase::Idle.is_concurrent_mark());
        assert!(GcPhase::Init.is_stw());
        assert!(GcPhase::MarkTermination.is_stw());
        assert!(!GcPhase::ConcurrentMark.is_stw());
    }
}
