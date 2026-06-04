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

### 文件位置

`src/stdlib/gc.rs`

### 注册方式

在 VM 初始化时，将 `gc` 模块注册为内置模块（不依赖文件系统加载）：

```rust
pub fn register_gc_module(vm: &mut VM) {
    let mut exports = HashMap::new();

    exports.insert("collect".into(), Object::BuiltinFn {
        name: "gc.collect".into(),
        arity: 0,
        func: gc_collect,
    });
    exports.insert("collect_minor".into(), Object::BuiltinFn {
        name: "gc.collect_minor".into(),
        arity: 0,
        func: gc_collect_minor,
    });
    exports.insert("enable".into(), Object::BuiltinFn {
        name: "gc.enable".into(),
        arity: 0,
        func: gc_enable,
    });
    exports.insert("disable".into(), Object::BuiltinFn {
        name: "gc.disable".into(),
        arity: 0,
        func: gc_disable,
    });
    exports.insert("is_enabled".into(), Object::BuiltinFn {
        name: "gc.is_enabled".into(),
        arity: 0,
        func: gc_is_enabled,
    });
    exports.insert("set_threshold".into(), Object::BuiltinFn {
        name: "gc.set_threshold".into(),
        arity: 2,
        func: gc_set_threshold,
    });
    exports.insert("set_promotion_age".into(), Object::BuiltinFn {
        name: "gc.set_promotion_age".into(),
        arity: 1,
        func: gc_set_promotion_age,
    });
    exports.insert("set_gc_threads".into(), Object::BuiltinFn {
        name: "gc.set_gc_threads".into(),
        arity: 1,
        func: gc_set_gc_threads,
    });
    exports.insert("stats".into(), Object::BuiltinFn {
        name: "gc.stats".into(),
        arity: 0,
        func: gc_stats,
    });
    exports.insert("count".into(), Object::BuiltinFn {
        name: "gc.count".into(),
        arity: 0,
        func: gc_count,
    });
    exports.insert("mem_alloc".into(), Object::BuiltinFn {
        name: "gc.mem_alloc".into(),
        arity: 0,
        func: gc_mem_alloc,
    });
    exports.insert("mem_live".into(), Object::BuiltinFn {
        name: "gc.mem_live".into(),
        arity: 0,
        func: gc_mem_live,
    });

    vm.builtin_modules.insert("gc".into(), Module { name: "gc".into(), exports, globals: HashMap::new() });
}
```

### gc_collect 实现

```rust
fn gc_collect(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    vm.gc.major_collect();
    vm.gc.minor_collect();
    Ok(Object::Nil)
}
```

### gc_stats 实现

```rust
fn gc_stats(vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let stats = vm.gc.get_stats();
    let mut pairs = Vec::new();
    pairs.push((Object::String("minor_count".into()), Object::Int(stats.minor_count as i64)));
    pairs.push((Object::String("major_count".into()), Object::Int(stats.major_count as i64)));
    pairs.push((Object::String("total_pause_ns".into()), Object::Int(stats.total_pause_ns as i64)));
    pairs.push((Object::String("last_pause_ns".into()), Object::Int(stats.last_pause_ns as i64)));
    pairs.push((Object::String("young_size".into()), Object::Int(stats.young_size as i64)));
    pairs.push((Object::String("old_size".into()), Object::Int(stats.old_size as i64)));
    pairs.push((Object::String("los_size".into()), Object::Int(stats.los_size as i64)));
    pairs.push((Object::String("bytes_freed".into()), Object::Int(stats.bytes_freed as i64)));
    pairs.push((Object::String("promotion_age".into()), Object::Int(stats.promotion_age as i64)));
    pairs.push((Object::String("gc_threads".into()), Object::Int(stats.gc_threads as i64)));
    pairs.push((Object::String("gc_enabled".into()), Object::Bool(stats.gc_enabled)));
    Ok(Object::Dict(pairs))
}
```

## 验证标准

1. `gc.collect()` 成功触发 Full GC
2. `gc.enable()` / `gc.disable()` 正确切换自动 GC 状态
3. `gc.is_enabled()` 返回正确的状态
4. `gc.stats()` 返回包含所有统计字段的 dict
5. `gc.set_threshold("major", 2.0)` 正确更新阈值
6. `gc.set_promotion_age(3)` 正确更新晋升年龄

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
