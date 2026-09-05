//! `regex` 原生模块（task 85）。
//!
//! 参照 [85-stdlib-regex-hash](../../../docs/mslang/tasks/85-stdlib-regex-hash.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.16。
//!
//! 函数式（pattern 在前）+ `compile()` 对象式双入口；Match 对象带分组。
//! regex crate 语法为 Rust 方言（Unicode 类等超集；Perl 反向引用支持子集），
//! pattern 直接透传（差异在 10-builtins.md §regex 注明）。
//!
//! 偏移语义：regex crate 的 span 为**字节**偏移；Match 方法（start/end/span）
//! 为**字符**偏移（与 `s.index` 一致）。构造 Match 时经 [`byte_to_char_map`]
//! 一次前缀和预转换（查询 O(1)）；字节 spans 镜像保留供子串 O(1) 切片。

use super::{expect_int, expect_string};
use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::object::{
    alloc_list, alloc_match, alloc_module, alloc_regex, alloc_string, alloc_tuple, read_match,
    read_module_mut, read_regex, read_str, MsObjHeader, Object, TypeTag,
};
use crate::vm::VM;

/// 构造 `regex` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_regex_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    // sub/split 为 arity MAX（§2.2 同名冲突治理），native 内自校验参数个数。
    let funcs: [(&str, NativeFn); 6] = [
        ("match", native_regex_match),
        ("search", native_regex_search),
        ("findall", native_regex_findall),
        ("sub", native_regex_sub),
        ("split", native_regex_split),
        ("compile", native_regex_compile),
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
    let m = alloc_module("regex");
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

/// Regex 对象方法名 → 原生函数指针（供 GET_ATTR 包装为 BoundMethod）。
/// 方法调用经 BoundMethod→FUNCTION 路径不查 native_arities，各方法自校验参数个数。
pub fn lookup_regex_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "match" => native_regex_obj_match,
        "search" => native_regex_obj_search,
        "findall" => native_regex_obj_findall,
        "sub" => native_regex_obj_sub,
        "split" => native_regex_obj_split,
        "pattern" => native_regex_obj_pattern,
        _ => return None,
    };
    Some(func)
}

/// Match 对象方法名 → 原生函数指针（供 GET_ATTR 包装为 BoundMethod）。
pub fn lookup_match_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "group" => native_match_group,
        "groups" => native_match_groups,
        "start" => native_match_start,
        "end" => native_match_end,
        "span" => native_match_span,
        _ => return None,
    };
    Some(func)
}

// ---------------------------------------------------------------------------
// 参数校验辅助
// ---------------------------------------------------------------------------

