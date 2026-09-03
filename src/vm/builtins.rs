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
    alloc_class, alloc_dict, alloc_iterator, alloc_list, alloc_set, alloc_string, alloc_tuple,
    read_class, read_dict, read_instance, read_iterator, read_list, read_set, read_str, read_tuple,
    CmpOp, DictMap, IteratorState, MsObjHeader, Object, TypeTag,
};
use super::VM;
use std::collections::HashSet;

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

// ---------------------------------------------------------------------------
// C 原生函数堆对象（task 70）
// ---------------------------------------------------------------------------

#[cfg(feature = "capi")]
use crate::capi::types::MsCFunction;

/// C 原生函数堆对象（TypeTag::NATIVE_C_FUNCTION）。
/// 与 `MsNativeFunction` 分离：C 函数签名不兼容 Rust `NativeFn`。
#[repr(C)]
pub struct MsCNativeFunction {
    pub header: MsObjHeader,
    pub name_ptr: *const u8,
    pub name_len: u32,
    #[cfg(feature = "capi")]
    pub func: MsCFunction,
    pub arity: i32,
}

#[cfg(feature = "capi")]
impl MsCNativeFunction {
    pub fn name(&self) -> &str {
        unsafe {
            let slice = std::slice::from_raw_parts(self.name_ptr, self.name_len as usize);
            std::str::from_utf8_unchecked(slice)
        }
    }
}

