# C API — 集合操作（List/Dict/Tuple/Set + 迭代器 + 字符串操作）

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 67-capi-value-creation

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
| `MsValue` | `src/capi/vm.rs` | 值不透明结构体 |
| `ms_throw` | `src/capi/error.rs` | C API 异常抛出辅助 |
| `object_to_ms_value` | `src/capi/value.rs` | Object → MsValue* 转换 |
| `ms_value_to_object` | `src/capi/value.rs` | MsValue* → Object 转换 |
| `with_vm` | `src/capi/vm.rs` | 锁定 VM 并执行闭包 |

### 通用实现模式

每个函数遵循以下步骤：

1. 验证 `MsVM*` 和 `MsValue*` 参数非 NULL
2. 通过 `with_vm` 锁定 VM mutex
3. 调用 `ms_value_to_object` 提取内部 `Object`
4. 验证 Object 类型匹配（通过 `type_tag` 检查）
5. 执行操作
6. 返回结果（通过 `object_to_ms_value` 将 Object 转为 `MsValue*`）

### 字符串操作实现

```rust
#[no_mangle]
pub extern "C" fn msStringLen(vm: *mut MsVM, str: *mut MsValue) -> usize {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, str);
        match &obj {
            Object::Ref(ptr) if type_tag(*ptr) == TypeTag::STRING => {
                unsafe { read_str(*ptr).len() }
            }
            _ => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn msStringData(vm: *mut MsVM, str_val: *mut MsValue) -> *const c_char {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, str_val);
        match &obj {
            Object::Ref(ptr) if type_tag(*ptr) == TypeTag::STRING => {
                unsafe { read_str(*ptr).as_ptr() as *const c_char }
            }
            _ => std::ptr::null(),
        }
    })
}

#[no_mangle]
pub extern "C" fn msStringConcat(
    vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj_a = ms_value_to_object(vm_inner, a);
        let obj_b = ms_value_to_object(vm_inner, b);
        let str_a = extract_str(&obj_a);
        let str_b = extract_str(&obj_b);
        match (str_a, str_b) {
            (Some(sa), Some(sb)) => {
                let concat = format!("{}{}", sa, sb);
                let result = alloc_string(&concat);
                object_to_ms_value(vm_inner, result)
            }
            _ => std::ptr::null_mut(),
        }
    })
}

#[no_mangle]
pub extern "C" fn msStringSlice(
    vm: *mut MsVM, str_val: *mut MsValue, start: c_int, end: c_int,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, str_val);
        let s = match extract_str(&obj) {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        let len = s.len() as isize;
        let s_idx = normalize_index(start, len);
        let e_idx = normalize_index(end, len);
        let (lo, hi) = if s_idx <= e_idx {
            (s_idx as usize, e_idx as usize)
        } else {
            (0, 0)
        };
        let sliced = s[lo..hi].to_string();
        let result = alloc_string(&sliced);
        object_to_ms_value(vm_inner, result)
    })
}
```

辅助函数 `normalize_index`：

```rust
fn normalize_index(idx: c_int, len: isize) -> isize {
    if idx < 0 {
        (len + idx as isize).max(0)
    } else {
        (idx as isize).min(len)
    }
}

fn extract_str(obj: &Object) -> Option<&str> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::STRING => {
            Some(unsafe { read_str(*ptr) })
        }
        _ => None,
    }
}
```

### List 操作实现

