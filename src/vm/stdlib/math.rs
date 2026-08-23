//! `math` 原生模块。
//!
//! 参照 [47-stdlib-math](../../../docs/mslang/tasks/47-stdlib-math.md)。

use super::{expect_number, float_to_int};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{alloc_module, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

// ---------------------------------------------------------------------------
// math 模块
// ---------------------------------------------------------------------------

/// 构造 `math` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 2 个 inline Float 常量（pi/e）+ 13 个原生函数。
pub fn register_math_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();

    // 常量（inline Object::Float，无需堆分配；14-gc.md:54 内联值不参与 GC 扫描）
    exports.insert("pi".to_string(), Object::Float(std::f64::consts::PI));
    exports.insert("e".to_string(), Object::Float(std::f64::consts::E));

    // 函数（alloc_native_function → Object::Ref + TypeTag::FUNCTION）
    let funcs: [(&str, NativeFn); 13] = [
        ("sqrt", native_math_sqrt),
        ("pow", native_math_pow),
        ("abs", native_math_abs),
        ("sin", native_math_sin),
        ("cos", native_math_cos),
        ("tan", native_math_tan),
        ("log", native_math_log),
        ("log2", native_math_log2),
        ("log10", native_math_log10),
        ("exp", native_math_exp),
        ("ceil", native_math_ceil),
        ("floor", native_math_floor),
        ("round", native_math_round),
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

    let m = alloc_module("math");
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

// 注：sqrt/pow/sin/cos/tan/log/log2/log10/exp 返回 Object::Float；
// abs 保留入参类型；ceil/floor/round 经 float_to_int 返回 Object::Int。

fn native_math_sqrt(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sqrt(x)")?;
    Ok(Object::Float(x.sqrt()))
}

fn native_math_pow(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let base = expect_number(args.get(0), "pow(base, exp)")?;
    let exp = expect_number(args.get(1), "pow(base, exp)")?;
    Ok(Object::Float(base.powf(exp)))
}

fn native_math_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 保留入参类型：Int→Int, Float→Float, Bool→Int（与全局 abs(n)->number 一致）
    match args.get(0) {
        Some(Object::Int(n)) => Ok(Object::Int(n.wrapping_abs())),
        Some(Object::Float(x)) => Ok(Object::Float(x.abs())),
        Some(Object::Bool(true)) => Ok(Object::Int(1)),
        Some(Object::Bool(false)) => Ok(Object::Int(0)),
        other => Err(format!(
            "TypeError: abs(x) expects number, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

fn native_math_sin(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sin(x)")?;
    Ok(Object::Float(x.sin()))
}

fn native_math_cos(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "cos(x)")?;
    Ok(Object::Float(x.cos()))
}

fn native_math_tan(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "tan(x)")?;
    Ok(Object::Float(x.tan()))
}

fn native_math_log(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log(x)")?;
    Ok(Object::Float(x.ln()))
}

fn native_math_log2(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log2(x)")?;
    Ok(Object::Float(x.log2()))
}

fn native_math_log10(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "log10(x)")?;
    Ok(Object::Float(x.log10()))
}

fn native_math_exp(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "exp(x)")?;
    Ok(Object::Float(x.exp()))
}

fn native_math_ceil(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "ceil(x)")?;
    float_to_int(x.ceil(), "ceil")
}

fn native_math_floor(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "floor(x)")?;
    float_to_int(x.floor(), "floor")
}

