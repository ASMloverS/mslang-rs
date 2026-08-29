# test_uuid.ms — uuid 模块（task 81）
# 覆盖: uuid4 格式/版本位/唯一性（验证标准 9）

import uuid

HEXCHARS = "0123456789abcdef-"

u = uuid.uuid4()

# 36 字符 + 连字符位置（8/13/18/23，0-based）
assert(len(u) == 36, "长度 36")
assert(u[8] == "-" and u[13] == "-" and u[18] == "-" and u[23] == "-", "连字符位置")

# 第 13 个 hex 字符 = '4'（version，全串 0-based 下标 14）；
# 第 17 个 hex 字符 ∈ 89ab（variant，全串 0-based 下标 19）
assert(u[14] == "4", "version 4 位")
v = u[19]
assert(v == "8" or v == "9" or v == "a" or v == "b", "variant 位 89ab")

# 全字符 ∈ 小写 hex + '-'
ok = true
i = 0
while i < 36 {
    ok = ok and (u[i] in HEXCHARS)
    i = i + 1
}
assert(ok, "字符集为小写 hex + '-'")

# 连续生成 100 个互不相同
seen = {}
j = 0
while j < 100 {
    x = uuid.uuid4()
    assert(len(x) == 36 and x[14] == "4", "批量生成格式")
    seen[x] = true
    j = j + 1
}
assert(len(seen) == 100, "100 个互不相同")

print("ALL PASSED")
