# C API — 函数调用

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure（cbindgen + 手写类型头文件 + 构建集成）
- 66-capi-vm（VM 生命周期 C API：msVmNew、msVmFree、msExecString、msGetGlobal 等）
- 68-capi-value-convert（值创建、类型判断、转换 C API：msInt、msToString 等）

## 目标

实现 call.h 的同步调用部分：`msCall` 核心函数、`msCall0`–`msCall3` 便捷宏、C 函数类型（`MsCFunction`）桥接机制。使 C 程序能调用 mslang 脚本定义的函数，以及 mslang 脚本能调用 C 注册的原生函数。

> **范围限制**：`13-capi.md` call.h 还包含异步调用部分
> （`msCallAsync`、`msAwait`、`msFutureResolve/Reject`、`MsAsyncFunction` 等）。
> 异步部分依赖 Phase 7 并发特性（task 53-55），属于
> [76-capi-async-channel](76-capi-async-channel.md)，本任务不实现。

## 设计规格

参照 [13-capi](../13-capi.md) § call.h — 同步调用。

### msCall

```c
MS_API MsValue* msCall(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
```

- `func` 必须是可调用对象（Function、Closure、BoundMethod、C 原生函数，或带 `__call__` 的 Instance）
- 返回函数返回值（新引用），异常时返回 NULL
- 线程安全：内部获取 VM 互斥锁
- 可调用性判断由 VM `call_value` 内部 match 自然处理，C API 层不预检

### 便捷宏

```c
#define msCall0(vm, f)                        msCall(vm, f, NULL, 0)
#define msCall1(vm, f, a)                     /* 见实现细节 */
#define msCall2(vm, f, a, b)                  /* 见实现细节 */
#define msCall3(vm, f, a, b, c)               /* 见实现细节 */
```

cbindgen 无法生成 C 宏。这些宏需手写在 `include/mslang/call_macros.h` 中，由 `call.h` 末尾 `#include "call_macros.h"` 引入。

### MsCFunction 类型

已在 [65-capi-infrastructure](./65-capi-infrastructure.md) 的 `types.h` 中定义：

```c
typedef MsValue* (*MsCFunction)(MsVM* vm, MsValue* const* args, int nargs);
```

C 函数指针签名。返回 `MsValue*`（新引用），异常时调用 `msThrow*` 并返回 NULL。

## 实现细节

### 文件结构

```
变更文件:
  src/vm/object.rs                   // TypeTag::NATIVE_C_FUNCTION = 21
  src/vm/builtins.rs                 // MsCNativeFunction + alloc/read 辅助函数
  src/vm/mod.rs                      // VM.capi_vm_ptr + call_value 新增分支
  src/vm/gc.rs                       // NATIVE_C_FUNCTION TypeDescriptor 注册
  src/capi/types.rs                  // MsCFunction 类型别名
  src/capi/vm.rs                     // msVmNew 设置 capi_vm_ptr 反向指针
  src/capi/call.rs                   // msCall + msMakeCFunction 实现
  include/mslang/call.h              // cbindgen 生成（msCall 函数声明）
  include/mslang/call_macros.h       // 手写（msCall0-msCall3 宏）
  include/mslang/mslang.h            // 取消 call.h 注释

依赖（前置任务已完成）:
  src/capi/vm.rs                      // MsVM 包装、msExecString、msGetGlobal
  src/capi/value.rs                   // MsValue 包装、msInt、msToString
  src/vm/mod.rs                       // VM 内部结构（stack、call_stack、globals）
  src/vm/frame.rs                     // CallFrame
  src/vm/object.rs                    // Object、MsObjHeader、TypeTag、Function、Closure
```

### 1. TypeTag 扩展 — NATIVE_C_FUNCTION

在 `src/vm/object.rs` 的 TypeTag 枚举中新增变体。注意：值 `16`–`20` 已被
JOIN_HANDLE / UPVALUE / EXCEPTION / EXCEPTION_CLASS / FILE_HANDLE 占用，
下一个可用值为 **21**。

```rust
// src/vm/object.rs — 在 FILE_HANDLE = 20 之后、LARGE_OBJECT 之前新增：
    NATIVE_C_FUNCTION = 21,           // C 原生函数（task 70 新增）
```

