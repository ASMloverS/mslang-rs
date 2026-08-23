//! Dict 内建方法（GET_ATTR → BoundMethod 分派）。
//!
//! 参照 [51-builtin-methods-list-dict-set](../../../docs/mslang/tasks/51-builtin-methods-list-dict-set.md)。

use super::hash_key;
use crate::vm::builtins::NativeFn;
use crate::vm::object::{alloc_list, alloc_tuple, read_dict, MsObjHeader, Object, TypeTag};
use crate::vm::VM;

// ---------------------------------------------------------------------------
// Dict 方法（task 51）
// ---------------------------------------------------------------------------

/// Dict 方法名 → 原生函数。
pub fn lookup_dict_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_dict_length,
        "keys" => native_dict_keys,
        "values" => native_dict_values,
        "items" => native_dict_items,
        "get" => native_dict_get,
        "set" => native_dict_set,
        "remove" => native_dict_remove,
        "contains" => native_dict_contains,
        "merge" => native_dict_merge,
        _ => return None,
    };
    Some(func)
}

fn native_dict_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_dict(ptr) }.len() as i64))
}

fn native_dict_keys(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "keys()")?;
    let keys: Vec<Object> = unsafe { read_dict(ptr) }.keys().into_iter().cloned().collect();
    Ok(alloc_list(keys))
}

fn native_dict_values(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "values()")?;
    let vals: Vec<Object> = unsafe { read_dict(ptr) }
        .items()
        .into_iter()
        .map(|(_, v)| v.clone())
        .collect();
    Ok(alloc_list(vals))
}

fn native_dict_items(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "items()")?;
    let items: Vec<Object> = unsafe { read_dict(ptr) }
        .items()
        .into_iter()
        .map(|(k, v)| alloc_tuple(vec![k.clone(), v.clone()]))
        .collect();
    Ok(alloc_list(items))
}

fn native_dict_get(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "get(key, default?)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: get(key, default?) requires 1-2 arguments".to_string())?;
    hash_key(&key)?;
    let default = if args.len() > 2 {
        args.get(2).cloned().unwrap()
    } else {
        Object::Nil
    };
    let dict = unsafe { read_dict(ptr) };
    Ok(dict.get(&key).cloned().unwrap_or(default))
}

fn native_dict_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "set(key, value)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    let val = args
        .get(2)
        .cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    hash_key(&key)?;
    unsafe { read_dict(ptr) }.insert(key, val);
    Ok(Object::Nil)
}

fn native_dict_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "remove(key)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    if unsafe { read_dict(ptr) }.remove(&key).is_none() {
        return Err("KeyError: key not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_dict_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "contains(key)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    let found = unsafe { read_dict(ptr) }.get(&key).is_some();
    Ok(Object::Bool(found))
}

fn native_dict_merge(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "merge(other)")?;
    let other_ptr = expect_dict_ref(args.get(1), "merge(other)")?;
    let pairs: Vec<(Object, Object)> = unsafe { read_dict(other_ptr) }
        .items()
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in pairs {
        unsafe { read_dict(ptr) }.insert(k, v);
    }
    Ok(Object::Nil)
}

/// 校验首参数为 Dict Ref，返回裸指针。
fn expect_dict_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects dict, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{ilist, run_source, s, vm};
    use crate::vm::object::{alloc_dict, read_list, DictMap};

    #[test]
    fn test_lookup_dict_method() {
        let names = [
            "length", "keys", "values", "items", "get", "set", "remove", "contains", "merge",
        ];
        for name in &names {
            assert!(lookup_dict_method(name).is_some(), "missing dict method: {}", name);
        }
        assert!(lookup_dict_method("nosuch").is_none());
    }

    #[test]
    fn test_dict_methods_basic() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(s("a"), Object::Int(1));
        m.insert(s("b"), Object::Int(2));
        let d = alloc_dict(m);

        // length
        assert_eq!(native_dict_length(&mut v, &[d.clone()]).unwrap(), Object::Int(2));

        // keys
        let keys = native_dict_keys(&mut v, &[d.clone()]).unwrap();
        let keys_vec = unsafe { read_list(match &keys { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(keys_vec.len(), 2);

        // values
        let vals = native_dict_values(&mut v, &[d.clone()]).unwrap();
        let vals_vec = unsafe { read_list(match &vals { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(vals_vec.len(), 2);

        // items
        let items = native_dict_items(&mut v, &[d.clone()]).unwrap();
        let items_vec = unsafe { read_list(match &items { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(items_vec.len(), 2);

        // get with default
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("c"), Object::Int(0)]).unwrap(), Object::Int(0));
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("a")]).unwrap(), Object::Int(1));

        // set
        native_dict_set(&mut v, &[d.clone(), s("c"), Object::Int(3)]).unwrap();
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("c")]).unwrap(), Object::Int(3));

        // contains
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("a")]).unwrap(), Object::Bool(true));
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("z")]).unwrap(), Object::Bool(false));

        // remove
        native_dict_remove(&mut v, &[d.clone(), s("c")]).unwrap();
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("c")]).unwrap(), Object::Bool(false));
    }

    #[test]
    fn test_dict_remove_missing_key_error() {
        let mut v = vm();
        let d = alloc_dict(DictMap::new());
        let err = native_dict_remove(&mut v, &[d, s("nope")]).unwrap_err();
        assert!(err.starts_with("KeyError:"), "got: {}", err);
    }

    #[test]
    fn test_dict_merge() {
        let mut v = vm();
        let mut m1 = DictMap::new();
        m1.insert(s("a"), Object::Int(1));
        let d1 = alloc_dict(m1);

        let mut m2 = DictMap::new();
        m2.insert(s("b"), Object::Int(2));
        let d2 = alloc_dict(m2);

        native_dict_merge(&mut v, &[d1.clone(), d2]).unwrap();
        assert_eq!(native_dict_length(&mut v, &[d1]).unwrap(), Object::Int(2));
    }

    #[test]
    fn test_dict_merge_self_reference() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(s("a"), Object::Int(1));
        let d = alloc_dict(m);
        // d.merge(d) should not deadlock
        native_dict_merge(&mut v, &[d.clone(), d.clone()]).unwrap();
        assert_eq!(native_dict_length(&mut v, &[d]).unwrap(), Object::Int(1));
    }

    #[test]
    fn test_dict_set_unhashable_key() {
        let mut v = vm();
        let d = alloc_dict(DictMap::new());
        let list_key = ilist(&[1, 2]);
        let err = native_dict_set(&mut v, &[d, list_key, Object::Int(3)]).unwrap_err();
        assert!(err.starts_with("TypeError:"), "got: {}", err);
        assert!(err.contains("unhashable"));
    }

    #[test]
    fn test_integration_dict_methods() {
        let src = r#"
d = {"a": 1, "b": 2}
assert(d.length() == 2)
assert(d.get("a") == 1)
assert(d.get("c", 0) == 0)
d.set("c", 3)
assert(d.contains("c"))
assert(d.get("c") == 3)
d.remove("c")
assert(not d.contains("c"))
d.merge({"d": 4})
assert(d.contains("d"))
ks = d.keys()
assert(ks.length() == 3)
vs = d.values()
assert(vs.length() == 3)
it = d.items()
assert(it.length() == 3)
print(d)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "dict integration failed: {:?}", r.err());
    }
}
