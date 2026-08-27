//! `string` 原生模块与 String 内建方法。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)
//! 与 [50-builtin-methods-string](../../../docs/mslang/tasks/50-builtin-methods-string.md)。

use super::{expect_int, expect_list_ref, expect_string};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_list, alloc_module, alloc_string, alloc_tuple, read_list, read_module_mut, read_str,
    MsObjHeader, Object, TypeTag,
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
/// exports 含 5 个原有函数 + 18 个扩充函数（task 80，16-stdlib-expansion.md §4.2）。
pub fn register_string_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 23] = [
        ("format", native_string_format),
        ("repeat", native_string_repeat),
        ("reverse", native_string_reverse),
        ("is_alpha", native_string_is_alpha),
        ("is_digit", native_string_is_digit),
        // task 80 扩充
        ("count", native_string_count),
        ("find", native_string_find),
        ("title", native_string_title),
        ("capitalize", native_string_capitalize),
        ("pad_start", native_string_pad_start),
        ("pad_end", native_string_pad_end),
        ("center", native_string_center),
        ("zfill", native_string_zfill),
        ("split_lines", native_string_split_lines),
        ("trim_start", native_string_trim_start),
        ("trim_end", native_string_trim_end),
        ("is_alnum", native_string_is_alnum),
        ("is_space", native_string_is_space),
        ("is_upper", native_string_is_upper),
        ("is_lower", native_string_is_lower),
        ("cut", native_string_cut),
        ("fields", native_string_fields),
        ("join", native_string_join),
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

/// string.format(template, *args) → 占位符替换（task 80 增强）。
///
/// - `{}` 顺序替换：非 string 参数经 object_to_string 转换（与 print/str 一致）；
/// - `{{` / `}}` 输出字面花括号；
/// - `{:.Nf}` 定点（N ∈ 0..=9）：接受 Float 与 Int（Int 按 Float 格式化），
///   其余类型 → TypeError；
/// - 其余任何 `{x`/未闭合/非法规格 → ValueError 附原文片段；单独 `}` → ValueError
///   （Python 对齐：Single '}' encountered）。
///
/// 解析为单遍字符扫描状态机（见 task 80 §format 解析状态机）。
fn native_string_format(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let template = expect_string(args.get(0), "format(template, ...)")?;
    let mut result = String::new();
    let mut arg_idx = 1usize;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => match chars.peek() {
                Some('}') => {
                    // `{}` 占位：顺序替换。
                    chars.next(); // 消费 '}'
                    let val = args.get(arg_idx).ok_or_else(|| {
                        format!(
                            "ValueError: format: not enough arguments for placeholder #{}",
                            arg_idx
                        )
                    })?;
                    result.push_str(&crate::vm::builtins::object_to_string(vm, val)?);
                    arg_idx += 1;
                }
                Some('{') => {
                    // `{{` 转义：输出字面 `{`（消费两个字符）。
                    chars.next();
                    result.push('{');
                }
                Some(':') => {
                    // 格式段：读至 `}`，段内须为 `.` + 1 位数字 + `f`（{:.Nf}）。
                    chars.next(); // 消费 ':'
                    let mut seg = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(ch) => seg.push(ch),
                            None => {
                                return Err(format!(
                                    "ValueError: format: unclosed format spec '{{:{}'",
                                    seg
                                ))
                            }
                        }
                    }
                    let seg_chars: Vec<char> = seg.chars().collect();
                    let precision = match seg_chars.as_slice() {
                        ['.', d, 'f'] if d.is_ascii_digit() => *d as u8 - b'0',
                        _ => {
                            return Err(format!(
                                "ValueError: format: invalid format spec '{{:{}{}'",
                                seg, '}'
                            ))
                        }
                    };
                    let val = args.get(arg_idx).ok_or_else(|| {
                        format!(
                            "ValueError: format: not enough arguments for placeholder #{}",
                            arg_idx
                        )
                    })?;
                    let f = match val {
                        Object::Int(i) => *i as f64,
                        Object::Float(x) => *x,
                        other => {
                            return Err(format!(
                                "TypeError: format spec '{{:.{}f}}' expects number, got {}",
                                precision,
                                other.type_name()
                            ))
                        }
                    };
                    result.push_str(&format!("{:.*}", precision as usize, f));
                    arg_idx += 1;
                }
                other => {
                    // `{` 后其余任何字符（含 `{` 嵌套）或未闭合 → ValueError 附片段。
                    return match other {
                        Some(ch) => Err(format!(
                            "ValueError: format: unexpected '{{{}' in format string",
                            ch
                        )),
                        None => Err("ValueError: format: unclosed '{' in format string".to_string()),
                    };
                }
            },
            '}' => match chars.peek() {
                Some('}') => {
                    // `}}` 转义：输出字面 `}`（消费两个字符）。
                    chars.next();
                    result.push('}');
                }
                _ => {
                    return Err(
                        "ValueError: format: single '}' encountered in format string".to_string()
                    )
                }
            },
            c => result.push(c),
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
// task 80 扩充（16-stdlib-expansion.md §4.2）
// ---------------------------------------------------------------------------

