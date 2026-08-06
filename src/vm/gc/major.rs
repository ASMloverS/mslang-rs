//! task 62：并发三色标记状态机 + GC Coordinator / Worker 池 + STW 收尾。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § Major GC（348-497 行）与
//! [62-concurrent-mark](../../../docs/mslang/tasks/62-concurrent-mark.md) §9-15。
//!
//! ## 线程模型与死锁规避（§15）
//!
//! `major_collect_concurrent` 的 Init（根集扫描）与 Mark Termination（重扫 + Sweep）
//! 阶段需访问 VM 结构体（拥有 `&mut VM`）。本实现把这些 STW 工作放在 **mutator 线程**
//! （在安全点处执行），而 **Coordinator 线程仅做并发标记**（操作 `GcRuntime` 的灰色队列、
//! 对象图只读 trace、原子着色），从而规避「mutator 阻塞在 major_collect 内等自己
//! park」的死锁，也避免跨线程 `&mut VM` 的别名 UB。Coordinator 经 SafepointCoordinator
//! 与 mutator 协调 Mark Termination 的 STW 窗口。
//!
//! ## 终止协议（§10）
//!
//! Worker 主循环用 `active` 计数器做静止检测：trace 前 fetch_add、后 fetch_sub；
//! 队列空 + active==0 → 全局静止 → 所有 Worker 退出。Coordinator `join` 后兜底 drain，
//! 防御 race。

use super::header::{
    set_color_atomic, try_color_transition, GcPhase,
};
use super::runtime::{GcManagedSet, GcRuntime};
use super::{run_finalizers, sweep_heap, type_descriptor, Color, MsObjHeader};
use crate::vm::VM;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

// ===========================================================================
// STW 路径（降级 / 手动 gc.collect）
// ===========================================================================

/// 降级模式 / 手动 collect 的同步 STW Major GC：复用 Task 52 `major_gc`（标记-清除）+
/// `run_finalizers`。由 mutator 线程在已停止状态下调用，无需安全点协议。
/// `concurrent_enabled = false` 时 `maybe_gc` 走此路径（默认，行为保持 Task 52）。
pub fn major_collect_stw(vm: &mut VM) {
    super::major_gc(
        &mut vm.heap,
        &vm.stack,
        &vm.globals,
        &vm.defer_stack,
        &vm.call_stack,
    );
    run_finalizers(&mut vm.heap);
}

// ===========================================================================
// 根集扫描 + 灰色队列 drain（Init / Mark Termination 共用）
// ===========================================================================

/// 把根集中的 GC 托管 White 对象 CAS 标灰 + 入灰色队列。仅标记 `gc_managed` 中的对象
/// （过滤 alloc_* 分配的非托管对象，布局不兼容 Gc* trace）。CAS 保证幂等。
fn scan_roots_gray(gc: &GcRuntime, vm: &VM, gc_managed: &GcManagedSet) {
    let mark = |obj: *mut MsObjHeader| {
        if !gc_managed.contains(obj) {
            return;
        }
        // SAFETY: obj 在 gc_managed 中，为有效 MsObjHeader。
        if unsafe { try_color_transition(obj, Color::White, Color::Gray) } {
            gc.gray_queue.push(obj);
        }
    };
    for v in vm.stack.iter() {
        if let crate::vm::object::Object::Ref(r) = v {
            mark(*r);
        }
    }
    for v in vm.globals.values() {
        if let crate::vm::object::Object::Ref(r) = v {
            mark(*r);
        }
    }
    for entry in vm.defer_stack.iter() {
        if let crate::vm::object::Object::Ref(r) = &entry.call_tuple {
            mark(*r);
        }
    }
    for frame in vm.call_stack.iter() {
        if !frame.closure.is_null() {
            mark(frame.closure);
        }
        if let Some(crate::vm::object::Object::Ref(r)) = &frame.current_exc {
            mark(*r);
        }
    }
    // [task 45/65/53] module_cache / c_roots / 暂停协程随对应 task 落地补扫。
}

