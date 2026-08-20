# test_multi_return.ms — 多返回值与解包
# 覆盖: task 30（04-functions.md § 多返回值与元组解包）

# 基本多返回
fn divmod(a, b) {
    return a // b, a % b
}
q, m = divmod(17, 5)
assert(q == 3 and m == 2, "divmod 解包")

# 返回值本质是元组
t = divmod(17, 5)
assert(type(t) == "tuple", "多返回值为元组")
assert(t == (3, 2), "元组相等")
assert(t[0] == 3 and t[1] == 2, "元组索引")

# 三返回值
fn stats(nums) {
    lo = nums[0]
    hi = nums[0]
    total = 0
    for v in nums {
        if v < lo {
            lo = v
        }
        if v > hi {
            hi = v
        }
        total += v
    }
    return lo, hi, total
}
lo3, hi3, sum3 = stats([4, 1, 7, 2])
assert(lo3 == 1 and hi3 == 7 and sum3 == 14, "三返回值解包")

# 交换语义（右侧先全部求值）
a, b = 1, 2
a, b = b, a
assert(a == 2 and b == 1, "交换")

# 再次交换还原（右侧先求值的直接证据）
a, b = b, a
assert(a == 1 and b == 2, "再次交换还原")

# 接收为元组后二次解包
pair = (10, 20)
p1, p2 = pair
assert(p1 == 10 and p2 == 20, "元组二次解包")

# 多返回值作为函数调用链的一部分
fn both() {
    return "left", "right"
}
l, r = both()
assert(l + "|" + r == "left|right", "字符串多返回")

# 嵌套调用多返回
fn wrap() {
    return divmod(20, 6)
}
w1, w2 = wrap()
assert(w1 == 3 and w2 == 2, "嵌套调用的多返回")

# 返回值数量不匹配报错由 negative 用例覆盖

print("ALL PASSED")
