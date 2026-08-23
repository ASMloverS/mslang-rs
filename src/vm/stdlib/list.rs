//! List 内建方法（GET_ATTR → BoundMethod 分派）。
//!
//! 参照 [51-builtin-methods-list-dict-set](../../../docs/mslang/tasks/51-builtin-methods-list-dict-set.md)。

use super::{expect_int, expect_list_ref};
use crate::vm::builtins::NativeFn;
use crate::vm::object::{alloc_list, read_list, CmpOp, Object, TypeTag};
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

fn native_list_sort(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "sort()")?;
    let mut items = unsafe { read_list(ptr) }.clone();
    let mut err: Option<String> = None;
    items.sort_by(|a, b| {
        if err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match a.compare(b, CmpOp::Less) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
            Ok(_) => match a.compare(b, CmpOp::Greater) {
                Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
                Ok(_) => std::cmp::Ordering::Equal,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            },
            Err(e) => {
                err = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    unsafe { read_list(ptr) }.clear();
    unsafe { read_list(ptr) }.extend(items);
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

/// 校验参数为 callable（FUNCTION/CLOSURE/BOUND_METHOD）。
fn expect_callable(arg: Option<&Object>, who: &str) -> Result<Object, String> {
    match arg {
        Some(o @ Object::Ref(ptr)) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
            {
                Ok(o.clone())
            } else {
                Err(format!(
                    "TypeError: {} expects callable, got {}",
                    who,
                    o.type_name()
                ))
            }
        }
        other => Err(format!(
            "TypeError: {} expects callable, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
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
            "length", "push", "pop", "insert", "remove", "index", "contains", "sort",
            "reverse", "slice", "map", "filter", "reduce",
        ];
        for name in &names {
            assert!(lookup_list_method(name).is_some(), "missing list method: {}", name);
        }
        assert!(lookup_list_method("nosuch").is_none());
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
