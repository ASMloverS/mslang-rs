//! `http` 原生模块 + external completion 架构（task 86）。
//!
//! 参照 [86-stdlib-http](../../../docs/mslang/tasks/86-stdlib-http.md) 与
//! [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.18、§5。
//!
//! 架构：参数校验在 VM 线程完成（违例同步返回 rejected Future，不 spawn）；
//! 请求在 detached 后台线程执行（纯 Rust 数据，零 GC 交互），结果 push 进
//! `VM.external_completions` 队列，事件循环每轮由 VM 线程 drain（resolve/reject
//! Future + wake waiters + inflight 移除，见 vm/mod.rs `drain_external_completions`）。
//!
//! 手写 HTTP/1.1（std::net::TcpStream，无 TLS、无新依赖）：URL 解析
//! （scheme/host/port/path/query，IPv4 字面 host，不含 userinfo）、Content-Length
//! 与 chunked 解码、重定向 ≤5（301/302/303 → GET 丢 body；307/308 保方法与 body）。

use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_dict, alloc_exception, alloc_future, alloc_module, alloc_string, read_dict,
    read_module_mut, read_str, DictMap, FutureState, MsObjHeader, Object, TypeTag,
};
use crate::vm::{ExternalCompletion, ExternalResult, VM};

use super::expect_string;
use std::io::{BufRead, Read, Write};
use std::net::ToSocketAddrs;

/// 重定向最大跟随次数（>5 → IOError "too many redirects"）。
const MAX_REDIRECTS: usize = 5;
/// 默认超时（毫秒），覆盖连接与单次读。
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// 响应头区（状态行 + 头部）大小上限，防畸形服务器无限读。
const MAX_HEADER_SECTION_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// 模块注册（task 86）
// ---------------------------------------------------------------------------

/// 构造 `http` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 get/post/request 三个原生函数（arity MAX，各自自校验）。
pub fn register_http_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 3] = [
        ("get", native_http_get),
        ("post", native_http_post),
        ("request", native_http_request),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("http");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// 构造一个 Rejected Future（参数校验失败时使用）。await 时经 AWAIT Rejected
/// 路径抛出异常，可被 try/except 捕获（与 async 模块 rejected_future 一致）。
fn rejected_future(class_name: &str, message: &str) -> Object {
    let exc = alloc_exception(
        class_name,
        alloc_string(message),
        alloc_string(""),
        Object::Nil,
    );
    alloc_future(FutureState::Rejected(exc))
}

/// 三入口（参数位置不同，共用校验/派发逻辑）。
#[derive(Clone, Copy)]
enum HttpEntry {
    /// (url, headers?, timeout_ms=30000)：1-3 参。
    Get,
    /// (url, body, headers?, timeout_ms=30000)：2-4 参。
    Post,
    /// (method, url, body?, headers?, timeout_ms=30000)：2-5 参。
    Request,
}

fn native_http_get(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    http_request_entry(vm, args, HttpEntry::Get)
}

fn native_http_post(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    http_request_entry(vm, args, HttpEntry::Post)
}

fn native_http_request(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    http_request_entry(vm, args, HttpEntry::Request)
}

/// 入口共用实现：校验（VM 线程，违例同步 rejected Future，不 spawn）→
/// alloc Pending Future → inflight 登记 → spawn 后台线程 → 返回 Future。
fn http_request_entry(vm: &mut VM, args: &[Object], entry: HttpEntry) -> Result<Object, String> {
    let (method, url, body, mut headers, timeout_ms) =
        match validate_http_args(args, entry) {
            Ok(v) => v,
            Err(msg) => return Ok(rejected_future_from_message(&msg)),
        };
    // post 默认 Content-Type（用户未提供且带 body 时）。
    if matches!(entry, HttpEntry::Post)
        && body.is_some()
        && !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        headers.push((
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        ));
    }

    // alloc Pending Future + inflight 登记（GC 根集扩展，resolve 后移除）。
    let future_obj = alloc_future(FutureState::Pending);
    let Object::Ref(fp) = &future_obj else {
        unreachable!()
    };
    vm.inflight_futures.push(*fp);
    let spec = RequestSpec {
        method,
        url,
        body,
        user_headers: headers,
        timeout_ms,
    };
    match spawn_http_worker(spec, std::sync::Arc::clone(&vm.external_completions), *fp) {
        Ok(()) => Ok(future_obj),
        Err(e) => {
            // spawn 失败：回滚 inflight 登记（根集不变量），错误经 native Err 上抛。
            vm.inflight_futures.retain(|&p| p != *fp);
            Err(format!("IOError: failed to spawn http worker thread: {}", e))
        }
    }
}

/// 带类名前缀的校验错误消息 → rejected Future（类名缺省 IOError）。
fn rejected_future_from_message(msg: &str) -> Object {
    let (class, text) = split_error_class(msg);
    rejected_future(class, text)
}

// ---------------------------------------------------------------------------
// 参数校验（VM 线程）
// ---------------------------------------------------------------------------

