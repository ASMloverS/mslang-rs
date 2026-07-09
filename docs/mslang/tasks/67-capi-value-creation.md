# C API — 值创建与类型判断

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure（C API 基础设施：MsVM 结构体、互斥锁、不透明类型定义、capi 模块骨架）
- 66-capi-vm（VM 生命周期 C API：`src/capi/types.rs` 中 MsValue/MsType 定义、`src/capi/vm.rs` 中 `lock_vm` 辅助函数、`msValueFree`）

## 目标

实现 `value.h` 的前半部分 API，涵盖 C 侧值生命周期管理的核心能力：

1. **GC Root 注册**：`msRoot` / `msUnroot`，让 C 侧持有跨调用帧的值引用
2. **特殊值**：`msNil` / `msBoolVal`，提供 Nil 与 Bool 的 C 侧构造
3. **值创建**：`msInt` / `msFloat` / `msString` / `msStringn` / `msStringFmt`，从 C 原始类型构造 mslang 值
4. **集合创建**：`msListNew` / `msDictNew` / `msSetNew` / `msListFrom` / `msTupleFrom` / `msDictFrom`
5. **类型判断**：`msTypeof` 及全部 `msIs*` 函数，运行时类型查询

## 设计规格

参照 [13-capi](../13-capi.md) § value.h（前半：引用管理 + 特殊值 + 值创建 + 集合创建 + 类型判断）。

### 引用管理（GC Root 注册）

```c
MS_API MsValue* msRoot(MsVM* vm, MsValue* val);
MS_API void     msUnroot(MsVM* vm, MsValue* val);
```

- `msRoot(vm, val)`：将对象注册为 GC 根，返回 `val` 本身。注册后 GC 不会回收此对象。
- `msUnroot(vm, val)`：注销 GC 根。注销后对象可能被 GC 回收，C 侧不应再访问该指针。
- 仅对 Ref 类型（堆对象）有效。内联值（Nil/Bool/Int/Float）注册为 no-op，安全但无副作用。

**不需要 root 的场景**：
- API 返回值在当前调用帧内立即使用（如 `msToString` 提取 C 字符串后立刻使用）
- 值仅作为参数传递给其他 API 调用（调用期间 GC 不会回收参数）

### 特殊值

```c
MS_API MsValue* msNil(void);
MS_API MsValue* msBoolVal(int val);

#define MS_NIL       (msNil())
#define MS_TRUE_VAL  (msBoolVal(1))
#define MS_FALSE_VAL (msBoolVal(0))
```

单例值，不需要 root（但 root/unroot 安全）。`msBoolVal(0)` 返回 `MS_FALSE_VAL`，`msBoolVal(非零)` 返回 `MS_TRUE_VAL`。

### 值创建

```c
MS_API MsValue* msInt(int64_t val);
MS_API MsValue* msFloat(double val);

MS_API MsValue* msString(MsVM* vm, const char* str);
MS_API MsValue* msStringn(MsVM* vm, const char* str, size_t len);
MS_API MsValue* msStringFmt(MsVM* vm, const char* fmt, ...);
```

- `msInt` / `msFloat`：不依赖 VM，创建内联值
- `msString`：从 C 空终止字符串创建，等价于 `msStringn(vm, str, strlen(str))`
- `msStringn`：从指定长度的字节创建（可包含 `\0`）
- `msStringFmt`：printf 风格格式化创建字符串

### 集合创建

```c
MS_API MsValue* msListNew(MsVM* vm);
MS_API MsValue* msDictNew(MsVM* vm);
MS_API MsValue* msSetNew(MsVM* vm);

MS_API MsValue* msListFrom(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* msTupleFrom(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* msDictFrom(MsVM* vm, MsValue* const* pairs, int count);
```

- `msDictFrom` 的 `pairs` 是 key-value 扁平数组，`count` 为键值对数量（数组长度 = `count * 2`）
- 创建的集合自动注册到 VM 堆

### 类型判断

