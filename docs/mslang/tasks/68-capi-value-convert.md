# C API — 值转换、比较与通用操作

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 67-capi-value-creation（值创建、类型判断已实现）

## 目标

实现 `value.h` 中段 API，覆盖以下四个功能组：

1. **值转换**：`msToInt` / `msToFloat` / `msToBool` / `msToString` / `msToStringCopy` — 从 `MsValue` 提取 C 原生值
2. **显式类型转换**：`msConvertInt` / `msConvertFloat` / `msConvertStr` / `msConvertBool` / `msConvertList` — 对应 mslang 内置函数 `int()` / `float()` / `str()` / `bool()` / `list()`
3. **比较操作**：`msEq` / `msLt` / `msLe` / `msGt` / `msGe` / `msIs` / `msHash` — 值比较与哈希
4. **通用属性/下标访问**：`msGetAttr` / `msSetAttr` / `msGetItem` / `msSetItem` / `msLen` / `msRepr` — 统一的属性访问与表示

## 设计规格

参照 [13-capi.md](../13-capi.md) § value.h（值转换 + 显式类型转换 + 比较 + 通用属性/下标访问）。

### 值转换

```c
MS_API int64_t     msToInt(MsVM* vm, MsValue* val);
MS_API double      msToFloat(MsVM* vm, MsValue* val);
MS_API int         msToBool(MsValue* val);
MS_API const char* msToString(MsVM* vm, MsValue* val);
MS_API char*       msToStringCopy(MsVM* vm, MsValue* val);
```

| 函数 | 行为 |
|---|---|
| `msToInt` | `Int` → i64；`Float` → 截断为 i64；其余类型设置 TypeError 异常并返回 0 |
| `msToFloat` | `Int` → f64；`Float` → f64；其余类型设置 TypeError 异常并返回 0.0 |
| `msToBool` | 按 truthy 规则转换，返回 `MS_TRUE`/`MS_FALSE`，不设异常 |
| `msToString` | 返回内部指针（借用引用），不需要 free，仅在 val 存活期间有效 |
| `msToStringCopy` | 返回副本（`strdup`），调用方必须 `free()` |

**truthy 规则**（参照 [02-types.md](../02-types.md)）：
- Truthy：`true`、非零数值、非空字符串、非空集合
- Falsy：`false`、`nil`、`0`、`0.0`、`""`、空集合

**msToString 内部缓冲区**：格式化字符串存储在 `thread_local` 缓冲区中，每次调用覆盖上次内容。String 类型直接返回内部数据指针，跳过格式化。

### 显式类型转换

```c
MS_API MsValue* msConvertInt(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertFloat(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertStr(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertBool(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertList(MsVM* vm, MsValue* val);
```

对应 mslang 内置转换函数，返回新的 `MsValue*`。转换失败时设置异常并返回 NULL。

| 函数 | 转换规则 |
|---|---|
| `msConvertInt` | Bool→0/1；Int→自身；Float→截断；String→解析为整数；其余报错 |
| `msConvertFloat` | Bool→0.0/1.0；Int→f64；Float→自身；String→解析为浮点；其余报错 |
| `msConvertStr` | 所有类型使用 Display 格式化，String→自身 |
| `msConvertBool` | 按 truthy 规则，返回 MS_TRUE_VAL/MS_FALSE_VAL |
| `msConvertList` | String→字符列表；List→自身；Tuple/Set→转换；Dict→key 列表；其余报错 |

### 比较

```c
MS_API int     msEq(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msLt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msLe(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msGt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msGe(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msIs(MsValue* a, MsValue* b);
MS_API int64_t msHash(MsVM* vm, MsValue* val);
```

| 函数 | 行为 |
|---|---|
| `msEq` | 值相等比较（`==`），返回 `MS_TRUE`/`MS_FALSE` |
| `msLt`/`msLe`/`msGt`/`msGe` | 顺序比较，类型不兼容时设置 TypeError |
| `msIs` | 身份比较（`is`），对 Ref 类型比较指针，对值类型比较值 |
| `msHash` | 返回哈希值，不可哈希类型设置异常并返回 0 |

