# test_embedded.ms — 嵌入式 .ms 标准库机制
# 覆盖: task 79（16-stdlib-expansion.md §3.2）；task 84 填充真实实现

import collections   # task 84 起为真实实现（deque/Counter/defaultdict）
assert(type(collections) == "module", "embedded module loadable")
assert(type(collections.deque) == "class", "real exports accessible")

# 其余三个模块同样可载入且导出真实内容
import itertools
import functools
import test as testmod
assert(type(itertools.count) == "function", "itertools real export")
assert(type(functools.memoize) == "function", "functools real export")
assert(type(testmod.assert_eq) == "function", "test real export")

# 嵌入模块缓存命中：重复 import 返回同一 Module 对象（id 相同）
import collections as c2
assert(id(collections) == id(c2), "embedded module cached")

print("ALL PASSED")
