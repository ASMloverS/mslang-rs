# C API — 值创建与类型判断

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure（C API 基础设施：MsVM 结构体、互斥锁、不透明类型定义、capi 模块骨架）

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
| `src/capi/value.rs` | 值创建、类型判断、Root 管理的全部实现 |
| `src/capi/mod.rs` | capi 模块入口，声明 `pub mod value` |
| `include/mslang/value.h` | C 头文件（宏定义 + 函数声明） |
| `src/capi/vsnprintf_shim.c` | `msStringFmt` 的 C va_list 辅助函数 |

### MsValue 内部表示

`MsValue*` 是不透明指针。内部表示为 `Box<Object>` 暴露为裸指针：

```rust
#[repr(C)]
pub struct MsValue {
    pub obj: Object,
}
```

C 侧看到的 `MsValue*` 实际是 `*mut MsValue`。每个 API 函数负责：
- 创建值：`Box::into_raw(Box::new(MsValue { obj }))` 返回裸指针
- 销毁值：由 GC 回收或 `Box::from_raw` 手动回收（仅测试用）

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

### msRoot / msUnroot 实现

```rust
#[no_mangle]
pub extern "C" fn msRoot(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || val.is_null() {
        return val;
    }
    let vm = unsafe { &mut *vm };
    let _lock = vm.mutex.lock();

    // 仅 Ref 类型需要注册为 GC root
    if let Object::Ref(header_ptr) = unsafe { &(*val).obj } {
        vm.c_roots.insert(*header_ptr);
    }
    val
}

#[no_mangle]
pub extern "C" fn msUnroot(vm: *mut MsVM, val: *mut MsValue) {
    if vm.is_null() || val.is_null() {
        return;
    }
    let vm = unsafe { &mut *vm };
    let _lock = vm.mutex.lock();

    if let Object::Ref(header_ptr) = unsafe { &(*val).obj } {
        vm.c_roots.remove(header_ptr);
    }
}
```

`MsVM` 结构需包含 `c_roots: HashSet<*mut MsObjHeader>` 字段（在 Task 65 中定义）。Root 集合在 GC 标记阶段作为额外根集参与扫描。

### 特殊值实现

```rust
#[no_mangle]
pub extern "C" fn msNil() -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { obj: Object::Nil }))
}

#[no_mangle]
pub extern "C" fn msBoolVal(val: c_int) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { obj: Object::Bool(val != 0) }))
}
```

每次调用创建新的 Box。由于 Nil/Bool 是内联值，无 GC 管理需求，重复分配的开销极低。

### 值创建实现

```rust
#[no_mangle]
pub extern "C" fn msInt(val: i64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { obj: Object::Int(val) }))
}

#[no_mangle]
pub extern "C" fn msFloat(val: f64) -> *mut MsValue {
    Box::into_raw(Box::new(MsValue { obj: Object::Float(val) }))
}

#[no_mangle]
pub extern "C" fn msString(vm: *mut MsVM, str: *const c_char) -> *mut MsValue {
    if str.is_null() {
        return msStringn(vm, std::ptr::null(), 0);
    }
    let len = unsafe { libc::strlen(str) };
    msStringn(vm, str, len)
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
        obj: Object::Ref(header),
    }))
}
```

`alloc_string_on_heap` 封装堆分配逻辑：分配 `MsObjHeader` + 字节数据，设置 `type_tag = TypeTag::STRING`，注册到 VM 堆。

### msStringFmt 实现

Rust 的 `extern "C"` 可声明可变参数函数（`...`），但无法在 Rust 侧直接访问 `va_list`（需要 nightly `std::ffi::VaList`）。采用 C 辅助函数方案：

**C 辅助 shim**（`src/capi/vsnprintf_shim.c`）：

```c
#include <stdio.h>
#include <stdarg.h>

int ms_vsnprintf_shim(char* buf, size_t size, const char* fmt, va_list ap) {
    return vsnprintf(buf, size, fmt, ap);
}
```

**Rust 侧**（`src/capi/value.rs`）：

