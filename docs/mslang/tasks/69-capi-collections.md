# C API — 集合操作（List/Dict/Tuple/Set + 迭代器 + 字符串操作）

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 67-capi-value-creation
- 68-capi-value-convert（`set_type_error`、`compare_objects`、`is_hashable` 辅助函数）

## 目标

实现 value.h 的后半部分 API：字符串操作、List 操作、Dict 操作、Tuple 操作、Set 操作和迭代器。这些函数使 C 程序能够对 mslang 集合类型进行完整的读写操作，包括元素增删改查、切片、迭代遍历。

## 设计规格

参照 [13-capi](../13-capi.md) § value.h（后半: 字符串操作 + List 操作 + Dict 操作 + Tuple 操作 + Set 操作 + 迭代器）。

### 字符串操作

```c
MS_API size_t     msStringLen(MsVM* vm, MsValue* str);
MS_API const char* msStringData(MsVM* vm, MsValue* str);
MS_API MsValue*   msStringConcat(MsVM* vm, MsValue* a, MsValue* b);
MS_API MsValue*   msStringSlice(MsVM* vm, MsValue* str, int start, int end);
```

- `msStringLen`：返回字符串字节长度
- `msStringData`：返回内部 UTF-8 数据指针（借用引用，无需 free，仅在 str 存活期间有效）
- `msStringConcat`：连接两个字符串，返回新字符串
- `msStringSlice`：切片，支持负索引，返回新字符串

### List 操作

```c
MS_API int      msListLen(MsVM* vm, MsValue* list);
MS_API MsValue* msListGet(MsVM* vm, MsValue* list, int index);
MS_API MsStatus msListSet(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API MsStatus msListPush(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* msListPop(MsVM* vm, MsValue* list);
MS_API MsStatus msListInsert(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API int      msListContains(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* msListSlice(MsVM* vm, MsValue* list, int start, int end, int step);
```

- `msListGet`：支持负索引（wrap around）。越界 → 设置 IndexError，返回 NULL
- `msListSet`：原地修改指定位置。越界 → 设置 IndexError，返回 MS_ERROR
- `msListPush`：尾部追加，返回 MS_OK
- `msListPop`：弹出末尾元素并返回。空列表 → 设置 IndexError，返回 NULL
- `msListInsert`：在指定位置插入元素，支持负索引
- `msListContains`：包含则返回 MS_TRUE，否则 MS_FALSE。使用 Object 的 Eq 判断
- `msListSlice`：创建新列表。支持负索引和 step。step=0 → 设置 ValueError

### Dict 操作

```c
MS_API int      msDictLen(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictGet(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* msDictGetDefault(MsVM* vm, MsValue* dict, MsValue* key, MsValue* defaultVal);
MS_API MsStatus msDictSet(MsVM* vm, MsValue* dict, MsValue* key, MsValue* val);
MS_API MsStatus msDictRemove(MsVM* vm, MsValue* dict, MsValue* key);
MS_API int      msDictContains(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* msDictKeys(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictValues(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictItems(MsVM* vm, MsValue* dict);
```

- `msDictGet`：键不存在时返回 NULL（不设异常）
- `msDictGetDefault`：键不存在时返回 defaultVal
- `msDictSet`：设置键值对（存在则覆盖）。键不可哈希 → 设置 TypeError，返回 MS_ERROR
- `msDictRemove`：删除键值对。键不存在 → 设置 KeyError，返回 MS_ERROR
- `msDictContains`：包含则返回 MS_TRUE，否则 MS_FALSE
- `msDictKeys`：返回新 List，包含所有键（保持插入顺序）
- `msDictValues`：返回新 List，包含所有值（保持插入顺序）
- `msDictItems`：返回新 List，每个元素为二元 Tuple `(key, value)`

### Tuple 操作

```c
MS_API int      msTupleLen(MsVM* vm, MsValue* tup);
MS_API MsValue* msTupleGet(MsVM* vm, MsValue* tup, int index);
MS_API MsStatus msTupleUnpack(MsVM* vm, MsValue* tup, MsValue*** items, int* count);
```

- `msTupleGet`：支持负索引。越界 → 设置 IndexError，返回 NULL
- `msTupleUnpack`：通过 `malloc` 分配 `MsValue**` 数组，设置每个元素为对应 tuple 元素的借用引用。调用方负责 `free(items)`，但不需要释放各元素（借用引用）。元素数量通过 `*count` 返回

