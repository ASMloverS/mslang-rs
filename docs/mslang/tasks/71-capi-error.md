# C API — 异常处理

## 所属阶段
Phase 6 — 模块系统 + 标准库

## 前置任务
65-capi-infrastructure, 66-capi-vm, 68-capi-value-convert

## 目标
实现 `error.h` 全部 API：异常查询（`msErrOccurred`/`msErrFetch`/`msErrClear`）、异常对象属性访问（`msErrTypeName`/`msErrMessage`/`msErrTraceback`/`msErrCause`）、C 侧抛出异常（`msThrow`/`msThrowValue`/`msThrowRethrow` 及六个便捷函数）、try/catch 模式（`msTry`）。使 C 扩展模块能够抛出和捕获 mslang 异常，与脚本侧异常系统无缝对接。

## 设计规格

参照 [13-capi](../13-capi.md) § error.h：

### 异常查询

```c
MS_API int      msErrOccurred(MsVM* vm);
MS_API MsValue* msErrFetch(MsVM* vm);
MS_API void     msErrClear(MsVM* vm);
```

| 函数 | 返回值 | 说明 |
|---|---|---|
| `msErrOccurred` | `MS_TRUE`/`MS_FALSE` | 是否有待处理异常 |
| `msErrFetch` | `MsValue*`（新引用）或 `NULL` | 取出异常对象并清除 pending 状态 |
| `msErrClear` | 无 | 清除异常（不获取对象） |

### 异常对象属性

```c
MS_API const char* msErrTypeName(MsVM* vm, MsValue* err);
MS_API const char* msErrMessage(MsVM* vm, MsValue* err);
MS_API const char* msErrTraceback(MsVM* vm, MsValue* err);
MS_API MsValue*    msErrCause(MsVM* vm, MsValue* err);
```

全部返回借用引用，仅在 `err` 存活期间有效。

### 从 C 抛出异常

```c
MS_API MsStatus msThrow(MsVM* vm, const char* type, const char* fmt, ...);
MS_API MsStatus msThrowValue(MsVM* vm, MsValue* err);
MS_API MsStatus msThrowRethrow(MsVM* vm);

MS_API MsStatus msThrowTypeError(MsVM* vm, const char* expected, const char* actual);
MS_API MsStatus msThrowValueError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowIndexError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowKeyError(MsVM* vm, MsValue* key);
MS_API MsStatus msThrowRuntimeError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowIoError(MsVM* vm, const char* fmt, ...);
```

所有 `msThrow*` 始终返回 `MS_ERROR`，可直接 `return`。设置 VM 的 pending error 后返回。

### try/catch 模式

```c
MS_API MsStatus msTry(MsVM* vm, MsValue* func, MsValue* const* args, int nargs,
    MsValue** result);
```

| 情况 | 返回值 | `*result` | pending error |
|---|---|---|---|
| 函数正常返回 | `MS_OK` | 返回值（新引用） | 无 |
| 函数抛出异常 | `MS_ERROR` | `NULL` | 保留（可用 `msErrFetch` 获取） |

## 实现细节

### 文件结构

```
src/capi/error.rs    — 全部 error.h API 实现
```

### 1. VmInner 扩展 — pending_error 字段

在 Task 66（`src/capi/vm.rs`）定义的 `VmInner` 中追加 `pending_error`：

```rust
struct VmInner {
    // ... Task 66 已有字段 ...
    pending_error: Option<Box<CapiError>>,
}
```

`CapiError` 定义（`src/capi/error.rs`）：

```rust
struct CapiError {
    type_name: String,
    message: String,
    traceback: String,
    cause: Option<NonNull<MsValueInner>>,
}
```

初始化时 `pending_error = None`。`VmInner` 由 `Mutex` 保护，`pending_error` 的读写自动线程安全。

### 2. 异常查询函数

#### msErrOccurred

```rust
#[no_mangle]
pub extern "C" fn msErrOccurred(vm: *mut MsVM) -> c_int {
    let vm = unsafe { &mut *vm };
    let inner = vm.inner.lock().unwrap();
    if inner.pending_error.is_some() {
        MS_TRUE
    } else {
        MS_FALSE
    }
}
```

#### msErrFetch

```rust
#[no_mangle]
pub extern "C" fn msErrFetch(vm: *mut MsVM) -> *mut MsValue {
    let vm = unsafe { &mut *vm };
    let mut inner = vm.inner.lock().unwrap();
    match inner.pending_error.take() {
        Some(err) => {
            let val = error_to_value(vm, err);
            val as *mut MsValue
        }
        None => std::ptr::null_mut(),
    }
}
```

