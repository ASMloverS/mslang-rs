# test_string_methods.ms — 内置 String 方法
# 覆盖: task 50（10-builtins.md § String 方法）

s = "Hello World"

# 大小写转换
assert(s.lower() == "hello world", "lower")
assert(s.upper() == "HELLO WORLD", "upper")
assert("aBc".upper() == "ABC", "混合大小写 upper")

# 去除两端空白
assert("  trim  ".strip() == "trim", "strip 两端空白")
assert("\ttab\t".strip() == "tab", "strip 制表符")
assert("x".strip() == "x", "无需 strip")

# 分割与连接
parts = "a,b,c".split(",")
assert(parts == ["a", "b", "c"], "split 按分隔符")
assert(len("a  b   c".split()) == 3, "无参 split 按空白")
assert("-".join(["a", "b", "c"]) == "a-b-c", "join 列表")
assert("".join(["x", "y"]) == "xy", "join 空分隔符")

# 替换与查找
assert("hello".replace("l", "r") == "herro", "replace 全部")
assert("aaa".replace("aa", "b") == "ba", "replace 非重叠")
assert("hello".contains("ell"), "contains 存在")
assert(("hello".contains("xyz")) == false, "contains 不存在")

# 前缀后缀
assert("hello".startswith("hel"), "startswith")
assert(("hello".startswith("ell")) == false, "startswith 不匹配")
assert("hello".endswith("llo"), "endswith")
assert(("hello".endswith("hel")) == false, "endswith 不匹配")

# index 查找子串位置
assert("hello".index("ll") == 2, "index 命中")
assert("hello".index("h") == 0, "index 首字符")

# slice 切片方法
assert("hello".slice(1, 3) == "el", "slice 区间")
assert("hello".slice(0, 2) == "he", "slice 从零开始")

# length
assert("hello".length() == 5, "length 方法")
assert(("日本語").length() == 3, "UTF-8 字符计数")

# 不可变性：所有方法返回新串，原串不变
orig = "abc"
_ = orig.upper()
assert(orig == "abc", "方法调用不改变原串")

print("ALL PASSED")
