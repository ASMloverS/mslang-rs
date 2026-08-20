# test_string_module.ms — string 模块（模块级函数，与实例方法区分）
# 覆盖: task 48（10-builtins.md § string 模块）

import string

# format 位置占位
assert(string.format("{} + {} = {}", 1, 2, 3) == "1 + 2 = 3", "format 三占位")
assert(string.format("hello {}", "ms") == "hello ms", "format 单占位")
assert(string.format("no placeholders") == "no placeholders", "无占位")
assert(string.format("{}-{}", "a", "b") == "a-b", "字符串参数")
assert(string.format("{}", true) == "true", "bool 参数")
assert(string.format("{}", nil) == "nil", "nil 参数")

# repeat
assert(string.repeat("ab", 3) == "ababab", "repeat 三次")
assert(string.repeat("x", 0) == "", "repeat 零次")
assert(string.repeat("", 5) == "", "空串 repeat")

# reverse
assert(string.reverse("abc") == "cba", "reverse")
assert(string.reverse("") == "", "reverse 空串")
assert(string.reverse("a") == "a", "reverse 单字符")

# is_alpha / is_digit
assert(string.is_alpha("abc"), "is_alpha 真")
assert((string.is_alpha("ab1")) == false, "is_alpha 含数字")
assert((string.is_alpha("")) == false, "is_alpha 空串")
assert(string.is_digit("123"), "is_digit 真")
assert((string.is_digit("12a")) == false, "is_digit 含字母")
assert((string.is_digit("")) == false, "is_digit 空串")

print("ALL PASSED")