```rust
#[no_mangle]
pub extern "C" fn msListLen(vm: *mut MsVM, list: *mut MsValue) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        match &obj {
            Object::Ref(ptr) if type_tag(*ptr) == TypeTag::LIST => {
                unsafe { read_list(*ptr).len() as c_int }
            }
            _ => -1,
        }
    })
}

#[no_mangle]
pub extern "C" fn msListGet(
    vm: *mut MsVM, list: *mut MsValue, index: c_int,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let items = match extract_list_mut(&obj) {
            Some(v) => v,
            None => return std::ptr::null_mut(),
        };
        let len = items.len() as isize;
        let idx = resolve_index(index, len);
        match idx {
            Some(i) => {
                let val = items[i].clone();
                object_to_ms_value(vm_inner, val)
            }
            None => {
                ms_throw(vm_inner, "IndexError",
                    &format!("list index {} out of range", index));
                std::ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn msListSet(
    vm: *mut MsVM, list: *mut MsValue, index: c_int, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let new_val = ms_value_to_object(vm_inner, val);
        let items = match extract_list_mut(&obj) {
            Some(v) => v,
            None => return MS_ERROR,
        };
        let len = items.len() as isize;
        let idx = resolve_index(index, len);
        match idx {
            Some(i) => {
                items[i] = new_val;
                MS_OK
            }
            None => {
                ms_throw(vm_inner, "IndexError",
                    &format!("list index {} out of range", index));
                MS_ERROR
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn msListPush(
    vm: *mut MsVM, list: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let new_val = ms_value_to_object(vm_inner, val);
        match extract_list_mut(&obj) {
            Some(items) => {
                items.push(new_val);
                MS_OK
            }
            None => MS_ERROR,
        }
    })
}

#[no_mangle]
pub extern "C" fn msListPop(vm: *mut MsVM, list: *mut MsValue) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let items = match extract_list_mut(&obj) {
            Some(v) => v,
            None => return std::ptr::null_mut(),
        };
        match items.pop() {
            Some(val) => object_to_ms_value(vm_inner, val),
            None => {
                ms_throw(vm_inner, "IndexError", "pop from empty list");
                std::ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn msListInsert(
    vm: *mut MsVM, list: *mut MsValue, index: c_int, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let new_val = ms_value_to_object(vm_inner, val);
        let items = match extract_list_mut(&obj) {
            Some(v) => v,
            None => return MS_ERROR,
        };
        let len = items.len() as isize;
        let pos = if index < 0 {
            (len + index as isize).max(0) as usize
        } else {
            (index as usize).min(items.len())
        };
        items.insert(pos, new_val);
        MS_OK
    })
}

#[no_mangle]
pub extern "C" fn msListContains(
    vm: *mut MsVM, list: *mut MsValue, val: *mut MsValue,
) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, list);
        let target = ms_value_to_object(vm_inner, val);
        match extract_list_ref(&obj) {
            Some(items) => {
                if items.contains(&target) { MS_TRUE } else { MS_FALSE }
            }
            _ => MS_FALSE,
        }
    })
}

#[no_mangle]
pub extern "C" fn msListSlice(
    vm: *mut MsVM, list: *mut MsValue,
    start: c_int, end: c_int, step: c_int,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        if step == 0 {
            ms_throw(vm_inner, "ValueError", "slice step cannot be zero");
            return std::ptr::null_mut();
        }
        let obj = ms_value_to_object(vm_inner, list);
        let items = match extract_list_ref(&obj) {
            Some(v) => v,
            None => return std::ptr::null_mut(),
        };
        let len = items.len() as isize;
        let step = step as isize;
        let (s_idx, e_idx) = compute_slice_bounds(start, end, step, len);
        let mut result = Vec::new();
        let mut i = s_idx;
        if step > 0 {
            while i < e_idx {
                result.push(items[i as usize].clone());
                i += step;
            }
        } else {
            while i > e_idx {
                result.push(items[i as usize].clone());
                i += step;
            }
        }
        let result_obj = alloc_list(result);
        object_to_ms_value(vm_inner, result_obj)
    })
}
```

辅助函数：

```rust
fn extract_list_mut(obj: &Object) -> Option<&mut Vec<Object>> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::LIST => {
            Some(unsafe { read_list(*ptr) })
        }
        _ => None,
    }
}

fn extract_list_ref(obj: &Object) -> Option<&Vec<Object>> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::LIST => {
            Some(unsafe { &*(*ptr as *const MsList).data_ptr })
        }
        _ => None,
    }
}

fn resolve_index(index: c_int, len: isize) -> Option<usize> {
    let idx = if index < 0 { len + index as isize } else { index as isize };
    if idx < 0 || idx >= len {
        None
    } else {
        Some(idx as usize)
    }
}

fn compute_slice_bounds(start: c_int, end: c_int, step: isize, len: isize)
    -> (isize, isize)
{
    let s = if start < 0 { (len + start as isize).max(0) } else { (start as isize).min(len) };
    let e = if end < 0 { (len + end as isize).max(0) } else { (end as isize).min(len) };
    (s, e)
}
```

### Dict 操作实现

