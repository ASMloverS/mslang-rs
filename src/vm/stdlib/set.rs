//! Set 内建方法（GET_ATTR → BoundMethod 分派）。
//!
//! 参照 [51-builtin-methods-list-dict-set](../../../docs/mslang/tasks/51-builtin-methods-list-dict-set.md)。

use super::hash_key;
use crate::vm::builtins::NativeFn;
use crate::vm::object::{alloc_set, read_set, MsObjHeader, Object, TypeTag};
use crate::vm::VM;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Set 方法（task 51）
// ---------------------------------------------------------------------------

/// Set 方法名 → 原生函数。
pub fn lookup_set_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_set_length,
        "add" => native_set_add,
        "remove" => native_set_remove,
        "contains" => native_set_contains,
        "union" => native_set_union,
        "intersection" => native_set_intersection,
        "difference" => native_set_difference,
        _ => return None,
    };
    Some(func)
}

fn native_set_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_set(ptr) }.len() as i64))
}

fn native_set_add(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "add(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: add(value) requires 1 argument".to_string())?;
    hash_key(&val)?;
    unsafe { read_set(ptr) }.insert(val);
    Ok(Object::Nil)
}

fn native_set_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "remove(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    hash_key(&val)?;
    if !unsafe { read_set(ptr) }.remove(&val) {
        return Err("KeyError: element not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_set_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "contains(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    let found = if hash_key(&val).is_ok() {
        unsafe { read_set(ptr) }.contains(&val)
    } else {
        false
    };
    Ok(Object::Bool(found))
}

fn native_set_union(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "union(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "union(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.union(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_intersection(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "intersection(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "intersection(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.intersection(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_difference(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "difference(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "difference(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.difference(&b).cloned().collect();
    Ok(alloc_set(result))
}

/// 校验首参数为 Set Ref，返回裸指针。
fn expect_set_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects set, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{ilist, run_source, vm};

    #[test]
    fn test_lookup_set_method() {
        let names = ["length", "add", "remove", "contains", "union", "intersection", "difference"];
        for name in &names {
            assert!(lookup_set_method(name).is_some(), "missing set method: {}", name);
        }
        assert!(lookup_set_method("nosuch").is_none());
    }

    #[test]
    fn test_set_methods_basic() {
        use std::collections::HashSet;
        let mut v = vm();

        let set1 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(1));
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs
        });

        // length
        assert_eq!(native_set_length(&mut v, &[set1.clone()]).unwrap(), Object::Int(3));

        // add
        native_set_add(&mut v, &[set1.clone(), Object::Int(4)]).unwrap();
        assert_eq!(native_set_contains(&mut v, &[set1.clone(), Object::Int(4)]).unwrap(), Object::Bool(true));

        // remove
        native_set_remove(&mut v, &[set1.clone(), Object::Int(4)]).unwrap();
        assert_eq!(native_set_contains(&mut v, &[set1.clone(), Object::Int(4)]).unwrap(), Object::Bool(false));

        // union
        let set2 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(5));
            hs.insert(Object::Int(6));
            hs
        });
        let u = native_set_union(&mut v, &[set1.clone(), set2]).unwrap();
        assert_eq!(native_set_length(&mut v, &[u]).unwrap(), Object::Int(5));

        // intersection
        let set3 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs.insert(Object::Int(7));
            hs
        });
        let inter = native_set_intersection(&mut v, &[set1.clone(), set3.clone()]).unwrap();
        assert_eq!(native_set_length(&mut v, &[inter]).unwrap(), Object::Int(2));

        // difference
        let diff = native_set_difference(&mut v, &[set1, set3]).unwrap();
        assert_eq!(native_set_length(&mut v, &[diff]).unwrap(), Object::Int(1)); // {1}
    }

    #[test]
    fn test_set_remove_missing_error() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let err = native_set_remove(&mut v, &[set, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("KeyError:"), "got: {}", err);
    }

    #[test]
    fn test_set_add_unhashable() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let list_val = ilist(&[1, 2]);
        let err = native_set_add(&mut v, &[set, list_val]).unwrap_err();
        assert!(err.starts_with("TypeError:"), "got: {}", err);
        assert!(err.contains("unhashable"));
    }

    #[test]
    fn test_set_contains_unhashable_returns_false() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let list_val = ilist(&[1, 2]);
        // contains on unhashable → false (not error)
        let result = native_set_contains(&mut v, &[set, list_val]).unwrap();
        assert_eq!(result, Object::Bool(false));
    }

    #[test]
    fn test_set_union_self_reference() {
        use std::collections::HashSet;
        let mut v = vm();
        let s = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(1));
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs
        });
        let u = native_set_union(&mut v, &[s.clone(), s.clone()]).unwrap();
        assert_eq!(native_set_length(&mut v, &[u]).unwrap(), Object::Int(3));
    }

    #[test]
    fn test_integration_set_methods() {
        let src = r#"
s = {1, 2, 3}
s.add(4)
assert(s.contains(4))
u = s.union({5, 6})
assert(u.length() == 6)
i = s.intersection({2, 3, 7})
assert(i.length() == 2)
d = s.difference({1, 2})
assert(d.length() == 2)
s.remove(4)
assert(not s.contains(4))
assert(s.length() == 3)
print(s)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "set integration failed: {:?}", r.err());
    }
}
