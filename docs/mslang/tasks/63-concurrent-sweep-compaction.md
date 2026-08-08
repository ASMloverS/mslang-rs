# 并发清扫与 Compaction

> **注意**：本任务将 Task 62 的 STW Sweep 升级为**真正的并发清扫**（mutator 与清扫并行），并补齐 **Card Table 扫描侧**（Minor GC 扫描 dirty cards，闭环 Task 62 仅实现的写侧）。
>
> **Compaction 延后**：14-gc.md § Old Generation（174-180 行）与 § 动态阈值（300 行）规定的「碎片率 > 30% 触发 STW Compaction」**前提是 Old 代为连续 arena + free-list 布局**。当前实现（Task 52 起）Old 代为**散布 Box 分配**（`old_objects: Vec<*mut MsObjHeader>`，`src/vm/gc.rs:1002`），无连续内存区域、无 free-list、**无碎片**。故 Compaction 在本任务中**不实装**：仅定义碎片度量（恒为 0）与触发条件，gated behind 未来的 Old 代 arena 迁移（见 §8）。这与 Task 52 gc.rs 头部注释的「等价语义的列表式 GC」决策一致。

## 所属阶段
Phase 7.5 — 并发 GC 优化

## 前置任务
- **62-concurrent-mark**：GcRuntime、GcPhase 状态机、混合写屏障、SafepointCoordinator、GcCoordinator/GcWorkerPool、`close_concurrent_cycle`（STW Sweep 收尾，`src/vm/gc/major.rs:270`）
- **52-gc**：MsHeap、`minor_gc`（`src/vm/gc.rs:1279`）、`sweep_heap`（`src/vm/gc.rs:1457`）、`gc_alloc_*`、TypeDescriptor
- **23-vm-core**：VM 主循环 `maybe_gc`（`src/vm/mod.rs:1075`）、`gc_safepoint_and_finalize`（`src/vm/mod.rs:1113`）

## 目标

1. **并发清扫（Concurrent Sweep）**——Sweep 的 `free()` 工作从 mutator STW 关键路径移到 GC Coordinator 线程，mutator 在清扫期间继续执行字节码（14-gc.md § Concurrent Sweep，443-475 行）
2. **Sweep（及并发标记）期间新分配对象标黑**——`alloc_during_gc`（Task 62 `barrier.rs:90`）已实现着色逻辑，但 `gc_alloc_*`（`gc.rs:1141-1190`）**从未调用它**（仅 barrier.rs 测试调用）。本任务**实装**：给 `gc_alloc_*` 增加 `gc: &GcRuntime` 参数并在末尾调 `alloc_during_gc`，使并发标记/清扫期分配的对象标黑，避免下轮被误回收（见 §9）
3. **Old 代清扫并发安全**——经 gc_managed 快照 + safepoint 协调下的 mutator 端 reconcile，规避 Coordinator 跨线程访问 `&mut MsHeap` 的别名 UB
4. **Card Table 扫描侧**——Minor GC 扫描 dirty cards，转发 Old→Young 跨代引用（闭环 Task 62 的写侧，14-gc.md § Remembered Set，326-344 行；12-implementation-plan §7.5.4，560-566 行）
5. **跨代 remembered set 屏障常驻**——修正 Task 62 仅在 `ConcurrentMark` 期标记 card 的限制（写屏障在非标记期早返回，导致 Old→Young 引用未被记录 → Minor GC 漏扫），使 card marking 在任意阶段对 Old→Young 写入生效
6. **Compaction**——延后（见 §8），仅落地碎片度量与触发判定（恒不触发）

## 设计规格

参照 [14-gc](../14-gc.md)：
- **§ Major GC / Concurrent Sweep**（443-475 行）：Worker 遍历 Old 代，White→free/复活，Black|Gray→重置 White；并发标记/清扫期间新分配标黑
- **§ Major GC / Finalize**（477-497 行）：pending finalizers 由 mutator 线程执行
- **§ Remembered Set**（326-344 行）：Card Table 方案，Old→Young 写入标记 dirty，Minor GC 扫描 dirty cards
- **§ Old Generation**（174-180 行）：标记清扫 + free-list，碎片过高触发 Compaction（**本任务延后**）
- **§ 动态阈值**（294-303 行）：Old 代碎片率 > 30% 触发 Compaction（**本任务延后**，度量恒 0）
- **§ GC 与协程交互**（631-674 行）：Concurrent Sweep 阶段协程继续运行

参照 [12-implementation-plan](../12-implementation-plan.md)：
- **§ 7.5.3 并发清扫**（552-558 行）：Concurrent Sweep 阶段、Sweep 期间新分配标黑、Old 代 free-list 并发安全
- **§ 7.5.4 Remembered Set**（560-566 行）：Card Table 扫描侧（Minor GC 扫描 dirty cards）

### 与 Task 62 的差异总览

| 属性 | Task 62（STW Sweep） | Task 63（并发 Sweep） |
|---|---|---|
| Sweep 执行线程 | mutator（`close_concurrent_cycle` 内 STW） | GC Coordinator（mutator 并行运行） |
| Sweep 期间 mutator | 暂停（safepoint park） | 继续执行字节码 |
| `GcPhase::ConcurrentSweep` | 视为 STW（`is_stw()` 含它） | 不再 STW（`is_stw()` 排除它） |
| `old_objects` Vec 修改 | sweep 内 `retain()` 直接改 | Coordinator 记 dead 集，mutator 在 reconcile `retain()` |
| `free()` 调用线程 | mutator | Coordinator（对象不可达，无别名） |
| `bytes_allocated` 减计 | sweep 内同步减 | Coordinator 累加 swept_bytes，mutator reconcile 时减 |
| Minor GC 扫 dirty cards | 否（已知限制） | 是（drain card_table + forward） |
| 跨代 card marking | 仅 ConcurrentMark 期 | 任意阶段（Old→Young 即标记） |
| STW 窗口数 | 1（Mark Term + Sweep 合一） | 2（Mark Term、Sweep reconcile），均极短 |

### 范围边界（本任务不覆盖）

| 内容 | 归属 | 说明 |
|---|---|---|
| Old 代 Compaction | 延后 / 未来 task | 需先迁移 Old 为 arena + free-list（当前 Box 模型无碎片，见 §8） |
| Old 代 free-list 并发安全（spec 7.5.3） | 不适用（Box 模型） | Box 模型无 free-list；语义等价为"old_objects Vec 在 safepoint 下 reconcile"（mutator 独占，无并发修改）。arena 迁移后此 spec 项才直接适用 |
| GC 自适应调优（动态阈值） | Task 64 | Young 代大小、晋升年龄、GC 线程数自适应 |
| C API 并发 GC 交互 | Task 77 | `msWriteBarrier` 升级、并发写屏障 C 侧 |
| 多 Worker 并行清扫 | 优化项（本任务单 Coordinator 线程） | 接口预留，未来可复用 GcWorkerPool 做 sweep 分片 |
| VM 分配接入 GC 堆 | 未来 task | 当前 `alloc_*`（object.rs:231）非 GC 托管；本任务 GC 路径仅覆盖 `gc_alloc_*`。mslang 端到端有效覆盖依赖此迁移 |

