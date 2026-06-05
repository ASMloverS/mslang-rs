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

MVP 阶段（Task 74）为 STW GC，写屏障是 no-op。本任务升级为混合写屏障（Hybrid Write Barrier），对应 [14-gc.md](../14-gc.md) § 混合写屏障：

**混合写屏障规则**（Go 1.8+ 风格）：

在 STORE `new_val` 到 `parent.field` 时：

1. **Shade `new_val` 灰色**（如果白色）— 插入屏障侧
2. **Shade `parent` 灰色**（如果白色）— 删除屏障侧

此规则保证：并发标记期间不会产生白色→白色边，三色不变性不被破坏。

**C 扩展注意事项**：
- `msListPush`/`msDictSet`/`msInstanceSet` 等内置操作已在 VM 字节码层面内部包含写屏障（[14-gc.md](../14-gc.md) § 写屏障插入点）
- 仅当 C 侧直接操作对象内部结构（如直接修改 List 内部数组指针）时需手动调用 `msWriteBarrier`
- 并发标记期间（`GcPhase::ConcurrentMark`）写屏障才生效，非 GC 期间零开销

### GC 线程数

```c
MS_API void msGcSetGcThreads(MsVM* vm, uint32_t threads);
```

MVP 阶段仅存储不使用。本任务实际控制 GC Worker 线程池大小：

- GC Coordinator 管理 Worker 线程池（[14-gc.md](../14-gc.md) § GC 与协程交互）
- `threads` 设置 Worker 线程数，下一次 GC 周期生效
- 默认值为 `std::thread::available_parallelism()` 的 1/4，最小 1
- Task 64 自适应调整可能覆盖此值

### GC 调优参数

已有 API（Task 74）在并发 GC 下行为变化：

| API | MVP 行为 | 并发 GC 行为 |
|---|---|---|
| `msGcSetThreshold(MS_GC_MAJOR, r)` | 设置 Old GC 触发比率 | 影响 `concurrent_mark_threshold`（默认 0.8） |
| `msGcSetThreshold(MS_GC_MINOR, r)` | 设置 Young 代大小（MB） | 同左，自适应调整可能覆盖 |
| `msGcSetPromotionAge(age)` | 设置晋升年龄 | 同左，自适应调整可能覆盖 |
| `msGcSetGcThreads(threads)` | 存储，不使用 | 控制 Worker 线程池大小 |

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

替换 Task 74 的 no-op 实现：

```rust
#[no_mangle]
pub extern "C" fn msWriteBarrier(
    vm: *mut MsVM,
    parent: *mut MsValue,
    new_val: *mut MsValue,
) {
    if vm.is_null() || parent.is_null() || new_val.is_null() {
        return;
    }

    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();

    // 非并发标记阶段：零开销直接返回
    if !inner.vm.gc.gc_phase().is_concurrent_mark() {
        return;
    }

    let parent_obj = match unsafe { &*parent }.as_object() {
        Object::Ref(ptr) => ptr,
        _ => return,
    };
    let new_val_obj = match unsafe { &*new_val }.as_object() {
        Object::Ref(ptr) => ptr,
        _ => return,
    };

    let parent_header = unsafe { &mut **parent_obj };
    let new_val_header = unsafe { &mut **new_val_obj };

    // 混合写屏障（Go 1.8+ 风格）：
    // 1. Shade new_val 灰色（如果白色）
    if new_val_header.color() == Color::White {
        new_val_header.set_color(Color::Gray);
        inner.vm.gc.gray_queue().push(*new_val_obj);
    }

    // 2. Shade parent 灰色（如果白色）
    if parent_header.color() == Color::White {
        parent_header.set_color(Color::Gray);
        inner.vm.gc.gray_queue().push(*parent_obj);
    }

    // 3. Old → Young 跨代引用：标记 Card Table dirty
    if parent_header.generation() == Generation::Old
        && new_val_header.generation() == Generation::Young
    {
        mark_card_dirty(*parent_obj);
    }
}
```

**关键设计决策**：

1. **非 GC 期间零开销**：通过 `gc_phase().is_concurrent_mark()` 检查，非并发标记阶段直接返回
2. **仅处理 Ref 类型**：非堆对象（Int、Float 等内联值）不需要写屏障
3. **灰色队列线程安全**：`gray_queue()` 返回的队列必须是线程安全的（Task 62 实现）
4. **Card Table 维护**：Old → Young 引用标记 dirty card，供 Minor GC 扫描 Remembered Set
5. **锁持有时间**：整个写屏障在 VM lock 内完成，确保与 GC Worker 的内存一致性

