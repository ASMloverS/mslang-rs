# test_control_flow.ms — 控制流语句
# 覆盖: task 24（05-control-flow.md），task 32（for..in 迭代器协议）

# if / elif / else
grade = 85
if grade >= 90 {
    level = "A"
} elif grade >= 80 {
    level = "B"
} elif grade >= 70 {
    level = "C"
} else {
    level = "D"
}
assert(level == "B", "elif 分支命中")

grade2 = 60
if grade2 >= 90 {
    level2 = "A"
} elif grade2 >= 80 {
    level2 = "B"
} else {
    level2 = "D"
}
assert(level2 == "D", "else 分支命中")

# 嵌套 if
if true {
    if 1 < 2 {
        nested = "inner"
    }
}
assert(nested == "inner", "嵌套 if")

# while 循环
total = 0
i = 1
while i <= 100 {
    total += i
    i += 1
}
assert(total == 5050, "while 累加")

# while + break / continue
odd_sum = 0
j = 0
while true {
    j += 1
    if j > 10 {
        break
    }
    if j % 2 == 0 {
        continue
    }
    odd_sum += j
}
assert(odd_sum == 25, "while break/continue 奇数和")

# for..in 遍历列表 / 元组 / 字符串 / range / dict
acc = 0
for v in [10, 20, 30] {
    acc += v
}
assert(acc == 60, "for 遍历列表")

tup_acc = 0
for tv in (1, 2, 3) {
    tup_acc += tv
}
assert(tup_acc == 6, "for 遍历元组")

chars = 0
for ch in "abc" {
    chars += 1
}
assert(chars == 3, "for 遍历字符串")

r_sum = 0
for k in range(5) {
    r_sum += k
}
assert(r_sum == 10, "for 遍历 range(5)")

keys = []
for dk in {"x": 1, "y": 2} {
    keys.push(dk)
}
assert(len(keys) == 2 and "x" in keys and "y" in keys, "for 遍历 dict 键")

# range 带步长与负步长
fw = []
for r1 in range(2, 10, 3) {
    fw.push(r1)
}
assert(fw == [2, 5, 8], "range 正步长")

bw = []
for r2 in range(3, 0, -1) {
    bw.push(r2)
}
assert(bw == [3, 2, 1], "range 负步长")

# for + break / continue 与嵌套循环
collected = []
for outer in range(5) {
    if outer == 4 {
        break
    }
    if outer % 2 == 1 {
        continue
    }
    for inner in range(2) {
        collected.push(outer * 10 + inner)
    }
}
assert(collected == [0, 1, 20, 21], "嵌套循环 break/continue 只影响内层")

print("ALL PASSED")
