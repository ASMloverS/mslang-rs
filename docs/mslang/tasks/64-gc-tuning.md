# GC 调优接口与自适应阈值

> **注意**：本任务落地 Phase 7.5.6「动态阈值与自适应」（[12-implementation-plan](../12-implementation-plan.md) §7.5.6，577-584 行；[14-gc](../14-gc.md) § 动态阈值，280-303 行）。Task 60 已实现 gc.ms 模块的**手动**调优 API（`set_threshold`/`set_promotion_age`/`set_gc_threads`），但其中 `gc_threads` 在 stats 中仍硬编码返回 1（`stdlib.rs:1656`，Task 60 MVP 存根），且 Task 62/63 引入的并发统计（`concurrent_mark_ns`/`concurrent_sweep_ns`/`init_stw_ns`/`term_stw_ns`/`swept_bytes`/`gray_queue_peak`，`runtime.rs:113-127`）**未暴露**给脚本。本任务补齐：(1) 自适应调优引擎；(2) 完整 gc 调优 API 与统计字段；(3) 并发标记阈值与定时间隔配置。
>
> **Compaction 触发不实装**：14-gc.md:300 规定「Old 碎片率 > 30% → Compaction」，但 Task 63 已确立 Old 代为散布 Box 模型（`fragmentation_ratio()` 恒 0.0，`should_compact()` 恒 false，`compact_old` 为 stub）。本任务仅在自适应引擎中**保留触发判定调用**（恒不触发），等价语义；实装 gated behind 未来的 Old 代 arena 迁移（见 Task 63 §8）。

## 所属阶段
Phase 7.5 — 并发 GC 优化（§7.5.6 动态阈值与自适应）

## 前置任务
- **63-concurrent-sweep-compaction**：`concurrent_sweep`/`reconcile_sweep`（`gc/major.rs`）、`GcRuntime` 并发统计（`runtime.rs:113-127`）、`fragmentation_ratio`/`should_compact`/`compact_old`（恒 0/false/stub）
- **62-concurrent-mark**：`GcRuntime.gc_threads`（`runtime.rs:110`）、`init_concurrent_mark`（`major.rs:206`，从 `heap.gc_threads_setting` 写入 gc_threads）、`GcWorkerPool`
- **60-stdlib-gc**：gc.ms 模块（`stdlib.rs:1497` `register_gc_module`）、`gc_stats`（`stdlib.rs:1643`）、`set_threshold`/`set_promotion_age`/`set_gc_threads`/`set_concurrent`
- **52-gc**：`MsHeap` 配置字段（`gc.rs:1021-1032`）、`minor_gc`/`major_gc`/`maybe_gc`、常量 `MAJOR_GC_RATIO=2.0`/`INITIAL_MINOR_THRESHOLD=1MB`/`INITIAL_MAJOR_THRESHOLD=2MB`（`gc.rs:70-74`）

## 目标

1. **自适应调优引擎**——在每次 GC 周期收尾调用 `run_adaptive_tuning`，按 14-gc.md:294-303 的启发式规则自动调整 `young_size`（Young 代阈值）、`promotion_age`、`gc_threads`（14-gc.md § 动态阈值）
2. **完整 gc 调优 API**——修正 `gc_stats` 中 `gc_threads` 的 MVP 存根（返回真实值）；新增并发统计字段；新增 `gc.set_adaptive(bool)` 开关；扩展 `set_threshold` 支持 `concurrent_mark` 阈值与 `major_interval_ms`
3. **GcConfig 集中化**——引入显式配置载体（替代散布于 MsHeap 字段 + 模块常量），含 `concurrent_mark_threshold`（默认 0.8）与 `major_gc_interval_ms`（默认 5000ms）
4. **Coordinator 定时触发**——`major_gc_interval_ms` 到期且 Old 代非空时由 Coordinator 主动发起并发 Major 周期（14-gc.md:700 `gc_coordinator_loop`），覆盖「分配缓慢但 Old 代持续缓慢增长」场景
5. **Compaction 触发判定**——自适应引擎调用 `heap.should_compact()`（恒 false，Task 63），保留触发点供未来 arena 迁移后激活

## 设计规格

参照 [14-gc](../14-gc.md)：
- **§ 动态阈值 / GcConfig**（282-292 行）：`promotion_age`(默认2,[1,3])、`old_gc_ratio`(默认2.0)、`young_size`(默认4MB)、`gc_threads`(默认CPU核数)、`concurrent_mark_threshold`(默认0.8)、`minor_gc_interval_ms`(默认0)、`major_gc_interval_ms`(默认5000)
- **§ 自适应调整规则**（294-303 行）：Minor 频率、young_size 倍增/减半、Old 碎片率、Major STW、晋升率五条规则
- **§ 触发机制 / Coordinator 定时**（695-710 行）：`gc_coordinator_loop` 按 `major_gc_interval_ms` 周期触发
- **§ GC 统计与调优 API**（729-769 行）：gc 模块函数与 stats dict
- **§ Phase 7.5 降级路径**（796-801 行）：`gc.set_concurrent(false)` 回退 STW

参照 [12-implementation-plan](../12-implementation-plan.md)：
- **§ 7.5.6 动态阈值与自适应**（577-584 行）：Young 代大小 / 晋升年龄 / GC 线程数自适应 + 完整 gc 调优 API；验证「长时间运行脚本 GC 开销 < 10%」

### 与现状差异总览

