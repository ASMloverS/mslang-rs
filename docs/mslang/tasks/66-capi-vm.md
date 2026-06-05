# C API — VM 生命周期与配置

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure

## 目标

实现 `vm.h` 中定义的全部 C API 函数，覆盖 VM 创建/销毁、配置（模块路径、命令行参数、输出重定向）、脚本执行（`msExecFile`/`msExecString`/`msEval`）、全局变量操作、per-VM 互斥锁（API 内部自动加锁）。

## 设计规格

参照 [13-capi.md](../13-capi.md) § vm.h — VM 生命周期：

### 创建与销毁

```c
MS_API MsVM* msVmNew(void);
MS_API void  msVmFree(MsVM* vm);
```

每个 `MsVM` 拥有独立的全局作用域、模块缓存、GC 堆。不同 VM 实例可在不同线程中并行使用。

### 配置

```c
MS_API void msAddModulePath(MsVM* vm, const char* path);
MS_API void msSetArgs(MsVM* vm, int argc, const char** argv);

typedef int (*MsWriteFn)(const char* data, size_t len, void* userdata);
MS_API void msSetStdout(MsVM* vm, MsWriteFn fn, void* userdata);
MS_API void msSetStderr(MsVM* vm, MsWriteFn fn, void* userdata);
```

### 脚本执行

```c
MS_API MsStatus msExecFile(MsVM* vm, const char* path);
MS_API MsStatus msExecString(MsVM* vm, const char* source, const char* filename);
MS_API MsValue* msEval(MsVM* vm, const char* expr);
```

### 全局变量

```c
MS_API MsValue* msGetGlobal(MsVM* vm, const char* name);
MS_API MsStatus msSetGlobal(MsVM* vm, const char* name, MsValue* val);
MS_API void     msDelGlobal(MsVM* vm, const char* name);
```

### 线程安全

```c
MS_API void msVmLock(MsVM* vm);
MS_API void msVmUnlock(MsVM* vm);
```

### 设计原则（摘自 13-capi.md）

| 原则 | 说明 |
|---|---|
| 稳定 ABI | MsValue 等核心结构体完全隐藏，仅通过函数操作 |
| 线程安全 | per-VM 互斥锁，不同 VM 实例可并行 |
| 自动加锁 | 所有 `ms*` API 内部自动管理锁，`msVmLock`/`msVmUnlock` 仅在需要多步操作原子性时使用 |

## 实现细节

### 文件位置

- `src/capi/mod.rs` — C API 模块入口，声明子模块并 re-export
- `src/capi/vm.rs` — VM 生命周期、配置、执行、全局变量、锁
- `src/capi/types.rs` — MsValue 不透明类型、MsStatus 等枚举定义（由 65-capi-infrastructure 创建，本任务扩展）

### 模块声明

`src/lib.rs` 新增：

```rust
pub mod capi;
```

`src/capi/mod.rs`：

```rust
pub mod types;
pub mod vm;
```

### MsVM 不透明结构

`src/capi/vm.rs`：

```rust
use std::sync::Mutex;

#[repr(C)]
pub struct MsVM {
    inner: Mutex<VmInner>,
}

struct VmInner {
    vm: crate::vm::Vm,
    module_paths: Vec<String>,
    args: Vec<String>,
    stdout_cb: Option<WriteCallback>,
    stderr_cb: Option<WriteCallback>,
}

struct WriteCallback {
    fn_ptr: MsWriteFn,
    userdata: *mut std::ffi::c_void,
}

unsafe impl Send for WriteCallback {}
unsafe impl Sync for WriteCallback {}
```

`VmInner` 持有实际 VM 状态。`MsVM` 对外仅暴露不透明指针，C 侧无法访问内部字段。

### MsWriteFn 类型

```rust
type MsWriteFn = Option<extern "C" fn(data: *const i8, len: usize, userdata: *mut std::ffi::c_void) -> i32>;
```