```rust
#[no_mangle]
pub extern "C" fn msDictLen(vm: *mut MsVM, dict: *mut MsValue) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        match extract_dict_mut(&obj) {
            Some(map) => map.len() as c_int,
            _ => -1,
        }
    })
}

#[no_mangle]
pub extern "C" fn msDictGet(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let key_obj = ms_value_to_object(vm_inner, key);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return std::ptr::null_mut(),
        };
        match map.get(&key_obj) {
            Some(val) => object_to_ms_value(vm_inner, val.clone()),
            None => std::ptr::null_mut(),
        }
    })
}

#[no_mangle]
pub extern "C" fn msDictGetDefault(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue,
    default_val: *mut MsValue,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let key_obj = ms_value_to_object(vm_inner, key);
        let default_obj = ms_value_to_object(vm_inner, default_val);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return default_val,
        };
        match map.get(&key_obj) {
            Some(val) => object_to_ms_value(vm_inner, val.clone()),
            None => default_val,
        }
    })
}

#[no_mangle]
pub extern "C" fn msDictSet(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let key_obj = ms_value_to_object(vm_inner, key);
        let val_obj = ms_value_to_object(vm_inner, val);
        let map = match extract_dict_mut(&obj) {
            Some(m) => m,
            None => return MS_ERROR,
        };
        map.insert(key_obj, val_obj);
        MS_OK
    })
}

#[no_mangle]
pub extern "C" fn msDictRemove(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let key_obj = ms_value_to_object(vm_inner, key);
        let map = match extract_dict_mut(&obj) {
            Some(m) => m,
            None => return MS_ERROR,
        };
        if map.remove(&key_obj).is_some() {
            MS_OK
        } else {
            ms_throw(vm_inner, "KeyError", "key not found");
            MS_ERROR
        }
    })
}

#[no_mangle]
pub extern "C" fn msDictContains(
    vm: *mut MsVM, dict: *mut MsValue, key: *mut MsValue,
) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let key_obj = ms_value_to_object(vm_inner, key);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return MS_FALSE,
        };
        if map.get(&key_obj).is_some() { MS_TRUE } else { MS_FALSE }
    })
}

#[no_mangle]
pub extern "C" fn msDictKeys(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return std::ptr::null_mut(),
        };
        let keys: Vec<Object> = map.keys().cloned().collect();
        let result = alloc_list(keys);
        object_to_ms_value(vm_inner, result)
    })
}

#[no_mangle]
pub extern "C" fn msDictValues(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return std::ptr::null_mut(),
        };
        let vals: Vec<Object> = map.items().iter()
            .map(|(_, v)| (*v).clone())
            .collect();
        let result = alloc_list(vals);
        object_to_ms_value(vm_inner, result)
    })
}

#[no_mangle]
pub extern "C" fn msDictItems(vm: *mut MsVM, dict: *mut MsValue) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, dict);
        let map = match extract_dict_ref(&obj) {
            Some(m) => m,
            None => return std::ptr::null_mut(),
        };
        let items: Vec<Object> = map.items().iter()
            .map(|(k, v)| alloc_tuple(vec![(*k).clone(), (*v).clone()]))
            .collect();
        let result = alloc_list(items);
        object_to_ms_value(vm_inner, result)
    })
}
```

Dict 辅助函数：

```rust
fn extract_dict_mut(obj: &Object) -> Option<&mut DictMap> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::DICT => {
            Some(unsafe { read_dict(*ptr) })
        }
        _ => None,
    }
}

fn extract_dict_ref(obj: &Object) -> Option<&DictMap> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::DICT => {
            Some(unsafe { &*(*ptr as *const MsDict).data_ptr })
        }
        _ => None,
    }
}
```

### Tuple 操作实现

```rust
#[no_mangle]
pub extern "C" fn msTupleLen(vm: *mut MsVM, tup: *mut MsValue) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, tup);
        match &obj {
            Object::Ref(ptr) if type_tag(*ptr) == TypeTag::TUPLE => {
                unsafe { read_tuple(*ptr).len() as c_int }
            }
            _ => -1,
        }
    })
}

#[no_mangle]
pub extern "C" fn msTupleGet(
    vm: *mut MsVM, tup: *mut MsValue, index: c_int,
) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, tup);
        let items = match extract_tuple_ref(&obj) {
            Some(v) => v,
            None => return std::ptr::null_mut(),
        };
        let len = items.len() as isize;
        let idx = resolve_index(index, len);
        match idx {
            Some(i) => {
                let val = items[i].clone();
                object_to_ms_value(vm_inner, val)
            }
            None => {
                ms_throw(vm_inner, "IndexError",
                    &format!("tuple index {} out of range", index));
                std::ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn msTupleUnpack(
    vm: *mut MsVM, tup: *mut MsValue,
    items_out: *mut *mut *mut MsValue, count_out: *mut c_int,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, tup);
        let elements = match extract_tuple_ref(&obj) {
            Some(v) => v,
            None => return MS_ERROR,
        };
        let n = elements.len();
        let layout = std::alloc::Layout::array::<*mut MsValue>(n).unwrap();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut *mut MsValue };
        if ptr.is_null() {
            return MS_ERROR;
        }
        for (i, elem) in elements.iter().enumerate() {
            let ms_val = object_to_ms_value(vm_inner, elem.clone());
            unsafe { *ptr.add(i) = ms_val };
        }
        unsafe {
            *items_out = ptr;
            *count_out = n as c_int;
        }
        MS_OK
    })
}
```

