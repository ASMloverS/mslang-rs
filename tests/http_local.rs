//! task 86：http 模块本地服务器集成测试。
//!
//! 参照 [86-stdlib-http](../docs/mslang/tasks/86-stdlib-http.md) §测试用例：
//! 内置 `TcpListener` 固定响应服务器（单线程逐连接处理），经 `ms run` 子进程
//! 驱动 .ms 脚本（复用 ms_corpus 模式），端口随机分配、注入生成的临时 .ms 脚本。
//! 覆盖验证标准 1-5、7-10（错误路径语料见 tests/ms/stdlib/test_http_errors.ms；
//! worker panic 注入见 src/vm/stdlib/http.rs 单测）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 单脚本执行超时（防事件循环死锁类缺陷挂起测试）。
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// 测试服务器
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Mode {
    /// 固定 200 响应（含重复头，供 header 合并断言）。
    Fixed,
    /// 请求路由 echo（method/path/body/content-type/x-custom）+ /chunked + /cl。
    Echo,
    /// 重定向链：/start 301→/mid 307→/end；/s303 303→/end；/hop/N 连跳。
    Redirect,
    /// accept 后不响应（驱动超时）。
    Hung,
    /// accept 后立即断连。
    Disconnect,
}

/// 绑定 127.0.0.1:0（随机端口）并启动 accept 循环线程，返回端口。
fn spawn_server(mode: Mode) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            match mode {
                Mode::Fixed => handle_fixed(&mut stream),
                Mode::Echo => handle_echo(&mut stream),
                Mode::Redirect => handle_redirect(&mut stream, port),
                Mode::Hung => std::thread::sleep(Duration::from_secs(5)),
                Mode::Disconnect => drop(stream),
            }
        }
    });
    port
}

