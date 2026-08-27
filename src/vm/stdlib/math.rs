//! `math` 原生模块。
//!
//! 参照 [47-stdlib-math](../../../docs/mslang/tasks/47-stdlib-math.md)。

use super::{expect_int, expect_number, float_to_int};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{alloc_module, alloc_tuple, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;

// ---------------------------------------------------------------------------
// math 模块
// ---------------------------------------------------------------------------

/// 构造 `math` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 5 个 inline Float 常量（pi/e/tau/inf/nan）+ 40 个原生函数
/// （task 80 扩充：+3 常量、+27 函数、log 升级 (x, base?)）。
pub fn register_math_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();

    // 常量（inline Object::Float，无需堆分配；14-gc.md:54 内联值不参与 GC 扫描）
    exports.insert("pi".to_string(), Object::Float(std::f64::consts::PI));
    exports.insert("e".to_string(), Object::Float(std::f64::consts::E));
    // task 80：tau/inf/nan（16-stdlib-expansion.md §4.1）
    exports.insert("tau".to_string(), Object::Float(std::f64::consts::TAU));
    exports.insert("inf".to_string(), Object::Float(f64::INFINITY));
    exports.insert("nan".to_string(), Object::Float(f64::NAN));

    // 函数（alloc_native_function → Object::Ref + TypeTag::FUNCTION）
    let funcs: [(&str, NativeFn); 40] = [
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
        // task 80 扩充（16-stdlib-expansion.md §4.1）
        ("asin", native_math_asin),
        ("acos", native_math_acos),
        ("atan", native_math_atan),
        ("atan2", native_math_atan2),
        ("sinh", native_math_sinh),
        ("cosh", native_math_cosh),
        ("tanh", native_math_tanh),
        ("asinh", native_math_asinh),
        ("acosh", native_math_acosh),
        ("atanh", native_math_atanh),
        ("cbrt", native_math_cbrt),
        ("hypot", native_math_hypot),
        ("trunc", native_math_trunc),
        ("sign", native_math_sign),
        ("fmod", native_math_fmod),
        ("modf", native_math_modf),
        ("copysign", native_math_copysign),
        ("degrees", native_math_degrees),
        ("radians", native_math_radians),
        ("gcd", native_math_gcd),
        ("lcm", native_math_lcm),
        ("factorial", native_math_factorial),
        ("comb", native_math_comb),
        ("perm", native_math_perm),
        ("isqrt", native_math_isqrt),
        ("is_nan", native_math_is_nan),
        ("is_inf", native_math_is_inf),
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
    // task 80：log(x, base?)（base 缺省 e）。与 gc.count 等同名治理无关，但 arity
    // 升级 MAX（§2.2），native 内自校验 1-2 参。
    if args.is_empty() || args.len() > 2 {
        return Err(format!(
            "TypeError: log(x, base?) takes 1-2 arguments, got {}",
            args.len()
        ));
    }
    let x = expect_number(args.get(0), "log(x, base?)")?;
    if args.len() == 1 {
        return Ok(Object::Float(x.ln()));
    }
    let base = expect_number(args.get(1), "log(x, base?)")?;
    if base == 1.0 {
        return Err("ValueError: log() base must not be 1".to_string());
    }
    if base <= 0.0 {
        return Err("ValueError: log() base must be positive".to_string());
    }
    // base 2/10 走专用 log2/log10（精确：log(8,2)=3.0、log(100,10)=2.0）。
    if base == 2.0 {
        Ok(Object::Float(x.log2()))
    } else if base == 10.0 {
        Ok(Object::Float(x.log10()))
    } else {
        Ok(Object::Float(x.ln() / base.ln()))
    }
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

// ---------------------------------------------------------------------------
// task 80 扩充（16-stdlib-expansion.md §4.1）
// ---------------------------------------------------------------------------

// ---- 反三角 / 双曲（域外返回 NaN，与现状 sqrt/log 一致，不抛错）----

fn native_math_asin(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "asin(x)")?;
    Ok(Object::Float(x.asin()))
}

fn native_math_acos(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "acos(x)")?;
    Ok(Object::Float(x.acos()))
}