完整枚举（修复后）：

```rust
#[repr(u8)]
enum TypeTag {
    STRING       = 1,
    LIST         = 2,
    DICT         = 3,
    TUPLE        = 4,
    SET          = 5,
    FUNCTION     = 6,        // Rust 原生函数 + 用户函数元数据
    CLOSURE      = 7,
    CLASS        = 8,
    INSTANCE     = 9,
    MODULE       = 10,
    ITERATOR     = 11,
    GENERATOR    = 12,
    FUTURE       = 13,
    CHANNEL      = 14,
    BOUND_METHOD = 15,
    JOIN_HANDLE  = 16,
    UPVALUE      = 17,
    EXCEPTION    = 18,
    EXCEPTION_CLASS = 19,
    FILE_HANDLE  = 20,
    NATIVE_C_FUNCTION = 21,           // 本任务新增：C extern "C" 原生函数
    LARGE_OBJECT = 0xFF,
}
```

> **MsType 不新增**：`NATIVE_C_FUNCTION` 为内部 TypeTag，不暴露到 C 侧
> `MsType` 枚举（与 UPVALUE/EXCEPTION/EXCEPTION_CLASS/FILE_HANDLE 同策略，
> 见 `67-capi-value-creation.md:439` 的 fallback 注释）。C 侧通过
> `msIsFunction` 判断可调用性即可。

> **14-gc.md 规格回写**：`14-gc.md:89-112` 的 TypeTag 表须同步补
> `NATIVE_C_FUNCTION = 21`。

### 2. MsCNativeFunction 堆对象 — src/vm/builtins.rs

C 原生函数签名 `MsCFunction`（已在 `types.h:49` 定义）与现有 Rust 原生函数签名
`NativeFn = fn(&mut VM, &[Object]) -> Result<Object, String>`（`builtins.rs:23`）
不兼容，因此使用独立堆对象 `MsCNativeFunction`（`TypeTag::NATIVE_C_FUNCTION`）。

```rust
use crate::capi::types::MsCFunction;

/// C 原生函数堆对象布局（TypeTag::NATIVE_C_FUNCTION）。
#[repr(C)]
pub struct MsCNativeFunction {
    pub header: MsObjHeader,
    pub name_ptr: *const u8,
    pub name_len: u32,
    pub func: MsCFunction,          // Option<extern "C" fn ...>（可为 NULL）
    pub arity: i32,                 // -1 = 可变参数
}

impl MsCNativeFunction {
    pub fn name(&self) -> &str {
        unsafe {
            let slice = std::slice::from_raw_parts(self.name_ptr, self.name_len as usize);
            std::str::from_utf8_unchecked(slice)
        }
    }
}
```

分配辅助函数：

```rust
/// 分配 MsCNativeFunction 堆对象，返回 Object::Ref。
/// MVP：Box 分配（与 alloc_native_function 同策略）。
pub fn alloc_c_native_function(
    name: &str,
    func: MsCFunction,
    arity: i32,
) -> Object {
    let name_bytes = name.as_bytes();
    let name_box: Box<[u8]> = Box::from(name_bytes);
    let name_len = name_box.len() as u32;
    let name_ptr = Box::into_raw(name_box) as *const u8;

    let obj = Box::new(MsCNativeFunction {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::NATIVE_C_FUNCTION as u8,
            size:      std::mem::size_of::<MsCNativeFunction>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        name_ptr,
        name_len,
        func,
        arity,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

pub unsafe fn read_c_native_function<'a>(ptr: *mut MsObjHeader) -> &'a MsCNativeFunction {
    &*(ptr as *const MsCNativeFunction)
}
```

> **注意**：`MsCFunction` 类型为 `Option<extern "C" fn(...)>`（因 C 侧可为 NULL）。
> 在 `src/capi/types.rs` 中需增加类型别名：
> ```rust
> pub type MsCFunction = Option<extern "C" fn(
>     *mut MsVM, *const *mut MsValue, i32,
> ) -> *mut MsValue>;
> ```