/// 校验并提取请求参数。返回 `(method, url, body, headers, timeout_ms)`；
/// Err 消息携带 `类名: ` 前缀。
///
/// 校验规则（86-stdlib-http.md §http API）：
/// - timeout_ms 须为 Int 且 ≥1（非 Int → TypeError；负数/0 → ValueError，
///   防 `as u64` 回绕与 Windows 零超时异常）；
/// - header 名须为 HTTP token 字符集，名与值不得含 CR/LF/NUL（防请求头注入）；
/// - method 须为合法 token（大小写不敏感，统一大写）；
/// - 仅 `http://`；URL path/query 含控制字符 / 端口解析失败 → ValueError。
#[allow(clippy::type_complexity)]
fn validate_http_args(
    args: &[Object],
    entry: HttpEntry,
) -> Result<(String, ParsedUrl, Option<String>, Vec<(String, String)>, u64), String> {
    let (min_args, max_args, sig) = match entry {
        HttpEntry::Get => (1usize, 3usize, "http.get(url, headers?, timeout_ms=30000)"),
        HttpEntry::Post => (
            2,
            4,
            "http.post(url, body, headers?, timeout_ms=30000)",
        ),
        HttpEntry::Request => (
            2,
            5,
            "http.request(method, url, body?, headers?, timeout_ms=30000)",
        ),
    };
    if args.len() < min_args || args.len() > max_args {
        return Err(format!(
            "TypeError: {} takes {}-{} arguments, got {}",
            sig,
            min_args,
            max_args,
            args.len()
        ));
    }

    let (method_str, url_idx, body_idx, headers_idx, timeout_idx) = match entry {
        HttpEntry::Get => (None, 0, None, Some(1), Some(2)),
        HttpEntry::Post => (None, 0, Some(1), Some(2), Some(3)),
        HttpEntry::Request => (Some(0), 1, Some(2), Some(3), Some(4)),
    };

    // method（request 首参；get/post 固定）。
    let method = match method_str {
        Some(i) => {
            let m = expect_string(args.get(i), sig)?;
            let upper = m.to_ascii_uppercase();
            if !is_http_token(&upper) {
                return Err(format!("ValueError: invalid HTTP method: '{}'", m));
            }
            upper
        }
        None => match entry {
            HttpEntry::Post => "POST".to_string(),
            _ => "GET".to_string(),
        },
    };

    // url（必填 string）。
    let url_str = expect_string(args.get(url_idx), sig)?;
    let url = parse_http_url(&url_str)?;

    // body（可选 string；缺省或显式 nil 视为无 body）。
    let body = match body_idx.and_then(|i| args.get(i)) {
        None | Some(Object::Nil) => None,
        Some(arg) => Some(expect_string(Some(arg), sig)?),
    };

    // headers（可选 dict<string,string>；缺省或显式 nil 视为空）。
    let headers = match headers_idx.and_then(|i| args.get(i)) {
        None | Some(Object::Nil) => Vec::new(),
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            extract_headers(*ptr, sig)?
        }
        Some(other) => {
            return Err(format!(
                "TypeError: {} expects dict headers, got {}",
                sig,
                other.type_name()
            ))
        }
    };

    // timeout_ms（可选 Int ≥1；缺省或显式 nil 用默认值）。
    let timeout_ms = match timeout_idx.and_then(|i| args.get(i)) {
        None | Some(Object::Nil) => DEFAULT_TIMEOUT_MS,
        Some(Object::Int(n)) => {
            if *n < 1 {
                return Err(format!(
                    "ValueError: {} expects timeout_ms >= 1, got {}",
                    sig, n
                ));
            }
            *n as u64
        }
        Some(other) => {
            return Err(format!(
                "TypeError: {} expects int timeout_ms, got {}",
                sig,
                other.type_name()
            ))
        }
    };

    Ok((method, url, body, headers, timeout_ms))
}

/// 从 headers dict 提取 (name, value) 对并校验（键值均须 string；
/// 名为 HTTP token；名与值不得含 CR/LF/NUL——防请求头注入）。
fn extract_headers(ptr: *mut MsObjHeader, sig: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    // SAFETY: 调用方已校验 type_tag 为 DICT；块内完成提取即释放借用。
    {
        let map = unsafe { read_dict(ptr) };
        for (k, v) in map.items() {
            let name = match k {
                Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::STRING as u8 => {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    unsafe { read_str(*p) }.to_owned()
                }
                other => {
                    return Err(format!(
                        "TypeError: {} expects dict with string header names, got {}",
                        sig,
                        other.type_name()
                    ))
                }
            };
            let value = match v {
                Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::STRING as u8 => {
                    // SAFETY: type_tag 为 STRING，指针由 alloc_string 分配。
                    unsafe { read_str(*p) }.to_owned()
                }
                other => {
                    return Err(format!(
                        "TypeError: {} expects dict with string header values, got {}",
                        sig,
                        other.type_name()
                    ))
                }
            };
            if !is_http_token(&name) {
                return Err(format!("ValueError: invalid header name: '{}'", name));
            }
            if name.contains('\r') || name.contains('\n') || name.contains('\0')
                || value.contains('\r') || value.contains('\n') || value.contains('\0')
            {
                return Err(
                    "ValueError: header name/value must not contain CR/LF/NUL".to_string(),
                );
            }
            out.push((name, value));
        }
    }
    Ok(out)
}

