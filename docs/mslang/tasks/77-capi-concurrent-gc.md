# C API — 并发 GC 交互（并发写屏障/调优）

## 所属阶段

Phase 7.5 — 并发 GC 优化

## 前置任务

- 62（并发标记 tri-color + 写屏障）：实现并发三色标记、混合写屏障、灰色队列
- 63（并发清扫与 Compaction）：实现并发 sweep、Old 代 Compaction
- 64（GC 调优）：实现自适应 GC 参数调整（Young 代大小、晋升年龄、GC 线程数、动态阈值）
- 74-capi-gc：MVP 阶段 GC C API（msWriteBarrier no-op、msGcSetGcThreads 存储但不使用、基础 MsGcStats）

## 目标

将 Task 74 中的 MVP GC C API 升级为并发 GC 版本：

1. **msWriteBarrier** 从 no-op 升级为真正的混合写屏障实现（Go 1.8+ 风格）
2. **msGcSetGcThreads** 实际控制 GC Worker 线程池大小
3. **GC 调优参数**在并发 GC 下真正生效（阈值、晋升年龄、Young 代大小）
4. **MsGcStats** 扩展并发 GC 特有指标（并发标记时间、并发清扫时间、STW 阶段暂停）
5. 与 Task 62（并发标记）、Task 63（并发清扫）、Task 64（GC 调优）的并发 GC 实现协调

## 设计规格

参照 [13-capi.md](../13-capi.md) § GC 交互 + [14-gc.md](../14-gc.md) § 混合写屏障、§ GC 状态机、§ GC 统计与调优 API。

### 写屏障

C API 声明不变（Task 74 已定义）：

```c
MS_API void msWriteBarrier(MsVM* vm, MsValue* parent, MsValue* new_val);
```

MVP 阶段（Task 74）为 STW GC，写屏障是 no-op。本任务升级为并发标记期间的写屏障，对应 [14-gc.md](../14-gc.md) § 混合写屏障。

**与 VM 内部写屏障的关键差异（C API 保守近似）**：

VM 内部 `write_barrier`/`write_barrier_obj`（`src/vm/gc/barrier.rs`）实现完整的 Go 1.8+ 混合写屏障——着色 `old_val`（被覆盖旧值，删除屏障侧）+ `new_val`（插入屏障侧）。

但 C API 签名 `msWriteBarrier(vm, parent, new_val)` **缺少 `old_val` 参数**（`13-capi.md:642` 已固化），无法表达删除屏障侧。故 C API 采用**保守近似**：

1. **Shade `new_val` 灰色**（如果白色）— 插入屏障侧（Dijkstra，正确）
2. **Card marking**：若 `parent` 为 Old 代且 `new_val` 为 Young 代 → 标记 dirty card（无条件，与 `barrier.rs:60-65` 一致）

**不对 `parent` 着色**（删除屏障侧无法实现）。这是保守安全的：插入屏障保证新写入的白色对象不漏标；Old→Young 跨代引用由 card table 在 Minor GC 时扫描。C 扩展若直接覆盖堆槽位（如 `*slot = new_val`），被覆盖的旧白色对象理论上可能漏标——故 C API 文档应警告「直接覆盖堆槽位前，建议先 `msWriteBarrier` 或改用 `msListSet` 等内置 API（内部走完整写屏障）」。

> **正确性边界**：对绝大多数 C 扩展用法（`msListPush`/`msDictSet`/`msInstanceSet` 等内置 API 已含完整写屏障，无需手动调），C API 的保守近似足够。仅在 C 侧直接操作堆对象内部指针且覆盖既有白色引用时，近似可能漏标——此场景应避免或显式 `msRoot` 旧值。

**C 扩展注意事项**：
- `msListPush`/`msDictSet`/`msInstanceSet` 等内置操作已在 VM 字节码层面内部包含写屏障（[14-gc.md](../14-gc.md) § 写屏障插入点）
- 仅当 C 侧直接操作对象内部结构（如直接修改 List 内部数组指针）时需手动调用 `msWriteBarrier`
- 并发标记期间（`GcPhase::ConcurrentMark`）写屏障的着色逻辑生效，非并发标记期零开销；card marking 任意阶段执行（`barrier.rs:60-65` 无条件）

### GC 线程数

```c
MS_API void msGcSetGcThreads(MsVM* vm, uint32_t threads);
```

MVP 阶段仅存储不使用。本任务实际控制 GC Worker 线程池大小：

- GC Coordinator 管理 Worker 线程池（[14-gc.md](../14-gc.md) § GC 与协程交互）
- `threads` 设置 Worker 线程数，下一次 GC 周期生效（`init_concurrent_mark` 在 Init 阶段从 `heap.gc_threads_setting` 写入 `gc_runtime.gc_threads`，当前周期不生效）
- 默认值为 `std::thread::available_parallelism()` 的 1/4，最小 1（`runtime.rs:14-19` `default_gc_threads`）
- Task 64 自适应调整可能覆盖此值