> **GC trace 注册**：`MsCNativeFunction` 持有 `name_ptr: *const u8`（Box<[u8]>
> 分配的堆内存）。须在 `src/vm/gc.rs` 的 TypeDescriptor 表新增
> `NATIVE_C_FUNCTION` 条目，trace 函数扫描 `name_ptr`/`name_len` 指向的
> Box<[u8]>（非 GC 对象，但需 dealloc）。MVP（Box 分配 + STW GC）阶段：
> TypeDescriptor 注册 `name` 的 dealloc（回收 Box<[u8]>），
> trace 为空（无 Object 引用字段 — func/arity 为原始类型）。
> 参照 task 52 的 `MsNativeFunction` TypeDescriptor 注册模式。
> 对象回收时需同时释放 `name_ptr` 的 Box<[u8]> 和 MsCNativeFunction 自身的 Box。

### 3. VM MsVM 反向指针 — src/vm/mod.rs + src/capi/vm.rs

CALL 指令调用 C 函数时需将 `MsVM*` 传给 C 侧（C 函数通过它调用 ms\* API）。
当前 VM 无 MsVM 反向指针，需新增。

在 `VM` 结构体中新增字段：

```rust
pub struct VM {
    // ... 现有字段 ...
    /// C API 句柄（MsVM*）。运行于 C API 上下文时由 msVmNew 设置；
    /// 纯 Rust 调用时为 null。CALL 的 NATIVE_C_FUNCTION 分支使用此指针。
    pub capi_vm_ptr: *mut u8,
}
```

`VM::new()` 中初始化为 `std::ptr::null_mut()`。

`msVmNew` 中分配 MsVM 后设置反向指针：

```rust
pub extern "C" fn msVmNew() -> *mut MsVM {
    let inner = VmInner {
        vm: crate::vm::VM::new(),
        // ... 其他字段 ...
    };
    let vm = Box::new(MsVM {
        inner: ReentrantMutex::new(UnsafeCell::new(inner)),
    });
    let ptr = Box::into_raw(vm);
    // 设置反向指针
    let guard = lock_vm(ptr);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.capi_vm_ptr = ptr as *mut u8;
    drop(guard);
    ptr
}
```

### 4. call_value 新增 NATIVE_C_FUNCTION 分支 — src/vm/mod.rs

在 `call_value`（`src/vm/mod.rs:729`）的现有 GENERATOR / FUNCTION / CLOSURE /
BOUND_METHOD / EXCEPTION_CLASS 分支之后，新增 NATIVE_C_FUNCTION 分支。

该分支将栈上的 `Object` 参数转换为 `MsValue*` 数组传给 C 函数，
C 函数返回 `MsValue*`（NULL 表示异常）。

```rust
Object::Ref(ptr) if unsafe { (**ptr).type_tag }
    == TypeTag::NATIVE_C_FUNCTION as u8 =>
{
    let (c_func, arity, name) = {
        let cnf = unsafe { read_c_native_function(*ptr) };
        (cnf.func, cnf.arity, cnf.name().to_owned())
    };

    // arity 校验（-1 = 可变参数，跳过）
    if arity >= 0 && arity as usize != argc {
        return Err(format!(
            "TypeError: {}() takes exactly {} argument{} but {} were given",
            name, arity, if arity == 1 { "" } else { "s" }, argc,
        ));
    }

    // Object → MsValue*（Box 分配，调用后立即释放）
    let arg_vals: Vec<Box<MsValue>> = self.stack[self.stack.len() - argc..]
        .iter()
        .map(|obj| Box::new(MsValue { inner: obj.clone() }))
        .collect();
    let arg_ptrs: Vec<*mut MsValue> =
        arg_vals.iter().map(|b| b.as_ref() as *const MsValue as *mut MsValue).collect();

    // 弹出 callee + args
    self.stack.truncate(callee_idx);

    let c_fn = c_func.ok_or("TypeError: null C function pointer")?;
    let result_ptr = c_fn(
        self.capi_vm_ptr as *mut MsVM,
        arg_ptrs.as_ptr(),
        argc as i32,
    );

    // arg_vals 在此 drop（释放 Box<MsValue> 包装，不影响堆 Object）

    if result_ptr.is_null() {
        // C 函数通过 msThrow* 设置了异常（has_error + error_message）
        return Err(self.error_message.clone());
    }

    // MsValue* → Object（克隆后释放 Box 包装）
    let result = unsafe { (*result_ptr).inner.clone() };
    unsafe { drop(Box::from_raw(result_ptr)); }
    self.push(result)?;
}
```