与 13-capi.md 中 `typedef int (*MsWriteFn)(const char* data, size_t len, void* userdata)` 对应。

### MsValue 不透明类型

`src/capi/types.rs`（由 65-capi-infrastructure 定义基础框架，本任务确认接口）：

```rust
use crate::vm::object::Object;

#[repr(C)]
pub struct MsValue {
    inner: Object,
}
```

MsValue 在 C 侧为不透明指针 `MsValue*`，实际由 `Box<MsValue>` 经 `Box::into_raw` 转为裸指针返回给 C。

### msVmNew

```rust
#[no_mangle]
pub extern "C" fn msVmNew() -> *mut MsVM {
    let inner = VmInner {
        vm: crate::vm::Vm::new(),
        module_paths: Vec::new(),
        args: Vec::new(),
        stdout_cb: None,
        stderr_cb: None,
    };
    let vm = Box::new(MsVM {
        inner: Mutex::new(inner),
    });
    Box::into_raw(vm)
}
```

堆分配 `MsVM`，返回裸指针。VM 初始状态无模块路径、无参数、输出回调为 None（使用默认 stdout/stderr）。

### msVmFree

```rust
#[no_mangle]
pub extern "C" fn msVmFree(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    unsafe {
        let _ = Box::from_raw(vm);
    }
}
```

从裸指针恢复 `Box<MsVM>`，离开作用域时自动 drop。NULL 指针安全处理。

### msAddModulePath

```rust
#[no_mangle]
pub extern "C" fn msAddModulePath(vm: *mut MsVM, path: *const i8) {
    if vm.is_null() || path.is_null() {
        return;
    }
    let path_str = unsafe {
        std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned()
    };
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    inner.module_paths.push(path_str);
}
```

自动加锁 → 追加模块路径 → 自动解锁。

### msSetArgs

```rust
#[no_mangle]
pub extern "C" fn msSetArgs(vm: *mut MsVM, argc: i32, argv: *const *const i8) {
    if vm.is_null() {
        return;
    }
    let args = if argc > 0 && !argv.is_null() {
        (0..argc as usize)
            .map(|i| unsafe {
                std::ffi::CStr::from_ptr(*argv.add(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    } else {
        Vec::new()
    };
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    inner.args = args;
}
```

从 C 的 `argc/argv` 转为 `Vec<String>` 存储。

### msSetStdout / msSetStderr

```rust
#[no_mangle]
pub extern "C" fn msSetStdout(vm: *mut MsVM, fn_ptr: MsWriteFn, userdata: *mut std::ffi::c_void) {
    set_write_callback(vm, fn_ptr, userdata, |inner, cb| inner.stdout_cb = cb)
}

#[no_mangle]
pub extern "C" fn msSetStderr(vm: *mut MsVM, fn_ptr: MsWriteFn, userdata: *mut std::ffi::c_void) {
    set_write_callback(vm, fn_ptr, userdata, |inner, cb| inner.stderr_cb = cb)
}

fn set_write_callback(
    vm: *mut MsVM,
    fn_ptr: MsWriteFn,
    userdata: *mut std::ffi::c_void,
    setter: impl FnOnce(&mut VmInner, Option<WriteCallback>),
) {
    if vm.is_null() {
        return;
    }
    let cb = fn_ptr.map(|f| WriteCallback { fn_ptr: Some(f), userdata });
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    setter(&mut inner, cb);
}
```

将回调函数指针 + userdata 封装为 `WriteCallback`，存入 VmInner。`MsWriteFn` 为 `Option` 类型，传入 NULL 表示恢复默认输出。

### msExecFile

```rust
#[no_mangle]
pub extern "C" fn msExecFile(vm: *mut MsVM, path: *const i8) -> MsStatus {
    if vm.is_null() || path.is_null() {
        return MsStatus::MS_ERROR;
    }
    let path_str = unsafe {
        std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned()
    };
    let source = match std::fs::read_to_string(&path_str) {
        Ok(s) => s,
        Err(_) => return MsStatus::MS_ERROR,
    };
    exec_source(vm, &source, Some(&path_str))
}
```

