//! `time` 原生模块。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)。

use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{alloc_module, alloc_string, read_module_mut, MsObjHeader, Object};
use crate::vm::VM;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// time 模块
// ---------------------------------------------------------------------------

/// 构造 `time` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 now/sleep/format 三个原生函数。
pub fn register_time_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 3] = [
        ("now", native_time_now),
        ("sleep", native_time_sleep),
        ("format", native_time_format),
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
    let m = alloc_module("time");
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

/// time.now() → 当前 Unix 时间戳（秒，f64）。
/// 不使用 .unwrap()：系统时间早于 epoch 时返回 Err 而非 panic。
fn native_time_now(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("ClockError: system time before epoch: {}", e))?;
    Ok(Object::Float(dur.as_secs_f64()))
}

/// time.sleep(secs) → 阻塞指定秒数（int 或 float）。
/// 单位为秒（与 10-builtins.md:326 一致，非毫秒）。负数 / 非有限值 → ValueError。
fn native_time_sleep(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let secs = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => {
            return Err(format!(
                "TypeError: sleep(secs) expects number, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    // from_secs_f64 在 NaN/±Inf 上 panic，须先拒绝非有限值。
    if !secs.is_finite() {
        return Err("ValueError: sleep duration must be finite".into());
    }
    if secs < 0.0 {
        return Err("ValueError: sleep duration cannot be negative".into());
    }
    std::thread::sleep(Duration::from_secs_f64(secs));
    Ok(Object::Nil)
}

/// time.format(ts) → 将 Unix 时间戳格式化为 UTC 字符串 "YYYY-MM-DD HH:MM:SS"。
/// MVP 手动格式化（不引入 chrono 依赖）。时区固定 UTC。
fn native_time_format(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ts = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => {
            return Err(format!(
                "TypeError: format(ts) expects number, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    if ts < 0.0 {
        return Err("ValueError: timestamp cannot be negative".into());
    }
    let secs = ts as u64;
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    Ok(alloc_string(&format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )))
}

/// Unix 时间戳（秒）→ UTC 年月日时分秒（民用历法算法，Howard Hinnant
/// `civil_from_days`）。纯整数运算，无 chrono 依赖。
fn unix_to_ymdhms(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64; // 自 1970-01-01 的天数
    let rem = secs % 86_400;
    let hour = (rem / 3_600) as u32;
    let min = ((rem % 3_600) / 60) as u32;
    let sec = (rem % 60) as u32;

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32, hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{fval, run_source, s, vm};
    use crate::vm::object::TypeTag;

    // ---- task 48：time 模块 ----

    #[test]
    fn test_time_module_registration() {
        let ptr = register_time_module();
        // SAFETY: ptr 由 register_time_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "time");
            for name in ["now", "sleep", "format"] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_time_now() {
        let mut v = vm();
        let r = native_time_now(&mut v, &[]).unwrap();
        let ts = fval(&r);
        // 合理的时间戳（> 2001-09-09，即 > 1e9）
        assert!(ts > 1_000_000_000.0, "time.now() returned {}", ts);
    }

    #[test]
    fn test_time_sleep_zero() {
        let mut v = vm();
        // sleep(0) / sleep(0.0) 立即返回 nil
        assert_eq!(native_time_sleep(&mut v, &[Object::Int(0)]).unwrap(), Object::Nil);
        assert_eq!(
            native_time_sleep(&mut v, &[Object::Float(0.0)]).unwrap(),
            Object::Nil
        );
    }

    #[test]
    fn test_time_sleep_errors() {
        let mut v = vm();
        // 负数 → ValueError
        let err = native_time_sleep(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("negative"));
        let err = native_time_sleep(&mut v, &[Object::Float(-0.5)]).unwrap_err();
        assert!(err.contains("ValueError"));
        // NaN/Inf → ValueError（防止 from_secs_f64 panic）
        let err = native_time_sleep(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("ValueError"));
        let err = native_time_sleep(&mut v, &[Object::Float(f64::INFINITY)]).unwrap_err();
        assert!(err.contains("ValueError"));
        // 非数值 → TypeError
        let err = native_time_sleep(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_unix_to_ymdhms_epoch() {
        assert_eq!(unix_to_ymdhms(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn test_unix_to_ymdhms_known() {
        // 1700000000 = 2023-11-14 22:13:20 UTC
        assert_eq!(unix_to_ymdhms(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        // 1000000000 = 2001-09-09 01:46:40 UTC
        assert_eq!(unix_to_ymdhms(1_000_000_000), (2001, 9, 9, 1, 46, 40));
    }

    #[test]
    fn test_unix_to_ymdhms_leap_day() {
        // 2020-02-29 12:00:00 UTC = 1582977600（闰日）
        assert_eq!(unix_to_ymdhms(1_582_977_600), (2020, 2, 29, 12, 0, 0));
    }

    #[test]
    fn test_time_format() {
        let mut v = vm();
        // Int 时间戳
        assert_eq!(
            native_time_format(&mut v, &[Object::Int(0)]).unwrap(),
            s("1970-01-01 00:00:00")
        );
        assert_eq!(
            native_time_format(&mut v, &[Object::Int(1_700_000_000)]).unwrap(),
            s("2023-11-14 22:13:20")
        );
        // Float 时间戳（截断小数）
        assert_eq!(
            native_time_format(&mut v, &[Object::Float(0.0)]).unwrap(),
            s("1970-01-01 00:00:00")
        );
        assert_eq!(
            native_time_format(&mut v, &[Object::Float(1_700_000_000.999)]).unwrap(),
            s("2023-11-14 22:13:20")
        );
    }

    #[test]
    fn test_time_format_errors() {
        let mut v = vm();
        let err = native_time_format(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError") && err.contains("negative"));
        let err = native_time_format(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_integration_time() {
        let src = r#"
import time
assert(time.format(0) == "1970-01-01 00:00:00")
assert(time.format(1700000000) == "2023-11-14 22:13:20")
assert(time.now() > 1000000000)
assert(time.sleep(0) == nil)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "time integration failed: {:?}", r.err());
    }
}
