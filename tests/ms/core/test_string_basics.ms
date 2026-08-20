# test_string_basics.ms — 字符串基础操作
# 覆盖: task 21/22（02-types.md § String），10-builtins.md § len

s = "hello"

# 长度与索引
assert(len(s) == 5, "len")
assert(s[0] == "h", "首字符索引")
assert(s[4] == "o", "末字符索引")
assert(s[-1] == "o", "负索引 -1")
assert(s[-5] == "h", "负索引 -len")

# 拼接
assert("ab" + "cd" == "abcd", "字符串拼接")
assert("" + "x" == "x", "空串拼接")

# str() 转换与拼接
assert(str(42) == "42", "str(int)")
assert(str(nil) == "nil", "str(nil)")
assert(str(true) == "true", "str(bool)")
assert("n = " + str(7) == "n = 7", "str 用于拼接")

# int()/float() 从字符串解析
assert(int("42") == 42, "int 解析十进制字符串")
assert(int("-17") == -17, "int 解析负数")
assert(float("3.5") == 3.5, "float 解析")

# 比较与成员
assert("a" < "b", "字典序")
assert("ll" in "hello", "子串成员")
assert(("LL" in "hello") == false, "成员区分大小写")

# 遍历字符
vowels = 0
for ch in "aeiou" {
    vowels += 1
}
assert(vowels == 5, "for 遍历字符串")

# 索引超出范围的负向边界由 negative 用例覆盖

# bool() 语义
assert(bool("") == false, "空串为假")
assert(bool("a"), "非空串为真")

print("ALL PASSED")