| 属性 | 现状（Task 60/62/63） | Task 64 目标 |
|---|---|---|
| `gc_threads`（stats） | 硬编码 1（`stdlib.rs:1656` MVP 存根） | 返回 `gc_runtime.gc_threads` 真实值 |
| 并发统计字段 | 存于 GcRuntime 但**未暴露**给脚本 | stats dict 新增 6 个并发字段 |
| young_size 默认 | `INITIAL_MINOR_THRESHOLD=1MB`（`gc.rs:72`） | 对齐设计 4MB（`gc.rs:72` 改 4MB） |
| 自适应调整 | 无（仅手动 set_*） | 每周期 `run_adaptive_tuning` 自动调整 |
| `concurrent_mark_threshold` | 不存在（Major 在 `bytes_allocated > next_major_gc` 时全量触发） | 新增 0.8 阈值，Old 占用率达此值即启动并发标记 |
| `major_gc_interval_ms` | 无（仅分配驱动） | Coordinator 定时器，默认 5s |
| `set_adaptive` | 不存在 | 新增，默认 true |
| Compaction 触发 | `should_compact()` 恒 false（Task 63） | 引擎调用判定，恒不触发（同 Task 63 决策） |

### 范围边界（本任务不覆盖）

| 内容 | 归属 | 说明 |
|---|---|---|
| Old 代 Compaction 实装 | 延后 / 未来 task | Task 63 §8 已定：Box 模型无碎片，`compact_old` 为 stub。本任务仅保留 `should_compact()` 调用点 |
| `minor_gc_interval_ms` 定时 | 不实装（设计默认 0） | 14-gc.md:289 默认 0 = 仅空间不足触发；Minor GC 仍由 `next_minor_gc` 阈值驱动。字段定义但不接入定时器 |
| Young 代半空间固定容量 | 不适用 | 当前 Young 为散布 Box（非 arena from/to-space），`young_size` 语义为「Minor GC 触发阈值（bytes_allocated 门槛）」而非固定区域容量 |
| GC 开销 < 10% 的硬性保证 | 验证目标（非断言） | 12-implementation-plan:584 为方向性目标；本任务以基准测试**记录**开销，不强制 CI 断言（依赖负载） |
| Task 77 C API 暴露新字段 | Task 77 | `msGcGetStats` 读取新字段由 Task 77 协调 |

### 已知限制（沿用 Task 60/63 现状）

- **VM 日常分配未接入 GC 堆**：`alloc_*`（`object.rs`）不经 `gc_alloc_*`，故 `young_size`/`old_size` 统计与自适应引擎基于 `gc_managed` + 少量 GC 堆对象，覆盖率有限（同 Task 63 已知限制）。全量接入为后续增量 task。
- **自适应启发式为经验值**：14-gc.md:294-303 的阈值（10次/秒、50% 晋升率等）来自设计文档，未经本仓库负载校准。Task 64 落地常量 + 基准记录，调参留作后续。

## 实现细节

### 文件组织

新增自适应调优模块，复用 Task 62/63 既有结构（`src/vm/gc/`）：

```
src/vm/gc/
├── tuning.rs       # task 64 新增：GcConfig + run_adaptive_tuning + 频率/晋升统计
├── runtime.rs      # 修改：新增 adaptive_enabled、major_gc_interval_ms、last_major_gc_ms
├── major.rs        # 修改：maybe_gc 检查 concurrent_mark_threshold；reconcile_sweep 末尾调 run_adaptive_tuning
└── ...
src/vm/gc.rs        # 修改：INITIAL_MINOR_THRESHOLD→4MB；minor_gc 末尾调 run_adaptive_tuning + 记 promoted/survived
src/vm/stdlib.rs    # 修改：gc_stats 补字段 + gc_threads 真实值；set_threshold 扩展；新增 gc_set_adaptive
src/vm/mod.rs       # 修改：maybe_gc / Coordinator 接入定时触发
```

### 1. GcConfig 与配置字段

参照 14-gc.md:282-292。配置散布于 `MsHeap`（mutator 独占读写）与 `GcRuntime`（跨线程读）。为避免跨线程 `&mut MsHeap`，**Coordinator 只读**这些字段（`Atomic` 或快照），自适应调整由 **mutator 在 reconcile/minor_gc 收尾时写入**（独占 `&mut heap`）。

```rust
// src/vm/gc.rs — MsHeap 新增字段（mutator 独占）
pub struct MsHeap {
    // ... 既有 next_minor_gc/next_major_gc/promotion_age/gc_threads_setting/minor_count/major_count ...
    /// task 64：Young 代目标大小（字节）。自适应引擎调整此值，minor_gc 收尾按此重置 next_minor_gc。
    pub young_size: usize,           // 默认 4MB（INITIAL_MINOR_THRESHOLD 改 4MB，见 §3）
    /// task 64：Old GC 触发比率（14-gc.md:285 old_gc_ratio，默认 2.0）。持久化：major_gc/reconcile
    /// 重算 next_major_gc = bytes_allocated * old_gc_ratio（替代 MAJOR_GC_RATIO 常量）。
    pub old_gc_ratio: f64,
    /// task 64：并发标记触发阈值（Old 占用率近似，0.0-1.0）。默认 0.8。语义见 §4 注。
    pub concurrent_mark_threshold: f64,
    /// task 64：Major GC 最大间隔（毫秒）。默认 5000。0 = 禁用定时（仅分配驱动）。
    /// 镜像至 GcRuntime.major_gc_interval_ms 供 Coordinator 只读（mutator 在 set_threshold 与 VM::new 同步）。
    pub major_gc_interval_ms: u64,
    /// task 64：自适应开关。默认 true。
    pub adaptive_enabled: bool,
    /// task 64：Minor GC 频率采样（最近完成时间戳，单调时钟毫秒）。自适应引擎读取。
    pub minor_gc_times: std::collections::VecDeque<u64>,
    /// task 64：上次 Minor GC 的存活/晋升字节（Copier 记录）。
    pub last_minor_survived: usize,
    pub last_minor_promoted: usize,
    /// task 64：上次 Major GC 完成的单调时钟（毫秒）。
    /// 镜像至 GcRuntime.last_major_gc_ms 供 Coordinator 定时判定（mutator 在 reconcile_sweep 末尾同步）。
    pub last_major_gc_ms: u64,
}
```

