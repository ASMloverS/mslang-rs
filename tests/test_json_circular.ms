# json 循环引用检测演示（task 49）。
# 注：当前 VM 中原生函数错误尚不可被 try/except 捕获；错误消息已由 Rust 单元测试验证。
# 待 VM 原生错误可捕获后即可端到端运行。
import json

# 循环引用：list 自引用
a = []
a.push(a)
try {
    json.stringify(a)
} except e {
    print("caught: " + str(e))
}

# 循环引用：dict 互引用
d1 = {}
d2 = {}
d1["link"] = d2
d2["back"] = d1
try {
    json.stringify(d1)
} except e {
    print("caught: " + str(e))
}