/// RFC 7230 tchar（HTTP token 字符集：方法名与 header 名合法性）。
fn is_http_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            matches!(b,
                b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.'
                | b'^' | b'_' | b'`' | b'|' | b'~'
                | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z')
        })
}

// ---------------------------------------------------------------------------
// URL 解析（scheme/host/port/path/query；IPv4 字面 host；不含 userinfo）
// ---------------------------------------------------------------------------

/// 解析后的 http URL（scheme 恒为 http，由 parse_http_url 保证）。
#[derive(Clone, Debug)]
struct ParsedUrl {
    /// 主机名（小写；字母/数字/`-`/`.`——覆盖 IPv4 字面量）。
    host: String,
    port: u16,
    /// 以 `/` 开头的 path + query（无路径时为 "/"）。
    path_and_query: String,
}

/// 解析 `http://host[:port][/path][?query]`。
/// Err 消息携带 `ValueError: ` 前缀（https 专属提示 TLS 不支持）。
fn parse_http_url(url: &str) -> Result<ParsedUrl, String> {
    // scheme 分割（大小写不敏感）。
    let (scheme, rest) = match url.find("://") {
        Some(i) => (&url[..i], &url[i + 3..]),
        None => return Err(format!("ValueError: invalid URL (missing scheme): {}", url)),
    };
    if scheme.eq_ignore_ascii_case("https") {
        return Err(format!(
            "ValueError: https:// is not supported (no TLS in mslang-http): {}",
            url
        ));
    }
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(format!(
            "ValueError: unsupported URL scheme '{}' (only http://): {}",
            scheme, url
        ));
    }
    if rest.is_empty() {
        return Err(format!("ValueError: invalid URL (missing host): {}", url));
    }
    // fragment 不参与请求（RFC 3986：客户端发送前剥离）。
    let rest = rest.split('#').next().unwrap_or(rest);

    // authority 止于首个 '/' 或 '?'。
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.contains('@') {
        return Err(format!(
            "ValueError: userinfo in URL is not supported: {}",
            url
        ));
    }
    let (host, port_str) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if host.is_empty() {
        return Err(format!("ValueError: invalid URL (missing host): {}", url));
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(format!(
            "ValueError: invalid character in URL host: '{}'",
            host
        ));
    }
    let port = match port_str {
        None => 80,
        Some("") => {
            return Err(format!("ValueError: empty port in URL: {}", url))
        }
        Some(p) => p.parse::<u16>().map_err(|_| {
            format!("ValueError: invalid port in URL: ':{}'", p)
        })?,
    };
    let path_and_query = validate_path_and_query(tail)?;

    Ok(ParsedUrl {
        host: host.to_ascii_lowercase(),
        port,
        path_and_query,
    })
}

/// 校验 path/query 无控制字符（< 0x20 或 0x7F）；空 → "/"；`?` 开头补 "/"。
fn validate_path_and_query(tail: &str) -> Result<String, String> {
    if tail
        .bytes()
        .any(|b| b < 0x20 || b == 0x7f)
    {
        return Err("ValueError: URL path/query contains control characters".to_string());
    }
    Ok(if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{}", tail)
    } else {
        tail.to_string()
    })
}

/// 重定向 Location 解析（绝对 / 相对当前 URL；86-stdlib-http.md §重定向细节）。
/// 非 `http://` 目标 → ValueError（与入口 scheme 校验一致）。
fn resolve_location(base: &ParsedUrl, location: &str) -> Result<ParsedUrl, String> {
    let loc = location.trim();
    if loc.is_empty() {
        return Err("ValueError: empty Location header in redirect".to_string());
    }
    if loc.contains("://") {
        parse_http_url(loc)
    } else if let Some(rest) = loc.strip_prefix("//") {
        // scheme 相对（`//host/path`）→ 按绝对 http URL 解析。
        parse_http_url(&format!("http://{}", rest))
    } else if loc.starts_with('/') {
        Ok(ParsedUrl {
            host: base.host.clone(),
            port: base.port,
            path_and_query: validate_path_and_query(loc)?,
        })
    } else if loc.starts_with('?') {
        // query 替换：保留 base path。
        let base_path = base.path_and_query.split('?').next().unwrap_or("/");
        let merged = format!("{}{}", base_path, loc);
        Ok(ParsedUrl {
            host: base.host.clone(),
            port: base.port,
            path_and_query: validate_path_and_query(&merged)?,
        })
    } else {
        // 相对路径合并：取 base path 目录前缀 + loc（不做 dot-segment 归一化，v1 限制）。
        let base_path = base.path_and_query.split('?').next().unwrap_or("/");
        let dir_end = base_path.rfind('/').map(|i| i + 1).unwrap_or(0);
        let merged = format!("{}{}", &base_path[..dir_end], loc);
        Ok(ParsedUrl {
            host: base.host.clone(),
            port: base.port,
            path_and_query: validate_path_and_query(&merged)?,
        })
    }
}