/// count(s, sub) → 非重叠出现次数；空 sub → 0。
/// arity MAX（与 gc.count=0 共享名）：native 内自校验恰 2 参。
fn native_string_count(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err(format!(
            "TypeError: count(s, sub) takes exactly 2 arguments, got {}",
            args.len()
        ));
    }
    let s = expect_string(args.get(0), "count(s, sub)")?;
    let sub = expect_string(args.get(1), "count(s, sub)")?;
    if sub.is_empty() {
        return Ok(Object::Int(0));
    }
    Ok(Object::Int(s.matches(&sub).count() as i64))
}

/// find(s, sub) → 首个字符索引；未找到 -1（与 `s.index()` 抛 ValueError 区分）。
fn native_string_find(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "find(s, sub)")?;
    let sub = expect_string(args.get(1), "find(s, sub)")?;
    match s.find(&sub) {
        // find 返回字节位置，转字符位置（与 length/index/slice 一致）。
        Some(byte_pos) => Ok(Object::Int(s[..byte_pos].chars().count() as i64)),
        None => Ok(Object::Int(-1)),
    }
}

/// title(s) → 每个词首字母大写其余小写（Python 语义：非字母后的字符大写）。
fn native_string_title(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "title(s)")?;
    let mut out = String::with_capacity(s.len());
    let mut prev_alpha = false;
    for c in s.chars() {
        if prev_alpha {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        prev_alpha = c.is_alphabetic();
    }
    Ok(alloc_string(&out))
}

/// capitalize(s) → 首字符大写其余小写。
fn native_string_capitalize(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "capitalize(s)")?;
    let mut chars = s.chars();
    let mut out = String::with_capacity(s.len());
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.extend(chars.flat_map(|c| c.to_lowercase()));
    }
    Ok(alloc_string(&out))
}

/// pad 校验与公共参数：n 为结果总长（字符数）；pad 取首字符循环。
/// 已长于 n（或 n 为负）返回 None（调用方返回 s 副本）；
/// Some((pad 字符, 填充数))。
fn pad_args(
    s: &str,
    n: i64,
    pad: Option<&Object>,
    who: &str,
) -> Result<Option<(char, usize)>, String> {
    let len = s.chars().count();
    let target = if n < 0 { 0 } else { n as usize };
    if target <= len {
        return Ok(None);
    }
    let pad_str = match pad {
        None => " ".to_string(),
        Some(_) => expect_string(pad, who)?,
    };
    let pad_char = pad_str
        .chars()
        .next()
        .ok_or_else(|| "ValueError: pad string must not be empty".to_string())?;
    Ok(Some((pad_char, target - len)))
}

