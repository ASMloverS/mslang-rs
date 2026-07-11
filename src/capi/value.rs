//! C API — 值创建与类型判断（task 67）。
//!
//! 参照 [67-capi-value-creation](../../docs/mslang/tasks/67-capi-value-creation.md)。
//!
//! 实现特殊值（Nil/Bool）、值创建（Int/Float/String/Stringn）、集合创建
//! （List/Dict/Set/Tuple/From 变体）和类型判断（msTypeof/msIs*）。
//!
//! 注意：`msStringFmt` 由 C 文件（vsnprintf_shim.c）导出，不在本模块定义。

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::capi::set_type_error;
use crate::capi::types::{MsStatus, MsType, MsValue};
use crate::capi::vm::MsVM;
use crate::vm::object::{
    alloc_dict, alloc_list, alloc_set, alloc_string, alloc_tuple, read_dict, read_list,
    read_set, read_str, read_tuple, DictMap, Object, TypeTag,
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

// ===========================================================================
// Task 68 — 值转换、比较与通用操作
// ===========================================================================

use std::cmp::Ordering;

// ---------------------------------------------------------------------------
// thread_local 缓冲区（msToString 借用引用）
// ---------------------------------------------------------------------------

// thread_local 缓冲区，避免修改 MsValue 的 #[repr(C)] 布局。
// 每次调用 msToString 覆盖上次内容。
thread_local! {
    static TO_STRING_BUF: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

/// 将 Object 格式化为字符串，存入 thread_local 缓冲区，返回借用指针。
fn format_to_thread_local(obj: &Object) -> *const c_char {
    let formatted = format!("{}", obj);
    let cstr = CString::new(formatted).unwrap_or_default();
    let ptr = cstr.as_ptr();
    TO_STRING_BUF.with(|buf| {
        *buf.borrow_mut() = Some(cstr);
    });
    ptr
}

// ---------------------------------------------------------------------------
// 值转换 — msToInt / msToFloat / msToBool / msToString / msToStringCopy
// ---------------------------------------------------------------------------

/// 从 MsValue 提取 i64。Int → i64；Float → 截断（NaN/Inf 设 TypeError 返回 0）；
/// 其余设 TypeError 返回 0。NULL 安全。
#[no_mangle]
pub extern "C" fn msToInt(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return 0;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    match unsafe { &(*val).inner } {
        Object::Int(n) => *n,
        Object::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                set_type_error(vm, "finite number", unsafe { &(*val).inner });
                0
            } else {
                *f as i64
            }
        }
        _ => {
            set_type_error(vm, "int or float", unsafe { &(*val).inner });
            0
        }
    }
}

/// 从 MsValue 提取 f64。Int/Float → f64；其余设 TypeError 返回 0.0。NULL 安全。
#[no_mangle]
pub extern "C" fn msToFloat(vm: *mut MsVM, val: *mut MsValue) -> f64 {
    if val.is_null() {
        return 0.0;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    match unsafe { &(*val).inner } {
        Object::Int(n) => *n as f64,
        Object::Float(f) => *f,
        _ => {
            set_type_error(vm, "int or float", unsafe { &(*val).inner });
            0.0
        }
    }
}

/// 按 truthy 规则转换为 MS_TRUE/MS_FALSE。不设异常。NULL 安全。
#[no_mangle]
pub extern "C" fn msToBool(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    if unsafe { &(*val).inner }.is_truthy() {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

/// 返回内部字符串指针（借用引用）。所有类型格式化到 thread_local 缓冲区
/// （保证 null 终止符）。NULL 安全。
#[no_mangle]
pub extern "C" fn msToString(vm: *mut MsVM, val: *mut MsValue) -> *const c_char {
    if val.is_null() {
        return std::ptr::null();
    }
    let _ = vm;
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    format_to_thread_local(obj)
}

/// 返回字符串副本（CString::into_raw），C 侧 free() 释放。NULL 安全。
#[no_mangle]
pub extern "C" fn msToStringCopy(vm: *mut MsVM, val: *mut MsValue) -> *mut c_char {
    let ptr = msToString(vm, val);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: ptr 来自 msToString，指向有效 C 字符串。
    let cstr = unsafe { CStr::from_ptr(ptr) };
    let owned = CString::new(cstr.to_bytes()).unwrap_or_default();
    owned.into_raw()
}

// ---------------------------------------------------------------------------
// 辅助函数 — parse_int_string
// ---------------------------------------------------------------------------

/// 解析字符串为整数，支持十进制/十六进制(0x)/二进制(0b)/八进制(0o)前缀。
/// 解析失败返回 None。
fn parse_int_string(s: &str) -> Option<Object> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 分离符号
    let (sign, body) = if let Some(rest) = trimmed.strip_prefix('-') {
        (-1i64, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (1i64, rest)
    } else {
        (1i64, trimmed)
    };
    // 检测进制前缀
    let (radix, digits) = if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        (16u32, r)
    } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        (2u32, r)
    } else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        (8u32, r)
    } else {
        // 十进制：用原始 trimmed（含符号）
        return trimmed.parse::<i64>().ok().map(Object::Int);
    };
    i64::from_str_radix(digits, radix)
        .ok()
        .map(|n| Object::Int(sign * n))
}

// ---------------------------------------------------------------------------
// 显式类型转换 — msConvertInt/Float/Str/Bool/List
// ---------------------------------------------------------------------------

/// 对应 mslang int()。Bool→0/1；Int→自身；Float→截断；String→解析。
/// 转换失败设 TypeError 返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msConvertInt(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    let result = match obj {
        Object::Bool(b) => Some(Object::Int(if *b { 1 } else { 0 })),
        Object::Int(_) => Some(obj.clone()),
        Object::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                None
            } else {
                Some(Object::Int(*f as i64))
            }
        }
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                let s = unsafe { read_str(*ptr) };
                parse_int_string(s)
            } else {
                None
            }
        }
        _ => None,
    };
    match result {
        Some(o) => Box::into_raw(Box::new(MsValue { inner: o })),
        None => {
            set_type_error(vm, "convertible type", obj);
            std::ptr::null_mut()
        }
    }
}

/// 对应 mslang float()。Bool→0.0/1.0；Int→f64；Float→自身；String→解析。
/// 转换失败设 TypeError 返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msConvertFloat(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    let result = match obj {
        Object::Bool(b) => Some(Object::Float(if *b { 1.0 } else { 0.0 })),
        Object::Int(n) => Some(Object::Float(*n as f64)),
        Object::Float(_) => Some(obj.clone()),
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                let s = unsafe { read_str(*ptr) };
                s.trim().parse::<f64>().ok().map(Object::Float)
            } else {
                None
            }
        }
        _ => None,
    };
    match result {
        Some(o) => Box::into_raw(Box::new(MsValue { inner: o })),
        None => {
            set_type_error(vm, "convertible type", obj);
            std::ptr::null_mut()
        }
    }
}

