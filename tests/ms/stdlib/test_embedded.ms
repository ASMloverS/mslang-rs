# test_embedded.ms — 嵌入式 .ms 标准库机制
# 覆盖: task 79（16-stdlib-expansion.md §3.2）

import collections   # 占位模块导出 VERSION 占位常量
assert(type(collections) == "module", "embedded module loadable")
assert(type(collections.VERSION) == "string", "placeholder constant accessible")

# 其余三个占位模块同样可载入
import itertools
import functools
import test as testmod
assert(itertools.VERSION == "0.1.0", "itertools placeholder")
assert(functools.VERSION == "0.1.0", "functools placeholder")
assert(testmod.VERSION == "0.1.0", "test placeholder")

# 嵌入模块缓存命中：重复 import 返回同一 Module 对象（id 相同）
import collections as c2
assert(id(collections) == id(c2), "embedded module cached")

print("ALL PASSED")
