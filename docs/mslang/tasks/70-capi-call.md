# C API — 函数调用

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure（cbindgen + 手写类型头文件 + 构建集成）
- 66-capi-vm（VM 生命周期 C API：msVmNew、msVmFree、msExecString、msGetGlobal 等）
- 68-capi-value-convert（值创建、类型判断、转换 C API：msInt、msToString 等）

## 目标

实现 call.h 的同步调用部分：`msCall` 核心函数、`msCall0`–`msCall3` 便捷宏、C 函数类型（`MsCFunction`）桥接机制。使 C 程序能调用 mslang 脚本定义的函数，以及 mslang 脚本能调用 C 注册的原生函数。

## 设计规格

参照 [13-capi](../13-capi.md) § call.h — 同步调用。

### msCall

```c
MS_API MsValue* msCall(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
```

- `func` 必须是可调用对象（Function、Closure、BoundMethod，或带 `__call__` 的 Instance）
- 返回函数返回值（新引用），异常时返回 NULL
- 线程安全：内部获取 VM 互斥锁

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
  src/capi/call.rs                    // msCall 实现 + NativeFunction 桥接
  include/mslang/call.h               // cbindgen 生成（msCall 函数声明）
  include/mslang/call_macros.h        // 手写（msCall0-msCall3 宏）
  include/mslang/mslang.h             // 取消 call.h 注释

依赖（前置任务已完成）:
  src/capi/vm.rs                      // MsVM 包装、msExecString、msGetGlobal
  src/capi/value.rs                   // MsValue 包装、msInt、msToString
  src/vm/mod.rs                       // VM 内部结构（stack、call_stack、globals）
  src/vm/frame.rs                     // CallFrame
  src/vm/object.rs                    // Object、MsObjHeader、TypeTag、Function、Closure
