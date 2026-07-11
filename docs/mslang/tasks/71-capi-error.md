# C API — 异常处理

## 所属阶段
Phase 6 — 模块系统 + 标准库

## 前置任务
65-capi-infrastructure, 66-capi-vm, 68-capi-value-convert, 70-capi-call

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
include/mslang/mslang.h  — 取消 error.h 注释
```

依赖（前置任务已完成）:
```
src/capi/vm.rs       — MsVM、lock_vm、VmInner
src/capi/types.rs     — MsValue、MsStatus
src/capi/call.rs      — msCall（task 70，msTry 复用）
src/vm/object.rs      — MsException、TypeTag::EXCEPTION、alloc_exception、read_exception
src/vm/mod.rs         — VM.has_error / VM.error_message
```

### 1. 错误状态机制 — 复用 VM.has_error / VM.error_message

不新增 `CapiError` 结构体或 `pending_error` 字段。Task 68 已在 `VM` 上
引入 `has_error: bool` + `error_message: String`（`src/vm/mod.rs:141-146`，
`#[cfg(feature = "capi")]`），作为 C API 错误状态的统一存储。
Task 71 的全部 `msThrow*` / `msErr*` 函数直接读写这两个字段。

| 操作 | 代码 |
|---|---|
| 设置错误 | `inner.vm.has_error = true; inner.vm.error_message = format!(...)` |
| 检查错误 | `inner.vm.has_error` |
| 清除错误 | `inner.vm.has_error = false; inner.vm.error_message.clear()` |

> **set_type_error 迁移**：`src/capi/mod.rs:32-43` 的占位函数 `set_type_error`
> 标注 "Task 71 完成后由 msThrowTypeError 取代"。本任务完成后，`set_type_error`
> 的调用方（value.rs 中的类型错误设置）改为调用 `msThrowTypeError`，
> `set_type_error` 函数可移除或改为 `msThrowTypeError` 的 thin wrapper。

异常对象（msErrFetch 返回值）使用已有的 `MsException` 堆对象
（`TypeTag::EXCEPTION`，`src/vm/object.rs:771-777`）：
`class_name: String`, `message: Object`, `traceback: Object`, `cause: Object`。

### 2. 异常查询函数 — src/capi/error.rs

#### msErrOccurred

```rust
#[no_mangle]
pub extern "C" fn msErrOccurred(vm: *mut MsVM) -> c_int {
    if vm.is_null() {
        return MS_FALSE;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    if inner.vm.has_error { MS_TRUE } else { MS_FALSE }
}
```

#### msErrFetch

取出错误状态，构建 `MsException` 堆对象（`TypeTag::EXCEPTION`）返回。

```rust
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

    // 解析 error_message → type_name + message
    // VM 错误格式通常为 "TypeName: message" 或纯 message
    let (type_name, message) = parse_error_message(&inner.vm.error_message);

    // 重置错误状态
    inner.vm.has_error = false;
    inner.vm.error_message.clear();

    // 构建 MsException 堆对象
    let exc = alloc_exception(
        &type_name,
        alloc_string(&message),
        alloc_string(""),
        Object::Nil,
    );
    Box::into_raw(Box::new(MsValue { inner: exc }))
}

/// 从 error_message 解析 type_name 和 message。
/// 格式 "TypeName: message" → ("TypeName", "message")；
/// 无前缀 → ("Error", full_message)。
fn parse_error_message(msg: &str) -> (String, String) {
    if let Some(colon) = msg.find(": ") {
        (msg[..colon].to_string(), msg[colon + 2..].to_string())
    } else {
        ("Error".to_string(), msg.to_string())
    }
}
```

#### msErrClear

```rust
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
```

### 3. 异常对象属性函数

四个属性函数检查 `MsValue.inner` 是否为 `Object::Ref` 且 `type_tag == TypeTag::EXCEPTION`，
然后用 `read_exception(ptr)` 提取字段。返回的 `const char*` 指向 MsException 堆对象
的 String 字段内部缓冲器，只要 MsValue\* 有效（Ref 存活），指针有效。

#### msErrTypeName

```rust
#[no_mangle]
pub extern "C" fn msErrTypeName(_vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    if err.is_null() {
        return std::ptr::null();
    }
    let val = unsafe { &*err };
    match &val.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            exc.class_name.as_ptr() as *const c_char
        }
        _ => std::ptr::null(),
    }
}
```

> **生命周期**：`exc.class_name` 是 `MsException` 堆对象的 `String` 字段。
> 只要 MsValue\* 有效（Ref 指向的 MsException 未被 GC 回收），String 存活，
> `as_ptr()` 有效。C 侧应在 MsValue\* 存活期间使用返回的指针。

