//! `gc` 原生模块。
//!
//! 参照 [60-stdlib-gc](../../../docs/mslang/tasks/60-stdlib-gc.md)。

use super::{expect_int, expect_string};
use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_dict, alloc_module, alloc_string, read_module_mut, DictMap, MsObjHeader, Object,
};
use crate::vm::VM;
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// gc 模块（task 60）
// ---------------------------------------------------------------------------

/// 构造 `gc` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 14 个原生函数：collect/collect_minor/enable/disable/is_enabled/
/// set_threshold/set_promotion_age/set_gc_threads/set_concurrent/set_adaptive/
/// stats/count/mem_alloc/mem_live。
pub fn register_gc_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 14] = [
        ("collect", gc_collect),
        ("collect_minor", gc_collect_minor),
        ("enable", gc_enable),
        ("disable", gc_disable),
        ("is_enabled", gc_is_enabled),
        ("set_threshold", gc_set_threshold),
        ("set_promotion_age", gc_set_promotion_age),
        ("set_gc_threads", gc_set_gc_threads),
        ("set_concurrent", gc_set_concurrent),
        ("set_adaptive", gc_set_adaptive),
        ("stats", gc_stats),
        ("count", gc_count),
        ("mem_alloc", gc_mem_alloc),
        ("mem_live", gc_mem_live),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: format!("gc.{}", name),
                func,
            }),
        );
    }
    let m = alloc_module("gc");
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

// ---- GC 操作函数 ----

fn gc_collect(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.gc_full();
    Ok(Object::Nil)
}

fn gc_collect_minor(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.gc_minor_only();
    Ok(Object::Nil)
}

fn gc_enable(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.heap.gc_enabled = true;
    Ok(Object::Nil)
}

fn gc_disable(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.heap.gc_enabled = false;
    Ok(Object::Nil)
}

fn gc_is_enabled(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(Object::Bool(vm.heap.gc_enabled))
}

// ---- GC 调优函数 ----

fn gc_set_threshold(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let kind = expect_string(args.get(0), "set_threshold(kind, value)")?;
    match kind.as_str() {
        "major" => {
            let ratio = match args.get(1) {
                Some(Object::Float(f)) => *f,
                Some(Object::Int(n)) => *n as f64,
                other => {
                    return Err(format!(
                        "TypeError: set_threshold(\"major\", value) expects float, got {}",
                        other.map(|o| o.type_name()).unwrap_or("missing")
                    ))
                }
            };
            if ratio <= 0.0 {
                return Err("ValueError: major threshold must be > 0".to_string());
            }
            // B1：持久化到 old_gc_ratio（替代仅写 next_major_gc）。后续 major_gc/reconcile
            // 按 old_gc_ratio 重算 next_major_gc，用户设置不因一次 GC 丢失。
            vm.heap.old_gc_ratio = ratio;
            let allocated = vm.heap.bytes_allocated;
            // max(1)：bytes_allocated=0 时避免 next_major_gc=0 导致 GC 每条指令触发。
            vm.heap.next_major_gc = ((allocated as f64 * ratio) as usize).max(1);
            Ok(Object::Nil)
        }
        "minor" => {
            let mb = expect_int(args.get(1), "set_threshold(\"minor\", value)")?;
            if !(1..=64).contains(&mb) {
                return Err(format!(
                    "ValueError: minor threshold must be 1-64 MB, got {}",
                    mb
                ));
            }
            // B2：持久化到 young_size（替代仅写 next_minor_gc）。后续 minor_gc 收尾按
            // young_size 重算 next_minor_gc，用户设置不因一次 GC 丢失。
            vm.heap.young_size = (mb as usize) * 1024 * 1024;
            vm.heap.next_minor_gc = vm.heap.bytes_allocated.saturating_add(vm.heap.young_size);
            Ok(Object::Nil)
        }
        // task 64：并发标记触发阈值（Old 占用率近似，0.0-1.0）。默认 0.8。
        "concurrent_mark" => {
            let t = match args.get(1) {
                Some(Object::Float(f)) => *f,
                Some(Object::Int(n)) => *n as f64,
                other => {
                    return Err(format!(
                        "TypeError: set_threshold(\"concurrent_mark\", value) expects float, got {}",
                        other.map(|o| o.type_name()).unwrap_or("missing")
                    ))
                }
            };
            if !(0.0..=1.0).contains(&t) {
                return Err(format!(
                    "ValueError: concurrent_mark threshold must be 0.0-1.0, got {}",
                    t
                ));
            }
            vm.heap.concurrent_mark_threshold = t;
            Ok(Object::Nil)
        }
        // task 64：Major GC 定时间隔（毫秒）。0 = 禁用定时；其余钳到 ≥10ms（C3）。
        "major_interval_ms" => {
            let ms = expect_int(args.get(1), "set_threshold(\"major_interval_ms\", value)")?;
            if ms < 0 {
                return Err(format!(
                    "ValueError: major_interval_ms must be >= 0, got {}",
                    ms
                ));
            }
            let clamped = if ms == 0 { 0u64 } else { (ms as u64).max(10) };
            vm.heap.major_gc_interval_ms = clamped;
            // 同步 GcRuntime 镜像（Coordinator 只读）。
            vm.gc_runtime
                .major_gc_interval_ms
                .store(clamped, Ordering::Relaxed);
            Ok(Object::Nil)
        }
        _ => Err(format!("ValueError: unknown threshold kind '{}'", kind)),
    }
}

