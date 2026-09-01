//! `json` 原生模块：解析器与序列化器。
//!
//! 参照 [49-stdlib-json](../../../docs/mslang/tasks/49-stdlib-json.md)。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_dict, alloc_list, alloc_module, alloc_string, read_dict, read_list, read_module_mut,
    read_str, DictMap, MsObjHeader, Object, TypeTag,
};
use crate::vm::VM;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// json 模块（task 49）
// ---------------------------------------------------------------------------

/// JSON 解析/序列化的最大嵌套深度（task 49 §验证标准 #10）。
/// MAX_NESTING=1000 兼顾常规用例与栈安全：默认线程栈（8 MiB）下 ~1000 层递归
/// 不会溢出，同时拒绝恶意深嵌套输入。
const MAX_NESTING: u32 = 1000;

/// 构造 `json` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 parse/stringify 两个原生函数。
pub fn register_json_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 2] = [
        ("parse", native_json_parse),
        ("stringify", native_json_stringify),
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
    let m = alloc_module("json");
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

/// json.parse(string) → 解析 JSON 文本为 mslang 值（task 49 §方案 B，手动解析，零依赖）。
/// 类型映射：null→nil、bool→bool、整数→int、浮点→float、字符串→string、
/// 数组→list、对象→dict。超出 i64 的整数退化为 float（与 Python JSON 一致）。
/// task 83（§2.2 同名冲突）：time.parse 加入后 native_arities["parse"] 升级
/// MAX，此处自校验恰 1 参（time.parse 自校验恰 2 参）。
fn native_json_parse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 1 {
        return Err(format!(
            "TypeError: parse(string) takes exactly 1 argument, got {}",
            args.len()
        ));
    }
    let json_str = expect_string(args.get(0), "parse(string)")?;
    let bytes = json_str.as_bytes();
    let mut p = JsonParser { src: bytes, pos: 0 };
    p.skip_ws();
    let v = p.parse_value(0)?;
    p.skip_ws();
    if p.pos != bytes.len() {
        return Err(format!(
            "ValueError: json trailing characters at byte {}",
            p.pos
        ));
    }
    Ok(v)
}

/// json.stringify(value) → 将 mslang 值序列化为 JSON 文本。
/// nil→null、bool→true/false、int→整数、float→数字（NaN/Infinity 报错）、
/// string→字符串字面量、list→数组、dict→对象（键必须为 string）。
/// tuple/set/function/... 不支持，返回 TypeError（Phase 6.2d 无 __to_json__）。
fn native_json_stringify(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let obj = args
        .get(0)
        .ok_or_else(|| "ValueError: stringify expects 1 argument".to_string())?;
    let mut out = String::new();
    let mut seen: HashSet<usize> = HashSet::new();
    stringify_into(obj, &mut out, 0, &mut seen)?;
    Ok(alloc_string(&out))
}

