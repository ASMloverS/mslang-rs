# test_hash.ms — hash 模块（task 85）
# 覆盖: 验证标准 9——空串与 "abc" 标准向量断言（RFC 1321 / FIPS 180-1/-2）

import hash

# 空串标准向量
assert(hash.md5("") == "d41d8cd98f00b204e9800998ecf8427e")
assert(hash.sha1("") == "da39a3ee5e6b4b0d3255bfef95601890afd80709")
assert(hash.sha256("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
assert(hash.sha512("") == "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e")

# "abc" 标准向量
assert(hash.md5("abc") == "900150983cd24fb0d6963f7d28e17f72")
assert(hash.sha1("abc") == "a9993e364706816aba3e25717850c26c9cd0d89d")
assert(hash.sha256("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
assert(hash.sha512("abc") == "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")

# 长度 / 小写 hex 不变式
assert(len(hash.md5("x")) == 32)
assert(len(hash.sha1("x")) == 40)
assert(len(hash.sha256("x")) == 64)
assert(len(hash.sha512("x")) == 128)
HEX = "0123456789abcdef"
for h in [hash.md5("mslang"), hash.sha1("mslang"), hash.sha256("mslang"), hash.sha512("mslang")] {
    i = 0
    while i < len(h) {
        assert(h[i] in HEX, "小写 hex 字符集")
        i = i + 1
    }
}

# UTF-8 字节输入（中文）与确定性
assert(hash.md5("你好") == hash.md5("你好"))
assert(hash.md5("你好") != hash.md5("你好!"))
assert(hash.sha256("中文输入") == hash.sha256("中文输入"))

print("ALL PASSED")