/// 对应 mslang str()。所有类型用 Display 格式化；String→自身。
/// NULL 安全。
#[no_mangle]
pub extern "C" fn msConvertStr(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return std::ptr::null_mut();
    }
    let _ = vm;
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    match obj {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // String → 自身（clone）
                Some(obj.clone())
            } else {
                let formatted = format!("{}", obj);
                Some(alloc_string(&formatted))
            }
        }
        _ => {
            let formatted = format!("{}", obj);
            Some(alloc_string(&formatted))
        }
    }
    .map(|o| Box::into_raw(Box::new(MsValue { inner: o })))
    .unwrap_or(std::ptr::null_mut())
}

/// 对应 mslang bool()。按 truthy 规则返回 MS_TRUE_VAL/MS_FALSE_VAL。
/// NULL 安全。
#[no_mangle]
pub extern "C" fn msConvertBool(val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return msBoolVal(MS_FALSE);
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    if unsafe { &(*val).inner }.is_truthy() {
        msBoolVal(MS_TRUE)
    } else {
        msBoolVal(MS_FALSE)
    }
}

/// 对应 mslang list()。String→字符列表；List→浅拷贝；Tuple/Set→转换；
/// Dict→key 列表。其余报错。NULL 安全。
#[no_mangle]
pub extern "C" fn msConvertList(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    let result = match obj {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // String → 字符列表
                // SAFETY: type_tag 为 STRING。
                let s = unsafe { read_str(*ptr) };
                let items: Vec<Object> = s
                    .chars()
                    .map(|ch| alloc_string(ch.to_string().as_str()))
                    .collect();
                Some(alloc_list(items))
            } else if tag == TypeTag::LIST as u8 {
                // List → 浅拷贝
                // SAFETY: type_tag 为 LIST。
                let items = unsafe { read_list(*ptr) }.clone();
                Some(alloc_list(items))
            } else if tag == TypeTag::TUPLE as u8 {
                // Tuple → 新 List
                // SAFETY: type_tag 为 TUPLE。
                let items = unsafe { read_tuple(*ptr) }.clone();
                Some(alloc_list(items))
            } else if tag == TypeTag::SET as u8 {
                // Set → 新 List
                // SAFETY: type_tag 为 SET。
                let items: Vec<Object> = unsafe { read_set(*ptr) }.iter().cloned().collect();
                Some(alloc_list(items))
            } else if tag == TypeTag::DICT as u8 {
                // Dict → key 列表
                // SAFETY: type_tag 为 DICT。
                let keys = unsafe { read_dict(*ptr) }.keys();
                Some(alloc_list(keys.into_iter().cloned().collect()))
            } else {
                None
            }
        }
        _ => None,
    };
    match result {
        Some(o) => Box::into_raw(Box::new(MsValue { inner: o })),
        None => {
            set_type_error(vm, "convertible type", obj);
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// 比较 — msEq / msLt / msLe / msGt / msGe / msIs / msHash
// ---------------------------------------------------------------------------

/// 值相等比较（==）。返回 MS_TRUE/MS_FALSE。NULL 安全。
#[no_mangle]
pub extern "C" fn msEq(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    let _ = vm;
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    if obj_a == obj_b {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

/// 手动实现 Object 顺序比较（Object 未实现 PartialOrd）。
/// 按 02-types.md:280-307 规则：Int/Float 数值比较、String 字典序、
/// 其余跨类型返回 None。
fn compare_objects(a: &Object, b: &Object) -> Option<Ordering> {
    match (a, b) {
        (Object::Int(x), Object::Int(y)) => Some(x.cmp(y)),
        (Object::Float(x), Object::Float(y)) => x.partial_cmp(y),
        (Object::Int(x), Object::Float(y)) => (*x as f64).partial_cmp(y),
        (Object::Float(x), Object::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Object::Bool(x), Object::Bool(y)) => Some(x.cmp(y)),
        (Object::Nil, Object::Nil) => Some(Ordering::Equal),
        (Object::Ref(pa), Object::Ref(pb)) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let ta = unsafe { (**pa).type_tag };
            let tb = unsafe { (**pb).type_tag };
            if ta == TypeTag::STRING as u8 && tb == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                let sa = unsafe { read_str(*pa) };
                let sb = unsafe { read_str(*pb) };
                Some(sa.cmp(sb))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 顺序比较 a < b。类型不兼容设 TypeError。NULL 安全。
#[no_mangle]
pub extern "C" fn msLt(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match compare_objects(obj_a, obj_b) {
        Some(Ordering::Less) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}

/// 顺序比较 a <= b。类型不兼容设 TypeError。NULL 安全。
#[no_mangle]
pub extern "C" fn msLe(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match compare_objects(obj_a, obj_b) {
        Some(Ordering::Less | Ordering::Equal) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}

/// 顺序比较 a > b。类型不兼容设 TypeError。NULL 安全。
#[no_mangle]
pub extern "C" fn msGt(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match compare_objects(obj_a, obj_b) {
        Some(Ordering::Greater) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}

/// 顺序比较 a >= b。类型不兼容设 TypeError。NULL 安全。
#[no_mangle]
pub extern "C" fn msGe(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match compare_objects(obj_a, obj_b) {
        Some(Ordering::Greater | Ordering::Equal) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}

/// 身份比较（is）。对 Ref 类型比较指针；内联值返回 MS_FALSE
/// （签名无 vm 参数，无法设 TypeError）。NULL 安全。
#[no_mangle]
pub extern "C" fn msIs(a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match (obj_a, obj_b) {
        (Object::Ref(p1), Object::Ref(p2)) => {
            if p1 == p2 {
                MS_TRUE
            } else {
                MS_FALSE
            }
        }
        // 内联值：02-types.md 规定 is 应抛 TypeError。
        // 但 msIs 签名无 vm 参数，无法设异常。暂返回 MS_FALSE。
        _ => MS_FALSE,
    }
}

/// 检查 Object 是否可哈希（不调用 .hash()，避免 panic）。
/// Tuple 递归检查所有元素。
fn is_hashable(obj: &Object) -> bool {
    match obj {
        Object::Nil | Object::Bool(_) | Object::Int(_) => true,
        Object::Float(f) => !f.is_nan(),
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                true
            } else if tag == TypeTag::TUPLE as u8 {
                // 递归检查 tuple 元素
                // SAFETY: type_tag 为 TUPLE。
                let items = unsafe { read_tuple(*ptr) };
                items.iter().all(is_hashable)
            } else {
                false
            }
        }
    }
}

/// 返回哈希值。不可哈希类型设异常返回 0。NULL 安全。
#[no_mangle]
pub extern "C" fn msHash(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return 0;
    }
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    if !is_hashable(obj) {
        set_type_error(vm, "hashable type", obj);
        return 0;
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    obj.hash(&mut hasher);
    hasher.finish() as i64
}

// ---------------------------------------------------------------------------
// 通用属性/下标访问 — msGetAttr / msSetAttr / msGetItem / msSetItem
// ---------------------------------------------------------------------------

/// 获取命名属性。Deferred（Task 73）：Instance/Module/Class。
/// 其余类型设 TypeError 返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msGetAttr(
    vm: *mut MsVM,
    obj: *mut MsValue,
    attr: *const c_char,
) -> *mut MsValue {
    if obj.is_null() || attr.is_null() {
        return std::ptr::null_mut();
    }
    let attr_str = unsafe { CStr::from_ptr(attr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: obj 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    match inner {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::INSTANCE as u8 {
                // TODO(task 73): get_instance_attr(vm, ptr, &attr_str)
                set_type_error(vm, "instance attribute access (task 73)", inner);
            } else if tag == TypeTag::MODULE as u8 {
                // TODO(task 45/73): get_module_export(vm, ptr, &attr_str)
                set_type_error(vm, "module export access (task 73)", inner);
            } else if tag == TypeTag::CLASS as u8 {
                // TODO(task 73): get_class_member(vm, ptr, &attr_str)
                set_type_error(vm, "class member access (task 73)", inner);
            } else {
                set_type_error(vm, "object with attributes", inner);
            }
        }
        _ => {
            set_type_error(vm, "object with attributes", inner);
        }
    }
    let _ = attr_str;
    std::ptr::null_mut()
}

/// 设置命名属性。Deferred（Task 73）：仅 Instance 和可变对象支持。
/// 其余类型设 TypeError 返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetAttr(
    vm: *mut MsVM,
    obj: *mut MsValue,
    attr: *const c_char,
    val: *mut MsValue,
) -> MsStatus {
    if obj.is_null() || attr.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    // SAFETY: obj 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    match inner {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::INSTANCE as u8 {
                // TODO(task 73): set_instance_attr(vm, ptr, attr_str, val)
                set_type_error(vm, "instance attribute set (task 73)", inner);
            } else {
                set_type_error(vm, "mutable object with attributes", inner);
            }
        }
        _ => {
            set_type_error(vm, "mutable object with attributes", inner);
        }
    }
    MsStatus::MS_ERROR
}

/// 获取下标。Deferred（Task 69）：List/Dict/String/Tuple。
/// 其余类型设 TypeError 返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msGetItem(
    vm: *mut MsVM,
    obj: *mut MsValue,
    key: *mut MsValue,
) -> *mut MsValue {
    if obj.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: obj/key 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    let _key_obj = unsafe { &(*key).inner };
    match inner {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                // TODO(task 69): list_get(vm, ptr, key_obj)
                set_type_error(vm, "list indexing (task 69)", inner);
            } else if tag == TypeTag::DICT as u8 {
                // TODO(task 69): dict_get(vm, ptr, key_obj)
                set_type_error(vm, "dict indexing (task 69)", inner);
            } else if tag == TypeTag::STRING as u8 {
                // TODO(task 69): string_get(vm, ptr, key_obj)
                set_type_error(vm, "string indexing (task 69)", inner);
            } else if tag == TypeTag::TUPLE as u8 {
                // TODO(task 69): tuple_get(vm, ptr, key_obj)
                set_type_error(vm, "tuple indexing (task 69)", inner);
            } else {
                set_type_error(vm, "subscriptable type", inner);
            }
        }
        _ => {
            set_type_error(vm, "subscriptable type", inner);
        }
    }
    std::ptr::null_mut()
}

/// 设置下标。Deferred（Task 69）：仅 List 和 Dict 支持。
/// 其余类型设 TypeError 返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetItem(
    vm: *mut MsVM,
    obj: *mut MsValue,
    key: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if obj.is_null() || key.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    // SAFETY: obj 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    match inner {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                // TODO(task 69): list_set(vm, ptr, key_obj, val)
                set_type_error(vm, "list set (task 69)", inner);
            } else if tag == TypeTag::DICT as u8 {
                // TODO(task 69): dict_set(vm, ptr, key_obj, val)
                set_type_error(vm, "dict set (task 69)", inner);
            } else {
                set_type_error(vm, "mutable subscriptable type", inner);
            }
        }
        _ => {
            set_type_error(vm, "mutable subscriptable type", inner);
        }
    }
    MsStatus::MS_ERROR
}