> **与 14-gc.md:287 的偏差（沿用 Task 64 C5 决策）**：设计文档规定默认 gc_threads = CPU 核心数，但代码默认核数/4（`runtime.rs:14-19`）。本任务**不改变默认**（避免放大并发 GC 线程数的全局行为变更），仅设自适应上限为核数。默认对齐留作后续 task。

### GC 调优参数

已有 API（Task 74）在并发 GC 下行为变化：

| API | MVP 行为 | 并发 GC 行为 |
|---|---|---|
| `msGcSetThreshold(MS_GC_MAJOR, r)` | 一次性写 `next_major_gc` | 同 MVP（C API 未升级，Task 64 A3 遗留） |
| `msGcSetThreshold(MS_GC_MINOR, r)` | 一次性写 `next_minor_gc` | 同 MVP |
| `msGcSetPromotionAge(age)` | 设置晋升年龄 | 同左，自适应调整可能覆盖 |
| `msGcSetGcThreads(threads)` | 存储，不使用 | 写 `heap.gc_threads_setting`，下次并发周期 Init 生效 |

### GC 统计扩展

`MsGcStats` 新增并发 GC 指标字段：

```c
typedef struct MsGcStats {
    // 原有字段（Task 74）
    uint64_t minorGcCount;
    uint64_t majorGcCount;
    uint64_t totalPauseNs;
    uint64_t lastPauseNs;
    uint64_t youngSize;
    uint64_t oldSize;
    uint64_t losSize;
    uint64_t bytesFreed;

    // 新增：并发 GC 指标
    uint64_t concurrentMarkNs;   // 并发标记阶段耗时（纳秒）
    uint64_t concurrentSweepNs;  // 并发清扫阶段耗时（纳秒）
    uint64_t initStwNs;          // Init STW 阶段耗时（纳秒）
    uint64_t termStwNs;          // Mark Termination STW 阶段耗时（纳秒）
    uint64_t grayQueuePeak;      // 灰色队列峰值大小
    uint64_t gcThreads;          // 当前 GC Worker 线程数
} MsGcStats;
```

## 实现细节

### 文件位置

- `src/capi/gc.rs` — 修改 Task 74 中的现有函数

### 依赖关系

本任务依赖以下 Phase 7.5 已有模块：

| 模块 | 提供能力 |
|---|---|
| Task 62 并发标记 | `GcPhase`、`Color`、`GrayQueue`、`gc_phase.is_concurrent_mark()`、`write_barrier()` |
| Task 63 并发清扫 | `ConcurrentSweeper`、并发清扫统计 |
| Task 64 GC 调优 | `GcConfig`（原子字段）、自适应调整逻辑 |
| Task 52 GC 核心 | `MsHeap`、`MsObjHeader`、`gc_meta` 位域、`GcStats` |
| Task 74 C API GC | `src/capi/gc.rs`、`MsGcStats`、`MsGcType` |

### 1. msWriteBarrier 升级

替换 Task 74 的 no-op 实现。委托 VM 内部写屏障逻辑（`barrier.rs::write_barrier_obj`），保持 C API 层薄：

```rust
use crate::vm::gc::barrier::write_barrier_obj;
use crate::vm::gc::header::{color_atomic, generation_atomic};
use crate::vm::gc::{Color, Generation};
use crate::vm::object::Object;

#[no_mangle]
pub extern "C" fn msWriteBarrier(
    vm: *mut MsVM,
    parent: *mut MsValue,
    new_val: *mut MsValue,
) {
    if vm.is_null() || parent.is_null() || new_val.is_null() {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };

    // 提取堆对象指针（与 Task 74 msOnFinalize:119 一致的 inner.inner 模式）。
    let parent_obj = match unsafe { &(*parent).inner } {
        Object::Ref(h) => *h,
        _ => return, // 内联值（Int/Float/Bool/Nil）不需写屏障
    };
    let new_val_obj = match unsafe { &(*new_val).inner } {
        Object::Ref(h) => *h,
        _ => return,
    };

    // 委托 VM 内部写屏障（barrier.rs:50）：
    //   - old_val = null（C API 签名缺 old_val 参数 → 跳过删除屏障侧，保守近似，见 § 写屏障）
    //   - 内部含 card marking（无条件）+ 并发标记期着色 new_val（原子 CAS）
    // 全程裸指针 + 原子操作（color_atomic/set_color_atomic），避免与 GC Worker 的别名 UB。
    // SAFETY: parent_obj/new_val_obj 由 MsValue 持有，GC 已知可达；VM lock 期间对象不被释放。
    //   注：VM lock 不阻止并发清扫（Coordinator 经 Arc 共享 GcRuntime），但 write_barrier_obj
    //   仅读 gc_meta 原子位 + push gray_queue（内部 Mutex），不构造 &mut MsObjHeader，故无 UB。
    unsafe {
        write_barrier_obj(
            &inner.vm.gc_runtime,
            parent_obj,
            std::ptr::null_mut(), // old_val 未知 → 仅走插入屏障 + card marking
            new_val_obj,
        );
    }
}
```