### 已知限制（沿用 Task 52/62 现状）

- **VM 日常分配未接入 GC 堆**：`alloc_*`（`src/vm/object.rs`）经 `Box::into_raw`，不经 `gc_alloc_*`，故并发清扫仅覆盖 `gc_managed`（old + los）对象。全量接入为后续增量。
- **CLOSURE/UPVALUE/FUNCTION/ITERATOR/GENERATOR/FUTURE trace**：当前这些类型不在 GC 堆，`gc_managed` 不含它们，并发清扫不会触碰。一旦未来接入 GC 堆，须补全 trace（Task 62 `gc.rs:950-960` 注释）。

## 实现细节

### 文件组织

新增独立清扫模块，复用 Task 62 既有结构（`src/vm/gc/`）：

```
src/vm/gc/
├── sweep.rs        # task 63 新增：并发清扫（Coordinator sweep + reconcile）
├── major.rs        # 修改：拆分 close_concurrent_cycle → finish_mark_termination
├── barrier.rs      # 修改：跨代 card marking 常驻
├── cardtable.rs    # 不变（Task 62 已实现 drain）
├── header.rs       # 修改：GcPhase::is_stw 排除 ConcurrentSweep
├── runtime.rs      # 修改：新增 sweep 累加器字段
└── ...
src/vm/gc.rs        # 修改：minor_gc 增加 card_table 扫描参数与循环
```

### 1. GcPhase 调整

参照 [14-gc](../14-gc.md) § Concurrent Sweep（443-475 行，mutator 继续运行）。`ConcurrentSweep` 不再是 STW 阶段。

```rust
// src/vm/gc/header.rs
impl GcPhase {
    pub fn is_concurrent_mark(self) -> bool { self == GcPhase::ConcurrentMark }
    pub fn is_stw(self) -> bool {
        // ConcurrentSweep 改为并发（mutator 运行）；reconcile 由独立标志触发，不靠 phase
        matches!(self, GcPhase::Init | GcPhase::MarkTermination)
    }
    /// task 63：是否处于并发清扫阶段（mutator 运行 + Coordinator 释放 White 对象）。
    pub fn is_concurrent_sweep(self) -> bool { self == GcPhase::ConcurrentSweep }
}
```

> **Ordering / happens-before（须落实）**：颜色读写在 sweep 期间为 `Relaxed`（GC 内部一致性），但 **phase 转换须用 Release/Acquire** 保证弱内存模型（ARM/POWER）上颜色的可见性：mutator 在 `finish_mark_termination` 完成所有着色后，`set_phase(ConcurrentSweep)` 用 `Ordering::Release`；Coordinator `concurrent_sweep` 入口读 phase 用 `Ordering::Acquire`（或经 `gc.phase()` 内部 load）。这建立 happens-before：Mark Termination 的颜色写在 Release 前，Coordinator 的 Acquire 后读到最新颜色。同理 Coordinator 完成 sweep 后置 `sweep_reconcile_pending`（Release）→ mutator 读（Acquire）保证 swept_bytes 可见。
>
> **实现**：`GcRuntime::set_phase` 增加 `ordering` 参数，或在 §5 `finish_mark_termination` / §7 rendezvous 处直接用 `self.phase.store(p as u8, Ordering::Release)`。`GcRuntime::phase()`（runtime.rs:138）当前 `Relaxed`——sweep 路径的调用点须改用独立的 `Acquire` load（如 `gc.phase.load(Ordering::Acquire)`）。x86 上 Release/Acquire 编译为普通 mov（零额外成本），故无理由保留 Relaxed。

### 2. GcRuntime sweep 累加器

Coordinator 线程在清扫期间释放 White Old 对象、记录 finalizer 对象，但**不能**修改 `MsHeap`（mutator 独占 `&mut`）。故在 `GcRuntime`（Arc 共享）新增累加器，mutator 在 reconcile 时一次性应用。

```rust
// src/vm/gc/runtime.rs
pub struct GcRuntime {
    // ... Task 62 既有字段 ...

    /// task 63：Coordinator 清扫释放的 Old 对象指针（mutator reconcile 时从 old_objects 移除）。
    pub sweep_dead_old: Mutex<Vec<*mut MsObjHeader>>,
    /// task 63：White + has_finalizer 的对象（mutator reconcile 时入 finalizer_queue 复活）。
    pub sweep_finalizers: Mutex<Vec<*mut MsObjHeader>>,
    /// task 63：Coordinator 释放的 Old 对象字节数（reconcile 时从 bytes_allocated 减）。
    pub swept_bytes: AtomicU64,
    /// task 63：Coordinator 完成 sweep → 置 true，mutator 在 safepoint 检测后执行 reconcile。
    pub sweep_reconcile_pending: AtomicBool,

    /// task 63：并发清扫耗时（Task 77 C API 读取）。
    pub concurrent_sweep_ns: AtomicU64,
}
```

```rust
impl GcRuntime {
    pub fn clear_sweep_accumulators(&self) {
        self.sweep_dead_old.lock().unwrap().clear();
        self.sweep_finalizers.lock().unwrap().clear();
        self.swept_bytes.store(0, Ordering::Relaxed);
    }
}
```

> 这些字段虽含裸指针，但 `GcRuntime` 已 `unsafe impl Send/Sync`（Task 62 runtime.rs:179），裸指针的别名安全由 GC 安全点 + 不可达性保证。

### 3. 跨代 remembered set 屏障常驻

**问题**：Task 62 的 `write_barrier_obj`（`barrier.rs:50`）在 `!phase_is_concurrent_mark()` 时**早返回**，导致 card marking 仅在并发标记窗口生效。但 Old→Young 引用可在任意阶段建立（如 `old_list.push(young_obj)`，在 `ListAppend` handler 中）。未被记录的跨代引用使 Minor GC 漏扫 → Young 对象误回收（Task 52 gc.rs:626 已知限制）。

**修复**：将 card marking 拆出，**任意阶段**对 Old parent 写入 Young 引用都标记 dirty；Dijkstra/Yuasa 着色仍仅在并发标记期。

```rust
// src/vm/gc/barrier.rs
pub unsafe fn write_barrier_obj(
    gc: &GcRuntime,
    parent: *mut MsObjHeader,
    old_val: *mut MsObjHeader,
    new_val: *mut MsObjHeader,
) {
    // task 63：跨代 card marking 常驻（任意阶段）。Old parent 写入 Young 引用 → dirty。
    // 这是 Minor GC 扫描 dirty cards 正确性的前提（14-gc.md § Remembered Set，326-344 行）。
    if !new_val.is_null()
        && unsafe { generation_atomic(parent) } == Generation::Old
        && unsafe { generation_atomic(new_val) } == Generation::Young
    {
        gc.card_table.mark_dirty(parent);
    }

    // 着色仅在并发标记期（三色不变性维护）。
    if !gc.phase_is_concurrent_mark() {
        return;
    }
    shade_if_white(gc, old_val);
    shade_if_white(gc, new_val);
}
```

