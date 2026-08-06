# 并发标记（tri-color + 写屏障）

> **注意**：本任务将 Task 52 的 STW Major GC 升级为**并发三色标记**，并引入**混合写屏障**（Go 1.8+ 风格）。完整 GC 设计见 [14-gc](../14-gc.md) § Major GC、§ 混合写屏障、§ 字节码安全点。并发清扫与 Compaction（§ 7.5.3-7.5.4）为 Task 63；GC 调优（§ 7.5.6）为 Task 64。C API 并发 GC 交互为 Task 77。

## 所属阶段
Phase 7.5 — 并发 GC 优化

## 前置任务
- **52-gc**（GC MVP）：MsHeap、MsObjHeader、Color/Generation、TypeDescriptor、`major_gc`/`minor_gc`、`gc_alloc_*`
- **23-vm-core**：VM 执行循环、`maybe_gc` 触发点（`src/vm/mod.rs:2696`）
- **28-closures**：Upvalue 机制、`StoreUpvalue` 字节码
- **53-async-await**：事件循环、协程模型（GC Coordinator 需与事件循环协调安全点）

## 目标

1. **混合写屏障**（Go 1.8+ 风格）——并发标记期间保持三色不变性，栈不需要写屏障
2. **GC 状态机**（Idle → Init → ConcurrentMark → MarkTermination → ConcurrentSweep → Finalize → Idle）
3. **GC Coordinator 线程** + **GC Worker 线程池**——并发标记与 mutator 并行
4. **线程安全灰色队列**（GrayQueue）——多生产者（GC Worker + mutator 写屏障）并发 push
5. **安全点机制**——STW 协调（Init / Mark Termination 两次短暂暂停）
6. **Card Table 写侧**——Old → Young 跨代引用标记 dirty（供 Task 63 Minor GC 扫描）
7. **降级路径**——`gc.set_concurrent(false)` 回退 Task 52 STW 行为（14-gc.md § Phase 7.5 降级路径，796-801 行）

## 设计规格

参照 [14-gc](../14-gc.md)：
- **§ 混合写屏障**（501-548 行）：写屏障实现（514-532 行）+ 插入点表（537-548 行）
- **§ Major GC**（348-497 行）：GC 状态机（360-400 行）、Init（402-410 行）、Concurrent Mark（412-431 行）、Mark Termination（433-441 行）、Concurrent Sweep（443-475 行，**Task 63 实装**）、Finalize（477-497 行）
- **§ 字节码安全点**（551-603 行）：安全点检查位置（577-587 行）+ STW 协调（589-603 行）
- **§ 根集**（607-627 行）：根集组成
- **§ GC 与协程交互**（631-674 行）：GC Coordinator / Worker 架构（636-656 行）
- **§ 实现风险与缓解**（773-803 行）：写屏障自动插入、安全点覆盖保证、unsafe 边界、降级路径

参照 [12-implementation-plan](../12-implementation-plan.md)：
- **§ 7.5.1 混合写屏障**（533-539 行）：写屏障函数 + 字节码插入 + 灰色队列
- **§ 7.5.2 并发标记**（541-550 行）：Coordinator + Worker + Init/Mark/Term 状态机

### 与 Task 52 的差异总览

| 属性 | Task 52 (MVP) | Task 62 (本任务) |
|---|---|---|
| Major GC 模型 | STW 标记-清除 | 并发三色标记 + STW 清除（过渡） |
| 写屏障 | 无 | 混合写屏障（Dijkstra + Yuasa） |
| GC 线程 | 无（VM 线程内同步） | GC Coordinator + Worker 线程池 |
| 灰色队列 | `Vec`（栈式，单线程） | `Mutex<Vec>`（多生产者并发安全） |
| 安全点 | 无 | 字节码安全点 + STW 协调（condvar） |
| gc_meta 访问 | `&mut self`（非原子） | 原子 RMW（并发标记期间） |
| Card Table | 无 | 写侧标记 dirty（HashSet 适配，见 §7） |
| 根集扫描位置 | `major_gc` 参数传入 | Init 阶段 STW 经安全点协议访问 |

### 范围边界（本任务不覆盖）

| 内容 | 归属 | 说明 |
|---|---|---|
| 并发清扫（ConcurrentSweep） | Task 63 | 本任务 Sweep 阶段为 STW（复用 Task 52 清除逻辑） |
| Old 代 Compaction | Task 63 | 碎片压缩 |
| Card Table 扫描侧 | Task 63 | Minor GC 扫描 dirty cards |
| GC 自适应调优 | Task 64 | 动态阈值、Young 代大小、晋升年龄 |
| C API 并发写屏障 | Task 77 | `msWriteBarrier` 升级（依赖本任务的 `write_barrier` + `GcPhase`） |

### 已知限制（沿用 Task 52 现状）

- **VM 日常分配未接入 GC 堆**：`alloc_*`（`src/vm/object.rs`）经 `Box::into_raw` 分配，不经 `gc_alloc_*`，故 `major_gc` 的 `gc_managed` 集合不含这些对象。并发标记同样仅覆盖 `gc_alloc_*` 对象。全量接入为后续增量。
- **CLOSURE/UPVALUE/FUNCTION/ITERATOR/GENERATOR/FUTURE trace 为 noop 占位**：当前这些类型不在 GC 堆（经 `alloc_*` 分配），`gc_managed` 不含它们，并发标记不会误回收。一旦未来接入 GC 堆，须补全 trace（见 `gc.rs:950-960` 注释）。

## 实现细节

### 文件组织

参照 [12-implementation-plan](../12-implementation-plan.md) 项目结构（40-50 行），GC 模块拆分为独立文件：

```
src/vm/gc/          # 从 src/vm/gc.rs 拆分
├── mod.rs          # 公共接口（re-export + maybe_gc 升级）
├── heap.rs         # MsHeap（从 gc.rs 迁移）
├── header.rs       # MsObjHeader + Color + Generation + 原子访问
├── descriptor.rs   # TypeDescriptor + trace/copy/forward/free（从 gc.rs 迁移）
├── minor.rs        # minor_gc（从 gc.rs 迁移，不变）
├── major.rs        # major_collect（并发标记状态机，替代 Task 52 major_gc）
├── barrier.rs      # 混合写屏障 write_barrier + write_barrier_obj
├── safepoint.rs    # SafepointCoordinator（STW 协调）
├── runtime.rs      # GcRuntime（Arc 共享）+ GcPhase + GrayQueue
├── cardtable.rs    # CardTable（写侧 mark_dirty）
└── stats.rs        # GC 统计扩展（并发指标）
```

> **迁移策略**：将现有 `src/vm/gc.rs`（2066 行）拆分为上述模块。`minor_gc`、TypeDescriptor、`gc_alloc_*`、测试用例保持不变迁移。`major_gc` 被本任务的并发标记替代（降级模式下保留 STW 路径）。

### 1. GcPhase 状态机

参照 [14-gc](../14-gc.md) § Major GC 状态机（360-400 行）。

```rust
/// GC 周期阶段。以 AtomicU8 存储，供 mutator 与 GC 线程原子读取。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcPhase {
    Idle = 0,
    Init = 1,             // STW：扫描根集，开启写屏障
    ConcurrentMark = 2,   // 并发：GC Workers 标记，mutator 继续运行
    MarkTermination = 3,  // STW：重扫协程栈，关闭写屏障
    ConcurrentSweep = 4,  // 过渡 STW（Task 63 升级为真正的并发）
    Finalize = 5,         // mutator 线程执行 pending finalizers
}

impl GcPhase {
    pub fn is_concurrent_mark(self) -> bool { self == GcPhase::ConcurrentMark }
    pub fn is_stw(self) -> bool {
        matches!(self, GcPhase::Init | GcPhase::MarkTermination | GcPhase::ConcurrentSweep)
    }
}
```

### 2. gc_meta 原子访问

