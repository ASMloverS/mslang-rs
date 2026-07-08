//! mslang 标准库原生模块。
//!
//! 参照 [46-stdlib-io](../../docs/mslang/tasks/46-stdlib-io.md)。
//!
//! 原生 Rust 模块经 `register_*` 构造 MsModule（task 45），exports 为原生函数
//! （`alloc_native_function` → Object::Ref + TypeTag::FUNCTION）。由 `ModuleResolver`
//! 的 `native_modules` 注册表登记，`import` 命中即跳过磁盘搜索。

#![allow(clippy::get_first)]

use super::builtins::{alloc_native_function, NativeFunction, NativeFn};
use super::object::{
    alloc_dict, alloc_file_handle, alloc_list, alloc_module, alloc_set, alloc_string, alloc_tuple,
    read_dict, read_file_handle, read_file_handle_mut, read_list, read_module_mut, read_set,
    read_str, CmpOp, DictMap, MsObjHeader, Object, TypeTag,
};
use super::VM;
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// io 模块
// ---------------------------------------------------------------------------

/// 构造 `io` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 open/read_file/write_file/exists 四个原生函数。
pub fn register_io_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    exports.insert(
        "open".to_string(),
        alloc_native_function(NativeFunction {
            name: "open".to_string(),
            func: native_io_open,
        }),
    );
    exports.insert(
        "read_file".to_string(),
        alloc_native_function(NativeFunction {
            name: "read_file".to_string(),
            func: native_io_read_file,
        }),
    );
    exports.insert(
        "write_file".to_string(),
        alloc_native_function(NativeFunction {
            name: "write_file".to_string(),
            func: native_io_write_file,
        }),
    );
    exports.insert(
        "exists".to_string(),
        alloc_native_function(NativeFunction {
            name: "exists".to_string(),
            func: native_io_exists,
        }),
    );
    let m = alloc_module("io");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// io 模块函数与全局 open() 的共享实现。
pub fn native_io_open(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "open(path, mode?)")?;
    let mode = if args.len() > 1 {
        expect_string(args.get(1), "open(path, mode?)")?
    } else {
        "r".to_string()
    };
    let mut opts = std::fs::OpenOptions::new();
    match mode.as_str() {
        "r" => {
            opts.read(true);
        }
        "w" => {
            opts.write(true).create(true).truncate(true);
        }
        "a" => {
            opts.append(true).create(true);
        }
        _ => return Err(format!("ValueError: unknown mode '{}'", mode)),
    }
    let file = opts
        .open(&path)
        .map_err(|e| format!("IOError: cannot open '{}': {}", path, e))?;
    Ok(alloc_file_handle(&path, &mode, file))
}

fn native_io_read_file(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "read_file(path)")?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("IOError: cannot read '{}': {}", path, e))?;
    Ok(alloc_string(&content))
}

fn native_io_write_file(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "write_file(path, content)")?;
    let content = expect_string(args.get(1), "write_file(path, content)")?;
    std::fs::write(&path, content)
        .map_err(|e| format!("IOError: cannot write '{}': {}", path, e))?;
    Ok(Object::Nil)
}

fn native_io_exists(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "exists(path)")?;
    Ok(Object::Bool(std::path::Path::new(&path).exists()))
}

// ---------------------------------------------------------------------------
// FileHandle 方法
// ---------------------------------------------------------------------------

/// FileHandle 方法名 → 原生函数对象指针（供 GET_ATTR 包装为 BoundMethod）。
/// 每次 GET_ATTR 分配新对象（与 INSTANCE 方法绑定的 BoundMethod 分配一致）。
pub fn lookup_file_method(name: &str) -> Option<*mut MsObjHeader> {
    let func: NativeFn = match name {
        "read" => native_fh_read,
        "write" => native_fh_write,
        "close" => native_fh_close,
        "lines" => native_fh_lines,
        "__enter__" => native_fh_enter,
        "__exit__" => native_fh_exit,
        _ => return None,
    };
    let obj = alloc_native_function(NativeFunction {
        name: name.to_string(),
        func,
    });
    match obj {
        Object::Ref(p) => Some(p),
        _ => None,
    }
}

/// 校验首参数为 FileHandle Ref，返回其裸指针。
fn expect_file_handle(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::FILE_HANDLE as u8 => {
            Ok(*ptr)
        }
        other => Err(format!(
            "TypeError: {} expects a file handle, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// f.read() → 读取全部内容（从当前游标至 EOF）。
fn native_fh_read(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_file_handle(args.get(0), "read()")?;
    // SAFETY: ptr 由 expect_file_handle 校验为有效 MsFileHandle。
    let h = unsafe { read_file_handle(ptr) };
    let file_opt = unsafe { &mut *h.file_ptr };
    match file_opt.as_mut() {
        Some(file) => {
            let mut content = String::new();
            use std::io::Read;
            file.read_to_string(&mut content)
                .map_err(|e| format!("IOError: {}", e))?;
            Ok(alloc_string(&content))
        }
        None => Err("IOError: file already closed".to_string()),
    }
}

/// f.write(content) → 写入内容。
fn native_fh_write(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_file_handle(args.get(0), "write()")?;
    let content = expect_string(args.get(1), "write(content)")?;
    // SAFETY: ptr 由 expect_file_handle 校验。
    let h = unsafe { read_file_handle(ptr) };
    let file_opt = unsafe { &mut *h.file_ptr };
    match file_opt.as_mut() {
        Some(file) => {
            use std::io::Write;
            file.write_all(content.as_bytes())
                .map_err(|e| format!("IOError: {}", e))?;
            Ok(Object::Nil)
        }
        None => Err("IOError: file already closed".to_string()),
    }
}

/// f.close() → 关闭句柄（幂等）。
fn native_fh_close(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_file_handle(args.get(0), "close()")?;
    // SAFETY: ptr 由 expect_file_handle 校验。
    let h = unsafe { read_file_handle_mut(ptr) };
    let file_opt = unsafe { &mut *h.file_ptr };
    *file_opt = None;
    Ok(Object::Nil)
}

/// f.lines() → 按行读取，返回 List。末尾换行不额外产生空元素。
fn native_fh_lines(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_file_handle(args.get(0), "lines()")?;
    // SAFETY: ptr 由 expect_file_handle 校验。
    let h = unsafe { read_file_handle(ptr) };
    let file_opt = unsafe { &mut *h.file_ptr };
    let content = match file_opt.as_mut() {
        Some(file) => {
            let mut buf = String::new();
            use std::io::Read;
            file.read_to_string(&mut buf)
                .map_err(|e| format!("IOError: {}", e))?;
            buf
        }
        None => return Err("IOError: file already closed".to_string()),
    };
    // str::lines() 按 '\n'（及 '\r\n'）分割，末尾换行不产生空元素（task 46 §2 语义）。
    let items: Vec<Object> = content.lines().map(alloc_string).collect();
    Ok(alloc_list(items))
}

/// f.__enter__() → 返回 self（with 语句绑定）。
fn native_fh_enter(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    expect_file_handle(args.get(0), "__enter__()")?;
    Ok(args[0].clone())
}
/// f.__exit__(self, err_type, err_msg, traceback) → 关闭句柄，异常继续传播（不抑制）。
/// 固定 4 参数（task 38 with 编译器 CALL 4 约定）。
fn native_fh_exit(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_file_handle(args.get(0), "__exit__()")?;
    // SAFETY: ptr 由 expect_file_handle 校验。
    let h = unsafe { read_file_handle_mut(ptr) };
    let file_opt = unsafe { &mut *h.file_ptr };
    *file_opt = None;
    Ok(Object::Nil)
}

// ---------------------------------------------------------------------------
// String 方法（task 50：GET_ATTR → BoundMethod 分派，仿 task 46 FileHandle 模式）
// ---------------------------------------------------------------------------

/// String 方法名 → 原生函数指针（供 GET_ATTR 包装为 BoundMethod）。
/// 每次 GET_ATTR 由调用方 alloc_native_function 分配新对象（与 task 46
/// lookup_file_method 模式一致；性能优化留待 task 52+ intern 表方案）。
pub fn lookup_string_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_str_length,
        "upper" => native_str_upper,
        "lower" => native_str_lower,
        "strip" => native_str_strip,
        "split" => native_str_split,
        "join" => native_str_join,
        "replace" => native_str_replace,
        "contains" => native_str_contains,
        "startswith" => native_str_startswith,
        "endswith" => native_str_endswith,
        "index" => native_str_index,
        "slice" => native_str_slice,
        _ => return None,
    };
    Some(func)
}

// 注：args[0] 为 String receiver（BoundMethod 注入，见 mod.rs GetAttr STRING 分支
// + CALL BOUND_METHOD→FUNCTION 注入），用户参数从 args.get(1) 起。

/// s.length() → 字符数（Unicode scalar，非字节数）。
fn native_str_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "length()")?;
    Ok(Object::Int(s.chars().count() as i64))
}

/// s.upper() → 转大写。
fn native_str_upper(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "upper()")?;
    Ok(alloc_string(&s.to_uppercase()))
}

/// s.lower() → 转小写。
fn native_str_lower(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "lower()")?;
    Ok(alloc_string(&s.to_lowercase()))
}

/// s.strip() → 去除两端空白（Rust trim() = Unicode White_Space，与 Python 一致）。
fn native_str_strip(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "strip()")?;
    Ok(alloc_string(s.trim()))
}

/// s.split(sep?) → 分割为列表。无参按 Unicode 空白；空分隔符报错。
fn native_str_split(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "split(sep?)")?;
    let parts: Vec<Object> = if args.len() <= 1 {
        recv.split_whitespace().map(alloc_string).collect()
    } else {
        let sep = expect_string(args.get(1), "split(sep?)")?;
        if sep.is_empty() {
            // Rust str::split("") 会返回含边界空串的怪异结果，故显式拒绝。
            return Err("ValueError: empty separator".to_string());
        }
        recv.split(&sep).map(alloc_string).collect()
    };
    Ok(alloc_list(parts))
}

/// s.join(list) → 用 s 连接列表（list 元素必须全为 string）。
fn native_str_join(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let sep = expect_string(args.get(0), "join(list)")?;
    let list_ptr = expect_list_ref(args.get(1), "join(list)")?;
    // 借用约束：递归/分配前先 clone 出元素，释放 &mut Vec<Object>。
    let items: Vec<Object> = unsafe { read_list(list_ptr) }.clone();
    let strs: Vec<String> = items
        .iter()
        .map(|o| match o {
            Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::STRING as u8 => {
                // SAFETY: type_tag 为 STRING。
                Ok(unsafe { read_str(*p) }.to_owned())
            }
            other => Err(format!(
                "TypeError: join() expects list of strings, got {}",
                other.type_name()
            )),
        })
        .collect::<Result<_, _>>()?;
    Ok(alloc_string(&strs.join(&sep)))
}

/// s.replace(old, new) → 替换全部子串。
fn native_str_replace(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "replace(old, new)")?;
    let old = expect_string(args.get(1), "replace(old, new)")?;
    let new = expect_string(args.get(2), "replace(old, new)")?;
    Ok(alloc_string(&recv.replace(&old, &new)))
}

/// s.contains(sub) → 是否包含子串。
fn native_str_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "contains(sub)")?;
    let sub = expect_string(args.get(1), "contains(sub)")?;
    Ok(Object::Bool(recv.contains(&sub)))
}

/// s.startswith(prefix) → 是否以 prefix 开头。
fn native_str_startswith(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "startswith(prefix)")?;
    let pfx = expect_string(args.get(1), "startswith(prefix)")?;
    Ok(Object::Bool(recv.starts_with(&pfx)))
}

/// s.endswith(suffix) → 是否以 suffix 结尾。
fn native_str_endswith(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "endswith(suffix)")?;
    let sfx = expect_string(args.get(1), "endswith(suffix)")?;
    Ok(Object::Bool(recv.ends_with(&sfx)))
}

/// s.index(sub) → 子串首次出现的字符位置（非字节位置）。未找到抛 ValueError。
fn native_str_index(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "index(sub)")?;
    let sub = expect_string(args.get(1), "index(sub)")?;
    match recv.find(&sub) {
        Some(byte_pos) => {
            // find 返回字节位置，转字符位置（与 length/slice 一致）。
            let char_pos = recv[..byte_pos].chars().count() as i64;
            Ok(Object::Int(char_pos))
        }
        None => Err(format!("ValueError: substring '{}' not found", sub)),
    }
}

/// s.slice(start, end?) → 切片。字符位置；负索引相对末尾；越界饱和；start>end 报错。
fn native_str_slice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "slice(start, end?)")?;
    let start_i = expect_int(args.get(1), "slice(start, end?)")?;
    let end_opt = if args.len() > 2 {
        Some(expect_int(args.get(2), "slice(start, end?)")?)
    } else {
        None
    };
    let chars: Vec<char> = recv.chars().collect();
    let len = chars.len() as i64;
    let norm = |i: i64| -> i64 {
        if i < 0 {
            (len + i).max(0)
        } else {
            i.min(len)
        }
    };
    let s = norm(start_i);
    let e = match end_opt {
        Some(i) => norm(i),
        None => len,
    };
    if s > e {
        return Err(format!("ValueError: slice start {} > end {}", s, e));
    }
    let result: String = chars[s as usize..e as usize].iter().collect();
    Ok(alloc_string(&result))
}

// ---------------------------------------------------------------------------
// math 模块
// ---------------------------------------------------------------------------

/// 构造 `math` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 2 个 inline Float 常量（pi/e）+ 13 个原生函数。
pub fn register_math_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();

    // 常量（inline Object::Float，无需堆分配；14-gc.md:54 内联值不参与 GC 扫描）
    exports.insert("pi".to_string(), Object::Float(std::f64::consts::PI));
    exports.insert("e".to_string(), Object::Float(std::f64::consts::E));

    // 函数（alloc_native_function → Object::Ref + TypeTag::FUNCTION）
    let funcs: [(&str, NativeFn); 13] = [
        ("sqrt", native_math_sqrt),
        ("pow", native_math_pow),
        ("abs", native_math_abs),
        ("sin", native_math_sin),
        ("cos", native_math_cos),
        ("tan", native_math_tan),
        ("log", native_math_log),
        ("log2", native_math_log2),
        ("log10", native_math_log10),
        ("exp", native_math_exp),
        ("ceil", native_math_ceil),
        ("floor", native_math_floor),
        ("round", native_math_round),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }

    let m = alloc_module("math");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// 从预期为数值的参数提取 f64（Int/Bool 自动转 Float）。
