//! `time` 原生模块。
//!
//! 参照 [48-stdlib-os-string-time](../../../docs/mslang/tasks/48-stdlib-os-string-time.md)。

use super::{expect_int, expect_string, float_to_int};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_dict, alloc_module, alloc_string, read_module_mut, DictMap, MsObjHeader, Object,
};
use crate::vm::VM;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// time 模块
// ---------------------------------------------------------------------------

/// 构造 `time` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 now/sleep/format（task 48）与 task 83 扩充的
/// now_ms/monotonic/iso/date_parts/sleep_ms/format_ts/parse 共 10 个原生函数。
pub fn register_time_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 10] = [
        ("now", native_time_now),
        ("sleep", native_time_sleep),
        ("format", native_time_format),
        // task 83 扩充（16-stdlib-expansion.md §4.10）
        ("now_ms", native_time_now_ms),
        ("monotonic", native_time_monotonic),
        ("iso", native_time_iso),
        ("date_parts", native_time_date_parts),
        ("sleep_ms", native_time_sleep_ms),
        ("format_ts", native_time_format_ts),
        ("parse", native_time_parse),
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

// ---------------------------------------------------------------------------
// task 83 扩充（16-stdlib-expansion.md §4.10）
// ---------------------------------------------------------------------------

/// 单调基线：进程内首次调用 monotonic() 时固定（OnceLock 惰性初始化），
/// 之后所有读数相对同一原点（进程启动为 0 点）。
static MONO_BASE: OnceLock<Instant> = OnceLock::new();

/// time.now_ms() → 当前 Unix 时间戳（毫秒，Int）。
fn native_time_now_ms(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("ClockError: system time before epoch: {}", e))?;
    Ok(Object::Int(dur.as_millis() as i64))
}

/// time.monotonic() → 单调秒（Float），进程启动为 0 点；用于计时非报时。
fn native_time_monotonic(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let base = MONO_BASE.get_or_init(Instant::now);
    Ok(Object::Float(
        Instant::now().duration_since(*base).as_secs_f64(),
    ))
}

/// 当前 Unix 秒（iso/date_parts 缺省 ts 用，与 time.now 同源）。
fn now_secs() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| format!("ClockError: system time before epoch: {}", e))
}