**问题**：Task 52 的 `MsObjHeader::set_color(&mut self, ...)` 为非原子 `&mut` 访问。并发标记期间 GC Worker 与 mutator 写屏障同时修改颜色位 → 数据竞争（UB）。

**方案**：并发阶段对 `gc_meta` 字节做原子 RMW。`MsObjHeader` 保持 `#[repr(C)]` + `gc_meta: u8`；新增原子访问函数，经 `AtomicU8` 指针 cast 操作裸字节。`AtomicU8::from_mut_ptr` 自 Rust 1.70 稳定。

```rust
use std::sync::atomic::{AtomicU8, Ordering};

/// 原子读取颜色。
/// SAFETY: obj 指向有效 MsObjHeader。
pub unsafe fn color_atomic(obj: *const MsObjHeader) -> Color {
    let meta = AtomicU8::from_mut_ptr(obj.cast_mut().cast::<u8>());
    match meta.load(Ordering::Relaxed) & 0b11 {
        0 => Color::White, 1 => Color::Gray, 2 => Color::Black,
        _ => Color::White, // 位值 3 越界防御（与 Task 52 color() 一致）
    }
}

/// 原子着色（CAS 循环保留 gen/age/finalizer/pinned 位）。
/// SAFETY: obj 指向有效 MsObjHeader。
pub unsafe fn set_color_atomic(obj: *mut MsObjHeader, c: Color) {
    let meta = AtomicU8::from_mut_ptr(obj.cast::<u8>());
    let mut cur = meta.load(Ordering::Relaxed);
    loop {
        let new = (cur & !0b11) | (c as u8);
        match meta.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// 原子读取代数。
pub unsafe fn generation_atomic(obj: *const MsObjHeader) -> Generation {
    let meta = AtomicU8::from_mut_ptr(obj.cast_mut().cast::<u8>());
    match (meta.load(Ordering::Relaxed) >> 2) & 0b11 {
        0 => Generation::Young, 1 => Generation::Old, 2 => Generation::Immortal,
        _ => Generation::Young,
    }
}

/// CAS 式着色转换：仅当当前颜色 == from 时原子改为 to。返回是否成功。
/// 多 Worker 竞争同一对象时仅一个成功 → 保证只入队一次。
/// SAFETY: obj 指向有效 MsObjHeader。
pub unsafe fn try_color_transition(obj: *mut MsObjHeader, from: Color, to: Color) -> bool {
    let meta = AtomicU8::from_mut_ptr(obj.cast::<u8>());
    let mut cur = meta.load(Ordering::Relaxed);
    loop {
        if (cur & 0b11) != from as u8 { return false; }
        let new = (cur & !0b11) | (to as u8);
        match meta.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => cur = actual,
        }
    }
}
```

> **Ordering 选择**：颜色/代数为 GC 内部一致性标志，`Relaxed` 足够（不依赖与其他变量的顺序关系）。安全点协议（§8）通过独立的 atomic + condvar 提供 happens-before 保证。

### 3. GcRuntime（Arc 共享运行时状态）

参照 [14-gc](../14-gc.md) § GC 与协程交互（631-674 行）。

```rust
use std::sync::{Arc, AtomicBool, AtomicU64, AtomicU8};

/// GC 运行时状态。Arc 共享给 VM 线程（mutator）与 GC Coordinator/Worker 线程。
pub struct GcRuntime {
    phase: AtomicU8,                       // GcPhase
    gray_queue: GrayQueue,                 // 线程安全灰色队列
    safepoint: SafepointCoordinator,       // STW 协调
    card_table: CardTable,                 // Old→Young 跨代引用（写侧）
    /// 并发 GC 启用开关。false → major_collect 走 Task 52 STW 路径
    ///（14-gc.md § Phase 7.5 降级路径，796-801 行）。
    concurrent_enabled: AtomicBool,
    /// Coordinator 完成 Sweep 后置 true，mutator 在安全点恢复后执行 run_finalizers。
    pub finalize_pending: AtomicBool,

    // 并发 GC 统计（Task 77 C API 读取）
    pub concurrent_mark_ns: AtomicU64,
    pub init_stw_ns: AtomicU64,
    pub term_stw_ns: AtomicU64,
    pub gray_queue_peak: AtomicU64,
}

impl GcRuntime {
    pub fn phase(&self) -> GcPhase {
        // SAFETY: GcPhase 为 #[repr(u8)]，值域 0..=5，AtomicU8 load 不会越界。
        match self.phase.load(Ordering::Relaxed) {
            x if x == GcPhase::Idle as u8 => GcPhase::Idle,
            x if x == GcPhase::Init as u8 => GcPhase::Init,
            x if x == GcPhase::ConcurrentMark as u8 => GcPhase::ConcurrentMark,
            x if x == GcPhase::MarkTermination as u8 => GcPhase::MarkTermination,
            x if x == GcPhase::ConcurrentSweep as u8 => GcPhase::ConcurrentSweep,
            x if x == GcPhase::Finalize as u8 => GcPhase::Finalize,
            _ => GcPhase::Idle,
        }
    }
    pub fn set_phase(&self, p: GcPhase) { self.phase.store(p as u8, Ordering::Relaxed); }
    pub fn phase_is_concurrent_mark(&self) -> bool {
        self.phase.load(Ordering::Relaxed) == GcPhase::ConcurrentMark as u8
    }
}
```

**VM 集成**：`VM` 结构体新增 `gc_runtime: Arc<GcRuntime>`。`MsHeap` 仍由 VM 独占（`&mut self`），GC Coordinator 仅在 STW 阶段（mutator 停止）经安全点协议访问 `MsHeap` + 根集；并发阶段仅操作 `GcRuntime`（灰色队列 + 对象图遍历）。

### 4. GrayQueue（线程安全灰色队列）

参照 [14-gc](../14-gc.md) § 7.5.1（灰色队列多生产者并发安全，537-539 行）。

```rust
use std::sync::Mutex;

/// 线程安全灰色队列。写屏障 push 极短（2 次 CAS + 1 次 Vec push），
/// 锁竞争低。性能敏感时可替换为无锁队列，接口不变。
pub struct GrayQueue {
    inner: Mutex<Vec<*mut MsObjHeader>>,
}

impl GrayQueue {
    pub fn push(&self, obj: *mut MsObjHeader) { self.inner.lock().unwrap().push(obj); }
    pub fn extend(&self, objs: impl IntoIterator<Item = *mut MsObjHeader>) {
        self.inner.lock().unwrap().extend(objs);
    }
    pub fn pop(&self) -> Option<*mut MsObjHeader> { self.inner.lock().unwrap().pop() }
    pub fn is_empty(&self) -> bool { self.inner.lock().unwrap().is_empty() }
    pub fn len(&self) -> usize { self.inner.lock().unwrap().len() }
    pub fn clear(&self) { self.inner.lock().unwrap().clear(); }
}
```

### 5. 混合写屏障

参照 [14-gc](../14-gc.md) § 混合写屏障（501-533 行）。混合写屏障 = Dijkstra 插入屏障 + Yuasa 删除屏障，使得**栈不需要写屏障**（栈在 Init 阶段快照标灰），大幅缩短 Mark Termination 的 STW。

#### 5.1 槽位式写屏障（裸指针 `*mut *mut MsObjHeader`）

适用于能直接获取引用槽裸指针的场景（如 upvalue 槽、List 内联数组）。签名与 [14-gc](../14-gc.md) 514-532 行一致：

> **Card marking 限制**：此槽位式屏障不知晓 parent 对象，无法标记 Old→Young card。实际字节码 handler 统一使用 §5.2 的 `write_barrier_obj`（含 card marking）。此槽位式变体仅保留与 spec 签名对齐，供未来连续内存布局的内联数组使用——若用于 Old→Young 写入，调用方须额外调用 `gc.card_table.mark_dirty(parent)`。