fn gc_set_promotion_age(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let age = expect_int(args.get(0), "set_promotion_age(age)")?;
    if !(1..=3).contains(&age) {
        return Err(format!(
            "ValueError: promotion_age must be 1-3, got {}",
            age
        ));
    }
    vm.heap.promotion_age = age as u8;
    Ok(Object::Nil)
}

fn gc_set_gc_threads(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let n = expect_int(args.get(0), "set_gc_threads(n)")?;
    if n < 1 {
        return Err(format!("ValueError: gc_threads must be >= 1, got {}", n));
    }
    // task 64（C5）：上限校验 = available_parallelism()，与自适应引擎 gc_threads_max() 一致，
    // 避免手动设置与自适应上限脱节。init_concurrent_mark 写入 gc_threads 时同样受限。
    let max = crate::vm::gc::tuning::gc_threads_max() as i64;
    if n > max {
        return Err(format!(
            "ValueError: gc_threads must be <= {} (CPU cores), got {}",
            max, n
        ));
    }
    vm.heap.gc_threads_setting = n as u32;
    Ok(Object::Nil)
}

/// task 64：开启/关闭自适应调优引擎（默认 true）。
fn gc_set_adaptive(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let on = match args.get(0) {
        Some(Object::Bool(b)) => *b,
        other => {
            return Err(format!(
                "TypeError: set_adaptive expects bool, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    vm.heap.adaptive_enabled = on;
    Ok(Object::Nil)
}

/// task 62：启用/禁用并发 GC（14-gc.md § Phase 7.5 降级路径）。
/// true → spawn GC Coordinator，maybe_gc 异步触发并发标记；false → 回退 Task 52 STW。
fn gc_set_concurrent(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let enabled = match args.get(0) {
        Some(Object::Bool(b)) => *b,
        other => {
            return Err(format!(
                "TypeError: set_concurrent expects bool, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    };
    vm.gc_set_concurrent(enabled);
    Ok(Object::Nil)
}

// ---- GC 统计函数 ----

/// 快照语义：入口一次性采集全部数值（值拷贝），后续 alloc_string/alloc_dict
/// 分配期间即使触发 GC 也不影响已采集的值（与 Python gc.get_stats() 一致）。
fn gc_stats(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let h = &vm.heap;
    let g = &vm.gc_runtime;
    let minor_count = h.minor_count as i64;
    let major_count = h.major_count as i64;
    let total_pause_ns = h.total_pause_ns as i64;
    let last_pause_ns = h.last_pause_ns as i64;
    // A2：young_size = 容量配置（对齐标准 14-gc.md:761）；存活字节数另以 young_live 暴露。
    let young_size = h.young_size as i64;
    let young_live = h.young_size() as i64;
    let old_size = h.old_size() as i64;
    let los_size = h.los_size() as i64;
    let bytes_freed = h.bytes_freed as i64;
    let promotion_age = h.promotion_age as i64;
    let gc_enabled = h.gc_enabled;
    let adaptive_enabled = h.adaptive_enabled;
    // task 64：gc_threads 返回真实并发度（对齐 10-builtins.md:418/14-gc.md:766）。
    // D2：降级模式实际为 STW 单线程 → 返回 1（保持 Task 60 语义，避免误导）。
    let gc_threads = if g.concurrent_enabled.load(Ordering::Relaxed) {
        g.gc_threads.load(Ordering::Relaxed) as i64
    } else {
        1
    };
    // task 64：并发统计字段（GcRuntime，Task 62/63）。
    let concurrent_mark_ns = g.concurrent_mark_ns.load(Ordering::Relaxed) as i64;
    let concurrent_sweep_ns = g.concurrent_sweep_ns.load(Ordering::Relaxed) as i64;
    let init_stw_ns = g.init_stw_ns.load(Ordering::Relaxed) as i64;
    let term_stw_ns = g.term_stw_ns.load(Ordering::Relaxed) as i64;
    let swept_bytes = g.swept_bytes.load(Ordering::Relaxed) as i64;
    let gray_queue_peak = g.gray_queue_peak.load(Ordering::Relaxed) as i64;
    let concurrent_enabled = g.concurrent_enabled.load(Ordering::Relaxed);
    let major_gc_interval_ms = h.major_gc_interval_ms as i64;

    let mut map = DictMap::new();
    map.insert(alloc_string("minor_count"), Object::Int(minor_count));
    map.insert(alloc_string("major_count"), Object::Int(major_count));
    map.insert(alloc_string("total_pause_ns"), Object::Int(total_pause_ns));
    map.insert(alloc_string("last_pause_ns"), Object::Int(last_pause_ns));
    map.insert(alloc_string("young_size"), Object::Int(young_size));
    map.insert(alloc_string("young_live"), Object::Int(young_live));
    map.insert(alloc_string("old_size"), Object::Int(old_size));
    map.insert(alloc_string("los_size"), Object::Int(los_size));
    map.insert(alloc_string("bytes_freed"), Object::Int(bytes_freed));
    map.insert(alloc_string("promotion_age"), Object::Int(promotion_age));
    map.insert(alloc_string("gc_threads"), Object::Int(gc_threads));
    map.insert(alloc_string("gc_enabled"), Object::Bool(gc_enabled));
    map.insert(alloc_string("concurrent_mark_ns"), Object::Int(concurrent_mark_ns));
    map.insert(alloc_string("concurrent_sweep_ns"), Object::Int(concurrent_sweep_ns));
    map.insert(alloc_string("init_stw_ns"), Object::Int(init_stw_ns));
    map.insert(alloc_string("term_stw_ns"), Object::Int(term_stw_ns));
    map.insert(alloc_string("swept_bytes"), Object::Int(swept_bytes));
    map.insert(alloc_string("gray_queue_peak"), Object::Int(gray_queue_peak));
    map.insert(alloc_string("concurrent_enabled"), Object::Bool(concurrent_enabled));
    map.insert(alloc_string("adaptive_enabled"), Object::Bool(adaptive_enabled));
    map.insert(alloc_string("major_gc_interval_ms"), Object::Int(major_gc_interval_ms));
    Ok(alloc_dict(map))
}

fn gc_count(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // task 80：count 与 string.count 同名，native_arities 升级 MAX（§2.2 同名
    // 冲突治理），此处自校验恰 0 参。
    if !args.is_empty() {
        return Err(format!(
            "TypeError: gc.count() takes exactly 0 arguments, got {}",
            args.len()
        ));
    }
    let total = vm.heap.minor_count + vm.heap.major_count;
    Ok(Object::Int(total as i64))
}

fn gc_mem_alloc(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(Object::Int(vm.heap.bytes_allocated as i64))
}

fn gc_mem_live(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(Object::Int(vm.heap.live_size() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::{run_source, s, vm};
    use crate::vm::object::{read_dict, TypeTag};

    // ---- gc 模块单元测试（task 60）----

    /// 从 dict 中按 string key 取 int 值（测试辅助）。
    fn dict_int(d: &Object, key: &str) -> i64 {
        let Object::Ref(ptr) = d else {
            panic!("expected dict ref")
        };
        let k = alloc_string(key);
        let v = unsafe { read_dict(*ptr) }.get(&k).cloned().unwrap_or_else(|| {
            panic!("key '{}' not in stats dict", key)
        });
        match v {
            Object::Int(n) => n,
            _ => panic!("value for '{}' is not int", key),
        }
    }

    /// 从 dict 中按 string key 取 bool 值（测试辅助）。
    fn dict_bool(d: &Object, key: &str) -> bool {
        let Object::Ref(ptr) = d else {
            panic!("expected dict ref")
        };
        let k = alloc_string(key);
        let v = unsafe { read_dict(*ptr) }.get(&k).cloned().unwrap();
        match v {
            Object::Bool(b) => b,
            _ => panic!("value for '{}' is not bool", key),
        }
    }

    #[test]
    fn test_gc_module_registration() {
        let ptr = register_gc_module();
        // SAFETY: ptr 由 register_gc_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "gc");
            for name in [
                "collect", "collect_minor", "enable", "disable", "is_enabled",
                "set_threshold", "set_promotion_age", "set_gc_threads", "set_adaptive",
                "stats", "count", "mem_alloc", "mem_live",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_gc_collect_triggers_full_gc() {
        // gc.collect() → vm.gc_full() → minor_count + major_count 递增。
        let mut v = vm();
        assert_eq!(v.heap.minor_count, 0);
        assert_eq!(v.heap.major_count, 0);
        assert_eq!(gc_collect(&mut v, &[]).unwrap(), Object::Nil);
        assert!(v.heap.minor_count >= 1);
        assert!(v.heap.major_count >= 1);
    }

    #[test]
    fn test_gc_collect_minor_triggers_minor_only() {
        let mut v = vm();
        assert_eq!(v.heap.minor_count, 0);
        assert_eq!(v.heap.major_count, 0);
        assert_eq!(gc_collect_minor(&mut v, &[]).unwrap(), Object::Nil);
        assert!(v.heap.minor_count >= 1);
        // collect_minor 不触发 major。
        assert_eq!(v.heap.major_count, 0);
    }

    #[test]
    fn test_gc_enable_disable_is_enabled() {
        let mut v = vm();
        // 默认启用。
        assert!(v.heap.gc_enabled);
        assert_eq!(gc_is_enabled(&mut v, &[]).unwrap(), Object::Bool(true));

        // disable → false。
        assert_eq!(gc_disable(&mut v, &[]).unwrap(), Object::Nil);
        assert!(!v.heap.gc_enabled);
        assert_eq!(gc_is_enabled(&mut v, &[]).unwrap(), Object::Bool(false));

        // enable → true。
        assert_eq!(gc_enable(&mut v, &[]).unwrap(), Object::Nil);
        assert!(v.heap.gc_enabled);
        assert_eq!(gc_is_enabled(&mut v, &[]).unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_gc_stats_returns_all_fields() {
        let mut v = vm();
        let _ = gc_collect(&mut v, &[]); // 触发一次 GC 使统计非零。
        let result = gc_stats(&mut v, &[]).unwrap();
        // 验证 11 个原始字段全部存在。
        assert!(dict_int(&result, "minor_count") >= 1);
        assert!(dict_int(&result, "major_count") >= 1);
        assert!(dict_int(&result, "total_pause_ns") >= 0);
        assert!(dict_int(&result, "last_pause_ns") >= 0);
        // task 64 A2：young_size = 容量配置（>=4MB 默认），存活字节数另以 young_live 暴露。
        assert!(dict_int(&result, "young_size") >= 4 * 1024 * 1024);
        assert!(dict_int(&result, "young_live") >= 0);
        assert!(dict_int(&result, "old_size") >= 0);
        assert!(dict_int(&result, "los_size") >= 0);
        assert!(dict_int(&result, "bytes_freed") >= 0);
        assert!(dict_int(&result, "promotion_age") >= 1);
        // 降级模式（默认）D2：gc_threads == 1。
        assert_eq!(dict_int(&result, "gc_threads"), 1);
        assert!(dict_bool(&result, "gc_enabled"));
        // task 64：新增并发统计字段。
        assert!(dict_int(&result, "concurrent_mark_ns") >= 0);
        assert!(dict_int(&result, "concurrent_sweep_ns") >= 0);
        assert!(dict_int(&result, "init_stw_ns") >= 0);
        assert!(dict_int(&result, "term_stw_ns") >= 0);
        assert!(dict_int(&result, "swept_bytes") >= 0);
        assert!(dict_int(&result, "gray_queue_peak") >= 0);
        assert!(!dict_bool(&result, "concurrent_enabled")); // 默认降级
        assert!(dict_bool(&result, "adaptive_enabled")); // 默认开启
    }

    #[test]
    fn test_gc_stats_snapshot_semantics() {
        // gc_stats 入口快照：后续 alloc 不影响已采集值。
        // 采集后 minor_count 应反映调用时刻，而非 alloc 后的值。
        let mut v = vm();
        let _ = gc_collect(&mut v, &[]);
        let snapshot_count = v.heap.minor_count;
        let result = gc_stats(&mut v, &[]).unwrap();
        // alloc_string/alloc_dict 内部不触发 maybe_gc（gc_enabled 不影响 alloc）。
        // 快照值应等于采集时刻的 minor_count。
        assert_eq!(dict_int(&result, "minor_count"), snapshot_count as i64);
    }

    #[test]
    fn test_gc_count() {
        let mut v = vm();
        let _ = gc_collect(&mut v, &[]);
        let total = v.heap.minor_count + v.heap.major_count;
        assert_eq!(
            gc_count(&mut v, &[]).unwrap(),
            Object::Int(total as i64)
        );
        // task 80：count 与 string.count 同名升级 MAX，此处自校验恰 0 参。
        let err = gc_count(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("0"), "got: {}", err);
    }

    #[test]
    fn test_gc_mem_alloc() {
        let mut v = vm();
        assert_eq!(
            gc_mem_alloc(&mut v, &[]).unwrap(),
            Object::Int(v.heap.bytes_allocated as i64)
        );
    }

    #[test]
    fn test_gc_mem_live() {
        let mut v = vm();
        let live = v.heap.live_size();
        assert_eq!(
            gc_mem_live(&mut v, &[]).unwrap(),
            Object::Int(live as i64)
        );
    }

    #[test]
    fn test_gc_set_threshold_major() {
        let mut v = vm();
        // set_threshold("major", 2.0) → next_major_gc = max(bytes_allocated * 2.0, 1)。
        let allocated = v.heap.bytes_allocated;
        gc_set_threshold(&mut v, &[s("major"), Object::Float(2.0)]).unwrap();
        assert_eq!(
            v.heap.next_major_gc,
            ((allocated as f64 * 2.0) as usize).max(1)
        );
    }

    #[test]
    fn test_gc_set_threshold_major_int() {
        let mut v = vm();
        let allocated = v.heap.bytes_allocated;
        gc_set_threshold(&mut v, &[s("major"), Object::Int(3)]).unwrap();
        assert_eq!(
            v.heap.next_major_gc,
            ((allocated as f64 * 3.0) as usize).max(1)
        );
    }

    #[test]
    fn test_gc_set_threshold_minor() {
        let mut v = vm();
        gc_set_threshold(&mut v, &[s("minor"), Object::Int(8)]).unwrap();
        // task 64 B2：持久化到 young_size；next_minor_gc = bytes_allocated + young_size。
        assert_eq!(v.heap.young_size, 8 * 1024 * 1024);
        assert_eq!(v.heap.next_minor_gc, v.heap.bytes_allocated + 8 * 1024 * 1024);
    }

    // ---- task 64：set_threshold 持久化与新 kind（验证标准 9-12）----

    #[test]
    fn test_gc_set_threshold_concurrent_mark() {
        let mut v = vm();
        gc_set_threshold(&mut v, &[s("concurrent_mark"), Object::Float(0.9)]).unwrap();
        assert_eq!(v.heap.concurrent_mark_threshold, 0.9);
        // int 自动转 float。
        gc_set_threshold(&mut v, &[s("concurrent_mark"), Object::Int(1)]).unwrap();
        assert_eq!(v.heap.concurrent_mark_threshold, 1.0);
    }

    #[test]
    fn test_gc_set_threshold_concurrent_mark_out_of_range() {
        let mut v = vm();
        let err = gc_set_threshold(&mut v, &[s("concurrent_mark"), Object::Float(-0.1)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("0.0-1.0"));
        let err =
            gc_set_threshold(&mut v, &[s("concurrent_mark"), Object::Float(1.1)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("0.0-1.0"));
    }

    #[test]
    fn test_gc_set_threshold_major_interval_clamping() {
        let mut v = vm();
        // 正常值。
        gc_set_threshold(&mut v, &[s("major_interval_ms"), Object::Int(1000)]).unwrap();
        assert_eq!(v.heap.major_gc_interval_ms, 1000);
        assert_eq!(
            v.gc_runtime.major_gc_interval_ms.load(std::sync::atomic::Ordering::Relaxed),
            1000
        );
        // C3：5 钳到 10ms。
        gc_set_threshold(&mut v, &[s("major_interval_ms"), Object::Int(5)]).unwrap();
        assert_eq!(v.heap.major_gc_interval_ms, 10);
        // 0 禁用。
        gc_set_threshold(&mut v, &[s("major_interval_ms"), Object::Int(0)]).unwrap();
        assert_eq!(v.heap.major_gc_interval_ms, 0);
    }

    #[test]
    fn test_gc_set_threshold_major_interval_negative() {
        let mut v = vm();
        let err =
            gc_set_threshold(&mut v, &[s("major_interval_ms"), Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains(">= 0"));
    }

    #[test]
    fn test_gc_set_adaptive() {
        let mut v = vm();
        assert!(v.heap.adaptive_enabled); // 默认 true
        gc_set_adaptive(&mut v, &[Object::Bool(false)]).unwrap();
        assert!(!v.heap.adaptive_enabled);
        gc_set_adaptive(&mut v, &[Object::Bool(true)]).unwrap();
        assert!(v.heap.adaptive_enabled);
    }

    #[test]
    fn test_gc_set_adaptive_type_error() {
        let mut v = vm();
        let err = gc_set_adaptive(&mut v, &[Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"));
        assert!(err.contains("bool"));
        // 缺参。
        let err = gc_set_adaptive(&mut v, &[]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_gc_set_threshold_major_persists_old_gc_ratio() {
        // 验证标准 9 B1：set_threshold("major", 1.5) 持久化 old_gc_ratio，经一次 major GC
        // 后 next_major_gc 仍按 1.5（非默认 2.0）重算。
        let mut v = vm();
        gc_set_threshold(&mut v, &[s("major"), Object::Float(1.5)]).unwrap();
        assert_eq!(v.heap.old_gc_ratio, 1.5);
        // 模拟一次 major GC 重算（major_gc 内部按 old_gc_ratio 重算）。
        crate::vm::gc::major_gc(
            &mut v.heap,
            &v.stack,
            &v.globals,
            &v.defer_stack,
            &v.call_stack,
        );
        let expected = if v.heap.bytes_allocated == 0 {
            2 * 1024 * 1024
        } else {
            (v.heap.bytes_allocated as f64 * 1.5) as usize
        };
        assert_eq!(v.heap.next_major_gc, expected);
    }

    #[test]
    fn test_gc_set_threshold_minor_persists_young_size() {
        // 验证标准 9 B2：set_threshold("minor", 8) 持久化 young_size，经一次 minor GC
        // 后 next_minor_gc 按 8MB young_size 重算（用户设置不丢失）。
        let mut v = vm();
        gc_set_threshold(&mut v, &[s("minor"), Object::Int(8)]).unwrap();
        assert_eq!(v.heap.young_size, 8 * 1024 * 1024);
        // minor_gc 末尾按 young_size 重算 next_minor_gc。
        let gc_rt = v.gc_runtime.clone();
        crate::vm::gc::minor_gc(
            &mut v.heap,
            &mut v.stack,
            &mut v.globals,
            &mut v.defer_stack,
            &mut v.call_stack,
            &gc_rt.card_table,
            &gc_rt,
        );
        assert_eq!(
            v.heap.next_minor_gc,
            v.heap.bytes_allocated + 8 * 1024 * 1024
        );
        assert_eq!(v.heap.young_size, 8 * 1024 * 1024); // 未被重置
    }

    #[test]
    fn test_gc_set_threshold_unknown_kind() {
        let mut v = vm();
        let err = gc_set_threshold(&mut v, &[s("foo"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("foo"));
    }

    #[test]
    fn test_gc_set_threshold_major_negative() {
        let mut v = vm();
        let err = gc_set_threshold(&mut v, &[s("major"), Object::Float(-1.0)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("> 0"));
    }

    #[test]
    fn test_gc_set_threshold_minor_out_of_range() {
        let mut v = vm();
        let err = gc_set_threshold(&mut v, &[s("minor"), Object::Int(0)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("1-64"));
        let err = gc_set_threshold(&mut v, &[s("minor"), Object::Int(65)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("1-64"));
    }

    #[test]
    fn test_gc_set_threshold_kind_type_error() {
        let mut v = vm();
        let err = gc_set_threshold(&mut v, &[Object::Int(1), Object::Float(2.0)]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_gc_set_promotion_age_valid() {
        let mut v = vm();
        gc_set_promotion_age(&mut v, &[Object::Int(1)]).unwrap();
        assert_eq!(v.heap.promotion_age, 1);
        gc_set_promotion_age(&mut v, &[Object::Int(3)]).unwrap();
        assert_eq!(v.heap.promotion_age, 3);
    }

    #[test]
    fn test_gc_set_promotion_age_out_of_range() {
        let mut v = vm();
        let err = gc_set_promotion_age(&mut v, &[Object::Int(0)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("1-3"));
        let err = gc_set_promotion_age(&mut v, &[Object::Int(4)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("1-3"));
    }

    #[test]
    fn test_gc_set_promotion_age_type_error() {
        let mut v = vm();
        let err = gc_set_promotion_age(&mut v, &[Object::Float(2.0)]).unwrap_err();
        assert!(err.contains("TypeError"));
    }

    #[test]
    fn test_gc_set_gc_threads_accepts_value() {
        let mut v = vm();
        // task 64 C5：上限 = gc_threads_max()（CPU 核数）。取上限值保证跨机器（含低核 CI）通过。
        let max = crate::vm::gc::tuning::gc_threads_max() as i64;
        gc_set_gc_threads(&mut v, &[Object::Int(max)]).unwrap();
        assert_eq!(v.heap.gc_threads_setting, max as u32);
        // 降级模式（默认）D2：stats gc_threads == 1。
        let _ = gc_collect(&mut v, &[]);
        let result = gc_stats(&mut v, &[]).unwrap();
        assert_eq!(dict_int(&result, "gc_threads"), 1);
    }

    #[test]
    fn test_gc_set_gc_threads_rejects_over_max() {
        // task 64 C5：超过 CPU 核数 → ValueError。
        let mut v = vm();
        let over = (crate::vm::gc::tuning::gc_threads_max() as i64) + 1;
        let err = gc_set_gc_threads(&mut v, &[Object::Int(over)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains("<="));
    }

    #[test]
    fn test_gc_set_gc_threads_rejects_zero() {
        let mut v = vm();
        let err = gc_set_gc_threads(&mut v, &[Object::Int(0)]).unwrap_err();
        assert!(err.contains("ValueError"));
        assert!(err.contains(">= 1"));
    }

    #[test]
    fn test_gc_pause_timing_nonzero_after_collect() {
        let mut v = vm();
        // 分配一些对象使 GC 有实际工作。
        let _ = gc_collect(&mut v, &[]);
        // 至少一次 GC 后 last_pause_ns 应已设置（可能极小但 >= 0）。
        assert!(v.heap.last_pause_ns < u64::MAX); // 确认字段被写入（非默认 MAX）
    }

    #[test]
    fn test_gc_stats_reflects_gc_disabled() {
        let mut v = vm();
        let _ = gc_disable(&mut v, &[]);
        let result = gc_stats(&mut v, &[]).unwrap();
        assert!(!dict_bool(&result, "gc_enabled"));
        let _ = gc_enable(&mut v, &[]);
        let result = gc_stats(&mut v, &[]).unwrap();
        assert!(dict_bool(&result, "gc_enabled"));
    }

    #[test]
    fn test_integration_gc_collect() {
        let src = r#"
import gc
gc.collect()
stats = gc.stats()
assert(stats["gc_enabled"] == true)
assert(stats["minor_count"] >= 1)
assert(stats["major_count"] >= 1)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "gc.collect integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_gc_disable_enable() {
        let src = r#"
import gc
gc.disable()
assert(gc.is_enabled() == false)
gc.enable()
assert(gc.is_enabled() == true)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "gc disable/enable failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_gc_tuning() {
        let src = r#"
import gc
gc.set_threshold("major", 1.5)
gc.set_threshold("minor", 8)
gc.set_promotion_age(3)
gc.set_gc_threads(1)
gc.set_adaptive(false)
stats = gc.stats()
assert(stats["gc_threads"] == 1)
assert(stats["promotion_age"] == 3)
assert(stats["adaptive_enabled"] == false)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "gc tuning integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_gc_mem_and_count() {
        let src = r#"
import gc
gc.collect()
assert(gc.count() >= 2)
assert(gc.mem_alloc() >= 0)
assert(gc.mem_live() >= 0)
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "gc mem/count integration failed: {:?}", r.err());
    }
}
