//! C API — 异常处理（task 71）。
//!
//! 参照 [71-capi-error](../../docs/mslang/tasks/71-capi-error.md)。
//!
//! 实现 `error.h` 全部 API：异常查询（msErrOccurred/msErrFetch/msErrClear）、
//! 异常对象属性访问（msErrTypeName/msErrMessage/msErrTraceback/msErrCause）、
//! C 侧抛出异常（msThrow/msThrowValue/msThrowRethrow 及六个便捷函数）、
//! try/catch 模式（msTry）。

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::thread::LocalKey;

use crate::capi::call::msCall;
use crate::capi::types::{MsStatus, MsValue};
use crate::capi::value::{MS_FALSE, MS_TRUE};
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::object::{alloc_exception, alloc_string, read_exception, read_str, Object, TypeTag};

// ---------------------------------------------------------------------------
// thread_local 缓冲区（msErrTypeName/msErrMessage/msErrTraceback 借用引用）
// ---------------------------------------------------------------------------

thread_local! {
    static ERR_TYPE_BUF: RefCell<Option<CString>> = const { RefCell::new(None) };
    static ERR_MSG_BUF: RefCell<Option<CString>> = const { RefCell::new(None) };
    static ERR_TB_BUF: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn store_cstr(key: &'static LocalKey<RefCell<Option<CString>>>, s: &str) -> *const c_char {
    let cstr = CString::new(s).unwrap_or_default();
    let ptr = cstr.as_ptr();
    key.with(|buf| {
        *buf.borrow_mut() = Some(cstr);
    });
    ptr
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

fn parse_error_message(msg: &str) -> (String, String) {
    if let Some(colon) = msg.find(": ") {
        (msg[..colon].to_string(), msg[colon + 2..].to_string())
    } else {
        ("Error".to_string(), msg.to_string())
    }
}

fn set_capi_error(vm: *mut MsVM, type_name: &str, message: &str) -> MsStatus {
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.has_error = true;
    inner.vm.error_message = format!("{}: {}", type_name, message);
    MsStatus::MS_ERROR
}

fn object_to_string(obj: &Object) -> String {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        Object::Int(n) => n.to_string(),
        Object::Float(f) => f.to_string(),
        Object::Bool(b) => b.to_string(),
        Object::Nil => "nil".to_string(),
        _ => format!("{:?}", obj),
    }
}

fn throw_with_msg(vm: *mut MsVM, type_name: &str, fmt: *const c_char) -> MsStatus {
    if vm.is_null() || fmt.is_null() {
        return MsStatus::MS_ERROR;
    }
    let msg = unsafe { CStr::from_ptr(fmt) }
        .to_string_lossy()
        .into_owned();
    set_capi_error(vm, type_name, &msg)
}

// ---------------------------------------------------------------------------
// 异常查询 — msErrOccurred / msErrFetch / msErrClear
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msErrOccurred(vm: *mut MsVM) -> c_int {
    if vm.is_null() {
        return MS_FALSE;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    if inner.vm.has_error {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msErrFetch(vm: *mut MsVM) -> *mut MsValue {
    if vm.is_null() {
        return std::ptr::null_mut();
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    if !inner.vm.has_error {
        return std::ptr::null_mut();
    }
    let (type_name, message) = parse_error_message(&inner.vm.error_message);
    inner.vm.has_error = false;
    inner.vm.error_message.clear();
    let exc = alloc_exception(
        &type_name,
        alloc_string(&message),
        alloc_string(""),
        Object::Nil,
    );
    Box::into_raw(Box::new(MsValue { inner: exc }))
}

#[no_mangle]
pub extern "C" fn msErrClear(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.has_error = false;
    inner.vm.error_message.clear();
}

// ---------------------------------------------------------------------------
// 异常对象属性 — msErrTypeName / msErrMessage / msErrTraceback / msErrCause
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msErrTypeName(_vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    if err.is_null() {
        return std::ptr::null();
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            store_cstr(&ERR_TYPE_BUF, &exc.class_name)
        }
        _ => std::ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn msErrMessage(_vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    if err.is_null() {
        return std::ptr::null();
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            let msg = object_to_string(&exc.message);
            store_cstr(&ERR_MSG_BUF, &msg)
        }
        _ => std::ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn msErrTraceback(_vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    if err.is_null() {
        return std::ptr::null();
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            let tb = object_to_string(&exc.traceback);
            store_cstr(&ERR_TB_BUF, &tb)
        }
        _ => std::ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn msErrCause(_vm: *mut MsVM, err: *mut MsValue) -> *mut MsValue {
    if err.is_null() {
        return std::ptr::null_mut();
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            match &exc.cause {
                Object::Nil => std::ptr::null_mut(),
                cause_obj => Box::into_raw(Box::new(MsValue {
                    inner: cause_obj.clone(),
                })),
            }
        }
        _ => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// C 侧抛出异常 — msThrow / msThrowValue / msThrowRethrow
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msThrow(
    vm: *mut MsVM,
    type_: *const c_char,
    fmt: *const c_char,
) -> MsStatus {
    if vm.is_null() || type_.is_null() || fmt.is_null() {
        return MsStatus::MS_ERROR;
    }
    let type_name = unsafe { CStr::from_ptr(type_) }
        .to_string_lossy()
        .into_owned();
    let message = unsafe { CStr::from_ptr(fmt) }
        .to_string_lossy()
        .into_owned();
    set_capi_error(vm, &type_name, &message)
}

#[no_mangle]
pub extern "C" fn msThrowValue(vm: *mut MsVM, err: *mut MsValue) -> MsStatus {
    if vm.is_null() || err.is_null() {
        return MsStatus::MS_ERROR;
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            let type_name = exc.class_name.clone();
            let message = object_to_string(&exc.message);
            set_capi_error(vm, &type_name, &message)
        }
        other => {
            let msg = format!("{:?}", other);
            set_capi_error(vm, "Error", &msg)
        }
    }
}

#[no_mangle]
pub extern "C" fn msThrowRethrow(vm: *mut MsVM) -> MsStatus {
    if vm.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    if !inner.vm.has_error {
        inner.vm.has_error = true;
        inner.vm.error_message = "Error: rethrow with no pending error".into();
    }
    MsStatus::MS_ERROR
}

// ---------------------------------------------------------------------------
// 便捷 throw 函数
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msThrowTypeError(
    vm: *mut MsVM,
    expected: *const c_char,
    actual: *const c_char,
) -> MsStatus {
    if vm.is_null() || expected.is_null() || actual.is_null() {
        return MsStatus::MS_ERROR;
    }
    let exp = unsafe { CStr::from_ptr(expected) }.to_string_lossy();
    let act = unsafe { CStr::from_ptr(actual) }.to_string_lossy();
    set_capi_error(vm, "TypeError", &format!("expected {}, got {}", exp, act))
}

#[no_mangle]
pub extern "C" fn msThrowValueError(vm: *mut MsVM, fmt: *const c_char) -> MsStatus {
    throw_with_msg(vm, "ValueError", fmt)
}

#[no_mangle]
pub extern "C" fn msThrowIndexError(vm: *mut MsVM, fmt: *const c_char) -> MsStatus {
    throw_with_msg(vm, "IndexError", fmt)
}

#[no_mangle]
pub extern "C" fn msThrowKeyError(vm: *mut MsVM, key: *mut MsValue) -> MsStatus {
    if vm.is_null() || key.is_null() {
        return MsStatus::MS_ERROR;
    }
    let key_str = object_to_string(unsafe { &(*key).inner });
    set_capi_error(vm, "KeyError", &key_str)
}

#[no_mangle]
pub extern "C" fn msThrowRuntimeError(vm: *mut MsVM, fmt: *const c_char) -> MsStatus {
    throw_with_msg(vm, "RuntimeError", fmt)
}

#[no_mangle]
pub extern "C" fn msThrowIoError(vm: *mut MsVM, fmt: *const c_char) -> MsStatus {
    throw_with_msg(vm, "IOError", fmt)
}

// ---------------------------------------------------------------------------
// try/catch 模式 — msTry
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msTry(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
    result: *mut *mut MsValue,
) -> MsStatus {
    if vm.is_null() || func.is_null() || result.is_null() {
        return MsStatus::MS_ERROR;
    }
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner.vm.has_error = false;
        inner.vm.error_message.clear();
    }
    let ret = msCall(vm, func, args, nargs);
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    if inner.vm.has_error {
        unsafe { *result = std::ptr::null_mut() };
        MsStatus::MS_ERROR
    } else {
        unsafe { *result = ret };
        MsStatus::MS_OK
    }
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    use super::*;
    use crate::capi::gc::{msRoot, msUnroot};
    use crate::capi::value::*;
    use crate::capi::vm::*;
    use std::ffi::CStr;
    use std::os::raw::c_char;

    fn new_vm() -> *mut MsVM {
        msVmNew()
    }

    #[test]
    fn test_err_occurred_initially_false() {
        let vm = new_vm();
        assert_eq!(msErrOccurred(vm), MS_FALSE);
        msVmFree(vm);
    }

    #[test]
    fn test_throw_and_catch() {
        let vm = new_vm();

        let c_type = b"ValueError\0".as_ptr() as *const c_char;
        let c_fmt = b"invalid value: %d\0".as_ptr() as *const c_char;
        let status = msThrow(vm, c_type, c_fmt);

        assert_eq!(status, MsStatus::MS_ERROR);
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err = msErrFetch(vm);
        assert!(!err.is_null());
        assert_eq!(msErrOccurred(vm), MS_FALSE);

        let msg = msErrMessage(vm, err);
        let msg_str = unsafe { CStr::from_ptr(msg).to_str().unwrap() };
        assert!(msg_str.contains("invalid value"));

        let type_name = msErrTypeName(vm, err);
        let type_str = unsafe { CStr::from_ptr(type_name).to_str().unwrap() };
        assert_eq!(type_str, "ValueError");

        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_type_error() {
        let vm = new_vm();

        let expected = b"string\0".as_ptr() as *const c_char;
        let actual = b"int\0".as_ptr() as *const c_char;
        let status = msThrowTypeError(vm, expected, actual);

        assert_eq!(status, MsStatus::MS_ERROR);
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "TypeError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("expected string, got int"));

        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_value_error() {
        let vm = new_vm();
        let fmt = b"out of range\0".as_ptr() as *const c_char;
        let _ = msThrowValueError(vm, fmt);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "ValueError");
        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_index_error() {
        let vm = new_vm();
        let fmt = b"index 10 out of bounds\0".as_ptr() as *const c_char;
        let _ = msThrowIndexError(vm, fmt);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "IndexError");
        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_key_error() {
        let vm = new_vm();
        let key = msString(vm, b"missing_key\0".as_ptr() as *const c_char);
        let _ = msThrowKeyError(vm, key);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "KeyError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("missing_key"));

        msUnroot(vm, err);
        msValueFree(key);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_runtime_error() {
        let vm = new_vm();
        let fmt = b"unexpected state\0".as_ptr() as *const c_char;
        let _ = msThrowRuntimeError(vm, fmt);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "RuntimeError");
        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_convenience_throw_io_error() {
        let vm = new_vm();
        let fmt = b"cannot open file\0".as_ptr() as *const c_char;
        let _ = msThrowIoError(vm, fmt);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "IOError");
        msUnroot(vm, err);
        msVmFree(vm);
    }

    #[test]
    fn test_err_clear() {
        let vm = new_vm();

        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = msThrow(vm, c_type, c_fmt);
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        msErrClear(vm);
        assert_eq!(msErrOccurred(vm), MS_FALSE);

        let err = msErrFetch(vm);
        assert!(err.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_err_fetch_returns_null_when_no_error() {
        let vm = new_vm();
        let err = msErrFetch(vm);
        assert!(err.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_throw_value() {
        let vm = new_vm();

        let c_type = b"MyError\0".as_ptr() as *const c_char;
        let c_fmt = b"something went wrong\0".as_ptr() as *const c_char;
        let _ = msThrow(vm, c_type, c_fmt);
        let err_obj = msErrFetch(vm);

        let _ = msThrowValue(vm, err_obj);
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err2 = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err2)).to_str().unwrap() };
        assert_eq!(type_str, "MyError");

        msValueFree(err_obj);
        msValueFree(err2);
        msVmFree(vm);
    }

    #[test]
    fn test_err_traceback_empty() {
        let vm = new_vm();
        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = msThrow(vm, c_type, c_fmt);

        let err = msErrFetch(vm);
        let tb = msErrTraceback(vm, err);
        let tb_str = unsafe { CStr::from_ptr(tb).to_str().unwrap() };
        assert_eq!(tb_str, "");

        msValueFree(err);
        msVmFree(vm);
    }

    #[test]
    fn test_err_cause_none() {
        let vm = new_vm();
        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = msThrow(vm, c_type, c_fmt);

        let err = msErrFetch(vm);
        let cause = msErrCause(vm, err);
        assert!(cause.is_null());

        msValueFree(err);
        msVmFree(vm);
    }

    #[test]
    fn test_try_success() {
        let vm = new_vm();

        let script = b"fn add(a, b) { return a + b }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let status = msExecString(vm, script, filename);
        assert_eq!(status, MsStatus::MS_OK);

        let func = msGetGlobal(vm, b"add\0".as_ptr() as *const c_char);
        assert!(!func.is_null());
        msRoot(vm, func);

        let a = msInt(3);
        let b = msInt(4);
        let args = [a, b];

        let mut result: *mut MsValue = std::ptr::null_mut();
        let try_status = msTry(vm, func, args.as_ptr(), 2, &mut result);

        assert_eq!(try_status, MsStatus::MS_OK);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 7);

        msValueFree(result);
        msValueFree(a);
        msValueFree(b);
        msUnroot(vm, func);
        msValueFree(func);
        msVmFree(vm);
    }

    #[test]
    fn test_try_exception() {
        let vm = new_vm();

        let script = b"fn boom() { throw ValueError(\"boom\") }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let status = msExecString(vm, script, filename);
        assert_eq!(status, MsStatus::MS_OK);

        let func = msGetGlobal(vm, b"boom\0".as_ptr() as *const c_char);
        assert!(!func.is_null());
        msRoot(vm, func);

        let mut result: *mut MsValue = std::ptr::null_mut();
        let try_status = msTry(vm, func, std::ptr::null(), 0, &mut result);

        assert_eq!(try_status, MsStatus::MS_ERROR);
        assert!(result.is_null());
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "ValueError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("boom"));

        msValueFree(err);
        msUnroot(vm, func);
        msValueFree(func);
        msVmFree(vm);
    }

    #[test]
    fn test_nested_try() {
        let vm = new_vm();

        let script =
            b"fn outer() { throw RuntimeError(\"outer error\") }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let _ = msExecString(vm, script, filename);

        let func = msGetGlobal(vm, b"outer\0".as_ptr() as *const c_char);
        msRoot(vm, func);

        let mut result1: *mut MsValue = std::ptr::null_mut();
        let s1 = msTry(vm, func, std::ptr::null(), 0, &mut result1);
        assert_eq!(s1, MsStatus::MS_ERROR);

        let mut result2: *mut MsValue = std::ptr::null_mut();
        let s2 = msTry(vm, func, std::ptr::null(), 0, &mut result2);
        assert_eq!(s2, MsStatus::MS_ERROR);

        let err = msErrFetch(vm);
        assert!(!err.is_null());
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "RuntimeError");

        msValueFree(err);
        msUnroot(vm, func);
        msValueFree(func);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety() {
        assert_eq!(msErrOccurred(std::ptr::null_mut()), MS_FALSE);
        assert!(msErrFetch(std::ptr::null_mut()).is_null());

        msErrClear(std::ptr::null_mut());

        assert_eq!(
            msThrow(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()),
            MsStatus::MS_ERROR
        );
        assert_eq!(
            msThrowValue(std::ptr::null_mut(), std::ptr::null_mut()),
            MsStatus::MS_ERROR
        );
        assert_eq!(
            msThrowRethrow(std::ptr::null_mut()),
            MsStatus::MS_ERROR
        );
        assert_eq!(
            msThrowTypeError(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()),
            MsStatus::MS_ERROR
        );
        assert_eq!(
            msThrowValueError(std::ptr::null_mut(), std::ptr::null()),
            MsStatus::MS_ERROR
        );
        assert_eq!(
            msThrowKeyError(std::ptr::null_mut(), std::ptr::null_mut()),
            MsStatus::MS_ERROR
        );

        assert!(msErrTypeName(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        assert!(msErrMessage(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        assert!(msErrTraceback(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        assert!(msErrCause(std::ptr::null_mut(), std::ptr::null_mut()).is_null());

        let mut result: *mut MsValue = std::ptr::null_mut();
        assert_eq!(
            msTry(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), 0, &mut result),
            MsStatus::MS_ERROR
        );
    }
}