/// ts 参数统一校验（iso/date_parts/format_ts 共用）：接受 Int 与 Float（与
/// time.format 一致）；负数 → ValueError；Float 非有限值 → ValueError、超出
/// i64 可表示范围 → OverflowError（经 `float_to_int` 语义校验，§2.3 禁止
/// `as u64` 静默饱和），再截断取整秒。
fn ts_to_secs(arg: Option<&Object>, who: &str) -> Result<u64, String> {
    match arg {
        Some(Object::Int(n)) => {
            if *n >= 0 {
                Ok(*n as u64)
            } else {
                Err(format!("ValueError: {} timestamp cannot be negative", who))
            }
        }
        Some(Object::Float(x)) => {
            if !x.is_finite() {
                return Err(format!("ValueError: {} timestamp must be finite", who));
            }
            match float_to_int(*x, who)? {
                Object::Int(secs) if secs >= 0 => Ok(secs as u64),
                _ => Err(format!("ValueError: {} timestamp cannot be negative", who)),
            }
        }
        other => Err(format!(
            "TypeError: {} expects number, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

/// time.iso(ts?) → UTC ISO 8601 "YYYY-MM-DDTHH:MM:SSZ"；缺省当前时间。
/// arity MAX（native 内自校验 0-1 参，§2.2）。
fn native_time_iso(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() > 1 {
        return Err(format!(
            "TypeError: iso(ts?) takes 0 or 1 argument, got {}",
            args.len()
        ));
    }
    let secs = match args.get(0) {
        Some(arg) => ts_to_secs(Some(arg), "iso(ts?)")?,
        None => now_secs()?,
    };
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    Ok(alloc_string(&format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    )))
}

/// time.date_parts(ts?) → dict `{year,month,day,hour,minute,second,weekday}`；
/// weekday 0=周一…6=周日（Python 约定；1970-01-01 为周四=3）。
/// 缺省当前时间；arity MAX（native 内自校验 0-1 参，§2.2）。
fn native_time_date_parts(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() > 1 {
        return Err(format!(
            "TypeError: date_parts(ts?) takes 0 or 1 argument, got {}",
            args.len()
        ));
    }
    let secs = match args.get(0) {
        Some(arg) => ts_to_secs(Some(arg), "date_parts(ts?)")?,
        None => now_secs()?,
    };
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    // 1970-01-01 为周四：days_since_epoch 偏移 +3 后 mod 7 → 0=周一。
    let weekday = (secs / 86_400 + 3) % 7;
    let mut map = DictMap::new();
    map.insert(alloc_string("year"), Object::Int(year));
    map.insert(alloc_string("month"), Object::Int(month as i64));
    map.insert(alloc_string("day"), Object::Int(day as i64));
    map.insert(alloc_string("hour"), Object::Int(hour as i64));
    map.insert(alloc_string("minute"), Object::Int(min as i64));
    map.insert(alloc_string("second"), Object::Int(sec as i64));
    map.insert(alloc_string("weekday"), Object::Int(weekday as i64));
    Ok(alloc_dict(map))
}

/// time.sleep_ms(ms) → 阻塞指定毫秒（Int）；负数 → ValueError、非 int → TypeError。
/// 协程场景应使用 async.sleep（毫秒、非阻塞）；阻塞 sleep_ms 会饿死其他协程。
fn native_time_sleep_ms(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ms = expect_int(args.get(0), "sleep_ms(ms)")?;
    if ms < 0 {
        return Err("ValueError: sleep_ms duration cannot be negative".to_string());
    }
    std::thread::sleep(Duration::from_millis(ms as u64));
    Ok(Object::Nil)
}

/// 未知格式指令错误前缀（format_ts 与 parse 共用指令集 %Y %m %d %H %M %S %%）。
const DIRECTIVE_ERROR: &str = "ValueError: invalid time directive";

/// 格式串 token（format_ts / parse 共享扫描）。
enum FmtToken {
    /// 字面字符（format_ts 原样输出；parse 精确匹配）。
    Literal(char),
    /// 指令字符（`%` 后一字符；`%` 表示 `%%` 字面百分号）。
    Directive(char),
}

/// 格式串逐字符扫描：`%` + 指令字符为一段，其余为字面字符。
/// 指令集外指令字符（如 `%q`）与孤立 `%` → ValueError（format_ts 与 parse 同）。
fn scan_fmt(fmt: &str) -> Result<Vec<FmtToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            tokens.push(FmtToken::Literal(c));
            continue;
        }
        match chars.next() {
            Some(dir) => {
                if matches!(dir, 'Y' | 'm' | 'd' | 'H' | 'M' | 'S' | '%') {
                    tokens.push(FmtToken::Directive(dir));
                } else {
                    return Err(format!("{}: %{}", DIRECTIVE_ERROR, dir));
                }
            }
            None => return Err("ValueError: dangling % in format".to_string()),
        }
    }
    Ok(tokens)
}

/// time.format_ts(ts, fmt) → 按指令集格式化 UTC 时间。
/// `%Y` 4 位、`%m/%d/%H/%M/%S` 2 位零填充、`%%` 输出 `%`；字面段原样输出。
fn native_time_format_ts(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let secs = ts_to_secs(args.get(0), "format_ts(ts, fmt)")?;
    let fmt = expect_string(args.get(1), "format_ts(ts, fmt)")?;
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    let mut out = String::new();
    for token in scan_fmt(&fmt)? {
        match token {
            FmtToken::Literal(c) => out.push(c),
            FmtToken::Directive('Y') => out.push_str(&format!("{:04}", year)),
            FmtToken::Directive('m') => out.push_str(&format!("{:02}", month)),
            FmtToken::Directive('d') => out.push_str(&format!("{:02}", day)),
            FmtToken::Directive('H') => out.push_str(&format!("{:02}", hour)),
            FmtToken::Directive('M') => out.push_str(&format!("{:02}", min)),
            FmtToken::Directive('S') => out.push_str(&format!("{:02}", sec)),
            FmtToken::Directive(_) => out.push('%'), // %%
        }
    }
    Ok(alloc_string(&out))
}