> **安全说明**：`arg_vals` 持有 `Box<MsValue>` 的所有权，C 函数返回后立即 drop。
> C 函数内部如需长期持有参数引用，应通过 `msRoot` 注册。结果 `MsValue*`
> 由 C 函数通过 `Box::into_raw` 分配，本分支 `Box::from_raw` 回收。

### 5. msCall 实现 — src/capi/call.rs

msCall 复用 VM 已有的 `call_function` 方法（`src/vm/mod.rs:1046`），
该方法完整处理 CLOSURE/FUNCTION/BOUND_METHOD 的压栈、call_value、
嵌套 run_loop、结果弹出。msCall 仅负责 MsValue\* ↔ Object 转换和错误桥接。

```rust
use std::os::raw::c_int;
use crate::capi::types::MsValue;
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::object::Object;

#[no_mangle]
pub extern "C" fn msCall(
    vm: *mut MsVM,
    func: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
) -> *mut MsValue {
    // NULL 安全
    if vm.is_null() || func.is_null() {
        return std::ptr::null_mut();
    }
    // nargs 负值防护
    if nargs < 0 {
        return std::ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    // MsValue* → Object
    let func_obj = unsafe { (*func).inner.clone() };

    // 构建 arg Objects（NULL arg_ptr 视为错误）
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

    // 委托 VM::call_function 处理所有 callable 类型
    // call_function 内部：push callee+args → call_value → run_loop → pop result
    // call_value 已含 NATIVE_C_FUNCTION 分支（§4）
    match inner.vm.call_function(&func_obj, &arg_objects) {
        Ok(result) => {
            Box::into_raw(Box::new(MsValue { inner: result }))
        }
        Err(msg) => {
            inner.vm.has_error = true;
            inner.vm.error_message = msg;
            std::ptr::null_mut()
        }
    }
}
```

> **可调用性由 call_value 判断**：不在 msCall 层预检 callable 类型，
> 而是 push 后由 `call_value` 的 match 自然分派。不可调用对象落入
> catch-all `_ =>` 分支返回 `Err("not a callable object")`。
> 这覆盖 CLOSURE / FUNCTION / BOUND_METHOD / NATIVE_C_FUNCTION /
> GENERATOR / EXCEPTION_CLASS，以及 Instance `__call__`（经 call_value
> 现有逻辑或 GET_ATTR 查找 `__call__` 魔术方法）。

> **Instance `__call__`**：当前 call_value 未显式处理 INSTANCE 的
> `__call__` 魔术方法。若 func_obj 为 INSTANCE，call_value 会走 catch-all
> 返回错误。完整 `__call__` 支持需在 call_value 增加 INSTANCE 分支
>（查 `__call__` 方法后递归调用）。MVP 阶段可标注为已知限制，
> 完整实现在后续 task 补全。

### 6. VM 辅助方法 — 无需新增

msCall 直接复用 `VM::call_function`（`src/vm/mod.rs:1046-1061`），
该方法已完整实现 callable 调用逻辑：

```rust
pub fn call_function(&mut self, callee: &Object, args: &[Object])
    -> Result<Object, String>
{
    self.push(callee.clone())?;
    for arg in args { self.push(arg.clone())?; }
    let caller_depth = self.call_stack.len();
    self.call_value(args.len())?;
    if self.call_stack.len() > caller_depth {
        self.run_loop(Some(caller_depth))?;
    }
    self.pop()
}
```

内部 `call_value` 的 match 分派自动处理所有 callable TypeTag，
包括 §4 新增的 NATIVE_C_FUNCTION 分支。无需新增 `execute_call_from_capi`。

### 7. C 函数注册桥接 — src/capi/call.rs

提供将 `MsCFunction` 指针包装为 VM 可调用对象的辅助函数：

```rust
use std::os::raw::c_int;
use crate::capi::types::MsValue;
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::builtins::alloc_c_native_function;

#[no_mangle]
pub extern "C" fn msMakeCFunction(
    vm: *mut MsVM,
    name: *const std::os::raw::c_char,
    func: crate::capi::types::MsCFunction,
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
```

