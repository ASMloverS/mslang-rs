//! mslang 基础内置函数（native function）。
//!
//! 参照 [10-builtins](../../docs/mslang/10-builtins.md) 与
//! [25-builtins-basic](../../docs/mslang/tasks/25-builtins-basic.md)。
//!
//! 原生函数通过堆分配为 Function 对象（`Object::Ref` + `TypeTag::FUNCTION`），
//! 不新增 Object 变体。`register_builtins` 在 `VM::new()` 中调用，将所有内置函数
//! 注入全局变量表，使其无需 import 即可全局调用。

// 内置函数签名规范以 args.get(0) 取首参数（与设计文档一致）；clippy 偏好 .first()，
// 此处按文档风格统一保留 get(0)。
#![allow(clippy::get_first)]

use super::object::{
    alloc_dict, alloc_list, alloc_set, alloc_string, alloc_tuple, read_dict, read_list, read_set,
    read_str, read_tuple, CmpOp, MsObjHeader, Object, TypeTag,
};
use super::VM;

/// 原生函数调用签名。
pub type NativeFn = fn(&mut VM, &[Object]) -> Result<Object, String>;

/// 注册用的输入结构（持有 name 所有权）。
pub struct NativeFunction {
    pub name: String,
    pub func: NativeFn,
}

/// 堆上 Native Function 对象布局（参照 Task 20 对象模型）。
#[repr(C)]
pub struct MsNativeFunction {
    pub header: MsObjHeader,
    pub name_ptr: *const u8,
    pub name_len: u32,
    pub func: NativeFn,
}

impl MsNativeFunction {
    /// 读取函数名。`name_ptr`/`name_len` 由 `alloc_native_function` 从合法 UTF-8 设置。
    pub fn name(&self) -> &str {
        // SAFETY：name_ptr/name_len 由 alloc_native_function 从 Box<[u8]>（合法 UTF-8）设置，
        // 对象在调用期间保持有效。
        unsafe {
            let slice = std::slice::from_raw_parts(self.name_ptr, self.name_len as usize);
            std::str::from_utf8_unchecked(slice)
        }
    }
}

/// 分配 NativeFunction 堆对象，返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_native_function(native: NativeFunction) -> Object {
    let name_bytes = native.name.as_bytes();
    let name_box: Box<[u8]> = Box::from(name_bytes);
    let name_len = name_box.len() as u32;
    let name_ptr = Box::into_raw(name_box) as *const u8;

    let ms_fn = Box::new(MsNativeFunction {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::FUNCTION as u8,
            size: std::mem::size_of::<MsNativeFunction>() as u16,
            _padding: 0,
            class_ptr: 0,
        },
        name_ptr,
        name_len,
        func: native.func,
    });
    Object::Ref(Box::into_raw(ms_fn) as *mut MsObjHeader)
}

/// 读取 NativeFunction 堆对象（alloc_native_function 的对偶）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_native_function` 分配的、在 `'a` 期间保持有效的
/// `MsNativeFunction`。不得嵌套调用（借用约束）。
pub unsafe fn read_native_function<'a>(ptr: *mut MsObjHeader) -> &'a MsNativeFunction {
    &*(ptr as *mut MsNativeFunction)
}