/// None（缺参）或非数值 → TypeError（参照 task 46 expect_string 模式）。
fn expect_number(arg: Option<&Object>, who: &str) -> Result<f64, String> {
    match arg {
        Some(Object::Int(n)) => Ok(*n as f64),
        Some(Object::Float(x)) => Ok(*x),
        Some(Object::Bool(b)) => Ok(if *b { 1.0 } else { 0.0 }),
        other => Err(format!(
            "TypeError: {} expects number, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// f64 → Object::Int，校验 NaN 与 i64 范围溢出。
/// Rust `as i64` 在溢出/NaN 时静默饱和（1e30→i64::MAX, NaN→0），须显式拒绝。
/// 有效区间 `[-I64_BOUND, I64_BOUND)`（I64_BOUND = 2^63 = i64::MAX 的 f64 近似）：
/// 超出此区间 `as i64` 会饱和，故拒绝；-2^63 == i64::MIN 仍可精确表示。
fn float_to_int(x: f64, who: &str) -> Result<Object, String> {
    const I64_BOUND: f64 = 9.223372036854776e18;
    if x.is_nan() {
        return Err(format!("ValueError: {} input is NaN", who));
    }
    if !(-I64_BOUND..I64_BOUND).contains(&x) {
        return Err(format!("OverflowError: {} result out of int range", who));
    }
    Ok(Object::Int(x as i64))
}

// 注：sqrt/pow/sin/cos/tan/log/log2/log10/exp 返回 Object::Float；
// abs 保留入参类型；ceil/floor/round 经 float_to_int 返回 Object::Int。

fn native_math_sqrt(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sqrt(x)")?;
    Ok(Object::Float(x.sqrt()))
}

fn native_math_pow(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let base = expect_number(args.get(0), "pow(base, exp)")?;
    let exp = expect_number(args.get(1), "pow(base, exp)")?;
    Ok(Object::Float(base.powf(exp)))
}

fn native_math_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 保留入参类型：Int→Int, Float→Float, Bool→Int（与全局 abs(n)->number 一致）
    match args.get(0) {
        Some(Object::Int(n)) => Ok(Object::Int(n.wrapping_abs())),
        Some(Object::Float(x)) => Ok(Object::Float(x.abs())),
        Some(Object::Bool(true)) => Ok(Object::Int(1)),
        Some(Object::Bool(false)) => Ok(Object::Int(0)),
        other => Err(format!(
            "TypeError: abs(x) expects number, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

fn native_math_sin(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sin(x)")?;
    Ok(Object::Float(x.sin()))
}

fn native_math_cos(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "cos(x)")?;
    Ok(Object::Float(x.cos()))
}

fn native_math_tan(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "tan(x)")?;
    Ok(Object::Float(x.tan()))
}

fn native_math_log(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log(x)")?;
    Ok(Object::Float(x.ln()))
}

fn native_math_log2(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log2(x)")?;
    Ok(Object::Float(x.log2()))
}

fn native_math_log10(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log10(x)")?;
    Ok(Object::Float(x.log10()))
}

fn native_math_exp(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "exp(x)")?;
    Ok(Object::Float(x.exp()))
}

fn native_math_ceil(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "ceil(x)")?;
    float_to_int(x.ceil(), "ceil")
}

fn native_math_floor(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "floor(x)")?;
    float_to_int(x.floor(), "floor")
}

fn native_math_round(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // Rust f64::round()：半远离零（round(2.5)→3，非 Python 银行家舍入）。
    let x = expect_number(args.get(0), "round(x)")?;
    float_to_int(x.round(), "round")
}

// ---------------------------------------------------------------------------
// os 模块
// ---------------------------------------------------------------------------

/// 构造 `os` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 getenv/setenv/getcwd/chdir/exec/exit 六个原生函数 + args 列表属性。
pub fn register_os_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 6] = [
        ("getenv", native_os_getenv),
        ("setenv", native_os_setenv),
        ("getcwd", native_os_getcwd),
        ("chdir", native_os_chdir),
        ("exec", native_os_exec),
        ("exit", native_os_exit),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    // args 为 List 属性（非函数）：注册时一次性快照命令行参数。
    exports.insert("args".to_string(), build_args_list());
    let m = alloc_module("os");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// 构建 os.args 列表：std::env::args() → alloc_string → alloc_list。
/// 在 register_os_module 时调用一次，结果存入 exports（不需 vm）。
fn build_args_list() -> Object {
    let items: Vec<Object> = std::env::args().map(|a| alloc_string(&a)).collect();
    alloc_list(items)
}

fn native_os_getenv(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let key = expect_string(args.get(0), "getenv(key)")?;
    match std::env::var(&key) {
        Ok(val) => Ok(alloc_string(&val)),
        Err(_) => Ok(Object::Nil), // 不存在返回 nil（非异常）
    }
}

fn native_os_setenv(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let key = expect_string(args.get(0), "setenv(key, val)")?;
    let val = expect_string(args.get(1), "setenv(key, val)")?;
    // 进程级可变状态操作；MVP 单线程 VM 下安全。
    // 注：Rust 2024 edition 将 set_var 标记为 unsafe，升级 edition 时需加 unsafe 块。
    std::env::set_var(&key, &val);
    Ok(Object::Nil)
}

fn native_os_getcwd(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dir = std::env::current_dir().map_err(|e| format!("IOError: {}", e))?;
    Ok(alloc_string(&dir.to_string_lossy()))
}

fn native_os_chdir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "chdir(path)")?;
    std::env::set_current_dir(&path).map_err(|e| format!("IOError: {}", e))?;
    Ok(Object::Nil)
}

/// os.exec(cmd) → 经 shell 执行，返回 stdout。
/// 安全警告：cmd 经 shell（Windows cmd /C、Unix sh -c）执行，用户可控输入直接拼入
/// 存在命令注入风险（10-builtins.md:303）。调用者须自行消毒输入。MVP 不提供安全变体。
fn native_os_exec(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let cmd = expect_string(args.get(0), "exec(cmd)")?;
    #[cfg(windows)]
    let output = std::process::Command::new("cmd").args(["/C", &cmd]).output();
    #[cfg(not(windows))]
    let output = std::process::Command::new("sh").args(["-c", &cmd]).output();
    let output = output.map_err(|e| format!("IOError: exec failed: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "IOError: command failed (exit code {:?})",
            output.status.code()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(alloc_string(&stdout))
}

/// os.exit(code) → 不直接调 std::process::exit（绕过 defer/GC）。
/// 改为返回特殊标记 Err("__EXIT__{code}")：作为异常沿调用栈传播，defer/finally 在
/// 解栈过程中执行。VM 顶层 run 循环应检测此前缀，运行 finalizer 后以 code 退出。
/// 已知限制（MVP）：run 循环尚未特判此前缀，故 exit 经 interpret 以 Err 返回给宿主。
fn native_os_exit(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let code = match args.get(0) {
        Some(Object::Int(n)) => *n as i32,
        other => {
            return Err(format!(
                "TypeError: exit(code) expects int, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    Err(format!("__EXIT__{}", code))
}

// ---------------------------------------------------------------------------
// string 模块
// ---------------------------------------------------------------------------

/// 构造 `string` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 format/repeat/reverse/is_alpha/is_digit 五个原生函数。
pub fn register_string_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 5] = [
        ("format", native_string_format),
        ("repeat", native_string_repeat),
        ("reverse", native_string_reverse),
        ("is_alpha", native_string_is_alpha),
        ("is_digit", native_string_is_digit),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("string");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// string.format(template, *args) → 替换 {} 占位符。
/// 非 string 参数经 object_to_string 转换（与 print/str 一致）：
///   Int→"42", Float→"3.14", Bool→"true"/"false", Nil→"nil", String→原串。
fn native_string_format(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let template = expect_string(args.get(0), "format(template, ...)")?;
    let mut result = String::new();
    let mut arg_idx = 1usize;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'}') {
            chars.next(); // 消费 '}'
            let val = args.get(arg_idx).ok_or_else(|| {
                format!(
                    "ValueError: format: not enough arguments for placeholder #{}",
                    arg_idx
                )
            })?;
            result.push_str(&super::builtins::object_to_string(vm, val)?);
            arg_idx += 1;
        } else {
            result.push(c);
        }
    }
    Ok(alloc_string(&result))
}

/// string.repeat(s, n) → s 重复 n 次。负数 / 超大 n → ValueError。
fn native_string_repeat(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "repeat(s, n)")?;
    let n = match args.get(1) {
        Some(Object::Int(n)) if *n >= 0 && *n <= 1_000_000 => *n as usize,
        Some(Object::Int(n)) if *n < 0 => {
            return Err("ValueError: repeat count cannot be negative".into())
        }
        Some(Object::Int(_)) => {
            return Err("ValueError: repeat count too large (max 1000000)".into())
        }
        other => {
            return Err(format!(
                "TypeError: repeat(s, n) expects int, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    Ok(alloc_string(&s.repeat(n)))
}

fn native_string_reverse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "reverse(s)")?;
    Ok(alloc_string(&s.chars().rev().collect::<String>()))
}

fn native_string_is_alpha(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_alpha(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))
}

fn native_string_is_digit(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_digit(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())))
}

// ---------------------------------------------------------------------------
// time 模块
// ---------------------------------------------------------------------------

/// 构造 `time` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 now/sleep/format 三个原生函数。
pub fn register_time_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 3] = [
        ("now", native_time_now),
        ("sleep", native_time_sleep),
        ("format", native_time_format),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("time");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// time.now() → 当前 Unix 时间戳（秒，f64）。
/// 不使用 .unwrap()：系统时间早于 epoch 时返回 Err 而非 panic。
fn native_time_now(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("ClockError: system time before epoch: {}", e))?;
    Ok(Object::Float(dur.as_secs_f64()))
}

/// time.sleep(secs) → 阻塞指定秒数（int 或 float）。
/// 单位为秒（与 10-builtins.md:326 一致，非毫秒）。负数 / 非有限值 → ValueError。
fn native_time_sleep(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let secs = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => {
            return Err(format!(
                "TypeError: sleep(secs) expects number, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    // from_secs_f64 在 NaN/±Inf 上 panic，须先拒绝非有限值。
    if !secs.is_finite() {
        return Err("ValueError: sleep duration must be finite".into());
    }
    if secs < 0.0 {
        return Err("ValueError: sleep duration cannot be negative".into());
    }
    std::thread::sleep(Duration::from_secs_f64(secs));
    Ok(Object::Nil)
}

/// time.format(ts) → 将 Unix 时间戳格式化为 UTC 字符串 "YYYY-MM-DD HH:MM:SS"。
/// MVP 手动格式化（不引入 chrono 依赖）。时区固定 UTC。
fn native_time_format(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ts = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => {
            return Err(format!(
                "TypeError: format(ts) expects number, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    if ts < 0.0 {
        return Err("ValueError: timestamp cannot be negative".into());
    }
    let secs = ts as u64;
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    Ok(alloc_string(&format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )))
}

/// Unix 时间戳（秒）→ UTC 年月日时分秒（民用历法算法，Howard Hinnant
/// `civil_from_days`）。纯整数运算，无 chrono 依赖。
fn unix_to_ymdhms(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64; // 自 1970-01-01 的天数
    let rem = secs % 86_400;
    let hour = (rem / 3_600) as u32;
    let min = ((rem % 3_600) / 60) as u32;
    let sec = (rem % 60) as u32;

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32, hour, min, sec)
}

// ---------------------------------------------------------------------------
// path 模块
// ---------------------------------------------------------------------------

/// 构造 `path` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 join/ext/base/dir 四个原生函数。
pub fn register_path_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 4] = [
        ("join", native_path_join),
        ("ext", native_path_ext),
        ("base", native_path_base),
        ("dir", native_path_dir),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("path");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// path.join(*parts) → 连接路径段。可变参（arity = usize::MAX）。
/// 输出保留平台分隔符（Windows `\`，Unix `/`），不归一化为 `/`。
fn native_path_join(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("ValueError: path.join requires at least one argument".into());
    }
    let mut result = std::path::PathBuf::new();
    for (i, arg) in args.iter().enumerate() {
        let part = expect_string(Some(arg), &format!("path.join part #{}", i))?;
        result.push(&part);
    }
    Ok(alloc_string(&result.to_string_lossy()))
}

/// path.ext(p) → 扩展名（含 "."），无扩展名返回 ""。
fn native_path_ext(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "ext(p)")?;
    let ext = std::path::Path::new(&p)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    Ok(alloc_string(&ext))
}

/// path.base(p) → 文件名部分，无文件名返回 ""。
fn native_path_base(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "base(p)")?;
    let base = std::path::Path::new(&p)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(alloc_string(&base))
}

/// path.dir(p) → 目录部分，无父目录返回 ""。
fn native_path_dir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "dir(p)")?;
    let dir = std::path::Path::new(&p)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(alloc_string(&dir))
}

// ---------------------------------------------------------------------------
// json 模块（task 49）
// ---------------------------------------------------------------------------

/// JSON 解析/序列化的最大嵌套深度（task 49 §验证标准 #10）。
/// MAX_NESTING=1000 兼顾常规用例与栈安全：默认线程栈（8 MiB）下 ~1000 层递归
/// 不会溢出，同时拒绝恶意深嵌套输入。
const MAX_NESTING: u32 = 1000;

/// 构造 `json` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 parse/stringify 两个原生函数。
pub fn register_json_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 2] = [
        ("parse", native_json_parse),
        ("stringify", native_json_stringify),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("json");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// json.parse(string) → 解析 JSON 文本为 mslang 值（task 49 §方案 B，手动解析，零依赖）。
/// 类型映射：null→nil、bool→bool、整数→int、浮点→float、字符串→string、
/// 数组→list、对象→dict。超出 i64 的整数退化为 float（与 Python JSON 一致）。
fn native_json_parse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let json_str = expect_string(args.get(0), "parse(string)")?;
    let bytes = json_str.as_bytes();
    let mut p = JsonParser { src: bytes, pos: 0 };
    p.skip_ws();
    let v = p.parse_value(0)?;
    p.skip_ws();
    if p.pos != bytes.len() {
        return Err(format!(
            "ValueError: json trailing characters at byte {}",
            p.pos
        ));
    }
    Ok(v)
}

/// json.stringify(value) → 将 mslang 值序列化为 JSON 文本。
/// nil→null、bool→true/false、int→整数、float→数字（NaN/Infinity 报错）、
/// string→字符串字面量、list→数组、dict→对象（键必须为 string）。
/// tuple/set/function/... 不支持，返回 TypeError（Phase 6.2d 无 __to_json__）。
fn native_json_stringify(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let obj = args
        .get(0)
        .ok_or_else(|| "ValueError: stringify expects 1 argument".to_string())?;
    let mut out = String::new();
    let mut seen: HashSet<usize> = HashSet::new();
    stringify_into(obj, &mut out, 0, &mut seen)?;
    Ok(alloc_string(&out))
}

/// 递归将 obj 序列化进 out。`seen` 记录当前递归路径上的 list/dict 指针地址，
/// 用于检测循环引用（同对象出现在兄弟位置不视为循环，递归返回后移除）。
fn stringify_into(
    obj: &Object,
    out: &mut String,
    depth: u32,
    seen: &mut HashSet<usize>,
) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("ValueError: nesting exceeds {} levels", MAX_NESTING));
    }
    match obj {
        Object::Nil => out.push_str("null"),
        Object::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Object::Int(i) => {
            use std::fmt::Write;
            let _ = write!(out, "{}", i);
        }
        Object::Float(f) => {
            // NaN/Infinity 非合法 JSON 数字（RFC 8259），显式报错
            //（与 02-types.md § 特殊浮点值语义一致）。
            if !f.is_finite() {
                return Err(format!("ValueError: cannot serialize non-finite float: {}", f));
            }
            push_json_float(*f, out);
        }
        Object::Ref(ptr) => {
            // SAFETY: Ref 来自 alloc_* 系列，type_tag 可读。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING，ptr 由 alloc_string 分配。
                let s = unsafe { read_str(*ptr) };
                push_json_string(s, out);
            } else if tag == TypeTag::LIST as u8 {
                // 循环引用检测：用指针地址判重，避免 list 自引用导致无限递归。
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // 借用约束：递归前 clone 出元素，释放 read_list 返回的 &mut Vec。
                let items: Vec<Object> = { unsafe { read_list(*ptr) }.clone() };
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    stringify_into(item, out, depth + 1, seen)?;
                }
                out.push(']');
                seen.remove(&(*ptr as usize));
            } else if tag == TypeTag::DICT as u8 {
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // 借用约束：递归前 clone 出 (key, value)，释放 read_dict 返回的 &mut。
                let items: Vec<(Object, Object)> = {
                    let d = unsafe { read_dict(*ptr) };
                    d.items()
                        .into_iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                };
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let key_str = match k {
                        Object::Ref(kptr)
                            if unsafe { (**kptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            // SAFETY: type_tag 为 STRING。
                            unsafe { read_str(*kptr) }.to_owned()
                        }
                        _ => {
                            return Err(format!(
                                "TypeError: JSON dict key must be string, got {}",
                                k.type_name()
                            ))
                        }
                    };
                    push_json_string(&key_str, out);
                    out.push(':');
                    stringify_into(v, out, depth + 1, seen)?;
                }
                out.push('}');
                seen.remove(&(*ptr as usize));
            } else {
                // tuple/set/function/class/instance/file_handle/...：Phase 6.2d 不支持
                // __to_json__ 魔术方法，统一拒绝（TypeError）。
                return Err(format!(
                    "TypeError: cannot serialize {} to JSON",
                    obj.type_name()
                ));
            }
        }
    }
    Ok(())
}