**哈希规则**：Nil、Bool、Int、Float、String 可哈希。Tuple 当且仅当所有元素可哈希时可哈希。其余类型不可哈希。

### 通用属性/下标访问

```c
MS_API MsValue*  msGetAttr(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus  msSetAttr(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API MsValue*  msGetItem(MsVM* vm, MsValue* obj, MsValue* key);
MS_API MsStatus  msSetItem(MsVM* vm, MsValue* obj, MsValue* key, MsValue* val);
MS_API int64_t   msLen(MsVM* vm, MsValue* val);
MS_API MsValue*  msRepr(MsVM* vm, MsValue* val);
```

| 函数 | 行为 |
|---|---|
| `msGetAttr` | 获取命名属性。Instance → 字段/方法；Module → 导出；Class → 静态成员；其余报错 |
| `msSetAttr` | 设置命名属性。仅 Instance 和可变对象支持 |
| `msGetItem` | 获取下标。List[int]、Dict[key]、String[int]、Tuple[int] |
| `msSetItem` | 设置下标。仅 List 和 Dict 支持 |
| `msLen` | 通用长度。String、List、Dict、Tuple、Set 返回元素数；其余设置异常并返回 -1 |
| `msRepr` | 返回表示字符串（类似 Python `repr()`）。String 带引号，对象显示类型信息 |

## 实现细节

文件：`src/capi/value.rs`（追加到任务 67 同一文件）。

> **前置任务更新**：本任务依赖 Task 67（值创建/类型判断）。`msGetAttr`/`msSetAttr`/`msGetItem`/`msSetItem` 涉及集合操作（Task 69）和 Class 操作（Task 73），需标注为 deferred 或提供占位实现。`set_type_error` 需 Task 71（msThrowTypeError），本任务先用 VM 内部错误标志占位。

> **MsValue 访问约定**：MsValue 的字段为 `pub(crate) inner: Object`（非方法）。只读访问用 `unsafe { &(*val).inner }`；需 VM 锁的操作使用 `lock_vm` + `guard.get()`。MsValue 无 `MsValueInner` 包装、无 `cached_str` 字段。

### 1. msToInt

```rust
#[no_mangle]
pub extern "C" fn msToInt(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return 0;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    match unsafe { &(*val).inner } {
        Object::Int(n) => *n,
        Object::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                set_type_error(vm, "finite number", "float");
                0
            } else {
                *f as i64
            }
        }
        _ => {
            set_type_error(vm, "int or float", unsafe { &(*val).inner });
            0
        }
    }
}
```

### 2. msToFloat

```rust
#[no_mangle]
pub extern "C" fn msToFloat(vm: *mut MsVM, val: *mut MsValue) -> f64 {
    if val.is_null() {
        return 0.0;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    match unsafe { &(*val).inner } {
        Object::Int(n) => *n as f64,
        Object::Float(f) => *f,
        _ => {
            set_type_error(vm, "int or float", unsafe { &(*val).inner });
            0.0
        }
    }
}
```

### 3. msToBool

```rust
#[no_mangle]
pub extern "C" fn msToBool(val: *mut MsValue) -> c_int {
    if val.is_null() {
        return MS_FALSE;
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    if unsafe { &(*val).inner }.is_truthy() {
        MS_TRUE
    } else {
        MS_FALSE
    }
}
```

不设异常。`is_truthy()` 来自任务 20 `Object` 的真值实现。

### 4. msToString / msToStringCopy

```rust
/// thread_local 缓冲区，避免修改 MsValue 的 #[repr(C)] 布局。
thread_local! {
    static TO_STRING_BUF: std::cell::RefCell<Option<CString>> = std::cell::RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn msToString(vm: *mut MsVM, val: *mut MsValue) -> *const c_char {
    if val.is_null() {
        return std::ptr::null();
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    match obj {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                unsafe { read_str(*ptr) }.as_ptr() as *const c_char
            } else {
                format_to_thread_local(obj)
            }
        }
        _ => format_to_thread_local(obj),
    }
}

fn format_to_thread_local(obj: &Object) -> *const c_char {
    let formatted = format!("{}", obj);
    let cstr = CString::new(formatted).unwrap_or_default();
    let ptr = cstr.as_ptr();
    TO_STRING_BUF.with(|buf| {
        *buf.borrow_mut() = Some(cstr);
    });
    ptr
}
```

