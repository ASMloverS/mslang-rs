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
        if let Some(_) = self.entries.remove(key) {
            self.order.retain(|k| k != key);
        }
        None
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
/// `ptr` 必须指向由 `alloc_list` 分配的有效 `MsList`。
pub unsafe fn read_list(ptr: *mut MsObjHeader) -> &'static mut Vec<Object> {
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
/// `ptr` 必须指向由 `alloc_dict` 分配的有效 `MsDict`。
pub unsafe fn read_dict(ptr: *mut MsObjHeader) -> &'static mut DictMap {
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

/// 读取 Tuple 对象的内部 Vec。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_tuple` 分配的有效 `MsTuple`。
pub unsafe fn read_tuple(ptr: *mut MsObjHeader) -> &'static Vec<Object> {
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
/// `ptr` 必须指向由 `alloc_set` 分配的有效 `MsSet`。
pub unsafe fn read_set(ptr: *mut MsObjHeader) -> &'static mut HashSet<Object> {
    let ms_set = ptr as *mut MsSet;
    &mut *(*ms_set).data_ptr
}
```

> **Hash + Eq 约束**：`HashSet<Object>` 和 `HashMap<Object, Object>` 要求 `Object` 实现 `Hash` 和 `Eq`。
> Task 20 已为基础类型实现 `Hash`，本任务需扩展至 `Tuple`（当所有元素可哈希时可哈希），
> 并确保 `List`/`Dict`/`Set` 在 `hash()` 时 panic（运行时 TypeError）。
> `Eq`（`PartialEq`）需扩展至集合类型的逐元素比较。

### Display 扩展

在 task 20 的 `Display` 实现中，`Object::Ref` 的 match 臂按 type_tag 分发：

```rust
Object::Ref(ptr) => {
    let tag = unsafe { (*(*ptr)).type_tag };
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
        let strs: Vec<String> = inner.iter().map(|o| format!("{}", o)).collect();
        write!(f, "{{{}}}", strs.join(", "))
    } else {
        write!(f, "<object:{}>", tag)
    }
}
```

### 真值规则扩展

```rust
Object::Ref(ptr) => {
    let tag = unsafe { (*(*ptr)).type_tag };
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
    let tag = unsafe { (*(*ptr)).type_tag };
    if tag == TypeTag::STRING as u8 {
        unsafe { read_str(*ptr) }.hash(state)
    } else if tag == TypeTag::TUPLE as u8 {
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
    let tag_a = unsafe { (*(*a)).type_tag };
    let tag_b = unsafe { (*(*b)).type_tag };
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
        let ma = unsafe { read_dict(*a) };
        let mb = unsafe { read_dict(*b) };
        ma.entries == mb.entries && ma.order == mb.order
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
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => {
                unsafe { read_list(*ptr) }.push(value);
                Ok(Object::Nil)
            }
            _ => Err("TypeError: push requires a list".to_string()),
        }
    }

    pub fn list_pop(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => {
                unsafe { read_list(*ptr) }.pop()
                    .ok_or_else(|| "IndexError: pop from empty list".to_string())
            }
            _ => Err("TypeError: pop requires a list".to_string()),
        }
    }

    pub fn list_get_index(&self, index: i64) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => {
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
}
```

### Tuple 构造辅助

```rust
impl Object {
    pub fn make_tuple(elements: Vec<Object>) -> Object {
        alloc_tuple(elements)
    }
}
```

`items()` 等方法使用 `Object::make_tuple(vec![key, value])` 构造二元组。

```rust
impl Object {
    pub fn set_union(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (*(*a)).type_tag } == TypeTag::SET as u8
                && unsafe { (*(*b)).type_tag } == TypeTag::SET as u8 =>
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
                if unsafe { (*(*a)).type_tag } == TypeTag::SET as u8
                && unsafe { (*(*b)).type_tag } == TypeTag::SET as u8 =>
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
}
```

## 验证标准

1. List 可创建、push/pop、下标访问
2. Dict 保持插入顺序，键必须可哈希
3. Tuple 不可变，可哈希（当元素可哈希时）
4. Set 元素唯一，支持集合运算
5. 空集合为 falsy，非空集合为 truthy
6. List/Dict/Set 不可哈希（作为 dict 键或 set 元素时 panic）
7. Display 格式化正确

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
}
```