/// pad_start(s, n, pad=" ") → 左填充至总长 n（Python rjust 语义）。arity MAX（2-3 参）。
fn native_string_pad_start(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "TypeError: pad_start(s, n, pad?) takes 2-3 arguments, got {}",
            args.len()
        ));
    }
    let s = expect_string(args.get(0), "pad_start(s, n, pad?)")?;
    let n = expect_int(args.get(1), "pad_start(s, n, pad?)")?;
    match pad_args(&s, n, args.get(2), "pad_start(s, n, pad?)")? {
        None => Ok(alloc_string(&s)),
        Some((pad_char, pad_n)) => {
            let mut out = String::with_capacity(s.len() + pad_n);
            out.extend(std::iter::repeat_n(pad_char, pad_n));
            out.push_str(&s);
            Ok(alloc_string(&out))
        }
    }
}

/// pad_end(s, n, pad=" ") → 右填充至总长 n（Python ljust 语义）。arity MAX（2-3 参）。
fn native_string_pad_end(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "TypeError: pad_end(s, n, pad?) takes 2-3 arguments, got {}",
            args.len()
        ));
    }
    let s = expect_string(args.get(0), "pad_end(s, n, pad?)")?;
    let n = expect_int(args.get(1), "pad_end(s, n, pad?)")?;
    match pad_args(&s, n, args.get(2), "pad_end(s, n, pad?)")? {
        None => Ok(alloc_string(&s)),
        Some((pad_char, pad_n)) => {
            let mut out = String::with_capacity(s.len() + pad_n);
            out.push_str(&s);
            out.extend(std::iter::repeat_n(pad_char, pad_n));
            Ok(alloc_string(&out))
        }
    }
}

/// center(s, n, pad=" ") → 居中，左短右长（Python 语义）。arity MAX（2-3 参）。
fn native_string_center(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "TypeError: center(s, n, pad?) takes 2-3 arguments, got {}",
            args.len()
        ));
    }
    let s = expect_string(args.get(0), "center(s, n, pad?)")?;
    let n = expect_int(args.get(1), "center(s, n, pad?)")?;
    match pad_args(&s, n, args.get(2), "center(s, n, pad?)")? {
        None => Ok(alloc_string(&s)),
        Some((pad_char, pad_n)) => {
            let left = pad_n / 2; // 左短右长
            let right = pad_n - left;
            let mut out = String::with_capacity(s.len() + pad_n);
            out.extend(std::iter::repeat_n(pad_char, left));
            out.push_str(&s);
            out.extend(std::iter::repeat_n(pad_char, right));
            Ok(alloc_string(&out))
        }
    }
}

/// zfill(s, n) → 左补零至长 n，保留符号位（"-42" → "-0042"）。
fn native_string_zfill(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "zfill(s, n)")?;
    let n = expect_int(args.get(1), "zfill(s, n)")?;
    let len = s.chars().count();
    if n <= len as i64 {
        return Ok(alloc_string(&s));
    }
    let pad_n = (n - len as i64) as usize;
    // 符号位（-/+）后补零（Python zfill 语义）。
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => match s.strip_prefix('+') {
            Some(rest) => ("+", rest),
            None => ("", s.as_str()),
        },
    };
    let mut out = String::with_capacity(s.len() + pad_n);
    out.push_str(sign);
    out.extend(std::iter::repeat_n('0', pad_n));
    out.push_str(digits);
    Ok(alloc_string(&out))
}

/// split_lines(s) → 按行分割去除行尾；`\n`/`\r\n`/`\r` 均识别。
/// 尾部行尾不产生额外空行；空串 → 空 list（Python splitlines 语义）。
fn native_string_split_lines(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "split_lines(s)")?;
    let mut lines: Vec<Object> = Vec::new();
    if s.is_empty() {
        return Ok(alloc_list(lines));
    }
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                lines.push(alloc_string(&current));
                current.clear();
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next(); // 消费 \r\n 的 \n
                }
                lines.push(alloc_string(&current));
                current.clear();
            }
            c => current.push(c),
        }
    }
    // 尾部无行尾的残余内容（空则丢弃：尾行尾不产生空行）。
    if !current.is_empty() {
        lines.push(alloc_string(&current));
    }
    Ok(alloc_list(lines))
}

