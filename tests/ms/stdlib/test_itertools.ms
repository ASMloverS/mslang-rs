# test_itertools.ms — itertools 嵌入模块（task 84）
# 覆盖: 验证标准 7（count 配 islice、cycle 首轮重复、chain/zip_longest/product/
#       combinations/permutations 与手写期望、accumulate 默认 + 与自定义 fn、
#       take_while/drop_while/pairwise/repeat/batched、边界与 ValueError）

import itertools

# ---------------------------------------------------------------------------
# count + islice（无限序列截断）
# ---------------------------------------------------------------------------

vals = []
for x in itertools.islice(itertools.count(10, 5), 0, 3) {
    vals.push(x)
}
assert(vals == [10, 15, 20], "count(start, step) 配 islice")

vals2 = []
for x in itertools.islice(itertools.count(), 2, 7, 2) {
    vals2.push(x)
}
assert(vals2 == [2, 4, 6], "islice(start, stop, step)")

inf = []
for x in itertools.islice(itertools.count(1), 3, nil) {
    inf.push(x)
    if inf.length() >= 4 {
        break
    }
}
assert(inf == [4, 5, 6, 7], "islice stop=nil 无限 + break（start=3 跳过前 3 项）")

finite = []
for x in itertools.islice([0, 1, 2, 3, 4], 1, nil) {
    finite.push(x)
}
assert(finite == [1, 2, 3, 4], "有限输入 stop=nil 取到耗尽")

# islice 参数校验
bad_step = false
try {
    itertools.islice([1], 0, 1, 0)
    for _x in itertools.islice([1], 0, 1, 0) {
        break
    }
} except ValueError as e {
    bad_step = true
}
assert(bad_step, "islice step<1 → ValueError")

# ---------------------------------------------------------------------------
# cycle / repeat
# ---------------------------------------------------------------------------

cyc = []
for x in itertools.islice(itertools.cycle([1, 2, 3]), 0, 7) {
    cyc.push(x)
}
assert(cyc == [1, 2, 3, 1, 2, 3, 1], "cycle 首轮重复环绕")

cyc_empty = []
for x in itertools.cycle([]) {
    cyc_empty.push(x)
}
assert(cyc_empty == [], "cycle 空输入立即结束")

rep = []
for x in itertools.repeat(7, 3) {
    rep.push(x)
}
assert(rep == [7, 7, 7], "repeat(x, n)")

rep_inf = []
for x in itertools.islice(itertools.repeat("z"), 0, 2) {
    rep_inf.push(x)
}
assert(rep_inf == ["z", "z"], "repeat(x) 无限")

# ---------------------------------------------------------------------------
# chain / zip_longest / product
# ---------------------------------------------------------------------------

ch = []
for x in itertools.chain([1, 2], "ab", [3]) {
    ch.push(x)
}
assert(ch == [1, 2, "a", "b", 3], "chain 混合可迭代")

ch0 = []
for x in itertools.chain() {
    ch0.push(x)
}
assert(ch0 == [], "chain() 零参空迭代")

zl = []
for t in itertools.zip_longest([1, 2, 3], ["a"]) {
    zl.push(t)
}
assert(zl == [tuple([1, "a"]), tuple([2, nil]), tuple([3, nil])], "zip_longest fill=nil")

zl0 = []
for t in itertools.zip_longest() {
    zl0.push(t)
}
assert(zl0 == [], "zip_longest() 零参空迭代")

pr = []
for t in itertools.product([1, 2], "xy") {
    pr.push(t)
}
assert(pr == [
    tuple([1, "x"]), tuple([1, "y"]),
    tuple([2, "x"]), tuple([2, "y"]),
], "product 笛卡尔积右端变化最快")

pr0 = []
for t in itertools.product() {
    pr0.push(t)
}
assert(pr0 == [tuple([])], "product() 零参产出一个空 tuple")

pr_empty = []
for t in itertools.product([1, 2], []) {
    pr_empty.push(t)
}
assert(pr_empty == [], "product 含空输入不产出")

pr3 = []
for t in itertools.product([0, 1], [0, 1], [0, 1]) {
    pr3.push(t)
}
assert(pr3.length() == 8, "三元 product 8 组合")

# ---------------------------------------------------------------------------
# take_while / drop_while / pairwise
# ---------------------------------------------------------------------------

tw = []
for x in itertools.take_while(fn(x) { return x < 3 }, [1, 2, 3, 1]) {
    tw.push(x)
}
assert(tw == [1, 2], "take_while 首个假即止")

dw = []
for x in itertools.drop_while(fn(x) { return x < 3 }, [1, 2, 3, 1]) {
    dw.push(x)
}
assert(dw == [3, 1], "drop_while 一次失假全放行")