> **嵌入 `\0` 限制**：mslang String 可包含 `\0`（通过 `msStringn` 创建）。但 `msToString` 返回 `*const c_char`（C 空终止字符串），数据在首个 `\0` 后不可见。C 侧需要完整字节的应用应使用 `msStringLen`（Task 69）+ `msStringData`（Task 69）获取长度和原始指针。

`msToStringCopy` 返回 C 可 `free()` 的副本。使用 Rust `CString::into_raw` 分配，C 侧 `free()` 释放（Rust 分配器与 C free 在主流平台兼容）：

```rust
#[no_mangle]
pub extern "C" fn msToStringCopy(vm: *mut MsVM, val: *mut MsValue) -> *mut c_char {
    let ptr = msToString(vm, val);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: ptr 来自 msToString，指向有效 C 字符串。
    let cstr = unsafe { CStr::from_ptr(ptr) };
    let owned = CString::new(cstr.to_bytes()).unwrap_or_default();
    owned.into_raw()
}
```

> **替代方案**：若需平台严格的 C malloc/free 一致性，可在 C 侧提供 `msStringCopyImpl` 包装函数（类似 Task 67 的 msStringFmt C shim），使用 `malloc` + `memcpy`。

### 5. 显式类型转换

以 `msConvertInt` 为例：

```rust
#[no_mangle]
pub extern "C" fn msConvertInt(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: val 由 ms* 创建，指向有效 MsValue。
    let obj = unsafe { &(*val).inner };
    let result = match obj {
        Object::Bool(b) => Some(Object::Int(if *b { 1 } else { 0 })),
        Object::Int(_) => Some(obj.clone()),
        Object::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                None
            } else {
                Some(Object::Int(*f as i64))
            }
        }
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING。
                let s = unsafe { read_str(*ptr) };
                parse_int_string(s)
            } else {
                None
            }
        }
        _ => None,
    };
    match result {
        Some(o) => Box::into_raw(Box::new(MsValue { inner: o })),
        None => {
            set_type_error(vm, "convertible type", obj);
            std::ptr::null_mut()
        }
    }
}
```

> **字符串整数解析**：`parse_int_string` 需支持十进制、十六进制（`0x`）、二进制（`0b`）、八进制（`0o`）前缀（`02-types.md:52-59`）。`s.parse::<i64>()` 仅支持十进制。实现时需检测前缀并使用 `i64::from_str_radix`。解析失败返回 `None`（设置 TypeError）；超出 i64 范围设置 OverflowError。

其余 `msConvertFloat`、`msConvertStr`、`msConvertBool`、`msConvertList` 按相同模式实现，转换规则见设计规格表格。所有函数使用 `unsafe { &(*val).inner }` 访问 Object。

`msConvertBool` 直接调用 `is_truthy()` 返回 `MS_TRUE_VAL`/`MS_FALSE_VAL`，无需分配。

`msConvertList` 需处理各可迭代类型的转换：String → 字符列表，Tuple/Set → 新 List，Dict → key 列表，List → 浅拷贝。

### 6. 比较操作

> **注意**：Object 未实现 `PartialOrd` trait。`msLt`/`msLe`/`msGt`/`msGe` 需自行实现比较逻辑，按 `02-types.md:280-307` 规则：Int/Float 数值比较、String 字典序、其余跨类型返回 None（设置 TypeError）。

```rust
#[no_mangle]
pub extern "C" fn msEq(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    if obj_a == obj_b { MS_TRUE } else { MS_FALSE }
}

#[no_mangle]
pub extern "C" fn msLt(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match compare_objects(obj_a, obj_b) {
        Some(std::cmp::Ordering::Less) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}
```

`compare_objects` 按 `02-types.md` 规则实现：