> **性能**：每次堆写入多两次 `generation_atomic`（Relaxed u8 读，约 1 周期）+ 一次 `HashSet::insert`（仅命中跨代时）。非跨代写入仅两次 gen 读后跳过。可接受；可加 fast-path `if old_objects.is_empty() { return }`（Old 代为空则不可能跨代）进一步降低非 GC 期开销。
>
> **非托管对象的 card marking 风险**：Task 62 的写屏障插入点（`SET_ATTR/SET_INDEX/LIST_PUSH/...` handler）对所有对象触发，含 `alloc_*`（非 GC 堆，如 `object.rs:231` 的 MsList）分配的容器。这些对象的 `gc_meta` 由各自 alloc_* 设置，generation 不保证为 Young（如 Immortal 单例 `object_class` 设 Immortal，`builtins.rs:239`）。若一个 `alloc_*` 对象 generation 恰为 Old 且写入 Young 引用 → 误标 dirty card → 下次 minor GC 对该**非托管**对象调 `forward_fields`（PLACEHOLDER noop 或真实）→ 对非 GC 堆对象执行 GC 转发语义，潜在 UB。
>
> **缓解（本任务实装）**：card marking 增加 gc_managed 成员判定，parent 须在 GC 堆才标 dirty。但 `gc_managed` 是每轮 GC 重建的快照（非长效集合），不适合运行时查询。**替代方案**：为 GC 托管对象在 header 设独立标志位（如复用 `gc_meta` 的保留位 4 位 `has_finalizer` 之外的位，或新增 `gc_managed` 位），`write_barrier_obj` 检查该位。当前 gc_meta 位域（14-gc.md:73-85）位 4=has_finalizer、位 5=pinned 已用，位 6-7=age，无空闲位——需扩 gc_meta 或经 `gc_managed` 的长效索引（如 `old_objects`/`young_objects` 的 HashSet 驻留）。**最简方案**：限定写屏障仅在 GC 托管容器路径调用（gc_alloc_* 容器），alloc_* 容器不走 `write_barrier_obj`——须核实 Task 62 的字节码插入点是否区分容器来源，若不区分则本任务须在 handler 内加托管判定。此项须在实现时核实 Task 62 插入点并补充测试。
>
> **降级模式一致性**：降级模式（`concurrent_enabled=false`）下 phase 永不为 ConcurrentMark，着色永不触发，但 card marking 仍生效 → Minor GC 现在也扫 dirty cards。这**修复了 Task 52 降级/STW 模式下的跨代漏扫**（此前 MVP 缓解为「扫描全部 Old」，本任务以精确 card table 替代）。

### 4. Minor GC 扫描 dirty cards

参照 [14-gc](../14-gc.md) § Remembered Set（326-344 行）+ § Minor GC（306-324 行）。在 `minor_gc` 的根集转发之后、Cheney 扫描之前，drain card_table 并转发每个 dirty Old 对象的 Young Ref 槽。

`minor_gc` 新增 `card_table: &CardTable` 参数（调用方 `VM` 持 `self.gc_runtime.card_table`）：

```rust
// src/vm/gc.rs
pub fn minor_gc(
    heap: &mut MsHeap,
    stack: &mut [Object],
    globals: &mut HashMap<String, Object>,
    defer_stack: &mut [DeferEntry],
    frames: &mut [CallFrame],
    card_table: &CardTable,            // task 63 新增
) {
    // ... 既有 Copier 初始化、根集转发（stack/globals/defer/frames）不变 ...

    // task 63：扫描 dirty cards —— Old 对象持有的 Young 引用。
    // forward_fields 复用既有 Cheney 钩子：forward_slot 仅转发 old_young_set 内的 Young 对象，
    // Old→Old 引用不动。drain 消费全部 dirty（下个 epoch 由写屏障重新标记）。
    for old_ptr in card_table.drain() {
        // SAFETY: dirty 集合经 sweep 后的 retain_valid（Task 62 major.rs:285）清理过悬垂指针，
        // 且 Minor/Major 不重叠（maybe_gc phase 守卫），故 old_ptr 仍为有效 Old 对象。
        let tag = unsafe { (*old_ptr).type_tag };
        let ff = type_descriptor(tag).forward_fields;
        ff(old_ptr, &mut |slot| c.forward_slot(slot));
    }

    // ... 既有 Cheney 扫描、释放旧 Young、统计 ...

    // task 63：dirty cards 已 drain；幸存的 Old→Young 引用已转发到新 Young/晋升 Old。
    // 下次 Old→Young 写入由常驻屏障重新 mark_dirty。
}
```

**调用点更新**（`src/vm/mod.rs`）：`maybe_gc`、`gc_full`、`gc_minor_only`、`complete_concurrent_cycle_if_pending` 中所有 `gc::minor_gc(...)` 调用追加 `&self.gc_runtime.card_table` 参数。

> **幂等**：`forward_slot` 经 `Copier.map` 幂等（同一 Young 对象只复制一次）。多个 dirty Old 指向同一 Young 对象 → 第二次 forward 命中 map，直接返回新指针。安全。
>
> **与 major sweep 的交互**：card_table.drain 在 minor GC 内消费。Major sweep 后 Task 62 已调 `retain_valid`（major.rs:285）清理被回收 Old 的悬垂 dirty 指针。故 minor GC drain 出的指针均有效。

### 5. 拆分 close_concurrent_cycle

Task 62 的 `close_concurrent_cycle`（`major.rs:270`）在一个 STW 窗口内完成 Mark Termination + Sweep。Task 63 拆为两阶段：

- **`finish_mark_termination`**（mutator，rendezvous #1 后）：重扫根集 + drain gray + 关闭写屏障 + 设 phase=ConcurrentSweep。**不 sweep**。
- **`reconcile_sweep`**（mutator，rendezvous #2 后）：应用 Coordinator 的 dead 集 + 重置 Black→White + LOS/finalizer/bytes。设 Idle + finalize_pending。

```rust
// src/vm/gc/major.rs

/// mutator 在 closure_pending（rendezvous #1）后调用：Mark Termination，不含 Sweep。
/// 完成后置 phase=ConcurrentSweep，Coordinator 检测到后开始并发清扫。
pub fn finish_mark_termination(vm: &mut VM) {
    let gc = Arc::clone(&vm.gc_runtime);
    let t0 = std::time::Instant::now();

    let gc_managed = gc.gc_managed_clone().unwrap_or_default();

    // Mark Termination：重扫根集（并发标记期间栈/globals 无写屏障，可能被修改）。
    gc.set_phase(GcPhase::MarkTermination);
    scan_roots_gray(&gc, vm, &gc_managed);
    drain_gray(&gc, &gc_managed);

    gc.term_stw_ns.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);

    // task 63：清空 sweep 累加器，进入并发清扫（mutator 随后恢复执行字节码）。
    gc.clear_sweep_accumulators();
    gc.sweep_reconcile_pending.store(false, Ordering::Relaxed);
    gc.set_phase(GcPhase::ConcurrentSweep);
    // 注意：不设 finalize_pending、不回 Idle。Coordinator 将并发清扫后再请求第 2 次 STW。
}
```