```c
MS_API MsType msTypeof(MsValue* val);

MS_API int msIsNil(MsValue* val);
MS_API int msIsBool(MsValue* val);
MS_API int msIsInt(MsValue* val);
MS_API int msIsFloat(MsValue* val);
MS_API int msIsNumber(MsValue* val);
MS_API int msIsString(MsValue* val);
MS_API int msIsList(MsValue* val);
MS_API int msIsDict(MsValue* val);
MS_API int msIsTuple(MsValue* val);
MS_API int msIsSet(MsValue* val);
MS_API int msIsFunction(MsValue* val);
MS_API int msIsClass(MsValue* val);
MS_API int msIsInstance(MsValue* val);
MS_API int msIsGenerator(MsValue* val);
MS_API int msIsFuture(MsValue* val);
MS_API int msIsChannel(MsValue* val);
```

所有 `msIs*` 函数返回 `MS_TRUE`（1）或 `MS_FALSE`（0）。`msIsNumber` 对 Int 和 Float 均返回 `MS_TRUE`。

### MsType 枚举

```c
typedef enum MsType {
  MS_TYPE_NIL = 0,
  MS_TYPE_BOOL,
  MS_TYPE_INT,
  MS_TYPE_FLOAT,
  MS_TYPE_STRING,
  MS_TYPE_LIST,
  MS_TYPE_DICT,
  MS_TYPE_TUPLE,
  MS_TYPE_SET,
  MS_TYPE_FUNCTION,
  MS_TYPE_CLASS,
  MS_TYPE_INSTANCE,
  MS_TYPE_MODULE,
  MS_TYPE_GENERATOR,
  MS_TYPE_FUTURE,
  MS_TYPE_CHANNEL,
  MS_TYPE_ITERATOR,
  MS_TYPE_BOUND_METHOD,
  MS_TYPE_JOIN_HANDLE,
} MsType;
```

## 实现细节

### 文件位置

| 文件 | 职责 |
|---|---|
| `src/capi/value.rs` | 值创建、类型判断的实现 |
| `src/capi/gc.rs` | GC Root 注册（`msRoot` / `msUnroot`）— 与 Task 65 的 gc.rs 分工一致 |
| `src/capi/mod.rs` | capi 模块入口（task 65/66 已声明 `pub mod value` / `pub mod gc`） |
| `include/mslang/value.h` | C 头文件（cbindgen 从 `src/capi/value.rs` 生成） |
| `src/capi/vsnprintf_shim.c` | `msStringFmt` 的 C va_list 辅助函数 |

> **文件归属说明**：`13-capi.md:205-227` 将 `msRoot`/`msUnroot` 放在 value.h 段，但 Task 65 决定将 GC 交互函数集中到独立的 `gc.rs`/`gc.h`（见 `65-capi-infrastructure.md:31`）。本任务遵循 Task 65 的决策，将 root 管理实现放在 `src/capi/gc.rs`。

### MsValue 内部表示

`MsValue` 已由 Task 66 在 `src/capi/types.rs` 中定义：

```rust
#[repr(C)]
pub struct MsValue {
    pub(crate) inner: Object,
}
```

本任务**不重复定义** MsValue，直接使用 Task 66 的定义。`inner` 字段为 `pub(crate)`，capi 模块内可访问。C 侧看到的 `MsValue*` 实际是 `*mut MsValue`（`Box::into_raw` 返回）。每个 API 函数负责：
- 创建值：`Box::into_raw(Box::new(MsValue { inner: obj }))` 返回裸指针
- 销毁值：由 GC 回收或 `msValueFree`（Task 66）手动回收

**内联值 vs 堆对象**：

| mslang 类型 | Rust Object 变体 | MsValue 指向 | GC 管理 |
|---|---|---|---|
| Nil | `Object::Nil` | Box\<MsValue\> | 不需要 |
| Bool | `Object::Bool(bool)` | Box\<MsValue\> | 不需要 |
| Int | `Object::Int(i64)` | Box\<MsValue\> | 不需要 |
| Float | `Object::Float(f64)` | Box\<MsValue\> | 不需要 |
| String | `Object::Ref(*mut MsObjHeader)` | Box\<MsValue\>（内含堆指针） | 需要 |
| List/Dict/... | `Object::Ref(*mut MsObjHeader)` | Box\<MsValue\>（内含堆指针） | 需要 |

> **设计说明**：C API 的 `MsValue*` 统一为堆分配的 Box\<MsValue\>，即使内联值也如此。这样 C 侧不需要区分指针类型，所有值统一通过 `MsValue*` 操作。对内联值 root/unroot 是安全的 no-op。
>
> **生命周期注意**：内联值（Nil/Bool/Int/Float）的 MsValue Box 不在 GC 堆上，必须由 C 侧调用 `msValueFree`（Task 66）释放，否则泄漏。Ref 类型同样需 `msValueFree` 释放 Box 外壳，并通过 `msRoot` 保护内部堆对象不被 GC 回收。