// ---------------------------------------------------------------------------
// 请求执行（后台线程；纯 Rust 数据，零 GC 交互）
// ---------------------------------------------------------------------------

/// 后台线程携带的请求规格（Send：全 String/u64 纯数据）。
struct RequestSpec {
    /// 大写 HTTP 方法。
    method: String,
    url: ParsedUrl,
    body: Option<String>,
    /// 用户提供的额外头（已过 token/CRLF 校验）。
    user_headers: Vec<(String, String)>,
    timeout_ms: u64,
}

/// 单次请求响应（纯数据；VM 线程 drain 时转 alloc_dict/alloc_string）。
#[derive(Debug)]
struct HttpResponseData {
    status: u16,
    /// 小写键，按出现顺序保留（同名重复由 dict 构建侧逗号拼接）。
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// spawn detached 后台线程执行请求并 push Completion。
/// 线程闭包仅携带纯数据 + 队列 Arc + future tag（usize 传递满足 Send 约束，
/// 后台线程绝不解引用 future 指针）。JoinHandle 丢弃 → detached。
fn spawn_http_worker(
    spec: RequestSpec,
    queue: std::sync::Arc<std::sync::Mutex<Vec<ExternalCompletion>>>,
    future: *mut MsObjHeader,
) -> std::io::Result<()> {
    let tag = future as usize;
    std::thread::Builder::new()
        .name("mslang-http".to_string())
        .spawn(move || {
            // worker 体内整体 catch_unwind：panic 转换为 Err(String) push——
            // 防 Future 永不完成导致事件循环无限等待，亦防 panic 污染 Mutex
            //（push 在 catch_unwind 结果处理之后，锁只在最后短临界区持有）。
            let completion = execute_job(spec, tag as *mut MsObjHeader);
            let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(completion);
        })
        .map(|_| ())
}

/// 执行单个 HTTP job（catch_unwind 包装）。`worker` 参数化供单测注入 panic。
fn execute_job_with(
    spec: RequestSpec,
    future: *mut MsObjHeader,
    worker: fn(&RequestSpec) -> Result<HttpResponseData, String>,
) -> ExternalCompletion {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(&spec))) {
        Ok(Ok(resp)) => ExternalCompletion {
            future,
            result: ExternalResult::HttpResponse {
                status: resp.status,
                headers: resp.headers,
                body: resp.body,
            },
        },
        Ok(Err(msg)) => ExternalCompletion {
            future,
            result: ExternalResult::Error(msg),
        },
        Err(payload) => ExternalCompletion {
            future,
            result: ExternalResult::Error(format!(
                "IOError: http worker panicked: {}",
                panic_text(payload)
            )),
        },
    }
}

/// 默认 worker = perform_request。
fn execute_job(spec: RequestSpec, future: *mut MsObjHeader) -> ExternalCompletion {
    execute_job_with(spec, future, perform_request)
}

/// catch_unwind payload → 可读消息。
fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// 完整请求流程（含重定向循环）。所有错误路径 → Err（消息携带类名前缀，
/// 网络类 IOError / 重定向目标非法 ValueError）。
fn perform_request(spec: &RequestSpec) -> Result<HttpResponseData, String> {
    let mut method = spec.method.clone();
    let mut url = spec.url.clone();
    let mut body = spec.body.clone();
    let mut user_headers = spec.user_headers.clone();
    let mut redirects = 0usize;
    loop {
        let resp =
            single_request(&method, &url, body.as_deref(), &user_headers, spec.timeout_ms)?;
        let redirectable =
            matches!(resp.status, 301 | 302 | 303 | 307 | 308);
        if !redirectable {
            return Ok(resp);
        }
        // 无 Location 的 3xx 视为最终响应。
        let Some(location) = header_value(&resp.headers, "location") else {
            return Ok(resp);
        };
        if redirects >= MAX_REDIRECTS {
            return Err(format!(
                "IOError: too many redirects (max {}): {}",
                MAX_REDIRECTS, location
            ));
        }
        redirects += 1;
        let next = resolve_location(&url, &location)?;
        if matches!(resp.status, 301..=303) {
            // 301/302/303 → GET 且丢 body（body 描述头一并移除）。
            method = "GET".to_string();
            body = None;
            user_headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
        }
        // 307/308：保持方法与 body。Host 头随目标更新（默认头路径）。
        url = next;
    }
}

/// 响应头列表取值（键已小写）。
fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