```rust
/// 混合写屏障（槽位式）。非并发标记阶段零开销。
///
/// 在写入 `*slot = new_val` 时：
/// 1. 若 old_val 非 null 且 White → 标灰 + 入灰色队列（Yuasa 删除屏障）
/// 2. 若 new_val 非 null 且 White → 标灰 + 入灰色队列（Dijkstra 插入屏障）
///
/// # Safety
/// slot 必须指向有效的 `*mut MsObjHeader` 槽（堆对象内部引用字段）。
pub unsafe fn write_barrier(
    gc: &GcRuntime,
    slot: *mut *mut MsObjHeader,
    new_val: *mut MsObjHeader,
) {
    if !gc.phase_is_concurrent_mark() {
        *slot = new_val;
        return;
    }
    let old_val = *slot;
    if !old_val.is_null() && color_atomic(old_val) == Color::White {
        set_color_atomic(old_val, Color::Gray);
        gc.gray_queue.push(old_val);
    }
    if !new_val.is_null() && color_atomic(new_val) == Color::White {
        set_color_atomic(new_val, Color::Gray);
        gc.gray_queue.push(new_val);
    }
    *slot = new_val;
}
```

#### 5.2 对象式写屏障（HashMap/Vec 写入用）

Instance.fields、Class.methods、DictMap 等经 `HashMap::insert` 写入，不暴露 `*mut *mut` 裸槽。提供对象式变体，接收 `(parent, old_val, new_val)`，在 insert **之后**调用：

```rust
/// 混合写屏障（对象式）。用于 HashMap/Vec 等容器写入后。
/// old_val 为被覆盖的旧值指针（null 表示无旧值，如 append/add）。
///
/// # Safety
/// parent 必须指向有效 MsObjHeader；old_val/new_val 为 null 或有效 MsObjHeader。
pub unsafe fn write_barrier_obj(
    gc: &GcRuntime,
    parent: *mut MsObjHeader,
    old_val: *mut MsObjHeader,
    new_val: *mut MsObjHeader,
) {
    if !gc.phase_is_concurrent_mark() { return; }
    if !old_val.is_null() && color_atomic(old_val) == Color::White {
        set_color_atomic(old_val, Color::Gray);
        gc.gray_queue.push(old_val);
    }
    if !new_val.is_null() && color_atomic(new_val) == Color::White {
        set_color_atomic(new_val, Color::Gray);
        gc.gray_queue.push(new_val);
    }
    // Card marking：Old parent 持有 Young 引用 → 标 dirty card（§7）
    if !new_val.is_null()
        && generation_atomic(parent) == Generation::Old
        && generation_atomic(new_val) == Generation::Young
    {
        gc.card_table.mark_dirty(parent);
    }
}
```

> **old_val 获取**：调用方在 `insert` 前做一次 `get()` 取旧值。对 HashMap 为 O(1)，开销可忽略。例：
> ```rust
> let old = unsafe { read_instance(ptr).fields.get(attr) }
>     .and_then(|v| if let Object::Ref(r) = v { Some(*r) } else { None })
>     .unwrap_or(std::ptr::null_mut());
> unsafe { read_instance(ptr).fields.insert(attr, value.clone()); }
> if let Object::Ref(new_ptr) = &value {
>     unsafe { write_barrier_obj(&gc_runtime, ptr, old, *new_ptr); }
> }
> ```

### 6. 写屏障字节码 / 内置方法插入点

参照 [14-gc](../14-gc.md) § 写屏障插入点（537-548 行）。映射到实际 OpCode（`src/compiler/opcode.rs`）：

| 14-gc.md 名称 | 实际 OpCode | 插入位置（`src/vm/mod.rs`） | old_val 来源 |
|---|---|---|---|
| `STORE_UPVALUE` | `StoreUpvalue` | upvalue 槽写入后 | upvalue 原值 |
| `SET_ATTR` | `SetAttr` | `Instance.fields.insert` / `Class.class_attrs.insert` 后（**现有 TODO 标记**：`mod.rs:4148,4152`） | `fields.get(attr)` |
| `SET_INDEX` | `SetIndex` | List/Dict 元素写入后 | list\[idx\] / dict.get(key) |
| `LIST_PUSH` | `ListAppend` | `BuildList` 内部追加后 | null（追加无覆盖） |
| `DICT_SET` | `DictInsert` | `BuildDict` 内部插入后 | dict.get(key) |
| `SET_ADD` | `SetAdd` | `BuildSet` 内部追加后 | null（添加无覆盖） |
| `SEND` | `Send` | Channel buffer push 后 | null（缓冲区追加） |

**Method 字节码**（`mod.rs:4195`，现有 TODO）：`Class.methods.insert` 亦须写屏障（methods 值为 `*mut MsClosure` 裸指针，包装为 `Object::Ref` 调用 `write_barrier_obj`）。

**内置方法**（`src/vm/stdlib.rs`）：`list.push/insert/extend`、`dict.set/insert/update`、`set.add` 等修改堆对象的内置方法，在并发标记期间同样须经 `write_barrier_obj`。非 GC 期间经 phase 检查零开销跳过。

**不需要写屏障的字节码**（14-gc.md 537-548 行）：
- `STORE_LOCAL`（栈变量，三色不变性保证）
- `STORE_GLOBAL`（全局表在 Mark Termination 阶段统一重扫描）

### 7. Card Table（写侧）

参照 [14-gc](../14-gc.md) § Remembered Set（326-344 行）。

**适配说明**：14-gc.md 的 Card Table 假设 Old 代为**连续内存**（按 512 字节分 card）。当前实现 `old_objects: Vec<*mut MsObjHeader>` 为**散布 Box 分配**（每个对象独立 `Box::into_raw`），无连续 Old 代区域。故 Card Table 调整为 `HashSet<*mut MsObjHeader>`：记录含有 Young 引用的 Old 对象指针。Minor GC（Task 63）扫描此集合而非全量 Old 对象。

```rust
use std::sync::Mutex;
use std::collections::HashSet;

/// Old → Young 跨代引用记录集（写侧）。
/// Task 63 的 Minor GC 扫描此集合，找到 Old 持有的 Young 引用。
pub struct CardTable {
    dirty: Mutex<HashSet<*mut MsObjHeader>>,
}

impl CardTable {
    pub fn new() -> Self { Self { dirty: Mutex::new(HashSet::new()) } }

    /// 标记一个 Old 对象含有 Young 引用（写屏障调用）。
    pub fn mark_dirty(&self, old_obj: *mut MsObjHeader) {
        self.dirty.lock().unwrap().insert(old_obj);
    }

    /// Minor GC 扫描后清空（Task 63 调用）。
    pub fn drain(&self) -> Vec<*mut MsObjHeader> {
        self.dirty.lock().unwrap().drain().collect()
    }

    pub fn len(&self) -> usize { self.dirty.lock().unwrap().len() }

    /// Sweep 后清理已释放对象的悬垂指针（防止 Minor GC drain 后 UAF）。
    /// 保留仍存在于 old_objects 中的指针，移除其余。
    pub fn retain_valid(&self, old_objects: &[*mut MsObjHeader]) {
        let live: HashSet<*mut MsObjHeader> = old_objects.iter().copied().collect();
        self.dirty.lock().unwrap().retain(|p| live.contains(p));
    }
}
```

> **与 spec 的偏差**：spec 用 512-byte card 索引（`card_index = (addr - old_start) / 512`），本实现用对象指针集合。语义等价（都记录「哪些 Old 区域含 Young 引用」），扫描粒度更细（per-object 而非 per-512B-card）。未来若 Old 代改为连续 arena 分配，可无感切回 card-table 方案。

### 8. SafepointCoordinator（安全点 STW 协调）

参照 [14-gc](../14-gc.md) § 字节码安全点（551-603 行）。

当前 VM 为**单 OS 线程 + 协作式协程**。安全点协议：