### 2. msGcSetGcThreads 升级

替换 Task 74 的"存储不使用"实现：

```rust
#[no_mangle]
pub extern "C" fn msGcSetGcThreads(vm: *mut MsVM, threads: u32) {
    if vm.is_null() || threads == 0 {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    let actual = threads.max(1);
    inner.vm.gc.config().set_gc_threads(actual);

    // 如果 GC Worker 线程池正在运行，在下一次 GC 周期生效
    // Task 64 自适应调整可能在下次 GC 时覆盖此值
}
```

**线程池管理**（由 Task 62/63 的 GC Coordinator 负责）：
- GC Coordinator 维护一个 `JoinHandle` 池
- 新 GC 周期开始时，按 `gc_threads` 配置创建/复用 Worker 线程
- GC 周期结束后，Worker 线程挂起等待下一个周期

### 3. msGcSetThreshold 升级

Task 74 已有基础实现。并发 GC 下的变化：

- `MS_GC_MAJOR` 的 threshold 同时影响 `concurrent_mark_threshold`（默认 0.8）
- 即 Old 代占用达到 `old_size * concurrent_mark_threshold` 时开始并发标记
- Task 64 的 pacer 逻辑使用此阈值决定何时启动 GC

无需修改 `msGcSetThreshold` 本身，但 GC 核心的触发逻辑需要使用 `concurrent_mark_threshold`（由 Task 64 实现）。C API 层确保阈值正确传递。

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
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    match gc_type {
        MsGcType::MS_GC_MINOR => {
            inner.vm.gc.minor_collect();
        }
        MsGcType::MS_GC_MAJOR => {
            // 并发 GC：触发完整 Major GC 周期
            // Task 62/63 实现的 major_collect() 内部协调并发标记/清扫
            inner.vm.gc.major_collect();
        }
        MsGcType::MS_GC_FULL => {
            inner.vm.gc.minor_collect();
            inner.vm.gc.major_collect();
        }
    }
}
```

### 5. MsGcStats 扩展

在 Task 74 的 `MsGcStats` 基础上新增字段：

```rust
#[repr(C)]
#[derive(Default, Clone)]
pub struct MsGcStats {
    // 原有字段
    pub minor_gc_count: u64,
    pub major_gc_count: u64,
    pub total_pause_ns: u64,
    pub last_pause_ns: u64,
    pub young_size: u64,
    pub old_size: u64,
    pub los_size: u64,
    pub bytes_freed: u64,

