//! `encoding` 原生模块（task 81）。
//!
//! 参照 [81-stdlib-random-encoding-uuid](../../../docs/mslang/tasks/81-stdlib-random-encoding-uuid.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.5。
//!
//! base64/hex/url 编解码全部手写（零新增依赖）。语言无 bytes 类型，
//! 解码结果须为合法 UTF-8，否则 ValueError（与 url_decode 非法 UTF-8 规则一致）。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

/// RFC 4648 标准字母表（含 `+/`；URL-safe 变体不在本模块范围）。
const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 小写十六进制表（hex_encode 输出与 uuid 格式化共用风格）。
const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

/// 大写十六进制表（url_encode %HH 转义）。
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// 构造 `encoding` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_encoding_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 6] = [
        ("base64_encode", native_encoding_base64_encode),
        ("base64_decode", native_encoding_base64_decode),
        ("hex_encode", native_encoding_hex_encode),
        ("hex_decode", native_encoding_hex_decode),
        ("url_encode", native_encoding_url_encode),
        ("url_decode", native_encoding_url_decode),
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
    let m = alloc_module("encoding");
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

/// base64 字节 → sextet 值（RFC 4648 标准字母表）。
fn b64_value(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(26 + (b - b'a')),
        b'0'..=b'9' => Some(52 + (b - b'0')),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// 单个 hex 字符 → 值（大小写均接受；仅输出侧固定小写/大写）。
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

/// 解码字节流 → mslang string（非法 UTF-8 → ValueError；无 bytes 类型的必然约束）。
fn bytes_to_object(bytes: Vec<u8>, who: &str) -> Result<Object, String> {
    match String::from_utf8(bytes) {
        Ok(s) => Ok(alloc_string(&s)),
        Err(_) => Err(format!("ValueError: {}() result is not valid UTF-8", who)),
    }
}

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

fn native_encoding_base64_encode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "base64_encode(s)")?;
    let bytes = s.as_bytes();
    // 3 字节 → 4 字符，尾部 `=` padding（1 或 2 个）。
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64_ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64_ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    Ok(alloc_string(&out))
}

fn native_encoding_base64_decode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "base64_decode(s)")?;
    // 剔除 ASCII 空白（保留原始字节位置供错误消息）。
    let data: Vec<(u8, usize)> = s
        .bytes()
        .enumerate()
        .filter(|(_, b)| !b.is_ascii_whitespace())
        .map(|(i, b)| (b, i))
        .collect();
    if !data.len().is_multiple_of(4) {
        return Err(format!(
            "ValueError: base64_decode(): invalid length {} (not a multiple of 4)",
            data.len()
        ));
    }
    // `=` 仅允许末组尾部 0/1/2 个（"AB=="/"ABC="/"ABCD"）。
    let n_pad = data.iter().rev().take_while(|(b, _)| *b == b'=').count();
    if n_pad > 2 {
        return Err("ValueError: base64_decode(): invalid padding (more than 2 '=')".to_string());
    }
    if let Some((_, i)) = data[..data.len() - n_pad].iter().find(|(b, _)| *b == b'=') {
        return Err(format!(
            "ValueError: base64_decode(): invalid character '=' at position {}",
            i
        ));
    }
    let body: Vec<u8> = data[..data.len() - n_pad]
        .iter()
        .map(|&(b, i)| match b64_value(b) {
            Some(v) => Ok(v),
            None => Err(format!(
                "ValueError: base64_decode(): invalid character 0x{:02x} at position {}",
                b, i
            )),
        })
        .collect::<Result<_, _>>()?;
    // 4 sextet → 3 字节；末组 2/3 个数据位（对应 2/1 padding）。
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    for group in body.chunks(4) {
        let n = ((group[0] as u32) << 18)
            | ((group[1] as u32) << 12)
            | ((*group.get(2).unwrap_or(&0) as u32) << 6)
            | *group.get(3).unwrap_or(&0) as u32;
        out.push((n >> 16) as u8);
        if group.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if group.len() > 3 {
            out.push(n as u8);
        }
    }
    bytes_to_object(out, "base64_decode")
}

// ---------------------------------------------------------------------------
// hex
// ---------------------------------------------------------------------------

fn native_encoding_hex_encode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "hex_encode(s)")?;
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        out.push(HEX_LOWER[(b >> 4) as usize] as char);
        out.push(HEX_LOWER[(b & 0x0f) as usize] as char);
    }
    Ok(alloc_string(&out))
}

