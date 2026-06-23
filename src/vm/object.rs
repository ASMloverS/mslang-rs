//! mslang 运行时对象系统基础类型。
//!
//! 参照 [11-bytecode-vm](../../../docs/mslang/11-bytecode-vm.md) Object 枚举与 MsObjHeader 布局，
//! [14-gc](../../../docs/mslang/14-gc.md) TypeTag 枚举，
//! [02-types](../../../docs/mslang/02-types.md) 类型和真值规则。
//!
//! 本模块为下游任务（21–55）引用对象模型的规范锚点。

use std::fmt;
use std::hash::{Hash, Hasher};

/// 堆对象类型标签。来自 14-gc.md。
///
/// 本定义为全局唯一权威 TypeTag，其他任务（52-gc 等）应引用此处，不得重复定义。
///
/// 变体名称采用 SCREAMING_SNAKE_CASE 以与 14-gc.md 设计文档保持一致；
/// `non_camel_case_types` 仅为本枚举显式放行。
#[allow(non_camel_case_types)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeTag {
    STRING = 1,
    LIST = 2,
    DICT = 3,
    TUPLE = 4,
    SET = 5,
    FUNCTION = 6,
    CLOSURE = 7,
    CLASS = 8,
    INSTANCE = 9,
    MODULE = 10,
    ITERATOR = 11,
    GENERATOR = 12,
    FUTURE = 13,
    CHANNEL = 14,
    BOUND_METHOD = 15,
    JOIN_HANDLE = 16,
    LARGE_OBJECT = 0xFF,
}

/// 统一对象头，16 bytes，所有堆对象的公共前缀。
/// 布局来自 11-bytecode-vm.md；GC 语义见 14-gc.md。
#[repr(C)]
pub struct MsObjHeader {
    pub gc_meta: u8,    // GC 元数据（三色标记、代数、finalizer、pin）
    pub type_tag: u8,   // TypeTag 枚举值
    pub size: u16,      // 对象总大小（字节，含头部）
    pub _padding: u16,  // 对齐填充，保留
    pub class_ptr: u64, // 指向 Class 元数据或类型描述表
}

/// 运行时值。基本类型内联，堆对象通过 Ref 指针访问。
/// 来自 11-bytecode-vm.md。
#[derive(Clone, Debug)]
pub enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Ref(*mut MsObjHeader), // 引用类型：String/List/Dict/...
}

/// 堆上 String 对象的内存布局（MVP：独立数据分配）。
/// task 52 升级为 header 后紧跟数据的内联布局。
#[repr(C)]
pub struct MsStr {
    pub header: MsObjHeader,
    pub data_ptr: *const u8,
    pub data_len: u32,
}

// ---------------------------------------------------------------------------
// 堆分配辅助函数（DRY API，下游任务 21–51 通过这些函数操作堆对象）
// ---------------------------------------------------------------------------

/// 在堆上分配一个 String 对象，返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_string(s: &str) -> Object {
    let bytes: Box<[u8]> = Box::from(s.as_bytes());
    let data_len = bytes.len() as u32;
    let data_ptr = Box::into_raw(bytes) as *const u8;

    let ms_str = Box::new(MsStr {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::STRING as u8,
            size: std::mem::size_of::<MsStr>() as u16,
            _padding: 0,
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

// ---------------------------------------------------------------------------
// Object 行为
// ---------------------------------------------------------------------------

impl Object {
    /// 真值判断。引用 02-types.md 真值规则。
    pub fn is_truthy(&self) -> bool {
        match self {
            Object::Nil => false,
            Object::Bool(b) => *b,
            Object::Int(n) => *n != 0,
            Object::Float(n) => *n != 0.0,
            Object::Ref(ptr) => {
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    unsafe { !read_str(*ptr).is_empty() }
                } else {
                    true
                }
            }
        }
    }

    /// 类型名称。引用 02-types.md。
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Nil => "nil",
            Object::Bool(_) => "bool",
            Object::Int(_) => "int",
            Object::Float(_) => "float",
            Object::Ref(ptr) => {
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
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

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    write!(f, "{}", unsafe { read_str(*ptr) })
                } else {
                    // 非 String 的 Ref 类型由后续任务（21+）扩展
                    write!(f, "<object:{}>", tag)
                }
            }
        }
    }
}

