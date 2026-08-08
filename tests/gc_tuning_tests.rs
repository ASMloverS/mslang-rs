//! Task 64 集成测试：GC 调优接口与自适应阈值。
//!
//! 参照 [64-gc-tuning](../../docs/mslang/tasks/64-gc-tuning.md) §测试用例 / §验证标准。
//!
//! 覆盖：
//! - 并发统计字段 + gc_threads 真实值（验证标准 7/8）。
//! - major_interval_ms 定时触发（验证标准 13）。
//! - interval=0 禁用定时（验证标准 14）。
//! - 降级模式定时无效（验证标准 15/C2）。
//! - VM drop 在 recv_timeout Coordinator 下不挂起（实现注意事项 3）。
//!
//! 注：stdlib 的 gc_stats / gc_set_adaptive / gc_set_threshold 为模块私有函数，
//! 其字段暴露与校验由 `src/vm/stdlib.rs` 单元测试覆盖；本文件聚焦 VM 级并发周期行为，
//! 经 `vm.gc_runtime()` 公开访问器读取 GcRuntime 原子统计。

use mslang::vm::gc::{self, GcPhase};
use mslang::vm::object::Object;
use mslang::vm::VM;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// 经 gc_alloc_list 分配 GC 托管对象并压栈。
fn alloc_list_on_stack(vm: &mut VM, items: Vec<Object>) {
    // 先 clone Arc，避免 vm.heap_mut() 与 vm.gc_runtime() 同时借用 vm。
    let gc = vm.gc_runtime().clone();
    let obj = gc::gc_alloc_list(vm.heap_mut(), &gc, items);
    vm.stack_mut().push(obj);
}

/// 构造 Old 代非空：分配对象 + minor_gc 晋升（promotion_age=1）。
/// minor_gc 末尾同步 gc.old_size 镜像，供 Coordinator 定时判定。
fn populate_old(vm: &mut VM) {
    vm.heap_mut().promotion_age = 1;
    alloc_list_on_stack(vm, vec![Object::Int(1)]);
    vm.gc_minor_only();
    assert!(
        vm.heap().old_objects_len() > 0,
        "precondition: Old generation must be non-empty"
    );
}

#[test]
fn test_concurrent_stats_fields_and_real_gc_threads() {
    // 验证标准 7/8：并发模式下 gc_threads 反映真实并发度；并发统计字段在周期后被写入。
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    // 设 gc_threads 为已知值（在 gc_threads_max 内），init_concurrent_mark 写入原子镜像。
    let target = gc::tuning::gc_threads_max();
    vm.heap_mut().gc_threads_setting = target;
    // 分配 GC 托管对象（触发写屏障/根集）。
    for _ in 0..500 {
        alloc_list_on_stack(&mut vm, vec![Object::Int(1)]);
    }
    // task 64：默认 minor 阈值已升至 4MB，强制 major 阈值触发并发周期。
    vm.heap_mut().next_major_gc = 0;
    vm.maybe_gc();
    vm.complete_concurrent_cycle_if_pending();
    // 周期完成后 phase 回 Idle，major_count 递增。
    assert_eq!(vm.gc_runtime().phase(), GcPhase::Idle);
    assert!(vm.heap().major_count >= 1);
    // 并发统计字段存在且可读（访问即证明字段已落地）。concurrent_mark_ns 在真实并发周期后 > 0。
    assert!(vm.gc_runtime().concurrent_mark_ns.load(Ordering::Relaxed) > 0);
    // 其余并发统计字段（concurrent_sweep_ns/init_stw_ns/term_stw_ns/swept_bytes/gray_queue_peak）
    // 为 AtomicU64，>= 0 恒真；此处仅读取验证可达（编译期已保证字段存在）。
    let _ = vm.gc_runtime().concurrent_sweep_ns.load(Ordering::Relaxed);
    let _ = vm.gc_runtime().init_stw_ns.load(Ordering::Relaxed);
    let _ = vm.gc_runtime().term_stw_ns.load(Ordering::Relaxed);
    let _ = vm.gc_runtime().swept_bytes.load(Ordering::Relaxed);
    let _ = vm.gc_runtime().gray_queue_peak.load(Ordering::Relaxed);
    // gc_threads 反映 init_concurrent_mark 写入的真实值（== target）。
    assert_eq!(vm.gc_runtime().gc_threads.load(Ordering::Relaxed), target);
}

