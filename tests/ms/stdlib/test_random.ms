# test_random.ms — random 模块（task 81）
# 覆盖: seed 确定性、值域、choice/sample/shuffle 语义
#       （16-stdlib-expansion.md §4.4 / task 81 验证标准 1-4 的值域部分）
# 错误路径（randint(2,1) ValueError、shuffle("abc") TypeError 等）为原生 Err，
# 不经 try/except（task 80 惯例），由 Rust 单测（stdlib/random.rs tests）覆盖。

import random

# seed 确定性：两次 seed(42) 的 random()/randint 序列一致（验证标准 1）
random.seed(42)
r1 = random.random()
r2 = random.random()
i1 = random.randint(1, 100)
random.seed(42)
r3 = random.random()
r4 = random.random()
i2 = random.randint(1, 100)
assert(r1 == r3 and r2 == r4, "seed 后 random() 序列一致")
assert(i1 == i2, "seed 后 randint 序列一致")

# random() ∈ [0, 1)
assert(r1 >= 0.0 and r1 < 1.0, "random 值域 [0,1)")

# randint 闭区间（验证标准 2 的值域部分）
assert(random.randint(1, 1) == 1, "randint(1,1) 单点闭区间")
ok = true
n = 0
while n < 200 {
    d = random.randint(1, 6)
    if d < 1 or d > 6 {
        ok = false
    }
    n = n + 1
}
assert(ok, "randint(1,6) 全部落在闭区间")

# uniform 端点语义（验证标准 3）
assert(random.uniform(0, 0) == 0.0, "uniform(0,0) = 0.0")
u = random.uniform(5, 10)
assert(u >= 5.0 and u <= 10.0, "uniform 值域")

# gauss：固定 seed 后均值近似 mu（确定性，无抖动）
random.seed(7)
total = 0.0
n = 0
while n < 5000 {
    total = total + random.gauss(10, 2)
    n = n + 1
}
mean = total / 5000
assert(mean > 9.0 and mean < 11.0, "gauss 均值近似 mu")

# choice：list/tuple/string（string 返回单字符 string）
c = random.choice([1, 2, 3])
assert(c == 1 or c == 2 or c == 3, "choice(list) 取值域")
cs = random.choice("abc")
assert(len(cs) == 1 and (cs == "a" or cs == "b" or cs == "c"), "choice(string) 单字符")
ct = random.choice((7, 8))
assert(ct == 7 or ct == 8, "choice(tuple)")

# shuffle：原地、返回 nil、多重集保持（验证标准 4 的值域部分）
lst = [1, 2, 3, 4, 5, 6, 7, 8]
ret = random.shuffle(lst)
assert(ret == nil, "shuffle 返回 nil")
assert(len(lst) == 8, "shuffle 长度不变")
assert(sorted(lst) == [1, 2, 3, 4, 5, 6, 7, 8], "shuffle 元素多重集不变")

# sample：不放回采样；string 总体返回单字符 list
smp = random.sample([1, 2, 3, 4], 3)
assert(len(smp) == 3, "sample 长度")
all_in = true
for v in smp {
    if v < 1 or v > 4 {
        all_in = false
    }
}
assert(all_in, "sample 元素来自总体")
assert(len(random.sample([1, 2, 3, 4], 0)) == 0, "sample n=0 空列表")
ss = random.sample("abcd", 2)
assert(len(ss) == 2 and len(ss[0]) == 1 and len(ss[1]) == 1, "string 采样单字符 list")

# 无参 seed()（系统熵）不报错（验证标准 10）
random.seed()

# GC 压力冒烟：大 list 洗牌后元素总值不变
import gc
big = []
n = 0
while n < 100 {
    big.push(n)
    n = n + 1
}
random.shuffle(big)
gc.collect()
total2 = 0
for v in sorted(big) {
    total2 = total2 + v
}
assert(total2 == 4950, "洗牌后元素总值不变")

print("ALL PASSED")
