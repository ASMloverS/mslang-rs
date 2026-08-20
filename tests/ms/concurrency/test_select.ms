# test_select.ms — select 多路复用
# 覆盖: task 59（08-concurrency.md § select）

# 就绪 case：命中第一个就绪分支
sc = channel(1)
sc <- "ready"
picked = ""
select {
case v = <-sc {
    picked = "recv:" + v
} default {
    picked = "default"
}
}
assert(picked == "recv:ready", "就绪接收分支")

# 无就绪 → default
empty = channel(1)
picked2 = ""
select {
case v2 = <-empty {
    picked2 = "recv"
} default {
    picked2 = "default"
}
}
assert(picked2 == "default", "无就绪走 default")

# 多 case：只有一个就绪
a = channel(1)
b = channel(1)
b <- "from-b"
result = ""
select {
case va = <-a {
    result = "a:" + va
} case vb = <-b {
    result = "b:" + vb
} default {
    result = "none"
}
}
assert(result == "b:from-b", "多 case 单就绪")

# 发送 case：缓冲有空位即就绪
sendable = channel(1)
sent = ""
select {
case sendable <- "sent-value" {
    sent = "sent"
} default {
    sent = "blocked"
}
}
assert(sent == "sent", "发送分支就绪")
assert((<-sendable) == "sent-value", "发送值可回收")

# 发送分支满 → default
full = channel(1)
full <- "occupies"
sent2 = ""
select {
case full <- "another" {
    sent2 = "sent"
} default {
    sent2 = "blocked"
}
}
assert(sent2 == "blocked", "满缓冲走 default")
assert((<-full) == "occupies", "原值未受影响")

# select 在循环中轮询（工作窃取式消费）
work = channel(3)
work <- 1
work <- 2
work <- 3
drained = 0
while true {
    got_one = false
    select {
case w = <-work {
        drained += w
        got_one = true
    } default {
        got_one = false
    }
    }
    if not got_one {
        break
    }
}
assert(drained == 6, "循环 select 排空")

# 与 go 配合：生产者 go + 主协程阻塞接收（无 default，确定性）
src = channel(2)
go fn() {
    src <- 10
    src <- 20
}()
acc = 0
for i in range(2) {
    acc += <-src
}
assert(acc == 30, "go 生产 + 阻塞消费")

print("ALL PASSED")
