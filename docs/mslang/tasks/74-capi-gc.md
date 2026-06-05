# C API — GC 交互（Root/写屏障/Finalizer/控制/统计）

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 52-gc（GC 核心实现：Young 代半空间复制 + Old 代 STW 标记-清除）
- 65-capi-infrastructure（C API 基础设施：cbindgen、types.h、模块框架）

> 注：msRoot/msUnroot 由 Task 67（C API — Value 操作）实现。本任务覆盖其余全部 GC 交互 API。

## 目标

实现 GC 交互相关的全部 C API：写屏障（msWriteBarrier）、Finalizer 注册（msOnFinalize）、GC 控制（msGcCollect/msGcEnable/msGcIsEnabled）、GC 调优（msGcSetThreshold/msGcSetPromotionAge/msGcSetGcThreads）、GC 调试模式（msGcSetDebug）、GC 统计（MsGcStats + msGcStats）。

## 设计规格

参照 [13-capi.md](../13-capi.md) § GC 交互：

### 写屏障

```c
MS_API void msWriteBarrier(MsVM* vm, MsValue* parent, MsValue* new_val);
```

- C 扩展直接修改堆对象引用字段时必须调用
- `msListPush`/`msDictSet`/`msInstanceSet` 等内置操作已内部包含写屏障
- 仅当 C 侧直接操作对象内部结构时需要手动调用

### Finalizer 注册

```c
MS_API MsStatus msOnFinalize(MsVM* vm, MsValue* obj, MsFinalizerFn fn, void* userdata);
```

- 注册 C finalizer 回调
- 对象被 GC 回收前在 mutator 线程中调用回调
- `MsFinalizerFn` 签名：`void (*MsFinalizerFn)(MsVM* vm, MsValue* obj, void* userdata)`

### GC 控制

```c
MS_API void msGcCollect(MsVM* vm, MsGcType type);
MS_API void msGcEnable(MsVM* vm, int enable);
MS_API int  msGcIsEnabled(MsVM* vm);
```

- `msGcCollect`：按 `MsGcType` 触发 GC（Minor / Major / Full）
- `msGcEnable`：`enable=1` 启用自动 GC，`enable=0` 禁用
- `msGcIsEnabled`：返回当前自动 GC 状态

### GC 调优

```c
MS_API void msGcSetThreshold(MsVM* vm, MsGcType type, double threshold);
MS_API void msGcSetPromotionAge(MsVM* vm, uint32_t age);
MS_API void msGcSetGcThreads(MsVM* vm, uint32_t threads);
```

### GC 调试模式

```c
MS_API void msGcSetDebug(MsVM* vm, int enable);
```

- 仅 `debug_assertions` 构建可用
- root/unroot 配对检查、解引用类型标签校验、GC 后堆一致性验证

### GC 统计

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
} MsGcStats;

MS_API MsGcStats msGcStats(MsVM* vm);
```

## 实现细节

### 文件位置

- `src/capi/gc.rs` — 全部 GC 交互 C API 实现

### 依赖关系

本任务依赖以下已有模块：

| 模块 | 路径 | 提供能力 |
|---|---|---|
| GC 核心 | `src/vm/gc.rs` | `MsHeap`、`minor_gc()`、`major_gc()`、`GcConfig`、`MsGcStats` |
| C API 类型 | `src/capi/types.rs` | `MsVM`、`MsValue`、`MsStatus`、`VmInner` |
| 对象头 | `src/vm/gc.rs` | `MsObjHeader`、`gc_meta` 位域 |

### 1. msWriteBarrier

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
    // MVP Phase 6: STW GC 无并发写屏障需求，此处为 no-op。
    // Phase 7.5 升级为真实写屏障（snprintf/snapshot-at-the-barrier）。
}
```

MVP 阶段为 STW GC，无并发标记，写屏障是 no-op。仅做指针非空校验。

Phase 7.5 升级路径：
- 引入快照-at-the-barrier（snapshot-at-the-barrier）写屏障
- 在 `parent` 的旧引用被覆盖前，将旧值标记为灰色
- 配合并发三色标记清扫 GC 使用

### 2. msOnFinalize