fn native_encoding_hex_decode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "hex_decode(s)")?;
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(format!(
            "ValueError: hex_decode(): odd-length input {}",
            bytes.len()
        ));
    }
    let invalid = |b: u8, i: usize| {
        format!(
            "ValueError: hex_decode(): invalid hex character 0x{:02x} at position {}",
            b, i
        )
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_value(bytes[i]).ok_or_else(|| invalid(bytes[i], i))?;
        let lo = hex_value(bytes[i + 1]).ok_or_else(|| invalid(bytes[i + 1], i + 1))?;
        out.push(hi << 4 | lo);
    }
    bytes_to_object(out, "hex_decode")
}

// ---------------------------------------------------------------------------
// url（百分号编码，非 form 语义：`+` 保持字面）
// ---------------------------------------------------------------------------

fn native_encoding_url_encode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // arity MAX（§2.2）：native 内自校验 1-2 参（safe 缺省 "/"）。
    if args.is_empty() || args.len() > 2 {
        return Err(format!(
            "TypeError: url_encode(s, safe?) takes 1-2 arguments, got {}",
            args.len()
        ));
    }
    let s = expect_string(args.get(0), "url_encode(s, safe?)")?;
    let safe = match args.get(1) {
        None => "/".to_string(),
        Some(arg) => expect_string(Some(arg), "url_encode(s, safe?)")?,
    };
    // 按 char 迭代：保留 A-Za-z0-9-_.~ 与 safe，其余逐 UTF-8 字节 %HH（大写）。
    let mut out = String::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for c in s.chars() {
        let unreserved = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~');
        if unreserved || safe.contains(c) {
            out.push(c);
        } else {
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push('%');
                out.push(HEX_UPPER[(b >> 4) as usize] as char);
                out.push(HEX_UPPER[(b & 0x0f) as usize] as char);
            }
        }
    }
    Ok(alloc_string(&out))
}