/// 分配 MsCNativeFunction 堆对象，返回 Object::Ref。
#[cfg(feature = "capi")]
pub fn alloc_c_native_function(name: &str, func: MsCFunction, arity: i32) -> Object {
    let name_bytes = name.as_bytes();
    let name_box: Box<[u8]> = Box::from(name_bytes);
    let name_len = name_box.len() as u32;
    let name_ptr = Box::into_raw(name_box) as *const u8;

    let obj = Box::new(MsCNativeFunction {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::NATIVE_C_FUNCTION as u8,
            size: std::mem::size_of::<MsCNativeFunction>() as u16,
            _padding: 0,
            class_ptr: 0,
        },
        name_ptr,
        name_len,
        func,
        arity,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsCNativeFunction 堆对象（alloc_c_native_function 的对偶）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_c_native_function` 分配的、在 `'a` 期间保持有效的
/// `MsCNativeFunction`。
#[cfg(feature = "capi")]
pub unsafe fn read_c_native_function<'a>(ptr: *mut MsObjHeader) -> &'a MsCNativeFunction {
    &*(ptr as *mut MsCNativeFunction)
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
            ("list", usize::MAX, builtin_list),
            ("tuple", usize::MAX, builtin_tuple),
            ("set", usize::MAX, builtin_set),
            ("dict", usize::MAX, builtin_dict),
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
            // 迭代器与容器函数（task 26）：range 覆盖 task 25 的 List 版本为迭代器；
            // sorted/reversed/enumerate/zip/map/filter/any/all 为新增。
            ("range", usize::MAX, builtin_range),
            // task 80：sorted 扩展 (iterable, key?, reverse?)；sorted_by 为 key 显式版。
            // 两者均 MAX（native 内自校验 1-3 / 2-3 参）。
            ("sorted", usize::MAX, builtin_sorted),
            ("sorted_by", usize::MAX, builtin_sorted_by),
            ("reversed", 1, builtin_reversed),
            ("enumerate", 1, builtin_enumerate),
            ("zip", usize::MAX, builtin_zip),
            ("map", 2, builtin_map),
            ("filter", 2, builtin_filter),
            ("any", 1, builtin_any),
            ("all", 1, builtin_all),
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

    /// task 42：初始化隐式 Object 基类。注入原生 __repr__/__eq__/__ne__ 方法，
    /// 标记 Immortal 代，存入 self.object_class。无显式父类的类自动继承之。
    pub fn init_object_class(&mut self) {
        let object_obj = alloc_class("Object".to_string());
        let Object::Ref(object_ptr) = object_obj else {
            unreachable!()
        };
        let Object::Ref(repr_ptr) = alloc_native_function(NativeFunction {
            name: "__repr__".to_string(),
            func: object_repr,
        }) else {
            unreachable!()
        };
        let Object::Ref(eq_ptr) = alloc_native_function(NativeFunction {
            name: "__eq__".to_string(),
            func: object_eq,
        }) else {
            unreachable!()
        };
        let Object::Ref(ne_ptr) = alloc_native_function(NativeFunction {
            name: "__ne__".to_string(),
            func: object_ne,
        }) else {
            unreachable!()
        };
        unsafe {
            (*object_ptr).set_generation(crate::vm::gc::Generation::Immortal);
            read_class(object_ptr).methods.insert("__repr__".to_string(), repr_ptr);
            read_class(object_ptr).methods.insert("__eq__".to_string(), eq_ptr);
            read_class(object_ptr).methods.insert("__ne__".to_string(), ne_ptr);
        }
        self.object_class = object_ptr;
    }
}

// ---------------------------------------------------------------------------
// Object 基类原生方法（task 42）
// ---------------------------------------------------------------------------

/// Object.__repr__(self) → "{ClassName} instance"。
fn object_repr(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let self_obj = args.get(0).ok_or("__repr__ requires self")?;
    if let Object::Ref(ptr) = self_obj {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            let name = unsafe { read_class(class_ptr) }.name.clone();
            return Ok(alloc_string(&format!("{} instance", name)));
        }
    }
    Ok(alloc_string("Object instance"))
}

/// Object.__eq__(self, other) → self is other。
fn object_eq(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let self_obj = args.get(0).ok_or("__eq__ requires self")?;
    let other = args.get(1).ok_or("__eq__ requires 2 arguments")?;
    self_obj.is_identity(other)
}

/// Object.__ne__(self, other) → not (self is other)。
fn object_ne(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let self_obj = args.get(0).ok_or("__ne__ requires self")?;
    let other = args.get(1).ok_or("__ne__ requires 2 arguments")?;
    match self_obj.is_identity(other)? {
        Object::Bool(b) => Ok(Object::Bool(!b)),
        _ => Ok(Object::Bool(true)),
    }
}

// ---------------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------------

fn builtin_print(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let mut output: Vec<String> = Vec::with_capacity(args.len());
    for a in args {
        output.push(object_to_string(vm, a)?);
    }
    let line = format!("{}\n", output.join(" "));

    #[cfg(feature = "capi")]
    {
        if let Some(cb_ptr) = vm.stdout_writer {
            if !cb_ptr.is_null() {
                // SAFETY: cb_ptr 指向 capi::vm::WriteCallback（由 msSetStdout 设置），
                // MsVM 经 Box 分配地址稳定，回调在 VM 锁保护下访问。
                let cb = unsafe {
                    &*(cb_ptr as *const crate::capi::vm::WriteCallback)
                };
                if let Some(fn_ptr) = cb.fn_ptr {
                    fn_ptr(line.as_ptr() as *const i8, line.len(), cb.userdata);
                }
                return Ok(Object::Nil);
            }
        }
    }

    print!("{}", line);
    Ok(Object::Nil)
}

/// `println` 是 `print` 的别名（行为完全一致）。
fn builtin_println(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    builtin_print(vm, args)
}

/// task 40 §10：将任意 Object 转为显示字符串。
/// Instance：优先 `__str__`（task 43），次 `__repr__`，最后默认 `<ClassName instance>`。
/// 调用 `__repr__` 需运行闭包，故需 `&mut VM`（经 invoke_method 驱动子帧至返回）。
pub(crate) fn object_to_string(vm: &mut VM, obj: &Object) -> Result<String, String> {
    if let Object::Ref(ptr) = obj {
        debug_assert!(!ptr.is_null(), "null Object::Ref");
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let inst_ptr = *ptr;
            let class_ptr = unsafe { read_instance(inst_ptr) }.class;
            let (str_ptr_opt, repr_ptr_opt, name) = {
                let c = unsafe { read_class(class_ptr) };
                let s = unsafe { c.find_method("__str__") };
                let r = unsafe { c.find_method("__repr__") };
                (s, r, c.name.clone())
            };
            // task 43 §3：__str__ 优先于 __repr__，返回值须为 String。
            if let Some(str_ptr) = str_ptr_opt {
                let result = vm.invoke_method(str_ptr, obj.clone(), &[])?;
                return rust_string(&result, "__str__");
            }
            if let Some(repr_ptr) = repr_ptr_opt {
                let result = vm.invoke_method(repr_ptr, obj.clone(), &[])?;
                return rust_string(&result, "__repr__");
            }
            return Ok(format!("<{} instance>", name));
        }
        // 容器内嵌套 Instance：递归经本函数渲染（fmt::Display 无 VM 访问，
        // 嵌套实例会显示 <object:N>）；标量路径结果与 Display 一致。
        let tag = unsafe { (**ptr).type_tag };
        if tag == TypeTag::LIST as u8 {
            let items = unsafe { read_list(*ptr) };
            let parts: Vec<String> = items
                .iter()
                .map(|it| object_to_string(vm, it))
                .collect::<Result<_, _>>()?;
            return Ok(format!("[{}]", parts.join(", ")));
        }
        if tag == TypeTag::TUPLE as u8 {
            let items = unsafe { read_tuple(*ptr) };
            let parts: Vec<String> = items
                .iter()
                .map(|it| object_to_string(vm, it))
                .collect::<Result<_, _>>()?;
            if parts.len() == 1 {
                return Ok(format!("({},)", parts[0]));
            }
            return Ok(format!("({})", parts.join(", ")));
        }
        if tag == TypeTag::DICT as u8 {
            let map = unsafe { read_dict(*ptr) };
            let parts: Vec<String> = map
                .items()
                .iter()
                .map(|(k, v)| {
                    Ok::<String, String>(format!("{}: {}", k, object_to_string(vm, v)?))
                })
                .collect::<Result<_, _>>()?;
            return Ok(format!("{{{}}}", parts.join(", ")));
        }
        if tag == TypeTag::SET as u8 {
            let set = unsafe { read_set(*ptr) };
            let parts: Vec<String> = set
                .iter()
                .map(|it| object_to_string(vm, it))
                .collect::<Result<_, _>>()?;
            return Ok(format!("{{{}}}", parts.join(", ")));
        }
    }
    Ok(format!("{}", obj))
}

/// task 43 §3：从预期为 String 的 Object 提取 Rust String；非 String 报错。
/// `method_name` 用于错误信息（如 "__str__ must return a string"）。
fn rust_string(obj: &Object, method_name: &str) -> Result<String, String> {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            Ok(unsafe { read_str(*ptr) }.to_owned())
        }
        _ => Err(format!("{} must return a string", method_name)),
    }
}

// ---------------------------------------------------------------------------
// 类型检查
// ---------------------------------------------------------------------------

fn builtin_type(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("type() requires 1 argument")?;
    // task 42：INSTANCE 返回动态类名（非 "instance"），供 Object.__repr__ 等使用。
    if let Object::Ref(ptr) = arg {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            let name = unsafe { read_class(class_ptr) }.name.clone();
            return Ok(alloc_string(&name));
        }
    }
    Ok(alloc_string(arg.type_name()))
}

