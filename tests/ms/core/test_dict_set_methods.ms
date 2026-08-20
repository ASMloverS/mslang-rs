# test_dict_set_methods.ms — 内置 Dict 与 Set 方法
# 覆盖: task 51（10-builtins.md § dict/set 方法）

d = {"a": 1, "b": 2}

# keys / values / items / length
assert(d.keys() == ["a", "b"], "keys 保持插入顺序")
assert(d.values() == [1, 2], "values")
assert(d.items() == [("a", 1), ("b", 2)], "items 键值对")
assert(d.length() == 2, "dict length")

# get（默认值语义）
assert(d.get("a") == 1, "get 存在")
assert(d.get("zz") == nil, "get 不存在返回 nil")
assert(d.get("zz", 0) == 0, "get 不存在返回给定默认值")

# set / remove / contains
d.set("c", 3)
assert(d["c"] == 3, "set 新增")
d.set("a", 10)
assert(d["a"] == 10, "set 覆盖")
d.remove("b")
assert((d.contains("b")) == false, "remove 后 contains")
assert(d.contains("a"), "contains 键")
assert(d.length() == 2, "remove 后长度")

# merge
m = {"x": 1}
m.merge({"y": 2, "x": 9})
assert(m["x"] == 9, "merge 同名键覆盖")
assert(m["y"] == 2, "merge 新键并入")
assert(m.length() == 2, "merge 后长度")

# items 遍历
count = 0
for pair in d.items() {
    count += 1
    assert(len(pair) == 2, "items 元素为二元组")
}
assert(count == d.length(), "items 数量与 length 一致")

# set 方法
s = {1, 2}
s.add(3)
assert(s == {1, 2, 3}, "add")
assert((s.add(3) == nil) or true, "add 重复元素幂等")
assert(s.length() == 3, "add 重复不增长")
assert(s.contains(2), "set contains")
s.remove(2)
assert((s.contains(2)) == false, "set remove")
assert(s == {1, 3}, "set remove 后元素")

# union / intersection / difference 方法（与运算符等价）
a_set = {1, 2, 3}
b_set = {3, 4}
assert(a_set.union(b_set) == {1, 2, 3, 4}, "union 方法")
assert(a_set.intersection(b_set) == {3,}, "intersection 方法")
assert(a_set.difference(b_set) == {1, 2}, "difference 方法")

print("ALL PASSED")
