# json 错误契约演示（task 49）。
# 注：当前 VM 中原生函数错误（Err(String)）尚不可被 try/except 捕获（仅显式 throw 可
# 捕获）；错误经 run() 以 Err 返回。各错误消息（ValueError/TypeError 前缀）已由
# src/vm/stdlib.rs 的 Rust 单元测试直接调用 native 函数验证。本文件记录预期契约，
# 待 VM 原生错误可捕获后即可端到端运行。
import json

# parse 非法 JSON → ValueError（行列号定位，仅含位置，不含原文片段）
try {
    json.parse("invalid json")
} except e {
    print("caught: " + str(e))
}

# stringify NaN → ValueError（RFC 8259 不允许）
try {
    nan = 0.0 / 0.0
    json.stringify(nan)
} except e {
    print("caught: " + str(e))
}

# stringify 不可序列化类型（function）→ TypeError
try {
    json.stringify(print)
} except e {
    print("caught: " + str(e))
}

# 缺参 → TypeError（CALL 站点 arity 校验）
try {
    json.stringify()
} except e {
    print("caught: " + str(e))
}