pw = []
for t in itertools.pairwise([1, 2, 3, 4]) {
    pw.push(t)
}
assert(pw == [tuple([1, 2]), tuple([2, 3]), tuple([3, 4])], "pairwise 相邻对")

pw1 = []
for t in itertools.pairwise([9]) {
    pw1.push(t)
}
assert(pw1 == [], "pairwise 单元素无对")

# ---------------------------------------------------------------------------
# accumulate（验证标准 7：默认 + 与自定义 fn）
# ---------------------------------------------------------------------------

ac = []
for v in itertools.accumulate([1, 2, 3, 4]) {
    ac.push(v)
}
assert(ac == [1, 3, 6, 10], "accumulate 默认 +")

ac_fn = []
for v in itertools.accumulate([2, 3, 4], fn(a, b) { return a * b }) {
    ac_fn.push(v)
}
assert(ac_fn == [2, 6, 24], "accumulate 自定义 fn")

ac_empty = []
for v in itertools.accumulate([]) {
    ac_empty.push(v)
}
assert(ac_empty == [], "accumulate 空输入")

ac_str = []
for v in itertools.accumulate(["a", "b", "c"]) {
    ac_str.push(v)
}
assert(ac_str == ["a", "ab", "abc"], "accumulate 字符串拼接")

# ---------------------------------------------------------------------------
# combinations / permutations
# ---------------------------------------------------------------------------

cb = []
for t in itertools.combinations([1, 2, 3], 2) {
    cb.push(t)
}
assert(cb == [tuple([1, 2]), tuple([1, 3]), tuple([2, 3])], "combinations 字典序")

cb0 = []
for t in itertools.combinations([1, 2], 0) {
    cb0.push(t)
}
assert(cb0 == [tuple([])], "combinations r=0 产出一个空 tuple")

cb_over = []
for t in itertools.combinations([1], 2) {
    cb_over.push(t)
}
assert(cb_over == [], "combinations r>len 不产出")

pm = []
for t in itertools.permutations([1, 2, 3], 2) {
    pm.push(t)
}
assert(pm == [
    tuple([1, 2]), tuple([1, 3]),
    tuple([2, 1]), tuple([2, 3]),
    tuple([3, 1]), tuple([3, 2]),
], "permutations r=2 字典序 6 个")

pm_full = []
for t in itertools.permutations([1, 2, 3]) {
    pm_full.push(t)
}
assert(pm_full.length() == 6, "permutations r 缺省 = len")
assert(pm_full[0] == tuple([1, 2, 3]) and pm_full[5] == tuple([3, 2, 1]), "全排列端点")

pm_empty = []
for t in itertools.permutations([], 2) {
    pm_empty.push(t)
}
assert(pm_empty == [], "permutations 空输入")

pm_zero = []
for t in itertools.permutations([], 0) {
    pm_zero.push(t)
}
assert(pm_zero == [tuple([])], "permutations r=0 产出一个空 tuple")

# r 越界 → ValueError
pm_bad = false
try {
    for _t in itertools.permutations([1], -1) {
        break
    }
} except ValueError as e {
    pm_bad = true
}
assert(pm_bad, "permutations r<0 → ValueError")

cb_bad = false
try {
    for _t in itertools.combinations([1], -1) {
        break
    }
} except ValueError as e {
    cb_bad = true
}
assert(cb_bad, "combinations r<0 → ValueError")

# ---------------------------------------------------------------------------
# batched
# ---------------------------------------------------------------------------

bt = []
for b in itertools.batched([1, 2, 3, 4, 5], 2) {
    bt.push(b)
}
assert(bt == [tuple([1, 2]), tuple([3, 4]), tuple([5])], "batched 尾批可不满")

bt_exact = []
for b in itertools.batched([1, 2, 3, 4], 2) {
    bt_exact.push(b)
}
assert(bt_exact == [tuple([1, 2]), tuple([3, 4])], "batched 整除无尾批")

bt_empty = []
for b in itertools.batched([], 3) {
    bt_empty.push(b)
}
assert(bt_empty == [], "batched 空输入")

bt_bad = false
try {
    for _b in itertools.batched([1], 0) {
        break
    }
} except ValueError as e {
    bt_bad = true
}
assert(bt_bad, "batched n<1 → ValueError（Python 3.12）")

# 生成器输入（惰性管道组合）
pipeline = []
for b in itertools.batched(itertools.islice(itertools.count(1), 0, 5), 2) {
    pipeline.push(b)
}
assert(pipeline == [tuple([1, 2]), tuple([3, 4]), tuple([5])], "生成器输入物化")

print("ALL PASSED")
