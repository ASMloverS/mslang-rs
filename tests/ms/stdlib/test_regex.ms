# test_regex.ms — regex 模块（task 85）
# 覆盖: 验证标准 1-8、10 的非错误路径（16-stdlib-expansion.md §4.16）
# 错误路径（非法 pattern / ${99} 越界 / count<0 / group 负索引 / repl 函数
# 返回非 string）为原生 Err，不经 try/except（task 80 惯例），
# 由 Rust 单测（stdlib/regex.rs tests）覆盖。

import regex

# ---------------------------------------------------------------------------
# 验证标准 1：search 的 group/start/end/span
# ---------------------------------------------------------------------------
m = regex.search("l+", "hello")
assert(m.group(0) == "ll")
assert(m.start() == 2)
assert(m.end() == 4)
assert(m.span() == (2, 4))

# 验证标准 2：match 锚定开头
assert(regex.match("h", "hello") != nil)
assert(regex.match("e", "hello") == nil)
assert(regex.match("hel", "hello").group(0) == "hel")

# 多分组：groups / group(i) / 未参组 nil
g = regex.search("(\\w+)-(\\d+)?", "ab-")
assert(g.group(0) == "ab-")
assert(g.group(1) == "ab")
assert(g.group(2) == nil)
assert(g.groups() == ("ab", nil))

g2 = regex.search("(a)(b)", "ab")
assert(g2.groups() == ("a", "b"))
assert(g2.group(2) == "b")

# ---------------------------------------------------------------------------
# 验证标准 3：findall 三态（0/1/多组）+ 中文输入字符偏移
# ---------------------------------------------------------------------------
assert(regex.findall("l+", "hello") == ["ll"])
assert(regex.findall("x+", "hello") == [])
assert(regex.findall("\\d", "a1b22c333") == ["1", "2", "2", "3", "3", "3"])
assert(regex.findall("(\\d)", "a1b2") == ["1", "2"])
assert(regex.findall("(\\d)-(\\w)", "1-a 2-b") == [("1", "a"), ("2", "b")])

# 中文：字节 3/字符 1，findall 与偏移不受字节宽度影响
zh = regex.findall("世", "你好世界")
assert(zh == ["世"])
z = regex.search("世界", "你好世界")
assert(z.start() == 2 and z.end() == 4)
assert(z.span() == (2, 4))
assert(z.group(0) == "世界")
assert(z.start() == "你好世界".index("世界"))
z2 = regex.findall("(好)(世)", "你好世界")
assert(z2 == [("好", "世")])

# ---------------------------------------------------------------------------
# 验证标准 4：sub 分组展开 / count 截断 / repl 函数回调
# ---------------------------------------------------------------------------
assert(regex.sub("(\\w+)-(\\w+)", "${2}-${1}", "ab-cd") == "cd-ab")
assert(regex.sub("l", "L", "hello", 1) == "heLlo")
assert(regex.sub("l", "L", "hello") == "heLLo")
assert(regex.sub("x", "y", "hello") == "hello")
assert(regex.sub("o", "0", "foo", 2) == "f00")
assert(regex.sub("(a+)", "<${1}>", "baaa") == "b<aaa>")
# ${0} 整体引用 + 字面 $ 透传
assert(regex.sub("l+", "[${0}]", "hello") == "he[ll]o")
assert(regex.sub("l", "$x", "hello") == "he$x$xo")

# repl 函数：接收 Match 返回 string
up = fn(mm) { return mm.group(0).upper() }
assert(regex.sub("[a-z]+", up, "ab cd") == "AB CD")
wrap = fn(mm) { return "<" + mm.group(1) + ">" }
assert(regex.sub("(\\d)", wrap, "a1b2") == "a<1>b<2>")

# sub 后 s.index 位置语义一致（字符偏移，非字节）
after = regex.sub("世", "W", "你好世界")
assert(after == "你好W界")

# ---------------------------------------------------------------------------
# 验证标准 6：split 未匹配返回 [s]；多分隔正确；同名交叉回归（§2.2）
# ---------------------------------------------------------------------------
assert(regex.split(",", "a,b,c") == ["a", "b", "c"])
assert(regex.split("x", "abc") == ["abc"])
assert(regex.split("\\d", "a1b22c") == ["a", "b", "", "c"])
assert(regex.split(",\\s*", "a, b ,c") == ["a", "b ", "c"])
assert(regex.split("\\s*,\\s*", "a , b , c") == ["a", "b", "c"])

# string 方法与 regex 模块同名 split 并存（split 注册 MAX，各自自校验）
s = "a,b c"
assert(s.split() == ["a,b", "c"])
assert(s.split(",") == ["a", "b c"])
assert(regex.split(",", s) == ["a", "b c"])
assert(regex.split("\\s+", s) == ["a,b", "c"])

# ---------------------------------------------------------------------------
# 验证标准 7：compile 对象与函数式等价；pattern() 回读
# ---------------------------------------------------------------------------
re = regex.compile("l+")
assert(re.pattern() == "l+")
m2 = re.search("hello")
assert(m2.group(0) == "ll")
assert(m2.start() == 2 and m2.end() == 4)
assert(re.match("llo").group(0) == "ll")
assert(re.match("hello") == nil)
assert(re.findall("hello") == ["ll"])
assert(re.sub("L", "hello", 1) == "heLo")
assert(re.sub("L", "hello") == "heLo")
assert(re.split("hello") == ["he", "o"])

rd = regex.compile("(\\d+)-(\\d+)")
assert(rd.sub("${2}-${1}", "12-34") == "34-12")
assert(rd.findall("1-2 3-45") == [("1", "2"), ("3", "45")])

# 对象与函数式结果等价（同一 pattern 两入口）
fa = regex.findall("\\d+", "a1b22")
fb = regex.compile("\\d+").findall("a1b22")
assert(fa == fb and fa == ["1", "22"])

# type() 名称
assert(type(regex.compile("a")) == "regex")
assert(type(regex.search("a", "a")) == "match")

print("ALL PASSED")
