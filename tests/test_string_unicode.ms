# 内置 String 方法 - Unicode 字符位置一致性（task 50）
# index 返回字符位置（非字节位置），与 slice/length 互相对应。
print("日本語".index("本"))      # 字符位置 1（非字节位置 3）
# length/index/slice 组合使用：截取从"本"开始的 1 个字符
i = "日本語".index("本")
print("日本語".slice(i, i + 1))  # 本
# 负索引：相对末尾
print("hello".slice(-1))         # o
print("hello".slice(1, -1))      # ell（去首尾）
# 越界饱和：返回空串，不报错
print("hello".slice(100, 200))   # （空行）