```rust
fn compare_objects(a: &Object, b: &Object) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (a, b) {
        (Object::Int(x), Object::Int(y)) => Some(x.cmp(y)),
        (Object::Float(x), Object::Float(y)) => x.partial_cmp(y),
        (Object::Int(x), Object::Float(y)) => (*x as f64).partial_cmp(y),
        (Object::Float(x), Object::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Object::Bool(x), Object::Bool(y)) => Some(x.cmp(y)),
        (Object::Nil, Object::Nil) => Some(Ordering::Equal),
        // String 字典序比较
        (Object::Ref(pa), Object::Ref(pb)) => {
            let ta = unsafe { (**pa).type_tag };
            let tb = unsafe { (**pb).type_tag };
            if ta == TypeTag::STRING as u8 && tb == TypeTag::STRING as u8 {
                let sa = unsafe { read_str(*pa) };
                let sb = unsafe { read_str(*pb) };
                Some(sa.cmp(sb))
            } else {
                None // 其余引用类型不可顺序比较
            }
        }
        _ => None, // 跨类型比较返回 None → TypeError
    }
}
```

`msLe`/`msGt`/`msGe` 结构相同，仅替换 `Ordering` 变体判断。

### 7. msIs（身份比较）

> **02-types.md:311-324**：`is` 对内联值（int、float、bool、nil）**抛出 TypeError**。仅 Ref 类型进行身份（指针）比较。
>
> **API 签名冲突**：`13-capi.md:329` 的 `msIs` 签名不含 `vm` 参数，无法设置 TypeError 异常。本任务采用务实方案：对内联值返回 `MS_FALSE`，不设置异常（Task 71 的错误机制完成后可添加 thread_local 错误标志）。完整 TypeError 行为留待 Task 71/75 集成时处理。

```rust
#[no_mangle]
pub extern "C" fn msIs(a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a.is_null() || b.is_null() {
        return MS_FALSE;
    }
    // SAFETY: a/b 由 ms* 创建。
    let obj_a = unsafe { &(*a).inner };
    let obj_b = unsafe { &(*b).inner };
    match (obj_a, obj_b) {
        (Object::Ref(p1), Object::Ref(p2)) => {
            if p1 == p2 { MS_TRUE } else { MS_FALSE }
        }
        // 内联值：02-types.md 规定 is 应抛 TypeError。
        // 但 msIs 签名无 vm 参数，无法设异常。暂返回 MS_FALSE。
        _ => MS_FALSE,
    }
}
```

### 8. msHash

> **注意**：Object 的 `Hash` impl 对 List/Dict/Set/NaN 会 **panic**（`object.rs:2349`, `2361`）。`msHash` 必须自行检查可哈希性，绝不调用可能 panic 的 `.hash()`。

```rust
#[no_mangle]
pub extern "C" fn msHash(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return 0;
    }
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    if !is_hashable(obj) {
        set_type_error(vm, "hashable type", obj);
        return 0;
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    obj.hash(&mut hasher);
    hasher.finish() as i64
}

/// 检查 Object 是否可哈希（不调用 .hash()，避免 panic）。
fn is_hashable(obj: &Object) -> bool {
    match obj {
        Object::Nil | Object::Bool(_) | Object::Int(_) => true,
        Object::Float(f) => !f.is_nan(), // NaN 不可哈希（02-types.md:352）
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            tag == TypeTag::STRING as u8 || tag == TypeTag::TUPLE as u8
            // Tuple 可哈希当且仅当所有元素可哈希；此处保守返回 true，
            // 实际哈希时若含不可哈希元素仍会 panic。需递归检查 tuple 元素。
        }
    }
}
```

### 9. msGetAttr / msSetAttr

> **Deferred 说明**：`msGetAttr`/`msSetAttr` 涉及 Instance 字段（Task 73）、Module 导出（Task 45）、Class 静态成员（Task 40）的底层访问。本任务提供框架代码，实际 `get_instance_attr`/`get_module_export`/`get_class_member` 辅助函数由 Task 73 补充。本任务可先返回 TypeError 占位。