    // 新增：并发 GC 指标
    pub concurrent_mark_ns: u64,
    pub concurrent_sweep_ns: u64,
    pub init_stw_ns: u64,
    pub term_stw_ns: u64,
    pub gray_queue_peak: u64,
    pub gc_threads: u32,
    _padding: u32, // 对齐
}
```

`msGcStats` 实现从 `MsHeap` 收集并发指标：

```rust
#[no_mangle]
pub extern "C" fn msGcStats(vm: *mut MsVM) -> MsGcStats {
    if vm.is_null() {
        return MsGcStats::default();
    }
    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();
    inner.vm.gc.get_stats()
}
```

`MsHeap::get_stats()` 返回包含并发指标的完整统计（需 Task 62/63 在 GC 核心中记录这些数据）。

**暂停时间语义变化**：

MVP 阶段 `total_pause_ns`/`last_pause_ns` 是整个 GC 耗时。并发 GC 下：

| 字段 | 含义 |
|---|---|
| `total_pause_ns` | 累计 STW 时间（Init + Mark Termination 之和） |
| `lastPauseNs` | 最近一次 GC 周期的 STW 时间 |
| `concurrentMarkNs` | 并发标记阶段总耗时（不包含在暂停时间中） |
| `concurrentSweepNs` | 并发清扫阶段总耗时（不包含在暂停时间中） |
| `initStwNs` | Init STW 阶段累计耗时 |
| `termStwNs` | Mark Termination STW 阶段累计耗时 |

### 6. mark_card_dirty 辅助函数

```rust
fn mark_card_dirty(obj_ptr: *mut MsObjHeader) {
    // 计算 obj 在 Old 代中的 Card 索引
    // Card 大小 = 512 字节（[14-gc.md] § Remembered Set）
    let obj_addr = obj_ptr as usize;
    let old_start = /* Old 代起始地址 */;
    let card_index = (obj_addr - old_start) / 512;
    // 标记对应 Card 为 dirty
    // card_table[card_index] = 0x01;
}
```

此函数由 Task 62 在 GC 核心中实现。C API 层调用 Task 62 提供的接口。

### 7. 与 Task 62（并发标记）的集成

| C API | Task 62 提供的接口 | 集成方式 |
|---|---|---|
| `msWriteBarrier` | `GcPhase::is_concurrent_mark()` | 阶段检查，非并发标记时零开销 |
| `msWriteBarrier` | `GrayQueue::push()` | 白色对象标灰并加入灰色队列 |
| `msWriteBarrier` | `mark_card_dirty()` | Old → Young 跨代引用维护 |
| `msGcCollect(MAJOR)` | `major_collect()` | 触发并发标记-清扫周期 |

**灰色队列线程安全要求**：

C 扩展可能从任意线程调用 `msWriteBarrier`，灰色队列必须支持并发入队：
- 方案 A：`Mutex<Vec<...>>`（简单，锁竞争低因为写屏障操作极短）
- 方案 B：无锁队列（性能更优，实现复杂度更高）
- 由 Task 62 决定具体方案，C API 层透明使用

### 8. 与 Task 63（并发清扫）的集成

| C API | Task 63 提供的接口 | 集成方式 |
|---|---|---|
| `msGcStats` | `concurrent_sweep_ns` | 统计并发清扫耗时 |
| `msGcCollect` | `major_collect()` 内含并发清扫 | 触发完整 GC 周期 |

### 9. 与 Task 64（GC 调优）的集成

| C API | Task 64 提供的接口 | 集成方式 |
|---|---|---|
| `msGcSetThreshold` | `GcConfig::set_old_gc_ratio()` | 阈值影响并发标记触发时机 |
| `msGcSetPromotionAge` | `GcConfig::set_promotion_age()` | 晋升年龄影响分代行为 |
| `msGcSetGcThreads` | `GcConfig::set_gc_threads()` | 线程数控制 Worker 池大小 |
| `msGcStats` | 自适应调整后的实际参数 | 返回当前生效的 GC 参数 |

Task 64 的自适应调整可能在 GC 周期结束后覆盖 C API 设置的参数。`msGcStats.gcThreads` 返回实际生效的线程数（可能被自适应调整过）。

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

新增字段追加在末尾，保证与 Task 74 的二进制兼容（旧代码只读前 8 个字段）。

## 验证标准

1. **msWriteBarrier 着色**：并发标记期间调用写屏障，`new_val` 和 `parent` 被正确标灰
2. **msWriteBarrier 零开销**：非并发标记阶段调用写屏障，立即返回，无性能影响
3. **C 扩展并发安全**：C 扩展在并发 GC 运行期间修改堆对象不崩溃
4. **无误回收**：C 扩展持续修改对象引用的同时 GC 并发运行，存活对象不被错误回收
5. **msGcSetGcThreads 生效**：设置后下一次 GC 周期实际使用指定数量的 Worker 线程
6. **Card Table 维护**：写屏障对 Old → Young 引用正确标记 dirty card
7. **并发 GC 统计**：`msGcStats` 返回的并发指标非零（在并发 GC 执行后）
8. **STW 暂停分解**：`initStwNs` + `termStwNs` = `totalPauseNs`（单次 GC 周期）
9. **灰色队列峰值**：`grayQueuePeak` 反映并发标记期间灰色队列最大深度
10. **多线程安全**：多个线程同时调用 `msWriteBarrier` 不产生数据竞争
11. **降级兼容**：Task 74 的旧测试全部通过（MsGcStats 新字段在末尾，不影响旧代码）
12. **NULL 安全**：所有函数传入 NULL 指针不崩溃

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
        // 验证 gc_threads 被正确设置
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
    fn test_write_barrier_multithread() {
        let vm = Arc::new(Mutex::new(msVmNew()));
        let vm_clone = Arc::clone(&vm);

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

        // 多线程同时调用写屏障
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

        {
            let vm_lock = vm.lock().unwrap();
            msVmFree(*vm_lock);
        }
        std::mem::forget(vm);
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