### msRoot / msUnroot 实现

> **文件位置**：`src/capi/gc.rs`（非 value.rs — 见文件归属说明）。

```rust
use crate::capi::vm::lock_vm;

#[no_mangle]
pub extern "C" fn msRoot(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || val.is_null() {
        return val;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    // 仅 Ref 类型需要注册为 GC root
    if let Object::Ref(header_ptr) = unsafe { &(*val).inner } {
        inner.vm.c_roots.insert(*header_ptr);
    }
    val
}

#[no_mangle]
pub extern "C" fn msUnroot(vm: *mut MsVM, val: *mut MsValue) {
    if vm.is_null() || val.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    if let Object::Ref(header_ptr) = unsafe { &(*val).inner } {
        inner.vm.c_roots.remove(header_ptr);
    }
}
```

`c_roots: HashSet<*mut MsObjHeader>` 是 `VM` 结构的字段（`11-bytecode-vm.md:320`），通过 `inner.vm.c_roots` 访问。Root 集合在 GC 标记阶段作为额外根集参与扫描。

> **GC forwarding 注意**：Minor GC 半空间复制会将 Young 代对象移动到新地址。GC 必须在复制后遍历 `c_roots`，将旧指针更新为 forwarding address。否则 GC 后 C 侧持有的 `MsObjHeader*` 将成为悬垂指针。

### 特殊值实现

```rust
#[no_mangle]
pub extern "C" fn msNil() -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { inner: Object::Nil }))
}

#[no_mangle]
pub extern "C" fn msBoolVal(val: c_int) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { inner: Object::Bool(val != 0) }))
}
```

每次调用创建新的 Box。由于 Nil/Bool 是内联值，无 GC 管理需求，重复分配的开销极低。

### 值创建实现

```rust
#[no_mangle]
pub extern "C" fn msInt(val: i64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { inner: Object::Int(val) }))
}

#[no_mangle]
pub extern "C" fn msFloat(val: f64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { inner: Object::Float(val) }))
}

#[no_mangle]
pub extern "C" fn msString(vm: *mut MsVM, str: *const c_char) -> *mut MsValue {
    if str.is_null() {
        return msStringn(vm, std::ptr::null(), 0);
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(str) }.to_bytes();
    msStringn(vm, str, bytes.len())
}

#[no_mangle]
pub extern "C" fn msStringn(vm: *mut MsVM, str: *const c_char, len: size_t) -> *mut MsValue {
    let bytes = if str.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(str as *const u8, len) }.to_vec()
    };
    let header = alloc_string_on_heap(vm, &bytes);
    Box::into_raw(Box::new(MsValue {
        inner: Object::Ref(header),
    }))
}
```

`alloc_string_on_heap` 封装堆分配逻辑：分配 `MsObjHeader` + 字节数据，设置 `type_tag = TypeTag::STRING`，注册到 VM 堆。

> **辅助函数签名**（在 `src/capi/value.rs` 或 `src/capi/mod.rs` 中定义）：
> ```rust
> /// 分配 String 堆对象，返回 MsObjHeader 裸指针。type_tag = STRING。
> fn alloc_string_on_heap(vm: *mut MsVM, bytes: &[u8]) -> *mut MsObjHeader;
> /// 分配通用堆对象，指定 type_tag 和总大小（含 header）。返回 MsObjHeader 裸指针。
> fn alloc_heap_object(vm: *mut MsVM, tag: TypeTag, total_size: usize) -> *mut MsObjHeader;
> ```
> 这些函数封装 Task 52（GC）的堆分配机制。MVP 阶段（GC 未接入日常分配时）可使用 `Box::into_raw` 简化实现。

### msStringFmt 实现

Rust stable 无法从 `extern "C" fn(...)` 的可变参数中读取 `va_list`（`std::ffi::VaList` 仍在 nightly）。采用 **纯 C 包装函数**方案：在 C 侧完成 `va_start/vsnprintf/va_end`，调用 Rust 的 `msStringn` 构造字符串。