```rust
#[no_mangle]
pub extern "C" fn msGetAttr(
    vm: *mut MsVM, obj: *mut MsValue, attr: *const c_char,
) -> *mut MsValue {
    if obj.is_null() || attr.is_null() {
        return std::ptr::null_mut();
    }
    let attr_str = unsafe { CStr::from_ptr(attr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: obj 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    match inner {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            match tag {
                t if t == TypeTag::INSTANCE as u8 => {
                    // TODO(task 73): get_instance_attr(vm, ptr, &attr_str)
                    set_type_error(vm, "instance attribute access (task 73)", inner);
                    std::ptr::null_mut()
                }
                t if t == TypeTag::MODULE as u8 => {
                    // TODO(task 45/73): get_module_export(vm, ptr, &attr_str)
                    set_type_error(vm, "module export access (task 73)", inner);
                    std::ptr::null_mut()
                }
                t if t == TypeTag::CLASS as u8 => {
                    // TODO(task 73): get_class_member(vm, ptr, &attr_str)
                    set_type_error(vm, "class member access (task 73)", inner);
                    std::ptr::null_mut()
                }
                _ => {
                    set_type_error(vm, "object with attributes", inner);
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "object with attributes", inner);
            std::ptr::null_mut()
        }
    }
}
```

`msSetAttr` 结构相同，返回 `MsStatus`。仅 Instance 和支持属性设置的类型允许操作。

### 10. msGetItem / msSetItem

> **Deferred 说明**：`msGetItem`/`msSetItem` 委托 List/Dict/String/Tuple 的下标操作。完整实现依赖 Task 69（集合操作）。本任务提供框架，辅助函数由 Task 69 补充。

```rust
#[no_mangle]
pub extern "C" fn msGetItem(
    vm: *mut MsVM, obj: *mut MsValue, key: *mut MsValue,
) -> *mut MsValue {
    if obj.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: obj/key 由 ms* 创建。
    let inner = unsafe { &(*obj).inner };
    let key_obj = unsafe { &(*key).inner };
    match inner {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            match tag {
                t if t == TypeTag::LIST as u8 => {
                    // TODO(task 69): list_get(vm, ptr, key_obj)
                    set_type_error(vm, "list indexing (task 69)", inner);
                    std::ptr::null_mut()
                }
                t if t == TypeTag::DICT as u8 => {
                    // TODO(task 69): dict_get(vm, ptr, key_obj)
                    set_type_error(vm, "dict indexing (task 69)", inner);
                    std::ptr::null_mut()
                }
                t if t == TypeTag::STRING as u8 => {
                    // TODO(task 69): string_get(vm, ptr, key_obj)
                    set_type_error(vm, "string indexing (task 69)", inner);
                    std::ptr::null_mut()
                }
                t if t == TypeTag::TUPLE as u8 => {
                    // TODO(task 69): tuple_get(vm, ptr, key_obj)
                    set_type_error(vm, "tuple indexing (task 69)", inner);
                    std::ptr::null_mut()
                }
                _ => {
                    set_type_error(vm, "subscriptable type", inner);
                    std::ptr::null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "subscriptable type", inner);
            std::ptr::null_mut()
        }
    }
}
```

`msSetItem` 仅支持 List 和 Dict，其余类型设置 TypeError。

### 11. msLen

```rust
#[no_mangle]
pub extern "C" fn msLen(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    if val.is_null() {
        return -1;
    }
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    match obj {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                unsafe { read_str(*ptr) }.len() as i64
            } else if tag == TypeTag::LIST as u8 {
                // SAFETY: type_tag 为 LIST。
                unsafe { read_list(*ptr) }.len() as i64
            } else if tag == TypeTag::DICT as u8 {
                // SAFETY: type_tag 为 DICT。
                unsafe { read_dict(*ptr) }.len() as i64
            } else if tag == TypeTag::TUPLE as u8 {
                // SAFETY: type_tag 为 TUPLE。
                unsafe { read_tuple(*ptr) }.len() as i64
            } else if tag == TypeTag::SET as u8 {
                // SAFETY: type_tag 为 SET。
                unsafe { read_set(*ptr) }.len() as i64
            } else {
                set_type_error(vm, "type with length", obj);
                -1
            }
        }
        _ => {
            set_type_error(vm, "type with length", obj);
            -1
        }
    }
}
```

