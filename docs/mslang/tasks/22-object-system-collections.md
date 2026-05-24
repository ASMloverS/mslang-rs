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

`src/vm/object.rs`（扩展 Object 枚举）

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

### Object 枚举扩展

```rust
#[derive(Clone, Debug)]
pub enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Gc<String>),
    List(Gc<Vec<Object>>),
    Dict(Gc<DictMap>),
    Tuple(Gc<Vec<Object>>),
    Set(Gc<HashSetWrapper>),
}

#[derive(Clone, Debug)]
pub struct HashSetWrapper {
    inner: HashSet<Object>,
}
```

### Display 扩展

```rust
impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // ... 基础类型同任务 20 ...
            Object::List(items) => {
                let strs: Vec<String> = items.borrow().data.iter()
                    .map(|o| format!("{}", o))
                    .collect();
                write!(f, "[{}]", strs.join(", "))
            }
            Object::Dict(map) => {
                let strs: Vec<String> = map.borrow().data.items().iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{{{}}}", strs.join(", "))
            }
            Object::Tuple(items) => {
                let strs: Vec<String> = items.borrow().data.iter()
                    .map(|o| format!("{}", o))
                    .collect();
                if strs.len() == 1 {
                    write!(f, "({},)", strs[0])
                } else {
                    write!(f, "({})", strs.join(", "))
                }
            }
            Object::Set(items) => {
                let strs: Vec<String> = items.borrow().data.inner.iter()
                    .map(|o| format!("{}", o))
                    .collect();
                write!(f, "{{{}}}", strs.join(", "))
            }
        }
    }
}
```

### 真值规则扩展

```rust
impl Object {
    pub fn is_truthy(&self) -> bool {
        match self {
            // ... 基础类型同任务 20 ...
            Object::List(items) => !items.borrow().data.is_empty(),
            Object::Dict(map) => map.borrow().data.len() > 0,
            Object::Tuple(items) => !items.borrow().data.is_empty(),
            Object::Set(items) => !items.borrow().data.inner.is_empty(),
        }
    }
}
```

### Hash 扩展

```rust
impl std::hash::Hash for Object {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            // ... 基础类型同任务 20 ...
            Object::Tuple(items) => {
                items.borrow().data.hash(state);
            }
            Object::List(_) | Object::Dict(_) | Object::Set(_) => {
                panic!("TypeError: unhashable type: '{}'", self.type_name());
            }
        }
    }
}
```

### 相等性扩展

```rust
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // ... 基础类型同任务 20 ...
            (Object::List(a), Object::List(b)) => a.borrow().data == b.borrow().data,
            (Object::Tuple(a), Object::Tuple(b)) => a.borrow().data == b.borrow().data,
            (Object::Dict(a), Object::Dict(b)) => {
                let a_map = &a.borrow().data;
                let b_map = &b.borrow().data;
                a_map.entries == b_map.entries && a_map.order == b_map.order
            }
            (Object::Set(a), Object::Set(b)) => {
                a.borrow().data.inner == b.borrow().data.inner
            }
            _ => false,
        }
    }
}
```

### List 操作

```rust
impl Object {
    pub fn list_push(&self, value: Object) -> Result<Object, String> {
        match self {
            Object::List(items) => {
                items.borrow_mut().data.push(value);
                Ok(Object::Nil)
            }
            _ => Err("TypeError: push requires a list".to_string()),
        }
    }

    pub fn list_pop(&self) -> Result<Object, String> {
        match self {
            Object::List(items) => {
                items.borrow_mut().data.pop()
                    .ok_or_else(|| "IndexError: pop from empty list".to_string())
            }
            _ => Err("TypeError: pop requires a list".to_string()),
        }
    }

    pub fn list_get_index(&self, index: i64) -> Result<Object, String> {
        match self {
            Object::List(items) => {
                let len = items.borrow().data.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(format!("IndexError: list index {} out of range", index));
                }
                Ok(items.borrow().data[idx as usize].clone())
            }
            _ => Err("TypeError: index access requires a list".to_string()),
        }
    }
}
```

### Set 操作

```rust
impl Object {
    pub fn set_union(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Set(a), Object::Set(b)) => {
                let mut result = a.borrow().data.inner.clone();
                result.extend(b.borrow().data.inner.iter().cloned());
                Ok(Object::Set(Gc::new(HashSetWrapper { inner: result })))
            }
            _ => Err("TypeError: | requires sets".to_string()),
        }
    }

    pub fn set_intersection(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Set(a), Object::Set(b)) => {
                let result: HashSet<Object> = a.borrow().data.inner.iter()
                    .filter(|x| b.borrow().data.inner.contains(x))
                    .cloned()
                    .collect();
                Ok(Object::Set(Gc::new(HashSetWrapper { inner: result })))
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
        let list = Object::List(Gc::new(vec![
            Object::Int(1), Object::Int(2), Object::Int(3),
        ]));
        assert_eq!(list.list_get_index(0).unwrap(), Object::Int(1));
        assert_eq!(list.list_get_index(-1).unwrap(), Object::Int(3));
    }

    #[test]
    fn test_list_push_pop() {
        let list = Object::List(Gc::new(vec![]));
        list.list_push(Object::Int(1)).unwrap();
        list.list_push(Object::Int(2)).unwrap();
        assert_eq!(list.list_pop().unwrap(), Object::Int(2));
    }

    #[test]
    fn test_dict_insertion_order() {
        let dict = Object::Dict(Gc::new(DictMap::new()));
        dict.dict_insert(Object::String(Gc::new("b".into())), Object::Int(2));
        dict.dict_insert(Object::String(Gc::new("a".into())), Object::Int(1));
        let keys = dict.dict_keys();
        assert_eq!(keys[0], Object::String(Gc::new("b".into())));
        assert_eq!(keys[1], Object::String(Gc::new("a".into())));
    }

    #[test]
    fn test_tuple_hashable() {
        let tuple = Object::Tuple(Gc::new(vec![Object::Int(1), Object::Int(2)]));
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
        assert!(!Object::List(Gc::new(vec![])).is_truthy());
        assert!(!Object::Dict(Gc::new(DictMap::new())).is_truthy());
        assert!(!Object::Tuple(Gc::new(vec![])).is_truthy());
        assert!(Object::List(Gc::new(vec![Object::Int(1)])).is_truthy());
    }

    #[test]
    fn test_collection_display() {
        let list = Object::List(Gc::new(vec![Object::Int(1), Object::Int(2)]));
        assert_eq!(format!("{}", list), "[1, 2]");

        let tuple = Object::Tuple(Gc::new(vec![Object::Int(1)]));
        assert_eq!(format!("{}", tuple), "(1,)");

        let tuple2 = Object::Tuple(Gc::new(vec![Object::Int(1), Object::Int(2)]));
        assert_eq!(format!("{}", tuple2), "(1, 2)");
    }
}
```
