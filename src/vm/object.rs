//! mslang 运行时对象系统基础类型。
//!
//! 参照 [11-bytecode-vm](../../../docs/mslang/11-bytecode-vm.md) Object 枚举与 MsObjHeader 布局，
//! [14-gc](../../../docs/mslang/14-gc.md) TypeTag 枚举，
//! [02-types](../../../docs/mslang/02-types.md) 类型和真值规则。
//!
//! 本模块为下游任务（21–55）引用对象模型的规范锚点。

use std::collections::{HashMap, HashSet};
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
/// 布局来自 11-bytecode-vm.md（偏移图）；GC 语义见 14-gc.md。
///
/// 字段偏移（与 14-gc.md 的字节偏移图严格一致）：
///   0     gc_meta   (u8)
///   1     type_tag  (u8)
///   2-3   size      (u16)
///   4-7   _padding  (u32)   ← bytes 4-7 全部显式命名，无隐式填充
///   8-15  class_ptr (u64)
#[repr(C)]
pub struct MsObjHeader {
    pub gc_meta: u8,    // GC 元数据（三色标记、代数、finalizer、pin）
    pub type_tag: u8,   // TypeTag 枚举值
    pub size: u16,      // 对象总大小（字节，含头部）
    pub _padding: u32,  // bytes 4-7，对齐填充，保留
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

/// 有序可变映射（保持插入顺序，与 Python 3.7+ 一致）。
/// `entries` 保存键值，`order` 保存键的插入顺序以供迭代/Display。
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

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> Vec<&Object> {
        self.order.iter().collect()
    }

    pub fn items(&self) -> Vec<(&Object, &Object)> {
        self.order
            .iter()
            .filter_map(|k| self.entries.get(k).map(|v| (k, v)))
            .collect()
    }
}

impl Default for DictMap {
    fn default() -> Self {
        Self::new()
    }
}

/// 堆上 List 对象。data_ptr 指向 Box<Vec<Object>>。
#[repr(C)]
pub struct MsList {
    pub header: MsObjHeader,
    pub data_ptr: *mut Vec<Object>,
}

/// 堆上 Dict 对象。data_ptr 指向 Box<DictMap>。
#[repr(C)]
pub struct MsDict {
    pub header: MsObjHeader,
    pub data_ptr: *mut DictMap,
}

/// 堆上 Tuple 对象。data_ptr 指向 Box<Vec<Object>>（不可变语义由上层保证）。
#[repr(C)]
pub struct MsTuple {
    pub header: MsObjHeader,
    pub data_ptr: *mut Vec<Object>,
    pub len: u32,
}

/// 堆上 Set 对象。data_ptr 指向 Box<HashSet<Object>>。
#[repr(C)]
pub struct MsSet {
    pub header: MsObjHeader,
    pub data_ptr: *mut HashSet<Object>,
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
/// 返回值的生命周期由调用方约束（`'a`），**不可**用 `'static`——
/// 数据来自堆分配，task 52 GC 上线后会被回收，`'static` 会绕过借用检查器、
/// 掩盖 use-after-free。调用方须在 unsafe 契约中保证 MsStr 在 `'a` 期间有效。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_string` 分配的、在 `'a` 期间保持有效的 `MsStr`，
/// 且其内容为合法 UTF-8（`alloc_string` 保证）。
pub unsafe fn read_str<'a>(ptr: *mut MsObjHeader) -> &'a str {
    let ms_str = ptr as *mut MsStr;
    let data_ptr = (*ms_str).data_ptr;
    let data_len = (*ms_str).data_len as usize;
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(data_ptr, data_len))
}

