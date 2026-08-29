# test_encoding.ms — encoding 模块（task 81）
# 覆盖: base64/hex/url 编解码往返与合法输入（16-stdlib-expansion.md §4.5）
# 非法输入（base64_decode("A") / hex_decode("abc") / url_decode("%ZZ") 等验证标准
# 6/8 的错误路径）为原生 Err，不经 try/except（task 80 惯例），
# 由 Rust 单测（stdlib/encoding.rs tests）覆盖。

import encoding

samples = ["", "f", "fo", "foo", "foob", "fooba", "foobar", "hello, mslang!", "中文Base64"]

# base64 往返（含中文/空串，验证标准 5）
for s in samples {
    enc = encoding.base64_encode(s)
    assert(len(enc) % 4 == 0, "base64 长度 4 倍数")
    assert(encoding.base64_decode(enc) == s, "base64 往返")
}

# RFC 4648 §10 测试向量
assert(encoding.base64_encode("") == "", "b64 空串")
assert(encoding.base64_encode("f") == "Zg==", "b64 f")
assert(encoding.base64_encode("fo") == "Zm8=", "b64 fo")
assert(encoding.base64_encode("foo") == "Zm9v", "b64 foo")
assert(encoding.base64_encode("foob") == "Zm9vYg==", "b64 foob")
assert(encoding.base64_encode("fooba") == "Zm9vYmE=", "b64 fooba")
assert(encoding.base64_encode("foobar") == "Zm9vYmFy", "b64 foobar")

# 解码 padding 0/1/2（验证标准 11 矩阵项）
assert(encoding.base64_decode("Zg==") == "f", "b64 解码 2 pad")
assert(encoding.base64_decode("Zm8=") == "fo", "b64 解码 1 pad")
assert(encoding.base64_decode("Zm9v") == "foo", "b64 解码 0 pad")

# 含 ASCII 空白输入剔除后成功（验证标准 10）
assert(encoding.base64_decode("Zm9v\nYg==\t ") == "foob", "b64 空白剔除")

# hex 往返 + 输出小写 + 大写输入等价（验证标准 11 矩阵项）
for s in samples {
    assert(encoding.hex_decode(encoding.hex_encode(s)) == s, "hex 往返")
}
assert(encoding.hex_encode("mslang") == "6d736c616e67", "hex 小写输出")
assert(encoding.hex_decode("4D53") == "MS", "hex 大写输入")
assert(encoding.hex_decode("4d53") == "MS", "hex 小写输入")
assert(encoding.hex_encode("") == "", "hex 空串")

# url 编码（验证标准 7）
assert(encoding.url_encode("a b/c") == "a%20b/c", "url safe=/ 缺省")
assert(encoding.url_encode("a b/c", "") == "a%20b%2Fc", "url 自定义 safe")
assert(encoding.url_encode("AZaz09-_.~") == "AZaz09-_.~", "url 非保留字符不转义")
assert(encoding.url_encode("中") == "%E4%B8%AD", "url UTF-8 逐字节大写 %HH")
assert(encoding.url_encode("100%") == "100%25", "url 百分号转义")
assert(encoding.url_encode("a&b=c?d") == "a%26b%3Dc%3Fd", "url 保留符号转义")

# url 解码：往返一致 + `+` 保持字面 + 小写 %xx 接受
assert(encoding.url_decode("a%20b/c") == "a b/c", "url 解码")
assert(encoding.url_decode("a+b") == "a+b", "加号保持字面（非 form 语义）")
assert(encoding.url_decode("%e4%b8%ad") == "中", "小写 %xx 解码")
for s in samples {
    assert(encoding.url_decode(encoding.url_encode(s)) == s, "url 往返")
}

print("ALL PASSED")
