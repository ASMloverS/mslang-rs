# test_stdlib_misc.ms — 标准库综合交互
# 覆盖: task 46-49 交叉场景（json 值经 io 持久化往返）

import io
import json
import math

F = "mslang_test_misc.json"

# 构造数据 → stringify → 写文件 → 读回 → parse → 相等
data = {
    "kind": "report",
    "count": 42,
    "ratio": 0.75,
    "flags": [true, false],
    "tags": {"lang": "ms", "ver": [1, 0]}
}
io.write_file(F, json.stringify(data))
loaded = json.parse(io.read_file(F))
assert(loaded == data, "json 文件 roundtrip")
assert(loaded["tags"]["ver"] == [1, 0], "嵌套取值")
assert(loaded["ratio"] == 0.75, "浮点字段")

# 数学结果入 json
report = {
    "root2": math.sqrt(4),
    "pi10": math.round(math.pi * 1000),
}
io.write_file(F, json.stringify(report))
back = json.parse(io.read_file(F))
assert(back["root2"] == 2.0, "sqrt 结果持久化")
assert(back["pi10"] == 3142, "round 结果持久化")
io.write_file(F, "")

# 多模块协作：字符串格式化 + path
import path
import string
conf_path = path.join("conf", "app.json")
assert(string.format("loading {}", path.base(conf_path)) == "loading app.json", "format + base 协作")

print("ALL PASSED")
