# test_http_errors.ms — http 模块错误路径（task 86，不依赖网络）
# 覆盖: 验证标准 6——https 拒绝 / 非法 URL / timeout_ms=0 与负数 / header 值
# 含 CRLF → await 抛 ValueError（try/except 捕获断言）

import http

# https → ValueError（TLS 不支持）
try {
    await http.get("https://example.com/")
    assert(false, "https should have been rejected")
} except as e {
    assert(e.type == "ValueError", "https -> " + e.type)
}

# 非法 URL：无 scheme / 其他 scheme / 缺 host / 端口越界 / host 非法字符
try {
    await http.get("example.com/")
    assert(false, "no-scheme should have been rejected")
} except as e {
    assert(e.type == "ValueError", "no-scheme -> " + e.type)
}
try {
    await http.get("ftp://example.com/")
    assert(false, "ftp-scheme should have been rejected")
} except as e {
    assert(e.type == "ValueError", "ftp-scheme -> " + e.type)
}
try {
    await http.get("http://")
    assert(false, "no-host should have been rejected")
} except as e {
    assert(e.type == "ValueError", "no-host -> " + e.type)
}
try {
    await http.get("http://example.com:99999/")
    assert(false, "port-overflow should have been rejected")
} except as e {
    assert(e.type == "ValueError", "port-overflow -> " + e.type)
}
try {
    await http.get("http://exa mple.com/")
    assert(false, "bad-host-char should have been rejected")
} except as e {
    assert(e.type == "ValueError", "bad-host-char -> " + e.type)
}

# timeout_ms = 0 / 负数 → ValueError（防 u64 回绕与 Windows 零超时异常）
try {
    await http.get("http://127.0.0.1:1/", {}, 0)
    assert(false, "timeout-zero should have been rejected")
} except as e {
    assert(e.type == "ValueError", "timeout-zero -> " + e.type)
}
try {
    await http.get("http://127.0.0.1:1/", {}, -5)
    assert(false, "timeout-negative should have been rejected")
} except as e {
    assert(e.type == "ValueError", "timeout-negative -> " + e.type)
}

# header 值含 CRLF → ValueError（请求头注入防护）
try {
    await http.get("http://127.0.0.1:1/", {"X-A": "v\r\nHost: evil"}, 1)
    assert(false, "header-crlf should have been rejected")
} except as e {
    assert(e.type == "ValueError", "header-crlf -> " + e.type)
}
# header 名非 token 字符集 → ValueError
try {
    await http.get("http://127.0.0.1:1/", {"Bad Name": "v"}, 1)
    assert(false, "header-bad-name should have been rejected")
} except as e {
    assert(e.type == "ValueError", "header-bad-name -> " + e.type)
}
# method 非 token → ValueError
try {
    await http.request("BAD METHOD", "http://127.0.0.1:1/", nil, nil, 1)
    assert(false, "bad-method should have been rejected")
} except as e {
    assert(e.type == "ValueError", "bad-method -> " + e.type)
}

# TypeError 族（类型不符）：url 非 string / timeout 非 int / headers 非 dict
try {
    await http.get(42)
    assert(false, "non-string url should have been rejected")
} except as e {
    assert(e.type == "TypeError", "url type -> " + e.type)
}
try {
    await http.get("http://127.0.0.1:1/", {}, "fast")
    assert(false, "non-int timeout should have been rejected")
} except as e {
    assert(e.type == "TypeError", "timeout type -> " + e.type)
}
try {
    await http.get("http://127.0.0.1:1/", ["not-a-dict"], 1)
    assert(false, "non-dict headers should have been rejected")
} except as e {
    assert(e.type == "TypeError", "headers type -> " + e.type)
}
# arity 超范围 → TypeError（MAX 自校验：get 1-3 参）
try {
    await http.get()
    assert(false, "get() arity should have been rejected")
} except as e {
    assert(e.type == "TypeError", "get arity -> " + e.type)
}

print("ALL PASSED")
