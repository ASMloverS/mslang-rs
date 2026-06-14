# Object 系统基础类型

## 所属阶段
Phase 2.3a - 字节码编译 + VM 核心

## 前置任务
- 16-opcode-definition

## 目标

定义运行时对象系统的基础类型（Nil, Bool, Int, Float, String），基于 MsObjHeader 对象模型，包括堆对象布局、堆分配辅助函数、Display 实现、真值规则、类型名称和相等性比较。本任务是下游任务（21–55）引用对象模型的规范锚点。

## 设计规格

引用 [11-bytecode-vm.md](../11-bytecode-vm.md) Object 枚举定义与 MsObjHeader 布局，[14-gc.md](../14-gc.md) TypeTag 枚举，[02-types.md](../02-types.md) 类型和真值规则。

### Object 枚举

来自 [11-bytecode-vm.md](../11-bytecode-vm.md)：

```rust
enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Ref(*mut MsObjHeader),   // 引用类型：String/List/Dict/...
}
```

基本类型（Nil、Bool、Int、Float）直接内联存储，无需堆分配。所有堆对象统一通过 `Ref(*mut MsObjHeader)` 表示，类型由 `MsObjHeader.type_tag` 区分。

### MsObjHeader 布局

来自 [11-bytecode-vm.md](../11-bytecode-vm.md)，16 bytes：

```
字节:   0         1         2-3       4-7        8-15
     ┌─────────┬─────────┬────────┬─────────┬──────────┐
     │ gc_meta │type_tag │  size  │ padding │class_ptr │
     │ 1 byte  │ 1 byte  │ 2 byte │ 2 byte  │ 8 byte   │
     └─────────┴─────────┴────────┴─────────┴──────────┘
```

- `gc_meta`：GC 元数据（三色标记、代数、finalizer、pin 标志）
- `type_tag`：类型标签，对应 `TypeTag` 枚举值
- `size`：对象大小（字节，含头部）
- `class_ptr`：指向 Class 元数据或类型描述表

### TypeTag 枚举

来自 [14-gc.md](../14-gc.md)：

```rust
#[repr(u8)]
enum TypeTag {
    STRING       = 1,
    LIST         = 2,
    DICT         = 3,
    TUPLE        = 4,
    SET          = 5,
    FUNCTION     = 6,
    CLOSURE      = 7,
    CLASS        = 8,
    INSTANCE     = 9,
    MODULE       = 10,
    ITERATOR     = 11,
    GENERATOR    = 12,
    FUTURE       = 13,
    CHANNEL      = 14,
    BOUND_METHOD = 15,
    JOIN_HANDLE  = 16,
    LARGE_OBJECT = 0xFF,
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

### MsObjHeader 结构体

```rust
/// 统一对象头，16 bytes，所有堆对象的公共前缀。
/// 布局来自 11-bytecode-vm.md；GC 语义见 14-gc.md。
#[repr(C)]
pub struct MsObjHeader {
    pub gc_meta:   u8,   // GC 元数据（三色标记、代数、finalizer、pin）
    pub type_tag:  u8,   // TypeTag 枚举值
    pub size:      u16,  // 对象总大小（字节，含头部）
    pub _padding:  u16,  // 对齐填充，保留
    pub class_ptr: u64,  // 指向 Class 元数据或类型描述表
}
```

### TypeTag 枚举

```rust
/// 堆对象类型标签。来自 14-gc.md。
/// 本定义为全局唯一权威 TypeTag，其他任务（52-gc 等）应引用此处，不得重复定义。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeTag {
    STRING       = 1,
    LIST         = 2,
    DICT         = 3,
    TUPLE        = 4,
    SET          = 5,
    FUNCTION     = 6,
    CLOSURE      = 7,
    CLASS        = 8,
    INSTANCE     = 9,
    MODULE       = 10,
    ITERATOR     = 11,
    GENERATOR    = 12,
    FUTURE       = 13,
    CHANNEL      = 14,
    BOUND_METHOD = 15,
    JOIN_HANDLE  = 16,
    LARGE_OBJECT = 0xFF,
}
```

### Object 枚举

```rust
/// 运行时值。基本类型内联，堆对象通过 Ref 指针访问。
/// 来自 11-bytecode-vm.md。
#[derive(Clone, Debug)]
pub enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Ref(*mut MsObjHeader),   // 引用类型：String/List/Dict/...
}
```

### String 堆布局

```rust
/// 堆上 String 对象的内存布局（MVP：独立数据分配）。
/// task 52 升级为 header 后紧跟数据的内联布局。
#[repr(C)]
pub struct MsStr {
    pub header:   MsObjHeader,
    pub data_ptr: *const u8,
    pub data_len: u32,
}
```

### 堆分配辅助函数

这些函数是下游任务（21–51）操作堆对象的 DRY API，无需直接使用裸指针。

```rust
/// 在堆上分配一个 String 对象，返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_string(s: &str) -> Object {
    let bytes: Box<[u8]> = Box::from(s.as_bytes());
    let data_len = bytes.len() as u32;
    let data_ptr = Box::into_raw(bytes) as *const u8;

    let ms_str = Box::new(MsStr {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::STRING as u8,
            size:      std::mem::size_of::<MsStr>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        data_ptr,
        data_len,
    });

    Object::Ref(Box::into_raw(ms_str) as *mut MsObjHeader)
}