### Set 操作

```c
MS_API int      msSetLen(MsVM* vm, MsValue* set);
MS_API MsStatus msSetAdd(MsVM* vm, MsValue* set, MsValue* val);
MS_API MsStatus msSetRemove(MsVM* vm, MsValue* set, MsValue* val);
MS_API int      msSetContains(MsVM* vm, MsValue* set, MsValue* val);
```

- `msSetAdd`：添加元素（已存在则无操作）。元素不可哈希 → 设置 TypeError，返回 MS_ERROR
- `msSetRemove`：删除元素。不存在 → 无异常、无错误（与 mslang 语义一致）
- `msSetContains`：包含则返回 MS_TRUE，否则 MS_FALSE

### 迭代器

```c
MS_API MsValue* msIter(MsVM* vm, MsValue* iterable);
MS_API MsStatus msNext(MsVM* vm, MsValue* iterator, MsValue** out);
```

- `msIter`：调用可迭代对象的 `__iter__` 协议，返回迭代器对象。不可迭代 → 设置 TypeError，返回 NULL
- `msNext`：调用迭代器的 `__next__`。成功返回 MS_OK，`*out` 设为当前值；迭代结束（StopIteration）返回 MS_ERROR。异常情况同样返回 MS_ERROR，需通过 `msErrOccurred` 区分

## 实现细节

### 文件位置

`src/capi/value.rs`（追加到 Task 67-68 的同一文件）

所有本任务函数添加到 `value.rs` 中，在 Task 67-68 已有的值创建、类型判断、比较函数之后。由 cbindgen 自动生成对应的 `value.h` 声明。

### 复用依赖

本任务复用以下已有符号：

| 符号 | 路径 | 用途 |
|---|---|---|
| `read_list` | `src/vm/object.rs` | 从 `*mut MsObjHeader` 读取 `&mut Vec<Object>` |
| `read_dict` | `src/vm/object.rs` | 从 `*mut MsObjHeader` 读取 `&mut DictMap` |
| `read_tuple` | `src/vm/object.rs` | 从 `*mut MsObjHeader` 读取 `&Vec<Object>` |
| `read_set` | `src/vm/object.rs` | 从 `*mut MsObjHeader` 读取 `&mut HashSet<Object>` |
| `read_str` | `src/vm/object.rs` | 从 `*mut MsObjHeader` 读取 `&str` |
| `alloc_list` | `src/vm/object.rs` | 创建新 List 对象 |
| `alloc_tuple` | `src/vm/object.rs` | 创建新 Tuple 对象 |
| `alloc_string` | `src/vm/object.rs` | 创建新 String 对象 |
| `TypeTag` | `src/vm/object.rs` | 类型标签枚举 |
| `Object` | `src/vm/object.rs` | 核心值枚举 |
| `MsVM` | `src/capi/vm.rs` | VM 不透明结构体 |
| `MsValue` | `src/capi/types.rs` | 值不透明结构体（`pub(crate) inner: Object`） |
| `lock_vm` | `src/capi/vm.rs` | 锁定 VM 并返回 guard |
| `set_type_error` | `src/capi/mod.rs` | TypeError 占位辅助（Task 68） |
| `is_hashable` | `src/capi/value.rs` | 检查 Object 可哈希性（Task 68） |
| `MS_TRUE`/`MS_FALSE` | `src/capi/value.rs` | C 布尔常量（Task 67） |

### 通用实现模式

每个函数遵循以下步骤：

1. 验证 `MsVM*` 和 `MsValue*` 参数非 NULL（返回安全默认值）
2. 通过 `lock_vm(vm)` 锁定 VM，获取 guard
3. 通过 `guard.get()` 获取 `&mut VmInner`，访问 `inner.vm`
4. 通过 `unsafe { &(*val).inner }` / `unsafe { (*val).inner.clone() }` 提取内部 Object
5. 验证 Object 类型匹配（通过 type_tag 检查）
6. 执行操作
7. 返回结果（`Box::into_raw(Box::new(MsValue { inner: obj }))` 创建新 MsValue*）

### 字符串操作实现