#### msErrMessage / msErrTraceback

```rust
#[no_mangle]
pub extern "C" fn msErrMessage(_vm: *mut MsVM, err: *mut MsValue) -> *const c_char {
    if err.is_null() { return std::ptr::null(); }
    match &unsafe { &*err }.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            // message 为 Object，提取 String
            string_object_to_cstr(&exc.message)
        }
        _ => std::ptr::null(),
    }
}
```

`msErrTraceback` 同理访问 `exc.traceback`。

`string_object_to_cstr` 辅助函数：从 `Object::Ref` (TypeTag::STRING) 提取
`*const c_char`，非 String 类型返回 null。

#### msErrCause

```rust
#[no_mangle]
pub extern "C" fn msErrCause(_vm: *mut MsVM, err: *mut MsValue) -> *mut MsValue {
    if err.is_null() { return std::ptr::null_mut(); }
    match &unsafe { &*err }.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            match &exc.cause {
                Object::Nil => std::ptr::null_mut(),
                cause_obj => {
                    // 返回借用引用（新 Box<MsValue>，不注册 GC root）
                    Box::into_raw(Box::new(MsValue { inner: cause_obj.clone() }))
                }
            }
        }
        _ => std::ptr::null_mut(),
    }
}
```

### 4. C 侧抛出异常

所有 `msThrow*` 设置 `inner.vm.has_error = true` +
`inner.vm.error_message = "TypeName: message"`，返回 `MS_ERROR`。

错误消息格式统一为 `"TypeName: message"`，使 `msErrFetch` 的
`parse_error_message` 能正确拆分 type_name 和 message。

#### msThrow

```rust
#[no_mangle]
pub extern "C" fn msThrow(
    vm: *mut MsVM,
    type_: *const c_char,
    fmt: *const c_char,
) -> MsStatus {
    if vm.is_null() || type_.is_null() || fmt.is_null() {
        return MsStatus::MS_ERROR;
    }
    let type_name = unsafe { CStr::from_ptr(type_).to_string_lossy().into_owned() };
    // MVP：fmt 直接作为消息文本（va_list 不可用，见下文注释）
    let message = unsafe { CStr::from_ptr(fmt).to_string_lossy().into_owned() };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.has_error = true;
    inner.vm.error_message = format!("{}: {}", type_name, message);

    MsStatus::MS_ERROR
}
```

> **va_list 限制**：Rust stable 不支持在 `extern "C" fn(...)` 中提取
> C 可变参数。与 Task 67 的 `msStringFmt` 同策略：MVP 阶段 `fmt` 直接
> 作为消息文本（无 `%d` / `%s` 格式化替换）。后续可通过 C shim 文件
> （如 `vsnprintf_shim.c`）在 C 侧预格式化后传入。

#### msThrowValue

```rust
#[no_mangle]
pub extern "C" fn msThrowValue(vm: *mut MsVM, err: *mut MsValue) -> MsStatus {
    if vm.is_null() || err.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    match &unsafe { &*err }.inner {
        // err 已是 MsException 堆对象：提取 type_name + message
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            let type_name = exc.class_name.clone();
            let message = object_to_string(&exc.message);
            inner.vm.has_error = true;
            inner.vm.error_message = format!("{}: {}", type_name, message);
        }
        // 非 Exception：将值转为字符串作为 message
        other => {
            inner.vm.has_error = true;
            inner.vm.error_message = format!("Error: {:?}", other);
        }
    }

    MsStatus::MS_ERROR
}
```

#### msThrowRethrow

重新抛出当前已有错误（保持 has_error = true）：

```rust
#[no_mangle]
pub extern "C" fn msThrowRethrow(vm: *mut MsVM) -> MsStatus {
    if vm.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    // 若无 pending error，设置一个默认错误
    if !inner.vm.has_error {
        drop(guard);
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner.vm.has_error = true;
        inner.vm.error_message = "Error: rethrow with no pending error".into();
    }
    // 已有错误保持不变
    MsStatus::MS_ERROR
}
```

#### 便捷 throw 函数

六个便捷函数设置不同的 `type_name`，统一使用 `has_error` + `error_message`：

| 函数 | type_name | message 格式 |
|---|---|---|
| `msThrowTypeError(vm, expected, actual)` | `"TypeError"` | `"expected {expected}, got {actual}"` |
| `msThrowValueError(vm, fmt)` | `"ValueError"` | `CStr::from_ptr(fmt)` |
| `msThrowIndexError(vm, fmt)` | `"IndexError"` | `CStr::from_ptr(fmt)` |
| `msThrowKeyError(vm, key)` | `"KeyError"` | `msToString(vm, key)` |
| `msThrowRuntimeError(vm, fmt)` | `"RuntimeError"` | `CStr::from_ptr(fmt)` |
| `msThrowIoError(vm, fmt)` | `"IOError"` | `CStr::from_ptr(fmt)` |

