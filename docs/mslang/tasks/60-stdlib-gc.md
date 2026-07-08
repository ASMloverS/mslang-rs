# 标准库 - gc 模块

## 所属阶段
Phase 6.2 - 模块系统 + 标准库

## 前置任务
52-gc, 45-module-system

## 目标

实现 mslang 标准库 `gc` 模块，暴露 GC 控制与统计 API，使 mslang 脚本可以手动触发 GC、查询堆状态和调优 GC 参数。

## 设计规格

参照 [10-builtins](../10-builtins.md) § gc、[14-gc](../14-gc.md) § GC 统计与调优 API：

### API 列表

| 函数 | 签名 | 说明 |
|---|---|---|
| `gc.collect()` | `collect() -> nil` | 触发 Full GC（Major + Minor） |
| `gc.collect_minor()` | `collect_minor() -> nil` | 仅触发 Minor GC |
| `gc.enable()` | `enable() -> nil` | 启用自动 GC |
| `gc.disable()` | `disable() -> nil` | 禁用自动 GC |
| `gc.is_enabled()` | `is_enabled() -> bool` | 返回自动 GC 是否启用 |
| `gc.set_threshold()` | `set_threshold(kind, value) -> nil` | 设置 GC 阈值 |
| `gc.set_promotion_age()` | `set_promotion_age(age) -> nil` | 设置 Young→Old 晋升年龄（1-3） |
| `gc.set_gc_threads()` | `set_gc_threads(n) -> nil` | 设置 GC Worker 线程数 |
| `gc.stats()` | `stats() -> dict` | 返回统计信息 dict |
| `gc.count()` | `count() -> int` | GC 总次数（minor + major） |
| `gc.mem_alloc()` | `mem_alloc() -> int` | 当前堆分配字节数 |
| `gc.mem_live()` | `mem_live() -> int` | 当前存活字节数 |

### gc.stats() 返回的 dict

```ms
{
    "minor_count": 42,
    "major_count": 3,
    "total_pause_ns": 1520000,
    "last_pause_ns": 23000,
    "young_size": 4194304,
    "old_size": 1048576,
    "los_size": 0,
    "bytes_freed": 8388608,
    "promotion_age": 2,
    "gc_threads": 8,
    "gc_enabled": true,
}
```

### gc.set_threshold() 参数

| kind | value 含义 | 范围 |
|---|---|---|
| `"major"` | Old GC 触发比率（allocated > live * ratio 时触发） | float, > 0 |
| `"minor"` | Young 代大小（MB） | int, 1-64 |

## 实现细节

> **对象模型约束**（task 20/25/46-50）：`NativeFunction` 结构体为 `{ name: String, func: NativeFn }`（**无 `arity` 字段**，`builtins.rs:26-29`）。`NativeFn = fn(&mut VM, &[Object]) -> Result<Object, String>`。模块注册参照 task 46-49 范式：`register_gc_module() -> *mut MsObjHeader`（返回裸指针，不接受 `&mut VM`），在 `VM::new()` 中经 `vm.module_resolver.native_modules.insert(...)` 注册 + `vm.native_arities.insert(...)` 注册各函数 arity。

### 0. 前置：MsHeap 统计字段扩展（task 52 补齐）

task 52 的 `MsHeap`（`gc.rs:800-815`）仅有 `bytes_allocated`/`next_minor_gc`/`next_major_gc`/`promotion_age`。本任务需在 `MsHeap` 补齐以下字段，并在 GC 函数中添加统计追踪：

```rust
// gc.rs — MsHeap 新增字段
pub struct MsHeap {
    // ... 既有字段 ...
    pub minor_count: u64,
    pub major_count: u64,
    pub total_pause_ns: u64,
    pub last_pause_ns: u64,
    pub bytes_freed: u64,       // 累计回收字节数
    pub gc_enabled: bool,       // enable/disable 开关（默认 true）
}
```

