# test_string_ext.ms — string 扩充（task 80）
# 覆盖: 16-stdlib-expansion.md §4.2 值域抽查 + format 增强
# （错误路径 {:.2f} 于 "x" / 单独 } / {:x} / {:.10f} 为原生 Err，
#   不经 try/except，由 Rust 单测 test_integration_string_format_error_paths 覆盖）

import string

# count / find
assert(string.count("aaa", "a") == 3, "count")
assert(string.count("aaaa", "aa") == 2, "count 非重叠")
assert(string.count("abc", "") == 0, "count 空 sub")
assert(string.find("hello", "ll") == 2, "find")
assert(string.find("日本語", "本") == 1, "find 字符位置")
assert(string.find("hello", "xx") == -1, "find 未找到 -1")

# 大小写
assert(string.title("hello world") == "Hello World", "title")
assert(string.capitalize("hello WORLD") == "Hello world", "capitalize")

# 填充（n 为结果总长）
assert(string.pad_start("42", 5) == "   42", "pad_start 默认空格")
assert(string.pad_start("42", 5, "0") == "00042", "pad_start 指定 pad")
assert(string.pad_end("42", 5, "*") == "42***", "pad_end")
assert(string.pad_start("hello", 3) == "hello", "已长于 n 返回原串")
assert(string.center("abc", 10) == "   abc    ", "center 左短右长")
assert(string.center("abc", 7, "-") == "--abc--", "center 指定 pad")
assert(string.zfill("-42", 5) == "-0042", "zfill 符号保留")
assert(string.zfill("42", 5) == "00042", "zfill")
assert(string.zfill("+42", 5) == "+0042", "zfill 正号")

# 行分割 / 修剪
lines = string.split_lines("a\nb\r\nc\rd")
assert(len(lines) == 4, "split_lines 三种行尾")
assert(lines[0] == "a" and lines[1] == "b" and lines[2] == "c" and lines[3] == "d", "行内容")
assert(len(string.split_lines("a\n")) == 1, "尾行尾不产生空行")
assert(len(string.split_lines("")) == 0, "空串为空 list")
assert(string.trim_start("  x  ") == "x  ", "trim_start")
assert(string.trim_end("  x  ") == "  x", "trim_end")

# 谓词
assert(string.is_alnum("abc123"), "is_alnum")
assert(not string.is_alnum(""), "is_alnum 空串")
assert(not string.is_alnum("a b"), "is_alnum 含空白")
assert(string.is_space(" \t\n"), "is_space")
assert(not string.is_space(""), "is_space 空串")
assert(string.is_upper("ABC1"), "is_upper")
assert(not string.is_upper("abc"), "is_lower 字符")
assert(not string.is_upper("123"), "is_upper 需有大小写字母")
assert(string.is_lower("abc1"), "is_lower")
assert(not string.is_lower("123"), "is_lower 需有大小写字母")

# cut / fields / join
before, after = string.cut("a,b,c", ",")
assert(before == "a" and after == "b,c", "cut 首个 sep")
b2, a2 = string.cut("abc", ",")
assert(b2 == "abc" and a2 == "", "cut 无 sep → (s, \"\")")
f = string.fields("  a \t b  ")
assert(len(f) == 2 and f[0] == "a" and f[1] == "b", "fields 连续空白")
assert(string.join("-", ["a", "b", "c"]) == "a-b-c", "模块级 join")

# format 增强
assert(string.format("{:.2f}", 3.14159) == "3.14", "{:.2f} Float")
assert(string.format("{:.2f}", 3) == "3.00", "Int 按 Float 格式化")
assert(string.format("{:.0f}", 3.7) == "4", "N=0")
assert(string.format("{{}}") == "{}", "{{}} 转义")
assert(string.format("{{a}}") == "{a}", "{{ 与 }} 组合")
assert(string.format("x = {:.1f}, y = {}", 2.26, 7) == "x = 2.3, y = 7", "混合占位")

print("ALL PASSED")
