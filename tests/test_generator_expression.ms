# 生成器表达式 — 惰性求值
squares = (x * x for x in range(5))
for s in squares {
    print(s)
}

# 带过滤的生成器表达式
nums = [1, -2, 3, -4, 5]
positives = (x for x in nums if x > 0)
for p in positives {
    print(p)
}
