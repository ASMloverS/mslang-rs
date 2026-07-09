//! C API — 值创建与类型判断（task 67）。
//!
//! 参照 [67-capi-value-creation](../../docs/mslang/tasks/67-capi-value-creation.md)。
//!
//! 实现特殊值（Nil/Bool）、值创建（Int/Float/String/Stringn）、集合创建
//! （List/Dict/Set/Tuple/From 变体）和类型判断（msTypeof/msIs*）。
//!
//! 注意：`msStringFmt` 由 C 文件（vsnprintf_shim.c）导出，不在本模块定义。

use std::os::raw::{c_char, c_int};

use crate::capi::types::{MsType, MsValue};
use crate::capi::vm::MsVM;
use crate::vm::object::{
    alloc_dict, alloc_list, alloc_set, alloc_string, alloc_tuple, DictMap, Object, TypeTag,
};

// ---------------------------------------------------------------------------
// C 风格常量别名（与 types.h 中 #define / enum 值一致，供内部使用）
// ---------------------------------------------------------------------------

pub(crate) const MS_TRUE: c_int = 1;
pub(crate) const MS_FALSE: c_int = 0;

// ---------------------------------------------------------------------------
// 特殊值
// ---------------------------------------------------------------------------

/// 创建 Nil 值。每次调用返回新 Box（内联值，无需 GC 管理）。
#[no_mangle]
pub extern "C" fn msNil() -> *mut MsValue {
    Box::into_raw(Box::new(MsValue {
        inner: Object::Nil,
    }))
}

/// 创建 Bool 值。val != 0 → true，val == 0 → false。
#[no_mangle]
pub extern "C" fn msBoolVal(val: c_int) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue {
        inner: Object::Bool(val != 0),
    }))
}

// ---------------------------------------------------------------------------
// 值创建 — 内联值
// ---------------------------------------------------------------------------

/// 创建 Int 值（不依赖 VM）。
#[no_mangle]
pub extern "C" fn msInt(val: i64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue {
        inner: Object::Int(val),
    }))
}

/// 创建 Float 值（不依赖 VM）。
#[no_mangle]
pub extern "C" fn msFloat(val: f64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue {
        inner: Object::Float(val),
    }))
}

// ---------------------------------------------------------------------------
// 值创建 — 字符串
// ---------------------------------------------------------------------------

/// 从 C 空终止字符串创建 String 值。等价于 msStringn(vm, str, strlen(str))。
/// str 为 NULL 时创建空字符串。
#[no_mangle]
pub extern "C" fn msString(vm: *mut MsVM, str: *const c_char) -> *mut MsValue {
    if str.is_null() {
        return msStringn(vm, std::ptr::null(), 0);
    }
    // SAFETY: str 非空，指向有效 C 字符串（调用方保证）。
    let bytes = unsafe { std::ffi::CStr::from_ptr(str) }.to_bytes();
    msStringn(vm, str, bytes.len())
}