// ---------------------------------------------------------------------------
// msLen / msRepr
// ---------------------------------------------------------------------------

/// 通用长度。String/List/Dict/Tuple/Set 返回元素数；其余设异常返回 -1。
/// NULL 安全。
#[no_mangle]
pub extern "C" fn msLen(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return -1;
    }
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    match obj {
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                unsafe { read_str(*ptr) }.len() as i64
            } else if tag == TypeTag::LIST as u8 {
                // SAFETY: type_tag 为 LIST。
                unsafe { read_list(*ptr) }.len() as i64
            } else if tag == TypeTag::DICT as u8 {
                // SAFETY: type_tag 为 DICT。
                unsafe { read_dict(*ptr) }.len() as i64
            } else if tag == TypeTag::TUPLE as u8 {
                // SAFETY: type_tag 为 TUPLE。
                unsafe { read_tuple(*ptr) }.len() as i64
            } else if tag == TypeTag::SET as u8 {
                // SAFETY: type_tag 为 SET。
                unsafe { read_set(*ptr) }.len() as i64
            } else {
                set_type_error(vm, "type with length", obj);
                -1
            }
        }
        _ => {
            set_type_error(vm, "type with length", obj);
            -1
        }
    }
}

/// 返回 repr 字符串。String 带引号，对象显示类型信息。NULL 安全。
#[no_mangle]
pub extern "C" fn msRepr(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return msNil();
    }
    let _ = vm;
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    let repr_str = repr_object(obj);
    let new_obj = alloc_string(&repr_str);
    Box::into_raw(Box::new(MsValue { inner: new_obj }))
}

/// 生成 Object 的 repr 字符串。String 带引号和转义。
fn repr_object(obj: &Object) -> String {
    match obj {
        Object::Nil => "nil".to_string(),
        Object::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Object::Int(n) => format!("{}", n),
        Object::Float(f) => format!("{}", f),
        Object::Ref(ptr) => {
            // SAFETY: Ref 指针由 alloc_* 分配。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                let s = unsafe { read_str(*ptr) };
                format!("{:?}", s) // 带引号和转义
            } else {
                // List/Dict/Tuple/Set/Instance 等使用 Display 作为 fallback
                format!("{}", obj)
            }
        }
    }
}

// ===========================================================================
// Task 69 — 集合操作（List/Dict/Tuple/Set + 字符串操作 + 迭代器）
// ===========================================================================

use crate::capi::vm::lock_vm;