Tuple 辅助函数：

```rust
fn extract_tuple_ref(obj: &Object) -> Option<&Vec<Object>> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::TUPLE => {
            Some(unsafe { read_tuple(*ptr) })
        }
        _ => None,
    }
}
```

> **msTupleUnpack 内存语义**：调用方通过 `free(items)` 释放数组本身。数组中各 `MsValue*` 指针为借用引用（borrowed ref），调用方不需要也不应该逐个释放或 unroot。若需长期持有某元素，应单独 `msRoot`。

### Set 操作实现

```rust
#[no_mangle]
pub extern "C" fn msSetLen(vm: *mut MsVM, set: *mut MsValue) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, set);
        match extract_set_mut(&obj) {
            Some(inner) => inner.len() as c_int,
            _ => -1,
        }
    })
}

#[no_mangle]
pub extern "C" fn msSetAdd(
    vm: *mut MsVM, set: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, set);
        let val_obj = ms_value_to_object(vm_inner, val);
        let inner = match extract_set_mut(&obj) {
            Some(s) => s,
            None => return MS_ERROR,
        };
        inner.insert(val_obj);
        MS_OK
    })
}

#[no_mangle]
pub extern "C" fn msSetRemove(
    vm: *mut MsVM, set: *mut MsValue, val: *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, set);
        let val_obj = ms_value_to_object(vm_inner, val);
        let inner = match extract_set_mut(&obj) {
            Some(s) => s,
            None => return MS_ERROR,
        };
        inner.remove(&val_obj);
        MS_OK
    })
}

#[no_mangle]
pub extern "C" fn msSetContains(
    vm: *mut MsVM, set: *mut MsValue, val: *mut MsValue,
) -> c_int {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, set);
        let val_obj = ms_value_to_object(vm_inner, val);
        let inner = match extract_set_ref(&obj) {
            Some(s) => s,
            None => return MS_FALSE,
        };
        if inner.contains(&val_obj) { MS_TRUE } else { MS_FALSE }
    })
}
```

Set 辅助函数：

```rust
fn extract_set_mut(obj: &Object) -> Option<&mut HashSet<Object>> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::SET => {
            Some(unsafe { read_set(*ptr) })
        }
        _ => None,
    }
}

fn extract_set_ref(obj: &Object) -> Option<&HashSet<Object>> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::SET => {
            Some(unsafe { &*(*ptr as *const MsSet).data_ptr })
        }
        _ => None,
    }
}
```

### 迭代器实现

```rust
#[no_mangle]
pub extern "C" fn msIter(vm: *mut MsVM, iterable: *mut MsValue) -> *mut MsValue {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, iterable);
        match call_iter_protocol(vm_inner, &obj) {
            Some(iter_obj) => object_to_ms_value(vm_inner, iter_obj),
            None => {
                ms_throw(vm_inner, "TypeError", "object is not iterable");
                std::ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn msNext(
    vm: *mut MsVM, iterator: *mut MsValue, out: *mut *mut MsValue,
) -> MsStatus {
    with_vm(vm, |vm_inner| {
        let obj = ms_value_to_object(vm_inner, iterator);
        match call_next_protocol(vm_inner, &obj) {
            Ok(val) => {
                unsafe { *out = object_to_ms_value(vm_inner, val) };
                MS_OK
            }
            Err(_) => MS_ERROR,
        }
    })
}
```

迭代器协议调用：

