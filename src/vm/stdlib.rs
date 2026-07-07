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
    alloc_file_handle, alloc_list, alloc_module, alloc_string, read_file_handle,
    read_file_handle_mut, read_module_mut, read_str, MsObjHeader, Object, TypeTag,
};
use super::VM;
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
}