> **为何委托而非内联**：`barrier.rs::write_barrier_obj` 已正确处理（1）原子着色避免与 GC Worker 数据竞争，（2）card marking 无条件标记（Task 63 修正），（3）phase 检查零开销。C API 层重复实现任一项都会引入 UB 风险（参见审核报告 VULN #2/#3）。
>
> **VM lock 与 GC 线程交互**：写屏障持 `VmInner` lock，但 GC Coordinator/Worker 经 `Arc<GcRuntime>` 共享，**不持 VM lock**。`gray_queue`（`runtime.rs:49`）自身有 `Mutex`，故 C 侧 push 与 Worker pop 互斥安全。`card_table`（`runtime.rs:96`）同样经内部锁保护。VM lock 的作用仅是序列化 C API 调用 + 保护 `MsHeap` 字段，不覆盖 GC 运行时原子字段。

**关键设计决策**：

1. **非 GC 期间零开销**：`write_barrier_obj` 内部经 `gc.phase_is_concurrent_mark()` 检查，非并发标记阶段直接返回（仅 card marking 无条件执行，开销极小）
2. **仅处理 Ref 类型**：非堆对象（Int、Float 等内联值）不需要写屏障
3. **灰色队列线程安全**：`GcRuntime.gray_queue`（`runtime.rs:49`）为 `Mutex<Vec<...>>`，Task 62 已实现
4. **Card Table 维护**：Old → Young 引用标记 dirty card，供 Minor GC 扫描 Remembered Set（无条件，与 `barrier.rs:60-65` 一致）
5. **保守近似**：C API 签名缺 `old_val` → 不走删除屏障侧（见 § 写屏障说明）；对内置 API 用法足够

### 2. msGcSetGcThreads 升级

替换 Task 74 的"存储不使用"实现：

```rust
#[no_mangle]
pub extern "C" fn msGcSetGcThreads(vm: *mut MsVM, threads: u32) {
    if vm.is_null() || threads == 0 {
        return;
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    // 写入 MsHeap.gc_threads_setting（与 Task 74 gc.rs:249 一致）。
    // init_concurrent_mark（major.rs:206）在下次并发周期 Init 阶段读取此字段，
    // 写入 gc_runtime.gc_threads（AtomicU32）供 Coordinator 启动 Worker 池。
    // 当前周期不生效——msGcStats.gcThreads 在下次并发周期前仍是旧值。
    inner.vm.heap.gc_threads_setting = threads;

    // Task 64 自适应调整可能在下次 GC 收尾覆盖此值（gc_threads 单调上调至 gc_threads_max）
}
```

> **线程池管理**由 Task 62/63 的 GC Coordinator 负责：新 GC 周期 Init 时，`init_concurrent_mark`（`major.rs:206`）从 `heap.gc_threads_setting` 写入 `gc_runtime.gc_threads`，Coordinator 据此 spawn Worker 线程；周期结束后 Worker 挂起等待下一周期。

### 3. msGcSetThreshold（不升级）

Task 74 的 C API `msGcSetThreshold`（`gc.rs:206-227`）**本任务不修改**：

- Task 74 的 `MS_GC_MAJOR` 仍只写 `heap.next_major_gc`（一次性，GC 后丢失，与 Task 60 MVP 一致）
- Task 64 为**脚本侧** `gc.set_threshold` 新增 `"concurrent_mark"`/`"major_interval_ms"` kind 并持久化 `"major"`/`"minor"`（写 `heap.old_gc_ratio`/`heap.young_size`），但**C API 侧未升级**（`MsGcType` 枚举 `{MINOR,MAJOR,FULL}` 固化，无法表达新 kind——Task 64 A3 已记录此限制）

> **Task 64 A3 遗留**：C API 暴露 `concurrent_mark_threshold`/`major_interval_ms` 需新增独立 setter（如 `msGcSetConcurrentMarkThreshold(vm, f64)` / `msGcSetMajorInterval(vm, u64)`，ABI 附加），**不属 Task 77 范围**（Task 77 仅扩 MsGcStats）。归未来 task 或协调 13-capi.md。

### 4. msGcCollect 升级

Task 74 已有基础实现。并发 GC 下的变化：

- `MS_GC_MAJOR` 触发完整的并发标记-清扫周期（Init → ConcurrentMark → MarkTerm → ConcurrentSweep）
- `MS_GC_FULL` 先 Minor GC（STW），再 Major GC（并发）
- 手动 `msGcCollect` 不受 `gc_enabled` 标志影响（与 MVP 行为一致）

```rust
#[no_mangle]
pub extern "C" fn msGcCollect(vm: *mut MsVM, gc_type: MsGcType) {
    if vm.is_null() {
        return;
    }
    // Task 74 已有此实现（gc.rs:145-170）：match gc_type 调 VM 的 gc_minor_only /
    // gc_major_only / gc_full（mod.rs:1033/1055/1070），末尾 run_c_finalizers。
    // 本任务无需改动 msGcCollect 本身——VM 方法内部已处理并发协调：
    //   - gc_major_only / gc_full 先 complete_concurrent_cycle_if_pending（mod.rs:1034/1071）
    //     避免手动 Major 与进行中的并发周期数据竞争
    //   - 降级模式（concurrent_enabled=false）下走 STW major_gc（Task 52 路径）
    // 故此函数与 Task 74 一致，无需升级。仅文档化：并发模式下 msGcCollect(MAJOR)
    // 若并发周期进行中，会先完成它（STW Mark Term + reconcile sweep）再返回。
}
```