**minor_gc / major_gc 计时**：在函数入口 `let t0 = std::time::Instant::now();`，出口 `let elapsed = t0.elapsed().as_nanos() as u64; heap.total_pause_ns += elapsed; heap.last_pause_ns = elapsed;`。minor_gc 出口 `heap.minor_count += 1;`，major_gc 出口 `heap.major_count += 1;`。

**maybe_gc guard**（M5）：入口添加 `if !heap.gc_enabled { return; }`。

**young/old/los size 统计**（R5）：`get_stats` 中遍历 `young_objects`/`old_objects`/`los_objects` 累加 `(*ptr).size`，或维护增量计数器（alloc 加 / free 减）。

### 1. 文件位置与注册方式

`src/vm/stdlib.rs`（与 task 46-49 同文件，**非** `src/stdlib/gc.rs`）。

```rust
pub fn register_gc_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();

    let funcs: [(&str, NativeFn); 12] = [
        ("collect", gc_collect),
        ("collect_minor", gc_collect_minor),
        ("enable", gc_enable),
        ("disable", gc_disable),
        ("is_enabled", gc_is_enabled),
        ("set_threshold", gc_set_threshold),
        ("set_promotion_age", gc_set_promotion_age),
        ("set_gc_threads", gc_set_gc_threads),
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
            unsafe { read_module_mut(p).exports = exports; }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}
```

在 `VM::new()`（`src/vm/mod.rs`）中，紧随 task 49 json 注册之后追加：

```rust
// task 60：注册原生 gc 模块 + 模块函数 arity。
let gc_ptr = stdlib::register_gc_module();
vm.module_resolver
    .native_modules
    .insert("gc".to_string(), gc_ptr);
vm.native_arities.insert("collect".to_string(), 0);
vm.native_arities.insert("collect_minor".to_string(), 0);
vm.native_arities.insert("enable".to_string(), 0);
vm.native_arities.insert("disable".to_string(), 0);
vm.native_arities.insert("is_enabled".to_string(), 0);
vm.native_arities.insert("set_threshold".to_string(), 2);
vm.native_arities.insert("set_promotion_age".to_string(), 1);
vm.native_arities.insert("set_gc_threads".to_string(), 1);
vm.native_arities.insert("stats".to_string(), 0);
vm.native_arities.insert("count".to_string(), 0);
vm.native_arities.insert("mem_alloc".to_string(), 0);
vm.native_arities.insert("mem_live".to_string(), 0);
```

### 2. GC 操作函数

> **GC 调用路径**（M3）：GC 函数为 free functions（`gc.rs`），需传 5 参数（heap + stack + globals + defer_stack + frames）。VM 字段为 `self.heap`（非 `self.gc`）。为简化 native 函数调用，封装 `VM::gc_full`/`VM::gc_minor_only` 方法。

```rust
// src/vm/mod.rs — VM 便捷方法
impl VM {
    /// Full GC = minor + major + finalizers（参照 gc::maybe_gc 的 Full 路径）。
    pub fn gc_full(&mut self) {
        gc::minor_gc(
            &mut self.heap, &mut self.stack, &mut self.globals,
            &mut self.defer_stack, &mut self.call_stack,
        );
        gc::major_gc(
            &mut self.heap, &mut self.stack, &self.globals,
            &self.defer_stack, &self.call_stack,
        );
        gc::run_finalizers(&mut self.heap);
    }

    /// 仅 Minor GC + finalizers。
    pub fn gc_minor_only(&mut self) {
        gc::minor_gc(
            &mut self.heap, &mut self.stack, &mut self.globals,
            &mut self.defer_stack, &mut self.call_stack,
        );
        gc::run_finalizers(&mut self.heap);
    }
}

// src/vm/stdlib.rs — native 函数
fn gc_collect(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.gc_full();
    Ok(Object::Nil)
}

fn gc_collect_minor(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.gc_minor_only();
    Ok(Object::Nil)
}

fn gc_enable(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    _vm.heap.gc_enabled = true;
    Ok(Object::Nil)
}

fn gc_disable(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    _vm.heap.gc_enabled = false;
    Ok(Object::Nil)
}

fn gc_is_enabled(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(Object::Bool(_vm.heap.gc_enabled))
}
```

