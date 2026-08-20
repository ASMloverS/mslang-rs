# test_slicing.ms — 切片操作
# 覆盖: task 35（07-advanced.md § 切片），语法 03-syntax.md postfix slice

lst = [0, 1, 2, 3, 4, 5]

# 基本区间
assert(lst[1:3] == [1, 2], "区间切片")
assert(lst[0:2] == [0, 1], "从头切片")

# 省略端点
assert(lst[4:] == [4, 5], "省略终点")
assert(lst[:2] == [0, 1], "省略起点")
assert(lst[:] == lst, "全量切片（副本值相等）")
full = lst[:]
assert(full == [0, 1, 2, 3, 4, 5], "全量切片内容")

# 步长
assert(lst[::2] == [0, 2, 4], "步长 2")
assert(lst[1::2] == [1, 3, 5], "起点 + 步长")
assert(lst[::3] == [0, 3], "步长 3")

# 负索引端点
assert(lst[-3:] == [3, 4, 5], "负起点到末尾")
assert(lst[:-2] == [0, 1, 2, 3], "从头到负终点")

# 字符串切片
s = "abcdef"
assert(s[1:4] == "bcd", "字符串区间")
assert(s[::2] == "ace", "字符串步长")
assert(s[-2:] == "ef", "字符串负端点")
assert(s[:] == "abcdef", "字符串全量")

# 元组切片
t = (10, 20, 30, 40)
assert(t[1:3] == (20, 30), "元组区间")
assert(t[::2] == (10, 30), "元组步长")

# 切片不修改原对象
orig = [1, 2, 3]
_ = orig[0:2]
assert(orig == [1, 2, 3], "切片不改原对象")

# 越界端点收敛
short = [1, 2, 3]
assert(short[0:99] == [1, 2, 3], "终点越界收敛到长度")
assert(short[99:] == [], "起点越界为空")

# 切片方法形式（String.slice，List.slice）
assert("hello".slice(1, 3) == "el", "String.slice")
assert([1, 2, 3, 4].slice(1, 3) == [2, 3], "List.slice")

print("ALL PASSED")
