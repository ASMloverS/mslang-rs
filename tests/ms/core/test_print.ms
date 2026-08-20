# test_print.ms — print 输出格式（输出比对类，配 test_print.expected）
# 覆盖: task 25（10-builtins.md § print：参数空格分隔，追加换行）

# 多参数空格分隔
print("a", "b", 1, 2)

# 各类型显示格式
print(42)
print(-17)
print(3.5)
print(true)
print(false)
print(nil)
print("str")
print([1, 2, 3])
print((1, "a"))
print({"k": 1, "j": 2})
# set 显示为哈希序（不稳定），不经输出比对（相等性由 test_collections 覆盖）
print([1, [2, 3], (4,)])

# 嵌套容器
print({"list": [1, {"inner": true}]})

# 无参数
print()

# str() 与 print 一致性由上面覆盖；f-string 不存在，拼接用 +
print("x = " + str(10))

print("ALL PASSED")