### 3. GC 调优函数

```rust
fn gc_set_threshold(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let kind = match args.get(0) {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        other => return Err(format!(
            "TypeError: set_threshold(kind, value) expects string kind, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    };
    match kind.as_str() {
        "major" => {
            // Old GC 触发比率（float > 0）
            let ratio = match args.get(1) {
                Some(Object::Float(f)) => *f,
                Some(Object::Int(n)) => *n as f64,
                other => return Err(format!(
                    "TypeError: set_threshold(\"major\", value) expects float, got {}",
                    other.map(|o| o.type_name()).unwrap_or("missing")
                )),
            };
            if ratio <= 0.0 {
                return Err("ValueError: major threshold must be > 0".to_string());
            }
            // 更新 next_major_gc = bytes_allocated * ratio（近似 GOGC 语义）。
            let allocated = _vm.heap.bytes_allocated;
            _vm.heap.next_major_gc = (allocated as f64 * ratio) as usize;
            Ok(Object::Nil)
        }
        "minor" => {
            // Young 代大小（MB, 1-64）
            let mb = match args.get(1) {
                Some(Object::Int(n)) => *n,
                other => return Err(format!(
                    "TypeError: set_threshold(\"minor\", value) expects int, got {}",
                    other.map(|o| o.type_name()).unwrap_or("missing")
                )),
            };
            if !(1..=64).contains(&mb) {
                return Err(format!(
                    "ValueError: minor threshold must be 1-64 MB, got {}", mb
                ));
            }
            _vm.heap.next_minor_gc = (mb as usize) * 1024 * 1024;
            Ok(Object::Nil)
        }
        _ => Err(format!("ValueError: unknown threshold kind '{}'", kind)),
    }
}

fn gc_set_promotion_age(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let age = match args.get(0) {
        Some(Object::Int(n)) => *n,
        other => return Err(format!(
            "TypeError: set_promotion_age(age) expects int, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    };
    // V2：gc_meta age 字段仅 2 位（值 0-3），范围必须 [1,3]（14-gc.md:85）。
    if !(1..=3).contains(&age) {
        return Err(format!(
            "ValueError: promotion_age must be 1-3, got {}", age
        ));
    }
    _vm.heap.promotion_age = age as u8;
    Ok(Object::Nil)
}

fn gc_set_gc_threads(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let n = match args.get(0) {
        Some(Object::Int(n)) => *n,
        other => return Err(format!(
            "TypeError: set_gc_threads(n) expects int, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    };
    if n < 1 {
        return Err(format!("ValueError: gc_threads must be >= 1, got {}", n));
    }
    // M6/R2：MVP STW GC 为单线程，此值存入字段但不生效。
    // Phase 7.5 并发 GC 上线时由 gc.rs 读取此字段启动 Worker 线程池。
    // stats 中 gc_threads 固定返回 1（MVP），字段值仅记录用户偏好。
    _vm.heap.gc_threads_setting = n as u32;  // MsHeap 新增字段
    Ok(Object::Nil)
}
```

> **`set_gc_threads` MVP 存根**（M6/R2）：MVP (Phase 2.5) 为 STW 单线程 GC，无 GC Worker 线程池。此函数接受并校验参数、存入 MsHeap 字段，但 stats 中 `gc_threads` **固定返回 1**。Phase 7.5 并发 GC 上线时读取此字段启动 Worker 线程池。

### 4. GC 统计函数

> **快照语义**（V3/V4）：`gc_stats` 在函数入口一次性采集全部数值快照（值拷贝），后续 `alloc_string`/`alloc_dict` 分配期间即使触发 GC 也不影响已采集的快照值。返回的 dict 反映**调用时刻**的堆状态（与 Python `gc.get_stats()` 一致）。
>
> **`alloc_dict` 签名**（M7）：`alloc_dict(map: DictMap) -> Object`（接受 DictMap 所有权，非 `&Vec`）。先构建完整 `DictMap` 再传所有权。