fn native_encoding_url_decode(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "url_decode(s)")?;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // %XX：不足 2 位或非 hex → ValueError（附位置）。
            if i + 3 > bytes.len() {
                return Err(format!(
                    "ValueError: url_decode(): incomplete %-escape at position {}",
                    i
                ));
            }
            let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) else {
                return Err(format!(
                    "ValueError: url_decode(): invalid %-escape at position {}",
                    i
                ));
            };
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            // `+` 与其余字节保持字面（非 form 语义）。
            out.push(bytes[i]);
            i += 1;
        }
    }
    bytes_to_object(out, "url_decode")
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{run_source, s, strval, vm};
    use super::*;

    // ---- base64 ----

    #[test]
    fn test_encoding_module_registration() {
        let ptr = register_encoding_module();
        // SAFETY: ptr 由 register_encoding_module 返回的有效 MsModule。
        unsafe {
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "encoding");
            for name in [
                "base64_encode",
                "base64_decode",
                "hex_encode",
                "hex_decode",
                "url_encode",
                "url_decode",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_base64_rfc4648_vectors() {
        let mut v = vm();
        // RFC 4648 §10 测试向量（padding 0/1/2 全覆盖）。
        for (input, expect) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let got = strval(&native_encoding_base64_encode(&mut v, &[s(input)]).unwrap());
            assert_eq!(got, expect, "encode({:?})", input);
            let back = strval(&native_encoding_base64_decode(&mut v, &[s(expect)]).unwrap());
            assert_eq!(back, input, "decode({:?})", expect);
        }
    }

    #[test]
    fn test_base64_roundtrip_multibyte() {
        // 验证标准 5：含中文/多字节 UTF-8 的编解码往返。
        let mut v = vm();
        for input in ["中文编码", "mslang ✓ àé", "a", "ab", "abc", "隐私箇所"] {
            let enc = strval(&native_encoding_base64_encode(&mut v, &[s(input)]).unwrap());
            assert_eq!(enc.len() % 4, 0, "长度 4 倍数：{}", enc);
            let dec = strval(&native_encoding_base64_decode(&mut v, &[s(&enc)]).unwrap());
            assert_eq!(dec, input, "往返：{}", input);
        }
    }

    #[test]
    fn test_base64_decode_whitespace_stripped() {
        // 验证标准 10：含 ASCII 空白输入剔除后成功。
        let mut v = vm();
        let got = strval(
            &native_encoding_base64_decode(&mut v, &[s("Zm9v\nYg==\t ")])
                .expect("空白剔除后可解码"),
        );
        assert_eq!(got, "foob");
        let got = strval(&native_encoding_base64_decode(&mut v, &[s("Z g =\r\n=")]).unwrap());
        assert_eq!(got, "f");
    }

    #[test]
    fn test_base64_decode_invalid_matrix() {
        let mut v = vm();
        for (input, why) in [
            ("A", "长度非 4 倍数（验证标准 6）"),
            ("A===", "padding 3 个"),
            ("====", "纯 padding"),
            ("AB=C", "'=' 出现在中部"),
            ("=ABC", "'=' 出现在首位"),
            ("Zm9v!", "非法字母表字符"),
            ("Zm9v@bcd", "非法字符 @"),
            ("Zg==x", "padding 后仍有数据"),
        ] {
            let err = native_encoding_base64_decode(&mut v, &[s(input)]).unwrap_err();
            assert!(err.contains("ValueError"), "{} ({:?}): {}", why, input, err);
        }
        // 非法字符附位置。
        let err = native_encoding_base64_decode(&mut v, &[s("Zm9v*9vo")]).unwrap_err();
        assert!(err.contains("position 4"), "got: {}", err);
        // 非 string 入参 → TypeError。
        let err = native_encoding_base64_decode(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // 解码结果非合法 UTF-8（0xFF 0xFF = "//8="）→ ValueError。
        let err = native_encoding_base64_decode(&mut v, &[s("//8=")]).unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("UTF-8"),
            "got: {}",
            err
        );
    }

    // ---- hex ----

    #[test]
    fn test_hex_roundtrip_and_case() {
        let mut v = vm();
        assert_eq!(
            strval(&native_encoding_hex_encode(&mut v, &[s("mslang")]).unwrap()),
            "6d736c616e67"
        );
        assert_eq!(
            strval(&native_encoding_hex_encode(&mut v, &[s("")]).unwrap()),
            ""
        );
        for input in ["", "f", "mslang", "中文", "✓"] {
            let enc = strval(&native_encoding_hex_encode(&mut v, &[s(input)]).unwrap());
            let dec = strval(&native_encoding_hex_decode(&mut v, &[s(&enc)]).unwrap());
            assert_eq!(dec, input, "hex 往返：{}", input);
        }
        // 大写 hex 输入与小写输入等价（验证标准 11 矩阵项）。
        assert_eq!(
            strval(&native_encoding_hex_decode(&mut v, &[s("4D53")]).unwrap()),
            "MS"
        );
        assert_eq!(
            strval(&native_encoding_hex_decode(&mut v, &[s("4d53")]).unwrap()),
            "MS"
        );
        // 输出固定小写。
        assert_eq!(
            strval(&native_encoding_hex_encode(&mut v, &[s("MS")]).unwrap()),
            "4d53"
        );
    }

    #[test]
    fn test_hex_decode_invalid_matrix() {
        let mut v = vm();
        // 验证标准 6：hex_decode("abc") → ValueError（奇数长度）。
        let err = native_encoding_hex_decode(&mut v, &[s("abc")]).unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("odd"),
            "got: {}",
            err
        );
        // 非 hex 字符（附位置）。
        let err = native_encoding_hex_decode(&mut v, &[s("gg")]).unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("position 0"),
            "got: {}",
            err
        );
        let err = native_encoding_hex_decode(&mut v, &[s("4g")]).unwrap_err();
        assert!(err.contains("position 1"), "got: {}", err);
        // 解码结果非合法 UTF-8（0xAB）→ ValueError。
        let err = native_encoding_hex_decode(&mut v, &[s("ab")]).unwrap_err();
        assert!(err.contains("UTF-8"), "got: {}", err);
        // 非 string → TypeError。
        let err = native_encoding_hex_decode(&mut v, &[Object::Bool(true)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    // ---- url ----

    #[test]
    fn test_url_encode() {
        let mut v = vm();
        // 验证标准 7：url_encode("a b/c") == "a%20b/c"（safe="/" 缺省）。
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("a b/c")]).unwrap()),
            "a%20b/c"
        );
        // 自定义 safe：空 safe → '/' 也转义。
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("a b/c"), s("")]).unwrap()),
            "a%20b%2Fc"
        );
        // 保留 A-Za-z0-9-_.~ 不转义。
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("AZaz09-_.~")]).unwrap()),
            "AZaz09-_.~"
        );
        // 非 ASCII 逐 UTF-8 字节大写 %HH。
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("中")]).unwrap()),
            "%E4%B8%AD"
        );
        // '%' 与其他保留符号转义。
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("100%")]).unwrap()),
            "100%25"
        );
        assert_eq!(
            strval(&native_encoding_url_encode(&mut v, &[s("a&b=c?d")]).unwrap()),
            "a%26b%3Dc%3Fd"
        );
    }

    #[test]
    fn test_url_decode() {
        let mut v = vm();
        // 验证标准 7：url_decode 往返一致。
        assert_eq!(
            strval(&native_encoding_url_decode(&mut v, &[s("a%20b/c")]).unwrap()),
            "a b/c"
        );
        // `+` 保持字面（非 form 语义）。
        assert_eq!(
            strval(&native_encoding_url_decode(&mut v, &[s("a+b")]).unwrap()),
            "a+b"
        );
        // 小写 %xx 接受（仅编码侧固定大写）。
        assert_eq!(
            strval(&native_encoding_url_decode(&mut v, &[s("%e4%b8%ad")]).unwrap()),
            "中"
        );
        for input in ["", "a b/c", "中?&=100%", "~._-AZ09"] {
            let enc = strval(&native_encoding_url_encode(&mut v, &[s(input)]).unwrap());
            let dec = strval(&native_encoding_url_decode(&mut v, &[s(&enc)]).unwrap());
            assert_eq!(dec, input, "url 往返：{:?}", input);
        }
    }

    #[test]
    fn test_url_invalid_matrix() {
        let mut v = vm();
        // 验证标准 8：url_decode("%ZZ") → ValueError。
        let err = native_encoding_url_decode(&mut v, &[s("%ZZ")]).unwrap_err();
        assert!(
            err.contains("ValueError") && err.contains("position 0"),
            "got: {}",
            err
        );
        // %XX 缺位（验证标准 11）。
        let err = native_encoding_url_decode(&mut v, &[s("%A")]).unwrap_err();
        assert!(err.contains("incomplete"), "got: {}", err);
        let err = native_encoding_url_decode(&mut v, &[s("100%")]).unwrap_err();
        assert!(err.contains("incomplete"), "got: {}", err);
        // 验证标准 10：url_decode("%FF") → ValueError（非法 UTF-8）。
        let err = native_encoding_url_decode(&mut v, &[s("%FF")]).unwrap_err();
        assert!(err.contains("UTF-8"), "got: {}", err);
        // url_decode 非 string → TypeError。
        let err = native_encoding_url_decode(&mut v, &[Object::Nil]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_url_encode_arity_and_safe_type() {
        let mut v = vm();
        // arity MAX 自校验：0 参 / 3 参 → TypeError。
        let err = native_encoding_url_encode(&mut v, &[]).unwrap_err();
        assert!(
            err.contains("TypeError") && err.contains("1-2"),
            "got: {}",
            err
        );
        let err = native_encoding_url_encode(&mut v, &[s("a"), s(""), s("b")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // safe 非 string → TypeError。
        let err = native_encoding_url_encode(&mut v, &[s("a"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    // ---- 端到端集成 ----

    #[test]
    fn test_integration_encoding_module() {
        let src = r#"
import encoding
assert(encoding.base64_encode("foobar") == "Zm9vYmFy", "b64 向量")
assert(encoding.base64_decode("Zm9vYmFy") == "foobar", "b64 解码")
s = "中文 mslang ✓"
assert(encoding.base64_decode(encoding.base64_encode(s)) == s, "b64 往返")
assert(encoding.hex_decode(encoding.hex_encode(s)) == s, "hex 往返")
assert(encoding.url_decode(encoding.url_encode(s)) == s, "url 往返")
assert(encoding.url_encode("a b/c") == "a%20b/c", "url safe=/")
assert(encoding.url_decode("a+b") == "a+b", "加号字面")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "encoding integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_encoding_error_paths() {
        // 错误路径为原生 Err（不经 try/except，task 80 惯例）：整体 Err + 前缀断言。
        for (call, expect) in [
            ("encoding.base64_decode(\"A\")", "ValueError"),
            ("encoding.hex_decode(\"abc\")", "ValueError"),
            ("encoding.url_decode(\"%ZZ\")", "ValueError"),
            ("encoding.url_decode(\"%FF\")", "ValueError"),
            ("encoding.base64_encode(42)", "TypeError"),
            ("encoding.url_encode(\"a\", 1)", "TypeError"),
        ] {
            let full = format!("import encoding\n{}", call);
            let r = run_source(&full);
            assert!(r.is_err(), "{} should fail", call);
            let e = r.unwrap_err();
            assert!(e.contains(expect), "{}: expected {} in {}", call, expect, e);
        }
    }
}