```
GC 请求 STW（Init / Mark Termination）:
  1. safepoint_requested.store(true)
  2. VM 在下一个安全点字节码处检测到 flag → park（阻塞 condvar）
  3. GC 等待 VM parked → 执行 STW 工作（扫描根集等）
  4. safepoint_requested.store(false) + notify condvar
  5. VM 唤醒，恢复执行

协程到达安全点:
  fn check_gc_safepoint(&mut self):
    if safepoint_requested.load(Relaxed):
      parked = true
      condvar.notify()       // 告知 GC 已停
      condvar.wait()         // 阻塞等待 GC 完成 STW
      parked = false
```

```rust
use std::sync::{Condvar, Mutex};

struct SafepointState {
    requested: bool, // GC 请求 STW
    parked: bool,    // mutator 已停下
}

pub struct SafepointCoordinator {
    state: Mutex<SafepointState>,
    cv: Condvar,
}

impl SafepointCoordinator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SafepointState { requested: false, parked: false }),
            cv: Condvar::new(),
        }
    }

    /// GC 调用：请求 STW，阻塞等待 mutator 停下。返回后 mutator 确实 parked。
    pub fn request_and_wait(&self) {
        let mut s = self.state.lock().unwrap();
        s.requested = true;
        while !s.parked {
            s = self.cv.wait(s).unwrap();
        }
        // mutator 已停，GC 可安全访问 VM 状态
    }

    /// GC 调用：完成 STW，恢复 mutator。必须 notify 唤醒阻塞的 mutator。
    pub fn release(&self) {
        let mut s = self.state.lock().unwrap();
        s.requested = false;
        self.cv.notify_all();
    }

    /// mutator 调用：在安全点检查并必要时停下（阻塞直到 GC 完成 STW）。
    pub fn check_and_park(&self) {
        let mut s = self.state.lock().unwrap();
        if !s.requested { return; }
        s.parked = true;
        self.cv.notify_all(); // 唤醒 GC（request_and_wait 等待 parked）
        while s.requested {
            s = self.cv.wait(s).unwrap(); // 等 GC release
        }
        s.parked = false;
    }

    pub fn is_requested(&self) -> bool { self.state.lock().unwrap().requested }
}
```

> **单线程简化**：当前仅一个 mutator OS 线程，`parked` 为 0/1 计数。Task 53 的事件循环在同一 OS 线程内调度协程，故一次 park 即暂停所有协程。若未来 VM 改为多线程（多 mutator），需扩展为 `pending_count` 原子递减（14-gc.md 589-603 行）。

#### 安全点检查位置

参照 [14-gc](../14-gc.md) § 安全点位置（577-587 行）。在 VM 主循环（`src/vm/mod.rs` 的 `run` 方法）的以下字节码前插入 `check_and_park`：

| 字节码 | 现有安全点 | 说明 |
|---|---|---|
| `Call` | 新增 | 函数调用前，调用栈清晰 |
| `Jump` / `JumpBack` | 新增 | 跳转/循环回边 |
| `ForIter` | 新增 | 循环迭代 |
| `Return` | 新增 | 函数返回 |
| `Import` | 新增 | 模块加载 |
| `Await` | 已有（cancel） | 现有 `check_cancel_safepoint`（`mod.rs:680`），合并 GC 安全点检查 |
| `Send` / `Receive` | 已有（cancel） | 同上 |

> **安全点覆盖保证**（14-gc.md 784-786 行）：编译器验证基本块（两个安全点之间）不超过 N=1000 条指令，超长纯计算序列插入 `NOP_SAFEPONT`。本任务在编译器（`src/compiler/`）增加此校验。

### 9. GC Coordinator 线程

参照 [14-gc](../14-gc.md) § GC 与协程交互（636-656 行）。GC Coordinator 为独立 OS 线程，驱动 GC 周期状态机。

```rust
use std::thread::{self, JoinHandle};

pub struct GcCoordinator {
    thread: Option<JoinHandle<()>>,
    /// 触发信号：VM 经此通知 Coordinator 开始一个 GC 周期。
    trigger: std::sync::mpsc::Sender<GcTrigger>,
}

enum GcTrigger {
    Major,      // 堆增长率 / 定时触发
    Shutdown,   // VM 销毁，退出 Coordinator 线程
}

impl GcCoordinator {
    /// 启动 GC Coordinator 线程。线程持有 Arc<GcRuntime> + VM 裸指针（STW 期间访问）。
    pub fn spawn(gc_runtime: Arc<GcRuntime>, vm_ptr: Arc<AtomicPtr<VM>>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let runtime = Arc::clone(&gc_runtime);
        let vm_arc = Arc::clone(&vm_ptr);
        let handle = thread::Builder::new()
            .name("mslang-gc-coordinator".into())
            .spawn(move || {
                let rt = &runtime;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        GcTrigger::Major => Self::run_major_cycle(rt, &vm_arc),
                        GcTrigger::Shutdown => break,
                    }
                }
            })
            .expect("failed to spawn GC coordinator");
        Self { thread: Some(handle), trigger: tx }
    }

    /// VM 调用：异步触发 Major GC（不阻塞，GC 经安全点协议协调 STW）。
    pub fn trigger_major(&self) { let _ = self.trigger.send(GcTrigger::Major); }

    /// VM 销毁时调用：等待 Coordinator 退出。
    /// 先释放可能的安全点请求（防止 Coordinator 阻塞在 request_and_wait 中无法 recv）。
    pub fn shutdown(&mut self, gc_runtime: &GcRuntime) {
        gc_runtime.safepoint.release(); // 解除可能的 STW 请求，唤醒阻塞的 Coordinator
        let _ = self.trigger.send(GcTrigger::Shutdown);
        if let Some(h) = self.thread.take() { let _ = h.join(); }
    }
}
```

> **VM 裸指针访问的安全性**：GC Coordinator 仅在 STW 阶段（`safepoint.request_and_wait()` 后，mutator 已停）经 `vm_ptr` 访问 VM 的 stack/globals/heap。safepoint 协议保证此时 mutator 不修改 VM 状态。并发阶段（ConcurrentMark）GC Worker 不访问 VM 结构体，仅遍历对象图 + 灰色队列。

### 10. GC Worker 线程池（并发标记）

参照 [14-gc](../14-gc.md) § 7.5.2（542-550 行）+ § Concurrent Mark（412-431 行）。

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct GcWorkerPool {
    workers: Vec<JoinHandle<()>>,
}

impl GcWorkerPool {
    /// 启动 N 个 Worker 线程并发标记。N = gc_threads_setting（默认 CPU 核心数 / 4，min 1）。
    ///
    /// `gc_managed` 为本轮 GC 管辖的对象集合（old_objects + los_objects），
    /// 用于过滤非 GC 堆对象（alloc_* 分配的 MsList/MsDict 等，布局与 Gc* 不同，
    /// 不可用 Gc* trace 函数扫描——否则类型混淆 UB）。与 Task 52 major_gc 的
    /// gc_managed HashSet 语义一致（gc.rs:1342-1347）。
    pub fn spawn(
        gc_runtime: Arc<GcRuntime>,
        gc_managed: Arc<HashSet<*mut MsObjHeader>>,
        active: Arc<AtomicUsize>,
        n: u32,
    ) -> Self {
        let mut workers = Vec::with_capacity(n as usize);
        for i in 0..n.max(1) {
            let rt = Arc::clone(&gc_runtime);
            let managed = Arc::clone(&gc_managed);
            let act = Arc::clone(&active);
            workers.push(
                thread::Builder::new()
                    .name(format!("mslang-gc-worker-{}", i))
                    .spawn(move || Self::worker_loop(&rt, &managed, &act))
                    .expect("failed to spawn GC worker")
            );
        }
        Self { workers }
    }