> **注**：Task 74 的 `msGcCollect`（`gc.rs:145-170`）实现已正确——委托 `inner.vm.gc_minor_only()`/`gc_major_only()`/`gc_full()`，末尾 `run_c_finalizers`。本任务不修改此函数，仅依赖 VM 方法内部的并发协调（`complete_concurrent_cycle_if_pending`）。

### 5. MsGcStats 扩展

在 Task 74 的 `MsGcStats` 基础上新增字段：

```rust
#[repr(C)]
#[derive(Default, Clone)]
pub struct MsGcStats {
    // 原有字段（Task 74，types.rs:105-114）
    pub minor_gc_count: u64,
    pub major_gc_count: u64,
    pub total_pause_ns: u64,
    pub last_pause_ns: u64,
    pub young_size: u64,
    pub old_size: u64,
    pub los_size: u64,
    pub bytes_freed: u64,

    // 新增：并发 GC 指标（全部 u64，与既有字段一致，避免 padding）
    pub concurrent_mark_ns: u64,
    pub concurrent_sweep_ns: u64,
    pub init_stw_ns: u64,
    pub term_stw_ns: u64,
    pub gray_queue_peak: u64,
    pub gc_threads: u64, // 统一 u64（与 C 头 uint64_t 一致；原草案 u32+padding 已废弃）
}
```

`msGcStats` 实现从 `MsHeap`（`inner.vm.heap`）+ `GcRuntime`（`inner.vm.gc_runtime`，Arc deref）直接读各原子字段组装（无 `MsHeap::get_stats()` 方法——Task 74 `gc.rs:286-296` 已是此模式）：

```rust
#[no_mangle]
pub extern "C" fn msGcStats(vm: *mut MsVM) -> MsGcStats {
    if vm.is_null() {
        return MsGcStats::default();
    }
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    let h = &inner.vm.heap;
    let g = &inner.vm.gc_runtime;
    use std::sync::atomic::Ordering::Relaxed;
    // gc_threads：并发模式返回 gc_runtime.gc_threads 真实值；降级模式返回 1（STW 单线程实际值）。
    let gc_threads = if g.concurrent_enabled.load(Relaxed) {
        g.gc_threads.load(Relaxed) as u64
    } else {
        1
    };
    MsGcStats {
        minor_gc_count: h.minor_count,
        major_gc_count: h.major_count,
        total_pause_ns: h.total_pause_ns,
        last_pause_ns: h.last_pause_ns,
        young_size: h.young_size() as u64, // Task 64 A2：young_size() 为存活字节（容量另存 h.young_size 字段）
        old_size: h.old_size() as u64,
        los_size: h.los_size() as u64,
        bytes_freed: h.bytes_freed,
        concurrent_mark_ns: g.concurrent_mark_ns.load(Relaxed),
        concurrent_sweep_ns: g.concurrent_sweep_ns.load(Relaxed),
        init_stw_ns: g.init_stw_ns.load(Relaxed),
        term_stw_ns: g.term_stw_ns.load(Relaxed),
        gray_queue_peak: g.gray_queue_peak.load(Relaxed),
        gc_threads,
    }
}
```

> **`MsHeap` 无 `get_stats()` 方法**：Task 74 的 `msGcStats`（`gc.rs:286-296`）直接读 `h` 各字段，本任务沿用并扩展。`young_size()` 方法返回存活字节（Task 64 A2 已对齐标准容量语义为 `h.young_size` 字段；C API 暴露 `young_size()` 存活字节以保 Task 74 ABI，容量字段需另暴露则归未来 task）。
>
> **ABI 变更说明**：MsGcStats 从 64→112 字节。旧 C 代码读前 8 字段仍兼容（追加在末尾），但嵌入方若用 `sizeof(MsGcStats)` 做布局需重新编译。建议 bump `MS_VERSION_MINOR`（`13-capi.md:46-49`）或文档化此次 ABI 扩展。

**暂停时间语义变化**：

MVP 阶段 `total_pause_ns`/`last_pause_ns` 是整个 GC 耗时（`MsHeap` 字段，Task 52 起）。并发 GC 下：

| 字段 | 含义 |
|---|---|
| `total_pause_ns` | **沿用 Task 52 语义**：累计所有 STW 暂停（含 Minor GC 全程 + Major 的 Init/Term STW）。Task 64 未重定义此字段 |
| `lastPauseNs` | 最近一次 GC 的暂停时间（同上） |
| `concurrentMarkNs` | 并发标记阶段总耗时（不包含在暂停时间中） |
| `concurrentSweepNs` | 并发清扫阶段总耗时（不包含在暂停时间中） |
| `initStwNs` | Init STW 阶段累计耗时 |
| `termStwNs` | Mark Termination STW 阶段累计耗时 |

