# Object 集合类型

## 所属阶段
Phase 2.3c - 字节码编译 + VM 核心

## 前置任务
- 20-object-system-basic

## 目标

在 Object 系统中实现四种集合类型：List、Dict、Tuple、Set。包括类型定义、基本操作、Hash 实现和 Display 格式化。

## 设计规格

引用 [02-types.md](../02-types.md) 集合类型规范。

### List

- 有序可变序列，存储任意类型元素
- 操作：下标访问、切片、push/pop、length、contains、拼接（+）、重复（*）

### Dict

- 有序可变映射（保持插入顺序，与 Python 3.7+ 一致）
- 键必须为可哈希类型：int, float, bool, string, nil, tuple
- 操作：访问（不存在返回 nil）、设置、删除、contains、length

### Tuple

- 有序不可变序列
- 可哈希（当所有元素可哈希时）
- 操作：下标访问、length、contains

### Set

- 无序可变集合，元素唯一
- 元素必须为可哈希类型
- 操作：add、remove、contains、并集（|）、交集（&）、差集（-）、对称差（^）

### 可哈希类型

引用 [02-types.md](../02-types.md)：
- int, float, bool, string, nil, tuple（所有元素可哈希）
- 不可哈希：list, dict, set, class 实例

## 实现细节

### 文件位置

`src/vm/object.rs`（扩展 task 20 的 Object 系统）

### 对象模型说明

本任务引用 [20-object-system-basic](./20-object-system-basic.md) 定义的 `MsObjHeader`、`TypeTag`、`Object` 枚举和辅助函数。集合类型均通过 `Object::Ref(*mut MsObjHeader)` 存储，类型由 `MsObjHeader.type_tag` 区分（`TypeTag::LIST = 2`、`TypeTag::DICT = 3`、`TypeTag::TUPLE = 4`、`TypeTag::SET = 5`）。

**不新增 Object 枚举变体**：List、Dict、Tuple、Set 均使用 `Object::Ref` + type_tag 区分，与 String 保持一致的模型。

### DictMap 类型

```rust
use std::collections::HashMap;

pub struct DictMap {
    entries: HashMap<Object, Object>,
    order: Vec<Object>,
}

impl DictMap {
    pub fn new() -> Self {
        DictMap {
            entries: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn insert(&mut self, key: Object, value: Object) {
        if !self.entries.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.entries.insert(key, value);
    }

    pub fn get(&self, key: &Object) -> Option<&Object> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &Object) -> Option<Object> {
        let old = self.entries.remove(key);
        if old.is_some() {
            self.order.retain(|k| k != key);
        }
        old
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn keys(&self) -> Vec<&Object> {
        self.order.iter().collect()
    }

    pub fn items(&self) -> Vec<(&Object, &Object)> {
        self.order.iter()
            .filter_map(|k| self.entries.get(k).map(|v| (k, v)))
            .collect()
    }
}
```

### 集合堆对象布局

```rust
/// 堆上 List 对象。data_ptr 指向 Box<Vec<Object>>。
#[repr(C)]
pub struct MsList {
    pub header:   MsObjHeader,
    pub data_ptr: *mut Vec<Object>,
}

/// 堆上 Dict 对象。data_ptr 指向 Box<DictMap>。
#[repr(C)]
pub struct MsDict {
    pub header:   MsObjHeader,
    pub data_ptr: *mut DictMap,
}

/// 堆上 Tuple 对象。data_ptr 指向 Box<Vec<Object>>（不可变语义由上层保证）。
#[repr(C)]
pub struct MsTuple {
    pub header:   MsObjHeader,
    pub data_ptr: *mut Vec<Object>,
    pub len:      u32,
}

/// 堆上 Set 对象。data_ptr 指向 Box<HashSet<Object>>。
#[repr(C)]
pub struct MsSet {
    pub header:   MsObjHeader,
    pub data_ptr: *mut HashSet<Object>,
}
```

### 堆分配辅助函数

```rust
/// 分配 List 对象，返回 Object::Ref。
pub fn alloc_list(items: Vec<Object>) -> Object {
    let data_ptr = Box::into_raw(Box::new(items));
    let obj = Box::new(MsList {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::LIST as u8,
            size:      std::mem::size_of::<MsList>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        data_ptr,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 List 对象的内部 Vec。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_list` 分配的、在 `'a` 期间保持有效的 `MsList`。