fn native_math_atan(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "atan(x)")?;
    Ok(Object::Float(x.atan()))
}

fn native_math_atan2(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let y = expect_number(args.get(0), "atan2(y, x)")?;
    let x = expect_number(args.get(1), "atan2(y, x)")?;
    Ok(Object::Float(y.atan2(x)))
}

fn native_math_sinh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "sinh(x)")?;
    Ok(Object::Float(x.sinh()))
}

fn native_math_cosh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "cosh(x)")?;
    Ok(Object::Float(x.cosh()))
}

fn native_math_tanh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "tanh(x)")?;
    Ok(Object::Float(x.tanh()))
}

fn native_math_asinh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "asinh(x)")?;
    Ok(Object::Float(x.asinh()))
}

fn native_math_acosh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "acosh(x)")?;
    Ok(Object::Float(x.acosh()))
}

fn native_math_atanh(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "atanh(x)")?;
    Ok(Object::Float(x.atanh()))
}

// ---- 数值 ----

fn native_math_cbrt(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "cbrt(x)")?;
    Ok(Object::Float(x.cbrt()))
}

fn native_math_hypot(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "hypot(x, y)")?;
    let y = expect_number(args.get(1), "hypot(x, y)")?;
    Ok(Object::Float(x.hypot(y)))
}

fn native_math_trunc(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "trunc(x)")?;
    float_to_int(x.trunc(), "trunc")
}

fn native_math_sign(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // -1/0/1；NaN → 0（Go math.Sign 语义）。
    let x = expect_number(args.get(0), "sign(x)")?;
    let s = if x.is_nan() || x == 0.0 {
        0
    } else if x > 0.0 {
        1
    } else {
        -1
    };
    Ok(Object::Int(s))
}

fn native_math_fmod(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // C 语义取余（截断，与 `%` 的地板取整区分）；Rust f64 `%` 即 C fmod。
    let x = expect_number(args.get(0), "fmod(x, y)")?;
    let y = expect_number(args.get(1), "fmod(x, y)")?;
    Ok(Object::Float(x % y))
}

fn native_math_modf(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // (小数部分, 整数部分)，均为 Float（Python 语义）。
    let x = expect_number(args.get(0), "modf(x)")?;
    Ok(alloc_tuple(vec![
        Object::Float(x.fract()),
        Object::Float(x.trunc()),
    ]))
}

fn native_math_copysign(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 取 x 幅值 + y 符号。
    let x = expect_number(args.get(0), "copysign(x, y)")?;
    let y = expect_number(args.get(1), "copysign(x, y)")?;
    Ok(Object::Float(x.copysign(y)))
}

// ---- 角度 ----

fn native_math_degrees(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "degrees(x)")?;
    Ok(Object::Float(x.to_degrees()))
}

fn native_math_radians(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "radians(x)")?;
    Ok(Object::Float(x.to_radians()))
}

// ---- 整数族（参数非法 → ValueError；checked 运算溢出 → OverflowError）----

fn native_math_gcd(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 非负；gcd(0,n)=n；负数取绝对值（Python math.gcd 语义）。
    // u64 Euclid 无溢出；结果 > i64::MAX → OverflowError（如 gcd(MIN, MIN)）。
    let a = expect_int(args.get(0), "gcd(a, b)")?.unsigned_abs();
    let b = expect_int(args.get(1), "gcd(a, b)")?.unsigned_abs();
    let (mut x, mut y) = (a, b);
    while y != 0 {
        let t = x % y;
        x = y;
        y = t;
    }
    i64::try_from(x).map(Object::Int).map_err(|_| {
        "OverflowError: gcd() result out of int range".to_string()
    })
}

fn native_math_lcm(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // lcm(0,n)=0；负数取绝对值；checked 乘法溢出 → OverflowError。
    let a = expect_int(args.get(0), "lcm(a, b)")?.unsigned_abs();
    let b = expect_int(args.get(1), "lcm(a, b)")?.unsigned_abs();
    let (mut x, mut y) = (a, b);
    while y != 0 {
        let t = x % y;
        x = y;
        y = t;
    }
    let l = if x == 0 {
        0u64
    } else {
        (a / x).checked_mul(b).ok_or_else(|| {
            "OverflowError: lcm() result out of int range".to_string()
        })?
    };
    i64::try_from(l).map(Object::Int).map_err(|_| {
        "OverflowError: lcm() result out of int range".to_string()
    })
}

