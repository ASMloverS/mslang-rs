# Task 63 mslang 级别验证：并发清扫 + remembered set（冒烟）。
# 参照 docs/mslang/tasks/63-concurrent-sweep-compaction.md § mslang 级别验证。
#
# aspirational：当前 VM 字面量分配走 alloc_*（非 GC 堆），故此 mslang 测试不真正触达
# 并发清扫/remembered set 的 GC 路径，仅验证脚本不 panic + 存活对象语义正确。
# 真正覆盖由 Rust 单元/集成测试（直接用 gc_alloc_*）承担。

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
