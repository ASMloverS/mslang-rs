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

**msToString 内部缓冲区**：格式化字符串存储在 `MsValueInner` 内部字段中，随 MsValue 生命周期存在。多次调用同一 MsValue 会覆盖上次的缓冲区内容。

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

### 1. msToInt

```rust
#[no_mangle]
pub unsafe extern "C" fn msToInt(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    let inner = (*val).inner();
    match &inner.object {
        Object::Int(n) => *n,
        Object::Float(f) => *f as i64,
        _ => {
            set_type_error(vm, "int or float", &inner.object);
            0
        }
    }
}
```

### 2. msToFloat

```rust
#[no_mangle]
pub unsafe extern "C" fn msToFloat(vm: *mut MsVM, val: *mut MsValue) -> f64 {
    let inner = (*val).inner();
    match &inner.object {
        Object::Int(n) => *n as f64,
        Object::Float(f) => *f,
        _ => {
            set_type_error(vm, "int or float", &inner.object);
            0.0
        }
    }
}
```

### 3. msToBool

```rust
#[no_mangle]
pub unsafe extern "C" fn msToBool(val: *mut MsValue) -> c_int {
    let inner = (*val).inner();
    if inner.object.is_truthy() { MS_TRUE } else { MS_FALSE }
}
```

不设异常。`is_truthy()` 来自任务 20 `Object` 的真值实现。

### 4. msToString / msToStringCopy

```rust
#[no_mangle]
pub unsafe extern "C" fn msToString(vm: *mut MsVM, val: *mut MsValue) -> *const c_char {
    let inner = (*val).inner_mut();
    match &inner.object {
        Object::Ref(ptr) if (**ptr).type_tag == TypeTag::STRING => {
            let s = (*(ptr as *mut MsString)).as_str();
            s.as_ptr() as *const c_char
        }
        _ => {
            let formatted = format!("{}", inner.object);
            inner.cached_str = Some(formatted);
            inner.cached_str.as_ref().unwrap().as_ptr() as *const c_char
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn msToStringCopy(vm: *mut MsVM, val: *mut MsValue) -> *mut c_char {
    let ptr = msToString(vm, val);
    libc::strdup(ptr)
}
```

`MsValueInner` 需增加 `cached_str: Option<String>` 字段，用于缓存非字符串类型的格式化结果。String 类型直接返回内部数据指针，跳过格式化。

`msToStringCopy` 在 `msToString` 基础上 `strdup`，C 调用方必须 `free()`。

### 5. 显式类型转换

以 `msConvertInt` 为例：

```rust
#[no_mangle]
pub unsafe extern "C" fn msConvertInt(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    let inner = (*val).inner();
    let result = match &inner.object {
        Object::Bool(b) => Some(Object::Int(if *b { 1 } else { 0 })),
        Object::Int(_) => Some(inner.object.clone()),
        Object::Float(f) => Some(Object::Int(*f as i64)),
        Object::Ref(ptr) if (**ptr).type_tag == TypeTag::STRING => {
            let s = (*(ptr as *mut MsString)).as_str();
            s.parse::<i64>().ok().map(Object::Int)
        }
        _ => None,
    };
    match result {
        Some(obj) => alloc_value(vm, obj),
        None => {
            set_type_error(vm, "convertible type", &inner.object);
            null_mut()
        }
    }
}
```

其余 `msConvertFloat`、`msConvertStr`、`msConvertBool`、`msConvertList` 按相同模式实现，转换规则见设计规格表格。

`msConvertBool` 直接调用 `is_truthy()` 返回 `MS_TRUE_VAL`/`MS_FALSE_VAL` 单例，无需分配。

`msConvertList` 需处理各可迭代类型的转换：String → 字符列表，Tuple/Set → 新 List，Dict → key 列表，List → 浅拷贝。

### 6. 比较操作

```rust
#[no_mangle]
pub unsafe extern "C" fn msEq(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    let obj_a = &(*a).inner().object;
    let obj_b = &(*b).inner().object;
    if obj_a == obj_b { MS_TRUE } else { MS_FALSE }
}

#[no_mangle]
pub unsafe extern "C" fn msLt(vm: *mut MsVM, a: *mut MsValue, b: *mut MsValue) -> c_int {
    let obj_a = &(*a).inner().object;
    let obj_b = &(*b).inner().object;
    match obj_a.partial_cmp(obj_b) {
        Some(Ordering::Less) => MS_TRUE,
        Some(_) => MS_FALSE,
        None => {
            set_type_error(vm, "comparable types", obj_a);
            MS_FALSE
        }
    }
}
```