/// 分配 List 对象，返回 Object::Ref。
pub fn alloc_list(items: Vec<Object>) -> Object {
    let data_ptr = Box::into_raw(Box::new(items));
    let obj = Box::new(MsList {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::LIST as u8,
            size: std::mem::size_of::<MsList>() as u16,
            _padding: 0,
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
            gc_meta: 0,
            type_tag: TypeTag::DICT as u8,
            size: std::mem::size_of::<MsDict>() as u16,
            _padding: 0,
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
            gc_meta: 0,
            type_tag: TypeTag::TUPLE as u8,
            size: std::mem::size_of::<MsTuple>() as u16,
            _padding: 0,
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
            gc_meta: 0,
            type_tag: TypeTag::SET as u8,
            size: std::mem::size_of::<MsSet>() as u16,
            _padding: 0,
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
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    unsafe { !read_str(*ptr).is_empty() }
                } else if tag == TypeTag::LIST as u8 {
                    // SAFETY: type_tag 为 LIST，指针由 alloc_list 分配。
                    unsafe { !read_list(*ptr).is_empty() }
                } else if tag == TypeTag::DICT as u8 {
                    // SAFETY: type_tag 为 DICT，指针由 alloc_dict 分配。
                    unsafe { !read_dict(*ptr).is_empty() }
                } else if tag == TypeTag::TUPLE as u8 {
                    // SAFETY: type_tag 为 TUPLE，指针由 alloc_tuple 分配。
                    unsafe { !read_tuple(*ptr).is_empty() }
                } else if tag == TypeTag::SET as u8 {
                    // SAFETY: type_tag 为 SET，指针由 alloc_set 分配。
                    unsafe { !read_set(*ptr).is_empty() }
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
                debug_assert!(!ptr.is_null(), "null Object::Ref");
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

/// 比较算子（VM 本地定义，与 compiler::OpCode 解耦）。
#[derive(Debug, Clone, Copy)]
pub enum CmpOp {
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

// ---------------------------------------------------------------------------
// 算术运算（task 21）
// ---------------------------------------------------------------------------

impl Object {
    pub fn add(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => a
                .checked_add(*b)
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer addition overflow".to_string()),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 + b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a + *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a + b)),
            (Object::Ref(a), Object::Ref(b))
                if unsafe { (*(*a)).type_tag } == TypeTag::STRING as u8
                    && unsafe { (*(*b)).type_tag } == TypeTag::STRING as u8 =>
            {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let result = unsafe { read_str(*a) }.to_owned() + unsafe { read_str(*b) };
                Ok(alloc_string(&result))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for +: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn subtract(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => a
                .checked_sub(*b)
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer subtraction overflow".to_string()),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 - b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a - *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a - b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for -: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    pub fn multiply(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => a
                .checked_mul(*b)
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer multiplication overflow".to_string()),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 * b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a * *b as f64)),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a * b)),
            (Object::Ref(a), Object::Int(b)) | (Object::Int(b), Object::Ref(a))
                if unsafe { (*(*a)).type_tag } == TypeTag::STRING as u8 =>
            {
                debug_assert!(!a.is_null(), "null Object::Ref");
                if *b < 0 {
                    return Err("TypeError: can't multiply string by negative int".to_string());
                }
                // 防止 `*b as usize` 触发 OOM abort：限制结果总长度（DoS 缓解）
                const MAX_REPEAT_LEN: usize = 1 << 30; // 1 GiB 上限
                let unit = unsafe { read_str(*a) }.len();
                let total = unit
                    .checked_mul(*b as usize)
                    .ok_or_else(|| "OverflowError: string repeat count too large".to_string())?;
                if total > MAX_REPEAT_LEN {
                    return Err("MemoryError: string repeat result too large".to_string());
                }
                let repeated = unsafe { read_str(*a) }.repeat(*b as usize);
                Ok(alloc_string(&repeated))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for *: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn divide(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) => Err("ZeroDivisionError: division by zero".to_string()),
            (Object::Int(a), Object::Int(b)) => Ok(Object::Float(*a as f64 / *b as f64)),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float(*a as f64 / b)),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a / *b as f64)),
            // Float 除零遵循 IEEE 754：1.0/0.0 = +inf, -1.0/0.0 = -inf, 0.0/0.0 = NaN
            // 参照 02-types.md § 特殊浮点值
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a / b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for /: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    pub fn floor_divide(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) | (_, Object::Float(0.0)) => {
                Err("ZeroDivisionError: integer division or modulo by zero".to_string())
            }
            // 整数 floor 除法须为精确整数运算（02-types.md:32），不走 f64（>2^53 丢精度）。
            // 向负无穷取整（Python `//`）：截断商在"被除数与除数异号且不整除"时减 1，
            // 与 modulo 自洽（a == (a//b)*b + (a%b)）。注：Rust `div_euclid` 为 Euclid 除法，
            // 负除数时商 != floor（余数恒 ≥ 0），故不能用。
            (Object::Int(a), Object::Int(b)) => {
                let q = a / b;
                let r = a % b;
                let q = if r != 0 && (r < 0) != (*b < 0) {
                    q - 1
                } else {
                    q
                };
                Ok(Object::Int(q))
            }
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float((*a as f64 / b).floor())),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float((a / *b as f64).floor())),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float((a / b).floor())),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for //: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn modulo(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (_, Object::Int(0)) | (_, Object::Float(0.0)) => {
                Err("ZeroDivisionError: integer division or modulo by zero".to_string())
            }
            // floor-mod（Python `%`，符号跟随除数）：截断余数在"被除数与除数异号且不整除"
            // 时加除数校正，与 floor_divide 自洽（a == (a//b)*b + (a%b)）。注：Rust
            // `rem_euclid` 为 Euclid 余数（恒 ≥ 0），负除数时 != Python %，故不能用。
            (Object::Int(a), Object::Int(b)) => {
                let r = a % b;
                let r = if r != 0 && (r < 0) != (*b < 0) {
                    r + b
                } else {
                    r
                };
                Ok(Object::Int(r))
            }
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a - (a / b).floor() * b)),
            (Object::Int(a), Object::Float(b)) => {
                let a = *a as f64;
                Ok(Object::Float(a - (a / b).floor() * b))
            }
            (Object::Float(a), Object::Int(b)) => {
                let b = *b as f64;
                Ok(Object::Float(a - (a / b).floor() * b))
            }
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for %: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    pub fn power(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) if *b >= 0 => {
                // i64 的 ** ：指数 ≥ 64 必溢出（|a|≥2 时），且 checked_pow 取 u32 指数，
                // 超大指数会被 `as u32` 截断导致静默错误值。先按溢出处理。
                if *b >= 64 {
                    return Err("OverflowError: integer power overflow".to_string());
                }
                a.checked_pow(*b as u32)
                    .map(Object::Int)
                    .ok_or_else(|| "OverflowError: integer power overflow".to_string())
            }
            (Object::Int(a), Object::Int(b)) => Ok(Object::Float((*a as f64).powf(*b as f64))),
            (Object::Int(a), Object::Float(b)) => Ok(Object::Float((*a as f64).powf(*b))),
            (Object::Float(a), Object::Int(b)) => Ok(Object::Float(a.powf(*b as f64))),
            (Object::Float(a), Object::Float(b)) => Ok(Object::Float(a.powf(*b))),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for **: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn negate(&self) -> Result<Object, String> {
        match self {
            // checked_neg：-i64::MIN 溢出，须报 OverflowError（02-types.md:79）
            Object::Int(n) => n
                .checked_neg()
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer negation overflow".to_string()),
            Object::Float(n) => Ok(Object::Float(-n)),
            _ => Err(format!(
                "TypeError: bad operand type for unary -: '{}'",
                self.type_name()
            )),
        }
    }
}

