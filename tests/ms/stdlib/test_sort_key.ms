# test_sort_key.ms — 排序增强（task 80）
# 覆盖: sorted(iterable, key?, reverse?) / sorted_by / list.sort / list.sort_by、
#       DSU 稳定性、reverse 等值保序、key 异常上抛（try/except 捕获）、GC 压力

import gc

# 无 key 兼容旧用例
assert(sorted([3, 1, 2]) == [1, 2, 3], "sorted 无 key")
assert(sorted([3, 1, 2], nil, true) == [3, 2, 1], "sorted nil key + reverse")

# key + 稳定性（等 key 元素保持原序）
words = ["bb", "a", "ccc", "dd", "e", "aa"]
by_len = sorted(words, fn(w) { return len(w) })
assert(by_len == ["a", "e", "bb", "dd", "aa", "ccc"], "len 稳定排序")

# reverse=true 反转比较器（等值元素保持原序，Python 语义）
rev = sorted(words, fn(w) { return len(w) }, true)
assert(rev == ["ccc", "bb", "dd", "aa", "a", "e"], "reverse 等值保序")

# sorted_by 与 sorted(iter, key) 等价
assert(sorted_by(words, fn(w) { return len(w) }) == by_len, "sorted_by 等价")
assert(sorted_by(words, fn(w) { return len(w) }, true) == rev, "sorted_by reverse")

# sorted 接受任意可迭代对象
assert(sorted((3, 1, 2), fn(x) { return -x }) == [3, 2, 1], "tuple + 降序 key")

# list.sort / sort_by 原地生效
lst = [3, 1, 2]
r = lst.sort()
assert(r == nil, "sort 返回 nil")
assert(lst == [1, 2, 3], "原地排序")

lst2 = ["bb", "a", "ccc"]
lst2.sort(fn(w) { return len(w) })
assert(lst2 == ["a", "bb", "ccc"], "list.sort(key)")

lst3 = [3, 1, 2]
lst3.sort(nil, true)
assert(lst3 == [3, 2, 1], "list.sort(nil, true)")

lst4 = ["bb", "a", "ccc"]
lst4.sort_by(fn(w) { return len(w) })
assert(lst4 == ["a", "bb", "ccc"], "list.sort_by(key)")

# DSU：单次 sort 每元素仅调用 key 一次（counter 经 list 装箱跨作用域可变）
counter = [0]
fn counting_key(x) {
    counter[0] = counter[0] + 1
    return x
}
sorted([5, 3, 1, 4, 2], counting_key)
assert(counter[0] == 5, "key 每元素仅调用一次")

# key 抛错上抛调用方（try/except 捕获 key 内抛出的 ValueError）
fn bad_key(x) {
    if x > 2 {
        throw ValueError("too big: " + str(x))
    }
    return x
}
caught = false
msg = ""
try {
    sorted([1, 3, 2], bad_key)
} except ValueError as e {
    caught = true
    msg = e.message
}
assert(caught, "sorted key 异常被调用方捕获")
assert(msg == "too big: 3", "异常消息")

caught2 = false
try {
    [1, 3, 2].sort_by(bad_key)
} except ValueError as e {
    caught2 = true
}
assert(caught2, "list.sort_by key 异常上抛")

# key 异常中止后不写回部分结果
lst5 = [3, 1]
try {
    lst5.sort(bad_key)
} except Error as e {
}
assert(lst5 == [3, 1], "key 异常后原 list 不变")

# GC 压力：大 list + key 内逐元素分配新对象 + gc.collect 后逐项校验
big = []
i = 0
while i < 200 {
    big.push(200 - i)
    i = i + 1
}
keyed = sorted(big, fn(x) {
    junk = ["k", x, x * 2]
    return x % 7
})
gc.collect()
assert(len(keyed) == 200, "长度保持")
k = 0
while k < len(keyed) - 1 {
    assert(keyed[k] % 7 <= keyed[k + 1] % 7, "按键非降")
    k = k + 1
}
total = 0
for v in keyed {
    total = total + v
}
assert(total == 20100, "元素总值不变（无丢失/错值）")

print("ALL PASSED")