/// 单线程 drain 灰色队列：trace 每个对象的子引用（CAS White→Gray 入队），标 Black。
/// 用于 Coordinator 兜底与 Mark Termination 收尾。
fn drain_gray(gc: &GcRuntime, gc_managed: &GcManagedSet) {
    while let Some(obj) = gc.gray_queue.pop() {
        if !gc_managed.contains(obj) {
            continue;
        }
        let tag = unsafe { (*obj).type_tag };
        let desc = type_descriptor(tag);
        (desc.trace)(obj, &mut |child| {
            if !gc_managed.contains(child) {
                return;
            }
            // SAFETY: child 为 trace 回调报告的 GC 托管对象指针。
            if unsafe { try_color_transition(child, Color::White, Color::Gray) } {
                gc.gray_queue.push(child);
            }
        });
        // SAFETY: obj 为有效 MsObjHeader（在 gc_managed 中）。
        unsafe {
            set_color_atomic(obj, Color::Black);
        }
        let len = gc.gray_queue.len() as u64;
        gc.gray_queue_peak.fetch_max(len, Ordering::Relaxed);
    }
}

// ===========================================================================
// GC Worker 线程池（并发标记）
// ===========================================================================

pub struct GcWorkerPool {
    workers: Vec<JoinHandle<()>>,
}

impl GcWorkerPool {
    /// 启动 N 个 Worker 线程并发标记。
    pub fn spawn(
        gc_runtime: Arc<GcRuntime>,
        gc_managed: Arc<GcManagedSet>,
        active: Arc<AtomicUsize>,
        n: u32,
    ) -> Self {
        let n = n.max(1);
        let mut workers = Vec::with_capacity(n as usize);
        for i in 0..n {
            let rt = Arc::clone(&gc_runtime);
            let managed = Arc::clone(&gc_managed);
            let act = Arc::clone(&active);
            workers.push(
                thread::Builder::new()
                    .name(format!("mslang-gc-worker-{}", i))
                    .spawn(move || Self::worker_loop(&rt, &managed, &act))
                    .expect("failed to spawn GC worker"),
            );
        }
        Self { workers }
    }

    /// Worker 主循环（参照 14-gc.md 412-431 行）。
    /// 终止协议：active 计数器在 trace 前后 fetch_add/fetch_sub；
    /// 队列空 + active==0 → 全局静止 → 退出；队列空 + active>0 → yield 重试。
    fn worker_loop(gc: &GcRuntime, gc_managed: &GcManagedSet, active: &AtomicUsize) {
        loop {
            let Some(obj) = gc.gray_queue.pop() else {
                // 队列空：检查全局静止。
                if active.load(Ordering::Relaxed) == 0 {
                    return;
                }
                thread::yield_now();
                continue;
            };
            // 跳过非 GC 堆对象（alloc_* 分配，布局不兼容 Gc* trace）。
            if !gc_managed.contains(obj) {
                continue;
            }

            active.fetch_add(1, Ordering::Relaxed);
            let tag = unsafe { (*obj).type_tag };
            let desc = type_descriptor(tag);
            (desc.trace)(obj, &mut |child| {
                if !gc_managed.contains(child) {
                    return;
                }
                // SAFETY: child 为 trace 回调报告的 GC 托管对象指针。
                if unsafe { try_color_transition(child, Color::White, Color::Gray) } {
                    gc.gray_queue.push(child);
                }
            });
            // SAFETY: obj 为有效 MsObjHeader（在 gc_managed 中）。
            unsafe {
                set_color_atomic(obj, Color::Black);
            }
            active.fetch_sub(1, Ordering::Relaxed);

            let len = gc.gray_queue.len() as u64;
            gc.gray_queue_peak.fetch_max(len, Ordering::Relaxed);
        }
    }

    /// 等待所有 Worker 完成（全局静止：队列空 + active==0）。
    pub fn join(self) {
        for h in self.workers {
            let _ = h.join();
        }
    }
}

// ===========================================================================
// 并发周期驱动（mutator 侧 Init / 收尾；Coordinator 侧并发标记）
// ===========================================================================

/// mutator 在 maybe_gc 并发分支调用：构建 gc_managed、CAS 标灰根集、置 ConcurrentMark、
/// 触发 Coordinator。返回后 mutator 继续执行字节码（写屏障生效）。
pub fn init_concurrent_mark(vm: &mut VM) {
    let gc = Arc::clone(&vm.gc_runtime);
    let gc_managed: Arc<GcManagedSet> = Arc::new(GcManagedSet(
        vm.heap
            .old_objects
            .iter()
            .chain(vm.heap.los_objects.iter())
            .copied()
            .collect(),
    ));
    gc.set_gc_managed(Arc::clone(&gc_managed));
    gc.gc_threads
        .store(vm.heap.gc_threads_setting.max(1), Ordering::Relaxed);

    let t0 = std::time::Instant::now();
    // Init：扫描根集（mutator 拥有 &VM），CAS 标灰入队。
    gc.set_phase(GcPhase::Init);
    scan_roots_gray(&gc, vm, &gc_managed);
    gc.init_stw_ns
        .store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);

    // 进入并发标记，写屏障开始生效。
    gc.set_phase(GcPhase::ConcurrentMark);

    // 异步触发 Coordinator（mutator 不阻塞）。
    if let Some(c) = vm.gc_coordinator.as_ref() {
        c.trigger_major();
    } else {
        // Coordinator 未启动（降级或单测）：mutator 线程同步完成标记 + 收尾。
        run_concurrent_mark_only(&gc, &gc_managed);
        close_concurrent_cycle(vm);
    }
}