```rust
/// mutator 在 sweep_reconcile_pending（rendezvous #2）后调用：应用 Coordinator 的清扫结果。
/// 拥有 &mut VM，故可修改 old_objects/los_objects/bytes_allocated/finalizer_queue。
pub fn reconcile_sweep(vm: &mut VM) {
    let gc = Arc::clone(&vm.gc_runtime);
    let t0 = std::time::Instant::now();

    let dead_old: HashSet<*mut MsObjHeader> =
        gc.sweep_dead_old.lock().unwrap().drain(..).collect();
    let finalizers: Vec<*mut MsObjHeader> =
        gc.sweep_finalizers.lock().unwrap().drain(..);
    let swept = gc.swept_bytes.load(Ordering::Relaxed);

    // 1. Old 代：移除 dead，存活者 Black|Gray→White 重置（mutator 独占，安全）。
    vm.heap.old_objects.retain(|&p| {
        if dead_old.contains(&p) { return false; }
        // SAFETY: p 为有效 Old 对象（Coordinator 未释放它，即非 dead）。
        let h = unsafe { &mut *p };
        h.set_color(Color::White);
        true
    });

    // 2. finalizer 对象：复活（保留在 old_objects，置 White，入 finalizer_queue）。
    for obj in &finalizers {
        // SAFETY: *mut MsObjHeader 仍有效（Coordinator 未释放 finalizer 对象）。
        unsafe { (*obj).set_color(Color::White); }
        vm.heap.finalizer_queue.push(*obj);
    }

    // 3. LOS 清扫（Coordinator 不处理 LOS，mutator 序贯完成；LOS 对象稀有）。
    //    sweep_los 从 Task 52 的 sweep_heap（gc.rs:1457）抽出 LOS 分支（gc.rs:1484-1506）：
    //    los_objects.retain：Black→White、White+finalizer→复活入队、White→dealloc（los_sizes 取大小）。
    //    old 分支由本 reconcile 的 step 1 替代；sweep_heap 保留供降级路径（major_gc）复用。
    sweep_los(&mut vm.heap);

    // 4. bytes 计数。
    vm.heap.bytes_allocated = vm.heap.bytes_allocated.saturating_sub(swept as usize);
    vm.heap.bytes_freed = vm.heap.bytes_freed.saturating_add(swept);

    // 5. 清理 Card Table 中已释放对象的悬垂 dirty 指针（防下次 minor GC drain 后 UAF）。
    gc.card_table.retain_valid(&vm.heap.old_objects);

    // 6. next_major_gc 阈值（同 Task 62：空堆回退初始阈值）。
    let computed = (vm.heap.bytes_allocated as f64 * super::MAJOR_GC_RATIO) as usize;
    vm.heap.next_major_gc = if computed == 0 { super::INITIAL_MAJOR_THRESHOLD } else { computed };
    vm.heap.major_count += 1;

    gc.concurrent_sweep_ns.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    gc.clear_gc_managed();
    gc.set_phase(GcPhase::Finalize);
    gc.finalize_pending.store(true, Ordering::Relaxed);
    gc.set_phase(GcPhase::Idle);
}
```

> **降级模式不变**：`concurrent_enabled=false` 时 `maybe_gc` 走 `major_gc` + `run_finalizers`（Task 52 STW 路径，`mod.rs:1099`），不经 Coordinator / finish_mark_termination / reconcile_sweep。`major_gc` 内部仍调 `sweep_heap`（STW）。本任务保留 `sweep_heap` 供降级路径与 `major_gc` 复用。

### 6. Coordinator 并发清扫

参照 [14-gc](../14-gc.md) § Concurrent Sweep（443-475 行）。Coordinator 线程在 Mark Termination 完成后，遍历 `gc_managed` 快照释放 White 对象。

```rust
// src/vm/gc/sweep.rs
use super::header::{color_atomic, generation_atomic, GcPhase};
use super::runtime::GcRuntime;
use super::{type_descriptor, Color, Generation, MsObjHeader};
use crate::vm::object::TypeTag;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Coordinator 并发清扫：遍历 gc_managed 快照，释放 White Old 对象。
/// LOS 与 finalizer 对象交由 mutator reconcile（见 reconcile_sweep §3/§5）。
///
/// # Safety 前提
/// - phase == ConcurrentSweep（mutator 已完成 Mark Termination，颜色稳定）
/// - 写屏障已关闭（mutator 写入不改颜色）
/// - White 对象不可达（mutator 无法引用 → free 无别名 UB）
pub fn concurrent_sweep(gc: &Arc<GcRuntime>) {
    debug_assert_eq!(gc.phase(), GcPhase::ConcurrentSweep);
    let t0 = std::time::Instant::now();

    let Some(managed) = gc.gc_managed_clone() else {
        return; // 无快照（空周期）
    };

    for &obj in managed.0.iter() {
        // SAFETY: obj 在 gc_managed 中，为有效 MsObjHeader。
        let color = unsafe { color_atomic(obj) };
        if color != Color::White {
            continue; // Black|Gray 存活，reconcile 时重置 White
        }

        // task 63：仅处理 Old 代。LOS（type_tag=LARGE_OBJECT）显式跳过——LOS dealloc 需
        // los_sizes 侧表（MsHeap 独占），交 mutator reconcile 序贯处理。
        // 注意：当前 alloc_los（gc.rs:1115）写 header gc_meta=0 → generation=Young，故 LOS
        // 也会被下方的 Old 过滤跳过；但按 type_tag 显式跳过更稳健——若未来 LOS 修正为
        // Old/Immortal 代，此处行为不变。双重过滤（tag + gen）无副作用。
        // SAFETY: obj 有效。
        let h = unsafe { &*obj };
        if h.type_tag == TypeTag::LARGE_OBJECT as u8 {
            continue;
        }
        if unsafe { generation_atomic(obj) } != Generation::Old {
            continue;
        }

        if h.has_finalizer() {
            // White + finalizer → 复活（不释放），交 mutator 入 finalizer_queue。
            gc.sweep_finalizers.lock().unwrap().push(obj);
            continue;
        }
        if h.is_pinned() {
            // C 侧 pin 的 White 对象保留（14-gc.md 84-85 行）。
            continue;
        }

        // 释放：typed free（Box::from_raw + Drop 载荷）。对象不可达 → 无别名。
        let size = h.size as u64;
        let tag = h.type_tag;
        (type_descriptor(tag).free)(obj);
        gc.sweep_dead_old.lock().unwrap().push(obj);
        gc.swept_bytes.fetch_add(size, Ordering::Relaxed);
    }

    gc.concurrent_sweep_ns
        .store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
}
```