### 12. msRepr

```rust
#[no_mangle]
pub extern "C" fn msRepr(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    if val.is_null() {
        return msNil();
    }
    // SAFETY: val 由 ms* 创建。
    let obj = unsafe { &(*val).inner };
    let repr_str = repr_object(obj);
    let new_obj = alloc_string(&repr_str);
    Box::into_raw(Box::new(MsValue { inner: new_obj }))
}

fn repr_object(obj: &Object) -> String {
    match obj {
        Object::Nil => "nil".to_string(),
        Object::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Object::Int(n) => format!("{}", n),
        Object::Float(f) => format!("{}", f),
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                let s = unsafe { read_str(*ptr) };
                format!("{:?}", s)  // 带引号和转义
            } else {
                // List/Dict/Tuple/Set/Instance 等递归 repr
                // 使用 Display 作为 fallback（Display 已含正确格式）
                format!("{}", obj)
            }
        }
    }
}
```

String 的 repr 包含引号和转义字符。List/Dict 递归调用 repr 生成子元素的表示。

### 辅助函数

`set_type_error` 定义在 `src/capi/mod.rs`。Task 71 实现 `msThrowTypeError` 前，使用 VM 内部错误标志占位：

```rust
/// 设置 TypeError 异常。Task 71 完成后委托给 msThrowTypeError。
pub(crate) unsafe fn set_type_error(vm: *mut MsVM, expected: &str, actual: &Object) {
    // TODO(task 71): msThrowTypeError(vm, expected, actual.type_name());
    // 占位：在 VM 上设置错误标记
    let guard = lock_vm(vm);
    let inner = &mut *guard.get();
    inner.vm.has_error = true;
    inner.vm.error_message = format!(
        "TypeError: expected {}, got {}",
        expected, actual.type_name()
    );
}
```

> **注意**：`VM.has_error` / `VM.error_message` 字段需确认是否存在。若 VM 无此字段，Task 68 需在 VM 上添加（或等待 Task 71 提供完整错误机制后再实现 set_type_error）。

### MsValue 无需修改

Task 66 已定义 `MsValue { pub(crate) inner: Object }`。本任务**不修改 MsValue 结构**——不添加 `cached_str` 字段。`msToString` 的非 String 类型缓冲使用 `thread_local!`（见 §4），避免修改 `#[repr(C)]` 布局。

## 验证标准

1. `msToInt(vm, Int(42))` 返回 `42`
2. `msToInt(vm, Float(3.7))` 返回 `3`（截断）
3. `msToInt(vm, String("hello"))` 设置 TypeError，返回 `0`
4. `msToFloat(vm, Int(42))` 返回 `42.0`
5. `msToBool(nil)` 返回 `MS_FALSE`
6. `msToBool(Int(1))` 返回 `MS_TRUE`
7. `msToBool(Int(0))` 返回 `MS_FALSE`
8. `msToString(vm, Int(42))` 返回 `"42"`（借用引用）
9. `msToStringCopy(vm, Int(42))` 返回可 `free()` 的副本
10. `msConvertInt(vm, Bool(true))` 返回 `Int(1)` 的 `MsValue*`
11. `msConvertStr(vm, Int(42))` 返回 `String("42")` 的 `MsValue*`
12. `msEq(vm, Int(1), Int(1))` 返回 `MS_TRUE`
13. `msLt(vm, Int(1), Int(2))` 返回 `MS_TRUE`
14. `msIs(list_a, list_a)` 同一 Ref 指针返回 `MS_TRUE`
15. `msIs(list_a, list_b)` 不同 Ref 指针返回 `MS_FALSE`
16. `msIs(Int(1), Int(1))` 内联值返回 `MS_FALSE`（`13-capi.md` 签名无 vm，无法设 TypeError；02-types.md 规定 `is` 对内联值应抛 TypeError，完整行为待 Task 71 集成）
17. `msHash(vm, Int(42))` 返回非零哈希值
18. `msHash(vm, List)` 不可哈希类型设置异常并返回 `0`
19. `msLen(vm, list_of_3)` 返回 `3`
20. `msLen(vm, string("hello"))` 返回 `5`
21. `msRepr(vm, String("hello"))` 返回 `'"hello"'`（带引号）
22. `msRepr(vm, Int(42))` 返回 `'42'`
23. `msRepr(vm, nil)` 返回 `'nil'`
24. `msToString(vm, NULL)` 返回 NULL（NULL 安全）
25. `msToInt(vm, NULL)` 返回 0（NULL 安全）