```rust
#[no_mangle]
pub extern "C" fn msOnFinalize(
    vm: *mut MsVM,
    obj: *mut MsValue,
    fn_ptr: Option<extern "C" fn(*mut MsVM, *mut MsValue, *mut std::ffi::c_void)>,
    userdata: *mut std::ffi::c_void,
) -> MsStatus {
    if vm.is_null() || obj.is_null() {
        return MsStatus::MS_ERROR;
    }
    let fn_ptr = match fn_ptr {
        Some(f) => f,
        None => return MsStatus::MS_ERROR,
    };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    // 验证 obj 是 Ref 类型（堆对象）
    let header = match inner.vm.get_obj_header(obj) {
        Some(h) => h,
        None => return MsStatus::MS_ERROR,
    };

    // 设置 has_finalizer 标志位
    unsafe {
        (*header).gc_meta |= 0b0001_0000; // HAS_FINALIZER
    }

    // 注册 finalizer 回调
    inner.vm.gc.register_finalizer(
        header as usize,
        fn_ptr,
        userdata,
        obj,
    );

    MsStatus::MS_OK
}
```

实现步骤：
1. 加锁 VM
2. 校验 `obj` 是 Ref 类型（堆对象），获取其 `MsObjHeader` 指针
3. 设置 `gc_meta` 的 `HAS_FINALIZER` 位（bit 4）
4. 在 VM 的 finalizer 注册表中记录 `(obj_addr, fn, userdata, MsValue*)` 四元组
5. GC 回收对象前，在 finalizer 队列中查找匹配项，调用回调

**Finalizer 调用机制**（复用 GC 核心的 finalizer 队列）：

GC 在标记-清除的 sweep 阶段，发现对象的 `HAS_FINALIZER` 位置位但对象不可达时：
1. 将对象加入 finalizer 队列（而非立即回收）
2. 重新标记该对象为可达（resurrection 一次机会）
3. 下一次 GC 时，如果对象仍然不可达，调用注册的 C finalizer
4. 调用后在 mutator 线程中执行 `fn(vm, obj, userdata)`
5. 真正回收对象内存

Finalizer 注册表结构：

```rust
struct FinalizerEntry {
    obj_addr: usize,
    fn_ptr: extern "C" fn(*mut MsVM, *mut MsValue, *mut std::ffi::c_void),
    userdata: *mut std::ffi::c_void,
    value_ptr: *mut MsValue,
}

unsafe impl Send for FinalizerEntry {}
unsafe impl Sync for FinalizerEntry {}
```

存储在 `VmInner` 或 `MsHeap` 中：

```rust
finalizers: Vec<FinalizerEntry>,
```

### 3. msGcCollect

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
            inner.vm.gc.major_collect();
        }
        MsGcType::MS_GC_FULL => {
            inner.vm.gc.minor_collect();
            inner.vm.gc.major_collect();
        }
    }
}
```

`MsGcType` 映射：

| C 枚举值 | Rust 行为 |
|---|---|
| `MS_GC_MINOR` | 调用 `minor_collect()` |
| `MS_GC_MAJOR` | 调用 `major_collect()` |
| `MS_GC_FULL` | 先 `minor_collect()` 再 `major_collect()` |

Full GC 先执行 Minor 是因为 Minor 可能产生新的晋升对象到 Old 代，需要 Major 来处理。

### 4. msGcEnable / msGcIsEnabled

```rust
#[no_mangle]
pub extern "C" fn msGcEnable(vm: *mut MsVM, enable: i32) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    inner.vm.gc.set_enabled(enable != 0);
}