> **young_size vs next_minor_gc**：`young_size` 是**持久目标**（用户 `set_threshold("minor", mb)` 与自适应引擎写），`next_minor_gc` 是**当前触发门槛**。每次 minor_gc 收尾：`heap.next_minor_gc = heap.bytes_allocated + heap.young_size`（即「当前已分配 + 目标 Young 容量」）。这与 Task 52 的「`next_minor_gc` 每次按 `bytes_allocated` 重算」语义一致，仅把倍率来源从固定阈值改为 `young_size`。

### 2. 自适应调优引擎

参照 14-gc.md:294-303。`run_adaptive_tuning` 在 **mutator 线程**、每次 GC 周期收尾调用（minor_gc 末尾、reconcile_sweep 末尾），独占 `&mut MsHeap`，无并发风险。

```rust
// src/vm/gc/tuning.rs
use std::time::Instant;

/// task 64：自适应调优常量（14-gc.md:294-303）。
const MINOR_FREQ_HIGH_PER_SEC: u64 = 10;   // > 10 次/秒 → young_size 翻倍
const MINOR_FREQ_LOW_PER_10S: u64 = 1;     // < 1 次/10秒 → young_size 减半
const YOUNG_SIZE_MIN: usize = 1 * 1024 * 1024;   // 1MB（与 set_threshold("minor") 下限一致）
const YOUNG_SIZE_MAX: usize = 64 * 1024 * 1024;  // 64MB（与 set_threshold("minor") 上限一致）
const PROMOTION_RATE_HIGH: f64 = 0.5;      // 晋升率 > 50% → promotion_age +1
const MAJOR_STW_HIGH_NS: u64 = 10_000_000; // Major STW > 10ms → gc_threads +1
// C5：GC_THREADS_MAX = available_parallelism()（CPU 核数），与设计 14-gc.md:287 上限一致。
// 注：代码当前「默认」gc_threads = 核数/4（runtime.rs:15 default_gc_threads），与设计「默认=核数」不符——
// 本任务不改变默认（避免放大并发 GC 线程数的全局行为变更），仅设自适应上限为核数。默认对齐留作后续。
fn gc_threads_max() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)
}

/// task 64：在每次 GC 周期收尾调用（mutator 独占 &mut MsHeap）。
/// `gc` 用于读并发统计（init_stw_ns+term_stw_ns）；不写 gc（写由 mutator 在别处）。
pub fn run_adaptive_tuning(heap: &mut MsHeap, gc: &GcRuntime, now_ms: u64) {
    if !heap.adaptive_enabled { return; }

    // 规则 1+2：Minor GC 频率 → young_size（14-gc.md:298-299）
    prune_minor_times(&mut heap.minor_gc_times, now_ms);
    let last_1s = count_in_window(&heap.minor_gc_times, now_ms, 1_000);
    let last_10s = count_in_window(&heap.minor_gc_times, now_ms, 10_000);
    if last_1s > MINOR_FREQ_HIGH_PER_SEC {
        heap.young_size = (heap.young_size * 2).min(YOUNG_SIZE_MAX);
    } else if last_10s < MINOR_FREQ_LOW_PER_10S {
        heap.young_size = (heap.young_size / 2).max(YOUNG_SIZE_MIN);
    }

    // 规则 5：晋升率 → promotion_age（14-gc.md:302）
    // 仅在有存活数据时判定（首次 Minor GC 无数据）。
    if heap.last_minor_survived > 0 {
        let rate = heap.last_minor_promoted as f64 / heap.last_minor_survived as f64;
        if rate > PROMOTION_RATE_HIGH && heap.promotion_age < 3 {
            heap.promotion_age += 1;
        }
    }
    // 注：promotion_age 不自动下调（设计未规定下调规则；避免抖动）。

    // 规则 4：Major STW → gc_threads（14-gc.md:301）。仅 reconcile_sweep 后有意义。
    let stw_ns = gc.init_stw_ns.load(Ordering::Relaxed) + gc.term_stw_ns.load(Ordering::Relaxed);
    if stw_ns > MAJOR_STW_HIGH_NS {
        let cur = heap.gc_threads_setting;
        if cur < gc_threads_max() {
            heap.gc_threads_setting = cur + 1;
        }
    }

    // 规则 3：Old 碎片率 → Compaction（14-gc.md:300）。Task 63：恒不触发。
    if heap.should_compact() {
        // Task 63 §8：fragmentation_ratio() 恒 0.0 → 此分支 unreachable。
        compact_old_stub();
    }
}
```

> **`compact_old_stub`**：即 Task 63 `compact_old`（`major.rs`，`debug_assert!(false, ...)`）。本任务不重复定义，直接调用既有函数。
>
> **频率采样窗口**：`minor_gc_times` 为最近 10 秒的 Minor GC 完成时间戳（单调时钟 ms）。`prune_minor_times` 丢弃 10s 前的项；上限保留最近 256 条（防极端频率下内存膨胀）。`count_in_window` 线性计数窗口内条目。
>
> **`gc_threads` 下调**：设计未规定下调规则，本任务不下调（仅单调上调，受 `GC_THREADS_MAX` 封顶）。下调可经 `gc.set_gc_threads` 手动重置。

### 3. minor_gc 接入：promoted/survived 统计与自适应调用

参照 14-gc.md:317-320（存活复制 + 晋升）。`minor_gc`（`gc.rs:1279`）的 `Copier` 已复制存活对象并晋升。本任务在 Copier 增加计数器，minor_gc 出口写入 heap 并调用自适应引擎。