impl VM {
    /// 注册全部内置函数到全局变量表。
    ///
    /// arity（参数个数）不存入堆对象，而是记入独立的 `native_arities` 表，
    /// 供 CALL 处理器校验参数个数（`usize::MAX` 表示可变参数）。
    pub fn register_builtins(&mut self) {
        let builtins: Vec<(&str, usize, NativeFn)> = vec![
            ("print", usize::MAX, builtin_print),
            ("println", usize::MAX, builtin_println),
            ("type", 1, builtin_type),
            ("len", 1, builtin_len),
            // 类型转换（10-builtins.md § 类型转换 完整 8 个）
            ("int", 1, builtin_int),
            ("float", 1, builtin_float),
            ("str", 1, builtin_str),
            ("bool", 1, builtin_bool),
            ("list", 1, builtin_list),
            ("tuple", 1, builtin_tuple),
            ("set", 1, builtin_set),
            ("dict", 1, builtin_dict),
            // 数学
            ("abs", 1, builtin_abs),
            ("max", usize::MAX, builtin_max),
            ("min", usize::MAX, builtin_min),
            ("sum", 1, builtin_sum),
            ("ceil", 1, builtin_ceil),
            ("floor", 1, builtin_floor),
            ("round", usize::MAX, builtin_round),
            // 类型检查
            ("isinstance", 2, builtin_isinstance),
            ("assert", usize::MAX, builtin_assert),
            // 其他全局内置（参照 10-builtins.md）
            ("input", usize::MAX, builtin_input),
            ("id", 1, builtin_id),
            ("hash", 1, builtin_hash),
            ("copy", 1, builtin_copy),
            ("range", usize::MAX, builtin_range),
            // 占位：依赖后续 task，MVP 返回 Err（见各自实现）
            ("open", usize::MAX, builtin_open), // task 46（stdlib-io）
            ("deepcopy", 1, builtin_deepcopy),  // task 22 扩展 / task 26
        ];

        for (name, arity, func) in builtins {
            let native_fn = NativeFunction {
                name: name.to_string(),
                func,
            };
            // 注：转换函数 int/float/bool/str/list/tuple/set/dict 同时充当内置类型
            // 对象（isinstance 第二参数），从函数名读取类型名——避免 globals 表中
            // 「函数」与「类型常量」同键冲突。task 40 升级为完整 Class 对象。
            self.globals
                .insert(name.to_string(), alloc_native_function(native_fn));
            self.native_arities.insert(name.to_string(), arity);
        }
    }
}

// ---------------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------------

fn builtin_print(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let output: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
    println!("{}", output.join(" "));
    Ok(Object::Nil)
}

/// `println` 是 `print` 的别名（行为完全一致）。
fn builtin_println(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    builtin_print(vm, args)
}

// ---------------------------------------------------------------------------
// 类型检查
// ---------------------------------------------------------------------------

fn builtin_type(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("type() requires 1 argument")?;
    Ok(alloc_string(arg.type_name()))
}

fn builtin_len(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("len() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            let len = if tag == TypeTag::STRING as u8 {
                unsafe { read_str(*ptr) }.len()
            } else if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.len()
            } else if tag == TypeTag::DICT as u8 {
                unsafe { read_dict(*ptr) }.len()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.len()
            } else if tag == TypeTag::SET as u8 {
                unsafe { read_set(*ptr) }.len()
            } else {
                return Err(format!(
                    "TypeError: object of type '{}' has no len()",
                    arg.type_name()
                ));
            };
            Ok(Object::Int(len as i64))
        }
        _ => Err(format!(
            "TypeError: object of type '{}' has no len()",
            arg.type_name()
        )),
    }
}

// ---------------------------------------------------------------------------
// 类型转换
// ---------------------------------------------------------------------------

fn builtin_int(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("int() requires 1 argument")?;
    arg.to_int()
}

fn builtin_float(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("float() requires 1 argument")?;
    arg.to_float()
}

fn builtin_str(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("str() requires 1 argument")?;
    Ok(arg.to_str())
}

fn builtin_bool(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("bool() requires 1 argument")?;
    Ok(arg.to_bool())
}