impl Object {
    pub fn compare(&self, other: &Object, op: CmpOp) -> Result<Object, String> {
        let result = match op {
            CmpOp::Equal => self == other,
            CmpOp::NotEqual => self != other,
            CmpOp::Less => self.try_less(other)?,
            CmpOp::Greater => self.try_greater(other)?,
            CmpOp::LessEqual => self.try_less_equal(other)?,
            CmpOp::GreaterEqual => self.try_greater_equal(other)?,
        };
        Ok(Object::Bool(result))
    }

    fn try_less(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a < b),
            (Object::Float(a), Object::Float(b)) => Ok(a < b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) < *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a < (*b as f64)),
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    Ok(unsafe { read_str(*a) } < unsafe { read_str(*b) })
                } else {
                    Err(format!(
                        "TypeError: '<' not supported between instances of '{}' and '{}'",
                        self.type_name(),
                        other.type_name()
                    ))
                }
            }
            _ => Err(format!(
                "TypeError: '<' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    fn try_greater(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a > b),
            (Object::Float(a), Object::Float(b)) => Ok(a > b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) > *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a > (*b as f64)),
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    Ok(unsafe { read_str(*a) } > unsafe { read_str(*b) })
                } else {
                    Err(format!(
                        "TypeError: '>' not supported between instances of '{}' and '{}'",
                        self.type_name(),
                        other.type_name()
                    ))
                }
            }
            _ => Err(format!(
                "TypeError: '>' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    fn try_less_equal(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a <= b),
            (Object::Float(a), Object::Float(b)) => Ok(a <= b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) <= *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a <= (*b as f64)),
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    Ok(unsafe { read_str(*a) } <= unsafe { read_str(*b) })
                } else {
                    Err(format!(
                        "TypeError: '<=' not supported between instances of '{}' and '{}'",
                        self.type_name(),
                        other.type_name()
                    ))
                }
            }
            _ => Err(format!(
                "TypeError: '<=' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    fn try_greater_equal(&self, other: &Object) -> Result<bool, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(a >= b),
            (Object::Float(a), Object::Float(b)) => Ok(a >= b),
            (Object::Int(a), Object::Float(b)) => Ok((*a as f64) >= *b),
            (Object::Float(a), Object::Int(b)) => Ok(*a >= (*b as f64)),
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a == TypeTag::STRING as u8 && tag_b == TypeTag::STRING as u8 {
                    Ok(unsafe { read_str(*a) } >= unsafe { read_str(*b) })
                } else {
                    Err(format!(
                        "TypeError: '>=' not supported between instances of '{}' and '{}'",
                        self.type_name(),
                        other.type_name()
                    ))
                }
            }
            _ => Err(format!(
                "TypeError: '>=' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    /// `is`：身份比较。Ref↔Ref 比指针；inline 类型抛 TypeError（02-types.md:313）。
    pub fn is_identity(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Ref(a), Object::Ref(b)) => {
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                Ok(Object::Bool(*a == *b))
            }
            // 任意一侧为 inline 类型：is 不可用
            _ => Err(format!(
                "TypeError: 'is' cannot be used with inline types '{}'/'{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    /// String 子串 `in`：判断 self 是否包含 needle。
    /// List/Dict/Set 的 `in` 由 task 22 实现（集合成员判断）。
    pub fn contains_str(&self, needle: &Object) -> Result<Object, String> {
        match (self, needle) {
            (Object::Ref(h), Object::Ref(n))
                if unsafe { (*(*h)).type_tag } == TypeTag::STRING as u8
                    && unsafe { (*(*n)).type_tag } == TypeTag::STRING as u8 =>
            {
                debug_assert!(!h.is_null() && !n.is_null(), "null Object::Ref");
                Ok(Object::Bool(unsafe { read_str(*h).contains(read_str(*n)) }))
            }
            _ => Err(format!(
                "TypeError: 'in' (string) requires 'str' in 'str', got '{}' and '{}'",
                self.type_name(),
                needle.type_name()
            )),
        }
    }
}

impl Object {
    pub fn bit_and(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a & b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for &: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn bit_or(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a | b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for |: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn bit_xor(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a ^ b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for ^: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn bit_not(&self) -> Result<Object, String> {
        match self {
            Object::Int(n) => Ok(Object::Int(!n)),
            _ => Err(format!(
                "TypeError: bad operand type for unary ~: '{}'",
                self.type_name()
            )),
        }
    }

    pub fn left_shift(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(_), Object::Int(b)) if *b < 0 => {
                Err("ValueError: negative shift count".to_string())
            }
            (Object::Int(_), Object::Int(b)) if *b >= 64 => {
                Err("ValueError: shift count too large".to_string())
            }
            // 注：b ∈ [0,63] 时 checked_shl 必返回 Some（仅校验位移量，已由上面守卫保证）。
            // 位移结果若越过 i64 范围（如 1<<63 得 i64::MIN 负值）按 i64 回绕返回——
            // 02-types.md 未规定左移溢出语义，此处采用"回绕"而非 OverflowError。
            (Object::Int(a), Object::Int(b)) => a
                .checked_shl(*b as u32)
                .map(Object::Int)
                .ok_or_else(|| "OverflowError: integer left shift overflow".to_string()),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for <<: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }

    pub fn right_shift(&self, other: &Object) -> Result<Object, String> {
        match (self, other) {
            (Object::Int(_), Object::Int(b)) if *b < 0 => {
                Err("ValueError: negative shift count".to_string())
            }
            (Object::Int(_), Object::Int(b)) if *b >= 64 => {
                Err("ValueError: shift count too large".to_string())
            }
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a >> b)),
            _ => Err(format!(
                "TypeError: unsupported operand type(s) for >>: '{}' and '{}'",
                self.type_name(),
                other.type_name()
            )),
        }
    }
}

impl Object {
    pub fn logical_not(&self) -> Object {
        Object::Bool(!self.is_truthy())
    }

    pub fn logical_and(&self, other: &Object) -> Object {
        if self.is_truthy() {
            other.clone()
        } else {
            self.clone()
        }
    }

    pub fn logical_or(&self, other: &Object) -> Object {
        if self.is_truthy() {
            self.clone()
        } else {
            other.clone()
        }
    }
}

impl Object {
    pub fn to_int(&self) -> Result<Object, String> {
        match self {
            Object::Int(_) => Ok(self.clone()),
            Object::Float(f) => {
                // 拒绝 NaN / ±Infinity / 越界（Python 报 ValueError/OverflowError），
                // 避免 `*f as i64` 静默饱和或 NaN→0。
                if f.is_nan() {
                    return Err("ValueError: cannot convert NaN to int".to_string());
                }
                if f.is_infinite() || *f < i64::MIN as f64 || *f > i64::MAX as f64 {
                    return Err("OverflowError: float too large to convert to int".to_string());
                }
                Ok(Object::Int(*f as i64))
            }
            Object::Bool(b) => Ok(Object::Int(if *b { 1 } else { 0 })),
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let s = unsafe { read_str(*ptr) };
                s.parse::<i64>()
                    .map(Object::Int)
                    .map_err(|_| format!("ValueError: invalid literal for int(): '{}'", s))
            }
            Object::Nil => Err("TypeError: cannot convert nil to int".to_string()),
            _ => Err(format!(
                "TypeError: cannot convert {} to int",
                self.type_name()
            )),
        }
    }

    pub fn to_float(&self) -> Result<Object, String> {
        match self {
            Object::Float(_) => Ok(self.clone()),
            Object::Int(n) => Ok(Object::Float(*n as f64)),
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                let s = unsafe { read_str(*ptr) };
                s.parse::<f64>()
                    .map(Object::Float)
                    .map_err(|_| format!("ValueError: invalid literal for float(): '{}'", s))
            }
            _ => Err(format!(
                "TypeError: cannot convert {} to float",
                self.type_name()
            )),
        }
    }

    pub fn to_str(&self) -> Object {
        alloc_string(&format!("{}", self))
    }

    pub fn to_bool(&self) -> Object {
        Object::Bool(self.is_truthy())
    }
}

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
                unsafe { read_list(*ptr) }
                    .pop()
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
                    return Err(format!(
                        "IndexError: list assignment index {} out of range",
                        index
                    ));
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
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => Ok(
                Object::Bool(unsafe { read_list(*ptr) }.iter().any(|x| x == value)),
            ),
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
                    Err("ValueError: list.remove(x): x not in list".to_string())
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
                    .take(*b as usize)
                    .flatten()
                    .collect();
                Ok(alloc_list(result))
            }
            _ => Err("TypeError: * requires a list and an int".to_string()),
        }
    }
}

