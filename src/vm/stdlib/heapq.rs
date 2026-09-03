//! `heapq` 原生模块（task 84）。
//!
//! 参照 [84-stdlib-collections-itertools-functools-test](../../../docs/mslang/tasks/84-stdlib-collections-itertools-functools-test.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.11
//!（最小堆，Python heapq 语义，6 个原生函数）。
//!
//! 比较沿用 [`Object::compare`]（`CmpOp::Less`，同 sorted 语义）：仅
//! Int/Float/String 可排序，Instance 等其余类型 → TypeError（`<` 运算符经
//! `__lt__` 分派是 opcode 路径，两者语义差异见 10-builtins.md heapq 章）。
//!
//! GC 安全（task 84 §GC 安全）：sift 全程处于 [`read_list`] 长借用下，
//! compare 保持纯函数（无 GC 对象分配、无用户代码分派）——无 GC 窗口，
//! 元素重排于存活 list 内不产生新根集（task 81 shuffle 同款注记）。

use super::{expect_int, expect_list_ref};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_list, alloc_module, read_list, read_module_mut, CmpOp, MsObjHeader, Object,
};
use crate::vm::VM;

/// 构造 `heapq` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_heapq_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 6] = [
        ("heapify", native_heap_heapify),
        ("heap_push", native_heap_heap_push),
        ("heap_pop", native_heap_heap_pop),
        ("push_pop", native_heap_push_pop),
        ("n_largest", native_heap_n_largest),
        ("n_smallest", native_heap_n_smallest),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("heapq");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

// ---------------------------------------------------------------------------
// sift 纯函数（直接操作 read_list 的 &mut Vec<Object>）
// ---------------------------------------------------------------------------

/// 纯比较辅助：`a < b`（[`Object::compare`]，同 sorted 语义）。
/// 错误（跨类型等）原样上抛——中断时 list 处于部分堆序，可接受（Python 语义）。
fn less(a: &Object, b: &Object) -> Result<bool, String> {
    match a.compare(b, CmpOp::Less) {
        Ok(Object::Bool(x)) => Ok(x),
        Ok(_) => Err("TypeError: compare returned non-bool".to_string()),
        Err(e) => Err(e),
    }
}

/// 最小堆 sift-up：尾插元素自 `hole` 上浮至父不大于为止。
fn sift_up(items: &mut [Object], mut hole: usize) -> Result<(), String> {
    while hole > 0 {
        let parent = (hole - 1) / 2;
        if !less(&items[hole], &items[parent])? {
            break;
        }
        items.swap(hole, parent);
        hole = parent;
    }
    Ok(())
}

/// 最小堆 sift-down：自 `hole` 下沉至较小子不小于为止（取两子较小者交换）。
fn sift_down(items: &mut [Object], mut hole: usize) -> Result<(), String> {
    let n = items.len();
    loop {
        let left = 2 * hole + 1;
        if left >= n {
            break;
        }
        let mut child = left;
        let right = left + 1;
        if right < n && less(&items[right], &items[left])? {
            child = right;
        }
        if !less(&items[child], &items[hole])? {
            break;
        }
        items.swap(hole, child);
        hole = child;
    }
    Ok(())
}