/// time.parse(s, fmt) → 按指令集解析为 Unix 秒（Float）。
/// §2.2 同名冲突：与 json.parse（1 参）共享名 → arity MAX，自校验恰 2 参。
/// 字面段精确匹配；指令段贪婪扫描数字（`%Y` 1-4 位、其余 1-2 位）；
/// 域越界 / 多余输入 / 结果 ts < 0（1970 前日期）→ ValueError。
fn native_time_parse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err(format!(
            "TypeError: parse(s, fmt) takes exactly 2 arguments, got {}",
            args.len()
        ));
    }
    let input = expect_string(args.get(0), "parse(s, fmt)")?;
    let fmt = expect_string(args.get(1), "parse(s, fmt)")?;
    let ts = parse_with_fmt(&input, &fmt)?;
    if ts < 0 {
        return Err("ValueError: parse result before epoch 1970-01-01".to_string());
    }
    Ok(Object::Float(ts as f64))
}

/// strptime 式解析：fmt/input 双指针推进。缺省域取 Python 默认 1900-01-01
/// 00:00:00（缺 `%Y` 的结果通常早于 1970 → 由调用方 ts<0 校验拒绝，保证往返一致）。
fn parse_with_fmt(input: &str, fmt: &str) -> Result<i64, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0usize;
    // (year, month, day, hour, minute, second)
    let mut f: [Option<i64>; 6] = [None; 6];
    for token in scan_fmt(fmt)? {
        match token {
            // 字面字符与 %% 精确匹配
            FmtToken::Literal(c) => match_literal(&chars, &mut pos, c)?,
            FmtToken::Directive('%') => match_literal(&chars, &mut pos, '%')?,
            // 指令段按位数贪婪扫描数字：%Y 1-4 位、其余 1-2 位
            FmtToken::Directive(dir) => {
                let (slot, max) = match dir {
                    'Y' => (0, 4),
                    'm' => (1, 2),
                    'd' => (2, 2),
                    'H' => (3, 2),
                    'M' => (4, 2),
                    _ => (5, 2), // 'S'（指令集已由 scan_fmt 限定）
                };
                f[slot] = Some(scan_digits(&chars, &mut pos, max)?);
            }
        }
    }
    if pos != chars.len() {
        return Err(format!("ValueError: trailing characters at char {}", pos));
    }
    let year = f[0].unwrap_or(1900);
    let month = f[1].unwrap_or(1);
    let day = f[2].unwrap_or(1);
    let (hour, min, sec) = (f[3].unwrap_or(0), f[4].unwrap_or(0), f[5].unwrap_or(0));
    if !(1..=12).contains(&month) {
        return Err(format!("ValueError: month {} out of range 1-12", month));
    }
    if !(0..=23).contains(&hour) {
        return Err(format!("ValueError: hour {} out of range 0-23", hour));
    }
    if !(0..=59).contains(&min) {
        return Err(format!("ValueError: minute {} out of range 0-59", min));
    }
    if !(0..=59).contains(&sec) {
        return Err(format!("ValueError: second {} out of range 0-59", sec));
    }
    let dim = days_in_month(year, month);
    if day < 1 || day > dim {
        return Err(format!(
            "ValueError: day {} out of range 1-{} for {}-{:02}",
            day, dim, year, month
        ));
    }
    Ok(ymdhms_to_unix(year, month, day, hour, min, sec))
}

/// 精确匹配一个字面字符并推进 pos（字面段与 %% 共用）。
fn match_literal(chars: &[char], pos: &mut usize, c: char) -> Result<(), String> {
    if *pos < chars.len() && chars[*pos] == c {
        *pos += 1;
        Ok(())
    } else {
        Err(format!(
            "ValueError: literal mismatch at char {}: expected '{}'",
            pos, c
        ))
    }
}

/// 从 chars[pos..] 贪婪扫描 1..=max 位 ASCII 数字并推进 pos。
fn scan_digits(chars: &[char], pos: &mut usize, max: usize) -> Result<i64, String> {
    let start = *pos;
    while *pos < chars.len() && *pos - start < max && chars[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return Err(format!("ValueError: expected digits at char {}", start));
    }
    let s: String = chars[start..*pos].iter().collect();
    s.parse::<i64>()
        .map_err(|_| format!("ValueError: number out of range at char {}", start))
}

/// 闰年（Gregorian，与 civil 历法一致）。
fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 当月天数（month 已由 parse 域校验限定 1-12）。
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
    }
}