/// 相等性比较。引用 02-types.md 比较规则。
///
/// - 同类型：直接值比较
/// - Int 与 Float 交叉比较：数值比较（`42 == 42.0` → `true`）
/// - 其他不同类型：`==` 返回 `false`
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Object::Nil, Object::Nil) => true,
            (Object::Bool(a), Object::Bool(b)) => a == b,
            (Object::Int(a), Object::Int(b)) => a == b,
            (Object::Float(a), Object::Float(b)) => a == b,
            (Object::Int(a), Object::Float(b)) => (*a as f64) == *b,
            (Object::Float(a), Object::Int(b)) => *a == (*b as f64),
            (Object::Ref(a), Object::Ref(b)) => {
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    // SAFETY: 两侧 type_tag 均为 STRING。
                    unsafe { read_str(*a) == read_str(*b) }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// Hash（为后续集合类型准备）。引用 02-types.md § 可哈希类型。
impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Object::Nil => 0u8.hash(state),
            Object::Bool(b) => b.hash(state),
            Object::Int(n) => n.hash(state),
            Object::Float(f) => {
                // NaN 不可哈希（参照 02-types.md § hash）
                if f.is_nan() {
                    panic!("TypeError: unhashable type: float NaN");
                }
                // -0.0 与 0.0 哈希一致（视为同一键）
                if *f == 0.0 {
                    0.0f64.to_bits().hash(state)
                } else {
                    f.to_bits().hash(state)
                }
            }
            Object::Ref(ptr) => {
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING。
                    unsafe { read_str(*ptr) }.hash(state)
                } else {
                    (*ptr as usize).hash(state)
                }
            }
        }
    }
}

#[cfg(test)]
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
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

    #[test]
    fn test_float_display_whole_number() {
        // 整数值的 float 显示一位小数
        assert_eq!(format!("{}", Object::Float(3.0)), "3.0");
        // 非整数 float 正常显示
        assert_eq!(format!("{}", Object::Float(3.14)), "3.14");
    }

    #[test]
    fn test_falsy_display() {
        assert_eq!(format!("{}", Object::Bool(false)), "false");
        assert_eq!(format!("{}", Object::Int(0)), "0");
    }

    #[test]
    fn test_hash_consistency_for_float_zero() {
        use std::collections::hash_map::DefaultHasher;
        let mut h1 = DefaultHasher::new();
        Object::Float(0.0).hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        Object::Float(-0.0).hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    #[should_panic(expected = "TypeError: unhashable type: float NaN")]
    fn test_hash_nan_panics() {
        use std::collections::hash_map::DefaultHasher;
        let mut h = DefaultHasher::new();
        Object::Float(f64::NAN).hash(&mut h);
    }

    #[test]
    fn test_hash_basic_types() {
        // 仅验证可哈希不 panic
        use std::collections::hash_map::DefaultHasher;
        let mut h = DefaultHasher::new();
        Object::Nil.hash(&mut h);
        Object::Bool(true).hash(&mut h);
        Object::Int(42).hash(&mut h);
        Object::Float(3.14).hash(&mut h);
        alloc_string("hello").hash(&mut h);
    }

    #[test]
    fn test_type_tag_values() {
        // TypeTag 值必须与 14-gc.md 完全一致
        assert_eq!(TypeTag::STRING as u8, 1);
        assert_eq!(TypeTag::LIST as u8, 2);
        assert_eq!(TypeTag::DICT as u8, 3);
        assert_eq!(TypeTag::TUPLE as u8, 4);
        assert_eq!(TypeTag::SET as u8, 5);
        assert_eq!(TypeTag::FUNCTION as u8, 6);
        assert_eq!(TypeTag::CLOSURE as u8, 7);
        assert_eq!(TypeTag::CLASS as u8, 8);
        assert_eq!(TypeTag::INSTANCE as u8, 9);
        assert_eq!(TypeTag::MODULE as u8, 10);
        assert_eq!(TypeTag::ITERATOR as u8, 11);
        assert_eq!(TypeTag::GENERATOR as u8, 12);
        assert_eq!(TypeTag::FUTURE as u8, 13);
        assert_eq!(TypeTag::CHANNEL as u8, 14);
        assert_eq!(TypeTag::BOUND_METHOD as u8, 15);
        assert_eq!(TypeTag::JOIN_HANDLE as u8, 16);
        assert_eq!(TypeTag::LARGE_OBJECT as u8, 0xFF);
    }

    #[test]
    fn test_header_size_16_bytes() {
        // MsObjHeader 必须为 16 bytes（来自 11-bytecode-vm.md）
        assert_eq!(std::mem::size_of::<MsObjHeader>(), 16);
    }

    #[test]
    fn test_string_roundtrip() {
        let obj = alloc_string("hello world");
        // SAFETY: ptr 由 alloc_string 分配
        if let Object::Ref(ptr) = obj {
            unsafe {
                assert_eq!(read_str(ptr), "hello world");
                assert_eq!((*ptr).type_tag, TypeTag::STRING as u8);
            }
        } else {
            panic!("alloc_string should return Ref");
        }
    }
}