// 集合转换。迭代器统一协议在 task 26/32；此处对已实现的集合类型做直接转换。
fn builtin_list(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("list() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // list("abc") -> ["a","b","c"]
                let chars: Vec<Object> = unsafe { read_str(*ptr) }
                    .chars()
                    .map(|c| alloc_string(c.to_string().as_str()))
                    .collect();
                Ok(alloc_list(chars))
            } else if tag == TypeTag::LIST as u8 {
                Ok(alloc_list(unsafe { read_list(*ptr) }.clone()))
            } else if tag == TypeTag::TUPLE as u8 {
                Ok(alloc_list(unsafe { read_tuple(*ptr) }.clone()))
            } else if tag == TypeTag::SET as u8 {
                Ok(alloc_list(
                    unsafe { read_set(*ptr) }.iter().cloned().collect(),
                ))
            } else {
                Err(format!(
                    "TypeError: '{}' object is not iterable",
                    arg.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            arg.type_name()
        )),
    }
}

fn builtin_tuple(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("tuple() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                Ok(alloc_tuple(unsafe { read_list(*ptr) }.clone()))
            } else if tag == TypeTag::TUPLE as u8 {
                Ok(alloc_tuple(unsafe { read_tuple(*ptr) }.clone()))
            } else {
                Err(format!(
                    "TypeError: '{}' object is not iterable",
                    arg.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            arg.type_name()
        )),
    }
}

fn builtin_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("set() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            let items: Vec<Object> = if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.clone()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.clone()
            } else {
                return Err(format!(
                    "TypeError: '{}' object is not iterable",
                    arg.type_name()
                ));
            };
            Ok(alloc_set(items.into_iter().collect()))
        }
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            arg.type_name()
        )),
    }
}

fn builtin_dict(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("dict() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            Ok(alloc_dict(unsafe { read_dict(*ptr) }.clone()))
        }
        // 从二元 tuple 列表构造 dict 的完整支持依赖 task 26 迭代器协议；MVP 仅支持 dict→dict 拷贝。
        _ => Err(format!(
            "TypeError: cannot convert '{}' to dict (MVP: only dict supported)",
            arg.type_name()
        )),
    }
}

// ---------------------------------------------------------------------------
// 数学
// ---------------------------------------------------------------------------

fn builtin_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("abs() requires 1 argument")?;
    match arg {
        Object::Int(n) => Ok(Object::Int(n.abs())),
        Object::Float(n) => Ok(Object::Float(n.abs())),
        _ => Err(format!(
            "TypeError: bad operand type for abs(): '{}'",
            arg.type_name()
        )),
    }
}

fn builtin_max(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("max() requires at least 1 argument".to_string());
    }
    let mut result = args[0].clone();
    for arg in &args[1..] {
        // CmpOp 与 OpCode 解耦（task 21，object.rs）。
        if let Object::Bool(true) = result.compare(arg, CmpOp::Less)? {
            result = arg.clone();
        }
    }
    Ok(result)
}

fn builtin_min(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("min() requires at least 1 argument".to_string());
    }
    let mut result = args[0].clone();
    for arg in &args[1..] {
        if let Object::Bool(true) = result.compare(arg, CmpOp::Greater)? {
            result = arg.clone();
        }
    }
    Ok(result)
}

fn builtin_sum(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("sum() requires 1 argument")?;
    // 支持可迭代集合：List / Tuple / Set（完整迭代器协议待 task 26/32）。
    let items: Vec<Object> = match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.clone()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.clone()
            } else if tag == TypeTag::SET as u8 {
                unsafe { read_set(*ptr) }.iter().cloned().collect()
            } else {
                return Err(format!(
                    "TypeError: '{}' object is not iterable",
                    arg.type_name()
                ));
            }
        }
        _ => {
            return Err(format!(
                "TypeError: '{}' object is not iterable",
                arg.type_name()
            ))
        }
    };
    let mut total = Object::Int(0);
    for item in items.iter() {
        total = total.add(item)?;
    }
    Ok(total)
}

fn builtin_ceil(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("ceil() requires 1 argument")?;
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => Ok(Object::Int(n.ceil() as i64)),
        _ => Err(format!(
            "TypeError: bad operand type for ceil(): '{}'",
            arg.type_name()
        )),
    }
}