fn native_math_round(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // Rust f64::round()：半远离零（round(2.5)→3，非 Python 银行家舍入）。
    let x = expect_number(args.get(0), "round(x)")?;
    float_to_int(x.round(), "round")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{fval, run_source, s, vm};
    use crate::vm::object::TypeTag;

    // ---- math 模块 ----

    /// 浮点近似相等（单测辅助）。
    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn test_math_module_registration() {
        // register_math_module 返回 MODULE，exports 含 2 常量 + 13 函数。
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "math");
            assert!(m.exports.contains_key("pi"));
            assert!(m.exports.contains_key("e"));
            for name in [
                "sqrt", "pow", "abs", "sin", "cos", "tan", "log", "log2", "log10", "exp", "ceil",
                "floor", "round",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_math_constants() {
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            let m = read_module_mut(ptr);
            assert!(approx(fval(&m.exports["pi"]), std::f64::consts::PI));
            assert!(approx(fval(&m.exports["e"]), std::f64::consts::E));
        }
    }

    #[test]
    fn test_math_sqrt() {
        let mut v = vm();
        assert!(approx(fval(&native_math_sqrt(&mut v, &[Object::Float(16.0)]).unwrap()), 4.0));
        // 整数入参自动转 Float
        assert!(approx(fval(&native_math_sqrt(&mut v, &[Object::Int(9)]).unwrap()), 3.0));
        // 负数 → NaN（不抛，IEEE 754）
        assert!(fval(&native_math_sqrt(&mut v, &[Object::Float(-1.0)]).unwrap()).is_nan());
    }

    #[test]
    fn test_math_pow() {
        let mut v = vm();
        assert!(approx(
            fval(&native_math_pow(&mut v, &[Object::Int(2), Object::Int(10)]).unwrap()),
            1024.0
        ));
        // pow(0, -1) → Infinity；pow(-1, 0.5) → NaN（§7 域错误）
        assert!(fval(&native_math_pow(&mut v, &[Object::Int(0), Object::Int(-1)]).unwrap()).is_infinite());
        assert!(fval(&native_math_pow(&mut v, &[Object::Int(-1), Object::Float(0.5)]).unwrap()).is_nan());
    }

    #[test]
    fn test_math_abs_preserves_type() {
        let mut v = vm();
        // Int→Int
        assert_eq!(native_math_abs(&mut v, &[Object::Int(-42)]).unwrap(), Object::Int(42));
        // Float→Float
        assert!(approx(fval(&native_math_abs(&mut v, &[Object::Float(-2.5)]).unwrap()), 2.5));
        // Bool→Int
        assert_eq!(native_math_abs(&mut v, &[Object::Bool(true)]).unwrap(), Object::Int(1));
        assert_eq!(native_math_abs(&mut v, &[Object::Bool(false)]).unwrap(), Object::Int(0));
    }

    #[test]
    fn test_math_trig() {
        let mut v = vm();
        assert!(approx(fval(&native_math_sin(&mut v, &[Object::Float(0.0)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_cos(&mut v, &[Object::Float(0.0)]).unwrap()), 1.0));
        assert!(approx(fval(&native_math_tan(&mut v, &[Object::Float(0.0)]).unwrap()), 0.0));
        // sin(π/2) ≈ 1
        assert!(approx(
            fval(&native_math_sin(&mut v, &[Object::Float(std::f64::consts::FRAC_PI_2)]).unwrap()),
            1.0
        ));
    }

    #[test]
    fn test_math_logs_and_exp() {
        let mut v = vm();
        assert!(approx(fval(&native_math_log(&mut v, &[Object::Float(100.0)]).unwrap()), 4.605170185988091));
        assert!(approx(fval(&native_math_log2(&mut v, &[Object::Float(8.0)]).unwrap()), 3.0));
        assert!(approx(fval(&native_math_log10(&mut v, &[Object::Float(100.0)]).unwrap()), 2.0));
        assert!(approx(fval(&native_math_exp(&mut v, &[Object::Float(1.0)]).unwrap()), std::f64::consts::E));
        // 域错误：log(0) → -Inf；log(-1) → NaN；exp(710) → +Inf（§7）
        assert!(fval(&native_math_log(&mut v, &[Object::Float(0.0)]).unwrap()).is_infinite());
        assert!(fval(&native_math_log(&mut v, &[Object::Float(-1.0)]).unwrap()).is_nan());
        assert!(fval(&native_math_exp(&mut v, &[Object::Float(710.0)]).unwrap()).is_infinite());
    }

    #[test]
    fn test_math_ceil_floor_round_return_int() {
        let mut v = vm();
        // 返回 Object::Int（非 Float）
        assert_eq!(native_math_ceil(&mut v, &[Object::Float(3.2)]).unwrap(), Object::Int(4));
        assert_eq!(native_math_floor(&mut v, &[Object::Float(3.8)]).unwrap(), Object::Int(3));
        assert_eq!(native_math_round(&mut v, &[Object::Float(3.5)]).unwrap(), Object::Int(4));
    }

    #[test]
    fn test_math_round_half_away_from_zero() {
        // §6：半远离零（round(2.5)→3，非 Python 银行家舍入 round(2.5)→2）
        let mut v = vm();
        assert_eq!(native_math_round(&mut v, &[Object::Float(2.5)]).unwrap(), Object::Int(3));
        assert_eq!(native_math_round(&mut v, &[Object::Float(3.5)]).unwrap(), Object::Int(4));
        assert_eq!(native_math_round(&mut v, &[Object::Float(0.5)]).unwrap(), Object::Int(1));
        assert_eq!(native_math_round(&mut v, &[Object::Float(-2.5)]).unwrap(), Object::Int(-3));
    }

    #[test]
    fn test_math_ceil_nan_and_overflow_errors() {
        let mut v = vm();
        // ceil(NaN) → ValueError（§5/§9）
        let err = native_math_ceil(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("NaN"));
        // ceil(1e30) → OverflowError（§5/§9，Rust as i64 会静默饱和）
        let err = native_math_ceil(&mut v, &[Object::Float(1e30)]).unwrap_err();
        assert!(err.contains("OverflowError"));
        // floor/round 同样受 float_to_int 保护
        let err = native_math_floor(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError"));
        let err = native_math_round(&mut v, &[Object::Float(-1e30)]).unwrap_err();
        assert!(err.contains("OverflowError"));
    }

    #[test]
    fn test_expect_number_type_errors() {
        let mut v = vm();
        // 非数值入参 → TypeError
        let err = native_math_sqrt(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
        // 缺参 → TypeError (missing)
        let err = native_math_sqrt(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("missing"));
        // abs 非数值 → TypeError
        let err = native_math_abs(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    // ---- 端到端集成测试 ----

    #[test]
    fn test_integration_math_basics() {
        // 等价 test_math.ms（值经 abs() 容差断言，避免浮点字面量位级歧义）
        let src = r#"
import math
assert(abs(math.pi - 3.141592653589793) < 1e-15)
assert(math.sqrt(16) == 4.0)
assert(math.pow(2, 10) == 1024.0)
assert(abs(math.sin(math.pi / 2) - 1.0) < 1e-12)
assert(abs(math.log(100) - 4.605170185988091) < 1e-12)
assert(math.log2(8) == 3.0)
assert(math.log10(100) == 2.0)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math basics failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_math_extra() {
        // 等价 test_math_extra.ms：含 math.round(2.5) → 3（半远离零）
        let src = r#"
import math
assert(math.cos(0) == 1.0)
assert(math.tan(0) == 0.0)
assert(abs(math.exp(1) - 2.718281828459045) < 1e-15)
assert(math.ceil(3.2) == 4)
assert(math.floor(3.8) == 3)
assert(math.round(3.5) == 4)
assert(math.round(2.5) == 3)
assert(math.abs(-42) == 42)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math extra failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_from_math_import() {
        // from math import 提取导出名（常量 + 函数）。
        let src = r#"
from math import sqrt, pi, e, abs
assert(sqrt(25) == 5.0)
assert(pi == 3.141592653589793)
assert(e == 2.718281828459045)
assert(abs(-7) == 7)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "from math import failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_import_std_prefix() {
        // import @std math：@std 前缀经 parse_std_prefix 剥离后命中原生模块。
        let src = r#"
import @std math
assert(math.sqrt(49) == 7.0)
assert(math.floor(2.9) == 2)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "import @std math failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_math_ceil_overflow_error() {
        // 端到端：math.ceil(1e30) 在 VM 中抛 OverflowError（异常传播路径）。
        let src = r#"
import math
math.ceil(1e30)
"#;
        let r = run_source(src);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("OverflowError"), "expected OverflowError");
    }

    #[test]
    fn test_integration_math_abs_type_preservation() {
        // math.abs(-42) 为 Int，math.abs(-3.14) 为 Float（§4 类型保留）。
        let src = r#"
import math
i = math.abs(-42)
assert(i == 42)
assert(type(i) == "int")
f = math.abs(-2.5)
assert(f == 2.5)
assert(type(f) == "float")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math abs type failed: {:?}", r.err());
    }
}