> **注意**：`msStringFmt` 的 C 符号由 C 文件导出（而非 Rust `#[no_mangle]`），Rust 侧不提供 `msStringFmt` 的实现。C 包装函数调用 Rust 导出的 `msStringn`。

**C 包装函数**（`src/capi/vsnprintf_shim.c`）：

```c
#include <stdio.h>
#include <stdarg.h>
#include <string.h>

/* msStringn 由 Rust 侧 #[no_mangle] 导出 */
extern void* msStringn(void* vm, const char* str, size_t len);

void* msStringFmt(void* vm, const char* fmt, ...) {
    char stack_buf[1024];
    va_list ap;
    va_start(ap, fmt);
    int written = vsnprintf(stack_buf, sizeof(stack_buf), fmt, ap);
    va_end(ap);

    if (written < 0) {
        return msStringn(vm, "", 0);
    }

    size_t len = (size_t)written;
    if (len < sizeof(stack_buf)) {
        /* 结果在栈缓冲区内 */
        return msStringn(vm, stack_buf, len);
    }

    /* 结果超过 1024 字节，动态分配重试 */
    char* heap_buf = malloc(len + 1);
    if (!heap_buf) {
        return msStringn(vm, stack_buf, sizeof(stack_buf) - 1);
    }
    va_start(ap, fmt);
    vsnprintf(heap_buf, len + 1, fmt, ap);
    va_end(ap);
    void* result = msStringn(vm, heap_buf, len);
    free(heap_buf);
    return result;
}
```

**Rust 侧**（`src/capi/value.rs`）：

```rust
// msStringn 已由 #[no_mangle] 导出，C 侧可调用。
// msStringFmt 不在 Rust 侧定义——由 C 文件提供。
```

**构建系统**：`build.rs`（Task 65 已有框架）添加 `cc::Build` 编译 `vsnprintf_shim.c`：

```rust
#[cfg(feature = "capi")]
{
    cc::Build::new()
        .file("src/capi/vsnprintf_shim.c")
        .compile("mslang_capi_shim");
    // ... cbindgen 生成 ...
}
```

### 集合创建实现

```rust
#[no_mangle]
pub extern "C" fn msListNew(vm: *mut MsVM) -> *mut MsValue {
    let header = alloc_heap_object(vm, TypeTag::LIST, std::mem::size_of::<MsObjHeader>() + std::mem::size_of::<Vec<Object>>());
    unsafe {
        let data_ptr = (header as *mut u8).add(std::mem::size_of::<MsObjHeader>()) as *mut Vec<Object>;
        data_ptr.write(Vec::new());
    }
    Box::into_raw(Box::new(MsValue { inner: Object::Ref(header) }))
}
```

> **对齐注意**：`alloc_heap_object` 分配的内存必须按 `max(align_of::<MsObjHeader>(), align_of::<Vec<Object>>())`（通常 8 字节）对齐，否则 `data_ptr.write(Vec::new())` 构成未对齐写（UB）。
>
> **GC 注意**：`Vec<Object>` 内嵌于 MsObjHeader 之后，GC 半空间复制时需通过 TypeDescriptor 的 `copy_for_gc` 正确复制 Vec 的内部缓冲区（深拷贝裸指针指向的 HashMap/Vec 数据），否则会导致双重释放或悬垂指针。

```rust
#[no_mangle]
pub extern "C" fn msListFrom(vm: *mut MsVM, items: *const *mut MsValue, count: c_int) -> *mut MsValue {
    let list = msListNew(vm);
    if items.is_null() || count <= 0 {
        return list;
    }
    for i in 0..count as usize {
        let item = unsafe { *items.add(i) };
        if item.is_null() { continue; }
        let obj = unsafe { (*item).inner.clone() };
        list_push(vm, list, obj);
    }
    list
}
```

> **写屏障**：`list_push` 在向 List 堆对象写入 Ref 引用时必须调用写屏障（`13-capi.md:633-639`）。并发 GC 的 Concurrent Mark 阶段，未触发写屏障的写入可能破坏三色不变性，导致活跃对象被误回收。

`msTupleFrom` 结构类似，使用 `TypeTag::TUPLE` 并基于 `Vec<Object>`（不可变）。同样需对每个元素做 NULL 检查。

`msDictFrom` 的 `pairs` 参数为扁平 key-value 数组：