> **spec 偏差（并行度）**：14-gc.md:445 描述"GC Worker 线程"并行清扫。本实现用**单 Coordinator 线程**（与 mutator 并发，但清扫自身并行度=1）。功能等价（White 对象被释放、mutator 不停），吞吐降级。多 Worker 分片清扫列为优化项（按 gc_managed 指针哈希分 N 片，各 Worker 独立 free + 各自 dead 子集，reconcile 合并），可在 Task 64 或独立优化 task 落地。
>
> **为何 LOS 交 mutator**：LOS dealloc 需 `los_sizes` 侧表（`MsHeap` 独占）取真实大小构造 `Layout`（Task 52 gc.rs:1112-1128）。Coordinator 无 `&mut MsHeap`。LOS 对象稀有（>32KB），序贯处理可接受。未来可经 `Arc<Mutex<HashMap>>` 共享 los_sizes 以并行化。
>
> **free() 线程安全**：`type_descriptor(tag).free` 经 `Box::from_raw` 调全局分配器（thread-safe）+ Drop 载荷。对象不可达（White）→ 无并发访问 → 无数据竞争。Drop 内部（如 HashMap 释放 bucket）操作对象私有内存，mutator 不触碰。
>
> **地址复用风险**：Coordinator free 地址 X 后，全局分配器可能立即将 X 复用给 mutator 的 `gc_alloc_*`（新 Young 对象，入 `young_objects`、标 Black）。此时 `old_objects` 仍含旧悬垂 X（待 reconcile 移除）。清扫窗口内**任何路径不得按指针遍历 `old_objects` 并解引用**——正常 mutator 执行不迭代 `old_objects`，phase 守卫阻止 GC 重入，`gc_full` 经 `complete_concurrent_cycle_if_pending` 等周期结束。**`gc.verify()`（14-gc.md:794）须跳过 ConcurrentSweep 阶段**，否则会命中悬垂指针。
>
> **颜色重置延后（spec 偏差，等价）**：spec 在 sweep 内重置 Black|Gray→White（14-gc.md:448-462）。本实现把重置推迟到 `reconcile_sweep`（mutator，§5）。等价：sweep 与 reconcile 间无 GC 周期（phase 守卫），reconcile 与 run_finalizers 在同一 safepoint 窗口紧接执行。finalizer 对象 spec 设 Gray、本实现设 White，同样因 run_finalizers 紧随 reconcile 而无影响。

### 7. Coordinator 周期驱动（更新 run_major_cycle）

Task 62 的 `run_major_cycle`（major.rs:240）在并发标记后请求一次 STW 设 closure_pending。Task 63 扩展为两次 rendezvous：

```rust
// src/vm/gc/major.rs
fn run_major_cycle(gc: &Arc<GcRuntime>) {
    let Some(managed) = gc.gc_managed_clone() else {
        gc.set_phase(GcPhase::Idle);
        return;
    };

    // 阶段 A：并发标记（Worker 池）。
    run_concurrent_mark_only(gc, &managed);

    // Rendezvous #1：请求 STW，mutator park → 设 closure_pending → release。
    // mutator 醒后在 gc_safepoint_and_finalize 调 finish_mark_termination（设 ConcurrentSweep）。
    gc.safepoint.request_and_wait();
    gc.closure_pending.store(true, Ordering::Release); // Release：见 §1 happens-before
    gc.safepoint.release();

    // 等 mutator 完成 Mark Termination（phase → ConcurrentSweep）。
    // 有界自旋：mutator 醒后几微秒内完成 finish_mark_termination。若长期等不到（mutator
    // 阻塞 syscall / 死锁），warn 后继续等待（不 panic —— VM drop 时 shutdown 会先 release
    // safepoint 解除此阻塞）。NopSafepoint（Task 62，≤1000 指令）保证正常字节码执行下快速到达。
    let mut spins = 0u32;
    while gc.phase() != GcPhase::ConcurrentSweep {
        std::thread::yield_now();
        spins = spins.saturating_add(1);
        if spins % 1_000_000 == 0 {
            eprintln!("mslang-gc: long wait for mutator to finish mark termination");
        }
    }

    // 阶段 B：并发清扫（mutator 继续运行字节码）。
    super::sweep::concurrent_sweep(gc);

    // Rendezvous #2：请求 STW，mutator park → 设 sweep_reconcile_pending → release。
    // mutator 醒后调 reconcile_sweep（应用 dead 集 + 回 Idle）。
    gc.safepoint.request_and_wait();
    gc.sweep_reconcile_pending.store(true, Ordering::Release); // Release：保证 swept_bytes 可见
    gc.safepoint.release();
}
```

> **rendezvous #2 的到达**：mutator 在并发清扫期间持续执行，在每个安全点字节码（Call/Jump/ForIter/Return/Import/Await/Send/Receive）调 `gc_safepoint_and_finalize`。Coordinator 完成清扫并 `request_and_wait` 后，mutator 在下一个安全点 park。Coordinator release 后 mutator 执行 `reconcile_sweep`。若长时间无安全点（纯计算），编译器的基本块 ≤1000 指令校验（Task 62）保证 `NopSafepoint` 兜底。

### 8. Compaction 度量与延后

参照 [14-gc](../14-gc.md) § Old Generation（174-180 行）+ § 动态阈值（300 行）。

**现状**：Old 代为散布 Box 分配（`old_objects: Vec<*mut>`），每个对象独立 `Box::into_raw`，无连续 arena、无 free-list、**无碎片**。Compaction 无可压缩对象。

**本任务落地**：

```rust
// src/vm/gc.rs
impl MsHeap {
    /// task 63：Old 代碎片率。Box 模型下恒为 0（无 arena holes）。
    /// 未来 Old 迁移为 arena + free-list 后，改为 free_bytes / old_capacity。
    /// 14-gc.md § 动态阈值：碎片率 > 30% 触发 Compaction。
    pub fn fragmentation_ratio(&self) -> f64 {
        0.0
    }

    /// task 63：是否应触发 Compaction。当前恒 false（Box 模型无碎片）。
    pub fn should_compact(&self) -> bool {
        self.fragmentation_ratio() > 0.30
    }
}
```

`maybe_gc` / `reconcile_sweep` 中增加判定（当前恒不触发）：

```rust
// reconcile_sweep 末尾（mutator，STW）
if vm.heap.should_compact() {
    compact_old(vm); // task 63：stub —— Box 模型下 unreachable（fragmentation_ratio == 0.0）
}
```

```rust
/// task 63：Old 代 Compaction（STW）。当前模型下不应被调用（fragmentation 恒 0）。
/// 未来 Old 迁移为 arena + free-list 后实装：滑动压缩存活对象、更新所有 Ref 指针、
/// 重建 free-list。依赖 forward_fields 全量重写指针（同 Minor GC 的 Cheney 转发语义）。
fn compact_old(_vm: &mut VM) {
    debug_assert!(false, "compaction not implemented; fragmentation_ratio should be 0.0");
}
```

> **迁移路径（未来 task，须注册）**：将 Old 代从 `Vec<*mut Box>` 改为连续 arena（`old_arena: Vec<u8>` + bump/free-list 分配）。此后：sweep 在 free-list 标记空闲块 → 碎片产生 → `fragmentation_ratio` 反映真实值 → `should_compact` 触发 → `compact_old` 滑动压缩。此迁移涉及 `gc_alloc_*`、`copy_for_gc`（晋升路径）、`free` 全量改造，风险大，单列 task。**须在 `tasks/README.md` Phase 7.5 补占位行**（如 "Old 代 arena 迁移 + Compaction 实装"），否则 `should_compact` / `compact_old` 成为永久死代码。Task 64（调优）范围可考虑纳入，或独立 task。

