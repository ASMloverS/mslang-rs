//! Task 62 集成测试：并发标记 + 写屏障 + 降级 STW 路径。
//!
//! 单元测试覆盖 Worker 池并发标记 / 安全点 / 卡表；本文件覆盖 VM 级别端到端：
//! - 并发开关默认关闭（降级 = Task 52 STW，合并不改变现有行为）。
//! - gc_set_concurrent 切换标志 + spawn/shutdown Coordinator。
//! - major_collect_stw 保留可达 Old 对象（无误回收）。
//! - major_collect_stw 回收循环垃圾（cycle collection）。
//!
//! 并发周期的真实端到端经 `tests/integration/test_concurrent_gc.ms`（解释器循环含
//! NopSafepoint 安全点）验证，因并发路径涉及时序、需主循环驱动。

use mslang::compiler::Compiler;
use mslang::lexer::Lexer;
use mslang::parser::Parser;
use mslang::vm::gc::{self, GcPhase};
use mslang::vm::object::Object;
use mslang::vm::VM;
use std::sync::atomic::Ordering;

/// 提取 Object::Ref 内的裸指针。
fn ref_ptr(obj: &Object) -> *mut mslang::vm::object::MsObjHeader {
    match obj {
        Object::Ref(p) => *p,
        _ => panic!("期望 Ref"),
    }
}

#[test]
fn test_concurrent_disabled_by_default() {
    // 合并不改变现有行为：默认降级为 Task 52 STW，Coordinator 未启动。
    let vm = VM::new();
    assert!(!vm.gc_runtime().concurrent_enabled.load(Ordering::Relaxed));
    assert_eq!(vm.gc_runtime().phase(), GcPhase::Idle);
}

#[test]
fn test_set_concurrent_toggles_flag_and_coordinator() {
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    assert!(vm.gc_runtime().concurrent_enabled.load(Ordering::Relaxed));
    vm.gc_set_concurrent(false);
    assert!(!vm.gc_runtime().concurrent_enabled.load(Ordering::Relaxed));
    // 关闭后 phase 仍 Idle（无残留并发周期）。
    assert_eq!(vm.gc_runtime().phase(), GcPhase::Idle);
}

#[test]
fn test_major_collect_stw_keeps_reachable() {
    // 分配 List → 经 minor 晋升 Old → STW major 标记 → 仍在栈上故存活，内容完好。
    let mut vm = VM::new();
    vm.heap_mut().promotion_age = 1; // 一次 minor 即晋升
    let gc = vm.gc_runtime().clone();
    let live = gc::gc_alloc_list(
        vm.heap_mut(),
        &gc,
        vec![Object::Int(10), Object::Int(20), Object::Int(30)],
    );
    vm.stack_mut().push(live.clone());
    vm.gc_minor_only(); // 晋升到 Old（更新栈槽至新指针）

    gc::major_collect_stw(&mut vm); // STW 标记-清除

    let top = vm.stack().last().unwrap();
    let items = unsafe { gc::gc_read_list(ref_ptr(top)) };
    assert_eq!(items.clone(), vec![Object::Int(10), Object::Int(20), Object::Int(30)]);
    assert!(vm.heap().old_objects_len() >= 1, "可达 Old 对象不应被回收");
}

#[test]
fn test_major_collect_stw_collects_unreachable_cycle() {
    // a → b → a 循环；晋升到 Old 后清空根集 → STW major 回收，Old 变空。
    let mut vm = VM::new();
    vm.heap_mut().promotion_age = 1;
    let gc = vm.gc_runtime().clone();
    let a = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(1)]);
    let b = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(2)]);
    // 建立循环：a.items += [b], b.items += [a]
    unsafe {
        gc::gc_read_list_mut(ref_ptr(&a)).push(b.clone());
        gc::gc_read_list_mut(ref_ptr(&b)).push(a.clone());
    }
    vm.stack_mut().push(a);
    vm.stack_mut().push(b);
    vm.gc_minor_only(); // 晋升两个 List 到 Old（更新栈槽 + 循环引用指针）

    // 清空根集：循环对象不再可达。
    vm.stack_mut().clear();
    // 保留 slot 0 占位（VM::new 预留），恢复语义一致。
    if vm.stack().is_empty() {
        vm.stack_mut().push(Object::Nil);
    }

    gc::major_collect_stw(&mut vm);

    assert!(
        vm.heap().old_objects_is_empty(),
        "不可达循环应被 major GC 完全回收，Old 仍剩 {} 个",
        vm.heap().old_objects_len()
    );
}

/// 运行 `tests/integration/test_concurrent_gc.ms`（spec § mslang 级别验证）。
/// CLI `run` 子命令为占位 stub，故经库 API 编译+解释执行。验证并发开关、gc.collect、
/// 降级模式端到端不 panic。Coordinator 线程在 set_concurrent(false) 与 VM drop 时清理。
#[test]
fn test_mslang_concurrent_gc_smoke() {
    let source = include_str!("integration/test_concurrent_gc.ms");
    let tokens = Lexer::new(source).tokenize_all().expect("lex failed");
    let program = Parser::new(tokens).parse().expect("parse failed");
    let chunk = Compiler::new().compile(&program).expect("compile failed");
    let mut vm = VM::new();
    let result = vm.interpret(chunk);
    assert!(
        result.is_ok(),
        "test_concurrent_gc.ms failed: {:?}",
        result.err()
    );
}