/// 同一对象不得嵌套调用 `read_list`（会产生重叠 `&mut`，见"借用约束"节）。
pub unsafe fn read_list<'a>(ptr: *mut MsObjHeader) -> &'a mut Vec<Object> {
    let ms_list = ptr as *mut MsList;
    &mut *(*ms_list).data_ptr
}

/// 分配 Dict 对象，返回 Object::Ref。
pub fn alloc_dict(map: DictMap) -> Object {
    let data_ptr = Box::into_raw(Box::new(map));
    let obj = Box::new(MsDict {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::DICT as u8,
            size:      std::mem::size_of::<MsDict>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        data_ptr,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 Dict 对象的内部 DictMap。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_dict` 分配的、在 `'a` 期间保持有效的 `MsDict`。
/// 不得嵌套调用（借用约束）。
pub unsafe fn read_dict<'a>(ptr: *mut MsObjHeader) -> &'a mut DictMap {
    let ms_dict = ptr as *mut MsDict;
    &mut *(*ms_dict).data_ptr
}

/// 分配 Tuple 对象，返回 Object::Ref。
pub fn alloc_tuple(items: Vec<Object>) -> Object {
    let len = items.len() as u32;
    let data_ptr = Box::into_raw(Box::new(items));
    let obj = Box::new(MsTuple {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::TUPLE as u8,
            size:      std::mem::size_of::<MsTuple>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        data_ptr,
        len,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 Tuple 对象的内部 Vec（只读，Tuple 不可变）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_tuple` 分配的、在 `'a` 期间保持有效的 `MsTuple`。
pub unsafe fn read_tuple<'a>(ptr: *mut MsObjHeader) -> &'a Vec<Object> {
    let ms_tuple = ptr as *mut MsTuple;
    &*(*ms_tuple).data_ptr
}

/// 分配 Set 对象，返回 Object::Ref。
pub fn alloc_set(inner: HashSet<Object>) -> Object {
    let data_ptr = Box::into_raw(Box::new(inner));
    let obj = Box::new(MsSet {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::SET as u8,
            size:      std::mem::size_of::<MsSet>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        data_ptr,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 Set 对象的内部 HashSet。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_set` 分配的、在 `'a` 期间保持有效的 `MsSet`。
/// 不得嵌套调用（借用约束）。
pub unsafe fn read_set<'a>(ptr: *mut MsObjHeader) -> &'a mut HashSet<Object> {
    let ms_set = ptr as *mut MsSet;
    &mut *(*ms_set).data_ptr
}
```

> **Hash + Eq 约束**：`HashSet<Object>` 和 `HashMap<Object, Object>` 要求 `Object` 实现 `Hash` 和 `Eq`。
> Task 20 已为基础类型实现 `Hash`，本任务需扩展至 `Tuple`（当所有元素可哈希时可哈希），
> 并确保 `List`/`Dict`/`Set` 在 `hash()` 时 panic（运行时 TypeError）。
> `PartialEq` 需扩展至集合类型的逐元素比较。
>
> **必须新增 `impl Eq for Object`**：task 20 仅实现了 `PartialEq`（无 `Eq`），而 `HashMap`/`HashSet` 要求 `Eq`。`Eq` 是 marker trait（断言 `==` 为等价关系），直接复用现有 `PartialEq`：
>
> ```rust
> /// Object 满足 Eq 的不变性：Float(NaN) 永不可哈希（Hash 在 NaN 上 panic），
> /// 故 NaN 不会进入 HashMap/HashSet；其余类型的 PartialEq 均为等价关系。
> /// 因此 `==` 的 NaN 非自反性不影响集合正确性，可安全 impl Eq。
> impl Eq for Object {}
> ```

### Display 扩展

在 task 20 的 `Display` 实现中，`Object::Ref` 的 match 臂按 type_tag 分发：

```rust
Object::Ref(ptr) => {
    debug_assert!(!ptr.is_null(), "null Object::Ref");
    let tag = unsafe { (**ptr).type_tag };
    if tag == TypeTag::STRING as u8 {
        write!(f, "{}", unsafe { read_str(*ptr) })
    } else if tag == TypeTag::LIST as u8 {
        let items = unsafe { read_list(*ptr) };
        let strs: Vec<String> = items.iter().map(|o| format!("{}", o)).collect();
        write!(f, "[{}]", strs.join(", "))
    } else if tag == TypeTag::DICT as u8 {
        let map = unsafe { read_dict(*ptr) };
        let strs: Vec<String> = map.items().iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        write!(f, "{{{}}}", strs.join(", "))
    } else if tag == TypeTag::TUPLE as u8 {
        let items = unsafe { read_tuple(*ptr) };
        let strs: Vec<String> = items.iter().map(|o| format!("{}", o)).collect();
        if strs.len() == 1 {
            write!(f, "({},)", strs[0])
        } else {
            write!(f, "({})", strs.join(", "))
        }
    } else if tag == TypeTag::SET as u8 {
        let inner = unsafe { read_set(*ptr) };
        // HashSet 迭代序不确定，Display 排序以保证输出稳定（便于调试与测试）
        let mut strs: Vec<String> = inner.iter().map(|o| format!("{}", o)).collect();
        strs.sort();
        write!(f, "{{{}}}", strs.join(", "))
    } else {
        write!(f, "<object:{}>", tag)
    }
}
```

### 真值规则扩展

```rust
Object::Ref(ptr) => {
    debug_assert!(!ptr.is_null(), "null Object::Ref");
    let tag = unsafe { (**ptr).type_tag };
    if tag == TypeTag::STRING as u8 {
        unsafe { !read_str(*ptr).is_empty() }
    } else if tag == TypeTag::LIST as u8 {
        unsafe { !read_list(*ptr).is_empty() }
    } else if tag == TypeTag::DICT as u8 {
        unsafe { read_dict(*ptr).len() > 0 }
    } else if tag == TypeTag::TUPLE as u8 {
        unsafe { !read_tuple(*ptr).is_empty() }
    } else if tag == TypeTag::SET as u8 {
        unsafe { !read_set(*ptr).is_empty() }
    } else {
        true
    }
}
```

### Hash 扩展

```rust
Object::Ref(ptr) => {
    debug_assert!(!ptr.is_null(), "null Object::Ref");
    let tag = unsafe { (**ptr).type_tag };
    if tag == TypeTag::STRING as u8 {
        unsafe { read_str(*ptr) }.hash(state)
    } else if tag == TypeTag::TUPLE as u8 {
        // 递归哈希元素；若 tuple 含 List/Dict/Set 元素，其 Hash 会 panic（TypeError 传播）
        unsafe { read_tuple(*ptr) }.hash(state)
    } else if tag == TypeTag::LIST as u8
        || tag == TypeTag::DICT as u8
        || tag == TypeTag::SET as u8
    {
        // 运行时通过 type_name 报 TypeError
        let type_str = if tag == TypeTag::LIST as u8 { "list" }
            else if tag == TypeTag::DICT as u8 { "dict" }
            else { "set" };
        panic!("TypeError: unhashable type: '{}'", type_str);
    } else {
        (*ptr as usize).hash(state)
    }
}
```

### 相等性扩展

```rust
(Object::Ref(a), Object::Ref(b)) => {
    debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
    let tag_a = unsafe { (**a).type_tag };
    let tag_b = unsafe { (**b).type_tag };
    if tag_a != tag_b {
        return false;
    }
    if tag_a == TypeTag::STRING as u8 {
        unsafe { read_str(*a) == read_str(*b) }
    } else if tag_a == TypeTag::LIST as u8 {
        unsafe { read_list(*a) == read_list(*b) }
    } else if tag_a == TypeTag::TUPLE as u8 {
        unsafe { read_tuple(*a) == read_tuple(*b) }
    } else if tag_a == TypeTag::DICT as u8 {
        // Dict 相等性仅比较 entries（与 Python 一致）；
        // 插入顺序仅影响 Display/迭代，不影响 ==。
        let ma = unsafe { read_dict(*a) };
        let mb = unsafe { read_dict(*b) };
        ma.entries == mb.entries
    } else if tag_a == TypeTag::SET as u8 {
        unsafe { read_set(*a) == read_set(*b) }
    } else {
        false
    }
}
```

### List 操作

```rust
impl Object {
    pub fn list_push(&self, value: Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                unsafe { read_list(*ptr) }.push(value);
                Ok(Object::Nil)
            }
            _ => Err("TypeError: push requires a list".to_string()),
        }
    }

    pub fn list_pop(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                unsafe { read_list(*ptr) }.pop()
                    .ok_or_else(|| "IndexError: pop from empty list".to_string())
            }
            _ => Err("TypeError: pop requires a list".to_string()),
        }
    }

    /// 负索引支持：-1 为末尾。越界抛 IndexError。
    pub fn list_get_index(&self, index: i64) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let items = unsafe { read_list(*ptr) };
                let len = items.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(format!("IndexError: list index {} out of range", index));
                }
                Ok(items[idx as usize].clone())
            }
            _ => Err("TypeError: index access requires a list".to_string()),
        }
    }

    /// `lst[i] = v`。负索引支持；越界抛 IndexError。
    pub fn list_set_index(&self, index: i64, value: Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let items = unsafe { read_list(*ptr) };
                let len = items.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(format!("IndexError: list assignment index {} out of range", index));
                }
                items[idx as usize] = value.clone();
                Ok(value)
            }
            _ => Err("TypeError: index assignment requires a list".to_string()),
        }
    }

    pub fn list_length(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                Ok(Object::Int(unsafe { read_list(*ptr) }.len() as i64))
            }
            _ => Err("TypeError: len requires a list".to_string()),
        }
    }

    /// `val in lst`。
    pub fn list_contains(&self, value: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                Ok(Object::Bool(unsafe { read_list(*ptr) }.iter().any(|x| x == value)))
            }
            _ => Err("TypeError: 'in' requires a list".to_string()),
        }
    }

    pub fn list_insert(&self, index: i64, value: Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                let items = unsafe { read_list(*ptr) };
                let len = items.len() as i64;
                let idx = (if index < 0 { len + index } else { index }).clamp(0, len) as usize;
                items.insert(idx, value.clone());
                Ok(value)
            }
            _ => Err("TypeError: insert requires a list".to_string()),
        }
    }

    /// 删除首个等于 value 的元素；不存在抛 ValueError。
    pub fn list_remove(&self, value: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                let items = unsafe { read_list(*ptr) };
                if let Some(pos) = items.iter().position(|x| x == value) {
                    items.remove(pos);
                    Ok(Object::Nil)
                } else {
                    Err(format!("ValueError: list.remove(x): x not in list"))
                }
            }
            _ => Err("TypeError: remove requires a list".to_string()),
        }
    }

    /// `lst1 + lst2` → 新 List（拼接）。task 21 已声明由本 task 实现。
    pub fn list_concat(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (**a).type_tag } == TypeTag::LIST as u8
                && unsafe { (**b).type_tag } == TypeTag::LIST as u8 =>
            {
                let mut merged = unsafe { read_list(*a) }.clone();
                merged.extend(unsafe { read_list(*b) }.iter().cloned());
                Ok(alloc_list(merged))
            }
            _ => Err("TypeError: + requires two lists".to_string()),
        }
    }

    /// `lst * n` → 新 List（重复）。n < 0 报 TypeError。
    pub fn list_repeat(&self, n: &Object) -> Result<Object, String> {
        match (self, n) {
            (Object::Ref(a), Object::Int(b))
                if unsafe { (**a).type_tag } == TypeTag::LIST as u8 =>
            {
                if *b < 0 {
                    return Err("TypeError: can't multiply list by negative int".into());
                }
                let src = unsafe { read_list(*a) };
                let result: Vec<Object> = std::iter::repeat_with(|| src.iter().cloned())
                    .take(*b as usize).flatten().collect();
                Ok(alloc_list(result))
            }
            _ => Err("TypeError: * requires a list and an int".to_string()),
        }
    }
}
```

### Dict 操作

```rust
impl Object {
    /// `d[key]`：不存在返回 `Object::Nil`（02-types.md:181，不抛异常）。
    pub fn dict_get(&self, key: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                Ok(unsafe { read_dict(*ptr) }.get(key).cloned().unwrap_or(Object::Nil))
            }
            _ => Err("TypeError: dict access requires a dict".to_string()),
        }
    }

    /// `d[key] = val`。key 必须可哈希（List/Dict/Set/NAN 会 panic）。
    pub fn dict_set(&self, key: Object, value: Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                unsafe { read_dict(*ptr) }.insert(key, value.clone());
                Ok(value)
            }
            _ => Err("TypeError: dict assignment requires a dict".to_string()),
        }
    }

    /// `d.remove(key)`：键不存在抛 KeyError（02-types.md:187）。
    pub fn dict_remove(&self, key: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                unsafe { read_dict(*ptr) }.remove(key)
                    .ok_or_else(|| format!("KeyError: {}", key))
            }
            _ => Err("TypeError: remove requires a dict".to_string()),
        }
    }

    /// `key in d`。
    pub fn dict_contains(&self, key: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                Ok(Object::Bool(unsafe { read_dict(*ptr) }.get(key).is_some()))
            }
            _ => Err("TypeError: 'in' requires a dict".to_string()),
        }
    }

    pub fn dict_length(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                Ok(Object::Int(unsafe { read_dict(*ptr) }.len() as i64))
            }
            _ => Err("TypeError: len requires a dict".to_string()),
        }
    }

    /// `d.keys()` → 新 List（按插入序）。
    pub fn dict_keys(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                let keys: Vec<Object> = unsafe { read_dict(*ptr) }.keys().into_iter().cloned().collect();
                Ok(alloc_list(keys))
            }
            _ => Err("TypeError: keys() requires a dict".to_string()),
        }
    }

    /// `d.items()` → 新 List of Tuple(key, value)（按插入序）。
    pub fn dict_items(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                let pairs: Vec<Object> = unsafe { read_dict(*ptr) }.items().into_iter()
                    .map(|(k, v)| Object::make_tuple(vec![k.clone(), v.clone()])).collect();
                Ok(alloc_list(pairs))
            }
            _ => Err("TypeError: items() requires a dict".to_string()),
        }
    }
}
```

### Tuple 操作

```rust
impl Object {
    pub fn make_tuple(elements: Vec<Object>) -> Object {
        alloc_tuple(elements)
    }

