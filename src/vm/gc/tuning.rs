//! task 64：GC 自适应调优引擎 + 调优常量。
//!
//! 参照 [14-gc](../../../docs/mslang/14-gc.md) § 动态阈值（294-303 行）与
//! [64-gc-tuning](../../../docs/mslang/tasks/64-gc-tuning.md) §2。
//!
//! ## 设计要点
//!
//! `run_adaptive_tuning` 在 **mutator 线程**、每次 GC 周期收尾调用（minor_gc 末尾、
//! reconcile_sweep 末尾），独占 `&mut MsHeap`，无并发风险。`gc: &GcRuntime` 仅用于读
//! 并发统计（init_stw_ns + term_stw_ns），不在引擎内写 GcRuntime。
//!
//! 配置字段散布于 MsHeap（mutator 独占）与 GcRuntime（跨线程镜像），不强行聚合为
//! GcConfig 结构体（spec §实现注意事项 6）——避免大规模字段迁移风险。

use super::runtime::GcRuntime;
use super::MsHeap;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

// ===========================================================================
// 自适应调优常量（14-gc.md:294-303）
// ===========================================================================

/// 规则 1：Minor GC 频率 > 10 次/秒 → young_size 翻倍。
const MINOR_FREQ_HIGH_PER_SEC: u64 = 10;
/// 规则 2：Minor GC 频率 < 1 次/10秒 → young_size 减半。
const MINOR_FREQ_LOW_PER_10S: u64 = 1;
/// young_size 下限（与 set_threshold("minor") 下限一致）。
const YOUNG_SIZE_MIN: usize = 1024 * 1024; // 1MB
/// young_size 上限（与 set_threshold("minor") 上限一致）。
const YOUNG_SIZE_MAX: usize = 64 * 1024 * 1024;
/// 规则 5：晋升率 > 50% → promotion_age +1。
const PROMOTION_RATE_HIGH: f64 = 0.5;
/// 规则 4：Major STW > 10ms → gc_threads +1。
const MAJOR_STW_HIGH_NS: u64 = 10_000_000;
/// minor_gc_times 采样窗口上限（防极端频率下内存膨胀）。
const MINOR_TIMES_CAP: usize = 256;
/// minor_gc_times 保留窗口（秒→毫秒）。
const MINOR_TIMES_WINDOW_MS: u64 = 10_000;

/// C5：GC_THREADS_MAX = available_parallelism()（CPU 核数），与设计 14-gc.md:287 上限一致。
/// 注：代码当前「默认」gc_threads = 核数/4（runtime.rs default_gc_threads），与设计「默认=核数」
/// 不符——本任务不改变默认（避免放大并发 GC 线程数的全局行为变更），仅设自适应上限为核数。
pub fn gc_threads_max() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

/// task 64：单调时钟毫秒。minor_gc 收尾、reconcile_sweep 末尾、Coordinator 定时判定
/// 必须共用同一时钟源（spec B3），否则频率窗口与定时判定会错乱。
/// 用 SystemTime since UNIX_EPOCH（跨线程一致；Instant 无跨线程基准点）。
pub fn now_mono_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 丢弃 minor_gc_times 中超过 10s 窗口的旧条目；上限保留最近 MINOR_TIMES_CAP 条。
fn prune_minor_times(times: &mut VecDeque<u64>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(MINOR_TIMES_WINDOW_MS);
    // 窗口外（严格早于 cutoff）的丢弃。
    times.retain(|&t| t >= cutoff);
    // 上限裁剪：保留最近的若干条。
    while times.len() > MINOR_TIMES_CAP {
        times.pop_front();
    }
}

/// 计数窗口内（含端点，[now-window, now]）的条目数。
fn count_in_window(times: &VecDeque<u64>, now_ms: u64, window_ms: u64) -> u64 {
    let lower = now_ms.saturating_sub(window_ms);
    times.iter().filter(|&&t| t >= lower).count() as u64
}