impl Object {
    /// `d[key]`：不存在返回 `Object::Nil`（02-types.md:181，不抛异常）。
    pub fn dict_get(&self, key: &Object) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                Ok(unsafe { read_dict(*ptr) }
                    .get(key)
                    .cloned()
                    .unwrap_or(Object::Nil))
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
                unsafe { read_dict(*ptr) }
                    .remove(key)
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
                let keys: Vec<Object> = unsafe { read_dict(*ptr) }
                    .keys()
                    .into_iter()
                    .cloned()
                    .collect();
                Ok(alloc_list(keys))
            }
            _ => Err("TypeError: keys() requires a dict".to_string()),
        }
    }

    /// `d.items()` → 新 List of Tuple(key, value)（按插入序）。
    pub fn dict_items(&self) -> Result<Object, String> {
        match self {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                let pairs: Vec<Object> = unsafe { read_dict(*ptr) }
                    .items()
                    .into_iter()
                    .map(|(k, v)| Object::make_tuple(vec![k.clone(), v.clone()]))
                    .collect();
                Ok(alloc_list(pairs))
            }
            _ => Err("TypeError: items() requires a dict".to_string()),
        }
    }
}

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
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => Ok(
                Object::Bool(unsafe { read_tuple(*ptr) }.iter().any(|x| x == value)),
            ),
            _ => Err("TypeError: 'in' requires a tuple".to_string()),
        }
    }
}

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
                let result: HashSet<Object> = unsafe { read_set(*a) }
                    .iter()
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
                let result: HashSet<Object> = unsafe { read_set(*a) }
                    .iter()
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
                    if !result.remove(x) {
                        result.insert(x.clone());
                    }
                }
                Ok(alloc_set(result))
            }
            _ => Err("TypeError: ^ requires sets".to_string()),
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
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    write!(f, "{}", unsafe { read_str(*ptr) })
                } else if tag == TypeTag::LIST as u8 {
                    // SAFETY: type_tag 为 LIST，指针由 alloc_list 分配。
                    let items = unsafe { read_list(*ptr) };
                    let strs: Vec<String> = items.iter().map(|o| format!("{}", o)).collect();
                    write!(f, "[{}]", strs.join(", "))
                } else if tag == TypeTag::DICT as u8 {
                    // SAFETY: type_tag 为 DICT，指针由 alloc_dict 分配。
                    let map = unsafe { read_dict(*ptr) };
                    let strs: Vec<String> = map
                        .items()
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect();
                    write!(f, "{{{}}}", strs.join(", "))
                } else if tag == TypeTag::TUPLE as u8 {
                    // SAFETY: type_tag 为 TUPLE，指针由 alloc_tuple 分配。
                    let items = unsafe { read_tuple(*ptr) };
                    let strs: Vec<String> = items.iter().map(|o| format!("{}", o)).collect();
                    if strs.len() == 1 {
                        write!(f, "({},)", strs[0])
                    } else {
                        write!(f, "({})", strs.join(", "))
                    }
                } else if tag == TypeTag::SET as u8 {
                    // SAFETY: type_tag 为 SET，指针由 alloc_set 分配。
                    let inner = unsafe { read_set(*ptr) };
                    // HashSet 迭代序不确定，Display 排序以保证输出稳定（便于调试与测试）
                    let mut strs: Vec<String> = inner.iter().map(|o| format!("{}", o)).collect();
                    strs.sort();
                    write!(f, "{{{}}}", strs.join(", "))
                } else {
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
                debug_assert!(!a.is_null() && !b.is_null(), "null Object::Ref");
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag_a = unsafe { (**a).type_tag };
                let tag_b = unsafe { (**b).type_tag };
                if tag_a != tag_b {
                    return false;
                }
                if tag_a == TypeTag::STRING as u8 {
                    // SAFETY: 两侧 type_tag 均为 STRING。
                    unsafe { read_str(*a) == read_str(*b) }
                } else if tag_a == TypeTag::LIST as u8 {
                    // SAFETY: 两侧 type_tag 均为 LIST。
                    unsafe { read_list(*a) == read_list(*b) }
                } else if tag_a == TypeTag::TUPLE as u8 {
                    // SAFETY: 两侧 type_tag 均为 TUPLE。
                    unsafe { read_tuple(*a) == read_tuple(*b) }
                } else if tag_a == TypeTag::DICT as u8 {
                    // Dict 相等性仅比较 entries（与 Python 一致）；
                    // 插入顺序仅影响 Display/迭代，不影响 ==。
                    // SAFETY: 两侧 type_tag 均为 DICT。
                    let ma = unsafe { read_dict(*a) };
                    let mb = unsafe { read_dict(*b) };
                    ma.entries == mb.entries
                } else if tag_a == TypeTag::SET as u8 {
                    // SAFETY: 两侧 type_tag 均为 SET。
                    unsafe { read_set(*a) == read_set(*b) }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// Object 满足 Eq 的不变性：Float(NaN) 永不可哈希（Hash 在 NaN 上 panic），
/// 故 NaN 不会进入 HashMap/HashSet；其余类型的 PartialEq 均为等价关系。
/// 因此 `==` 的 NaN 非自反性不影响集合正确性，可安全 impl Eq。
impl Eq for Object {}

/// Hash（为后续集合类型准备）。引用 02-types.md § 可哈希类型。
impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Object::Nil => 0u8.hash(state),
            Object::Bool(b) => b.hash(state),
            Object::Int(n) => {
                // 与 Float 保持哈希一致：能无损表示为 f64 的整数走 Float 路径
                let f = *n as f64;
                if (f as i64) == *n {
                    hash_f64_normalized(f, state);
                } else {
                    n.hash(state) // 超出 f64 精度，无 float 可与之相等
                }
            }
            Object::Float(f) => hash_f64_normalized(*f, state),
            Object::Ref(ptr) => {
                debug_assert!(!ptr.is_null(), "null Object::Ref");
                // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
                let tag = unsafe { (**ptr).type_tag };
                if tag == TypeTag::STRING as u8 {
                    // SAFETY: type_tag 为 STRING。
                    unsafe { read_str(*ptr) }.hash(state)
                } else if tag == TypeTag::TUPLE as u8 {
                    // 递归哈希元素；若 tuple 含 List/Dict/Set 元素，其 Hash 会 panic（TypeError 传播）
                    // SAFETY: type_tag 为 TUPLE。
                    unsafe { read_tuple(*ptr) }.hash(state)
                } else if tag == TypeTag::LIST as u8
                    || tag == TypeTag::DICT as u8
                    || tag == TypeTag::SET as u8
                {
                    // 运行时通过 type_name 报 TypeError
                    let type_str = if tag == TypeTag::LIST as u8 {
                        "list"
                    } else if tag == TypeTag::DICT as u8 {
                        "dict"
                    } else {
                        "set"
                    };
                    panic!("TypeError: unhashable type: '{}'", type_str);
                } else {
                    (*ptr as usize).hash(state)
                }
            }
        }
    }
}