> **cbindgen 排除**：`msMakeCFunction` 标记为 `#[no_mangle] pub extern "C"`，
> cbindgen 默认会生成声明到 `call.h`。若需排除，在 `cbindgen.toml` 的
> `[export] exclude` 中添加 `"msMakeCFunction"`。或者接受公开声明
> （C 侧可调用，但文档标注为内部辅助）。Module 注册 API
> （`msModuleAddFunc`）在 task 72 中实现，内部调用此函数。

### 8. call_macros.h — 手写便捷宏

文件：`include/mslang/call_macros.h`

```c
#ifndef MS_CALL_MACROS_H
#define MS_CALL_MACROS_H

#ifdef __GNUC__
#define msCall1(vm, f, a) __extension__ ({                    \
    MsValue* _args[] = {(a)};                                  \
    msCall((vm), (f), _args, 1);                               \
})
#define msCall2(vm, f, a, b) __extension__ ({                  \
    MsValue* _args[] = {(a), (b)};                             \
    msCall((vm), (f), _args, 2);                               \
})
#define msCall3(vm, f, a, b, c) __extension__ ({               \
    MsValue* _args[] = {(a), (b), (c)};                        \
    msCall((vm), (f), _args, 3);                               \
})
#else
#define msCall1(vm, f, a) msCall((vm), (f), (MsValue* const[]){(a)}, 1)
#define msCall2(vm, f, a, b) msCall((vm), (f), (MsValue* const[]){(a), (b)}, 2)
#define msCall3(vm, f, a, b, c) msCall((vm), (f), (MsValue* const[]){(a), (b), (c)}, 3)
#endif

#endif /* MS_CALL_MACROS_H */
```

cbindgen 生成的 `call.h` 末尾追加：

```c
#include "call_macros.h"
```

通过 `cbindgen.toml` 的 `trailer` 配置或 `build.rs` 后处理实现。

### 9. mslang.h 更新

取消 `call.h` 的注释：

```c
#ifndef MSLANG_H
#define MSLANG_H

#include "types.h"

#include "vm.h"
#include "value.h"
#include "call.h"
/* 以下头文件后续任务启用：
 * #include "error.h"
 * #include "module.h"
 * #include "class.h"
 * #include "gc.h"
 */

#endif /* MSLANG_H */
```

### 10. MsVM 内部访问模式

本任务的 C API 函数遵循已有 task 66–69 的统一访问模式：

```rust
let guard = lock_vm(vm);                         // 加锁（ReentrantMutex）
let inner = unsafe { &mut *guard.get() };        // VmInner 可变引用
// inner.vm  → &mut crate::vm::VM
// inner.vm.globals() / inner.vm.stack / inner.vm.call_function(...) 等
```

MsValue ↔ Object 转换：

| 方向 | 代码 |
|---|---|
| Object → `MsValue*` | `Box::into_raw(Box::new(MsValue { inner: obj }))` |
| `MsValue*` → Object | `unsafe { (*ptr).inner.clone() }` |
| 释放 `MsValue*` | `unsafe { drop(Box::from_raw(ptr)); }` 或 `msValueFree(ptr)` |

设置错误状态（替代 msThrow\*，task 71 前的占位）：

```rust
inner.vm.has_error = true;
inner.vm.error_message = format!("TypeError: ...");
```

## 验证标准