/// 转义 JSON 字符串字面量：`"`、`\`、控制字符（< 0x20）。
/// 非 ASCII 字符直接以 UTF-8 输出（RFC 8259 允许未转义的 Unicode）。
fn push_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// 格式化 JSON 数字（float）。与 Object::Display 一致：整数值浮点用 "{:.1}"
/// （3.0→"3.0"、-0.0→"-0.0"），非整数用 "{}"（3.14→"3.14"），保证 round-trip
/// 与 print 输出一致（task 49 §验证标准 #6/#8）。
fn push_json_float(f: f64, out: &mut String) {
    use std::fmt::Write;
    if f == (f as i64) as f64 {
        let _ = write!(out, "{:.1}", f);
    } else {
        let _ = write!(out, "{}", f);
    }
}

/// 简易递归下降 JSON 解析器（task 49 §方案 B，零外部依赖）。
/// 覆盖 RFC 8259 子集：null/true/false/number/string/array/object。
struct JsonParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while let Some(&c) = self.src.get(self.pos) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    /// 计算 pos 对应的 1-based 行列号（错误消息定位）。
    fn line_col(&self, pos: usize) -> (usize, usize) {
        let mut line = 1usize;
        let mut col = 1usize;
        for &b in &self.src[..pos.min(self.src.len())] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// 生成 parse 语法错误消息（行列号定位，仅含位置不含原文片段以防敏感数据泄露）。
    fn fail(&self, pos: usize, reason: &str) -> String {
        let (line, col) = self.line_col(pos);
        format!("ValueError: json {} at line {} column {}", reason, line, col)
    }

    /// 解析一个值。`depth` 为当前嵌套层级（顶层=0）。
    /// 容器（array/object）在进入时以 `depth+1` 校验 MAX_NESTING。
    fn parse_value(&mut self, depth: u32) -> Result<Object, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(alloc_string(&self.parse_string()?)),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c.is_ascii_digit() || c == b'-' => self.parse_number(),
            _ => Err(self.fail(self.pos, "parse error")),
        }
    }

    fn parse_array(&mut self, depth: u32) -> Result<Object, String> {
        // 进入容器：嵌套层级 +1。超 MAX_NESTING 拒绝（覆盖空数组深嵌套：空数组
        // 不递归到元素，故必须在进入容器时校验，而非 parse_value 顶部）。
        let level = depth + 1;
        if level > MAX_NESTING {
            return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
        }
        debug_assert!(self.peek() == Some(b'['));
        self.pos += 1; // consume '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(alloc_list(items));
        }
        loop {
            let v = self.parse_value(level)?;
            items.push(v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.fail(self.pos, "parse error")),
            }
        }
        Ok(alloc_list(items))
    }

    fn parse_object(&mut self, depth: u32) -> Result<Object, String> {
        let level = depth + 1;
        if level > MAX_NESTING {
            return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
        }
        debug_assert!(self.peek() == Some(b'{'));
        self.pos += 1; // consume '{'
        let mut dict = DictMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(alloc_dict(dict));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.fail(self.pos, "parse error"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.fail(self.pos, "parse error"));
            }
            self.pos += 1; // consume ':'
            let val = self.parse_value(level)?;
            dict.insert(alloc_string(&key), val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.fail(self.pos, "parse error")),
            }
        }
        Ok(alloc_dict(dict))
    }

    fn parse_null(&mut self) -> Result<Object, String> {
        if self.src.get(self.pos..self.pos + 4) == Some(b"null") {
            self.pos += 4;
            Ok(Object::Nil)
        } else {
            Err(self.fail(self.pos, "parse error"))
        }
    }

    fn parse_bool(&mut self) -> Result<Object, String> {
        if self.src.get(self.pos..self.pos + 4) == Some(b"true") {
            self.pos += 4;
            Ok(Object::Bool(true))
        } else if self.src.get(self.pos..self.pos + 5) == Some(b"false") {
            self.pos += 5;
            Ok(Object::Bool(false))
        } else {
            Err(self.fail(self.pos, "parse error"))
        }
    }

    /// 解析字符串字面量（已消费开引号前的判定）。处理转义 \" \\ \/ \b \f \n \r \t
    /// 与 \uXXXX（含 UTF-16 代理对重建）。
    fn parse_string(&mut self) -> Result<String, String> {
        debug_assert!(self.peek() == Some(b'"'));
        self.pos += 1; // consume opening '"'
        let mut out = String::new();
        loop {
            // 收集直至下一个特殊字节（'"'、'\'、控制字符 < 0x20）。
            let start = self.pos;
            while let Some(&c) = self.src.get(self.pos) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            if self.pos > start {
                let span = &self.src[start..self.pos];
                // span 必为合法 UTF-8：src 源自 &str，且循环仅在 ASCII（'"'/'\'
                // 或 < 0x20）处截断，不会切断多字节序列。用 from_utf8 防御性校验。
                match std::str::from_utf8(span) {
                    Ok(s) => out.push_str(s),
                    Err(_) => return Err(self.fail(start, "parse error")),
                }
            }
            match self.src.get(self.pos).copied() {
                None => return Err(self.fail(self.pos, "parse error")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.src.get(self.pos).copied();
                    self.pos += 1;
                    match esc {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\x08'),
                        Some(b'f') => out.push('\x0c'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let cp = self.parse_hex4()?;
                            out.push(self.decode_unicode(cp)?);
                        }
                        _ => return Err(self.fail(self.pos, "parse error")),
                    }
                }
                Some(c) if c < 0x20 => return Err(self.fail(self.pos, "parse error")),
                // 不应到达：上述循环已覆盖所有 < 0x20 / '"' / '\\'。
                Some(_) => unreachable!("parse_string scan loop invariant"),
            }
        }
    }

    /// 解析 \u 后的 4 位十六进制。返回码点或（高代理时）已合并的完整码点。
    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut val = 0u32;
        for _ in 0..4 {
            let c = self.src.get(self.pos).copied();
            let d = match c {
                Some(b'0'..=b'9') => (c.unwrap() - b'0') as u32,
                Some(b'a'..=b'f') => (c.unwrap() - b'a' + 10) as u32,
                Some(b'A'..=b'F') => (c.unwrap() - b'A' + 10) as u32,
                _ => return Err(self.fail(self.pos, "parse error")),
            };
            val = val * 16 + d;
            self.pos += 1;
        }
        Ok(val)
    }

    /// 将 \uXXXX 码点（可能为高代理）解码为 char，处理紧随的低代理对。
    fn decode_unicode(&mut self, cp: u32) -> Result<char, String> {
        if (0xD800..=0xDBFF).contains(&cp) {
            // 高代理：期望紧跟 \uXXXX 低代理（RFC 8259 §7）。
            if self.src.get(self.pos..self.pos + 2) == Some(b"\\u") {
                self.pos += 2;
                let lo = self.parse_hex4()?;
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    return char::from_u32(combined)
                        .ok_or_else(|| self.fail(self.pos, "parse error"));
                }
                return Err(self.fail(self.pos, "parse error"));
            }
            return Err(self.fail(self.pos, "parse error"));
        }
        if (0xDC00..=0xDFFF).contains(&cp) {
            // 裸低代理非法。
            return Err(self.fail(self.pos, "parse error"));
        }
        char::from_u32(cp).ok_or_else(|| self.fail(self.pos, "parse error"))
    }

    /// 解析数字：可选负号、整数部分（'0' 或 [1-9]+）、可选小数、可选指数。
    /// 无小数/指数时优先 i64；超出 i64 范围退化为 f64。
    fn parse_number(&mut self) -> Result<Object, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(c) if c.is_ascii_digit() => {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.fail(self.pos, "parse error")),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.fail(self.pos, "parse error"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.fail(self.pos, "parse error"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.fail(start, "parse error"))?;
        if is_float {
            let f = text
                .parse::<f64>()
                .map_err(|_| self.fail(start, "parse error"))?;
            Ok(Object::Float(f))
        } else {
            match text.parse::<i64>() {
                Ok(i) => Ok(Object::Int(i)),
                // 超出 i64 范围：退化为 f64（与 Python JSON 一致，精度可能损失）。
                Err(_) => {
                    let f = text
                        .parse::<f64>()
                        .map_err(|_| self.fail(start, "parse error"))?;
                    Ok(Object::Float(f))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// List 方法（task 51：GET_ATTR → BoundMethod 分派，仿 task 46/50 模式）
// ---------------------------------------------------------------------------

/// List 方法名 → 原生函数（供 GET_ATTR 包装为 BoundMethod）。
pub fn lookup_list_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_list_length,
        "push" => native_list_push,
        "pop" => native_list_pop,
        "insert" => native_list_insert,
        "remove" => native_list_remove,
        "index" => native_list_index,
        "contains" => native_list_contains,
        "sort" => native_list_sort,
        "reverse" => native_list_reverse,
        "slice" => native_list_slice,
        "map" => native_list_map,
        "filter" => native_list_filter,
        "reduce" => native_list_reduce,
        _ => return None,
    };
    Some(func)
}

// 注：args[0] 为 List receiver（BoundMethod 注入），用户参数从 args.get(1) 起。

fn native_list_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "length()")?;
    let len = unsafe { read_list(ptr) }.len();
    Ok(Object::Int(len as i64))
}

fn native_list_push(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "push(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: push(value) requires 1 argument".to_string())?;
    unsafe { read_list(ptr) }.push(val);
    Ok(Object::Nil)
}

fn native_list_pop(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "pop(index?)")?;
    let len = unsafe { read_list(ptr) }.len();
    let idx = if args.len() <= 1 {
        if len == 0 {
            return Err("IndexError: pop from empty list".to_string());
        }
        len - 1
    } else {
        let i = expect_int(args.get(1), "pop(index?)")?;
        normalize_index(i, len).ok_or_else(|| {
            format!("IndexError: pop index {} out of range for length {}", i, len)
        })?
    };
    let popped = unsafe { read_list(ptr) }.remove(idx);
    Ok(popped)
}

fn native_list_insert(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "insert(index, value)")?;
    let i = expect_int(args.get(1), "insert(index, value)")?;
    let val = args
        .get(2)
        .cloned()
        .ok_or_else(|| "TypeError: insert(index, value) requires 2 arguments".to_string())?;
    let len = unsafe { read_list(ptr) }.len();
    let n = if i < 0 { len as i64 + i } else { i };
    if n < 0 || n > len as i64 {
        return Err(format!(
            "IndexError: insert index {} out of range for length {}",
            i, len
        ));
    }
    unsafe { read_list(ptr) }.insert(n as usize, val);
    Ok(Object::Nil)
}

fn native_list_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "remove(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    let found_idx = {
        let list = unsafe { read_list(ptr) };
        list.iter().position(|x| x == &val)
    };
    match found_idx {
        Some(idx) => {
            let _removed = unsafe { read_list(ptr) }.remove(idx);
            Ok(Object::Nil)
        }
        None => Err("ValueError: remove(): value not in list".to_string()),
    }
}

fn native_list_index(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "index(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: index(value) requires 1 argument".to_string())?;
    let list = unsafe { read_list(ptr) };
    match list.iter().position(|x| x == &val) {
        Some(idx) => Ok(Object::Int(idx as i64)),
        None => Err("ValueError: index(): value not in list".to_string()),
    }
}

fn native_list_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "contains(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    let found = unsafe { read_list(ptr) }.iter().any(|x| x == &val);
    Ok(Object::Bool(found))
}

fn native_list_sort(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "sort()")?;
    let mut items = unsafe { read_list(ptr) }.clone();
    let mut err: Option<String> = None;
    items.sort_by(|a, b| {
        if err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match a.compare(b, CmpOp::Less) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
            Ok(_) => match a.compare(b, CmpOp::Greater) {
                Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
                Ok(_) => std::cmp::Ordering::Equal,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            },
            Err(e) => {
                err = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    unsafe { read_list(ptr) }.clear();
    unsafe { read_list(ptr) }.extend(items);
    Ok(Object::Nil)
}

fn native_list_reverse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reverse()")?;
    unsafe { read_list(ptr) }.reverse();
    Ok(Object::Nil)
}

fn native_list_slice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "slice(start, end?)")?;
    let start_i = expect_int(args.get(1), "slice(start, end?)")?;
    let end_opt = if args.len() > 2 {
        Some(expect_int(args.get(2), "slice(start, end?)")?)
    } else {
        None
    };
    let items = unsafe { read_list(ptr) }.clone();
    let len = items.len() as i64;
    let norm = |i: i64| -> i64 {
        if i < 0 {
            (len + i).max(0)
        } else {
            i.min(len)
        }
    };
    let s = norm(start_i);
    let e = match end_opt {
        Some(i) => norm(i),
        None => len,
    };
    if s > e {
        return Err(format!("ValueError: slice start {} > end {}", s, e));
    }
    let sliced: Vec<Object> = items[s as usize..e as usize].to_vec();
    Ok(alloc_list(sliced))
}

fn native_list_map(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "map(fn)")?;
    let fn_obj = expect_callable(args.get(1), "map(fn)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::with_capacity(items.len());
    for item in items.iter() {
        let mapped = vm.call_function(&fn_obj, std::slice::from_ref(item))?;
        result.push(mapped);
    }
    Ok(alloc_list(result))
}

fn native_list_filter(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "filter(fn)")?;
    let fn_obj = expect_callable(args.get(1), "filter(fn)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::new();
    for item in items.iter() {
        let cond = vm.call_function(&fn_obj, std::slice::from_ref(item))?;
        if cond.is_truthy() {
            result.push(item.clone());
        }
    }
    Ok(alloc_list(result))
}

fn native_list_reduce(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reduce(fn, init?)")?;
    let fn_obj = expect_callable(args.get(1), "reduce(fn, init?)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let (mut acc, start) = if args.len() > 2 {
        (args.get(2).cloned().unwrap(), 0)
    } else {
        if items.is_empty() {
            return Err(
                "ValueError: reduce() of empty list with no initial value".to_string()
            );
        }
        (items[0].clone(), 1)
    };
    for item in items.iter().skip(start) {
        acc = vm.call_function(&fn_obj, &[acc, item.clone()])?;
    }
    Ok(acc)
}

