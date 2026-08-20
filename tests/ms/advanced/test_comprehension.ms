# test_comprehension.ms — 推导式（list / dict / set）
# 覆盖: task 33/34（07-advanced.md § 推导式）
# 注：推导式应位于赋值/return 等栈基位置（编译器局部槽位模型限制）

# list 推导式基本形式
squares = [x * x for x in range(5)]
assert(squares == [0, 1, 4, 9, 16], "基本 list 推导式")

# 带过滤条件
evens = [x for x in range(10) if x % 2 == 0]
assert(evens == [0, 2, 4, 6, 8], "过滤偶数")

# 遍历既有集合
letters = [ch for ch in "abc"]
assert(letters == ["a", "b", "c"], "遍历字符串")

upper = [w.upper() for w in ["hi", "yo"]]
assert(upper == ["HI", "YO"], "方法调用载荷")

# 双 for 子句（笛卡尔积风格）
pairs = [x * 10 + y for x in range(3) for y in range(3)]
assert(pairs == [0, 1, 2, 10, 11, 12, 20, 21, 22], "双 for 子句")

# 双 for + 过滤
filtered = [x * 10 + y for x in range(3) for y in range(3) if x != y]
assert(filtered == [1, 2, 10, 12, 20, 21], "双 for + 过滤")

# 嵌套推导式（外层载荷为内层推导式）
grid = [[y * 2 for y in range(3)] for x in range(2)]
assert(grid == [[0, 2, 4], [0, 2, 4]], "嵌套推导式")

# dict 推导式
d = {k: v for k, v in [("a", 1), ("b", 2)]}
assert(d == {"a": 1, "b": 2}, "dict 推导式遍历键值对")

squares_d = {x: x * x for x in range(4)}
assert(squares_d == {0: 0, 1: 1, 2: 4, 3: 9}, "键值同为表达式")

lengths = {w: w.length() for w in ["a", "bb", "ccc"]}
assert(lengths == {"a": 1, "bb": 2, "ccc": 3}, "方法求值")

# dict 推导式过滤
big = {k: v for k, v in [("a", 1), ("b", 5)] if v > 2}
assert(big == {"b": 5}, "dict 推导式过滤")

# set 推导式
mods = {x % 3 for x in range(10)}
assert(mods == {0, 1, 2}, "set 推导式去重")

uniq = {c for c in "banana"}
assert(uniq == {"b", "a", "n"}, "字符串去重")

# set 推导式过滤
uniq_f = {c for c in "banana" if c != "a"}
assert(uniq_f == {"b", "n"}, "set 推导式过滤")

# 推导式内外变量隔离：载荷变量不泄漏到外层作用域
leak_before = [zz for zz in range(3)]
assert((leak_before) == [0, 1, 2], "推导式结果正常")

# for 循环变量在推导式后仍可正常使用（无残留冲突）
zz = 99
after = [zz + 1 for zz in range(2)]
assert(after == [1, 2], "推导式遮蔽外层同名")
assert(zz == 99, "外层变量不被推导式修改")

print("ALL PASSED")
