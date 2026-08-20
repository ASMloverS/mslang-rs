# test_os_time_path.ms — os / time / path 模块
# 覆盖: task 48（10-builtins.md；只测无副作用与可逆操作）

import os
import time
import path

# path.join 平台分隔符
joined = path.join("a", "b", "c")
sep = "/"
assert(joined == "a/b/c" or joined == "a\\b\\c", "join 多段")
assert(path.join("alone") == "alone", "join 单段")

# path.base / ext / dir
assert(path.base("dir/file.txt") == "file.txt", "base unix")
assert(path.ext("archive.tar.gz") == ".gz", "ext 最后扩展名")
assert(path.ext("noext") == "", "ext 无扩展")
assert(path.dir("a/b/c.txt") == "a/b", "dir unix")

# path 往返：dir + base 还原（分隔符自适应）
full = path.join("x", "y", "name.ms")
assert(path.base(full) == "name.ms", "join 后 base")

# os.getcwd 返回非空字符串
cwd = os.getcwd()
assert(type(cwd) == "string" and len(cwd) > 0, "getcwd")

# os.args：列表，至少含解释器相关项
assert(type(os.args) == "list" and len(os.args) >= 1, "args 列表")

# os.getenv：未定义变量返回 nil；已定义返回字符串
assert(os.getenv("MSLANG_UNDEFINED_VAR_XYZ") == nil, "getenv 未定义")
os.setenv("MSLANG_TEST_VAR", "42")
assert(os.getenv("MSLANG_TEST_VAR") == "42", "setenv + getenv")

# os.chdir 往返（切到 cwd 自身，无副作用）
os.chdir(os.getcwd())
assert(len(os.getcwd()) > 0, "chdir 后仍可 getcwd")

# time.now：单调合理的时间戳（> 2020-01-01）
assert(time.now() > 1577836800.0, "now 时间戳量级")
# 连续两次调用非递减
t1 = time.now()
t2 = time.now()
assert(t2 >= t1, "now 非递减")

# time.format 返回字符串
fmt = time.format(time.now())
assert(type(fmt) == "string" and len(fmt) > 0, "format 字符串")

print("ALL PASSED")
