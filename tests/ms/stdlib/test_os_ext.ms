# test_os_ext.ms — os 扩充（task 82）
# 覆盖: getpid/hostname/environ/unsetenv/run（验证标准 5-6；
#        os.run 按平台选命令；参数违规负路径由 Rust 单测覆盖）

import os
import sys

assert(os.getpid() > 0, "getpid 正整数")

e = os.environ()
assert(len(e) > 0, "environ 非空")
assert("PATH" in e, "environ 含 PATH 键")

os.setenv("__MSLANG_TEST_EXT__", "1")
assert(os.getenv("__MSLANG_TEST_EXT__") == "1")
os.unsetenv("__MSLANG_TEST_EXT__")
assert(os.getenv("__MSLANG_TEST_EXT__") == nil, "unsetenv 后 getenv nil")

if sys.platform() == "windows" {
    r = os.run(["cmd", "/C", "echo", "hi"])
} else {
    r = os.run(["sh", "-c", "echo hi"])
}
assert(r["status"] == 0, "run status 0")
assert("hi" in r["stdout"], "stdout 含 hi")
assert(type(r["stderr"]) == "string", "stderr 为 string")

print("ALL PASSED")
