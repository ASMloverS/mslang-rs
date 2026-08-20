# test_list_methods.ms — 内置 List 方法
# 覆盖: task 51（10-builtins.md § list 方法）

lst = [3, 1, 4, 1, 5]

# push / pop / insert / remove
lst.push(9)
assert(lst == [3, 1, 4, 1, 5, 9], "push 尾部追加")
assert(lst.pop() == 9, "pop 无参弹出末尾")
assert(lst == [3, 1, 4, 1, 5], "pop 后列表")
assert(lst.pop(1) == 1, "pop 指定位置")
assert(lst == [3, 4, 1, 5], "pop 指定位置后列表")
lst.insert(0, 0)
assert(lst == [0, 3, 4, 1, 5], "insert 头部")
lst.insert(2, 2)
assert(lst == [0, 3, 2, 4, 1, 5], "insert 中间")
lst.remove(2)
assert(lst == [0, 3, 4, 1, 5], "remove 删除第一个匹配")
lst.remove(1)
assert(lst == [0, 3, 4, 5], "remove 删除匹配值")

# index / contains / length
assert(lst.index(4) == 2, "index 查位置")
assert(lst.contains(5), "contains 存在")
assert((lst.contains(99)) == false, "contains 不存在")
assert(lst.length() == 4, "length 方法")

# sort / reverse（原地修改）
unsorted = [3, 1, 4, 1, 5]
unsorted.sort()
assert(unsorted == [1, 1, 3, 4, 5], "sort 原地升序")
rev_src = [1, 2, 3]
rev_src.reverse()
assert(rev_src == [3, 2, 1], "reverse 原地反转")

# slice 方法
sl = [10, 20, 30, 40, 50]
assert(sl.slice(1, 3) == [20, 30], "slice 区间")
assert(sl.slice(0, 2) == [10, 20], "slice 从头")

# map / filter / reduce 方法（匿名函数实参）
assert([1, 2, 3].map(fn(x) { return x * 2 }) == [2, 4, 6], "map 方法")
assert([1, 2, 3, 4].filter(fn(x) { return x > 2 }) == [3, 4], "filter 方法")
assert([1, 2, 3].reduce(fn(a, b) { return a + b }, 0) == 6, "reduce 带初值")
assert([1, 2, 3].reduce(fn(a, b) { return a * b }) == 6, "reduce 无初值")

# 高阶方法组合
doubled_evens = [1, 2, 3, 4].filter(fn(x) { return x % 2 == 0 }).map(fn(x) { return x * 2 })
assert(doubled_evens == [4, 8], "filter 链接 map")

print("ALL PASSED")
