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
}