读取文件内容，委托 `exec_source` 编译并执行。

### msExecString

```rust
#[no_mangle]
pub extern "C" fn msExecString(
    vm: *mut MsVM,
    source: *const i8,
    filename: *const i8,
) -> MsStatus {
    if vm.is_null() || source.is_null() {
        return MsStatus::MS_ERROR;
    }
    let source_str = unsafe {
        std::ffi::CStr::from_ptr(source).to_string_lossy().into_owned()
    };
    let filename_str = if filename.is_null() {
        None
    } else {
        Some(unsafe { std::ffi::CStr::from_ptr(filename).to_string_lossy().into_owned() })
    };
    exec_source(vm, &source_str, filename_str.as_deref())
}
```

`filename` 参数可选（可为 NULL），用于错误信息中标注来源文件。

### exec_source 内部函数

```rust
fn exec_source(vm: *mut MsVM, source: &str, filename: Option<&str>) -> MsStatus {
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    let tokens = match crate::lexer::Lexer::new(source).collect::<Result<Vec<_>, _>>() {
        Ok(t) => t,
        Err(_) => return MsStatus::MS_ERROR,
    };
    let ast = match crate::parser::Parser::new(tokens).parse() {
        Ok(a) => a,
        Err(_) => return MsStatus::MS_ERROR,
    };
    let chunk = match crate::compiler::Compiler::new().compile(&ast) {
        Ok(c) => c,
        Err(_) => return MsStatus::MS_ERROR,
    };
    match inner.vm.interpret(chunk) {
        Ok(_) => MsStatus::MS_OK,
        Err(_) => MsStatus::MS_ERROR,
    }
}
```

完整的编译执行管线：Lexer → Parser → Compiler → VM.interpret。加锁后在整个流程中持有锁，保证线程安全。

### msEval

```rust
#[no_mangle]
pub extern "C" fn msEval(vm: *mut MsVM, expr: *const i8) -> *mut MsValue {
    if vm.is_null() || expr.is_null() {
        return std::ptr::null_mut();
    }
    let expr_str = unsafe {
        std::ffi::CStr::from_ptr(expr).to_string_lossy().into_owned()
    };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    let tokens = match crate::lexer::Lexer::new(&expr_str).collect::<Result<Vec<_>, _>>() {
        Ok(t) => t,
        Err(_) => return std::ptr::null_mut(),
    };
    let ast = match crate::parser::Parser::new(tokens).parse_expression() {
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
```

编译表达式，执行，返回结果作为新的 `MsValue*`（所有权转移给 C 侧）。错误返回 NULL。

### msGetGlobal

```rust
#[no_mangle]
pub extern "C" fn msGetGlobal(vm: *mut MsVM, name: *const i8) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();
    match inner.vm.globals().get(&name_str) {
        Some(obj) => {
            let val = Box::new(MsValue { inner: obj.clone() });
            Box::into_raw(val)
        }
        None => std::ptr::null_mut(),
    }
}
```

返回全局变量的克隆作为新 `MsValue*`。不存在返回 NULL。

### msSetGlobal

```rust
#[no_mangle]
pub extern "C" fn msSetGlobal(
    vm: *mut MsVM,
    name: *const i8,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || name.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let value = unsafe { (*val).inner.clone() };
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    inner.vm.globals_mut().insert(name_str, value);
    MsStatus::MS_OK
}
```

### msDelGlobal

```rust
#[no_mangle]
pub extern "C" fn msDelGlobal(vm: *mut MsVM, name: *const i8) {
    if vm.is_null() || name.is_null() {
        return;
    }
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    inner.vm.globals_mut().remove(&name_str);
}
```

### msVmLock / msVmUnlock