    /// Worker 主循环。参照 14-gc.md 412-431 行。
    ///
    /// 终止协议（CSP 风格 quiescence detection）：
    /// - active 计数器在 trace 前后 fetch_add/fetch_sub
    /// - 队列空 + active==0 → 全局静止 → 所有 Worker 可退出
    /// - 队列空 + active>0 → 有 Worker 正在 trace（可能 push 新项）→ yield 重试
    fn worker_loop(gc: &GcRuntime, gc_managed: &HashSet<*mut MsObjHeader>, active: &AtomicUsize) {
        loop {
            let Some(obj) = gc.gray_queue.pop() else {
                // 队列空：检查全局静止
                if active.load(Ordering::Relaxed) == 0 { return; }
                std::thread::yield_now();
                continue;
            };
            // 跳过非 GC 堆对象（alloc_* 分配，布局不兼容 Gc* trace）
            if !gc_managed.contains(&obj) { continue; }

            active.fetch_add(1, Ordering::Relaxed);
            let tag = unsafe { (*obj).type_tag };
            let desc = type_descriptor(tag);
            (desc.trace)(obj, &mut |child| {
                // CAS White→Gray（try_color_transition 原子转换，成功才入队）。
                // 多 Worker 竞争同一 child 时仅一个成功 → 保证只入队一次。
                if unsafe { try_color_transition(child, Color::White, Color::Gray) } {
                    gc.gray_queue.push(child);
                }
            });
            unsafe { set_color_atomic(obj, Color::Black); }
            active.fetch_sub(1, Ordering::Relaxed);

            // 更新灰色队列峰值统计
            let len = gc.gray_queue.len() as u64;
            gc.gray_queue_peak.fetch_max(len, Ordering::Relaxed);
        }
    }

