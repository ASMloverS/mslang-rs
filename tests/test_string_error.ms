# 内置 String 方法 - 错误契约演示（task 50）。
# 注：当前 VM 中原生函数错误（Err(String)）尚不可被 try/except 捕获（仅显式 throw 可
# 捕获）；错误经 run() 以 Err 返回。各错误消息（TypeError/ValueError/AttributeError 前缀）
# 已由 src/vm/stdlib.rs 的 Rust 单元测试直接调用 native 函数验证。本文件记录预期契约，
# 待 VM 原生错误可捕获后即可端到端运行。
#
# 预期输出（恢复捕获后）：
#   split: ValueError: empty separator
#   index: ValueError: substring 'xx' not found
#   join: TypeError: join() expects list of strings, got int
#   slice: ValueError: slice start 3 > end 1
#   attr: AttributeError: 'string' has no attribute 'nosuch'

# split 空分隔符
try {
    "abc".split("")
} except e {
    print("split: " + str(e))
}
# index 未找到
try {
    "hello".index("xx")
} except e {
    print("index: " + str(e))
}
# join 非 string 元素
try {
    "-".join([1, 2, 3])
} except e {
    print("join: " + str(e))
}
# slice 反向（归一化后 start > end）
try {
    "hello".slice(3, 1)
} except e {
    print("slice: " + str(e))
}
# 未知方法
try {
    "hello".nosuch()
} except e {
    print("attr: " + str(e))
}