`msLe`/`msGt`/`msGe` 结构相同，仅替换 `Ordering` 变体。比较逻辑委托给 `Object` 的 `PartialOrd` 实现（任务 21）。

### 7. msIs（身份比较）

```rust
#[no_mangle]
pub unsafe extern "C" fn msIs(a: *mut MsValue, b: *mut MsValue) -> c_int {
    if a == b { return MS_TRUE; }
    let obj_a = &(*a).inner().object;
    let obj_b = &(*b).inner().object;
    match (obj_a, obj_b) {
        (Object::Ref(p1), Object::Ref(p2)) => {
            if p1 == p2 { MS_TRUE } else { MS_FALSE }
        }
        _ => {
            if obj_a == obj_b { MS_TRUE } else { MS_FALSE }
        }
    }
}
```

对 Ref 类型比较指针地址，值类型比较值。

### 8. msHash

```rust
#[no_mangle]
pub unsafe extern "C" fn msHash(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let inner = (*val).inner();
    let mut hasher = DefaultHasher::new();
    if inner.object.is_hashable() {
        inner.object.hash(&mut hasher);
        hasher.finish() as i64
    } else {
        set_type_error(vm, "hashable type", &inner.object);
        0
    }
}
```

`is_hashable()` 和 `Hash` 实现来自任务 20–22 Object 系统。

### 9. msGetAttr / msSetAttr

```rust
#[no_mangle]
pub unsafe extern "C" fn msGetAttr(
    vm: *mut MsVM, obj: *mut MsValue, attr: *const c_char,
) -> *mut MsValue {
    let attr_str = CStr::from_ptr(attr).to_str().unwrap();
    let inner = (*obj).inner();
    match &inner.object {
        Object::Ref(ptr) => {
            let type_tag = (**ptr).type_tag;
            match type_tag {
                TypeTag::INSTANCE => get_instance_attr(vm, ptr, attr_str),
                TypeTag::MODULE => get_module_export(vm, ptr, attr_str),
                TypeTag::CLASS => get_class_member(vm, ptr, attr_str),
                _ => {
                    set_type_error(vm, "object with attributes", &inner.object);
                    null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "object with attributes", &inner.object);
            null_mut()
        }
    }
}
```

`msSetAttr` 结构相同，返回 `MsStatus`。仅 Instance 和支持属性设置的类型允许操作。

### 10. msGetItem / msSetItem

```rust
#[no_mangle]
pub unsafe extern "C" fn msGetItem(
    vm: *mut MsVM, obj: *mut MsValue, key: *mut MsValue,
) -> *mut MsValue {
    let inner = (*obj).inner();
    let key_obj = &(*key).inner().object;
    match &inner.object {
        Object::Ref(ptr) => {
            let type_tag = (**ptr).type_tag;
            match type_tag {
                TypeTag::LIST => list_get(vm, ptr, key_obj),
                TypeTag::DICT => dict_get(vm, ptr, key_obj),
                TypeTag::STRING => string_get(vm, ptr, key_obj),
                TypeTag::TUPLE => tuple_get(vm, ptr, key_obj),
                _ => {
                    set_type_error(vm, "subscriptable type", &inner.object);
                    null_mut()
                }
            }
        }
        _ => {
            set_type_error(vm, "subscriptable type", &inner.object);
            null_mut()
        }
    }
}
```

`msSetItem` 仅支持 List 和 Dict，其余类型设置 TypeError。

### 11. msLen

```rust
#[no_mangle]
pub unsafe extern "C" fn msLen(vm: *mut MsVM, val: *mut MsValue) -> i64 {
    let inner = (*val).inner();
    match &inner.object {
        Object::Ref(ptr) => {
            let type_tag = (**ptr).type_tag;
            match type_tag {
                TypeTag::STRING => (*(ptr as *mut MsString)).len() as i64,
                TypeTag::LIST => (*(ptr as *mut MsList)).len() as i64,
                TypeTag::DICT => (*(ptr as *mut MsDict)).len() as i64,
                TypeTag::TUPLE => (*(ptr as *mut MsTuple)).len() as i64,
                TypeTag::SET => (*(ptr as *mut MsSet)).len() as i64,
                _ => {
                    set_type_error(vm, "type with length", &inner.object);
                    -1
                }
            }
        }
        _ => {
            set_type_error(vm, "type with length", &inner.object);
            -1
        }
    }
}
```