1. `msCall` 能调用 mslang 脚本定义的全局函数并正确返回结果
2. `msCall` 能调用 mslang 闭包（捕获的变量可正常访问）
3. `msCall0` 可调用零参数函数
4. `msCall1`/`msCall2`/`msCall3` 正确传递 1/2/3 个参数
5. `msCall` 调用抛出异常的函数时返回 NULL，异常可通过 `msErrFetch` 获取
6. `msCall` 传入非可调用对象时返回 NULL
7. `msCall` 对 NULL vm 或 NULL func 返回 NULL
8. C 原生函数（MsCFunction via msMakeCFunction）可从 mslang 脚本中被 CALL 指令调用
9. C 原生函数的 C 实现可通过 `msThrow*` 向 mslang 抛出异常
10. 递归 mslang 函数通过 `msCall` 调用工作正常
11. `msCall` 线程安全（per-VM 互斥锁保护）
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

    #[test]
    fn test_call_script_function() {
        // 定义 fn add(a, b) { return a + b }
        // msExecString 编译脚本
        // msGetGlobal 获取 add 函数
        // msCall2(vm, add, msInt(3), msInt(4)) 返回 7
        // 验证 msToInt(result) == 7
    }

    #[test]
    fn test_call_zero_args() {
        // 定义 fn fortytwo() { return 42 }
        // msCall0(vm, fortytwo) 返回 42
    }

    #[test]
    fn test_call_with_exception() {
        // 定义 fn boom() { throw "error" }
        // msCall0(vm, boom) 返回 NULL
        // msErrOccurred(vm) == MS_TRUE
        // msErrFetch(vm) 获取异常对象
    }

    #[test]
    fn test_call_closure() {
        // 定义:
        //   fn make_adder(x) {
        //       return fn(y) { return x + y }
        //   }
        //   adder = make_adder(10)
        // msCall1(vm, adder, msInt(5)) 返回 15
    }

    #[test]
    fn test_call_non_callable() {
        // msCall(vm, msInt(42), NULL, 0) 返回 NULL
    }

    #[test]
    fn test_call_null_args() {
        // msCall(NULL, ..., ..., 0) 返回 NULL
        // msCall(vm, NULL, ..., 0) 返回 NULL
    }

    #[test]
    fn test_native_function_bridge() {
        // 通过 msMakeCFunction 注册 C 函数 mul(a, b) → a * b
        // msSetGlobal 将其注册为全局变量
        // mslang 脚本调用 mul(3, 7)
        // 验证返回 21
    }

    #[test]
    fn test_native_function_throws() {
        // 通过 msMakeCFunction 注册 C 函数 check(x)：
        //   if x < 0: msThrowValueError(vm, "negative")
        //   else: return x
        // mslang 脚本调用 check(-1) 并 try/catch
        // 验证捕获到 "negative" 异常
    }

    #[test]
    fn test_recursive_call() {
        // 定义 fn fib(n) { if n <= 1 { return n } return fib(n-1) + fib(n-2) }
        // msCall1(vm, fib, msInt(10)) 返回 55
    }
}
```

### C 集成测试 — test_call.c

```c
#include <mslang.h>
#include <stdio.h>
#include <assert.h>

int main(void) {
    MsVM* vm = msVmNew();

    const char* script =
        "fn add(a, b) {\n"
        "  return a + b\n"
        "}\n"
        "fn greet() {\n"
        "  return \"hello\"\n"
        "}\n"
        "fn boom() {\n"
        "  throw \"exploded\"\n"
        "}\n";

    assert(msExecString(vm, script, "test.ms") == MS_OK);

    MsValue* add_fn = msGetGlobal(vm, "add");
    msRoot(vm, add_fn);

    MsValue* a = msInt(3);
    MsValue* b = msInt(4);
    MsValue* result = msCall2(vm, add_fn, a, b);

    assert(result != NULL);
    assert(msToInt(vm, result) == 7);

    msUnroot(vm, result);
    msUnroot(vm, a);
    msUnroot(vm, b);

    MsValue* greet_fn = msGetGlobal(vm, "greet");
    msRoot(vm, greet_fn);
    MsValue* greet_result = msCall0(vm, greet_fn);
    assert(greet_result != NULL);
    assert(strcmp(msToString(vm, greet_result), "hello") == 0);
    msUnroot(vm, greet_result);
    msUnroot(vm, greet_fn);

    MsValue* boom_fn = msGetGlobal(vm, "boom");
    msRoot(vm, boom_fn);
    MsValue* boom_result = msCall0(vm, boom_fn);
    assert(boom_result == NULL);
    assert(msErrOccurred(vm));
    MsValue* err = msErrFetch(vm);
    assert(err != NULL);
    msUnroot(vm, err);
    msUnroot(vm, boom_fn);

    msUnroot(vm, add_fn);
    msVmFree(vm);

    printf("test_call: all passed\n");
    return 0;
}
```

编译与运行：

```bash
cc -I include -o test_call test_call.c -L target/debug -lmslang
./test_call
```
