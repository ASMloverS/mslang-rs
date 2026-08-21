# 标准库 - http 模块与 external completion 架构

## 所属阶段
Phase 9 - 标准库扩展（M8）

## 前置任务
78-stdlib-split, 61-stdlib-async

> **依赖说明**：http 为手写 HTTP/1.1（std::net::TcpStream，无 TLS、无新依赖）；
> 返回 Future 需要 external completion 基础设施（后台线程 → VM 线程完成队列），
> 复用 task 53/61 的 Future 状态机与 EventLoop 调度循环。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.18、§5。

## 目标

1. 新增 **external completion** 基础设施：后台线程安全地完成 VM Future
   （通用机制，后续 net / 大文件 hash 可复用）。
2. `http` 模块：`get` / `post` / `request` 三入口，返回 `Future<dict>`。

## 设计规格

### external completion 架构

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §5：

```
脚本线程（VM/事件循环）                       后台线程
─────────────────────────                   ─────────────────────
http.get(url)
  ├─ alloc Pending Future
  ├─ inflight_futures.push(ptr)   ──GC 根──
  └─ spawn thread(请求参数, Arc<Mutex<Vec<Completion>>>)
                                             执行请求（纯 Rust 数据）
                                             lock queue.push(Completion{
                                                 future: ptr,   // 仅作 tag 传递，绝不解引用
                                                 result: Ok/Err(纯数据),
                                             })
事件循环每轮（timer 处理后）
  └─ drain completions:
       resolve / reject（VM 线程分配 dict/string）
       wake_waiters(ptr)
       inflight_futures.remove(ptr)
```

- `Completion.result`：`Ok(HttpResponseData{status, headers, body})` / `Err(String)`
  （错误消息，VM 线程转 reject）。
- **GC 安全**：`inflight_futures: Vec<*mut MsObjHeader>` 为 VM 级根集扩展（trace 扫描），
  保证 fire-and-forget 期间 Future 不被回收；resolve 后移除。
- **线程安全不变量**：后台线程闭包只携带 `Arc<Mutex<Vec>>` 与请求参数（String 等纯数据）；
  唯一的堆指针是作为完成标记的 future 裸指针，仅 VM 线程解引用。
- **Minor GC 移动**：inflight Future 若被 Minor GC 复制移动，`inflight_futures` 与
  Completion 中的 stale 指针需 forwarding 更新——实现方案：
  完成队列中的 future 字段在 VM 线程 drain 时以 `inflight_futures` 的当前值比对
  校验（见实现细节「指针失效防护」）。
- **关机**：线程 detached；解释器销毁后入队结果随 Arc 丢弃（开放问题 3，
  本 task 验证并回写结论）。

### http API

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.18：

| 函数 | 签名 | 说明 |
|---|---|---|
| get | (url, headers?, timeout_ms=30000) -> Future<dict> | |
| post | (url, body, headers?, timeout_ms=30000) -> Future<dict> | 默认 Content-Type `text/plain; charset=utf-8` |
| request | (method, url, body?, headers?, timeout_ms=30000) -> Future<dict> | method 大小写不敏感；arity MAX |

- 响应 dict：`{"status": int, "headers": dict(键小写，同名逗号拼接), "body": string(lossy UTF-8)}`。
- 仅 `http://`；`https://` → reject ValueError（TLS 开放问题 4）。
- 实现范围：URL 解析（scheme/host/port/path/query，IPv4 字面 host；不含 userinfo）、
  Content-Length 与 **chunked** 解码、重定向 ≤5（301/302/303 → GET 且丢 body；
  307/308 保持方法与 body）、默认头 Host / User-Agent: mslang-http/0.1 / Connection: close。
- 超时覆盖连接与单次读（`TcpStream::set_read_timeout`）；超时 → reject IOError。

## 实现细节

### 文件位置

- `src/vm/mod.rs` — VM 结构体新增 `external_completions:
  Arc<Mutex<Vec<ExternalCompletion>>>` 与 `inflight_futures: Vec<*mut MsObjHeader>`；
  EventLoop 主循环每轮（check_timers 之后）插入 `drain_external_completions()`；
  GC 根集扩展扫描 `inflight_futures`
- `src/vm/stdlib/http.rs` — `register_http_module` + 3 个 native 函数 +
  URL 解析器 + HTTP/1.1 请求写入/响应解析/chunked 解码/重定向循环（后台线程内执行）
- `src/vm/mod.rs` — 注册 + `native_arities`（get=MAX, post=MAX, request=MAX，
  各自自校验 1-4 参）

### ExternalCompletion 定义（src/vm/mod.rs 或独立小模块）