fn builtin_floor(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("floor() requires 1 argument")?;
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => Ok(Object::Int(n.floor() as i64)),
        _ => Err(format!(
            "TypeError: bad operand type for floor(): '{}'",
            arg.type_name()
        )),
    }
}

fn builtin_round(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("round() requires at least 1 argument")?;
    let digits = if args.len() > 1 {
        match &args[1] {
            Object::Int(d) => *d as i32,
            _ => return Err("round(): digits must be int".to_string()),
        }
    } else {
        0
    };
    // digits 范围校验（D3）：防止 powi 溢出 / 除零
    if !(0..=15).contains(&digits) {
        return Err(format!(
            "ValueError: round() digits must be in 0..=15, got {}",
            digits
        ));
    }
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => {
            let factor = 10f64.powi(digits);
            // Rust f64::round 为 round-half-away-from-zero（2.5→3）。
            Ok(Object::Float((n * factor).round() / factor))
        }
        _ => Err(format!(
            "TypeError: bad operand type for round(): '{}'",
            arg.type_name()
        )),
    }
}

// ---------------------------------------------------------------------------
// 类型检查函数
// ---------------------------------------------------------------------------

fn builtin_isinstance(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("isinstance() requires 2 arguments")?;
    let type_obj = args.get(1).ok_or("isinstance() requires 2 arguments")?;

    // 提取期望的类型名。MVP（task 25）：内置类型由对应的转换/构造函数充当类型对象
    // （int/str/list/...），从原生函数名读取；字符串字面量亦支持。
    // task 40 升级为 Class 对象后追加 TypeTag::CLASS 分支读取 class.name。
    let expected_type_name: String = match type_obj {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::FUNCTION as u8 {
                // SAFETY: type_tag 为 FUNCTION，指针由 alloc_native_function 分配。
                let name = unsafe { read_native_function(*ptr) }.name().to_owned();
                // 转换函数名 `str` 对应类型名 `string`
                if name == "str" {
                    "string".to_owned()
                } else {
                    name
                }
            } else if tag == TypeTag::STRING as u8 {
                unsafe { read_str(*ptr) }.to_owned()
            } else {
                return Err("isinstance(): second argument must be a type".to_string());
            }
        }
        _ => return Err("isinstance(): second argument must be a type".to_string()),
    };

    // INSTANCE 继承链匹配由 task 40/41 实现；MVP 仅比较 type_name。
    Ok(Object::Bool(val.type_name() == expected_type_name.as_str()))
}

// ---------------------------------------------------------------------------
// 断言
// ---------------------------------------------------------------------------

fn builtin_assert(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let cond = args.get(0).ok_or("assert() requires at least 1 argument")?;
    if !cond.is_truthy() {
        let msg = if args.len() > 1 {
            format!("{}", args[1])
        } else {
            "AssertionError".to_string()
        };
        return Err(format!("AssertionError: {}", msg));
    }
    Ok(Object::Nil)
}

// ---------------------------------------------------------------------------
// 其他全局内置（参照 10-builtins.md）
// ---------------------------------------------------------------------------

/// open(path, mode?) -> File。占位：真实实现由 task 46（stdlib-io）。
fn builtin_open(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Err("not yet implemented: open() (task 46 stdlib-io)".to_string())
}

/// input(prompt?) -> string。
fn builtin_input(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if let Some(prompt) = args.get(0) {
        print!("{}", prompt);
    }
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("IOError: {}", e))?;
    let line = line.trim_end_matches('\n').trim_end_matches('\r');
    Ok(alloc_string(line))
}

/// id(val) -> int。引用类型返回堆地址；内联值用值本身标识。
fn builtin_id(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("id() requires 1 argument")?;
    match val {
        Object::Ref(ptr) => Ok(Object::Int(*ptr as u64 as i64)),
        Object::Int(n) => Ok(Object::Int(*n)),
        Object::Float(f) => Ok(Object::Int(f.to_bits() as i64)),
        Object::Bool(b) => Ok(Object::Int(*b as i64)),
        Object::Nil => Ok(Object::Int(0)),
    }
}