fn native_math_factorial(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 范围 0-20（21! 溢出 i64 → OverflowError）；负数 ValueError。
    let n = expect_int(args.get(0), "factorial(n)")?;
    if n < 0 {
        return Err("ValueError: factorial() not defined for negative values".to_string());
    }
    if n > 20 {
        return Err("OverflowError: factorial() argument out of range (max 20)".to_string());
    }
    let mut acc: i64 = 1;
    for i in 2..=n {
        acc *= i;
    }
    Ok(Object::Int(acc))
}

fn native_math_comb(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 组合数；k>n → 0；负数 ValueError；checked 中间溢出 → OverflowError。
    let n = expect_int(args.get(0), "comb(n, k)")?;
    let k = expect_int(args.get(1), "comb(n, k)")?;
    if n < 0 || k < 0 {
        return Err("ValueError: comb() arguments must be non-negative".to_string());
    }
    if k > n {
        return Ok(Object::Int(0));
    }
    // 精确步进：每步 c = C(n-k+i, i)（整数除法精确）；中间值受 i64 约束。
    let k = k.min(n - k);
    let mut c: i64 = 1;
    for i in 1..=k {
        c = c
            .checked_mul(n - k + i)
            .ok_or_else(|| "OverflowError: comb() result out of int range".to_string())?;
        c /= i;
    }
    Ok(Object::Int(c))
}

fn native_math_perm(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // 排列数；k>n → 0；负数 ValueError；checked 溢出 → OverflowError。
    let n = expect_int(args.get(0), "perm(n, k)")?;
    let k = expect_int(args.get(1), "perm(n, k)")?;
    if n < 0 || k < 0 {
        return Err("ValueError: perm() arguments must be non-negative".to_string());
    }
    if k > n {
        return Ok(Object::Int(0));
    }
    let mut c: i64 = 1;
    for i in 0..k {
        c = c
            .checked_mul(n - i)
            .ok_or_else(|| "OverflowError: perm() result out of int range".to_string())?;
    }
    Ok(Object::Int(c))
}

fn native_math_isqrt(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // ⌊√n⌋；负数 ValueError。f64 初估 + 整数修正（checked 防近 i64::MAX 溢出）。
    let n = expect_int(args.get(0), "isqrt(n)")?;
    if n < 0 {
        return Err("ValueError: isqrt() argument must be non-negative".to_string());
    }
    let mut r = (n as f64).sqrt() as i64;
    while r.checked_mul(r).is_none_or(|v| v > n) {
        r -= 1;
    }
    while let Some(v) = (r + 1).checked_mul(r + 1) {
        if v <= n {
            r += 1;
        } else {
            break;
        }
    }
    Ok(Object::Int(r))
}

// ---- 谓词 ----

fn native_math_is_nan(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "is_nan(x)")?;
    Ok(Object::Bool(x.is_nan()))
}

