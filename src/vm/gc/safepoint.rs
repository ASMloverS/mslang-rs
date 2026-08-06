//! task 62：安全点 STW 协调（condvar）。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § 字节码安全点（551-603 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §8。
//!
//! 当前 VM 为单 OS 线程 + 协作式协程，故 `parked` 为 0/1 计数（一次 park 即暂停所有
//! 协程）。为使降级模式（concurrent_enabled=false，安全点永不被请求）零开销，`check_and_park`
//! 先以一次 `Relaxed` 原子读 `requested_fast` 快速返回；仅当确有 STW 请求时才加锁。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

struct SafepointState {
    requested: bool, // GC 请求 STW
    parked: bool,    // mutator 已停下
}

pub struct SafepointCoordinator {
    state: Mutex<SafepointState>,
    cv: Condvar,
    /// 快速路径镜像：与 state.requested 同步，供 mutator 每 1 指令做无锁检查。
    requested_fast: AtomicBool,
}

impl SafepointCoordinator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SafepointState {
                requested: false,
                parked: false,
            }),
            cv: Condvar::new(),
            requested_fast: AtomicBool::new(false),
        }
    }

    /// 降级模式下的零开销快速检查：仅一次 Relaxed 原子读。
    pub fn is_requested_fast(&self) -> bool {
        self.requested_fast.load(Ordering::Relaxed)
    }

    /// GC（Coordinator）调用：请求 STW，阻塞等待 mutator 停下。返回后 mutator 确实 parked。
    pub fn request_and_wait(&self) {
        let mut s = self.state.lock().unwrap();
        s.requested = true;
        self.requested_fast.store(true, Ordering::Relaxed);
        while !s.parked {
            s = self.cv.wait(s).unwrap();
        }
        // mutator 已停，GC 可安全触发 mutator 侧的 STW 工作。
    }

    /// GC（Coordinator）调用：完成 STW，恢复 mutator。必须 notify 唤醒阻塞的 mutator。
    pub fn release(&self) {
        let mut s = self.state.lock().unwrap();
        s.requested = false;
        self.requested_fast.store(false, Ordering::Relaxed);
        self.cv.notify_all();
    }

    /// mutator 调用：在安全点检查并必要时停下（阻塞直到 GC 完成 STW）。
    pub fn check_and_park(&self) {
        // 快速路径：无 STW 请求时一次原子读即返回（降级模式恒走此路径）。
        if !self.requested_fast.load(Ordering::Relaxed) {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if !s.requested {
            return;
        }
        s.parked = true;
        self.cv.notify_all(); // 唤醒 GC（request_and_wait 等待 parked）
        while s.requested {
            s = self.cv.wait(s).unwrap(); // 等 GC release
        }
        s.parked = false;
    }

    #[allow(dead_code)]
    pub fn is_requested(&self) -> bool {
        self.state.lock().unwrap().requested
    }
}

impl Default for SafepointCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_safepoint_parks_mutator() {
        // GC 请求 STW → mutator 在安全点停下 → GC 执行 STW → mutator 恢复。
        let sp = Arc::new(SafepointCoordinator::new());
        let sp_bg = Arc::clone(&sp);

        let handle = std::thread::spawn(move || {
            // 模拟 mutator：等待 requested → park → 等 release。
            std::thread::sleep(std::time::Duration::from_millis(10));
            sp_bg.check_and_park();
        });

        sp.request_and_wait(); // 等待 mutator park
                               // 此时 mutator 已停，GC 做 STW 工作...
        sp.release();
        handle.join().unwrap();
    }

    #[test]
    fn test_check_and_park_fast_path_no_request() {
        // 无 STW 请求时 check_and_park 立即返回（降级模式行为）。
        let sp = SafepointCoordinator::new();
        assert!(!sp.is_requested_fast());
        sp.check_and_park(); // 不阻塞
    }
}