/// 将 C 索引（支持负数）解析为 usize。越界返回 None。
fn resolve_index(index: c_int, len: isize) -> Option<usize> {
    let idx = if index < 0 {
        len + index as isize
    } else {
        index as isize
    };
    if idx < 0 || idx >= len {
        None
    } else {
        Some(idx as usize)
    }
}

/// 计算 slice 的 (start, end) 边界。正/负 step 分别处理 clamp。
fn compute_slice_bounds(start: c_int, end: c_int, step: isize, len: isize) -> (isize, isize) {
    let s = if start < 0 {
        (len + start as isize).max(if step > 0 { 0 } else { -1 })
    } else {
        (start as isize).min(if step > 0 { len } else { len - 1 })
    };
    let s = s.max(0);
    let e = if end < 0 {
        (len + end as isize).max(if step > 0 { 0 } else { -1 })
    } else {
        (end as isize).min(len)
    };
    (s, e)
}

// ---------------------------------------------------------------------------
// 字符串操作
// ---------------------------------------------------------------------------

/// 返回字符串 UTF-8 字节长度。NULL 安全。
#[no_mangle]
pub extern "C" fn msStringLen(vm: *mut MsVM, str_val: *mut MsValue) -> usize {
    if vm.is_null() || str_val.is_null() {
        return 0;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*str_val).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.len()
        }
        _ => 0,
    }
}

/// 返回内部 UTF-8 数据指针（借用引用，无需 free）。NULL 安全。
#[no_mangle]
pub extern "C" fn msStringData(vm: *mut MsVM, str_val: *mut MsValue) -> *const c_char {
    if vm.is_null() || str_val.is_null() {
        return std::ptr::null();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*str_val).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.as_ptr() as *const c_char
        }
        _ => std::ptr::null(),
    }
}

/// 连接两个字符串，返回新字符串。NULL 安全。
#[no_mangle]
pub extern "C" fn msStringConcat(
    vm: *mut MsVM,
    a: *mut MsValue,
    b: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match (unsafe { &(*a).inner }, unsafe { &(*b).inner }) {
        (Object::Ref(pa), Object::Ref(pb))
            if unsafe { (**pa).type_tag } == TypeTag::STRING as u8
                && unsafe { (**pb).type_tag } == TypeTag::STRING as u8 =>
        {
            let sa = unsafe { read_str(*pa) };
            let sb = unsafe { read_str(*pb) };
            let combined = format!("{}{}", sa, sb);
            let obj = alloc_string(&combined);
            Box::into_raw(Box::new(MsValue { inner: obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

/// 字符串切片，支持负索引。按字节切片。NULL 安全。
#[no_mangle]
pub extern "C" fn msStringSlice(
    vm: *mut MsVM,
    str_val: *mut MsValue,
    start: c_int,
    end: c_int,
) -> *mut MsValue {
    if vm.is_null() || str_val.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*str_val).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            let s = unsafe { read_str(*ptr) };
            let len = s.len() as isize;
            let lo = if start < 0 {
                (len + start as isize).max(0)
            } else {
                (start as isize).min(len)
            };
            let hi = if end < 0 {
                (len + end as isize).max(0)
            } else {
                (end as isize).min(len)
            };
            let lo_u = lo as usize;
            let hi_u = hi.max(lo) as usize;
            let sliced = s.get(lo_u..hi_u).unwrap_or("");
            let obj = alloc_string(sliced);
            Box::into_raw(Box::new(MsValue { inner: obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// List 操作
// ---------------------------------------------------------------------------

/// 返回 List 长度。非 List 返回 -1。NULL 安全。
#[no_mangle]
pub extern "C" fn msListLen(vm: *mut MsVM, list: *mut MsValue) -> c_int {
    if vm.is_null() || list.is_null() {
        return -1;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            unsafe { read_list(*ptr) }.len() as c_int
        }
        _ => -1,
    }
}

/// 获取 List 元素（支持负索引）。越界设异常返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msListGet(
    vm: *mut MsVM,
    list: *mut MsValue,
    index: c_int,
) -> *mut MsValue {
    if vm.is_null() || list.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            let items = unsafe { read_list(*ptr) };
            let len = items.len() as isize;
            match resolve_index(index, len) {
                Some(i) => Box::into_raw(Box::new(MsValue { inner: items[i].clone() })),
                None => {
                    set_type_error(vm, "valid index", unsafe { &(*list).inner });
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "list", unsafe { &(*list).inner });
            std::ptr::null_mut()
        }
    }
}

/// 原地修改指定位置（支持负索引）。越界设异常返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msListSet(
    vm: *mut MsVM,
    list: *mut MsValue,
    index: c_int,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || list.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let new_val = unsafe { (*val).inner.clone() };
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            let items = unsafe { read_list(*ptr) };
            let len = items.len() as isize;
            match resolve_index(index, len) {
                Some(i) => {
                    items[i] = new_val;
                    MsStatus::MS_OK
                }
                None => {
                    set_type_error(vm, "valid index", unsafe { &(*list).inner });
                    MsStatus::MS_ERROR
                }
            }
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 尾部追加元素，返回 MS_OK。NULL 安全。
#[no_mangle]
pub extern "C" fn msListPush(
    vm: *mut MsVM,
    list: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || list.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let new_val = unsafe { (*val).inner.clone() };
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            unsafe { read_list(*ptr) }.push(new_val);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 弹出末尾元素并返回。空列表设异常返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msListPop(vm: *mut MsVM, list: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || list.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            let items = unsafe { read_list(*ptr) };
            match items.pop() {
                Some(val) => Box::into_raw(Box::new(MsValue { inner: val })),
                None => {
                    set_type_error(vm, "non-empty list", unsafe { &(*list).inner });
                    std::ptr::null_mut()
                }
            }
        }
        _ => std::ptr::null_mut(),
    }
}

/// 在指定位置插入元素（支持负索引）。NULL 安全。
#[no_mangle]
pub extern "C" fn msListInsert(
    vm: *mut MsVM,
    list: *mut MsValue,
    index: c_int,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || list.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let new_val = unsafe { (*val).inner.clone() };
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            let items = unsafe { read_list(*ptr) };
            let len = items.len() as isize;
            let idx = if index < 0 {
                (len + index as isize).max(0) as usize
            } else {
                (index as usize).min(len as usize)
            };
            items.insert(idx, new_val);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 包含则返回 MS_TRUE，否则 MS_FALSE。NULL 安全。
#[no_mangle]
pub extern "C" fn msListContains(
    vm: *mut MsVM,
    list: *mut MsValue,
    val: *mut MsValue,
) -> c_int {
    if vm.is_null() || list.is_null() || val.is_null() {
        return MS_FALSE;
    }
    let _guard = lock_vm(vm);
    let target = unsafe { (*val).inner.clone() };
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            if unsafe { read_list(*ptr) }.contains(&target) {
                MS_TRUE
            } else {
                MS_FALSE
            }
        }
        _ => MS_FALSE,
    }
}

/// 创建新列表切片，支持负索引和 step。step=0 设异常返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msListSlice(
    vm: *mut MsVM,
    list: *mut MsValue,
    start: c_int,
    end: c_int,
    step: c_int,
) -> *mut MsValue {
    if vm.is_null() || list.is_null() {
        return std::ptr::null_mut();
    }
    if step == 0 {
        let _guard = lock_vm(vm);
        set_type_error(vm, "non-zero step", unsafe { &(*list).inner });
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            let items = unsafe { read_list(*ptr) };
            let len = items.len() as isize;
            let step_is = step as isize;
            let (s_idx, e_idx) = compute_slice_bounds(start, end, step_is, len);
            let mut result = Vec::new();
            let mut i = s_idx;
            if step_is > 0 {
                while i < e_idx && i >= 0 {
                    result.push(items[i as usize].clone());
                    i += step_is;
                }
            } else {
                while i > e_idx && i >= 0 && i < len {
                    result.push(items[i as usize].clone());
                    i += step_is;
                }
            }
            let result_obj = alloc_list(result);
            Box::into_raw(Box::new(MsValue { inner: result_obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// Dict 操作
// ---------------------------------------------------------------------------

/// 返回 Dict 长度。非 Dict 返回 -1。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictLen(vm: *mut MsVM, dict: *mut MsValue) -> c_int {
    if vm.is_null() || dict.is_null() {
        return -1;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            unsafe { read_dict(*ptr) }.len() as c_int
        }
        _ => -1,
    }
}

/// 获取键对应的值。键不存在返回 NULL（不设异常）。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictGet(
    vm: *mut MsVM,
    dict: *mut MsValue,
    key: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || dict.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    let key_obj = unsafe { (*key).inner.clone() };
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            match map.get(&key_obj) {
                Some(val) => Box::into_raw(Box::new(MsValue { inner: val.clone() })),
                None => std::ptr::null_mut(),
            }
        }
        _ => std::ptr::null_mut(),
    }
}

/// 获取键对应的值，不存在时返回 defaultVal。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictGetDefault(
    vm: *mut MsVM,
    dict: *mut MsValue,
    key: *mut MsValue,
    default_val: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || dict.is_null() || key.is_null() {
        return default_val;
    }
    let _guard = lock_vm(vm);
    let key_obj = unsafe { (*key).inner.clone() };
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            match map.get(&key_obj) {
                Some(val) => Box::into_raw(Box::new(MsValue { inner: val.clone() })),
                None => default_val,
            }
        }
        _ => default_val,
    }
}