### 12. msRepr

```rust
#[no_mangle]
pub unsafe extern "C" fn msRepr(vm: *mut MsVM, val: *mut MsValue) -> *mut MsValue {
    let inner = (*val).inner();
    let repr_str = match &inner.object {
        Object::Nil => "nil".to_string(),
        Object::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Object::Int(n) => format!("{}", n),
        Object::Float(f) => format!("{}", f),
        Object::Ref(ptr) => {
            let type_tag = (**ptr).type_tag;
            match type_tag {
                TypeTag::STRING => {
                    let s = (*(ptr as *mut MsString)).as_str();
                    format!("{:?}", s)  // 带引号和转义
                }
                TypeTag::LIST => { /* 递归 repr 各元素 */ }
                TypeTag::DICT => { /* 递归 repr 各 k:v */ }
                TypeTag::INSTANCE => { /* 显示类名和字段 */ }
                _ => format!("<{}>", type_name(&inner.object)),
            }
        }
    };
    alloc_value(vm, Object::from_string(repr_str))
}
```

String 的 repr 包含引号和转义字符。List/Dict 递归调用 msRepr 生成子元素的表示。

### 辅助函数

所有函数共用 `set_type_error` 辅助函数，定义在 `src/capi/mod.rs`：

```rust
unsafe fn set_type_error(vm: *mut MsVM, expected: &str, actual: &Object) {
    let msg = format!("expected {}, got {}", expected, actual.type_name());
    msThrowTypeError(vm, expected, actual.type_name());
}
```

### MsValueInner 修改

在任务 67 的 `MsValueInner` 结构体中增加字段：

```rust
struct MsValueInner {
    object: Object,
    cached_str: Option<String>,  // msToString 缓冲区
    // ... 其余任务 67 已有字段 ...
}
```

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
14. `msIs(a, a)` 同一指针返回 `MS_TRUE`
15. `msIs(Int(1), Int(1))` 值类型比较值，返回 `MS_TRUE`
16. `msHash(vm, Int(42))` 返回非零哈希值
17. `msGetAttr(vm, instance, "field")` 返回属性值
18. `msSetAttr` + `msGetAttr` 往返一致
19. `msGetItem(vm, list, Int(0))` 返回首元素
20. `msSetItem` + `msGetItem` 往返一致
21. `msLen(vm, list_of_3)` 返回 `3`
22. `msLen(vm, string("hello"))` 返回 `5`
23. `msRepr(vm, String("hello"))` 返回 `'"hello"'`（带引号）
24. `msRepr(vm, Int(42))` 返回 `'42'`
25. `msRepr(vm, nil)` 返回 `'nil'`

## 测试用例

Rust 单元测试位于 `src/capi/value.rs`：

### test_to_int_float_bool

```rust
#[test]
fn test_to_int_float_bool() {
    let vm = test_vm();

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
    assert!(msErrOccurred(vm));
    msErrClear(vm);
}
```

### test_to_string_and_copy

```rust
#[test]
fn test_to_string_and_copy() {
    let vm = test_vm();

    let int_val = msInt(42);
    let s = msToString(vm, int_val);
    let cstr = CStr::from_ptr(s);
    assert_eq!(cstr.to_str().unwrap(), "42");

    let copy = msToStringCopy(vm, int_val);
    let cstr_copy = CStr::from_ptr(copy);
    assert_eq!(cstr_copy.to_str().unwrap(), "42");
    libc::free(copy as *mut c_void);
}
```

### test_equality_comparisons

```rust
#[test]
fn test_equality_comparisons() {
    let vm = test_vm();

    let a = msInt(1);
    let b = msInt(1);
    let c = msInt(2);

    assert_eq!(msEq(vm, a, b), MS_TRUE);
    assert_eq!(msEq(vm, a, c), MS_FALSE);
    assert_eq!(msIs(a, a), MS_TRUE);
    assert_eq!(msIs(a, b), MS_TRUE);  // 值类型比较值

    let nil_a = msNil();
    let nil_b = msNil();
    assert_eq!(msEq(vm, nil_a, nil_b), MS_TRUE);
}
```

