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
- 参数校验（VM 线程完成，违例同步返回 rejected Future，不 spawn）：
  timeout_ms 须为 Int 且 ≥1（非 Int → TypeError；负数/0 → ValueError，防 `as u64`
  回绕与 Windows 零超时异常）；header 名须为 HTTP token 字符集，名与值不得含
  CR/LF/NUL（防请求头注入）；method 须为合法 token；URL path/query 含控制字符 →
  ValueError；端口 `parse::<u16>()` 失败（如 `:99999`）→ ValueError。
- 重定向细节：Location 支持绝对与相对 URL（相对当前 URL 解析）；无 Location 的
  3xx 视为最终响应；重定向目标非 `http://` → reject ValueError；Host 头随目标更新。
- 资源边界（v1 决策，记录为已知限制）：并发请求数与响应体大小**无上限**；body
  流式追加、不按 Content-Length 预 `reserve`（防伪造超大声明直接 OOM）。

## 实现细节

### 文件位置

- `src/vm/mod.rs` — VM 结构体新增 `external_completions:
  Arc<Mutex<Vec<ExternalCompletion>>>` 与 `inflight_futures: Vec<*mut MsObjHeader>`；
  EventLoop 主循环每轮（check_timers 之后）插入 `drain_external_completions()`；
  deadlock 判定放宽（见「事件循环集成」）
- `src/vm/gc.rs` + `src/vm/gc/major.rs` — GC 根集扩展落点：`minor_gc`/`major_gc`
  根集参数与并发标记 `scan_roots_gray` 增补 `inflight_futures` 扫描（与既有
  module_cache/c_roots/暂停协程占位注释一并补齐，或至少单独补 inflight）
- `src/vm/stdlib/http.rs` — `register_http_module` + 3 个 native 函数 +
  URL 解析器 + HTTP/1.1 请求写入/响应解析/chunked 解码/重定向循环（后台线程内执行）
- `src/vm/mod.rs` — 注册 + `native_arities`（get=MAX, post=MAX, request=MAX，
  各自自校验：get 1-3 参、post 2-4 参、request 2-5 参，超范围 → TypeError）
- `docs/mslang/10-builtins.md` — 新增 http API 章节；「未文档化的标准库模块」表
  移除 http 行（16-stdlib-expansion §2.4-3 / §8 交付项）

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

> **现状说明**：MsFuture 当前经 `alloc_future`（`Box::into_raw`）分配，非 GC 托管
> （gc.rs `TODO task 53: FUTURE`），既不移动也不被 GC 回收——本节为 GC 接管 Future
> 分配后的**前瞻设计**。实现期仅需在 `alloc_future` 补 Immortal 标记（方案 A），
> 无需 forwarding 基础设施（当前 minor_gc 亦无 VM 结构转发钩子）。

三选一（实现期定夺，验证后回写 16-stdlib-expansion.md）：

- **A（推荐）**：inflight Future 分配时**强制 Immortal 代**（参照 FileHandle/模块对象
  的 Immortal 模式），彻底免除移动问题。注意 **Immortal = 不移动且不回收**
  （14-gc.md §分代回收表：Gen 2「不回收、永不」）：当前 Box 分配下无影响；
  GC 接管 Future 分配后此方案将使 Future 永不回收（fire-and-forget 循环无界
  泄漏），届时须切换方案 C。
- B：GC forwarding hook 更新 `inflight_futures` + drain 时 stale 指针以
  `inflight_futures.contains` 校验丢弃。（依赖尚不存在的 VM 结构转发钩子，
  随 GC 接管一并实现。）
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
5. worker 体内整体 `catch_unwind`：panic 转换为 `Err(String)` push——防 Future
   永不完成导致事件循环无限等待（死锁判定已放宽），亦防 panic 污染 Mutex；
   VM 侧取锁用 `unwrap_or_else(|e| e.into_inner())` 抗毒化。

### 事件循环集成

```rust
// event_loop_run 每轮循环体开头，check_timers 之后：
self.drain_external_completions();
```

死锁判定处（现 mod.rs `try_wake_selects` / 空 select / timer sleep 之后、
`return Err("deadlock: all coroutines paused")` 之前）插入：

```rust
// task 86：无就绪协程但有暂停协程与 in-flight 外部请求——不判死锁，
// 短暂 sleep（1ms）防忙等
if !self.inflight_futures.is_empty() {
    std::thread::sleep(std::time::Duration::from_millis(1));
    continue;
}
```

> **退出语义**：事件循环退出条件（`ready_queue` 与 `paused` 皆空）**不**考虑
> inflight——fire-and-forget 且无协程暂停时循环直接退出，剩余 completion 不再
> drain（退出即弃，与 §7.3 msVmDestroy 行为一致），inflight 条目随 VM 丢弃。

## GC 安全

- `inflight_futures` 为根集扩展（Minor/Major 根扫描 + 若选方案 B/C 的对应处理）。
- 后台线程零 GC 交互（纯 Rust 数据）。
- resolve 在 VM 线程进行，分配 dict/string 走常规路径，无并发分配。
- 单测覆盖：fire-and-forget（不 await）+ 循环触发 `gc.collect()`，VM 无 crash、
  `gc.stats()` 正常返回（注：当前 Box 分配模型下 dict 不参与 GC 回收，根集
  正确性为 GC 接管 Future/dict 分配后的前瞻保障）。

## 验证标准

1. `await http.get(url)` 对本地测试服务器返回正确 status/headers/body
2. post body 传输正确（服务器回显比对）；request 自定义 method 生效
3. chunked 响应正确拼接；Content-Length 响应正确截取
4. 重定向：301/302/303 改 GET 丢 body；307/308 保方法；>5 次 → reject IOError
5. 超时：对不响应的服务器 `timeout_ms=200` → reject IOError（消息含 timeout）
6. `https://` → await 抛 ValueError；非法 URL / timeout_ms 非法 / header 含
   CRLF → reject ValueError
7. fire-and-forget：不 await 的请求 + 强制 GC，VM 无 crash、正常退出（「无泄漏」
   指无 crash / 无 GC 异常，退出即弃语义，非对象回收断言）
8. 并发 10 个请求（go 协程内 await）全部正确返回
9. 事件循环不死锁：全部协程 await http 时等待外部完成而非报 deadlock
10. 后台线程崩溃安全：单测中服务器提前断连 → reject IOError，VM 不 panic
11. worker panic 安全：单测注入 panic（catch_unwind 包装内触发）→ reject
    IOError，事件循环不挂起
12. `cargo test` 全绿

## 测试用例

### Rust 集成测试（tests/http_local.rs，新增）

- 内置 `TcpListener` 固定响应服务器（单线程逐连接处理），覆盖验证标准 1-5、8、
  10、11：
  - 基础 GET/POST 回显
  - chunked 响应（手工写分块字节流）
  - 重定向链（301 → 307 → 200；六连跳超限）
  - 挂死连接（accept 后不响应）驱动超时
  - 断连（accept 后立即 close）
- 测试经 `ms run` 子进程驱动 .ms 脚本（复用 ms_corpus 模式），端口随机分配
  注入脚本（环境变量或生成临时 .ms）。

### tests/ms/stdlib/test_http_errors.ms

仅错误路径（不依赖网络）：验证标准 6（https 拒绝 / 非法 URL / timeout_ms=0 与
负数 / header 值含 CRLF → await 抛 ValueError，try/except 捕获断言）。