/// 读取一个完整请求（头部 + Content-Length body）。
/// 返回 (method, path, body, headers_text)。
fn read_request(stream: &mut TcpStream) -> (String, String, String, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut head = String::new();
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                let s = String::from_utf8_lossy(&line).into_owned();
                let blank = s.trim().is_empty();
                head.push_str(&s);
                if blank {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let first = head.lines().next().unwrap_or("").to_string();
    let method = first.split(' ').next().unwrap_or("").to_string();
    let path = first.split(' ').nth(1).unwrap_or("").to_string();
    let cl = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find_map(|(k, v)| {
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let mut body = vec![0u8; cl];
    if cl > 0 {
        let _ = reader.read_exact(&mut body);
    }
    (method, path, String::from_utf8_lossy(&body).into_owned(), head)
}

/// 请求头取值（小写比对）。
fn req_header(head: &str, name: &str) -> String {
    for line in head.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

fn write_response(stream: &mut TcpStream, status_line: &str, headers: &[&str], body: &[u8]) {
    let mut resp = format!("{}\r\n", status_line);
    for h in headers {
        resp.push_str(h);
        resp.push_str("\r\n");
    }
    resp.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// 写原始（手工构造的）响应字节（chunked 用）。
fn write_raw(stream: &mut TcpStream, raw: &str) {
    let _ = stream.write_all(raw.as_bytes());
    let _ = stream.flush();
}

fn handle_fixed(stream: &mut TcpStream) {
    let _ = read_request(stream);
    write_response(
        stream,
        "HTTP/1.1 200 OK",
        &["Content-Type: text/plain", "X-Multi: a", "X-Multi: b"],
        b"hello world",
    );
}

fn handle_echo(stream: &mut TcpStream) {
    let (method, path, body, head) = read_request(stream);
    if path.starts_with("/chunked") {
        // 手工分块字节流 + 终结后的杂字节（校验解码器止于 0-chunk）。
        write_raw(
            stream,
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\nGARBAGE",
        );
        return;
    }
    if path.starts_with("/cl") {
        // Content-Length 截取：声明 5 字节但发送 9 字节。
        write_raw(stream, "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloXXXX");
        return;
    }
    let payload = format!(
        "method={}\npath={}\nbody={}\ncontent-type={}\nx-custom={}",
        method,
        path,
        body,
        req_header(&head, "content-type"),
        req_header(&head, "x-custom")
    );
    write_response(stream, "HTTP/1.1 200 OK", &[], payload.as_bytes());
}

fn handle_redirect(stream: &mut TcpStream, port: u16) {
    let (method, path, body, _head) = read_request(stream);
    let redirect = |stream: &mut TcpStream, status: &str, location: &str| {
        write_response(stream, status, &[&format!("Location: {}", location)], b"");
    };
    match path.as_str() {
        "/start" => redirect(stream, "HTTP/1.1 301 Moved Permanently", "/mid"),
        "/s303" => redirect(stream, "HTTP/1.1 303 See Other", "/end"),
        "/mid" => redirect(
            stream,
            "HTTP/1.1 307 Temporary Redirect",
            &format!("http://127.0.0.1:{}/end", port),
        ),
        "/end" => {
            let payload = format!("final:{}:{}", method, body);
            write_response(stream, "HTTP/1.1 200 OK", &[], payload.as_bytes());
        }
        p if p.starts_with("/hop/") => {
            let n: u32 = p
                .trim_start_matches("/hop/")
                .parse()
                .unwrap_or(0);
            if n >= 6 {
                write_response(stream, "HTTP/1.1 200 OK", &[], b"end");
            } else {
                redirect(
                    stream,
                    "HTTP/1.1 301 Moved Permanently",
                    &format!("/hop/{}", n + 1),
                );
            }
        }
        _ => write_response(stream, "HTTP/1.1 404 Not Found", &[], b""),
    }
}

// ---------------------------------------------------------------------------
// ms 子进程驱动（复用 ms_corpus 模式）
// ---------------------------------------------------------------------------

/// 生成临时 .ms 脚本并经 `ms run` 执行，返回 (exit_code, stdout, stderr)。
fn run_ms_script(test_name: &str, script: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join("mslang_http_tests");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(format!("{}.ms", test_name));
    std::fs::write(&path, script).expect("write temp script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ms"))
        .arg("run")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ms");
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().expect("wait after exit");
                return (
                    out.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                );
            }
            Ok(None) => {
                if start.elapsed() > SCRIPT_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (-1, String::new(), "script timeout".to_string());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("wait failed: {}", e),
        }
    }
}

/// 断言脚本成功（退出码 0 + stdout 以 OK 结尾），返回完整 stdout。
fn assert_script_ok(test_name: &str, script: &str) -> String {
    let (code, out, err) = run_ms_script(test_name, script);
    assert_eq!(code, 0, "{} failed\nstderr:\n{}", test_name, err);
    assert!(
        out.trim().ends_with("OK"),
        "{} unexpected stdout:\n{}",
        test_name,
        out
    );
    out
}

// ---------------------------------------------------------------------------
// 测试用例（验证标准 1-5、7-10）
// ---------------------------------------------------------------------------

/// 验证标准 1：基础 GET——status / headers（小写键、重复头逗号拼接）/ body。
#[test]
fn http_basic_get() {
    let port = spawn_server(Mode::Fixed);
    let script = format!(
        r#"
import http
r = await http.get("http://127.0.0.1:{}/a?x=1")
assert(r["status"] == 200, "status")
assert(r["headers"]["content-type"] == "text/plain", "lowercased header")
assert(r["headers"]["x-multi"] == "a, b", "joined: " + r["headers"]["x-multi"])
assert(r["body"] == "hello world", "body")
print("OK")
"#,
        port
    );
    assert_script_ok("basic_get", &script);
}

/// 验证标准 2：post body 传输（回显比对）+ 默认 Content-Type；request 自定义
/// method（大小写不敏感）；用户自定义头传输。
#[test]
fn http_post_echo_and_custom_method() {
    let port = spawn_server(Mode::Echo);
    let script = format!(
        r#"
import http
r = await http.post("http://127.0.0.1:{}/submit", "name=mslang")
assert(r["status"] == 200, "status")
assert(r["body"] == "method=POST\npath=/submit\nbody=name=mslang\ncontent-type=text/plain; charset=utf-8\nx-custom=", "echo: " + r["body"])
# 自定义 method（小写传入 → 大写发送）+ 用户头。
r2 = await http.request("delete", "http://127.0.0.1:{}/items/7", nil, {{"X-Custom": "cv"}})
assert(r2["body"] == "method=DELETE\npath=/items/7\nbody=\ncontent-type=\nx-custom=cv", "echo2: " + r2["body"])
print("OK")
"#,
        port, port
    );
    assert_script_ok("post_echo", &script);
}

/// 验证标准 3：chunked 响应正确拼接；Content-Length 响应正确截取。
#[test]
fn http_chunked_and_content_length() {
    let port = spawn_server(Mode::Echo);
    let script = format!(
        r#"
import http
r = await http.get("http://127.0.0.1:{}/chunked")
assert(r["body"] == "hello world", "chunked body: [" + r["body"] + "]")
r2 = await http.get("http://127.0.0.1:{}/cl")
assert(r2["body"] == "hello", "cl truncated: [" + r2["body"] + "]")
print("OK")
"#,
        port, port
    );
    assert_script_ok("chunked_cl", &script);
}

/// 验证标准 4：301/303 → GET 丢 body；307 保持方法与 body；六连跳超限 → IOError。
#[test]
fn http_redirect_semantics_and_limit() {
    let port = spawn_server(Mode::Redirect);
    let script = format!(
        r#"
import http
# 301 → GET 且丢 body → 307（保持 GET 无 body）→ 200。
r = await http.post("http://127.0.0.1:{}/start", "keepme")
assert(r["status"] == 200, "status")
assert(r["body"] == "final:GET:", "301 drops body: " + r["body"])
# 307 → 方法与 body 保持。
r2 = await http.request("POST", "http://127.0.0.1:{}/mid", "keepme")
assert(r2["body"] == "final:POST:keepme", "307 keeps: " + r2["body"])
# 303 → GET 且丢 body。
r3 = await http.request("PUT", "http://127.0.0.1:{}/s303", "b")
assert(r3["body"] == "final:GET:", "303 drops body: " + r3["body"])
# 六连跳（/hop/0 → /hop/6 需 6 次）超限 → IOError。
try {{
    await http.get("http://127.0.0.1:{}/hop/0")
    assert(false, "should have failed: too many redirects")
}} except as e {{
    assert(e.type == "IOError", "redirect limit type: " + e.type)
    print(e.message)
}}
print("OK")
"#,
        port, port, port, port
    );
    let out = assert_script_ok("redirect", &script);
    assert!(
        out.contains("too many redirects"),
        "redirect error message: {}",
        out
    );
}

/// 验证标准 5：对不响应的服务器 timeout_ms=200 → reject IOError（消息含 timeout）。
/// 同时覆盖验证标准 9：全部协程 await http 时事件循环等待外部完成而非报 deadlock
/// （本用例主协程是唯一协程，期间无 timer / 无就绪协程）。
#[test]
fn http_timeout_rejects_io_error() {
    let port = spawn_server(Mode::Hung);
    let script = format!(
        r#"
import http
try {{
    await http.get("http://127.0.0.1:{}/", {{}}, 200)
    assert(false, "should have timed out")
}} except as e {{
    assert(e.type == "IOError", "timeout type: " + e.type)
    print(e.message)
}}
print("OK")
"#,
        port
    );
    let out = assert_script_ok("timeout", &script);
    assert!(out.contains("timeout"), "timeout message: {}", out);
}

/// 验证标准 10：服务器提前断连 → reject IOError，VM 不 panic。
#[test]
fn http_disconnect_rejects_io_error() {
    let port = spawn_server(Mode::Disconnect);
    let script = format!(
        r#"
import http
try {{
    await http.get("http://127.0.0.1:{}/")
    assert(false, "should have failed on disconnect")
}} except as e {{
    assert(e.type == "IOError", "disconnect type: " + e.type)
}}
print("OK")
"#,
        port
    );
    assert_script_ok("disconnect", &script);
}

/// 验证标准 8 + 9：并发 10 个请求（go 协程内 await）全部正确返回；主协程同时
/// 直接 await http（多协程 await 外部完成不判死锁）。
#[test]
fn http_concurrent_ten_requests() {
    let port = spawn_server(Mode::Fixed);
    let script = format!(
        r#"
import http
ch = channel(10)
async fn fetch(n) {{
    r = await http.get("http://127.0.0.1:{}/c?n=" + str(n))
    ch <- r["status"]
}}
for i in range(10) {{
    go fetch(i)
}}
total = 0
for j in range(10) {{
    total += <-ch
}}
mine = await http.get("http://127.0.0.1:{}/main")
assert(total == 2000, "10 concurrent statuses sum: " + str(total))
assert(mine["status"] == 200, "main request")
print("OK")
"#,
        port, port
    );
    assert_script_ok("concurrent", &script);
}

/// 验证标准 7：fire-and-forget（不 await）+ 强制 GC——VM 无 crash、正常退出
///（「无泄漏」指无 crash / 无 GC 异常，退出即弃语义，非对象回收断言）。
#[test]
fn http_fire_and_forget_with_gc() {
    let port = spawn_server(Mode::Fixed);
    let script = format!(
        r#"
import gc
import http
i = 0
while i < 5 {{
    http.get("http://127.0.0.1:{}/ff")
    i = i + 1
}}
gc.collect()
assert(gc.stats().length() >= 0, "gc.stats() returns")
print("OK")
"#,
        port
    );
    assert_script_ok("fire_and_forget", &script);
}