/// hash(val) -> int。List/Dict/Set/NaN 不可哈希，返回 Err（避免宿主 panic）。
fn builtin_hash(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let val = args.get(0).ok_or("hash() requires 1 argument")?;
    if let Object::Ref(ptr) = val {
        debug_assert!(!ptr.is_null(), "null Object::Ref");
        let tag = unsafe { (**ptr).type_tag };
        if tag == TypeTag::LIST as u8 || tag == TypeTag::DICT as u8 || tag == TypeTag::SET as u8 {
            return Err(format!("TypeError: unhashable type: '{}'", val.type_name()));
        }
    }
    if let Object::Float(f) = val {
        if f.is_nan() {
            return Err("TypeError: unhashable type: 'float' (NaN)".to_string());
        }
    }
    let mut hasher = DefaultHasher::new();
    val.hash(&mut hasher);
    Ok(Object::Int(hasher.finish() as i64))
}

/// copy(val) -> 浅拷贝。
fn builtin_copy(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("copy() requires 1 argument")?;
    match val {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                let items = unsafe { read_list(*ptr) }.clone();
                Ok(alloc_list(items))
            } else if tag == TypeTag::DICT as u8 {
                let pairs = unsafe { read_dict(*ptr) }.clone();
                Ok(alloc_dict(pairs))
            } else {
                Ok(val.clone()) // 不可变类型直接返回
            }
        }
        _ => Ok(val.clone()),
    }
}

/// deepcopy(val) -> 深拷贝。MVP 占位：递归深拷贝由 task 22 扩展 / task 26 实现。
fn builtin_deepcopy(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Err("not yet implemented: deepcopy() (task 22 extension / task 26)".to_string())
}

/// range(start, stop?, step?) -> List。MVP 返回 List；task 32 升级为惰性迭代器。
fn builtin_range(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let (start, stop, step) = match args.len() {
        1 => (0, require_int(&args[0])?, 1),
        2 => (require_int(&args[0])?, require_int(&args[1])?, 1),
        3 => (
            require_int(&args[0])?,
            require_int(&args[1])?,
            require_int(&args[2])?,
        ),
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    if step == 0 {
        return Err("ValueError: range() step argument must not be zero".to_string());
    }
    let mut items = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            items.push(Object::Int(i));
            i += step;
        }
    } else {
        while i > stop {
            items.push(Object::Int(i));
            i += step;
        }
    }
    Ok(alloc_list(items))
}

/// 辅助：要求整型参数，返回 i64。
fn require_int(arg: &Object) -> Result<i64, String> {
    match arg {
        Object::Int(n) => Ok(*n),
        _ => Err(format!(
            "TypeError: '{}' object cannot be interpreted as an integer",
            arg.type_name()
        )),
    }
}

