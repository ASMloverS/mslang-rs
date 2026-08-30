# test_sys.ms — sys 模块（task 82）
# 覆盖: platform/version/executable（验证标准 7；
#        stdin_read_all 管道用例在 tests/sys_stdin.rs 子进程级执行）

import sys

p = sys.platform()
assert(p == "windows" or p == "linux" or p == "macos", "platform 枚举")

v = sys.version()
assert(v.startswith("mslang"), "version 前缀")

assert(len(sys.executable()) > 0, "executable 非空")

print("ALL PASSED")