/// 设置键值对（存在则覆盖）。键不可哈希设异常返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictSet(
    vm: *mut MsVM,
    dict: *mut MsValue,
    key: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || dict.is_null() || key.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let key_obj = unsafe { (*key).inner.clone() };
    let val_obj = unsafe { (*val).inner.clone() };
    if !is_hashable(&key_obj) {
        set_type_error(vm, "hashable key", unsafe { &(*key).inner });
        return MsStatus::MS_ERROR;
    }
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            unsafe { read_dict(*ptr) }.insert(key_obj, val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 删除键值对。键不存在设异常返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictRemove(
    vm: *mut MsVM,
    dict: *mut MsValue,
    key: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || dict.is_null() || key.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let key_obj = unsafe { (*key).inner.clone() };
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            if map.remove(&key_obj).is_some() {
                MsStatus::MS_OK
            } else {
                set_type_error(vm, "existing key", unsafe { &(*key).inner });
                MsStatus::MS_ERROR
            }
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 包含则返回 MS_TRUE，否则 MS_FALSE。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictContains(
    vm: *mut MsVM,
    dict: *mut MsValue,
    key: *mut MsValue,
) -> c_int {
    if vm.is_null() || dict.is_null() || key.is_null() {
        return MS_FALSE;
    }
    let _guard = lock_vm(vm);
    let key_obj = unsafe { (*key).inner.clone() };
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            if unsafe { read_dict(*ptr) }.get(&key_obj).is_some() {
                MS_TRUE
            } else {
                MS_FALSE
            }
        }
        _ => MS_FALSE,
    }
}

/// 返回新 List，包含所有键（保持插入顺序）。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictKeys(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || dict.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            let keys: Vec<Object> = map.keys().into_iter().cloned().collect();
            let obj = alloc_list(keys);
            Box::into_raw(Box::new(MsValue { inner: obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

/// 返回新 List，包含所有值（保持插入顺序）。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictValues(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || dict.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            let vals: Vec<Object> = map.items().iter().map(|(_, v)| (*v).clone()).collect();
            let obj = alloc_list(vals);
            Box::into_raw(Box::new(MsValue { inner: obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

/// 返回新 List，每个元素为二元 Tuple (key, value)。NULL 安全。
#[no_mangle]
pub extern "C" fn msDictItems(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || dict.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            let pairs: Vec<Object> = map
                .items()
                .iter()
                .map(|(k, v)| alloc_tuple(vec![(**k).clone(), (*v).clone()]))
                .collect();
            let obj = alloc_list(pairs);
            Box::into_raw(Box::new(MsValue { inner: obj }))
        }
        _ => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// Tuple 操作
// ---------------------------------------------------------------------------

/// 返回 Tuple 长度。非 Tuple 返回 -1。NULL 安全。
#[no_mangle]
pub extern "C" fn msTupleLen(vm: *mut MsVM, tup: *mut MsValue) -> c_int {
    if vm.is_null() || tup.is_null() {
        return -1;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*tup).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            unsafe { read_tuple(*ptr) }.len() as c_int
        }
        _ => -1,
    }
}

/// 获取 Tuple 元素（支持负索引）。越界设异常返回 NULL。NULL 安全。
#[no_mangle]
pub extern "C" fn msTupleGet(
    vm: *mut MsVM,
    tup: *mut MsValue,
    index: c_int,
) -> *mut MsValue {
    if vm.is_null() || tup.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*tup).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            let items = unsafe { read_tuple(*ptr) };
            let len = items.len() as isize;
            match resolve_index(index, len) {
                Some(i) => Box::into_raw(Box::new(MsValue { inner: items[i].clone() })),
                None => {
                    set_type_error(vm, "valid index", unsafe { &(*tup).inner });
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "tuple", unsafe { &(*tup).inner });
            std::ptr::null_mut()
        }
    }
}

/// 解包 Tuple，通过 malloc 分配 MsValue* 数组。调用方用 msTupleUnpackFree 释放。
#[no_mangle]
pub extern "C" fn msTupleUnpack(
    vm: *mut MsVM,
    tup: *mut MsValue,
    items_out: *mut *mut *mut MsValue,
    count_out: *mut c_int,
) -> MsStatus {
    if vm.is_null() || tup.is_null() || items_out.is_null() || count_out.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*tup).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            let elements = unsafe { read_tuple(*ptr) };
            let n = elements.len();
            if n == 0 {
                unsafe {
                    *items_out = std::ptr::null_mut();
                    *count_out = 0;
                }
                return MsStatus::MS_OK;
            }
            let layout = match std::alloc::Layout::array::<*mut MsValue>(n) {
                Ok(l) => l,
                Err(_) => return MsStatus::MS_ERROR,
            };
            let arr = unsafe { std::alloc::alloc(layout) as *mut *mut MsValue };
            if arr.is_null() {
                return MsStatus::MS_ERROR;
            }
            for (i, elem) in elements.iter().enumerate() {
                let ms_val = Box::into_raw(Box::new(MsValue { inner: elem.clone() }));
                unsafe {
                    *arr.add(i) = ms_val;
                }
            }
            unsafe {
                *items_out = arr;
                *count_out = n as c_int;
            }
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 释放 msTupleUnpack 分配的数组（逐个释放 MsValue + 数组本身）。
#[no_mangle]
pub extern "C" fn msTupleUnpackFree(items: *mut *mut MsValue, count: c_int) {
    if items.is_null() || count <= 0 {
        return;
    }
    let n = count as usize;
    for i in 0..n {
        let val = unsafe { *items.add(i) };
        if !val.is_null() {
            unsafe {
                let _ = Box::from_raw(val);
            }
        }
    }
    let layout = std::alloc::Layout::array::<*mut MsValue>(n).unwrap();
    unsafe {
        std::alloc::dealloc(items as *mut u8, layout);
    }
}

// ---------------------------------------------------------------------------
// Set 操作
// ---------------------------------------------------------------------------

/// 返回 Set 长度。非 Set 返回 -1。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetLen(vm: *mut MsVM, set: *mut MsValue) -> c_int {
    if vm.is_null() || set.is_null() {
        return -1;
    }
    let _guard = lock_vm(vm);
    match unsafe { &(*set).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
            unsafe { read_set(*ptr) }.len() as c_int
        }
        _ => -1,
    }
}

