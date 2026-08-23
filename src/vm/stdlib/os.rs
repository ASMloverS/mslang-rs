//! `os` 原生模块。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_list, alloc_module, alloc_string, read_module_mut, MsObjHeader, Object,
};
use crate::vm::VM;

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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, strval, vm};
    use crate::vm::object::{read_list, TypeTag};

    // ---- task 48：os 模块 ----

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
}
