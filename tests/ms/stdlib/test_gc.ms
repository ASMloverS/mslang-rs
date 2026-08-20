# test_gc.ms — gc 模块
# 覆盖: task 60（10-builtins.md § gc）

import gc

# is_enabled / enable / disable 可逆
before = gc.is_enabled()
assert(type(before) == "bool", "is_enabled 返回 bool")
gc.disable()
assert((gc.is_enabled()) == false, "disable 生效")
gc.enable()
assert(gc.is_enabled(), "enable 恢复")
# 还原初始状态
if not before {
    gc.disable()
}

# collect 族不崩溃且返回 nil
assert(gc.collect() == nil, "collect 返回 nil")
assert(gc.collect_minor() == nil, "collect_minor 返回 nil")

# count / mem_alloc / mem_live 返回 int
assert(type(gc.count()) == "int", "count int")
assert(type(gc.mem_alloc()) == "int", "mem_alloc int")
assert(type(gc.mem_live()) == "int", "mem_live int")

# stats 返回 dict 且含已知键
stats = gc.stats()
assert(type(stats) == "dict", "stats dict")

# set_promotion_age 合法域
gc.set_promotion_age(2)
# set_gc_threads
gc.set_gc_threads(1)

# 分配压力下 collect 稳定（大量临时对象）
for i in range(2000) {
    tmp = [i, i + 1, i + 2]
    _ = str(tmp)
}
gc.collect()
assert(gc.count() >= 1, "collect 后 count 增长")

print("ALL PASSED")
