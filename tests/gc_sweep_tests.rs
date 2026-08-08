//! Task 63 集成测试：Card Table 扫描侧（Minor GC remembered set）+ 并发清扫端到端。
//!
//! 参照 docs/mslang/tasks/63-concurrent-sweep-compaction.md § 测试用例。
//!
//! 测试经 `gc_alloc_*` 直接构造 GC 堆对象（绕过 VM 的 `alloc_*` 路径），验证 remembered
//! set / 并发清扫逻辑。当前 VM 字面量（如 `[1,2,3]`）走 `alloc_*`（非 GC 堆），故 mslang
//! 端到端覆盖由 Rust 单元/集成测试（直接用 gc_alloc_*）承担。

use mslang::vm::gc::{self, GcPhase};
use mslang::vm::object::{MsObjHeader, Object};
use mslang::vm::VM;

/// 提取 Object::Ref 内的裸指针。
fn ref_ptr(obj: &Object) -> *mut MsObjHeader {
    match obj {
        Object::Ref(p) => *p,
        _ => panic!("期望 Ref"),
    }
}

/// task 63 验证标准 8/9/10：Old 持有唯一 Young 引用 → Minor GC 经 dirty card 扫描存活。
#[test]
fn test_minor_gc_remembered_set_keeps_young() {
    let mut vm = VM::new();
    vm.heap_mut().promotion_age = 1;

    // 1. 分配并晋升 old_list 到 Old 代。
    let gc = vm.gc_runtime().clone();
    let old_list = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![]);
    vm.stack_mut().push(old_list);
    vm.gc_minor_only(); // 晋升 old_list；minor 转发了栈槽至晋升后指针

    // 注：晋升后 old_list 原指针已释放；从栈重读晋升后指针。
    let old_list_promoted = vm.stack().last().unwrap().clone();
    let old_ptr = ref_ptr(&old_list_promoted);

    // 2. 分配 young_obj（Young），append 到 old_list（建立 Old→Young 跨代引用）。
    let young_obj = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(42)]);
    let young_ptr = ref_ptr(&young_obj);
    // SAFETY: old_ptr 为晋升后有效 Old 对象；写经 write_barrier_obj 标 dirty card。
    unsafe {
        gc::gc_read_list_mut(old_ptr).push(young_obj);
        gc::write_barrier_obj(&gc, old_ptr, std::ptr::null_mut(), young_ptr);
    }
    // young_obj 唯一引用在 old_list 内（stack/globals 无）。

    // 3. 再分配垃圾 + 强制 Minor GC；young_obj 应经 dirty card 扫描存活（转发/晋升）。
    let _garbage = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(0)]);
    vm.heap_mut().next_minor_gc = 0; // 强制（gc_minor_only 本身亦无条件触发）
    vm.gc_minor_only();

    // 4. old_list 仍可达且其元素为存活的 young_obj（内容 42）。
    let top = vm.stack().last().unwrap();
    let op = ref_ptr(top);
    // SAFETY: old_list 存活（Old 代，未被 minor GC 触碰）。
    let items = unsafe { gc::gc_read_list(op) };
    assert_eq!(items.len(), 1, "remembered set 应保留跨代 Young 引用");
    match &items[0] {
        Object::Ref(yp) => {
            // SAFETY: young_obj 经 dirty card 扫描存活（晋升或转发）。
            assert_eq!(
                unsafe { gc::gc_read_list(*yp) }.clone(),
                vec![Object::Int(42)]
            );
        }
        _ => panic!("young_obj collected (remembered set failed)"),
    }
}

/// task 63 验证标准 1/2/5/6：并发清扫端到端 —— 可达 Old 存活、不可达 Old 回收、phase 回 Idle。
#[test]
fn test_concurrent_sweep_end_to_end() {
    let mut vm = VM::new();
    vm.gc_set_concurrent(true); // spawn GC Coordinator
    vm.heap_mut().promotion_age = 1;

    // 分配 + 晋升一批对象；live 留在栈，dead 之后移除使其不可达。
    let gc = vm.gc_runtime().clone();
    let live = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(7)]);
    let dead = gc::gc_alloc_list(vm.heap_mut(), &gc, vec![Object::Int(8)]);
    vm.stack_mut().push(live);
    vm.stack_mut().push(dead);
    vm.gc_minor_only(); // 晋升两者到 Old（栈槽更新至晋升后指针）

    // 移除栈顶 dead（晋升后指针），使其不可达。live 保留在栈。
    vm.stack_mut().pop();

    // 触发并发 Major GC + 清扫，等待周期结束（两次 rendezvous）。
    vm.heap_mut().next_major_gc = 0; // 强制 major
    vm.maybe_gc(); // 并发模式异步触发 Coordinator
    vm.complete_concurrent_cycle_if_pending(); // 驱动至 Idle

    // live 仍存活且可达（内容 7）；不可达 dead 被回收。
    let live_ptr = ref_ptr(vm.stack().last().unwrap());
    // SAFETY: live 存活（标记 Black → reconcile 保留）。
    assert_eq!(
        unsafe { gc::gc_read_list(live_ptr) }.clone(),
        vec![Object::Int(7)]
    );
    assert_eq!(
        vm.gc_runtime().phase(),
        GcPhase::Idle,
        "并发周期应回到 Idle"
    );
    // 两者晋升后 old_objects 含 2；并发清扫回收不可达 dead → 仅剩 live（1）。
    assert_eq!(
        vm.heap().old_objects_len(),
        1,
        "不可达 dead 应被并发清扫回收"
    );
}

/// task 63 mslang 级别冒烟：编译+解释执行 test_concurrent_sweep.ms，验证不 panic。
/// CLI `run` 子命令为占位 stub，故经库 API 执行（同 gc_concurrent_tests 的 smoke）。
/// aspirational：当前 VM 字面量走 alloc_*（非 GC 堆），不真正触达并发清扫 GC 路径。
#[test]
fn test_mslang_concurrent_sweep_smoke() {
    use mslang::compiler::Compiler;
    use mslang::lexer::Lexer;
    use mslang::parser::Parser;

    let source = include_str!("integration/test_concurrent_sweep.ms");
    let tokens = Lexer::new(source).tokenize_all().expect("lex failed");
    let program = Parser::new(tokens).parse().expect("parse failed");
    let chunk = Compiler::new().compile(&program).expect("compile failed");
    let mut vm = VM::new();
    let result = vm.interpret(chunk);
    assert!(
        result.is_ok(),
        "test_concurrent_sweep.ms failed: {:?}",
        result.err()
    );
}