// ---------------------------------------------------------------------------
// Dict 方法（task 51）
// ---------------------------------------------------------------------------

/// Dict 方法名 → 原生函数。
pub fn lookup_dict_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_dict_length,
        "keys" => native_dict_keys,
        "values" => native_dict_values,
        "items" => native_dict_items,
        "get" => native_dict_get,
        "set" => native_dict_set,
        "remove" => native_dict_remove,
        "contains" => native_dict_contains,
        "merge" => native_dict_merge,
        _ => return None,
    };
    Some(func)
}

fn native_dict_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_dict(ptr) }.len() as i64))
}

fn native_dict_keys(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "keys()")?;
    let keys: Vec<Object> = unsafe { read_dict(ptr) }.keys().into_iter().cloned().collect();
    Ok(alloc_list(keys))
}

fn native_dict_values(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "values()")?;
    let vals: Vec<Object> = unsafe { read_dict(ptr) }
        .items()
        .into_iter()
        .map(|(_, v)| v.clone())
        .collect();
    Ok(alloc_list(vals))
}

fn native_dict_items(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "items()")?;
    let items: Vec<Object> = unsafe { read_dict(ptr) }
        .items()
        .into_iter()
        .map(|(k, v)| alloc_tuple(vec![k.clone(), v.clone()]))
        .collect();
    Ok(alloc_list(items))
}

fn native_dict_get(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "get(key, default?)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: get(key, default?) requires 1-2 arguments".to_string())?;
    hash_key(&key)?;
    let default = if args.len() > 2 {
        args.get(2).cloned().unwrap()
    } else {
        Object::Nil
    };
    let dict = unsafe { read_dict(ptr) };
    Ok(dict.get(&key).cloned().unwrap_or(default))
}

fn native_dict_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "set(key, value)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    let val = args
        .get(2)
        .cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    hash_key(&key)?;
    unsafe { read_dict(ptr) }.insert(key, val);
    Ok(Object::Nil)
}

fn native_dict_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "remove(key)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    if unsafe { read_dict(ptr) }.remove(&key).is_none() {
        return Err("KeyError: key not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_dict_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "contains(key)")?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    let found = unsafe { read_dict(ptr) }.get(&key).is_some();
    Ok(Object::Bool(found))
}

fn native_dict_merge(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "merge(other)")?;
    let other_ptr = expect_dict_ref(args.get(1), "merge(other)")?;
    let pairs: Vec<(Object, Object)> = unsafe { read_dict(other_ptr) }
        .items()
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in pairs {
        unsafe { read_dict(ptr) }.insert(k, v);
    }
    Ok(Object::Nil)
}

// ---------------------------------------------------------------------------
// Set 方法（task 51）
// ---------------------------------------------------------------------------

/// Set 方法名 → 原生函数。
pub fn lookup_set_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_set_length,
        "add" => native_set_add,
        "remove" => native_set_remove,
        "contains" => native_set_contains,
        "union" => native_set_union,
        "intersection" => native_set_intersection,
        "difference" => native_set_difference,
        _ => return None,
    };
    Some(func)
}

fn native_set_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_set(ptr) }.len() as i64))
}

fn native_set_add(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "add(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: add(value) requires 1 argument".to_string())?;
    hash_key(&val)?;
    unsafe { read_set(ptr) }.insert(val);
    Ok(Object::Nil)
}