/// UTC 年月日时分秒 → Unix 秒（Howard Hinnant `days_from_civil`，与既有
/// `unix_to_ymdhms` 互逆，纯整数运算）。输入假定已通过域校验。
fn ymdhms_to_unix(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3_600 + min * 60 + sec
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{fval, run_source, s, strval, vm};
    use crate::vm::object::{read_dict, TypeTag};

    /// 从 dict 中按 string key 取 int 值（测试辅助，date_parts 断言用）。
    fn dict_int(d: &Object, key: &str) -> i64 {
        let Object::Ref(ptr) = d else {
            panic!("expected dict ref")
        };
        let k = alloc_string(key);
        let v = unsafe { read_dict(*ptr) }.get(&k).cloned().unwrap_or_else(|| {
            panic!("key '{}' not in date_parts dict", key)
        });
        match v {
            Object::Int(n) => n,
            _ => panic!("value for '{}' is not int", key),
        }
    }

    // ---- task 48：time 模块 ----

    #[test]
    fn test_time_module_registration() {
        let ptr = register_time_module();
        // SAFETY: ptr 由 register_time_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "time");
            for name in [
                "now",
                "sleep",
                "format",
                // task 83 扩充
                "now_ms",
                "monotonic",
                "iso",
                "date_parts",
                "sleep_ms",
                "format_ts",
                "parse",
            ] {
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

    // ---- task 83：time 扩充 ----

    #[test]
    fn test_time_now_ms_vs_now() {
        let mut v = vm();
        let now_ms = match native_time_now_ms(&mut v, &[]).unwrap() {
            Object::Int(n) => n,
            other => panic!("expected Int, got {}", other.type_name()),
        };
        let now = fval(&native_time_now(&mut v, &[]).unwrap());
        let diff_ms = (now_ms as f64 / 1000.0 - now).abs() * 1000.0;
        assert!(diff_ms < 50.0, "now_ms vs now diff {}ms", diff_ms);
    }

    #[test]
    fn test_time_monotonic_and_sleep_ms() {
        let mut v = vm();
        let t0 = fval(&native_time_monotonic(&mut v, &[]).unwrap());
        let t1 = fval(&native_time_monotonic(&mut v, &[]).unwrap());
        assert!(t0 >= 0.0, "monotonic 以进程启动为 0 点");
        assert!(t1 >= t0, "monotonic 非降");
        assert_eq!(
            native_time_sleep_ms(&mut v, &[Object::Int(0)]).unwrap(),
            Object::Nil
        );
        native_time_sleep_ms(&mut v, &[Object::Int(50)]).unwrap();
        let t2 = fval(&native_time_monotonic(&mut v, &[]).unwrap());
        assert!(t2 - t0 >= 0.05, "sleep_ms(50) 后单调差 {}s", t2 - t0);
    }

    #[test]
    fn test_time_sleep_ms_errors() {
        let mut v = vm();
        // 负数 → ValueError
        let e = native_time_sleep_ms(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(e.contains("ValueError") && e.contains("negative"));
        // 非 int（string / Float）→ TypeError（签名为 Int 毫秒）
        let e = native_time_sleep_ms(&mut v, &[s("x")]).unwrap_err();
        assert!(e.contains("TypeError"));
        let e = native_time_sleep_ms(&mut v, &[Object::Float(1.5)]).unwrap_err();
        assert!(e.contains("TypeError"));
    }

    #[test]
    fn test_time_iso() {
        let mut v = vm();
        assert_eq!(
            native_time_iso(&mut v, &[Object::Int(0)]).unwrap(),
            s("1970-01-01T00:00:00Z")
        );
        assert_eq!(
            native_time_iso(&mut v, &[Object::Int(1_700_000_000)]).unwrap(),
            s("2023-11-14T22:13:20Z")
        );
        // Float ts 截断取整秒
        assert_eq!(
            native_time_iso(&mut v, &[Object::Float(1_700_000_000.999)]).unwrap(),
            s("2023-11-14T22:13:20Z")
        );
        // 缺省当前时间：形如 YYYY-MM-DDTHH:MM:SSZ
        let d = strval(&native_time_iso(&mut v, &[]).unwrap());
        assert_eq!(d.len(), 20, "iso 缺省长度: {}", d);
        assert!(d.ends_with('Z') && d.as_bytes()[10] == b'T');
        assert!(d[..4].parse::<i64>().unwrap() >= 2024, "iso 缺省年份: {}", d);
    }

    #[test]
    fn test_time_iso_errors() {
        let mut v = vm();
        // ts < 0 → ValueError
        let e = native_time_iso(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(e.contains("ValueError") && e.contains("negative"));
        // NaN / ±Inf → ValueError（禁止静默饱和，§2.3）
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let e = native_time_iso(&mut v, &[Object::Float(x)]).unwrap_err();
            assert!(e.contains("ValueError"), "NaN/Inf got: {}", e);
        }
        // 超出 i64 可表示范围 → OverflowError
        let e = native_time_iso(&mut v, &[Object::Float(1e30)]).unwrap_err();
        assert!(e.contains("OverflowError"), "got: {}", e);
        // 非数值 → TypeError
        let e = native_time_iso(&mut v, &[s("x")]).unwrap_err();
        assert!(e.contains("TypeError"));
        // arity MAX 自校验：2 参 → TypeError
        let e = native_time_iso(&mut v, &[Object::Int(0), Object::Int(0)]).unwrap_err();
        assert!(e.contains("TypeError") && e.contains("0 or 1"), "got: {}", e);
    }

    #[test]
    fn test_time_date_parts() {
        let mut v = vm();
        let d = native_time_date_parts(&mut v, &[Object::Int(0)]).unwrap();
        // 1970-01-01 为周四（weekday 0=周一）
        assert_eq!(dict_int(&d, "year"), 1970);
        assert_eq!(dict_int(&d, "month"), 1);
        assert_eq!(dict_int(&d, "day"), 1);
        assert_eq!(dict_int(&d, "hour"), 0);
        assert_eq!(dict_int(&d, "minute"), 0);
        assert_eq!(dict_int(&d, "second"), 0);
        assert_eq!(dict_int(&d, "weekday"), 3);
        // 86400 = 1970-01-02 周五
        let d2 = native_time_date_parts(&mut v, &[Object::Int(86_400)]).unwrap();
        assert_eq!(dict_int(&d2, "weekday"), 4);
        // 2023-11-14 为周二
        let d3 = native_time_date_parts(&mut v, &[Object::Int(1_700_000_000)]).unwrap();
        assert_eq!(dict_int(&d3, "year"), 2023);
        assert_eq!(dict_int(&d3, "weekday"), 1);
        // 缺省当前时间
        let d4 = native_time_date_parts(&mut v, &[]).unwrap();
        assert!(dict_int(&d4, "year") >= 2024);
    }

    #[test]
    fn test_time_date_parts_errors() {
        let mut v = vm();
        let e = native_time_date_parts(&mut v, &[Object::Int(-1)]).unwrap_err();
        assert!(e.contains("ValueError") && e.contains("negative"));
        let e = native_time_date_parts(&mut v, &[Object::Float(f64::NAN)]).unwrap_err();
        assert!(e.contains("ValueError"));
        let e = native_time_date_parts(&mut v, &[s("x")]).unwrap_err();
        assert!(e.contains("TypeError"));
        let e = native_time_date_parts(&mut v, &[Object::Int(0), Object::Int(0)]).unwrap_err();
        assert!(e.contains("TypeError"));
    }

    #[test]
    fn test_time_format_ts() {
        let mut v = vm();
        assert_eq!(
            native_time_format_ts(&mut v, &[Object::Int(0), s("%Y-%m-%d %H:%M:%S")]).unwrap(),
            s("1970-01-01 00:00:00")
        );
        assert_eq!(
            native_time_format_ts(&mut v, &[Object::Int(1_700_000_000), s("%Y/%m/%d %H:%M:%S")])
                .unwrap(),
            s("2023/11/14 22:13:20")
        );
        // 指令相邻无分隔
        assert_eq!(
            native_time_format_ts(&mut v, &[Object::Int(1_700_000_000), s("%Y%m%d%H%M%S")])
                .unwrap(),
            s("20231114221320")
        );
        // %% 字面与字面段原样输出
        assert_eq!(
            native_time_format_ts(&mut v, &[Object::Int(0), s("%%Y %%")]).unwrap(),
            s("%Y %")
        );
        // Float ts 截断
        assert_eq!(
            native_time_format_ts(&mut v, &[Object::Float(59.9), s("%S")]).unwrap(),
            s("59")
        );
    }

    #[test]
    fn test_time_format_ts_errors() {
        let mut v = vm();
        // 未知指令 %q / 孤立 % 结尾 → ValueError（与 parse 同）
        let e = native_time_format_ts(&mut v, &[Object::Int(0), s("%q")]).unwrap_err();
        assert!(e.contains("ValueError") && e.contains("directive"));
        let e = native_time_format_ts(&mut v, &[Object::Int(0), s("abc%")]).unwrap_err();
        assert!(e.contains("ValueError") && e.contains("dangling"));
        // ts 校验：负数 / NaN / 非数值
        let e = native_time_format_ts(&mut v, &[Object::Int(-1), s("%Y")]).unwrap_err();
        assert!(e.contains("ValueError"));
        let e = native_time_format_ts(&mut v, &[Object::Float(f64::NAN), s("%Y")]).unwrap_err();
        assert!(e.contains("ValueError"));
        let e = native_time_format_ts(&mut v, &[s("x"), s("%Y")]).unwrap_err();
        assert!(e.contains("TypeError"));
        // fmt 非 string → TypeError
        let e = native_time_format_ts(&mut v, &[Object::Int(0), Object::Int(1)]).unwrap_err();
        assert!(e.contains("TypeError"));
    }

    /// parse 结果断言辅助：期望 Unix 秒。
    fn parse_secs(v: &mut VM, input: &str, fmt: &str) -> f64 {
        fval(&native_time_parse(v, &[s(input), s(fmt)]).unwrap())
    }

    /// parse 错误断言辅助：期望含 kind（ValueError/TypeError）的 Err。
    fn parse_err(v: &mut VM, input: &str, fmt: &str, kind: &str) {
        let e = native_time_parse(v, &[s(input), s(fmt)]).unwrap_err();
        assert!(e.contains(kind), "parse({:?}, {:?}): {}", input, fmt, e);
    }

    #[test]
    fn test_time_parse_known() {
        let mut v = vm();
        assert_eq!(
            parse_secs(&mut v, "2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S"),
            1_700_000_000.0
        );
        assert_eq!(
            parse_secs(&mut v, "1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S"),
            0.0
        );
        assert_eq!(parse_secs(&mut v, "2000-02-29", "%Y-%m-%d"), 951_782_400.0);
        // 贪婪 1-2 位：单数字月/日/时/分/秒
        assert_eq!(
            parse_secs(&mut v, "2023-1-4 5:6:7", "%Y-%m-%d %H:%M:%S"),
            1_672_808_767.0
        );
        // %% 匹配字面 %
        assert_eq!(parse_secs(&mut v, "2023%11", "%Y%%%m"), 1_698_796_800.0);
    }

    #[test]
    fn test_time_parse_errors() {
        let mut v = vm();
        // 字面不匹配 / 月 13 越界 / 日 32 越界 / 多余尾部输入
        parse_err(&mut v, "2023/11/14", "%Y-%m-%d", "ValueError");
        parse_err(&mut v, "2023-13-01", "%Y-%m-%d", "ValueError");
        parse_err(&mut v, "2023-01-32", "%Y-%m-%d", "ValueError");
        parse_err(&mut v, "2023-11-14x", "%Y-%m-%d", "ValueError");
        // 日不超当月天数：2023 非闰年 2 月 29 / 平年 2 月 30；闰年 2 月 29 合法
        parse_err(&mut v, "2023-02-29", "%Y-%m-%d", "ValueError");
        parse_err(&mut v, "2023-02-30", "%Y-%m-%d", "ValueError");
        assert!(native_time_parse(&mut v, &[s("2024-02-29"), s("%Y-%m-%d")]).is_ok());
        // 结果 ts < 0（1970 前）；缺 %Y 默认 1900 → 同样 ts < 0
        let e = native_time_parse(&mut v, &[s("1969-12-31 23:59:59"), s("%Y-%m-%d %H:%M:%S")])
            .unwrap_err();
        assert!(e.contains("ValueError") && e.contains("epoch"), "got: {}", e);
        parse_err(&mut v, "12:30:00", "%H:%M:%S", "ValueError");
        // 未知指令 / 孤立 % / 期望数字处非数字
        parse_err(&mut v, "x", "%q", "ValueError");
        parse_err(&mut v, "2023", "%Y-", "ValueError");
        parse_err(&mut v, "2023-ab", "%Y-%m", "ValueError");
        // arity MAX 自校验：1 参 / 3 参 → TypeError
        let e = native_time_parse(&mut v, &[s("2023")]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
        let e = native_time_parse(&mut v, &[s("a"), s("b"), s("c")]).unwrap_err();
        assert!(e.contains("TypeError"), "got: {}", e);
    }

    #[test]
    fn test_time_parse_format_roundtrip() {
        let mut v = vm();
        let fmt = "%Y-%m-%d %H:%M:%S";
        for ts in [0, 1, 86_400, 951_782_400, 1_582_977_600, 1_700_000_000, 4_107_456_000] {
            let f = native_time_format_ts(&mut v, &[Object::Int(ts), s(fmt)]).unwrap();
            let back = native_time_parse(&mut v, &[f, s(fmt)]).unwrap();
            assert_eq!(fval(&back), ts as f64, "roundtrip failed for {}", ts);
        }
    }

    #[test]
    fn test_ymdhms_to_unix_roundtrip() {
        // 已知锚点
        assert_eq!(ymdhms_to_unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(ymdhms_to_unix(2023, 11, 14, 22, 13, 20), 1_700_000_000);
        assert_eq!(ymdhms_to_unix(2000, 2, 29, 0, 0, 0), 951_782_400);
        assert_eq!(ymdhms_to_unix(2024, 2, 29, 12, 0, 0), 1_709_208_000);
        assert_eq!(ymdhms_to_unix(2100, 2, 28, 23, 59, 59), 4_107_542_399);
        // unix_to_ymdhms 与 ymdhms_to_unix 互逆（含闰日 2000/2024 与非闰世纪年 2100-02-28）
        for (y, mo, d, h, mi, sec) in [
            (1970, 1, 1, 0, 0, 0),
            (2000, 2, 29, 0, 0, 0),
            (2024, 2, 29, 12, 34, 56),
            (2100, 2, 28, 23, 59, 59),
            (2023, 11, 14, 22, 13, 20),
        ] {
            let ts = ymdhms_to_unix(y, mo, d, h, mi, sec);
            assert_eq!(
                unix_to_ymdhms(ts as u64),
                (y, mo as u32, d as u32, h as u32, mi as u32, sec as u32),
                "roundtrip failed for {}-{:02}-{:02}",
                y,
                mo,
                d
            );
        }
    }

    #[test]
    fn test_integration_time_ext() {
        let src = r#"
import time
assert(time.iso(0) == "1970-01-01T00:00:00Z")
assert(time.iso(1700000000) == "2023-11-14T22:13:20Z")
p = time.date_parts(0)
assert(p["year"] == 1970 and p["weekday"] == 3, "date_parts epoch")
assert(time.date_parts(86400)["weekday"] == 4, "周五")
assert(time.format_ts(0, "%Y-%m-%d %H:%M:%S") == "1970-01-01 00:00:00")
assert(time.parse("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S") == 1700000000.0)
fmt = "%Y-%m-%d %H:%M:%S"
for t in [0.0, 951782400.0, 1700000000.0] {
    assert(time.parse(time.format_ts(t, fmt), fmt) == t, "往返一致")
}
assert(time.sleep_ms(0) == nil)
assert(time.monotonic() >= 0.0)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "time ext integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_parse_arity_violations() {
        // json.parse 2 参 / time.parse 1 参 → TypeError（MAX 升级后各自自校验）
        let r = run_source("import json\njson.parse(\"1\", \"2\")");
        let e = r.unwrap_err();
        assert!(e.contains("TypeError") && e.contains("parse"), "got: {}", e);
        let r = run_source("import time\ntime.parse(\"2023\")");
        let e = r.unwrap_err();
        assert!(e.contains("TypeError") && e.contains("parse"), "got: {}", e);
    }

    #[test]
    fn test_integration_json_time_parse_coexist() {
        // 同名冲突回归（§2.2）：json.parse（1 参）与 time.parse（2 参）同脚本并存
        let src = r#"
import json
import time
assert(json.parse("1") == 1)
assert(time.parse("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S") == 1700000000.0)
assert(json.parse("{\"a\": 2}")["a"] == 2)
assert(time.parse("2000-02-29", "%Y-%m-%d") == 951782400.0)
assert(json.parse("[3, 4]") == [3, 4])
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "json.parse × time.parse coexist failed: {:?}", r.err());
    }
}