`error_to_value` 将 `CapiError` 转换为 `MsValue`（创建 `MsValueInner::Error` 变体），注册为 GC root，返回新引用。转换后 `pending_error` 被清空（`take()`）。

#### msErrClear

```rust
#[no_mangle]
pub extern "C" fn msErrClear(vm: *mut MsVM) {
    let vm = unsafe { &mut *vm };
    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = None;
}
```

### 3. 异常对象属性函数

四个属性函数从 `MsValueInner::Error` 中提取字段，返回借用指针。

#### MsValueInner 扩展

在 Task 68 定义的 `MsValueInner` 枚举中追加：

```rust
enum MsValueInner {
    // ... 已有变体 ...
    Error {
        type_name: String,
        message: String,
        traceback: String,
        cause: Option<NonNull<MsValueInner>>,
    },
}
```

字符串字段存储在 `MsValueInner` 内部，确保指针稳定性（只要 `MsValueInner` 存活，`const char*` 有效）。

#### msErrTypeName

```rust
#[no_mangle]
pub extern "C" fn msErrTypeName(vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    let inner = unsafe { &*err }.inner();
    match inner {
        MsValueInner::Error { type_name, .. } => {
            type_name.as_ptr() as *const c_char
        }
        _ => std::ptr::null(),
    }
}
```

`msErrMessage`、`msErrTraceback` 同理，分别返回 `message` 和 `traceback` 的 `as_ptr()`。

#### msErrCause

```rust
#[no_mangle]
pub extern "C" fn msErrCause(_vm: *mut MsVM, err: *mut MsValue) -> *mut MsValue {
    let inner = unsafe { &*err }.inner();
    match inner {
        MsValueInner::Error { cause: Some(c), .. } => {
            // 将 NonNull<MsValueInner> 转回 *mut MsValue
            c.as_ptr() as *mut MsValue
        }
        _ => std::ptr::null_mut(),
    }
}
```

返回借用引用（不增加 root 计数）。

### 4. C 侧抛出异常

#### msThrow

```rust
#[no_mangle]
pub extern "C" fn msThrow(
    vm: *mut MsVM,
    type_: *const c_char,
    fmt: *const c_char,
    ...
) -> MsStatus {
    let vm = unsafe { &mut *vm };
    let type_name = unsafe { CStr::from_ptr(type_).to_string_lossy().into_owned() };

    let message = format_c_string(fmt); // va_list → String

    let error = CapiError {
        type_name,
        message,
        traceback: String::new(),
        cause: None,
    };

    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = Some(Box::new(error));

    MsStatus::MS_ERROR
}
```

`format_c_string` 通过 `va_list`（使用 `std::ffi::VaList` 或 `libc::va_list`）格式化可变参数，与 Task 67 中 `msStringFmt` 使用相同机制。若 `va_list` 不可用，退化为直接将 `fmt` 作为最终字符串（无格式化占位符替换）。

#### msThrowValue

```rust
#[no_mangle]
pub extern "C" fn msThrowValue(vm: *mut MsVM, err: *mut MsValue) -> MsStatus {
    let vm = unsafe { &mut *vm };
    let inner_val = unsafe { &*err }.inner();

    let error = match inner_val {
        MsValueInner::Error { type_name, message, traceback, cause } => {
            CapiError {
                type_name: type_name.clone(),
                message: message.clone(),
                traceback: traceback.clone(),
                cause: *cause,
            }
        }
        _ => {
            // 非 Error 类型：将值转为字符串作为 message
            CapiError {
                type_name: "Error".into(),
                message: format!("{:?}", inner_val),
                traceback: String::new(),
                cause: None,
            }
        }
    };

    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = Some(Box::new(error));

    MsStatus::MS_ERROR
}
```

#### msThrowRethrow

重新抛出当前 pending error（不清除）：

```rust
#[no_mangle]
pub extern "C" fn msThrowRethrow(vm: *mut MsVM) -> MsStatus {
    // pending_error 已存在，直接返回 MS_ERROR
    MsStatus::MS_ERROR
}
```

若当前无 pending error，行为等同于无操作（仍返回 `MS_ERROR`）。

#### 便捷 throw 函数

六个便捷函数内部调用相同的逻辑，仅 `type_name` 和 `message` 格式不同：

