# test_literals.ms — 字面量与类型
# 覆盖: task 04/05/06（01-lexical.md），type() 返回类型名

# 整数字面量：十进制 / 十六进制 / 二进制 / 八进制
assert(42 == 42, "十进制")
assert(0xFF == 255, "十六进制小写前缀")
assert(0X10 == 16, "十六进制大写前缀")
assert(0b1010 == 10, "二进制")
assert(0B11 == 3, "二进制大写前缀")
assert(0o755 == 493, "八进制")
assert(0 == 0, "零")

# 浮点字面量与科学计数法
assert(1.5 + 0.5 == 2.0, "小数浮点")
assert(1.5e3 == 1500.0, "科学计数法正指数")
assert(2e-2 == 0.02, "科学计数法负指数")

# type() 返回类型名字符串
assert(type(1) == "int", "type int")
assert(type(1.0) == "float", "type float")
assert(type(true) == "bool", "type bool")
assert(type(nil) == "nil", "type nil")
assert(type("s") == "string", "type string")
assert(type([1]) == "list", "type list")
assert(type({}) == "dict", "type dict")
assert(type((1,)) == "tuple", "type tuple")
assert(type({1,}) == "set", "type set")

# 字符串转义序列（字符串不可跨行；无单引号字符串语法）
assert(len("\n") == 1, "转义换号为单字符")
assert(len("\t") == 1, "转义制表符为单字符")
assert("a\tb" == "a" + "\t" + "b", "转义参与拼接")
quoted = "say \"hi\""
assert(quoted == "say " + "\"" + "hi" + "\"", "转义双引号")
assert("a\\b" == "a" + "\\" + "b", "转义反斜杠")

# 布尔与 nil 字面量
assert(true, "true 字面量")
assert(not false, "false 字面量取反")
assert(nil == nil, "nil 等于 nil")

# var / := / = 三种声明等价
a1 = 10
var a2 = 10
a3 := 10
assert(a1 == a2 and a2 == a3, "三种声明方式等价")

# const 常量（编译期常量表达式）
const ANSWER = 42
assert(ANSWER == 42, "const 基本常量")
const MASK = 0xFF | 0x0F
assert(MASK == 255, "const 位运算表达式")

print("ALL PASSED")
