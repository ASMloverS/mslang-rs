# test_channel.ms — channel 通信
# 覆盖: task 54（08-concurrency.md § channel）

# 无缓冲 channel：发送/接收 rendezvous
ch = channel()
go fn() {
    ch <- 42
}()
assert((<-ch) == 42, "无缓冲 rendezvous")

# 有缓冲 channel：先填充后消费
buf = channel(3)
buf <- 1
buf <- 2
buf <- 3
assert((<-buf) == 1, "缓冲 FIFO 1")
assert((<-buf) == 2, "缓冲 FIFO 2")
assert((<-buf) == 3, "缓冲 FIFO 3")

# 主协程生产、go 协程消费（无缓冲确保 rendezvous 顺序）
results = channel()
go fn() {
    v = <-results
    results <- v * 2
}
results <- 21
assert((<-results) == 42, "双向往返")

# 多生产者 fan-in（确定性：接收数量等于发送数量）
fan = channel(6)
for i in range(6) {
    go fn(idx) {
        fan <- idx
    }(i)
}
total = 0
for j in range(6) {
    total += <-fan
}
assert(total == 0 + 1 + 2 + 3 + 4 + 5, "fan-in 总和")

# go 参数为启动时刻快照（每轮独立值）
snap = channel(3)
for i in range(3) {
    go fn(idx) {
        snap <- idx * 100
    }(i)
}
vals = []
for k in range(3) {
    vals.push(<-snap)
}
vals.sort()
assert(vals == [0, 100, 200], "参数快照排序后")

# for 循环变量为局部，go 启动时按轮次快照（捕获经上值）
snap_direct = channel(3)
for i in range(3) {
    go fn() {
        snap_direct <- i
    }()
}
sv = []
for k in range(3) {
    sv.push(<-snap_direct)
}
sv.sort()
assert(sv == [0, 1, 2], "循环变量按轮次快照")

# 顶层变量为全局：协程延迟读取（循环结束后的末值）
gv = 0
snap_global = channel(3)
for i in range(3) {
    gv = i
    go fn() {
        snap_global <- gv
    }()
}
sg = []
for k in range(3) {
    sg.push(<-snap_global)
}
sg.sort()
assert(sg == [2, 2, 2], "全局变量延迟读取末值")

# channel 作为一等值传递
fn pipe(src, dst) {
    dst <- (<-src)
}
c1 = channel(1)
c2 = channel(1)
c1 <- "x"
go pipe(c1, c2)
assert((<-c2) == "x", "channel 传参")

print("ALL PASSED")