```rust
// src/vm/gc.rs — Copier 新增（task 64）
pub struct Copier<'a> {
    // ... 既有 map/from/to ...
    /// task 64：本轮存活字节（复制到 to-space 的对象 size 之和）。
    pub survived_bytes: usize,
    /// task 64：本轮晋升字节（age >= promotion_age → Old 的对象 size 之和）。
    pub promoted_bytes: usize,
}
// forward_slot 内：复制时 survived_bytes += size；若目标代为 Old 则 promoted_bytes += size。
```

```rust
// src/vm/gc.rs — minor_gc 出口（既有 next_minor_gc 重算处）追加：
let now_ms = now_mono_ms();  // B3：单调时钟毫秒（与 §6 Coordinator 同一时钟源）
heap.last_minor_survived = copier.survived_bytes;
heap.last_minor_promoted = copier.promoted_bytes;
heap.minor_gc_times.push_back(now_ms);  // 自适应频率采样
heap.next_minor_gc = heap.bytes_allocated + heap.young_size;  // task 64：按 young_size 重置
tuning::run_adaptive_tuning(heap, gc, now_ms);  // gc: 新增参数（与 card_table 同源传入）
```

> **`now_mono_ms` 时钟源（B3）**：`minor_gc` 收尾与 §6 Coordinator 必须共用同一单调时钟（建议 `std::time::SystemTime` since `UNIX_EPOCH` 的 ms，或缓存的 `Instant` 基点）。混用时钟源会导致频率窗口与定时判定错乱。
>
> **采样窗口的挂起偏差（C4，已知）**：`minor_gc_times` 以壁钟时间标记。若 VM 长时间阻塞于 I/O/sleep（无 Minor GC），窗口推进 → 判为低频 → young_size 减半，与实际内存负载无关。缓解：未来可改用「两次 Minor GC 间壁钟间隔序列」而非「窗口内计数」；本任务沿用窗口法（设计 14-gc.md:298-299 即此），在挂起密集场景接受偏差。
>
> **`minor_gc` 签名扩展**：Task 63 已为 minor_gc 增加 `card_table: &CardTable` 参数。本任务再增 `gc: &GcRuntime`（供自适应引擎读并发统计）。**波及所有 minor_gc 调用点**（`maybe_gc`/`gc_full`/`gc_minor_only`，`mod.rs`），须全量更新并 `cargo check`。
>
> **常量调整**：`INITIAL_MINOR_THRESHOLD`（`gc.rs:72`）由 1MB 改 4MB，对齐设计默认 young_size（14-gc.md:286）。同时 `MsHeap::new`（`gc.rs:1049`）初始化 `young_size: 4*1024*1024`。此改变降低默认 GC 频率（与设计一致），既有测试若硬编码「分配 N MB 触发 N 次 Minor」需核实（多数测试显式设 `next_minor_gc=0` 强制触发，不受影响）。

### 4. 并发标记阈值与 maybe_gc

参照 14-gc.md:288（`concurrent_mark_threshold` 默认 0.8）。`maybe_gc`（`mod.rs:1075`）在并发模式下，当 Old 代占用率达 `concurrent_mark_threshold` 时启动并发标记周期（而非等到 `bytes_allocated > next_major_gc` 全量阈值）。

```rust
// src/vm/mod.rs — maybe_gc（并发分支）判定扩展
let old_occupancy = if heap.old_capacity() > 0 {
    heap.old_size() as f64 / heap.old_capacity() as f64
} else { 0.0 };
// task 64：Old 占用率达 concurrent_mark_threshold → 提前启动并发标记。
// Old 为散布 Box 模型无固定 capacity：old_capacity() 定义为 next_major_gc（近似），
// 即 occupancy = old_size / next_major_gc。语义近似「Old 接近 Major 阈值的 80%」。
let should_start_major = heap.bytes_allocated >= heap.next_major_gc
    || old_occupancy >= heap.concurrent_mark_threshold;
```

> **`old_capacity()` 近似（C1，已知偏差）**：Old 代为散布 Box（Task 52/63），无连续 arena capacity。本任务定义 `old_capacity()` 返回 `next_major_gc`（Major 触发阈值）作为近似上界，使 `concurrent_mark_threshold` 语义为「Old 字节达 Major 阈值的 80% 时提前并发标记」。**注意**：因 `next_major_gc ≈ bytes_allocated * 2.0`（含 young+old+los），`old_size/next_major_gc` 并非「Old 代自身占用率」，真实触发点与 14-gc.md:288 的 Old-occupancy 语义有偏差（可能偏早/偏晚）。此为 Box 模型下的等价近似；arena 迁移后改为真实 Old capacity 即对齐设计。
>
> **降级模式不受影响**：`concurrent_enabled=false` 时 `maybe_gc` 走 Task 52 STW `major_gc` 路径（`mod.rs:1099`），不经并发标记阈值判定。

### 5. 完整 gc 调优 API（stdlib）

参照 14-gc.md:729-769。修正 MVP 存根 + 新增字段/函数。

**5.1 gc_stats 补字段 + gc_threads 真实值**（`stdlib.rs:1643`）：