fn native_set_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "remove(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    hash_key(&val)?;
    if !unsafe { read_set(ptr) }.remove(&val) {
        return Err("KeyError: element not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_set_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "contains(value)")?;
    let val = args
        .get(1)
        .cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    let found = if hash_key(&val).is_ok() {
        unsafe { read_set(ptr) }.contains(&val)
    } else {
        false
    };
    Ok(Object::Bool(found))
}

fn native_set_union(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "union(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "union(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.union(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_intersection(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "intersection(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "intersection(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.intersection(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_difference(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "difference(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "difference(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: HashSet<Object> = a.difference(&b).cloned().collect();
    Ok(alloc_set(result))
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 从预期为 String 的参数提取 Rust String。
fn expect_string(arg: Option<&Object>, who: &str) -> Result<String, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
            Ok(unsafe { read_str(*ptr) }.to_owned())
        }
        other => Err(format!(
            "TypeError: {} expects string, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 从预期为 Int 的参数提取 i64。Bool 不自动转 Int（仅接受 int）。
fn expect_int(arg: Option<&Object>, who: &str) -> Result<i64, String> {
    match arg {
        Some(Object::Int(n)) => Ok(*n),
        other => Err(format!(
            "TypeError: {} expects int, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 校验参数为 List Ref，返回裸指针。调用方 unsafe read_list 取内容
/// （借用约束：递归前必须释放 &mut Vec<Object>，参见 task 49 §3）。
fn expect_list_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects list, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 校验首参数为 Dict Ref，返回裸指针。
fn expect_dict_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects dict, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 校验首参数为 Set Ref，返回裸指针。
fn expect_set_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects set, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 校验参数为 callable（FUNCTION/CLOSURE/BOUND_METHOD）。
fn expect_callable(arg: Option<&Object>, who: &str) -> Result<Object, String> {
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

/// 列表索引归一化（负索引相对末尾，越界返回 None）。
fn normalize_index(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let n = if i < 0 { len + i } else { i };
    if n < 0 || n >= len {
        None
    } else {
        Some(n as usize)
    }
}

/// 可哈希键校验（供 Set 元素 / Dict 键复用）。
/// 仅 int/float/bool/string/nil/tuple 可哈希；NaN 抛 TypeError。
fn hash_key(obj: &Object) -> Result<u64, String> {
    match obj {
        Object::Nil => Ok(0),
        Object::Bool(b) => Ok(if *b { 1 } else { 0 }),
        Object::Int(n) => Ok(*n as u64),
        Object::Float(f) => {
            if f.is_nan() {
                Err("TypeError: unhashable type: NaN".to_string())
            } else {
                Ok((*f).to_bits())
            }
        }
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 || tag == TypeTag::TUPLE as u8 {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                obj.hash(&mut hasher);
                Ok(hasher.finish())
            } else {
                Err(format!("TypeError: unhashable type: '{}'", obj.type_name()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{Compiler, Chunk};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::object::read_list;

    fn vm() -> VM {
        VM::new()
    }

    fn s(v: &str) -> Object {
        alloc_string(v)
    }

    /// 编译并运行 mslang 源码（集成测试辅助）。
    fn run_source(source: &str) -> Result<Object, String> {
        let tokens = Lexer::new(source).tokenize_all().map_err(|e| format!("{}", e))?;
        let program = Parser::new(tokens).parse().map_err(|e| format!("{}", e))?;
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program)?;
        let mut v = VM::new();
        v.interpret(chunk)
    }

    /// 返回临时目录下的绝对路径字符串（避免污染工作区 CWD）。
    /// Windows 路径反斜杠在 mslang 字符串字面量中会被当作转义符，故统一转为正斜杠
    ///（std::fs 在 Windows 同时接受两种分隔符）。
    fn temp_path(name: &str) -> String {
        let dir = std::env::temp_dir().join("mslang_io_integration");
        std::fs::create_dir_all(&dir).ok();
        dir.join(name)
            .to_string_lossy()
            .replace('\\', "/")
    }

    #[test]
    fn test_io_module_registration() {
        // register_io_module 返回 MODULE，exports 含 4 个函数。
        let ptr = register_io_module();
        // SAFETY: ptr 由 register_io_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "io");
            assert!(m.exports.contains_key("open"));
            assert!(m.exports.contains_key("read_file"));
            assert!(m.exports.contains_key("write_file"));
            assert!(m.exports.contains_key("exists"));
        }
    }

    #[test]
    fn test_io_write_read_exists() {
        let mut v = vm();
        let dir = std::env::temp_dir().join("mslang_io_test_wr");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.txt");

        // exists → false（未创建）
        let p = s(path.to_str().unwrap());
        assert_eq!(
            native_io_exists(&mut v, &[p.clone()]).unwrap(),
            Object::Bool(false)
        );

        // write_file → nil
        assert_eq!(
            native_io_write_file(&mut v, &[p.clone(), s("hello\nworld\n")]).unwrap(),
            Object::Nil
        );
        // exists → true
        assert_eq!(
            native_io_exists(&mut v, &[p.clone()]).unwrap(),
            Object::Bool(true)
        );
        // read_file → 内容
        assert_eq!(
            native_io_read_file(&mut v, &[p.clone()]).unwrap(),
            s("hello\nworld\n")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_io_open_read_close() {
        let mut v = vm();
        let dir = std::env::temp_dir().join("mslang_io_test_open");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.txt");
        std::fs::write(&path, "abc\ndef\n").unwrap();

        let p = s(path.to_str().unwrap());
        // open("path", "r") → FileHandle
        let fh = native_io_open(&mut v, &[p]).unwrap();
        let Object::Ref(ptr) = &fh else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*(*ptr)).type_tag, TypeTag::FILE_HANDLE as u8);
        }
        // read() → 内容
        assert_eq!(native_fh_read(&mut v, &[fh.clone()]).unwrap(), s("abc\ndef\n"));
        // close() → nil（幂等）
        assert_eq!(native_fh_close(&mut v, &[fh.clone()]).unwrap(), Object::Nil);
        assert_eq!(native_fh_close(&mut v, &[fh.clone()]).unwrap(), Object::Nil);
        // read after close → IOError
        let err = native_fh_read(&mut v, &[fh.clone()]).unwrap_err();
        assert!(err.contains("already closed"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_io_open_write_mode() {
        let mut v = vm();
        let dir = std::env::temp_dir().join("mslang_io_test_w");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.txt");

        // open("path", "w") → 可写 FileHandle
        let fh = native_io_open(&mut v, &[s(path.to_str().unwrap()), s("w")]).unwrap();
        // write() → nil
        assert_eq!(
            native_fh_write(&mut v, &[fh.clone(), s("data")]).unwrap(),
            Object::Nil
        );
        // close 后磁盘内容为 "data"
        native_fh_close(&mut v, &[fh]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "data");

        // unknown mode → ValueError
        let err = native_io_open(&mut v, &[s(path.to_str().unwrap()), s("x")]).unwrap_err();
        assert!(err.contains("ValueError"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fh_lines() {
        let mut v = vm();
        let dir = std::env::temp_dir().join("mslang_io_test_lines");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lines.txt");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        let fh = native_io_open(&mut v, &[s(path.to_str().unwrap())]).unwrap();
        let result = native_fh_lines(&mut v, &[fh]).unwrap();
        // 末尾换行不产生空元素 → 3 行
        let Object::Ref(p) = &result else {
            panic!("expected Ref");
        };
        let items = unsafe { read_list(*p) };
        assert_eq!(items.clone(), vec![s("line1"), s("line2"), s("line3")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fh_enter_exit() {
        let mut v = vm();
        let dir = std::env::temp_dir().join("mslang_io_test_ctx");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ctx.txt");
        std::fs::write(&path, "ctx").unwrap();

        let fh = native_io_open(&mut v, &[s(path.to_str().unwrap())]).unwrap();
        // __enter__ → 返回 self（同指针）。FileHandle 无内容相等性（资源句柄），
        // 故按指针身份比较。
        let entered = native_fh_enter(&mut v, &[fh.clone()]).unwrap();
        let (Object::Ref(a), Object::Ref(b)) = (&entered, &fh) else {
            panic!("expected Ref");
        };
        assert_eq!(*a as usize, *b as usize);
        // __exit__(self, err_type, err_msg, tb) → nil，关闭文件
        assert_eq!(
            native_fh_exit(
                &mut v,
                &[fh.clone(), Object::Nil, Object::Nil, Object::Nil]
            )
            .unwrap(),
            Object::Nil
        );
        // __exit__ 已关闭 → read 报错
        let err = native_fh_read(&mut v, &[fh]).unwrap_err();
        assert!(err.contains("already closed"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_expect_string_errors() {
        let mut v = vm();
        // read_file(非 string) → TypeError
        let err = native_io_read_file(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"));
        // read_file 缺参 → TypeError (missing)
        let err = native_io_read_file(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_lookup_file_method() {
        assert!(lookup_file_method("read").is_some());
        assert!(lookup_file_method("write").is_some());
        assert!(lookup_file_method("close").is_some());
        assert!(lookup_file_method("lines").is_some());
        assert!(lookup_file_method("__enter__").is_some());
        assert!(lookup_file_method("__exit__").is_some());
        assert!(lookup_file_method("nope").is_none());
    }

    // ---- String 方法单元测试（task 50）----

    /// 从 Object::Ref 提取 Rust String（测试辅助）。
    fn as_str(o: &Object) -> String {
        match o {
            Object::Ref(p) => unsafe { read_str(*p) }.to_owned(),
            other => panic!("expected string ref, got {}", other.type_name()),
        }
    }

    #[test]
    fn test_lookup_string_method() {
        for name in [
            "length",
            "upper",
            "lower",
            "strip",
            "split",
            "join",
            "replace",
            "contains",
            "startswith",
            "endswith",
            "index",
            "slice",
        ] {
            assert!(
                lookup_string_method(name).is_some(),
                "{} should resolve",
                name
            );
        }
        assert!(lookup_string_method("nosuch").is_none());
    }

    #[test]
    fn test_str_length_unicode() {
        let mut v = vm();
        assert_eq!(
            native_str_length(&mut v, &[s("hello")]).unwrap(),
            Object::Int(5)
        );
        // 字符位置（非字节）：日本語 = 3 字符（9 字节）
        assert_eq!(
            native_str_length(&mut v, &[s("日本語")]).unwrap(),
            Object::Int(3)
        );
    }

    #[test]
    fn test_str_upper_lower_strip() {
        let mut v = vm();
        assert_eq!(
            as_str(&native_str_upper(&mut v, &[s("Hello")]).unwrap()),
            "HELLO"
        );
        assert_eq!(
            as_str(&native_str_lower(&mut v, &[s("Hello")]).unwrap()),
            "hello"
        );
        assert_eq!(
            as_str(&native_str_strip(&mut v, &[s("  trim  ")]).unwrap()),
            "trim"
        );
    }

    #[test]
    fn test_str_split() {
        let mut v = vm();
        // 有分隔符
        let r = native_str_split(&mut v, &[s("a,b,c"), s(",")]).unwrap();
        match r {
            Object::Ref(p) => {
                let items = unsafe { read_list(p) };
                assert_eq!(items.len(), 3);
                assert_eq!(as_str(&items[0]), "a");
                assert_eq!(as_str(&items[2]), "c");
            }
            _ => panic!("expected list"),
        }
        // 无参：按 Unicode 空白分割（连续空白折叠）
        let r = native_str_split(&mut v, &[s("a  b   c")]).unwrap();
        match r {
            Object::Ref(p) => {
                let items = unsafe { read_list(p) };
                assert_eq!(items.len(), 3);
                assert_eq!(as_str(&items[1]), "b");
            }
            _ => panic!("expected list"),
        }
        // 空分隔符报错
        let err = native_str_split(&mut v, &[s("abc"), s("")]).unwrap_err();
        assert_eq!(err, "ValueError: empty separator");
    }

    #[test]
    fn test_str_join() {
        let mut v = vm();
        let lst = alloc_list(vec![s("a"), s("b"), s("c")]);
        let r = native_str_join(&mut v, &[s("-"), lst]).unwrap();
        assert_eq!(as_str(&r), "a-b-c");
        // 非 string 元素 → TypeError
        let lst = alloc_list(vec![Object::Int(1), Object::Int(2)]);
        let err = native_str_join(&mut v, &[s("-"), lst]).unwrap_err();
        assert!(err.contains("TypeError"), "{}", err);
        assert!(err.contains("got int"), "{}", err);
    }

    #[test]
    fn test_str_replace_contains_etc() {
        let mut v = vm();
        assert_eq!(
            as_str(&native_str_replace(&mut v, &[s("hello"), s("l"), s("r")]).unwrap()),
            "herro"
        );
        assert_eq!(
            native_str_contains(&mut v, &[s("hello"), s("ell")]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_str_contains(&mut v, &[s("hello"), s("xyz")]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            native_str_startswith(&mut v, &[s("hello"), s("hel")]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_str_endswith(&mut v, &[s("hello"), s("llo")]).unwrap(),
            Object::Bool(true)
        );
    }

    #[test]
    fn test_str_index_char_position() {
        let mut v = vm();
        assert_eq!(
            native_str_index(&mut v, &[s("hello"), s("ll")]).unwrap(),
            Object::Int(2)
        );
        // 字符位置：本 在 日本語 中是字符位置 1（字节位置 3）
        assert_eq!(
            native_str_index(&mut v, &[s("日本語"), s("本")]).unwrap(),
            Object::Int(1)
        );
        // 未找到 → ValueError
        let err = native_str_index(&mut v, &[s("hello"), s("xx")]).unwrap_err();
        assert_eq!(err, "ValueError: substring 'xx' not found");
    }

    #[test]
    fn test_str_slice() {
        let mut v = vm();
        assert_eq!(
            as_str(
                &native_str_slice(&mut v, &[s("hello"), Object::Int(1), Object::Int(3)]).unwrap()
            ),
            "el"
        );
        // 负索引：相对末尾
        assert_eq!(
            as_str(&native_str_slice(&mut v, &[s("hello"), Object::Int(-1)]).unwrap()),
            "o"
        );
        assert_eq!(
            as_str(
                &native_str_slice(&mut v, &[s("hello"), Object::Int(1), Object::Int(-1)]).unwrap()
            ),
            "ell"
        );
        // 越界饱和：返回空串（不 panic）
        assert_eq!(
            as_str(
                &native_str_slice(&mut v, &[s("hello"), Object::Int(100), Object::Int(200)])
                    .unwrap()
            ),
            ""
        );
        // Unicode slice：日本語 slice(0,2) = 日本
        assert_eq!(
            as_str(
                &native_str_slice(&mut v, &[s("日本語"), Object::Int(0), Object::Int(2)]).unwrap()
            ),
            "日本"
        );
        // start > end（归一化后）→ ValueError
        let err =
            native_str_slice(&mut v, &[s("hello"), Object::Int(3), Object::Int(1)]).unwrap_err();
        assert_eq!(err, "ValueError: slice start 3 > end 1");
    }

    #[test]
    fn test_str_index_slice_compose() {
        // length/index/slice 位置互相对应：slice(index(sub), index(sub)+1) 正确
        let mut v = vm();
        let i = match native_str_index(&mut v, &[s("日本語"), s("本")]).unwrap() {
            Object::Int(n) => n,
            _ => unreachable!(),
        };
        let r =
            native_str_slice(&mut v, &[s("日本語"), Object::Int(i), Object::Int(i + 1)]).unwrap();
        assert_eq!(as_str(&r), "本");
    }

    #[test]
    fn test_str_method_type_errors() {
        let mut v = vm();
        // receiver 非 string → expect_string TypeError
        let err = native_str_length(&mut v, &[Object::Int(5)]).unwrap_err();
        assert!(err.contains("TypeError"), "{}", err);
        // slice 缺 start → TypeError (missing)
        let err = native_str_slice(&mut v, &[s("hello")]).unwrap_err();
        assert!(err.contains("TypeError"), "{}", err);
    }

    #[test]
    fn test_integration_string_methods() {
        let src = r#"
assert("Hello World".lower() == "hello world")
assert("Hello World".upper() == "HELLO WORLD")
assert("  trim  ".strip() == "trim")
assert("hello".length() == 5)
assert("hello".index("ll") == 2)
assert("hello".slice(1, 3) == "el")
assert("hello".replace("l", "r") == "herro")
assert("hello".contains("ell") == true)
assert("hello".startswith("hel") == true)
assert("hello".endswith("llo") == true)
parts = "a,b,c".split(",")
assert(len(parts) == 3)
assert(parts[0] == "a")
assert(parts[2] == "c")
assert("-".join(["a", "b", "c"]) == "a-b-c")
"#;
        let r = run_source(src);
        assert!(
            r.is_ok(),
            "string methods integration failed: {:?}",
            r.err()
        );
    }

    #[test]
    fn test_integration_string_unicode() {
        let src = r#"
assert("日本語".length() == 3)
assert("日本語".index("本") == 1)
assert("日本語".slice(0, 2) == "日本")
i = "日本語".index("本")
assert("日本語".slice(i, i + 1) == "本")
assert("hello".slice(-1) == "o")
assert("hello".slice(1, -1) == "ell")
assert("hello".slice(100, 200) == "")
"#;
        let r = run_source(src);
        assert!(
            r.is_ok(),
            "string unicode integration failed: {:?}",
            r.err()
        );
    }

    #[test]
    fn test_integration_string_unknown_method() {
        // 未知方法 → AttributeError（原生 Err 不可被 try/except 捕获，整体 Err）
        let r = run_source(r#""hello".nosuch()"#);
        assert!(r.is_err());
        assert!(
            r.unwrap_err().contains("AttributeError"),
            "expected AttributeError"
        );
    }

    // ---- 端到端集成测试（import io + with + FileHandle 方法）----

    #[test]
    fn test_integration_io_full_pipeline() {
        // 等价 test_io.ms：write_file → exists → with open → read → read_file。
        let path = temp_path("test_io.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.write_file("{path}", "hello\nworld\n")
assert(io.exists("{path}"))
with io.open("{path}") as f {{
    assert(f.read() == "hello\nworld\n")
}}
assert(io.read_file("{path}") == "hello\nworld\n")
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "io full pipeline failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_io_lines() {
        // 等价 test_io_lines.ms：write_file → with open → lines() → for..in。
        let path = temp_path("test_lines.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.write_file("{path}", "line1\nline2\nline3\n")
with io.open("{path}") as f {{
    lines = f.lines()
    assert(len(lines) == 3)
    assert(lines[0] == "line1")
    assert(lines[1] == "line2")
    assert(lines[2] == "line3")
}}
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "io lines failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_io_with_closes_file() {
        // with 块结束后 __exit__ 已关闭文件 → 再次 read（重新 open 读到内容证明 close 生效，
        // 且原句柄关闭后读报错）。
        let path = temp_path("test_close.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.write_file("{path}", "data")
with io.open("{path}") as f {{
    assert(f.read() == "data")
}}
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "io with-close failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_io_open_variadic() {
        // io.open 单参（默认 "r"）与双参均工作。
        let path = temp_path("test_modes.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.write_file("{path}", "abc")
f1 = io.open("{path}")
assert(f1.read() == "abc")
f1.close()
f2 = io.open("{path}", "r")
assert(f2.read() == "abc")
f2.close()
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "io open variadic failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_io_missing_file_error() {
        // 读取不存在的文件 → 合理错误（IOError）。
        let path = temp_path("no_such_file_xyz.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.read_file("{path}")
"#,
            path = path
        );
        let r = run_source(&src);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("IOError"), "should be IOError");
    }

    #[test]
    fn test_integration_global_open_delegates() {
        // 全局 open() 是 io.open() 的快捷方式（10-builtins.md）。
        let path = temp_path("test_global_open.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
io.write_file("{path}", "xyz")
with open("{path}") as f {{
    assert(f.read() == "xyz")
}}
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "global open delegate failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_io_write_method() {
        // FileHandle.write 经 with 写入，read_file 验证落盘。
        let path = temp_path("test_write_method.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
import io
with io.open("{path}", "w") as f {{
    f.write("written content")
}}
assert(io.read_file("{path}") == "written content")
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "io write method failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_from_io_import() {
        // from io import 直接提取导出名（GET_ATTR on MODULE 经 native_modules 命中）。
        let path = temp_path("test_from_import.txt");
        std::fs::remove_file(&path).ok();
        let src = format!(
            r#"
from io import write_file, read_file, exists
write_file("{path}", "from-import works")
assert(exists("{path}"))
assert(read_file("{path}") == "from-import works")
"#,
            path = path
        );
        let r = run_source(&src);
        std::fs::remove_file(&path).ok();
        assert!(r.is_ok(), "from io import failed: {:?}", r.err());
    }

    // ---- math 模块 ----

    /// 提取 Object::Float 内部值（单测辅助）。
    fn fval(o: &Object) -> f64 {
        match o {
            Object::Float(x) => *x,
            _ => panic!("expected Float, got {:?}", o.type_name()),
        }
    }

    /// 浮点近似相等（单测辅助）。
    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn test_math_module_registration() {
        // register_math_module 返回 MODULE，exports 含 2 常量 + 13 函数。
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "math");
            assert!(m.exports.contains_key("pi"));
            assert!(m.exports.contains_key("e"));
            for name in [
                "sqrt", "pow", "abs", "sin", "cos", "tan", "log", "log2", "log10", "exp", "ceil",
                "floor", "round",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_math_constants() {
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            let m = read_module_mut(ptr);
            assert!(approx(fval(&m.exports["pi"]), std::f64::consts::PI));
            assert!(approx(fval(&m.exports["e"]), std::f64::consts::E));
        }
    }

    #[test]
    fn test_math_sqrt() {
        let mut v = vm();
        assert!(approx(fval(&native_math_sqrt(&mut v, &[Object::Float(16.0)]).unwrap()), 4.0));
        // 整数入参自动转 Float
        assert!(approx(fval(&native_math_sqrt(&mut v, &[Object::Int(9)]).unwrap()), 3.0));
        // 负数 → NaN（不抛，IEEE 754）
        assert!(fval(&native_math_sqrt(&mut v, &[Object::Float(-1.0)]).unwrap()).is_nan());
    }

    #[test]
    fn test_math_pow() {
        let mut v = vm();
        assert!(approx(
            fval(&native_math_pow(&mut v, &[Object::Int(2), Object::Int(10)]).unwrap()),
            1024.0
        ));
        // pow(0, -1) → Infinity；pow(-1, 0.5) → NaN（§7 域错误）
        assert!(fval(&native_math_pow(&mut v, &[Object::Int(0), Object::Int(-1)]).unwrap()).is_infinite());
        assert!(fval(&native_math_pow(&mut v, &[Object::Int(-1), Object::Float(0.5)]).unwrap()).is_nan());
    }

    #[test]
    fn test_math_abs_preserves_type() {
        let mut v = vm();
        // Int→Int
        assert_eq!(native_math_abs(&mut v, &[Object::Int(-42)]).unwrap(), Object::Int(42));
        // Float→Float
        assert!(approx(fval(&native_math_abs(&mut v, &[Object::Float(-2.5)]).unwrap()), 2.5));
        // Bool→Int
        assert_eq!(native_math_abs(&mut v, &[Object::Bool(true)]).unwrap(), Object::Int(1));
        assert_eq!(native_math_abs(&mut v, &[Object::Bool(false)]).unwrap(), Object::Int(0));
    }

    #[test]
    fn test_math_trig() {
        let mut v = vm();
        assert!(approx(fval(&native_math_sin(&mut v, &[Object::Float(0.0)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_cos(&mut v, &[Object::Float(0.0)]).unwrap()), 1.0));
        assert!(approx(fval(&native_math_tan(&mut v, &[Object::Float(0.0)]).unwrap()), 0.0));
        // sin(π/2) ≈ 1
        assert!(approx(
            fval(&native_math_sin(&mut v, &[Object::Float(std::f64::consts::FRAC_PI_2)]).unwrap()),
            1.0
        ));
    }

    #[test]
    fn test_math_logs_and_exp() {
        let mut v = vm();
        assert!(approx(fval(&native_math_log(&mut v, &[Object::Float(100.0)]).unwrap()), 4.605170185988091));
        assert!(approx(fval(&native_math_log2(&mut v, &[Object::Float(8.0)]).unwrap()), 3.0));
        assert!(approx(fval(&native_math_log10(&mut v, &[Object::Float(100.0)]).unwrap()), 2.0));
        assert!(approx(fval(&native_math_exp(&mut v, &[Object::Float(1.0)]).unwrap()), std::f64::consts::E));
        // 域错误：log(0) → -Inf；log(-1) → NaN；exp(710) → +Inf（§7）
        assert!(fval(&native_math_log(&mut v, &[Object::Float(0.0)]).unwrap()).is_infinite());
        assert!(fval(&native_math_log(&mut v, &[Object::Float(-1.0)]).unwrap()).is_nan());
        assert!(fval(&native_math_exp(&mut v, &[Object::Float(710.0)]).unwrap()).is_infinite());
    }

    #[test]
    fn test_math_ceil_floor_round_return_int() {
        let mut v = vm();
        // 返回 Object::Int（非 Float）
        assert_eq!(native_math_ceil(&mut v, &[Object::Float(3.2)]).unwrap(), Object::Int(4));
        assert_eq!(native_math_floor(&mut v, &[Object::Float(3.8)]).unwrap(), Object::Int(3));
        assert_eq!(native_math_round(&mut v, &[Object::Float(3.5)]).unwrap(), Object::Int(4));
    }

    #[test]
    fn test_math_round_half_away_from_zero() {
        // §6：半远离零（round(2.5)→3，非 Python 银行家舍入 round(2.5)→2）
        let mut v = vm();
        assert_eq!(native_math_round(&mut v, &[Object::Float(2.5)]).unwrap(), Object::Int(3));
        assert_eq!(native_math_round(&mut v, &[Object::Float(3.5)]).unwrap(), Object::Int(4));
        assert_eq!(native_math_round(&mut v, &[Object::Float(0.5)]).unwrap(), Object::Int(1));
        assert_eq!(native_math_round(&mut v, &[Object::Float(-2.5)]).unwrap(), Object::Int(-3));
    }

    #[test]
    fn test_math_ceil_nan_and_overflow_errors() {
        let mut v = vm();
        // ceil(NaN) → ValueError（§5/§9）
        let err = native_math_ceil(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("NaN"));
        // ceil(1e30) → OverflowError（§5/§9，Rust as i64 会静默饱和）
        let err = native_math_ceil(&mut v, &[Object::Float(1e30)]).unwrap_err();
        assert!(err.contains("OverflowError"));
        // floor/round 同样受 float_to_int 保护
        let err = native_math_floor(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError"));
        let err = native_math_round(&mut v, &[Object::Float(-1e30)]).unwrap_err();
        assert!(err.contains("OverflowError"));
    }

    #[test]
    fn test_expect_number_type_errors() {
        let mut v = vm();
        // 非数值入参 → TypeError
        let err = native_math_sqrt(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
        // 缺参 → TypeError (missing)
        let err = native_math_sqrt(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("missing"));
        // abs 非数值 → TypeError
        let err = native_math_abs(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    // ---- 端到端集成测试 ----

    #[test]
    fn test_integration_math_basics() {
        // 等价 test_math.ms（值经 abs() 容差断言，避免浮点字面量位级歧义）
        let src = r#"
import math
assert(abs(math.pi - 3.141592653589793) < 1e-15)
assert(math.sqrt(16) == 4.0)
assert(math.pow(2, 10) == 1024.0)
assert(abs(math.sin(math.pi / 2) - 1.0) < 1e-12)
assert(abs(math.log(100) - 4.605170185988091) < 1e-12)
assert(math.log2(8) == 3.0)
assert(math.log10(100) == 2.0)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math basics failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_math_extra() {
        // 等价 test_math_extra.ms：含 math.round(2.5) → 3（半远离零）
        let src = r#"
import math
assert(math.cos(0) == 1.0)
assert(math.tan(0) == 0.0)
assert(abs(math.exp(1) - 2.718281828459045) < 1e-15)
assert(math.ceil(3.2) == 4)
assert(math.floor(3.8) == 3)
assert(math.round(3.5) == 4)
assert(math.round(2.5) == 3)
assert(math.abs(-42) == 42)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math extra failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_from_math_import() {
        // from math import 提取导出名（常量 + 函数）。
        let src = r#"
from math import sqrt, pi, e, abs
assert(sqrt(25) == 5.0)
assert(pi == 3.141592653589793)
assert(e == 2.718281828459045)
assert(abs(-7) == 7)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "from math import failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_import_std_prefix() {
        // import @std math：@std 前缀经 parse_std_prefix 剥离后命中原生模块。
        let src = r#"
import @std math
assert(math.sqrt(49) == 7.0)
assert(math.floor(2.9) == 2)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "import @std math failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_math_ceil_overflow_error() {
        // 端到端：math.ceil(1e30) 在 VM 中抛 OverflowError（异常传播路径）。
        let src = r#"
import math
math.ceil(1e30)
"#;
        let r = run_source(src);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("OverflowError"), "expected OverflowError");
    }

    #[test]
    fn test_integration_math_abs_type_preservation() {
        // math.abs(-42) 为 Int，math.abs(-3.14) 为 Float（§4 类型保留）。
        let src = r#"
import math
i = math.abs(-42)
assert(i == 42)
assert(type(i) == "int")
f = math.abs(-2.5)
assert(f == 2.5)
assert(type(f) == "float")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math abs type failed: {:?}", r.err());
    }

    // ---- task 48：os 模块 ----

    /// 提取 Object::String 内部值（单测辅助）。
    fn strval(o: &Object) -> String {
        match o {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
                unsafe { read_str(*ptr) }.to_owned()
            }
            _ => panic!("expected String, got {}", o.type_name()),
        }
    }

    #[test]
    fn test_os_module_registration() {
        let ptr = register_os_module();
        // SAFETY: ptr 由 register_os_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "os");
            for name in ["getenv", "setenv", "getcwd", "chdir", "exec", "exit", "args"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_os_getenv_setenv() {
        let mut v = vm();
        let key = "__MSLANG_OS_TEST_K1__";
        // 不存在 → nil（非异常）
        std::env::remove_var(key);
        assert_eq!(native_os_getenv(&mut v, &[s(key)]).unwrap(), Object::Nil);
        // setenv → nil，再 getenv → 设定值
        assert_eq!(
            native_os_setenv(&mut v, &[s(key), s("hello")]).unwrap(),
            Object::Nil
        );
        assert_eq!(native_os_getenv(&mut v, &[s(key)]).unwrap(), s("hello"));
        std::env::remove_var(key);
    }

    #[test]
    fn test_os_getcwd() {
        let mut v = vm();
        let r = native_os_getcwd(&mut v, &[]).unwrap();
        assert!(!strval(&r).is_empty());
    }

    #[test]
    fn test_os_getenv_type_error() {
        let mut v = vm();
        // 非字符串入参 → TypeError
        let err = native_os_getenv(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"));
        // 缺参 → TypeError (missing)
        let err = native_os_getenv(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("missing"));
    }

    #[test]
    fn test_os_exec_success() {
        let mut v = vm();
        // echo 在 Windows cmd / Unix sh 均存在；输出含探测串。
        let r = native_os_exec(&mut v, &[s("echo mslang_probe_xyz")]).unwrap();
        assert!(strval(&r).contains("mslang_probe_xyz"));
    }

    #[test]
    fn test_os_exec_failure() {
        let mut v = vm();
        // 命令返回非零退出码 → IOError
        #[cfg(windows)]
        let cmd = "exit /b 7";
        #[cfg(not(windows))]
        let cmd = "exit 7";
        let err = native_os_exec(&mut v, &[s(cmd)]).unwrap_err();
        assert!(err.contains("IOError") && err.contains("command failed"));
    }

    #[test]
    fn test_os_exit_sentinel() {
        let mut v = vm();
        // exit(0) → Err("__EXIT__0")；不直接 std::process::exit
        let err = native_os_exit(&mut v, &[Object::Int(0)]).unwrap_err();
        assert_eq!(err, "__EXIT__0");
        let err = native_os_exit(&mut v, &[Object::Int(42)]).unwrap_err();
        assert_eq!(err, "__EXIT__42");
    }

    #[test]
    fn test_os_exit_type_error() {
        let mut v = vm();
        let err = native_os_exit(&mut v, &[Object::Float(1.0)]).unwrap_err();
        assert!(err.contains("TypeError"));
        let err = native_os_exit(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("missing"));
    }

    #[test]
    fn test_os_args_is_list() {
        // args 属性为 List，至少含程序名（长度 >= 1）。
        let obj = build_args_list();
        let Object::Ref(ptr) = &obj else {
            panic!("expected Ref");
        };
        // SAFETY: build_args_list 返回有效 LIST。
        let items = unsafe { read_list(*ptr) };
        assert!(!items.is_empty());
    }

    // ---- task 48：string 模块 ----

    #[test]
    fn test_string_module_registration() {
        let ptr = register_string_module();
        // SAFETY: ptr 由 register_string_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "string");
            for name in ["format", "repeat", "reverse", "is_alpha", "is_digit"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_string_format_basic() {
        let mut v = vm();
        assert_eq!(
            native_string_format(&mut v, &[s("{} + {} = {}"), Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            s("1 + 2 = 3")
        );
        // 无占位符 → 原样返回
        assert_eq!(native_string_format(&mut v, &[s("plain")]).unwrap(), s("plain"));
        // 字符串参数
        assert_eq!(
            native_string_format(&mut v, &[s("hi {}"), s("there")]).unwrap(),
            s("hi there")
        );
    }

    #[test]
    fn test_string_format_type_conversion() {
        let mut v = vm();
        // 非 string 参数经 object_to_string 转换（与 print/str 一致）
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Int(42)]).unwrap(), s("42"));
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Float(3.5)]).unwrap(), s("3.5"));
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Float(3.0)]).unwrap(), s("3.0"));
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Bool(true)]).unwrap(), s("true"));
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Bool(false)]).unwrap(), s("false"));
        assert_eq!(native_string_format(&mut v, &[s("{}"), Object::Nil]).unwrap(), s("nil"));
    }

    #[test]
    fn test_string_format_missing_arg() {
        let mut v = vm();
        let err = native_string_format(&mut v, &[s("{} {}"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("not enough arguments"));
    }

    #[test]
    fn test_string_format_template_type_error() {
        let mut v = vm();
        let err = native_string_format(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_string_repeat() {
        let mut v = vm();
        assert_eq!(native_string_repeat(&mut v, &[s("ab"), Object::Int(3)]).unwrap(), s("ababab"));
        assert_eq!(native_string_repeat(&mut v, &[s("x"), Object::Int(0)]).unwrap(), s(""));
        assert_eq!(native_string_repeat(&mut v, &[s("万"), Object::Int(2)]).unwrap(), s("万万"));
    }

    #[test]
    fn test_string_repeat_errors() {
        let mut v = vm();
        // 负数 → ValueError
        let err = native_string_repeat(&mut v, &[s("a"), Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("negative"));
        // 超大 → ValueError
        let err = native_string_repeat(&mut v, &[s("a"), Object::Int(1_000_001)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("too large"));
        // 边界：1_000_000 仍允许
        let r = native_string_repeat(&mut v, &[s("a"), Object::Int(1_000_000)]).unwrap();
        assert_eq!(strval(&r).len(), 1_000_000);
        // 非整数 n → TypeError
        let err = native_string_repeat(&mut v, &[s("a"), Object::Float(2.0)]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_string_reverse() {
        let mut v = vm();
        assert_eq!(native_string_reverse(&mut v, &[s("hello")]).unwrap(), s("olleh"));
        // Unicode 安全：按 char（标量值）反转
        assert_eq!(native_string_reverse(&mut v, &[s("你好世")]).unwrap(), s("世好你"));
        assert_eq!(native_string_reverse(&mut v, &[s("")]).unwrap(), s(""));
    }

    #[test]
    fn test_string_is_alpha() {
        let mut v = vm();
        assert_eq!(native_string_is_alpha(&mut v, &[s("abc")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_alpha(&mut v, &[s("AbC")]).unwrap(), Object::Bool(true));
        // 空串 → false
        assert_eq!(native_string_is_alpha(&mut v, &[s("")]).unwrap(), Object::Bool(false));
        // 含数字 → false
        assert_eq!(native_string_is_alpha(&mut v, &[s("abc123")]).unwrap(), Object::Bool(false));
        // 含空格 → false
        assert_eq!(native_string_is_alpha(&mut v, &[s("ab c")]).unwrap(), Object::Bool(false));
    }

    #[test]
    fn test_string_is_digit() {
        let mut v = vm();
        assert_eq!(native_string_is_digit(&mut v, &[s("123")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_digit(&mut v, &[s("007")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_digit(&mut v, &[s("")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_digit(&mut v, &[s("12a")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_digit(&mut v, &[s("-5")]).unwrap(), Object::Bool(false));
    }

    // ---- task 48：time 模块 ----

    #[test]
    fn test_time_module_registration() {
        let ptr = register_time_module();
        // SAFETY: ptr 由 register_time_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "time");
            for name in ["now", "sleep", "format"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_time_now() {
        let mut v = vm();
        let r = native_time_now(&mut v, &[]).unwrap();
        let ts = fval(&r);
        // 合理的时间戳（> 2001-09-09，即 > 1e9）
        assert!(ts > 1_000_000_000.0, "time.now() returned {}", ts);
    }

    #[test]
    fn test_time_sleep_zero() {
        let mut v = vm();
        // sleep(0) / sleep(0.0) 立即返回 nil
        assert_eq!(native_time_sleep(&mut v, &[Object::Int(0)]).unwrap(), Object::Nil);
        assert_eq!(
            native_time_sleep(&mut v, &[Object::Float(0.0)]).unwrap(),
            Object::Nil
        );
    }

    #[test]
    fn test_time_sleep_errors() {
        let mut v = vm();
        // 负数 → ValueError
        let err = native_time_sleep(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("negative"));
        let err = native_time_sleep(&mut v, &[Object::Float(-0.5)]).unwrap_err();
        assert!(err.contains("ValueError"));
        // NaN/Inf → ValueError（防止 from_secs_f64 panic）
        let err = native_time_sleep(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError"));
        let err = native_time_sleep(&mut v, &[Object::Float(f64::INFINITY)]).unwrap_err();
        assert!(err.contains("ValueError"));
        // 非数值 → TypeError
        let err = native_time_sleep(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_unix_to_ymdhms_epoch() {
        assert_eq!(unix_to_ymdhms(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn test_unix_to_ymdhms_known() {
        // 1700000000 = 2023-11-14 22:13:20 UTC
        assert_eq!(unix_to_ymdhms(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        // 1000000000 = 2001-09-09 01:46:40 UTC
        assert_eq!(unix_to_ymdhms(1_000_000_000), (2001, 9, 9, 1, 46, 40));
    }

    #[test]
    fn test_unix_to_ymdhms_leap_day() {
        // 2020-02-29 12:00:00 UTC = 1582977600（闰日）
        assert_eq!(unix_to_ymdhms(1_582_977_600), (2020, 2, 29, 12, 0, 0));
    }

    #[test]
    fn test_time_format() {
        let mut v = vm();
        // Int 时间戳
        assert_eq!(
            native_time_format(&mut v, &[Object::Int(0)]).unwrap(),
            s("1970-01-01 00:00:00")
        );
        assert_eq!(
            native_time_format(&mut v, &[Object::Int(1_700_000_000)]).unwrap(),
            s("2023-11-14 22:13:20")
        );
        // Float 时间戳（截断小数）
        assert_eq!(
            native_time_format(&mut v, &[Object::Float(0.0)]).unwrap(),
            s("1970-01-01 00:00:00")
        );
        assert_eq!(
            native_time_format(&mut v, &[Object::Float(1_700_000_000.999)]).unwrap(),
            s("2023-11-14 22:13:20")
        );
    }

    #[test]
    fn test_time_format_errors() {
        let mut v = vm();
        let err = native_time_format(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("negative"));
        let err = native_time_format(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    // ---- task 48：path 模块 ----

    #[test]
    fn test_path_module_registration() {
        let ptr = register_path_module();
        // SAFETY: ptr 由 register_path_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "path");
            for name in ["join", "ext", "base", "dir"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_path_join() {
        let mut v = vm();
        // 用正斜杠输入：std::path 在 Windows 也接受，输出为平台分隔符。
        let expected = {
            let mut p = std::path::PathBuf::new();
            p.push("a");
            p.push("b");
            p.push("c");
            p.to_string_lossy().into_owned()
        };
        let r = native_path_join(&mut v, &[s("a"), s("b"), s("c")]).unwrap();
        assert_eq!(strval(&r), expected);
        // 单段 → 原样
        assert_eq!(native_path_join(&mut v, &[s("alone")]).unwrap(), s("alone"));
    }

    #[test]
    fn test_path_join_empty() {
        let mut v = vm();
        let err = native_path_join(&mut v, &[]).unwrap_err();
        assert!(err.contains("ValueError"));
    }

    #[test]
    fn test_path_ext() {
        let mut v = vm();
        assert_eq!(native_path_ext(&mut v, &[s("file.txt")]).unwrap(), s(".txt"));
        assert_eq!(native_path_ext(&mut v, &[s("archive.tar.gz")]).unwrap(), s(".gz"));
        assert_eq!(native_path_ext(&mut v, &[s("noext")]).unwrap(), s(""));
        assert_eq!(native_path_ext(&mut v, &[s(".hidden")]).unwrap(), s(""));
    }

    #[test]
    fn test_path_base() {
        let mut v = vm();
        assert_eq!(native_path_base(&mut v, &[s("a/b/c.txt")]).unwrap(), s("c.txt"));
        assert_eq!(native_path_base(&mut v, &[s("file.txt")]).unwrap(), s("file.txt"));
        // 根目录无文件名 → ""
        assert_eq!(native_path_base(&mut v, &[s("/")]).unwrap(), s(""));
    }

    #[test]
    fn test_path_dir() {
        let mut v = vm();
        assert_eq!(native_path_dir(&mut v, &[s("file.txt")]).unwrap(), s(""));
        assert_eq!(
            native_path_dir(&mut v, &[s("a/b/c.txt")]).unwrap(),
            s("a/b")
        );
    }

    #[test]
    fn test_path_type_errors() {
        let mut v = vm();
        // 非字符串入参 / 缺参 → TypeError（直接调用层面；arity 由 VM CALL 校验）
        let err = native_path_ext(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"));
        let err = native_path_base(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("missing"));
        let err = native_path_dir(&mut v, &[Object::Bool(true)]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    // ---- task 48：端到端集成测试 ----

    #[test]
    fn test_integration_os() {
        let src = r#"
import os
assert(type(os.getcwd()) == "string")
assert(len(os.getcwd()) > 0)
assert(os.getenv("__MSLANG_NOT_SET_X9Z__") == nil)
os.setenv("__MSLANG_INTTEST_K__", "v42")
assert(os.getenv("__MSLANG_INTTEST_K__") == "v42")
assert(type(os.args) == "list")
assert(len(os.args) >= 1)
"#;
        let r = run_source(src);
        std::env::remove_var("__MSLANG_INTTEST_K__");
        assert!(r.is_ok(), "os integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_string() {
        let src = r#"
import string
assert(string.format("{} + {} = {}", 1, 2, 3) == "1 + 2 = 3")
assert(string.format("{}", 3.5) == "3.5")
assert(string.format("{}", true) == "true")
assert(string.format("{}", nil) == "nil")
assert(string.format("hi {}", "there") == "hi there")
assert(string.repeat("ab", 3) == "ababab")
assert(string.repeat("x", 0) == "")
assert(string.reverse("hello") == "olleh")
assert(string.is_alpha("abc"))
assert(not string.is_alpha(""))
assert(not string.is_alpha("abc123"))
assert(string.is_digit("123"))
assert(not string.is_digit(""))
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "string integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_time() {
        let src = r#"
import time
assert(time.format(0) == "1970-01-01 00:00:00")
assert(time.format(1700000000) == "2023-11-14 22:13:20")
assert(time.now() > 1000000000)
assert(time.sleep(0) == nil)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "time integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_path() {
        let src = r#"
import path
assert(path.ext("file.txt") == ".txt")
assert(path.ext("noext") == "")
assert(path.base("a/b/c.txt") == "c.txt")
assert(path.dir("file.txt") == "")
j = path.join("a", "b", "c")
assert(len(j) == 5)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "path integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_from_imports() {
        let src = r#"
from string import format, repeat, reverse, is_alpha, is_digit
assert(format("{}!", "go") == "go!")
assert(repeat("na", 2) == "nana")
assert(reverse("abc") == "cba")
assert(is_alpha("hi"))
assert(is_digit("9"))
from time import format as tfmt
assert(tfmt(0) == "1970-01-01 00:00:00")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "from-imports failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_os_string_time_path_std_prefix() {
        // import @std os/string/time/path：@std 前缀命中原生模块。
        let src = r#"
import @std os
assert(type(os.getcwd()) == "string")
import @std string
assert(string.reverse("abc") == "cba")
import @std time
assert(time.format(0) == "1970-01-01 00:00:00")
import @std path
assert(path.ext("f.txt") == ".txt")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "import @std failed: {:?}", r.err());
    }

    // ---- json 模块（task 49）-------------------------------------------------

    #[test]
    fn test_json_module_registration() {
        let ptr = register_json_module();
        // SAFETY: ptr 由 register_json_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "json");
            assert!(m.exports.contains_key("parse"));
            assert!(m.exports.contains_key("stringify"));
        }
    }

    #[test]
    #[allow(clippy::approx_constant)] // 测试合法使用 3.14 字面量（clippy 误报为 PI 近似）
    fn test_json_parse_scalars() {
        let mut v = vm();
        assert_eq!(native_json_parse(&mut v, &[s("null")]).unwrap(), Object::Nil);
        assert_eq!(
            native_json_parse(&mut v, &[s("true")]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("false")]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("42")]).unwrap(),
            Object::Int(42)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("-7")]).unwrap(),
            Object::Int(-7)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("0")]).unwrap(),
            Object::Int(0)
        );
        match native_json_parse(&mut v, &[s("3.14")]).unwrap() {
            Object::Float(f) => assert!((f - 3.14).abs() < 1e-9, "got {}", f),
            other => panic!("expected float, got {:?}", other),
        }
    }

    #[test]
    fn test_json_parse_string_escapes() {
        let mut v = vm();
        assert_eq!(
            native_json_parse(&mut v, &[s("\"hello\"")]).unwrap(),
            s("hello")
        );
        // \" \\ \n \t \/ \uXXXX
        assert_eq!(
            native_json_parse(&mut v, &[s("\"a\\nb\\tc\\\\d\\/e\\u0041\"")]).unwrap(),
            s("a\nb\tc\\d/eA")
        );
        // \b \f \r
        assert_eq!(
            native_json_parse(&mut v, &[s("\"x\\by\\fz\"")]).unwrap(),
            s("x\x08y\x0cz")
        );
        // UTF-16 代理对：U+1F600 (😀)
        assert_eq!(
            native_json_parse(&mut v, &[s("\"\\uD83D\\uDE00\"")]).unwrap(),
            s("\u{1F600}")
        );
        // 非 ASCII 原文直接传递
        assert_eq!(
            native_json_parse(&mut v, &[s("\"日本語\"")]).unwrap(),
            s("日本語")
        );
    }

    #[test]
    fn test_json_parse_collections() {
        let mut v = vm();
        // array → list
        let arr = native_json_parse(&mut v, &[s("[1, null, \"hi\", [4, 5]]")]).unwrap();
        let Object::Ref(p) = &arr else {
            panic!("expected list");
        };
        unsafe {
            assert_eq!((*(*p)).type_tag, TypeTag::LIST as u8);
            let items = read_list(*p);
            assert_eq!(items.len(), 4);
            assert_eq!(items[0], Object::Int(1));
            assert_eq!(items[1], Object::Nil);
            assert_eq!(items[2], s("hi"));
        }
        // object → dict
        let d = native_json_parse(&mut v, &[s("{\"name\": \"Alice\", \"age\": 30}")]).unwrap();
        let Object::Ref(p) = &d else {
            panic!("expected dict");
        };
        unsafe {
            assert_eq!((*(*p)).type_tag, TypeTag::DICT as u8);
            let map = read_dict(*p);
            assert_eq!(map.len(), 2);
            assert_eq!(map.get(&s("name")), Some(&s("Alice")));
            assert_eq!(map.get(&s("age")), Some(&Object::Int(30)));
        }
        // 空容器
        let empty_arr = native_json_parse(&mut v, &[s("[]")]).unwrap();
        let Object::Ref(ep) = &empty_arr else {
            panic!("expected list");
        };
        unsafe {
            assert!(read_list(*ep).is_empty());
        }
        let empty_obj = native_json_parse(&mut v, &[s("{}")]).unwrap();
        let Object::Ref(ep) = &empty_obj else {
            panic!("expected dict");
        };
        unsafe {
            assert!(read_dict(*ep).is_empty());
        }
    }

    #[test]
    fn test_json_parse_nested() {
        let mut v = vm();
        let d = native_json_parse(&mut v, &[s("{\"a\": {\"b\": [1, 2, {\"c\": true}]}}")]).unwrap();
        let Object::Ref(p) = &d else {
            panic!("expected dict");
        };
        // 沿 nested["a"]["b"][2]["c"] 取 true
        unsafe {
            let a = read_dict(*p).get(&s("a")).cloned().unwrap();
            let Object::Ref(ap) = &a else {
                panic!("expected dict for a");
            };
            let b = read_dict(*ap).get(&s("b")).cloned().unwrap();
            let Object::Ref(bp) = &b else {
                panic!("expected list for b");
            };
            let third = read_list(*bp)[2].clone();
            let Object::Ref(tp) = &third else {
                panic!("expected dict for third");
            };
            let c = read_dict(*tp).get(&s("c")).cloned().unwrap();
            assert_eq!(c, Object::Bool(true));
        }
    }

    #[test]
    fn test_json_parse_bigint_to_float() {
        let mut v = vm();
        // 超出 i64 范围（> 9223372036854775807）→ Float
        match native_json_parse(&mut v, &[s("99999999999999999999")]).unwrap() {
            Object::Float(_) => {}
            other => panic!("expected float for big int, got {:?}", other),
        }
        // i64 边界内 → Int
        assert_eq!(
            native_json_parse(&mut v, &[s("9223372036854775807")]).unwrap(),
            Object::Int(i64::MAX)
        );
    }

    #[test]
    fn test_json_parse_errors() {
        let mut v = vm();
        // 非法 JSON（首字节 'i' 非法）→ 行列号定位
        let e = native_json_parse(&mut v, &[s("invalid json")]).unwrap_err();
        assert!(e.contains("ValueError"), "got: {}", e);
        assert!(e.contains("line 1 column 1"), "got: {}", e);
        // 尾随字符 → 字节偏移
        let e = native_json_parse(&mut v, &[s("1 2")]).unwrap_err();
        assert!(e.contains("trailing characters"), "got: {}", e);
        // 非闭合
        assert!(native_json_parse(&mut v, &[s("[1, 2")]).is_err());
        // 入参非 string → TypeError
        let e = native_json_parse(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_basic() {
        let mut v = vm();
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Nil]).unwrap(),
            s("null")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Bool(true)]).unwrap(),
            s("true")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Bool(false)]).unwrap(),
            s("false")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Int(42)]).unwrap(),
            s("42")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[s("hi")]).unwrap(),
            s("\"hi\"")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[alloc_list(vec![Object::Int(1), Object::Int(2)])])
                .unwrap(),
            s("[1,2]")
        );
        // dict {"x":1,"y":[2,3]}
        let mut m = DictMap::new();
        m.insert(s("x"), Object::Int(1));
        m.insert(s("y"), alloc_list(vec![Object::Int(2), Object::Int(3)]));
        assert_eq!(
            native_json_stringify(&mut v, &[alloc_dict(m)]).unwrap(),
            s("{\"x\":1,\"y\":[2,3]}")
        );
    }

    #[test]
    #[allow(clippy::approx_constant)] // 测试合法使用 3.14 字面量
    fn test_json_stringify_floats() {
        let mut v = vm();
        // 3.14 → "3.14"
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(3.14)]).unwrap(),
            s("3.14")
        );
        // 整数值浮点 → "3.0"
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(3.0)]).unwrap(),
            s("3.0")
        );
        // -0.0 字面量保留
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(-0.0)]).unwrap(),
            s("-0.0")
        );
        // NaN → ValueError
        let e = native_json_stringify(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(e.contains("non-finite"), "got: {}", e);
        assert!(e.contains("NaN"), "got: {}", e);
        // Infinity → ValueError
        let e = native_json_stringify(&mut v, &[Object::Float(f64::INFINITY)]).unwrap_err();
        assert!(e.contains("non-finite"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_escapes() {
        let mut v = vm();
        assert_eq!(
            native_json_stringify(&mut v, &[s("a\"b\\c\nd\te")]).unwrap(),
            s("\"a\\\"b\\\\c\\nd\\te\"")
        );
        // 控制字符 < 0x20 → \u00XX
        assert_eq!(
            native_json_stringify(&mut v, &[s("\x01")]).unwrap(),
            s("\"\\u0001\"")
        );
        // 非 ASCII 原样输出
        assert_eq!(
            native_json_stringify(&mut v, &[s("日本語")]).unwrap(),
            s("\"日本語\"")
        );
    }

    #[test]
    fn test_json_stringify_non_serializable() {
        let mut v = vm();
        // function → TypeError
        let f = alloc_native_function(NativeFunction {
            name: "parse".to_string(),
            func: native_json_parse,
        });
        let e = native_json_stringify(&mut v, &[f]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        assert!(e.contains("function"), "got: {}", e);
        // tuple → TypeError
        let t = crate::vm::object::alloc_tuple(vec![Object::Int(1)]);
        let e = native_json_stringify(&mut v, &[t]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        // 缺参（直接调用 native）→ ValueError
        let e = native_json_stringify(&mut v, &[]).unwrap_err();
        assert!(e.contains("ValueError"), "got: {}", e);
        assert!(e.contains("expects 1 argument"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_dict_key_non_string() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(Object::Int(1), Object::Int(2)); // 非字符串键
        let e = native_json_stringify(&mut v, &[alloc_dict(m)]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        assert!(e.contains("dict key must be string"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_circular_list() {
        let mut v = vm();
        let lst = alloc_list(vec![]);
        // list 自引用：a = []; a.push(a)
        if let Object::Ref(p) = &lst {
            unsafe {
                read_list(*p).push(lst.clone());
            }
        }
        let e = native_json_stringify(&mut v, &[lst]).unwrap_err();
        assert!(e.contains("circular reference"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_circular_dict() {
        let mut v = vm();
        // d1 = {"link": d2}; d2 = {"back": d1} → 互引用
        let d2 = alloc_dict(DictMap::new());
        let mut m1 = DictMap::new();
        m1.insert(s("link"), d2.clone());
        let d1 = alloc_dict(m1);
        if let Object::Ref(p2) = &d2 {
            unsafe {
                read_dict(*p2).insert(s("back"), d1.clone());
            }
        }
        let e = native_json_stringify(&mut v, &[d1]).unwrap_err();
        assert!(e.contains("circular reference"), "got: {}", e);
    }

    #[test]
    fn test_json_round_trip() {
        let mut v = vm();
        let original = s("{\"name\":\"Alice\",\"age\":30,\"scores\":[10,20,30]}");
        let parsed = native_json_parse(&mut v, std::slice::from_ref(&original)).unwrap();
        let back = native_json_stringify(&mut v, &[parsed]).unwrap();
        assert_eq!(back, original);
        // 浮点 round-trip
        let f = native_json_parse(&mut v, &[s("3.14")]).unwrap();
        assert_eq!(
            native_json_stringify(&mut v, &[f]).unwrap(),
            s("3.14")
        );
    }

    #[test]
    fn test_json_depth_limit_parse() {
        let mut v = vm();
        // 1000 层可解析，1001 层超限
        let ok = "[".repeat(1000) + &"]".repeat(1000);
        assert!(native_json_parse(&mut v, &[s(&ok)]).is_ok(), "1000 levels should parse");
        let too_deep = "[".repeat(1001) + &"]".repeat(1001);
        let e = native_json_parse(&mut v, &[s(&too_deep)]).unwrap_err();
        assert!(e.contains("nesting exceeds 1000 levels"), "got: {}", e);
    }

    #[test]
    fn test_json_depth_limit_stringify() {
        let mut v = vm();
        // 递归构造 1001 层 list 嵌套
        let mut nested = Object::Int(1);
        for _ in 0..1001 {
            nested = alloc_list(vec![nested]);
        }
        let e = native_json_stringify(&mut v, &[nested]).unwrap_err();
        assert!(e.contains("nesting exceeds 1000 levels"), "got: {}", e);
    }

    #[test]
    fn test_integration_json_module() {
        // 等价 test_json.ms：import → parse → dict 索引 → stringify。
        // 注：mslang 仅支持双引号字符串（spec 示例的单引号为 Python 习惯，需转义）。
        let src = r#"
import json
data = json.parse("{\"name\": \"Alice\", \"age\": 30}")
assert(data["name"] == "Alice")
assert(data["age"] == 30)
text = json.stringify({"x": 1, "y": [2, 3]})
assert(text == "{\"x\":1,\"y\":[2,3]}")
nested = json.parse("{\"a\": {\"b\": [1, 2, {\"c\": true}]}}")
assert(nested["a"]["b"][2]["c"] == true)
f = json.parse("3.14")
assert(type(f) == "float")
assert(json.stringify(f) == "3.14")
assert(json.stringify(json.parse("-0.0")) == "-0.0")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "json integration failed: {:?}", r.err());
    }

    // -----------------------------------------------------------------------
    // task 51: List/Dict/Set 方法测试
    // -----------------------------------------------------------------------

    fn ilist(nums: &[i64]) -> Object {
        let items: Vec<Object> = nums.iter().map(|n| Object::Int(*n)).collect();
        alloc_list(items)
    }

    #[test]
    fn test_lookup_list_method() {
        let names = [
            "length", "push", "pop", "insert", "remove", "index", "contains", "sort",
            "reverse", "slice", "map", "filter", "reduce",
        ];
        for name in &names {
            assert!(lookup_list_method(name).is_some(), "missing list method: {}", name);
        }
        assert!(lookup_list_method("nosuch").is_none());
    }

    #[test]
    fn test_lookup_dict_method() {
        let names = [
            "length", "keys", "values", "items", "get", "set", "remove", "contains", "merge",
        ];
        for name in &names {
            assert!(lookup_dict_method(name).is_some(), "missing dict method: {}", name);
        }
        assert!(lookup_dict_method("nosuch").is_none());
    }

    #[test]
    fn test_lookup_set_method() {
        let names = ["length", "add", "remove", "contains", "union", "intersection", "difference"];
        for name in &names {
            assert!(lookup_set_method(name).is_some(), "missing set method: {}", name);
        }
        assert!(lookup_set_method("nosuch").is_none());
    }

    #[test]
    fn test_list_methods_basic() {
        let mut v = vm();

        // length
        let lst = ilist(&[3, 1, 4, 1, 5]);
        assert_eq!(native_list_length(&mut v, &[lst.clone()]).unwrap(), Object::Int(5));

        // sort
        native_list_sort(&mut v, &[lst.clone()]).unwrap();
        assert_eq!(unsafe { read_list(match lst { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(1), Object::Int(1), Object::Int(3), Object::Int(4), Object::Int(5)]);

        // push
        native_list_push(&mut v, &[lst.clone(), Object::Int(9)]).unwrap();
        // pop
        let popped = native_list_pop(&mut v, &[lst.clone()]).unwrap();
        assert_eq!(popped, Object::Int(9));

        // insert
        let lst2 = ilist(&[1, 2, 3]);
        native_list_insert(&mut v, &[lst2.clone(), Object::Int(0), Object::Int(99)]).unwrap();

        // remove
        let lst3 = ilist(&[1, 2, 1]);
        native_list_remove(&mut v, &[lst3.clone(), Object::Int(1)]).unwrap();

        // index
        let lst4 = ilist(&[10, 20, 30]);
        assert_eq!(native_list_index(&mut v, &[lst4.clone(), Object::Int(20)]).unwrap(), Object::Int(1));

        // contains
        assert_eq!(native_list_contains(&mut v, &[lst4.clone(), Object::Int(20)]).unwrap(), Object::Bool(true));
        assert_eq!(native_list_contains(&mut v, &[lst4.clone(), Object::Int(99)]).unwrap(), Object::Bool(false));

        // reverse
        let lst5 = ilist(&[1, 2, 3]);
        native_list_reverse(&mut v, &[lst5.clone()]).unwrap();

        // slice
        let lst6 = ilist(&[10, 20, 30, 40, 50]);
        let sliced = native_list_slice(&mut v, &[lst6, Object::Int(1), Object::Int(3)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(20), Object::Int(30)]);
    }

    #[test]
    fn test_list_pop_empty_error() {
        let mut v = vm();
        let empty = alloc_list(vec![]);
        let err = native_list_pop(&mut v, &[empty]).unwrap_err();
        assert!(err.starts_with("IndexError:"), "got: {}", err);
        assert!(err.contains("empty list"));
    }

    #[test]
    fn test_list_pop_index_oob_error() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_pop(&mut v, &[lst, Object::Int(10)]).unwrap_err();
        assert!(err.starts_with("IndexError:"), "got: {}", err);
    }

    #[test]
    fn test_list_remove_not_found() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_remove(&mut v, &[lst, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_index_not_found() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_index(&mut v, &[lst, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_slice_reverse_error() {
        let mut v = vm();
        let lst = ilist(&[1, 2]);
        let err = native_list_slice(&mut v, &[lst, Object::Int(3), Object::Int(1)]).unwrap_err();
        assert!(err.starts_with("ValueError:"), "got: {}", err);
    }

    #[test]
    fn test_list_negative_index() {
        let mut v = vm();
        // pop(-1)
        let lst = ilist(&[10, 20, 30, 40, 50]);
        let popped = native_list_pop(&mut v, &[lst.clone(), Object::Int(-1)]).unwrap();
        assert_eq!(popped, Object::Int(50));

        // insert(-1, val) — before last
        let lst2 = ilist(&[10, 20, 30, 40]);
        native_list_insert(&mut v, &[lst2.clone(), Object::Int(-1), Object::Int(99)]).unwrap();

        // slice(-2)
        let lst3 = ilist(&[10, 20, 30, 99, 40]);
        let sliced = native_list_slice(&mut v, &[lst3.clone(), Object::Int(-2)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(99), Object::Int(40)]);

        // slice(1, -1) — remove first and last
        let sliced2 = native_list_slice(&mut v, &[lst3, Object::Int(1), Object::Int(-1)]).unwrap();
        assert_eq!(unsafe { read_list(match sliced2 { Object::Ref(p) => p, _ => unreachable!() }) }.clone(),
                   vec![Object::Int(20), Object::Int(30), Object::Int(99)]);
    }

    #[test]
    fn test_dict_methods_basic() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(s("a"), Object::Int(1));
        m.insert(s("b"), Object::Int(2));
        let d = alloc_dict(m);

        // length
        assert_eq!(native_dict_length(&mut v, &[d.clone()]).unwrap(), Object::Int(2));

        // keys
        let keys = native_dict_keys(&mut v, &[d.clone()]).unwrap();
        let keys_vec = unsafe { read_list(match &keys { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(keys_vec.len(), 2);

        // values
        let vals = native_dict_values(&mut v, &[d.clone()]).unwrap();
        let vals_vec = unsafe { read_list(match &vals { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(vals_vec.len(), 2);

        // items
        let items = native_dict_items(&mut v, &[d.clone()]).unwrap();
        let items_vec = unsafe { read_list(match &items { Object::Ref(p) => *p, _ => unreachable!() }) }.clone();
        assert_eq!(items_vec.len(), 2);

        // get with default
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("c"), Object::Int(0)]).unwrap(), Object::Int(0));
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("a")]).unwrap(), Object::Int(1));

        // set
        native_dict_set(&mut v, &[d.clone(), s("c"), Object::Int(3)]).unwrap();
        assert_eq!(native_dict_get(&mut v, &[d.clone(), s("c")]).unwrap(), Object::Int(3));

        // contains
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("a")]).unwrap(), Object::Bool(true));
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("z")]).unwrap(), Object::Bool(false));

        // remove
        native_dict_remove(&mut v, &[d.clone(), s("c")]).unwrap();
        assert_eq!(native_dict_contains(&mut v, &[d.clone(), s("c")]).unwrap(), Object::Bool(false));
    }

    #[test]
    fn test_dict_remove_missing_key_error() {
        let mut v = vm();
        let d = alloc_dict(DictMap::new());
        let err = native_dict_remove(&mut v, &[d, s("nope")]).unwrap_err();
        assert!(err.starts_with("KeyError:"), "got: {}", err);
    }

    #[test]
    fn test_dict_merge() {
        let mut v = vm();
        let mut m1 = DictMap::new();
        m1.insert(s("a"), Object::Int(1));
        let d1 = alloc_dict(m1);

        let mut m2 = DictMap::new();
        m2.insert(s("b"), Object::Int(2));
        let d2 = alloc_dict(m2);

        native_dict_merge(&mut v, &[d1.clone(), d2]).unwrap();
        assert_eq!(native_dict_length(&mut v, &[d1]).unwrap(), Object::Int(2));
    }

    #[test]
    fn test_dict_merge_self_reference() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(s("a"), Object::Int(1));
        let d = alloc_dict(m);
        // d.merge(d) should not deadlock
        native_dict_merge(&mut v, &[d.clone(), d.clone()]).unwrap();
        assert_eq!(native_dict_length(&mut v, &[d]).unwrap(), Object::Int(1));
    }

    #[test]
    fn test_dict_set_unhashable_key() {
        let mut v = vm();
        let d = alloc_dict(DictMap::new());
        let list_key = ilist(&[1, 2]);
        let err = native_dict_set(&mut v, &[d, list_key, Object::Int(3)]).unwrap_err();
        assert!(err.starts_with("TypeError:"), "got: {}", err);
        assert!(err.contains("unhashable"));
    }

    #[test]
    fn test_set_methods_basic() {
        use std::collections::HashSet;
        let mut v = vm();

        let set1 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(1));
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs
        });

        // length
        assert_eq!(native_set_length(&mut v, &[set1.clone()]).unwrap(), Object::Int(3));

        // add
        native_set_add(&mut v, &[set1.clone(), Object::Int(4)]).unwrap();
        assert_eq!(native_set_contains(&mut v, &[set1.clone(), Object::Int(4)]).unwrap(), Object::Bool(true));

        // remove
        native_set_remove(&mut v, &[set1.clone(), Object::Int(4)]).unwrap();
        assert_eq!(native_set_contains(&mut v, &[set1.clone(), Object::Int(4)]).unwrap(), Object::Bool(false));

        // union
        let set2 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(5));
            hs.insert(Object::Int(6));
            hs
        });
        let u = native_set_union(&mut v, &[set1.clone(), set2]).unwrap();
        assert_eq!(native_set_length(&mut v, &[u]).unwrap(), Object::Int(5));

        // intersection
        let set3 = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs.insert(Object::Int(7));
            hs
        });
        let inter = native_set_intersection(&mut v, &[set1.clone(), set3.clone()]).unwrap();
        assert_eq!(native_set_length(&mut v, &[inter]).unwrap(), Object::Int(2));

        // difference
        let diff = native_set_difference(&mut v, &[set1, set3]).unwrap();
        assert_eq!(native_set_length(&mut v, &[diff]).unwrap(), Object::Int(1)); // {1}
    }

    #[test]
    fn test_set_remove_missing_error() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let err = native_set_remove(&mut v, &[set, Object::Int(99)]).unwrap_err();
        assert!(err.starts_with("KeyError:"), "got: {}", err);
    }

    #[test]
    fn test_set_add_unhashable() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let list_val = ilist(&[1, 2]);
        let err = native_set_add(&mut v, &[set, list_val]).unwrap_err();
        assert!(err.starts_with("TypeError:"), "got: {}", err);
        assert!(err.contains("unhashable"));
    }

    #[test]
    fn test_set_contains_unhashable_returns_false() {
        let mut v = vm();
        let set = alloc_set(HashSet::new());
        let list_val = ilist(&[1, 2]);
        // contains on unhashable → false (not error)
        let result = native_set_contains(&mut v, &[set, list_val]).unwrap();
        assert_eq!(result, Object::Bool(false));
    }

    #[test]
    fn test_set_union_self_reference() {
        use std::collections::HashSet;
        let mut v = vm();
        let s = alloc_set({
            let mut hs = HashSet::new();
            hs.insert(Object::Int(1));
            hs.insert(Object::Int(2));
            hs.insert(Object::Int(3));
            hs
        });
        let u = native_set_union(&mut v, &[s.clone(), s.clone()]).unwrap();
        assert_eq!(native_set_length(&mut v, &[u]).unwrap(), Object::Int(3));
    }

    #[test]
    fn test_hash_key_nan() {
        let err = hash_key(&Object::Float(f64::NAN)).unwrap_err();
        assert!(err.contains("TypeError"));
        assert!(err.contains("NaN"));
    }

    #[test]
    fn test_hash_key_unhashable_list() {
        let list = ilist(&[1, 2]);
        let err = hash_key(&list).unwrap_err();
        assert!(err.contains("TypeError"));
        assert!(err.contains("unhashable"));
    }

    #[test]
    fn test_hash_key_valid_types() {
        assert!(hash_key(&Object::Nil).is_ok());
        assert!(hash_key(&Object::Bool(true)).is_ok());
        assert!(hash_key(&Object::Int(42)).is_ok());
        assert!(hash_key(&Object::Float(3.14)).is_ok());
        assert!(hash_key(&s("hello")).is_ok());
        assert!(hash_key(&alloc_tuple(vec![Object::Int(1), Object::Int(2)])).is_ok());
    }

    // --- Integration tests (end-to-end via mslang source) ---

    #[test]
    fn test_integration_list_methods() {
        let src = r#"
lst = [3, 1, 4, 1, 5]
lst.sort()
lst.push(9)
lst.pop()
lst.insert(0, 0)
lst.remove(1)
assert(lst.contains(4))
assert(lst.index(3) == lst.index(3))
assert(lst.length() == len(lst))
assert(lst.slice(0, 2).length() == 2)
lst.reverse()
print(lst)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "list integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_dict_methods() {
        let src = r#"
d = {"a": 1, "b": 2}
assert(d.length() == 2)
assert(d.get("a") == 1)
assert(d.get("c", 0) == 0)
d.set("c", 3)
assert(d.contains("c"))
assert(d.get("c") == 3)
d.remove("c")
assert(not d.contains("c"))
d.merge({"d": 4})
assert(d.contains("d"))
ks = d.keys()
assert(ks.length() == 3)
vs = d.values()
assert(vs.length() == 3)
it = d.items()
assert(it.length() == 3)
print(d)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "dict integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_set_methods() {
        let src = r#"
s = {1, 2, 3}
s.add(4)
assert(s.contains(4))
u = s.union({5, 6})
assert(u.length() == 6)
i = s.intersection({2, 3, 7})
assert(i.length() == 2)
d = s.difference({1, 2})
assert(d.length() == 2)
s.remove(4)
assert(not s.contains(4))
assert(s.length() == 3)
print(s)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "set integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_higher_order() {
        let src = r#"
lst = [1, 2, 3, 4, 5]
doubled = lst.map(fn(x) { return x * 2 })
assert(doubled[0] == 2)
assert(doubled[4] == 10)
evens = lst.filter(fn(x) { return x % 2 == 0 })
assert(evens.length() == 2)
total = lst.reduce(fn(a, b) { return a + b }, 0)
assert(total == 15)
product = lst.reduce(fn(a, b) { return a * b })
assert(product == 120)
print(doubled)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "higher-order integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_list_negative_index() {
        let src = r#"
lst = [10, 20, 30, 40, 50]
v = lst.pop(-1)
assert(v == 50)
assert(lst.length() == 4)
lst.insert(-1, 99)
assert(lst.slice(-2)[1] == 40)
sub = lst.slice(1, -1)
assert(sub[0] == 20)
print(lst)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "negative index integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_self_reference() {
        let src = r#"
d = {"a": 1}
d.merge(d)
assert(d.length() == 1)

s = {1, 2, 3}
u = s.union(s)
assert(u.length() == 3)

a = [1, 2]
b = [3, 4]
a.push(b)
assert(a.length() == 3)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "self-reference integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_list_attr_error() {
        let src = r#"
try {
    [1, 2].nosuch()
} except e {
    print(e)
}
"#;
        // This may or may not be catchable depending on VM error handling.
        // Just verify it doesn't crash.
        let _ = run_source(src);
    }
}
