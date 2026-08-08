# Task 64 mslang 级别验证：GC 调优接口与自适应阈值。
# 参照 docs/mslang/tasks/64-gc-tuning.md § mslang 级别（aspirational 冒烟）。
#
# 注意：VM 字面量分配走 alloc_*（非 GC 堆），自适应引擎覆盖率有限（同 Task 63 已知限制）。
# 此测试验证调优 API 不 panic + 统计字段存在。

import gc

gc.set_concurrent(true)
gc.set_threshold("concurrent_mark", 0.7)
gc.set_threshold("major_interval_ms", 100)
gc.set_adaptive(true)
gc.set_gc_threads(1)

for i in range(500) {
    x = [i]
}
gc.collect()
s = gc.stats()
assert(s["gc_threads"] >= 1)
assert(s["adaptive_enabled"] == true)
assert(s["concurrent_enabled"] == true)
assert(s["young_size"] >= 4 * 1024 * 1024)
assert(s["young_live"] >= 0)
assert(s["major_gc_interval_ms"] == 100)
print("gc tuning ok")