```rust
extern "C" {
    fn ms_vsnprintf_shim(buf: *mut c_char, size: size_t, fmt: *const c_char, ap: va_list) -> c_int;
}

#[no_mangle]
pub extern "C" fn msStringFmt(vm: *mut MsVM, fmt: *const c_char, ...) -> *mut MsValue {
    let mut buf = [0u8; 1024];
    let written = unsafe {
        let mut ap: va_list;
        // 使用 va_start! / va_end! 宏（或 C 辅助宏）
        // 简化方案：调用 C helper wrapper
        ms_format_varargs(buf.as_mut_ptr() as *mut c_char, buf.len(), fmt)
    };
    let len = if written < 0 { 0 } else { written as usize };
    msStringn(vm, buf.as_ptr() as *const c_char, len)
}
```

> **备选方案**：若 `va_list` 不可用，可改为提供一个 C 包装函数 `ms_string_fmt_impl`，在 C 侧完成 `va_start/vsnprintf/va_end`，将结果传回 Rust。构建系统需将 `vsnprintf_shim.c` 编译为 `.o` 并链接。

### 集合创建实现

```rust
#[no_mangle]
pub extern "C" fn msListNew(vm: *mut MsVM) -> *mut MsValue {
    let header = alloc_heap_object(vm, TypeTag::LIST, std::mem::size_of::<MsObjHeader>() + std::mem::size_of::<Vec<Object>>());
    unsafe {
        let data_ptr = (header as *mut u8).add(std::mem::size_of::<MsObjHeader>()) as *mut Vec<Object>;
        data_ptr.write(Vec::new());
    }
    Box::into_raw(Box::new(MsValue { obj: Object::Ref(header) }))
}

#[no_mangle]
pub extern "C" fn msListFrom(vm: *mut MsVM, items: *const *mut MsValue, count: c_int) -> *mut MsValue {
    let list = msListNew(vm);
    if items.is_null() || count <= 0 {
        return list;
    }
    for i in 0..count as usize {
        let item = unsafe { *items.add(i) };
        let obj = unsafe { (*item).obj.clone() };
        list_push(list, obj);
    }
    list
}
```

`msTupleFrom` 结构类似，使用 `TypeTag::TUPLE` 并基于 `Vec<Object>`（不可变）。

`msDictFrom` 的 `pairs` 参数为扁平 key-value 数组：

```rust
#[no_mangle]
pub extern "C" fn msDictFrom(vm: *mut MsVM, pairs: *const *mut MsValue, count: c_int) -> *mut MsValue {
    let dict = msDictNew(vm);
    if pairs.is_null() || count <= 0 {
        return dict;
    }
    for i in 0..count as usize {
        let key = unsafe { *pairs.add(i * 2) };
        let val = unsafe { *pairs.add(i * 2 + 1) };
        let key_obj = unsafe { (*key).obj.clone() };
        let val_obj = unsafe { (*val).obj.clone() };
        dict_insert(dict, key_obj, val_obj);
    }
    dict
}
```

### 类型判断实现

```rust
fn obj_to_ms_type(obj: &Object) -> MsType {
    match obj {
        Object::Nil => MS_TYPE_NIL,
        Object::Bool(_) => MS_TYPE_BOOL,
        Object::Int(_) => MS_TYPE_INT,
        Object::Float(_) => MS_TYPE_FLOAT,
        Object::Ref(header) => {
            let tag = unsafe { (**header).type_tag };
            match TypeTag::from(tag) {
                TypeTag::STRING => MS_TYPE_STRING,
                TypeTag::LIST => MS_TYPE_LIST,
                TypeTag::DICT => MS_TYPE_DICT,
                TypeTag::TUPLE => MS_TYPE_TUPLE,
                TypeTag::SET => MS_TYPE_SET,
                TypeTag::FUNCTION => MS_TYPE_FUNCTION,
                TypeTag::CLOSURE => MS_TYPE_FUNCTION,
                TypeTag::CLASS => MS_TYPE_CLASS,
                TypeTag::INSTANCE => MS_TYPE_INSTANCE,
                TypeTag::MODULE => MS_TYPE_MODULE,
                TypeTag::GENERATOR => MS_TYPE_GENERATOR,
                TypeTag::FUTURE => MS_TYPE_FUTURE,
                TypeTag::CHANNEL => MS_TYPE_CHANNEL,
                TypeTag::ITERATOR => MS_TYPE_ITERATOR,
                TypeTag::BOUND_METHOD => MS_TYPE_BOUND_METHOD,
                _ => MS_TYPE_NIL,
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn msTypeof(val: *mut MsValue) -> MsType {
    if val.is_null() { return MS_TYPE_NIL; }
    obj_to_ms_type(unsafe { &(*val).obj })
}

#[no_mangle]
pub extern "C" fn msIsNil(val: *mut MsValue) -> c_int {
    if val.is_null() { return MS_FALSE; }
    if matches!(unsafe { &(*val).obj }, Object::Nil) { MS_TRUE } else { MS_FALSE }
}

#[no_mangle]
pub extern "C" fn msIsNumber(val: *mut MsValue) -> c_int {
    if val.is_null() { return MS_FALSE; }
    match unsafe { &(*val).obj } {
        Object::Int(_) | Object::Float(_) => MS_TRUE,
        _ => MS_FALSE,
    }
}
```