```rust
#[no_mangle]
pub extern "C" fn msVmLock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let guard = vm_ref.inner.lock().unwrap();
    std::mem::forget(guard);
}

#[no_mangle]
pub extern "C" fn msVmUnlock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let guard = vm_ref.inner.lock().unwrap();
    drop(guard);
}
```

`msVmLock`：获取锁后通过 `mem::forget` 阻止 guard drop，保持锁持有状态。

`msVmUnlock`：重新获取锁（此时已持有，需要调整策略——见下方 ReentrantMutex 方案）。

**实际实现使用 `parking_lot::ReentrantMutex`**：

```rust
use parking_lot::ReentrantMutex;

#[repr(C)]
pub struct MsVM {
    inner: ReentrantMutex<VmInner>,
}
```

`ReentrantMutex` 允许同一线程多次获取锁。`msVmLock` 调用 `lock()` 获取并 `forget` guard；`msVmUnlock` 再次 `lock()` 获取（因同线程可重入）然后 drop。

更安全的方案：使用 `RawMutex` + 手动计数：

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(C)]
pub struct MsVM {
    inner: Mutex<VmInner>,
    lock_count: AtomicUsize,
}

#[no_mangle]
pub extern "C" fn msVmLock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let _guard = vm_ref.inner.lock().unwrap();
    std::mem::forget(_guard);
    vm_ref.lock_count.fetch_add(1, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn msVmUnlock(vm: *mut MsVM) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    if vm_ref.lock_count.load(Ordering::SeqCst) == 0 {
        return;
    }
    let guard = vm_ref.inner.lock().unwrap();
    drop(guard);
    vm_ref.lock_count.fetch_sub(1, Ordering::SeqCst);
}
```

> **决策**：优先使用 `parking_lot::ReentrantMutex`，避免手动锁计数。Cargo.toml 添加 `parking_lot` 依赖。如果团队倾向减少依赖，则使用 `std::sync::Mutex` + `lock_count` 方案。

### 线程安全策略总结

| 场景 | 行为 |
|---|---|
| 单线程单 VM | 所有 API 正常工作，无锁竞争 |
| 多线程共享 VM | `ms*` 函数内部自动加锁，保证串行访问 |
| 多线程多 VM | 每个 VM 有独立 Mutex，不同 VM 实例可并行 |
| 原子多步操作 | `msVmLock`/`msVmUnlock` 包裹多个 API 调用 |

### VmInner 中输出回调的使用

VM 执行 `print()` 等输出时，检查 `stdout_cb` / `stderr_cb` 是否已设置：

- 已设置：调用回调函数，传入数据和 userdata
- 未设置：使用默认 `stdout`/`stderr`（`print!` / `eprint!`）

此逻辑需要配合 VM 的 print 内置函数实现（`src/vm/builtins.rs` 中的 `print` 函数）。VM 需要提供一种机制让 builtins 获取当前输出目标——可通过在 VmInner 中增加 `output` 字段，或通过闭包注入。

### 与 65-capi-infrastructure 的关系

任务 65 提供 C API 基础设施：
- `src/capi/mod.rs` 模块结构
- `src/capi/types.rs` 中 MsValue、MsStatus、MsType 等基础类型定义
- `include/mslang/types.h` C 头文件生成框架
- Cargo.toml 中 `crate-type = ["cdylib", "rlib"]` 配置

本任务在 65 的基础上：
- 新增 `src/capi/vm.rs`
- 在 `include/mslang/` 中新增 `vm.h` 头文件
- 使用 65 定义的 MsValue、MsStatus 等类型

## 验证标准

1. `msVmNew()` 返回非 NULL 的 `MsVM*`
2. `msExecString` 执行合法脚本返回 `MS_OK`
3. `msExecString` 执行非法脚本返回 `MS_ERROR`，`msErrOccurred` 返回 `MS_TRUE`
4. `msSetGlobal` + `msGetGlobal` 往返正确传递值
5. `msVmFree` 释放合法指针不崩溃
6. `msVmFree(NULL)` 安全处理，不崩溃
7. 两个 `MsVM` 实例可独立使用，互不影响
8. `msSetStdout` 回调正确捕获 `print()` 输出
9. `msSetStderr` 回调正确捕获错误输出
10. `msAddModulePath` 添加的路径在模块搜索时生效
11. `msSetArgs` 设置的参数可被脚本通过 `os.args` 访问
12. `msExecFile` 正确读取并执行 `.ms` 文件
13. `msEval` 返回表达式的求值结果
14. `msEval` 对非法表达式返回 NULL
15. `msVmLock` / `msVmUnlock` 配合使用可保证多步操作原子性
16. 多线程并发调用同一 VM 的 API 不崩溃

## 测试用例

### Rust 单元测试

`src/capi/vm.rs` 中 `#[cfg(test)] mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
        let source = std::ffi::CString::new("x = 42").unwrap();
        let filename = std::ffi::CString::new("test.ms").unwrap();
        let status = unsafe {
            msExecString(vm, source.as_ptr(), filename.as_ptr())
        };
        assert_eq!(status, MsStatus::MS_OK);
        msVmFree(vm);
    }

    #[test]
    fn test_exec_string_error() {
        let vm = msVmNew();
        let source = std::ffi::CString::new("fn (").unwrap();
        let filename = std::ffi::CString::new("bad.ms").unwrap();
        let status = unsafe {
            msExecString(vm, source.as_ptr(), filename.as_ptr())
        };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_global_roundtrip() {
        let vm = msVmNew();

        // 执行脚本设置全局变量
        let source = std::ffi::CString::new("answer = 42").unwrap();
        let filename = std::ffi::CString::new("test.ms").unwrap();
        unsafe {
            msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        // 读取全局变量
        let name = std::ffi::CString::new("answer").unwrap();
        let val = unsafe { msGetGlobal(vm, name.as_ptr()) };
        assert!(!val.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_global_get_missing() {
        let vm = msVmNew();
        let name = std::ffi::CString::new("nonexistent").unwrap();
        let val = unsafe { msGetGlobal(vm, name.as_ptr()) };
        assert!(val.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_global_set_get() {
        let vm = msVmNew();

        let name = std::ffi::CString::new("x").unwrap();
        let source = std::ffi::CString::new("x = 1").unwrap();
        let filename = std::ffi::CString::new("test.ms").unwrap();
        unsafe {
            msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let val = unsafe { msGetGlobal(vm, name.as_ptr()) };
        assert!(!val.is_null());

        // 删除全局变量
        unsafe { msDelGlobal(vm, name.as_ptr()) };
        let val2 = unsafe { msGetGlobal(vm, name.as_ptr()) };
        assert!(val2.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_two_vms_independent() {
        let vm1 = msVmNew();
        let vm2 = msVmNew();

        let source = std::ffi::CString::new("x = 1").unwrap();
        let filename = std::ffi::CString::new("test.ms").unwrap();
        unsafe {
            msExecString(vm1, source.as_ptr(), filename.as_ptr());
        }

        let name = std::ffi::CString::new("x").unwrap();
        let val1 = unsafe { msGetGlobal(vm1, name.as_ptr()) };
        let val2 = unsafe { msGetGlobal(vm2, name.as_ptr()) };
        assert!(!val1.is_null());
        assert!(val2.is_null());

        msVmFree(vm1);
        msVmFree(vm2);
    }

    #[test]
    fn test_output_redirect() {
        use std::sync::{Arc, Mutex};

        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_ptr = Arc::into_raw(captured) as *mut std::ffi::c_void;

        extern "C" fn write_cb(
            data: *const i8,
            len: usize,
            userdata: *mut std::ffi::c_void,
        ) -> i32 {
            let captured = unsafe {
                &*(userdata as *const Arc<Mutex<Vec<u8>>>)
            };
            let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
            captured.lock().unwrap().extend_from_slice(bytes);
            0
        }

        let vm = msVmNew();
        unsafe {
            msSetStdout(vm, Some(write_cb), captured_ptr);
        }

        let source = std::ffi::CString::new("print(\"hello\")").unwrap();
        let filename = std::ffi::CString::new("test.ms").unwrap();
        unsafe {
            msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let captured = unsafe { Arc::from_raw(captured_ptr as *const Mutex<Vec<u8>>) };
        let data = captured.lock().unwrap();
        let output = String::from_utf8_lossy(&data);
        assert!(output.contains("hello"));

        msVmFree(vm);
    }

    #[test]
    fn test_add_module_path() {
        let vm = msVmNew();
        let path = std::ffi::CString::new("/test/path").unwrap();
        unsafe {
            msAddModulePath(vm, path.as_ptr());
        }
        msVmFree(vm);
    }

    #[test]
    fn test_set_args() {
        let vm = msVmNew();
        let arg0 = std::ffi::CString::new("ms").unwrap();
        let arg1 = std::ffi::CString::new("script.ms").unwrap();
        let argv = [arg0.as_ptr(), arg1.as_ptr()];
        unsafe {
            msSetArgs(vm, 2, argv.as_ptr());
        }
        msVmFree(vm);
    }

    #[test]
    fn test_vm_lock_unlock() {
        let vm = msVmNew();
        unsafe {
            msVmLock(vm);
            let name = std::ffi::CString::new("x").unwrap();
            let source = std::ffi::CString::new("x = 1").unwrap();
            let filename = std::ffi::CString::new("test.ms").unwrap();
            msExecString(vm, source.as_ptr(), filename.as_ptr());
            let val = msGetGlobal(vm, name.as_ptr());
            assert!(!val.is_null());
            msVmUnlock(vm);
        }
        msVmFree(vm);
    }
}
```

### C 集成测试

`tests/c/test_vm.c`：

```c
#include <mslang/vm.h>
#include <assert.h>
#include <string.h>

static char captured_buf[4096];
static size_t captured_len = 0;

static int write_capture(const char* data, size_t len, void* userdata) {
    memcpy(captured_buf + captured_len, data, len);
    captured_len += len;
    return 0;
}

void test_vm_new_free(void) {
    MsVM* vm = msVmNew();
    assert(vm != NULL);
    msVmFree(vm);
}

void test_exec_string(void) {
    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "x = 42", "test.ms");
    assert(s == MS_OK);
    msVmFree(vm);
}

void test_exec_string_error(void) {
    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "fn (", "bad.ms");
    assert(s == MS_ERROR);
    msVmFree(vm);
}

void test_output_redirect(void) {
    MsVM* vm = msVmNew();
    msSetStdout(vm, write_capture, NULL);
    MsStatus s = msExecString(vm, "print(\"hello\")", "test.ms");
    assert(s == MS_OK);
    assert(strstr(captured_buf, "hello") != NULL);
    msVmFree(vm);
}

void test_global_roundtrip(void) {
    MsVM* vm = msVmNew();
    msExecString(vm, "answer = 42", "test.ms");
    MsValue* val = msGetGlobal(vm, "answer");
    assert(val != NULL);
    msVmFree(vm);
}

void test_two_vms_independent(void) {
    MsVM* vm1 = msVmNew();
    MsVM* vm2 = msVmNew();
    msExecString(vm1, "x = 1", "test.ms");
    MsValue* val1 = msGetGlobal(vm1, "x");
    MsValue* val2 = msGetGlobal(vm2, "x");
    assert(val1 != NULL);
    assert(val2 == NULL);
    msVmFree(vm1);
    msVmFree(vm2);
}

int main(void) {
    test_vm_new_free();
    test_exec_string();
    test_exec_string_error();
    test_output_redirect();
    test_global_roundtrip();
    test_two_vms_independent();
    return 0;
}
```