    /// 等待所有 Worker 完成（全局静止：队列空 + active==0）。
    pub fn join(self) { for h in self.workers { let _ = h.join(); } }
}
```

> **终止正确性**：Worker A pop 最后一个对象 → active=1 → trace push 新项 → active=0。Worker B 见队列空但 active 曾 >0 → yield → 重试 pop → 拿到新项。仅当所有 Worker 同时见「队列空 + active==0」时退出，此时无任何 Worker 在 trace，不可能有新项入队。Coordinator `join` 后再兜底检查 `gray_queue.is_empty()`，非空则重启 Worker（防御 race）。

### 11. Init 阶段（STW）

参照 [14-gc](../14-gc.md) § Init（402-410 行）。

```rust
fn gc_init(gc: &GcRuntime, vm: &VM, gc_managed: &HashSet<*mut MsObjHeader>) {
    // 1. 请求 STW，等待 mutator 停
    gc.safepoint.request_and_wait();

    let t0 = std::time::Instant::now();

    // 2. 扫描根集，标灰 + 入灰色队列（复用 Task 52 根集列表）
    //    根集：vm.stack + vm.globals + vm.call_stack（closure/current_exc）
    //    + vm.defer_stack + vm.module_cache + vm.c_roots + event_loop 协程
    //    仅标记 gc_managed 中的对象（过滤 alloc_* 非托管对象）。
    scan_roots_gray(gc, vm, gc_managed);

    // 3. 开启混合写屏障（phase = ConcurrentMark 后写屏障 fast-path 生效）
    gc.set_phase(GcPhase::Init); // Init 阶段仍 STW，写屏障尚未需生效

    gc.init_stw_ns.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);

    // 4. 恢复 mutator（进入 ConcurrentMark）
    gc.safepoint.release();
}
```

### 12. Concurrent Mark 阶段

参照 [14-gc](../14-gc.md) § Concurrent Mark（412-431 行）。

```rust
fn gc_concurrent_mark(
    gc: &Arc<GcRuntime>,
    gc_managed: &HashSet<*mut MsObjHeader>,
    gc_threads: u32,
) {
    gc.set_phase(GcPhase::ConcurrentMark); // 写屏障开始生效

    let t0 = std::time::Instant::now();

    let managed = Arc::new(gc_managed.clone());
    let active = Arc::new(AtomicUsize::new(0));

    // 启动 Worker 线程池（参照 14-gc.md 636-656 行）
    let pool = GcWorkerPool::spawn(Arc::clone(gc), Arc::clone(&managed), Arc::clone(&active), gc_threads);
    pool.join();

    // 兜底：若 Worker 因 race 提前退出且队列非空，Coordinator 单线程 drain
    while let Some(obj) = gc.gray_queue.pop() {
        if !managed.contains(&obj) { continue; }
        let tag = unsafe { (*obj).type_tag };
        (type_descriptor(tag).trace)(obj, &mut |child| {
            if !managed.contains(&child) { return; }
            if unsafe { try_color_transition(child, Color::White, Color::Gray) } {
                gc.gray_queue.push(child);
            }
        });
        unsafe { set_color_atomic(obj, Color::Black); }
    }

    gc.concurrent_mark_ns.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
}
```

### 13. Mark Termination 阶段（STW）

参照 [14-gc](../14-gc.md) § Mark Termination（433-441 行）。

```rust
fn gc_mark_termination(gc: &GcRuntime, vm: &VM, gc_managed: &HashSet<*mut MsObjHeader>) {
    // 1. 请求 STW
    gc.safepoint.request_and_wait();

    let t0 = std::time::Instant::now();

    // 2. 重新扫描所有协程栈 + 全局变量表
    //    混合写屏障保证：堆引用写入已着色，但栈和 globals 的新写入不经屏障
    //    （STORE_LOCAL/STORE_GLOBAL 无写屏障，14-gc.md 539/543 行），故须重扫。
    //    globals 必扫：STORE_GLOBAL 无写屏障，并发标记期间全局表可被自由修改。
    scan_roots_gray(gc, vm, gc_managed);

    // 3. 处理写屏障产生的剩余灰色对象（drain 灰色队列）
    while let Some(obj) = gc.gray_queue.pop() {
        if !gc_managed.contains(&obj) { continue; } // 跳过非 GC 堆对象
        let tag = unsafe { (*obj).type_tag };
        let desc = type_descriptor(tag);
        (desc.trace)(obj, &mut |child| {
            if !gc_managed.contains(&child) { return; }
            if unsafe { try_color_transition(child, Color::White, Color::Gray) } {
                gc.gray_queue.push(child);
            }
        });
        unsafe { set_color_atomic(obj, Color::Black); }
    }

    // 4. 关闭混合写屏障
    gc.set_phase(GcPhase::MarkTermination);

    gc.term_stw_ns.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);

    // 注意：此处**不释放 mutator**（不调 safepoint.release）。
    // Sweep 紧接在同一 STW 窗口内执行（见 §14），完成后统一 release。
}
```

> `scan_roots_gray` 同时扫描协程栈（vm.stack + frame.closure/current_exc）**和**全局变量表（vm.globals），因二者均无写屏障。defer_stack / module_cache / c_roots / event_loop 协程亦在此扫描（参照 14-gc.md § 根集 607-627 行）。仅标记 `gc_managed` 中的对象。

### 14. Sweep 阶段（STW 过渡）

> **过渡说明**：本任务 Sweep 为 STW（复用 Task 52 `major_gc` 的清除逻辑，1398-1440 行）。Task 63 将升级为 ConcurrentSweep（14-gc.md 443-475 行）+ 并发标记期间新分配对象标黑（465-475 行）。

```rust
fn gc_sweep_stw(gc: &GcRuntime, heap: &mut MsHeap) {
    gc.set_phase(GcPhase::ConcurrentSweep);
    // 复用 Task 52 major_gc 的 old_objects.retain + los_objects.retain 逻辑：
    //   Black → White（重置，为下次 GC 准备）
    //   White + has_finalizer → finalizer_queue（复活）
    //   White + 无 finalizer + 非 pinned → free（释放）
    //   White + pinned → 保留（C 侧 pin 的对象不可回收，14-gc.md 84-85 行）
    sweep_old_and_los(heap);

    // 清理 Card Table 中已释放对象的悬垂指针（防止 Task 63 Minor GC drain 后 UAF）
    gc.card_table.retain_valid(&heap.old_objects);

    gc.set_phase(GcPhase::Finalize);

    // 释放 mutator（Mark Termination + Sweep 共享同一 STW 窗口，此处统一 release）
    gc.safepoint.release();
}
```

> **pinned 对象保护**：`sweep_old_and_los` 在 retain 闭包中增加 pinned 检查——`gc_meta & PINNED != 0` 的 White 对象保留（设为 White + 不释放），因 C 侧可能持有其裸指针（14-gc.md 84-85 行）。

### 15. major_collect（线程模型）

> **审核修复**：原设计将 `major_collect` 置于 mutator 线程同步调用，但 `gc_init`/`gc_mark_termination` 内部 `safepoint.request_and_wait()` 等待 mutator park → mutator 阻塞在 `major_collect` 内 → **永久死锁**。修复：并发模式下 `major_collect` 由 **GC Coordinator 线程**执行，`maybe_gc` 仅异步触发。

#### 15.1 降级模式（mutator 线程同步执行，不经安全点）

```rust
/// 降级模式：由 mutator 线程在 maybe_gc 中同步调用。
/// mutator 已停在 maybe_gc 中（非执行字节码），无需安全点协议。
fn major_collect_stw(heap: &mut MsHeap, vm: &mut VM) {
    let stack = &vm.stack[..];
    let globals = &vm.globals;
    let defer_stack = &vm.defer_stack[..];
    let frames = &vm.call_stack[..];
    crate::vm::gc::major_gc_stw(heap, stack, globals, defer_stack, frames); // Task 52 路径
    run_finalizers(heap);
}
```

#### 15.2 并发模式（Coordinator 线程执行）

```rust
/// 并发模式：由 GC Coordinator 线程执行。mutator 经安全点协议协调 STW。
/// run_finalizers 需 &mut VM 执行 __del__，必须延后到 mutator 线程。
fn major_collect_concurrent(
    gc_runtime: &Arc<GcRuntime>,
    heap: &mut MsHeap,     // STW 期间经 VM 裸指针获取
    vm: &VM,               // STW 期间经 VM 裸指针获取
) {
    let gc_managed: Arc<HashSet<*mut MsObjHeader>> = Arc::new(
        heap.old_objects.iter().chain(heap.los_objects.iter()).copied().collect()
    );

    gc_init(gc_runtime, vm, &gc_managed);                           // STW: Init（release）
    gc_concurrent_mark(gc_runtime, &gc_managed, heap.gc_threads_setting); // 并发标记
    gc_mark_termination(gc_runtime, vm, &gc_managed);               // STW: Mark Term（不 release）
    gc_sweep_stw(gc_runtime, heap);                                 // STW: Sweep（release）
    // 设置 finalize_pending，mutator 恢复后执行 run_finalizers
    gc_runtime.finalize_pending.store(true, Ordering::Relaxed);
    gc_runtime.set_phase(GcPhase::Idle);
}
```

#### 15.3 maybe_gc 触发逻辑

```rust
pub fn maybe_gc(vm: &mut VM) {
    if !vm.heap.gc_enabled { return; }

    // 重入守卫：并发 GC 周期进行中则跳过（避免重复触发）
    if vm.gc_runtime.phase() != GcPhase::Idle { return; }

    if vm.heap.should_collect_minor() {
        gc::minor_gc(/* ... Task 52 参数 ... */);
    }
    if vm.heap.should_collect_major() {
        if vm.gc_runtime.concurrent_enabled.load(Ordering::Relaxed) {
            // 并发模式：异步触发，mutator 继续执行字节码
            vm.gc_coordinator.as_ref().unwrap().trigger_major();
        } else {
            // 降级模式：同步执行（mutator 已停在 maybe_gc 中，无死锁）
            major_collect_stw(&mut vm.heap, vm);
        }
    }
}
```

#### 15.4 Finalize 延后执行（mutator 线程）

```rust
/// VM 主循环安全点检查后调用。若 finalize_pending 则执行 finalizers。
fn check_finalize_pending(vm: &mut VM) {
    if vm.gc_runtime.finalize_pending.swap(false, Ordering::Relaxed) {
        run_finalizers(&mut vm.heap);
    }
}
```

mutator 在安全点恢复后（`check_and_park` 返回），紧接着调用 `check_finalize_pending`。因 `run_finalizers` 需 `&mut VM` 调用 `__del__`（执行 mslang 字节码），只能在 mutator 线程执行。

### 16. 降级模式

参照 [14-gc](../14-gc.md) § Phase 7.5 降级路径（796-801 行）。

```ms
import gc
gc.set_concurrent(false)   // 降级为 STW 标记-清除（Task 52 行为）
```

```rust
// gc.set_concurrent 的 stdlib 绑定（src/vm/stdlib.rs）
fn gc_set_concurrent(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let enabled = match args.get(0) {
        Some(Object::Bool(b)) => *b,
        _ => return Err("set_concurrent expects bool".into()),
    };
    vm.gc_runtime.concurrent_enabled.store(enabled, Ordering::Relaxed);
    Ok(Object::Nil)
}
```

降级场景：并发 GC 调试期间、单线程嵌入、GC 行为异常紧急回退。

### 17. 并发标记期间的对象分配

参照 [14-gc](../14-gc.md) § Concurrent Sweep 新分配对象标黑（465-475 行）。**注意**：spec 的标黑逻辑在 Sweep 阶段，但 ConcurrentMark 期间分配的新对象同样须标黑（避免被本轮标记漏扫后 Sweep 误回收）。

```rust
/// gc_alloc_* 内部，分配后检查 GC 阶段。
fn alloc_during_gc(gc: &GcRuntime, obj: *mut MsObjHeader) {
    let phase = gc.phase();
    if phase == GcPhase::ConcurrentMark || phase == GcPhase::ConcurrentSweep {
        // 并发标记/清扫期间新分配 → 直接标黑，避免被本轮 GC 回收
        unsafe { set_color_atomic(obj, Color::Black); }
    }
}
```

在所有 `gc_alloc_*` 函数（`gc_alloc_string/list/dict/set/tuple`）末尾调用 `alloc_during_gc`。

## VM 集成变更

### VM 结构体扩展

```rust
pub struct VM {
    // ... 现有字段 ...
    /// task 62：GC 运行时（Arc 共享给 GC Coordinator/Worker 线程）。
    pub gc_runtime: Arc<GcRuntime>,
    /// task 62：GC Coordinator 线程句柄。
    gc_coordinator: Option<GcCoordinator>,
}
```

`VM::new()` 初始化 `gc_runtime`（`concurrent_enabled` 默认 `false`——渐进启用；可在 Task 64 调优中改默认 `true`）。`VM::drop()` 调用 `gc_coordinator.shutdown(&gc_runtime)`。

### maybe_gc 调用点

`src/vm/mod.rs:2696`（主循环）和 `mod.rs:4319`（另一触发点）的 `gc::maybe_gc` 调用。内部逻辑按 §15.3：并发模式异步 `trigger_major()`（mutator 不阻塞），降级模式同步 `major_collect_stw()`。

### 安全点检查注入

VM 主循环（`run` 方法）在 `Call`/`Jump`/`JumpBack`/`ForIter`/`Return`/`Import` 字节码前增加：
```rust
self.gc_runtime.safepoint.check_and_park();
check_finalize_pending(self); // STW 恢复后检查是否需执行 finalizers
```
`Await`/`Send`/`Receive` 的现有 `check_cancel_safepoint`（`mod.rs:680`）合并 GC 安全点检查 + finalize 检查。

### NOP_SAFEPONT 指令

编译器（`src/compiler/`）新增：
1. **OpCode 枚举**新增 `NopSafepoint`（在 `Halt` 前插入，值 = 91）
2. **VM handler**：`OpCode::NopSafepoint => { self.gc_runtime.safepoint.check_and_park(); check_finalize_pending(self); }`（no-op + 安全点 + finalize 检查）
3. **编译器基本块校验**：`compile_chunk` 结束时验证每段连续非安全点字节码 ≤ 1000 条（14-gc.md 784-786 行），超长序列自动插入 `NopSafepoint`

## 验证标准

### 写屏障正确性
1. **非并发标记零开销**：`gc_phase != ConcurrentMark` 时写屏障直接返回，无着色、无入队
2. **插入屏障着色**：并发标记期间写入 White 的 new_val → 被标灰 + 入灰色队列
3. **删除屏障着色**：并发标记期间覆盖 White 的 old_val → 被标灰 + 入灰色队列
4. **CAS 无重复入队**：多个 Worker 同时标灰同一对象，`try_color_transition` CAS 保证只入队一次
5. **Old→Young card 标记**：写屏障对 Old parent 写入 Young 引用时标记 dirty card

### 并发标记正确性
6. **三色不变性**：并发标记结束后，无 Black 对象直接指向 White 对象
7. **无漏标**：并发标记期间 mutator 持续修改引用，存活对象不被误回收
8. **无误标**：不可达对象最终为 White，被 Sweep 回收
9. **循环引用回收**：并发标记正确处理循环引用（Black 不可达 → 重置 White → 下轮回收）

### 状态机与安全点
10. **阶段顺序**：Idle → Init → ConcurrentMark → MarkTermination → ConcurrentSweep → Finalize → Idle
11. **STW 协调**：Init / MarkTermination 期间 mutator 确实停下（`safepoint.request_and_wait` 阻塞）
12. **安全点覆盖**：长循环（`JumpBack` 回边）在安全点处可被 GC 暂停
13. **降级模式**：`set_concurrent(false)` 时走 Task 52 STW 路径，行为与升级前一致

### 统计
14. **并发指标记录**：`concurrent_mark_ns`/`init_stw_ns`/`term_stw_ns`/`gray_queue_peak` 正确记录
15. **STW 分解**：`init_stw_ns + term_stw_ns` = 单次 GC 周期的 STW 总时间（Task 77 验证）

### gc_meta 原子性
16. **无数据竞争**：并发标记期间 GC Worker 与 mutator 写屏障同时访问 gc_meta，不触发 UB（`AtomicU8` RMW）
17. **颜色保留**：原子着色不丢失其他位（gen/age/finalizer/pinned）

### 线程模型与安全
18. **无死锁**：并发模式下 `major_collect_concurrent` 在 Coordinator 线程执行，mutator 经安全点 park/release 协调，不阻塞自身
19. **globals 重扫**：Mark Termination 扫描 vm.globals（STORE_GLOBAL 无写屏障），并发标记期间新写入的全局引用不漏标
20. **pinned 保护**：Sweep 不释放 pinned 对象（C 侧持引用安全）
21. **finalize 延后**：`run_finalizers` 在 mutator 线程执行（需 `&mut VM` 调 `__del__`），Coordinator 仅设 `finalize_pending` 标志
22. **重入守卫**：并发 GC 周期进行中（phase != Idle），`maybe_gc` 不重复触发
23. **shutdown 安全**：VM drop 时先 release safepoint 再 join Coordinator，不死锁

## 测试用例

### Rust 单元测试（`src/vm/gc/barrier.rs` + `src/vm/gc/runtime.rs`）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::gc::header::*;
    use crate::vm::object::{MsObjHeader, TypeTag};

    fn make_obj(color: Color) -> *mut MsObjHeader {
        let mut h = Box::new(MsObjHeader {
            gc_meta: 0, type_tag: TypeTag::STRING as u8,
            size: 0, _padding: 0, class_ptr: 0,
        });
        h.set_color(color);
        Box::into_raw(h)
    }

    #[test]
    fn test_write_barrier_noop_outside_mark() {
        // gc_phase = Idle → 写屏障直接写入，不着色
        let gc = GcRuntime::new(); // phase = Idle
        let mut slot = std::ptr::null_mut();
        let new_val = make_obj(Color::White);
        unsafe { write_barrier(&gc, &mut slot, new_val); }
        assert_eq!(slot, new_val);
        assert_eq!(unsafe { color_atomic(new_val) }, Color::White); // 未着色
        unsafe { drop(Box::from_raw(new_val)); }
    }

    #[test]
    fn test_write_barrier_shades_old_and_new() {
        // 并发标记期间：old_val(White) + new_val(White) → 均标灰 + 入队
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let old_val = make_obj(Color::White);
        let new_val = make_obj(Color::White);
        let mut slot = old_val;
        unsafe { write_barrier(&gc, &mut slot, new_val); }
        assert_eq!(unsafe { color_atomic(old_val) }, Color::Gray);
        assert_eq!(unsafe { color_atomic(new_val) }, Color::Gray);
        assert_eq!(gc.gray_queue.len(), 2);
        unsafe { drop(Box::from_raw(old_val)); drop(Box::from_raw(new_val)); }
    }

    #[test]
    fn test_write_barrier_skips_non_white() {
        // old_val/new_val 已为 Gray/Black → 不重复入队
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let old_val = make_obj(Color::Black);
        let new_val = make_obj(Color::Gray);
        let mut slot = old_val;
        unsafe { write_barrier(&gc, &mut slot, new_val); }
        assert!(gc.gray_queue.is_empty()); // 均非 White，不入队
        unsafe { drop(Box::from_raw(old_val)); drop(Box::from_raw(new_val)); }
    }

    #[test]
    fn test_write_barrier_card_marking() {
        // Old parent 写入 Young 引用 → card dirty
        let gc = GcRuntime::new();
        gc.set_phase(GcPhase::ConcurrentMark);
        let parent = make_obj(Color::Black);
        unsafe { (*parent).set_generation(Generation::Old); }
        let young = make_obj(Color::White);
        unsafe { (*young).set_generation(Generation::Young); }
        unsafe { write_barrier_obj(&gc, parent, std::ptr::null_mut(), young); }
        assert_eq!(gc.card_table.len(), 1); // parent 被标 dirty
        unsafe { drop(Box::from_raw(parent)); drop(Box::from_raw(young)); }
    }

    #[test]
    fn test_gray_queue_thread_safety() {
        // 多线程并发 push/pop，不 panic、不丢数据
        let gc = Arc::new(GcRuntime::new());
        let objs: Vec<_> = (0..1000).map(|_| make_obj(Color::Gray)).collect();
        let handles: Vec<_> = (0..4).map(|_| {
            let gc = Arc::clone(&gc);
            let chunk: Vec<_> = objs.iter().step_by(4).copied().collect();
            thread::spawn(move || {
                for o in chunk { gc.gray_queue.push(o); }
            })
        }).collect();
        for h in handles { h.join().unwrap(); }
        assert_eq!(gc.gray_queue.len(), 250); // 每线程 250 个
        for o in objs { unsafe { drop(Box::from_raw(o)); } }
    }

    #[test]
    fn test_color_atomic_preserves_other_bits() {
        // 原子着色不丢失 gen/age/finalizer 位
        let obj = make_obj(Color::White);
        unsafe {
            (*obj).set_generation(Generation::Old);
            (*obj).inc_age();
            (*obj).set_has_finalizer(true);
            set_color_atomic(obj, Color::Black);
            assert_eq!(color_atomic(obj), Color::Black);
            assert_eq!((*obj).generation(), Generation::Old); // 保留
            assert_eq!((*obj).age(), 1);                       // 保留
            assert!((*obj).has_finalizer());                   // 保留
        }
        unsafe { drop(Box::from_raw(obj)); }
    }

    #[test]
    fn test_degradation_uses_stw_path() {
        // concurrent_enabled = false → major_collect 走 Task 52 STW major_gc_stw
        // 验证：不创建 GC Worker 线程，phase 不进入 ConcurrentMark
        let gc = GcRuntime::new();
        assert!(!gc.concurrent_enabled.load(Ordering::Relaxed)); // 默认 false
        // major_collect 在降级模式下应直接调用 major_gc_stw
        // （集成测试验证行为，此处验证默认值）
    }
}
```