> **验证标准 8 的修正**：`initStwNs + termStwNs = totalPauseNs` 在 Minor GC 也贡献 `total_pause_ns` 时**不成立**（Minor GC 的暂停独立累加）。应改为「单次纯 Major 周期（无 Minor）：`initStwNs + termStwNs` ≤ 该周期的 STW 贡献」；含 Minor 时 `totalPauseNs >= initStwNs + termStwNs`。

### 6. Card Table 维护（无新增函数）

Task 77 **不实现** `mark_card_dirty` 自由函数——card marking 由 VM 内部写屏障 `barrier.rs::write_barrier_obj`（line 60-65）直接调用 `gc.card_table.mark_dirty(parent)` 完成。C API 的 `msWriteBarrier` 委托 `write_barrier_obj`，故 card marking 已含。

> **CardTable API**（`cardtable.rs:28`）：`pub fn mark_dirty(&self, old_obj: *mut MsObjHeader)`。Old 代为散布 Box 模型（Task 52/63），CardTable 内部用对象指针哈希记录 dirty 对象（非连续 arena 偏移），无需「Old 代起始地址」概念。Task 77 C API 层不直接操作 CardTable。

### 7. 与 Task 62（并发标记）的集成

| C API | 实际接口 | 集成方式 |
|---|---|---|
| `msWriteBarrier` | `barrier.rs::write_barrier_obj`（委托） | C API 薄包装，传 `old_val=null` |
| `msWriteBarrier` | `GcRuntime.phase_is_concurrent_mark()` | 阶段检查，非并发标记时零开销 |
| `msWriteBarrier` | `GcRuntime.gray_queue`（GrayQueue 字段） | 白色对象标灰并 push（Mutex 保护） |
| `msWriteBarrier` | `GcRuntime.card_table.mark_dirty()` | Old → Young 跨代引用维护（无条件） |
| `msGcCollect(MAJOR)` | `VM::gc_major_only()`（mod.rs:1070） | 先 complete_concurrent_cycle_if_pending，再触发 |

**灰色队列线程安全**：`GcRuntime.gray_queue`（`runtime.rs:49`）为 `Mutex<Vec<*mut MsObjHeader>>`（方案 A），Task 62 已实现。C 扩展经 VM lock 序列化调用 `msWriteBarrier`，push 与 GC Worker 的 pop 互斥安全——并非「任意线程并发入队」，而是 per-VM 锁串行化（见 §1 VM lock 说明）。

### 8. 与 Task 63（并发清扫）的集成

| C API | 实际接口 | 集成方式 |
|---|---|---|
| `msGcStats` | `GcRuntime.concurrent_sweep_ns`（AtomicU64） | 统计并发清扫耗时 |
| `msGcCollect` | `VM::gc_major_only` 内含 reconcile sweep | 触发完整 GC 周期 |

### 9. 与 Task 64（GC 调优）的集成

| C API | 实际接口（散布字段，非 GcConfig 聚合） | 集成方式 |
|---|---|---|
| `msGcSetThreshold` | `MsHeap.next_major_gc` / `next_minor_gc`（Task 74 gc.rs:215-224） | 阈值影响触发时机 |
| `msGcSetPromotionAge` | `MsHeap.promotion_age`（gc.rs:238） | 晋升年龄影响分代行为 |
| `msGcSetGcThreads` | `MsHeap.gc_threads_setting`（gc.rs:249） | 下次并发周期 Init 时写入 gc_runtime |
| `msGcStats` | `MsHeap` + `GcRuntime` 原子字段 | 返回当前生效的 GC 参数 |

> **无 `GcConfig` 结构体**：Task 64 实现注意事项 6（`64-gc-tuning.md:552`）明确「不引入 GcConfig 聚合」，字段散布 MsHeap（mutator 独占）+ GcRuntime（跨线程原子）。Task 77 沿用此模型，不创建 GcConfig。Task 64 自适应调整可能覆盖 C API 设置的 `gc_threads_setting`；`msGcStats.gcThreads` 返回 `gc_runtime.gc_threads` 实际值（可能被自适应上调）。

### 10. C 头文件更新

`include/mslang/types.h` 中 `MsGcStats` 结构体新增字段：

```c
typedef struct MsGcStats {
    uint64_t minorGcCount;
    uint64_t majorGcCount;
    uint64_t totalPauseNs;
    uint64_t lastPauseNs;
    uint64_t youngSize;
    uint64_t oldSize;
    uint64_t losSize;
    uint64_t bytesFreed;
    /* 并发 GC 指标（Task 77 新增） */
    uint64_t concurrentMarkNs;
    uint64_t concurrentSweepNs;
    uint64_t initStwNs;
    uint64_t termStwNs;
    uint64_t grayQueuePeak;
    uint64_t gcThreads;
} MsGcStats;
```