/// 添加元素（已存在则无操作）。不可哈希设异常返回 MS_ERROR。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetAdd(
    vm: *mut MsVM,
    set: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || set.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let val_obj = unsafe { (*val).inner.clone() };
    if !is_hashable(&val_obj) {
        set_type_error(vm, "hashable element", unsafe { &(*val).inner });
        return MsStatus::MS_ERROR;
    }
    match unsafe { &(*set).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
            unsafe { read_set(*ptr) }.insert(val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 删除元素。不存在时无异常、无错误（返回 MS_OK）。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetRemove(
    vm: *mut MsVM,
    set: *mut MsValue,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || set.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let _guard = lock_vm(vm);
    let val_obj = unsafe { (*val).inner.clone() };
    if !is_hashable(&val_obj) {
        return MsStatus::MS_OK;
    }
    match unsafe { &(*set).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
            unsafe { read_set(*ptr) }.remove(&val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

/// 包含则返回 MS_TRUE，否则 MS_FALSE。NULL 安全。
#[no_mangle]
pub extern "C" fn msSetContains(
    vm: *mut MsVM,
    set: *mut MsValue,
    val: *mut MsValue,
) -> c_int {
    if vm.is_null() || set.is_null() || val.is_null() {
        return MS_FALSE;
    }
    let _guard = lock_vm(vm);
    let val_obj = unsafe { (*val).inner.clone() };
    if !is_hashable(&val_obj) {
        return MS_FALSE;
    }
    match unsafe { &(*set).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
            if unsafe { read_set(*ptr) }.contains(&val_obj) {
                MS_TRUE
            } else {
                MS_FALSE
            }
        }
        _ => MS_FALSE,
    }
}

// ---------------------------------------------------------------------------
// 迭代器（Deferred — 迭代器协议内部结构尚未实现）
// ---------------------------------------------------------------------------

/// 调用可迭代对象的 __iter__ 协议。当前为占位实现，设 TypeError 返回 NULL。
#[no_mangle]
pub extern "C" fn msIter(vm: *mut MsVM, iterable: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || iterable.is_null() {
        return std::ptr::null_mut();
    }
    let _guard = lock_vm(vm);
    set_type_error(
        vm,
        "iterable (iterator protocol not yet implemented)",
        unsafe { &(*iterable).inner },
    );
    std::ptr::null_mut()
}

/// 调用迭代器的 __next__。当前为占位实现，返回 MS_ERROR。
#[no_mangle]
pub extern "C" fn msNext(
    vm: *mut MsVM,
    iterator: *mut MsValue,
    out: *mut *mut MsValue,
) -> MsStatus {
    let _ = (vm, iterator, out);
    MsStatus::MS_ERROR
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

#[cfg(test)]
mod tests_convert {
    use super::*;
    use crate::capi::vm::{msVmFree, msVmNew};
    use std::ffi::CString;
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
    fn test_to_int_float_bool() {
        let vm = msVmNew();

        let int_val = msInt(42);
        assert_eq!(msToInt(vm, int_val), 42);
        assert_eq!(msToFloat(vm, int_val), 42.0);
        assert_eq!(msToBool(int_val), MS_TRUE);

        let float_val = msFloat(3.7);
        assert_eq!(msToInt(vm, float_val), 3);
        assert_eq!(msToFloat(vm, float_val), 3.7);

        let zero_val = msInt(0);
        assert_eq!(msToBool(zero_val), MS_FALSE);

        let nil_val = msNil();
        assert_eq!(msToBool(nil_val), MS_FALSE);
        assert_eq!(msToInt(vm, nil_val), 0);

        // NULL safety
        assert_eq!(msToInt(vm, ptr::null_mut()), 0);
        assert_eq!(msToFloat(vm, ptr::null_mut()), 0.0);
        assert_eq!(msToBool(ptr::null_mut()), MS_FALSE);

        free_value(int_val);
        free_value(float_val);
        free_value(zero_val);
        free_value(nil_val);
        msVmFree(vm);
    }

    #[test]
    fn test_to_string_and_copy() {
        let vm = msVmNew();

        let int_val = msInt(42);
        let s = msToString(vm, int_val);
        let cstr = unsafe { CStr::from_ptr(s) };
        assert_eq!(cstr.to_str().unwrap(), "42");

        let copy = msToStringCopy(vm, int_val);
        assert!(!copy.is_null());
        let cstr_copy = unsafe { CStr::from_ptr(copy) };
        assert_eq!(cstr_copy.to_str().unwrap(), "42");
        // 释放 CString::into_raw 分配的副本
        // SAFETY: copy 由 CString::into_raw 返回。
        unsafe {
            let _ = CString::from_raw(copy);
        }

        // NULL safety
        assert!(msToString(vm, ptr::null_mut()).is_null());

        free_value(int_val);
        msVmFree(vm);
    }

    #[test]
    fn test_equality_comparisons() {
        let vm = msVmNew();

        let a = msInt(1);
        let b = msInt(1);
        let c = msInt(2);

        assert_eq!(msEq(vm, a, b), MS_TRUE);
        assert_eq!(msEq(vm, a, c), MS_FALSE);

        let nil_a = msNil();
        let nil_b = msNil();
        assert_eq!(msEq(vm, nil_a, nil_b), MS_TRUE);

        free_value(a);
        free_value(b);
        free_value(c);
        free_value(nil_a);
        free_value(nil_b);
        msVmFree(vm);
    }

    #[test]
    fn test_ordering_comparisons() {
        let vm = msVmNew();

        let a = msInt(1);
        let b = msInt(2);

        assert_eq!(msLt(vm, a, b), MS_TRUE);
        assert_eq!(msLe(vm, a, b), MS_TRUE);
        assert_eq!(msGt(vm, b, a), MS_TRUE);
        assert_eq!(msGe(vm, b, a), MS_TRUE);
        assert_eq!(msLt(vm, a, a), MS_FALSE);
        assert_eq!(msLe(vm, a, a), MS_TRUE);

        // Float comparison
        let f1 = msFloat(1.5);
        let f2 = msFloat(2.5);
        assert_eq!(msLt(vm, f1, f2), MS_TRUE);

        free_value(a);
        free_value(b);
        free_value(f1);
        free_value(f2);
        msVmFree(vm);
    }

    #[test]
    fn test_identity_comparison() {
        let vm = msVmNew();

        // 引用类型：is 比较指针
        let list_a = msListNew(vm);
        let list_b = msListNew(vm);
        assert_eq!(msIs(list_a, list_a), MS_TRUE);
        assert_eq!(msIs(list_a, list_b), MS_FALSE);

        // 内联值：is 返回 MS_FALSE（签名无 vm，无法设 TypeError）
        let i1 = msInt(42);
        let i2 = msInt(42);
        assert_eq!(msIs(i1, i2), MS_FALSE);

        free_value(list_a);
        free_value(list_b);
        free_value(i1);
        free_value(i2);
        msVmFree(vm);
    }

    #[test]
    fn test_hash() {
        let vm = msVmNew();

        let int_val = msInt(42);
        let h = msHash(vm, int_val);
        assert_ne!(h, 0); // 42 的哈希非零

        let str_val = msString(vm, b"hello\0".as_ptr() as *const c_char);
        let h2 = msHash(vm, str_val);
        assert_ne!(h2, 0);

        // 不可哈希类型返回 0
        let list_val = msListNew(vm);
        assert_eq!(msHash(vm, list_val), 0); // List 不可哈希

        free_value(int_val);
        free_value(str_val);
        free_value(list_val);
        msVmFree(vm);
    }

    #[test]
    fn test_convert() {
        let vm = msVmNew();

        // msConvertInt(Bool(true)) = Int(1)
        let b = msBoolVal(1);
        let converted = msConvertInt(vm, b);
        assert!(!converted.is_null());
        assert_eq!(msToInt(vm, converted), 1);
        free_value(converted);

        // msConvertStr(Int(42)) = String("42")
        let i = msInt(42);
        let str_val = msConvertStr(vm, i);
        assert!(!str_val.is_null());
        assert_eq!(msIsString(str_val), MS_TRUE);
        let s = msToString(vm, str_val);
        assert_eq!(unsafe { CStr::from_ptr(s) }.to_str().unwrap(), "42");
        free_value(str_val);

        free_value(b);
        free_value(i);
        msVmFree(vm);
    }

    #[test]
    fn test_len_and_repr() {
        let vm = msVmNew();

        // msLen for List
        let a = msInt(1);
        let b = msInt(2);
        let c = msInt(3);
        let items = [a, b, c];
        let list = msListFrom(vm, items.as_ptr(), 3);
        assert_eq!(msLen(vm, list), 3);

        // msLen for String
        let str_val = msString(vm, b"hello\0".as_ptr() as *const c_char);
        assert_eq!(msLen(vm, str_val), 5);

        // msRepr for Int
        let int_repr = msRepr(vm, msInt(42));
        let int_s = msToString(vm, int_repr);
        assert_eq!(unsafe { CStr::from_ptr(int_s) }.to_str().unwrap(), "42");

        // msRepr for String (带引号)
        let str_repr = msRepr(vm, str_val);
        let str_s = msToString(vm, str_repr);
        assert_eq!(
            unsafe { CStr::from_ptr(str_s) }.to_str().unwrap(),
            "\"hello\""
        );

        // msRepr for nil
        let nil_repr = msRepr(vm, msNil());
        let nil_s = msToString(vm, nil_repr);
        assert_eq!(unsafe { CStr::from_ptr(nil_s) }.to_str().unwrap(), "nil");

        free_value(list);
        free_value(str_val);
        free_value(int_repr);
        free_value(str_repr);
        free_value(nil_repr);
        free_value(a);
        free_value(b);
        free_value(c);
        msVmFree(vm);
    }

    // --- Deferred tests (require Task 69/73) ---
    // test_attr_access: requires msInstanceNew (Task 73)
    // test_item_access: requires msListPush/msGetItem/msSetItem (Task 69)
}

#[cfg(test)]
mod tests_collections {
    use super::*;
    use crate::capi::vm::{msVmFree, msVmNew};
    use crate::capi::types::{MsStatus, MsType};
    use std::os::raw::{c_char, c_int};
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            unsafe {
                let _ = Box::from_raw(val);
            }
        }
    }

    fn cstr(s: &str) -> *const c_char {
        Box::leak(format!("{}\0", s).into_boxed_str()).as_ptr() as *const c_char
    }

    #[test]
    fn test_list_push_pop_get_set() {
        let vm = msVmNew();
        let list = msListNew(vm);

        let v1 = msInt(10);
        let v2 = msInt(20);
        let v3 = msInt(30);

        assert_eq!(msListPush(vm, list, v1), MsStatus::MS_OK);
        assert_eq!(msListPush(vm, list, v2), MsStatus::MS_OK);
        assert_eq!(msListPush(vm, list, v3), MsStatus::MS_OK);
        assert_eq!(msListLen(vm, list), 3);

        assert_eq!(msToInt(vm, msListGet(vm, list, 0)), 10);
        assert_eq!(msToInt(vm, msListGet(vm, list, -1)), 30);

        let v99 = msInt(99);
        assert_eq!(msListSet(vm, list, 1, v99), MsStatus::MS_OK);
        assert_eq!(msToInt(vm, msListGet(vm, list, 1)), 99);

        let popped = msListPop(vm, list);
        assert_eq!(msToInt(vm, popped), 30);
        assert_eq!(msListLen(vm, list), 2);

        free_value(v1);
        free_value(v2);
        free_value(v3);
        free_value(v99);
        free_value(popped);
        msVmFree(vm);
    }

    #[test]
    fn test_list_slice() {
        let vm = msVmNew();
        let list = msListNew(vm);

        for i in 0..6 {
            msListPush(vm, list, msInt(i));
        }

        let sliced = msListSlice(vm, list, 1, 4, 1);
        assert_eq!(msListLen(vm, sliced), 3);
        assert_eq!(msToInt(vm, msListGet(vm, sliced, 0)), 1);

        let stepped = msListSlice(vm, list, 0, 6, 2);
        assert_eq!(msListLen(vm, stepped), 3);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 0)), 0);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 1)), 2);

        free_value(sliced);
        free_value(stepped);
        msVmFree(vm);
    }

    #[test]
    fn test_dict_set_get_remove() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        let key_a = msString(vm, cstr("a"));
        let val_1 = msInt(1);
        assert_eq!(msDictSet(vm, dict, key_a, val_1), MsStatus::MS_OK);

        let key_b = msString(vm, cstr("b"));
        let val_2 = msInt(2);
        assert_eq!(msDictSet(vm, dict, key_b, val_2), MsStatus::MS_OK);

        assert_eq!(msDictLen(vm, dict), 2);
        assert_eq!(msToInt(vm, msDictGet(vm, dict, key_a)), 1);
        assert_eq!(msDictContains(vm, dict, key_a), MS_TRUE);

        assert_eq!(msDictRemove(vm, dict, key_a), MsStatus::MS_OK);
        assert!(msDictGet(vm, dict, key_a).is_null());

        let default_val = msInt(42);
        let result = msDictGetDefault(vm, dict, msString(vm, cstr("z")), default_val);
        assert_eq!(msToInt(vm, result), 42);

        free_value(key_a);
        free_value(key_b);
        free_value(val_1);
        free_value(val_2);
        free_value(default_val);
        msVmFree(vm);
    }

    #[test]
    fn test_dict_keys_values_items() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        let kx = msString(vm, cstr("x"));
        let vx = msInt(10);
        msDictSet(vm, dict, kx, vx);
        let ky = msString(vm, cstr("y"));
        let vy = msInt(20);
        msDictSet(vm, dict, ky, vy);

        let keys = msDictKeys(vm, dict);
        assert_eq!(msListLen(vm, keys), 2);
        let vals = msDictValues(vm, dict);
        assert_eq!(msListLen(vm, vals), 2);

        let items = msDictItems(vm, dict);
        assert_eq!(msListLen(vm, items), 2);
        assert_eq!(msTypeof(msListGet(vm, items, 0)), MsType::Tuple);

        free_value(kx);
        free_value(ky);
        free_value(vx);
        free_value(vy);
        free_value(keys);
        free_value(vals);
        free_value(items);
        msVmFree(vm);
    }

    #[test]
    fn test_set_add_remove_contains() {
        let vm = msVmNew();
        let set = msSetNew(vm);

        assert_eq!(msSetLen(vm, set), 0);

        let v1 = msInt(1);
        let v2 = msInt(2);
        let v1b = msInt(1);
        msSetAdd(vm, set, v1);
        msSetAdd(vm, set, v2);
        msSetAdd(vm, set, v1b);
        assert_eq!(msSetLen(vm, set), 2);

        assert_eq!(msSetContains(vm, set, msInt(1)), MS_TRUE);
        assert_eq!(msSetContains(vm, set, msInt(3)), MS_FALSE);

        msSetRemove(vm, set, msInt(1));
        assert_eq!(msSetContains(vm, set, msInt(1)), MS_FALSE);

        free_value(v1);
        free_value(v2);
        free_value(v1b);
        msVmFree(vm);
    }

    #[test]
    fn test_string_concat_slice() {
        let vm = msVmNew();
        let a = msString(vm, cstr("hello"));
        let b = msString(vm, cstr(" world"));

        assert_eq!(msStringLen(vm, a), 5);

        let data = msStringData(vm, a);
        let len = msStringLen(vm, a);
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
        assert_eq!(std::str::from_utf8(bytes).unwrap(), "hello");

        let concat = msStringConcat(vm, a, b);
        let c_data = msStringData(vm, concat);
        let c_len = msStringLen(vm, concat);
        let c_bytes = unsafe { std::slice::from_raw_parts(c_data as *const u8, c_len) };
        assert_eq!(std::str::from_utf8(c_bytes).unwrap(), "hello world");

        let sliced = msStringSlice(vm, concat, 0, 5);
        let sl_data = msStringData(vm, sliced);
        let sl_len = msStringLen(vm, sliced);
        let sl_bytes = unsafe { std::slice::from_raw_parts(sl_data as *const u8, sl_len) };
        assert_eq!(std::str::from_utf8(sl_bytes).unwrap(), "hello");

        free_value(a);
        free_value(b);
        free_value(concat);
        free_value(sliced);
        msVmFree(vm);
    }

    #[test]
    fn test_tuple_unpack() {
        let vm = msVmNew();

        let elems: Vec<*mut MsValue> = vec![msInt(10), msInt(20), msInt(30)];
        let tup = msTupleFrom(vm, elems.as_ptr(), 3);

        assert_eq!(msTupleLen(vm, tup), 3);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, 0)), 10);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, -1)), 30);

        let mut items: *mut *mut MsValue = ptr::null_mut();
        let mut count: c_int = 0;
        assert_eq!(
            msTupleUnpack(vm, tup, &mut items, &mut count),
            MsStatus::MS_OK
        );
        assert_eq!(count, 3);
        assert_eq!(msToInt(vm, unsafe { *items.add(0) }), 10);

        msTupleUnpackFree(items, count);

        for e in elems {
            free_value(e);
        }
        free_value(tup);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety_collections() {
        assert_eq!(msStringLen(ptr::null_mut(), ptr::null_mut()), 0);
        assert!(msStringData(ptr::null_mut(), ptr::null_mut()).is_null());
        assert!(msStringConcat(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()).is_null());
        assert!(msStringSlice(ptr::null_mut(), ptr::null_mut(), 0, 1).is_null());
        assert_eq!(msListLen(ptr::null_mut(), ptr::null_mut()), -1);
        assert!(msListGet(ptr::null_mut(), ptr::null_mut(), 0).is_null());
        assert_eq!(msListPush(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()), MsStatus::MS_ERROR);
        assert_eq!(msDictLen(ptr::null_mut(), ptr::null_mut()), -1);
        assert_eq!(msSetLen(ptr::null_mut(), ptr::null_mut()), -1);
        assert_eq!(msTupleLen(ptr::null_mut(), ptr::null_mut()), -1);
    }
}
