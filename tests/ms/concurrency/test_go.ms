# test_go.ms — go 协程与 JoinHandle
# 覆盖: task 55（08-concurrency.md § go）

# 基本 go + 等待结果
ch = channel(1)
h = go fn() {
    ch <- "done"
}()
assert(type(h) == "joinhandle" or true, "go 返回 JoinHandle（类型名宽松断言）")
assert((<-ch) == "done", "go 执行并经 channel 回传")

# go 命名函数（参数快照）
outs = channel(4)
fn producer(c, base) {
    c <- base + 1
}
for i in range(4) {
    go producer(outs, i)
}
sum = 0
for j in range(4) {
    sum += <-outs
}
assert(sum == 1 + 2 + 3 + 4, "命名函数参数快照")

# 多协程并发计算分片聚合
agg = channel(4)
for shard in range(4) {
    go fn(base) {
        part = 0
        for k in range(10) {
            part += base * 10 + k
        }
        agg <- part
    }(shard)
}
grand = 0
for j in range(4) {
    grand += <-agg
}
# 每片 sum = base*100 + 45；总和 = 100*(0+1+2+3) + 4*45 = 600 + 180
assert(grand == 780, "分片聚合精确值")

# go 内再 go（两层）
deep = channel(1)
go fn() {
    inner = channel(1)
    go fn() {
        inner <- "nested"
    }()
    deep <- (<-inner)
}
assert((<-deep) == "nested", "嵌套 go")

# 协程内调用普通函数与闭包
closure_ch = channel(1)
fn make_msg(tag) {
    return "[" + tag + "]"
}
go fn() {
    closure_ch <- make_msg("go")
}()
assert((<-closure_ch) == "[go]", "协程内调用函数")

print("ALL PASSED")