#[test]
fn test_timer_triggers_major_when_old_nonempty() {
    // 验证标准 13：major_interval_ms 到期 + Old 非空 → Coordinator 置 timer_major_pending，
    // mutator 在 safepoint 发起并发周期，major_count 递增。
    //
    // 注：此为真实定时测试，依赖 Coordinator OS 线程被调度。并行测试负载下 Coordinator 可能
    // 被短暂饿死，故预算宽松（正常通过 < 0.5s；饿死时容忍至 budget）。单线程运行必过。
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    populate_old(&mut vm);

    let major_before = vm.heap().major_count;
    // 短间隔（同步 heap 字段 + GcRuntime 镜像，Coordinator 读镜像）。
    vm.heap_mut().major_gc_interval_ms = 50;
    vm.gc_runtime()
        .major_gc_interval_ms
        .store(50, Ordering::Relaxed);

    // 轮询驱动：等待 Coordinator 定时触发 + mutator safepoint 处理周期。
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        vm.maybe_gc();
        if vm.heap().major_count > major_before {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    vm.complete_concurrent_cycle_if_pending();
    assert!(
        vm.heap().major_count > major_before,
        "timer should have triggered a major cycle"
    );
}

#[test]
fn test_interval_zero_disables_timer() {
    // 验证标准 14：interval=0 → 不定时触发（仅分配驱动）。
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    populate_old(&mut vm);

    let major_before = vm.heap().major_count;
    // interval=0：Coordinator 用 Duration::MAX，永不超时。
    vm.heap_mut().major_gc_interval_ms = 0;
    vm.gc_runtime().major_gc_interval_ms.store(0, Ordering::Relaxed);

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        vm.maybe_gc();
        std::thread::sleep(Duration::from_millis(50));
    }
    vm.complete_concurrent_cycle_if_pending();
    assert_eq!(
        vm.heap().major_count, major_before,
        "interval=0 must not trigger timed major cycle"
    );
}

#[test]
fn test_degraded_mode_timer_silent_noop() {
    // 验证标准 15 / C2：降级模式无 Coordinator，set major_interval_ms 静默无效。
    let mut vm = VM::new();
    // 注意：不调 gc_set_concurrent(true) —— 降级模式（默认）。
    populate_old(&mut vm);

    let major_before = vm.heap().major_count;
    vm.heap_mut().major_gc_interval_ms = 100;
    vm.gc_runtime()
        .major_gc_interval_ms
        .store(100, Ordering::Relaxed);

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        vm.maybe_gc();
        std::thread::sleep(Duration::from_millis(50));
    }
    // 降级模式无 Coordinator 消费定时器 → major_count 不因定时递增。
    assert_eq!(
        vm.heap().major_count, major_before,
        "degraded mode must not honor timer (no coordinator)"
    );
}

#[test]
fn test_vm_drop_does_not_hang_with_recv_timeout_coordinator() {
    // 实现注意事项 3：recv_timeout 替代 recv 后，VM drop 的 Shutdown 仍能唤醒
    // （消息优先于超时返回）。验证 drop 不挂起。
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    populate_old(&mut vm);
    vm.heap_mut().major_gc_interval_ms = 50;
    vm.gc_runtime()
        .major_gc_interval_ms
        .store(50, Ordering::Relaxed);
    // 短暂运行让 Coordinator 进入 recv_timeout 等待。
    std::thread::sleep(Duration::from_millis(120));
    vm.maybe_gc();
    // 显式 drop：若 shutdown 路径损坏会死锁（测试超时）。
    drop(vm);
}

/// 运行 `tests/integration/test_gc_tuning.ms`（spec § mslang 级别冒烟）。
/// CLI `run` 子命令为占位 stub，故经库 API 编译+解释执行。验证调优 API 不 panic +
/// 统计字段存在（aspirational：VM 字面量分配走 alloc_* 非 GC 堆，覆盖率有限）。
#[test]
fn test_mslang_gc_tuning_smoke() {
    let source = include_str!("integration/test_gc_tuning.ms");
    let tokens = mslang::lexer::Lexer::new(source).tokenize_all().expect("lex failed");
    let program = mslang::parser::Parser::new(tokens).parse().expect("parse failed");
    let chunk = mslang::compiler::Compiler::new()
        .compile(&program)
        .expect("compile failed");
    let mut vm = VM::new();
    let result = vm.interpret(chunk);
    assert!(
        result.is_ok(),
        "test_gc_tuning.ms failed: {:?}",
        result.err()
    );
}

/// spec 实现注意事项 7：GC 开销基准记录（12-implementation-plan:584 方向性目标 < 10%）。
/// `#[ignore]` —— 仅记录 total_pause_ns / 总运行时间，不强制阈值（依赖负载，CI 不跑）。
/// 用 `cargo test --test gc_tuning_tests -- --nocapture --ignored gc_overhead` 手动运行。
#[test]
#[ignore]
fn bench_gc_overhead_concurrent() {
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    // 强制高频 GC：低阈值 + 大量 gc_alloc（VM 日常 alloc_* 不经 GC 堆，故用 gc_alloc_list）。
    vm.heap_mut().next_minor_gc = 0;
    vm.heap_mut().next_major_gc = 0;
    let wall_start = Instant::now();
    for _ in 0..100_000 {
        alloc_list_on_stack(&mut vm, vec![Object::Int(1)]);
        vm.maybe_gc();
    }
    vm.complete_concurrent_cycle_if_pending();
    let wall_ns = wall_start.elapsed().as_nanos() as u64;
    let pause_ns = vm.heap().total_pause_ns;
    let ratio = if wall_ns > 0 {
        pause_ns as f64 / wall_ns as f64 * 100.0
    } else {
        0.0
    };
    // 仅记录，不断言。打印供人工评估。
    eprintln!(
        "bench_gc_overhead: pause_ns={}, wall_ns={}, gc_overhead={:.2}%, minor={}, major={}",
        pause_ns,
        wall_ns,
        ratio,
        vm.heap().minor_count,
        vm.heap().major_count
    );
}