/// 单次请求（连接 → 写请求 → 读响应）。
fn single_request(
    method: &str,
    url: &ParsedUrl,
    body: Option<&str>,
    user_headers: &[(String, String)],
    timeout_ms: u64,
) -> Result<HttpResponseData, String> {
    let timeout = std::time::Duration::from_millis(timeout_ms);
    // 连接（TcpStream::connect_timeout；超时覆盖连接阶段）。
    let addr_str = format!("{}:{}", url.host, url.port);
    let addrs: Vec<std::net::SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("IOError: failed to resolve host '{}': {}", url.host, e))?
        .collect();
    let mut stream = None;
    let mut last_err = None;
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let stream = stream.ok_or_else(|| {
        format!(
            "IOError: connect failed: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )
    })?;
    // 超时覆盖单次读/写（超时 → map_io_err 转 "timeout" 消息）。
    stream
        .set_read_timeout(Some(timeout))
        .map_err(map_io_err)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(map_io_err)?;

    let mut reader = std::io::BufReader::new(stream);
    let req = build_request_bytes(method, url, body, user_headers);
    {
        // 经 get_mut 取底层流写入（BufReader 不实现 Write）。
        let w = reader.get_mut();
        w.write_all(&req).map_err(map_io_err)?;
        w.flush().map_err(map_io_err)?;
    }

    // 状态行。
    let status_line = read_crlf_line(&mut reader)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(format!(
            "IOError: malformed status line: '{}'",
            truncate_for_error(&status_line)
        ));
    }
    let status: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| {
            format!(
                "IOError: invalid status code in: '{}'",
                truncate_for_error(&status_line)
            )
        })?;

    // 头部区。
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut section_bytes = status_line.len() + 2;
    loop {
        let line = read_crlf_line(&mut reader)?;
        section_bytes += line.len() + 2;
        if section_bytes > MAX_HEADER_SECTION_BYTES {
            return Err("IOError: response header section too large".to_string());
        }
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or_else(|| {
            format!(
                "IOError: malformed header line: '{}'",
                truncate_for_error(&line)
            )
        })?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return Err("IOError: malformed header line (empty name)".to_string());
        }
        headers.push((name.to_ascii_lowercase(), value.to_string()));
    }

    // body：chunked 优先于 Content-Length（RFC 7230 §3.3.3）；均无则读到 EOF
    //（Connection: close）。
    let chunked = header_value(&headers, "transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let resp_body = if chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(cl) = header_value(&headers, "content-length") {
        let n: usize = cl.trim().parse().map_err(|_| {
            format!("IOError: invalid Content-Length: '{}'", cl)
        })?;
        read_body_exactly(&mut reader, n)?
    } else {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).map_err(map_io_err)?;
        buf
    };

    Ok(HttpResponseData {
        status,
        headers,
        body: resp_body,
    })
}

/// 构造请求字节流。默认头 Host / User-Agent: mslang-http/0.1 / Connection: close
///（用户同名头覆盖；Host 取当前目标 url——重定向后已更新）。
fn build_request_bytes(
    method: &str,
    url: &ParsedUrl,
    body: Option<&str>,
    user_headers: &[(String, String)],
) -> Vec<u8> {
    let mut head = format!("{} {} HTTP/1.1\r\n", method, url.path_and_query);
    let has = |name: &str| {
        user_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name))
    };
    if !has("host") {
        let host = if url.port == 80 {
            url.host.clone()
        } else {
            format!("{}:{}", url.host, url.port)
        };
        head.push_str(&format!("Host: {}\r\n", host));
    }
    if !has("user-agent") {
        head.push_str("User-Agent: mslang-http/0.1\r\n");
    }
    if !has("connection") {
        head.push_str("Connection: close\r\n");
    }
    if let Some(bytes) = body.map(str::as_bytes) {
        head.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
    }
    for (k, v) in user_headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    if let Some(bytes) = body.map(str::as_bytes) {
        out.extend_from_slice(bytes);
    }
    out
}

/// 读一行（CRLF/LF 终结，剥行尾）。EOF / 超时 / IO 错误 → Err。
fn read_crlf_line(
    reader: &mut std::io::BufReader<std::net::TcpStream>,
) -> Result<String, String> {
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf).map_err(map_io_err)?;
    if n == 0 {
        return Err("IOError: connection closed before response completed".to_string());
    }
    while matches!(buf.last(), Some(b'\r') | Some(b'\n')) {
        buf.pop();
    }
    String::from_utf8(buf)
        .map_err(|_| "IOError: non-UTF8 bytes in response header line".to_string())
}

/// 按 Content-Length 读 body——流式追加、不按声明预 reserve（防伪造超大声明
/// 直接 OOM；v1 资源边界决策）。
fn read_body_exactly(
    reader: &mut std::io::BufReader<std::net::TcpStream>,
    n: usize,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while buf.len() < n {
        let want = std::cmp::min(chunk.len(), n - buf.len());
        let got = reader.read(&mut chunk[..want]).map_err(map_io_err)?;
        if got == 0 {
            return Err("IOError: connection closed before body completed".to_string());
        }
        buf.extend_from_slice(&chunk[..got]);
    }
    Ok(buf)
}