> **Deferred（Task 69/73 完成后验证）**：
> - `msGetAttr` / `msSetAttr` 往返（Task 73 Instance 字段访问）
> - `msGetItem` / `msSetItem` 往返（Task 69 List/Dict 下标操作）

## 测试用例

Rust 单元测试位于 `src/capi/value.rs`：

```rust
#[cfg(test)]
mod tests_convert {
    use super::*;
    use crate::capi::vm::{msVmFree, msVmNew};
    use std::ffi::CString;
    use std::os::raw::c_void;
    use std::ptr;

    fn free_value(val: *mut MsValue) {
        if !val.is_null() {
            unsafe { let _ = Box::from_raw(val); }
        }
    }

    #[test]
    fn test_to_int_float_bool() {
        let vm = msVmNew();

        let int_val = msInt(42);
        assert_eq!(msToInt(vm, int_val), 42);
        assert_eq!(msToFloat(vm, int_val), 42.0);
        assert_eq!(msToBool(int_val), MS_TRUE);

        let float_val = msFloat(3.7);
        assert_eq!(msToInt(vm, float_val), 3);
        assert_eq!(msToFloat(vm, float_val), 3.7);

        let zero_val = msInt(0);
        assert_eq!(msToBool(zero_val), MS_FALSE);

        let nil_val = msNil();
        assert_eq!(msToBool(nil_val), MS_FALSE);
        assert_eq!(msToInt(vm, nil_val), 0);

        // NULL safety
        assert_eq!(msToInt(vm, ptr::null_mut()), 0);
        assert_eq!(msToFloat(vm, ptr::null_mut()), 0.0);
        assert_eq!(msToBool(ptr::null_mut()), MS_FALSE);

        free_value(int_val);
        free_value(float_val);
        free_value(zero_val);
        free_value(nil_val);
        msVmFree(vm);
    }

    #[test]
    fn test_to_string_and_copy() {
        let vm = msVmNew();

        let int_val = msInt(42);
        let s = msToString(vm, int_val);
        let cstr = unsafe { CStr::from_ptr(s) };
        assert_eq!(cstr.to_str().unwrap(), "42");

        let copy = msToStringCopy(vm, int_val);
        assert!(!copy.is_null());
        let cstr_copy = unsafe { CStr::from_ptr(copy) };
        assert_eq!(cstr_copy.to_str().unwrap(), "42");
        // 释放 CString::into_raw 分配的副本
        unsafe { let _ = CString::from_raw(copy); }

        // NULL safety
        assert!(msToString(vm, ptr::null_mut()).is_null());

        free_value(int_val);
        msVmFree(vm);
    }

    #[test]
    fn test_equality_comparisons() {
        let vm = msVmNew();

        let a = msInt(1);
        let b = msInt(1);
        let c = msInt(2);

        assert_eq!(msEq(vm, a, b), MS_TRUE);
        assert_eq!(msEq(vm, a, c), MS_FALSE);

        let nil_a = msNil();
        let nil_b = msNil();
        assert_eq!(msEq(vm, nil_a, nil_b), MS_TRUE);

        free_value(a);
        free_value(b);
        free_value(c);
        free_value(nil_a);
        free_value(nil_b);
        msVmFree(vm);
    }

    #[test]
    fn test_ordering_comparisons() {
        let vm = msVmNew();

        let a = msInt(1);
        let b = msInt(2);

        assert_eq!(msLt(vm, a, b), MS_TRUE);
        assert_eq!(msLe(vm, a, b), MS_TRUE);
        assert_eq!(msGt(vm, b, a), MS_TRUE);
        assert_eq!(msGe(vm, b, a), MS_TRUE);
        assert_eq!(msLt(vm, a, a), MS_FALSE);
        assert_eq!(msLe(vm, a, a), MS_TRUE);

        // Float comparison
        let f1 = msFloat(1.5);
        let f2 = msFloat(2.5);
        assert_eq!(msLt(vm, f1, f2), MS_TRUE);

        free_value(a);
        free_value(b);
        free_value(f1);
        free_value(f2);
        msVmFree(vm);
    }

    #[test]
    fn test_identity_comparison() {
        let vm = msVmNew();

        // 引用类型：is 比较指针
        let list_a = msListNew(vm);
        let list_b = msListNew(vm);
        assert_eq!(msIs(list_a, list_a), MS_TRUE);
        assert_eq!(msIs(list_a, list_b), MS_FALSE);

        // 内联值：is 返回 MS_FALSE（签名无 vm，无法设 TypeError）
        let i1 = msInt(42);
        let i2 = msInt(42);
        assert_eq!(msIs(i1, i2), MS_FALSE);

        free_value(list_a);
        free_value(list_b);
        free_value(i1);
        free_value(i2);
        msVmFree(vm);
    }

    #[test]
    fn test_hash() {
        let vm = msVmNew();

        let int_val = msInt(42);
        let h = msHash(vm, int_val);
        assert_ne!(h, 0);  // 42 的哈希非零

        let str_val = msString(vm, b"hello\0".as_ptr() as *const c_char);
        let h2 = msHash(vm, str_val);
        assert_ne!(h2, 0);

        // 不可哈希类型返回 0
        let list_val = msListNew(vm);
        assert_eq!(msHash(vm, list_val), 0);  // List 不可哈希

        free_value(int_val);
        free_value(str_val);
        free_value(list_val);
        msVmFree(vm);
    }

    #[test]
    fn test_convert() {
        let vm = msVmNew();

        // msConvertInt(Bool(true)) = Int(1)
        let b = msBoolVal(1);
        let converted = msConvertInt(vm, b);
        assert!(!converted.is_null());
        assert_eq!(msToInt(vm, converted), 1);
        free_value(converted);

        // msConvertStr(Int(42)) = String("42")
        let i = msInt(42);
        let str_val = msConvertStr(vm, i);
        assert!(!str_val.is_null());
        assert_eq!(msIsString(str_val), MS_TRUE);
        let s = msToString(vm, str_val);
        assert_eq!(unsafe { CStr::from_ptr(s) }.to_str().unwrap(), "42");
        free_value(str_val);

        free_value(b);
        free_value(i);
        msVmFree(vm);
    }

    #[test]
    fn test_len_and_repr() {
        let vm = msVmNew();

        // msLen for List
        let list = msListFrom(vm, [msInt(1), msInt(2), msInt(3)].as_ptr(), 3);
        assert_eq!(msLen(vm, list), 3);

        // msLen for String
        let str_val = msString(vm, b"hello\0".as_ptr() as *const c_char);
        assert_eq!(msLen(vm, str_val), 5);

        // msRepr for Int
        let int_repr = msRepr(vm, msInt(42));
        let int_s = msToString(vm, int_repr);
        assert_eq!(unsafe { CStr::from_ptr(int_s) }.to_str().unwrap(), "42");

        // msRepr for String (带引号)
        let str_repr = msRepr(vm, str_val);
        let str_s = msToString(vm, str_repr);
        assert_eq!(unsafe { CStr::from_ptr(str_s) }.to_str().unwrap(), "\"hello\"");

        // msRepr for nil
        let nil_repr = msRepr(vm, msNil());
        let nil_s = msToString(vm, nil_repr);
        assert_eq!(unsafe { CStr::from_ptr(nil_s) }.to_str().unwrap(), "nil");

        free_value(list);
        free_value(str_val);
        free_value(int_repr);
        free_value(str_repr);
        free_value(nil_repr);
        msVmFree(vm);
    }

    // --- Deferred tests (require Task 69/73) ---
    // test_attr_access: requires msInstanceNew (Task 73)
    // test_item_access: requires msListPush/msGetItem/msSetItem (Task 69)
}
```
