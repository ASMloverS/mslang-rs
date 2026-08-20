# test_collections.ms — 集合字面量、索引与运算
# 覆盖: task 22/25/26（02-types.md § List/Dict/Tuple/Set），35（切片另测）

# list 字面量与索引
lst = [1, 2, 3]
assert(lst[0] == 1, "list 正索引")
assert(lst[-1] == 3, "list 负索引")
assert(len(lst) == 3, "list len")

# list 拼接与重复语义（+ 拼接）
assert([1, 2] + [3] == [1, 2, 3], "list + 拼接")

# 嵌套与任意类型元素
nested = [1, ["a", [true, nil]], {"k": 1}]
assert(nested[1][1][0] == true, "嵌套索引")
assert(nested[2]["k"] == 1, "元素为 dict")

# dict 字面量：取值、赋值、len、键覆盖
d = {"a": 1, "b": 2}
assert(d["a"] == 1, "dict 取值")
d["b"] = 20
assert(d["b"] == 20, "dict 键覆盖")
d["c"] = 3
assert(len(d) == 3, "dict 新增键")
assert(d == {"a": 1, "b": 20, "c": 3}, "dict 相等与插入顺序无关")

# dict 值可为任意类型；键须可哈希
dd = {"list": [1, 2], "nested": {"x": true}}
assert(dd["list"][1] == 2, "dict 值为 list")
assert(dd["nested"]["x"], "dict 嵌套")

# 空容器：{} 是空 dict，空 set 用 set()
assert(type({}) == "dict", "空花括号是 dict")
assert(len(set([])) == 0, "空 set")
assert(len([]) == 0, "空 list")

# tuple：字面量、索引、不可变（无修改方法）、单元素需尾逗号
t = (1, "a", true)
assert(t[0] == 1 and t[2] == true, "tuple 索引")
assert(len(t) == 3, "tuple len")
single = (42,)
assert(single[0] == 42, "单元素元组尾逗号")
# tuple 拼接（+）未定义于规范，不做断言

# 元组解包
x, y = 10, 20
assert(x == 10 and y == 20, "二元解包")
a, b, c = [1, 2, 3]
assert(a + b + c == 6, "list 解包到三变量")
p, q = (30, 40)
assert(p == 30 and q == 40, "tuple 解包")

# 函数多返回值 + 解包（task 30）
fn minmax(nums) {
    lo = nums[0]
    hi = nums[0]
    for v in nums {
        if v < lo {
            lo = v
        }
        if v > hi {
            hi = v
        }
    }
    return lo, hi
}
lo_v, hi_v = minmax([3, 1, 4, 1, 5])
assert(lo_v == 1 and hi_v == 5, "多返回值解包")

# set：字面量去重、in、交并差对称差（02-types.md § Set 操作表）
s = {1, 2, 2, 3, 3, 3}
assert(len(s) == 3, "set 字面量去重")
assert(2 in s, "set in")
s1 = {1, 2}
s2 = {2, 3}
assert((s1 | s2) == {1, 2, 3}, "并集")
assert((s1 & s2) == {2,}, "交集")
assert((s1 - s2) == {1,}, "差集")
assert((s1 ^ s2) == {1, 3}, "对称差")

# 单元素 set 规范语法：{1,} 或 set([1])；{1} 是分组表达式
assert(type({1,}) == "set", "尾逗号单元素 set")
assert(type(set([1])) == "set", "set() 构造")
assert(({1}) == 1, "{1} 解析为分组表达式（规范 03-syntax.md）")

# set/list/tuple 构造转换函数
assert(list((1, 2)) == [1, 2], "tuple → list")
assert(tuple([3, 4]) == (3, 4), "list → tuple")
assert(set([1, 1, 2]) == {1, 2}, "list → set 去重")
assert(len(list("ab")) == 2, "string → list 字符")

# sorted / reversed / enumerate / zip / map / filter / any / all（task 26）
assert(sorted([3, 1, 2]) == [1, 2, 3], "sorted")
assert(sorted((2, 1)) == [1, 2], "sorted 接受 tuple")
assert(list(reversed([1, 2, 3])) == [3, 2, 1], "reversed")
assert(list(enumerate(["a", "b"])) == [(0, "a"), (1, "b")], "enumerate")
assert(list(zip([1, 2], ["x", "y"])) == [(1, "x"), (2, "y")], "zip")
fn double(v) { return v * 2 }
assert(map(double, [1, 2, 3]) == [2, 4, 6], "map")
fn even(v) { return v % 2 == 0 }
assert(filter(even, [1, 2, 3, 4]) == [2, 4], "filter")
assert(any([false, true, false]), "any 存在真")
assert((any([false, false])) == false, "any 全假")
assert(all([true, 1, "x"]), "all 全真")
assert((all([true, 0])) == false, "all 含假")

print("ALL PASSED")
