# Task 62 mslang 级别验证：并发标记 + 降级 STW。
# 参照 docs/mslang/tasks/62-concurrent-mark.md § mslang 级别验证。

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