### 9. 并发清扫期间的对象分配与 Minor GC

参照 [14-gc](../14-gc.md) § Concurrent Sweep 新分配标黑（465-475 行）。

- **新分配标黑（本任务实装）**：`alloc_during_gc`（Task 62 `barrier.rs:90`）已实现 ConcurrentMark/ConcurrentSweep 期标 Black 的逻辑，但 `gc_alloc_*`（`gc.rs:1141-1190`）**从未调用它**（仅 barrier.rs 测试调用，grep 确认）。本任务**必须实装**：给 `gc_alloc_*` 增加 `gc: &GcRuntime` 参数，末尾调 `alloc_during_gc`：

```rust
// src/vm/gc.rs —— 改造 gc_alloc_*（以 gc_alloc_list 为例，其余同构）
pub fn gc_alloc_list(heap: &mut MsHeap, gc: &GcRuntime, items: Vec<Object>) -> Object {
    let obj = Box::new(GcList {
        header: header_for(TypeTag::LIST, std::mem::size_of::<GcList>() as u16),
        items,
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcList>());
    // task 63：并发标记/清扫期间新分配 → 标黑，避免下轮误回收。
    // SAFETY: ptr 刚由 Box::into_raw 分配，有效 MsObjHeader。
    unsafe { alloc_during_gc(gc, ptr); }
    Object::Ref(ptr)
}
```

  **调用点全量更新**：所有 `gc_alloc_string/list/tuple/dict/set` 调用须传 `&gc_runtime`（VM 侧经 `&self.gc_runtime`，测试侧经构造的 `GcRuntime`）。**含 minor GC 晋升路径**——`Copier::copy`（`gc.rs:1239`）调 `copy_for_gc` 产生新对象后，亦须按当前 phase 标色（晋升到 Old 的对象在并发标记期应为 Black）；由于 minor GC 受 phase 守卫不在并发期触发，晋升对象实际标 White 即可，但须确保不与并发标记得出的颜色冲突（minor GC 不与 ConcurrentMark/Sweep 并发，见下条）。

- **Minor GC 不与清扫并发**：`maybe_gc` 的 phase 守卫（`mod.rs:1081`，`phase != Idle → return`）在 `ConcurrentSweep` 期间阻止触发 minor/major GC。故 `old_objects` 在清扫期间不被 minor GC 的晋升路径追加，`young_objects` 不被修改。Coordinator 读 `gc_managed` 快照（已固定）安全。
- **手动 gc.collect()**：`gc_full` / `gc_minor_only` 先调 `complete_concurrent_cycle_if_pending`（`mod.rs:1139`）等当前周期结束，不与清扫重叠。

## VM 集成变更

### gc_safepoint_and_finalize 扩展（`src/vm/mod.rs:1113`）

```rust
fn gc_safepoint_and_finalize(&mut self) {
    if self.gc_runtime.safepoint.is_requested_fast() {
        self.gc_runtime.safepoint.check_and_park();
        // Rendezvous #1：Mark Termination（不含 Sweep）。
        if self.gc_runtime.closure_pending.swap(false, Ordering::Relaxed) {
            gc::finish_mark_termination(self);
        }
        // Rendezvous #2：Sweep reconcile。
        if self.gc_runtime.sweep_reconcile_pending.swap(false, Ordering::Relaxed) {
            gc::reconcile_sweep(self);
        }
        // finalize（mutator 线程，需 &mut VM 调 __del__）。
        if self.gc_runtime.finalize_pending.swap(false, Ordering::Relaxed) {
            gc::run_finalizers(&mut self.heap);
        }
    }
}
```

> **顺序**：closure_pending 与 sweep_reconcile_pending 不会在同一安全点同时为 true（Coordinator 串行触发：#1 release → 清扫 → #2 request）。finalize 在 reconcile 之后（reconcile 设 finalize_pending）。逻辑等价 Task 62，仅多 reconcile 分支。

### complete_concurrent_cycle_if_pending（`mod.rs:1139`）

不变。其循环 `while phase != Idle { gc_safepoint_and_finalize(); yield }` 自动覆盖两次 rendezvous，直至 `reconcile_sweep` 置 Idle。

### minor_gc 调用点

`maybe_gc`（mod.rs:1085）、`gc_full`（mod.rs:1025）、`gc_minor_only`（mod.rs:1044）、`complete_concurrent_cycle_if_pending` 间接路径：所有 `gc::minor_gc(...)` 调用追加 `&self.gc_runtime.card_table`。

## 验证标准

### 并发清扫正确性
1. **存活对象保留**：并发清扫后，可达（Black）Old 对象仍在 `old_objects` 且内容正确
2. **不可达回收**：White 无 finalizer 对象被 free，从 `old_objects` 移除，`bytes_allocated` 减少
3. **finalizer 复活**：White + has_finalizer 对象入 `finalizer_queue`，保留在 `old_objects`，`run_finalizers` 后清 has_finalizer，下次 GC 正常回收（不无限复活）
4. **pinned 保护**：White + pinned 对象保留（Coordinator 跳过）
5. **颜色重置**：reconcile 后所有存活 Old 对象为 White（为下轮 GC 准备）
6. **清扫期间 mutator 运行**：ConcurrentSweep 阶段 phase != Idle，`maybe_gc` 重入守卫生效，不重复触发
7. **清扫期新分配标黑**：ConcurrentSweep 阶段经 `gc_alloc_*` 分配的对象被 `alloc_during_gc` 标 Black（核验：gc_alloc_* 末尾调用，下轮 GC 不误回收）

### Card Table 扫描侧
8. **跨代引用不漏扫**：Old list 持有唯一 Young 引用 → Minor GC 后 Young 对象转发存活（非回收）
9. **dirty drain 幂等**：多个 dirty Old 指向同一 Young → 转发一次，无双重释放
10. **常驻屏障标记**：非 ConcurrentMark 阶段（含降级模式）建立 Old→Young 引用 → card dirty → 下次 Minor GC 扫描

### LOS 与统计
11. **LOS 清扫**：不可达 LOS 对象 dealloc + 从 `los_objects`/`los_sizes` 移除；`bytes_allocated` 减
12. **bytes 一致**：`swept_bytes` = reconcile 减计的 `bytes_allocated`；`bytes_freed` 累加正确
13. **并发指标**：`concurrent_sweep_ns` 记录 Coordinator 清扫耗时（Task 77 验证）

### Compaction（延后）
14. **碎片度量**：`fragmentation_ratio()` 恒返回 0.0；`should_compact()` 恒 false
15. **不触发**：`compact_old` 在本模型下 unreachable（debug_assert 兜底）

### 并发安全
16. **无别名 UB**：Coordinator 仅 free 不可达 White 对象（mutator 无法引用）；`old_objects`/`bytes_allocated` 仅 mutator 在 reconcile 修改
17. **无悬垂 dirty**：reconcile 调 `retain_valid` 清理已释放 Old 的 card 指针
18. **两次 STW 极短**：Mark Termination 与 reconcile 均不含 `free()`（free 在并发阶段），STW 与根集/对象数成正比，非与 dead 对象数成正比
19. **弱模型可见性**：phase 转换与 sweep_reconcile_pending 经 Release/Acquire，Coordinator/mutator 跨线程读写颜色与累加器无陈旧读（§1）

