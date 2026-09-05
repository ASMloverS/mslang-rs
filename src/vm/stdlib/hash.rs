//! `hash` 原生模块（task 85）。
//!
//! 参照 [85-stdlib-regex-hash](../../../docs/mslang/tasks/85-stdlib-regex-hash.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.17。
//!
//! md5 / sha1 / sha256 / sha512（string → 小写 hex）。仅 string 输入（UTF-8
//! 字节）；文件哈希留白（开放问题 5）。md5/sha1 为**非安全用途**（文档警示）。
//!
//! md-5/sha1/sha2 0.10 系的 `digest` 方法来自各自 re-export 的 `Digest` trait，
//! 三个 crate 共存时在各函数内 `use <crate>::Digest as _` 别名导入，避免
//! trait 名冲突（task 85 §sha/md 实现骨架）。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

/// 构造 `hash` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_hash_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 4] = [
        ("md5", native_hash_md5),
        ("sha1", native_hash_sha1),
        ("sha256", native_hash_sha256),
        ("sha512", native_hash_sha512),
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
    let m = alloc_module("hash");
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

/// 摘要字节 → 小写 hex。
fn to_hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        // u8 hex 写入 String 不会失败（容量预分配）。
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// hash.md5(s) -> string：32 位小写 hex（**非安全用途**）。arity 1。
fn native_hash_md5(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use md5::Digest as _;
    let s = expect_string(args.get(0), "md5(s)")?;
    let mut h = md5::Md5::new();
    h.update(s.as_bytes());
    Ok(alloc_string(&to_hex(&h.finalize())))
}

/// hash.sha1(s) -> string：40 位小写 hex（**非安全用途**）。arity 1。
fn native_hash_sha1(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use sha1::Digest as _;
    let s = expect_string(args.get(0), "sha1(s)")?;
    let mut h = sha1::Sha1::new();
    h.update(s.as_bytes());
    Ok(alloc_string(&to_hex(&h.finalize())))
}

/// hash.sha256(s) -> string：64 位小写 hex。arity 1。
fn native_hash_sha256(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use sha2::Digest as _;
    let s = expect_string(args.get(0), "sha256(s)")?;
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    Ok(alloc_string(&to_hex(&h.finalize())))
}

/// hash.sha512(s) -> string：128 位小写 hex。arity 1。
fn native_hash_sha512(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use sha2::Digest as _;
    let s = expect_string(args.get(0), "sha512(s)")?;
    let mut h = sha2::Sha512::new();
    h.update(s.as_bytes());
    Ok(alloc_string(&to_hex(&h.finalize())))
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::test_util::{run_source, strval, vm};
    use super::*;
    use crate::vm::object::TypeTag;

    /// 四函数的长度 / 小写不变式（验证标准 9）。
    fn check_format(digest: &str, expect_len: usize, name: &str) {
        assert_eq!(digest.len(), expect_len, "{} 长度", name);
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{} 须为小写 hex：{}",
            name,
            digest
        );
    }

    #[test]
    fn test_hash_module_registration() {
        let ptr = register_hash_module();
        // SAFETY: ptr 由 register_hash_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "hash");
            for name in ["md5", "sha1", "sha256", "sha512"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    /// 验证标准 9：空串与 "abc" 标准向量（RFC 1321 / FIPS 180-1 / FIPS 180-2）。
    #[test]
    fn test_hash_known_vectors() {
        use std::slice::from_ref;
        let mut v = vm();
        let empty = super::super::test_util::s("");
        let abc = super::super::test_util::s("abc");

        assert_eq!(
            strval(&native_hash_md5(&mut v, from_ref(&empty)).unwrap()),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            strval(&native_hash_md5(&mut v, from_ref(&abc)).unwrap()),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            strval(&native_hash_sha1(&mut v, from_ref(&empty)).unwrap()),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            strval(&native_hash_sha1(&mut v, from_ref(&abc)).unwrap()),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            strval(&native_hash_sha256(&mut v, from_ref(&empty)).unwrap()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            strval(&native_hash_sha256(&mut v, from_ref(&abc)).unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            strval(&native_hash_sha512(&mut v, from_ref(&empty)).unwrap()),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
             47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            strval(&native_hash_sha512(&mut v, from_ref(&abc)).unwrap()),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    /// UTF-8 字节输入（中文）+ 格式不变式。
    #[test]
    fn test_hash_utf8_and_format() {
        use std::slice::from_ref;
        let mut v = vm();
        let zh = super::super::test_util::s("你好");
        for (name, len, digest) in [
            ("md5", 32, native_hash_md5(&mut v, from_ref(&zh)).unwrap()),
            ("sha1", 40, native_hash_sha1(&mut v, from_ref(&zh)).unwrap()),
            (
                "sha256",
                64,
                native_hash_sha256(&mut v, from_ref(&zh)).unwrap(),
            ),
            (
                "sha512",
                128,
                native_hash_sha512(&mut v, from_ref(&zh)).unwrap(),
            ),
        ] {
            check_format(&strval(&digest), len, name);
        }
        // 确定性：同输入同输出。
        let a = strval(&native_hash_md5(&mut v, &[super::super::test_util::s("mslang")]).unwrap());
        let b = strval(&native_hash_md5(&mut v, &[super::super::test_util::s("mslang")]).unwrap());
        assert_eq!(a, b);
    }

    /// 非法输入 → TypeError（原生 Err 由本单测覆盖，task 80 惯例）。
    #[test]
    fn test_hash_type_errors() {
        let mut v = vm();
        let err = native_hash_md5(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(
            err.contains("TypeError") && err.contains("md5"),
            "got: {}",
            err
        );
        let err = native_hash_sha256(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_integration_hash_module() {
        let src = r#"
import hash
assert(hash.md5("") == "d41d8cd98f00b204e9800998ecf8427e")
assert(hash.md5("abc") == "900150983cd24fb0d6963f7d28e17f72")
assert(hash.sha1("abc") == "a9993e364706816aba3e25717850c26c9cd0d89d")
assert(hash.sha256("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
assert(len(hash.sha512("abc")) == 128)
assert(hash.md5("你好") == hash.md5("你好"))
h1 = hash.sha256("mslang")
assert(len(h1) == 64)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "hash integration failed: {:?}", r.err());
    }
}