| 函数 | type_name | message 格式 |
|---|---|---|
| `msThrowTypeError(vm, expected, actual)` | `"TypeError"` | `"expected {expected}, got {actual}"` |
| `msThrowValueError(vm, fmt, ...)` | `"ValueError"` | `sprintf(fmt, ...)` |
| `msThrowIndexError(vm, fmt, ...)` | `"IndexError"` | `sprintf(fmt, ...)` |
| `msThrowKeyError(vm, key)` | `"KeyError"` | `msToString(vm, key)` |
| `msThrowRuntimeError(vm, fmt, ...)` | `"RuntimeError"` | `sprintf(fmt, ...)` |
| `msThrowIoError(vm, fmt, ...)` | `"IOError"` | `sprintf(fmt, ...)` |

`msThrowTypeError` 实现：

```rust
#[no_mangle]
pub extern "C" fn msThrowTypeError(
    vm: *mut MsVM,
    expected: *const c_char,
    actual: *const c_char,
) -> MsStatus {
    let vm = unsafe { &mut *vm };
    let exp = unsafe { CStr::from_ptr(expected).to_string_lossy() };
    let act = unsafe { CStr::from_ptr(actual).to_string_lossy() };
    let message = format!("expected {}, got {}", exp, act);

    let error = CapiError {
        type_name: "TypeError".into(),
        message,
        traceback: String::new(),
        cause: None,
    };

    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = Some(Box::new(error));

    MsStatus::MS_ERROR
}
```

`msThrowKeyError` 特殊：接收 `MsValue* key` 而非格式字符串：

```rust
#[no_mangle]
pub extern "C" fn msThrowKeyError(vm: *mut MsVM, key: *mut MsValue) -> MsStatus {
    let vm = unsafe { &mut *vm };
    let message = value_to_string_repr(vm, key);

    let error = CapiError {
        type_name: "KeyError".into(),
        message,
        traceback: String::new(),
        cause: None,
    };

    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = Some(Box::new(error));

    MsStatus::MS_ERROR
}
```

其余四个（`msThrowValueError`、`msThrowIndexError`、`msThrowRuntimeError`、`msThrowIoError`）模式相同：接收 `(vm, fmt, ...)` 可变参数，仅 `type_name` 不同。可提取公共辅助函数：

```rust
fn throw_with_type(
    vm: &mut MsVmBox,
    type_name: &str,
    fmt: *const c_char,
) -> MsStatus {
    let message = format_c_string(fmt);
    let error = CapiError {
        type_name: type_name.into(),
        message,
        traceback: String::new(),
        cause: None,
    };
    let mut inner = vm.inner.lock().unwrap();
    inner.pending_error = Some(Box::new(error));
    MsStatus::MS_ERROR
}
```

> 注：`format_c_string` 的 `va_list` 处理在 MSVC 和 GCC/Clang 上 ABI 不同。Windows MSVC 使用 `cargo::va_list` crate 或内联汇编获取 `va_list`；Linux/macOS 使用 `std::ffi::VaList`（nightly）或 `libc::va_list`。若不稳定，可退化为直接使用 `CStr::from_ptr(fmt)` 作为消息。

### 5. msTry — try/catch 模式

```rust
#[no_mangle]
pub extern "C" fn msTry(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
    result: *mut *mut MsValue,
) -> MsStatus {
    let vm_ref = unsafe { &mut *vm };

    // 清除之前的 pending error
    {
        let mut inner = vm_ref.inner.lock().unwrap();
        inner.pending_error = None;
    }

    // 调用函数（复用 Task 69 的 msCall 内部逻辑）
    let ret = capi_call(vm_ref, func, args, nargs);

    // 检查是否产生异常
    let mut inner = vm_ref.inner.lock().unwrap();
    if inner.pending_error.is_some() {
        unsafe { *result = std::ptr::null_mut() };
        MsStatus::MS_ERROR
    } else {
        unsafe { *result = ret };
        MsStatus::MS_OK
    }
}
```

`capi_call` 是 Task 69（`src/capi/call.rs`）中 `msCall` 的内部实现函数，`msTry` 和 `msCall` 共用。当 `capi_call` 内部检测到 VM 执行异常时，将异常写入 `pending_error` 并返回 `NULL`。

### 6. error_to_value — CapiError → MsValue 转换

```rust
fn error_to_value(vm: &mut MsVmBox, err: Box<CapiError>) -> *mut MsValue {
    let inner = MsValueInner::Error {
        type_name: err.type_name,
        message: err.message,
        traceback: err.traceback,
        cause: err.cause,
    };
    let boxed = Box::new(inner);
    let ptr = Box::into_raw(boxed) as *mut MsValue;

    // 注册为 GC root
    msRoot(vm, ptr);

    ptr
}
```

### 7. mslang.h 更新

Task 65 中 `include/mslang/mslang.h` 的 `error.h` include 行取消注释：

```c
#include "error.h"
```

### 8. 与脚本侧异常系统的对接