```rust
fn gc_stats(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let h = &vm.heap;
    let g = &vm.gc_runtime;
    // 既有 11 字段快照不变 ...
    // task 64：gc_threads 返回真实并发度（对齐 10-builtins.md:418/14-gc.md:766 示例 gc_threads=核数）。
    // D2：降级模式实际为 STW 单线程 → 返回 1（保持 Task 60 语义，避免误导）。
    let gc_threads = if g.concurrent_enabled.load(Ordering::Relaxed) {
        g.gc_threads.load(Ordering::Relaxed) as i64
    } else {
        1
    };
    // task 64：新增并发统计字段（GcRuntime，Task 62/63）。
    let concurrent_mark_ns = g.concurrent_mark_ns.load(Ordering::Relaxed) as i64;
    let concurrent_sweep_ns = g.concurrent_sweep_ns.load(Ordering::Relaxed) as i64;
    let init_stw_ns = g.init_stw_ns.load(Ordering::Relaxed) as i64;
    let term_stw_ns = g.term_stw_ns.load(Ordering::Relaxed) as i64;
    let swept_bytes = g.swept_bytes.load(Ordering::Relaxed) as i64;
    let gray_queue_peak = g.gray_queue_peak.load(Ordering::Relaxed) as i64;
    let concurrent_enabled = g.concurrent_enabled.load(Ordering::Relaxed);
    let adaptive_enabled = h.adaptive_enabled;

    let mut map = DictMap::new();
    // ... 既有 11 字段插入 ...
    map.insert(alloc_string("concurrent_mark_ns"), Object::Int(concurrent_mark_ns));
    map.insert(alloc_string("concurrent_sweep_ns"), Object::Int(concurrent_sweep_ns));
    map.insert(alloc_string("init_stw_ns"), Object::Int(init_stw_ns));
    map.insert(alloc_string("term_stw_ns"), Object::Int(term_stw_ns));
    map.insert(alloc_string("swept_bytes"), Object::Int(swept_bytes));
    map.insert(alloc_string("gray_queue_peak"), Object::Int(gray_queue_peak));
    map.insert(alloc_string("concurrent_enabled"), Object::Bool(concurrent_enabled));
    map.insert(alloc_string("adaptive_enabled"), Object::Bool(adaptive_enabled));
    map.insert(alloc_string("young_size"), Object::Int(h.young_size as i64));  // A2：对齐标准=容量配置
    map.insert(alloc_string("young_live"), Object::Int(h.young_size() as i64)); // 新增（超出标准）：当前 Young 存活字节
    Ok(alloc_dict(map))
}
```

> **`young_size` 对齐标准（A2）**：标准 `10-builtins.md:413`/`14-gc.md:761` 的 stats dict 示例 `"young_size": 4194304`（= 4MB）与 GcConfig.young_size 默认容量（`14-gc.md:286`）一致——即标准本意 `young_size`=**容量配置**。Task 60 实现成「存活字节数」（`heap.young_size()`，`gc.rs:1076`）是既存偏差。本任务**修正**为容量语义（写入 `heap.young_size` 字段），非破坏性新变更。原存活字节数另以**新增字段** `young_live` 暴露（标准未定义，标注为超出标准）。**波及**：`stdlib.rs:4792` 单测（`young_size >= 0` 改读 `young_live >= 0`）、`60-stdlib-gc.md:42`、`14-gc.md:761`、`10-builtins.md:413` 的示例应统一标注 `young_size`=容量（实现侧无需改标准文档，但本任务实现注释须明示语义）。

**5.2 set_threshold 扩展**（`stdlib.rs:1564`）：

**既有 kind 行为修正（B1/B2）**：

| kind | 现状（Task 60） | Task 64 改动 | 写入字段 |
|---|---|---|---|
| `"major"` | 一次性 `next_minor_gc = allocated*ratio`（GC 后丢失） | 持久化 `old_gc_ratio`；`next_major_gc` 由其重算 | `heap.old_gc_ratio` |
| `"minor"` | 一次性 `next_minor_gc = mb*1MB` | 持久化 `young_size`；`next_minor_gc` 由其重算 | `heap.young_size` |

```rust
// set_threshold 既有分支改动（stdlib.rs:1564 起）
"major" => {
    let ratio /* float>0 校验不变 */;
    vm.heap.old_gc_ratio = ratio;                // B1：持久化（替代仅写 next_major_gc）
    vm.heap.next_major_gc = (vm.heap.bytes_allocated as f64 * ratio) as usize;
    Ok(Object::Nil)
}
"minor" => {
    let mb /* int 1..=64 校验不变 */;
    vm.heap.young_size = (mb as usize) * 1024 * 1024;  // B2：持久化（替代仅写 next_minor_gc）
    vm.heap.next_minor_gc = vm.heap.bytes_allocated + vm.heap.young_size;
    Ok(Object::Nil)
}
// 新增 kind：
"concurrent_mark" => {
    let t = expect_float(args.get(1), "set_threshold(\"concurrent_mark\", v)")?;
    if !(0.0..=1.0).contains(&t) {
        return Err(format!("ValueError: concurrent_mark threshold must be 0.0-1.0, got {}", t));
    }
    vm.heap.concurrent_mark_threshold = t;
    Ok(Object::Nil)
}
"major_interval_ms" => {
    let ms = expect_int(args.get(1), "set_threshold(\"major_interval_ms\", v)")?;
    // C3：0=禁用；其余钳到 ≥10ms 下限，防近连续 Major GC。
    let clamped = if ms == 0 { 0u64 } else { (ms as u64).max(10) };
    if ms < 0 { return Err(format!("ValueError: major_interval_ms must be >= 0, got {}", ms)); }
    vm.heap.major_gc_interval_ms = clamped;
    vm.gc_runtime.major_gc_interval_ms.store(clamped, Ordering::Relaxed);  // 同步 GcRuntime 镜像
    Ok(Object::Nil)
}
```

> **`major_interval_ms` 下限钳位（C3）**：用户传 1ms 会被钳到 10ms；传 0 禁用。校验顺序：先判 `<0` 抛错，再钳位。

**5.3 新增 gc.set_adaptive**（注册于 `register_gc_module`，arity 1）：

```rust
fn gc_set_adaptive(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let on = match args.get(0) {
        Some(Object::Bool(b)) => *b,
        other => return Err(format!(
            "TypeError: set_adaptive expects bool, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing"))),
    };
    vm.heap.adaptive_enabled = on;
    Ok(Object::Nil)
}
```