```rust
enum ExternalResult {
    HttpResponse { status: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    Error(String),
}
struct ExternalCompletion {
    future: *mut MsObjHeader,   // 完成标记；仅 VM 线程解引用
    result: ExternalResult,
}
```

VM 线程 drain 时：headers/body 在 VM 线程转 alloc_dict/alloc_string，
resolve_future + wake_waiters + inflight 移除。

### 指针失效防护（Minor GC 移动）

三选一（实现期定夺，验证后回写 16-stdlib-expansion.md）：

- **A（推荐）**：inflight Future 分配时**强制 Immortal 代**（参照 FileHandle/模块对象
  的 Immortal 模式），彻底免除移动问题；数量有限（并发请求级）无泄漏压力，
  resolve 后由 GC 正常回收（Immortal 亦可回收，仅不移动）。
- B：GC forwarding hook 更新 `inflight_futures` + drain 时 stale 指针以
  `inflight_futures.contains` 校验丢弃。
- C：完成标记改用自增 id（`u64`），VM 侧 `HashMap<u64, ptr>` 映射，队列零裸指针。

> 选择 A 时「resolve 后即移除」不变量依然必须保持（根集正确性），
> 否则 fire-and-forget 的 Future 泄漏（开放问题 7）。

### 后台线程请求流程（http.rs）

1. 参数解析/校验在 **VM 线程**完成（参数错误同步返回 rejected Future，不 spawn）。
2. spawn 线程携带：method/url/body（String）、headers（Vec<(String,String)>）、
   timeout_ms、`Arc<Mutex<Vec<ExternalCompletion>>>`、future 裸指针（tag）。
3. 线程内：URL 解析 → 连接（`TcpStream::connect_timeout`）→ 写请求 →
   读状态行+头 → 按Transfer-Encoding: chunked / Content-Length 读 body →
   重定向判定（≤5 次）→ push Completion。
4. 所有错误路径（连接失败/超时/协议解析失败/重定向超限）→ push Err(String)。

### 事件循环集成

```rust
// run_event_loop 每轮，check_timers 之后：
self.drain_external_completions();
if self.event_loop.ready_queue.is_empty()
    && self.event_loop.paused.is_empty()
    && !self.external_completions_arc.lock().is_empty()
{
    // 全部协程暂停但有 in-flight 外部请求：短暂 sleep（1ms）再循环，防忙等
}
```

死锁判定（"all coroutines paused" → Err）须放宽：存在 inflight_futures 非空时
不判死锁，等待外部完成。

## GC 安全

- `inflight_futures` 为根集扩展（Minor/Major 根扫描 + 若选方案 B/C 的对应处理）。
- 后台线程零 GC 交互（纯 Rust 数据）。
- resolve 在 VM 线程进行，分配 dict/string 走常规路径，无并发分配。
- 单测覆盖：fire-and-forget（不 await）+ 循环触发 `gc.collect()`，Future 与
  响应 dict 存活正确、无崩溃。

## 验证标准

1. `await http.get(url)` 对本地测试服务器返回正确 status/headers/body
2. post body 传输正确（服务器回显比对）；request 自定义 method 生效
3. chunked 响应正确拼接；Content-Length 响应正确截取
4. 重定向：301/302/303 改 GET 丢 body；307/308 保方法；>5 次 → reject IOError
5. 超时：对不响应的服务器 `timeout_ms=200` → reject IOError（消息含 timeout）
6. `https://` → await 抛 ValueError；非法 URL → reject ValueError
7. fire-and-forget：不 await 的请求完成后无泄漏、无崩溃（配合强制 GC）
8. 并发 10 个请求（go 协程内 await）全部正确返回
9. 事件循环不死锁：全部协程 await http 时等待外部完成而非报 deadlock
10. 后台线程崩溃安全：单测中服务器提前断连 → reject IOError，VM 不 panic
11. `cargo test` 全绿

## 测试用例

### Rust 集成测试（tests/http_local.rs，新增）

- 内置 `TcpListener` 固定响应服务器（单线程逐连接处理），覆盖验证标准 1-5、8、10：
  - 基础 GET/POST 回显
  - chunked 响应（手工写分块字节流）
  - 重定向链（301 → 307 → 200；六连跳超限）
  - 挂死连接（accept 后不响应）驱动超时
  - 断连（accept 后立即 close）
- 测试经 `ms run` 子进程驱动 .ms 脚本（复用 ms_corpus 模式），端口随机分配
  注入脚本（环境变量或生成临时 .ms）。

### tests/ms/stdlib/test_http_errors.ms

仅错误路径（不依赖网络）：验证标准 6（https 拒绝 / 非法 URL → await 抛错，
try/except 捕获断言）。
