//! C API — VM 生命周期与配置（task 66）。
//!
//! 参照 [66-capi-vm](../../docs/mslang/tasks/66-capi-vm.md)。
//!
//! 实现 vm.h 中定义的全部 C API 函数：VM 创建/销毁、配置（模块路径、命令行参数、
//! 输出重定向）、脚本执行（msExecFile/msExecString/msEval）、全局变量操作、
//! per-VM 互斥锁（API 内部自动加锁）。

use crate::capi::types::{MsStatus, MsValue};
use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use std::cell::{RefCell, UnsafeCell};
use std::ffi::c_void;

// ---------------------------------------------------------------------------
// 类型定义
// ---------------------------------------------------------------------------

/// C 回调函数指针类型（`typedef int (*MsWriteFn)(const char*, size_t, void*)`）。
/// `Option` 表示可空：NULL = None = 恢复默认输出。
pub(crate) type MsWriteFnRaw =
    extern "C" fn(data: *const i8, len: usize, userdata: *mut c_void) -> i32;

/// 与 C 侧 `MsWriteFn` 对应：可为 NULL 的函数指针。
pub(crate) type MsWriteFn = Option<MsWriteFnRaw>;

/// 输出回调存储（函数指针 + userdata）。
#[repr(C)]
pub struct WriteCallback {
    pub fn_ptr: MsWriteFn,
    pub userdata: *mut c_void,
}
// 裸指针字段需要手动声明 Send/Sync（FFI 场景，由 C 侧保证线程安全）。
unsafe impl Send for WriteCallback {}
unsafe impl Sync for WriteCallback {}

/// VM 内部状态，被 ReentrantMutex 保护。
pub(crate) struct VmInner {
    pub(crate) vm: crate::vm::VM,
    #[allow(dead_code)]
    module_paths: Vec<String>,
    #[allow(dead_code)]
    args: Vec<String>,
    stdout_cb: Option<WriteCallback>,
    stderr_cb: Option<WriteCallback>,
    /// task 74：C 侧 finalizer 注册表（msOnFinalize 注册、msGcCollect 时执行）。
    pub(crate) c_finalizers: Vec<crate::capi::gc::CFinalizerEntry>,
}

/// 不透明 VM 句柄（C 侧 `typedef struct MsVM MsVM`）。
///
/// `inner` 字段为 private，C 侧无法访问。ReentrantMutex 允许同线程重入
/// （msVmLock 后调用 ms* API 不会死锁）。UnsafeCell 提供 &mut 访问——
/// 安全性由 ReentrantMutex 保证（同一线程不并发创建多个 &mut）。
#[repr(C)]
pub struct MsVM {
    inner: ReentrantMutex<UnsafeCell<VmInner>>,
}

// ---------------------------------------------------------------------------
// 锁辅助函数
// ---------------------------------------------------------------------------

/// 加锁并返回 guard。guard 通过 Deref 提供 `UnsafeCell<VmInner>`，
/// 调用 `guard.get()` 可获取 `*mut VmInner`。
///
/// 'static 生命周期安全：MsVM 经 Box 分配地址稳定（不移动），原始指针有效。
pub(crate) fn lock_vm(
    vm: *mut MsVM,
) -> parking_lot::ReentrantMutexGuard<'static, UnsafeCell<VmInner>> {
    // SAFETY: MsVM 经 Box 分配，地址稳定。
    let vm_ref: &'static MsVM = unsafe { &*vm };
    vm_ref.inner.lock()
}

// ---------------------------------------------------------------------------
// 创建与销毁
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msVmNew() -> *mut MsVM {
    let inner = VmInner {
        vm: crate::vm::VM::new(),
        module_paths: Vec::new(),
        args: Vec::new(),
        stdout_cb: None,
        stderr_cb: None,
        c_finalizers: Vec::new(),
    };
    let vm = Box::new(MsVM {
        inner: ReentrantMutex::new(UnsafeCell::new(inner)),
    });
    let ptr = Box::into_raw(vm);
    let guard = lock_vm(ptr);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.capi_vm_ptr = ptr as *mut u8;
    drop(guard);
    ptr
}