/// Coordinator 线程的核心：并发标记（Worker 池），完成后经安全点协调 Mark Termination。
fn run_major_cycle(gc: &Arc<GcRuntime>) {
    let Some(gc_managed) = gc.gc_managed_clone() else {
        // 无 gc_managed（未 Init）→ 空周期，直接回 Idle。
        gc.set_phase(GcPhase::Idle);
        return;
    };
    run_concurrent_mark_only(gc, &gc_managed);

    // 并发标记完成 → 请求 STW，让 mutator 在安全点停下后执行收尾（拥有 &mut VM）。
    gc.safepoint.request_and_wait();
    gc.closure_pending.store(true, Ordering::Relaxed);
    gc.safepoint.release();
    // mutator 在 check_and_park 返回后执行 close_concurrent_cycle。
}

/// 启动 Worker 池并发标记 + 兜底 drain。结束后所有可达 gc_managed 对象为 Black。
fn run_concurrent_mark_only(gc: &Arc<GcRuntime>, gc_managed: &Arc<GcManagedSet>) {
    let t0 = std::time::Instant::now();
    let active = Arc::new(AtomicUsize::new(0));
    let n = gc.gc_threads.load(Ordering::Relaxed);
    let pool = GcWorkerPool::spawn(Arc::clone(gc), Arc::clone(gc_managed), Arc::clone(&active), n);
    pool.join();
    // 兜底：防御 Worker 因 race 提前退出且队列非空。
    drain_gray(gc, gc_managed);
    gc.concurrent_mark_ns
        .store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

/// mutator 在安全点恢复后调用（closure_pending）：Mark Termination 重扫根集 + drain +
/// STW Sweep + 设 finalize_pending。拥有 `&mut VM`，故可执行需 &mut 的工作。
pub fn close_concurrent_cycle(vm: &mut VM) {
    let gc = Arc::clone(&vm.gc_runtime);
    let t0 = std::time::Instant::now();

    let gc_managed = gc.gc_managed_clone().unwrap_or_default();

    // Mark Termination：重扫根集（并发标记期间栈/globals 无写屏障，可能被修改）。
    gc.set_phase(GcPhase::MarkTermination);
    scan_roots_gray(&gc, vm, &gc_managed);
    drain_gray(&gc, &gc_managed);

    // Sweep（STW，复用 Task 52 清除逻辑）。
    gc.set_phase(GcPhase::ConcurrentSweep);
    sweep_heap(&mut vm.heap);
    // 清理 Card Table 中已释放对象的悬垂指针（防 Task 63 Minor GC drain 后 UAF）。
    gc.card_table.retain_valid(&vm.heap.old_objects);

    // task 62：bytes_allocated=0（空堆）时回退初始阈值（同 major_gc，防 should_collect_major 恒真）。
    let computed = (vm.heap.bytes_allocated as f64 * super::MAJOR_GC_RATIO) as usize;
    vm.heap.next_major_gc = if computed == 0 {
        super::INITIAL_MAJOR_THRESHOLD
    } else {
        computed
    };
    vm.heap.major_count += 1;

    gc.term_stw_ns
        .store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    gc.set_phase(GcPhase::Finalize);
    gc.finalize_pending.store(true, Ordering::Relaxed);
    gc.clear_gc_managed();
    gc.set_phase(GcPhase::Idle);
}

// ===========================================================================
// GC Coordinator 线程
// ===========================================================================

enum GcTrigger {
    Major,
    Shutdown,
}

/// GC Coordinator：独立 OS 线程，驱动并发标记周期。仅操作 `Arc<GcRuntime>`，不访问 VM。
pub struct GcCoordinator {
    thread: Option<JoinHandle<()>>,
    trigger: mpsc::Sender<GcTrigger>,
}

impl GcCoordinator {
    /// 启动 Coordinator 线程。
    pub fn spawn(gc_runtime: Arc<GcRuntime>) -> Self {
        let (tx, rx) = mpsc::channel();
        let runtime = Arc::clone(&gc_runtime);
        let handle = thread::Builder::new()
            .name("mslang-gc-coordinator".into())
            .spawn(move || {
                let rt = &runtime;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        GcTrigger::Major => {
                            // 仅在并发标记阶段响应（防过时触发）。
                            if rt.phase_is_concurrent_mark() {
                                run_major_cycle(rt);
                            }
                        }
                        GcTrigger::Shutdown => break,
                    }
                }
            })
            .expect("failed to spawn GC coordinator");
        Self {
            thread: Some(handle),
            trigger: tx,
        }
    }

    /// VM 调用：异步触发 Major GC（不阻塞）。
    pub fn trigger_major(&self) {
        let _ = self.trigger.send(GcTrigger::Major);
    }

    /// VM 销毁时调用：先释放可能的安全点请求（唤醒阻塞的 Coordinator），再发送 Shutdown
    /// 并 join。确保 Coordinator 不在 VM 释放后访问 GcRuntime（GcRuntime 由 VM 的 Arc 共享，
    /// join 完成后 Coordinator 不再持引用）。
    pub fn shutdown(&mut self, gc_runtime: &GcRuntime) {
        gc_runtime.safepoint.release(); // 解除可能的 STW 请求，唤醒阻塞的 Coordinator
        let _ = self.trigger.send(GcTrigger::Shutdown);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::gc::runtime::GcRuntime;
    use crate::vm::gc::GcList;
    use crate::vm::object::{MsObjHeader, TypeTag};

    /// 构造一个 White 的 GC 托管 List（header + 内联 Vec<Object>，偏移 16）。
    fn make_list(child: Option<*mut MsObjHeader>) -> *mut MsObjHeader {
        use crate::vm::gc::{GcList, header_for};
        let items = match child {
            Some(c) => vec![crate::vm::object::Object::Ref(c)],
            None => vec![],
        };
        let obj = Box::new(GcList {
            header: header_for(TypeTag::LIST, std::mem::size_of::<GcList>() as u16),
            items,
        });
        Box::into_raw(obj) as *mut MsObjHeader
    }

    #[test]
    fn test_concurrent_mark_marks_reachable_black() {
        // 构造 root → mid → leaf 的对象图，经 Worker 池并发标记后三者均 Black。
        let gc = Arc::new(GcRuntime::new());
        let leaf = make_list(None);
        let mid = make_list(Some(leaf));
        let root = make_list(Some(mid));
        let managed: Arc<GcManagedSet> =
            Arc::new(GcManagedSet([root, mid, leaf].into_iter().collect()));
        gc.set_gc_managed(Arc::clone(&managed));

        // root 预置 Gray 入队（模拟 Init 根集扫描）。
        unsafe {
            set_color_atomic(root, Color::Gray);
        }
        gc.gray_queue.push(root);
        gc.set_phase(GcPhase::ConcurrentMark);

        run_concurrent_mark_only(&gc, &managed);

        // SAFETY: 三指针有效。
        unsafe {
            assert_eq!(
                super::super::header::color_atomic(root),
                Color::Black
            );
            assert_eq!(super::super::header::color_atomic(mid), Color::Black);
            assert_eq!(
                super::super::header::color_atomic(leaf),
                Color::Black
            );
        }
        // 清理。
        for p in [root, mid, leaf] {
            unsafe {
                drop(Box::from_raw(p as *mut GcList));
            }
        }
    }

    #[test]
    fn test_concurrent_mark_unreachable_stays_white() {
        // root 可达；orphan 不可达 → 保持 White。
        let gc = Arc::new(GcRuntime::new());
        let root = make_list(None);
        let orphan = make_list(None);
        let managed: Arc<GcManagedSet> =
            Arc::new(GcManagedSet([root, orphan].into_iter().collect()));
        gc.set_gc_managed(managed);
        unsafe {
            set_color_atomic(root, Color::Gray);
        }
        gc.gray_queue.push(root);
        gc.set_phase(GcPhase::ConcurrentMark);

        let m = gc.gc_managed_clone().unwrap();
        run_concurrent_mark_only(&gc, &m);

        // SAFETY: 两指针有效。
        unsafe {
            assert_eq!(
                super::super::header::color_atomic(root),
                Color::Black
            );
            assert_eq!(
                super::super::header::color_atomic(orphan),
                Color::White
            );
        }
        for p in [root, orphan] {
            unsafe {
                drop(Box::from_raw(p as *mut GcList));
            }
        }
    }
}
