# 期望：TypeError —— 调用未定义（nil）标识符
# 注：当前 VM 读取未定义全局返回 nil（MVP 行为），调用处才报错
undefined_function_xyz(1)