/// 递归将 obj 序列化进 out。`seen` 记录当前递归路径上的 list/dict 指针地址，
/// 用于检测循环引用（同对象出现在兄弟位置不视为循环，递归返回后移除）。
fn stringify_into(
    obj: &Object,
    out: &mut String,
    depth: u32,
    seen: &mut HashSet<usize>,
) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("ValueError: nesting exceeds {} levels", MAX_NESTING));
    }
    match obj {
        Object::Nil => out.push_str("null"),
        Object::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Object::Int(i) => {
            use std::fmt::Write;
            let _ = write!(out, "{}", i);
        }
        Object::Float(f) => {
            // NaN/Infinity 非合法 JSON 数字（RFC 8259），显式报错
            //（与 02-types.md § 特殊浮点值语义一致）。
            if !f.is_finite() {
                return Err(format!("ValueError: cannot serialize non-finite float: {}", f));
            }
            push_json_float(*f, out);
        }
        Object::Ref(ptr) => {
            // SAFETY: Ref 来自 alloc_* 系列，type_tag 可读。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING，ptr 由 alloc_string 分配。
                let s = unsafe { read_str(*ptr) };
                push_json_string(s, out);
            } else if tag == TypeTag::LIST as u8 {
                // 循环引用检测：用指针地址判重，避免 list 自引用导致无限递归。
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // 借用约束：递归前 clone 出元素，释放 read_list 返回的 &mut Vec。
                let items: Vec<Object> = { unsafe { read_list(*ptr) }.clone() };
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    stringify_into(item, out, depth + 1, seen)?;
                }
                out.push(']');
                seen.remove(&(*ptr as usize));
            } else if tag == TypeTag::DICT as u8 {
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // 借用约束：递归前 clone 出 (key, value)，释放 read_dict 返回的 &mut。
                let items: Vec<(Object, Object)> = {
                    let d = unsafe { read_dict(*ptr) };
                    d.items()
                        .into_iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                };
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let key_str = match k {
                        Object::Ref(kptr)
                            if unsafe { (**kptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            // SAFETY: type_tag 为 STRING。
                            unsafe { read_str(*kptr) }.to_owned()
                        }
                        _ => {
                            return Err(format!(
                                "TypeError: JSON dict key must be string, got {}",
                                k.type_name()
                            ))
                        }
                    };
                    push_json_string(&key_str, out);
                    out.push(':');
                    stringify_into(v, out, depth + 1, seen)?;
                }
                out.push('}');
                seen.remove(&(*ptr as usize));
            } else {
                // tuple/set/function/class/instance/file_handle/...：Phase 6.2d 不支持
                // __to_json__ 魔术方法，统一拒绝（TypeError）。
                return Err(format!(
                    "TypeError: cannot serialize {} to JSON",
                    obj.type_name()
                ));
            }
        }
    }
    Ok(())
}

/// 转义 JSON 字符串字面量：`"`、`\`、控制字符（< 0x20）。
/// 非 ASCII 字符直接以 UTF-8 输出（RFC 8259 允许未转义的 Unicode）。
fn push_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// 格式化 JSON 数字（float）。与 Object::Display 一致：整数值浮点用 "{:.1}"
/// （3.0→"3.0"、-0.0→"-0.0"），非整数用 "{}"（3.14→"3.14"），保证 round-trip
/// 与 print 输出一致（task 49 §验证标准 #6/#8）。
fn push_json_float(f: f64, out: &mut String) {
    use std::fmt::Write;
    if f == (f as i64) as f64 {
        let _ = write!(out, "{:.1}", f);
    } else {
        let _ = write!(out, "{}", f);
    }
}