```rust
use crate::capi::vm::lock_vm;
use crate::capi::types::MsValue;
use crate::vm::object::{read_str, read_list, read_dict, read_tuple, read_set,
    alloc_string, alloc_list, alloc_tuple, Object, TypeTag};

#[no_mangle]
pub extern "C" fn msStringLen(vm: *mut MsVM, str_val: *mut MsValue) -> usize {
    if vm.is_null() || str_val.is_null() { return 0; }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: str_val 由 ms* 创建。
    match unsafe { &(*str_val).inner } {
        Object::Ref(ptr) => {
            // SAFETY: ptr 由 alloc_string 分配（type_tag 已验证）。
            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 {
                unsafe { read_str(*ptr) }.len()
            } else { 0 }
        }
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn msStringData(vm: *mut MsVM, str_val: *mut MsValue) -> *const c_char {
    if vm.is_null() || str_val.is_null() { return std::ptr::null(); }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: str_val 由 ms* 创建。
    match unsafe { &(*str_val).inner } {
        Object::Ref(ptr) => {
            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                unsafe { read_str(*ptr) }.as_ptr() as *const c_char
            } else { std::ptr::null() }
        }
        _ => std::ptr::null(),
    }
}
```

> **嵌入 `\0` 限制**：mslang String 可包含 `\0`（通过 `msStringn` 创建）。`msStringData` 返回 `*const c_char`（C 空终止字符串），数据在首个 `\0` 后不可见。C 侧需完整字节的应用应使用 `msStringLen` + `msStringData` 手动按长度读取。

`msStringConcat` 和 `msStringSlice` 按相同模式实现。`msStringConcat` 使用 `read_str` 提取两个字符串，`format!("{}{}", sa, sb)` 连接后 `alloc_string` 创建新对象。`msStringSlice` 使用 `normalize_index` 处理负索引，对字节切片 `[lo..hi]` 取子串。

> **UTF-8 切片注意**：mslang String 是 UTF-8 编码。`msStringSlice` 按字节索引切片可能在多字节字符中间截断，产生无效 UTF-8。MVP 阶段按字节切片（与 C API 的 int 参数语义一致），完整 Unicode 字符切片留待后续优化。

### List 操作实现

以 `msListLen`、`msListGet`、`msListPush` 为模板，其余按相同模式实现：

```rust
#[no_mangle]
pub extern "C" fn msListLen(vm: *mut MsVM, list: *mut MsValue) -> c_int {
    if vm.is_null() || list.is_null() { return -1; }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: list 由 ms* 创建。
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) => {
            if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 {
                // SAFETY: type_tag 为 LIST。
                unsafe { read_list(*ptr).len() as c_int }
            } else { -1 }
        }
        _ => -1,
    }
}

#[no_mangle]
pub extern "C" fn msListGet(
    vm: *mut MsVM, list: *mut MsValue, index: c_int,
) -> *mut MsValue {
    if vm.is_null() || list.is_null() { return std::ptr::null_mut(); }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // SAFETY: list 由 ms* 创建。
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            // SAFETY: type_tag 为 LIST。
            let items = unsafe { read_list(*ptr) };
            let len = items.len() as isize;
            match resolve_index(index, len) {
                Some(i) => {
                    let val = items[i].clone();
                    Box::into_raw(Box::new(MsValue { inner: val }))
                }
                None => {
                    set_type_error(&mut inner.vm, "valid index", unsafe { &(*list).inner });
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(&mut inner.vm, "list", unsafe { &(*list).inner });
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn msListPush(
    vm: *mut MsVM, list: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || list.is_null() || val.is_null() { return MsStatus::MS_ERROR; }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: list/val 由 ms* 创建。
    let new_val = unsafe { (*val).inner.clone() };
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            // SAFETY: type_tag 为 LIST。
            unsafe { read_list(*ptr) }.push(new_val);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

`msListSet`、`msListPop`、`msListInsert`、`msListContains` 按相同模式实现。关键点：
- `msListSet`：`resolve_index` 越界时 `set_type_error` + 返回 `MS_ERROR`
- `msListPop`：空列表时 `set_type_error` + 返回 `null_mut()`
- `msListInsert`：负索引 `(len + index).max(0)`，正索引 `.min(len)`
- `msListContains`：使用 `Vec::contains(&target)` 判断（基于 `Object::PartialEq`）

`msListSlice` 支持 step 参数：

```rust
#[no_mangle]
pub extern "C" fn msListSlice(
    vm: *mut MsVM, list: *mut MsValue,
    start: c_int, end: c_int, step: c_int,
) -> *mut MsValue {
    if vm.is_null() || list.is_null() { return std::ptr::null_mut(); }
    if step == 0 {
        let guard = lock_vm(vm);
        let inner = unsafe { &mut *guard.get() };
        set_type_error(&mut inner.vm, "non-zero step", unsafe { &(*list).inner });
        return std::ptr::null_mut();
    }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: list 由 ms* 创建。
    match unsafe { &(*list).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
            // SAFETY: type_tag 为 LIST。
            let items = unsafe { &*read_list(*ptr) as &Vec<Object> };
            let len = items.len() as isize;
            let step = step as isize;
            let (s_idx, e_idx) = compute_slice_bounds(start, end, step, len);
            let mut result = Vec::new();
            let mut i = s_idx;
            if step > 0 {
                while i < e_idx && i >= 0 {
                    result.push(items[i as usize].clone());
                    i += step;
                }
            } else {
                while i > e_idx && i >= 0 && i < len {
                    result.push(items[i as usize].clone());
                    i += step;
                }
            }
            let result_obj = alloc_list(result);
            Box::into_raw(Box::new(MsValue { inner: result_obj }))
        }
        _ => std::ptr::null_mut(),
    }
}
```

辅助函数：

```rust
fn resolve_index(index: c_int, len: isize) -> Option<usize> {
    let idx = if index < 0 { len + index as isize } else { index as isize };
    if idx < 0 || idx >= len { None } else { Some(idx as usize) }
}