### 6. Coordinator 定时触发（major_gc_interval_ms）

参照 14-gc.md:695-710（`gc_coordinator_loop`）。当前 Coordinator（`major.rs:449` `spawn`）阻塞于 `rx.recv()`（`major.rs:456`），仅在 mutator 经 `init_concurrent_mark` → `trigger_major`（`major.rs:232/476`）异步唤醒。本任务改为 `recv_timeout`，超时后经「标志 + safepoint」让 mutator 发起周期（Init 需 `&mut VM`，不能在 Coordinator 完成）。

```rust
// src/vm/gc/runtime.rs — GcRuntime 新增（Coordinator 只读镜像；mutator 独占写入）
pub timer_major_pending: AtomicBool,      // Coordinator 请求 → mutator safepoint 发起
pub old_size: AtomicUsize,                // Old 字节镜像（mutator 在 minor_gc/reconcile 末尾 store）
pub major_gc_interval_ms: AtomicU64,      // 间隔镜像（mutator 在 set_threshold/VM::new 同步）
pub last_major_gc_ms: AtomicU64,          // 上次 Major 完成时间镜像（mutator 在 reconcile_sweep 末尾同步）

// src/vm/gc/major.rs — Coordinator 主循环改 recv_timeout（原 rx.recv() 阻塞，major.rs:456）
loop {
    let interval = rt.major_gc_interval_ms.load(Ordering::Relaxed);
    // interval==0（禁用定时）→ 等效永久阻塞，仅消息唤醒。
    let timeout = if interval == 0 { Duration::MAX } else { Duration::from_millis(interval) };
    match rx.recv_timeout(timeout) {
        Ok(GcTrigger::Major) => { if rt.phase_is_concurrent_mark() { run_major_cycle(rt); } }
        Ok(GcTrigger::Shutdown) => break,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // task 64：定时触发。仅 Idle + interval>0 + Old 非空 + 间隔到期才请求。
            let now = now_mono_ms();
            let last = rt.last_major_gc_ms.load(Ordering::Relaxed);
            if interval > 0
                && rt.phase() == GcPhase::Idle
                && rt.old_size.load(Ordering::Relaxed) > 0
                && now.saturating_sub(last) >= interval
            {
                rt.timer_major_pending.store(true, Ordering::Release);
            }
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => break, // channel 关闭（shutdown 已发）
    }
}
```

> **`recv_timeout` vs `recv`**：`recv_timeout` 返回 `Result<T, RecvTimeoutError>`（变体 `Timeout`/`Disconnected`），与 `recv()` 的 `Result` 不同——match 臂须用 `RecvTimeoutError`（已如上）。`recv_timeout` 在消息到达时优先返回消息（`Shutdown` 仍能唤醒），故 VM drop 的 `shutdown`（`major.rs:483`）语义不变。
>
> **mutator 侧**（`gc_safepoint_and_finalize`，`mod.rs:1118`）：在既有 `closure_pending`/`sweep_reconcile_pending`/`finalize_pending` 分支之前增加：
> ```rust
> if self.gc_runtime.timer_major_pending.swap(false, Ordering::Acquire) {
>     gc::init_concurrent_mark(self);  // 设 gc_managed + Init 扫描 + trigger_major
> }
> ```
> `init_concurrent_mark` 内部经 `trigger_major` 唤醒 Coordinator，Coordinator `run_major_cycle` 完成并发标记 + 清扫。`reconcile_sweep` 末尾更新 `gc.last_major_gc_ms` 与 `gc.old_size`（同步 MsHeap → GcRuntime 镜像）。
>
> **`now_mono_ms`**：单调时钟毫秒辅助（`std::time::SystemTime` since epoch 或缓存的 `Instant` 起点）。mutator 在 `minor_gc` 收尾推送 `minor_gc_times` 与 `reconcile_sweep` 写 `last_major_gc_ms` 须用同一时钟源。
>
> **降级模式不生效（C2）**：Coordinator 仅在 `concurrent_enabled=true` 时存在（`gc_set_concurrent`）。降级模式下 `set_threshold("major_interval_ms", N)` 无线程消费、**静默无效**；`gc_stats` 的 `major_interval_ms` 仍反映设置值。需定时 Major 的降级场景由 mutator `maybe_gc` 自检时间间隔兜底（本任务不实装，列为后续）。
>
> **极小间隔钳位（C3）**：`set_threshold("major_interval_ms")` 须加下限校验（0=禁用；否则 ≥ 10ms），避免用户设 1ms 触发近连续 Major GC。见 §5.2。

## VM 集成变更

- `maybe_gc`（`mod.rs:1075`）：并发分支增加 `concurrent_mark_threshold` 占用率判定（§4）。
- `gc_safepoint_and_finalize`（`mod.rs:1118`）：新增 `timer_major_pending` 分支（§6）。
- `minor_gc` 调用点（`mod.rs` 3 处）：追加 `gc: &GcRuntime` 参数（§3）。
- `reconcile_sweep`（`major.rs`）/ `minor_gc`（`gc.rs`）末尾：调 `run_adaptive_tuning`；同步 MsHeap→GcRuntime 镜像（`gc.old_size`、`gc.last_major_gc_ms`）；`reconcile_sweep`/`major_gc` 重算 `next_major_gc = bytes_allocated * heap.old_gc_ratio`（替代 `MAJOR_GC_RATIO` 常量读取，B1）。
- `VM::new`（`mod.rs`）：初始化 MsHeap 新字段（`young_size=4MB`、`old_gc_ratio=2.0`、`concurrent_mark_threshold=0.8`、`major_gc_interval_ms=5000`、`adaptive_enabled=true`、空 `minor_gc_times`）并同步对应 GcRuntime 镜像（`old_size=0`、`major_gc_interval_ms=5000`、`last_major_gc_ms=now`）。
- `register_gc_module`（`stdlib.rs:1497`）：函数表新增 `set_adaptive`；`native_arities` 注册 `set_adaptive=1`；`set_threshold` 的 `"major"`/`"minor"` 分支按 §5.2 改持久字段。