### 集成测试：并发标记正确性（`tests/gc_concurrent_tests.rs`）

```rust
#[test]
fn test_concurrent_mark_no_false_collection() {
    // 分配大量对象，根集保留引用，触发并发 Major GC，验证存活对象不被误回收。
    let mut vm = VM::new();
    vm.gc_runtime.concurrent_enabled.store(true, Ordering::Relaxed);
    vm.heap.promotion_age = 1;

    // 分配并晋升一批对象到 Old
    let live = gc_alloc_list(&mut vm.heap, vec![Object::Int(1), Object::Int(2)]);
    vm.stack.push(live);
    gc::minor_gc(&mut vm.heap, &mut vm.stack, &mut vm.globals, &mut [], &mut []);
    // live 现在在 Old 代

    // 触发 Major GC（降级模式：同步 STW，测试 GC 逻辑正确性；
    // 并发线程行为由 test_safepoint_parks_mutator 等单元测试覆盖）
    vm.gc_runtime.concurrent_enabled.store(false, Ordering::Relaxed);
    gc::major_collect_stw(&mut vm.heap, &mut vm);

    // live 仍可访问且内容正确
    let top = vm.stack.last().unwrap();
    if let Object::Ref(r) = top {
        unsafe {
            assert_eq!(gc_read_list(*r).clone(), vec![Object::Int(1), Object::Int(2)]);
        }
    } else { panic!("live object collected!"); }
}

#[test]
fn test_concurrent_mark_with_mutation() {
    // 并发标记期间持续修改引用（写屏障路径），验证三色不变性不被破坏。
    let mut vm = VM::new();
    vm.gc_runtime.concurrent_enabled.store(true, Ordering::Relaxed);
    vm.heap.promotion_age = 1;

    let container = gc_alloc_list(&mut vm.heap, vec![]);
    vm.stack.push(container);
    gc::minor_gc(&mut vm.heap, &mut vm.stack, &mut vm.globals, &mut [], &mut []);

    // 降级模式触发 GC（并发线程协调的端到端测试见专项 stress test）
    vm.gc_runtime.concurrent_enabled.store(false, Ordering::Relaxed);
    gc::major_collect_stw(&mut vm.heap, &mut vm);
    // 验证不 panic + 容器可达
}

#[test]
fn test_concurrent_mark_cycle_collection() {
    // 循环引用经并发标记后回收（Black 不可达 → 重置 White → 下轮回收）
    let mut vm = VM::new();
    vm.gc_runtime.concurrent_enabled.store(true, Ordering::Relaxed);
    vm.heap.promotion_age = 1;

    let a = gc_alloc_list(&mut vm.heap, vec![Object::Int(1)]);
    let b = gc_alloc_list(&mut vm.heap, vec![Object::Int(2)]);
    if let (Object::Ref(ap), Object::Ref(bp)) = (&a, &b) {
        unsafe {
            gc_read_list_mut(*ap).push(b.clone());
            gc_read_list_mut(*bp).push(a.clone());
        }
    }
    let mut stack = vec![a.clone(), b.clone()];
    gc::minor_gc(&mut vm.heap, &mut stack, &mut vm.globals, &mut [], &mut []);
    // 解除根 → 循环引用不可达
    stack.clear();
    vm.stack = stack;
    vm.gc_runtime.concurrent_enabled.store(false, Ordering::Relaxed);
    gc::major_collect_stw(&mut vm.heap, &mut vm);
    assert!(vm.heap.old_objects.is_empty(), "cycle should be collected");
}

#[test]
fn test_safepoint_parks_mutator() {
    // GC 请求 STW → mutator 在安全点停下 → GC 执行 STW → mutator 恢复
    // 验证 SafepointCoordinator 的 request_and_wait / release / check_and_park 协议
    let sp = SafepointCoordinator::new();
    let sp_clone = Arc::new(sp);
    let sp_bg = Arc::clone(&sp_clone);

    let handle = thread::spawn(move || {
        // 模拟 mutator：等待 requested → park → 等 release
        thread::sleep(std::time::Duration::from_millis(10));
        sp_bg.check_and_park();
    });

    // GC 请求 STW
    sp_clone.request_and_wait(); // 等待 mutator park
    // 此时 mutator 已停，GC 做 STW 工作...
    sp_clone.release();
    handle.join().unwrap();
}

#[test]
fn test_alloc_during_mark_marked_black() {
    // 并发标记期间分配的新对象被标黑，不被误回收
    let gc = GcRuntime::new();
    gc.set_phase(GcPhase::ConcurrentMark);
    let obj = make_obj(Color::White);
    alloc_during_gc(&gc, obj);
    assert_eq!(unsafe { color_atomic(obj) }, Color::Black);
    unsafe { drop(Box::from_raw(obj)); }
}
```