/// 简易递归下降 JSON 解析器（task 49 §方案 B，零外部依赖）。
/// 覆盖 RFC 8259 子集：null/true/false/number/string/array/object。
struct JsonParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while let Some(&c) = self.src.get(self.pos) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    /// 计算 pos 对应的 1-based 行列号（错误消息定位）。
    fn line_col(&self, pos: usize) -> (usize, usize) {
        let mut line = 1usize;
        let mut col = 1usize;
        for &b in &self.src[..pos.min(self.src.len())] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// 生成 parse 语法错误消息（行列号定位，仅含位置不含原文片段以防敏感数据泄露）。
    fn fail(&self, pos: usize, reason: &str) -> String {
        let (line, col) = self.line_col(pos);
        format!("ValueError: json {} at line {} column {}", reason, line, col)
    }

    /// 解析一个值。`depth` 为当前嵌套层级（顶层=0）。
    /// 容器（array/object）在进入时以 `depth+1` 校验 MAX_NESTING。
    fn parse_value(&mut self, depth: u32) -> Result<Object, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(alloc_string(&self.parse_string()?)),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c.is_ascii_digit() || c == b'-' => self.parse_number(),
            _ => Err(self.fail(self.pos, "parse error")),
        }
    }

    fn parse_array(&mut self, depth: u32) -> Result<Object, String> {
        // 进入容器：嵌套层级 +1。超 MAX_NESTING 拒绝（覆盖空数组深嵌套：空数组
        // 不递归到元素，故必须在进入容器时校验，而非 parse_value 顶部）。
        let level = depth + 1;
        if level > MAX_NESTING {
            return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
        }
        debug_assert!(self.peek() == Some(b'['));
        self.pos += 1; // consume '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(alloc_list(items));
        }
        loop {
            let v = self.parse_value(level)?;
            items.push(v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.fail(self.pos, "parse error")),
            }
        }
        Ok(alloc_list(items))
    }

    fn parse_object(&mut self, depth: u32) -> Result<Object, String> {
        let level = depth + 1;
        if level > MAX_NESTING {
            return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
        }
        debug_assert!(self.peek() == Some(b'{'));
        self.pos += 1; // consume '{'
        let mut dict = DictMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(alloc_dict(dict));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.fail(self.pos, "parse error"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.fail(self.pos, "parse error"));
            }
            self.pos += 1; // consume ':'
            let val = self.parse_value(level)?;
            dict.insert(alloc_string(&key), val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.fail(self.pos, "parse error")),
            }
        }
        Ok(alloc_dict(dict))
    }

    fn parse_null(&mut self) -> Result<Object, String> {
        if self.src.get(self.pos..self.pos + 4) == Some(b"null") {
            self.pos += 4;
            Ok(Object::Nil)
        } else {
            Err(self.fail(self.pos, "parse error"))
        }
    }

    fn parse_bool(&mut self) -> Result<Object, String> {
        if self.src.get(self.pos..self.pos + 4) == Some(b"true") {
            self.pos += 4;
            Ok(Object::Bool(true))
        } else if self.src.get(self.pos..self.pos + 5) == Some(b"false") {
            self.pos += 5;
            Ok(Object::Bool(false))
        } else {
            Err(self.fail(self.pos, "parse error"))
        }
    }

    /// 解析字符串字面量（已消费开引号前的判定）。处理转义 \" \\ \/ \b \f \n \r \t
    /// 与 \uXXXX（含 UTF-16 代理对重建）。
    fn parse_string(&mut self) -> Result<String, String> {
        debug_assert!(self.peek() == Some(b'"'));
        self.pos += 1; // consume opening '"'
        let mut out = String::new();
        loop {
            // 收集直至下一个特殊字节（'"'、'\'、控制字符 < 0x20）。
            let start = self.pos;
            while let Some(&c) = self.src.get(self.pos) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            if self.pos > start {
                let span = &self.src[start..self.pos];
                // span 必为合法 UTF-8：src 源自 &str，且循环仅在 ASCII（'"'/'\'
                // 或 < 0x20）处截断，不会切断多字节序列。用 from_utf8 防御性校验。
                match std::str::from_utf8(span) {
                    Ok(s) => out.push_str(s),
                    Err(_) => return Err(self.fail(start, "parse error")),
                }
            }
            match self.src.get(self.pos).copied() {
                None => return Err(self.fail(self.pos, "parse error")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.src.get(self.pos).copied();
                    self.pos += 1;
                    match esc {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\x08'),
                        Some(b'f') => out.push('\x0c'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let cp = self.parse_hex4()?;
                            out.push(self.decode_unicode(cp)?);
                        }
                        _ => return Err(self.fail(self.pos, "parse error")),
                    }
                }
                Some(c) if c < 0x20 => return Err(self.fail(self.pos, "parse error")),
                // 不应到达：上述循环已覆盖所有 < 0x20 / '"' / '\\'。
                Some(_) => unreachable!("parse_string scan loop invariant"),
            }
        }
    }

    /// 解析 \u 后的 4 位十六进制。返回码点或（高代理时）已合并的完整码点。
    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut val = 0u32;
        for _ in 0..4 {
            let c = self.src.get(self.pos).copied();
            let d = match c {
                Some(b'0'..=b'9') => (c.unwrap() - b'0') as u32,
                Some(b'a'..=b'f') => (c.unwrap() - b'a' + 10) as u32,
                Some(b'A'..=b'F') => (c.unwrap() - b'A' + 10) as u32,
                _ => return Err(self.fail(self.pos, "parse error")),
            };
            val = val * 16 + d;
            self.pos += 1;
        }
        Ok(val)
    }

    /// 将 \uXXXX 码点（可能为高代理）解码为 char，处理紧随的低代理对。
    fn decode_unicode(&mut self, cp: u32) -> Result<char, String> {
        if (0xD800..=0xDBFF).contains(&cp) {
            // 高代理：期望紧跟 \uXXXX 低代理（RFC 8259 §7）。
            if self.src.get(self.pos..self.pos + 2) == Some(b"\\u") {
                self.pos += 2;
                let lo = self.parse_hex4()?;
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    return char::from_u32(combined)
                        .ok_or_else(|| self.fail(self.pos, "parse error"));
                }
                return Err(self.fail(self.pos, "parse error"));
            }
            return Err(self.fail(self.pos, "parse error"));
        }
        if (0xDC00..=0xDFFF).contains(&cp) {
            // 裸低代理非法。
            return Err(self.fail(self.pos, "parse error"));
        }
        char::from_u32(cp).ok_or_else(|| self.fail(self.pos, "parse error"))
    }

    /// 解析数字：可选负号、整数部分（'0' 或 [1-9]+）、可选小数、可选指数。
    /// 无小数/指数时优先 i64；超出 i64 范围退化为 f64。
    fn parse_number(&mut self) -> Result<Object, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(c) if c.is_ascii_digit() => {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.fail(self.pos, "parse error")),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.fail(self.pos, "parse error"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.fail(self.pos, "parse error"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.fail(start, "parse error"))?;
        if is_float {
            let f = text
                .parse::<f64>()
                .map_err(|_| self.fail(start, "parse error"))?;
            Ok(Object::Float(f))
        } else {
            match text.parse::<i64>() {
                Ok(i) => Ok(Object::Int(i)),
                // 超出 i64 范围：退化为 f64（与 Python JSON 一致，精度可能损失）。
                Err(_) => {
                    let f = text
                        .parse::<f64>()
                        .map_err(|_| self.fail(start, "parse error"))?;
                    Ok(Object::Float(f))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, vm};

    // ---- json 模块（task 49）-------------------------------------------------

    #[test]
    fn test_json_module_registration() {
        let ptr = register_json_module();
        // SAFETY: ptr 由 register_json_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "json");
            assert!(m.exports.contains_key("parse"));
            assert!(m.exports.contains_key("stringify"));
        }
    }

    #[test]
    #[allow(clippy::approx_constant)] // 测试合法使用 3.14 字面量（clippy 误报为 PI 近似）
    fn test_json_parse_scalars() {
        let mut v = vm();
        assert_eq!(native_json_parse(&mut v, &[s("null")]).unwrap(), Object::Nil);
        assert_eq!(
            native_json_parse(&mut v, &[s("true")]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("false")]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("42")]).unwrap(),
            Object::Int(42)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("-7")]).unwrap(),
            Object::Int(-7)
        );
        assert_eq!(
            native_json_parse(&mut v, &[s("0")]).unwrap(),
            Object::Int(0)
        );
        match native_json_parse(&mut v, &[s("3.14")]).unwrap() {
            Object::Float(f) => assert!((f - 3.14).abs() < 1e-9, "got {}", f),
            other => panic!("expected float, got {:?}", other),
        }
    }

    #[test]
    fn test_json_parse_string_escapes() {
        let mut v = vm();
        assert_eq!(
            native_json_parse(&mut v, &[s("\"hello\"")]).unwrap(),
            s("hello")
        );
        // \" \\ \n \t \/ \uXXXX
        assert_eq!(
            native_json_parse(&mut v, &[s("\"a\\nb\\tc\\\\d\\/e\\u0041\"")]).unwrap(),
            s("a\nb\tc\\d/eA")
        );
        // \b \f \r
        assert_eq!(
            native_json_parse(&mut v, &[s("\"x\\by\\fz\"")]).unwrap(),
            s("x\x08y\x0cz")
        );
        // UTF-16 代理对：U+1F600 (😀)
        assert_eq!(
            native_json_parse(&mut v, &[s("\"\\uD83D\\uDE00\"")]).unwrap(),
            s("\u{1F600}")
        );
        // 非 ASCII 原文直接传递
        assert_eq!(
            native_json_parse(&mut v, &[s("\"日本語\"")]).unwrap(),
            s("日本語")
        );
    }

    #[test]
    fn test_json_parse_collections() {
        let mut v = vm();
        // array → list
        let arr = native_json_parse(&mut v, &[s("[1, null, \"hi\", [4, 5]]")]).unwrap();
        let Object::Ref(p) = &arr else {
            panic!("expected list");
        };
        unsafe {
            assert_eq!((*(*p)).type_tag, TypeTag::LIST as u8);
            let items = read_list(*p);
            assert_eq!(items.len(), 4);
            assert_eq!(items[0], Object::Int(1));
            assert_eq!(items[1], Object::Nil);
            assert_eq!(items[2], s("hi"));
        }
        // object → dict
        let d = native_json_parse(&mut v, &[s("{\"name\": \"Alice\", \"age\": 30}")]).unwrap();
        let Object::Ref(p) = &d else {
            panic!("expected dict");
        };
        unsafe {
            assert_eq!((*(*p)).type_tag, TypeTag::DICT as u8);
            let map = read_dict(*p);
            assert_eq!(map.len(), 2);
            assert_eq!(map.get(&s("name")), Some(&s("Alice")));
            assert_eq!(map.get(&s("age")), Some(&Object::Int(30)));
        }
        // 空容器
        let empty_arr = native_json_parse(&mut v, &[s("[]")]).unwrap();
        let Object::Ref(ep) = &empty_arr else {
            panic!("expected list");
        };
        unsafe {
            assert!(read_list(*ep).is_empty());
        }
        let empty_obj = native_json_parse(&mut v, &[s("{}")]).unwrap();
        let Object::Ref(ep) = &empty_obj else {
            panic!("expected dict");
        };
        unsafe {
            assert!(read_dict(*ep).is_empty());
        }
    }

    #[test]
    fn test_json_parse_nested() {
        let mut v = vm();
        let d = native_json_parse(&mut v, &[s("{\"a\": {\"b\": [1, 2, {\"c\": true}]}}")]).unwrap();
        let Object::Ref(p) = &d else {
            panic!("expected dict");
        };
        // 沿 nested["a"]["b"][2]["c"] 取 true
        unsafe {
            let a = read_dict(*p).get(&s("a")).cloned().unwrap();
            let Object::Ref(ap) = &a else {
                panic!("expected dict for a");
            };
            let b = read_dict(*ap).get(&s("b")).cloned().unwrap();
            let Object::Ref(bp) = &b else {
                panic!("expected list for b");
            };
            let third = read_list(*bp)[2].clone();
            let Object::Ref(tp) = &third else {
                panic!("expected dict for third");
            };
            let c = read_dict(*tp).get(&s("c")).cloned().unwrap();
            assert_eq!(c, Object::Bool(true));
        }
    }

    #[test]
    fn test_json_parse_bigint_to_float() {
        let mut v = vm();
        // 超出 i64 范围（> 9223372036854775807）→ Float
        match native_json_parse(&mut v, &[s("99999999999999999999")]).unwrap() {
            Object::Float(_) => {}
            other => panic!("expected float for big int, got {:?}", other),
        }
        // i64 边界内 → Int
        assert_eq!(
            native_json_parse(&mut v, &[s("9223372036854775807")]).unwrap(),
            Object::Int(i64::MAX)
        );
    }

    #[test]
    fn test_json_parse_errors() {
        let mut v = vm();
        // 非法 JSON（首字节 'i' 非法）→ 行列号定位
        let e = native_json_parse(&mut v, &[s("invalid json")]).unwrap_err();
        assert!(e.contains("ValueError"), "got: {}", e);
        assert!(e.contains("line 1 column 1"), "got: {}", e);
        // 尾随字符 → 字节偏移
        let e = native_json_parse(&mut v, &[s("1 2")]).unwrap_err();
        assert!(e.contains("trailing characters"), "got: {}", e);
        // 非闭合
        assert!(native_json_parse(&mut v, &[s("[1, 2")]).is_err());
        // 入参非 string → TypeError
        let e = native_json_parse(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
    }

    #[test]
    fn test_json_parse_arity_self_check() {
        let mut v = vm();
        // task 83：parse 升级 MAX（time.parse 同名）后 json.parse 自校验恰 1 参。
        let e = native_json_parse(&mut v, &[]).unwrap_err();
        assert!(e.contains("TypeError") && e.contains("exactly 1"), "got: {}", e);
        let e = native_json_parse(&mut v, &[s("1"), s("2")]).unwrap_err();
        assert!(e.contains("TypeError") && e.contains("exactly 1"), "got: {}", e);
        // 恰 1 参不受影响
        assert_eq!(
            native_json_parse(&mut v, &[s("42")]).unwrap(),
            Object::Int(42)
        );
    }

    #[test]
    fn test_json_stringify_basic() {
        let mut v = vm();
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Nil]).unwrap(),
            s("null")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Bool(true)]).unwrap(),
            s("true")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Bool(false)]).unwrap(),
            s("false")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Int(42)]).unwrap(),
            s("42")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[s("hi")]).unwrap(),
            s("\"hi\"")
        );
        assert_eq!(
            native_json_stringify(&mut v, &[alloc_list(vec![Object::Int(1), Object::Int(2)])])
                .unwrap(),
            s("[1,2]")
        );
        // dict {"x":1,"y":[2,3]}
        let mut m = DictMap::new();
        m.insert(s("x"), Object::Int(1));
        m.insert(s("y"), alloc_list(vec![Object::Int(2), Object::Int(3)]));
        assert_eq!(
            native_json_stringify(&mut v, &[alloc_dict(m)]).unwrap(),
            s("{\"x\":1,\"y\":[2,3]}")
        );
    }

    #[test]
    #[allow(clippy::approx_constant)] // 测试合法使用 3.14 字面量
    fn test_json_stringify_floats() {
        let mut v = vm();
        // 3.14 → "3.14"
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(3.14)]).unwrap(),
            s("3.14")
        );
        // 整数值浮点 → "3.0"
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(3.0)]).unwrap(),
            s("3.0")
        );
        // -0.0 字面量保留
        assert_eq!(
            native_json_stringify(&mut v, &[Object::Float(-0.0)]).unwrap(),
            s("-0.0")
        );
        // NaN → ValueError
        let e = native_json_stringify(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(e.contains("non-finite"), "got: {}", e);
        assert!(e.contains("NaN"), "got: {}", e);
        // Infinity → ValueError
        let e = native_json_stringify(&mut v, &[Object::Float(f64::INFINITY)]).unwrap_err();
        assert!(e.contains("non-finite"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_escapes() {
        let mut v = vm();
        assert_eq!(
            native_json_stringify(&mut v, &[s("a\"b\\c\nd\te")]).unwrap(),
            s("\"a\\\"b\\\\c\\nd\\te\"")
        );
        // 控制字符 < 0x20 → \u00XX
        assert_eq!(
            native_json_stringify(&mut v, &[s("\x01")]).unwrap(),
            s("\"\\u0001\"")
        );
        // 非 ASCII 原样输出
        assert_eq!(
            native_json_stringify(&mut v, &[s("日本語")]).unwrap(),
            s("\"日本語\"")
        );
    }

    #[test]
    fn test_json_stringify_non_serializable() {
        let mut v = vm();
        // function → TypeError
        let f = alloc_native_function(NativeFunction {
            name: "parse".to_string(),
            func: native_json_parse,
        });
        let e = native_json_stringify(&mut v, &[f]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        assert!(e.contains("function"), "got: {}", e);
        // tuple → TypeError
        let t = crate::vm::object::alloc_tuple(vec![Object::Int(1)]);
        let e = native_json_stringify(&mut v, &[t]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        // 缺参（直接调用 native）→ ValueError
        let e = native_json_stringify(&mut v, &[]).unwrap_err();
        assert!(e.contains("ValueError"), "got: {}", e);
        assert!(e.contains("expects 1 argument"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_dict_key_non_string() {
        let mut v = vm();
        let mut m = DictMap::new();
        m.insert(Object::Int(1), Object::Int(2)); // 非字符串键
        let e = native_json_stringify(&mut v, &[alloc_dict(m)]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        assert!(e.contains("dict key must be string"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_circular_list() {
        let mut v = vm();
        let lst = alloc_list(vec![]);
        // list 自引用：a = []; a.push(a)
        if let Object::Ref(p) = &lst {
            unsafe {
                read_list(*p).push(lst.clone());
            }
        }
        let e = native_json_stringify(&mut v, &[lst]).unwrap_err();
        assert!(e.contains("circular reference"), "got: {}", e);
    }

    #[test]
    fn test_json_stringify_circular_dict() {
        let mut v = vm();
        // d1 = {"link": d2}; d2 = {"back": d1} → 互引用
        let d2 = alloc_dict(DictMap::new());
        let mut m1 = DictMap::new();
        m1.insert(s("link"), d2.clone());
        let d1 = alloc_dict(m1);
        if let Object::Ref(p2) = &d2 {
            unsafe {
                read_dict(*p2).insert(s("back"), d1.clone());
            }
        }
        let e = native_json_stringify(&mut v, &[d1]).unwrap_err();
        assert!(e.contains("circular reference"), "got: {}", e);
    }

    #[test]
    fn test_json_round_trip() {
        let mut v = vm();
        let original = s("{\"name\":\"Alice\",\"age\":30,\"scores\":[10,20,30]}");
        let parsed = native_json_parse(&mut v, std::slice::from_ref(&original)).unwrap();
        let back = native_json_stringify(&mut v, &[parsed]).unwrap();
        assert_eq!(back, original);
        // 浮点 round-trip
        let f = native_json_parse(&mut v, &[s("3.14")]).unwrap();
        assert_eq!(
            native_json_stringify(&mut v, &[f]).unwrap(),
            s("3.14")
        );
    }

    #[test]
    fn test_json_depth_limit_parse() {
        let mut v = vm();
        // 1000 层可解析，1001 层超限
        let ok = "[".repeat(1000) + &"]".repeat(1000);
        assert!(native_json_parse(&mut v, &[s(&ok)]).is_ok(), "1000 levels should parse");
        let too_deep = "[".repeat(1001) + &"]".repeat(1001);
        let e = native_json_parse(&mut v, &[s(&too_deep)]).unwrap_err();
        assert!(e.contains("nesting exceeds 1000 levels"), "got: {}", e);
    }

    #[test]
    fn test_json_depth_limit_stringify() {
        let mut v = vm();
        // 递归构造 1001 层 list 嵌套
        let mut nested = Object::Int(1);
        for _ in 0..1001 {
            nested = alloc_list(vec![nested]);
        }
        let e = native_json_stringify(&mut v, &[nested]).unwrap_err();
        assert!(e.contains("nesting exceeds 1000 levels"), "got: {}", e);
    }

    #[test]
    fn test_integration_json_module() {
        // 等价 test_json.ms：import → parse → dict 索引 → stringify。
        // 注：mslang 仅支持双引号字符串（spec 示例的单引号为 Python 习惯，需转义）。
        let src = r#"
import json
data = json.parse("{\"name\": \"Alice\", \"age\": 30}")
assert(data["name"] == "Alice")
assert(data["age"] == 30)
text = json.stringify({"x": 1, "y": [2, 3]})
assert(text == "{\"x\":1,\"y\":[2,3]}")
nested = json.parse("{\"a\": {\"b\": [1, 2, {\"c\": true}]}}")
assert(nested["a"]["b"][2]["c"] == true)
f = json.parse("3.14")
assert(type(f) == "float")
assert(json.stringify(f) == "3.14")
assert(json.stringify(json.parse("-0.0")) == "-0.0")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "json integration failed: {:?}", r.err());
    }
}