fn native_math_is_inf(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let x = expect_number(args.get(0), "is_inf(x)")?;
    Ok(Object::Bool(x.is_infinite()))
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

    // ---- task 80：math 扩充 ----

    #[test]
    fn test_math_module_registration_ext() {
        // task 80：exports 含 5 常量 + 40 函数。
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            let m = read_module_mut(ptr);
            for name in ["tau", "inf", "nan"] {
                assert!(m.exports.contains_key(name), "missing constant: {}", name);
            }
            for name in [
                "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh",
                "atanh", "cbrt", "hypot", "trunc", "sign", "fmod", "modf", "copysign",
                "degrees", "radians", "gcd", "lcm", "factorial", "comb", "perm", "isqrt",
                "is_nan", "is_inf", "log",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_math_new_constants() {
        let ptr = register_math_module();
        // SAFETY: ptr 由 register_math_module 返回的有效 MsModule。
        unsafe {
            let m = read_module_mut(ptr);
            assert!(approx(fval(&m.exports["tau"]), std::f64::consts::TAU));
            assert!(fval(&m.exports["inf"]).is_infinite());
            assert!(fval(&m.exports["nan"]).is_nan());
        }
    }

    #[test]
    fn test_math_inverse_trig_and_hyperbolic() {
        let mut v = vm();
        assert!(approx(
            fval(&native_math_asin(&mut v, &[Object::Float(1.0)]).unwrap()),
            std::f64::consts::FRAC_PI_2
        ));
        assert!(approx(fval(&native_math_acos(&mut v, &[Object::Float(1.0)]).unwrap()), 0.0));
        assert!(approx(
            fval(&native_math_atan(&mut v, &[Object::Float(1.0)]).unwrap()),
            std::f64::consts::FRAC_PI_4
        ));
        assert!(approx(
            fval(&native_math_atan2(&mut v, &[Object::Float(1.0), Object::Float(1.0)]).unwrap()),
            std::f64::consts::FRAC_PI_4
        ));
        // 双曲零点
        assert!(approx(fval(&native_math_sinh(&mut v, &[Object::Int(0)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_cosh(&mut v, &[Object::Int(0)]).unwrap()), 1.0));
        assert!(approx(fval(&native_math_tanh(&mut v, &[Object::Int(0)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_asinh(&mut v, &[Object::Int(0)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_acosh(&mut v, &[Object::Int(1)]).unwrap()), 0.0));
        assert!(approx(fval(&native_math_atanh(&mut v, &[Object::Int(0)]).unwrap()), 0.0));
        // 域外 NaN（不抛错，与 sqrt/log 一致）
        assert!(fval(&native_math_asin(&mut v, &[Object::Float(2.0)]).unwrap()).is_nan());
        assert!(fval(&native_math_acosh(&mut v, &[Object::Float(0.5)]).unwrap()).is_nan());
        assert!(fval(&native_math_atanh(&mut v, &[Object::Float(2.0)]).unwrap()).is_nan());
    }

    #[test]
    fn test_math_numeric_functions() {
        let mut v = vm();
        assert!(approx(fval(&native_math_cbrt(&mut v, &[Object::Int(27)]).unwrap()), 3.0));
        // cbrt 负数域（区别于 sqrt）
        assert!(approx(fval(&native_math_cbrt(&mut v, &[Object::Int(-8)]).unwrap()), -2.0));
        assert!(approx(
            fval(&native_math_hypot(&mut v, &[Object::Int(3), Object::Int(4)]).unwrap()),
            5.0
        ));
        // hypot 无中间溢出：√(1e308² + 1e308²) = √2·1e308（直算 x²+y² 会溢出为 inf）
        let h = fval(
            &native_math_hypot(&mut v, &[Object::Float(1e308), Object::Float(1e308)]).unwrap(),
        );
        assert!(h.is_finite() && h > 1.414e308 && h < 1.415e308, "hypot = {}", h);
        // trunc 经 float_to_int 返回 Int
        assert_eq!(native_math_trunc(&mut v, &[Object::Float(3.7)]).unwrap(), Object::Int(3));
        assert_eq!(native_math_trunc(&mut v, &[Object::Float(-3.7)]).unwrap(), Object::Int(-3));
        let err = native_math_trunc(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError"));
        // sign：-1/0/1；NaN → 0
        assert_eq!(native_math_sign(&mut v, &[Object::Float(-2.5)]).unwrap(), Object::Int(-1));
        assert_eq!(native_math_sign(&mut v, &[Object::Int(0)]).unwrap(), Object::Int(0));
        assert_eq!(native_math_sign(&mut v, &[Object::Float(3.0)]).unwrap(), Object::Int(1));
        assert_eq!(
            native_math_sign(&mut v, &[Object::Float(f64::NAN)]).unwrap(),
            Object::Int(0)
        );
        // fmod：C 截断语义（与 % 地板取整区分）
        assert!(approx(fval(&native_math_fmod(&mut v, &[Object::Int(7), Object::Int(3)]).unwrap()), 1.0));
        assert!(approx(fval(&native_math_fmod(&mut v, &[Object::Int(-7), Object::Int(3)]).unwrap()), -1.0));
        // modf：(小数, 整数) 均为 Float
        match native_math_modf(&mut v, &[Object::Float(1.25)]).unwrap() {
            Object::Ref(p) => {
                // SAFETY: modf 返回 alloc_tuple 的 Ref。
                let t = unsafe { crate::vm::object::read_tuple(p) };
                assert!(approx(fval(&t[0]), 0.25));
                assert!(approx(fval(&t[1]), 1.0));
            }
            _ => panic!("modf must return tuple"),
        }
        // copysign：x 幅值 + y 符号
        assert!(approx(
            fval(&native_math_copysign(&mut v, &[Object::Int(3), Object::Int(-1)]).unwrap()),
            -3.0
        ));
        // degrees/radians
        assert!(approx(
            fval(&native_math_degrees(&mut v, &[Object::Float(std::f64::consts::PI)]).unwrap()),
            180.0
        ));
        assert!(approx(
            fval(&native_math_radians(&mut v, &[Object::Int(180)]).unwrap()),
            std::f64::consts::PI
        ));
    }

    #[test]
    fn test_math_integer_functions() {
        let mut v = vm();
        assert_eq!(
            native_math_gcd(&mut v, &[Object::Int(12), Object::Int(18)]).unwrap(),
            Object::Int(6)
        );
        // 负数取绝对值（Python math.gcd 语义）
        assert_eq!(
            native_math_gcd(&mut v, &[Object::Int(-12), Object::Int(18)]).unwrap(),
            Object::Int(6)
        );
        assert_eq!(
            native_math_gcd(&mut v, &[Object::Int(0), Object::Int(5)]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            native_math_lcm(&mut v, &[Object::Int(4), Object::Int(6)]).unwrap(),
            Object::Int(12)
        );
        assert_eq!(
            native_math_lcm(&mut v, &[Object::Int(0), Object::Int(5)]).unwrap(),
            Object::Int(0)
        );
        assert_eq!(
            native_math_factorial(&mut v, &[Object::Int(5)]).unwrap(),
            Object::Int(120)
        );
        assert_eq!(
            native_math_factorial(&mut v, &[Object::Int(0)]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            native_math_comb(&mut v, &[Object::Int(5), Object::Int(2)]).unwrap(),
            Object::Int(10)
        );
        // k > n → 0
        assert_eq!(
            native_math_comb(&mut v, &[Object::Int(3), Object::Int(5)]).unwrap(),
            Object::Int(0)
        );
        assert_eq!(
            native_math_perm(&mut v, &[Object::Int(5), Object::Int(2)]).unwrap(),
            Object::Int(20)
        );
        assert_eq!(
            native_math_isqrt(&mut v, &[Object::Int(17)]).unwrap(),
            Object::Int(4)
        );
        assert_eq!(
            native_math_isqrt(&mut v, &[Object::Int(16)]).unwrap(),
            Object::Int(4)
        );
        assert_eq!(
            native_math_isqrt(&mut v, &[Object::Int(0)]).unwrap(),
            Object::Int(0)
        );
        // i64 边界：⌊√(i64::MAX)⌋ = 3037000499
        assert_eq!(
            native_math_isqrt(&mut v, &[Object::Int(i64::MAX)]).unwrap(),
            Object::Int(3037000499)
        );
    }

    #[test]
    fn test_math_integer_function_errors() {
        let mut v = vm();
        // 21! 溢出 i64 → OverflowError
        let err = native_math_factorial(&mut v, &[Object::Int(21)]).unwrap_err();
        assert!(err.contains("OverflowError"), "got: {}", err);
        // 负数 → ValueError
        let err = native_math_factorial(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_math_isqrt(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // comb(100, 50) 中间值溢出 i64 → OverflowError
        let err = native_math_comb(&mut v, &[Object::Int(100), Object::Int(50)]).unwrap_err();
        assert!(err.contains("OverflowError"), "got: {}", err);
        let err = native_math_comb(&mut v, &[Object::Int(-1), Object::Int(2)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_math_perm(&mut v, &[Object::Int(2), Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // gcd(i64::MIN, i64::MIN) = 2^63 > i64::MAX → OverflowError
        let err =
            native_math_gcd(&mut v, &[Object::Int(i64::MIN), Object::Int(i64::MIN)]).unwrap_err();
        assert!(err.contains("OverflowError"), "got: {}", err);
        // lcm 溢出（2^62 与 3 互质 → 3·2^62 > i64::MAX）
        let err = native_math_lcm(&mut v, &[Object::Int(4611686018427387904), Object::Int(3)])
            .unwrap_err();
        assert!(err.contains("OverflowError"), "got: {}", err);
        // 非 Int 入参 → TypeError
        let err = native_math_gcd(&mut v, &[Object::Float(12.0), Object::Int(18)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_math_predicates() {
        let mut v = vm();
        assert_eq!(
            native_math_is_nan(&mut v, &[Object::Float(f64::NAN)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_math_is_nan(&mut v, &[Object::Float(1.0)]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            native_math_is_inf(&mut v, &[Object::Float(f64::INFINITY)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_math_is_inf(&mut v, &[Object::Float(f64::NEG_INFINITY)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_math_is_inf(&mut v, &[Object::Int(1)]).unwrap(),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_math_log_with_base() {
        let mut v = vm();
        // base 2/10 走专用路径（精确）
        assert!(approx(fval(&native_math_log(&mut v, &[Object::Int(8), Object::Int(2)]).unwrap()), 3.0));
        assert!(approx(
            fval(&native_math_log(&mut v, &[Object::Int(100), Object::Int(10)]).unwrap()),
            2.0
        ));
        // 缺省 base = e
        assert!(approx(fval(&native_math_log(&mut v, &[Object::Float(std::f64::consts::E)]).unwrap()), 1.0));
        // 其他 base：ln(x)/ln(base)
        assert!(approx(
            fval(&native_math_log(&mut v, &[Object::Int(9), Object::Float(3.0)]).unwrap()),
            2.0
        ));
        // base=1 → ValueError
        let err = native_math_log(&mut v, &[Object::Int(8), Object::Int(1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // base<=0 → ValueError（Python 对齐）
        let err = native_math_log(&mut v, &[Object::Int(8), Object::Int(-2)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        // 非数值 → TypeError
        let err = native_math_log(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // arity 自校验（MAX）：0 参 / 3 参 → TypeError
        let err = native_math_log(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("1-2"), "got: {}", err);
        let err =
            native_math_log(&mut v, &[Object::Int(2), Object::Int(2), Object::Int(2)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_integration_math_ext() {
        // 端到端：task 80 扩充函数（等价 test_math_ext.ms 的值域部分）。
        let src = r#"
import math
assert(math.tau == 2 * math.pi)
assert(math.inf > 1e308)
assert(math.is_nan(math.nan))
assert(abs(math.asin(1) - math.pi / 2) < 1e-12)
assert(math.hypot(3, 4) == 5.0)
assert(math.gcd(12, 18) == 6)
assert(math.gcd(-12, 18) == 6)
assert(math.factorial(5) == 120)
assert(math.isqrt(17) == 4)
assert(math.log(8, 2) == 3.0)
assert(math.log(100, 10) == math.log10(100))
assert(math.log(100, 10) == 2.0)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "math ext integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_math_ext_error_paths() {
        // 端到端错误路径（原生 Err 不经 try/except，整体 Err）：
        // factorial(21) → OverflowError；factorial(-1)/isqrt(-1) → ValueError；
        // log(8, 1) → ValueError；log("x") → TypeError。
        for (src, expect) in [
            ("math.factorial(21)", "OverflowError"),
            ("math.factorial(-1)", "ValueError"),
            ("math.isqrt(-1)", "ValueError"),
            ("math.comb(100, 50)", "OverflowError"),
            ("math.log(8, 1)", "ValueError"),
            ("math.log(\"x\")", "TypeError"),
        ] {
            let full = format!("import math\n{}", src);
            let r = run_source(&full);
            assert!(r.is_err(), "{} should fail", src);
            let e = r.unwrap_err();
            assert!(e.contains(expect), "{}: expected {} in {}", src, expect, e);
        }
    }
}