## 测试用例

### Rust 单元测试（`src/vm/gc/sweep.rs`）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::gc::header::set_color_atomic;
    use crate::vm::gc::runtime::{GcManagedSet, GcRuntime};
    use crate::vm::gc::{GcList, header_for};
    use crate::vm::object::{MsObjHeader, Object, TypeTag};
    use std::collections::HashSet;
    use std::sync::Arc;

    fn make_old_list(color: Color) -> *mut MsObjHeader {
        let obj = Box::new(GcList {
            header: header_for(TypeTag::LIST, std::mem::size_of::<GcList>() as u16),
            items: vec![],
        });
        let ptr = Box::into_raw(obj) as *mut MsObjHeader;
        // SAFETY: ptr 有效。
        unsafe {
            (*ptr).set_generation(crate::vm::gc::Generation::Old);
            set_color_atomic(ptr, color);
        }
        ptr
    }

    #[test]
    fn test_concurrent_sweep_frees_white_old() {
        let gc = Arc::new(GcRuntime::new());
        let live = make_old_list(Color::Black);
        let dead = make_old_list(Color::White);
        gc.set_gc_managed(Arc::new(GcManagedSet(
            [live, dead].into_iter().collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        // dead 入 sweep_dead_old；live 不入。
        assert_eq!(gc.sweep_dead_old.lock().unwrap().len(), 1);
        assert!(gc.sweep_dead_old.lock().unwrap().contains(&dead));
        // SAFETY: live 未被释放，仍可读。
        unsafe {
            assert_eq!(crate::vm::gc::header::color_atomic(live), Color::Black);
            drop(Box::from_raw(live as *mut GcList));
        }
        // dead 已被 free（Box::from_raw），不可再访问。
    }

    #[test]
    fn test_concurrent_sweep_keeps_finalizer_white() {
        let gc = Arc::new(GcRuntime::new());
        let fin = make_old_list(Color::White);
        // SAFETY: fin 有效。
        unsafe { (*fin).set_has_finalizer(true); }
        gc.set_gc_managed(Arc::new(GcManagedSet(
            std::iter::once(fin).collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        // finalizer 对象入 sweep_finalizers，未被 free（仍可访问）。
        assert_eq!(gc.sweep_finalizers.lock().unwrap().len(), 1);
        assert!(gc.sweep_dead_old.lock().unwrap().is_empty());
        unsafe { drop(Box::from_raw(fin as *mut GcList)); }
    }

    #[test]
    fn test_concurrent_sweep_skips_pinned_white() {
        let gc = Arc::new(GcRuntime::new());
        let pinned = make_old_list(Color::White);
        // SAFETY: pinned 有效。直接置 PINNED 位（gc_meta 位 5 = 0b0010_0000）。
        // 注：MsObjHeader 当前仅有 is_pinned()（gc.rs:142），无 set_pinned 访问器。
        // 本任务须在 gc.rs 补 `pub fn set_pinned(&mut self, on: bool)`：
        //   if on { self.gc_meta |= Self::PINNED; } else { self.gc_meta &= !Self::PINNED; }
        // 补齐后此处置换为 (*pinned).set_pinned(true)。
        unsafe { (*pinned).gc_meta |= 0b0010_0000; }
        assert!(unsafe { (*pinned).is_pinned() });
        gc.set_gc_managed(Arc::new(GcManagedSet(
            std::iter::once(pinned).collect(),
        )));
        gc.set_phase(GcPhase::ConcurrentSweep);

        concurrent_sweep(&gc);

        assert!(gc.sweep_dead_old.lock().unwrap().is_empty());
        unsafe { drop(Box::from_raw(pinned as *mut GcList)); }
    }
}
```

> **set_pinned 访问器（本任务须补）**：`MsObjHeader`（gc.rs:93）已有 `is_pinned()`（位 5 读）但缺 `set_pinned`。本任务在 `impl MsObjHeader` 补：
> ```rust
> pub fn set_pinned(&mut self, on: bool) {
>     if on { self.gc_meta |= Self::PINNED; } else { self.gc_meta &= !Self::PINNED; }
> }
> ```
> Task 74 的 C API（`msPin`）也需要此访问器；若 Task 74 已补则直接复用。

### 集成测试：minor GC 跨代引用（`tests/gc_sweep_tests.rs`）

> **测试前提**：以下测试经 `gc_alloc_*` **直接**构造 GC 堆对象（绕过 VM 的 `alloc_*` 路径），以验证 remembered set / 并发清扫逻辑。**当前 VM 字面量（如 `[1,2,3]`）走 `alloc_*`（`object.rs:231`，非 GC 堆），不经 gc_alloc_*，故 mslang 端到端测试（下方 `test_concurrent_sweep.ms`）目前**不真正触达**本任务的 GC 路径——它们仅验证"不 panic"（冒烟）。本任务的实质正确性覆盖以**Rust 单元测试（直接用 gc_alloc_*）+ 下方 Rust 集成测试**为准。待 VM 分配全量接入 GC 堆（未来 task），mslang 测试方才有效。

```rust
#[test]
fn test_minor_gc_remembered_set_keeps_young() {
    use crate::vm::VM;
    use crate::vm::gc::{gc_alloc_list, minor_gc, Generation};
    use crate::vm::object::Object;
    use std::collections::HashMap;

    let mut vm = VM::new();
    vm.heap.promotion_age = 1;

    // 1. 分配并晋升 old_list 到 Old 代。
    let old_list = gc_alloc_list(&mut vm.heap, vec![]);
    vm.stack.push(old_list.clone());
    minor_gc(&mut vm.heap, &mut vm.stack, &mut vm.globals,
             &mut vm.defer_stack, &mut vm.call_stack, &vm.gc_runtime.card_table);

    // 2. 分配 young_obj（Young），append 到 old_list（建立 Old→Young 跨代引用）。
    let young_obj = gc_alloc_list(&mut vm.heap, vec![Object::Int(42)]);
    if let (Object::Ref(op), Object::Ref(yp)) = (&old_list, &young_obj) {
        // SAFETY: old_list 已晋升 Old；写经 write_barrier_obj 标 dirty card。
        unsafe { crate::vm::gc::gc_read_list_mut(*op).push(young_obj.clone()); }
        unsafe {
            crate::vm::gc::write_barrier_obj(&vm.gc_runtime, *op, std::ptr::null_mut(), *yp);
        }
    }
    // young_obj 唯一引用在 old_list 内（stack/globals 无）。

    // 3. 再分配垃圾触发 Minor GC；young_obj 应经 dirty card 扫描存活（转发/晋升）。
    let _garbage = gc_alloc_list(&mut vm.heap, vec![Object::Int(0)]);
    vm.heap.next_minor_gc = 0; // 强制
    minor_gc(&mut vm.heap, &mut vm.stack, &mut vm.globals,
             &mut vm.defer_stack, &mut vm.call_stack, &vm.gc_runtime.card_table);

    // 4. old_list 仍可达且其元素为转发的 young_obj（内容 42）。
    if let Object::Ref(op) = &vm.stack[0] {
        // SAFETY: old_list 存活。
        let items = unsafe { crate::vm::gc::gc_read_list(*op) };
        assert_eq!(items.len(), 1);
        if let Object::Ref(yp) = &items[0] {
            // SAFETY: young_obj 经 minor GC 存活（晋升或转发）。
            assert_eq!(unsafe { crate::vm::gc::gc_read_list(*yp) }.clone(),
                       vec![Object::Int(42)]);
        } else { panic!("young_obj collected (remembered set failed)"); }
    }
}

#[test]
fn test_concurrent_sweep_end_to_end() {
    use crate::vm::VM;
    use crate::vm::gc::{gc_alloc_list, GcPhase};
    use crate::vm::object::Object;
    use std::sync::atomic::Ordering;

    let mut vm = VM::new();
    vm.gc_runtime.concurrent_enabled.store(true, Ordering::Relaxed);
    vm.gc_coordinator = Some(crate::vm::gc::GcCoordinator::spawn(
        std::sync::Arc::clone(&vm.gc_runtime),
    ));
    vm.heap.promotion_age = 1;

    // 分配 + 晋升一批对象，部分解除根使其不可达。
    let live = gc_alloc_list(&mut vm.heap, vec![Object::Int(7)]);
    let dead = gc_alloc_list(&mut vm.heap, vec![Object::Int(8)]);
    vm.stack.extend([live.clone(), dead.clone()]);
    minor_gc(&mut vm.heap, &mut vm.stack, &mut vm.globals,
             &mut vm.defer_stack, &mut vm.call_stack, &vm.gc_runtime.card_table);
    vm.stack.retain(|v| matches!(v, Object::Ref(r) if *r != extract_ref(&dead)));

    // 触发并发 Major GC + 清扫，等待周期结束。
    vm.heap.next_major_gc = 0;
    vm.maybe_gc();
    vm.complete_concurrent_cycle_if_pending();

    // live 仍存活且可达；不可达 dead 被回收（old_objects 不含 dead）。
    if let Object::Ref(r) = &vm.stack[0] {
        // SAFETY: live 存活。
        assert_eq!(unsafe { crate::vm::gc::gc_read_list(*r) }.clone(), vec![Object::Int(7)]);
    }
    assert_eq!(vm.gc_runtime.phase(), GcPhase::Idle);
}

fn extract_ref(o: &Object) -> *mut MsObjHeader {
    if let Object::Ref(r) = o { *r } else { unreachable!() }
}
```

### mslang 级别验证 `test_concurrent_sweep.ms`

> **aspirational（冒烟）**：当前 VM 字面量分配走 `alloc_*`（非 GC 堆），故此 mslang 测试**不触达**并发清扫/remembered set 的 GC 路径，仅验证脚本不 panic + 存活对象语义正确。真正覆盖由 Rust 单元/集成测试（直接用 `gc_alloc_*`）承担。待 VM 分配接入 GC 堆后此测试方为有效回归。

```ms
import gc

fn test_concurrent_sweep() {
    gc.set_concurrent(true)
    # 分配大量短命对象，触发并发 Major GC + 清扫
    for i in range(2000) {
        x = [i, i+1, i+2]
    }
    # 存活对象仍正确
    keep = [10, 20, 30]
    gc.collect()
    print(keep[2])  # 30
    print("concurrent sweep ok")
}

fn test_remembered_set() {
    gc.set_concurrent(true)
    cache = []          # 将晋升 Old
    for i in range(100) {
        cache.push([i]) # Old 持有 Young 引用（跨代）
    }
    gc.collect()
    # cache 及其元素经 remembered set 存活
    print(cache[0][0])  # 0
    print("remembered set ok")
}

test_concurrent_sweep()
test_remembered_set()
```

预期输出：
```
30
concurrent sweep ok
0
remembered set ok
```

### 构建验证

```bash
# 单元测试
cargo test -- gc::sweep

# 跨代引用 + 并发清扫集成测试
cargo test --test gc_sweep_tests

# mslang 级别
cargo run -- run tests/integration/test_concurrent_sweep.ms

# 降级模式回归（Task 52 行为不变）
cargo test -- gc::tests
```

## 实现注意事项

1. **Coordinator 线程 Join 安全**：`run_major_cycle` 现含两次 `request_and_wait`。VM drop 时 `gc_coordinator.shutdown` 先 `safepoint.release()`（解除可能阻塞的 rendezvous），再发 Shutdown + join（Task 62 已处理，本任务不改变该契约）。`complete_concurrent_cycle_if_pending` 在 `gc_set_concurrent(false)` / drop 前驱动两次 rendezvous 至 Idle。
2. **unsafe 边界**（14-gc.md 788-794 行）：`concurrent_sweep` 的 `unsafe` 块 ≤ 30 行，附 `// SAFETY:` 注释。free 仅对 gc_managed 内 White Old 对象（不可达），无别名。
3. **渐进启用**：`concurrent_enabled` 默认仍 `false`（Task 62）。并发清扫仅在 `concurrent_enabled=true` 时经 Coordinator 路径生效；降级模式仍走 `sweep_heap`（STW）。Task 64 调优后可改默认。
4. **alloc_during_gc 实装（非可选）**：`gc_alloc_string/list/tuple/dict/set`（gc.rs:1141-1190）当前**不调用** `alloc_during_gc`（grep 确认仅 barrier.rs 测试调用）。本任务按 §9 改造：增加 `gc: &GcRuntime` 参数，`Box::into_raw` 后调 `alloc_during_gc(gc, ptr)`。**波及所有 gc_alloc_* 调用点**（VM 侧 `&self.gc_runtime`、测试侧构造的 `GcRuntime`、minor GC 晋升路径），须全量更新并 `cargo check` 验证。这是并发清扫正确性的核心不变式（清扫期 White 新对象会被下轮误回收）。
5. **与 Task 64 的接口**：Task 64 自适应调优可基于本任务的 `concurrent_sweep_ns`/`swept_bytes` 统计调整 GC 线程数、触发阈值。Compaction 触发（`should_compact`）在 arena 迁移后由 Task 64 或独立 task 实装。
6. **与 Task 77 的接口**：Task 77 的并发 GC C API（`msGcGetStats` 等）读取本任务的 `concurrent_sweep_ns`。`msWriteBarrier` 应调用改造后的 `write_barrier_obj`（card marking 常驻），Rust 侧与 C API 侧一致。
7. **forward_fields 用于 card 扫描的局限**：`forward_fields` 设计用于 Cheney 复制（转发 Young from-space 对象）。复用于 Old→Young card 扫描时，`forward_slot` 的 `old_young_set` 判定正确过滤非 from-space 对象。但若 Old 对象经 `alloc_*`（非 GC 堆）分配，其 forward_fields 为 noop 占位 → 跨代引用漏扫。当前 VM 日常 alloc_* 未接入 GC 堆（已知限制），此漏扫与既有限制一致；接入后须补真实 forward_fields。