fn native_string_trim_start(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "trim_start(s)")?;
    Ok(alloc_string(s.trim_start()))
}

fn native_string_trim_end(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "trim_end(s)")?;
    Ok(alloc_string(s.trim_end()))
}

fn native_string_is_alnum(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_alnum(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())))
}

fn native_string_is_space(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_space(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_whitespace())))
}

/// is_upper(s)：所有有大小写字符为大写，且至少一个（Python 语义）。
fn native_string_is_upper(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_upper(s)")?;
    let mut has_cased = false;
    let mut all_upper = true;
    for c in s.chars() {
        if c.is_lowercase() {
            has_cased = true;
            all_upper = false;
        } else if c.is_uppercase() {
            has_cased = true;
        }
    }
    Ok(Object::Bool(has_cased && all_upper))
}

/// is_lower(s)：所有有大小写字符为小写，且至少一个（Python 语义）。
fn native_string_is_lower(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_lower(s)")?;
    let mut has_cased = false;
    let mut all_lower = true;
    for c in s.chars() {
        if c.is_uppercase() {
            has_cased = true;
            all_lower = false;
        } else if c.is_lowercase() {
            has_cased = true;
        }
    }
    Ok(Object::Bool(has_cased && all_lower))
}

/// cut(s, sep) → tuple(before, after)：以第一个 sep 切两段；无 sep → (s, "")
/// （Go strings.Cut 去 found 布尔）。空 sep → ValueError（与 split 一致）。
fn native_string_cut(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "cut(s, sep)")?;
    let sep = expect_string(args.get(1), "cut(s, sep)")?;
    if sep.is_empty() {
        return Err("ValueError: empty separator".to_string());
    }
    match s.find(&sep) {
        Some(byte_pos) => Ok(alloc_tuple(vec![
            alloc_string(&s[..byte_pos]),
            alloc_string(&s[byte_pos + sep.len()..]),
        ])),
        None => Ok(alloc_tuple(vec![alloc_string(&s), alloc_string("")])),
    }
}

/// fields(s) → 按连续空白分割（Go strings.Fields）。
fn native_string_fields(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "fields(s)")?;
    Ok(alloc_list(s.split_whitespace().map(alloc_string).collect()))
}