公共辅助函数：

```rust
fn set_capi_error(vm: *mut MsVM, type_name: &str, message: &str) -> MsStatus {
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.has_error = true;
    inner.vm.error_message = format!("{}: {}", type_name, message);
    MsStatus::MS_ERROR
}
```

`msThrowTypeError` 实现：

```rust
#[no_mangle]
pub extern "C" fn msThrowTypeError(
    vm: *mut MsVM,
    expected: *const c_char,
    actual: *const c_char,
) -> MsStatus {
    if vm.is_null() || expected.is_null() || actual.is_null() {
        return MsStatus::MS_ERROR;
    }
    let exp = unsafe { CStr::from_ptr(expected).to_string_lossy() };
    let act = unsafe { CStr::from_ptr(actual).to_string_lossy() };
    set_capi_error(vm, "TypeError", &format!("expected {}, got {}", exp, act))
}
```

`msThrowKeyError` 特殊：接收 `MsValue* key`：

```rust
#[no_mangle]
pub extern "C" fn msThrowKeyError(vm: *mut MsVM, key: *mut MsValue) -> MsStatus {
    if vm.is_null() || key.is_null() {
        return MsStatus::MS_ERROR;
    }
    // key 的字符串表示作为 message（简化：直接 Debug 格式）
    let key_str = format!("{:?}", unsafe { &(*key).inner });
    set_capi_error(vm, "KeyError", &key_str)
}
```

其余四个（`msThrowValueError`/`msThrowIndexError`/`msThrowRuntimeError`/`msThrowIoError`）
模式相同：接收 `(vm, fmt)`，用 `set_capi_error` 设置不同 `type_name`。

### 5. msTry — try/catch 模式

msTry 内部调用 `msCall`（Task 70），检查 `has_error` 判断成功/失败。

```rust
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

    // 清除之前的错误状态
    {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        inner.vm.has_error = false;
        inner.vm.error_message.clear();
    }

    // 调用函数（复用 Task 70 的 msCall）
    let ret = msCall(vm, func, args, nargs);

    // 检查是否产生异常
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
```

> **与 Task 70 协调**：`msCall` 在函数执行失败时已设置
> `inner.vm.has_error = true` + `inner.vm.error_message = msg`（`src/capi/call.rs:60-64`）。
> msTry 直接复用此机制，无需额外转换。

### 6. object_to_string 辅助函数

从 `Object`（通常为 `Object::Ref` 指向 String 堆对象）提取 `String`：

```rust
fn object_to_string(obj: &Object) -> String {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            let s = unsafe { read_str(*ptr) };
            s.data().to_owned()
        }
        Object::Int(n) => n.to_string(),
        Object::Float(f) => f.to_string(),
        Object::Bool(b) => b.to_string(),
        Object::Nil => "nil".to_string(),
        _ => format!("{:?}", obj),
    }
}
```

`string_object_to_cstr` 从 `Object` 提取 `*const c_char`：

```rust
fn string_object_to_cstr(obj: &Object) -> *const c_char {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            let s = unsafe { read_str(*ptr) };
            s.data().as_ptr() as *const c_char
        }
        _ => std::ptr::null(),
    }
}
```

### 7. mslang.h 更新

Task 65 中 `include/mslang/mslang.h` 的 `error.h` include 行取消注释：

```c
#include "error.h"
```

### 8. 与脚本侧异常系统的对接

VM 执行脚本时抛出异常，`call_function` / `interpret` 返回 `Err(String)`。
Task 70 的 `msCall` 已将 `Err(msg)` 桥接为 `inner.vm.has_error = true` +
`inner.vm.error_message = msg`。因此：

- VM 错误字符串格式通常为 `"TypeError: expected X, got Y"` 或
  `"ValueError: invalid value"`，与 `msErrFetch` 的 `parse_error_message` 兼容。
- `msThrow*` 设置的错误格式为 `"TypeName: message"`，与 VM 错误格式一致。
- `msTry` 调用 `msCall` 后检查 `has_error`，自然桥接 VM 侧和 C 侧异常。

> **GC root 注意**：`msErrFetch` 返回的 MsValue\*（含 MsException Ref）
> 由调用方负责 root/unroot 或及时使用后 `msValueFree`。

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