#[no_mangle]
pub extern "C" fn msVmFree(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    // SAFETY: vm 由 msVmNew 的 Box::into_raw 返回，此处恢复所有权并 drop。
    unsafe {
        let _ = Box::from_raw(vm);
    }
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msAddModulePath(vm: *mut MsVM, path: *const i8) {
    if vm.is_null() || path.is_null() {
        return;
    }
    let path_str = unsafe { std::ffi::CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.module_paths.push(path_str);
}

#[no_mangle]
pub extern "C" fn msSetArgs(vm: *mut MsVM, argc: i32, argv: *const *const i8) {
    if vm.is_null() {
        return;
    }
    let args: Vec<String> = if argc > 0 && !argv.is_null() {
        (0..argc as usize)
            .map(|i| {
                unsafe { std::ffi::CStr::from_ptr(*argv.add(i)) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    } else {
        Vec::new()
    };
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.args = args;
}

#[no_mangle]
pub extern "C" fn msSetStdout(vm: *mut MsVM, fn_ptr: MsWriteFn, userdata: *mut c_void) {
    set_write_callback(vm, fn_ptr, userdata, |inner, cb| {
        inner.stdout_cb = cb;
    });
}

#[no_mangle]
pub extern "C" fn msSetStderr(vm: *mut MsVM, fn_ptr: MsWriteFn, userdata: *mut c_void) {
    set_write_callback(vm, fn_ptr, userdata, |inner, cb| {
        inner.stderr_cb = cb;
    });
}

/// 通用回调设置：将 fn_ptr + userdata 封装为 WriteCallback，经 setter 存入 VmInner，
/// 然后同步 VM 的 stdout_writer/stderr_writer 字段指向最新回调。
fn set_write_callback(
    vm: *mut MsVM,
    fn_ptr: MsWriteFn,
    userdata: *mut c_void,
    setter: impl FnOnce(&mut VmInner, Option<WriteCallback>),
) {
    if vm.is_null() {
        return;
    }
    let cb = fn_ptr.map(|f| WriteCallback {
        fn_ptr: Some(f),
        userdata,
    });
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    setter(inner, cb);
    sync_writers(inner);
}

/// 将 VmInner.stdout_cb/stderr_cb 的地址同步到 VM 的 stdout_writer/stderr_writer。
/// VM 经 builtin_print 读取这些裸指针，在回调存在时调用 C 函数而非 Rust print!。
fn sync_writers(inner: &mut VmInner) {
    let stdout_ptr = inner
        .stdout_cb
        .as_ref()
        .map(|wc| wc as *const WriteCallback as *mut c_void);
    inner.vm.stdout_writer = stdout_ptr;
    let stderr_ptr = inner
        .stderr_cb
        .as_ref()
        .map(|wc| wc as *const WriteCallback as *mut c_void);
    inner.vm.stderr_writer = stderr_ptr;
}

// ---------------------------------------------------------------------------
// 脚本执行
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msExecFile(vm: *mut MsVM, path: *const i8) -> MsStatus {
    if vm.is_null() || path.is_null() {
        return MsStatus::MS_ERROR;
    }
    let path_str = unsafe { std::ffi::CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    let source = match std::fs::read_to_string(&path_str) {
        Ok(s) => s,
        Err(_) => return MsStatus::MS_ERROR,
    };
    exec_source(vm, &source, Some(&path_str))
}

#[no_mangle]
pub extern "C" fn msExecString(
    vm: *mut MsVM,
    source: *const i8,
    filename: *const i8,
) -> MsStatus {
    if vm.is_null() || source.is_null() {
        return MsStatus::MS_ERROR;
    }
    let source_str = unsafe { std::ffi::CStr::from_ptr(source) }
        .to_string_lossy()
        .into_owned();
    let filename_str = if filename.is_null() {
        None
    } else {
        Some(
            unsafe { std::ffi::CStr::from_ptr(filename) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    exec_source(vm, &source_str, filename_str.as_deref())
}

/// 完整的编译执行管线：Lexer → Parser → Compiler → VM.interpret。
/// 加锁后在整个流程中持有锁，保证线程安全。
fn exec_source(vm: *mut MsVM, source: &str, filename: Option<&str>) -> MsStatus {
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let _ = filename; // 错误信息标注来源（MVP 阶段暂未接入 filename 到错误消息）

    let tokens = match crate::lexer::Lexer::new(source).tokenize_all() {
        Ok(t) => t,
        Err(_) => return MsStatus::MS_ERROR,
    };
    let ast = match crate::parser::Parser::new(tokens).parse() {
        Ok(a) => a,
        Err(_) => return MsStatus::MS_ERROR,
    };
    let mut compiler = crate::compiler::Compiler::new();
    // module_mode 使顶层赋值走 StoreGlobal，使 msGetGlobal 可访问。
    compiler.set_module_mode(true);
    let chunk = match compiler.compile(&ast) {
        Ok(c) => c,
        Err(_) => return MsStatus::MS_ERROR,
    };
    match inner.vm.interpret(chunk) {
        Ok(_) => MsStatus::MS_OK,
        Err(_) => MsStatus::MS_ERROR,
    }
}

#[no_mangle]
pub extern "C" fn msEval(vm: *mut MsVM, expr: *const i8) -> *mut MsValue {
    if vm.is_null() || expr.is_null() {
        return std::ptr::null_mut();
    }
    let expr_str = unsafe { std::ffi::CStr::from_ptr(expr) }
        .to_string_lossy()
        .into_owned();
    // 包装为 return <expr> 脚本，使 interpret 返回表达式的值。
    let source = format!("return {}", expr_str);

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let tokens = match crate::lexer::Lexer::new(&source).tokenize_all() {
        Ok(t) => t,
        Err(_) => return std::ptr::null_mut(),
    };
    let ast = match crate::parser::Parser::new(tokens).parse() {
        Ok(a) => a,
        Err(_) => return std::ptr::null_mut(),
    };
    let chunk = match crate::compiler::Compiler::new().compile(&ast) {
        Ok(c) => c,
        Err(_) => return std::ptr::null_mut(),
    };
    match inner.vm.interpret(chunk) {
        Ok(obj) => {
            let val = Box::new(MsValue { inner: obj });
            Box::into_raw(val)
        }
        Err(_) => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// 全局变量
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msGetGlobal(vm: *mut MsVM, name: *const i8) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    match inner.vm.globals().get(&name_str) {
        Some(obj) => {
            let val = Box::new(MsValue {
                inner: obj.clone(),
            });
            Box::into_raw(val)
        }
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn msSetGlobal(
    vm: *mut MsVM,
    name: *const i8,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || name.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: val 由 msEval/msGetGlobal 的 Box::into_raw 返回，指向有效 MsValue。
    let value = unsafe { (*val).inner.clone() };
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.globals_mut().insert(name_str, value);
    MsStatus::MS_OK
}

#[no_mangle]
pub extern "C" fn msDelGlobal(vm: *mut MsVM, name: *const i8) {
    if vm.is_null() || name.is_null() {
        return;
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.globals_mut().remove(&name_str);
}

// ---------------------------------------------------------------------------
// 线程安全
// ---------------------------------------------------------------------------

// 同线程持有的锁 guard 栈。msVmLock push，msVmUnlock pop。
// ReentrantMutex 保证同线程不阻塞；pop 时 drop 释放一层锁。
thread_local! {
    static HELD_GUARDS: RefCell<Vec<ReentrantMutexGuard<'static, UnsafeCell<VmInner>>>> =
        RefCell::new(Vec::new());
}

#[no_mangle]
pub extern "C" fn msVmLock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    HELD_GUARDS.with(|s| s.borrow_mut().push(guard));
}

#[no_mangle]
pub extern "C" fn msVmUnlock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let _ = vm; // vm 仅用于 NULL 检查；guard 按 LIFO 弹出。
    HELD_GUARDS.with(|s| {
        s.borrow_mut().pop();
    });
}

// ---------------------------------------------------------------------------
// MsValue 内存释放
// ---------------------------------------------------------------------------

/// 释放 C 侧持有的 MsValue。NULL 安全。
/// 注意：仅释放 Box<MsValue> 包装，不释放 inner Object 引用的堆对象（由 GC 管理）。
#[no_mangle]
pub extern "C" fn msValueFree(val: *mut MsValue) {
    if val.is_null() {
        return;
    }
    // SAFETY: val 由 msEval/msGetGlobal 的 Box::into_raw 返回。
    unsafe {
        let _ = Box::from_raw(val);
    }
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_vm_new_free() {
        let vm = msVmNew();
        assert!(!vm.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_vm_free_null() {
        msVmFree(std::ptr::null_mut());
    }

    #[test]
    fn test_exec_string() {
        let vm = msVmNew();
        let source = CString::new("x = 42").unwrap();
        let filename = CString::new("test.ms").unwrap();
        let status = msExecString(vm, source.as_ptr(), filename.as_ptr());
        assert_eq!(status, MsStatus::MS_OK);
        msVmFree(vm);
    }

    #[test]
    fn test_exec_string_error() {
        let vm = msVmNew();
        let source = CString::new("fn (").unwrap();
        let filename = CString::new("bad.ms").unwrap();
        let status = msExecString(vm, source.as_ptr(), filename.as_ptr());
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_global_roundtrip() {
        let vm = msVmNew();
        let source = CString::new("answer = 42").unwrap();
        let filename = CString::new("test.ms").unwrap();
        msExecString(vm, source.as_ptr(), filename.as_ptr());
        let name = CString::new("answer").unwrap();
        let val = msGetGlobal(vm, name.as_ptr());
        assert!(!val.is_null());
        msValueFree(val);
        msVmFree(vm);
    }

    #[test]
    fn test_global_get_missing() {
        let vm = msVmNew();
        let name = CString::new("nonexistent").unwrap();
        let val = msGetGlobal(vm, name.as_ptr());
        assert!(val.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_global_set_get_del() {
        let vm = msVmNew();
        let name = CString::new("x").unwrap();
        let source = CString::new("x = 1").unwrap();
        let filename = CString::new("test.ms").unwrap();
        msExecString(vm, source.as_ptr(), filename.as_ptr());
        let val = msGetGlobal(vm, name.as_ptr());
        assert!(!val.is_null());
        msValueFree(val);

        // 删除全局变量
        msDelGlobal(vm, name.as_ptr());
        let val2 = msGetGlobal(vm, name.as_ptr());
        assert!(val2.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_two_vms_independent() {
        let vm1 = msVmNew();
        let vm2 = msVmNew();
        let source = CString::new("x = 1").unwrap();
        let filename = CString::new("test.ms").unwrap();
        msExecString(vm1, source.as_ptr(), filename.as_ptr());
        let name = CString::new("x").unwrap();
        let val1 = msGetGlobal(vm1, name.as_ptr());
        let val2 = msGetGlobal(vm2, name.as_ptr());
        assert!(!val1.is_null());
        assert!(val2.is_null());
        msValueFree(val1);
        msVmFree(vm1);
        msVmFree(vm2);
    }

    #[test]
    fn test_output_redirect() {
        use std::sync::Mutex;

        // Box<Arc<...>> ensures userdata points to an Arc on the heap,
        // so the callback's `&*(userdata as *const Arc<...>)` cast is valid.
        let captured: std::sync::Arc<Mutex<Vec<u8>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let captured_ptr = Box::into_raw(Box::new(captured));

        extern "C" fn write_cb(
            data: *const i8,
            len: usize,
            userdata: *mut c_void,
        ) -> i32 {
            // SAFETY: userdata points to Arc<Mutex<Vec<u8>>> on the heap.
            let captured =
                unsafe { &*(userdata as *const std::sync::Arc<Mutex<Vec<u8>>>) };
            let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
            captured.lock().unwrap().extend_from_slice(bytes);
            0
        }

        let vm = msVmNew();
        msSetStdout(vm, Some(write_cb), captured_ptr as *mut c_void);

        let source = CString::new("print(\"hello\")").unwrap();
        let filename = CString::new("test.ms").unwrap();
        msExecString(vm, source.as_ptr(), filename.as_ptr());

        // SAFETY: captured_ptr was created by Box::into_raw above.
        let captured = unsafe { *Box::from_raw(captured_ptr) };
        let data = captured.lock().unwrap();
        let output = String::from_utf8_lossy(&data);
        assert!(output.contains("hello"));

        msVmFree(vm);
    }

    #[test]
    fn test_add_module_path() {
        let vm = msVmNew();
        let path = CString::new("/test/path").unwrap();
        msAddModulePath(vm, path.as_ptr());
        msVmFree(vm);
    }

    #[test]
    fn test_set_args() {
        let vm = msVmNew();
        let arg0 = CString::new("ms").unwrap();
        let arg1 = CString::new("script.ms").unwrap();
        let argv = [arg0.as_ptr(), arg1.as_ptr()];
        msSetArgs(vm, 2, argv.as_ptr());
        msVmFree(vm);
    }

    #[test]
    fn test_vm_lock_unlock() {
        let vm = msVmNew();
        msVmLock(vm);
        let name = CString::new("x").unwrap();
        let source = CString::new("x = 1").unwrap();
        let filename = CString::new("test.ms").unwrap();
        msExecString(vm, source.as_ptr(), filename.as_ptr());
        let val = msGetGlobal(vm, name.as_ptr());
        assert!(!val.is_null());
        msValueFree(val);
        msVmUnlock(vm);
        msVmFree(vm);
    }

    #[test]
    fn test_eval() {
        let vm = msVmNew();
        let expr = CString::new("1 + 2").unwrap();
        let val = msEval(vm, expr.as_ptr());
        assert!(!val.is_null());
        msValueFree(val);
        msVmFree(vm);
    }

    #[test]
    fn test_eval_error() {
        let vm = msVmNew();
        let expr = CString::new("@@@invalid").unwrap();
        let val = msEval(vm, expr.as_ptr());
        assert!(val.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_value_free_null() {
        msValueFree(std::ptr::null_mut());
    }

    #[test]
    fn test_exec_null_safety() {
        let status = msExecString(std::ptr::null_mut(), std::ptr::null(), std::ptr::null());
        assert_eq!(status, MsStatus::MS_ERROR);
    }

    #[test]
    fn test_set_global_api() {
        let vm = msVmNew();
        // First create a value via eval
        let expr = CString::new("42").unwrap();
        let val = msEval(vm, expr.as_ptr());
        assert!(!val.is_null());

        // Set it as a global
        let name = CString::new("myvar").unwrap();
        let status = msSetGlobal(vm, name.as_ptr(), val);
        assert_eq!(status, MsStatus::MS_OK);

        // Retrieve it
        let val2 = msGetGlobal(vm, name.as_ptr());
        assert!(!val2.is_null());
        msValueFree(val2);

        // val was moved into msSetGlobal (cloned internally), but the pointer
        // still needs freeing since msSetGlobal clones, not consumes.
        msValueFree(val);

        msVmFree(vm);
    }
}