```rust
fn gc_stats(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    // 一次性快照（值拷贝），避免后续分配触发 GC 导致中间值变动。
    let h = &vm.heap;
    let minor_count = h.minor_count as i64;
    let major_count = h.major_count as i64;
    let total_pause_ns = h.total_pause_ns as i64;
    let last_pause_ns = h.last_pause_ns as i64;
    let bytes_freed = h.bytes_freed as i64;
    let promotion_age = h.promotion_age as i64;
    let gc_enabled = h.gc_enabled;
    let bytes_allocated = h.bytes_allocated as i64;
    // young/old/los size：遍历对象列表累加 header.size。
    // SAFETY: young/old/los_objects 为 MsHeap 内部 Vec，对象在统计期间有效
    // （GC 在 native 函数内不触发 maybe_gc，因 gc_enabled 检查在 maybe_gc 而非 alloc）。
    let young_size = h.young_objects.iter()
        .map(|p| unsafe { (**p).size as i64 }).sum();
    let old_size = h.old_objects.iter()
        .map(|p| unsafe { (**p).size as i64 }).sum();
    let los_size = h.los_objects.iter()
        .map(|p| h.los_sizes.get(p).copied().unwrap_or(0) as i64)
        .sum();
    // gc_threads：MVP 固定 1（STW 单线程），忽略用户 set_gc_threads 设置。
    let gc_threads = 1i64;

    // 构建 DictMap（所有权转移给 alloc_dict）。
    let mut map = DictMap::new();
    map.insert(alloc_string("minor_count"), Object::Int(minor_count));
    map.insert(alloc_string("major_count"), Object::Int(major_count));
    map.insert(alloc_string("total_pause_ns"), Object::Int(total_pause_ns));
    map.insert(alloc_string("last_pause_ns"), Object::Int(last_pause_ns));
    map.insert(alloc_string("young_size"), Object::Int(young_size));
    map.insert(alloc_string("old_size"), Object::Int(old_size));
    map.insert(alloc_string("los_size"), Object::Int(los_size));
    map.insert(alloc_string("bytes_freed"), Object::Int(bytes_freed));
    map.insert(alloc_string("promotion_age"), Object::Int(promotion_age));
    map.insert(alloc_string("gc_threads"), Object::Int(gc_threads));
    map.insert(alloc_string("gc_enabled"), Object::Bool(gc_enabled));
    Ok(alloc_dict(map))
}

fn gc_count(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let total = vm.heap.minor_count + vm.heap.major_count;
    Ok(Object::Int(total as i64))
}

fn gc_mem_alloc(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(Object::Int(vm.heap.bytes_allocated as i64))
}

fn gc_mem_live(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    // mem_live = bytes_allocated - bytes_freed（累计已回收的部分）。
    // 注：bytes_freed 为累计值，bytes_allocated 也为累计值（不减回），
    // 故 live = 当前堆上所有对象 size 之和。
    let live = vm.heap.young_objects.iter()
        .map(|p| unsafe { (**p).size as usize })
        .sum::<usize>()
        + vm.heap.old_objects.iter()
            .map(|p| unsafe { (**p).size as usize })
            .sum::<usize>()
        + vm.heap.los_objects.iter()
            .map(|p| vm.heap.los_sizes.get(p).copied().unwrap_or(0))
            .sum::<usize>();
    Ok(Object::Int(live as i64))
}
```

> **`gc_threads` 固定返回 1**（M6）：MVP STW GC 单线程，用户经 `set_gc_threads(n)` 设置的值存入 `heap.gc_threads_setting` 但不影响 stats 返回值。Phase 7.5 并发 GC 上线后改为返回实际线程数。

## 验证标准