/// 计算 slice 的 (start, end) 边界。正/负 step 分别处理默认值和 clamp。
fn compute_slice_bounds(start: c_int, end: c_int, step: isize, len: isize)
    -> (isize, isize)
{
    let (default_s, default_e) = if step > 0 { (0, len) } else { (len - 1, -1) };
    let s = if start < 0 {
        (len + start as isize).max(if step > 0 { 0 } else { -1 })
    } else {
        (start as isize).min(if step > 0 { len } else { len - 1 })
    };
    let s = if start == 0 && step > 0 { default_s } else { s.max(0) };
    let e = if end < 0 {
        (len + end as isize).max(if step > 0 { 0 } else { -1 })
    } else {
        (end as isize).min(len)
    };
    (s, e)
}
```

> **slice bounds 说明**：正 step 时 start 默认 0、end 默认 len（前向遍历）；负 step 时 start 默认 len-1、end 默认 -1（后向遍历）。负索引经 `len + idx` 转换后 clamp 到 `[0, len]`。

### Dict 操作实现

以 `msDictGet`、`msDictSet` 为模板：

```rust
#[no_mangle]
pub extern "C" fn msDictGet(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || dict.is_null() || key.is_null() { return std::ptr::null_mut(); }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: dict/key 由 ms* 创建。
    let key_obj = unsafe { (*key).inner.clone() };
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            // SAFETY: type_tag 为 DICT。
            let map = unsafe { read_dict(*ptr) };
            match map.get(&key_obj) {
                Some(val) => Box::into_raw(Box::new(MsValue { inner: val.clone() })),
                None => std::ptr::null_mut(),  // 键不存在，不设异常
            }
        }
        _ => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn msDictSet(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || dict.is_null() || key.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // SAFETY: dict/key/val 由 ms* 创建。
    let key_obj = unsafe { (*key).inner.clone() };
    let val_obj = unsafe { (*val).inner.clone() };
    // 检查键可哈希性（Object::hash() 对 List/Dict/Set/NaN 会 panic）
    if !is_hashable(&key_obj) {
        set_type_error(&mut inner.vm, "hashable key", &key_obj);
        return MsStatus::MS_ERROR;
    }
    match unsafe { &(*dict).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            // SAFETY: type_tag 为 DICT。
            unsafe { read_dict(*ptr) }.insert(key_obj, val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

> **不可哈希键检查**：`msDictSet` 必须在 `insert` 前检查 `is_hashable(&key_obj)`。Object 的 Hash impl 对 List/Dict/Set/NaN 会 **panic**（`object.rs:2349,2361`），直接 insert 会崩溃 C 进程。

其余 Dict 函数按相同模式实现：
- `msDictGetDefault`：键不存在返回 `default_val`（直接返回参数指针）
- `msDictRemove`：键不存在设置错误，返回 `MS_ERROR`
- `msDictContains`：使用 `map.get(&key).is_some()`
- `msDictKeys`：`map.keys().cloned().collect()` → `alloc_list(keys)`
- `msDictValues`：`map.items().iter().map(|(_, v)| v.clone()).collect()` → `alloc_list(vals)`
- `msDictItems`：每对 `(k, v)` → `alloc_tuple(vec![k.clone(), v.clone()])`，收集到 `alloc_list`

### Tuple 操作实现

以 `msTupleGet` 为模板：

```rust
#[no_mangle]
pub extern "C" fn msTupleGet(
    vm: *mut MsVM, tup: *mut MsValue, index: c_int,
) -> *mut MsValue {
    if vm.is_null() || tup.is_null() { return std::ptr::null_mut(); }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // SAFETY: tup 由 ms* 创建。
    match unsafe { &(*tup).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            // SAFETY: type_tag 为 TUPLE。
            let items = unsafe { read_tuple(*ptr) };
            let len = items.len() as isize;
            match resolve_index(index, len) {
                Some(i) => {
                    Box::into_raw(Box::new(MsValue { inner: items[i].clone() }))
                }
                None => {
                    set_type_error(&mut inner.vm, "valid index", unsafe { &(*tup).inner });
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(&mut inner.vm, "tuple", unsafe { &(*tup).inner });
            std::ptr::null_mut()
        }
    }
}
```

`msTupleLen` 按相同模式实现。`msTupleUnpack` 使用 `std::alloc::alloc` 分配 `*mut MsValue` 数组：

```rust
#[no_mangle]
pub extern "C" fn msTupleUnpack(
    vm: *mut MsVM, tup: *mut MsValue,
    items_out: *mut *mut *mut MsValue, count_out: *mut c_int,
) -> MsStatus {
    if vm.is_null() || tup.is_null() || items_out.is_null() || count_out.is_null() {
        return MsStatus::MS_ERROR;
    }
    let guard = lock_vm(vm);
    let _inner = unsafe { &*guard.get() };
    // SAFETY: tup 由 ms* 创建。
    match unsafe { &(*tup).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
            // SAFETY: type_tag 为 TUPLE。
            let elements = unsafe { read_tuple(*ptr) };
            let n = elements.len();
            if n == 0 {
                // 空 tuple：设置 null 指针，跳过分配。
                unsafe { *items_out = std::ptr::null_mut(); *count_out = 0; }
                return MsStatus::MS_OK;
            }
            let layout = std::alloc::Layout::array::<*mut MsValue>(n)
                .unwrap_or_else(|_| return MsStatus::MS_ERROR);
            // SAFETY: layout 非零大小（n > 0），alloc 返回有效指针或 null。
            let arr = unsafe { std::alloc::alloc(layout) as *mut *mut MsValue };
            if arr.is_null() { return MsStatus::MS_ERROR; }
            for (i, elem) in elements.iter().enumerate() {
                let ms_val = Box::into_raw(Box::new(MsValue { inner: elem.clone() }));
                unsafe { *arr.add(i) = ms_val; }
            }
            unsafe { *items_out = arr; *count_out = n as c_int; }
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

> **msTupleUnpack 内存语义**：调用方通过 `msTupleUnpackFree` 释放数组（见下）。数组中各 `MsValue*` 为新分配的拥有引用（owned ref），调用方应逐个 `msValueFree` 或 `msRoot`。

> **msTupleUnpackFree 辅助**：提供安全的释放函数，避免 C `free()` 与 Rust allocator 不匹配：

```rust
#[no_mangle]
pub extern "C" fn msTupleUnpackFree(items: *mut *mut MsValue, count: c_int) {
    if items.is_null() { return; }
    for i in 0..count as usize {
        // SAFETY: items 指向 count 个 MsValue*。
        let val = unsafe { *items.add(i) };
        if !val.is_null() {
            // SAFETY: val 由 msTupleUnpack 的 Box::into_raw 分配。
            unsafe { let _ = Box::from_raw(val); }
        }
    }
    // 释放数组本身
    if count > 0 {
        let layout = std::alloc::Layout::array::<*mut MsValue>(count as usize).unwrap();
        // SAFETY: items 由 msTupleUnpack 的 alloc 分配，layout 匹配。
        unsafe { std::alloc::dealloc(items as *mut u8, layout); }
    }
}
```

### Set 操作实现

以 `msSetAdd` 为模板：

```rust
#[no_mangle]
pub extern "C" fn msSetAdd(
    vm: *mut MsVM, set: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || set.is_null() || val.is_null() { return MsStatus::MS_ERROR; }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // SAFETY: set/val 由 ms* 创建。
    let val_obj = unsafe { (*val).inner.clone() };
    // 检查元素可哈希性（Object::hash() 对 List/Dict/Set/NaN 会 panic）
    if !is_hashable(&val_obj) {
        set_type_error(&mut inner.vm, "hashable element", &val_obj);
        return MsStatus::MS_ERROR;
    }
    match unsafe { &(*set).inner } {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
            // SAFETY: type_tag 为 SET。
            unsafe { read_set(*ptr) }.insert(val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

> **不可哈希元素检查**：同 `msDictSet`，`msSetAdd` 必须在 `insert` 前检查 `is_hashable`。

其余 Set 函数按相同模式实现。`msSetRemove` 对不存在的元素无异常、无错误（与 mslang `set.remove()` 语义一致需确认——02-types.md 中 `s.remove(val)` 删除元素，不存在的行为需查 10-builtins.md）。`msSetContains` 使用 `HashSet::contains`。

### 迭代器实现

> **Deferred**：迭代器需要 `TypeTag::ITERATOR` 类型的堆对象结构和对应的 alloc/read 函数，这些在当前代码库中不存在。`msIter`/`msNext` 在本任务中提供占位实现（返回 TypeError），完整实现待迭代器内部结构定义后补充。

```rust
#[no_mangle]
pub extern "C" fn msIter(vm: *mut MsVM, iterable: *mut MsValue) -> *mut MsValue {
    if vm.is_null() || iterable.is_null() { return std::ptr::null_mut(); }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    // TODO: 实现迭代器内部结构（ITERATOR 类型堆对象）
    set_type_error(&mut inner.vm, "iterable (iterator protocol not yet implemented)",
        unsafe { &(*iterable).inner });
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn msNext(
    vm: *mut MsVM, iterator: *mut MsValue, out: *mut *mut MsValue,
) -> MsStatus {
    // 迭代器未实现，返回 MS_ERROR
    MsStatus::MS_ERROR
}
```

> **迭代器完整实现路线**：(1) 定义 `MsIterator` 堆对象结构（含源对象引用 + 位置索引）；(2) 为 List/Dict/Tuple/Set/String 实现 `alloc_*_iterator` 函数；(3) `msNext` 调用迭代器的 `advance()` 提取下一个元素。这些依赖 ITERATOR TypeTag 的堆对象基础设施。

### 写屏障集成

> **MVP 阶段**：当前 GC 未接入日常分配（VM 日常 `alloc_*` 不受 GC 管理），写屏障为 no-op。以下说明供 Phase 7.5 并发 GC 上线后参考。

Phase 7.5 并发 GC 上线后，所有修改堆对象的操作（`msListSet`、`msListPush`、`msListInsert`、`msDictSet`、`msSetAdd`）需在修改后调用写屏障：

```rust
// Phase 7.5: 在 msListPush 成功 push 后：
// vm.gc.write_barrier(parent_header_ptr, new_val_header_ptr);
```

写屏障确保并发三色标记清扫 GC 不会丢失对新增引用的追踪。当前阶段（Phase 2.5 STW GC）无需写屏障。

### cbindgen 注意事项

所有 `#[no_mangle] pub extern "C"` 函数会被 cbindgen 自动扫描并生成 C 声明到 `include/mslang/value.h`。本任务的函数追加到 value.h 末尾，与 Task 67-68 的函数声明共存。

需确保 cbindgen 配置中 `fn.prefix = "MS_API"` 正确应用到所有新生成的函数声明。

## 验证标准

### 字符串操作

1. `msStringLen` 返回字符串 UTF-8 字节长度
2. `msStringData` 返回有效的 C 字符串指针（null-terminated）
3. `msStringConcat` 正确连接两个字符串
4. `msStringSlice` 支持负索引，结果正确

### List 操作

5. `msListPush` + `msListLen` 返回正确数量
6. `msListGet` 检索已推入的值，正负索引均正确
7. `msListSet` 原地修改元素，后续 `msListGet` 返回新值
8. `msListPop` 移除并返回末尾元素
9. `msListInsert` 在指定位置插入，后续元素后移
10. `msListContains` 对已存在元素返回 MS_TRUE，不存在返回 MS_FALSE
11. `msListSlice` 创建新列表，支持 step 参数
12. 越界访问正确抛出 IndexError

### Dict 操作

13. `msDictSet` + `msDictGet` 往返正确
14. `msDictGet` 键不存在时返回 NULL（不设异常）
15. `msDictGetDefault` 键不存在时返回默认值
16. `msDictRemove` 正确删除键值对
17. `msDictContains` 返回正确布尔值
18. `msDictKeys/Values/Items` 返回正确长度和内容的列表
19. `msDictItems` 中每个元素为二元 Tuple

### Tuple 操作

20. `msTupleLen` 返回正确长度
21. `msTupleGet` 支持负索引
22. `msTupleUnpack` 正确解包所有元素，调用方 `free(items)` 安全

### Set 操作

23. `msSetAdd` + `msSetContains` 正确工作
24. `msSetRemove` 后 `msSetContains` 返回 MS_FALSE
25. 重复 `msSetAdd` 同一值不影响长度

### 迭代器

26. `msIter` + `msNext` 可遍历 List 全部元素
27. 迭代结束后 `msNext` 返回 MS_ERROR
28. `msIter` 对不可迭代对象设置 TypeError

### 综合验证

29. 所有函数在 NULL 参数时不崩溃（返回安全默认值）
30. 多次 API 调用之间状态一致
31. GC 期间集合操作不导致 use-after-free

## 测试用例

Rust 单元测试位于 `src/capi/value.rs`，在 Task 68 测试块之后：

```rust
#[cfg(test)]
mod tests_collections {
    use super::*;
    use crate::capi::vm::{msVmFree, msVmNew};
    use crate::capi::types::{MsStatus, MsType};
    use std::os::raw::{c_char, c_int};
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() { unsafe { let _ = Box::from_raw(val); } }
    }

    fn cstr(s: &str) -> *const c_char {
        // 辅助：将 &str 转为 null-terminated C 字符指针
        // 使用 leak 绕过生命周期（测试中可接受）
        Box::leak(format!("{}\0", s).into_boxed_str()).as_ptr() as *const c_char
    }

    #[test]
    fn test_list_push_pop_get_set() {
        let vm = msVmNew();
        let list = msListNew(vm);

        let v1 = msInt(10);
        let v2 = msInt(20);
        let v3 = msInt(30);

        assert_eq!(msListPush(vm, list, v1), MsStatus::MS_OK);
        assert_eq!(msListPush(vm, list, v2), MsStatus::MS_OK);
        assert_eq!(msListPush(vm, list, v3), MsStatus::MS_OK);
        assert_eq!(msListLen(vm, list), 3);

        assert_eq!(msToInt(vm, msListGet(vm, list, 0)), 10);
        assert_eq!(msToInt(vm, msListGet(vm, list, -1)), 30);

        let v99 = msInt(99);
        assert_eq!(msListSet(vm, list, 1, v99), MsStatus::MS_OK);
        assert_eq!(msToInt(vm, msListGet(vm, list, 1)), 99);

        let popped = msListPop(vm, list);
        assert_eq!(msToInt(vm, popped), 30);
        assert_eq!(msListLen(vm, list), 2);

        free_value(v1); free_value(v2); free_value(v3); free_value(v99);
        free_value(popped);
        msVmFree(vm);
    }

    #[test]
    fn test_list_slice() {
        let vm = msVmNew();
        let list = msListNew(vm);

        for i in 0..6 { msListPush(vm, list, msInt(i)); }

        let sliced = msListSlice(vm, list, 1, 4, 1);
        assert_eq!(msListLen(vm, sliced), 3);
        assert_eq!(msToInt(vm, msListGet(vm, sliced, 0)), 1);

        let stepped = msListSlice(vm, list, 0, 6, 2);
        assert_eq!(msListLen(vm, stepped), 3);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 0)), 0);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 1)), 2);

        msVmFree(vm);
    }

    #[test]
    fn test_dict_set_get_remove() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        let key_a = msString(vm, cstr("a"));
        let val_1 = msInt(1);
        assert_eq!(msDictSet(vm, dict, key_a, val_1), MsStatus::MS_OK);

        let key_b = msString(vm, cstr("b"));
        let val_2 = msInt(2);
        assert_eq!(msDictSet(vm, dict, key_b, val_2), MsStatus::MS_OK);

        assert_eq!(msDictLen(vm, dict), 2);
        assert_eq!(msToInt(vm, msDictGet(vm, dict, key_a)), 1);
        assert_eq!(msDictContains(vm, dict, key_a), MS_TRUE);

        assert_eq!(msDictRemove(vm, dict, key_a), MsStatus::MS_OK);
        assert!(msDictGet(vm, dict, key_a).is_null());

        let default_val = msInt(42);
        let result = msDictGetDefault(vm, dict, msString(vm, cstr("z")), default_val);
        assert_eq!(msToInt(vm, result), 42);

        msVmFree(vm);
    }

    #[test]
    fn test_dict_keys_values_items() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        msDictSet(vm, dict, msString(vm, cstr("x")), msInt(10));
        msDictSet(vm, dict, msString(vm, cstr("y")), msInt(20));

        assert_eq!(msListLen(vm, msDictKeys(vm, dict)), 2);
        assert_eq!(msListLen(vm, msDictValues(vm, dict)), 2);

        let items = msDictItems(vm, dict);
        assert_eq!(msListLen(vm, items), 2);
        assert_eq!(msTypeof(msListGet(vm, items, 0)), MsType::Tuple);

        msVmFree(vm);
    }

    #[test]
    fn test_set_add_remove_contains() {
        let vm = msVmNew();
        let set = msSetNew(vm);

        assert_eq!(msSetLen(vm, set), 0);

        msSetAdd(vm, set, msInt(1));
        msSetAdd(vm, set, msInt(2));
        msSetAdd(vm, set, msInt(1));
        assert_eq!(msSetLen(vm, set), 2);

        assert_eq!(msSetContains(vm, set, msInt(1)), MS_TRUE);
        assert_eq!(msSetContains(vm, set, msInt(3)), MS_FALSE);

        msSetRemove(vm, set, msInt(1));
        assert_eq!(msSetContains(vm, set, msInt(1)), MS_FALSE);

        msVmFree(vm);
    }

    #[test]
    fn test_string_concat_slice() {
        let vm = msVmNew();
        let a = msString(vm, cstr("hello"));
        let b = msString(vm, cstr(" world"));

        assert_eq!(msStringLen(vm, a), 5);

        let data = msStringData(vm, a);
        let s = unsafe { std::ffi::CStr::from_ptr(data) };
        assert_eq!(s.to_str().unwrap(), "hello");

        let concat = msStringConcat(vm, a, b);
        let c_data = msStringData(vm, concat);
        let c_str = unsafe { std::ffi::CStr::from_ptr(c_data) };
        assert_eq!(c_str.to_str().unwrap(), "hello world");

        let sliced = msStringSlice(vm, concat, 0, 5);
        let sl_data = msStringData(vm, sliced);
        let sl_str = unsafe { std::ffi::CStr::from_ptr(sl_data) };
        assert_eq!(sl_str.to_str().unwrap(), "hello");

        msVmFree(vm);
    }

    #[test]
    fn test_tuple_unpack() {
        let vm = msVmNew();

        let elems: Vec<*mut MsValue> = vec![msInt(10), msInt(20), msInt(30)];
        let tup = msTupleFrom(vm, elems.as_ptr(), 3);

        assert_eq!(msTupleLen(vm, tup), 3);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, 0)), 10);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, -1)), 30);

        let mut items: *mut *mut MsValue = ptr::null_mut();
        let mut count: c_int = 0;
        assert_eq!(msTupleUnpack(vm, tup, &mut items, &mut count), MsStatus::MS_OK);
        assert_eq!(count, 3);
        assert_eq!(msToInt(vm, unsafe { *items.add(0) }), 10);

        // 安全释放（替代 libc::free）
        msTupleUnpackFree(items, count);

        msVmFree(vm);
    }

    // --- Deferred (iterator protocol not yet implemented) ---
    // test_iterator_walk: requires msIter/msNext (deferred)
}
```