    /// 负索引支持；越界抛 IndexError。Tuple 不可变，仅读。
    pub fn tuple_get_index(&self, index: i64) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let items = unsafe { read_tuple(*ptr) };
                let len = items.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(format!("IndexError: tuple index {} out of range", index));
                }
                Ok(items[idx as usize].clone())
            }
            _ => Err("TypeError: index access requires a tuple".to_string()),
        }
    }

    pub fn tuple_length(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
                Ok(Object::Int(unsafe { read_tuple(*ptr) }.len() as i64))
            }
            _ => Err("TypeError: len requires a tuple".to_string()),
        }
    }

    pub fn tuple_contains(&self, value: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
                Ok(Object::Bool(unsafe { read_tuple(*ptr) }.iter().any(|x| x == value)))
            }
            _ => Err("TypeError: 'in' requires a tuple".to_string()),
        }
    }
}
```

`items()` 等方法使用 `Object::make_tuple(vec![key, value])` 构造二元组。

### Set 操作

```rust
impl Object {
    /// `s.add(val)`。val 必须可哈希。
    pub fn set_add(&self, value: Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                unsafe { read_set(*ptr) }.insert(value.clone());
                Ok(value)
            }
            _ => Err("TypeError: add requires a set".to_string()),
        }
    }

    /// `s.remove(val)`：元素不存在抛 KeyError（02-types.md:246）。
    pub fn set_remove(&self, value: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                if unsafe { read_set(*ptr) }.remove(value) {
                    Ok(value.clone())
                } else {
                    Err(format!("KeyError: {}", value))
                }
            }
            _ => Err("TypeError: remove requires a set".to_string()),
        }
    }

    /// `val in s`。
    pub fn set_contains(&self, value: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
                Ok(Object::Bool(unsafe { read_set(*ptr) }.contains(value)))
            }
            _ => Err("TypeError: 'in' requires a set".to_string()),
        }
    }

    pub fn set_length(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
                Ok(Object::Int(unsafe { read_set(*ptr) }.len() as i64))
            }
            _ => Err("TypeError: len requires a set".to_string()),
        }
    }

    pub fn set_union(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (**a).type_tag } == TypeTag::SET as u8
                && unsafe { (**b).type_tag } == TypeTag::SET as u8 =>
            {
                let mut result = unsafe { read_set(*a) }.clone();
                result.extend(unsafe { read_set(*b) }.iter().cloned());
                Ok(alloc_set(result))
            }
            _ => Err("TypeError: | requires sets".to_string()),
        }
    }

    pub fn set_intersection(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (**a).type_tag } == TypeTag::SET as u8
                && unsafe { (**b).type_tag } == TypeTag::SET as u8 =>
            {
                let set_b = unsafe { read_set(*b) };
                let result: HashSet<Object> = unsafe { read_set(*a) }.iter()
                    .filter(|x| set_b.contains(x))
                    .cloned()
                    .collect();
                Ok(alloc_set(result))
            }
            _ => Err("TypeError: & requires sets".to_string()),
        }
    }

    /// `s1 - s2`（差集）。
    pub fn set_difference(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (**a).type_tag } == TypeTag::SET as u8
                && unsafe { (**b).type_tag } == TypeTag::SET as u8 =>
            {
                let set_b = unsafe { read_set(*b) };
                let result: HashSet<Object> = unsafe { read_set(*a) }.iter()
                    .filter(|x| !set_b.contains(x))
                    .cloned()
                    .collect();
                Ok(alloc_set(result))
            }
            _ => Err("TypeError: - requires sets".to_string()),
        }
    }

    /// `s1 ^ s2`（对称差）。
    pub fn set_symmetric_difference(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (**a).type_tag } == TypeTag::SET as u8
                && unsafe { (**b).type_tag } == TypeTag::SET as u8 =>
            {
                let mut result = unsafe { read_set(*a) }.clone();
                for x in unsafe { read_set(*b) }.iter() {
                    if !result.remove(x) { result.insert(x.clone()); }
                }
                Ok(alloc_set(result))
            }
            _ => Err("TypeError: ^ requires sets".to_string()),
        }
    }
}
```

### 借用约束

`read_list`/`read_dict`/`read_set` 返回 `&mut`，从裸指针产出，不经 `RefCell`/`UnsafeCell`。因此**同一 `Object::Ref` 的 `read_*` 调用不得嵌套**（如 `list.list_push(list.list_get_index(0))`），否则产生重叠 `&mut`，违反 Rust 别名规则 = UB。实现各操作方法时应在单次 `read_*` 调用内完成所有可变访问，不跨方法持引用。task 52（GC）重构时可改用 `RefCell` 提供运行时借用检查。

### GC 集成（task 52）

集合存储 `Object` 元素，其中可能含 `Object::Ref`（如 List of List、Dict 值为 List、循环引用 `a = []; a.push(a)`）。task 52 的 GC 需为四种集合提供 `TypeDescriptor::trace` 函数（`14-gc.md:116-130`）：

- **List/Tuple**：遍历 `Vec<Object>`，对每个 `Object::Ref` 元素递归调用 trace 回调。
- **Dict**：遍历 `entries` 的键与值（键也可能为 Ref，如 Tuple 键），递归 trace。
- **Set**：遍历 `HashSet<Object>` 元素，递归 trace。

此外，`data_ptr` 指向的 `Box<Vec<Object>>`/`Box<DictMap>`/`Box<HashSet<Object>>` 是**二级堆分配**（独立于 MsObjHeader 主体），与 task 20 的 `MsStr.data_ptr` 同类。GC 回收集合对象时须同时释放二级分配（注册 finalizer，或 task 52 升级为 header 后紧跟数据的内联布局）。本 task 不实现 trace/finalizer，但须保证 `data_ptr` 始终指向有效分配，便于 task 52 接管。

> **DictMap 双份键存储提醒**：`entries: HashMap<Object,Object>` 与 `order: Vec<Object>` 各存一份键。插入/删除须同步两处（`remove` 已显脆弱）。对大 dict 内存翻倍。若内存敏感，task 52 重构时可改用 `IndexMap<Object,Object>`（map + order 一体，单份存储）。本 task 维持双结构以保证 MVP 简单。

## 验证标准

1. List 可创建、push/pop、下标访问（含负索引）、`+` 拼接、`*` 重复、length、contains、`lst[i]=v`
2. Dict 保持插入顺序，键必须可哈希；访问不存在键返回 nil；`remove` 不存在键抛 KeyError；相等性忽略插入顺序
3. Tuple 不可变，可哈希（当元素可哈希时）；下标/length/contains
4. Set 元素唯一，支持 add/remove（不存在抛 KeyError）/contains/并集/交集/差集/对称差
5. 空集合为 falsy，非空集合为 truthy
6. List/Dict/Set 不可哈希（作为 dict 键或 set 元素时 panic，TypeError）；NaN 不可哈希
7. Display 格式化正确；Set Display 输出稳定（排序）
8. `Object: Eq` 已实现（`HashMap`/`HashSet` 可用）；`-0.0` 与 `0.0` 视为同一键

## 测试用例

```ms
# test_collections.ms
nums = [1, 2, 3, 4, 5]
person = {"name": "Alice", "age": 30}
point = (1, 2)
unique = {1, 2, 3}
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_basic() {
        let list = alloc_list(vec![
            Object::Int(1), Object::Int(2), Object::Int(3),
        ]);
        assert_eq!(list.list_get_index(0).unwrap(), Object::Int(1));
        assert_eq!(list.list_get_index(-1).unwrap(), Object::Int(3));
    }

    #[test]
    fn test_list_push_pop() {
        let list = alloc_list(vec![]);
        list.list_push(Object::Int(1)).unwrap();
        list.list_push(Object::Int(2)).unwrap();
        assert_eq!(list.list_pop().unwrap(), Object::Int(2));
    }

    #[test]
    fn test_dict_insertion_order() {
        let mut map = DictMap::new();
        map.insert(alloc_string("b"), Object::Int(2));
        map.insert(alloc_string("a"), Object::Int(1));
        let dict = alloc_dict(map);
        let Object::Ref(ptr) = dict else { panic!() };
        let keys = unsafe { read_dict(ptr) }.keys();
        assert_eq!(*keys[0], alloc_string("b"));
        assert_eq!(*keys[1], alloc_string("a"));
    }

    #[test]
    fn test_tuple_hashable() {
        let tuple = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        let mut set = HashSet::new();
        set.insert(tuple.clone());
        assert!(set.contains(&tuple));
    }

    #[test]
    fn test_set_uniqueness() {
        let mut inner = HashSet::new();
        inner.insert(Object::Int(1));
        inner.insert(Object::Int(1));
        inner.insert(Object::Int(2));
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn test_empty_collections_falsy() {
        assert!(!alloc_list(vec![]).is_truthy());
        assert!(!alloc_dict(DictMap::new()).is_truthy());
        assert!(!alloc_tuple(vec![]).is_truthy());
        assert!(alloc_list(vec![Object::Int(1)]).is_truthy());
    }

    #[test]
    fn test_collection_display() {
        let list = alloc_list(vec![Object::Int(1), Object::Int(2)]);
        assert_eq!(format!("{}", list), "[1, 2]");

        let tuple = alloc_tuple(vec![Object::Int(1)]);
        assert_eq!(format!("{}", tuple), "(1,)");

        let tuple2 = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        assert_eq!(format!("{}", tuple2), "(1, 2)");
    }

    #[test]
    fn test_dict_equality_ignores_order() {
        // 同内容不同插入序的 dict 应相等（与 Python 一致）
        let mut m1 = DictMap::new();
        m1.insert(alloc_string("a"), Object::Int(1));
        m1.insert(alloc_string("b"), Object::Int(2));
        let mut m2 = DictMap::new();
        m2.insert(alloc_string("b"), Object::Int(2));
        m2.insert(alloc_string("a"), Object::Int(1));
        assert_eq!(alloc_dict(m1), alloc_dict(m2));
    }

    #[test]
    fn test_dict_remove_returns_value_and_missing_raises() {
        let mut m = DictMap::new();
        m.insert(alloc_string("k"), Object::Int(9));
        let d = alloc_dict(m);
        assert_eq!(d.dict_remove(&alloc_string("k")).unwrap(), Object::Int(9));
        assert!(d.dict_remove(&alloc_string("missing")).is_err()); // KeyError
    }

    #[test]
    fn test_list_concat_and_repeat() {
        let l1 = alloc_list(vec![Object::Int(1), Object::Int(2)]);
        let l2 = alloc_list(vec![Object::Int(3)]);
        assert_eq!(l1.list_concat(&l2).unwrap(), alloc_list(vec![Object::Int(1), Object::Int(2), Object::Int(3)]));
        assert_eq!(l2.list_repeat(&Object::Int(3)).unwrap(), alloc_list(vec![Object::Int(3), Object::Int(3), Object::Int(3)]));
    }

    #[test]
    fn test_dict_zero_float_key_collision() {
        // -0.0 与 0.0 视为同一键（02-types.md:352）
        let mut m = DictMap::new();
        m.insert(Object::Float(0.0), Object::Int(1));
        m.insert(Object::Float(-0.0), Object::Int(2));
        assert_eq!(m.len(), 1);
        assert_eq!(m.get(&Object::Float(0.0)), Some(&Object::Int(2)));
    }
}
```
