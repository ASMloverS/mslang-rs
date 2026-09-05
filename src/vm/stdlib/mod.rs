//! mslang 标准库原生模块。
//!
//! 参照 [46-stdlib-io](../../docs/mslang/tasks/46-stdlib-io.md) 与
//! [16-stdlib-expansion](../../docs/mslang/16-stdlib-expansion.md) §3.1（task 78 目录拆分）。
//!
//! 原生 Rust 模块经 `register_*` 构造 MsModule（task 45），exports 为原生函数
//! （`alloc_native_function` → Object::Ref + TypeTag::FUNCTION）。由 `ModuleResolver`
//! 的 `native_modules` 注册表登记，`import` 命中即跳过磁盘搜索。
//!
//! 各模块实现见同名子模块文件；本文件汇集各子模块的 `register_*` / `lookup_*`
//! 转发（`pub use`）与跨模块公共 helper，对外引用路径保持不变。

#![allow(clippy::get_first)]

mod r#async;
mod dict;
mod encoding;
mod fs;
mod gc;
mod hash;
mod heapq;
mod io;
mod json;
mod list;
mod math;
mod os;
mod path;
mod random;
mod regex;
mod set;
mod string;
mod sys;
mod time;
mod uuid;

pub use dict::lookup_dict_method;
pub use encoding::register_encoding_module;
pub use fs::register_fs_module;
pub use gc::register_gc_module;
pub use hash::register_hash_module;
pub use heapq::register_heapq_module;
pub use io::{lookup_file_method, native_io_open, register_io_module};
pub use json::register_json_module;
pub use list::lookup_list_method;
pub use math::register_math_module;
pub use os::register_os_module;
pub use path::register_path_module;
pub use r#async::register_async_module;
pub use random::register_random_module;
pub use regex::{lookup_match_method, lookup_regex_method, register_regex_module};
pub use set::lookup_set_method;
pub use string::{lookup_string_method, register_string_module};
pub use sys::register_sys_module;
pub use time::register_time_module;
pub use uuid::register_uuid_module;

use std::collections::HashMap;

use super::object::{read_str, read_tuple, MsObjHeader, Object, TypeTag};

// ---------------------------------------------------------------------------
// task 79：嵌入式 .ms 标准库
// ---------------------------------------------------------------------------

/// 嵌入式 `.ms` 标准库源码注册表（[16-stdlib-expansion](../../docs/mslang/16-stdlib-expansion.md) §3.2）。
///
/// 源码位于本目录 `ms/` 子目录，经 `include_str!` 编入二进制（路径以本文件为基准），
/// 单二进制自足发行。`VM::load_module` 在磁盘解析未命中后查此表兜底；当前为
/// 占位模块（仅导出 `VERSION`），内容由 task 84 填充。
pub fn embedded_sources() -> HashMap<String, &'static str> {
    HashMap::from([
        ("collections".to_string(), include_str!("ms/collections.ms")),
        ("itertools".to_string(), include_str!("ms/itertools.ms")),
        ("functools".to_string(), include_str!("ms/functools.ms")),
        ("test".to_string(), include_str!("ms/test.ms")),
    ])
}

// ---------------------------------------------------------------------------
// 公共辅助函数（被 ≥2 个子模块使用；仅单模块使用的 helper 留在各模块文件内）
// ---------------------------------------------------------------------------

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
                // task 84 修复：tuple 递归哈希遇嵌套 List/Dict/Set 元素时 Hash impl
                // panic（object.rs）。先递归校验元素可哈希性再哈希，杜绝 panic
                //（memoize 键 tuple(args) 含 list 时须得 TypeError 而非 abort）。
                if tag == TypeTag::TUPLE as u8 {
                    // SAFETY: type_tag 已守卫为 TUPLE。
                    for elem in unsafe { read_tuple(*ptr) }.clone() {
                        hash_key(&elem)?;
                    }
                }
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
mod test_util {
    use crate::compiler::Compiler;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::object::{alloc_list, alloc_string, read_str, Object, TypeTag};
    use crate::vm::VM;

    pub(super) fn vm() -> VM {
        VM::new()
    }

    pub(super) fn s(v: &str) -> Object {
        alloc_string(v)
    }

    /// 编译并运行 mslang 源码（集成测试辅助）。
    pub(super) fn run_source(source: &str) -> Result<Object, String> {
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
    pub(super) fn temp_path(name: &str) -> String {
        let dir = std::env::temp_dir().join("mslang_io_integration");
        std::fs::create_dir_all(&dir).ok();
        dir.join(name)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// 提取 Object::Float 内部值（单测辅助）。
    pub(super) fn fval(o: &Object) -> f64 {
        match o {
            Object::Float(x) => *x,
            _ => panic!("expected Float, got {:?}", o.type_name()),
        }
    }

    /// 提取 Object::String 内部值（单测辅助）。
    pub(super) fn strval(o: &Object) -> String {
        match o {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
                unsafe { read_str(*ptr) }.to_owned()
            }
            _ => panic!("expected String, got {}", o.type_name()),
        }
    }

    pub(super) fn ilist(nums: &[i64]) -> Object {
        let items: Vec<Object> = nums.iter().map(|n| Object::Int(*n)).collect();
        alloc_list(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::object::alloc_tuple;
    use super::test_util::{ilist, run_source, s};

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
    #[allow(clippy::approx_constant)] // 测试合法使用 3.14 字面量（clippy 误报为 PI 近似）
    fn test_hash_key_valid_types() {
        assert!(hash_key(&Object::Nil).is_ok());
        assert!(hash_key(&Object::Bool(true)).is_ok());
        assert!(hash_key(&Object::Int(42)).is_ok());
        assert!(hash_key(&Object::Float(3.14)).is_ok());
        assert!(hash_key(&s("hello")).is_ok());
        assert!(hash_key(&alloc_tuple(vec![Object::Int(1), Object::Int(2)])).is_ok());
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
    fn test_memoize_unhashable_graceful() {
        // task 84：memoize 键 tuple(args) 含 list → TypeError（dict 行为上抛），
        // 须优雅 Err 而非 Rust panic（hash_key 递归校验嵌套可哈希性）。
        let src = r#"
import functools
m = functools.memoize(fn(x) { return x })
m([1, 2])
"#;
        let r = run_source(src);
        let e = r.unwrap_err();
        assert!(
            e.contains("TypeError") && e.contains("unhashable"),
            "got: {}",
            e
        );
    }

    #[test]
    fn test_nested_tuple_unhashable_graceful() {
        // 深层嵌套 unhashable（tuple 内 tuple 内 list）同样优雅报错。
        let src = r#"
d = {}
d[tuple([tuple([[1]])])] = 1
"#;
        let r = run_source(src);
        let e = r.unwrap_err();
        assert!(
            e.contains("TypeError") && e.contains("unhashable"),
            "got: {}",
            e
        );
    }
}