### test_ordering_comparisons

```rust
#[test]
fn test_ordering_comparisons() {
    let vm = test_vm();

    let a = msInt(1);
    let b = msInt(2);

    assert_eq!(msLt(vm, a, b), MS_TRUE);
    assert_eq!(msLe(vm, a, b), MS_TRUE);
    assert_eq!(msGt(vm, b, a), MS_TRUE);
    assert_eq!(msGe(vm, b, a), MS_TRUE);
    assert_eq!(msLt(vm, a, a), MS_FALSE);
    assert_eq!(msLe(vm, a, a), MS_TRUE);
}
```

### test_identity_comparison

```rust
#[test]
fn test_identity_comparison() {
    let vm = test_vm();

    // 值类型：is 比较值
    let a = msInt(42);
    let b = msInt(42);
    assert_eq!(msIs(a, b), MS_TRUE);

    // 引用类型：is 比较指针
    let list_a = msListNew(vm);
    let list_b = msListNew(vm);
    assert_eq!(msIs(list_a, list_a), MS_TRUE);
    assert_eq!(msIs(list_a, list_b), MS_FALSE);
}
```

### test_attr_access

```rust
#[test]
fn test_attr_access() {
    let vm = test_vm();

    // 创建 class 和 instance
    msExecString(vm, "class Point:\n  fn __init__(self, x, y):\n    self.x = x\n    self.y = y\n", "test.ms");
    let point_class = msGetGlobal(vm, "Point");
    let args = [msInt(10), msInt(20)];
    let inst = msInstanceNew(vm, point_class, args.as_ptr(), 2);

    // msGetAttr
    let x_attr = msGetAttr(vm, inst, "x\0".as_ptr() as *const c_char);
    assert_eq!(msToInt(vm, x_attr), 10);

    // msSetAttr
    msSetAttr(vm, inst, "x\0".as_ptr() as *const c_char, msInt(99));
    let x_after = msGetAttr(vm, inst, "x\0".as_ptr() as *const c_char);
    assert_eq!(msToInt(vm, x_after), 99);
}
```

### test_item_access

```rust
#[test]
fn test_item_access() {
    let vm = test_vm();

    // List
    let list = msListNew(vm);
    msListPush(vm, list, msInt(10));
    msListPush(vm, list, msInt(20));
    msListPush(vm, list, msInt(30));

    let item = msGetItem(vm, list, msInt(1));
    assert_eq!(msToInt(vm, item), 20);

    msSetItem(vm, list, msInt(1), msInt(99));
    let item_after = msGetItem(vm, list, msInt(1));
    assert_eq!(msToInt(vm, item_after), 99);

    // Dict
    let dict = msDictNew(vm);
    let key = msString(vm, "key\0".as_ptr() as *const c_char);
    msSetItem(vm, dict, key, msInt(42));
    let val = msGetItem(vm, dict, key);
    assert_eq!(msToInt(vm, val), 42);
}
```

### test_len_and_repr

```rust
#[test]
fn test_len_and_repr() {
    let vm = test_vm();

    // msLen
    let list = msListNew(vm);
    msListPush(vm, list, msInt(1));
    msListPush(vm, list, msInt(2));
    msListPush(vm, list, msInt(3));
    assert_eq!(msLen(vm, list), 3);

    let str_val = msString(vm, "hello\0".as_ptr() as *const c_char);
    assert_eq!(msLen(vm, str_val), 5);

    // msRepr
    let int_repr = msRepr(vm, msInt(42));
    let int_s = msToString(vm, int_repr);
    assert_eq!(CStr::from_ptr(int_s).to_str().unwrap(), "42");

    let str_repr = msRepr(vm, str_val);
    let str_s = msToString(vm, str_repr);
    assert_eq!(CStr::from_ptr(str_s).to_str().unwrap(), "\"hello\"");

    let nil_repr = msRepr(vm, msNil());
    let nil_s = msToString(vm, nil_repr);
    assert_eq!(CStr::from_ptr(nil_s).to_str().unwrap(), "nil");
}
```
