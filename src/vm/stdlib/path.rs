//! `path` 原生模块。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, strval, vm};
    use crate::vm::object::TypeTag;

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
}