/// 归一化 f64 哈希：NaN 不可哈希（panic）；±0.0 视为同一键（与 02-types.md:352 一致）。
fn hash_f64_normalized<H: Hasher>(f: f64, state: &mut H) {
    if f.is_nan() {
        panic!("TypeError: unhashable type: float NaN");
    }
    let bits = if f == 0.0 {
        0.0f64.to_bits()
    } else {
        f.to_bits()
    };
    bits.hash(state);
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
    fn test_hash_int_float_consistency() {
        // Eq/Hash 契约：Int(n) == Float(n as f64) ⟹ 二者哈希必须相等（02-types.md:305）
        use std::collections::hash_map::DefaultHasher;
        fn h(o: &Object) -> u64 {
            let mut s = DefaultHasher::new();
            o.hash(&mut s);
            s.finish()
        }
        assert_eq!(h(&Object::Int(0)), h(&Object::Float(0.0)));
        assert_eq!(h(&Object::Int(42)), h(&Object::Float(42.0)));
        assert_eq!(h(&Object::Int(-7)), h(&Object::Float(-7.0)));
    }

    #[test]
    fn test_hash_float_zero_sign() {
        // -0.0 与 0.0 哈希一致（02-types.md:352）
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
        Object::Float(f64::NAN).hash(&mut DefaultHasher::new());
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
    fn test_header_field_offsets() {
        // MsObjHeader 字段偏移须与 14-gc.md 布局图一致
        assert_eq!(std::mem::offset_of!(MsObjHeader, gc_meta), 0);
        assert_eq!(std::mem::offset_of!(MsObjHeader, type_tag), 1);
        assert_eq!(std::mem::offset_of!(MsObjHeader, size), 2);
        assert_eq!(std::mem::offset_of!(MsObjHeader, class_ptr), 8);
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

    // task 21 运算符测试

    #[test]
    fn test_int_add() {
        let result = Object::Int(10).add(&Object::Int(3)).unwrap();
        assert_eq!(result, Object::Int(13));
    }

    #[test]
    fn test_int_div_returns_float() {
        let result = Object::Int(10).divide(&Object::Int(3)).unwrap();
        assert!(matches!(result, Object::Float(_)));
    }

    #[test]
    fn test_floor_div_negative() {
        let result = Object::Int(-7).floor_divide(&Object::Int(2)).unwrap();
        assert_eq!(result, Object::Int(-4));
    }

    #[test]
    fn test_power() {
        let result = Object::Int(2).power(&Object::Int(10)).unwrap();
        assert_eq!(result, Object::Int(1024));
    }

    #[test]
    fn test_string_concat() {
        let result = alloc_string("hello").add(&alloc_string(" world")).unwrap();
        assert_eq!(result, alloc_string("hello world"));
    }

    #[test]
    fn test_string_repeat() {
        let result = alloc_string("ab").multiply(&Object::Int(3)).unwrap();
        assert_eq!(result, alloc_string("ababab"));
    }

    #[test]
    fn test_division_by_zero() {
        // Int 除零 → ZeroDivisionError
        let result = Object::Int(10).divide(&Object::Int(0));
        assert!(result.is_err());

        // Float 除零 → IEEE 754（参照 02-types.md § 特殊浮点值）
        let result = Object::Float(1.0).divide(&Object::Float(0.0)).unwrap();
        assert_eq!(result, Object::Float(f64::INFINITY));

        let result = Object::Float(-1.0).divide(&Object::Float(0.0)).unwrap();
        assert_eq!(result, Object::Float(f64::NEG_INFINITY));
    }

    #[test]
    fn test_integer_overflow() {
        let max_int = Object::Int(i64::MAX);
        let result = max_int.add(&Object::Int(1));
        assert!(result.is_err());

        let result = Object::Int(2).power(&Object::Int(63));
        assert!(result.is_err());
    }

    #[test]
    fn test_bitwise_int_only() {
        assert!(Object::Int(5).bit_and(&Object::Int(3)).is_ok());
        assert!(Object::Float(5.0).bit_and(&Object::Float(3.0)).is_err());
    }

    #[test]
    fn test_logical_short_circuit() {
        let result = Object::Int(0).logical_and(&Object::Int(42));
        assert_eq!(result, Object::Int(0));

        let result = Object::Int(1).logical_and(&Object::Int(42));
        assert_eq!(result, Object::Int(42));
    }

    #[test]
    fn test_type_conversion() {
        assert_eq!(alloc_string("42").to_int().unwrap(), Object::Int(42));
        assert_eq!(Object::Int(42).to_float().unwrap(), Object::Float(42.0));
        assert_eq!(Object::Int(0).to_bool(), Object::Bool(false));
    }

    #[test]
    fn test_floor_div_and_mod_consistency() {
        // // 与 % 自洽：a == (a//b)*b + (a%b)，且 % 取除数符号（floor-mod）
        // 负数场景（02-types.md:72-77，与 Python 一致）
        assert_eq!(
            Object::Int(-7).floor_divide(&Object::Int(2)).unwrap(),
            Object::Int(-4)
        );
        assert_eq!(
            Object::Int(-7).modulo(&Object::Int(2)).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            Object::Int(7).modulo(&Object::Int(-2)).unwrap(),
            Object::Int(-1)
        );
        // 不变式验证
        for (a, b) in [(-7i64, 2), (7, -2), (-7, -2), (7, 2), (1_000_003, 7)] {
            let av = Object::Int(a);
            let bv = Object::Int(b);
            let q = if let Object::Int(q) = av.floor_divide(&bv).unwrap() {
                q
            } else {
                unreachable!()
            };
            let r = if let Object::Int(r) = av.modulo(&bv).unwrap() {
                r
            } else {
                unreachable!()
            };
            assert_eq!(q * b + r, a, "a={} b={} 不满足 (a//b)*b + a%b == a", a, b);
            // floor-mod 余数符号跟随除数（或为 0）
            assert!(
                r == 0 || (r < 0) == (b < 0),
                "a={} b={} 余数符号错误: r={}",
                a,
                b,
                r
            );
        }
    }

    #[test]
    fn test_floor_div_large_int_no_f64_loss() {
        // > 2^53 的整数 floor division 必须精确（不走 f64 路径）
        let big = 9_007_199_254_740_993i64; // 2^53 + 1
        assert_eq!(
            Object::Int(big).floor_divide(&Object::Int(1)).unwrap(),
            Object::Int(big)
        );
    }

    #[test]
    fn test_float_mod_floor_semantics() {
        // Float % 与 // 自洽，符号跟随除数
        assert_eq!(
            Object::Float(-7.0).modulo(&Object::Float(2.0)).unwrap(),
            Object::Float(1.0)
        );
    }

    #[test]
    fn test_negate_overflow() {
        // -i64::MIN 溢出 → OverflowError（02-types.md:79）
        assert!(Object::Int(i64::MIN).negate().is_err());
        assert_eq!(Object::Int(5).negate().unwrap(), Object::Int(-5));
    }

    #[test]
    fn test_power_huge_exponent() {
        // 指数 ≥ 64 必溢出（i64），不因 `as u32` 截断返回静默错误值
        assert!(Object::Int(2).power(&Object::Int(64)).is_err());
        assert!(Object::Int(2).power(&Object::Int(1_000_000)).is_err());
    }

    #[test]
    fn test_is_identity() {
        // Ref↔Ref：同对象 → true，不同对象 → false（身份比较）
        let s1 = alloc_string("x");
        let s2 = alloc_string("x");
        assert_eq!(s1.clone().is_identity(&s1).unwrap(), Object::Bool(true));
        assert_eq!(s1.is_identity(&s2).unwrap(), Object::Bool(false));
        // inline 类型 → TypeError（02-types.md:313）
        assert!(Object::Int(42).is_identity(&Object::Int(42)).is_err());
    }

    // ---- task 22 集合类型测试 ----

    #[test]
    fn test_list_basic() {
        let list = alloc_list(vec![Object::Int(1), Object::Int(2), Object::Int(3)]);
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
        assert_eq!(
            l1.list_concat(&l2).unwrap(),
            alloc_list(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
        assert_eq!(
            l2.list_repeat(&Object::Int(3)).unwrap(),
            alloc_list(vec![Object::Int(3), Object::Int(3), Object::Int(3)])
        );
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

    fn iset(ints: &[i64]) -> Object {
        let mut h = HashSet::new();
        for i in ints {
            h.insert(Object::Int(*i));
        }
        alloc_set(h)
    }

    #[test]
    fn test_list_set_index_and_length() {
        let l = alloc_list(vec![Object::Int(10), Object::Int(20)]);
        assert_eq!(l.list_length().unwrap(), Object::Int(2));
        assert_eq!(
            l.list_set_index(-1, Object::Int(99)).unwrap(),
            Object::Int(99)
        );
        assert_eq!(l.list_get_index(1).unwrap(), Object::Int(99));
        assert!(l.list_set_index(5, Object::Int(0)).is_err()); // 越界
    }

    #[test]
    fn test_list_contains_insert_remove() {
        let l = alloc_list(vec![Object::Int(1), Object::Int(2)]);
        assert_eq!(
            l.list_contains(&Object::Int(2)).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            l.list_contains(&Object::Int(9)).unwrap(),
            Object::Bool(false)
        );
        l.list_insert(0, Object::Int(0)).unwrap();
        assert_eq!(l.list_get_index(0).unwrap(), Object::Int(0));
        l.list_remove(&Object::Int(2)).unwrap();
        assert_eq!(l.list_length().unwrap(), Object::Int(2));
        assert!(l.list_remove(&Object::Int(99)).is_err()); // ValueError
    }

    #[test]
    fn test_list_errors() {
        assert!(alloc_list(vec![]).list_pop().is_err()); // 空列表 pop
        assert!(alloc_list(vec![Object::Int(1)]).list_get_index(5).is_err()); // 越界
        assert!(alloc_list(vec![Object::Int(1)])
            .list_repeat(&Object::Int(-1))
            .is_err()); // 负数重复
        assert!(Object::Int(1).list_push(Object::Nil).is_err()); // 非 list
        assert!(alloc_list(vec![Object::Int(1)])
            .list_concat(&Object::Int(2))
            .is_err());
    }

    #[test]
    fn test_tuple_ops() {
        let t = alloc_tuple(vec![Object::Int(1), Object::Int(2), Object::Int(3)]);
        assert_eq!(t.tuple_get_index(0).unwrap(), Object::Int(1));
        assert_eq!(t.tuple_get_index(-1).unwrap(), Object::Int(3));
        assert_eq!(t.tuple_length().unwrap(), Object::Int(3));
        assert_eq!(
            t.tuple_contains(&Object::Int(2)).unwrap(),
            Object::Bool(true)
        );
        assert!(t.tuple_get_index(9).is_err()); // 越界
        assert!(Object::Int(1).tuple_get_index(0).is_err()); // 非 tuple
    }

    #[test]
    fn test_make_tuple() {
        let t = Object::make_tuple(vec![Object::Int(1), Object::Int(2)]);
        assert_eq!(t.tuple_length().unwrap(), Object::Int(2));
        assert_eq!(format!("{}", t), "(1, 2)");
    }

    #[test]
    fn test_dict_get_set_contains() {
        let d = alloc_dict(DictMap::new());
        d.dict_set(alloc_string("a"), Object::Int(1)).unwrap();
        assert_eq!(d.dict_get(&alloc_string("a")).unwrap(), Object::Int(1));
        assert_eq!(d.dict_get(&alloc_string("missing")).unwrap(), Object::Nil); // 不存在返回 nil
        assert_eq!(
            d.dict_contains(&alloc_string("a")).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            d.dict_contains(&alloc_string("z")).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(d.dict_length().unwrap(), Object::Int(1));
    }

    #[test]
    fn test_dict_keys_items_order() {
        let mut m = DictMap::new();
        m.insert(Object::Int(1), Object::Int(10));
        m.insert(Object::Int(2), Object::Int(20));
        let d = alloc_dict(m);
        let keys = d.dict_keys().unwrap();
        assert_eq!(keys.list_get_index(0).unwrap(), Object::Int(1));
        let items = d.dict_items().unwrap();
        // 每项为 tuple(key, value)，第二项的 value 为 20
        assert_eq!(
            items.list_get_index(1).unwrap().tuple_get_index(1).unwrap(),
            Object::Int(20)
        );
        assert!(Object::Int(1).dict_get(&Object::Nil).is_err()); // 非 dict
    }

    #[test]
    fn test_set_add_remove_contains() {
        let s = alloc_set(HashSet::new());
        s.set_add(Object::Int(1)).unwrap();
        s.set_add(Object::Int(1)).unwrap(); // 去重
        s.set_add(Object::Int(2)).unwrap();
        assert_eq!(s.set_length().unwrap(), Object::Int(2));
        assert_eq!(s.set_contains(&Object::Int(1)).unwrap(), Object::Bool(true));
        assert_eq!(s.set_remove(&Object::Int(2)).unwrap(), Object::Int(2));
        assert!(s.set_remove(&Object::Int(99)).is_err()); // KeyError
        assert!(Object::Int(1).set_add(Object::Nil).is_err()); // 非 set
    }

    #[test]
    fn test_set_algebra() {
        let s1 = iset(&[1, 2, 3]);
        let s2 = iset(&[2, 3, 4]);
        assert_eq!(s1.set_union(&s2).unwrap(), iset(&[1, 2, 3, 4]));
        assert_eq!(s1.set_intersection(&s2).unwrap(), iset(&[2, 3]));
        assert_eq!(s1.set_difference(&s2).unwrap(), iset(&[1]));
        assert_eq!(s1.set_symmetric_difference(&s2).unwrap(), iset(&[1, 4]));
        assert!(s1.set_union(&Object::Int(1)).is_err()); // 非 set
    }

    #[test]
    fn test_set_and_dict_display() {
        let s = iset(&[3, 1, 2]);
        assert_eq!(format!("{}", s), "{1, 2, 3}"); // 排序稳定
        let mut m = DictMap::new();
        m.insert(alloc_string("a"), Object::Int(1));
        let d = alloc_dict(m);
        assert_eq!(format!("{}", d), "{a: 1}");
    }

    #[test]
    fn test_empty_and_nested_display() {
        assert_eq!(format!("{}", alloc_list(vec![])), "[]");
        assert_eq!(format!("{}", alloc_dict(DictMap::new())), "{}");
        assert_eq!(format!("{}", alloc_tuple(vec![])), "()");
        assert_eq!(format!("{}", alloc_set(HashSet::new())), "{}");
        let inner = alloc_list(vec![Object::Int(2), Object::Int(3)]);
        let outer = alloc_list(vec![Object::Int(1), inner]);
        assert_eq!(format!("{}", outer), "[1, [2, 3]]");
    }

    #[test]
    fn test_all_collections_truthiness() {
        assert!(alloc_list(vec![Object::Int(1)]).is_truthy());
        assert!(alloc_tuple(vec![Object::Int(1)]).is_truthy());
        assert!(alloc_set(HashSet::from([Object::Int(1)])).is_truthy());
        let mut m = DictMap::new();
        m.insert(Object::Int(1), Object::Int(1));
        assert!(alloc_dict(m).is_truthy());
    }

    #[test]
    #[should_panic(expected = "TypeError: unhashable type: 'list'")]
    fn test_list_unhashable() {
        use std::collections::hash_map::DefaultHasher;
        alloc_list(vec![Object::Int(1)]).hash(&mut DefaultHasher::new());
    }

    #[test]
    #[should_panic(expected = "TypeError: unhashable type: 'dict'")]
    fn test_dict_unhashable() {
        use std::collections::hash_map::DefaultHasher;
        alloc_dict(DictMap::new()).hash(&mut DefaultHasher::new());
    }

    #[test]
    #[should_panic(expected = "TypeError: unhashable type: 'set'")]
    fn test_set_unhashable() {
        use std::collections::hash_map::DefaultHasher;
        alloc_set(HashSet::new()).hash(&mut DefaultHasher::new());
    }

    #[test]
    #[should_panic(expected = "TypeError: unhashable type: 'list'")]
    fn test_tuple_with_unhashable_element() {
        use std::collections::hash_map::DefaultHasher;
        let t = alloc_tuple(vec![alloc_list(vec![Object::Int(1)])]);
        t.hash(&mut DefaultHasher::new());
    }

    #[test]
    fn test_object_eq_marker_for_hashmap() {
        // impl Eq for Object 使 HashMap/HashSet 可用 Object 作键
        let mut m: HashMap<Object, Object> = HashMap::new();
        m.insert(Object::Int(1), alloc_string("one"));
        m.insert(alloc_string("k"), Object::Int(9));
        assert_eq!(m.get(&Object::Int(1)), Some(&alloc_string("one")));
        assert_eq!(m.get(&alloc_string("k")), Some(&Object::Int(9)));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn test_dict_zero_float_collision_via_api() {
        // 通过 Object API 验证 -0.0/0.0 同键（02-types.md:352）
        let d = alloc_dict(DictMap::new());
        d.dict_set(Object::Float(0.0), Object::Int(1)).unwrap();
        d.dict_set(Object::Float(-0.0), Object::Int(2)).unwrap();
        assert_eq!(d.dict_length().unwrap(), Object::Int(1));
        assert_eq!(d.dict_get(&Object::Float(0.0)).unwrap(), Object::Int(2));
    }
}