## 验证标准

### 自适应引擎
1. **young_size 上调**：高频 Minor GC（>10 次/秒，模拟：循环分配 + `next_minor_gc=0`）→ `stats()["young_size"]` 增大（上限 64MB）
2. **young_size 下调**：低频（<1 次/10秒，`time.sleep` 模拟）→ `young_size` 减半（下限 1MB）
3. **promotion_age 上调**：构造高晋升率（`last_minor_promoted/survived > 0.5`，经 gc_alloc_* 大对象存活至 Old）→ `promotion_age` 增至 3 上限
4. **gc_threads 上调**：Major STW > 10ms（大堆）→ `gc_threads_setting` 增（上限 CPU 核数）
5. **set_adaptive(false)**：引擎跳过所有调整，`young_size`/`promotion_age`/`gc_threads` 不变
6. **Compaction 不触发**：`should_compact()` 恒 false，`compact_old` 在本模型 unreachable

### API 与统计
7. **gc_threads 真实值**：并发模式 `stats()["gc_threads"]` == `gc_runtime.gc_threads`（非 1）；`set_gc_threads(4)` 后 `init_concurrent_mark` 写入 gc_threads，stats 反映；**降级模式（D2）** `stats()["gc_threads"]` == 1（STW 单线程实际值）
8. **新统计字段**：stats 含 `concurrent_mark_ns`/`concurrent_sweep_ns`/`init_stw_ns`/`term_stw_ns`/`swept_bytes`/`gray_queue_peak`/`concurrent_enabled`/`adaptive_enabled`/`young_size`（容量配置，对齐标准）/`young_live`（存活字节，新增超出标准）
9. **set_threshold 持久化（B1/B2）**：`set_threshold("major", 1.5)` 后经一次 major GC，`next_major_gc` 仍按 1.5（非默认 2.0）重算；`set_threshold("minor", 8)` 后经一次 minor GC，`next_minor_gc` 按 8MB young_size 重算（用户设置不丢失）
10. **set_threshold("concurrent_mark", 0.9)**：更新 `heap.concurrent_mark_threshold`；越界（<0 或 >1）抛 `ValueError`
11. **set_threshold("major_interval_ms")**：`1000` 更新字段；`5` 钳到 10ms（C3）；`0` 禁用；负值抛 `ValueError`
12. **set_adaptive**：非 bool 参数抛 `TypeError`

### 定时触发
13. **major_interval_ms 到期**：空转脚本（无分配）+ `set_threshold("major_interval_ms", 100)` + Old 代非空 → 100ms 后 Coordinator 置 `timer_major_pending`，mutator 下个 safepoint 发起周期，`major_count` 递增
14. **interval=0 禁用**：`set_threshold("major_interval_ms", 0)` → 不定时触发（仅分配驱动）
15. **降级模式定时无效（C2）**：`concurrent_enabled=false` + `set_threshold("major_interval_ms", 100)` → 无 Coordinator，`major_count` 不因定时递增（静默无效，已文档化）

### 回归
16. **降级模式不变**：`concurrent_enabled=false` 时 `maybe_gc` 走 STW `major_gc`，不经阈值/定时（自适应仍调，但 `gc_threads` 调整不影响 STW 单线程）
17. **Task 60 测试**：`stdlib.rs:4792` 单元测试 `young_size >= 0` 改读 `young_live >= 0` 后通过

## 测试用例

### Rust 单元测试（`src/vm/gc/tuning.rs`）

```rust
#[test]
fn test_adaptive_young_size_doubles_on_high_freq() {
    let mut heap = MsHeap::new();
    let gc = GcRuntime::new();
    heap.adaptive_enabled = true;
    heap.young_size = 4 * 1024 * 1024;
    // 模拟 1 秒内 12 次 Minor GC（> 10/秒 阈值）。
    for i in 0..12 { heap.minor_gc_times.push_back(i); }  // 同一秒内
    run_adaptive_tuning(&mut heap, &gc, 1000);
    assert_eq!(heap.young_size, 8 * 1024 * 1024);  // 翻倍
}

#[test]
fn test_adaptive_young_size_halves_on_low_freq() {
    let mut heap = MsHeap::new();
    let gc = GcRuntime::new();
    heap.adaptive_enabled = true;
    heap.young_size = 4 * 1024 * 1024;
    heap.minor_gc_times.push_back(0);  // 10 秒内仅 1 次（< 1/10秒 边界，1 次 == 阈值，应不调）
    run_adaptive_tuning(&mut heap, &gc, 10_001);
    // 0 次 < 1 → 减半
    heap.minor_gc_times.clear();
    run_adaptive_tuning(&mut heap, &gc, 10_001);
    assert_eq!(heap.young_size, 2 * 1024 * 1024);
}

#[test]
fn test_adaptive_disabled_skips() {
    let mut heap = MsHeap::new();
    let gc = GcRuntime::new();
    heap.adaptive_enabled = false;
    heap.young_size = 4 * 1024 * 1024;
    for i in 0..20 { heap.minor_gc_times.push_back(i); }
    run_adaptive_tuning(&mut heap, &gc, 1000);
    assert_eq!(heap.young_size, 4 * 1024 * 1024);  // 不变
}

#[test]
fn test_adaptive_promotion_age_caps_at_3() {
    let mut heap = MsHeap::new();
    let gc = GcRuntime::new();
    heap.adaptive_enabled = true;
    heap.promotion_age = 3;
    heap.last_minor_survived = 100;
    heap.last_minor_promoted = 80;  // 80% > 50%
    run_adaptive_tuning(&mut heap, &gc, 0);
    assert_eq!(heap.promotion_age, 3);  // 已达上限，不变
}
```