/// 原地建堆：sift-down 自底向上（末个非叶节点 → 根），O(n)。
fn heapify_items(items: &mut [Object]) -> Result<(), String> {
    for i in (0..items.len() / 2).rev() {
        sift_down(items, i)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 排序辅助（n_largest / n_smallest；不改动原 list）
// ---------------------------------------------------------------------------

/// n_largest(lst, n) / n_smallest(lst, n) 共用实现。
/// n≤0 → []；n≥len → 全量排序返回（Python 语义）；元素为原对象（无复制）。
/// 比较器复用 builtins::cmp_objects（同 sorted 比较语义）。
fn top_n(args: &[Object], largest: bool) -> Result<Object, String> {
    let who = if largest {
        "n_largest(lst, n)"
    } else {
        "n_smallest(lst, n)"
    };
    let ptr = expect_list_ref(args.get(0), who)?;
    let n = expect_int(args.get(1), who)?;
    if n <= 0 {
        return Ok(alloc_list(Vec::new()));
    }
    // 克隆后排序：Ref 元素仍存活于原 list（调用方根集），alloc_list 分配期间可达。
    // SAFETY: ptr 经 expect_list_ref 校验为 alloc_list 分配的 MsList。
    let mut sorted = unsafe { read_list(ptr) }.clone();
    let mut err = None;
    sorted.sort_by(|a, b| {
        let (x, y) = if largest { (b, a) } else { (a, b) };
        crate::vm::builtins::cmp_objects(x, y, &mut err)
    });
    if let Some(e) = err {
        return Err(e);
    }
    sorted.truncate(n as usize);
    Ok(alloc_list(sorted))
}

// ---------------------------------------------------------------------------
// 原生函数
// ---------------------------------------------------------------------------

/// heapify(lst) -> nil：原地建堆（sift-down 自底向上）。
fn native_heap_heapify(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "heapify(lst)")?;
    // SAFETY: ptr 经 expect_list_ref 校验为 alloc_list 分配的 MsList；
    // heapify_items 无分配、无 VM 重入，借用期间无 GC 窗口。
    heapify_items(unsafe { read_list(ptr) })?;
    Ok(Object::Nil)
}

/// heap_push(lst, v) -> nil：尾插 + sift-up。
fn native_heap_heap_push(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "heap_push(lst, v)")?;
    let v = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: heap_push(lst, v) requires 2 arguments".to_string())?;
    // SAFETY: 同 heapify；Vec::push 为 Rust 内存增长（非 GC 堆分配）。
    let items = unsafe { read_list(ptr) };
    items.push(v);
    let last = items.len() - 1;
    sift_up(items, last)?;
    Ok(Object::Nil)
}

/// heap_pop(lst) -> value：首位弹出（尾元素补首 + sift-down）；空 → IndexError。
fn native_heap_heap_pop(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "heap_pop(lst)")?;
    // SAFETY: 同 heapify；弹出的 root 在返回前无分配（借用期内无 GC 窗口）。
    let items = unsafe { read_list(ptr) };
    if items.is_empty() {
        return Err("IndexError: heap_pop(): heap is empty".to_string());
    }
    let last = items.pop().expect("non-empty checked above");
    if items.is_empty() {
        return Ok(last);
    }
    let root = std::mem::replace(&mut items[0], last);
    sift_down(items, 0)?;
    Ok(root)
}

/// push_pop(lst, v) -> value：push 后立即 pop 最小（合并语义，一次 sift）。
/// Python heappushpop 顺序：lst 空 v 直返；v ≤ 堆顶直返 v（不入堆）；
/// 否则弹出堆顶、v 入首并 sift-down。
fn native_heap_push_pop(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "push_pop(lst, v)")?;
    let v = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: push_pop(lst, v) requires 2 arguments".to_string())?;
    // SAFETY: 同 heapify。
    let items = unsafe { read_list(ptr) };
    if items.is_empty() {
        return Ok(v);
    }
    // v ≤ 堆顶（!(root < v)，全序下等价）→ 直返 v。
    if !less(&items[0], &v)? {
        return Ok(v);
    }
    let root = std::mem::replace(&mut items[0], v);
    sift_down(items, 0)?;
    Ok(root)
}

/// n_largest(lst, n) -> list：前 n 大（降序）；不改原 list。
fn native_heap_n_largest(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    top_n(args, true)
}

