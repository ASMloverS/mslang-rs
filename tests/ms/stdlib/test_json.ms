# test_json.ms — json 模块（解析与序列化）
# 覆盖: task 49（10-builtins.md § json）

import json

# parse：各标量类型
assert(json.parse("42") == 42, "parse int")
assert(json.parse("2.5") == 2.5, "parse float")
assert(json.parse("true"), "parse true")
assert((json.parse("false")) == false, "parse false")
assert(json.parse("null") == nil, "parse null")
assert(json.parse("\"txt\"") == "txt", "parse string")

# parse：数组与嵌套
arr = json.parse("[1, 2, 3]")
assert(arr == [1, 2, 3], "parse 数组")
nested = json.parse("[[1], [2, [3]]]")
assert(nested == [[1], [2, [3]]], "嵌套数组")

# parse：对象 → dict
obj = json.parse("{\"x\": 1, \"flag\": true}")
assert(obj["x"] == 1, "对象数值字段")
assert(obj["flag"], "对象布尔字段")

# parse：复杂嵌套（对象含数组含对象）
doc = json.parse("{\"items\": [{\"id\": 1}, {\"id\": 2}]}")
assert(doc["items"][1]["id"] == 2, "深度嵌套取值")

# stringify：紧凑格式（RFC 8259，无空格分隔符）
assert(json.stringify({"a": 1}) == "{\"a\":1}", "stringify 对象")
assert(json.stringify([1, "x", true, nil]) == "[1,\"x\",true,null]", "stringify 数组各类型")
assert(json.stringify("str") == "\"str\"", "stringify 字符串带引号")
assert(json.stringify(3.5) == "3.5", "stringify 浮点")
assert(json.stringify(7) == "7", "stringify 整数")
assert(json.stringify(nil) == "null", "stringify nil → null")
assert(json.stringify({"k": [1, {"z": nil}]}) == "{\"k\":[1,{\"z\":null}]}", "嵌套序列化")

# 空容器
assert(json.parse("{}") == {}, "parse 空对象")
assert(json.parse("[]") == [], "parse 空数组")
assert(json.stringify({}) == "{}", "stringify 空对象")

# roundtrip：序列化再解析保持值相等
src = {"name": "ms", "versions": [1, 2, 3], "meta": {"stable": true}}
rt = json.parse(json.stringify(src))
assert(rt == src, "roundtrip 相等")

# 转义字符
esc = json.parse("\"line1\\nline2\"")
assert(esc == "line1\nline2", "parse 转义换行")

print("ALL PASSED")