/// 从指向 MsStr 的 Ref 指针读取字符串内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_string` 分配的有效 `MsStr`。
pub unsafe fn read_str(ptr: *mut MsObjHeader) -> &'static str {
    let ms_str = ptr as *mut MsStr;
    let data_ptr = (*ms_str).data_ptr;
    let data_len = (*ms_str).data_len as usize;
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(data_ptr, data_len))
}
```

> **Safety 说明**：这些 unsafe 操作由 task 52-gc 的 GC 基础设施封装；上层任务（21–51）通过辅助函数（`alloc_string`、`read_str` 等）间接操作，无需直接使用裸指针。

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
            Object::Ref(ptr) => {
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    write!(f, "{}", unsafe { read_str(*ptr) })
                } else {
                    // 非 String 的 Ref 类型由后续任务（21+）扩展
                    write!(f, "<object:{}>", tag)
                }
            }
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
            Object::Ref(ptr) => {
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    unsafe { !read_str(*ptr).is_empty() }
                } else {
                    true
                }
            }
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
            Object::Ref(ptr) => {
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    "string"
                } else {
                    // 后续任务扩展其他 Ref 类型的 type_name
                    "object"
                }
            }
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
            (Object::Ref(a), Object::Ref(b)) => {
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    unsafe { read_str(*a) == read_str(*b) }
                } else {
                    false
                }
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
                // NaN 不可哈希（参照 02-types.md § hash）
                if f.is_nan() {
                    panic!("TypeError: unhashable type: float NaN");
                }
                if *f == 0.0 {
                    0.0f64.to_bits().hash(state)
                } else {
                    f.to_bits().hash(state)
                }
            }
            Object::Ref(ptr) => {
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    unsafe { read_str(*ptr) }.hash(state)
                } else {
                    (*ptr as usize).hash(state)
                }
            }
        }
    }
}
```

## 验证标准

1. `Object::Nil.is_truthy()` 返回 `false`
2. `Object::Int(0).is_truthy()` 返回 `false`
3. `Object::Int(42).is_truthy()` 返回 `true`
4. `alloc_string("").is_truthy()` 返回 `false`
5. `alloc_string("hello").is_truthy()` 返回 `true`
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
        assert!(!alloc_string("").is_truthy());
        assert!(alloc_string("hello").is_truthy());
    }

    #[test]
    fn test_type_names() {
        assert_eq!(Object::Nil.type_name(), "nil");
        assert_eq!(Object::Bool(true).type_name(), "bool");
        assert_eq!(Object::Int(42).type_name(), "int");
        assert_eq!(Object::Float(3.14).type_name(), "float");
        assert_eq!(alloc_string("hi").type_name(), "string");
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
        assert_eq!(alloc_string("hello"), alloc_string("hello"));
        assert_ne!(alloc_string("hello"), alloc_string("world"));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Object::Nil), "nil");
        assert_eq!(format!("{}", Object::Bool(true)), "true");
        assert_eq!(format!("{}", Object::Int(42)), "42");
        assert_eq!(format!("{}", alloc_string("hello")), "hello");
    }

    #[test]
    fn test_int_float_cross_equality() {
        assert_eq!(Object::Int(0), Object::Float(0.0));
        assert_eq!(Object::Int(42), Object::Float(42.0));
        assert_ne!(Object::Int(42), Object::Float(42.1));
    }
}
```