1. `gc.collect()` 成功触发 Full GC（minor + major + finalizers）
2. `gc.collect_minor()` 成功触发 Minor GC
3. `gc.enable()` / `gc.disable()` 正确切换自动 GC 状态；`maybe_gc` 在 disabled 时 no-op
4. `gc.is_enabled()` 返回与 enable/disable 一致的状态
5. `gc.stats()` 返回包含全部 11 个统计字段的 dict（minor_count/major_count/total_pause_ns/last_pause_ns/young_size/old_size/los_size/bytes_freed/promotion_age/gc_threads/gc_enabled）
6. `gc.set_threshold("major", 2.0)` 正确更新阈值；`set_threshold("minor", 4)` 正确更新 Young 代大小
7. `gc.set_threshold("unknown", ...)` 抛 `ValueError`；`set_threshold("major", -1.0)` 抛 `ValueError`；`set_threshold("minor", 0)` 抛 `ValueError`
8. `gc.set_promotion_age(3)` 正确更新；`set_promotion_age(0)` / `set_promotion_age(4)` 抛 `ValueError`
9. `gc.set_gc_threads(4)` 接受参数不报错；stats 中 `gc_threads` 固定返回 1（MVP STW 存根）
10. `gc.count()` 返回 `minor_count + major_count`
11. `gc.mem_alloc()` 返回 `bytes_allocated`
12. `gc.mem_live()` 返回当前堆存活对象总字节数
13. MsHeap 新增字段在 `minor_gc`/`major_gc` 中正确递增（minor_count/major_count/total_pause_ns/last_pause_ns/bytes_freed）
14. `gc.stats()` 返回快照语义：函数内 alloc 不影响已采集值

## 测试用例

```ms
import gc

gc.disable()
gc.collect()
stats = gc.stats()
print(stats["minor_count"])
print(stats["gc_enabled"])
gc.enable()
print(gc.is_enabled())
print(gc.mem_alloc())
print(gc.mem_live())
print(gc.count())
```

### test_gc_tuning.ms（调优与校验）

```ms
import gc

# set_threshold 合法值
gc.set_threshold("major", 1.5)
gc.set_threshold("minor", 8)

# set_promotion_age 合法值
gc.set_promotion_age(3)

# set_gc_threads MVP 存根（不报错，stats 固定返回 1）
gc.set_gc_threads(4)
stats = gc.stats()
print(stats["gc_threads"])    # 1（MVP STW）
print(stats["promotion_age"]) # 3
```

### test_gc_error.ms（错误路径，参照 task 50 §test_string_error.ms）

```ms
import gc

# set_promotion_age 越界
try { gc.set_promotion_age(0) } except e { print("age_low: " + str(e)) }
try { gc.set_promotion_age(4) } except e { print("age_high: " + str(e)) }
# set_threshold 未知 kind
try { gc.set_threshold("foo", 1) } except e { print("kind: " + str(e)) }
# set_threshold major 负值
try { gc.set_threshold("major", -1.0) } except e { print("major_neg: " + str(e)) }
# set_threshold minor 越界
try { gc.set_threshold("minor", 0) } except e { print("minor_low: " + str(e)) }
try { gc.set_threshold("minor", 65) } except e { print("minor_high: " + str(e)) }
```

预期输出：
```
age_low: ValueError: promotion_age must be 1-3, got 0
age_high: ValueError: promotion_age must be 1-3, got 4
kind: ValueError: unknown threshold kind 'foo'
major_neg: ValueError: major threshold must be > 0
minor_low: ValueError: minor threshold must be 1-64 MB, got 0
minor_high: ValueError: minor threshold must be 1-64 MB, got 65
```

> **错误路径备注**（同 task 50/51）：当前 VM 中原生函数 `Err(String)` 不可被 try/except 捕获（仅显式 `throw` 可捕获；影响全部 stdlib 模块的既有 VM 限制）。上述 `.ms` 测试记录错误契约；实际错误验证由 Rust 单元测试直接调用 native 函数完成。