/// chunked 传输解码：`SIZE[;ext]\r\n data \r\n` 循环至 0-size chunk，
/// 尾随 trailer 行读到空行（容忍 EOF，Connection: close）。
fn read_chunked_body(
    reader: &mut std::io::BufReader<std::net::TcpStream>,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line = read_crlf_line(reader)?;
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| format!("IOError: invalid chunk size: '{}'", size_str))?;
        if size == 0 {
            // trailer 区：逐行读到空行；EOF / IO 错误容忍（body 已完整）。
            loop {
                let mut t = Vec::new();
                match reader.read_until(b'\n', &mut t) {
                    Ok(0) => break,
                    Ok(_) => {
                        if t == b"\r\n" || t == b"\n" {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            break;
        }
        let mut remaining = size;
        let mut chunk = [0u8; 8192];
        while remaining > 0 {
            let want = std::cmp::min(chunk.len(), remaining);
            let got = reader.read(&mut chunk[..want]).map_err(map_io_err)?;
            if got == 0 {
                return Err("IOError: connection closed during chunk".to_string());
            }
            body.extend_from_slice(&chunk[..got]);
            remaining -= got;
        }
        // chunk 数据后的 CRLF。
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).map_err(map_io_err)?;
        if &crlf != b"\r\n" {
            return Err("IOError: malformed chunk terminator".to_string());
        }
    }
    Ok(body)
}

/// io::Error → 带类名前缀消息。超时（Windows WouldBlock / Unix TimedOut）统一
/// "network operation timeout"（验证标准 5：消息含 timeout）。
fn map_io_err(e: std::io::Error) -> String {
    if matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) {
        format!("IOError: network operation timeout: {}", e)
    } else {
        format!("IOError: {}", e)
    }
}

/// 错误消息截断（防畸形服务器回显超长行；按字符边界截断）。
fn truncate_for_error(s: &str) -> String {
    if s.chars().count() <= 64 {
        return s.to_string();
    }
    let head: String = s.chars().take(64).collect();
    format!("{}...", head)
}

// ---------------------------------------------------------------------------
// VM 线程 drain 辅助（供 vm/mod.rs drain_external_completions 调用）
// ---------------------------------------------------------------------------