```rust
#[no_mangle]
pub extern "C" fn msDictFrom(vm: *mut MsVM, pairs: *const *mut MsValue, count: c_int) -> *mut MsValue {
    let dict = msDictNew(vm);
    if pairs.is_null() || count <= 0 {
        return dict;
    }
    for i in 0..count as usize {
        let key_idx = i.checked_mul(2).unwrap_or(usize::MAX);
        if key_idx == usize::MAX { break; }
        let key = unsafe { *pairs.add(key_idx) };
        let val = unsafe { *pairs.add(key_idx + 1) };
        if key.is_null() || val.is_null() { continue; }
        let key_obj = unsafe { (*key).inner.clone() };
        let val_obj = unsafe { (*val).inner.clone() };
        dict_insert(vm, dict, key_obj, val_obj);
    }
    dict
}
```

### 类型判断实现

```rust
fn obj_to_ms_type(obj: &Object) -> MsType {
    match obj {
        Object::Nil => MsType::Nil,
        Object::Bool(_) => MsType::Bool,
        Object::Int(_) => MsType::Int,
        Object::Float(_) => MsType::Float,
        Object::Ref(header) => {
            let tag = unsafe { (**header).type_tag };
            match tag {  // tag 为 u8，与 TypeTag 的 #[repr(u8)] 数值匹配
                t if t == TypeTag::STRING as u8       => MsType::String,
                t if t == TypeTag::LIST as u8         => MsType::List,
                t if t == TypeTag::DICT as u8         => MsType::Dict,
                t if t == TypeTag::TUPLE as u8        => MsType::Tuple,
                t if t == TypeTag::SET as u8          => MsType::Set,
                t if t == TypeTag::FUNCTION as u8     => MsType::Function,
                t if t == TypeTag::CLOSURE as u8      => MsType::Function,
                t if t == TypeTag::CLASS as u8        => MsType::Class,
                t if t == TypeTag::INSTANCE as u8     => MsType::Instance,
                t if t == TypeTag::MODULE as u8       => MsType::Module,
                t if t == TypeTag::GENERATOR as u8    => MsType::Generator,
                t if t == TypeTag::FUTURE as u8       => MsType::Future,
                t if t == TypeTag::CHANNEL as u8      => MsType::Channel,
                t if t == TypeTag::ITERATOR as u8     => MsType::Iterator,
                t if t == TypeTag::BOUND_METHOD as u8 => MsType::BoundMethod,
                t if t == TypeTag::JOIN_HANDLE as u8  => MsType::JoinHandle,
                _ => MsType::Nil,  // UPVALUE/EXCEPTION/EXCEPTION_CLASS/LARGE_OBJECT 无对应 MsType
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn msTypeof(val: *mut MsValue) -> MsType {
    if val.is_null() { return MsType::Nil; }
    obj_to_ms_type(unsafe { &(*val).inner })
}

#[no_mangle]
pub extern "C" fn msIsNil(val: *mut MsValue) -> c_int {
    if val.is_null() { return MS_FALSE; }
    if matches!(unsafe { &(*val).inner }, Object::Nil) { MS_TRUE } else { MS_FALSE }
}

#[no_mangle]
pub extern "C" fn msIsNumber(val: *mut MsValue) -> c_int {
    if val.is_null() { return MS_FALSE; }
    match unsafe { &(*val).inner } {
        Object::Int(_) | Object::Float(_) => MS_TRUE,
        _ => MS_FALSE,
    }
}
```

其余 `msIsBool` / `msIsInt` / `msIsFloat` / `msIsString` 等函数结构相同，匹配对应的 Object 变体或 TypeTag。`msIsFunction` 同时匹配 `FUNCTION` 和 `CLOSURE`。

### MS_TYPE_* / MS_TRUE / MS_FALSE 常量

MsType 枚举已由 Task 66 在 `src/capi/types.rs` 中定义（PascalCase 变体）。本任务在 `src/capi/value.rs` 顶部添加 C 风格常量别名，使代码中可使用 `MS_TYPE_NIL` 等与 C 头文件一致的命名：