/// 从指定长度的字节创建 String 值（可包含 `\0`）。
/// str 为 NULL 或 len == 0 时创建空字符串。
#[no_mangle]
pub extern "C" fn msStringn(vm: *mut MsVM, str: *const c_char, len: usize) -> *mut MsValue {
    let _ = vm; // MVP：alloc_string 不依赖 VM 堆；VM 参数为未来 GC 集成预留。
    let bytes: &[u8] = if str.is_null() || len == 0 {
        b""
    } else {
        // SAFETY: str 指向至少 len 字节的有效内存（调用方保证）。
        unsafe { std::slice::from_raw_parts(str as *const u8, len) }
    };
    // C 侧字符串可能含非 UTF-8 字节；无效 UTF-8 时回退为空串。
    let s = std::str::from_utf8(bytes).unwrap_or("");
    let obj = alloc_string(s);
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

// ---------------------------------------------------------------------------
// 集合创建
// ---------------------------------------------------------------------------

/// 创建空 List。
#[no_mangle]
pub extern "C" fn msListNew(vm: *mut MsVM) -> *mut MsValue {
    let _ = vm;
    let obj = alloc_list(Vec::new());
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

/// 创建空 Dict。
#[no_mangle]
pub extern "C" fn msDictNew(vm: *mut MsVM) -> *mut MsValue {
    let _ = vm;
    let obj = alloc_dict(DictMap::new());
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

/// 创建空 Set。
#[no_mangle]
pub extern "C" fn msSetNew(vm: *mut MsVM) -> *mut MsValue {
    let _ = vm;
    let obj = alloc_set(std::collections::HashSet::new());
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

/// 从数组创建 List。NULL 元素被跳过。
#[no_mangle]
pub extern "C" fn msListFrom(
    vm: *mut MsVM,
    items: *const *mut MsValue,
    count: c_int,
) -> *mut MsValue {
    let _ = vm;
    let mut vec = Vec::new();
    if !items.is_null() && count > 0 {
        for i in 0..count as usize {
            // SAFETY: items 指向 count 个 MsValue* （调用方保证）。
            let item = unsafe { *items.add(i) };
            if item.is_null() {
                continue;
            }
            // SAFETY: item 由 ms* 创建，指向有效 MsValue。
            let obj = unsafe { (*item).inner.clone() };
            vec.push(obj);
        }
    }
    let obj = alloc_list(vec);
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

/// 从数组创建 Tuple。NULL 元素被跳过。
#[no_mangle]
pub extern "C" fn msTupleFrom(
    vm: *mut MsVM,
    items: *const *mut MsValue,
    count: c_int,
) -> *mut MsValue {
    let _ = vm;
    let mut vec = Vec::new();
    if !items.is_null() && count > 0 {
        for i in 0..count as usize {
            // SAFETY: items 指向 count 个 MsValue* （调用方保证）。
            let item = unsafe { *items.add(i) };
            if item.is_null() {
                continue;
            }
            // SAFETY: item 由 ms* 创建，指向有效 MsValue。
            let obj = unsafe { (*item).inner.clone() };
            vec.push(obj);
        }
    }
    let obj = alloc_tuple(vec);
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

/// 从扁平 key-value 数组创建 Dict。count 为键值对数量（数组长度 = count * 2）。
/// NULL key 或 value 对应的键值对被跳过。
#[no_mangle]
pub extern "C" fn msDictFrom(
    vm: *mut MsVM,
    pairs: *const *mut MsValue,
    count: c_int,
) -> *mut MsValue {
    let _ = vm;
    let mut map = DictMap::new();
    if !pairs.is_null() && count > 0 {
        for i in 0..count as usize {
            let key_idx = i * 2;
            // SAFETY: pairs 指向 count*2 个 MsValue* （调用方保证）。
            let key = unsafe { *pairs.add(key_idx) };
            let val = unsafe { *pairs.add(key_idx + 1) };
            if key.is_null() || val.is_null() {
                continue;
            }
            // SAFETY: key/val 由 ms* 创建，指向有效 MsValue。
            let key_obj = unsafe { (*key).inner.clone() };
            let val_obj = unsafe { (*val).inner.clone() };
            map.insert(key_obj, val_obj);
        }
    }
    let obj = alloc_dict(map);
    Box::into_raw(Box::new(MsValue { inner: obj }))
}

// ---------------------------------------------------------------------------
// 类型判断
// ---------------------------------------------------------------------------

/// 将内部 Object 转换为 C 侧 MsType 枚举值。
fn obj_to_ms_type(obj: &Object) -> MsType {
    match obj {
        Object::Nil => MsType::Nil,
        Object::Bool(_) => MsType::Bool,
        Object::Int(_) => MsType::Int,
        Object::Float(_) => MsType::Float,
        Object::Ref(header) => {
            // SAFETY: header 由 alloc_* 分配，指向有效 MsObjHeader。
            let tag = unsafe { (**header).type_tag };
            match tag {
                t if t == TypeTag::STRING as u8 => MsType::String,
                t if t == TypeTag::LIST as u8 => MsType::List,
                t if t == TypeTag::DICT as u8 => MsType::Dict,
                t if t == TypeTag::TUPLE as u8 => MsType::Tuple,
                t if t == TypeTag::SET as u8 => MsType::Set,
                t if t == TypeTag::FUNCTION as u8 => MsType::Function,
                t if t == TypeTag::CLOSURE as u8 => MsType::Function,
                t if t == TypeTag::CLASS as u8 => MsType::Class,
                t if t == TypeTag::INSTANCE as u8 => MsType::Instance,
                t if t == TypeTag::MODULE as u8 => MsType::Module,
                t if t == TypeTag::GENERATOR as u8 => MsType::Generator,
                t if t == TypeTag::FUTURE as u8 => MsType::Future,
                t if t == TypeTag::CHANNEL as u8 => MsType::Channel,
                t if t == TypeTag::ITERATOR as u8 => MsType::Iterator,
                t if t == TypeTag::BOUND_METHOD as u8 => MsType::BoundMethod,
                t if t == TypeTag::JOIN_HANDLE as u8 => MsType::JoinHandle,
                _ => MsType::Nil, // UPVALUE/EXCEPTION/FILE_HANDLE/LARGE_OBJECT 无对应 MsType
            }
        }
    }
}

/// 返回值类型的 MsType 枚举。NULL 安全（返回 Nil）。
#[no_mangle]
pub extern "C" fn msTypeof(val: *mut MsValue) -> MsType {
    if val.is_null() {
        return MsType::Nil;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    obj_to_ms_type(unsafe { &(*val).inner })
}

#[no_mangle]
pub extern "C" fn msIsNil(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    if matches!(unsafe { &(*val).inner }, Object::Nil) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsBool(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    if matches!(unsafe { &(*val).inner }, Object::Bool(_)) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsInt(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    if matches!(unsafe { &(*val).inner }, Object::Int(_)) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsFloat(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    if matches!(unsafe { &(*val).inner }, Object::Float(_)) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsNumber(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    match unsafe { &(*val).inner } {
        Object::Int(_) | Object::Float(_) => MS_TRUE,
        _ => MS_FALSE,
    }
}

/// 检查 val.inner 是否为 Ref(header) 且 header 的 type_tag 匹配指定 tag。
/// NULL val 返回 false。
fn is_ref_type(val: *mut MsValue, tag: TypeTag) -> bool {
    if val.is_null() {
        return false;
    }
    match unsafe { &(*val).inner } {
        Object::Ref(header) => {
            // SAFETY: header 由 alloc_* 分配。
            unsafe { (**header).type_tag == tag as u8 }
        }
        _ => false,
    }
}

#[no_mangle]
pub extern "C" fn msIsString(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::STRING) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsList(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::LIST) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsDict(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::DICT) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsTuple(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::TUPLE) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsSet(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::SET) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

/// msIsFunction 对 FUNCTION 和 CLOSURE 均返回 MS_TRUE。
#[no_mangle]
pub extern "C" fn msIsFunction(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    if is_ref_type(val, TypeTag::FUNCTION) || is_ref_type(val, TypeTag::CLOSURE) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsClass(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::CLASS) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsInstance(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::INSTANCE) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsGenerator(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::GENERATOR) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsFuture(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::FUTURE) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msIsChannel(val: *mut MsValue) -> c_int {
    if is_ref_type(val, TypeTag::CHANNEL) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::gc::{msRoot, msUnroot};
    use crate::capi::vm::{msVmFree, msVmNew};
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            // SAFETY: val 由 ms* 的 Box::into_raw 返回。
            unsafe {
                let _ = Box::from_raw(val);
            }
        }
    }

    #[test]
    fn test_create_int() {
        let val = msInt(42);
        assert!(!val.is_null());
        assert_eq!(msTypeof(val), MsType::Int);
        assert_eq!(msIsInt(val), MS_TRUE);
        assert_eq!(msIsFloat(val), MS_FALSE);
        assert_eq!(msIsNumber(val), MS_TRUE);
        free_value(val);
    }

    #[test]
    fn test_create_float() {
        let val = msFloat(3.14);
        assert!(!val.is_null());
        assert_eq!(msTypeof(val), MsType::Float);
        assert_eq!(msIsFloat(val), MS_TRUE);
        assert_eq!(msIsNumber(val), MS_TRUE);
        free_value(val);
    }

    #[test]
    fn test_create_nil_and_bool() {
        let nil = msNil();
        assert!(!nil.is_null());
        assert_eq!(msIsNil(nil), MS_TRUE);

        let t = msBoolVal(1);
        assert_eq!(msIsBool(t), MS_TRUE);
        assert_eq!(msIsNil(t), MS_FALSE);

        let f = msBoolVal(0);
        assert_eq!(msIsBool(f), MS_TRUE);

        free_value(nil);
        free_value(t);
        free_value(f);
    }

    #[test]
    fn test_create_string() {
        let vm = msVmNew();

        let s = msString(vm, b"hello\0".as_ptr() as *const c_char);
        assert!(!s.is_null());
        assert_eq!(msIsString(s), MS_TRUE);
        assert_eq!(msTypeof(s), MsType::String);
        free_value(s);
        msVmFree(vm);
    }

    #[test]
    fn test_create_stringn_with_null_bytes() {
        let vm = msVmNew();

        let data = b"ab\0cd";
        let s = msStringn(vm, data.as_ptr() as *const c_char, 5);
        assert!(!s.is_null());
        assert_eq!(msIsString(s), MS_TRUE);
        free_value(s);
        msVmFree(vm);
    }

    #[test]
    fn test_list_new_and_from() {
        let vm = msVmNew();

        let list = msListNew(vm);
        assert!(!list.is_null());
        assert_eq!(msIsList(list), MS_TRUE);
        assert_eq!(msTypeof(list), MsType::List);

        let a = msInt(1);
        let b = msInt(2);
        let c = msInt(3);
        let items = [a, b, c];
        let list2 = msListFrom(vm, items.as_ptr(), 3);
        assert!(!list2.is_null());
        assert_eq!(msIsList(list2), MS_TRUE);

        free_value(list);
        free_value(list2);
        free_value(a);
        free_value(b);
        free_value(c);
        msVmFree(vm);
    }

    #[test]
    fn test_tuple_from() {
        let vm = msVmNew();

        let a = msInt(10);
        let b = msInt(20);
        let items = [a, b];
        let tup = msTupleFrom(vm, items.as_ptr(), 2);
        assert!(!tup.is_null());
        assert_eq!(msIsTuple(tup), MS_TRUE);
        assert_eq!(msTypeof(tup), MsType::Tuple);

        free_value(tup);
        free_value(a);
        free_value(b);
        msVmFree(vm);
    }

    #[test]
    fn test_dict_new_and_from() {
        let vm = msVmNew();

        let dict = msDictNew(vm);
        assert!(!dict.is_null());
        assert_eq!(msIsDict(dict), MS_TRUE);

        let k1 = msString(vm, b"x\0".as_ptr() as *const c_char);
        let v1 = msInt(1);
        let k2 = msString(vm, b"y\0".as_ptr() as *const c_char);
        let v2 = msInt(2);
        let pairs = [k1, v1, k2, v2];
        let dict2 = msDictFrom(vm, pairs.as_ptr(), 2);
        assert!(!dict2.is_null());
        assert_eq!(msIsDict(dict2), MS_TRUE);

        free_value(dict);
        free_value(dict2);
        free_value(k1);
        free_value(v1);
        free_value(k2);
        free_value(v2);
        msVmFree(vm);
    }

    #[test]
    fn test_set_new() {
        let vm = msVmNew();

        let set = msSetNew(vm);
        assert!(!set.is_null());
        assert_eq!(msIsSet(set), MS_TRUE);
        assert_eq!(msTypeof(set), MsType::Set);

        free_value(set);
        msVmFree(vm);
    }

    #[test]
    fn test_type_checking_all_types() {
        let vm = msVmNew();

        let nil = msNil();
        assert_eq!(msIsNil(nil), MS_TRUE);
        assert_eq!(msIsBool(nil), MS_FALSE);

        let b = msBoolVal(1);
        assert_eq!(msIsBool(b), MS_TRUE);
        assert_eq!(msIsInt(b), MS_FALSE);

        let i = msInt(42);
        assert_eq!(msIsInt(i), MS_TRUE);
        assert_eq!(msIsFloat(i), MS_FALSE);
        assert_eq!(msIsNumber(i), MS_TRUE);

        let f = msFloat(1.0);
        assert_eq!(msIsFloat(f), MS_TRUE);
        assert_eq!(msIsNumber(f), MS_TRUE);
        assert_eq!(msIsInt(f), MS_FALSE);

        let s = msString(vm, b"test\0".as_ptr() as *const c_char);
        assert_eq!(msIsString(s), MS_TRUE);
        assert_eq!(msIsList(s), MS_FALSE);

        let list = msListNew(vm);
        assert_eq!(msIsList(list), MS_TRUE);
        assert_eq!(msIsDict(list), MS_FALSE);

        let dict = msDictNew(vm);
        assert_eq!(msIsDict(dict), MS_TRUE);
        assert_eq!(msIsSet(dict), MS_FALSE);

        let set = msSetNew(vm);
        assert_eq!(msIsSet(set), MS_TRUE);
        assert_eq!(msIsTuple(set), MS_FALSE);

        let tup = msTupleFrom(vm, ptr::null(), 0);
        assert_eq!(msIsTuple(tup), MS_TRUE);
        assert_eq!(msIsList(tup), MS_FALSE);

        for v in [nil, b, i, f, s, list, dict, set, tup] {
            free_value(v);
        }
        msVmFree(vm);
    }

    #[test]
    fn test_root_unroot() {
        let vm = msVmNew();

        let s = msString(vm, b"rooted\0".as_ptr() as *const c_char);
        assert!(!s.is_null());

        let result = msRoot(vm, s);
        assert_eq!(result, s);

        msUnroot(vm, s);

        free_value(s);
        msVmFree(vm);
    }

    #[test]
    fn test_root_inline_value_noop() {
        let vm = msVmNew();

        let i = msInt(42);
        msRoot(vm, i);
        msUnroot(vm, i);
        free_value(i);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety() {
        assert!(msTypeof(ptr::null_mut()) == MsType::Nil);

        let vm = msVmNew();

        msRoot(vm, ptr::null_mut());
        msUnroot(vm, ptr::null_mut());
        msVmFree(vm);
    }

    #[test]
    fn test_list_from_with_null_element() {
        let vm = msVmNew();

        let a = msInt(1);
        let c = msInt(3);
        let items = [a, ptr::null_mut(), c];
        let list = msListFrom(vm, items.as_ptr(), 3);
        assert!(!list.is_null());
        assert_eq!(msIsList(list), MS_TRUE);

        free_value(list);
        free_value(a);
        free_value(c);
        msVmFree(vm);
    }
}