```rust
fn call_iter_protocol(vm_inner: &mut VmInner, obj: &Object) -> Option<Object> {
    // 根据 Object 类型创建对应的迭代器
    match obj {
        Object::Ref(ptr) => {
            let tag = type_tag(*ptr);
            if tag == TypeTag::LIST as u8 {
                let len = unsafe { read_list(*ptr).len() };
                Some(alloc_list_iterator(*ptr, len))
            } else if tag == TypeTag::DICT as u8 {
                let len = unsafe { read_dict(*ptr).len() };
                Some(alloc_dict_iterator(*ptr, len))
            } else if tag == TypeTag::TUPLE as u8 {
                let len = unsafe { read_tuple(*ptr).len() };
                Some(alloc_tuple_iterator(*ptr, len))
            } else if tag == TypeTag::SET as u8 {
                Some(alloc_set_iterator(*ptr))
            } else if tag == TypeTag::STRING as u8 {
                let s = unsafe { read_str(*ptr) };
                Some(alloc_string_iterator(*ptr, s.len()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn call_next_protocol(
    vm_inner: &mut VmInner, obj: &Object,
) -> Result<Object, ()> {
    match obj {
        Object::Ref(ptr) if type_tag(*ptr) == TypeTag::ITERATOR => {
            let state = unsafe { read_iterator(*ptr) };
            state.advance().ok_or(())
        }
        _ => Err(()),
    }
}
```

> **迭代器内部结构**：迭代器为 `TypeTag::ITERATOR` 类型的堆对象，`data_ptr` 指向包含源对象引用和当前位置索引的结构。`advance()` 方法根据源对象类型提取下一个元素，越界时返回 `None`（对应 StopIteration）。

### 写屏障集成

所有修改堆对象的操作（`msListSet`、`msListPush`、`msListInsert`、`msDictSet`、`msSetAdd`）内部需要触发写屏障，通知 GC 父对象引用了新值。

在 Task 67 中已实现的 `with_vm` 上下文中，写屏障由 VM 的 GC 子系统自动管理。对于直接修改集合内部的场景，需要在修改后调用：

```rust
// 在 msListPush 成功 push 后：
vm_inner.gc.write_barrier(parent_obj, new_val);

// 在 msDictSet 成功 insert 后：
vm_inner.gc.write_barrier(dict_obj, val_obj);
```

这确保并发三色标记清扫 GC 不会丢失对新增引用的追踪。

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

### Rust 单元测试 — test_list_push_pop_get_set

```rust
#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    use super::*;

    #[test]
    fn test_list_push_pop_get_set() {
        let vm = msVmNew();
        let list = msListNew(vm);

        let v1 = msInt(10);
        let v2 = msInt(20);
        let v3 = msInt(30);

        assert_eq!(msListPush(vm, list, v1), MS_OK);
        assert_eq!(msListPush(vm, list, v2), MS_OK);
        assert_eq!(msListPush(vm, list, v3), MS_OK);
        assert_eq!(msListLen(vm, list), 3);

        assert_eq!(msToInt(vm, msListGet(vm, list, 0)), 10);
        assert_eq!(msToInt(vm, msListGet(vm, list, -1)), 30);

        let v99 = msInt(99);
        assert_eq!(msListSet(vm, list, 1, v99), MS_OK);
        assert_eq!(msToInt(vm, msListGet(vm, list, 1)), 99);

        let popped = msListPop(vm, list);
        assert_eq!(msToInt(vm, popped), 30);
        assert_eq!(msListLen(vm, list), 2);

        msVmFree(vm);
    }
}
```

### Rust 单元测试 — test_list_slice

```rust
    #[test]
    fn test_list_slice() {
        let vm = msVmNew();
        let list = msListNew(vm);

        for i in 0..6 {
            msListPush(vm, list, msInt(i));
        }

        let sliced = msListSlice(vm, list, 1, 4, 1);
        assert_eq!(msListLen(vm, sliced), 3);
        assert_eq!(msToInt(vm, msListGet(vm, sliced, 0)), 1);
        assert_eq!(msToInt(vm, msListGet(vm, sliced, 2)), 3);

        let stepped = msListSlice(vm, list, 0, 6, 2);
        assert_eq!(msListLen(vm, stepped), 3);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 0)), 0);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 1)), 2);
        assert_eq!(msToInt(vm, msListGet(vm, stepped, 2)), 4);

        let neg = msListSlice(vm, list, -3, -1, 1);
        assert_eq!(msListLen(vm, neg), 2);
        assert_eq!(msToInt(vm, msListGet(vm, neg, 0)), 3);
        assert_eq!(msToInt(vm, msListGet(vm, neg, 1)), 4);

        msVmFree(vm);
    }
```