```

### 1. TypeTag 扩展 — NATIVE_FUNCTION

在 `src/vm/object.rs` 的 TypeTag 枚举中新增变体：

```rust
#[repr(u8)]
enum TypeTag {
    STRING       = 1,
    LIST         = 2,
    DICT         = 3,
    TUPLE        = 4,
    SET          = 5,
    FUNCTION     = 6,
    CLOSURE      = 7,
    CLASS        = 8,
    INSTANCE     = 9,
    MODULE       = 10,
    ITERATOR     = 11,
    GENERATOR    = 12,
    FUTURE       = 13,
    CHANNEL      = 14,
    BOUND_METHOD = 15,
    NATIVE_FUNCTION = 16,             // 新增：C 原生函数
    LARGE_OBJECT = 0xFF,
}
```

同步更新 `include/mslang/types.h` 的 MsType 枚举：

```c
MS_TYPE_NATIVE_FUNCTION = 19,         // 新增，在 MS_TYPE_JOIN_HANDLE 之后
```

### 2. NativeFunction 堆对象 — src/vm/object.rs

```rust
#[repr(C)]
pub struct NativeFunction {
    pub header: MsObjHeader,
    pub name: String,
    pub func: unsafe extern "C" fn(
        *mut MsVM,                    // C 侧的 MsVM*（Rust 侧为 CApiVM 包装）
        *const *mut MsValue,          // MsValue* const* args
        i32,                          // nargs
    ) -> *mut MsValue,
    pub arity: i32,
}
```

分配辅助函数：

```rust
pub fn alloc_native_function(
    name: String,
    func: unsafe extern "C" fn(*mut MsVM, *const *mut MsValue, i32) -> *mut MsValue,
    arity: i32,
) -> Object {
    let obj = Box::new(NativeFunction {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::NATIVE_FUNCTION as u8,
            size:      std::mem::size_of::<NativeFunction>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        name,
        func,
        arity,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

pub unsafe fn read_native_function(ptr: *mut MsObjHeader) -> &NativeFunction {
    &*(ptr as *const NativeFunction)
}
```

### 3. VM CALL 指令扩展 — src/vm/mod.rs

在现有 CALL 指令的 match 分支中增加 NativeFunction 分支：

```rust
OpCode::CALL => {
    let argc = self.read_byte() as usize;
    let stack_top = self.stack.len();
    let callee_idx = stack_top - argc - 1;
    let callee = self.stack[callee_idx].clone();

    match callee {
        // ... 现有 FUNCTION / CLOSURE / BuiltinFunc 分支保持不变 ...

        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::NATIVE_FUNCTION as u8 => {
            let native = unsafe { read_native_function(ptr) };

            // 构建 MsValue* const* 参数数组
            let arg_ptrs: Vec<*mut MsValue> = self.stack[stack_top - argc..stack_top]
                .iter()
                .map(|obj| object_to_msvalue_ptr(obj))
                .collect();

            let result_ptr = unsafe {
                (native.func)(
                    vm_as_capi_ptr(self),         // VM 转为 C 侧不透明指针
                    arg_ptrs.as_ptr(),             // MsValue* const*
                    argc as i32,
                )
            };

            // 清理栈：弹出 callee + args
            self.stack.truncate(callee_idx);

            if result_ptr.is_null() {
                // C 函数抛出了异常（msThrow 设置了 VM 异常状态）
                return Err(RuntimeError::CapiException);
            }

            let result = msvalue_ptr_to_object(result_ptr);
            self.stack.push(result);
        }

        _ => return self.runtime_error("not a callable object"),
    }
}
```

### 4. msCall 实现 — src/capi/call.rs

```rust
use crate::vm::object::{Object, TypeTag, read_closure, read_function, read_native_function};
use crate::vm::frame::CallFrame;
use std::os::raw::c_int;

#[no_mangle]
pub unsafe extern "C" fn msCall(
    vm: *mut crate::capi::vm::CApiVM,
    func: *mut crate::capi::value::MsValue,
    args: *const *mut crate::capi::value::MsValue,
    nargs: c_int,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || func.is_null() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let _lock = capi_vm.mutex.lock();

    let func_obj = match capi_vm.unwrap_value(func) {
        Some(obj) => obj,
        None => return std::ptr::null_mut(),
    };

    // 检查可调用性
    let callable = match &func_obj {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
                || tag == TypeTag::NATIVE_FUNCTION as u8
                || tag == TypeTag::CLASS as u8
        }
        _ => false,
    };

    if !callable {
        capi_vm.set_error("not a callable object");
        return std::ptr::null_mut();
    }

    // 将 args 从 MsValue*[] 转为 Object[]
    let nargs_usize = nargs as usize;
    let arg_objects: Vec<Object> = if nargs_usize > 0 && !args.is_null() {
        std::slice::from_raw_parts(args, nargs_usize)
            .iter()
            .filter_map(|&arg_ptr| {
                if arg_ptr.is_null() { None } else { capi_vm.unwrap_value(arg_ptr) }
            })
            .collect()
    } else {
        Vec::new()
    };

    // 在 VM 栈上压入 callee + args
    let vm_inner = &mut capi_vm.vm;
    vm_inner.stack.push(func_obj);
    for arg in &arg_objects {
        vm_inner.stack.push(arg.clone());
    }

    // 执行 CALL
    let result = vm_inner.execute_call_from_capi(nargs_usize);

    match result {
        Ok(return_value) => {
            // 从栈上弹出返回值
            vm_inner.stack.pop();
            capi_vm.wrap_value(return_value)
        }
        Err(_) => {
            // 异常已在 VM 内部设置
            std::ptr::null_mut()
        }
    }
}
```

### 5. VM 辅助方法 — src/vm/mod.rs

在 VM 上新增方法，供 C API 调用使用：

```rust
impl VM {
    /// 从 C API 上下文发起函数调用。
    /// 调用前 callee 和 args 已压入栈。
    /// 执行完成后返回结果 Object（不弹出栈）。
    pub fn execute_call_from_capi(&mut self, argc: usize) -> Result<Object, ()> {
        let stack_top = self.stack.len();
        let callee_idx = stack_top - argc - 1;
        let callee = self.stack[callee_idx].clone();

        match callee {
            Object::Ref(ptr) => {
                let tag = unsafe { (*ptr).type_tag };

                if tag == TypeTag::CLOSURE as u8 {
                    let closure = unsafe { read_closure(ptr) };
                    let func = unsafe { read_function(closure.function) };
                    if argc != func.arity {
                        self.stack.truncate(callee_idx);
                        return Err(());
                    }
                    if self.call_stack.len() >= MAX_CALL_DEPTH {
                        self.stack.truncate(callee_idx);
                        return Err(());
                    }

                    // 保存当前帧状态
                    let current_frame = self.call_stack.last().unwrap();
                    let saved_ip = current_frame.ip;

                    // 创建新帧
                    self.call_stack.push(CallFrame::new(ptr, callee_idx));

                    // 执行直到 RETURN 回到当前帧深度
                    let original_depth = self.call_stack.len();
                    loop {
                        match self.run() {
                            Ok(()) => break,
                            Err(e) => {
                                return Err(());
                            }
                        }
                        if self.call_stack.len() < original_depth {
                            break;
                        }
                    }

                    // 返回值在栈顶
                    self.stack.last().cloned().ok_or(())
                } else if tag == TypeTag::FUNCTION as u8 {
                    // 类似 Closure 处理（无上值的函数）
                    // ...同上逻辑...
                    todo!("FUNCTION 分支镜像 CLOSURE 处理")
                } else if tag == TypeTag::NATIVE_FUNCTION as u8 {
                    let native = unsafe { read_native_function(ptr) };
                    let arg_ptrs: Vec<*mut crate::capi::value::MsValue> = Vec::new();
                    // NativeFunction 由 CALL 指令内部处理
                    // 此处不应走到，因为 C API 调用不经过字节码
                    Err(())
                } else {
                    self.stack.truncate(callee_idx);
                    Err(())
                }
            }
            _ => {
                self.stack.truncate(callee_idx);
                Err(())
            }
        }
    }
}
```

### 6. C 函数注册桥接 — src/capi/call.rs

提供将 `MsCFunction` 指针包装为 VM 可调用对象的辅助函数：

```rust
/// 将 C 函数指针包装为 NativeFunction 堆对象，返回 MsValue*。
/// 用于 module 注册（task 72）和 class 方法注册（task 73）。
#[no_mangle]
pub unsafe extern "C" fn msMakeCFunction(
    vm: *mut crate::capi::vm::CApiVM,
    name: *const std::os::raw::c_char,
    func: crate::capi::types::MsCFunction,
    arity: c_int,
) -> *mut crate::capi::value::MsValue {
    if vm.is_null() || name.is_null() || func.is_none() {
        return std::ptr::null_mut();
    }

    let capi_vm = &mut *vm;
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();

    let native_func = alloc_native_function(
        name_str,
        func.unwrap(),
        arity,
    );

    capi_vm.wrap_value(native_func)
}
```

> **注意**：`msMakeCFunction` 为内部辅助函数，不暴露在公共头文件中。Module 注册 API（`msModuleAddFunc`）在 task 72 中实现，内部调用此函数。

### 7. call_macros.h — 手写便捷宏

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

### 8. mslang.h 更新

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

### 9. CApiVM 内部结构

`src/capi/call.rs` 依赖 `CApiVM` 上的以下方法（由 task 66/68 定义）：

| 方法 | 说明 |
|---|---|
| `unwrap_value(MsValue*) -> Option<Object>` | 从 C 侧 MsValue* 提取内部 Object |
| `wrap_value(Object) -> *mut MsValue` | 将 Object 包装为 C 侧 MsValue* |
| `set_error(&str)` | 设置 VM 异常状态 |
| `mutex` | 内部 Mutex 字段 |
| `vm` | 内部 VM 实例引用 |

### 10. Object ↔ MsValue* 转换

`src/capi/value.rs`（task 68）提供以下转换函数，本任务直接复用：

```rust
/// Object → MsValue*（新引用，注册为 GC root）
fn object_to_msvalue_ptr(obj: &Object) -> *mut MsValue;

/// MsValue* → Object（借用引用，不增加引用计数）
fn msvalue_ptr_to_object(ptr: *mut MsValue) -> Object;
```

## 验证标准

1. `msCall` 能调用 mslang 脚本定义的全局函数并正确返回结果
2. `msCall` 能调用 mslang 闭包（捕获的变量可正常访问）
3. `msCall0` 可调用零参数函数
4. `msCall1`/`msCall2`/`msCall3` 正确传递 1/2/3 个参数
5. `msCall` 调用抛出异常的函数时返回 NULL，异常可通过 `msErrFetch` 获取
6. `msCall` 传入非可调用对象时返回 NULL
7. `msCall` 对 NULL vm 或 NULL func 返回 NULL
8. NativeFunction（MsCFunction）可从 mslang 脚本中被 CALL 指令调用
9. NativeFunction 的 C 实现可通过 `msThrow*` 向 mslang 抛出异常
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
        // 注册 C 函数 mul(a, b) → a * b
        // mslang 脚本调用 mul(3, 7)
        // 验证返回 21
    }

    #[test]
    fn test_native_function_throws() {
        // 注册 C 函数 check(x)：
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