/// n_smallest(lst, n) -> list：前 n 小（升序）；不改原 list。
fn native_heap_n_smallest(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    top_n(args, false)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::run_source;
    use super::*;
    use crate::vm::object::{alloc_string, read_list, TypeTag};

    fn int(n: i64) -> Object {
        Object::Int(n)
    }

    fn heap_invariant(items: &[Object]) -> bool {
        for i in 0..items.len() {
            let l = 2 * i + 1;
            let r = 2 * i + 2;
            for c in [l, r] {
                if c < items.len() {
                    let parent_ge = match (&items[i], &items[c]) {
                        (Object::Int(a), Object::Int(b)) => a <= b,
                        _ => false,
                    };
                    if !parent_ge {
                        return false;
                    }
                }
            }
        }
        true
    }

    #[test]
    fn test_sift_up_hand_computed() {
        // 前缀 [2, 4, 6] 为合法堆，尾插 1：hole=3 父=1(值 4)：1<4 交换
        // → [2, 1, 6, 4]，hole=1 父=0(值 2)：1<2 交换 → [1, 2, 6, 4]。
        let mut items = vec![int(2), int(4), int(6), int(1)];
        sift_up(&mut items, 3).unwrap();
        let vals: Vec<i64> = items.iter().map(|o| match o {
            Object::Int(n) => *n,
            _ => panic!(),
        }).collect();
        assert_eq!(vals, vec![1, 2, 6, 4], "sift_up 手算期望");
        assert!(heap_invariant(&items));
    }

    #[test]
    fn test_sift_down_hand_computed() {
        // 堆 [1, 4, 6, 9]，首弹后尾补首：[9, 4, 6]，hole=0：
        // 子 [4, 6] 取 4（左较小）→ 交换 [4, 9, 6]，hole=1 无子 → 停。
        let mut items = vec![int(9), int(4), int(6)];
        sift_down(&mut items, 0).unwrap();
        let vals: Vec<i64> = items.iter().map(|o| match o {
            Object::Int(n) => *n,
            _ => panic!(),
        }).collect();
        assert_eq!(vals, vec![4, 9, 6], "sift_down 手算期望");
        assert!(heap_invariant(&items));
    }

    #[test]
    fn test_heapify_hand_computed() {
        // [5, 3, 8, 1, 4]：len/2=2，自 i=1 起 sift-down。
        // i=1：子 [1(3), 4(4)] 取 1 → 交换 → [5, 1, 8, 3, 4]，hole=3 无子。
        // i=0：子 [1(1), 2(8)] 取 1 → 交换 → [1, 5, 8, 3, 4]，hole=1 子 [3(3), 4(4)]
        //   取 3 → 交换 → [1, 3, 8, 5, 4]，hole=3 无子。
        let mut items = vec![int(5), int(3), int(8), int(1), int(4)];
        heapify_items(&mut items).unwrap();
        let vals: Vec<i64> = items.iter().map(|o| match o {
            Object::Int(n) => *n,
            _ => panic!(),
        }).collect();
        assert_eq!(vals, vec![1, 3, 8, 5, 4], "heapify 手算期望");
        assert!(heap_invariant(&items));
    }

    #[test]
    fn test_module_registration() {
        let ptr = register_heapq_module();
        // SAFETY: ptr 由 register_heapq_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "heapq");
            for name in [
                "heapify", "heap_push", "heap_pop", "push_pop", "n_largest", "n_smallest",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_heap_roundtrip_sorted_output() {
        // 验证标准 1：heapify 后逐次 heap_pop 输出升序。
        let src = r#"
import heapq
lst = [7, 2, 9, 4, 3, 8, 1, 5, 6]
heapq.heapify(lst)
out = []
for i in range(lst.length()) {
    out.push(heapq.heap_pop(lst))
}
assert(out == [1, 2, 3, 4, 5, 6, 7, 8, 9], "逐次弹出升序")
assert(lst == [], "弹出后清空")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "heapify/heap_pop 升序失败: {:?}", r.err());
    }

    #[test]
    fn test_push_pop_sequence_matches_sorted() {
        // 验证标准 1：heap_push/heap_pop 随机序列与 sorted 结果一致。
        let src = r#"
import heapq
import random
random.seed(84)
lst = []
expect = []
for i in range(200) {
    v = random.randint(0, 999)
    heapq.heap_push(lst, v)
    expect.push(v)
}
out = []
for i in range(200) {
    out.push(heapq.heap_pop(lst))
}
assert(out == sorted(expect), "随机 push/pop 与 sorted 一致")
assert(heapq.push_pop([3], 9) == 3, "push_pop 返回较小堆顶")
assert(heapq.push_pop([9], 3) == 3, "push_pop v<=堆顶直返 v")
assert(heapq.push_pop([], 7) == 7, "push_pop 空堆直返 v")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "push/pop 序列失败: {:?}", r.err());
    }

    #[test]
    fn test_error_paths() {
        // 验证标准 2/3：空弹出 IndexError；n 边界；混合类型 TypeError；非 list TypeError。
        let mut v = VM::new();
        let empty = alloc_list(Vec::new());
        let err = native_heap_heap_pop(&mut v, &[empty]).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
        let err = native_heap_heapify(&mut v, &[Object::Int(42)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // 混合类型（int 与 string 比较）→ TypeError 上抛
        let mixed = alloc_list(vec![Object::Int(1), alloc_string("x")]);
        let err = native_heap_heapify(&mut v, &[mixed]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // n 边界
        let lst = alloc_list(vec![int(3), int(1), int(2)]);
        let got = native_heap_n_largest(&mut v, &[lst.clone(), int(2)]).unwrap();
        let vals = match &got {
            Object::Ref(p) => {
                // SAFETY: got 由 top_n 的 alloc_list 分配。
                unsafe { read_list(*p) }.clone()
            }
            _ => panic!("n_largest must return list"),
        };
        assert!(vals == vec![int(3), int(2)], "n_largest([3,1,2],2) == [3,2]");
        for n in [0, -1] {
            let got = native_heap_n_smallest(&mut v, &[lst.clone(), int(n)]).unwrap();
            match &got {
                Object::Ref(p) => {
                    // SAFETY: 同上。
                    assert!(unsafe { read_list(*p) }.is_empty(), "n<=0 → []");
                }
                _ => panic!("n_smallest must return list"),
            }
        }
        // n ≥ len：全量排序返回
        let got = native_heap_n_smallest(&mut v, &[lst.clone(), int(10)]).unwrap();
        match &got {
            Object::Ref(p) => {
                // SAFETY: 同上。
                let vals = unsafe { read_list(*p) }.clone();
                assert!(vals == vec![int(1), int(2), int(3)], "n>=len 全量升序");
            }
            _ => panic!("n_smallest must return list"),
        }
        // 原 list 不被 n_* 修改
        match &lst {
            Object::Ref(p) => {
                // SAFETY: lst 由本测试 alloc_list 分配。
                let vals = unsafe { read_list(*p) }.clone();
                assert!(vals == vec![int(3), int(1), int(2)], "n_* 不改原 list");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_integration_heapq_errors_not_caught() {
        // 原生错误不可捕获（task 48/49 惯例）：整体 Err + 前缀断言。
        for (call, expect) in [
            ("heapq.heap_pop([])", "IndexError"),
            ("heapq.heapify(42)", "TypeError"),
            ("heapq.n_largest([1, \"x\"], 1)", "TypeError"),
        ] {
            let full = format!("import heapq\n{}", call);
            let r = run_source(&full);
            assert!(r.is_err(), "{} should fail", call);
            let e = r.unwrap_err();
            assert!(
                e.contains(expect),
                "{}: expected {} in {}",
                call,
                expect,
                e
            );
        }
    }

    #[test]
    fn test_string_heap_ordering() {
        // 字符串堆（字典序）。
        let mut v = VM::new();
        let lst = alloc_list(vec![
            alloc_string("pear"),
            alloc_string("apple"),
            alloc_string("fig"),
        ]);
        native_heap_heapify(&mut v, std::slice::from_ref(&lst)).unwrap();
        let got = native_heap_heap_pop(&mut v, &[lst]).unwrap();
        let got_s = match &got {
            Object::Ref(p) => {
                // SAFETY: heap_pop 返回 list 内元素（STRING Ref）。
                unsafe { crate::vm::object::read_str(*p) }.to_string()
            }
            _ => panic!("expected String"),
        };
        assert_eq!(got_s, "apple");
    }
}
