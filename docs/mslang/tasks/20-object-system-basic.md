# Object 系统基础类型

## 所属阶段
Phase 2.3a - 字节码编译 + VM 核心

## 前置任务
- 16-opcode-definition

## 目标

定义运行时对象系统的基础类型（Nil, Bool, Int, Float, String），包括 Gc<T> 智能指针、Display 实现、真值规则、类型名称和相等性比较。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) Object 枚举定义，[02-types.md](../02-types.md) 类型和真值规则。

### Object 枚举（基础部分）

```rust
enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Gc<String>),
    // 后续任务扩展：
    // List, Dict, Tuple, Set, Function, Closure, ...
}
```

### 真值规则

引用 [02-types.md](../02-types.md)：
- **Truthy**：`true`、非零数值、非空字符串、非空集合
- **Falsy**：`false`、`nil`、`0`、`0.0`、`""`、空集合

### 类型名称

引用 [02-types.md](../02-types.md)：
- `type(nil)` → `"nil"`
- `type(true)` → `"bool"`
- `type(42)` → `"int"`
- `type(3.14)` → `"float"`
- `type("hello")` → `"string"`

### 相等性

- 同类型：直接值比较
- Int 与 Float 交叉比较：数值比较（`42 == 42.0` → `true`）
- 其他不同类型：`==` 返回 `false`

## 实现细节

### 文件位置

`src/vm/object.rs`

### Gc<T> 智能指针（MVP 简化版）

```rust
use std::rc::Rc;
use std::cell::RefCell;

pub struct GcBox<T> {
    data: T,
    marked: bool,
}

pub struct Gc<T> {
    inner: Rc<RefCell<GcBox<T>>>,
}

impl<T> Gc<T> {
    pub fn new(data: T) -> Self {
        Gc {
            inner: Rc::new(RefCell::new(GcBox {
                data,
                marked: false,
            })),
        }
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, GcBox<T>> {
        self.inner.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, GcBox<T>> {
        self.inner.borrow_mut()
    }
}

impl<T: Clone> Clone for Gc<T> {
    fn clone(&self) -> Self {
        Gc {
            inner: Rc::clone(&self.inner),
        }
    }
}
```

> **注意**：MVP 阶段使用 `Rc<RefCell<>>` 模拟 GC。后续阶段替换为真正的标记-清除 GC（见 [11-bytecode-vm.md](../11-bytecode-vm.md) 垃圾回收章节）。

### Object 枚举

```rust
#[derive(Clone, Debug)]
pub enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Gc<String>),
}
```

### Display trait

```rust
impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Object::Nil => write!(f, "nil"),
            Object::Bool(b) => write!(f, "{}", b),
            Object::Int(n) => write!(f, "{}", n),
            Object::Float(n) => {
                if *n == (*n as i64) as f64 && !n.is_nan() && !n.is_infinite() {
                    write!(f, "{:.1}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            Object::String(s) => write!(f, "{}", s.borrow().data),
        }
    }
}
```

### 真值判断

```rust
impl Object {
    pub fn is_truthy(&self) -> bool {
        match self {
            Object::Nil => false,
            Object::Bool(b) => *b,
            Object::Int(n) => *n != 0,
            Object::Float(n) => *n != 0.0,
            Object::String(s) => !s.borrow().data.is_empty(),
        }
    }
}
```

### 类型名称

```rust
impl Object {
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Nil => "nil",
            Object::Bool(_) => "bool",
            Object::Int(_) => "int",
            Object::Float(_) => "float",
            Object::String(_) => "string",
        }
    }
}
```

### 相等性比较

```rust
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Object::Nil, Object::Nil) => true,
            (Object::Bool(a), Object::Bool(b)) => a == b,
            (Object::Int(a), Object::Int(b)) => a == b,
            (Object::Float(a), Object::Float(b)) => a == b,
            (Object::String(a), Object::String(b)) => {
                a.borrow().data == b.borrow().data
            }
            (Object::Int(a), Object::Float(b)) => (*a as f64) == *b,
            (Object::Float(a), Object::Int(b)) => *a == (*b as f64),
            _ => false,
        }
    }
}
```

### Hash（为后续集合类型准备）

基础类型需要实现 Hash 以支持作为 dict 的键或 set 的元素：

```rust
impl std::hash::Hash for Object {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Object::Nil => 0u8.hash(state),
            Object::Bool(b) => b.hash(state),
            Object::Int(n) => n.hash(state),
            Object::Float(f) => {
                if *f == 0.0 {
                    0.0f64.to_bits().hash(state)
                } else {
                    f.to_bits().hash(state)
                }
            }
            Object::String(s) => s.borrow().data.hash(state),
        }
    }
}
```

## 验证标准

1. `Object::Nil.is_truthy()` 返回 `false`
2. `Object::Int(0).is_truthy()` 返回 `false`
3. `Object::Int(42).is_truthy()` 返回 `true`
4. `Object::String("".to_string()).is_truthy()` 返回 `false`
5. `Object::String("hello".to_string()).is_truthy()` 返回 `true`
6. `Object::Int(42) == Object::Float(42.0)` 返回 `true`
7. `Object::Bool(true) == Object::Int(1)` 返回 `false`（不同类型除 int/float 外不等）
8. 每个类型的 `type_name()` 返回正确字符串
9. `Display` 格式化正确

## 测试用例

无 `.ms` 测试文件，使用 Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truthiness() {
        assert!(!Object::Nil.is_truthy());
        assert!(!Object::Bool(false).is_truthy());
        assert!(Object::Bool(true).is_truthy());
        assert!(!Object::Int(0).is_truthy());
        assert!(Object::Int(1).is_truthy());
        assert!(!Object::Float(0.0).is_truthy());
        assert!(Object::Float(1.0).is_truthy());
        assert!(!Object::String(Gc::new(String::new())).is_truthy());
        assert!(Object::String(Gc::new("hello".to_string())).is_truthy());
    }

    #[test]
    fn test_type_names() {
        assert_eq!(Object::Nil.type_name(), "nil");
        assert_eq!(Object::Bool(true).type_name(), "bool");
        assert_eq!(Object::Int(42).type_name(), "int");
        assert_eq!(Object::Float(3.14).type_name(), "float");
        assert_eq!(Object::String(Gc::new("hi".to_string())).type_name(), "string");
    }

    #[test]
    fn test_equality() {
        assert_eq!(Object::Nil, Object::Nil);
        assert_eq!(Object::Bool(true), Object::Bool(true));
        assert_ne!(Object::Bool(true), Object::Bool(false));
        assert_eq!(Object::Int(42), Object::Int(42));
        assert_eq!(Object::Int(42), Object::Float(42.0));
        assert_eq!(Object::Float(3.14), Object::Float(3.14));
        assert_ne!(Object::Int(1), Object::Bool(true));
        assert_ne!(Object::Nil, Object::Bool(false));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Object::Nil), "nil");
        assert_eq!(format!("{}", Object::Bool(true)), "true");
        assert_eq!(format!("{}", Object::Int(42)), "42");
        assert_eq!(format!("{}", Object::String(Gc::new("hello".to_string()))), "hello");
    }

    #[test]
    fn test_int_float_cross_equality() {
        assert_eq!(Object::Int(0), Object::Float(0.0));
        assert_eq!(Object::Int(42), Object::Float(42.0));
        assert_ne!(Object::Int(42), Object::Float(42.1));
    }
}
```