### 集成测试（`tests/gc_tuning_tests.rs`）

```rust
#[test]
fn test_stats_exposes_concurrent_fields() {
    let mut vm = VM::new();
    vm.gc_set_concurrent(true);
    // 分配触发至少一次并发 Major + reconcile
    for _ in 0..2000 { let _ = gc::gc_alloc_list(&mut vm.heap, &vm.gc_runtime, vec![Object::Int(1)]); }
    vm.maybe_gc();
    vm.complete_concurrent_cycle_if_pending();
    let stats = stdlib::gc_stats(&mut vm, &[]).unwrap();
    // 断言新字段存在且 >= 0（concurrent_mark_ns 在并发周期后 > 0）
    assert!(dict_int(&stats, "concurrent_mark_ns") >= 0);
    assert!(dict_int(&stats, "gc_threads") >= 1);  // 非硬编码 1 存根（>=1）
}
```

### mslang 级别 `tests/integration/test_gc_tuning.ms`

> **aspirational（冒烟）**：VM 字面量分配走 `alloc_*`（非 GC 堆），自适应引擎覆盖率有限（同 Task 63 已知限制）。此测试验证 API 不 panic + 统计字段存在。

```ms
import gc

gc.set_concurrent(true)
gc.set_threshold("concurrent_mark", 0.7)
gc.set_threshold("major_interval_ms", 100)
gc.set_adaptive(true)
gc.set_gc_threads(2)

for i in range(500) { x = [i] }
gc.collect()
s = gc.stats()
print(s["gc_threads"])
print(s["adaptive_enabled"])
print(s["concurrent_enabled"])
print("gc tuning ok")
```

预期输出（gc_threads 为运行时值，>=1）：
```
<gc_threads 值>
true
true
gc tuning ok
```

### 构建验证

```bash
cargo test -- gc::tuning
cargo test --test gc_tuning_tests
cargo test -- gc::tests          # 降级回归
cargo test -- stdlib::gc         # Task 60 gc 模块（young_live 改名后）
cargo run -- run tests/integration/test_gc_tuning.ms
```

## 实现注意事项

1. **顺序**：先改 `gc_stats`/`set_threshold`/`set_adaptive`（纯 stdlib，低风险）→ 再加 GcConfig 字段 + `run_adaptive_tuning`（不接入调用）→ 再接 minor_gc/reconcile 收尾调用 → 最后 Coordinator 定时。每步 `cargo check`。
2. **`young_size` 对齐标准（A2，非破坏性）**：标准 `10-builtins.md:413`/`14-gc.md:761` 示例 `young_size`=4MB = 容量配置语义，Task 60 实现成存活字节是偏差。本任务修正 stats `young_size`=容量，并以**新增** `young_live` 暴露存活字节（标准未定义）。须 grep `young_size` 在测试/文档的使用并区分：`stdlib.rs:4792` 单测（改读 `young_live`）；`60-stdlib-gc.md:42`、`14-gc.md:761`、`10-builtins.md:413` 示例值已是容量语义，无需改值，但实现注释须明示。
3. **`recv_timeout` 替代 `recv`**：改变 Coordinator 阻塞语义。VM drop 的 `shutdown`（`major.rs:483`）发 `Shutdown` 仍能唤醒 `recv_timeout`（消息优先）。须新增测试验证 drop 不挂起。Err 臂须用 `RecvTimeoutError`（非 `TryRecvError`）。
4. **`gc_threads_max()` + `set_gc_threads` 上限（C5）**：运行时取 `std::thread::available_parallelism()`。自适应上限用此；`set_gc_threads`（Task 60 仅校验 `>=1`）应同步加 `<= gc_threads_max()`，否则自适应与手动设置不一致。代码当前**默认** gc_threads=核数/4（`runtime.rs:15`），与设计默认=核数不符；本任务不改变默认，仅设上限。
5. **与 Task 74/77 的 C API 兼容（A3）**：脚本 `set_threshold` 用字符串 kind（可扩展），但 C API `msGcSetThreshold(MsVM*, MsGcType, double)`（`13-capi.md:662`）取固定枚举 `MsGcType={MINOR,MAJOR,FULL}`（Task 74 已固化），**无法表达** `concurrent_mark`/`major_interval_ms`。归属：(a) 扩展 `MsGcType` 枚举新增 `MS_GC_CONCURRENT_MARK`/`MS_GC_MAJOR_INTERVAL`（C ABI 破坏性，须 Task 74/77 协调），或 (b) 新增独立 setter `msGcSetConcurrentMarkThreshold(ms,f64)`/`msGcSetMajorInterval(ms,u64)`（推荐，ABI 附加）。Task 77 `msGcStats`（MsGcStats 结构，Task 74 已定义）需扩字段以暴露 8 个新统计——属 Task 77 C 结构变更。本任务仅保证 Rust 脚本 API 落地，C 暴露交 Task 77。
6. **不引入 `GcConfig` 结构体聚合**：设计 14-gc.md:282 展示独立 `GcConfig` struct，但当前字段散布 MsHeap（mutator）+ GcRuntime（跨线程）有正当理由（所有权/线程模型）。本任务保持散布 + 集中常量（`tuning.rs` 顶部），不强行聚合，避免大规模字段迁移风险。聚合可作未来重构。
7. **基准记录**：12-implementation-plan:584「GC 开销 < 10%」——本任务新增一个长跑基准（`benches/gc_overhead.rs` 或 `tests` 内 `#[ignore]` 测试），分配 10万对象、并发模式、记录 `total_pause_ns / 总运行时间`，**仅记录不强制阈值**。

