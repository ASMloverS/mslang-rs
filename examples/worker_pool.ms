# worker_pool.ms — go 协程 + channel 并发示例
# 启动 4 个 worker 协程处理任务队列，聚合结果

fn worker(id, jobs, results) {
    for job in jobs {
        # 模拟计算：任务值的平方
        results <- [id, job, job * job]
    }
}

jobs = channel(8)
results = channel(8)

for j in range(1, 9) {
    jobs <- j
}

for w in range(4) {
    go worker(w, jobs, results)
}

total = 0
for i in range(8) {
    task = <-results
    print("worker", task[0], "job", task[1], "->", task[2])
    total += task[2]
}
print("total:", total)