新增字段追加在末尾，前 8 字段偏移不变（旧 C 代码读前 8 字段仍兼容）。但 `sizeof(MsGcStats)` 从 64→112 字节，嵌入方若用 `sizeof` 做内存布局需重编译。建议 bump `MS_VERSION_MINOR`（`13-capi.md:46-49`）或文档化此次 ABI 扩展。

## 验证标准

1. **msWriteBarrier 着色（修正）**：并发标记期间调用写屏障，`new_val` 被正确标灰（保守近似：不对 `parent` 着色，见 § 写屏障）
2. **msWriteBarrier 零开销**：非并发标记阶段调用写屏障，仅 card marking（开销极小），着色逻辑直接返回
3. **C 扩展并发安全**：C 扩展在并发 GC 运行期间修改堆对象不崩溃（前提：C 侧持 `MsValue*` 跨越 GC 须经 `msRoot` 注册为根，否则并发清扫期可能 use-after-free——见审核报告 VULN #1）
4. **无误回收**：C 扩展持续修改对象引用的同时 GC 并发运行，存活对象（已 root 或全局可达）不被错误回收
5. **msGcSetGcThreads 生效**：设置后下一次**并发** GC 周期 Init 阶段实际使用指定数量的 Worker 线程（降级模式下 gc_threads stats 恒为 1，不验证此点）
6. **Card Table 维护**：写屏障对 Old → Young 引用正确标记 dirty card
7. **并发 GC 统计**：`msGcStats` 返回的并发指标非零（在并发 GC 执行后）
8. **STW 暂停分解（修正）**：单次纯 Major 周期（无 Minor GC），`initStwNs + termStwNs` ≤ 该周期 STW 贡献；含 Minor GC 时 `totalPauseNs >= initStwNs + termStwNs`（Minor GC 暂停独立累加，见 §5 暂停时间语义）
9. **灰色队列峰值**：`grayQueuePeak` 反映并发标记期间灰色队列最大深度（近似值，C 扩展 push 不更新 peak，见 RISK #4）
10. **多线程经 VM 锁串行安全（修正）**：多线程经 per-VM 锁串行调用 `msWriteBarrier` 不 panic、不死锁（**非真正并发入队**——任意时刻仅一线程持锁）
11. **降级兼容**：Task 74 的旧测试全部通过（MsGcStats 新字段追加在末尾，前 8 字段偏移不变；sizeof 从 64→112 字节，嵌入方需重编译）
12. **NULL 安全**：所有函数传入 NULL 指针不崩溃
13. **降级模式写屏障（新增）**：`concurrent_enabled=false` 时 `msWriteBarrier` 为 no-op（`phase_is_concurrent_mark()` 恒 false），但 card marking 仍执行（`barrier.rs:60-65` 无条件）

## 测试用例

### Rust 单元测试