其余 `msIsBool` / `msIsInt` / `msIsFloat` / `msIsString` 等函数结构相同，匹配对应的 Object 变体或 TypeTag。`msIsFunction` 同时匹配 `FUNCTION` 和 `CLOSURE`。

### MsType 的 Rust 侧定义

在 `src/capi/types.rs` 中（或 `src/capi/value.rs` 顶部）：

```rust
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MsType {
    Nil = 0,
    Bool,
    Int,
    Float,
    String,
    List,
    Dict,
    Tuple,
    Set,
    Function,
    Class,
    Instance,
    Module,
    Generator,
    Future,
    Channel,
    Iterator,
    BoundMethod,
    JoinHandle,
}

pub const MS_TRUE: c_int = 1;
pub const MS_FALSE: c_int = 0;
```

### 构建系统变更

`Cargo.toml` 添加 `cbindgen` 构建依赖（生成 C 头文件），以及 `cc` crate 用于编译 C 辅助文件：

```toml
[build-dependencies]
cbindgen = "0.26"
cc = "1.0"

[dependencies]
libc = "0.2"
```

`build.rs` 添加：

```rust
fn main() {
    // 编译 C va_list 辅助函数
    cc::Build::new()
        .file("src/capi/vsnprintf_shim.c")
        .compile("mslang_capi_shim");

    // 生成 C 头文件
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let config = cbindgen::Config::from_root_or_default(&crate_dir);
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("Unable to generate C bindings")
        .write_to_file("include/mslang/mslang.h");
}
```

## 验证标准

1. `msInt(42)` 返回非 NULL 指针，`msIsInt` 返回 `MS_TRUE`，`msTypeof` 返回 `MS_TYPE_INT`
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

## 测试用例

### Rust 单元测试