```rust
use std::os::raw::{c_int, c_char};
use crate::capi::types::{MsType, MsValue};
use crate::vm::object::Object;

// C 风格常量别名（与 types.h 中 #define / enum 值一致）
pub const MS_TYPE_NIL:          MsType = MsType::Nil;
pub const MS_TYPE_BOOL:         MsType = MsType::Bool;
pub const MS_TYPE_INT:          MsType = MsType::Int;
pub const MS_TYPE_FLOAT:        MsType = MsType::Float;
pub const MS_TYPE_STRING:       MsType = MsType::String;
pub const MS_TYPE_LIST:         MsType = MsType::List;
pub const MS_TYPE_DICT:         MsType = MsType::Dict;
pub const MS_TYPE_TUPLE:        MsType = MsType::Tuple;
pub const MS_TYPE_SET:          MsType = MsType::Set;
pub const MS_TYPE_FUNCTION:     MsType = MsType::Function;
pub const MS_TYPE_CLASS:        MsType = MsType::Class;
pub const MS_TYPE_INSTANCE:     MsType = MsType::Instance;
pub const MS_TYPE_MODULE:       MsType = MsType::Module;
pub const MS_TYPE_GENERATOR:    MsType = MsType::Generator;
pub const MS_TYPE_FUTURE:       MsType = MsType::Future;
pub const MS_TYPE_CHANNEL:      MsType = MsType::Channel;
pub const MS_TYPE_ITERATOR:     MsType = MsType::Iterator;
pub const MS_TYPE_BOUND_METHOD: MsType = MsType::BoundMethod;
pub const MS_TYPE_JOIN_HANDLE:  MsType = MsType::JoinHandle;

pub const MS_TRUE:  c_int = 1;
pub const MS_FALSE: c_int = 0;
```

> **注意**：`obj_to_ms_type` 也可直接使用 `MsType::Nil` 等枚举变体，常量别名仅用于与 C 风格代码保持一致。两种写法等价。

### 构建系统变更

`cbindgen` 已由 Task 65 添加为 build-dependency。本任务仅需追加 `cc` crate（编译 C 辅助文件）：

```toml
[build-dependencies]
# cbindgen = "0.26"  ← 已由 Task 65 添加
cc = "1.0"
```

> **注意**：不需要 `libc` 依赖——`msString` 使用 Rust 标准库的 `std::ffi::CStr::from_ptr(str).to_bytes()` 获取 C 字符串长度。

`build.rs` 在 Task 65 的基础上追加 C 文件编译（在 `#[cfg(feature = "capi")]` 块内）：

```rust
#[cfg(feature = "capi")]
{
    // 编译 C va_list 辅助函数（msStringFmt 由 C 文件导出）
    cc::Build::new()
        .file("src/capi/vsnprintf_shim.c")
        .compile("mslang_capi_shim");

    // ... Task 65 的 cbindgen 生成逻辑不变 ...
}
```

## 验证标准

1. `msInt(42)` 返回非 NULL 指针，`msIsInt` 返回 `MS_TRUE`，`msTypeof` 返回 `MsType::Int`
2. `msFloat(3.14)` 返回非 NULL，`msIsFloat` 返回 `MS_TRUE`，`msIsNumber` 返回 `MS_TRUE`
3. `msString(vm, "hello")` 返回非 NULL，`msIsString` 返回 `MS_TRUE`
4. `msStringn(vm, "ab\0cd", 5)` 正确处理含 `\0` 的字符串
5. `msNil()` 返回非 NULL，`msIsNil` 返回 `MS_TRUE`
6. `msBoolVal(0)` 返回 false 值，`msBoolVal(1)` 返回 true 值
7. `msListNew(vm)` 返回非 NULL，`msIsList` 返回 `MS_TRUE`
8. `msListFrom` 传入 3 个元素，创建长度为 3 的 List
9. `msTupleFrom` 传入 4 个元素，创建长度为 4 的 Tuple
10. `msDictFrom` 传入 2 个键值对，创建长度为 2 的 Dict
11. `msRoot` / `msUnroot` 配对使用不崩溃，root 后对象在 GC 后仍存活
12. 对未 root 的对象触发 GC 后访问不崩溃（GC 安全性）
13. 所有 `msIs*` 函数对各类型值返回正确结果
14. `msIsNumber` 对 Int 和 Float 均返回 `MS_TRUE`
15. `msIsFunction` 对 Function 和 Closure 均返回 `MS_TRUE`
16. `msStringFmt(vm, "%d + %d = %d", 1, 2, 3)` 返回 `"1 + 2 = 3"`
17. `msListFrom` 传入含 NULL 元素的数组不崩溃（NULL 元素被跳过）
18. `msDictFrom` 传入含 NULL key/value 的数组不崩溃（对应对被跳过）
19. 所有 `ms*` 值创建函数返回的 `MsValue*` 经 `msValueFree`（Task 66）释放后不崩溃