#[no_mangle]
pub extern "C" fn msGcIsEnabled(vm: *mut MsVM) -> i32 {
    if vm.is_null() {
        return 0;
    }
    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();
    if inner.vm.gc.is_enabled() {
        1
    } else {
        0
    }
}
```

`gc_enabled` 标志存储在 `GcConfig` 中（Task 52 创建的 `MsHeap` 扩展）：

```rust
struct GcConfig {
    enabled: bool,
    debug: bool,
    gc_threads: u32,
}
```

禁用自动 GC 时：
- `maybe_gc()` 检查 `enabled` 标志，为 false 时跳过
- 分配仍正常进行，Young 代满时回退到 Old 代分配
- 手动 `msGcCollect` 不受此标志影响（始终执行）

### 5. msGcSetThreshold

```rust
#[no_mangle]
pub extern "C" fn msGcSetThreshold(
    vm: *mut MsVM,
    gc_type: MsGcType,
    threshold: f64,
) {
    if vm.is_null() || threshold <= 0.0 {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    match gc_type {
        MsGcType::MS_GC_MAJOR => {
            inner.vm.gc.set_major_ratio(threshold);
        }
        MsGcType::MS_GC_MINOR => {
            // threshold 作为 Young 代大小（MB → bytes）
            let young_bytes = (threshold * 1024.0 * 1024.0) as usize;
            inner.vm.gc.set_young_size(young_bytes);
        }
        MsGcType::MS_GC_FULL => {
            inner.vm.gc.set_major_ratio(threshold);
        }
    }
}
```

阈值语义：

| MsGcType | threshold 含义 | 存储位置 |
|---|---|---|
| `MS_GC_MINOR` | Young 代大小（MB），转换为 bytes | `MsHeap.young_from.len()` 重分配 |
| `MS_GC_MAJOR` | Old GC 触发比率（`MAJOR_GC_RATIO`） | `MsHeap.next_major_gc` 计算依据 |
| `MS_GC_FULL` | 等同 `MS_GC_MAJOR` | 同上 |

### 6. msGcSetPromotionAge

```rust
#[no_mangle]
pub extern "C" fn msGcSetPromotionAge(vm: *mut MsVM, age: u32) {
    if vm.is_null() {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    // age 范围 1-3，对应 gc_meta 的 2-bit age 字段
    let clamped = age.clamp(1, 3);
    inner.vm.gc.set_promotion_age(clamped as u8);
}
```

晋升年龄范围限制在 1-3（`gc_meta` bit 6-7 为 2-bit age 字段，最大值 3）。默认值为 2。

### 7. msGcSetGcThreads

```rust
#[no_mangle]
pub extern "C" fn msGcSetGcThreads(vm: *mut MsVM, threads: u32) {
    if vm.is_null() || threads == 0 {
        return;
    }
    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();
    // MVP: 仅存储，不使用（无并发 GC 线程）
    inner.vm.gc.set_gc_threads(threads);
}
```

MVP 阶段（STW GC）：线程数存储但无实际效果。

Phase 7.5 升级路径：
- 控制 GC Worker 线程池大小
- 并发标记阶段的工作线程数
- 默认值为 `std::thread::available_parallelism()` 的 1/4

### 8. msGcSetDebug

```rust
#[no_mangle]
pub extern "C" fn msGcSetDebug(vm: *mut MsVM, enable: i32) {
    if vm.is_null() {
        return;
    }
    #[cfg(debug_assertions)]
    {
        let vm_ref = unsafe { &*vm };
        let mut inner = vm_ref.inner.lock().unwrap();
        inner.vm.gc.set_debug(enable != 0);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = enable;
    }
}
```

仅在 `#[cfg(debug_assertions)]` 构建中有实际效果。release 构建中为 no-op。

Debug 模式启用的检查项：

| 检查项 | 触发时机 | 检测内容 |
|---|---|---|
| root/unroot 配对 | `msRoot`/`msUnroot` 调用时 | 重复 unroot、未 root 先 unroot |
| 类型标签校验 | 每次通过 `MsValue*` 访问堆对象 | `type_tag` 在合法 `TypeTag` 范围内 |
| 堆一致性验证 | 每次 GC 完成后 | 遍历堆，所有可达对象的类型标签合法、颜色一致 |

Debug 模式对性能有显著影响，仅用于开发调试。

### 9. MsGcStats / msGcStats

#### C 结构体定义

`MsGcStats` 定义在 `include/mslang/types.h` 中（Task 65 已创建），由 cbindgen 从 Rust 侧 `#[repr(C)]` 结构体生成或手写：

```rust
#[repr(C)]
#[derive(Default, Clone)]
pub struct MsGcStats {
    pub minor_gc_count: u64,
    pub major_gc_count: u64,
    pub total_pause_ns: u64,
    pub last_pause_ns: u64,
    pub young_size: u64,
    pub old_size: u64,
    pub los_size: u64,
    pub bytes_freed: u64,
}
```

字段映射：

| Rust 字段 | C 字段 | 来源 |
|---|---|---|
| `minor_gc_count` | `minorGcCount` | `MsHeap` 统计：Minor GC 执行次数 |
| `major_gc_count` | `majorGcCount` | `MsHeap` 统计：Major GC 执行次数 |
| `total_pause_ns` | `totalPauseNs` | 累计 GC 暂停时间（纳秒） |
| `last_pause_ns` | `lastPauseNs` | 最近一次 GC 暂停时间（纳秒） |
| `young_size` | `youngSize` | Young 代当前使用字节数 |
| `old_size` | `oldSize` | Old 代当前使用字节数 |
| `los_size` | `losSize` | Large Object Space 当前使用字节数 |
| `bytes_freed` | `bytesFreed` | 累计释放字节数 |

#### 实现

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

**暂停时间追踪**：

在 `minor_collect()` 和 `major_collect()` 中，使用 `std::time::Instant` 记录耗时：

```rust
fn minor_collect(&mut self) {
    let start = std::time::Instant::now();
    // ... Minor GC 逻辑 ...
    let elapsed = start.elapsed().as_nanos() as u64;
    self.stats.total_pause_ns += elapsed;
    self.stats.last_pause_ns = elapsed;
    self.stats.minor_gc_count += 1;
}
```

**MsHeap 统计字段扩展**（在 Task 52 基础上扩展）：

```rust
struct GcStats {
    minor_gc_count: u64,
    major_gc_count: u64,
    total_pause_ns: u64,
    last_pause_ns: u64,
    bytes_freed: u64,
}
```

`young_size`、`old_size`、`los_size` 从 `MsHeap` 的空间分配状态实时计算：

```rust
fn get_stats(&self) -> MsGcStats {
    MsGcStats {
        minor_gc_count: self.stats.minor_gc_count,
        major_gc_count: self.stats.major_gc_count,
        total_pause_ns: self.stats.total_pause_ns,
        last_pause_ns: self.stats.last_pause_ns,
        young_size: self.young_cursor as u64,
        old_size: self.old_space.bytes_used as u64,
        los_size: self.los_space.map_or(0, |los| los.bytes_used as u64),
        bytes_freed: self.stats.bytes_freed,
    }
}
```

### 10. MsGcType 枚举映射

Rust 侧 `MsGcType` 定义（复用 Task 65 types.h 中的 C 枚举）：

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsGcType {
    MS_GC_MINOR = 0,
    MS_GC_MAJOR = 1,
    MS_GC_FULL  = 2,
}
```

### 11. VM 内部 GC 接口扩展

本任务需要在 `MsHeap`（Task 52）上扩展以下接口：

```rust
impl MsHeap {
    fn set_enabled(&mut self, enabled: bool);
    fn is_enabled(&self) -> bool;
    fn set_debug(&mut self, debug: bool);
    fn set_major_ratio(&mut self, ratio: f64);
    fn set_young_size(&mut self, bytes: usize);
    fn set_promotion_age(&mut self, age: u8);
    fn set_gc_threads(&mut self, threads: u32);
    fn get_stats(&self) -> MsGcStats;
    fn register_finalizer(&mut self, addr: usize, fn_ptr: ..., userdata: ..., value: ...);
}
```

这些方法在 GC 核心模块（`src/vm/gc.rs`）中实现，C API 层（`src/capi/gc.rs`）仅做薄封装：加锁 → 调用 → 返回。

### 12. 模块注册

`src/capi/mod.rs` 已在 Task 65 中声明 `pub mod gc;`。本任务填充 `src/capi/gc.rs` 的完整实现。

`build.rs` 中 `gc` 模块已配置为生成 `include/mslang/gc.h`。实现完成后取消 `include/mslang/mslang.h` 中 `#include "gc.h"` 的注释。

## 验证标准

1. **msWriteBarrier**：调用不崩溃（MVP no-op），NULL 指针安全处理
2. **msOnFinalize**：注册回调成功，GC 回收对象前回调被调用
3. **msOnFinalize userdata**：回调接收到正确的 userdata 指针
4. **msGcCollect(MS_GC_MINOR)**：触发 Minor GC，不崩溃
5. **msGcCollect(MS_GC_MAJOR)**：触发 Major GC，不崩溃
6. **msGcCollect(MS_GC_FULL)**：触发 Full GC，不崩溃
7. **msGcEnable(false)**：禁用自动 GC，`msGcIsEnabled()` 返回 0
8. **msGcEnable(true)**：重新启用自动 GC，`msGcIsEnabled()` 返回 1
9. **msGcSetThreshold**：接受有效值（> 0），不崩溃
10. **msGcSetPromotionAge**：接受 1-3 范围值，超出范围被 clamp
11. **msGcSetGcThreads**：存储线程数，不崩溃
12. **msGcSetDebug**：debug 构建中启用调试模式，release 构建为 no-op
13. **msGcStats**：GC 执行后返回非零计数，暂停时间 > 0
14. **msGcStats**：未执行 GC 时 `minorGcCount`/`majorGcCount` 为 0
15. **Finalizer 回调时序**：回调在 GC sweep 阶段、对象被回收前调用
16. **多 VM 隔离**：不同 VM 的 GC 状态互不影响
17. **NULL 安全**：所有函数传入 NULL vm 指针不崩溃

## 测试用例

### Rust 单元测试

`src/capi/gc.rs` 中 `#[cfg(test)] mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::msVmNew;
    use crate::capi::vm::msVmFree;
    use std::ffi::CString;

    #[test]
    fn test_gc_collect_minor() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_collect_major() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_collect_full() {
        let vm = msVmNew();
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        msVmFree(vm);
    }

    #[test]
    fn test_gc_enable_disable() {
        let vm = msVmNew();

        // 默认启用
        assert_eq!(msGcIsEnabled(vm), 1);

        // 禁用
        msGcEnable(vm, 0);
        assert_eq!(msGcIsEnabled(vm), 0);

        // 重新启用
        msGcEnable(vm, 1);
        assert_eq!(msGcIsEnabled(vm), 1);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_stats() {
        let vm = msVmNew();

        // 初始状态
        let stats = msGcStats(vm);
        assert_eq!(stats.minor_gc_count, 0);
        assert_eq!(stats.major_gc_count, 0);

        // 执行 GC 后检查统计
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        let stats = msGcStats(vm);
        assert!(stats.minor_gc_count > 0 || stats.major_gc_count > 0);
        assert!(stats.young_size > 0 || stats.old_size > 0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_stats_after_multiple_collects() {
        let vm = msVmNew();

        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msGcCollect(vm, MsGcType::MS_GC_MINOR);
        msGcCollect(vm, MsGcType::MS_GC_MAJOR);

        let stats = msGcStats(vm);
        assert!(stats.minor_gc_count >= 2);
        assert!(stats.major_gc_count >= 1);
        assert!(stats.total_pause_ns > 0);
        assert!(stats.last_pause_ns > 0);

        msVmFree(vm);
    }

    #[test]
    fn test_finalizer() {
        use std::sync::{Arc, Mutex};

        let called: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let called_ptr = Arc::into_raw(called) as *mut std::ffi::c_void;

        extern "C" fn my_finalizer(
            _vm: *mut MsVM,
            _obj: *mut MsValue,
            userdata: *mut std::ffi::c_void,
        ) {
            let called = unsafe {
                &*(userdata as *const Arc<Mutex<bool>>)
            };
            *called.lock().unwrap() = true;
        }

        let vm = msVmNew();

        // 执行脚本创建一个对象
        let source = CString::new("obj = [1, 2, 3]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        // 获取对象并注册 finalizer
        let name = CString::new("obj").unwrap();
        let obj = unsafe { crate::capi::vm::msGetGlobal(vm, name.as_ptr()) };
        assert!(!obj.is_null());

        let status = msOnFinalize(vm, obj, Some(my_finalizer), called_ptr);
        assert_eq!(status, MsStatus::MS_OK);

        // 删除全局引用，触发 GC
        unsafe { crate::capi::vm::msDelGlobal(vm, name.as_ptr()) };
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let called = unsafe { Arc::from_raw(called_ptr as *const Mutex<bool>) };
        assert!(*called.lock().unwrap());

        msVmFree(vm);
    }

    #[test]
    fn test_write_barrier_noop() {
        let vm = msVmNew();

        let source = CString::new("a = [1]").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        let name_a = CString::new("a").unwrap();
        let a = unsafe { crate::capi::vm::msGetGlobal(vm, name_a.as_ptr()) };

        let source2 = CString::new("b = [2]").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source2.as_ptr(), filename.as_ptr());
        }
        let name_b = CString::new("b").unwrap();
        let b = unsafe { crate::capi::vm::msGetGlobal(vm, name_b.as_ptr()) };

        // MVP no-op，不应崩溃
        msWriteBarrier(vm, a, b);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_threshold() {
        let vm = msVmNew();

        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, 3.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_MINOR, 8.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_FULL, 2.5);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_threshold_invalid() {
        let vm = msVmNew();

        // 无效值（<= 0）应被忽略，不崩溃
        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, 0.0);
        msGcSetThreshold(vm, MsGcType::MS_GC_MAJOR, -1.0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_promotion_age() {
        let vm = msVmNew();

        msGcSetPromotionAge(vm, 1);
        msGcSetPromotionAge(vm, 2);
        msGcSetPromotionAge(vm, 3);

        // 超出范围应被 clamp
        msGcSetPromotionAge(vm, 0);
        msGcSetPromotionAge(vm, 10);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_set_gc_threads() {
        let vm = msVmNew();

        msGcSetGcThreads(vm, 1);
        msGcSetGcThreads(vm, 4);
        msGcSetGcThreads(vm, 8);

        // 0 应被忽略
        msGcSetGcThreads(vm, 0);

        msVmFree(vm);
    }

    #[test]
    fn test_gc_debug_mode() {
        let vm = msVmNew();

        msGcSetDebug(vm, 1);
        msGcCollect(vm, MsGcType::MS_GC_FULL);
        msGcSetDebug(vm, 0);

        msVmFree(vm);
    }

    #[test]
    fn test_null_vm_safe() {
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
    }

    #[test]
    fn test_gc_stats_pause_time() {
        let vm = msVmNew();

        // 分配一些对象以产生有意义的数据
        let source = CString::new("
            for i in range(100) {
                x = [1, 2, 3, 4, 5]
            }
        ").unwrap();
        let filename = CString::new("test.ms").unwrap();
        unsafe {
            crate::capi::vm::msExecString(vm, source.as_ptr(), filename.as_ptr());
        }

        msGcCollect(vm, MsGcType::MS_GC_FULL);

        let stats = msGcStats(vm);
        assert!(stats.last_pause_ns > 0);
        assert!(stats.total_pause_ns >= stats.last_pause_ns);

        msVmFree(vm);
    }
}
```

### C 集成测试

`tests/c/test_gc.c`：

```c
#include <mslang.h>
#include <assert.h>
#include <stdio.h>
#include <string.h>

static int finalizer_called = 0;
static void* finalizer_userdata = NULL;

static void my_finalizer(MsVM* vm, MsValue* obj, void* userdata) {
    finalizer_called = 1;
    finalizer_userdata = userdata;
}

static char captured_buf[4096];
static size_t captured_len = 0;

static int write_capture(const char* data, size_t len, void* userdata) {
    memcpy(captured_buf + captured_len, data, len);
    captured_len += len;
    return 0;
}

void test_gc_collect(void) {
    MsVM* vm = msVmNew();
    msGcCollect(vm, MS_GC_MINOR);
    msGcCollect(vm, MS_GC_MAJOR);
    msGcCollect(vm, MS_GC_FULL);
    msVmFree(vm);
}

void test_gc_enable_disable(void) {
    MsVM* vm = msVmNew();

    assert(msGcIsEnabled(vm) == MS_TRUE);

    msGcEnable(vm, MS_FALSE);
    assert(msGcIsEnabled(vm) == MS_FALSE);

    msGcEnable(vm, MS_TRUE);
    assert(msGcIsEnabled(vm) == MS_TRUE);

    msVmFree(vm);
}

void test_gc_stats(void) {
    MsVM* vm = msVmNew();

    MsGcStats s = msGcStats(vm);
    assert(s.minorGcCount == 0);
    assert(s.majorGcCount == 0);

    msGcCollect(vm, MS_GC_FULL);

    s = msGcStats(vm);
    assert(s.minorGcCount > 0 || s.majorGcCount > 0);

    msVmFree(vm);
}

void test_gc_stats_pause(void) {
    MsVM* vm = msVmNew();

    msSetStdout(vm, write_capture, NULL);
    msExecString(vm,
        "for i in range(100) { x = [1,2,3,4,5] }",
        "test.ms");

    msGcCollect(vm, MS_GC_FULL);

    MsGcStats s = msGcStats(vm);
    assert(s.lastPauseNs > 0);
    assert(s.totalPauseNs >= s.lastPauseNs);

    msVmFree(vm);
}

void test_finalizer(void) {
    MsVM* vm = msVmNew();

    msExecString(vm, "obj = [1, 2, 3]", "test.ms");
    MsValue* obj = msGetGlobal(vm, "obj");
    assert(obj != NULL);

    int dummy_data = 42;
    MsStatus s = msOnFinalize(vm, obj, my_finalizer, &dummy_data);
    assert(s == MS_OK);

    msDelGlobal(vm, "obj");
    msGcCollect(vm, MS_GC_FULL);
    msGcCollect(vm, MS_GC_FULL);

    assert(finalizer_called == 1);
    assert(finalizer_userdata == &dummy_data);

    msVmFree(vm);
}

void test_write_barrier(void) {
    MsVM* vm = msVmNew();

    msExecString(vm, "a = [1]", "test.ms");
    msExecString(vm, "b = [2]", "test.ms");

    MsValue* a = msGetGlobal(vm, "a");
    MsValue* b = msGetGlobal(vm, "b");

    msWriteBarrier(vm, a, b);

    msVmFree(vm);
}

void test_gc_threshold(void) {
    MsVM* vm = msVmNew();

    msGcSetThreshold(vm, MS_GC_MAJOR, 3.0);
    msGcSetThreshold(vm, MS_GC_MINOR, 8.0);
    msGcSetThreshold(vm, MS_GC_FULL, 2.5);

    msVmFree(vm);
}

void test_gc_promotion_age(void) {
    MsVM* vm = msVmNew();

    msGcSetPromotionAge(vm, 1);
    msGcSetPromotionAge(vm, 3);

    msVmFree(vm);
}

void test_gc_threads(void) {
    MsVM* vm = msVmNew();

    msGcSetGcThreads(vm, 4);
    msGcSetGcThreads(vm, 8);

    msVmFree(vm);
}

void test_gc_debug(void) {
    MsVM* vm = msVmNew();

    msGcSetDebug(vm, MS_TRUE);
    msGcCollect(vm, MS_GC_FULL);
    msGcSetDebug(vm, MS_FALSE);

    msVmFree(vm);
}

void test_null_vm(void) {
    assert(msGcIsEnabled(NULL) == 0);

    msGcCollect(NULL, MS_GC_FULL);
    msGcEnable(NULL, 1);
    msGcSetThreshold(NULL, MS_GC_MAJOR, 2.0);
    msGcSetPromotionAge(NULL, 2);
    msGcSetGcThreads(NULL, 4);
    msGcSetDebug(NULL, 1);
    msWriteBarrier(NULL, NULL, NULL);

    MsGcStats s = msGcStats(NULL);
    assert(s.minorGcCount == 0);
}

int main(void) {
    test_gc_collect();
    test_gc_enable_disable();
    test_gc_stats();
    test_gc_stats_pause();
    test_finalizer();
    test_write_barrier();
    test_gc_threshold();
    test_gc_promotion_age();
    test_gc_threads();
    test_gc_debug();
    test_null_vm();

    printf("all gc tests passed\n");
    return 0;
}
```

### 构建验证

```bash
# Rust 单元测试
cargo test --features capi -- capi::gc

# C 集成测试
cargo build --features capi
cc -I include -L target/debug -lmslang tests/c/test_gc.c -o test_gc
./test_gc
```
