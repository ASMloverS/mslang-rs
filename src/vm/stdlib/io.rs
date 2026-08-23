//! `io` 原生模块与 FileHandle 方法。
//!
//! 参照 [46-stdlib-io](../../../docs/mslang/tasks/46-stdlib-io.md)。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_file_handle, alloc_list, alloc_module, alloc_string, read_file_handle,
    read_file_handle_mut, read_module_mut, MsObjHeader, Object, TypeTag,
};
use crate::vm::VM;

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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, temp_path, vm};
    use crate::vm::object::read_list;

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
}