/// 校验首参数为 Regex Ref，返回其裸指针。
fn expect_regex(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::REGEX as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects regex, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// 校验首参数为 Match Ref，返回其裸指针。
fn expect_match(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::MATCH as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: {} expects match, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// pattern → regex::Regex。非法 pattern → ValueError（附 regex::Error 详情）。
fn compile_pattern(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(pattern)
        .map_err(|e| format!("ValueError: invalid regex pattern '{}': {}", pattern, e))
}

/// repl 参数是否为可调用对象（FUNCTION/CLOSURE/BOUND_METHOD）。
fn is_callable_object(obj: &Object) -> bool {
    matches!(obj, Object::Ref(ptr)
    if {
        let tag = unsafe { (**ptr).type_tag };
        tag == TypeTag::FUNCTION as u8
            || tag == TypeTag::CLOSURE as u8
            || tag == TypeTag::BOUND_METHOD as u8
    })
}

// ---------------------------------------------------------------------------
// 字节偏移 ↔ 字符偏移（task 85 §字节偏移 ↔ 字符偏移）
// ---------------------------------------------------------------------------

/// 字节偏移 → 字符偏移前缀映射。`map[b] = b 处的字符索引`（长度 = 字节数 + 1；
/// 仅字符边界槽位有效——regex 匹配 span 恒为边界）。一次 `char_indices` O(n)。
fn byte_to_char_map(text: &str) -> Vec<usize> {
    let mut map = vec![0usize; text.len() + 1];
    let mut chars = 0usize;
    for (i, ch) in text.char_indices() {
        map[i] = chars;
        chars += 1;
        map[i + ch.len_utf8()] = chars;
    }
    map
}

/// Captures → 字节 spans（索引 0 = 整体匹配；未参组 None）。
fn byte_spans_from_caps(caps: &regex::Captures) -> Vec<Option<(usize, usize)>> {
    (0..caps.len())
        .map(|i| caps.get(i).map(|m| (m.start(), m.end())))
        .collect()
}

/// 字节 spans → 字符 spans（经映射预转换，平行镜像）。
fn char_spans_from_byte(
    bmap: &[usize],
    byte_spans: &[Option<(usize, usize)>],
) -> Vec<Option<(usize, usize)>> {
    byte_spans
        .iter()
        .map(|sp| sp.map(|(s, e)| (bmap[s], bmap[e])))
        .collect()
}

/// Captures → MsMatch 对象（构造期完成字节→字符预转换）。
fn match_object_from_caps(text: &str, bmap: &[usize], caps: &regex::Captures) -> Object {
    let byte_spans = byte_spans_from_caps(caps);
    let char_spans = char_spans_from_byte(bmap, &byte_spans);
    alloc_match(text.to_string(), byte_spans, char_spans)
}

// ---------------------------------------------------------------------------
// 核心操作（函数式与对象式共用）
// ---------------------------------------------------------------------------

/// Python re.match 语义：锚定开头。captures 返回最左匹配——若最左匹配起点
/// 非 0，则位置 0 不存在任何匹配（最左性），故 start == 0 判定等价锚定。
fn anchored_match(re: &regex::Regex, s: &str, bmap: &[usize]) -> Option<Object> {
    let caps = re.captures(s)?;
    if caps.get(0).is_some_and(|m| m.start() == 0) {
        Some(match_object_from_caps(s, bmap, &caps))
    } else {
        None
    }
}

/// Python re.search 语义：首个（最左）匹配。
fn search_once(re: &regex::Regex, s: &str, bmap: &[usize]) -> Option<Object> {
    let caps = re.captures(s)?;
    Some(match_object_from_caps(s, bmap, &caps))
}

/// findall 组策略（§findall 组策略）：0 组 → 整体 string；1 组 → 该组内容
/// （未参组 ""，维持 list[string] 不变量，Python 对齐）；≥2 组 → tuple
/// （未参组 nil）。
fn findall_impl(re: &regex::Regex, s: &str) -> Object {
    let n_groups = re.captures_len() - 1;
    let mut out: Vec<Object> = Vec::new();
    for caps in re.captures_iter(s) {
        if n_groups == 0 {
            let m = caps.get(0).expect("group 0 always participates");
            out.push(alloc_string(&s[m.start()..m.end()]));
        } else if n_groups == 1 {
            match caps.get(1) {
                Some(g) => out.push(alloc_string(&s[g.start()..g.end()])),
                None => out.push(alloc_string("")),
            }
        } else {
            let items: Vec<Object> = (1..caps.len())
                .map(|i| match caps.get(i) {
                    Some(g) => alloc_string(&s[g.start()..g.end()]),
                    None => Object::Nil,
                })
                .collect();
            out.push(alloc_tuple(items));
        }
    }
    alloc_list(out)
}

// ---------------------------------------------------------------------------
// repl 展开状态机（§repl 展开状态机）
// ---------------------------------------------------------------------------

/// repl 模板展开：`${N}` 分组引用（仅索引，无命名组 v1；`${0}` = 整体）；
/// `$` 后非 `{` 字面透传（含 `$` 结尾字面）。
///
/// 越界组引用 / 畸形格式（`${1x`、`${`、`${}`）→ ValueError，消息附 repl
/// 原文片段（对齐 string.format 错误风格）。未参组展开为空串（Python 3.5+ 对齐）。
fn expand_repl(
    template: &str,
    byte_spans: &[Option<(usize, usize)>],
    text: &str,
) -> Result<String, String> {
    let n_groups = byte_spans.len() - 1;
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar..];
        if after.as_bytes().get(1) == Some(&b'{') {
            let Some(close) = after.find('}') else {
                return Err(format!(
                    "ValueError: sub: unclosed group reference '{}'",
                    after
                ));
            };
            let digits = &after[2..close];
            let fragment = &after[..=close];
            let idx: usize = if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!(
                    "ValueError: sub: malformed group reference '{}'",
                    fragment
                ));
            } else {
                digits.parse().map_err(|_| {
                    format!("ValueError: sub: invalid group reference '{}'", fragment)
                })?
            };
            if idx > n_groups {
                return Err(format!(
                    "ValueError: sub: invalid group reference '{}' (pattern has {} group{})",
                    fragment,
                    n_groups,
                    if n_groups == 1 { "" } else { "s" }
                ));
            }
            // 未参组（None）展开为空串（Python 3.5+ re.sub 语义）。
            if let Some((bs, be)) = byte_spans[idx] {
                out.push_str(&text[bs..be]);
            }
            rest = &after[close + 1..];
        } else {
            // `$` 结尾或 `$` 后非 `{`：字面透传。
            out.push('$');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// sub 的 count 参数：缺省 / Nil → 0（全替换）；负值 → ValueError；非 Int → TypeError。
fn parse_sub_count(arg: Option<&Object>) -> Result<i64, String> {
    match arg {
        None | Some(Object::Nil) => Ok(0),
        Some(Object::Int(n)) => {
            if *n < 0 {
                Err("ValueError: count must be non-negative".to_string())
            } else {
                Ok(*n)
            }
        }
        other => Err(format!(
            "TypeError: sub count must be int, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

// ---------------------------------------------------------------------------
// sub 核心（§GC 安全：repl 函数回调复用 task 80 key 调用的同步入口）
// ---------------------------------------------------------------------------

/// sub 核心（函数式与对象式共用）。
///
/// `re` 传入 Rust 拥有的克隆（regex::Regex: Clone，内部 Arc 共享、廉价）、
/// `s` 为 Rust 本地 String——两者均无堆借用跨越 VM 重入（GC 窗口）。
/// repl 为函数时压栈根化（task 80 sort_items_dsu 同款纪律），每次回调后
/// 从根槽重取；回调异常经 escaped_exc 暂存，native 返回后由 call_value 重抛。
fn sub_impl(
    vm: &mut VM,
    re: regex::Regex,
    repl: Object,
    s: String,
    count: i64,
) -> Result<Object, String> {
    // repl 前置类型校验：string（`${N}` 模板）或 callable（接收 Match 返回 string）。
    let repl_is_fn = is_callable_object(&repl);
    let template = if repl_is_fn {
        String::new()
    } else {
        expect_string(Some(&repl), "sub(repl, s, count?)")?
    };
    // Phase 1（无 VM 重入，无 GC 窗口）：收集前 count 个匹配的字节 spans。
    let bmap = byte_to_char_map(&s);
    let mut all_spans: Vec<Vec<Option<(usize, usize)>>> = Vec::new();
    for caps in re.captures_iter(&s) {
        if count > 0 && all_spans.len() as i64 >= count {
            break;
        }
        all_spans.push(byte_spans_from_caps(&caps));
    }
    // Phase 2：repl 函数路径有 GC 窗口，根化后逐匹配替换。
    let root_base = vm.stack().len();
    if repl_is_fn {
        vm.push(repl.clone())?;
    }
    let ret = sub_replace_rooted(vm, root_base, repl_is_fn, &template, &all_spans, &bmap, &s);
    vm.stack_mut().truncate(root_base);
    ret
}

/// Phase 2：`root_base` 槽为 repl 函数根（仅 repl_is_fn 时有效）。
fn sub_replace_rooted(
    vm: &mut VM,
    root_base: usize,
    repl_is_fn: bool,
    template: &str,
    all_spans: &[Vec<Option<(usize, usize)>>],
    bmap: &[usize],
    s: &str,
) -> Result<Object, String> {
    let mut out = String::with_capacity(s.len());
    let mut last = 0usize;
    for spans in all_spans {
        let (bs, be) = spans[0].expect("group 0 always participates");
        out.push_str(&s[last..bs]);
        if repl_is_fn {
            let m = alloc_match(
                s.to_string(),
                spans.clone(),
                char_spans_from_byte(bmap, spans),
            );
            // repl 从根槽重取（防上次回调触发 GC 移动后旧 Ref 悬垂）。
            let repl_now = vm.stack()[root_base].clone();
            let piece = vm.call_function(&repl_now, &[m])?;
            if vm.escaped_exc.is_some() {
                // 回调异常已截获暂存：中止（结果丢弃），call_value 在本 native
                // 返回后重抛给调用方（可 try/except 捕获）。
                return Ok(Object::Nil);
            }
            match &piece {
                Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
                    // SAFETY: type_tag 已守卫为 STRING，指针由 alloc_string 分配。
                    out.push_str(unsafe { read_str(*ptr) });
                }
                other => {
                    return Err(format!(
                        "TypeError: sub repl function must return string, got {}",
                        other.type_name()
                    ))
                }
            }
        } else {
            out.push_str(&expand_repl(template, spans, s)?);
        }
        last = be;
    }
    out.push_str(&s[last..]);
    Ok(alloc_string(&out))
}

// ---------------------------------------------------------------------------
// 模块级函数（pattern 在前；sub/split 为 arity MAX，自校验参数个数）
// ---------------------------------------------------------------------------

/// regex.match(pattern, s) -> Match/nil：锚定开头（Python re.match）。arity 2。
fn native_regex_match(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let pattern = expect_string(args.get(0), "match(pattern, s)")?;
    let s = expect_string(args.get(1), "match(pattern, s)")?;
    let re = compile_pattern(&pattern)?;
    let bmap = byte_to_char_map(&s);
    Ok(anchored_match(&re, &s, &bmap).unwrap_or(Object::Nil))
}

/// regex.search(pattern, s) -> Match/nil：首个匹配。arity 2。
fn native_regex_search(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let pattern = expect_string(args.get(0), "search(pattern, s)")?;
    let s = expect_string(args.get(1), "search(pattern, s)")?;
    let re = compile_pattern(&pattern)?;
    let bmap = byte_to_char_map(&s);
    Ok(search_once(&re, &s, &bmap).unwrap_or(Object::Nil))
}

/// regex.findall(pattern, s) -> list。arity 2。
fn native_regex_findall(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let pattern = expect_string(args.get(0), "findall(pattern, s)")?;
    let s = expect_string(args.get(1), "findall(pattern, s)")?;
    let re = compile_pattern(&pattern)?;
    Ok(findall_impl(&re, &s))
}

/// regex.sub(pattern, repl, s, count=0) -> string。arity MAX：自校验 3-4 参
///（§2.2；count 缺省 0 = 全替换，负值 → ValueError，非 Int → TypeError）。
fn native_regex_sub(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() < 3 || args.len() > 4 {
        return Err(format!(
            "TypeError: sub(pattern, repl, s, count?) takes 3-4 arguments, got {}",
            args.len()
        ));
    }
    let pattern = expect_string(args.get(0), "sub(pattern, repl, s, count?)")?;
    let s = expect_string(args.get(2), "sub(pattern, repl, s, count?)")?;
    let count = parse_sub_count(args.get(3))?;
    let re = compile_pattern(&pattern)?;
    sub_impl(vm, re, args[1].clone(), s, count)
}

/// regex.split(pattern, s) -> list。arity MAX：与 string 方法 `s.split(sep?)`
/// 同名（§2.2 冲突治理），自校验恰好 2 参。未匹配返回 `[s]`。
fn native_regex_split(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err(format!(
            "TypeError: split(pattern, s) takes exactly 2 arguments, got {}",
            args.len()
        ));
    }
    let pattern = expect_string(args.get(0), "split(pattern, s)")?;
    let s = expect_string(args.get(1), "split(pattern, s)")?;
    let re = compile_pattern(&pattern)?;
    Ok(split_impl(&re, &s))
}

/// regex.compile(pattern) -> Regex：对象式入口。arity 1。
fn native_regex_compile(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let pattern = expect_string(args.get(0), "compile(pattern)")?;
    let re = compile_pattern(&pattern)?;
    Ok(alloc_regex(&pattern, re))
}

/// split 核心：regex crate `Regex::split`（空匹配按其内建规则分割，
/// Python 3.7+ 对齐）；无匹配时返回整体 `[s]`。
fn split_impl(re: &regex::Regex, s: &str) -> Object {
    alloc_list(re.split(s).map(alloc_string).collect())
}

// ---------------------------------------------------------------------------
// Regex 对象方法（receiver = MsRegex，经 BoundMethod 注入 args[0]）
// ---------------------------------------------------------------------------

/// re.match(s) -> Match/nil。恰 1 参。
fn native_regex_obj_match(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "match(s)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: match(s) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let s = expect_string(args.get(1), "match(s)")?;
    // SAFETY: ptr 经 expect_regex 校验为 alloc_regex 分配的 MsRegex；
    // 本函数无 VM 重入，&Regex 借用无 GC 窗口。
    let re = unsafe { read_regex(ptr) }.compiled.as_ref();
    let bmap = byte_to_char_map(&s);
    Ok(anchored_match(re, &s, &bmap).unwrap_or(Object::Nil))
}

/// re.search(s) -> Match/nil。恰 1 参。
fn native_regex_obj_search(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "search(s)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: search(s) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let s = expect_string(args.get(1), "search(s)")?;
    // SAFETY: ptr 经 expect_regex 校验为有效 MsRegex；无 VM 重入。
    let re = unsafe { read_regex(ptr) }.compiled.as_ref();
    let bmap = byte_to_char_map(&s);
    Ok(search_once(re, &s, &bmap).unwrap_or(Object::Nil))
}

/// re.findall(s) -> list。恰 1 参。
fn native_regex_obj_findall(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "findall(s)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: findall(s) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let s = expect_string(args.get(1), "findall(s)")?;
    // SAFETY: ptr 经 expect_regex 校验为有效 MsRegex；无 VM 重入。
    let re = unsafe { read_regex(ptr) }.compiled.as_ref();
    Ok(findall_impl(re, &s))
}

/// re.sub(repl, s, count?) -> string。2-3 参（BoundMethod 路径不查
/// native_arities，自校验）。
fn native_regex_obj_sub(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "sub(repl, s, count?)")?;
    if args.len() < 3 || args.len() > 4 {
        return Err(format!(
            "TypeError: sub(repl, s, count?) takes 2-3 arguments, got {}",
            args.len() - 1
        ));
    }
    let s = expect_string(args.get(2), "sub(repl, s, count?)")?;
    let count = parse_sub_count(args.get(3))?;
    // SAFETY: ptr 经 expect_regex 校验为有效 MsRegex；compiled 克隆为 Rust
    // 拥有值（Arc 共享廉价），杜绝堆借用跨越回调重入（GC 窗口）。
    let re = unsafe { read_regex(ptr) }.compiled.as_ref().clone();
    sub_impl(vm, re, args[1].clone(), s, count)
}

/// re.split(s) -> list。恰 1 参（对象方法路径自校验）。
fn native_regex_obj_split(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "split(s)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: split(s) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let s = expect_string(args.get(1), "split(s)")?;
    // SAFETY: ptr 经 expect_regex 校验为有效 MsRegex；无 VM 重入。
    let re = unsafe { read_regex(ptr) }.compiled.as_ref();
    Ok(split_impl(re, &s))
}

/// re.pattern() -> string：编译时 pattern 回读。0 参。
fn native_regex_obj_pattern(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_regex(args.get(0), "pattern()")?;
    if args.len() != 1 {
        return Err(format!(
            "TypeError: pattern() takes 0 arguments, got {}",
            args.len() - 1
        ));
    }
    // SAFETY: ptr 经 expect_regex 校验为有效 MsRegex。
    let pattern = unsafe { read_regex(ptr) }.pattern.as_str();
    Ok(alloc_string(pattern))
}

// ---------------------------------------------------------------------------
// Match 对象方法（receiver = MsMatch，经 BoundMethod 注入 args[0]）
// ---------------------------------------------------------------------------

/// m.group(i)：第 i 组（0 = 整体）。越界 / 负索引（v1 按越界，不支持 Python
/// 回绕）→ IndexError；非 Int → TypeError；未参组 → nil。恰 1 参。
fn native_match_group(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_match(args.get(0), "group(i)")?;
    if args.len() != 2 {
        return Err(format!(
            "TypeError: group(i) takes exactly 1 argument, got {}",
            args.len() - 1
        ));
    }
    let i = expect_int(args.get(1), "group(i)")?;
    // SAFETY: ptr 经 expect_match 校验为有效 MsMatch。
    let m = unsafe { read_match(ptr) };
    if i < 0 || i as usize >= m.byte_spans.len() {
        return Err(format!(
            "IndexError: no such group {} (pattern has {} group{})",
            i,
            m.byte_spans.len() - 1,
            if m.byte_spans.len() == 2 { "" } else { "s" }
        ));
    }
    // 字节偏移 O(1) 切片（镜像 spans 用途）。
    Ok(match m.byte_spans[i as usize] {
        Some((bs, be)) => alloc_string(&m.text[bs..be]),
        None => Object::Nil,
    })
}

/// m.groups() -> tuple：分组 1..n；未参组 nil。0 参。
fn native_match_groups(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_match(args.get(0), "groups()")?;
    if args.len() != 1 {
        return Err(format!(
            "TypeError: groups() takes 0 arguments, got {}",
            args.len() - 1
        ));
    }
    // SAFETY: ptr 经 expect_match 校验为有效 MsMatch。
    let m = unsafe { read_match(ptr) };
    let items: Vec<Object> = m.byte_spans[1..]
        .iter()
        .map(|sp| match sp {
            Some((bs, be)) => alloc_string(&m.text[*bs..*be]),
            None => Object::Nil,
        })
        .collect();
    Ok(alloc_tuple(items))
}

/// m.start()：整体匹配的起始字符偏移。0 参。
fn native_match_start(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_match(args.get(0), "start()")?;
    if args.len() != 1 {
        return Err(format!(
            "TypeError: start() takes 0 arguments, got {}",
            args.len() - 1
        ));
    }
    // SAFETY: ptr 经 expect_match 校验为有效 MsMatch。
    let m = unsafe { read_match(ptr) };
    match m.char_spans.first().copied().flatten() {
        Some((s, _)) => Ok(Object::Int(s as i64)),
        None => Err("IndexError: match has no span".to_string()),
    }
}

/// m.end()：整体匹配的结束字符偏移。0 参。
fn native_match_end(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_match(args.get(0), "end()")?;
    if args.len() != 1 {
        return Err(format!(
            "TypeError: end() takes 0 arguments, got {}",
            args.len() - 1
        ));
    }
    // SAFETY: ptr 经 expect_match 校验为有效 MsMatch。
    let m = unsafe { read_match(ptr) };
    match m.char_spans.first().copied().flatten() {
        Some((_, e)) => Ok(Object::Int(e as i64)),
        None => Err("IndexError: match has no span".to_string()),
    }
}

/// m.span() -> tuple(start, end)：字符偏移（与 `s.index` 语义一致）。0 参。
fn native_match_span(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_match(args.get(0), "span()")?;
    if args.len() != 1 {
        return Err(format!(
            "TypeError: span() takes 0 arguments, got {}",
            args.len() - 1
        ));
    }
    // SAFETY: ptr 经 expect_match 校验为有效 MsMatch。
    let m = unsafe { read_match(ptr) };
    match m.char_spans.first().copied().flatten() {
        Some((s, e)) => Ok(alloc_tuple(vec![
            Object::Int(s as i64),
            Object::Int(e as i64),
        ])),
        None => Err("IndexError: match has no span".to_string()),
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::test_util::{run_source, vm};
    use super::*;

    // ---- 字节↔字符偏移转换（中文/emoji 混合样本）----

    #[test]
    fn test_byte_to_char_map_ascii() {
        let m = byte_to_char_map("abc");
        assert_eq!(m, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_byte_to_char_map_mixed_cjk_emoji() {
        // "a中😀b"：字节长 1+3+4+1 = 9；字符边界 0(a) 1(中) 4(😀) 8(b) 9。
        let text = "a中😀b";
        assert_eq!(text.len(), 9);
        let m = byte_to_char_map(text);
        assert_eq!(m.len(), 10);
        assert_eq!(m[0], 0); // 'a'
        assert_eq!(m[1], 1); // '中'
        assert_eq!(m[4], 2); // '😀'
        assert_eq!(m[8], 3); // 'b'
        assert_eq!(m[9], 4); // 末尾
    }

    #[test]
    fn test_byte_to_char_map_empty() {
        assert_eq!(byte_to_char_map(""), vec![0]);
    }

    /// 端到端偏移一致性：中文字符串上 search 的 start/end/span 为字符偏移
    ///（与 `s.index` 语义一致，验证标准 3）。
    #[test]
    fn test_char_offsets_on_chinese_input() {
        let src = r#"
import regex
zh = regex.search("世界", "你好世界，mslang")
assert(zh.start() == 2, "start 字符偏移")
assert(zh.end() == 4)
assert(zh.span() == (2, 4))
assert(zh.group(0) == "世界")
assert(zh.start() == "你好世界，mslang".index("世界"))
e = regex.search("mslang", "你好世界，mslang")
assert(e.start() == 5, "中文后的 ASCII 起点仍按字符计（含全角逗号）")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "char offsets failed: {:?}", r.err());
    }

    // ---- repl 状态机边界 ----

    /// 构造单次整体匹配的 byte_spans（整体 = text 全串）。
    fn whole_span(text: &str) -> Vec<Option<(usize, usize)>> {
        vec![Some((0, text.len())), None]
    }

    #[test]
    fn test_expand_repl_dollar_tail_literal() {
        // `$` 结尾字面（无 `{` 跟随）。
        let spans = whole_span("ab");
        assert_eq!(expand_repl("x$", &spans, "ab").unwrap(), "x$");
        assert_eq!(expand_repl("$", &spans, "ab").unwrap(), "$");
        assert_eq!(expand_repl("a$b", &spans, "ab").unwrap(), "a$b");
    }

    #[test]
    fn test_expand_repl_group0_whole_reference() {
        // `{0}` 整体引用。
        let spans = whole_span("ab");
        assert_eq!(expand_repl("[${0}]", &spans, "ab").unwrap(), "[ab]");
    }

    #[test]
    fn test_expand_repl_group_expansion() {
        // 分组引用 + 未参组 → 空串（Python 3.5+ 对齐）。
        let spans = vec![Some((0, 3)), Some((0, 1)), Some((2, 3)), None];
        assert_eq!(expand_repl("${1}-${2}", &spans, "abc").unwrap(), "a-c");
        assert_eq!(expand_repl("<${3}>", &spans, "abc").unwrap(), "<>");
        assert_eq!(expand_repl("${0}!", &spans, "abc").unwrap(), "abc!");
    }

    #[test]
    fn test_expand_repl_out_of_range() {
        // `${99}` 越界 → ValueError，消息附 repl 原文片段（验证标准 5）。
        let spans = vec![Some((0, 2)), Some((0, 1))];
        let err = expand_repl("${99}", &spans, "ab").unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        assert!(err.contains("'${99}'"), "消息须附原文片段: {}", err);
    }

    #[test]
    fn test_expand_repl_malformed() {
        let spans = vec![Some((0, 2)), Some((0, 1))];
        // `${1x` 畸形（} 前非纯数字）
        let err = expand_repl("${1x}", &spans, "ab").unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("'${1x}'"),
            "got: {}",
            err
        );
        // `${` 未闭合
        let err = expand_repl("a${", &spans, "ab").unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("${"),
            "got: {}",
            err
        );
        // `${}` 空索引
        let err = expand_repl("${}", &spans, "ab").unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
    }

    // ---- 错误路径（原生 Err 不经 try/except，task 80 惯例由本单测覆盖）----

    fn run_err(src: &str) -> String {
        run_source(src).unwrap_err()
    }

    #[test]
    fn test_invalid_pattern_value_error() {
        // 验证标准 8：非法 pattern → ValueError 含 regex::Error 信息。
        let err = run_err("import regex\nregex.match(\"(\", \"abc\")");
        assert!(err.contains("ValueError"), "got: {}", err);
        assert!(
            err.contains("unclosed group"),
            "附 regex::Error 详情: {}",
            err
        );
    }

    #[test]
    fn test_sub_count_negative_and_non_int() {
        // 验证标准 10：count=-1 → ValueError。
        let err = run_err("import regex\nregex.sub(\"a\", \"b\", \"aaa\", -1)");
        assert!(
            err.contains("ValueError") && err.contains("count must be non-negative"),
            "got: {}",
            err
        );
        // count 非 Int → TypeError。
        let err = run_err("import regex\nregex.sub(\"a\", \"b\", \"aaa\", \"x\")");
        assert!(
            err.contains("TypeError") && err.contains("count"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_sub_repl_fn_non_string_return() {
        // 验证标准 5：repl 函数返回非 string → TypeError。
        let err = run_err("import regex\nregex.sub(\"a\", fn(m) { return 1 }, \"aaa\")");
        assert!(
            err.contains("TypeError") && err.contains("must return string"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_sub_out_of_range_group_reference() {
        let err = run_err("import regex\nregex.sub(\"(a)\", \"${99}\", \"aaa\")");
        assert!(
            err.contains("ValueError") && err.contains("'${99}'"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_group_negative_and_out_of_range() {
        // 验证标准 10：group(-1) → IndexError（负索引按越界，v1 不回绕）。
        let err = run_err("import regex\nregex.search(\"(l)\", \"hello\").group(-1)");
        assert!(err.contains("IndexError"), "got: {}", err);
        let err = run_err("import regex\nregex.search(\"(l)\", \"hello\").group(2)");
        assert!(err.contains("IndexError"), "got: {}", err);
    }

    #[test]
    fn test_arity_self_validation() {
        // split MAX：恰 2 参自校验（§2.2）。
        let err = run_err("import regex\nregex.split(\",\")");
        assert!(
            err.contains("TypeError") && err.contains("exactly 2"),
            "got: {}",
            err
        );
        let err = run_err("import regex\nregex.split(\",\", \"a\", 1)");
        assert!(err.contains("TypeError"), "got: {}", err);
        // sub MAX：3-4 参自校验。
        let err = run_err("import regex\nregex.sub(\"a\", \"b\")");
        assert!(
            err.contains("TypeError") && err.contains("3-4"),
            "got: {}",
            err
        );
    }

    // ---- 模块注册与集成 ----

    #[test]
    fn test_module_registration() {
        let ptr = register_regex_module();
        // SAFETY: ptr 由 register_regex_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "regex");
            for name in ["match", "search", "findall", "sub", "split", "compile"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_lookup_method_tables() {
        for name in ["match", "search", "findall", "sub", "split", "pattern"] {
            assert!(lookup_regex_method(name).is_some(), "regex.{}", name);
        }
        assert!(lookup_regex_method("nosuch").is_none());
        for name in ["group", "groups", "start", "end", "span"] {
            assert!(lookup_match_method(name).is_some(), "match.{}", name);
        }
        assert!(lookup_match_method("nosuch").is_none());
    }

    /// 验证标准 1-7 集成（脚本级，与 test_regex.ms 同源）。
    #[test]
    fn test_integration_functional_and_object_forms() {
        let src = r#"
import regex
m = regex.search("l+", "hello")
assert(m.group(0) == "ll")
assert(m.start() == 2 and m.end() == 4)
assert(m.span() == (2, 4))
assert(regex.match("h", "hello") != nil)
assert(regex.match("e", "hello") == nil)
assert(regex.findall("[lo]", "hello") == ["l", "l", "o"])
assert(regex.findall("(\\d)-(\\w)", "1-a 2-b") == [("1", "a"), ("2", "b")])
assert(regex.sub("(\\w+)-(\\w+)", "${2}-${1}", "ab-cd") == "cd-ab")
assert(regex.sub("l", "L", "hello", 1) == "heLlo")
assert(regex.sub("l", "L", "hello") == "heLLo")
assert(regex.sub("\\d", fn(m) { return "<" + m.group(0) + ">" }, "a1b2") == "a<1>b<2>")
assert(regex.split(",", "a,b,c") == ["a", "b", "c"])
assert(regex.split("x", "abc") == ["abc"])
re = regex.compile("(\\d+)")
assert(re.pattern() == "(\\d+)")
assert(re.findall("a12b34") == ["12", "34"])
assert(re.match("12").group(1) == "12")
assert(re.search("b34").group(1) == "34")
rec = regex.compile(",")
assert(rec.split("1,2") == ["1", "2"])
assert(re.sub("N", "a1b") == "aNb")
mm = re.search("x7y")
assert(mm.group(1) == "7")
g = regex.search("(a)(b)?", "a")
assert(g.groups() == ("a", nil))
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "integration failed: {:?}", r.err());
    }

    /// 验证标准 6：split 同名交叉回归——`s.split()`（空白）、`s.split(",")`、
    /// `regex.split(",", s)` 三者并存（§2.2 split=MAX 治理）。
    #[test]
    fn test_split_name_collision_regression() {
        let src = r#"
import regex
s = "a,b c"
assert(s.split() == ["a,b", "c"])
assert(s.split(",") == ["a", "b c"])
assert(regex.split(",", s) == ["a", "b c"])
assert(regex.split("\\s", s) == ["a,b", "c"])
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "split collision failed: {:?}", r.err());
    }

    /// sub 回调重入（GC 根化纪律冒烟）：回调内大量分配字符串，repl 根槽重取。
    #[test]
    fn test_sub_callback_with_allocations() {
        let src = r#"
import regex
out = regex.sub("\\w+", fn(m) {
    junk = ""
    j = 0
    while j < 20 {
        junk = junk + str(m.group(0)) + "-"
        j = j + 1
    }
    return "[" + m.group(0) + "]"
}, "aa bb cc dd ee ff gg hh")
assert(out == "[aa] [bb] [cc] [dd] [ee] [ff] [gg] [hh]", out)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "sub callback failed: {:?}", r.err());
    }

    /// 保留字属性访问（验证标准 11 正侧）：`regex.match(...)` 可解析执行。
    #[test]
    fn test_reserved_word_attribute_access() {
        let src = r#"
import regex
assert(regex.match("h", "hello") != nil)
re = regex.compile("h")
assert(re.match("hello") != nil)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "reserved attr failed: {:?}", r.err());
    }

    /// object_to_string / type() 渲染（Display 接入点）。
    #[test]
    fn test_display_and_type_name() {
        let s = super::super::test_util::s;
        let mut v = vm();
        let re = native_regex_compile(&mut v, &[s(r"(\d+)")]).unwrap();
        assert_eq!(re.type_name(), "regex");
        assert_eq!(format!("{}", re), r"/(\d+)/");
        // search（match 锚定开头，"x12" 不以 "12" 开头，须用 search 构造）。
        let m = native_regex_search(&mut v, &[s("12"), s("x12")]).unwrap();
        assert_eq!(m.type_name(), "match");
        assert_eq!(format!("{}", m), "<match span=(1, 3), match='12'>");
    }
}
