//! C API — 函数调用（task 70）。
//!
//! 参照 [70-capi-call](../../docs/mslang/tasks/70-capi-call.md)。
//!
//! 实现 `msCall`（同步函数调用）和 `msMakeCFunction`（C 原生函数注册桥接）。
//! msCall 复用 VM 已有的 `call_function` 方法，仅负责 MsValue* ↔ Object 转换
//! 和错误桥接。

use std::os::raw::c_int;

use crate::capi::types::{MsCFunction, MsValue};
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::builtins::alloc_c_native_function;
use crate::vm::object::Object;

/// 调用可调用对象，返回结果（新引用）。异常时返回 NULL。
///
/// func 必须是可调用对象（Function、Closure、BoundMethod、C 原生函数，
/// 或带 `__call__` 的 Instance）。可调用性由 VM `call_value` 内部 match 处理。
#[no_mangle]
pub extern "C" fn msCall(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
) -> *mut MsValue {
    if vm.is_null() || func.is_null() {
        return std::ptr::null_mut();
    }
    if nargs < 0 {
        return std::ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let func_obj = unsafe { (*func).inner.clone() };

    let nargs_usize = nargs as usize;
    let mut arg_objects: Vec<Object> = Vec::with_capacity(nargs_usize);
    if nargs_usize > 0 {
        if args.is_null() {
            inner.vm.has_error = true;
            inner.vm.error_message = "msCall: args is NULL but nargs > 0".into();
            return std::ptr::null_mut();
        }
        let arg_slice = unsafe { std::slice::from_raw_parts(args, nargs_usize) };
        for &arg_ptr in arg_slice {
            if arg_ptr.is_null() {
                inner.vm.has_error = true;
                inner.vm.error_message = "msCall: NULL argument in args".into();
                return std::ptr::null_mut();
            }
            arg_objects.push(unsafe { (*arg_ptr).inner.clone() });
        }
    }

    match inner.vm.call_function(&func_obj, &arg_objects) {
        Ok(result) => Box::into_raw(Box::new(MsValue { inner: result })),
        Err(msg) => {
            inner.vm.has_error = true;
            inner.vm.error_message = msg;
            std::ptr::null_mut()
        }
    }
}

/// 将 C 函数指针包装为 VM 可调用对象（MsValue*）。
/// 返回的值可作为全局变量注册，供 mslang 脚本调用。
#[no_mangle]
pub extern "C" fn msMakeCFunction(
    vm: *mut MsVM,
    name: *const std::os::raw::c_char,
    func: MsCFunction,
    arity: c_int,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() || func.is_none() {
        return std::ptr::null_mut();
    }

    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let obj = alloc_c_native_function(&name_str, func, arity);

    Box::into_raw(Box::new(MsValue { inner: obj }))
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::gc::{msRoot, msUnroot};
    use crate::capi::types::MsStatus;
    use crate::capi::value::*;
    use crate::capi::vm::*;
    use std::ffi::CString;
    use std::os::raw::c_char;
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            unsafe {
                let _ = Box::from_raw(val);
            }
        }
    }

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn exec(vm: *mut MsVM, src: &str) {
        let cs = cstr(src);
        let fname = cstr("test.ms");
        let status = msExecString(vm, cs.as_ptr(), fname.as_ptr());
        assert_eq!(status, MsStatus::MS_OK, "exec failed for: {}", src);
    }

    fn get_global(vm: *mut MsVM, name: &str) -> *mut MsValue {
        let cs = cstr(name);
        let val = msGetGlobal(vm, cs.as_ptr());
        assert!(!val.is_null(), "global '{}' not found", name);
        val
    }

    #[test]
    fn test_call_script_function() {
        let vm = msVmNew();
        exec(vm, "fn add(a, b) {\n  return a + b\n}\n");

        let add_fn = get_global(vm, "add");
        msRoot(vm, add_fn);

        let a = msInt(3);
        let b = msInt(4);
        let args = [a, b];
        let result = msCall(vm, add_fn, args.as_ptr(), 2);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 7);

        free_value(result);
        free_value(a);
        free_value(b);
        msUnroot(vm, add_fn);
        free_value(add_fn);
        msVmFree(vm);
    }

    #[test]
    fn test_call_zero_args() {
        let vm = msVmNew();
        exec(vm, "fn fortytwo() {\n  return 42\n}\n");

        let fn_val = get_global(vm, "fortytwo");
        msRoot(vm, fn_val);

        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 42);

        free_value(result);
        msUnroot(vm, fn_val);
        free_value(fn_val);
        msVmFree(vm);
    }

    #[test]
    fn test_call_with_exception() {
        let vm = msVmNew();
        exec(vm, "fn boom() {\n  throw \"exploded\"\n}\n");

        let fn_val = get_global(vm, "boom");
        msRoot(vm, fn_val);

        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(result.is_null());

        let guard = lock_vm(vm);
        let inner = unsafe { &*guard.get() };
        assert!(inner.vm.has_error);
        assert!(inner.vm.error_message.contains("exploded"));
        drop(guard);

        msUnroot(vm, fn_val);
        free_value(fn_val);
        msVmFree(vm);
    }

    #[test]
    fn test_call_closure() {
        let vm = msVmNew();
        exec(
            vm,
            "fn make_adder(x) {\n  return fn(y) {\n    return x + y\n  }\n}\nadder = make_adder(10)\n",
        );

        let adder = get_global(vm, "adder");
        msRoot(vm, adder);

        let arg = msInt(5);
        let args = [arg];
        let result = msCall(vm, adder, args.as_ptr(), 1);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 15);

        free_value(result);
        free_value(arg);
        msUnroot(vm, adder);
        free_value(adder);
        msVmFree(vm);
    }

    #[test]
    fn test_call_non_callable() {
        let vm = msVmNew();

        let not_callable = msInt(42);
        let result = msCall(vm, not_callable, ptr::null(), 0);
        assert!(result.is_null());

        let guard = lock_vm(vm);
        let inner = unsafe { &*guard.get() };
        assert!(inner.vm.has_error);
        drop(guard);

        free_value(not_callable);
        msVmFree(vm);
    }

    #[test]
    fn test_call_null_safety() {
        let result = msCall(ptr::null_mut(), ptr::null_mut(), ptr::null(), 0);
        assert!(result.is_null());

        let vm = msVmNew();
        let result = msCall(vm, ptr::null_mut(), ptr::null(), 0);
        assert!(result.is_null());

        let result = msCall(vm, msInt(1), ptr::null(), -1);
        assert!(result.is_null());

        free_value(msInt(1));
        msVmFree(vm);
    }

    #[test]
    fn test_recursive_call() {
        let vm = msVmNew();
        exec(
            vm,
            "fn fib(n) {\n  if n <= 1 {\n    return n\n  }\n  return fib(n - 1) + fib(n - 2)\n}\n",
        );

        let fib_fn = get_global(vm, "fib");
        msRoot(vm, fib_fn);

        let arg = msInt(10);
        let args = [arg];
        let result = msCall(vm, fib_fn, args.as_ptr(), 1);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 55);

        free_value(result);
        free_value(arg);
        msUnroot(vm, fib_fn);
        free_value(fib_fn);
        msVmFree(vm);
    }

    extern "C" fn c_mul(
        _vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: i32,
    ) -> *mut MsValue {
        if nargs < 2 {
            return ptr::null_mut();
        }
        let a = unsafe { (*(*args.add(0))).inner.clone() };
        let b = unsafe { (*(*args.add(1))).inner.clone() };
        match (a, b) {
            (Object::Int(x), Object::Int(y)) => {
                Box::into_raw(Box::new(MsValue {
                    inner: Object::Int(x * y),
                }))
            }
            _ => ptr::null_mut(),
        }
    }

    #[test]
    fn test_native_function_bridge() {
        let vm = msVmNew();

        let name = cstr("mul");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_mul), 2);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let global_name = cstr("mul");
        assert_eq!(msSetGlobal(vm, global_name.as_ptr(), cfn), MsStatus::MS_OK);

        exec(vm, "result = mul(3, 7)\n");

        let result_val = get_global(vm, "result");
        msRoot(vm, result_val);
        assert_eq!(msToInt(vm, result_val), 21);

        msUnroot(vm, result_val);
        free_value(result_val);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }

    extern "C" fn c_check_positive(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: i32,
    ) -> *mut MsValue {
        if nargs < 1 {
            return ptr::null_mut();
        }
        let val = unsafe { (*(*args.add(0))).inner.clone() };
        match val {
            Object::Int(n) if n >= 0 => {
                Box::into_raw(Box::new(MsValue {
                    inner: Object::Int(n),
                }))
            }
            _ => {
                let guard = lock_vm(vm);
                let inner = unsafe { &mut *guard.get() };
                inner.vm.has_error = true;
                inner.vm.error_message = "ValueError: negative".into();
                drop(guard);
                ptr::null_mut()
            }
        }
    }

    #[test]
    fn test_native_function_throws() {
        let vm = msVmNew();

        let name = cstr("check_pos");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_check_positive), 1);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let global_name = cstr("check_pos");
        assert_eq!(msSetGlobal(vm, global_name.as_ptr(), cfn), MsStatus::MS_OK);

        exec(
            vm,
            "fn try_catch() {\n  try {\n    check_pos(-1)\n    return 999\n  } except Error as e {\n    return -1\n  }\n}\n",
        );

        let fn_val = get_global(vm, "try_catch");
        msRoot(vm, fn_val);
        let result = msCall(vm, fn_val, ptr::null(), 0);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), -1);

        free_value(result);
        msUnroot(vm, fn_val);
        free_value(fn_val);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }

    #[test]
    fn test_make_c_function_null_safety() {
        let vm = msVmNew();
        let name = cstr("noop");

        assert!(msMakeCFunction(ptr::null_mut(), name.as_ptr(), Some(c_mul), 0).is_null());
        assert!(msMakeCFunction(vm, ptr::null(), Some(c_mul), 0).is_null());
        assert!(msMakeCFunction(vm, name.as_ptr(), None, 0).is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_native_function_bridge_direct_call() {
        let vm = msVmNew();

        let name = cstr("mul");
        let cfn = msMakeCFunction(vm, name.as_ptr(), Some(c_mul), 2);
        assert!(!cfn.is_null());
        msRoot(vm, cfn);

        let a = msInt(6);
        let b = msInt(7);
        let args = [a, b];
        let result = msCall(vm, cfn, args.as_ptr(), 2);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 42);

        free_value(result);
        free_value(a);
        free_value(b);
        msUnroot(vm, cfn);
        free_value(cfn);
        msVmFree(vm);
    }
}
