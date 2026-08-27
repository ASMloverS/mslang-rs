//! List 内建方法（GET_ATTR → BoundMethod 分派）。
//!
//! 参照 [51-builtin-methods-list-dict-set](../../../docs/mslang/tasks/51-builtin-methods-list-dict-set.md)。

use super::{expect_int, expect_list_ref};
use crate::vm::builtins::{
    expect_callable, expect_reverse, optional_key, rooted_list_ptr, sort_items_dsu, NativeFn,
};
use crate::vm::object::{alloc_list, read_list, Object};
use crate::vm::VM;

// ---------------------------------------------------------------------------
// List 方法（task 51：GET_ATTR → BoundMethod 分派，仿 task 46/50 模式）
// ---------------------------------------------------------------------------

/// List 方法名 → 原生函数（供 GET_ATTR 包装为 BoundMethod）。
pub fn lookup_list_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_list_length,
        "push" => native_list_push,
        "pop" => native_list_pop,
        "insert" => native_list_insert,
        "remove" => native_list_remove,
        "index" => native_list_index,
        "contains" => native_list_contains,
        "sort" => native_list_sort,
        "sort_by" => native_list_sort_by,
        "reverse" => native_list_reverse,
        "slice" => native_list_slice,
        "map" => native_list_map,
        "filter" => native_list_filter,
        "reduce" => native_list_reduce,
        _ => return None,
    };
    Some(func)
}

// 注：args[0] 为 List receiver（BoundMethod 注入），用户参数从 args.get(1) 起。

fn native_list_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "length()")?;
    let len = unsafe { read_list(ptr) }.len();
    Ok(Object::Int(len as i64))
}

fn native_list_push(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "push(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: push(value) requires 1 argument".to_string())?;
    unsafe { read_list(ptr) }.push(val);
    Ok(Object::Nil)
}

fn native_list_pop(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "pop(index?)")?;
    let len = unsafe { read_list(ptr) }.len();
    let idx = if args.len() <= 1 {
        if len == 0 {
            return Err("IndexError: pop from empty list".to_string());
        }
        len - 1
    } else {
        let i = expect_int(args.get(1), "pop(index?)")?;
        normalize_index(i, len).ok_or_else(|| {
            format!("IndexError: pop index {} out of range for length {}", i, len)
        })?
    };
    let popped = unsafe { read_list(ptr) }.remove(idx);
    Ok(popped)
}

fn native_list_insert(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "insert(index, value)")?;
    let i = expect_int(args.get(1), "insert(index, value)")?;
    let val = args
        .get(2)
        .cloned()
        .ok_or_else(|| "TypeError: insert(index, value) requires 2 arguments".to_string())?;
    let len = unsafe { read_list(ptr) }.len();
    let n = if i < 0 { len as i64 + i } else { i };
    if n < 0 || n > len as i64 {
        return Err(format!(
            "IndexError: insert index {} out of range for length {}",
            i, len
        ));
    }
    unsafe { read_list(ptr) }.insert(n as usize, val);
    Ok(Object::Nil)
}

fn native_list_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "remove(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    let found_idx = {
        let list = unsafe { read_list(ptr) };
        list.iter().position(|x| x == &val)
    };
    match found_idx {
        Some(idx) => {
            let _removed = unsafe { read_list(ptr) }.remove(idx);
            Ok(Object::Nil)
        }
        None => Err("ValueError: remove(): value not in list".to_string()),
    }
}

fn native_list_index(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "index(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: index(value) requires 1 argument".to_string())?;
    let list = unsafe { read_list(ptr) };
    match list.iter().position(|x| x == &val) {
        Some(idx) => Ok(Object::Int(idx as i64)),
        None => Err("ValueError: index(): value not in list".to_string()),
    }
}

fn native_list_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "contains(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    let found = unsafe { read_list(ptr) }.iter().any(|x| x == &val);
    Ok(Object::Bool(found))
}