当 VM 执行脚本抛出异常（Task 37 try/except/finally 实现的 `Error` 对象），`capi_call` 需要捕获并将其转换为 `CapiError` 存入 `pending_error`。转换逻辑：

```rust
fn vm_error_to_capi(vm: &VmInner, vm_err: &VmError) -> CapiError {
    CapiError {
        type_name: vm_err.type_name().to_string(),
        message: vm_err.message().to_string(),
        traceback: vm_err.traceback().to_string(),
        cause: None, // 首层异常无 cause
    }
}
```

VM 侧异常对象（`src/vm/object.rs` 中的 `Error` 实例）的 `type`、`message`、`traceback` 属性映射到 `CapiError` 对应字段。

## 验证标准

1. `msErrOccurred` 初始返回 `MS_FALSE`
2. 调用 `msThrow*` 后 `msErrOccurred` 返回 `MS_TRUE`
3. `msErrFetch` 取出异常对象后 `msErrOccurred` 返回 `MS_FALSE`
4. `msErrMessage` 返回与 `msThrow` 传入的格式字符串一致的文本
5. `msThrowTypeError` 设置 `type_name` 为 `"TypeError"`
6. 六个便捷 throw 函数分别设置正确的 `type_name`
7. `msTry` 成功时返回 `MS_OK`，`*result` 为函数返回值
8. `msTry` 异常时返回 `MS_ERROR`，`*result` 为 `NULL`，`msErrOccurred` 返回 `MS_TRUE`
9. `msErrClear` 清除异常状态，之后 `msErrOccurred` 返回 `MS_FALSE`
10. `msThrowValue` 可将已有 `MsValue`（Error 类型）设为 pending error
11. `msErrCause` 对有 cause 链的异常返回正确的借用引用
12. `cargo build --features capi` 编译无错误
13. `cargo test --features capi` 全部通过

## 测试用例

### Rust 单元测试