/// 响应 dict：`{"status": int, "headers": dict(键小写，同名逗号拼接),
/// "body": string(lossy UTF-8)}`。VM 线程执行（常规分配路径，无并发分配）。
pub(crate) fn http_response_dict(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> Object {
    // 同名合并（键已小写，按出现顺序）："a, b"。
    let mut merged: Vec<(String, String)> = Vec::new();
    for (k, v) in headers {
        if let Some(e) = merged.iter_mut().find(|(mk, _)| mk == k) {
            e.1.push_str(", ");
            e.1.push_str(v);
        } else {
            merged.push((k.clone(), v.clone()));
        }
    }
    let mut hd = DictMap::new();
    for (k, v) in merged {
        hd.insert(alloc_string(&k), alloc_string(&v));
    }
    let mut rd = DictMap::new();
    rd.insert(alloc_string("status"), Object::Int(status as i64));
    rd.insert(alloc_string("headers"), alloc_dict(hd));
    rd.insert(
        alloc_string("body"),
        alloc_string(&String::from_utf8_lossy(body)),
    );
    alloc_dict(rd)
}

/// 拆 `类名: 消息` 前缀 → (类名, 消息)。无识别前缀 → IOError（后台网络错误
/// 默认类）。
pub(crate) fn split_error_class(msg: &str) -> (&'static str, &str) {
    const CLASSES: [&str; 4] = ["ValueError", "TypeError", "IOError", "RuntimeError"];
    for class in CLASSES {
        let prefix = format!("{}: ", class);
        if let Some(rest) = msg.strip_prefix(&prefix) {
            return (class, rest);
        }
    }
    ("IOError", msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::run_source;

    // ---- 模块注册 ----

    #[test]
    fn test_register_http_module() {
        let ptr = register_http_module();
        // SAFETY: register_http_module 返回有效 MODULE Ref。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let module = read_module_mut(ptr);
            for name in &["get", "post", "request"] {
                let f = module
                    .exports
                    .get(*name)
                    .unwrap_or_else(|| panic!("missing export: {}", name));
                match f {
                    Object::Ref(p) => assert_eq!((**p).type_tag, TypeTag::FUNCTION as u8),
                    other => panic!("{} export is not a function ref: {:?}", name, other),
                }
            }
        }
    }

    // ---- URL 解析（验证标准 6 的解析器基础） ----

    #[test]
    fn test_parse_url_basic() {
        let u = parse_http_url("http://example.com").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.path_and_query, "/");

        let u = parse_http_url("http://127.0.0.1:8080/a/b?x=1&y=2").unwrap();
        assert_eq!(u.host, "127.0.0.1");
        assert_eq!(u.port, 8080);
        assert_eq!(u.path_and_query, "/a/b?x=1&y=2");

        // scheme 大小写不敏感；host 小写化；query-only；fragment 剥离。
        let u = parse_http_url("HTTP://ExAmple.COM?q=1#frag").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.path_and_query, "/?q=1");
    }

    #[test]
    fn test_parse_url_errors() {
        // https → TLS 不支持（专属消息）。
        let e = parse_http_url("https://example.com/").unwrap_err();
        assert!(e.starts_with("ValueError:") && e.contains("https"), "{}", e);
        // 无 scheme / 其他 scheme。
        assert!(parse_http_url("example.com/").is_err());
        assert!(parse_http_url("ftp://example.com/").is_err());
        // 端口越界（:99999 超 u16）与空端口。
        let e = parse_http_url("http://example.com:99999/").unwrap_err();
        assert!(e.contains("port"), "{}", e);
        assert!(parse_http_url("http://example.com:/").is_err());
        // userinfo 不支持。
        assert!(parse_http_url("http://user:pass@example.com/").is_err());
        // host 非法字符（IPv6 括号 / 空格）。
        assert!(parse_http_url("http://[::1]/").is_err());
        assert!(parse_http_url("http://exa mple.com/").is_err());
        // path/query 控制字符。
        let e = parse_http_url("http://example.com/a\u{01}b").unwrap_err();
        assert!(e.contains("control"), "{}", e);
        let e = parse_http_url("http://example.com/?q=\u{7f}").unwrap_err();
        assert!(e.contains("control"), "{}", e);
    }

    #[test]
    fn test_resolve_location() {
        let base = parse_http_url("http://a.com:81/dir/page?q=1").unwrap();
        // 绝对 URL。
        let r = resolve_location(&base, "http://b.com/x").unwrap();
        assert_eq!((r.host.as_str(), r.port, r.path_and_query.as_str()), ("b.com", 80, "/x"));
        // scheme 相对。
        let r = resolve_location(&base, "//c.com/y").unwrap();
        assert_eq!((r.host.as_str(), r.path_and_query.as_str()), ("c.com", "/y"));
        // 根相对。
        let r = resolve_location(&base, "/top").unwrap();
        assert_eq!((r.host.as_str(), r.port, r.path_and_query.as_str()), ("a.com", 81, "/top"));
        // query 替换（保留 base path）。
        let r = resolve_location(&base, "?z=9").unwrap();
        assert_eq!(r.path_and_query, "/dir/page?z=9");
        // 相对路径合并（取 base 目录前缀）。
        let r = resolve_location(&base, "sub").unwrap();
        assert_eq!(r.path_and_query, "/dir/sub");
        // 空 Location / https 目标 → ValueError。
        assert!(resolve_location(&base, "  ").is_err());
        let e = resolve_location(&base, "https://s.com/").unwrap_err();
        assert!(e.starts_with("ValueError:"), "{}", e);
    }

    // ---- VM 线程参数校验（同步 rejected Future，验证标准 6） ----

    /// 运行源码并要求全部断言通过（await rejected Future 经 try/except 捕获断言）。
    fn assert_ms_ok(src: &str) {
        let r = run_source(src);
        assert!(r.is_ok(), "ms source failed: {:?}\n{}", r.err(), src);
    }

    #[test]
    fn test_https_rejected_value_error() {
        assert_ms_ok(
            r#"
import http
try {
    await http.get("https://example.com/")
    assert(false, "https should have been rejected")
} except as e {
    assert(e.type == "ValueError", "got " + e.type)
}
"#,
        );
    }

    #[test]
    fn test_invalid_url_rejected() {
        assert_ms_ok(
            r#"
import http
for bad in ["notaurl", "ftp://x.com/", "http://", "http://a.com:99999/", "http://[::1]/"] {
    try {
        await http.get(bad)
        assert(false, "should have been rejected: " + bad)
    } except as e {
        assert(e.type == "ValueError", bad + " -> " + e.type)
    }
}
"#,
        );
    }

    #[test]
    fn test_timeout_ms_validation() {
        assert_ms_ok(
            r#"
import http
for bad in [0, -1] {
    try {
        await http.get("http://127.0.0.1:1/", {}, bad)
        assert(false, "timeout_ms should have been rejected: " + str(bad))
    } except as e {
        assert(e.type == "ValueError", "timeout " + str(bad) + " -> " + e.type)
    }
}
try {
    await http.get("http://127.0.0.1:1/", {}, "fast")
    assert(false, "non-int timeout should have been rejected")
} except as e {
    assert(e.type == "TypeError", "timeout str -> " + e.type)
}
"#,
        );
    }

    #[test]
    fn test_header_injection_rejected() {
        assert_ms_ok(
            r#"
import http
# 值含 CRLF（请求头注入防护）。
try {
    await http.get("http://127.0.0.1:1/", {"X-A": "v\r\nHost: evil"}, 1)
    assert(false, "CRLF header should have been rejected")
} except as e {
    assert(e.type == "ValueError", "crlf -> " + e.type)
}
# 名非 token 字符集。
try {
    await http.get("http://127.0.0.1:1/", {"Bad Name": "v"}, 1)
    assert(false, "bad header name should have been rejected")
} except as e {
    assert(e.type == "ValueError", "name -> " + e.type)
}
# headers 非 dict。
try {
    await http.get("http://127.0.0.1:1/", ["not-a-dict"], 1)
    assert(false, "non-dict headers should have been rejected")
} except as e {
    assert(e.type == "TypeError", "type -> " + e.type)
}
"#,
        );
    }

    #[test]
    fn test_arity_and_method_validation() {
        assert_ms_ok(
            r#"
import http
# arity 超范围（get 1-3 / post 2-4 / request 2-5）。
try {
    await http.get()
    assert(false, "get() should have been rejected")
} except as e {
    assert(e.type == "TypeError", "get arity -> " + e.type)
}
try {
    await http.post("http://127.0.0.1:1/")
    assert(false, "post(/) should have been rejected")
} except as e {
    assert(e.type == "TypeError", "post arity -> " + e.type)
}
# method 非 token。
try {
    await http.request("BAD METHOD", "http://127.0.0.1:1/", nil, nil, 1)
    assert(false, "bad method should have been rejected")
} except as e {
    assert(e.type == "ValueError", "method -> " + e.type)
}
# method 大小写不敏感（小写 get 合法——走校验后由超时拒绝，见集成测试）。
"#,
        );
    }

    #[test]
    fn test_fire_and_forget_rejected_gc_collect_no_crash() {
        // 验证标准 7 的无网络子集：rejected Future + 强制 GC 不 crash（inflight
        // 根集为 spawn 路径的覆盖见 tests/http_local.rs）。
        assert_ms_ok(
            r#"
import gc
import http
i = 0
while i < 5 {
    try {
        await http.get("https://example.com/")
    } except as e {
        assert(e.type == "ValueError")
    }
    i = i + 1
}
gc.collect()
assert(gc.stats().length() >= 0, "gc.stats() 正常返回")
"#,
        );
    }

    // ---- worker panic 安全（验证标准 11） ----

    #[test]
    fn test_execute_job_panic_becomes_io_error() {
        let spec = RequestSpec {
            method: "GET".to_string(),
            url: parse_http_url("http://127.0.0.1:1/").unwrap(),
            body: None,
            user_headers: Vec::new(),
            timeout_ms: 1,
        };
        let comp = execute_job_with(spec, std::ptr::null_mut(), |_| {
            panic!("injected worker panic")
        });
        match comp.result {
            ExternalResult::Error(msg) => {
                assert!(msg.contains("panicked"), "{}", msg);
                assert!(msg.starts_with("IOError:"), "{}", msg);
            }
            ExternalResult::HttpResponse { .. } => panic!("expected Error, got HttpResponse"),
        }
    }

    // ---- 后台断连（验证标准 10 的直连子集） ----

    #[test]
    fn test_perform_request_disconnect() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            // accept 后立即断连。
            while let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });
        let spec = RequestSpec {
            method: "GET".to_string(),
            url: parse_http_url(&format!("http://127.0.0.1:{}/", port)).unwrap(),
            body: None,
            user_headers: Vec::new(),
            timeout_ms: 2000,
        };
        let e = perform_request(&spec).unwrap_err();
        assert!(e.starts_with("IOError:"), "{}", e);
    }

    // ---- split_error_class / http_response_dict ----

    #[test]
    fn test_split_error_class() {
        assert_eq!(split_error_class("ValueError: x"), ("ValueError", "x"));
        assert_eq!(split_error_class("IOError: y"), ("IOError", "y"));
        assert_eq!(split_error_class("no prefix"), ("IOError", "no prefix"));
    }

    #[test]
    fn test_http_response_dict_shape() {
        // SAFETY: 读回 dict 校验形状（读后立即释放借用）。
        let obj = http_response_dict(
            200,
            &[
                ("content-type".to_string(), "text/plain".to_string()),
                ("x-multi".to_string(), "a".to_string()),
                ("x-multi".to_string(), "b".to_string()),
            ],
            b"hello",
        );
        let Object::Ref(p) = &obj else { unreachable!() };
        // SAFETY: http_response_dict 经 alloc_dict 分配。
        let d = unsafe { read_dict(*p) };
        assert_eq!(d.len(), 3);
        let status = d.get(&alloc_string("status")).cloned().unwrap();
        assert!(matches!(status, Object::Int(200)));
        let body = d.get(&alloc_string("body")).cloned().unwrap();
        let Object::Ref(bp) = &body else { unreachable!() };
        // SAFETY: alloc_string 分配。
        assert_eq!(unsafe { read_str(*bp) }, "hello");
        let Object::Ref(hp) = &d.get(&alloc_string("headers")).cloned().unwrap() else {
            unreachable!()
        };
        // SAFETY: alloc_dict 分配。
        let hd = unsafe { read_dict(*hp) };
        let Object::Ref(mp) = &hd.get(&alloc_string("x-multi")).cloned().unwrap() else {
            unreachable!()
        };
        // SAFETY: alloc_string 分配。
        assert_eq!(unsafe { read_str(*mp) }, "a, b");
    }
}