在 `src/capi/value.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::test_utils::new_test_vm;
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
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let s = msString(vm_ptr, b"hello\0".as_ptr() as *const c_char);
        assert!(!s.is_null());
        assert_eq!(msIsString(s), MS_TRUE);
        assert_eq!(msTypeof(s), MsType::String);
        free_value(s);
    }

    #[test]
    fn test_create_stringn_with_null_bytes() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let data = b"ab\0cd";
        let s = msStringn(vm_ptr, data.as_ptr() as *const c_char, 5);
        assert!(!s.is_null());
        assert_eq!(msIsString(s), MS_TRUE);
        free_value(s);
    }

    #[test]
    fn test_list_new_and_from() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let list = msListNew(vm_ptr);
        assert!(!list.is_null());
        assert_eq!(msIsList(list), MS_TRUE);
        assert_eq!(msTypeof(list), MsType::List);

        let a = msInt(1);
        let b = msInt(2);
        let c = msInt(3);
        let items = [a, b, c];
        let list2 = msListFrom(vm_ptr, items.as_ptr(), 3);
        assert!(!list2.is_null());
        assert_eq!(msIsList(list2), MS_TRUE);

        free_value(list);
        free_value(list2);
    }

    #[test]
    fn test_tuple_from() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let a = msInt(10);
        let b = msInt(20);
        let items = [a, b];
        let tup = msTupleFrom(vm_ptr, items.as_ptr(), 2);
        assert!(!tup.is_null());
        assert_eq!(msIsTuple(tup), MS_TRUE);
        assert_eq!(msTypeof(tup), MsType::Tuple);

        free_value(tup);
    }

    #[test]
    fn test_dict_new_and_from() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let dict = msDictNew(vm_ptr);
        assert!(!dict.is_null());
        assert_eq!(msIsDict(dict), MS_TRUE);

        let k1 = msString(vm_ptr, b"x\0".as_ptr() as *const c_char);
        let v1 = msInt(1);
        let k2 = msString(vm_ptr, b"y\0".as_ptr() as *const c_char);
        let v2 = msInt(2);
        let pairs = [k1, v1, k2, v2];
        let dict2 = msDictFrom(vm_ptr, pairs.as_ptr(), 2);
        assert!(!dict2.is_null());
        assert_eq!(msIsDict(dict2), MS_TRUE);

        free_value(dict);
        free_value(dict2);
    }

    #[test]
    fn test_set_new() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let set = msSetNew(vm_ptr);
        assert!(!set.is_null());
        assert_eq!(msIsSet(set), MS_TRUE);
        assert_eq!(msTypeof(set), MsType::Set);

        free_value(set);
    }

    #[test]
    fn test_type_checking_all_types() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

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

        let s = msString(vm_ptr, b"test\0".as_ptr() as *const c_char);
        assert_eq!(msIsString(s), MS_TRUE);
        assert_eq!(msIsList(s), MS_FALSE);

        let list = msListNew(vm_ptr);
        assert_eq!(msIsList(list), MS_TRUE);
        assert_eq!(msIsDict(list), MS_FALSE);

        let dict = msDictNew(vm_ptr);
        assert_eq!(msIsDict(dict), MS_TRUE);
        assert_eq!(msIsSet(dict), MS_FALSE);

        let set = msSetNew(vm_ptr);
        assert_eq!(msIsSet(set), MS_TRUE);
        assert_eq!(msIsTuple(set), MS_FALSE);

        let tup = msTupleFrom(vm_ptr, ptr::null(), 0);
        assert_eq!(msIsTuple(tup), MS_TRUE);
        assert_eq!(msIsList(tup), MS_FALSE);

        for v in [nil, b, i, f, s, list, dict, set, tup] {
            free_value(v);
        }
    }

    #[test]
    fn test_root_unroot() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let s = msString(vm_ptr, b"rooted\0".as_ptr() as *const c_char);
        assert!(!s.is_null());

        let result = msRoot(vm_ptr, s);
        assert_eq!(result, s);

        // root 后对象在 c_roots 中
        {
            let vm_ref = unsafe { &mut *vm_ptr };
            let _lock = vm_ref.mutex.lock();
            if let Object::Ref(h) = unsafe { &(*s).obj } {
                assert!(vm_ref.c_roots.contains(h));
            }
        }

        msUnroot(vm_ptr, s);

        // unroot 后对象不在 c_roots 中
        {
            let vm_ref = unsafe { &mut *vm_ptr };
            let _lock = vm_ref.mutex.lock();
            if let Object::Ref(h) = unsafe { &(*s).obj } {
                assert!(!vm_ref.c_roots.contains(h));
            }
        }

        free_value(s);
    }

    #[test]
    fn test_root_inline_value_noop() {
        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        let i = msInt(42);
        msRoot(vm_ptr, i);
        msUnroot(vm_ptr, i);
        // 内联值 root/unroot 是 no-op，不应 panic
        free_value(i);
    }

    #[test]
    fn test_null_safety() {
        assert!(msTypeof(ptr::null_mut()) == MsType::Nil);

        let mut vm = new_test_vm();
        let vm_ptr = &mut vm as *mut _;

        msRoot(vm_ptr, ptr::null_mut());
        msUnroot(vm_ptr, ptr::null_mut());
        // NULL 指针操作不应 panic
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
