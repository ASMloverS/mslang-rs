# test_stack_trace.ms — 运行时错误堆栈跟踪（输出比对类）
# 覆盖: task 57（11-bytecode-vm.md § 栈跟踪；错误格式化）
# 注：本脚本预期整体失败退出，stderr 含堆栈跟踪，由 negative 机制校验——
# 但它是正面验证格式：改用「捕获后断言 traceback 内容」不可行（原生错误
# 不可捕获），故本文件按 negative 方式配置 .expected。

fn divide(a, b) {
    return a / b
}

fn calculate() {
    return divide(10, 0)
}

calculate()