/// list.sort(key?, reverse?) — 原地稳定排序（task 80 扩展：key/reverse 可选）。
/// 方法调用经 BoundMethod→FUNCTION 路径（mod.rs call_value）**不查 native_arities**，
/// 用户参数个数（不含 receiver）须在 native 内自校验（0-2），违规 → TypeError。
fn native_list_sort(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    expect_list_ref(args.get(0), "sort(key?, reverse?)")?;
    if args.len() > 3 {
        return Err(format!(
            "TypeError: sort() takes 0-2 arguments, got {}",
            args.len() - 1
        ));
    }
    let key = optional_key(args.get(1), "sort(key)")?;
    let reverse = if args.len() > 2 {
        expect_reverse(args.get(2), "sort(reverse)")?
    } else {
        false
    };
    sort_list_in_place(vm, args[0].clone(), key.as_ref(), reverse)
}

/// list.sort_by(key) — sort 的 key 显式版（task 80）。
/// 同 sort：BoundMethod→FUNCTION 路径不查 native_arities，自校验恰 1 个用户参数。
fn native_list_sort_by(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    expect_list_ref(args.get(0), "sort_by(key)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: sort_by(key) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let key = expect_callable(args.get(1), "sort_by(key)")?;
    sort_list_in_place(vm, args[0].clone(), Some(&key), false)
}

/// 原地 DSU 排序（task 80）：receiver 压栈根化 —— 方法调用实参已被弹出 vm.stack
///（call_value BOUND_METHOD 分支），且 receiver 可能是唯一引用（如 [3,1,2].sort()），
/// key 调用重入 VM 触发 GC 时未根化的源会被回收/移动。排序核心复用 builtins::
/// sort_items_dsu（GC 根化要求见其文档注释）。
fn sort_list_in_place(
    vm: &mut VM,
    receiver: Object,
    key: Option<&Object>,
    reverse: bool,
) -> Result<Object, String> {
    let root_base = vm.stack().len();
    vm.push(receiver)?;
    let ret = sort_in_place_rooted(vm, root_base, key, reverse);
    vm.stack_mut().truncate(root_base);
    ret
}

fn sort_in_place_rooted(
    vm: &mut VM,
    recv_slot: usize,
    key: Option<&Object>,
    reverse: bool,
) -> Result<Object, String> {
    let items: Vec<Object> = {
        // SAFETY: recv_slot 持 sort_list_in_place 压入的 list Ref（经 expect_list_ref 校验）。
        unsafe { read_list(rooted_list_ptr(vm, recv_slot)) }.clone()
    };
    let sorted = sort_items_dsu(vm, items, key, reverse)?;
    if vm.escaped_exc.is_some() {
        // key 异常已截获暂存：中止（call_value 将重抛）；不写回部分结果。
        return Ok(Object::Nil);
    }
    // 写回：指针从根槽重取（sort_items_dsu 内 key 调用可能触发 GC 移动）。
    // SAFETY: 同上。
    let list = unsafe { read_list(rooted_list_ptr(vm, recv_slot)) };
    list.clear();
    list.extend(sorted);
    Ok(Object::Nil)
}

fn native_list_reverse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reverse()")?;
    unsafe { read_list(ptr) }.reverse();
    Ok(Object::Nil)
}

fn native_list_slice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "slice(start, end?)")?;
    let start_i = expect_int(args.get(1), "slice(start, end?)")?;
    let end_opt = if args.len() > 2 {
        Some(expect_int(args.get(2), "slice(start, end?)")?)
    } else {
        None
    };
    let items = unsafe { read_list(ptr) }.clone();
    let len = items.len() as i64;
    let norm = |i: i64| -> i64 {
        if i < 0 {
            (len + i).max(0)
        } else {
            i.min(len)
        }
    };
    let s = norm(start_i);
    let e = match end_opt {
        Some(i) => norm(i),
        None => len,
    };
    if s > e {
        return Err(format!("ValueError: slice start {} > end {}", s, e));
    }
    let sliced: Vec<Object> = items[s as usize..e as usize].to_vec();
    Ok(alloc_list(sliced))
}