/// task 64：在每次 GC 周期收尾调用（mutator 独占 `&mut MsHeap`）。
/// `gc` 用于读并发统计（init_stw_ns + term_stw_ns）；不在本函数内写 gc。
///
/// 规则（14-gc.md:294-303）：
/// 1. Minor 频率 > 10/秒 → young_size 翻倍（上限 64MB）。
/// 2. Minor 频率 < 1/10秒 → young_size 减半（下限 1MB）。
/// 3. Old 碎片率 > 30% → Compaction（Task 63：恒不触发，分支 unreachable）。
/// 4. Major STW > 10ms → gc_threads +1（上限 CPU 核数）。
/// 5. 晋升率 > 50% → promotion_age +1（上限 3；不自动下调，避免抖动）。
pub fn run_adaptive_tuning(heap: &mut MsHeap, gc: &GcRuntime, now_ms: u64) {
    if !heap.adaptive_enabled {
        return;
    }

    // 规则 1+2：Minor GC 频率 → young_size（14-gc.md:298-299）
    prune_minor_times(&mut heap.minor_gc_times, now_ms);
    let last_1s = count_in_window(&heap.minor_gc_times, now_ms, 1_000);
    let last_10s = count_in_window(&heap.minor_gc_times, now_ms, MINOR_TIMES_WINDOW_MS);
    if last_1s > MINOR_FREQ_HIGH_PER_SEC {
        heap.young_size = (heap.young_size * 2).min(YOUNG_SIZE_MAX);
    } else if last_10s < MINOR_FREQ_LOW_PER_10S {
        heap.young_size = (heap.young_size / 2).max(YOUNG_SIZE_MIN);
    }

    // 规则 5：晋升率 → promotion_age（14-gc.md:302）。
    // 仅在有存活数据时判定（首次 Minor GC 无数据）。不自动下调（设计未规定，避免抖动）。
    if heap.last_minor_survived > 0 {
        let rate = heap.last_minor_promoted as f64 / heap.last_minor_survived as f64;
        if rate > PROMOTION_RATE_HIGH && heap.promotion_age < 3 {
            heap.promotion_age += 1;
        }
    }

    // 规则 4：Major STW → gc_threads（14-gc.md:301）。仅 reconcile_sweep 后有实际意义，
    // minor_gc 收尾读到的是上次 Major 的 STW（无 Major 发生时为 0，不触发）。
    let stw_ns = gc.init_stw_ns.load(Ordering::Relaxed) + gc.term_stw_ns.load(Ordering::Relaxed);
    if stw_ns > MAJOR_STW_HIGH_NS {
        let cur = heap.gc_threads_setting;
        if cur < gc_threads_max() {
            heap.gc_threads_setting = cur + 1;
        }
    }

    // 规则 3：Old 碎片率 → Compaction（14-gc.md:300）。Task 63：Box 模型 fragmentation_ratio
    // 恒 0.0 → should_compact() 恒 false → 此分支 unreachable。保留判定点供未来 arena 迁移
    // 后激活（届时改调 compact_old，需 &mut VM，见 major.rs）。
    if heap.should_compact() {
        // unreachable：fragmentation_ratio() 恒 0.0。
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptive_young_size_doubles_on_high_freq() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.young_size = 4 * 1024 * 1024;
        // 模拟 1 秒内 12 次 Minor GC（> 10/秒 阈值）。
        for i in 0..12 {
            heap.minor_gc_times.push_back(i);
        }
        run_adaptive_tuning(&mut heap, &gc, 1000);
        assert_eq!(heap.young_size, 8 * 1024 * 1024); // 翻倍
    }

    #[test]
    fn test_adaptive_young_size_halves_on_low_freq() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.young_size = 4 * 1024 * 1024;
        // 10 秒窗口内 0 次 Minor GC（< 1/10秒）→ young_size 减半。
        run_adaptive_tuning(&mut heap, &gc, 10_001);
        assert_eq!(heap.young_size, 2 * 1024 * 1024);
        // 再次低频 → 再减半（下限 1MB）。
        run_adaptive_tuning(&mut heap, &gc, 10_001);
        assert_eq!(heap.young_size, 1 * 1024 * 1024);
    }

    #[test]
    fn test_adaptive_young_size_caps_at_max() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.young_size = 64 * 1024 * 1024;
        for i in 0..12 {
            heap.minor_gc_times.push_back(i);
        }
        run_adaptive_tuning(&mut heap, &gc, 1000);
        assert_eq!(heap.young_size, 64 * 1024 * 1024); // 已达上限
    }

    #[test]
    fn test_adaptive_disabled_skips() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = false;
        heap.young_size = 4 * 1024 * 1024;
        for i in 0..20 {
            heap.minor_gc_times.push_back(i);
        }
        run_adaptive_tuning(&mut heap, &gc, 1000);
        assert_eq!(heap.young_size, 4 * 1024 * 1024); // 不变
    }

    #[test]
    fn test_adaptive_promotion_age_caps_at_3() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.promotion_age = 3;
        heap.last_minor_survived = 100;
        heap.last_minor_promoted = 80; // 80% > 50%
        run_adaptive_tuning(&mut heap, &gc, 0);
        assert_eq!(heap.promotion_age, 3); // 已达上限，不变
    }

    #[test]
    fn test_adaptive_promotion_age_increases() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.promotion_age = 1;
        heap.last_minor_survived = 100;
        heap.last_minor_promoted = 60; // 60% > 50%
        run_adaptive_tuning(&mut heap, &gc, 0);
        assert_eq!(heap.promotion_age, 2);
    }

    #[test]
    fn test_adaptive_gc_threads_increases_on_high_stw() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        heap.gc_threads_setting = 1;
        gc.init_stw_ns.store(6_000_000, Ordering::Relaxed); // 6ms
        gc.term_stw_ns.store(6_000_000, Ordering::Relaxed); // 6ms，合计 12ms > 10ms
        run_adaptive_tuning(&mut heap, &gc, 0);
        assert!(heap.gc_threads_setting >= 2);
    }

    #[test]
    fn test_adaptive_gc_threads_caps_at_max() {
        let mut heap = MsHeap::new();
        let gc = GcRuntime::new();
        heap.adaptive_enabled = true;
        let mx = gc_threads_max();
        heap.gc_threads_setting = mx;
        gc.init_stw_ns.store(100_000_000, Ordering::Relaxed);
        gc.term_stw_ns.store(100_000_000, Ordering::Relaxed);
        run_adaptive_tuning(&mut heap, &gc, 0);
        assert_eq!(heap.gc_threads_setting, mx); // 不超过核数
    }

    #[test]
    fn test_prune_and_count_window() {
        let mut times = VecDeque::new();
        // 15 条时间戳：0..15。
        for i in 0..15u64 {
            times.push_back(i);
        }
        // now=20000：10s 窗口 [10000,20000] 内无条目（均 < 10000）。
        assert_eq!(count_in_window(&times, 20_000, 10_000), 0);
        // now=10：10s 窗口 [0,10] 含全部 15 条（含端点）。
        assert_eq!(count_in_window(&times, 10, 10_000), 15);
        // 1s 窗口 [0,10]（now-window=0）含全部 15 条。
        assert_eq!(count_in_window(&times, 10, 1_000), 15);

        // prune：now=10001，cutoff=1 → 丢弃 0，保留 1..14 = 14 条。
        prune_minor_times(&mut times, 10_001);
        assert_eq!(times.len(), 14);
        assert!(!times.contains(&0));
    }

    #[test]
    fn test_minor_times_cap() {
        let mut heap = MsHeap::new();
        heap.adaptive_enabled = true;
        // 塞入 300 条全在窗口内 → 高频判定后裁剪到 256。
        for i in 0..300 {
            heap.minor_gc_times.push_back(i);
        }
        let gc = GcRuntime::new();
        run_adaptive_tuning(&mut heap, &gc, 100);
        assert!(heap.minor_gc_times.len() <= MINOR_TIMES_CAP);
    }
}