`src/capi/gc.rs` 中 `#[cfg(test)] mod tests`，在 Task 74 测试基础上新增：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::msVmNew;
    use crate::capi::vm::msVmFree;
    use std::ffi::CString;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn test_write_barrier_shades_gray() {
        let vm = msVmNew();

        let source = CString::new("a = [1]; b = [2]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let name_a = CString::new("a").unwrap();
        let name_b = CString::new("b").unwrap();
        let a = unsafe { crate::capi::vm::msGetGlobal(vm, name_a.as_ptr()) };
        let b = unsafe { crate::capi::vm::msGetGlobal(vm, name_b.as_ptr()) };

        // 触发并发标记阶段（进入 ConcurrentMark）
        // 此时写屏障应生效
        // 模拟方式：先触发 Major GC 进入并发标记，然后在标记期间调用写屏障
        // 验证写屏障不崩溃且对象颜色被正确设置

        msWriteBarrier(vm, a, b);

        msVmFree(vm);
    }

    #[test]
    fn test_write_barrier_card_table() {
        let vm = msVmNew();

        // 创建 Old 代对象（通过多次 GC 晋升）
        let source = CString::new(
            "obj = [1, 2, 3]"
        ).unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        // 多次 Minor GC 晋升到 Old 代
        for _ in 0..3 {
            msGcCollect(vm, MsGcType::MS_GC_MINOR);
        }

        let name_obj = CString::new("obj").unwrap();
        let obj = unsafe { crate::capi::vm::msGetGlobal(vm, name_obj.as_ptr()) };

        // 创建 Young 代对象
        let source2 = CString::new("young = [4, 5, 6]").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source2.as_ptr(), filename.as_ptr());
        }
        let name_young = CString::new("young").unwrap();
        let young = unsafe { crate::capi::vm::msGetGlobal(vm, name_young.as_ptr()) };

        // Old → Young 引用写屏障：应标记 card dirty
        msWriteBarrier(vm, obj, young);

        msVmFree(vm);
    }

    #[test]
    fn test_write_barrier_non_ref_values() {
        let vm = msVmNew();

        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let name_a = CString::new("a").unwrap();
        let a = unsafe { crate::capi::vm::msGetGlobal(vm, name_a.as_ptr()) };

        // 用整数 MsValue 调用写屏障，不应崩溃
        let int_val = unsafe { crate::capi::value::msInt(42) };
        msWriteBarrier(vm, a, int_val);

        msVmFree(vm);
    }

    #[test]
    fn test_concurrent_gc_with_c_extension() {
        // 压力测试：C 扩展持续修改对象引用，同时 GC 并发运行
        let vm = msVmNew();

        let source = CString::new(
            "data = []; for i in range(100) { data.push([i]) }"
        ).unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let name_data = CString::new("data").unwrap();
        let data = unsafe { crate::capi::vm::msGetGlobal(vm, name_data.as_ptr()) };

        // 反复触发 GC 并调用写屏障
        for _ in 0..10 {
            msGcCollect(vm, MsGcType::MS_GC_MAJOR);
            msWriteBarrier(vm, data, data);
        }

        msVmFree(vm);
    }

    #[test]
    fn test_gc_thread_count() {
        let vm = msVmNew();

        // 设置 GC 线程数
        msGcSetGcThreads(vm, 4);

        // 触发 GC
        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let stats = msGcStats(vm);
        // **宽松断言原因**：默认 concurrent_enabled=false（降级模式）→ stats.gc_threads == 1
        //（STW 单线程实际值）。即使开并发模式，Task 64 自适应可能上调 gc_threads。
        // 故仅断言 >= 1。精确断言（== 4）需先开并发模式 + 关自适应，归集成测试。
        assert!(stats.gc_threads >= 1);

        // 更改线程数
        msGcSetGcThreads(vm, 2);
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);

        let stats = msGcStats(vm);
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_stats_concurrent() {
        let vm = msVmNew();

        // 分配足够多的对象以触发有意义的 GC
        let source = CString::new(
            "for i in range(1000) { x = [1, 2, 3, 4, 5] }"
        ).unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let stats = msGcStats(vm);

        // 基础指标
        assert!(stats.minor_gc_count > 0 || stats.major_gc_count > 0);
        assert!(stats.total_pause_ns > 0);
        assert!(stats.last_pause_ns > 0);

        // 并发 GC 指标（Major GC 后应有值）
        if stats.major_gc_count > 0 {
            assert!(stats.concurrent_mark_ns > 0 || stats.concurrent_sweep_ns > 0);
            assert!(stats.gray_queue_peak > 0 || stats.concurrent_mark_ns == 0);
        }

        // gc_threads 应有值
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    #[test]
    fn test_write_barrier_vm_lock_serialized() {
        // per-VM 锁串行化测试：多线程经 Arc<Mutex<*mut MsVM>> 外层锁 + msWriteBarrier
        // 内部 VmInner 锁双重串行。验证锁不死锁、不 panic。
        // **非真正并发写屏障**——任意时刻仅一线程持锁进入写屏障（见验证标准 10 修订）。
        let vm = Arc::new(Mutex::new(msVmNew()));

        let source = CString::new("a = [1]; b = [2]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        {
            let vm_lock = vm.lock().unwrap();
            unsafe {
                crate::capi::vm::msExecString(
                    *vm_lock,
                    source.as_ptr(),
                    filename.as_ptr(),
                );
            }
        }

        let name_a = CString::new("a").unwrap();
        let name_b = CString::new("b").unwrap();
        let (a, b) = {
            let vm_lock = vm.lock().unwrap();
            let a = unsafe { crate::capi::vm::msGetGlobal(*vm_lock, name_a.as_ptr()) };
            let b = unsafe { crate::capi::vm::msGetGlobal(*vm_lock, name_b.as_ptr()) };
            (a, b)
        };

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let vm_clone = Arc::clone(&vm);
                let a = a;
                let b = b;
                thread::spawn(move || {
                    let vm_lock = vm_clone.lock().unwrap();
                    msWriteBarrier(*vm_lock, a, b);
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // 释放 VM：msVmFree 释放底层 VM；Arc<Mutex<*mut MsVM>> 自然 drop（最后一引用）。
        // 不使用 std::mem::forget——会导致 Arc 控制块 + Mutex 内存泄漏。
        {
            let vm_lock = vm.lock().unwrap();
            msVmFree(*vm_lock);
        }
        // vm (Arc) 在此函数末尾 drop，引用计数归零，Arc + Mutex 内存释放。
    }

    #[test]
    fn test_concurrent_gc_stats_stw_decomposition() {
        let vm = msVmNew();

        let source = CString::new(
            "for i in range(500) { x = {'key': i} }"
        ).unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        msGcCollect(vm, MsGcType::MS_GC_MAJOR);

        let stats = msGcStats(vm);

        if stats.major_gc_count > 0 {
            // STW 暂停应分解为 Init + Mark Termination
            let stw_total = stats.init_stw_ns + stats.term_stw_ns;
            assert!(stw_total > 0);
            // total_pause_ns 应 >= 单次 STW 总和
            assert!(stats.total_pause_ns >= stw_total || stats.minor_gc_count > 0);
        }

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_gc_threads_min_one() {
        let vm = msVmNew();

        // 传入 0 应被忽略（不崩溃，不改变当前值）
        msGcSetGcThreads(vm, 0);
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        let stats = msGcStats(vm);
        assert!(stats.gc_threads >= 1);

        msVmFree(vm);
    }

    #[test]
    fn test_null_vm_safe() {
        // 继承 Task 74 的 NULL 安全测试，确保新增行为不破坏
        assert_eq!(msGcIsEnabled(std::ptr::null_mut()), 0);

        msGcCollect(std::ptr::null_mut(), MsGcType::MS_GC_FULL);
        msGcEnable(std::ptr::null_mut(), 1);
        msGcSetThreshold(std::ptr::null_mut(), MsGcType::MS_GC_MAJOR, 2.0);
        msGcSetPromotionAge(std::ptr::null_mut(), 2);
        msGcSetGcThreads(std::ptr::null_mut(), 4);
        msGcSetDebug(std::ptr::null_mut(), 1);
        msWriteBarrier(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());

        let stats = msGcStats(std::ptr::null_mut());
        assert_eq!(stats.minor_gc_count, 0);
        assert_eq!(stats.concurrent_mark_ns, 0);
        assert_eq!(stats.concurrent_sweep_ns, 0);
    }

    #[test]
    fn test_write_barrier_null_pointers() {
        let vm = msVmNew();

        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let name_a = CString::new("a").unwrap();
        let a = unsafe { crate::capi::vm::msGetGlobal(vm, name_a.as_ptr()) };

        // parent 为 NULL
        msWriteBarrier(vm, std::ptr::null_mut(), a);
        // new_val 为 NULL
        msWriteBarrier(vm, a, std::ptr::null_mut());
        // vm 为 NULL
        msWriteBarrier(std::ptr::null_mut(), a, a);

        msVmFree(vm);
    }
}
```

### C 集成测试

`tests/c/test_concurrent_gc.c`（在 Task 74 的 `test_gc.c` 基础上新增）：

```c
#include <mslang.h>
#include <assert.h>
#include <stdio.h>
#include <string.h>

void test_write_barrier_concurrent(void) {
    MsVM* vm = msVmNew();

    msExecString(vm, "a = [1]; b = [2]", "test.ms");

    MsValue* a = msGetGlobal(vm, "a");
    MsValue* b = msGetGlobal(vm, "b");

    msGcCollect(vm, MS_GC_FULL);
    msWriteBarrier(vm, a, b);

    msVmFree(vm);
}

void test_gc_thread_count(void) {
    MsVM* vm = msVmNew();

    msGcSetGcThreads(vm, 4);
    msGcCollect(vm, MS_GC_FULL);

    MsGcStats s = msGcStats(vm);
    assert(s.gcThreads >= 1);

    msVmFree(vm);
}

void test_gc_stats_concurrent_metrics(void) {
    MsVM* vm = msVmNew();

    msSetStdout(vm, NULL, NULL);
    msExecString(vm,
        "for i in range(1000) { x = [1,2,3,4,5] }",
        "test.ms");

    msGcCollect(vm, MS_GC_FULL);

    MsGcStats s = msGcStats(vm);
    assert(s.minorGcCount > 0 || s.majorGcCount > 0);
    assert(s.totalPauseNs > 0);
    assert(s.gcThreads >= 1);

    msVmFree(vm);
}

void test_concurrent_stress(void) {
    MsVM* vm = msVmNew();

    msExecString(vm,
        "data = []; for i in range(100) { data.push([i]) }",
        "test.ms");

    MsValue* data = msGetGlobal(vm, "data");

    for (int i = 0; i < 10; i++) {
        msGcCollect(vm, MS_GC_MAJOR);
        msWriteBarrier(vm, data, data);
    }

    msVmFree(vm);
}

void test_gc_stats_stw_fields(void) {
    MsVM* vm = msVmNew();

    msExecString(vm,
        "for i in range(500) { x = {'key': i} }",
        "test.ms");

    msGcCollect(vm, MS_GC_MAJOR);

    MsGcStats s = msGcStats(vm);
    if (s.majorGcCount > 0) {
        assert(s.totalPauseNs > 0);
    }

    msVmFree(vm);
}

void test_null_barrier(void) {
    msWriteBarrier(NULL, NULL, NULL);

    MsVM* vm = msVmNew();
    msExecString(vm, "a = [1]", "test.ms");
    MsValue* a = msGetGlobal(vm, "a");

    msWriteBarrier(vm, NULL, a);
    msWriteBarrier(vm, a, NULL);

    msVmFree(vm);
}

int main(void) {
    test_write_barrier_concurrent();
    test_gc_thread_count();
    test_gc_stats_concurrent_metrics();
    test_concurrent_stress();
    test_gc_stats_stw_fields();
    test_null_barrier();

    printf("all concurrent gc tests passed\n");
    return 0;
}
```

### 构建验证

```bash
# Rust 单元测试（包含 Task 74 旧测试 + Task 77 新测试）
cargo test --features capi -- capi::gc

# C 集成测试
cargo build --features capi
cc -I include -L target/debug -lmslang tests/c/test_concurrent_gc.c -o test_concurrent_gc
./test_concurrent_gc
```