#[cfg(test)]
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::vm::object::DictMap;
    use std::collections::HashSet;

    fn vm() -> VM {
        // register_builtins 已在 new() 中调用。
        VM::new()
    }

    fn s(v: &str) -> Object {
        alloc_string(v)
    }

    #[test]
    fn test_type_name_inline_and_refs() {
        // B3 扩展：type() 须对集合类型返回正确名称（而非 "object"）
        let mut v = vm();
        assert_eq!(builtin_type(&mut v, &[Object::Int(42)]).unwrap(), s("int"));
        assert_eq!(builtin_type(&mut v, &[s("hi")]).unwrap(), s("string"));
        assert_eq!(
            builtin_type(&mut v, &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            s("list")
        );
        assert_eq!(
            builtin_type(&mut v, &[alloc_tuple(vec![Object::Int(1)])]).unwrap(),
            s("tuple")
        );
        assert_eq!(
            builtin_type(&mut v, &[alloc_set(HashSet::new())]).unwrap(),
            s("set")
        );
    }

    #[test]
    fn test_len_collections() {
        let mut v = vm();
        assert_eq!(builtin_len(&mut v, &[s("hello")]).unwrap(), Object::Int(5));
        assert_eq!(
            builtin_len(
                &mut v,
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(3)
        );
        assert!(builtin_len(&mut v, &[Object::Int(5)]).is_err()); // TypeError
    }

    #[test]
    fn test_conversions() {
        let mut v = vm();
        assert_eq!(builtin_int(&mut v, &[s("42")]).unwrap(), Object::Int(42));
        assert_eq!(
            builtin_float(&mut v, &[s("3.14")]).unwrap(),
            Object::Float(3.14)
        );
        assert_eq!(builtin_str(&mut v, &[Object::Int(42)]).unwrap(), s("42"));
        assert_eq!(
            builtin_bool(&mut v, &[Object::Int(0)]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            builtin_bool(&mut v, &[Object::Int(7)]).unwrap(),
            Object::Bool(true)
        );
    }

    #[test]
    fn test_collection_conversions() {
        let mut v = vm();
        // list("abc") -> ["a","b","c"]
        assert_eq!(
            builtin_list(&mut v, &[s("abc")]).unwrap(),
            alloc_list(vec![s("a"), s("b"), s("c")])
        );
        // tuple([1,2]) -> (1,2)
        assert_eq!(
            builtin_tuple(&mut v, &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_tuple(vec![Object::Int(1), Object::Int(2)])
        );
        // set([1,2,2]) -> {1,2}
        let set_obj = builtin_set(
            &mut v,
            &[alloc_list(vec![
                Object::Int(1),
                Object::Int(2),
                Object::Int(2),
            ])],
        )
        .unwrap();
        let Object::Ref(ptr) = &set_obj else {
            panic!("expected Ref");
        };
        let inner = unsafe { read_set(*ptr) };
        assert_eq!(inner.len(), 2);
        assert!(inner.contains(&Object::Int(1)));
        assert!(inner.contains(&Object::Int(2)));
    }

    #[test]
    fn test_math() {
        let mut v = vm();
        assert_eq!(
            builtin_abs(&mut v, &[Object::Int(-5)]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            builtin_abs(&mut v, &[Object::Float(-3.5)]).unwrap(),
            Object::Float(3.5)
        );
        assert_eq!(
            builtin_max(&mut v, &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(3)
        );
        assert_eq!(
            builtin_min(&mut v, &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            builtin_sum(
                &mut v,
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(6)
        );
        assert_eq!(
            builtin_ceil(&mut v, &[Object::Float(3.2)]).unwrap(),
            Object::Int(4)
        );
        assert_eq!(
            builtin_floor(&mut v, &[Object::Float(3.7)]).unwrap(),
            Object::Int(3)
        );
    }

    #[test]
    fn test_round() {
        let mut v = vm();
        // round(3.5) -> 4.0
        assert_eq!(
            builtin_round(&mut v, &[Object::Float(3.5)]).unwrap(),
            Object::Float(4.0)
        );
        // round(3.14159, 2) -> 3.14
        assert_eq!(
            builtin_round(&mut v, &[Object::Float(3.14159), Object::Int(2)]).unwrap(),
            Object::Float(3.14)
        );
        // round(x, 20) -> ValueError（digits 越界，D3）
        assert!(builtin_round(&mut v, &[Object::Float(1.0), Object::Int(20)]).is_err());
    }

    #[test]
    fn test_isinstance() {
        let mut v = vm();
        // isinstance(42, int) -> true；int 为内置转换函数充当类型对象
        assert_eq!(
            builtin_isinstance(&mut v, &[Object::Int(42), s("int")]).unwrap(),
            Object::Bool(true)
        );
        // isinstance("hi", int) -> false
        assert_eq!(
            builtin_isinstance(&mut v, &[s("hi"), s("int")]).unwrap(),
            Object::Bool(false)
        );
        // isinstance("hi", string) -> true（类型名匹配）
        assert_eq!(
            builtin_isinstance(&mut v, &[s("hi"), s("string")]).unwrap(),
            Object::Bool(true)
        );
        // 非类型参数 -> Err
        assert!(builtin_isinstance(&mut v, &[Object::Int(42), Object::Int(1)]).is_err());
    }

    #[test]
    fn test_assert() {
        let mut v = vm();
        assert_eq!(
            builtin_assert(&mut v, &[Object::Int(1)]).unwrap(),
            Object::Nil
        );
        let r = builtin_assert(&mut v, &[Object::Int(0)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("AssertionError"));
    }

    #[test]
    fn test_range() {
        let mut v = vm();
        // range(5) -> [0,1,2,3,4]
        assert_eq!(
            builtin_range(&mut v, &[Object::Int(5)]).unwrap(),
            alloc_list(vec![
                Object::Int(0),
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4)
            ])
        );
        // range(2, 8, 2) -> [2,4,6]
        assert_eq!(
            builtin_range(&mut v, &[Object::Int(2), Object::Int(8), Object::Int(2)]).unwrap(),
            alloc_list(vec![Object::Int(2), Object::Int(4), Object::Int(6)])
        );
        // range(3, 0, -1) -> [3,2,1]
        assert_eq!(
            builtin_range(&mut v, &[Object::Int(3), Object::Int(0), Object::Int(-1)]).unwrap(),
            alloc_list(vec![Object::Int(3), Object::Int(2), Object::Int(1)])
        );
        // step == 0 -> ValueError
        assert!(builtin_range(&mut v, &[Object::Int(0), Object::Int(5), Object::Int(0)]).is_err());
    }

    #[test]
    fn test_id_and_hash() {
        let mut v = vm();
        // id 对内联返回值本身标识
        assert_eq!(
            builtin_id(&mut v, &[Object::Int(42)]).unwrap(),
            Object::Int(42)
        );
        assert_eq!(builtin_id(&mut v, &[Object::Nil]).unwrap(), Object::Int(0));
        // hash("key") 返回稳定哈希
        let h1 = builtin_hash(&mut v, &[s("key")]).unwrap();
        let h2 = builtin_hash(&mut v, &[s("key")]).unwrap();
        assert_eq!(h1, h2);
        // hash([1,2]) -> TypeError（unhashable，C3）
        let r = builtin_hash(&mut v, &[alloc_list(vec![Object::Int(1), Object::Int(2)])]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_copy_and_placeholders() {
        let mut v = vm();
        // copy 浅拷贝 list（返回内容相等的新对象）
        let src = alloc_list(vec![Object::Int(1), Object::Int(2)]);
        let cpy = builtin_copy(&mut v, std::slice::from_ref(&src)).unwrap();
        assert_eq!(cpy, src);
        // copy 对不可变类型直接返回（string）
        assert_eq!(builtin_copy(&mut v, &[s("x")]).unwrap(), s("x"));
        // dict 浅拷贝
        let mut m = DictMap::new();
        m.insert(s("a"), Object::Int(1));
        let d = alloc_dict(m);
        assert_eq!(builtin_copy(&mut v, std::slice::from_ref(&d)).unwrap(), d);
        // 占位：open / deepcopy 返回 Err
        assert!(builtin_open(&mut v, &[]).is_err());
        assert!(builtin_deepcopy(&mut v, &[Object::Int(1)]).is_err());
    }

    #[test]
    fn test_native_function_name_roundtrip() {
        // alloc_native_function -> read_native_function: name/type_tag 可读
        let obj = alloc_native_function(NativeFunction {
            name: "print".to_string(),
            func: builtin_print,
        });
        let Object::Ref(ptr) = obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::FUNCTION as u8);
            assert_eq!(read_native_function(ptr).name(), "print");
        }
    }
}