/// join(sep, list) → 模块级 join，与 `sep.join(list)` 方法等价。
/// arity MAX（与 path.join 共享名）：native 内自校验恰 2 参。
fn native_string_join(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err(format!(
            "TypeError: join(sep, list) takes exactly 2 arguments, got {}",
            args.len()
        ));
    }
    native_str_join(vm, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, strval, vm};
    use crate::vm::object::read_tuple;

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

    // ---- task 80：format 增强（状态机全分支）----

    #[test]
    // 3.14159 为设计文档示例值（非 PI 近似），spec 指定保留。
    #[allow(clippy::approx_constant)]
    fn test_format_precision() {
        let mut v = vm();
        assert_eq!(
            native_string_format(&mut v, &[s("{:.2f}"), Object::Float(3.14159)]).unwrap(),
            s("3.14")
        );
        // Int 按 Float 格式化：{:.2f} 于 3 → "3.00"
        assert_eq!(
            native_string_format(&mut v, &[s("{:.2f}"), Object::Int(3)]).unwrap(),
            s("3.00")
        );
        // N=0 与 N=9 边界
        assert_eq!(
            native_string_format(&mut v, &[s("{:.0f}"), Object::Float(3.7)]).unwrap(),
            s("4")
        );
        assert_eq!(
            native_string_format(&mut v, &[s("{:.9f}"), Object::Float(1.0)]).unwrap(),
            s("1.000000000")
        );
        // 混合占位：顺序消费参数
        assert_eq!(
            native_string_format(&mut v, &[s("x = {:.1f}, y = {}"), Object::Float(2.26), Object::Int(7)]).unwrap(),
            s("x = 2.3, y = 7")
        );
    }

    #[test]
    fn test_format_brace_escapes() {
        let mut v = vm();
        assert_eq!(native_string_format(&mut v, &[s("{{}}")]).unwrap(), s("{}"));
        assert_eq!(native_string_format(&mut v, &[s("{{a}}")]).unwrap(), s("{a}"));
        assert_eq!(native_string_format(&mut v, &[s("{{")]).unwrap(), s("{"));
        assert_eq!(native_string_format(&mut v, &[s("}}")]).unwrap(), s("}"));
        // 转义与占位混合
        assert_eq!(
            native_string_format(&mut v, &[s("{{{}}}"), Object::Int(1)]).unwrap(),
            s("{1}")
        );
    }

    #[test]
    fn test_format_errors() {
        let mut v = vm();
        // {:.Nf} 于非数值 → TypeError
        let err = native_string_format(&mut v, &[s("{:.2f}"), s("x")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // 单独 } → ValueError（Python 对齐：Single '}' encountered）
        let err = native_string_format(&mut v, &[s("a}b")]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("'}'"), "got: {}", err);
        // 非法规格 {:x}
        let err = native_string_format(&mut v, &[s("{:x}"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("{:x"), "got: {}", err);
        // 超范围精度 {:.10f}（N ∈ 0..=9）
        let err = native_string_format(&mut v, &[s("{:.10f}"), Object::Float(1.0)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("{:.10f"), "got: {}", err);
        // { 后非法字符
        let err = native_string_format(&mut v, &[s("{a}")]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // 未闭合 {:
        let err = native_string_format(&mut v, &[s("{:.2")]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("unclosed"), "got: {}", err);
        // 未闭合 lone {
        let err = native_string_format(&mut v, &[s("ab{")]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // 占位参数不足（规格段同样计数）
        let err = native_string_format(&mut v, &[s("{:.2f}")]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("not enough"), "got: {}", err);
    }

    // ---- task 80：string 扩充函数 ----

    /// 从 Object 提取 list 的 Vec 拷贝（测试辅助）。
    fn list_items(o: &Object) -> Vec<Object> {
        match o {
            Object::Ref(p) => unsafe { read_list(*p) }.clone(),
            _ => panic!("expected list ref"),
        }
    }

    /// 从 Object 提取 tuple 的 Vec 拷贝（测试辅助）。
    fn tuple_items(o: &Object) -> Vec<Object> {
        match o {
            Object::Ref(p) => unsafe { read_tuple(*p) }.clone(),
            _ => panic!("expected tuple ref"),
        }
    }

    #[test]
    fn test_string_count_and_find() {
        let mut v = vm();
        assert_eq!(
            native_string_count(&mut v, &[s("aaa"), s("a")]).unwrap(),
            Object::Int(3)
        );
        // 非重叠
        assert_eq!(
            native_string_count(&mut v, &[s("aaaa"), s("aa")]).unwrap(),
            Object::Int(2)
        );
        // 空 sub → 0
        assert_eq!(
            native_string_count(&mut v, &[s("abc"), s("")]).unwrap(),
            Object::Int(0)
        );
        // arity 自校验（MAX，与 gc.count 共享名）
        let err = native_string_count(&mut v, &[s("a")]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("exactly 2"), "got: {}", err);
        let err = native_string_count(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // find：字符位置；未找到 -1
        assert_eq!(
            native_string_find(&mut v, &[s("hello"), s("ll")]).unwrap(),
            Object::Int(2)
        );
        assert_eq!(
            native_string_find(&mut v, &[s("日本語"), s("本")]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            native_string_find(&mut v, &[s("hello"), s("xx")]).unwrap(),
            Object::Int(-1)
        );
    }

    #[test]
    fn test_string_title_and_capitalize() {
        let mut v = vm();
        assert_eq!(as_str(&native_string_title(&mut v, &[s("hello world")]).unwrap()), "Hello World");
        // 非字母后首字母大写（Python 语义）
        assert_eq!(as_str(&native_string_title(&mut v, &[s("they're")]).unwrap()), "They'Re");
        assert_eq!(as_str(&native_string_title(&mut v, &[s("")]).unwrap()), "");
        assert_eq!(
            as_str(&native_string_capitalize(&mut v, &[s("hello WORLD")]).unwrap()),
            "Hello world"
        );
        assert_eq!(as_str(&native_string_capitalize(&mut v, &[s("")]).unwrap()), "");
    }

    #[test]
    fn test_string_padding() {
        let mut v = vm();
        // n 为结果总长（Python rjust/ljust 语义）
        assert_eq!(as_str(&native_string_pad_start(&mut v, &[s("42"), Object::Int(5)]).unwrap()), "   42");
        assert_eq!(
            as_str(&native_string_pad_start(&mut v, &[s("42"), Object::Int(5), s("0")]).unwrap()),
            "00042"
        );
        assert_eq!(
            as_str(&native_string_pad_end(&mut v, &[s("42"), Object::Int(5), s("*")]).unwrap()),
            "42***"
        );
        // 已长于 n → 返回 s 副本；n 负 → 同
        assert_eq!(as_str(&native_string_pad_start(&mut v, &[s("hello"), Object::Int(3)]).unwrap()), "hello");
        assert_eq!(as_str(&native_string_pad_end(&mut v, &[s("hello"), Object::Int(-1)]).unwrap()), "hello");
        // center：左短右长
        assert_eq!(as_str(&native_string_center(&mut v, &[s("abc"), Object::Int(10)]).unwrap()), "   abc    ");
        assert_eq!(
            as_str(&native_string_center(&mut v, &[s("abc"), Object::Int(7), s("-")]).unwrap()),
            "--abc--"
        );
        // zfill 符号保留
        assert_eq!(as_str(&native_string_zfill(&mut v, &[s("-42"), Object::Int(5)]).unwrap()), "-0042");
        assert_eq!(as_str(&native_string_zfill(&mut v, &[s("42"), Object::Int(5)]).unwrap()), "00042");
        assert_eq!(as_str(&native_string_zfill(&mut v, &[s("+42"), Object::Int(5)]).unwrap()), "+0042");
        assert_eq!(as_str(&native_string_zfill(&mut v, &[s("12345"), Object::Int(3)]).unwrap()), "12345");
        // arity 自校验（MAX）
        let err = native_string_pad_start(&mut v, &[s("42")]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("2-3"), "got: {}", err);
        let err = native_string_center(&mut v, &[s("a"), Object::Int(1), s("x"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_string_split_lines_and_trim() {
        let mut v = vm();
        // 三种行尾
        let r = native_string_split_lines(&mut v, &[s("a\nb\r\nc\rd")]).unwrap();
        let items = list_items(&r);
        assert_eq!(items.len(), 4);
        assert_eq!(as_str(&items[1]), "b");
        assert_eq!(as_str(&items[3]), "d");
        // 尾部行尾不产生空行
        let r = native_string_split_lines(&mut v, &[s("a\n")]).unwrap();
        assert_eq!(list_items(&r).len(), 1);
        // 空行保留（行间）
        let r = native_string_split_lines(&mut v, &[s("a\n\nb")]).unwrap();
        assert_eq!(list_items(&r).len(), 3);
        // 空串 → 空 list
        let r = native_string_split_lines(&mut v, &[s("")]).unwrap();
        assert_eq!(list_items(&r).len(), 0);
        // trim
        assert_eq!(as_str(&native_string_trim_start(&mut v, &[s("  x  ")]).unwrap()), "x  ");
        assert_eq!(as_str(&native_string_trim_end(&mut v, &[s("  x  ")]).unwrap()), "  x");
    }

    #[test]
    fn test_string_predicates() {
        let mut v = vm();
        assert_eq!(native_string_is_alnum(&mut v, &[s("abc123")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_alnum(&mut v, &[s("")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_alnum(&mut v, &[s("a b")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_space(&mut v, &[s(" \t\n")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_space(&mut v, &[s("")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_space(&mut v, &[s(" x")]).unwrap(), Object::Bool(false));
        // is_upper/is_lower：至少一个有大小写字母（Python 语义）
        assert_eq!(native_string_is_upper(&mut v, &[s("ABC1")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_upper(&mut v, &[s("abc")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_upper(&mut v, &[s("123")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_upper(&mut v, &[s("")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_lower(&mut v, &[s("abc1")]).unwrap(), Object::Bool(true));
        assert_eq!(native_string_is_lower(&mut v, &[s("Abc")]).unwrap(), Object::Bool(false));
        assert_eq!(native_string_is_lower(&mut v, &[s("123")]).unwrap(), Object::Bool(false));
    }

    #[test]
    fn test_string_cut_fields_join() {
        let mut v = vm();
        // cut：首个 sep 切两段
        let t = native_string_cut(&mut v, &[s("a,b,c"), s(",")]).unwrap();
        let items = tuple_items(&t);
        assert_eq!(items.len(), 2);
        assert_eq!(as_str(&items[0]), "a");
        assert_eq!(as_str(&items[1]), "b,c");
        // 无 sep → (s, "")
        let t = native_string_cut(&mut v, &[s("abc"), s(",")]).unwrap();
        let items = tuple_items(&t);
        assert_eq!(as_str(&items[0]), "abc");
        assert_eq!(as_str(&items[1]), "");
        // 空 sep → ValueError
        let err = native_string_cut(&mut v, &[s("abc"), s("")]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // fields：连续空白分割
        let r = native_string_fields(&mut v, &[s("  a \t b  ")]).unwrap();
        let items = list_items(&r);
        assert_eq!(items.len(), 2);
        assert_eq!(as_str(&items[0]), "a");
        assert_eq!(as_str(&items[1]), "b");
        // join：模块级与 sep.join(list) 等价
        let lst = alloc_list(vec![s("a"), s("b"), s("c")]);
        assert_eq!(as_str(&native_string_join(&mut v, &[s("-"), lst]).unwrap()), "a-b-c");
        // join arity 自校验（MAX，与 path.join 共享名）
        let err = native_string_join(&mut v, &[s("-")]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("exactly 2"), "got: {}", err);
    }

    #[test]
    fn test_integration_string_ext() {
        // 端到端：task 80 扩充（等价 test_string_ext.ms 的值域部分）。
        let src = r#"
import string
assert(string.count("aaa", "a") == 3)
assert(string.find("hello", "xx") == -1)
assert(string.title("hello world") == "Hello World")
assert(string.capitalize("hello WORLD") == "Hello world")
assert(string.pad_start("42", 5) == "   42")
assert(string.zfill("-42", 5) == "-0042")
before, after = string.cut("a,b,c", ",")
assert(before == "a" and after == "b,c")
f = string.fields("  a \t b  ")
assert(len(f) == 2 and f[0] == "a" and f[1] == "b")
assert(string.join("-", ["a", "b"]) == "a-b")
lines = string.split_lines("a\nb\r\nc\rd")
assert(len(lines) == 4)
assert(string.format("{:.2f}", 3.14159) == "3.14")
assert(string.format("{:.2f}", 3) == "3.00")
assert(string.format("{{}}") == "{}")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "string ext integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_string_format_error_paths() {
        // 端到端错误路径（原生 Err 不经 try/except，整体 Err）。
        for (src, expect) in [
            ("string.format(\"{:.2f}\", \"x\")", "TypeError"),
            ("string.format(\"a}b\")", "ValueError"),
            ("string.format(\"{:x}\", 1)", "ValueError"),
            ("string.format(\"{:.10f}\", 1.0)", "ValueError"),
        ] {
            let full = format!("import string\n{}", src);
            let r = run_source(&full);
            assert!(r.is_err(), "{} should fail", src);
            let e = r.unwrap_err();
            assert!(e.contains(expect), "{}: expected {} in {}", src, expect, e);
        }
    }
}
