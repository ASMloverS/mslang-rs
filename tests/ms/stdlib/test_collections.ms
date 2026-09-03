# test_collections.ms — collections 嵌入模块（task 84）
# 覆盖: 验证标准 4-6（deque 对拍/空弹出/for-in/len、Counter 计数/缺失 0 不写入/
#       most_common/elements、defaultdict factory 触发与 get 不触发/KeyError）

import collections
import random

# ---------------------------------------------------------------------------
# deque（验证标准 4）
# ---------------------------------------------------------------------------

d = collections.deque()
assert(d.is_empty(), "新建 deque 为空")
assert(len(d) == 0, "len == 0")

d.push_back(1)
d.push_back(2)
d.push_front(0)
assert(d.to_list() == [0, 1, 2], "两端压入")
assert(len(d) == 3, "len == 3")
assert(d.front() == 0, "front")
assert(d.back() == 2, "back")

seen = []
for x in d {
    seen.push(x)
}
assert(seen == [0, 1, 2], "for-in 逻辑序")

d.extend([3, 4])
assert(d.to_list() == [0, 1, 2, 3, 4], "extend 尾部追加")

assert(d.pop_front() == 0, "pop_front")
assert(d.pop_back() == 4, "pop_back")
assert(d.to_list() == [1, 2, 3], "两端弹出")

# 空弹出 → IndexError（.ms throw 可捕获）
empty = collections.deque()
for op in ["pop_front", "pop_back", "front", "back"] {
    hit = false
    try {
        if op == "pop_front" {
            empty.pop_front()
        }
        if op == "pop_back" {
            empty.pop_back()
        }
        if op == "front" {
            empty.front()
        }
        if op == "back" {
            empty.back()
        }
    } except IndexError as e {
        hit = true
    }
    assert(hit, "空 deque " + op + " 抛 IndexError")
}

# 2000 次混合 push/pop 序列与对拍参考实现一致（.ms 内 list 参考双端行为）
random.seed(4)
dq = collections.deque()
ref = []
ref_front = false   # 参考实现：pop_front 用 remove 首个等于值？不可——参考用
                    # 普通 list + 头索引模拟（见下）
# 参考实现：head 偏移 list（逻辑视角）
ref_list = []
ref_head = 0
for step in range(2000) {
    op = random.randint(0, 5)
    if op == 0 {
        v = random.randint(0, 10000)
        dq.push_back(v)
        ref_list.push(v)
    }
    if op == 1 {
        v = random.randint(0, 10000)
        dq.push_front(v)
        ref_list.insert(ref_head, v)
    }
    if op == 2 {
        # pop_front：参考实现物化头索引
        if ref_head < ref_list.length() and not dq.is_empty() {
            a = dq.pop_front()
            b = ref_list[ref_head]
            ref_head += 1
            assert(a == b, "pop_front 对拍不一致 step=" + str(step))
        }
    }
    if op == 3 {
        if not dq.is_empty() {
            a = dq.pop_back()
            if ref_head < ref_list.length() {
                b = ref_list.pop()
                assert(a == b, "pop_back 对拍不一致 step=" + str(step))
            }
        }
    }
    if op == 4 {
        if not dq.is_empty() {
            assert(dq.front() == ref_list[ref_head], "front 对拍")
            assert(dq.back() == ref_list[ref_list.length() - 1], "back 对拍")
        }
    }
    if op == 5 {
        assert(len(dq) == ref_list.length() - ref_head, "len 对拍")
    }
}
# 终态对拍
ref_tail = []
for i in range(ref_head, ref_list.length()) {
    ref_tail.push(ref_list[i])
}
assert(dq.to_list() == ref_tail, "终态一致")
assert(len(dq) == ref_list.length() - ref_head, "终态 len 一致")

# 环形缓冲环绕正确性（容量 4 起倍增：超过 4 元素后 head 环绕）
ring = collections.deque()
for i in range(10) {
    ring.push_back(i)
}
for i in range(10) {
    ring.push_front(100 + i)
}
expect_ring = []
for i in range(10) {
    expect_ring.push(109 - i)
}
for i in range(10) {
    expect_ring.push(i)
}
assert(ring.to_list() == expect_ring, "环绕后逻辑序")

# for-in 与 to_list 等价
iter_vals = []
for v in ring {
    iter_vals.push(v)
}
assert(iter_vals == ring.to_list(), "for-in == to_list")

# ---------------------------------------------------------------------------
# Counter（验证标准 5）
# ---------------------------------------------------------------------------

c = collections.Counter(["a", "b", "a", "c", "a"])
assert(c["a"] == 3, "计数 a == 3")
assert(c["b"] == 1, "计数 b == 1")
assert(c["z"] == 0, "缺失键读 0")
assert(not c.items().contains(tuple(["z", 0])), "缺失读不写入（Python 语义）")
assert(c.get("z") == 0, "get 缺省 0")
assert(c.get("z", 9) == 9, "get 自定义缺省")

# 空构造 + update
c2 = collections.Counter()
c2.update("abca")
assert(c2["a"] == 2, "update iterable 逐个计数")
c2.update(["a"])
assert(c2["a"] == 3, "update 追加")

# most_common 按频次降序（等频稳定保插入序）
mc = c.most_common()
assert(mc[0] == tuple(["a", 3]), "most_common 头部")
assert(mc.length() == 3, "most_common 全量长度")
mc2 = c.most_common(2)
assert(mc2.length() == 2, "most_common(n) 截断")
assert(mc2[0] == tuple(["a", 3]), "most_common(2) 头部")
eq_freq = collections.Counter(["x", "y"])
mc3 = eq_freq.most_common()
assert(mc3 == [tuple(["x", 1]), tuple(["y", 1])], "等频保插入序")

# elements 展开
elems = []
for x in c.elements() {
    elems.push(x)
}
assert(sorted(elems) == ["a", "a", "a", "b", "c"], "elements 展开")
assert(c.items().length() == 3, "items 长度")

# 空迭代器构造
c3 = collections.Counter()
assert(c3.items() == [], "空 Counter")

# ---------------------------------------------------------------------------
# defaultdict（验证标准 6）
# ---------------------------------------------------------------------------

dd = collections.defaultdict(fn() { return 0 })
assert(dd["missing"] == 0, "factory 触发并返回")
assert(dd.get("missing2") == nil, "get 不触发 factory（Python 一致）")
dd2 = collections.defaultdict(fn() { return [] })
dd2["k"].push(1)
dd2["k"].push(2)
assert(dd2["k"] == [1, 2], "factory 返回可变默认值独立使用")
dd2["j"].push(3)
assert(dd2["j"] == [3], "另一键独立默认值")
assert(dd2.get("k") == [1, 2], "get 已存在键")
assert(dd2.get("nope", "dflt") == "dflt", "get 自定义缺省")

# factory 为 nil → KeyError
dn = collections.defaultdict(nil)
hit = false
try {
    dn["x"]
} except KeyError as e {
    hit = true
}
assert(hit, "factory nil → KeyError")

print("ALL PASSED")