fn builtin_len(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("len() requires 1 argument")?;
    // task 43 §9：Instance 有 __len__ 时分派（沿继承链），返回值须为 Int。
    if let Object::Ref(ptr) = arg {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let len_ptr = unsafe {
                let class_ptr = read_instance(*ptr).class;
                read_class(class_ptr).find_method("__len__")
            };
            let len_ptr = len_ptr.ok_or_else(|| {
                format!("TypeError: object of type '{}' has no len()", arg.type_name())
            })?;
            let result = vm.invoke_method(len_ptr, arg.clone(), &[])?;
            return match &result {
                Object::Int(n) => Ok(Object::Int(*n)),
                _ => Err("__len__() should return an int".to_string()),
            };
        }
    }
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

fn builtin_str(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("str() requires 1 argument")?;
    Ok(alloc_string(&object_to_string(vm, arg)?))
}

fn builtin_bool(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("bool() requires 1 argument")?;
    Ok(arg.to_bool())
}

// ---------------------------------------------------------------------------
// 迭代器协议（task 26）
// ---------------------------------------------------------------------------

/// 将任意可迭代对象转为 `IteratorState`。
///
/// 支持 LIST / TUPLE / STRING / DICT（键） / SET / ITERATOR（克隆状态）。
/// 性能说明：对 list/tuple/dict/set 做整表克隆入 `IteratorState`（大集合下内存翻倍）。
/// MVP 接受此开销以换取实现简洁与 `FOR_ITER` 无别名安全。
pub(crate) fn to_iterator(obj: &Object) -> Result<IteratorState, String> {
    match obj {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            // SAFETY: 调用方保证 Ref 指针指向有效 MsObjHeader。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_list(*ptr) }.clone(),
                    index: 0,
                })
            } else if tag == TypeTag::TUPLE as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_tuple(*ptr) }.clone(),
                    index: 0,
                })
            } else if tag == TypeTag::STRING as u8 {
                Ok(IteratorState::StringIter {
                    chars: unsafe { read_str(*ptr) }.chars().collect(),
                    index: 0,
                })
            } else if tag == TypeTag::DICT as u8 {
                Ok(IteratorState::DictKeys {
                    keys: unsafe { read_dict(*ptr) }
                        .keys()
                        .into_iter()
                        .cloned()
                        .collect(),
                    index: 0,
                })
            } else if tag == TypeTag::SET as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_set(*ptr) }.iter().cloned().collect(),
                    index: 0,
                })
            } else if tag == TypeTag::ITERATOR as u8 {
                Ok(unsafe { read_iterator(*ptr) }.state.clone())
            } else {
                Err(format!(
                    "TypeError: '{}' object is not iterable",
                    obj.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            obj.type_name()
        )),
    }
}

/// 将任意可迭代对象消费为 `Vec<Object>`（DRY：供 list/tuple/set 构造复用）。
fn collect_iter(arg: &Object) -> Result<Vec<Object>, String> {
    let mut out = Vec::new();
    let mut it = to_iterator(arg)?;
    while let Some(v) = it.next() {
        out.push(v);
    }
    Ok(out)
}

/// `list()` 空列表；`list(iterable)` 从可迭代对象构造。
/// 覆盖 task 25 同名实现：改可变参数（0 或 1），统一走 `to_iterator`（含 SET）。
fn builtin_list(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_list(Vec::new()));
    }
    if args.len() > 1 {
        return Err("list() takes 0 or 1 arguments".to_string());
    }
    Ok(alloc_list(collect_iter(&args[0])?))
}

/// `tuple()` 空元组；`tuple(iterable)` 从可迭代对象构造。
fn builtin_tuple(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_tuple(Vec::new()));
    }
    if args.len() > 1 {
        return Err("tuple() takes 0 or 1 arguments".to_string());
    }
    Ok(alloc_tuple(collect_iter(&args[0])?))
}

/// `set()` 空集合；`set(iterable)` 从可迭代对象构造。
fn builtin_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_set(HashSet::new()));
    }
    if args.len() > 1 {
        return Err("set() takes 0 or 1 arguments".to_string());
    }
    let items = collect_iter(&args[0])?;
    Ok(alloc_set(items.into_iter().collect()))
}

/// `dict()` 空字典；`dict(d)` dict 拷贝。
/// 从 (k, v) 对可迭代对象构造（dict([(k,v),...])）依赖元组解包迭代，随 task 30
/// （多返回值与元组解包）完善，本 MVP 暂不支持并显式报错。
fn builtin_dict(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_dict(DictMap::new()));
    }
    if args.len() > 1 {
        return Err("dict() takes 0 or 1 arguments".to_string());
    }
    match &args[0] {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            Ok(alloc_dict(unsafe { read_dict(*ptr) }.clone()))
        }
        _ => Err(format!(
            "TypeError: cannot convert '{}' to dict (MVP: only dict supported)",
            args[0].type_name()
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
            } else if tag == TypeTag::CLASS as u8 {
                // task 40：用户类作为类型对象，读取类名
                unsafe { read_class(*ptr) }.name.clone()
            } else {
                return Err("isinstance(): second argument must be a type".to_string());
            }
        }
        _ => return Err("isinstance(): second argument must be a type".to_string()),
    };

    // INSTANCE 沿继承链匹配类名（task 40/41）；其余仅比较 type_name。
    if let Object::Ref(ptr) = val {
        debug_assert!(!ptr.is_null(), "null Object::Ref");
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let mut class_ptr = unsafe { read_instance(*ptr) }.class;
            loop {
                let c = unsafe { read_class(class_ptr) };
                if c.name == expected_type_name {
                    return Ok(Object::Bool(true));
                }
                match c.parent {
                    Some(p) => class_ptr = p,
                    None => break,
                }
            }
            return Ok(Object::Bool(false));
        }
    }
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