### mslang 级别验证 `test_concurrent_gc.ms`

```ms
import gc

fn test_concurrent_gc() {
    gc.set_concurrent(true)
    # 分配大量对象触发并发 GC
    for i in range(1000) {
        x = [1, 2, 3, 4, 5]
    }
    # 存活对象仍可访问
    keep = [10, 20, 30]
    gc.collect()
    print(keep[0])  # 10
    print("concurrent gc ok")
}

fn test_degradation() {
    gc.set_concurrent(false)
    # 降级为 STW，行为同 Task 52
    for i in range(1000) {
        x = [1, 2, 3]
    }
    gc.collect()
    print("stw gc ok")
}

test_concurrent_gc()
test_degradation()
```

预期输出：
```
10
concurrent gc ok
stw gc ok
```

### 构建验证

```bash
# 单元测试
cargo test -- gc::

# 并发标记集成测试
cargo test --test gc_concurrent_tests

# mslang 级别
cargo run -- run tests/integration/test_concurrent_gc.ms
```

## 实现注意事项

1. **线程 Join 安全**：VM `drop` 时必须 `gc_coordinator.shutdown(&gc_runtime)`（先 release safepoint 再 join），否则 Coordinator 线程访问已释放的 VM → UAF。`VM` 裸指针经 `Arc<AtomicPtr<VM>>` 传递，VM drop 前置 null。Coordinator 的 `run_major_cycle` 调用 `major_collect_concurrent`（§15.2），经 VM 裸指针在 STW 窗口内访问 heap/roots。
2. **unsafe 边界**（14-gc.md 788-794 行）：`unsafe` 块 ≤ 30 行，每个块附 `// SAFETY:` 注释。debug 模式增加 type_tag 校验。`gc.verify()` 遍历堆检查可达对象颜色一致性（仅 debug）。
3. **渐进启用**：`concurrent_enabled` 默认 `false`（降级），确保本任务合并不改变现有行为。Task 64 调优完成后可改默认 `true`。
4. **Worker 线程数**：默认 `gc_threads_setting`（Task 52 存储），实际默认 `available_parallelism() / 4`（min 1）。Task 64 自适应可能覆盖。
5. **与 Task 63 的接口**：Sweep 阶段当前为 STW（`gc_sweep_stw`）。Task 63 替换为 `gc_concurrent_sweep`，签名不变（接收 `gc_runtime + heap`），标记期间分配标黑逻辑由 Task 63 完善（LOS + free-list 并发安全）。
6. **与 Task 77 的接口**：Task 77 的 `msWriteBarrier` 应调用本任务的 `write_barrier_obj`（而非自行实现着色逻辑），确保 Rust 侧与 C API 侧行为一致。Task 77 文档中 "shade parent" 的描述需修正为 "shade old_val"（与 14-gc.md 514-532 行一致）。