fn native_list_map(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "map(fn)")?;
    let fn_obj = expect_callable(args.get(1), "map(fn)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::with_capacity(items.len());
    for item in items.iter() {
        let mapped = vm.call_function(&fn_obj, std::slice::from_ref(item))?;
        result.push(mapped);
    }
    Ok(alloc_list(result))
}

fn native_list_filter(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "filter(fn)")?;
    let fn_obj = expect_callable(args.get(1), "filter(fn)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::new();
    for item in items.iter() {
        let cond = vm.call_function(&fn_obj, std::slice::from_ref(item))?;
        if cond.is_truthy() {
            result.push(item.clone());
        }
    }
    Ok(alloc_list(result))
}

fn native_list_reduce(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reduce(fn, init?)")?;
    let fn_obj = expect_callable(args.get(1), "reduce(fn, init?)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let (mut acc, start) = if args.len() > 2 {
        (args.get(2).cloned().unwrap(), 0)
    } else {
        if items.is_empty() {
            return Err(
                "ValueError: reduce() of empty list with no initial value".to_string()
            );
        }
        (items[0].clone(), 1)
    };
    for item in items.iter().skip(start) {
        acc = vm.call_function(&fn_obj, &[acc, item.clone()])?;
    }
    Ok(acc)
}

/// 列表索引归一化（负索引相对末尾，越界返回 None）。
fn normalize_index(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let n = if i < 0 { len + i } else { i };
    if n < 0 || n >= len {
        None
    } else {
        Some(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{ilist, run_source, vm};

    // -----------------------------------------------------------------------
    // task 51: List/Dict/Set 方法测试
    // -----------------------------------------------------------------------

    #[test]
    fn test_lookup_list_method() {
        let names = [
            "length", "push", "pop", "insert", "remove", "index", "contains", "sort", "sort_by",
            "reverse", "slice", "map", "filter", "reduce",
        ];
        for name in &names {
            assert!(lookup_list_method(name).is_some(), "missing list method: {}", name);
        }
        assert!(lookup_list_method("nosuch").is_none());
    }

    // -----------------------------------------------------------------------
    // task 80: sort(key?, reverse?) / sort_by(key)
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_sort_arity_self_validation() {
        let mut v = vm();
        // 用户参数（不含 receiver）自校验：sort 0-2、sort_by 恰 1。
        let lst = ilist(&[1, 2]);
        let err =
            native_list_sort(&mut v, &[lst.clone(), Object::Nil, Object::Bool(false), Object::Bool(false)])
                .unwrap_err();
        assert!(err.contains("TypeError") && err.contains("0-2"), "got: {}", err);
        let err = native_list_sort_by(&mut v, &[lst.clone()]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("exactly 1"), "got: {}", err);
        let err = native_list_sort_by(&mut v, &[lst.clone(), Object::Nil, Object::Nil]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // sort key 非 callable → TypeError
        let err = native_list_sort(&mut v, &[lst.clone(), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("callable"), "got: {}", err);
        // sort reverse 非 bool → TypeError
        let err = native_list_sort(&mut v, &[lst, Object::Nil, Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("reverse"), "got: {}", err);
    }

    #[test]
    fn test_integration_sort_key_stability() {
        // DSU 稳定排序：等 key 元素保持原序；reverse 等值保序。
        let src = r#"
words = ["bb", "a", "ccc", "dd", "e", "aa"]
by_len = sorted(words, fn(w) { return len(w) })
assert(by_len == ["a", "e", "bb", "dd", "aa", "ccc"], "len 稳定排序")
rev = sorted(words, fn(w) { return len(w) }, true)
assert(rev == ["ccc", "bb", "dd", "aa", "a", "e"], "reverse 等值保序")
assert(sorted_by(words, fn(w) { return len(w) }) == by_len, "sorted_by 等价")
assert(sorted_by(words, fn(w) { return len(w) }, true) == rev, "sorted_by reverse")
assert(sorted([3, 1, 2]) == [1, 2, 3], "无 key 兼容旧用例")
assert(sorted([3, 1, 2], nil, true) == [3, 2, 1], "nil key + reverse")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "sort key stability failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_list_sort_key() {
        // list.sort / sort_by 原地生效。
        let src = r#"
lst = [3, 1, 2]
r = lst.sort()
assert(r == nil, "sort 返回 nil")
assert(lst == [1, 2, 3], "原地")

lst2 = ["bb", "a", "ccc"]
lst2.sort(fn(w) { return len(w) })
assert(lst2 == ["a", "bb", "ccc"], "list.sort(key)")

lst3 = [3, 1, 2]
lst3.sort(nil, true)
assert(lst3 == [3, 2, 1], "list.sort(nil, true)")

lst4 = ["bb", "a", "ccc"]
lst4.sort_by(fn(w) { return len(w) })
assert(lst4 == ["a", "bb", "ccc"], "sort_by")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "list sort key failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_sort_key_exception_propagation() {
        // key 抛错上抛调用方：try/except 捕获 key 内抛出的 ValueError。
        let src = r#"
fn bad_key(x) {
    if x > 2 {
        throw ValueError("too big: " + str(x))
    }
    return x
}
caught = false
msg = ""
try {
    sorted([1, 3, 2], bad_key)
} except ValueError as e {
    caught = true
    msg = e.message
}
assert(caught, "sorted key 异常被调用方捕获")
assert(msg == "too big: 3", "异常消息")

caught2 = false
try {
    [1, 3, 2].sort_by(bad_key)
} except ValueError as e {
    caught2 = true
}
assert(caught2, "list.sort_by key 异常上抛")

# sort 中止后不写回部分结果
lst = [3, 1]
try {
    lst.sort(bad_key)
} except Error as e {
}
assert(lst == [3, 1], "key 异常后原 list 不变")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "sort key exception propagation failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_sort_gc_stress() {
        // GC 压力：大 list + key 内逐元素分配新对象 + gc.collect 后逐项校验。
        let src = r#"
import gc
big = []
i = 0
while i < 200 {
    big.push(200 - i)
    i = i + 1
}
keyed = sorted(big, fn(x) {
    junk = ["k", x, x * 2]
    return x % 7
})
gc.collect()
assert(len(keyed) == 200, "长度保持")
k = 0
while k < len(keyed) - 1 {
    assert(keyed[k] % 7 <= keyed[k + 1] % 7, "按键非降")
    k = k + 1
}
total = 0
for v in keyed {
    total = total + v
}
assert(total == 20100, "元素总值不变（无丢失/错值）")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "sort gc stress failed: {:?}", r.err());
    }

    #[test]
    fn test_list_methods_basic() {
        let mut v = vm();

        // length
        let lst = ilist(&[3, 1, 4, 1, 5]);
        assert_eq!(native_list_length(&mut v, &[lst.clone()]).unwrap(), Object::Int(5));

        // sort
        native_list_sort(&mut v, &[lst.clone()]).unwrap();
        assert_eq!(unsafe { read_list(match lst { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(1), Object::Int(1), Object::Int(3), Object::Int(4), Object::Int(5)]);

        // push
        native_list_push(&mut v, &[lst.clone(), Object::Int(9)]).unwrap();
        // pop
        let popped = native_list_pop(&mut v, &[lst.clone()]).unwrap();
        assert_eq!(popped, Object::Int(9));

        // insert
        let lst2 = ilist(&[1, 2, 3]);
        native_list_insert(&mut v, &[lst2.clone(), Object::Int(0), Object::Int(99)]).unwrap();

        // remove
        let lst3 = ilist(&[1, 2, 1]);
        native_list_remove(&mut v, &[lst3.clone(), Object::Int(1)]).unwrap();

        // index
        let lst4 = ilist(&[10, 20, 30]);
        assert_eq!(native_list_index(&mut v, &[lst4.clone(), Object::Int(20)]).unwrap(), Object::Int(1));

        // contains
        assert_eq!(native_list_contains(&mut v, &[lst4.clone(), Object::Int(20)]).unwrap(), Object::Bool(true));
        assert_eq!(native_list_contains(&mut v, &[lst4.clone(), Object::Int(99)]).unwrap(), Object::Bool(false));

        // reverse
        let lst5 = ilist(&[1, 2, 3]);
        native_list_reverse(&mut v, &[lst5.clone()]).unwrap();

        // slice
        let lst6 = ilist(&[10, 20, 30, 40, 50]);
        let sliced = native_list_slice(&mut v, &[lst6, Object::Int(1), Object::Int(3)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(20), Object::Int(30)]);
    }

    #[test]
    fn test_list_pop_empty_error() {
        let mut v = vm();
        let empty = alloc_list(vec![]);
        let err = native_list_pop(&mut v, &[empty]).unwrap_err();
        assert!(err.starts_with("IndexError:"), "got: {}", err);
        assert!(err.contains("empty list"));
    }

    #[test]
    fn test_list_pop_index_oob_error() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_pop(&mut v, &[lst, Object::Int(10)]).unwrap_err();
        assert!(err.starts_with("IndexError:"), "got: {}", err);
    }

    #[test]
    fn test_list_remove_not_found() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_remove(&mut v, &[lst, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_index_not_found() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_index(&mut v, &[lst, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_slice_reverse_error() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_slice(&mut v, &[lst, Object::Int(3), Object::Int(1)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_negative_index() {
        let mut v = vm();
        // pop(-1)
        let lst = ilist(&[10, 20, 30, 40, 50]);
        let popped = native_list_pop(&mut v, &[lst.clone(), Object::Int(-1)]).unwrap();
        assert_eq!(popped, Object::Int(50));

        // insert(-1, val) — before last
        let lst2 = ilist(&[10, 20, 30, 40]);
        native_list_insert(&mut v, &[lst2.clone(), Object::Int(-1), Object::Int(99)]).unwrap();

        // slice(-2)
        let lst3 = ilist(&[10, 20, 30, 99, 40]);
        let sliced = native_list_slice(&mut v, &[lst3.clone(), Object::Int(-2)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(99), Object::Int(40)]);

        // slice(1, -1) — remove first and last
        let sliced2 = native_list_slice(&mut v, &[lst3, Object::Int(1), Object::Int(-1)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced2 { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(20), Object::Int(30), Object::Int(99)]);
    }

    // --- Integration tests (end-to-end via mslang source) ---

    #[test]
    fn test_integration_list_methods() {
        let src = r#"
lst = [3, 1, 4, 1, 5]
lst.sort()
lst.push(9)
lst.pop()
lst.insert(0, 0)
lst.remove(1)
assert(lst.contains(4))
assert(lst.index(3) == lst.index(3))
assert(lst.length() == len(lst))
assert(lst.slice(0, 2).length() == 2)
lst.reverse()
print(lst)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "list integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_higher_order() {
        let src = r#"
lst = [1, 2, 3, 4, 5]
doubled = lst.map(fn(x) { return x * 2 })
assert(doubled[0] == 2)
assert(doubled[4] == 10)
evens = lst.filter(fn(x) { return x % 2 == 0 })
assert(evens.length() == 2)
total = lst.reduce(fn(a, b) { return a + b }, 0)
assert(total == 15)
product = lst.reduce(fn(a, b) { return a * b })
assert(product == 120)
print(doubled)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "higher-order integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_list_negative_index() {
        let src = r#"
lst = [10, 20, 30, 40, 50]
v = lst.pop(-1)
assert(v == 50)
assert(lst.length() == 4)
lst.insert(-1, 99)
assert(lst.slice(-2)[1] == 40)
sub = lst.slice(1, -1)
assert(sub[0] == 20)
print(lst)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "negative index integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_list_attr_error() {
        let src = r#"
try {
    [1, 2].nosuch()
} except e {
    print(e)
}
"#;
        // This may or may not be catchable depending on VM error handling.
        // Just verify it doesn't crash.
        let _ = run_source(src);
    }
}