/// open(path, mode?) -> FileHandle。全局快捷方式，委托 io.open（task 46 stdlib-io）。
fn builtin_open(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    super::stdlib::native_io_open(vm, args)
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
/// task 82（§2.2 同名冲突治理）：fs.copy 加入后 native_arities 的 copy 升级
/// usize::MAX，此处自校验恰 1 参（fs.copy 自校验恰 2 参，两侧并存）。
fn builtin_copy(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 1 {
        return Err(format!(
            "TypeError: copy() takes exactly 1 argument but {} were given",
            args.len()
        ));
    }
    match &args[0] {
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
                Ok(args[0].clone()) // 不可变类型直接返回
            }
        }
        _ => Ok(args[0].clone()),
    }
}

/// deepcopy(val) -> 深拷贝。MVP 占位：递归深拷贝由 task 22 扩展 / task 26 实现。
fn builtin_deepcopy(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Err("not yet implemented: deepcopy() (task 22 extension / task 26)".to_string())
}

/// range(end) / range(start, end) / range(start, end, step) -> 迭代器。
/// task 26 升级为迭代器（覆盖 task 25 的 List 版本，符合 10-builtins.md:97-99）。
/// require_int 复用 task 25 已有实现，不重复定义。
fn builtin_range(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let (start, end, step) = match args.len() {
        1 => (0, require_int(&args[0])?, 1),
        2 => (require_int(&args[0])?, require_int(&args[1])?, 1),
        3 => {
            let step = require_int(&args[2])?;
            if step == 0 {
                return Err("ValueError: range() step must not be zero".to_string());
            }
            (require_int(&args[0])?, require_int(&args[1])?, step)
        }
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    Ok(alloc_iterator(IteratorState::Range {
        current: start,
        end,
        step,
    }))
}

// ---------------------------------------------------------------------------
// 迭代器函数（task 26）
// ---------------------------------------------------------------------------

/// 校验参数为 callable（FUNCTION/CLOSURE/BOUND_METHOD）。task 51 引入（list.rs），
/// task 80 提升至此供 sorted/sorted_by/sort/sort_by 的 key 校验共用。
pub(crate) fn expect_callable(arg: Option<&Object>, who: &str) -> Result<Object, String> {
    match arg {
        Some(o @ Object::Ref(ptr)) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
            {
                Ok(o.clone())
            } else {
                Err(format!(
                    "TypeError: {} expects callable, got {}",
                    who,
                    o.type_name()
                ))
            }
        }
        other => Err(format!(
            "TypeError: {} expects callable, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// reverse 参数校验（task 80）：仅接受 Bool。
pub(crate) fn expect_reverse(arg: Option<&Object>, who: &str) -> Result<bool, String> {
    match arg {
        Some(Object::Bool(b)) => Ok(*b),
        other => Err(format!(
            "TypeError: {} expects bool for reverse, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// key 可选参数解析（task 80）：缺省 / Nil → None（Python key=None 语义），
/// 其余须为 callable。
pub(crate) fn optional_key(arg: Option<&Object>, who: &str) -> Result<Option<Object>, String> {
    match arg {
        None | Some(Object::Nil) => Ok(None),
        Some(_) => Ok(Some(expect_callable(arg, who)?)),
    }
}

/// 比较辅助（升序）：错误置入 err 槽延迟上抛（sort_by 闭包无法返回 Result）。
/// 比较器（Less/Greater 组合，错误暂存后统一上抛）。task 84 起 heapq
/// n_largest/n_smallest 复用（同 sorted 比较语义）。
pub(crate) fn cmp_objects(a: &Object, b: &Object, err: &mut Option<String>) -> std::cmp::Ordering {
    if err.is_some() {
        return std::cmp::Ordering::Equal;
    }
    match a.compare(b, CmpOp::Less) {
        Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
        Ok(_) => match a.compare(b, CmpOp::Greater) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
            Ok(_) => std::cmp::Ordering::Equal,
            Err(e) => {
                *err = Some(e);
                std::cmp::Ordering::Equal
            }
        },
        Err(e) => {
            *err = Some(e);
            std::cmp::Ordering::Equal
        }
    }
}

/// 从根化栈槽重取 heap list 裸指针（task 80 GC 安全）。
/// call_function 重入解释器期间可能触发 GC，堆指针须从 vm.stack 根槽重取，
/// 防对象移动/回收后悬垂（16-stdlib-expansion.md §GC 安全）。
pub(crate) fn rooted_list_ptr(vm: &VM, slot: usize) -> *mut MsObjHeader {
    match &vm.stack()[slot] {
        Object::Ref(p) => *p,
        _ => unreachable!("rooted slot must hold list Ref"),
    }
}

/// DSU 中间对（tuple）元素读取：slot 0 = key，slot 1 = 原元素。
fn tuple_elem(obj: &Object, idx: usize) -> Object {
    match obj {
        Object::Ref(p) => {
            // SAFETY: DSU 对由 alloc_tuple 分配。
            let t = unsafe { read_tuple(*p) };
            t[idx].clone()
        }
        _ => unreachable!("DSU pair must be tuple Ref"),
    }
}

/// task 80：DSU 排序核心（sorted/sorted_by/list.sort/list.sort_by 共用）。
///
/// - 稳定排序（Rust `sort_by`）；reverse 反转比较器（等值元素保持原序，Python 语义）；
/// - decorate-sort-undecorate：单次 sort 每元素仅调用 key 一次；
/// - key 抛错上抛调用方（call_function 截获的异常暂存 `escaped_exc`，由 call_value
///   在 native 返回后重抛，可被调用方 try/except 捕获）；
/// - GC 安全：key 调用经 call_function 重入解释器（每条字节码前 maybe_gc），native
///   栈不在 GC 根集 —— key、源元素 heap list、DSU 中间对 heap list 均压入 vm.stack
///   根化（list 被 trace，元素随之存活）；call_function 之后一切堆指针从根槽重取。
///   （builtin_map/filter 与 list.map/filter 的未根化结果 Vec 写法存在同构隐患，
///   本 task 不修复，但新代码禁止复刻。）
pub(crate) fn sort_items_dsu(
    vm: &mut VM,
    items: Vec<Object>,
    key: Option<&Object>,
    reverse: bool,
) -> Result<Vec<Object>, String> {
    // 无 key：比较器不重入 VM，无 GC 窗口（与旧 builtin_sorted 一致，无需根化）。
    let Some(key_obj) = key else {
        let mut sorted = items;
        let mut err = None;
        sorted.sort_by(|a, b| {
            let (x, y) = if reverse { (b, a) } else { (a, b) };
            cmp_objects(x, y, &mut err)
        });
        return match err {
            Some(e) => Err(e),
            None => Ok(sorted),
        };
    };
    let root_base = vm.stack().len();
    vm.push(key_obj.clone())?; // slot root_base+0：key 根
    vm.push(alloc_list(items))?; // slot root_base+1：源元素 heap list 根
    let ret = dsu_sort_rooted(vm, root_base, reverse);
    vm.stack_mut().truncate(root_base);
    ret
}

/// DSU 路径（key 为 Some）：decorate → sort → undecorate。
/// 根布局：root_base+0 = key、root_base+1 = 源元素 list、root_base+2 = DSU 对 list。
fn dsu_sort_rooted(vm: &mut VM, root_base: usize, reverse: bool) -> Result<Vec<Object>, String> {
    // DSU 中间对放入 heap list 并压栈根化（list 被 trace，tuple 元素随之存活）。
    vm.push(alloc_list(Vec::new()))?;
    let pairs_slot = root_base + 2;
    let n = {
        // SAFETY: slot root_base+1 由 sort_items_dsu 压入 alloc_list 的 Ref。
        let items = unsafe { read_list(rooted_list_ptr(vm, root_base + 1)) };
        items.len()
    };
    for i in 0..n {
        // 元素从根化的源 list 重取（防上次迭代 key 调用触发 GC 后指针悬垂）。
        let elem = {
            let items_ptr = rooted_list_ptr(vm, root_base + 1);
            // SAFETY: 同上。
            let items = unsafe { read_list(items_ptr) };
            items[i].clone()
        };
        let key_now = vm.stack()[root_base].clone();
        let k = vm.call_function(&key_now, &[elem])?;
        if vm.escaped_exc.is_some() {
            // key 抛出的异常已被截获暂存：中止排序（结果将被丢弃），
            // call_value 会在本 native 返回后重抛给调用方。
            return Ok(Vec::new());
        }
        // call_function 之后重读源元素（GC 移动后旧 Ref 可能悬垂）。
        let elem_fresh = {
            let items_ptr = rooted_list_ptr(vm, root_base + 1);
            // SAFETY: 同上。
            let items = unsafe { read_list(items_ptr) };
            items[i].clone()
        };
        let pair = alloc_tuple(vec![k, elem_fresh]);
        // SAFETY: pairs_slot 由本函数压入 alloc_list 的 Ref。
        let pairs = unsafe { read_list(rooted_list_ptr(vm, pairs_slot)) };
        pairs.push(pair);
    }
    // 排序：纯 Rust 比较器（不重入 VM，无 GC 窗口）。
    let mut err = None;
    {
        let pairs_ptr = rooted_list_ptr(vm, pairs_slot);
        // SAFETY: 同上。
        let pairs = unsafe { read_list(pairs_ptr) };
        pairs.sort_by(|a, b| {
            let (ka, kb) = (tuple_elem(a, 0), tuple_elem(b, 0));
            let (x, y) = if reverse { (kb, ka) } else { (ka, kb) };
            cmp_objects(&x, &y, &mut err)
        });
    }
    if let Some(e) = err {
        return Err(e);
    }
    // undecorate：物化结果（此后不再重入 VM）。
    let pairs_ptr = rooted_list_ptr(vm, pairs_slot);
    // SAFETY: 同上。
    let pairs = unsafe { read_list(pairs_ptr) };
    Ok(pairs.iter().map(|p| tuple_elem(p, 1)).collect())
}

/// sorted(iterable, key?, reverse?) -> 新列表（task 80：key/reverse 可选 + DSU 稳定排序）。
/// arity MAX（native_arities）：native 内自校验 1-3 参。
/// 比较失败须上抛 TypeError（不静默错序）。
fn builtin_sorted(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() || args.len() > 3 {
        return Err(format!(
            "TypeError: sorted() takes 1-3 arguments, got {}",
            args.len()
        ));
    }
    let key = optional_key(args.get(1), "sorted(key)")?;
    let reverse = if args.len() > 2 {
        expect_reverse(args.get(2), "sorted(reverse)")?
    } else {
        false
    };
    // 统一走 to_iterator，接受任意可迭代对象（list/tuple/string/set/dict/range/iterator）。
    let items = collect_iter(&args[0])?;
    let sorted = sort_items_dsu(vm, items, key.as_ref(), reverse)?;
    Ok(alloc_list(sorted))
}

/// sorted_by(iterable, key, reverse?) -> 新列表（sorted 的 key 显式版，task 80）。
/// arity MAX：native 内自校验 2-3 参。
fn builtin_sorted_by(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "TypeError: sorted_by() takes 2-3 arguments, got {}",
            args.len()
        ));
    }
    let key = expect_callable(args.get(1), "sorted_by(key)")?;
    let reverse = if args.len() > 2 {
        expect_reverse(args.get(2), "sorted_by(reverse)")?
    } else {
        false
    };
    let items = collect_iter(&args[0])?;
    let sorted = sort_items_dsu(vm, items, Some(&key), reverse)?;
    Ok(alloc_list(sorted))
}

/// reversed(iterable) -> 反转迭代器。仅支持有确定序的 list/tuple/string。
fn builtin_reversed(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("reversed() requires 1 argument")?;
    let items = match arg {
        Object::Ref(ptr) => {
            debug_assert!(!ptr.is_null(), "null Object::Ref");
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.clone()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.clone()
            } else if tag == TypeTag::STRING as u8 {
                // reversed("abc") -> ["c","b","a"]（与 Python 对等）
                unsafe { read_str(*ptr) }
                    .chars()
                    .map(|c| alloc_string(&c.to_string()))
                    .collect()
            } else {
                return Err(format!(
                    "TypeError: '{}' object is not reversible",
                    arg.type_name()
                ));
            }
        }
        _ => {
            return Err(format!(
                "TypeError: '{}' object is not reversible",
                arg.type_name()
            ))
        }
    };
    let len = items.len();
    Ok(alloc_iterator(IteratorState::Reversed {
        items,
        index: len,
    }))
}

/// enumerate(iterable) -> 产生 (index, value) 对的迭代器。
fn builtin_enumerate(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("enumerate() requires 1 argument")?;
    let inner = to_iterator(arg)?;
    Ok(alloc_iterator(IteratorState::Enumerate {
        inner: Box::new(inner),
        index: 0,
    }))
}

/// zip(*iterables) -> 并行迭代，产生各迭代器当前值组成的元组。
fn builtin_zip(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("zip() requires at least 1 argument".to_string());
    }
    let iterators: Result<Vec<IteratorState>, String> = args.iter().map(to_iterator).collect();
    Ok(alloc_iterator(IteratorState::Zip {
        iterators: iterators?,
    }))
}

/// map(fn, iterable) -> 列表。急切求值（10-builtins.md:104-105）。
/// 依赖用户函数调用（task 27/28），本 task 以存根返回 Err。
fn builtin_map(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let fn_arg = args.get(0).ok_or("map() requires 2 arguments")?;
    let iterable = args.get(1).ok_or("map() requires 2 arguments")?;
    // 急切求值（10-builtins.md:104）：逐元素经 call_function 调用映射函数。
    let mut iter = to_iterator(iterable)?;
    let mut out: Vec<Object> = Vec::new();
    while let Some(val) = iter.next() {
        out.push(vm.call_function(fn_arg, &[val])?);
    }
    Ok(alloc_list(out))
}

/// filter(fn, iterable) -> 列表。急切求值（10-builtins.md:104-105）。
fn builtin_filter(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let fn_arg = args.get(0).ok_or("filter() requires 2 arguments")?;
    let iterable = args.get(1).ok_or("filter() requires 2 arguments")?;
    let mut iter = to_iterator(iterable)?;
    let mut out: Vec<Object> = Vec::new();
    while let Some(val) = iter.next() {
        let keep = vm.call_function(fn_arg, std::slice::from_ref(&val))?;
        if keep.is_truthy() {
            out.push(val);
        }
    }
    Ok(alloc_list(out))
}

/// any(iterable) -> 任一为 truthy 即 true。
fn builtin_any(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("any() requires 1 argument")?;
    let mut iter = to_iterator(arg)?;
    while let Some(val) = iter.next() {
        if val.is_truthy() {
            return Ok(Object::Bool(true));
        }
    }
    Ok(Object::Bool(false))
}

/// all(iterable) -> 全部为 truthy 才 true。
fn builtin_all(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("all() requires 1 argument")?;
    let mut iter = to_iterator(arg)?;
    while let Some(val) = iter.next() {
        if !val.is_truthy() {
            return Ok(Object::Bool(false));
        }
    }
    Ok(Object::Bool(true))
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
        // range 现返回迭代器（task 26，覆盖 task 25 的 List 版本）；
        // 经 collect_iter 消费为 Vec 后比对。
        // range(5) -> [0,1,2,3,4]
        let it = builtin_range(&mut v, &[Object::Int(5)]).unwrap();
        assert_eq!(
            alloc_list(collect_iter(&it).unwrap()),
            alloc_list(vec![
                Object::Int(0),
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4)
            ])
        );
        // range(2, 8, 2) -> [2,4,6]
        let it = builtin_range(&mut v, &[Object::Int(2), Object::Int(8), Object::Int(2)]).unwrap();
        assert_eq!(
            alloc_list(collect_iter(&it).unwrap()),
            alloc_list(vec![Object::Int(2), Object::Int(4), Object::Int(6)])
        );
        // range(3, 0, -1) -> [3,2,1]
        let it = builtin_range(&mut v, &[Object::Int(3), Object::Int(0), Object::Int(-1)]).unwrap();
        assert_eq!(
            alloc_list(collect_iter(&it).unwrap()),
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

    // ---- task 26：迭代器与容器函数 ----

    /// 消费迭代器 Object 为 Vec（测试辅助）。
    fn drain(obj: &Object) -> Vec<Object> {
        collect_iter(obj).unwrap()
    }

    #[test]
    fn test_range_basic() {
        let mut iter = IteratorState::Range {
            current: 0,
            end: 5,
            step: 1,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], Object::Int(0));
        assert_eq!(values[4], Object::Int(4));
    }

    #[test]
    fn test_range_with_step() {
        let mut iter = IteratorState::Range {
            current: 0,
            end: 10,
            step: 2,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(
            values,
            vec![
                Object::Int(0),
                Object::Int(2),
                Object::Int(4),
                Object::Int(6),
                Object::Int(8),
            ]
        );
    }

    #[test]
    fn test_range_negative_step() {
        let mut iter = IteratorState::Range {
            current: 5,
            end: 0,
            step: -1,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], Object::Int(5));
        assert_eq!(values[4], Object::Int(1));
    }

    #[test]
    fn test_enumerate() {
        let inner = IteratorState::ListIter {
            items: vec![alloc_string("a"), alloc_string("b")],
            index: 0,
        };
        let mut iter = IteratorState::Enumerate {
            inner: Box::new(inner),
            index: 0,
        };
        let first = iter.next().unwrap();
        assert_eq!(first, alloc_tuple(vec![Object::Int(0), alloc_string("a"),]));
        let second = iter.next().unwrap();
        assert_eq!(
            second,
            alloc_tuple(vec![Object::Int(1), alloc_string("b"),])
        );
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_zip() {
        let iter = IteratorState::Zip {
            iterators: vec![
                IteratorState::ListIter {
                    items: vec![Object::Int(1), Object::Int(2)],
                    index: 0,
                },
                IteratorState::ListIter {
                    items: vec![alloc_string("x"), alloc_string("y")],
                    index: 0,
                },
            ],
        };
        let mut iter = iter;
        let first = iter.next().unwrap();
        assert_eq!(first, alloc_tuple(vec![Object::Int(1), alloc_string("x"),]));
        let second = iter.next().unwrap();
        assert_eq!(
            second,
            alloc_tuple(vec![Object::Int(2), alloc_string("y"),])
        );
        // 长度对齐到最短：第二个迭代器耗尽后整体停止
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_any_all() {
        let mut v = vm();

        let result = builtin_any(
            &mut v,
            &[alloc_list(vec![Object::Bool(false), Object::Bool(true)])],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(true));

        let result = builtin_all(
            &mut v,
            &[alloc_list(vec![Object::Bool(true), Object::Bool(false)])],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(false));

        // 空集合：any -> false，all -> true（与 Python 一致）
        assert_eq!(
            builtin_any(&mut v, &[alloc_list(vec![])]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            builtin_all(&mut v, &[alloc_list(vec![])]).unwrap(),
            Object::Bool(true)
        );
    }

    #[test]
    fn test_sorted() {
        let mut v = vm();
        // sorted([3,1,2]) -> [1,2,3]
        assert_eq!(
            builtin_sorted(
                &mut v,
                &[alloc_list(vec![
                    Object::Int(3),
                    Object::Int(1),
                    Object::Int(2)
                ])]
            )
            .unwrap(),
            alloc_list(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
        // 接受任意可迭代对象：sorted(range(5)) 倒序经 reversed 验证；此处直接排序 tuple
        assert_eq!(
            builtin_sorted(&mut v, &[alloc_tuple(vec![Object::Int(2), Object::Int(1)])]).unwrap(),
            alloc_list(vec![Object::Int(1), Object::Int(2)])
        );
        // 不可比较元素 -> TypeError（不静默错序）
        let r = builtin_sorted(&mut v, &[alloc_list(vec![s("a"), Object::Int(1)])]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_reversed() {
        let mut v = vm();
        // reversed([1,2,3]) -> 迭代器，消费得 [3,2,1]
        let it = builtin_reversed(
            &mut v,
            &[alloc_list(vec![
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
            ])],
        )
        .unwrap();
        assert_eq!(
            alloc_list(drain(&it)),
            alloc_list(vec![Object::Int(3), Object::Int(2), Object::Int(1)])
        );
        // reversed("abc") -> ["c","b","a"]
        let it = builtin_reversed(&mut v, &[s("abc")]).unwrap();
        assert_eq!(
            alloc_list(drain(&it)),
            alloc_list(vec![s("c"), s("b"), s("a")])
        );
        // 不可逆类型 -> TypeError
        assert!(builtin_reversed(&mut v, &[Object::Int(1)]).is_err());
    }

    #[test]
    fn test_enumerate_builtin() {
        let mut v = vm();
        let it = builtin_enumerate(&mut v, &[alloc_list(vec![s("a"), s("b")])]).unwrap();
        let drained = drain(&it);
        assert_eq!(
            alloc_list(drained),
            alloc_list(vec![
                alloc_tuple(vec![Object::Int(0), s("a")]),
                alloc_tuple(vec![Object::Int(1), s("b")]),
            ])
        );
    }

    #[test]
    fn test_zip_builtin() {
        let mut v = vm();
        let it = builtin_zip(
            &mut v,
            &[
                alloc_list(vec![Object::Int(1), Object::Int(2)]),
                alloc_list(vec![s("x"), s("y")]),
            ],
        )
        .unwrap();
        assert_eq!(
            alloc_list(drain(&it)),
            alloc_list(vec![
                alloc_tuple(vec![Object::Int(1), s("x")]),
                alloc_tuple(vec![Object::Int(2), s("y")]),
            ])
        );
        assert!(builtin_zip(&mut v, &[]).is_err()); // 至少 1 参
    }

    #[test]
    fn test_container_constructors() {
        let mut v = vm();
        // 空构造（0 参）
        assert_eq!(builtin_list(&mut v, &[]).unwrap(), alloc_list(vec![]));
        assert_eq!(builtin_tuple(&mut v, &[]).unwrap(), alloc_tuple(vec![]));
        // list("abc") -> ["a","b","c"]；set([1,2,2]) -> {1,2}
        assert_eq!(
            builtin_list(&mut v, &[s("abc")]).unwrap(),
            alloc_list(vec![s("a"), s("b"), s("c")])
        );
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
        // 多参 -> Err
        assert!(builtin_list(&mut v, &[Object::Int(1), Object::Int(2)]).is_err());
        // tuple 从 range 迭代器构造（验证迭代器可被消费）
        let it = builtin_range(&mut v, &[Object::Int(3)]).unwrap();
        assert_eq!(
            builtin_tuple(&mut v, std::slice::from_ref(&it)).unwrap(),
            alloc_tuple(vec![Object::Int(0), Object::Int(1), Object::Int(2)])
        );
    }

    #[test]
    fn test_dict_constructor() {
        let mut v = vm();
        // 空 dict
        assert_eq!(
            builtin_dict(&mut v, &[]).unwrap(),
            alloc_dict(DictMap::new())
        );
        // dict 拷贝
        let mut m = DictMap::new();
        m.insert(s("k"), Object::Int(9));
        let src = alloc_dict(m);
        assert_eq!(
            builtin_dict(&mut v, std::slice::from_ref(&src)).unwrap(),
            src
        );
        // 非 dict（MVP 不支持 (k,v) 对构造）-> Err
        assert!(builtin_dict(&mut v, &[alloc_list(vec![])]).is_err());
    }

    #[test]
    fn test_to_iterator_all_types() {
        // SET 支持（task 26 订正）：to_iterator 须接受 set
        let mut s = HashSet::new();
        s.insert(Object::Int(7));
        let set_obj = alloc_set(s);
        assert!(to_iterator(&set_obj).is_ok());
        // 迭代器自身可再次转迭代（克隆状态）
        let it = alloc_iterator(IteratorState::Range {
            current: 0,
            end: 3,
            step: 1,
        });
        assert!(to_iterator(&it).is_ok());
        // 不可迭代 -> Err
        assert!(to_iterator(&Object::Int(1)).is_err());
    }

    #[test]
    fn test_iterator_heap_object() {
        // alloc_iterator -> read_iterator：type_tag 与状态可读/可推进
        let obj = alloc_iterator(IteratorState::Range {
            current: 0,
            end: 2,
            step: 1,
        });
        let Object::Ref(ptr) = obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::ITERATOR as u8);
            let it = read_iterator(ptr);
            assert_eq!(it.state.next(), Some(Object::Int(0)));
            assert_eq!(it.state.next(), Some(Object::Int(1)));
            assert_eq!(it.state.next(), None);
        }
    }

    #[test]
    fn test_map_filter_stubs() {
        let mut v = vm();
        // 依赖用户函数调用（task 27/28），本 task 以存根返回 Err。
        assert!(builtin_map(&mut v, &[Object::Int(1), Object::Int(2)]).is_err());
        assert!(builtin_filter(&mut v, &[Object::Int(1), Object::Int(2)]).is_err());
    }

    // ---- task 80：sorted/sorted_by 参数自校验（DSU 稳定性/reverse/异常穿透
    //     的端到端用例见 list.rs tests，需经 test_util::run_source 驱动）----

    #[test]
    fn test_sorted_arity_self_validation() {
        let mut v = vm();
        // 0 参 / 4 参 → TypeError（arity MAX，native 内自校验 1-3）
        let err = builtin_sorted(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("1-3"), "got: {}", err);
        let err = builtin_sorted(
            &mut v,
            &[
                alloc_list(vec![]),
                Object::Nil,
                Object::Bool(false),
                Object::Bool(false),
            ],
        )
        .unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // key 非 callable → TypeError
        let err = builtin_sorted(&mut v, &[alloc_list(vec![]), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("callable"), "got: {}", err);
        // reverse 非 bool → TypeError
        let err = builtin_sorted(
            &mut v,
            &[alloc_list(vec![]), Object::Nil, Object::Int(1)],
        )
        .unwrap_err();
        assert!(err.contains("TypeError") && err.contains("reverse"), "got: {}", err);
        // nil key 视为缺省（合法）
        assert!(builtin_sorted(&mut v, &[alloc_list(vec![Object::Int(2), Object::Int(1)]), Object::Nil]).is_ok());
    }

    #[test]
    fn test_sorted_by_arity_self_validation() {
        let mut v = vm();
        // 1 参 / 4 参 → TypeError（自校验 2-3）
        let err = builtin_sorted_by(&mut v, &[alloc_list(vec![])]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("2-3"), "got: {}", err);
        let err = builtin_sorted_by(
            &mut v,
            &[
                alloc_list(vec![]),
                Object::Nil,
                Object::Bool(true),
                Object::Bool(true),
            ],
        )
        .unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // key 必填：nil → TypeError（callable 校验）
        let err = builtin_sorted_by(&mut v, &[alloc_list(vec![]), Object::Nil]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("callable"), "got: {}", err);
    }

    #[test]
    fn test_sorted_no_key_and_reverse() {
        let mut v = vm();
        // 无 key 兼容旧用例
        assert_eq!(
            builtin_sorted(&mut v, &[alloc_list(vec![Object::Int(3), Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_list(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
        // 无 key + reverse（nil 占位 key）
        assert_eq!(
            builtin_sorted(
                &mut v,
                &[
                    alloc_list(vec![Object::Int(3), Object::Int(1), Object::Int(2)]),
                    Object::Nil,
                    Object::Bool(true)
                ]
            )
            .unwrap(),
            alloc_list(vec![Object::Int(3), Object::Int(2), Object::Int(1)])
        );
        // 不可比较元素 → TypeError 上抛
        let err = builtin_sorted(&mut v, &[alloc_list(vec![s("a"), Object::Int(1)])]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }
}