### Rust 单元测试 — test_dict_set_get_remove

```rust
    #[test]
    fn test_dict_set_get_remove() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        let key_a = msString(vm, "a");
        let val_1 = msInt(1);
        assert_eq!(msDictSet(vm, dict, key_a, val_1), MS_OK);

        let key_b = msString(vm, "b");
        let val_2 = msInt(2);
        assert_eq!(msDictSet(vm, dict, key_b, val_2), MS_OK);

        assert_eq!(msDictLen(vm, dict), 2);

        let got = msDictGet(vm, dict, key_a);
        assert!(!got.is_null());
        assert_eq!(msToInt(vm, got), 1);

        assert_eq!(msDictContains(vm, dict, key_a), MS_TRUE);
        assert_eq!(msDictContains(vm, dict, msString(vm, "z")), MS_FALSE);

        assert_eq!(msDictRemove(vm, dict, key_a), MS_OK);
        assert_eq!(msDictLen(vm, dict), 1);
        assert!(msDictGet(vm, dict, key_a).is_null());

        let default_val = msInt(42);
        let result = msDictGetDefault(vm, dict, msString(vm, "z"), default_val);
        assert_eq!(msToInt(vm, result), 42);

        msVmFree(vm);
    }
```

### Rust 单元测试 — test_dict_keys_values_items

```rust
    #[test]
    fn test_dict_keys_values_items() {
        let vm = msVmNew();
        let dict = msDictNew(vm);

        msDictSet(vm, dict, msString(vm, "x"), msInt(10));
        msDictSet(vm, dict, msString(vm, "y"), msInt(20));

        let keys = msDictKeys(vm, dict);
        assert_eq!(msListLen(vm, keys), 2);

        let vals = msDictValues(vm, dict);
        assert_eq!(msListLen(vm, vals), 2);

        let items = msDictItems(vm, dict);
        assert_eq!(msListLen(vm, items), 2);
        assert_eq!(msTypeof(msListGet(vm, items, 0)), MS_TYPE_TUPLE);

        msVmFree(vm);
    }
```

### Rust 单元测试 — test_set_add_remove_contains

```rust
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
        assert_eq!(msSetLen(vm, set), 1);

        msVmFree(vm);
    }
```

### Rust 单元测试 — test_string_concat_slice

```rust
    #[test]
    fn test_string_concat_slice() {
        let vm = msVmNew();
        let a = msString(vm, "hello");
        let b = msString(vm, " world");

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
```

### Rust 单元测试 — test_tuple_unpack

```rust
    #[test]
    fn test_tuple_unpack() {
        let vm = msVmNew();

        let elems: Vec<*mut MsValue> = vec![
            msInt(10), msInt(20), msInt(30),
        ];
        let tup = msTupleFrom(vm, elems.as_ptr(), 3);

        assert_eq!(msTupleLen(vm, tup), 3);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, 0)), 10);
        assert_eq!(msToInt(vm, msTupleGet(vm, tup, -1)), 30);

        let mut items: *mut *mut MsValue = std::ptr::null_mut();
        let mut count: c_int = 0;
        assert_eq!(msTupleUnpack(vm, tup, &mut items, &mut count), MS_OK);
        assert_eq!(count, 3);
        assert_eq!(msToInt(vm, unsafe { *items.add(0) }), 10);
        assert_eq!(msToInt(vm, unsafe { *items.add(2) }), 30);

        if !items.is_null() {
            unsafe { libc::free(items as *mut c_void) };
        }

        msVmFree(vm);
    }
```

### Rust 单元测试 — test_iterator_walk

```rust
    #[test]
    fn test_iterator_walk() {
        let vm = msVmNew();
        let list = msListNew(vm);

        msListPush(vm, list, msInt(100));
        msListPush(vm, list, msInt(200));
        msListPush(vm, list, msInt(300));

        let iter = msIter(vm, list);
        assert!(!iter.is_null());

        let mut collected: Vec<i64> = Vec::new();
        let mut out: *mut MsValue = std::ptr::null_mut();

        while msNext(vm, iter, &mut out) == MS_OK {
            collected.push(msToInt(vm, out));
        }

        assert_eq!(collected, vec![100, 200, 300]);

        msVmFree(vm);
    }
```
