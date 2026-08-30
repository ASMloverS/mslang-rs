//! `sys` 原生模块（task 82）。
//!
//! 参照 [82-stdlib-fs-os-sys](../../../docs/mslang/tasks/82-stdlib-fs-os-sys.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.9。

use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

/// 构造 `sys` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 platform/version/executable/stdin_read_all 四个原生函数。
pub fn register_sys_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 4] = [
        ("platform", native_sys_platform),
        ("version", native_sys_version),
        ("executable", native_sys_executable),
        ("stdin_read_all", native_sys_stdin_read_all),
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
    let m = alloc_module("sys");
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

/// sys.platform() → "windows" / "linux" / "macos"（cfg! 编译期映射）。
fn native_sys_platform(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let name = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    Ok(alloc_string(name))
}

/// sys.version() → "mslang {CARGO_PKG_VERSION}"
///（env! 编译期读取，与 Cargo.toml 自动同步）。
fn native_sys_version(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(alloc_string(&format!(
        "mslang {}",
        env!("CARGO_PKG_VERSION")
    )))
}

/// sys.executable() → current_exe 绝对路径；失败（二进制已删等）→ IOError。
fn native_sys_executable(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("IOError: cannot resolve executable: {}", e))?;
    Ok(alloc_string(&exe.to_string_lossy()))
}

/// sys.stdin_read_all() → 读 stdin 至 EOF（管道/重定向场景）。
/// 非 UTF-8 → IOError（lossy 不可逆，宁可报错）。交互 REPL 下阻塞至 EOF，
/// 仅面向 `ms run script.ms < input` 用法（10-builtins.md 注明）。
fn native_sys_stdin_read_all(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("IOError: cannot read stdin: {}", e))?;
    Ok(alloc_string(&buf))
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{run_source, strval, vm};
    use super::*;
    use crate::vm::object::TypeTag;

    #[test]
    fn test_sys_module_registration() {
        let ptr = register_sys_module();
        // SAFETY: ptr 由 register_sys_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "sys");
            assert_eq!(m.exports.len(), 4);
            for name in ["platform", "version", "executable", "stdin_read_all"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_sys_platform() {
        let mut v = vm();
        let p = strval(&native_sys_platform(&mut v, &[]).unwrap());
        assert!(
            ["windows", "linux", "macos"].contains(&p.as_str()),
            "platform: {}",
            p
        );
    }

    #[test]
    fn test_sys_version() {
        let mut v = vm();
        let ver = strval(&native_sys_version(&mut v, &[]).unwrap());
        assert!(ver.starts_with("mslang "), "version 前缀: {}", ver);
        assert!(ver.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn test_sys_executable() {
        let mut v = vm();
        match native_sys_executable(&mut v, &[]) {
            Ok(exe) => assert!(!strval(&exe).is_empty()),
            Err(e) => assert!(e.contains("IOError")),
        }
    }

    // stdin_read_all 的管道用例见 tests/sys_stdin.rs（子进程级，
    // 避免吞掉测试进程自身的 stdin）。

    #[test]
    fn test_integration_sys() {
        let src = r#"
import sys
p = sys.platform()
assert(p == "windows" or p == "linux" or p == "macos", "platform 枚举")
v = sys.version()
assert(v.startswith("mslang"), "version 前缀")
assert(len(sys.executable()) > 0, "executable 非空")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "sys integration failed: {:?}", r.err());
    }
}
