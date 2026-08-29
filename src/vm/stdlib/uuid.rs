//! `uuid` 原生模块（task 81）。
//!
//! 参照 [81-stdlib-random-encoding-uuid](../../../docs/mslang/tasks/81-stdlib-random-encoding-uuid.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.6。
//!
//! RFC 4122 版本 4：122 位熵取自 random 模块的 thread_local StdRng
//!（`random.seed` 后可复现），version/variant 位手工置位，格式化手写。

use rand::RngCore;

use super::random::with_rng;
use crate::vm::builtins::{alloc_native_function, NativeFunction};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

/// 构造 `uuid` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_uuid_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    exports.insert(
        "uuid4".to_string(),
        alloc_native_function(NativeFunction {
            name: "uuid4".to_string(),
            func: native_uuid4,
        }),
    );
    let m = alloc_module("uuid");
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

/// uuid4() -> string：36 字符小写连字符（8-4-4-4-12）。
/// time_hi_and_version 高 4 位 = 0100（version 4，byte 6）；
/// clock_seq_hi_and_reserved 高 2 位 = 10（variant，byte 8）。
fn native_uuid4(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let mut b = [0u8; 16];
    with_rng(|rng| rng.fill_bytes(&mut b));
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10xx
    let hex: String = b.iter().map(|x| format!("{:02x}", x)).collect();
    Ok(alloc_string(&format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )))
}

#[cfg(test)]
mod tests {
    use super::super::random::native_random_seed;
    use super::super::test_util::{run_source, strval, vm};
    use super::*;
    use crate::vm::object::TypeTag;

    /// 校验单个 uuid 的格式不变式，返回该字符串。
    fn check_format(u: &str) {
        assert_eq!(u.len(), 36, "长度 36：{}", u);
        let hyphens: Vec<usize> = u
            .char_indices()
            .filter(|(_, c)| *c == '-')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hyphens, vec![8, 13, 18, 23], "连字符位置：{}", u);
        assert!(
            u.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
            "字符集为 hex + '-'：{}",
            u
        );
        assert!(
            u.chars()
                .filter(|&c| c != '-')
                .all(|c| !c.is_ascii_uppercase()),
            "小写输出：{}",
            u
        );
        // 0-based 第 14 位 = version '4'；第 19 位 ∈ 89ab（variant）
        //（spec 1-based 计数为第 13/17 个 hex 字符，连字符不计）。
        let chars: Vec<char> = u.chars().collect();
        assert_eq!(chars[14], '4', "version 位：{}", u);
        assert!("89ab".contains(chars[19]), "variant 位：{}", u);
    }

    #[test]
    fn test_uuid_module_registration() {
        let ptr = register_uuid_module();
        // SAFETY: ptr 由 register_uuid_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "uuid");
            assert!(m.exports.contains_key("uuid4"), "missing export: uuid4");
        }
    }

    #[test]
    fn test_uuid4_format() {
        // 验证标准 9：36 字符模式、第 13 位（1-based）= '4'、第 17 位 ∈ 89ab。
        let mut v = vm();
        for _ in 0..200 {
            let u = strval(&native_uuid4(&mut v, &[]).unwrap());
            check_format(&u);
        }
    }

    #[test]
    fn test_uuid4_uniqueness() {
        // 验证标准 9：连续生成 100 个互不相同。
        let mut v = vm();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let u = strval(&native_uuid4(&mut v, &[]).unwrap());
            assert!(seen.insert(u), "重复 uuid");
        }
        assert_eq!(seen.len(), 100);
    }

    #[test]
    fn test_uuid4_seeded_reproducible() {
        // uuid4 复用 random 模块生成器：random.seed(n) 后序列确定（跨模块行为契约）。
        let mut v = vm();
        native_random_seed(&mut v, &[Object::Int(42)]).unwrap();
        let a = strval(&native_uuid4(&mut v, &[]).unwrap());
        native_random_seed(&mut v, &[Object::Int(42)]).unwrap();
        let b = strval(&native_uuid4(&mut v, &[]).unwrap());
        assert_eq!(a, b, "seed 后 uuid4 确定");
        assert_ne!(
            a,
            strval(&native_uuid4(&mut v, &[]).unwrap()),
            "继续生成不同值"
        );
    }

    #[test]
    fn test_integration_uuid_module() {
        let src = r#"
import uuid
u = uuid.uuid4()
assert(len(u) == 36, "长度 36")
assert(u[8] == "-" and u[13] == "-" and u[18] == "-" and u[23] == "-", "连字符位置")
assert(u[14] == "4", "version 4")
v = u[19]
assert(v == "8" or v == "9" or v == "a" or v == "b", "variant 89ab")
seen = {}
i = 0
while i < 100 {
    x = uuid.uuid4()
    assert(len(x) == 36 and x[14] == "4", "批量生成格式")
    seen[x] = true
    i = i + 1
}
assert(len(seen) == 100, "100 个互不相同")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "uuid integration failed: {:?}", r.err());
    }
}
