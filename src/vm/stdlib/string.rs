//! `string` 原生模块与 String 内建方法。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)
//! 与 [50-builtin-methods-string](../../../docs/mslang/tasks/50-builtin-methods-string.md)。

use super::{expect_int, expect_list_ref, expect_string};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_list, alloc_module, alloc_string, read_list, read_module_mut, read_str, MsObjHeader,
    Object, TypeTag,
};
use crate::vm::VM;

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
            result.push_str(&crate::vm::builtins::object_to_string(vm, val)?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, strval, vm};

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
}