## 测试用例

### Rust 单元测试

在 `src/capi/value.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::msVmNew;
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            unsafe { let _ = Box::from_raw(val); }
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
        // unroot 后 c_roots 中不再包含此对象（GC 实际接入后验证存活语义）

        free_value(s);
        msVmFree(vm);
    }

    #[test]
    fn test_root_inline_value_noop() {
        let vm = msVmNew();

        let i = msInt(42);
        msRoot(vm, i);
        msUnroot(vm, i);
        // 内联值 root/unroot 是 no-op，不应 panic
        free_value(i);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety() {
        assert!(msTypeof(ptr::null_mut()) == MsType::Nil);

        let vm = msVmNew();

        msRoot(vm, ptr::null_mut());
        msUnroot(vm, ptr::null_mut());
        // NULL 指针操作不应 panic
        msVmFree(vm);
    }

    #[test]
    fn test_list_from_with_null_element() {
        let vm = msVmNew();

        let a = msInt(1);
        let c = msInt(3);
        // NULL 中间元素不应崩溃
        let items = [a, ptr::null_mut(), c];
        let list = msListFrom(vm, items.as_ptr(), 3);
        assert!(!list.is_null());
        assert_eq!(msIsList(list), MS_TRUE);

        free_value(list);
        msVmFree(vm);
    }
}
```

### C 集成测试

在 `tests/c/test_value_creation.c`：

```c
#include <mslang.h>
#include <assert.h>
#include <string.h>

static void test_create_int_float_string_nil(void) {
    MsVM* vm = msVmNew();

    MsValue* i = msInt(42);
    assert(i != NULL);
    assert(msIsInt(i) == MS_TRUE);
    assert(msTypeof(i) == MS_TYPE_INT);

    MsValue* f = msFloat(3.14);
    assert(f != NULL);
    assert(msIsFloat(f) == MS_TRUE);
    assert(msIsNumber(f) == MS_TRUE);

    MsValue* s = msString(vm, "hello");
    assert(s != NULL);
    assert(msIsString(s) == MS_TRUE);

    MsValue* n = msNil();
    assert(n != NULL);
    assert(msIsNil(n) == MS_TRUE);
    assert(msTypeof(n) == MS_TYPE_NIL);

    MsValue* t = msBoolVal(1);
    assert(t != NULL);
    assert(msIsBool(t) == MS_TRUE);

    msVmFree(vm);
}

static void test_list_from_array(void) {
    MsVM* vm = msVmNew();

    MsValue* a = msInt(1);
    MsValue* b = msInt(2);
    MsValue* c = msInt(3);
    MsValue* items[] = { a, b, c };

    MsValue* list = msListFrom(vm, items, 3);
    assert(list != NULL);
    assert(msIsList(list) == MS_TRUE);

    msVmFree(vm);
}

static void test_dict_from_pairs(void) {
    MsVM* vm = msVmNew();

    MsValue* k1 = msString(vm, "x");
    MsValue* v1 = msInt(1);
    MsValue* k2 = msString(vm, "y");
    MsValue* v2 = msInt(2);
    MsValue* pairs[] = { k1, v1, k2, v2 };

    MsValue* dict = msDictFrom(vm, pairs, 2);
    assert(dict != NULL);
    assert(msIsDict(dict) == MS_TRUE);

    msVmFree(vm);
}

static void test_root_unroot(void) {
    MsVM* vm = msVmNew();

    MsValue* s = msString(vm, "rooted");
    MsValue* rooted = msRoot(vm, s);
    assert(rooted == s);

    msUnroot(vm, s);
    msVmFree(vm);
}

static void test_string_fmt(void) {
    MsVM* vm = msVmNew();

    MsValue* s = msStringFmt(vm, "%d + %d = %d", 1, 2, 3);
    assert(s != NULL);
    assert(msIsString(s) == MS_TRUE);

    msVmFree(vm);
}

int main(void) {
    test_create_int_float_string_nil();
    test_list_from_array();
    test_dict_from_pairs();
    test_root_unroot();
    test_string_fmt();
    return 0;
}
```