```rust
#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    use super::*;
    use crate::capi::vm::*;
    use crate::capi::value::*;

    fn new_vm() -> *mut MsVM {
        msVmNew()
    }

    #[test]
    fn test_err_occurred_initially_false() {
        let vm = new_vm();
        assert_eq!(msErrOccurred(vm), MS_FALSE);
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_throw_and_catch() {
        let vm = new_vm();

        let c_type = b"ValueError\0".as_ptr() as *const c_char;
        let c_fmt = b"invalid value: %d\0".as_ptr() as *const c_char;
        let status = unsafe { msThrow(vm, c_type, c_fmt) };

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

        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_type_error() {
        let vm = new_vm();

        let expected = b"string\0".as_ptr() as *const c_char;
        let actual = b"int\0".as_ptr() as *const c_char;
        let status = unsafe { msThrowTypeError(vm, expected, actual) };

        assert_eq!(status, MsStatus::MS_ERROR);
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "TypeError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("expected string, got int"));

        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_value_error() {
        let vm = new_vm();
        let fmt = b"out of range\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrowValueError(vm, fmt) };

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "ValueError");
        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_index_error() {
        let vm = new_vm();
        let fmt = b"index 10 out of bounds\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrowIndexError(vm, fmt) };

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "IndexError");
        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_key_error() {
        let vm = new_vm();
        let key = msString(vm, b"missing_key\0".as_ptr() as *const c_char);
        let _ = unsafe { msThrowKeyError(vm, key) };

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "KeyError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("missing_key"));

        unsafe { msUnroot(vm, err) };
        unsafe { msUnroot(vm, key) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_runtime_error() {
        let vm = new_vm();
        let fmt = b"unexpected state\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrowRuntimeError(vm, fmt) };

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "RuntimeError");
        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_convenience_throw_io_error() {
        let vm = new_vm();
        let fmt = b"cannot open file\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrowIoError(vm, fmt) };

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "IOError");
        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_err_clear() {
        let vm = new_vm();

        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrow(vm, c_type, c_fmt) };
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        msErrClear(vm);
        assert_eq!(msErrOccurred(vm), MS_FALSE);

        // msErrFetch 应返回 NULL
        let err = msErrFetch(vm);
        assert!(err.is_null());

        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_err_fetch_returns_null_when_no_error() {
        let vm = new_vm();
        let err = msErrFetch(vm);
        assert!(err.is_null());
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_throw_value() {
        let vm = new_vm();

        // 先创建一个 error 对象
        let c_type = b"MyError\0".as_ptr() as *const c_char;
        let c_fmt = b"something went wrong\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrow(vm, c_type, c_fmt) };
        let err_obj = msErrFetch(vm);

        // 用 msThrowValue 重新抛出
        let _ = unsafe { msThrowValue(vm, err_obj) };
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err2 = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err2)).to_str().unwrap() };
        assert_eq!(type_str, "MyError");

        unsafe { msUnroot(vm, err_obj) };
        unsafe { msUnroot(vm, err2) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_err_traceback_empty() {
        let vm = new_vm();
        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrow(vm, c_type, c_fmt) };

        let err = msErrFetch(vm);
        let tb = msErrTraceback(vm, err);
        // C 侧抛出的异常 traceback 为空字符串
        let tb_str = unsafe { CStr::from_ptr(tb).to_str().unwrap() };
        assert_eq!(tb_str, "");

        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_err_cause_none() {
        let vm = new_vm();
        let c_type = b"Error\0".as_ptr() as *const c_char;
        let c_fmt = b"test\0".as_ptr() as *const c_char;
        let _ = unsafe { msThrow(vm, c_type, c_fmt) };

        let err = msErrFetch(vm);
        let cause = msErrCause(vm, err);
        assert!(cause.is_null());

        unsafe { msUnroot(vm, err) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_try_success() {
        let vm = new_vm();

        // 执行脚本定义一个简单函数
        let script = b"fn add(a, b) { return a + b }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let status = unsafe { msExecString(vm, script, filename) };
        assert_eq!(status, MsStatus::MS_OK);

        let func = msGetGlobal(vm, b"add\0".as_ptr() as *const c_char);
        assert!(!func.is_null());
        unsafe { msRoot(vm, func) };

        let a = msInt(3);
        let b = msInt(4);
        let args = [a, b];

        let mut result: *mut MsValue = std::ptr::null_mut();
        let try_status = unsafe {
            msTry(vm, func, args.as_ptr(), 2, &mut result)
        };

        assert_eq!(try_status, MsStatus::MS_OK);
        assert!(!result.is_null());
        assert_eq!(msToInt(vm, result), 7);

        unsafe { msUnroot(vm, result) };
        unsafe { msUnroot(vm, func) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_try_exception() {
        let vm = new_vm();

        // 定义一个会抛异常的函数
        let script = b"fn boom() { throw ValueError(\"boom\") }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let status = unsafe { msExecString(vm, script, filename) };
        assert_eq!(status, MsStatus::MS_OK);

        let func = msGetGlobal(vm, b"boom\0".as_ptr() as *const c_char);
        assert!(!func.is_null());
        unsafe { msRoot(vm, func) };

        let mut result: *mut MsValue = std::ptr::null_mut();
        let try_status = unsafe {
            msTry(vm, func, std::ptr::null(), 0, &mut result)
        };

        assert_eq!(try_status, MsStatus::MS_ERROR);
        assert!(result.is_null());
        assert_eq!(msErrOccurred(vm), MS_TRUE);

        let err = msErrFetch(vm);
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "ValueError");

        let msg = unsafe { CStr::from_ptr(msErrMessage(vm, err)).to_str().unwrap() };
        assert!(msg.contains("boom"));

        unsafe { msUnroot(vm, err) };
        unsafe { msUnroot(vm, func) };
        unsafe { msVmFree(vm) };
    }

    #[test]
    fn test_nested_try() {
        let vm = new_vm();

        let script = b"fn outer() { throw RuntimeError(\"outer error\") }\0".as_ptr() as *const c_char;
        let filename = b"test.ms\0".as_ptr() as *const c_char;
        let _ = unsafe { msExecString(vm, script, filename) };

        let func = msGetGlobal(vm, b"outer\0".as_ptr() as *const c_char);
        unsafe { msRoot(vm, func) };

        // 外层 try
        let mut result1: *mut MsValue = std::ptr::null_mut();
        let s1 = unsafe { msTry(vm, func, std::ptr::null(), 0, &mut result1) };
        assert_eq!(s1, MsStatus::MS_ERROR);

        // 内层 try（在外层异常未处理的情况下再次 try）
        let mut result2: *mut MsValue = std::ptr::null_mut();
        let s2 = unsafe { msTry(vm, func, std::ptr::null(), 0, &mut result2) };
        assert_eq!(s2, MsStatus::MS_ERROR);

        // 内层异常可获取
        let err = msErrFetch(vm);
        assert!(!err.is_null());
        let type_str = unsafe { CStr::from_ptr(msErrTypeName(vm, err)).to_str().unwrap() };
        assert_eq!(type_str, "RuntimeError");

        unsafe { msUnroot(vm, err) };
        unsafe { msUnroot(vm, func) };
        unsafe { msVmFree(vm) };
    }
}
```

### 验证命令

```bash
cargo build --features capi
cargo test --features capi -- capi::error
```
