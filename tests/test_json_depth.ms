# json 嵌套深度限制演示（task 49，MAX_NESTING=1000）。
# 注：当前 VM 中原生函数错误尚不可被 try/except 捕获；错误消息已由 Rust 单元测试验证。
# 待 VM 原生错误可捕获后即可端到端运行。
import json

# 构造 1001 层嵌套（超出 MAX_NESTING=1000）
depth = 1001
s = "[" * depth + "]" * depth
try {
    json.parse(s)
} except e {
    print("parse caught: " + str(e))
}

# stringify 超深嵌套（递归构造深 list）
nested = 1
i = 0
while i < 1001 {
    nested = [nested]
    i = i + 1
}
try {
    json.stringify(nested)
} except e {
    print("stringify caught: " + str(e))
}
